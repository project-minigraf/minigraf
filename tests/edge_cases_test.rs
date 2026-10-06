//! Edge case integration tests for Phase 6.4a.
//!
//! Covers:
//! - Oversized fact rejected at insertion (file-backed, both execute() and commit() paths)
//! - Oversized fact accepted in-memory (no page size constraint)
//! - Stale WAL after checkpoint is replayed idempotently (no duplicate facts)
//! - More than 65,535 uncheckpointed facts query correctly (#445)
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, OpenOptions, QueryResult};

// ── Oversized fact — file-backed, execute() path ──────────────────────────────

#[test]
fn test_oversized_fact_rejected_at_insertion_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new()
        .path(path.to_str().unwrap())
        .open()
        .unwrap();

    let large_value = "x".repeat(8192); // well above MAX_FACT_BYTES = 4080
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    let result = db.execute(&cmd);

    assert!(
        result.is_err(),
        "oversized fact must be rejected at insertion for file-backed DB"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("4080"),
        "error message must cite the 4080-byte limit; got: {}",
        msg
    );
}

// ── Oversized fact — explicit WriteTransaction path ────────────────────────────

#[test]
fn test_oversized_fact_rejected_via_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new()
        .path(path.to_str().unwrap())
        .open()
        .unwrap();

    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);

    let mut tx = db.begin_write().unwrap();
    tx.execute(&cmd).unwrap(); // buffered in-memory — not yet validated
    let result = tx.commit(); // size check fires here

    assert!(
        result.is_err(),
        "oversized fact must be rejected at commit for file-backed DB"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("4080"),
        "error message must cite the 4080-byte limit; got: {}",
        msg
    );
}

// ── Oversized fact — in-memory (no constraint) ────────────────────────────────

#[test]
fn test_oversized_fact_accepted_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    assert!(
        db.execute(&cmd).is_ok(),
        "oversized fact must be accepted in an in-memory database (no page size constraint)"
    );
}

// ── Checkpoint-during-crash: stale WAL replay is idempotent ───────────────────

#[test]
fn test_stale_wal_after_checkpoint_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let path = path.to_str().unwrap();
    let wal_path = format!("{}.wal", path);

    // Phase 1: insert alice with checkpoint suppressed; save WAL bytes
    {
        let db = OpenOptions::default()
            .wal_checkpoint_threshold(usize::MAX)
            .path(path)
            .open()
            .unwrap();
        db.execute("(transact [[:alice :age 30]])").unwrap();
        // Drop without checkpointing — WAL exists
    }
    let stale_wal = std::fs::read(&wal_path).expect("WAL must exist after insert");

    // Phase 2: reopen (replays WAL → alice loaded), insert bob, checkpoint
    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute("(transact [[:bob :age 40]])").unwrap();
        db.checkpoint().unwrap(); // alice + bob → packed pages; WAL deleted
    }
    assert!(
        !std::path::Path::new(&wal_path).exists(),
        "WAL must be deleted after checkpoint"
    );

    // Phase 3: simulate crash — restore stale WAL (alice only, tx_count=1)
    std::fs::write(&wal_path, &stale_wal).unwrap();

    // Phase 4: reopen — WAL replay must skip alice (tx_count=1 ≤ last_checkpointed_tx_count)
    let db = OpenOptions::new().path(path).open().unwrap();

    let alice_result = db
        .execute("(query [:find ?age :where [:alice :age ?age]])")
        .unwrap();
    let alice_rows = match alice_result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults variant"),
    };
    assert_eq!(
        alice_rows.len(),
        1,
        "alice:age must appear exactly once — stale WAL replay must be idempotent"
    );

    let bob_result = db
        .execute("(query [:find ?age :where [:bob :age ?age]])")
        .unwrap();
    let bob_rows = match bob_result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults variant"),
    };
    assert_eq!(bob_rows.len(), 1, "bob:age must survive the checkpoint");
}

// ── More than 65,535 uncheckpointed facts (#445) ─────────────────────────────

const PAST_U16: u32 = 70_000;

fn transact_past_u16(db: &Minigraf) {
    let mut batch = String::from("(transact [");
    for i in 0..PAST_U16 {
        batch.push_str(&format!("[:e{i} :n {i}] "));
    }
    batch.push_str("])");
    db.execute(&batch).unwrap();
}

fn ints(r: QueryResult) -> Vec<i64> {
    let QueryResult::QueryResults { results, .. } = r else {
        panic!("expected query results");
    };
    results
        .iter()
        .flatten()
        .map(|v| match v {
            minigraf::Value::Integer(n) => *n,
            _ => panic!("expected an integer"),
        })
        .collect()
}

fn assert_all_facts_visible(db: &Minigraf, when: &str) {
    for i in [0u32, 65_535, 65_536, PAST_U16 - 1] {
        let got = ints(
            db.execute(&format!("(query [:find ?v :where [:e{i} :n ?v]])"))
                .unwrap(),
        );
        assert_eq!(got, vec![i64::from(i)], "entity lookup {when}");
    }
    let got = ints(
        db.execute("(query [:find (count ?e) (max ?v) :where [?e :n ?v]])")
            .unwrap(),
    );
    assert_eq!(
        got,
        vec![i64::from(PAST_U16), i64::from(PAST_U16) - 1],
        "attribute scan {when}"
    );
}

#[test]
fn test_more_than_u16_pending_facts_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    transact_past_u16(&db);
    assert_all_facts_visible(&db, "in memory");
}

#[test]
fn test_more_than_u16_pending_facts_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = Minigraf::open(&path).unwrap();
    transact_past_u16(&db);
    assert_all_facts_visible(&db, "before checkpoint");
    db.checkpoint().unwrap();
    assert_all_facts_visible(&db, "after checkpoint");
}
