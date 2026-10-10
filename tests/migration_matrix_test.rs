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
    drop(f);
    let before = std::fs::read(&path).unwrap();
    let result = Minigraf::open(&path);
    assert!(result.is_err(), "unsupported version must produce an error");
    assert_eq!(result.err().unwrap().code(), "STG-006");
    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        before, after,
        "a rejected future-version file must be left unmodified"
    );
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

/// A damaged type byte on a v7 fact page fails the open with STG-014, read-write
/// and read-only, and leaves the file as it was (#496). Before, the page was
/// skipped: the open succeeded with the page's facts missing, and the migration
/// freed the page. Uses the golden v7 file, as the `fact_page` fuzz target does.
#[test]
fn damaged_v7_fact_page_fails_open_and_leaves_file_unchanged() {
    use minigraf::OpenOptions;
    let golden = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/v7_basic.graph"),
    )
    .unwrap();
    // Page 1 with only its type byte changed, and page 1 overwritten with noise.
    let mut type_byte = golden.clone();
    type_byte[PAGE_SIZE] = 0x00;
    let mut noise = golden.clone();
    let mut x = 0x9E37_79B9u32;
    for b in &mut noise[PAGE_SIZE..2 * PAGE_SIZE] {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x.to_le_bytes()[0];
    }
    noise[PAGE_SIZE] = 0xA5;
    for bytes in [type_byte, noise] {
        for read_only in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("damaged.graph");
            std::fs::write(&path, &bytes).unwrap();
            let err =
                match Minigraf::open_with_options(&path, OpenOptions::new().read_only(read_only)) {
                    Ok(_) => panic!("a damaged v7 fact page must fail the open"),
                    Err(e) => e,
                };
            assert_eq!(err.code(), "STG-014");
            assert!(
                std::fs::read(&path).unwrap() == bytes,
                "a rejected v7 file must be left unmodified"
            );
            assert!(
                !dir.path().join("damaged.graph.wal").exists(),
                "no WAL is created"
            );
        }
    }
}
