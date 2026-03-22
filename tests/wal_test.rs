//! Integration tests for WAL-backed crash safety, recovery, and checkpoint.
//!
//! These tests exercise the file-backed `Minigraf` API end-to-end, verifying:
//! - Basic persistence (write → drop → reopen)
//! - WAL crash recovery (simulated crash via `mem::forget`)
//! - Duplicate-free recovery after post-checkpoint crash
//! - Partial WAL entry discarding
//! - Manual checkpoint behaviour
//! - Auto-checkpoint threshold
//! - Explicit transaction commit and rollback
//! - Concurrent reads while writer holds the write lock
//! - V2 → V3 file format upgrade on first checkpoint

use minigraf::db::{Minigraf, OpenOptions};
use minigraf::query::QueryResult;
use minigraf::storage::{FileHeader, PAGE_SIZE};

// ── helpers ──────────────────────────────────────────────────────────────────

fn count_results(result: QueryResult) -> usize {
    match result {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

fn wal_path_for(db_path: &std::path::Path) -> std::path::PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(".wal");
    std::path::PathBuf::from(p)
}

// ── 1. Basic file-backed persistence ─────────────────────────────────────────

/// Write a fact, drop (triggering checkpoint), reopen and verify the fact is present.
#[test]
fn test_file_backed_basic_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("basic.graph");

    // Session 1: write and close (drop triggers checkpoint)
    {
        let db = Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
    }

    // Session 2: reopen and verify
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(n, 1, "Alice must survive close/reopen");
}

// ── 2. WAL recovery after simulated crash ─────────────────────────────────────

/// Write a fact with a very high checkpoint threshold so the checkpoint never fires,
/// then `mem::forget` the DB to simulate a crash (skipping the Drop checkpoint).
/// Verify the WAL exists, then reopen and confirm the fact was recovered.
#[test]
fn test_wal_recovery_after_simulated_crash() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.graph");
    let wal_path = wal_path_for(&db_path);

    // "Crash" session: write fact, skip Drop
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: usize::MAX,
            },
        )
        .unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();

        // Simulate a crash: drop Inner without running Drop logic.
        // mem::forget on the Arc-backed Minigraf drops the Arc but leaves
        // the Inner alive as long as the clone lives — however, since this
        // is the only handle, forgetting it leaks the Arc permanently and
        // the Drop impl never runs.
        std::mem::forget(db);
    }

    // WAL must still exist (no checkpoint happened)
    assert!(wal_path.exists(), "WAL must exist after simulated crash");

    // Recovery session: opening should replay the WAL
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 1,
        "Alice must be recovered from WAL after simulated crash"
    );
}

// ── 3. No duplicate facts after post-checkpoint crash ────────────────────────

/// Write a fact, crash (no checkpoint), then backup the WAL, reopen (which
/// replays the WAL and checkpoints), restore the WAL backup, and reopen again.
/// The second reopen must not produce duplicate facts even though the WAL
/// contains entries that are now also in the main file.
#[test]
fn test_no_duplicate_facts_after_post_checkpoint_crash() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("dedup.graph");
    let wal_path = wal_path_for(&db_path);

    // Session 1: write fact and crash (skip Drop)
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: usize::MAX,
            },
        )
        .unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        std::mem::forget(db);
    }

    // Back up the WAL before the next open checkpoints it away
    let wal_backup = std::fs::read(&wal_path).unwrap();

    // Session 2: normal open replays WAL then checkpoint on close
    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])")
                .unwrap(),
        );
        assert_eq!(n, 1, "Alice must be visible in session 2");
        // Drop triggers checkpoint: WAL is flushed to main file and deleted
    }

    // WAL must be gone after normal close
    assert!(!wal_path.exists(), "WAL must be deleted after normal close");

    // Restore the stale WAL backup to simulate the scenario where the checkpoint
    // write succeeded but the WAL deletion failed (crash between the two).
    std::fs::write(&wal_path, &wal_backup).unwrap();

    // Session 3: open again with the stale WAL present; replay should skip already-
    // checkpointed entries, producing exactly 1 fact.
    let db3 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db3.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 1,
        "must have exactly 1 Alice — no duplicates after stale WAL replay"
    );
}

