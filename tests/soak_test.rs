//! Long-running soak test at scale (#392).
//!
//! Grows a file-backed database to millions of facts in batches of mixed
//! sizes, with multi-valued attributes, retracts, re-asserts and long
//! histories on a few hot entities. It checkpoints on a schedule, reopens at
//! random, and checks samples against a reference through EAVT, AVET, AEVT,
//! `:as-of` and, at the end, a full scan. Metrics go to a JSON-lines file.
//!
//! `soak_short` runs a tiny configuration in the per-PR suite. The full run
//! is ignored; `.github/workflows/soak.yml` runs it weekly:
//!
//! ```text
//! MINIGRAF_SOAK_MINUTES=20 MINIGRAF_SOAK_TARGET_FACTS=10000000 \
//!   cargo test --profile bench --test soak_test soak -- --ignored --nocapture
//! ```
//!
//! Other variables: `MINIGRAF_SOAK_SEED` (replays the write sequence; the
//! time-based schedule still varies), `MINIGRAF_SOAK_CHECK_MINUTES`,
//! `MINIGRAF_SOAK_CHURN_TPS`, `MINIGRAF_SOAK_METRICS`, `MINIGRAF_SOAK_DIR`.
//! Design: `docs/superpowers/specs/2026-10-10-soak-test-design.md`.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::db::{Minigraf, OpenOptions};
use minigraf::{QueryResult, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use uuid::Uuid;

// ── Random numbers ────────────────────────────────────────────────────────────

/// SplitMix64 finalizer: a fixed hash, independent of any crate version.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Log-uniform in `lo..=hi`: small batches are common, large ones occur.
    fn log_uniform(&mut self, lo: u64, hi: u64) -> u64 {
        let (l, h) = ((lo as f64).ln(), ((hi + 1) as f64).ln());
        ((l + (h - l) * self.unit()).exp() as u64).clamp(lo, hi)
    }
}

// ── Facts ─────────────────────────────────────────────────────────────────────

const SHARDS: u64 = 1024;
const HOT: usize = 32;
const HOT_STATES: usize = 8;
const HOT_TAGS: usize = 16;
const HOT_NOTES: usize = 3;
const HOT_SLOTS: usize = HOT_STATES + HOT_TAGS + HOT_NOTES;
/// Re-asserts are chosen once this many ordinary slots are retracted.
const FLIP_CAP: usize = 200_000;

#[derive(Clone)]
enum Val {
    Int(i64),
    Kw(String),
    Str(String),
    /// A ref to ordinary entity `n`.
    Ref(u64),
}

impl Val {
    fn edn(&self) -> String {
        match self {
            Val::Int(n) => n.to_string(),
            Val::Kw(k) => k.clone(),
            Val::Str(s) => format!("\"{s}\""),
            Val::Ref(n) => format!("#uuid \"{}\"", entity_uuid(&format!(":e{n}"))),
        }
    }

    fn render(&self) -> String {
        match self {
            Val::Int(n) => format!("i{n}"),
            Val::Kw(k) => format!("k{k}"),
            Val::Str(s) => render_str(s),
            Val::Ref(n) => format!("r:e{n}"),
        }
    }
}

fn render_str(s: &str) -> String {
    if s.len() <= 24 {
        format!("s\"{s}\"")
    } else {
        format!("s\"{}..{}\"#{}", &s[..12], &s[s.len() - 8..], s.len())
    }
}

/// The UUID a keyword entity such as `:e5` maps to.
fn entity_uuid(keyword: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, keyword.as_bytes())
}

/// Renders a query value; refs and entities appear by name, never as a `Uuid`.
fn render_value(v: &Value, names: &HashMap<Uuid, String>) -> String {
    match v {
        Value::Integer(n) => format!("i{n}"),
        Value::Keyword(k) => format!("k{k}"),
        Value::String(s) => render_str(s),
        Value::Ref(u) => match names.get(u) {
            Some(name) => format!("r{name}"),
            None => "r?".to_string(),
        },
        Value::Boolean(b) => format!("b{b}"),
        Value::Float(f) => format!("f{f}"),
        Value::Null => "null".to_string(),
    }
}

