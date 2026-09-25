#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    let mut content = vec![0u8; 4096 * 3];
    content[0..4].copy_from_slice(b"MGRF");
    // v8 header (v1-v6 are rejected; v7 would always force a full rebuild
    // from fact pages and never touch the root page below, which is a
    // different code path than the one this target means to fuzz).
    content[4..8].copy_from_slice(&8u32.to_le_bytes());
    // page_count = 3 (header page 0, unused page 1, fuzzed root page 2).
    content[8..16].copy_from_slice(&3u64.to_le_bytes());
    // eavt_root_page = 2: points `load()`'s index wiring, and the query
    // below, at the fuzzed page as the EAVT B+tree root. fact_page_count is
    // left at 0, so `load()` takes the "no facts, trust the root" branch
    // and does not force a rebuild that would ignore this page.
    content[32..40].copy_from_slice(&2u64.to_le_bytes());
    // header_checksum (bytes 80-83) and index_checksum (bytes 64-67) are left
    // zero: zero is the "unset" sentinel that skips checksum verification.
    let copy_len = data.len().min(4096);
    content[4096 * 2..4096 * 2 + copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &content).is_err() {
        return;
    }
    let Ok(db) = minigraf::db::Minigraf::open(&path) else {
        return;
    };
    // `open()` alone only wires up `OnDiskIndexReader` against the root page —
    // it never reads it. Run an entity-bound query so the EAVT range scan
    // actually decodes the fuzzed page as a v8 B+tree leaf/internal node.
    let _ = db.execute(
        r#"(query [:find ?a ?v :where [#uuid "00000000-0000-0000-0000-000000000000" ?a ?v]])"#,
    );
});
