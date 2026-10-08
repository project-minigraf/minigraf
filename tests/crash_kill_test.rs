//! A `SIGKILL` at any point while a child process writes must leave the
//! database holding exactly the transactions the child had committed (#384).
//!
//! The child runs a deterministic workload from a seed: multi-valued batches
//! (#371), batched retracts and re-assertions, with checkpoints at a rate set
//! by the round's variant. After each `execute` returns, it appends the
//! transaction's number to a confirmation log. The parent kills it at a random
//! point, replays the same workload into a reference model, and checks the
//! reopened file through the entity-bound (EAVT), attribute (AEVT), value
//! (AVET) and full-scan paths, `:as-of`, and `verify()`. The checks run again
//! after a second reopen and after a checkpoint plus reopen, to catch damage a
//! checkpoint copies forward.
//!
//! A transaction can reach the WAL or file before the kill and still miss the
//! log, so the reopened state is the model after `N` or `N + 1` transactions,
//! where `N` is the last confirmed one. Nothing else is allowed.
//!
//! The log is not fsynced: a process kill keeps the OS page cache, so a
//! written line survives exactly as the WAL entry does.
//!
//! This also covers the original #308 report (a kill mid-checkpoint left a
//! header checksum mismatch): every round must reopen.
//!
//! Round count: `MINIGRAF_CRASH_KILL_ROUNDS` (default 5, one per variant;
//! `.github/workflows/crash-kill.yml` runs 100 nightly on each OS). Replay a
//! failure with the base seed it prints: `MINIGRAF_CRASH_KILL_SEED`.
//!
//! Design: docs/superpowers/specs/2026-10-08-crash-kill-data-check-design.md
#![cfg(not(target_arch = "wasm32"))]

use minigraf::db::{Minigraf, OpenOptions, SyncMode};
use minigraf::{QueryResult, Value};
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_ROUNDS: usize = 5;

fn rounds() -> usize {
    std::env::var("MINIGRAF_CRASH_KILL_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ROUNDS)
}

// ── Workload ──────────────────────────────────────────────────────────────────

const ENTITIES: u8 = 4;
const ATTRS: [&str; 3] = [":a0", ":a1", ":tag/丿"];
/// Values 0..8 are integers; 8 and 9 are strings over the 64-byte inline limit
/// that share a 40-byte prefix.
const VALUES: u8 = 10;

fn long_string(i: u8) -> String {
    format!(
        "{}{}",
        "p".repeat(40),
        if i == 8 { "A" } else { "B" }.repeat(40)
    )
}

fn value_edn(v: u8) -> String {
    if v < 8 {
        v.to_string()
    } else {
        format!("\"{}\"", long_string(v))
    }
}

fn value_render(v: u8) -> String {
    if v < 8 {
        format!("i{v}")
    } else {
        format!("s{v}")
    }
}

/// An `(entity, attribute, value)` triple, as pool indices.
type Triple = (u8, u8, u8);

fn triple_edn((e, a, v): Triple) -> String {
    format!("[:e{e} {} {}]", ATTRS[a as usize], value_edn(v))
}

/// xorshift64: deterministic, dependency-free.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn below_u8(&mut self, n: u8) -> u8 {
        self.below(u64::from(n)) as u8
    }
}

