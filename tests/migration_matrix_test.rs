//! Migration matrix tests (#215).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::QueryResult;
use minigraf::db::Minigraf;

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

#[test]
fn v3_empty_migrates_without_error() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.graph");
    // Build a minimally valid v3 header: magic + version=3 + page_count=1
    // All other fields zero (roots=0 means empty index, fact_count=0).
    let mut page = vec![0u8; PAGE_SIZE];
    page[0..4].copy_from_slice(&MAGIC_NUMBER);
    page[4..8].copy_from_slice(&3u32.to_le_bytes()); // version
    page[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&page).unwrap();
    drop(f);
    assert!(
        Minigraf::open(&path).is_ok(),
        "v3 empty file should open without error"
    );
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
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute(r#"(transact [[:e2 :color "blue"]])"#).unwrap();
        std::mem::forget(db);
    }
    let db3 = Minigraf::open(&path).unwrap();
    let n = count_results(
        db3.execute("(query [:find ?c :where [?e :color ?c]])")
            .unwrap(),
    );
    assert_eq!(n, 2, "WAL replay after checkpoint must be idempotent");
}
