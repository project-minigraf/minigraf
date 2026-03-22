//! Edge case integration tests for Phase 6.4a.
//!
//! Covers:
//! - Oversized fact rejected at insertion (file-backed, both execute() and commit() paths)
//! - Oversized fact accepted in-memory (no page size constraint)
//! - Stale WAL after checkpoint is replayed idempotently (no duplicate facts)

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
        let db = OpenOptions {
            wal_checkpoint_threshold: usize::MAX,
            ..Default::default()
        }
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
