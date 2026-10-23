//! Acceptance test for large transactions (#550 spec §8.1, #473 spec §8): a
//! 1M-fact `transact` on a file database holds at most `TX_CHUNK_FACTS`
//! materialised facts at any time, and commits as a checkpoint.
//!
//! Ignored by default; `.github/workflows/smoke.yml` runs it nightly:
//!
//! ```text
//! cargo test --profile bench --features test-hooks --test large_transaction_acceptance_test -- --ignored --nocapture
//! ```
//!
//! The count of facts held comes from a high-water mark the statement reader
//! keeps (`minigraf::__statement_facts_high_water`, `test-hooks` only). Peak
//! RSS comes from `/proc/self/status` (Linux only) and is printed, not
//! checked: the bound is on facts, and the input string alone is about 25 MB.
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

use minigraf::{Minigraf, QueryResult};

const FACTS: usize = 1_000_000;
/// The default `OpenOptions::transaction_chunk_facts`.
const TX_CHUNK_FACTS: usize = 65_536;

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

fn count(db: &Minigraf, query: &str) -> usize {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("not a query result"),
    }
}

#[test]
#[ignore]
fn one_million_fact_transact_holds_one_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.graph");
    let db = Minigraf::open(&path).unwrap();
    // Half the facts on new entities, half on a few: the DICT grows and the
    // index leaves split, and repeated entities cross chunks.
    let mut input = String::with_capacity(FACTS * 26);
    input.push_str("(transact [");
    for i in 0..FACTS {
        if i % 2 == 0 {
            input.push_str(&format!("[:e{i} :n {i}]"));
        } else {
            input.push_str(&format!("[:hub{} :m {i}]", i % 64));
        }
    }
    input.push_str("])");

    let _ = minigraf::__statement_facts_high_water();
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let before_kb = proc_status_kb("VmRSS");
    let start = std::time::Instant::now();
    db.execute(&input).unwrap();
    let elapsed = start.elapsed();
    let high = minigraf::__statement_facts_high_water();
    let peak_kb = proc_status_kb("VmHWM");
    eprintln!(
        "one_million_fact_transact_holds_one_chunk: {FACTS} facts in {elapsed:.2?}; \
         at most {high} facts held; peak RSS {} MiB, {} MiB over the start{}",
        peak_kb / 1024,
        peak_kb.saturating_sub(before_kb) / 1024,
        if reset {
            ""
        } else {
            " (high-water mark not reset)"
        }
    );
    assert!(high > 0, "the statement was counted");
    assert!(
        high <= TX_CHUNK_FACTS,
        "at most one chunk of facts held at once"
    );

    let mut wal = path.as_os_str().to_owned();
    wal.push(".wal");
    assert!(
        !std::path::Path::new(&wal).exists(),
        "committed as a checkpoint"
    );
    assert_eq!(db.current_tx_count(), 1);
    assert_eq!(count(&db, "(query [:find ?v :where [:e10 :n ?v]])"), 1);
    assert_eq!(
        count(&db, "(query [:find ?v :where [:hub1 :m ?v]])"),
        FACTS / 64
    );
    assert!(db.verify().unwrap().is_ok());
    drop(db);
    let db = Minigraf::open(&path).unwrap();
    assert_eq!(count(&db, "(query [:find ?e :where [?e :n 999998]])"), 1);
}
