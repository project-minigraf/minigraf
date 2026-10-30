//! Runtime harness of the #797 reader A/B (measurement only, never shipped).
//!
//! `run.py` copies this file to `examples/reader_ab.rs` in every variant (B,
//! H, P) and builds it identically. One process runs the workloads it is
//! given and prints one JSON object per workload:
//!
//! ```text
//! reader_ab run FIXTURE WORKLOADS SECS [COLD_COPY]     (native)
//! reader_ab wasi WORKLOADS SECS ENTITIES               (wasm32-wasip1)
//! ```
//!
//! `FIXTURE` is a `perf_envelope __build` 100K fixture with its `.json`
//! metadata beside it, opened read-only for warm reads. Each workload is warmed
//! up (code and runtime state, never a cold observation's pages), then timed
//! one operation at a time until `SECS` and at least five operations. Before
//! timing, every distinct query it runs is executed once outside the timed
//! loop and its complete rows, in returned order, are hashed into `digest`;
//! every timed operation checks its row count.
//!
//! Workloads that need crate internals (`con_direct`, `wasi_committed_*`) exist
//! only in a `--cfg minigraf_ab` build with `ab-hook.patch` applied.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
// The WASI build uses only the in-memory workloads.
#![cfg_attr(target_arch = "wasm32", allow(unused_imports, dead_code))]

use minigraf::{BindValue, Minigraf, OpenOptions, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Counts allocations (and reallocations) of the whole process.
struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const WARMUP: Duration = Duration::from_millis(300);
const MIN_OPS: usize = 5;
/// Varied point keys per family.
const VARIED: usize = 256;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        #[cfg(not(target_arch = "wasm32"))]
        "run" => native::run(&args[1], &args[2], args[3].parse().unwrap(), args.get(4)),
        "wasi" => wasi::run(&args[1], args[2].parse().unwrap(), args[3].parse().unwrap()),
        other => panic!("unknown mode {other}"),
    }
}

// ─── Measurement ─────────────────────────────────────────────────────────────

/// A timed workload's result, printed as one JSON line.
struct Report {
    name: String,
    samples: Vec<u64>,
    allocs: u64,
    digest: u64,
    rows: u64,
    extra: String,
}

impl Report {
    fn print(mut self) {
        let ops = self.samples.len();
        let total: u128 = self.samples.iter().map(|&s| u128::from(s)).sum();
        self.samples.sort_unstable();
        let at = |p: f64| {
            let rank = ((p * ops as f64).ceil() as usize).clamp(1, ops);
            self.samples[rank - 1]
        };
        println!(
            "{{\"name\":\"{}\",\"ops\":{ops},\"total_ns\":{total},\"mean_ns\":{:.1},\
             \"p50_ns\":{},\"p99_ns\":{},\"allocs_per_op\":{:.2},\"digest\":\"{:016x}\",\
             \"rows\":{},\"rss_kb\":{}{}}}",
            self.name,
            total as f64 / ops as f64,
            at(0.50),
            at(0.99),
            self.allocs as f64 / ops as f64,
            self.digest,
            self.rows,
            rss_kb(),
            self.extra,
        );
    }
}

/// Warm `op` up, then time it one call at a time for `secs` (and `MIN_OPS`).
/// `op(i)` returns its row count, which must equal `expect(i)`.
fn timed(
    name: &str,
    secs: f64,
    digest: u64,
    mut op: impl FnMut(usize) -> usize,
    expect: impl Fn(usize) -> usize,
) -> Report {
    let start = Instant::now();
    let mut i = 0;
    while i < 3 || start.elapsed() < WARMUP {
        assert_eq!(op(i), expect(i), "warm-up row count");
        i += 1;
    }
    let budget = Duration::from_secs_f64(secs);
    let mut samples = Vec::with_capacity(1 << 16);
    let mut rows = 0u64;
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let start = Instant::now();
    let mut i = 0;
    while samples.len() < MIN_OPS || start.elapsed() < budget {
        let t = Instant::now();
        let r = op(i);
        samples.push(t.elapsed().as_nanos() as u64);
        assert_eq!(r, expect(i), "row count changed");
        rows += r as u64;
        i += 1;
    }
    let allocs = ALLOCS.load(Ordering::Relaxed) - allocs;
    Report {
        name: name.to_string(),
        samples,
        allocs,
        digest,
        rows,
        extra: String::new(),
    }
}