/// The next transaction of the workload, and the live triples after it.
fn next_tx(rng: &mut Rng, live: &BTreeSet<Triple>) -> (String, BTreeSet<Triple>) {
    let mut next = live.clone();
    if !live.is_empty() && rng.below(10) < 3 {
        // Batched retract of 1-4 live triples.
        let pool: Vec<Triple> = live.iter().copied().collect();
        let mut picked = BTreeSet::new();
        for _ in 0..1 + rng.below(4) {
            picked.insert(pool[rng.below(pool.len() as u64) as usize]);
        }
        let body: Vec<String> = picked.iter().map(|t| triple_edn(*t)).collect();
        for t in &picked {
            next.remove(t);
        }
        return (format!("(retract [{}])", body.join(" ")), next);
    }
    // Multi-valued batch: 1-3 values of one entity and attribute (#371), plus
    // 0-3 other triples. Triples retracted earlier come back this way. A
    // triple is never repeated within one transaction.
    let e = rng.below_u8(ENTITIES);
    let a = rng.below_u8(ATTRS.len() as u8);
    let mut picked = BTreeSet::new();
    for _ in 0..1 + rng.below(3) {
        picked.insert((e, a, rng.below_u8(VALUES)));
    }
    for _ in 0..rng.below(4) {
        picked.insert((
            rng.below_u8(ENTITIES),
            rng.below_u8(ATTRS.len() as u8),
            rng.below_u8(VALUES),
        ));
    }
    let body: Vec<String> = picked.iter().map(|t| triple_edn(*t)).collect();
    next.extend(picked);
    (format!("(transact [{}])", body.join(" ")), next)
}

/// Live triples after each transaction: `history[k]` is the state after `k`.
fn model(seed: u64, txs: u64) -> Vec<BTreeSet<Triple>> {
    let mut rng = Rng::new(seed);
    let mut history = vec![BTreeSet::new()];
    for _ in 0..txs {
        let (_, next) = next_tx(&mut rng, history.last().unwrap());
        history.push(next);
    }
    history
}

// ── Variants ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Variant {
    /// `checkpoint()` after every transaction: the kill mostly lands in a save.
    CheckpointEach,
    /// `checkpoint()` after every third transaction.
    CheckpointEvery3,
    /// Never checkpoints: reopen replays the WAL.
    WalOnly,
    /// `wal_checkpoint_threshold(2)`: checkpoints inside `execute`.
    AutoCheckpoint,
    /// As `CheckpointEvery3`, with `SyncMode::Normal` (no WAL fsync).
    NormalSync,
}

const VARIANTS: [Variant; 5] = [
    Variant::CheckpointEach,
    Variant::CheckpointEvery3,
    Variant::WalOnly,
    Variant::AutoCheckpoint,
    Variant::NormalSync,
];

impl Variant {
    fn options(self) -> OpenOptions {
        let opts = OpenOptions::new();
        match self {
            Variant::WalOnly => opts.wal_checkpoint_threshold(usize::MAX),
            Variant::AutoCheckpoint => opts.wal_checkpoint_threshold(2),
            Variant::NormalSync => opts.synchronous(SyncMode::Normal),
            Variant::CheckpointEach | Variant::CheckpointEvery3 => opts,
        }
    }

    fn checkpoint_after(self, k: u64) -> bool {
        match self {
            Variant::CheckpointEach => true,
            Variant::CheckpointEvery3 | Variant::NormalSync => k % 3 == 0,
            Variant::WalOnly | Variant::AutoCheckpoint => false,
        }
    }
}

// ── Checks ────────────────────────────────────────────────────────────────────

fn entity_uuid(e: u8) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!(":e{e}").as_bytes())
}

struct Checker {
    uuids: HashMap<Uuid, u8>,
    /// Names the round in every failure message.
    ctx: String,
}

impl Checker {
    fn render(&self, v: &Value) -> String {
        match v {
            Value::Integer(n) => format!("i{n}"),
            Value::String(s) => match (8..VALUES).find(|i| long_string(*i) == *s) {
                Some(i) => format!("s{i}"),
                None => format!("s?{}", s.len()),
            },
            Value::Keyword(k) => k.clone(),
            Value::Ref(u) => match self.uuids.get(u) {
                Some(e) => format!("r{e}"),
                None => "r?".to_string(),
            },
            _ => "?".to_string(),
        }
    }

