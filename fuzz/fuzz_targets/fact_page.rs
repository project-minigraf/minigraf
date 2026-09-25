#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    let mut content = vec![0u8; 4096 * 2];
    content[0..4].copy_from_slice(b"MGRF");
    // v8 header (v1-v6 are rejected; v7 would always force a full rebuild
    // regardless of these fields, which is a different code path than the
    // one this target means to fuzz).
    content[4..8].copy_from_slice(&8u32.to_le_bytes());
    // page_count = 2 (header page 0 + one fact page at page 1).
    content[8..16].copy_from_slice(&2u64.to_le_bytes());
    // fact_page_count = 1, eavt_root_page left at 0: with no index root,
    // `load()` always rebuilds from the fact pages, so opening alone decodes
    // page 1 as a packed fact page (postcard `Fact` deserialization) without
    // needing a follow-up query.
    content[72..80].copy_from_slice(&1u64.to_le_bytes());
    // header_checksum (bytes 80-83) and index_checksum (bytes 64-67) are left
    // zero: zero is the "unset" sentinel that skips checksum verification.
    let copy_len = data.len().min(4096);
    content[4096..4096 + copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &content).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&path);
});
