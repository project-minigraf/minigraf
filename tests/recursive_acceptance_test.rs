//! Acceptance test for recursive rules on the streaming engine (#473, #548
//! spec §7): a rule deriving about 5M facts completes, and its peak memory
//! follows the derived relation; the default `max_derived_facts` stops the
//! same rule with `INT-020`.
//!
//! The graph has 63 layers of 50 nodes with complete bipartite edges between
//! neighbouring layers (155,000 facts); `reach` derives 4,882,500 facts in 62
//! rounds. Ignored by default; `.github/workflows/smoke.yml` runs it nightly:
//!
//! ```text
//! cargo test --profile bench --test recursive_acceptance_test -- --ignored --nocapture
//! ```
//!
//! Peak RSS comes from `/proc/self/status` (Linux only): the high-water mark
//! is reset just before the query, and this file has one test so nothing else
//! runs while it is measured.
//
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, OpenOptions};

const LAYERS: usize = 63;
const WIDTH: usize = 50;
/// Every node reaches every node of every later layer.
const DERIVED: usize = WIDTH * WIDTH * (LAYERS - 1) * LAYERS / 2;
/// Peak RSS the query may add per derived fact: the relation's two id cells
/// per row, its identity set and entity index (about 165 bytes measured on
/// Linux, 2026-10-21), with headroom for allocators.
const MAX_BYTES_PER_DERIVED: usize = 256;

/// A file-backed database holding the layered graph, checkpointed.
fn layered(dir: &tempfile::TempDir, opts: OpenOptions) -> Minigraf {
    let db = opts.path(dir.path().join("layers.graph")).open().unwrap();
    for layer in 0..LAYERS - 1 {
        let mut cmd = String::from("(transact [");
        for a in 0..WIDTH {
            for b in 0..WIDTH {
                cmd.push_str(&format!("[:n{layer}x{a} :edge :n{}x{b}]", layer + 1));
            }
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    db.checkpoint().unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :edge ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])")
        .unwrap();
    db
}

/// A `kB` field of `/proc/self/status`; 0 where it does not exist.
fn proc_status_kb(field: &str) -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(field)?.strip_prefix(':').map(str::to_owned))
        })
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

#[test]
#[ignore]
fn reach_5m() {
    let dir = tempfile::tempdir().unwrap();
    let db = layered(&dir, OpenOptions::new().max_derived_facts(DERIVED + 1_000));
    // Reset the high-water mark to the current RSS, so the peak read after
    // the query is the query's own (Linux 4.0+; elsewhere nothing is checked).
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let before_kb = proc_status_kb("VmRSS");

    let start = std::time::Instant::now();
    let mut cursor = db
        .query("(query [:find ?x ?y :where (reach ?x ?y)])")
        .unwrap();
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(1_000).unwrap() {
        rows += batch.len();
    }
    drop(cursor);
    let peak_kb = proc_status_kb("VmHWM");
    let added = peak_kb.saturating_sub(before_kb) * 1024;
    eprintln!(
        "reach_5m: {rows} rows in {:.2?}; peak RSS {} MiB, {} MiB before the query, {} bytes per derived fact",
        start.elapsed(),
        peak_kb / 1024,
        before_kb / 1024,
        added / DERIVED
    );

    assert_eq!(rows, DERIVED, "every reach fact returned");
    if reset && peak_kb > 0 {
        assert!(
            added <= DERIVED * MAX_BYTES_PER_DERIVED,
            "peak RSS follows the derived relation"
        );
    }
    drop(db);

    let dir = tempfile::tempdir().unwrap();
    let db = layered(&dir, OpenOptions::new());
    let err = db
        .execute("(query [:find ?x ?y :where (reach ?x ?y)])")
        .err()
        .expect("over the default max_derived_facts");
    assert_eq!(err.code().to_string(), "INT-020", "stopped by the limit");
}