/// Hash of complete results in returned order.
fn hash_rows(h: &mut DefaultHasher, rows: &[Vec<Value>]) {
    rows.len().hash(h);
    for row in rows {
        for v in row {
            format!("{v:?}").hash(h);
        }
    }
}

fn result_rows(r: minigraf::QueryResult) -> Vec<Vec<Value>> {
    match r {
        minigraf::QueryResult::QueryResults { results, .. } => results,
        _ => panic!("not a query result"),
    }
}

/// Every row of a cursor, pulled in batches of `batch`.
fn drain(cursor: &mut minigraf::Cursor, batch: usize) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    while let Some(b) = cursor.next_batch(batch).unwrap() {
        rows.extend(b);
    }
    rows
}

fn count(cursor: &mut minigraf::Cursor, batch: usize) -> usize {
    let mut n = 0;
    while let Some(b) = cursor.next_batch(batch).unwrap() {
        n += b.len();
    }
    n
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn entity(name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
}

/// `n` keys in `0..range`, spread by a fixed LCG.
fn varied_keys(range: usize, n: usize) -> Vec<usize> {
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((x >> 33) as usize) % range
        })
        .collect()
}

// ─── Point families ──────────────────────────────────────────────────────────

/// One point family: a key-to-query mapping, read through the three entry
/// points with fixed, varied and missing keys.
struct Family {
    name: &'static str,
    /// Query text for key `k` (`None`: a missing entity or value).
    text: Box<dyn Fn(Option<usize>, usize) -> String>,
    /// Prepared form and its binding for a key.
    prepared: String,
    slot: &'static str,
    bind: Box<dyn Fn(Option<usize>, usize) -> BindValue>,
    fixed: usize,
    keys: Vec<usize>,
}

impl Family {
    /// The key of operation `i`: combination `i % 9` is (entry, key mode).
    fn key(&self, i: usize) -> (usize, Option<usize>, usize) {
        let combo = i % 9;
        let j = (i / 9) % self.keys.len();
        let key = match combo % 3 {
            0 => Some(self.fixed),
            1 => Some(self.keys[j]),
            _ => None,
        };
        (combo / 3, key, j)
    }
}

fn families(entities: usize, tx_half: u64) -> Vec<Family> {
    let m = entities / 2;
    vec![
        Family {
            name: "pt_ea",
            text: Box::new(|k, j| match k {
                Some(k) => format!("(query [:find ?a :where [:p{k} :age ?a]])"),
                None => format!("(query [:find ?a :where [:missing{j} :age ?a]])"),
            }),
            prepared: "(query [:find ?a :where [$e :age ?a]])".into(),
            slot: "e",
            bind: Box::new(|k, j| match k {
                Some(k) => BindValue::Entity(entity(&format!(":p{k}"))),
                None => BindValue::Entity(entity(&format!(":missing{j}"))),
            }),
            fixed: m,
            keys: varied_keys(entities, VARIED),
        },
        Family {
            name: "pt_av",
            text: Box::new(|k, j| match k {
                Some(k) => format!("(query [:find ?e :where [?e :name \"p{k}\"]])"),
                None => format!("(query [:find ?e :where [?e :name \"nope{j}\"]])"),
            }),
            prepared: "(query [:find ?e :where [?e :name $v]])".into(),
            slot: "v",
            bind: Box::new(|k, j| match k {
                Some(k) => BindValue::Val(Value::String(format!("p{k}"))),
                None => BindValue::Val(Value::String(format!("nope{j}"))),
            }),
            fixed: m,
            keys: varied_keys(entities, VARIED),
        },
        Family {
            name: "pt_asof",
            text: Box::new(move |k, j| match k {
                Some(k) => format!("(query [:find ?a :as-of {tx_half} :where [:p{k} :age ?a]])"),
                None => {
                    format!("(query [:find ?a :as-of {tx_half} :where [:missing{j} :age ?a]])")
                }
            }),
            prepared: format!("(query [:find ?a :as-of {tx_half} :where [$e :age ?a]])"),
            slot: "e",
            bind: Box::new(|k, j| match k {
                Some(k) => BindValue::Entity(entity(&format!(":p{k}"))),
                None => BindValue::Entity(entity(&format!(":missing{j}"))),
            }),
            // Entities below e/4 were written before `tx_half`.
            fixed: m / 4,
            keys: varied_keys(entities / 4, VARIED),
        },
    ]
}

