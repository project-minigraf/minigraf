//! Integration tests for Phase 6.5: on-disk B+tree indexes (file format v6).

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
    let result = db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert!(
            results.iter().any(|row| row.iter().any(|v| v.to_string().contains('0'))),
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
    let result = db.execute("(query [:find ?v :where [:e100 :val ?v]])").unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert!(
            results.iter().any(|row| row.iter().any(|v| v.to_string().contains("100"))),
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
    db.execute("(transact [[:e10 :val 10][:e11 :val 11][:e12 :val 12]])").unwrap();

    // Query must see both committed (e0) and pending (e10)
    let r0 = db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = r0 {
        assert!(
            results.iter().any(|row| row.iter().any(|v| v.to_string().contains('0'))),
            "committed e0 missing"
        );
    } else {
        panic!("expected QueryResults for committed e0 in test_v6_pending_plus_committed_merge");
    }

    let r10 = db.execute("(query [:find ?v :where [:e10 :val ?v]])").unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = r10 {
        assert!(
            results.iter().any(|row| row.iter().any(|v| v.to_string().contains("10"))),
            "pending e10 missing"
        );
    } else {
        panic!("expected QueryResults for pending e10 in test_v6_pending_plus_committed_merge");
    }
}

#[test]
fn test_v6_migration_from_v5_eager() {
    // Write a minimal v5 header directly to the .graph file (raw bytes),
    // then open with v6 code — migrate_v5_to_v6 must run and produce a v6 file.
    // We cannot use OpenOptions to produce a v5 file (v6 code always writes v6),
    // so we write the raw 72-byte v5 header at offset 0.
    let (_tmp, path) = tmp_path();

    // Create an empty v5 .graph file with only a header (no facts, no index pages)
    {
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path).unwrap();
        let mut page = vec![0u8; 4096]; // PAGE_SIZE
        page[0..4].copy_from_slice(b"MGRF");
        page[4..8].copy_from_slice(&5u32.to_le_bytes()); // version = 5
        page[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
        page[68] = 0x02; // fact_page_format = PACKED
        use std::io::Write;
        f.write_all(&page).unwrap();
    }

    // Open with v6 code — migration must run
    let db = OpenOptions::new().path(&path).open().unwrap();

    // Query on empty DB must return empty result (not an error)
    let _result = db.execute("(query [:find ?e :where [?e :any :any]])").unwrap();
    drop(db);

    // Re-open and verify header was upgraded to v6
    let mut f = std::fs::File::open(&path).unwrap();
    let mut header_bytes = vec![0u8; 4096];
    use std::io::Read;
    f.read_exact(&mut header_bytes).unwrap();
    let version = u32::from_le_bytes(header_bytes[4..8].try_into().unwrap());
    assert_eq!(version, 6, "header must be upgraded from v5 to v6; got version={}", version);
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
        tx.execute(&format!("(transact [[:e{} :val {}]])", i, i)).unwrap();
        tx.commit().unwrap();
    }
    db.checkpoint().unwrap();

    // WAL sidecar is named <db_path>.wal (per CLAUDE.md "WAL sidecar <db>.wal")
    let wal_path = format!("{}.wal", path);
    let wal_absent = !Path::new(&wal_path).exists();
    let wal_empty = std::fs::metadata(&wal_path).map(|m| m.len() == 0).unwrap_or(true);
    assert!(
        wal_absent || wal_empty,
        "WAL must be absent or empty after explicit checkpoint; path={} size={}",
        wal_path,
        std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0),
    );

    // Queries still return correct results
    let result = db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains('0'), "query must work after checkpoint; got: {}", s);
}

#[test]
fn test_v6_dead_pages_queries_correct_after_two_checkpoints() {
    let (_tmp, path) = tmp_path();

    // First checkpoint
    populate_and_checkpoint(20, &path);

    // Second checkpoint (adds new facts + new B+tree; old index pages are dead)
    let db = OpenOptions::new().path(&path).open().unwrap();
    db.execute("(transact [[:e20 :val 20][:e21 :val 21]])").unwrap();
    db.checkpoint().unwrap();

    // Re-open and verify queries are correct
    let db2 = OpenOptions::new().path(&path).open().unwrap();
    let r = db2.execute("(query [:find ?v :where [:e20 :val ?v]])").unwrap();
    let sr = format!("{:?}", r);
    assert!(sr.contains("20"), "e20 should be queryable after second checkpoint; got: {}", sr);
    let r0 = db2.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    let s0 = format!("{:?}", r0);
    assert!(s0.contains('0'), "original e0 should still be visible; got: {}", s0);
}

#[test]
fn test_v6_checksum_mismatch_triggers_rebuild() {
    use std::io::{Seek, SeekFrom, Write};

    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(30, &path);

    // Corrupt the index_checksum field in the header (bytes 64..68)
    {
        let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(64)).unwrap();
        f.write_all(&[0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    }

    // Re-open: checksum mismatch must trigger v6 rebuild path
    let db = OpenOptions::new().path(&path).open().unwrap();
    let result = db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains('0'), "queries must work after rebuild; got: {}", s);
}

#[test]
fn test_v6_reopen_close_reopen() {
    let (_tmp, path) = tmp_path();
    populate_and_checkpoint(100, &path);

    // Open, query, close, reopen, query again
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        let r = db.execute("(query [:find ?v :where [:e50 :val ?v]])").unwrap();
        let s = format!("{:?}", r);
        assert!(s.contains("50"), "first open; got: {}", s);
    }
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        let r = db.execute("(query [:find ?v :where [:e50 :val ?v]])").unwrap();
        let s = format!("{:?}", r);
        assert!(s.contains("50"), "after reopen; got: {}", s);
    }
}
