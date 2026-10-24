//! Acceptance tests for query memory accounting (#549 spec §6.2, and #575):
//! a query whose state is large either completes or fails with `QRY-012`
//! under `OpenOptions::query_memory_limit`, a cartesian product streams
//! instead of holding its rows, and an ungrouped `count` holds no rows
//! (#473 acceptance, #551).
//!
//! Ignored by default; `.github/workflows/smoke.yml` runs them nightly, one
//! at a time because the peak RSS is per process:
//!
//! ```text
//! cargo test --profile bench --test memory_limit_acceptance_test -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Peak RSS comes from `/proc/self/status` (Linux only): the high-water mark
//! is reset just before the query.
//
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, OpenOptions};

const GROUPS: usize = 1_000_000;
/// Below what 1M groups cost the meter: each group's key and hash entry are
/// at least 40 bytes (spec §3.2), 40 MB in all.
const GROUPS_LIMIT: usize = 32 << 20;

/// Patterns in the cartesian query, each of the 3 facts: 3^13 rows.
const K: u32 = 13;
/// Peak RSS the cartesian query may add. The executor held every row (more
/// than 5.7 GB at k = 13, #575); the engine holds a 3-row build side per join.
const CARTESIAN_MAX_RSS: usize = 64 << 20;

/// What an ungrouped `count` over every fact may hold: one set of
/// accumulators. A query holding even one row id per fact would need
/// megabytes at 1M facts.
const COUNT_LIMIT: usize = 4 << 10;
/// Peak RSS the `count` may add: the reader's pages and one batch.
const COUNT_MAX_RSS: usize = 32 << 20;

/// `n` entities, each with its number as `:g`, checkpointed.
fn groups_db(dir: &tempfile::TempDir, opts: OpenOptions) -> Minigraf {
    let db = opts.path(dir.path().join("groups.graph")).open().unwrap();
    for chunk in 0..GROUPS / 100_000 {
        let facts: String = (chunk * 100_000..(chunk + 1) * 100_000)
            .map(|i| format!("[:e{i} :g {i}]"))
            .collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
        db.checkpoint().unwrap();
    }
    db
}

/// The `datalog_eval` fuzz harness's 3 facts (#575), checkpointed.
fn cartesian_db(dir: &tempfile::TempDir, opts: OpenOptions) -> Minigraf {
    let db = opts
        .path(dir.path().join("cartesian.graph"))
        .open()
        .unwrap();
    db.execute(r#"(transact [[:alice :name "Alice"] [:bob :name "Bob"] [:alice :age 30]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    db
}

/// `K` patterns that share no variable, returning one pattern's value.
fn cartesian_query() -> String {
    let patterns: String = (0..K).map(|i| format!("[?e{i} ?a{i} ?v{i}]")).collect();
    format!("(query [:find ?v0 :where {patterns}])")
}

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

/// Every row of `query`, counted and dropped; or the first error.
fn drain(db: &Minigraf, query: &str) -> Result<usize, minigraf::MinigrafError> {
    let mut cursor = db.query(query)?;
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(1_000)? {
        rows += batch.len();
    }
    Ok(rows)
}

#[test]
#[ignore]
fn grouped_count_1m_groups() {
    let query = "(query [:find ?g (count ?e) :where [?e :g ?g]])";
    let dir = tempfile::tempdir().unwrap();
    let db = groups_db(&dir, OpenOptions::new());
    let start = std::time::Instant::now();
    let rows = drain(&db, query).unwrap();
    eprintln!(
        "grouped_count_1m_groups: {rows} rows in {:.2?}",
        start.elapsed()
    );
    assert_eq!(rows, GROUPS, "one row per group");
    drop(db);

    let db = OpenOptions::new()
        .query_memory_limit(GROUPS_LIMIT)
        .path(dir.path().join("groups.graph"))
        .open()
        .unwrap();
    let err = drain(&db, query).err().expect("over the limit");
    assert_eq!(err.code(), "QRY-012", "stopped by the limit");
    assert!(err.to_string().contains("Aggregate"), "names Aggregate");
    // The database answers other queries.
    assert_eq!(
        drain(&db, "(query [:find ?e :where [?e :g 7]])").unwrap(),
        1,
        "usable after QRY-012"
    );
}

#[test]
#[ignore]
fn cartesian_k13_streams() {
    let query = cartesian_query();
    let dir = tempfile::tempdir().unwrap();
    let db = cartesian_db(&dir, OpenOptions::new());
    // Reset the high-water mark to the current RSS, so the peak read after
    // the query is the query's own (Linux 4.0+; elsewhere nothing is checked).
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let before_kb = proc_status_kb("VmRSS");
    let start = std::time::Instant::now();
    let rows = drain(&db, &query).unwrap();
    let peak_kb = proc_status_kb("VmHWM");
    let added = peak_kb.saturating_sub(before_kb) * 1024;
    eprintln!(
        "cartesian_k13_streams: {rows} rows in {:.2?}; peak RSS {} MiB, {} KiB added by the query",
        start.elapsed(),
        peak_kb / 1024,
        added / 1024
    );
    assert_eq!(rows, 3usize.pow(K), "one row per binding");
    if reset && peak_kb > 0 {
        assert!(added < CARTESIAN_MAX_RSS, "the product streams");
    }
    drop(db);

    let db = OpenOptions::new()
        .query_memory_limit(64)
        .path(dir.path().join("cartesian.graph"))
        .open()
        .unwrap();
    let err = drain(&db, &query).err().expect("over the limit");
    assert_eq!(err.code(), "QRY-012", "stopped by the limit");
    assert!(err.to_string().contains("HashJoin"), "names HashJoin");
}

/// `count` over every fact runs in memory that does not grow with the
/// facts: under a 4 KiB `query_memory_limit` it counts 1M facts, so the
/// meter (#549) bounds what it holds at any size. The envelope's `count`
/// shape records the RSS at 10M facts (`docs/BENCHMARKS.md`).
#[test]
#[ignore]
fn count_holds_no_rows() {
    let query = "(query [:find (count ?v) :where [?e ?a ?v]])";
    let dir = tempfile::tempdir().unwrap();
    drop(groups_db(&dir, OpenOptions::new()));
    let db = OpenOptions::new()
        .query_memory_limit(COUNT_LIMIT)
        .path(dir.path().join("groups.graph"))
        .open()
        .unwrap();
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let before_kb = proc_status_kb("VmRSS");
    let start = std::time::Instant::now();
    let result = db.execute(query).unwrap();
    let peak_kb = proc_status_kb("VmHWM");
    let added = peak_kb.saturating_sub(before_kb) * 1024;
    eprintln!(
        "count_holds_no_rows: {:.2?}; peak RSS {} MiB, {} KiB added by the query",
        start.elapsed(),
        peak_kb / 1024,
        added / 1024
    );
    let minigraf::QueryResult::QueryResults { results, .. } = result else {
        panic!("expected query results");
    };
    assert_eq!(results.len(), 1, "one row");
    assert!(
        matches!(results[0][0], minigraf::Value::Integer(n) if n == GROUPS as i64),
        "counts every fact"
    );
    if reset && peak_kb > 0 {
        assert!(added < COUNT_MAX_RSS, "the count holds no rows");
    }
}