/// The facts ordinary entity `n` is created with, in slot order: a unique
/// name, a score, a sparse shard attribute, 1–4 tags in the same transaction,
/// a long string on every 50th entity, a ref on every 10th.
fn content(seed: u64, n: u64) -> Vec<(String, Val)> {
    let h = mix(seed ^ n.wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut facts = vec![
        (":p/name".to_string(), Val::Str(format!("name-{n}"))),
        (":p/score".to_string(), Val::Int((h % 1000) as i64)),
        (format!(":p/shard{}", n % SHARDS), Val::Int(n as i64)),
    ];
    let start = (h >> 10) % 16;
    for i in 0..1 + (h >> 20) % 4 {
        // 5 is coprime with 16, so the tags are distinct.
        facts.push((
            ":p/tag".to_string(),
            Val::Kw(format!(":t{}", (start + i * 5) % 16)),
        ));
    }
    if n % 50 == 0 {
        facts.push((
            ":p/bio".to_string(),
            Val::Str(format!("bio-{n}-{}", "b".repeat(80))),
        ));
    }
    if n % 10 == 0 && n > 0 {
        facts.push((":p/ref".to_string(), Val::Ref((h >> 30) % n)));
    }
    facts
}

fn hot_fact(slot: usize) -> (&'static str, Val) {
    if slot < HOT_STATES {
        (":h/state", Val::Kw(format!(":s{slot}")))
    } else if slot < HOT_STATES + HOT_TAGS {
        (":h/tag", Val::Kw(format!(":g{}", slot - HOT_STATES)))
    } else {
        let i = slot - HOT_STATES - HOT_TAGS;
        (":h/note", Val::Str(format!("note-{i}-{}", "n".repeat(90))))
    }
}

// ── Reference ─────────────────────────────────────────────────────────────────

/// What the database must hold, without storing every fact: ordinary
/// entities follow from `content`, and only retracted slots are kept.
struct Reference {
    seed: u64,
    /// Ordinary entities `0..next_entity` exist.
    next_entity: u64,
    /// Facts the ordinary entities were created with.
    created: u64,
    flipped: Vec<(u64, u8)>,
    flipped_at: HashMap<(u64, u8), usize>,
    hot_live: [[bool; HOT_SLOTS]; HOT],
    /// `tx << 16 | hot << 8 | slot << 1 | asserted`, in commit order.
    hot_history: Vec<u64>,
    /// Transactions committed (`current_tx_count`).
    tx: u64,
    /// Fact versions written: every assert and every retract.
    versions: u64,
}

impl Reference {
    fn new(seed: u64) -> Self {
        Reference {
            seed,
            next_entity: 0,
            created: 0,
            flipped: Vec::new(),
            flipped_at: HashMap::new(),
            hot_live: [[false; HOT_SLOTS]; HOT],
            hot_history: Vec::new(),
            tx: 0,
            versions: 0,
        }
    }

    fn live_ordinary(&self) -> u64 {
        self.created - self.flipped.len() as u64
    }

    fn live_hot(&self) -> u64 {
        self.hot_live.iter().flatten().filter(|l| **l).count() as u64
    }

    fn is_flipped(&self, n: u64, slot: u8) -> bool {
        self.flipped_at.contains_key(&(n, slot))
    }

    fn flip(&mut self, n: u64, slot: u8) {
        self.flipped_at.insert((n, slot), self.flipped.len());
        self.flipped.push((n, slot));
    }

    fn unflip(&mut self, n: u64, slot: u8) {
        let i = self.flipped_at.remove(&(n, slot)).expect("slot is flipped");
        self.flipped.swap_remove(i);
        if let Some(moved) = self.flipped.get(i) {
            self.flipped_at.insert(*moved, i);
        }
    }

    fn record_hot(&mut self, hot: usize, slot: usize, asserted: bool) {
        self.hot_live[hot][slot] = asserted;
        self.hot_history
            .push(self.tx << 16 | (hot as u64) << 8 | (slot as u64) << 1 | asserted as u64);
    }

    /// `[attr value]` rows of ordinary entity `n`, sorted.
    fn entity_rows(&self, n: u64) -> Vec<String> {
        let mut rows: Vec<String> = content(self.seed, n)
            .into_iter()
            .enumerate()
            .filter(|(slot, _)| !self.is_flipped(n, *slot as u8))
            .map(|(_, (a, v))| format!("k{a} {}", v.render()))
            .collect();
        rows.sort();
        rows
    }

    /// `[attr value]` rows of hot entity `h`, now or as of transaction `t`.
    fn hot_rows(&self, h: usize, as_of: Option<u64>) -> Vec<String> {
        let live = match as_of {
            None => self.hot_live[h],
            Some(t) => {
                let mut live = [false; HOT_SLOTS];
                for r in &self.hot_history {
                    if r >> 16 > t {
                        break;
                    }
                    if (r >> 8 & 0xFF) as usize == h {
                        live[(r >> 1 & 0x7F) as usize] = r & 1 == 1;
                    }
                }
                live
            }
        };
        let mut rows: Vec<String> = (0..HOT_SLOTS)
            .filter(|s| live[*s])
            .map(|s| {
                let (a, v) = hot_fact(s);
                format!("k{a} {}", v.render())
            })
            .collect();
        rows.sort();
        rows
    }

    /// `[entity value]` rows of attribute `:p/shard{k}`, sorted.
    fn shard_rows(&self, k: u64) -> Vec<String> {
        let mut rows: Vec<String> = (k..self.next_entity)
            .step_by(SHARDS as usize)
            .filter(|n| !self.is_flipped(*n, 2))
            .map(|n| format!(":e{n} i{n}"))
            .collect();
        rows.sort();
        rows
    }
}

// ── Configuration ─────────────────────────────────────────────────────────────

struct Config {
    seed: u64,
    target_facts: u64,
    /// Schedules count steps instead of milliseconds (`soak_short`).
    step_clock: bool,
    /// Long run: wall-clock budget. Short run: churn steps after growth.
    budget: u64,
    check_every: u64,
    reopen_every: (u64, u64),
    checkpoint_every: (u64, u64),
    churn_tps: Option<f64>,
    sample_entities: u64,
    sample_names: u64,
    sample_shards: u64,
    sample_as_of: usize,
    /// A full scan builds every row in memory, so it runs once when the live
    /// count first reaches `full_scan_at`, and at the end only up to
    /// `full_scan_max` live facts.
    full_scan_at: u64,
    full_scan_max: u64,
    /// Fail on unbounded memory growth (spec §8).
    memory_bound: bool,
    metrics: Option<PathBuf>,
    dir: Option<PathBuf>,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name} must be an integer"))
        })
        .unwrap_or(default)
}

