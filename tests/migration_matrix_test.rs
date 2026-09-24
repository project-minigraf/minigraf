//! Migration matrix tests (#215).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::QueryResult;
use minigraf::db::Minigraf;

mod common;
use common::{CrashTx, run_crashing_child};

const PAGE_SIZE: usize = 4096;
const MAGIC_NUMBER: [u8; 4] = *b"MGRF";

fn count_results(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

#[test]
fn v7_round_trip_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute(r#"(transact [[:e1 :name "Alice"]])"#).unwrap();
        db.checkpoint().unwrap();
    }
    let db2 = Minigraf::open(&path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?n :where [?e :name ?n]])")
            .unwrap(),
    );
    assert_eq!(n, 1, "v7 round-trip: Alice must survive close/reopen");
}

/// Formats v1–v6 are no longer readable (v3.0.0 file-format policy). Opening
/// one must fail with STG-028 and must not modify the file.
#[test]
fn pre_v7_versions_are_rejected_with_stg_028() {
    for version in 1u32..=6 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.graph");
        let mut page = vec![0u8; PAGE_SIZE];
        page[0..4].copy_from_slice(&MAGIC_NUMBER);
        page[4..8].copy_from_slice(&version.to_le_bytes());
        page[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
        std::fs::write(&path, &page).unwrap();

        let err = match Minigraf::open(&path) {
            Ok(_) => panic!("pre-v7 file must not open"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "STG-028", "pre-v7 file must fail with STG-028");
        assert!(
            err.to_string().contains("v2.x"),
            "STG-028 message must tell the user how to upgrade"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            page,
            "rejected file must be left unmodified"
        );
    }
}

#[test]
fn corrupt_magic_fails_loudly() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.graph");
    let mut page = vec![0u8; PAGE_SIZE];
    page[0..4].copy_from_slice(b"XXXX");
    page[4..8].copy_from_slice(&7u32.to_le_bytes());
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&page).unwrap();
    let result = Minigraf::open(&path);
    assert!(result.is_err(), "corrupt magic must produce an error");
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("magic") || msg.contains("invalid") || msg.contains("not a"),
        "error message must describe the corrupt magic"
    );
}

#[test]
fn unsupported_version_fails_loudly() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.graph");
    let mut page = vec![0u8; PAGE_SIZE];
    page[0..4].copy_from_slice(&MAGIC_NUMBER);
    page[4..8].copy_from_slice(&99u32.to_le_bytes());
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&page).unwrap();
    let result = Minigraf::open(&path);
    assert!(result.is_err(), "unsupported version must produce an error");
    assert_eq!(result.err().unwrap().code(), "STG-006");
}

#[test]
fn wal_replay_after_migration_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute(r#"(transact [[:e1 :color "red"]])"#).unwrap();
        db.checkpoint().unwrap();
    }
    // Crash before checkpoint: no Drop runs, so the WAL is left for the next
    // open to replay.
    run_crashing_child(
        &path,
        1000,
        &[r#"(transact [[:e2 :color "blue"]])"#],
        CrashTx::Implicit,
    );
    let db3 = Minigraf::open(&path).unwrap();
    let n = count_results(
        db3.execute("(query [:find ?c :where [?e :color ?c]])")
            .unwrap(),
    );
    assert_eq!(n, 2, "WAL replay after checkpoint must be idempotent");
}
