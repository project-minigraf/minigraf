//! Cross-platform `.graph` file compatibility tests (native side).
// These tests use Minigraf::open() and the file system, which are not available
// on wasm32 targets. wasm-pack compiles integration tests too, so gate the
// entire file.
#![cfg(not(target_arch = "wasm32"))]

//!
//!
//! Verifies that raw page bytes produced by the native storage layer are
//! self-consistent (mimicking what `BrowserDb::export_graph` / `import_graph`
//! do) and that the committed binary fixture is readable by `Minigraf::open`.
//!
//! The companion browser side lives in `src/browser/mod.rs` under
//! `#[wasm_bindgen_test]` and loads the same fixture via `include_bytes!`.

use minigraf::{Minigraf, QueryResult};
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("minigraf_compat_{tag}.graph"))
}

fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("graph.wal"));
}

/// Produce a `.graph` file, read its raw bytes, write them to a second file,
/// open the second file — verifying that byte-for-byte copies are readable.
///
/// This mirrors what `BrowserDb::export_graph()` + `BrowserDb::import_graph()`
/// do: the export concatenates raw 4 KB pages; the import splits them back and
/// loads via a fresh `PersistentFactStorage`.
#[test]
fn native_raw_page_bytes_round_trip() {
    let src = tmp("src");
    let dst = tmp("dst");
    cleanup(&src);
    cleanup(&dst);

    // Produce a populated, checkpointed .graph file.
    {
        let db = Minigraf::open(&src).expect("open src");
        db.execute(r#"(transact [[:alice :name "Alice"]])"#)
            .expect("transact name");
        db.execute("(transact [[:alice :age 30]])")
            .expect("transact age");
        db.checkpoint().expect("checkpoint");
    }

    // Copy raw bytes to a new path (simulates export → import across the boundary).
    let bytes = std::fs::read(&src).expect("read src");
    std::fs::write(&dst, &bytes).expect("write dst");

    // Open the byte-copy and verify both facts survive.
    let db2 = Minigraf::open(&dst).expect("open dst");

    let r = db2
        .execute("(query [:find ?name :where [?e :name ?name]])")
        .expect("query name");
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected 1 name result");
        }
        _ => panic!("expected QueryResults for name query"),
    }

    let r2 = db2
        .execute("(query [:find ?age :where [?e :age ?age]])")
        .expect("query age");
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected 1 age result");
        }
        _ => panic!("expected QueryResults for age query"),
    }

    cleanup(&src);
    cleanup(&dst);
}

/// Load the committed fixture (produced by `cargo run --example generate_compat_fixture`)
/// via `Minigraf::open` and assert that the known facts are present.
///
/// The fixture is a frozen v7 file written by Minigraf v2.0.0; opening it
/// here exercises v7→v8 migration (#371, #287). It is also loaded by the
/// browser WASM tests via `include_bytes!` + `BrowserDb::import_graph`,
/// completing the cross-boundary coverage.
#[test]
fn fixture_readable_by_native() {
    let fixture: &[u8] = include_bytes!("fixtures/compat.graph");

    // The whole point of this fixture is exercising v7->v8 migration on open;
    // catch a silent regeneration (e.g. someone running
    // `cargo run --example generate_compat_fixture` against the old output
    // path and overwriting the frozen fixture) before it opens successfully
    // for the wrong reason.
    let embedded_version = u32::from_le_bytes(fixture[4..8].try_into().unwrap());
    assert_eq!(
        embedded_version, 7,
        "compat.graph fixture must remain the frozen v7 file"
    );

    // Write to a temp path so Minigraf::open can use it.
    let path = tmp("fixture");
    cleanup(&path);
    std::fs::write(&path, fixture).expect("write fixture");

    let db = Minigraf::open(&path).expect("open fixture");

    let r = db
        .execute("(query [:find ?name :where [?e :name ?name]])")
        .expect("query name");
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected 1 name result from fixture");
        }
        _ => panic!("expected QueryResults for name query"),
    }

    let r2 = db
        .execute("(query [:find ?age :where [?e :age ?age]])")
        .expect("query age");
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected 1 age result from fixture");
        }
        _ => panic!("expected QueryResults for age query"),
    }

    drop(db);
    let raw = std::fs::read(&path).expect("read migrated fixture");
    let version = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    assert_eq!(version, 8, "v7 fixture must be upgraded to v8 on open");

    cleanup(&path);
}
