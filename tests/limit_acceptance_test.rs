//! Acceptance test for `:limit` and top-k (#306 spec §5, moved from #473):
//! over 10M facts, `:limit 10` stops the query once it has its rows, so its
//! time and memory do not grow with the facts; `:order-by [?v :desc] :limit
//! 10` reads every fact but holds ten rows, so its memory does not grow with
//! them. One test, so the fixture is built once.
//!
//! Ignored by default; `.github/workflows/smoke.yml` runs it nightly:
//!
//! ```text
//! cargo test --profile bench --test limit_acceptance_test -- --ignored --nocapture
//! ```
//!
//! Peak RSS comes from `/proc/self/status` (Linux only): the high-water mark
//! is reset just before the query.
//
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{FactRecord, LogWriter, OpenOptions, QueryResult, Value};

const FACTS: usize = 10_000_000;
/// Facts per transaction.
const TX_FACTS: usize = 10_000;
/// What the query may hold: a `Limit` holds nothing and a scan one batch of
/// ids. Holding even one id per fact would need megabytes.
const MEMORY_LIMIT: usize = 64 << 10;
/// Peak RSS the query may add: the reader's pages and one batch.
const MAX_RSS: usize = 32 << 20;
/// Far below a scan of 10M facts (seconds), well above one batch (well
/// under a millisecond on a desktop), for slow CI runners.
const MAX_TIME: std::time::Duration = std::time::Duration::from_millis(250);
/// What top-k may hold: ten rows and one batch's DICT lookups, which `Sort`
/// drops after each batch. One lookup per fact would need hundreds of MiB.
const TOP_K_MEMORY_LIMIT: usize = 1 << 20;

/// A `kB` field of `/proc/self/status`; 0 where it does not exist.
fn proc_status_kb(field: &str) -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(field)?.strip_prefix(':').map(str::to_owned))
        })
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// Run `query` on `path` under `limit`: its rows, time and the peak RSS it
/// added, when measurable.
fn measure(
    path: &std::path::Path,
    limit: usize,
    query: &str,
) -> (Vec<Vec<Value>>, std::time::Duration, Option<usize>) {
    let db = OpenOptions::new()
        .query_memory_limit(limit)
        .path(path)
        .open()
        .unwrap();
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let before_kb = proc_status_kb("VmRSS");
    let start = std::time::Instant::now();
    let result = db.execute(query).unwrap();
    let elapsed = start.elapsed();
    let peak_kb = proc_status_kb("VmHWM");
    let added = peak_kb.saturating_sub(before_kb) * 1024;
    eprintln!(
        "{query}: {elapsed:.2?}; peak RSS {} MiB, {} KiB added by the query",
        peak_kb / 1024,
        added / 1024
    );
    let QueryResult::QueryResults { results, .. } = result else {
        panic!("expected query results");
    };
    (results, elapsed, (reset && peak_kb > 0).then_some(added))
}

#[test]
#[ignore]
fn limit_and_top_k_over_10m_facts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("limit.graph");
    // `LogWriter` writes the records straight into the trees in batched
    // checkpoints: the `transact` path took 42 minutes for 10M facts.
    let start = std::time::Instant::now();
    let mut w = LogWriter::create(&path, OpenOptions::new()).unwrap();
    for i in 0..FACTS {
        let tx = (i / TX_FACTS) as u64 + 1;
        w.append(&FactRecord {
            entity: uuid::Uuid::from_u128(i as u128 + 1),
            attribute: ":a".to_string(),
            value: Value::Integer(i as i64),
            tx_count: tx,
            tx_id: 1_700_000_000_000 + tx,
            valid_from: (1_700_000_000_000 + tx) as i64,
            valid_to: i64::MAX,
            asserted: true,
        })
        .unwrap();
    }
    w.finish().unwrap();
    eprintln!("10M facts: built in {:.2?}", start.elapsed());
    let (rows, elapsed, added) = measure(
        &path,
        MEMORY_LIMIT,
        "(query [:find ?e ?v :where [?e :a ?v] :limit 10])",
    );
    assert_eq!(rows.len(), 10, "ten rows");
    assert!(elapsed < MAX_TIME, "stops once it has its rows");
    if let Some(added) = added {
        assert!(added < MAX_RSS, "holds no rows");
    }

    let (rows, _, added) = measure(
        &path,
        TOP_K_MEMORY_LIMIT,
        "(query [:find ?e ?v :where [?e :a ?v] :order-by [?v :desc] :limit 10])",
    );
    let top: Vec<i64> = rows
        .iter()
        .map(|r| match r.get(1) {
            Some(Value::Integer(n)) => *n,
            _ => panic!("an integer"),
        })
        .collect();
    let n = FACTS as i64;
    assert_eq!(
        top,
        ((n - 10)..n).rev().collect::<Vec<i64>>(),
        "the ten largest"
    );
    if let Some(added) = added {
        assert!(added < MAX_RSS, "holds ten rows");
    }
}