    fn rows(&self, db: &Minigraf, query: &str) -> Vec<String> {
        let result = db
            .execute(query)
            .unwrap_or_else(|e| panic!("{}: query failed ({}): {query}", self.ctx, e.code()));
        let QueryResult::QueryResults { results, .. } = result else {
            panic!("{}: not a query result: {query}", self.ctx);
        };
        let mut rows: Vec<String> = results
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| self.render(v))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        rows.sort();
        rows
    }

    fn expect(&self, db: &Minigraf, query: &str, want: impl Iterator<Item = String>) {
        let mut want: Vec<String> = want.collect();
        want.sort();
        assert_eq!(self.rows(db, query), want, "{}: {query}", self.ctx);
    }

    /// The reopened state must be the model after `confirmed` or
    /// `confirmed + 1` transactions, through every index path.
    fn check(&self, db: &Minigraf, history: &[BTreeSet<Triple>], confirmed: u64) {
        let t = db.current_tx_count();
        assert!(
            t == confirmed || t == confirmed + 1,
            "{}: tx count {t} after {confirmed} confirmed transactions",
            self.ctx
        );
        let live = &history[t as usize];
        let full = |s: &BTreeSet<Triple>| -> Vec<String> {
            s.iter()
                .map(|&(e, a, v)| format!("r{e} {} {}", ATTRS[a as usize], value_render(v)))
                .collect()
        };

        // Full scan.
        self.expect(
            db,
            "(query [:find ?e ?a ?v :where [?e ?a ?v]])",
            full(live).into_iter(),
        );
        // AEVT: attribute scans; AVET: attribute and value bound.
        for (ai, attr) in ATTRS.iter().enumerate() {
            let ai = ai as u8;
            self.expect(
                db,
                &format!("(query [:find ?e ?v :where [?e {attr} ?v]])"),
                live.iter()
                    .filter(|t| t.1 == ai)
                    .map(|&(e, _, v)| format!("r{e} {}", value_render(v))),
            );
            for v in 0..VALUES {
                self.expect(
                    db,
                    &format!("(query [:find ?e :where [?e {attr} {}]])", value_edn(v)),
                    live.iter()
                        .filter(|t| t.1 == ai && t.2 == v)
                        .map(|&(e, _, _)| format!("r{e}")),
                );
            }
        }
        // EAVT: entity-bound lookups (the path #370 broke).
        for e in 0..ENTITIES {
            self.expect(
                db,
                &format!("(query [:find ?a ?v :where [:e{e} ?a ?v]])"),
                live.iter()
                    .filter(|t| t.0 == e)
                    .map(|&(_, a, v)| format!("{} {}", ATTRS[a as usize], value_render(v))),
            );
        }
        // Earlier states, through :as-of.
        let samples: BTreeSet<u64> = [1, t / 2, t.saturating_sub(1)]
            .into_iter()
            .filter(|k| (1..t).contains(k))
            .collect();
        for k in samples {
            self.expect(
                db,
                &format!("(query [:find ?e ?a ?v :as-of {k} :where [?e ?a ?v]])"),
                full(&history[k as usize]).into_iter(),
            );
        }
        let report = db
            .verify()
            .unwrap_or_else(|e| panic!("{}: verify failed ({})", self.ctx, e.code()));
        assert!(report.is_ok(), "{}: verify found problems", self.ctx);
    }
}

fn open(path: &Path, ctx: &str) -> Minigraf {
    Minigraf::open(path).unwrap_or_else(|e| panic!("{ctx}: reopen failed ({})", e.code()))
}

/// Complete lines in the confirmation log; the last is the newest confirmed
/// transaction. A partial last line is ignored.
fn confirmed(log: &Path) -> u64 {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let complete = &text[..text.rfind('\n').map_or(0, |i| i + 1)];
    let mut n = 0;
    for (i, line) in complete.lines().enumerate() {
        let k: u64 = line.parse().expect("log line is a number");
        assert_eq!(k, i as u64 + 1, "log lines are 1, 2, 3, ...");
        n = k;
    }
    n
}

// ── Test ──────────────────────────────────────────────────────────────────────