impl Config {
    fn short(seed: u64) -> Self {
        Config {
            seed,
            target_facts: 20_000,
            step_clock: true,
            budget: 400,
            check_every: 150,
            reopen_every: (60, 160),
            checkpoint_every: (3, 12),
            churn_tps: None,
            sample_entities: 20,
            sample_names: 3,
            sample_shards: 2,
            sample_as_of: 2,
            full_scan_at: 10_000,
            full_scan_max: u64::MAX,
            memory_bound: false,
            metrics: None,
            dir: None,
        }
    }

    fn from_env() -> Self {
        let seed = env_u64(
            "MINIGRAF_SOAK_SEED",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64),
        );
        let minute = 60_000;
        Config {
            seed,
            target_facts: env_u64("MINIGRAF_SOAK_TARGET_FACTS", 10_000_000),
            step_clock: false,
            budget: env_u64("MINIGRAF_SOAK_MINUTES", 20) * minute,
            check_every: env_u64("MINIGRAF_SOAK_CHECK_MINUTES", 10) * minute,
            reopen_every: (2 * minute, 10 * minute),
            checkpoint_every: (20, 200),
            churn_tps: Some(env_u64("MINIGRAF_SOAK_CHURN_TPS", 50) as f64),
            sample_entities: 200,
            sample_names: 20,
            sample_shards: 4,
            sample_as_of: 4,
            full_scan_at: 2_000_000,
            full_scan_max: env_u64("MINIGRAF_SOAK_FULL_SCAN_MAX", 3_000_000),
            memory_bound: true,
            metrics: Some(
                std::env::var_os("MINIGRAF_SOAK_METRICS")
                    .map_or_else(|| PathBuf::from("target/soak/metrics.jsonl"), PathBuf::from),
            ),
            dir: std::env::var_os("MINIGRAF_SOAK_DIR").map(PathBuf::from),
        }
    }
}

// ── Metrics ───────────────────────────────────────────────────────────────────

const PATHS: [&str; 5] = ["eavt", "avet", "aevt", "as_of", "hot"];

#[derive(Default)]
struct Metrics {
    /// Query latencies in µs since the last emitted line, per path.
    latency: HashMap<&'static str, Vec<u64>>,
    checkpoint_ms: Vec<u64>,
    checkpoints: u64,
    reopens: u64,
    open_ms_file: Option<u64>,
    open_ms_wal: Option<u64>,
    /// Peak RSS of the last full-scan child process.
    full_scan_rss: Option<u64>,
    out: Option<File>,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// `(VmRSS, VmHWM)` in bytes; `None` where `/proc` is not available.
fn rss() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| -> Option<u64> {
        let line = status.lines().find(|l| l.starts_with(name))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    };
    Some((field("VmRSS:")?, field("VmHWM:")?))
}