/// Time one point family on `db`: the nine (entry point × key mode)
/// combinations in turn. Query texts are built before timing.
fn point_family(db: &Minigraf, f: &Family, secs: f64, name: &str) -> Report {
    let prepared = db.prepare(&f.prepared).unwrap();
    let n = f.keys.len();
    let texts: Vec<[String; 3]> = (0..n)
        .map(|j| {
            [
                (f.text)(Some(f.fixed), j),
                (f.text)(Some(f.keys[j]), j),
                (f.text)(None, j),
            ]
        })
        .collect();
    let binds: Vec<[BindValue; 3]> = (0..n)
        .map(|j| {
            [
                (f.bind)(Some(f.fixed), j),
                (f.bind)(Some(f.keys[j]), j),
                (f.bind)(None, j),
            ]
        })
        .collect();
    let run = |i: usize| -> Vec<Vec<Value>> {
        let (entry, key, j) = f.key(i);
        let mode = (i % 9) % 3;
        match entry {
            0 => result_rows(db.execute(&texts[j][mode]).unwrap()),
            1 => result_rows(
                prepared
                    .execute(&[(f.slot, binds[j][mode].clone())])
                    .unwrap(),
            ),
            _ => {
                let mut c = db.query(&texts[j][mode]).unwrap();
                let _ = key;
                drain(&mut c, 1)
            }
        }
    };
    // Validation: every distinct query, all three entry points equal.
    let mut h = DefaultHasher::new();
    let mut expect = vec![0usize; 9 * n];
    for i in 0..9 * n {
        let rows = run(i);
        expect[i] = rows.len();
        hash_rows(&mut h, &rows);
    }
    for i in 0..9 * n {
        let e = expect[i - (i % 9) + (i % 3)];
        assert_eq!(expect[i], e, "entry points disagree");
    }
    let digest = h.finish();
    let mut r = timed(
        name,
        secs,
        digest,
        |i| {
            let (entry, _, j) = f.key(i);
            let mode = (i % 9) % 3;
            match entry {
                0 => result_rows(db.execute(&texts[j][mode]).unwrap()).len(),
                1 => result_rows(
                    prepared
                        .execute(&[(f.slot, binds[j][mode].clone())])
                        .unwrap(),
                )
                .len(),
                _ => count(&mut db.query(&texts[j][mode]).unwrap(), 1),
            }
        },
        |i| expect[i % (9 * n)],
    );
    // Per-entry-point means, for the breakdown (not the decision metric).
    r.extra = String::new();
    r
}

// ─── Query shapes ────────────────────────────────────────────────────────────