#[test]
fn sigkill_keeps_exactly_the_committed_transactions() {
    let exe = std::env::current_exe().expect("test binary path");
    let base_seed: u64 = std::env::var("MINIGRAF_CRASH_KILL_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        });
    println!("crash-kill base seed {base_seed} (replay: MINIGRAF_CRASH_KILL_SEED={base_seed})");
    let uuids: HashMap<Uuid, u8> = (0..ENTITIES).map(|e| (entity_uuid(e), e)).collect();

    for round in 0..rounds() {
        let seed = base_seed ^ (round as u64 + 1).wrapping_mul(0x2545_F491_4F6C_DD1D);
        let variant_idx = round % VARIANTS.len();
        let variant = VARIANTS[variant_idx];
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("kill.graph");
        let log_path = dir.path().join("confirmed.log");

        let mut child = Command::new(&exe)
            .args(["crash_kill_child_entrypoint", "--exact", "--nocapture"])
            .env("MINIGRAF_KILL_DB", &db_path)
            .env("MINIGRAF_KILL_LOG", &log_path)
            .env("MINIGRAF_KILL_SEED", seed.to_string())
            .env("MINIGRAF_KILL_VARIANT", variant_idx.to_string())
            .spawn()
            .expect("spawn crash-kill child");

        // Kill after a random number of confirmed transactions, plus jitter so
        // the kill lands anywhere inside the next execute or checkpoint.
        let mut rng = Rng::new(seed.rotate_left(17));
        let target = 1 + rng.below(60);
        let deadline = Instant::now() + Duration::from_secs(30);
        while confirmed(&log_path) < target {
            if let Some(status) = child.try_wait().expect("poll child") {
                panic!("round {round} seed {seed}: child exited early ({status})");
            }
            assert!(
                Instant::now() < deadline,
                "round {round} seed {seed}: child too slow"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_micros(rng.below(20_000)));
        child.kill().expect("SIGKILL crash-kill child");
        let status = child.wait().expect("reap crash-kill child");
        assert!(
            !status.success(),
            "round {round}: child must not exit cleanly before being killed"
        );

        let n = confirmed(&log_path);
        let history = model(seed, n + 1);
        let ctx = |stage: &str| {
            format!("round {round} seed {seed} variant {variant:?} confirmed {n}: {stage}")
        };
        let checker = |stage: &str| Checker {
            uuids: uuids.clone(),
            ctx: ctx(stage),
        };

        let db = open(&db_path, &ctx("first reopen"));
        checker("first reopen").check(&db, &history, n);
        drop(db);

        let db = open(&db_path, &ctx("second reopen"));
        checker("second reopen").check(&db, &history, n);
        db.checkpoint()
            .unwrap_or_else(|e| panic!("{}: checkpoint failed ({})", ctx("checkpoint"), e.code()));
        drop(db);

        let db = open(&db_path, &ctx("reopen after checkpoint"));
        checker("reopen after checkpoint").check(&db, &history, n);
    }
}

/// Child half of the test above: runs the seeded workload until killed. A no-op
/// when `MINIGRAF_KILL_DB` is absent, as in `tests/common::crash_child_entrypoint`.
#[test]
fn crash_kill_child_entrypoint() {
    let Ok(db_path) = std::env::var("MINIGRAF_KILL_DB") else {
        return;
    };
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("child needs {k}"));
    let seed: u64 = env("MINIGRAF_KILL_SEED").parse().expect("seed");
    let variant = VARIANTS[env("MINIGRAF_KILL_VARIANT")
        .parse::<usize>()
        .expect("variant")];
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(env("MINIGRAF_KILL_LOG"))
        .expect("child opens log");

    let db = Minigraf::open_with_options(&db_path, variant.options()).expect("child opens db");
    let mut rng = Rng::new(seed);
    let mut live = BTreeSet::new();
    for k in 1.. {
        let (stmt, next) = next_tx(&mut rng, &live);
        db.execute(&stmt).expect("child transaction");
        log.write_all(format!("{k}\n").as_bytes())
            .expect("child logs transaction");
        live = next;
        if variant.checkpoint_after(k) {
            db.checkpoint().expect("child checkpoint");
        }
    }
}
