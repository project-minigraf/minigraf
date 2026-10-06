//! Regression tests for #360 (part of the #277 structured error codes rollout):
//! `src/wal.rs`'s migrated `bail_coded!`/`err_coded!` call sites must surface
//! their real `WAL-0xx` code through the public `Minigraf` API, not just
//! `INT-000`.
//!
//! `WAL-004` (fact serialised size exceeds u32 range, ~4 GB) and `WAL-005`
//! (WAL num_facts exceeds platform `usize`) are both documented as
//! "practically unreachable" — the latter can never actually fail on any
//! 64-bit CI target, since `usize` and `u64` are the same width there. Both
//! codes are still covered by `error::tests::registry_is_a_subset_of_error_reference_doc`
//! and `error::tests::every_error_code_variant_has_a_registry_entry`; there is
//! no way to trigger them from the public API without an unrealistic (multi-GB)
//! fixture, so they are intentionally not exercised here.

use minigraf::{ErrorCategory, Minigraf, OpenOptions};
use std::io::Write;

/// `Inner`'s `Drop` impl performs a best-effort checkpoint on close, which
/// deletes the `.wal` sidecar — exactly the file these tests need to corrupt
/// after closing the handle. `wal_checkpoint_threshold: usize::MAX` is the
/// documented sentinel that suppresses all checkpointing, including on drop.
fn no_checkpoint_options() -> OpenOptions {
    OpenOptions::default().wal_checkpoint_threshold(usize::MAX)
}

fn wal_path_for(db_path: &std::path::Path) -> std::path::PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(".wal");
    std::path::PathBuf::from(p)
}

/// WAL corruption: a `.wal` file with a bad magic number, encountered while
/// `Minigraf::open()` replays the WAL on startup, must surface as `WAL-001`.
#[test]
fn wal_corruption_bad_magic_returns_wal_001() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bad-magic.graph");
    let wal_path = wal_path_for(&db_path);

    // Create the database and write one fact so a real WAL file exists.
    // `no_checkpoint_options()` keeps close-time checkpointing from deleting
    // the WAL when `db` is dropped below.
    let db = Minigraf::open_with_options(&db_path, no_checkpoint_options()).unwrap();
    db.execute(r#"(transact [[:alice :name "Alice"]])"#)
        .unwrap();
    assert!(wal_path.exists(), "WAL must exist after a write");
    drop(db); // release the file lock so the WAL can be reopened below

    // Corrupt just the magic bytes, leaving the rest of the header intact.
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&wal_path)
            .unwrap();
        file.write_all(b"XXXX").unwrap();
    }

    let result = Minigraf::open(&db_path);
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("opening a database with a bad WAL magic number should fail"),
    };
    assert_eq!(err.category(), ErrorCategory::Wal);
    assert_eq!(err.code(), "WAL-001");
}

/// Replay failure: a `.wal` file written by an unsupported WAL version,
/// encountered while `Minigraf::open()` replays the WAL on startup, must
/// surface as `WAL-002`.
#[test]
fn wal_replay_bad_version_returns_wal_002() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bad-version.graph");
    let wal_path = wal_path_for(&db_path);

    let db = Minigraf::open_with_options(&db_path, no_checkpoint_options()).unwrap();
    db.execute(r#"(transact [[:bob :name "Bob"]])"#).unwrap();
    assert!(wal_path.exists(), "WAL must exist after a write");
    drop(db);

    // Corrupt only the version field (bytes 4..8), leaving a valid magic.
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&wal_path)
            .unwrap();
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(4)).unwrap();
        file.write_all(&99u32.to_le_bytes()).unwrap();
    }

    let result = Minigraf::open(&db_path);
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("opening a database with an unsupported WAL version should fail"),
    };
    assert_eq!(err.category(), ErrorCategory::Wal);
    assert_eq!(err.code(), "WAL-002");
}

/// Size limit: a single fact whose serialised size exceeds the WAL's
/// per-entry limit must be rejected at `transact` time as `WAL-003`, through
/// the public `execute()` API — not just in the internal WAL unit tests.
#[test]
fn wal_size_limit_oversized_fact_returns_wal_003() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("oversized.graph");

    let db = Minigraf::open(&db_path).unwrap();

    // Comfortably over MAX_FACT_BYTES (4052 bytes) but well under the
    // parser's own 1 MB string cap, so the parser accepts it and the WAL
    // layer is what rejects it.
    let huge_value = "a".repeat(20_000);
    let query = format!(r#"(transact [[:doc :body "{huge_value}"]])"#);

    let result = db.execute(&query);
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("transacting an oversized fact should fail"),
    };
    assert_eq!(err.category(), ErrorCategory::Wal);
    assert_eq!(err.code(), "WAL-003");
}