/// The 17 warm-read shapes of the #772 audit (`compare.py` indices of the
/// pinned `perf_envelope`), rebuilt from the fixture metadata.
fn audit_shapes(entities: usize, tx_half: u64, tx_half_time: &str, hot_tx_half: u64) -> Vec<(&'static str, String)> {
    let e = entities;
    let m = e / 2;
    let leaf = e - 1;
    vec![
        ("point_entity", format!("[:find ?a :where [:p{m} :age ?a]]")),
        ("attribute_value", format!("[:find ?e :where [?e :name \"p{m}\"]]")),
        ("attribute_scan", "[:find ?e ?a :where [?e :age ?a]]".into()),
        ("join_2", format!("[:find ?n :where [:p{m} :follows ?f] [?f :name ?n]]")),
        ("join_3_scan", "[:find ?e ?n :where [?e :follows ?f] [?f :group 7] [?f :name ?n]]".into()),
        ("not_probe", "[:find ?e :where [?e :group 7] (not [?e :age 7])]".into()),
        (
            "not_join_hash",
            "[:find ?e :where [?e :group 7] [?e :age ?a] \
             (not-join [?a] [?f :group 17] [?f :age ?a] [(< ?a 50)])]"
                .into(),
        ),
        (
            "or_2",
            "[:find ?e :where [?e :group 7] [?e :age ?a] \
             (or-join [?a] (and [?f :group 17] [?f :age ?a] [(< ?a 30)]) \
             (and [?g :group 27] [?g :age ?a] [(> ?a 60)]))]"
                .into(),
        ),
        ("recursive_ancestors", format!("[:find ?b :where (boss :p{leaf} ?b)]")),
        ("as_of", format!("[:find ?a :as-of {tx_half} :where [:p{} :age ?a]]", m / 4)),
        (
            "as_of_ts",
            format!("[:find ?a :as-of \"{tx_half_time}\" :where [:p{} :age ?a]]", m / 4),
        ),
        (
            "as_of_ts_scan",
            format!("[:find ?e ?a :as-of \"{tx_half_time}\" :where [?e :age ?a]]"),
        ),
        ("count", "[:find (count ?v) :where [?e ?a ?v]]".into()),
        ("count_grouped", "[:find ?g (count ?e) :where [?e :group ?g]]".into()),
        ("window", "[:find ?e (rank :over (:order-by ?a)) :where [?e :age ?a]]".into()),
        ("hot_current", "[:find ?v :where [:hot :counter ?v]]".into()),
        (
            "hot_as_of",
            format!(
                "[:find ?v :as-of {hot_tx_half} :valid-at :any-valid-time :where [:hot :counter ?v]]"
            ),
        ),
    ]
}

/// Time one query drained in cursor batches of `batch`.
fn shape(db: &Minigraf, name: &str, query: &str, secs: f64, batch: usize) -> Report {
    let q = if query.starts_with('(') {
        query.to_string()
    } else {
        format!("(query {query})")
    };
    let mut h = DefaultHasher::new();
    let rows = drain(&mut db.query(&q).unwrap(), batch);
    hash_rows(&mut h, &rows);
    let n = rows.len();
    drop(rows);
    timed(
        name,
        secs,
        h.finish(),
        |_| count(&mut db.query(&q).unwrap(), batch),
        |_| n,
    )
}

fn define_rules(db: &Minigraf) {
    db.execute("(rule [(boss ?x ?y) [?x :manager ?y]])").unwrap();
    db.execute("(rule [(boss ?x ?z) [?x :manager ?y] (boss ?y ?z)])")
        .unwrap();
}