// ── 4. Partial WAL entry is discarded; earlier entries intact ─────────────────

/// Write 1 fact, crash (no checkpoint), then append garbage bytes to the WAL
/// to simulate a partial write. Reopen and verify exactly 1 fact is recovered.
#[test]
fn test_partial_wal_entry_discarded_earlier_entries_intact() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("partial.graph");
    let wal_path = wal_path_for(&db_path);

    // Session 1: write 1 fact and crash
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: usize::MAX,
            },
        )
        .unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        std::mem::forget(db);
    }

    // Append garbage bytes after the valid WAL entry (simulate partial second write)
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&wal_path)
            .unwrap();
        // Bad checksum + partial payload
        file.write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0xDE, 0xAD, 0xBE, 0xEF])
            .unwrap();
    }

    // Recovery session
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 1,
        "exactly 1 fact (Alice) must survive despite partial WAL entry"
    );
}

// ── 5. Manual checkpoint deletes WAL ─────────────────────────────────────────

/// Write with a high threshold (WAL will not auto-checkpoint), verify WAL exists,
/// then call checkpoint() and verify the fact is still visible (and WAL is gone).
#[test]
fn test_manual_checkpoint_deletes_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("manual_cp.graph");
    let wal_path = wal_path_for(&db_path);

    let db = Minigraf::open_with_options(
        &db_path,
        OpenOptions {
            wal_checkpoint_threshold: usize::MAX,
        },
    )
    .unwrap();

    db.execute(r#"(transact [[:alice :name "Alice"]])"#)
        .unwrap();

    // WAL must exist (no auto-checkpoint fired)
    assert!(
        wal_path.exists(),
        "WAL must exist after write with high threshold"
    );

    // Manual checkpoint
    db.checkpoint().unwrap();

    // WAL must be gone
    assert!(
        !wal_path.exists(),
        "WAL must be deleted after manual checkpoint"
    );

    // Fact must still be visible
    let n = count_results(
        db.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(n, 1, "Alice must still be visible after checkpoint");

    // Main file header must reflect the checkpoint
    {
        use std::io::Read;
        let mut f = std::fs::File::open(&db_path).unwrap();
        let mut page = vec![0u8; PAGE_SIZE];
        f.read_exact(&mut page).unwrap();
        let header = FileHeader::from_bytes(&page).unwrap();
        assert!(
            header.last_checkpointed_tx_count > 0,
            "last_checkpointed_tx_count must be set after checkpoint"
        );
    }

    // Simulate crash: skip Drop (checkpoint already happened, no WAL to write)
    std::mem::forget(db);

    // Reopen: must recover the fact from the main file alone (no WAL needed)
    let db2 = Minigraf::open(&db_path).unwrap();
    let n2 = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n2, 1,
        "Alice must be present after crash-reopen when already checkpointed"
    );
}

// ── 6. Auto-checkpoint fires at threshold ─────────────────────────────────────

/// Set threshold=2, write 2 facts (triggering auto-checkpoint on the 2nd write).
/// Crash (mem::forget), then reopen. The facts must be in the main file — no WAL
/// needed for recovery.
#[test]
fn test_auto_checkpoint_fires_at_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("autocheckpoint.graph");
    let wal_path = wal_path_for(&db_path);

    // Session 1: 2 writes → auto-checkpoint fires on 2nd write
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: 2,
            },
        )
        .unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        db.execute(r#"(transact [[:bob :name "Bob"]])"#).unwrap();
        // After 2nd write the auto-checkpoint should have fired and deleted the WAL.
        assert!(
            !wal_path.exists(),
            "WAL must be deleted after auto-checkpoint at threshold=2"
        );
        // Crash: skip Drop checkpoint (but checkpoint already happened)
        std::mem::forget(db);
    }

    // No WAL after crash
    assert!(
        !wal_path.exists(),
        "WAL must not exist after auto-checkpoint crash"
    );

    // Session 2: facts must be in main file (no WAL replay needed)
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 2,
        "both Alice and Bob must survive via main file after auto-checkpoint"
    );
}

