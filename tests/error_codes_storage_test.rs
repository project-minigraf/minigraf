//! Regression tests for #359 (STG-0xx storage error codes, part of the #277
//! structured error codes rollout): asserts that specific `STG-NNN` codes
//! come back through the public `Minigraf::open()` API for corrupt/invalid
//! `.graph` files, not just a generic error.
//!
//! Only covers codes reachable through the public API by constructing
//! byte-level `.graph` files directly (the `storage` module itself is
//! crate-private, so these tests cannot call `FileHeader`/`StorageBackend`
//! directly — see the unit tests inside `src/storage/*.rs` for codes that
//! are only reachable at that lower level).

use minigraf::{ErrorCategory, Minigraf};
use std::io::Write;

/// Unique scratch directory per test run so parallel tests never collide.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("minigraf_stg_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &std::path::Path, bytes: &[u8]) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(bytes).unwrap();
}

/// A minimal, valid v7 header prefix (magic + version + page_count), padded
/// to a full 4096-byte page with the given `patch` applied on top.
fn header_page(patch: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut bytes = vec![0u8; 4096];
    bytes[0..4].copy_from_slice(b"MGRF");
    bytes[4..8].copy_from_slice(&7u32.to_le_bytes()); // version = 7
    bytes[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
    patch(&mut bytes);
    bytes
}

#[test]
fn open_bad_magic_number_returns_stg_002() {
    let dir = scratch_dir("bad_magic");
    let path = dir.join("bad.graph");
    // A full page of non-MGRF bytes.
    write_file(&path, &vec![0xAAu8; 4096]);

    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("opening a file with a bad magic number should fail"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-002");
    assert_eq!(err.category(), ErrorCategory::Storage);
}

#[test]
fn open_unsupported_version_returns_stg_006() {
    let dir = scratch_dir("bad_version");
    let path = dir.join("bad.graph");
    let bytes = header_page(|b| {
        b[4..8].copy_from_slice(&99u32.to_le_bytes()); // version = 99
    });
    write_file(&path, &bytes);

    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("opening a file with an unsupported version should fail"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-006");
    assert_eq!(err.category(), ErrorCategory::Storage);
}

#[test]
fn open_zero_page_count_returns_stg_007() {
    let dir = scratch_dir("zero_pages");
    let path = dir.join("bad.graph");
    let bytes = header_page(|b| {
        b[8..16].copy_from_slice(&0u64.to_le_bytes()); // page_count = 0
    });
    write_file(&path, &bytes);

    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("opening a file with page_count=0 should fail"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-007");
    assert_eq!(err.category(), ErrorCategory::Storage);
}

#[test]
fn open_eavt_root_page_out_of_bounds_returns_stg_008() {
    let dir = scratch_dir("eavt_oob");
    let path = dir.join("bad.graph");
    let bytes = header_page(|b| {
        // eavt_root_page (bytes 32..40) >= page_count (1) is invalid.
        b[32..40].copy_from_slice(&5u64.to_le_bytes());
    });
    write_file(&path, &bytes);

    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("opening a file with eavt_root_page >= page_count should fail"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-008");
    assert_eq!(err.category(), ErrorCategory::Storage);
}

#[test]
fn open_fact_page_count_exceeds_page_count_returns_stg_009() {
    let dir = scratch_dir("fact_pc_oob");
    let path = dir.join("bad.graph");
    let bytes = header_page(|b| {
        // fact_page_count (bytes 72..80, v6+) > page_count (1) is invalid.
        b[72..80].copy_from_slice(&5u64.to_le_bytes());
    });
    write_file(&path, &bytes);

    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("opening a file with fact_page_count > page_count should fail"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-009");
    assert_eq!(err.category(), ErrorCategory::Storage);
}

#[test]
fn second_open_in_same_process_returns_stg_025() {
    let dir = scratch_dir("dup_open");
    let path = dir.join("dup.graph");

    let first = Minigraf::open(&path).expect("first open should succeed");
    let err = match Minigraf::open(&path) {
        Ok(_) => panic!("a second handle in the same process must be refused"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "STG-025");
    assert_eq!(err.category(), ErrorCategory::Storage);
    drop(first);
}