// ─── Native ──────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use minigraf::{FactFilter, FactOrder};
    use std::path::Path;

    struct Meta {
        entities: usize,
        tx_half: u64,
        hot_tx_half: u64,
        tx_half_time: String,
    }

    fn meta(fixture: &Path) -> Meta {
        let s = std::fs::read_to_string(fixture.with_extension("json")).unwrap();
        let num = |k: &str| -> u64 {
            let at = s.find(&format!("\"{k}\":")).unwrap() + k.len() + 3;
            s[at..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap()
        };
        let tx_half = num("tx_half");
        let db = OpenOptions::new().read_only(true).path(fixture).open().unwrap();
        let rec = db
            .fact_log(&FactFilter::new().tx_range(tx_half..=tx_half))
            .unwrap()
            .next_batch(1)
            .unwrap()
            .unwrap()
            .remove(0);
        let t = chrono::DateTime::from_timestamp_millis(rec.tx_id as i64).unwrap();
        Meta {
            entities: num("entities") as usize,
            tx_half,
            hot_tx_half: num("hot_tx_half"),
            tx_half_time: t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        }
    }

    fn open_ro(fixture: &Path, cache: Option<usize>) -> Minigraf {
        let mut o = OpenOptions::new()
            .read_only(true)
            .max_derived_facts(usize::MAX);
        if let Some(c) = cache {
            o = o.page_cache_size(c);
        }
        let db = o.path(fixture).open().unwrap();
        define_rules(&db);
        db
    }

    pub fn run(fixture: &str, workloads: &str, secs: f64, cold_copy: Option<&String>) {
        let fixture = Path::new(fixture);
        let meta = meta(fixture);
        let db = open_ro(fixture, None);
        let fams = families(meta.entities, meta.tx_half);
        let shapes = audit_shapes(meta.entities, meta.tx_half, &meta.tx_half_time, meta.hot_tx_half);
        for w in workloads.split(',') {
            let report = match w {
                "pt_ea" | "pt_av" | "pt_asof" => {
                    let f = fams.iter().find(|f| f.name == w).unwrap();
                    point_family(&db, f, secs, w)
                }
                _ if w.starts_with("sh_") => {
                    let (_, q) = shapes.iter().find(|(n, _)| *n == &w[3..]).unwrap();
                    shape(&db, w, q, secs, 1_000)
                }
                // Populated committed scans: short history, long history,
                // dense and sparse UUID projection.
                "sc_short" => shape(&db, w, "[:find ?e ?a :where [?e :age ?a]]", secs, 1_000),
                "sc_long_history" => shape(
                    &db,
                    w,
                    "[:find ?e ?v :valid-at :any-valid-time :where [?e :counter ?v]]",
                    secs,
                    1_000,
                ),
                "sc_dense_uuid" => shape(&db, w, "[:find ?e :where [?e :age ?a]]", secs, 1_000),
                "sc_sparse_uuid" => shape(&db, w, "[:find ?e :where [?e :group 7]]", secs, 1_000),
                // Long cursor walk in small batches.
                "cw_long" => shape(&db, w, "[:find ?e ?a :where [?e :age ?a]]", secs, 100),
                "fl_paged" => fact_log(&db, w, secs),
                // Application page cache disabled (OS cache warm).
                "nc_point" => {
                    let db = open_ro(fixture, Some(0));
                    point_family(&db, &fams[0], secs, w)
                }
                "nc_scan" => {
                    let db = open_ro(fixture, Some(0));
                    shape(&db, w, "[:find ?e ?a :where [?e :age ?a]]", secs, 1_000)
                }
                "con_open" => con_open(fixture, w, secs),
                "con_checkpoint" => con_checkpoint(cold_copy.expect("scratch copy"), w, secs),
                "con_large_tx" => con_large_tx(cold_copy.expect("scratch copy"), w, secs, &meta),
                #[cfg(minigraf_ab)]
                "con_direct" => {
                    let mut r = timed(
                        w,
                        secs,
                        0,
                        |_| {
                            db.ab_construct_readers(1).unwrap();
                            0
                        },
                        |_| 0,
                    );
                    r.extra = String::new();
                    r
                }
                "cold_point" | "cold_scan" => cold(fixture, cold_copy.expect("cold copy"), w, secs, &meta),
                _ if w.starts_with("cc_") => concurrent(fixture, &fams[0], w, secs),
                other => panic!("unknown workload {other}"),
            };
            report.print();
        }
    }

    /// Every committed fact version through the fact log, paged.
    fn fact_log(db: &Minigraf, name: &str, secs: f64) -> Report {
        let filter = FactFilter::new()
            .attributes([":age"])
            .order(FactOrder::Storage);
        let walk = |h: Option<&mut DefaultHasher>| -> usize {
            let mut log = db.fact_log(&filter).unwrap();
            let mut n = 0;
            let mut h = h;
            while let Some(page) = log.next_batch(500).unwrap() {
                n += page.len();
                if let Some(h) = h.as_deref_mut() {
                    for r in &page {
                        (r.entity.as_u128(), &r.attribute, format!("{:?}", r.value), r.tx_count, r.tx_id, r.valid_from, r.valid_to, r.asserted).hash(h);
                    }
                }
            }
            n
        };
        let mut h = DefaultHasher::new();
        let n = walk(Some(&mut h));
        timed(name, secs, h.finish(), |_| walk(None), |_| n)
    }

    fn con_open(fixture: &Path, name: &str, secs: f64) -> Report {
        timed(
            name,
            secs,
            0,
            |_| {
                let db = OpenOptions::new().read_only(true).path(fixture).open().unwrap();
                drop(db);
                0
            },
            |_| 0,
        )
    }

    /// A dirty checkpoint that publishes a new generation: one small
    /// transaction (not timed), then the timed checkpoint.
    fn con_checkpoint(copy: &str, name: &str, secs: f64) -> Report {
        let db = OpenOptions::new()
            .wal_checkpoint_threshold(usize::MAX)
            .path(copy)
            .open()
            .unwrap();
        let mut n = 0u64;
        let mut samples = Vec::new();
        let write = |n: &mut u64| {
            db.execute(&format!("(transact [[:ckpt :n {n}]])")).unwrap();
            *n += 1;
        };
        for _ in 0..3 {
            write(&mut n);
            db.checkpoint().unwrap();
        }
        let allocs = ALLOCS.load(Ordering::Relaxed);
        let mut alloc_skip = 0;
        let start = Instant::now();
        while samples.len() < MIN_OPS || start.elapsed() < Duration::from_secs_f64(secs) {
            let a = ALLOCS.load(Ordering::Relaxed);
            write(&mut n);
            alloc_skip += ALLOCS.load(Ordering::Relaxed) - a;
            let t = Instant::now();
            db.checkpoint().unwrap();
            samples.push(t.elapsed().as_nanos() as u64);
        }
        Report {
            name: name.into(),
            samples,
            allocs: ALLOCS.load(Ordering::Relaxed) - allocs - alloc_skip,
            digest: 0,
            rows: 0,
            extra: String::new(),
        }
    }

    /// Reads inside a large file-backed `WriteTransaction`: each read builds
    /// a fresh reader over the working trees.
    fn con_large_tx(copy: &str, name: &str, secs: f64, meta: &Meta) -> Report {
        let db = OpenOptions::new()
            .wal_checkpoint_threshold(usize::MAX)
            .transaction_chunk_facts(1)
            .path(copy)
            .open()
            .unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:big :a 1] [:big :b 2] [:big :c 3]])")
            .unwrap();
        let m = meta.entities / 2;
        let q = format!("(query [:find ?a :where [:p{m} :age ?a]])");
        let mut h = DefaultHasher::new();
        let rows = result_rows(tx.execute(&q).unwrap());
        hash_rows(&mut h, &rows);
        let big = result_rows(tx.execute("(query [:find ?v :where [:big :b ?v]])").unwrap());
        assert_eq!(big.len(), 1, "read-your-writes");
        let n = rows.len();
        let r = timed(
            name,
            secs,
            h.finish(),
            |_| result_rows(tx.execute(&q).unwrap()).len(),
            |_| n,
        );
        tx.rollback();
        r
    }

    unsafe extern "C" {
        fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        fn munmap(addr: *mut u8, len: usize) -> i32;
        fn mincore(addr: *mut u8, len: usize, vec: *mut u8) -> i32;
    }
    const POSIX_FADV_DONTNEED: i32 = 4;
    const PROT_READ: i32 = 1;
    const MAP_SHARED: i32 = 1;

    /// Drop `path`'s clean pages from the OS page cache, then count the
    /// pages still resident.
    fn evict(path: &Path) -> usize {
        use std::os::fd::AsRawFd;
        let f = std::fs::File::open(path).unwrap();
        let len = f.metadata().unwrap().len() as usize;
        let rc = unsafe { posix_fadvise(f.as_raw_fd(), 0, 0, POSIX_FADV_DONTNEED) };
        assert_eq!(rc, 0, "posix_fadvise");
        let pages = len.div_ceil(4096);
        let mut vec = vec![0u8; pages];
        unsafe {
            let p = mmap(std::ptr::null_mut(), len, PROT_READ, MAP_SHARED, f.as_raw_fd(), 0);
            assert!(!p.is_null() && p as isize != -1, "mmap");
            assert_eq!(mincore(p, len, vec.as_mut_ptr()), 0, "mincore");
            munmap(p, len);
        }
        vec.iter().filter(|&&b| b & 1 == 1).count()
    }

    /// Cold observations: each opens its own handle (empty application
    /// cache), evicts the copy from the OS page cache, checks residency, then
    /// times one query. Warm-up runs on the fixture, not the copy.
    fn cold(fixture: &Path, copy: &str, name: &str, secs: f64, meta: &Meta) -> Report {
        let copy = Path::new(copy);
        let q = if name == "cold_point" {
            format!("(query [:find ?a :where [:p{} :age ?a]])", meta.entities / 2)
        } else {
            "(query [:find ?e ?a :where [?e :age ?a]])".to_string()
        };
        let warm = open_ro(fixture, None);
        let mut h = DefaultHasher::new();
        let rows = drain(&mut warm.query(&q).unwrap(), 1_000);
        hash_rows(&mut h, &rows);
        let n = rows.len();
        for _ in 0..3 {
            assert_eq!(count(&mut warm.query(&q).unwrap(), 1_000), n);
        }
        drop(warm);
        let mut samples = Vec::new();
        let mut resident = 0usize;
        let mut file_pages = 0usize;
        let mut allocs = 0u64;
        let start = Instant::now();
        while samples.len() < MIN_OPS || start.elapsed() < Duration::from_secs_f64(secs) {
            let db = OpenOptions::new().read_only(true).path(copy).open().unwrap();
            resident += evict(copy);
            file_pages = (std::fs::metadata(copy).unwrap().len() as usize).div_ceil(4096);
            let a = ALLOCS.load(Ordering::Relaxed);
            let t = Instant::now();
            let r = count(&mut db.query(&q).unwrap(), 1_000);
            samples.push(t.elapsed().as_nanos() as u64);
            allocs += ALLOCS.load(Ordering::Relaxed) - a;
            assert_eq!(r, n, "row count changed");
        }
        let ops = samples.len();
        Report {
            name: name.into(),
            samples,
            allocs,
            digest: h.finish(),
            rows: (n * ops) as u64,
            extra: format!(",\"resident_pages_after_evict\":{resident},\"file_pages\":{file_pages}"),
        }
    }

    /// `threads` readers sharing one handle, each running the entity/attribute
    /// family's text queries. `mean_ns` is wall time per operation (inverse
    /// throughput); percentiles are per query.
    fn concurrent(fixture: &Path, f: &Family, name: &str, secs: f64) -> Report {
        let threads: usize = name[3..].parse().unwrap();
        let db = std::sync::Arc::new(open_ro(fixture, None));
        let texts: Vec<String> = (0..f.keys.len())
            .flat_map(|j| [(f.text)(Some(f.fixed), j), (f.text)(Some(f.keys[j]), j), (f.text)(None, j)])
            .collect();
        let texts = std::sync::Arc::new(texts);
        let expect = |i: usize| usize::from(i % 3 != 2);
        for i in 0..texts.len() {
            assert_eq!(result_rows(db.execute(&texts[i]).unwrap()).len(), expect(i));
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(threads + 1));
        let budget = Duration::from_secs_f64(secs);
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (db, texts, barrier) = (db.clone(), texts.clone(), barrier.clone());
                std::thread::spawn(move || {
                    for i in 0..texts.len() {
                        db.execute(&texts[(i + t) % texts.len()]).unwrap();
                    }
                    barrier.wait();
                    let mut samples = Vec::new();
                    let start = Instant::now();
                    let mut i = t * 7;
                    while samples.len() < MIN_OPS || start.elapsed() < budget {
                        let k = i % texts.len();
                        let s = Instant::now();
                        let r = result_rows(db.execute(&texts[k]).unwrap()).len();
                        samples.push(s.elapsed().as_nanos() as u64);
                        assert_eq!(r, usize::from(k % 3 != 2));
                        i += 1;
                    }
                    samples
                })
            })
            .collect();
        barrier.wait();
        let allocs = ALLOCS.load(Ordering::Relaxed);
        let start = Instant::now();
        let mut samples = Vec::new();
        for h in handles {
            samples.extend(h.join().unwrap());
        }
        let wall = start.elapsed().as_nanos() as u64;
        let ops = samples.len();
        let allocs = ALLOCS.load(Ordering::Relaxed) - allocs;
        let mut r = Report {
            name: name.into(),
            samples,
            allocs,
            digest: 0,
            rows: 0,
            extra: format!(",\"threads\":{threads},\"wall_ns\":{wall},\"wall_ns_per_op\":{:.1}", wall as f64 / ops as f64),
        };
        // The decision metric is wall time per operation: report it as the mean.
        let per_op = wall as f64 / ops as f64;
        r.extra.push_str(&format!(",\"query_mean_ns\":{:.1}", r.samples.iter().map(|&s| s as f64).sum::<f64>() / ops as f64));
        r.extra.push_str(&format!(",\"inverse_throughput_ns\":{per_op:.1}"));
        r
    }
}