fn file_bytes(path: &Path) -> u64 {
    let mut wal = path.as_os_str().to_os_string();
    wal.push(".wal");
    [path, Path::new(&wal)]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

// ── Driver ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Growth,
    Churn,
}

struct Soak {
    cfg: Config,
    path: PathBuf,
    db: Option<Minigraf>,
    /// Drives every write, so a seed replays the same write sequence.
    rng: Rng,
    /// Drives sampling, reads and the schedule.
    read_rng: Rng,
    rf: Reference,
    metrics: Metrics,
    phase: Phase,
    start: Instant,
    steps: u64,
    churn_start: Option<(u64, Instant)>,
    churn_writes: u64,
    since_checkpoint: u64,
    checkpoint_due: u64,
    next_check: u64,
    next_reopen: u64,
    /// RSS right after the first reopen-after-checkpoint of the churn phase.
    rss_baseline: Option<u64>,
    full_scanned: bool,
}

fn open(path: &Path) -> Minigraf {
    OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()
        .unwrap_or_else(|e| panic!("open failed: {}", e.code()))
}

impl Soak {
    fn new(cfg: Config, path: PathBuf) -> Self {
        let out = cfg.metrics.as_ref().map(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).expect("metrics directory");
            }
            File::create(p).expect("metrics file")
        });
        let mut read_rng = Rng(cfg.seed ^ 0x5EED);
        let checkpoint_due = read_rng.range(cfg.checkpoint_every.0, cfg.checkpoint_every.1);
        let next_reopen = read_rng.range(cfg.reopen_every.0, cfg.reopen_every.1);
        Soak {
            db: Some(open(&path)),
            path,
            rng: Rng(cfg.seed),
            read_rng,
            rf: Reference::new(cfg.seed),
            metrics: Metrics {
                out,
                ..Metrics::default()
            },
            phase: Phase::Growth,
            start: Instant::now(),
            steps: 0,
            churn_start: None,
            churn_writes: 0,
            since_checkpoint: 0,
            checkpoint_due,
            next_check: cfg.check_every,
            next_reopen,
            rss_baseline: None,
            full_scanned: false,
            cfg,
        }
    }

    fn db(&self) -> &Minigraf {
        self.db.as_ref().expect("database is open")
    }

    fn clock(&self) -> u64 {
        if self.cfg.step_clock {
            self.steps
        } else {
            self.start.elapsed().as_millis() as u64
        }
    }

    fn context(&self) -> String {
        format!(
            "seed {} step {} tx {} entities {}",
            self.cfg.seed, self.steps, self.rf.tx, self.rf.next_entity
        )
    }

    fn done(&self) -> bool {
        if self.cfg.step_clock {
            return self
                .churn_start
                .is_some_and(|(s, _)| self.steps - s >= self.cfg.budget);
        }
        if self.clock() < self.cfg.budget {
            return false;
        }
        assert!(
            self.phase == Phase::Churn,
            "budget ended before {} live facts ({} reached); {}",
            self.cfg.target_facts,
            self.rf.live_ordinary(),
            self.context()
        );
        true
    }

    fn run(&mut self) {
        eprintln!(
            "soak: seed {} target {} facts, budget {} {}",
            self.cfg.seed,
            self.cfg.target_facts,
            self.cfg.budget,
            if self.cfg.step_clock {
                "churn steps"
            } else {
                "ms"
            }
        );
        while !self.done() {
            self.steps += 1;
            if !self.full_scanned && self.rf.live_ordinary() >= self.cfg.full_scan_at {
                self.full_scanned = true;
                let ms = self.full_scan();
                self.emit(Some(ms));
            }
            if self.phase == Phase::Growth && self.rf.live_ordinary() >= self.cfg.target_facts {
                self.enter_churn();
            }
            if self.write_allowed() {
                self.write_step();
            } else {
                self.read_step();
            }
            let now = self.clock();
            if now >= self.next_reopen {
                let after_checkpoint = self.read_rng.chance(0.5);
                self.reopen(after_checkpoint);
                self.next_reopen = self.clock()
                    + self
                        .read_rng
                        .range(self.cfg.reopen_every.0, self.cfg.reopen_every.1);
            }
            if now >= self.next_check {
                self.check_all();
                self.emit(None);
                self.next_check = self.clock() + self.cfg.check_every;
            }
        }
        self.finish();
    }

    fn enter_churn(&mut self) {
        eprintln!(
            "soak: growth done at {} live facts after {:?}; {}",
            self.rf.live_ordinary(),
            self.start.elapsed(),
            self.context()
        );
        self.phase = Phase::Churn;
        self.churn_start = Some((self.steps, Instant::now()));
        // The memory baseline: a fresh handle on a checkpointed file.
        self.reopen(true);
        self.check_all();
        self.emit(None);
    }

    fn write_allowed(&self) -> bool {
        match (self.phase, self.cfg.churn_tps, self.churn_start) {
            (Phase::Churn, Some(tps), Some((_, at))) => {
                (self.churn_writes as f64) < tps * at.elapsed().as_secs_f64()
            }
            _ => true,
        }
    }

    // ── Writes ────────────────────────────────────────────────────────────────

    fn execute_write(&mut self, cmd: &str, facts: u64) {
        if let Err(e) = self.db().execute(cmd) {
            panic!("write failed ({}); {}", e.code(), self.context());
        }
        self.rf.tx += 1;
        self.rf.versions += facts;
        if self.phase == Phase::Churn {
            self.churn_writes += 1;
        }
        self.since_checkpoint += 1;
        if self.since_checkpoint >= self.checkpoint_due {
            self.checkpoint();
        }
    }

    fn checkpoint(&mut self) {
        let t = Instant::now();
        if let Err(e) = self.db().checkpoint() {
            panic!("checkpoint failed ({}); {}", e.code(), self.context());
        }
        self.metrics
            .checkpoint_ms
            .push(t.elapsed().as_millis() as u64);
        self.metrics.checkpoints += 1;
        self.since_checkpoint = 0;
        self.checkpoint_due = self
            .read_rng
            .range(self.cfg.checkpoint_every.0, self.cfg.checkpoint_every.1);
    }

    fn write_step(&mut self) {
        let roll = self.rng.below(100);
        let (grow, flip) = match self.phase {
            Phase::Growth => (70, 85),
            Phase::Churn => (10, 60),
        };
        if roll < grow || self.rf.next_entity == 0 {
            self.grow();
        } else if roll < flip {
            self.flip_ordinary();
        } else {
            self.churn_hot();
        }
    }

    fn grow(&mut self) {
        let want = self.rng.log_uniform(1, 2_000);
        let mut cmd = String::from("(transact [");
        let mut facts = 0u64;
        let first = self.rf.next_entity;
        let mut n = first;
        while facts < want || n == first {
            for (a, v) in content(self.cfg.seed, n) {
                cmd.push_str(&format!("[:e{n} {a} {}]", v.edn()));
                facts += 1;
            }
            n += 1;
        }
        cmd.push_str("])");
        self.execute_write(&cmd, facts);
        self.rf.next_entity = n;
        self.rf.created += facts;
    }

    fn flip_ordinary(&mut self) {
        let k = self.rng.log_uniform(1, 50) as usize;
        let reassert = !self.rf.flipped.is_empty()
            && (self.rf.flipped.len() >= FLIP_CAP || self.rng.chance(0.5));
        let mut chosen: HashSet<(u64, u8)> = HashSet::new();
        if reassert {
            for _ in 0..k {
                let i = self.rng.below(self.rf.flipped.len() as u64) as usize;
                chosen.insert(self.rf.flipped[i]);
            }
        } else {
            for _ in 0..k * 2 {
                let n = self.rng.below(self.rf.next_entity);
                let slot = self.rng.below(content(self.cfg.seed, n).len() as u64) as u8;
                if !self.rf.is_flipped(n, slot) {
                    chosen.insert((n, slot));
                }
                if chosen.len() == k {
                    break;
                }
            }
        }
        if chosen.is_empty() {
            return;
        }
        // Sorted so the command, and so a replayed run, is deterministic.
        let mut chosen: Vec<(u64, u8)> = chosen.into_iter().collect();
        chosen.sort_unstable();
        let mut cmd = String::from(if reassert {
            "(transact ["
        } else {
            "(retract ["
        });
        for (n, slot) in &chosen {
            let (a, v) = &content(self.cfg.seed, *n)[*slot as usize];
            cmd.push_str(&format!("[:e{n} {a} {}]", v.edn()));
        }
        cmd.push_str("])");
        self.execute_write(&cmd, chosen.len() as u64);
        for (n, slot) in chosen {
            if reassert {
                self.rf.unflip(n, slot);
            } else {
                self.rf.flip(n, slot);
            }
        }
    }

    fn churn_hot(&mut self) {
        let live = self.rf.live_hot();
        let assert = live == 0 || (live < (HOT * HOT_SLOTS) as u64 && self.rng.chance(0.5));
        let k = self.rng.range(1, 8) as usize;
        let mut chosen: Vec<(usize, usize)> = Vec::new();
        for _ in 0..k * 4 {
            let h = self.rng.below(HOT as u64) as usize;
            let s = self.rng.below(HOT_SLOTS as u64) as usize;
            if self.rf.hot_live[h][s] != assert && !chosen.contains(&(h, s)) {
                chosen.push((h, s));
            }
            if chosen.len() == k {
                break;
            }
        }
        if chosen.is_empty() {
            return;
        }
        let mut cmd = String::from(if assert { "(transact [" } else { "(retract [" });
        for (h, s) in &chosen {
            let (a, v) = hot_fact(*s);
            cmd.push_str(&format!("[:h{h} {a} {}]", v.edn()));
        }
        cmd.push_str("])");
        self.execute_write(&cmd, chosen.len() as u64);
        for (h, s) in chosen {
            self.rf.record_hot(h, s, assert);
        }
    }

    // ── Reads and checks ──────────────────────────────────────────────────────

    /// Runs `query`, times it under `path`, and renders each row.
    fn rows(
        &mut self,
        path: &'static str,
        query: &str,
        names: &HashMap<Uuid, String>,
    ) -> Vec<String> {
        let t = Instant::now();
        let result = self.db().execute(query);
        let us = t.elapsed().as_micros() as u64;
        self.metrics.latency.entry(path).or_default().push(us);
        match result {
            Ok(QueryResult::QueryResults { results, .. }) => {
                let mut rows: Vec<String> = results
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|v| render_value(v, names))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                rows.sort();
                rows
            }
            Ok(_) => panic!("not a query result: {query}; {}", self.context()),
            Err(e) => panic!("query failed ({}): {query}; {}", e.code(), self.context()),
        }
    }

    fn expect_rows(&self, what: &str, got: Vec<String>, want: Vec<String>) {
        if got != want {
            let missing: Vec<&String> = want.iter().filter(|r| !got.contains(r)).take(10).collect();
            let extra: Vec<&String> = got.iter().filter(|r| !want.contains(r)).take(10).collect();
            panic!(
                "{what}: {} rows, expected {}; missing {missing:?}; unexpected {extra:?}; {}",
                got.len(),
                want.len(),
                self.context()
            );
        }
    }

    /// Names for the referents of entity `n`'s ref slot.
    fn ref_names(&self, n: u64) -> HashMap<Uuid, String> {
        content(self.cfg.seed, n)
            .into_iter()
            .filter_map(|(_, v)| match v {
                Val::Ref(m) => Some((entity_uuid(&format!(":e{m}")), format!(":e{m}"))),
                _ => None,
            })
            .collect()
    }

    fn check_entity(&mut self, n: u64) {
        let names = self.ref_names(n);
        let got = self.rows(
            "eavt",
            &format!("(query [:find ?a ?v :where [:e{n} ?a ?v]])"),
            &names,
        );
        self.expect_rows(&format!("EAVT :e{n}"), got, self.rf.entity_rows(n));
    }

    fn check_name(&mut self, n: u64) {
        let names = HashMap::from([(entity_uuid(&format!(":e{n}")), format!(":e{n}"))]);
        let got = self.rows(
            "avet",
            &format!("(query [:find ?e :where [?e :p/name \"name-{n}\"]])"),
            &names,
        );
        let want = if self.rf.is_flipped(n, 0) {
            vec![]
        } else {
            vec![format!("r:e{n}")]
        };
        self.expect_rows(&format!("AVET name-{n}"), got, want);
    }

    fn check_shard(&mut self, k: u64) {
        let names: HashMap<Uuid, String> = (k..self.rf.next_entity)
            .step_by(SHARDS as usize)
            .map(|n| (entity_uuid(&format!(":e{n}")), format!(":e{n}")))
            .collect();
        let got = self.rows(
            "aevt",
            &format!("(query [:find ?e ?v :where [?e :p/shard{k} ?v]])"),
            &names,
        );
        // `?e` renders as a ref; the reference writes the bare name.
        let got = got
            .into_iter()
            .map(|r| r.trim_start_matches('r').to_string())
            .collect();
        self.expect_rows(&format!("AEVT :p/shard{k}"), got, self.rf.shard_rows(k));
    }

    fn check_hot(&mut self, h: usize, as_of: Option<u64>) {
        let (path, clause) = match as_of {
            Some(t) => ("as_of", format!(":as-of {t} ")),
            None => ("hot", String::new()),
        };
        let got = self.rows(
            path,
            &format!("(query [:find ?a ?v {clause}:where [:h{h} ?a ?v]])"),
            &HashMap::new(),
        );
        let label = match as_of {
            Some(t) => format!("hot :h{h} as of {t}"),
            None => format!("hot :h{h}"),
        };
        self.expect_rows(&label, got, self.rf.hot_rows(h, as_of));
    }

    /// One checked read between capped churn writes.
    fn read_step(&mut self) {
        let n = self.read_rng.below(self.rf.next_entity);
        if self.read_rng.chance(0.9) {
            self.check_entity(n);
        } else {
            self.check_name(n);
        }
    }

    fn check_all(&mut self) {
        let tx = self.db().current_tx_count();
        assert_eq!(tx, self.rf.tx, "tx counter; {}", self.context());
        if self.rf.next_entity > 0 {
            for _ in 0..self.cfg.sample_entities {
                let n = self.read_rng.below(self.rf.next_entity);
                self.check_entity(n);
            }
            for _ in 0..self.cfg.sample_names {
                let n = self.read_rng.below(self.rf.next_entity);
                self.check_name(n);
            }
            for _ in 0..self.cfg.sample_shards {
                let k = self.read_rng.below(SHARDS);
                self.check_shard(k);
            }
        }
        for h in 0..HOT {
            self.check_hot(h, None);
        }
        for _ in 0..self.cfg.sample_as_of {
            let h = self.read_rng.below(HOT as u64) as usize;
            for _ in 0..3 {
                let t = self.read_rng.range(1, self.rf.tx.max(1));
                self.check_hot(h, Some(t));
            }
        }
    }

    fn reopen(&mut self, after_checkpoint: bool) {
        if after_checkpoint {
            self.checkpoint();
        }
        drop(self.db.take());
        let t = Instant::now();
        self.db = Some(open(&self.path));
        let ms = t.elapsed().as_millis() as u64;
        self.metrics.reopens += 1;
        if after_checkpoint {
            self.metrics.open_ms_file = Some(ms);
        } else {
            self.metrics.open_ms_wal = Some(ms);
        }
        self.check_all();
        if after_checkpoint && self.phase == Phase::Churn {
            self.check_memory();
        }
    }

    /// Spec §8: a fresh handle on a checkpointed file stays near the
    /// baseline taken when the churn phase began.
    fn check_memory(&mut self) {
        let Some((now, _)) = rss() else { return };
        let Some(base) = self.rss_baseline else {
            self.rss_baseline = Some(now);
            eprintln!("soak: memory baseline {} MiB", now >> 20);
            return;
        };
        let limit = base + base / 4 + (256 << 20);
        if self.cfg.memory_bound && now > limit {
            panic!(
                "RSS {} MiB above the bound {} MiB (baseline {} MiB); {}",
                now >> 20,
                limit >> 20,
                base >> 20,
                self.context()
            );
        }
    }

    fn emit(&mut self, full_scan_ms: Option<u64>) {
        let mut query_us = serde_json::Map::new();
        for path in PATHS {
            let mut v = self.metrics.latency.remove(path).unwrap_or_default();
            v.sort_unstable();
            query_us.insert(
                path.to_string(),
                serde_json::json!({"n": v.len(), "p50": percentile(&v, 0.5), "p99": percentile(&v, 0.99)}),
            );
        }
        let mut ck = std::mem::take(&mut self.metrics.checkpoint_ms);
        ck.sort_unstable();
        let (rss_now, rss_peak) = rss().map_or((None, None), |(a, b)| (Some(a), Some(b)));
        let line = serde_json::json!({
            "t_s": self.start.elapsed().as_secs(),
            "phase": if self.phase == Phase::Growth { "growth" } else { "churn" },
            "tx": self.rf.tx,
            "live_facts": self.rf.live_ordinary() + self.rf.live_hot(),
            "versions": self.rf.versions,
            "file_bytes": file_bytes(&self.path),
            "open_ms_file": self.metrics.open_ms_file,
            "open_ms_wal": self.metrics.open_ms_wal,
            "reopens": self.metrics.reopens,
            "checkpoints": self.metrics.checkpoints,
            "checkpoint_ms_p50": percentile(&ck, 0.5),
            "checkpoint_ms_max": ck.last().copied().unwrap_or(0),
            "rss_bytes": rss_now,
            "peak_rss_bytes": rss_peak,
            "query_us": query_us,
            "full_scan_ms": full_scan_ms,
            "full_scan_peak_rss_bytes": self.metrics.full_scan_rss,
        });
        eprintln!("soak: {line}");
        if let Some(out) = self.metrics.out.as_mut() {
            writeln!(out, "{line}").expect("write metrics");
        }
    }

    /// Checks the row count of `[?e ?a ?v]` in a child process, which
    /// prints the count, the time and its own peak RSS. A query builds its
    /// whole answer when it opens (#432), about 1 KB per row here, and the
    /// allocator keeps that memory after it is freed: run in this process, it
    /// would hide a leak of that size from the memory bound (spec §8).
    fn full_scan(&mut self) -> u64 {
        self.checkpoint();
        drop(self.db.take());
        let out = Command::new(std::env::current_exe().expect("test binary path"))
            .args(["soak_full_scan_child", "--exact", "--nocapture"])
            .env("MINIGRAF_SOAK_SCAN_DB", &self.path)
            .output()
            .expect("spawn full-scan child");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let Some(line) = stdout.lines().find_map(|l| l.strip_prefix("soak-scan: ")) else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
            panic!("full-scan child failed: {tail:?}; {}", self.context());
        };
        let field = |name: &str| -> Option<u64> {
            let mut words = line.split_whitespace();
            words.find(|w| *w == name)?;
            words.next()?.parse().ok()
        };
        let rows = field("rows").expect("child row count");
        let ms = field("ms").expect("child time");
        self.metrics.full_scan_rss = field("peak");
        self.db = Some(open(&self.path));
        assert_eq!(
            rows,
            self.rf.live_ordinary() + self.rf.live_hot(),
            "full scan row count; {}",
            self.context()
        );
        ms
    }

    fn finish(&mut self) {
        self.reopen(true);
        let full_scan_ms = if self.rf.live_ordinary() <= self.cfg.full_scan_max {
            Some(self.full_scan())
        } else {
            eprintln!(
                "soak: final full scan skipped above {} live facts",
                self.cfg.full_scan_max
            );
            None
        };
        let report = self.db().verify().expect("verify runs");
        assert!(
            report.is_ok(),
            "verify found {} problems; {}",
            report.problems.len(),
            self.context()
        );
        self.emit(full_scan_ms);
        eprintln!(
            "soak: done: {} live facts, {} versions, {} tx, {} checkpoints, {} reopens in {:?}",
            self.rf.live_ordinary() + self.rf.live_hot(),
            self.rf.versions,
            self.rf.tx,
            self.metrics.checkpoints,
            self.metrics.reopens,
            self.start.elapsed()
        );
    }
}

