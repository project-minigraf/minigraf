//! Integration tests for Phase 6.5: on-disk B+tree indexes (file format v6).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::OpenOptions;
use tempfile::NamedTempFile;

fn tmp_path() -> (NamedTempFile, String) {
    let f = NamedTempFile::new().unwrap();
    let p = f.path().to_str().unwrap().to_string();
    (f, p)
}

/// Create and checkpoint a file DB with N facts about entity `:eN` with attribute `:val`.
fn populate_and_checkpoint(n: usize, path: &str) {
    let db = OpenOptions::new().path(path).open().unwrap();
    let cmd: String = {
        let mut s = String::from("(transact [");
        for i in 0..n {
            s.push_str(&format!("[:e{} :val {}]", i, i));
        }
        s.push_str("])");
        s
    };
    db.execute(&cmd).unwrap();
    db.checkpoint().unwrap();
}

#[test]
fn test_v6_roundtrip_basic() {
    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(10, &path);

    let db = OpenOptions::new().path(&path).open().unwrap();
    let result = db
        .execute("(query [:find ?v :where [:e0 :val ?v]])")
        .unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert!(
            results
                .iter()
                .any(|row| row.iter().any(|v| format!("{:?}", v).contains('0'))),
            "entity e0 should have val 0"
        );
    } else {
        panic!("expected QueryResults for test_v6_roundtrip_basic");
    }
}

#[test]
fn test_v6_range_scan_across_leaves() {
    // 500 facts force multiple leaf pages per index
    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(500, &path);

    let db = OpenOptions::new().path(&path).open().unwrap();
    let result = db
        .execute("(query [:find ?v :where [:e100 :val ?v]])")
        .unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert!(
            results
                .iter()
                .any(|row| row.iter().any(|v| format!("{:?}", v).contains("100"))),
            "entity e100 should have val 100"
        );
    } else {
        panic!("expected QueryResults for test_v6_range_scan_across_leaves");
    }
}

#[test]
fn test_v6_pending_plus_committed_merge() {
    let (_tmp, path) = tmp_path();
    // Checkpoint 10 facts (committed)
    populate_and_checkpoint(10, &path);

    // Add 5 more (pending, in WAL)
    let db = OpenOptions::new().path(&path).open().unwrap();
    db.execute("(transact [[:e10 :val 10][:e11 :val 11][:e12 :val 12]])")
        .unwrap();

    // Query must see both committed (e0) and pending (e10)
    let r0 = db
        .execute("(query [:find ?v :where [:e0 :val ?v]])")
        .unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = r0 {
        assert!(
            results
                .iter()
                .any(|row| row.iter().any(|v| format!("{:?}", v).contains('0'))),
            "committed e0 missing"
        );
    } else {
        panic!("expected QueryResults for committed e0 in test_v6_pending_plus_committed_merge");
    }

    let r10 = db
        .execute("(query [:find ?v :where [:e10 :val ?v]])")
        .unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = r10 {
        assert!(
            results
                .iter()
                .any(|row| row.iter().any(|v| format!("{:?}", v).contains("10"))),
            "pending e10 missing"
        );
    } else {
        panic!("expected QueryResults for pending e10 in test_v6_pending_plus_committed_merge");
    }
}

