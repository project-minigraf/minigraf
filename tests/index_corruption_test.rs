//! Index corruption recovery tests (#216).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::QueryResult;
use minigraf::db::Minigraf;

const PAGE_SIZE: usize = 4096;

fn count_results(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

fn build_valid_db(path: &std::path::Path, n_facts: usize) {
    let db = Minigraf::open(path).unwrap();
    for i in 0..n_facts {
        db.execute(&format!(r#"(transact [[:e{i} :idx {i}]])"#))
            .unwrap();
    }
    db.checkpoint().unwrap();
}

fn corrupt_bytes_at(path: &std::path::Path, offset: u64, len: usize) {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).unwrap();
    for b in &mut buf {
        *b ^= 0xFF;
    }
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.write_all(&buf).unwrap();
    f.sync_all().unwrap();
}

#[test]
fn corrupted_header_checksum_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt_checksum.graph");
    build_valid_db(&path, 3);
    corrupt_bytes_at(&path, 76, 4);
    match Minigraf::open(&path) {
        Ok(db) => {
            // If the open succeeded despite checksum corruption, facts must still be readable.
            let n = count_results(
                db.execute("(query [:find ?e :where [?e :idx ?i]])")
                    .unwrap(),
            );
            assert_eq!(
                n, 3,
                "facts must be readable even after header checksum corruption"
            );
        }
        Err(_) => {
            // Rejected at open — also acceptable; either way: no panic.
        }
    }
}

#[test]
fn corrupted_btree_leaf_page_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt_leaf.graph");
    build_valid_db(&path, 5);
    let leaf_offset = (PAGE_SIZE * 2) as u64;
    corrupt_bytes_at(&path, leaf_offset, 64);
    let _ = Minigraf::open(&path);
}

#[test]
fn corrupted_btree_internal_page_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt_internal.graph");
    build_valid_db(&path, 10);
    let offset = (PAGE_SIZE * 3) as u64;
    corrupt_bytes_at(&path, offset, 128);
    let _ = Minigraf::open(&path);
}

#[test]
fn root_page_pointer_mismatch_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt_root.graph");
    build_valid_db(&path, 3);
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(16)).unwrap();
        f.write_all(&u64::MAX.to_le_bytes()).unwrap();
    }
    let _ = Minigraf::open(&path);
}

#[test]
fn query_results_after_non_critical_corruption_match_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recover_check.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .unwrap();
        db.execute(r#"(transact [[:bob :name "Bob"]])"#).unwrap();
        db.checkpoint().unwrap();
    }
    corrupt_bytes_at(&path, 84, 16);
    match Minigraf::open(&path) {
        Ok(db) => {
            let n = count_results(
                db.execute("(query [:find ?n :where [?e :name ?n]])")
                    .unwrap(),
            );
            assert_eq!(
                n, 2,
                "both facts must be visible after non-critical corruption"
            );
        }
        Err(_) => {}
    }
}