fn run(cfg: Config) {
    let tmp = match &cfg.dir {
        Some(dir) => {
            std::fs::create_dir_all(dir).expect("soak directory");
            tempfile::tempdir_in(dir).expect("temp dir")
        }
        None => tempfile::tempdir().expect("temp dir"),
    };
    let path = tmp.path().join("soak.graph");
    Soak::new(cfg, path).run();
}

/// The driver at a size a debug build runs in seconds.
#[test]
fn soak_short() {
    run(Config::short(392));
}

/// The full soak: see the module docs for its variables.
#[test]
#[ignore]
fn soak() {
    run(Config::from_env());
}

/// Child half of `Soak::full_scan`. A no-op when `MINIGRAF_SOAK_SCAN_DB` is
/// absent, as in `crash_kill_child_entrypoint`.
#[test]
fn soak_full_scan_child() {
    let Some(path) = std::env::var_os("MINIGRAF_SOAK_SCAN_DB") else {
        return;
    };
    let db = open(Path::new(&path));
    let t = Instant::now();
    let rows = match db.execute("(query [:find ?e ?a ?v :where [?e ?a ?v]])") {
        Ok(QueryResult::QueryResults { results, .. }) => results.len(),
        Ok(_) => panic!("full scan: not a query result"),
        Err(e) => panic!("full scan failed ({})", e.code()),
    };
    let ms = t.elapsed().as_millis();
    let peak = rss().map_or(0, |(_, peak)| peak);
    println!("soak-scan: rows {rows} ms {ms} peak {peak}");
}