// ─── WASI ────────────────────────────────────────────────────────────────────

mod wasi {
    use super::*;

    /// The fixture's facts for `entities` entities, as `perf_envelope` builds
    /// them (without valid-time history).
    fn transactions(entities: usize) -> Vec<String> {
        let mut out = Vec::new();
        for lo in (0..entities).step_by(200) {
            let hi = (lo + 200).min(entities);
            let mut cmd = String::from("(transact [");
            for i in lo..hi {
                let follows = (i * 7919 + 1) % entities;
                cmd.push_str(&format!(
                    "[:p{i} :name \"p{i}\"][:p{i} :age {}][:p{i} :group {}][:p{i} :follows :p{follows}]",
                    i % 90,
                    i % 100
                ));
                if i > 0 {
                    cmd.push_str(&format!("[:p{i} :manager :p{}]", i / 10));
                }
            }
            cmd.push_str("])");
            out.push(cmd);
        }
        out
    }

    /// `wasi_pending_*`: the CLI's own path, `Minigraf::in_memory`, where
    /// every fact stays pending. `wasi_committed_*`: the same facts
    /// checkpointed into a memory backend and read through a populated
    /// committed reader (`--cfg minigraf_ab`).
    pub fn run(workloads: &str, secs: f64, entities: usize) {
        let txs = transactions(entities);
        let pending = Minigraf::in_memory().unwrap();
        for t in &txs {
            pending.execute(t).unwrap();
        }
        define_rules(&pending);
        #[cfg(minigraf_ab)]
        let committed = {
            let c = minigraf::db::AbCommittedMemory::new(&txs).unwrap();
            define_rules(&c.db);
            c
        };
        let fams = families(entities, 0);
        for w in workloads.split(',') {
            let (kind, rest) = w.strip_prefix("wasi_").unwrap().split_once('_').unwrap();
            let db: &Minigraf = match kind {
                "pending" => &pending,
                #[cfg(minigraf_ab)]
                "committed" => &committed.db,
                other => panic!("unknown kind {other}"),
            };
            let report = match rest {
                "pt_ea" | "pt_av" => {
                    let f = fams.iter().find(|f| f.name == rest).unwrap();
                    point_family(db, f, secs, w)
                }
                "scan" => shape(db, w, "[:find ?e ?a :where [?e :age ?a]]", secs, 1_000),
                "join_2" => shape(
                    db,
                    w,
                    &format!("[:find ?n :where [:p{} :follows ?f] [?f :name ?n]]", entities / 2),
                    secs,
                    1_000,
                ),
                #[cfg(minigraf_ab)]
                "con_direct" if kind == "committed" => timed(
                    w,
                    secs,
                    0,
                    |_| {
                        committed.construct_readers(1);
                        0
                    },
                    |_| 0,
                ),
                other => panic!("unknown workload {other}"),
            };
            report.print();
        }
    }
}
