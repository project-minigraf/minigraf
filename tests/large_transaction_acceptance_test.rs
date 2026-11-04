//! Acceptance test for large transactions (#550 spec §8.1, #473 spec §8): a
//! 1M-fact `transact` on a file database holds at most `TX_CHUNK_FACTS`
//! materialised facts at any time, and commits as a checkpoint.
//!
//! Ignored by default; `.github/workflows/smoke.yml` runs it nightly:
//!
//! ```text
//! cargo test --profile bench --features test-hooks --test large_transaction_acceptance_test -- --ignored --nocapture --test-threads=1
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

/// Exercise the public option, including both clamps, and cross the configured
/// boundary. Large custom chunks use million-fact statements; tiny chunks use
/// smaller fixtures to keep the nightly test cost bounded.
#[test]
#[ignore]
fn custom_chunk_bounds() {
    for (configured, facts) in [
        (0usize, 17usize),
        (1, 17),
        (2, 257),
        (257, 1_000_000),
        (131_072, 1_000_000),
        (1_000_000, 1_000_001),
        (usize::MAX, 1_000_001),
    ] {
        let chunk = configured.clamp(1, 1_000_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.graph");
        let db = minigraf::OpenOptions::new()
            .transaction_chunk_facts(configured)
            .path(&path)
            .open()
            .unwrap();
        let mut input = String::from("(transact [");
        for i in 0..facts {
            input.push_str(&format!("[:e{i} :n {i}]"));
        }
        input.push_str("])");
        let _ = minigraf::__statement_facts_high_water();
        let start = std::time::Instant::now();
        db.execute(&input).unwrap();
        let high = minigraf::__statement_facts_high_water();
        assert_eq!(
            high,
            chunk.min(facts),
            "exact peak honors the public chunk option and clamps"
        );
        assert_eq!(db.current_tx_count(), 1);
        assert!(db.verify().unwrap().is_ok());
        eprintln!(
            "custom_chunk_bounds: configured={configured} effective={chunk} facts={facts} held={high} elapsed={:?}",
            start.elapsed()
        );
        drop(db);
        let db = Minigraf::open(&path).unwrap();
        let QueryResult::QueryResults { results, .. } = db
            .execute("(query [:find (count ?v) :where [?e :n ?v]])")
            .unwrap()
        else {
            panic!("expected count result");
        };
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0][0], minigraf::Value::Integer(n) if n == facts as i64),
            "all facts survive reopen"
        );
        assert_eq!(db.current_tx_count(), 1);
    }
}