// ── 7. Explicit tx: all-or-nothing commit ─────────────────────────────────────

/// begin_write → 2 transacts → commit → crash (mem::forget) → reopen.
/// Both facts must be present.
#[test]
fn test_explicit_tx_all_or_nothing_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("explicit_commit.graph");

    // Session 1: explicit commit then crash
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: usize::MAX,
            },
        )
        .unwrap();

        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        tx.execute(r#"(transact [[:bob :name "Bob"]])"#).unwrap();
        tx.commit().unwrap();

        // Crash before Drop checkpoint
        std::mem::forget(db);
    }

    // Recovery session
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 2,
        "both Alice and Bob must survive explicit commit + crash"
    );
}

// ── 8. Explicit tx: rollback not persisted ────────────────────────────────────

/// begin_write → transact → rollback → close normally → reopen.
/// Zero facts must be present.
#[test]
fn test_explicit_tx_rollback_not_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("rollback.graph");

    // Session 1: write then rollback
    {
        let db = Minigraf::open(&db_path).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        tx.rollback();
        // Normal close (Drop checkpoints — nothing to checkpoint since rollback
        // means no WAL entry was written).
    }

    // Session 2: reopen and verify 0 facts
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(n, 0, "rolled-back facts must not survive reopen");
}

// ── 9. Explicit tx: multiple transacts then rollback ─────────────────────────

/// begin_write → 2 transacts → rollback → close normally → reopen.
/// Zero facts must be present (both transacts were inside the rolled-back tx).
#[test]
fn test_explicit_tx_multiple_transacts_rollback_not_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("multi_rollback.graph");
    let opts = OpenOptions {
        wal_checkpoint_threshold: usize::MAX,
    };

    {
        let db = Minigraf::open_with_options(&db_path, opts).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        tx.execute(r#"(transact [[:bob :name "Bob"]])"#).unwrap();
        tx.rollback();
        // db drops here → checkpoint (nothing to checkpoint since both facts were rolled back)
    }

    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(n, 0, "both rolled-back facts must not persist after reopen");
}

// ── 10. Concurrent reads while writer holds lock ─────────────────────────────

/// Commit a fact, then begin_write on the main thread (holds write lock).
/// Spawn a reader thread — it should be able to execute a query concurrently
/// because read-only `execute()` does not acquire the write lock.
#[test]
fn test_concurrent_reads_while_writer_holds_lock() {
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    let db = Minigraf::open(&db_path).unwrap();
    db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    db.checkpoint().unwrap();

    let db2 = db.clone();
    let barrier = Arc::new(Barrier::new(2));
    let barrier2 = Arc::clone(&barrier);

    // Hold write lock on main thread
    let _tx = db.begin_write().unwrap();

    // Spawn reader — must wait at barrier (guaranteeing write lock is held), then query
    let reader = std::thread::spawn(move || {
        barrier2.wait(); // synchronize: write lock is held at this point
        count_results(
            db2.execute("(query [:find ?name :where [?e :name ?name]])")
                .unwrap(),
        )
    });

    barrier.wait(); // signal: write lock is now held, reader may proceed
    let n = reader.join().unwrap();
    assert_eq!(
        n, 1,
        "reader must see committed state while writer holds the lock"
    );
    // _tx drops here (implicit rollback)
}

// ── 11. Implicit execute() write survives WAL replay ─────────────────────────