#[test]
fn test_v5_file_is_rejected() {
    // v3.0.0 dropped support for formats v1–v6. Write a minimal v5 header
    // directly to the .graph file (raw bytes) and verify that opening it
    // fails with STG-028 and leaves the file unmodified.
    let (_tmp, path) = tmp_path();

    let mut page = vec![0u8; 4096]; // PAGE_SIZE
    page[0..4].copy_from_slice(b"MGRF");
    page[4..8].copy_from_slice(&5u32.to_le_bytes()); // version = 5
    page[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
    page[68] = 0x02; // fact_page_format = PACKED
    std::fs::write(&path, &page).unwrap();

    let err = match OpenOptions::new().path(&path).open() {
        Ok(_) => panic!("v5 file must not open"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-028", "v5 file must fail with STG-028");

    assert_eq!(
        std::fs::read(&path).unwrap(),
        page,
        "rejected file must be left unmodified"
    );
}

#[test]
fn test_v6_explicit_checkpoint_clears_wal() {
    // After writing facts and calling checkpoint(), the WAL sidecar file must
    // be absent or empty, and subsequent queries must still work.
    use std::path::Path;
    let (_tmp, path) = tmp_path();

    let db = OpenOptions::new().path(&path).open().unwrap();

    // Write 10 facts then explicitly checkpoint
    for i in 0..10 {
        let mut tx = db.begin_write().unwrap();
        tx.execute(&format!("(transact [[:e{} :val {}]])", i, i))
            .unwrap();
        tx.commit().unwrap();
    }
    db.checkpoint().unwrap();

    // WAL sidecar is named <db_path>.wal (per CLAUDE.md "WAL sidecar <db>.wal")
    let wal_path = format!("{}.wal", path);
    let wal_absent = !Path::new(&wal_path).exists();
    let wal_empty = std::fs::metadata(&wal_path)
        .map(|m| m.len() == 0)
        .unwrap_or(true);
    assert!(
        wal_absent || wal_empty,
        "WAL must be absent or empty after explicit checkpoint; path={} size={}",
        wal_path,
        std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0),
    );

    // Queries still return correct results
    let result = db
        .execute("(query [:find ?v :where [:e0 :val ?v]])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains('0'), "query must work after checkpoint");
}

#[test]
fn test_v6_dead_pages_queries_correct_after_two_checkpoints() {
    let (_tmp, path) = tmp_path();

    // First checkpoint
    populate_and_checkpoint(20, &path);

    // Second checkpoint (adds new facts + new B+tree; old index pages are dead)
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        db.execute("(transact [[:e20 :val 20][:e21 :val 21]])")
            .unwrap();
        db.checkpoint().unwrap();
    }

    // Re-open and verify queries are correct
    let db2 = OpenOptions::new().path(&path).open().unwrap();
    let r = db2
        .execute("(query [:find ?v :where [:e20 :val ?v]])")
        .unwrap();
    let sr = format!("{:?}", r);
    assert!(
        sr.contains("20"),
        "e20 should be queryable after second checkpoint"
    );
    let r0 = db2
        .execute("(query [:find ?v :where [:e0 :val ?v]])")
        .unwrap();
    let s0 = format!("{:?}", r0);
    assert!(s0.contains('0'), "original e0 should still be visible");
}

/// Damaging the newest meta page after a completed checkpoint (its WAL already
/// deleted) must refuse to open with STG-033, never silently fall back to the
/// older meta and lose that checkpoint's facts.
#[test]
fn newest_meta_damaged_after_commit_is_refused() {
    use std::io::{Read, Seek, SeekFrom, Write};

    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(30, &path);

    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut pages = [vec![0u8; 4096], vec![0u8; 4096]];
    for p in pages.iter_mut() {
        f.read_exact(p).unwrap();
    }
    let generation = |p: &[u8]| {
        if &p[8..12] == b"META" {
            u64::from_le_bytes(p[16..24].try_into().unwrap())
        } else {
            0
        }
    };
    let newest = if generation(&pages[0]) > generation(&pages[1]) {
        0
    } else {
        1
    };
    assert!(
        generation(&pages[newest]) >= 2,
        "a checkpoint was committed"
    );
    pages[newest][30] ^= 0xFF;
    f.seek(SeekFrom::Start(newest as u64 * 4096)).unwrap();
    f.write_all(&pages[newest]).unwrap();
    drop(f);

    match OpenOptions::new().path(&path).open() {
        Ok(_) => panic!("opening with the newest meta damaged must fail"),
        Err(e) => assert_eq!(e.code(), "STG-033"),
    }
}

#[test]
fn test_v6_reopen_close_reopen() {
    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(100, &path);

    // Open, query, close, reopen, query again
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        let r = db
            .execute("(query [:find ?v :where [:e50 :val ?v]])")
            .unwrap();
        let s = format!("{:?}", r);
        assert!(s.contains("50"), "first open");
    }
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        let r = db
            .execute("(query [:find ?v :where [:e50 :val ?v]])")
            .unwrap();
        let s = format!("{:?}", r);
        assert!(s.contains("50"), "after reopen");
    }
}