/// Verifies that `Minigraf::execute("(transact ...)")` writes to the WAL
/// *before* applying facts to in-memory FactStorage, so that WAL replay on
/// reopen returns the correct facts.
///
/// Test strategy:
/// 1. Open a file-backed database with a very high checkpoint threshold.
/// 2. Call `execute("(transact ...)")` — the implicit-transaction path.
/// 3. `mem::forget` the database to simulate a crash (no Drop checkpoint).
/// 4. Reopen the database (triggers WAL replay).
/// 5. Assert the fact is present — proving the WAL was written during step 2.
#[test]
fn test_implicit_tx_execute_survives_replay() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("implicit_tx.graph");
    let wal_path = wal_path_for(&db_path);

    // Session 1: write via implicit execute() then crash (skip Drop)
    {
        let db = Minigraf::open_with_options(
            &db_path,
            OpenOptions {
                wal_checkpoint_threshold: usize::MAX,
            },
        )
        .unwrap();

        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();

        // Simulate crash: skip Drop (and its checkpoint).
        std::mem::forget(db);
    }

    // WAL must exist — no checkpoint fired.
    assert!(wal_path.exists(), "WAL must exist after simulated crash");

    // Session 2: reopen triggers WAL replay.
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])")
            .unwrap(),
    );
    assert_eq!(
        n, 1,
        "Alice must be recovered via WAL replay after implicit execute() crash"
    );
}

// ── 12. V2 file upgrades to V3 on checkpoint ─────────────────────────────────

/// Create a v2-format `.graph` file manually (version field = 2, no
/// `last_checkpointed_tx_count`), open it with `Minigraf`, write a fact,
/// checkpoint, then read the raw header and verify it is now v3.
#[test]
fn test_v2_file_opens_and_upgrades_to_v3_on_checkpoint() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v2.graph");

    // ── Build a minimal v2 `.graph` file ──────────────────────────────────
    //
    // V2 and V3 have identical binary layouts.
    // The only difference is the version field (bytes 4-7): 2 vs 3.
    // In V2, bytes 24-31 were the unused `edge_count` field (always 0).
    // Phase 5 repurposed that slot as `last_checkpointed_tx_count` without
    // changing the wire layout. Opening a V2 file with Phase 5 code works
    // transparently because `last_checkpointed_tx_count` reads as 0 from
    // the old `edge_count` slot.
    //
    // The file contains exactly 1 page (the header page, 4096 bytes).
    {
        let mut page = vec![0u8; PAGE_SIZE];
        // magic
        page[0..4].copy_from_slice(b"MGRF");
        // version = 2
        page[4..8].copy_from_slice(&2u32.to_le_bytes());
        // page_count = 1
        page[8..16].copy_from_slice(&1u64.to_le_bytes());
        // node_count = 0 (bytes 16..24 already zero)
        // last_checkpointed_tx_count = 0 (bytes 24..32 already zero)
        // reserved = 0 (bytes 32..64 already zero)

        let mut file = std::fs::File::create(&db_path).unwrap();
        file.write_all(&page).unwrap();
        file.sync_all().unwrap();
    }

    // ── Open, write a fact, and checkpoint ────────────────────────────────
    {
        let db = Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        db.checkpoint().unwrap();
        // Drop runs another checkpoint, but that's idempotent.
    }

    // ── Read the raw header and assert version = 3 ───────────────────────
    let raw = std::fs::read(&db_path).unwrap();
    assert!(
        raw.len() >= PAGE_SIZE,
        "file must be at least one page after checkpoint"
    );
    let header = FileHeader::from_bytes(&raw[..PAGE_SIZE]).unwrap();
    assert_eq!(
        header.version, 5,
        "file must be upgraded to v5 on checkpoint"
    );
    assert_eq!(header.magic, *b"MGRF", "magic number must be preserved");
    assert!(
        header.last_checkpointed_tx_count > 0,
        "last_checkpointed_tx_count must be set after checkpoint on v2→v5 upgrade"
    );
}
