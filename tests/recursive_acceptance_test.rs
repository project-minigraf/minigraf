//! Acceptance tests for recursive rules on the streaming engine (#473, #548
//! spec §7): a rule deriving about 5M facts completes, and its peak memory
//! follows the derived relation; the default `max_derived_facts` stops the
//! same rule with `INT-020`.
//!
//! The graph has 63 layers of 50 nodes with complete bipartite edges between
//! neighbouring layers (155,000 facts); `reach` derives 4,882,500 facts in 62
//! rounds. Nonlinear closure and three-stratum negation also run at 256/512
//! nodes with explicit RSS, time-scaling and derived-limit assertions.
//! Ignored by default; `.github/workflows/smoke.yml` runs it nightly:
//!
//! ```text
//! cargo test --profile bench --test recursive_acceptance_test -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Peak RSS comes from `/proc/self/status` (Linux only): the high-water mark
//! is reset just before the query; tests run serially to isolate the measurement.
//! Linux fails if RSS cannot be measured; other platforms print a skip.
//
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

mod acceptance;

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
    acceptance::check_rss(
        "reach_5m",
        reset,
        before_kb,
        peak_kb,
        DERIVED * MAX_BYTES_PER_DERIVED,
    );
    drop(db);

    let dir = tempfile::tempdir().unwrap();
    let db = layered(&dir, OpenOptions::new());
    let err = db
        .execute("(query [:find ?x ?y :where (reach ?x ?y)])")
        .err()
        .expect("over the default max_derived_facts");
    assert_eq!(err.code().to_string(), "INT-020", "stopped by the limit");
}

/// Both sizes must meet a retained-relation bound and a coarse time-scaling
/// guard. Doubling a chain quadruples its closure; nonlinear self-joins can
/// do cubic work. The 16x + 1s guard tolerates CI noise without accepting an
/// unbounded blow-up. Absolute per-query budget: 60 seconds.
fn closure_scaling(multistratum: bool) {
    let mut small_time = None;
    for nodes in [256usize, 512] {
        let pairs = nodes * (nodes - 1) / 2;
        // reach + allowed + eligible (the latter two partition reach).
        let derived = if multistratum { 2 * pairs + 1 } else { pairs };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closure.graph");
        let db = OpenOptions::new()
            .max_derived_facts(derived + 1_000)
            .path(&path)
            .open()
            .unwrap();
        let facts: String = (0..nodes - 1)
            .map(|i| format!("[:n{i} :edge :n{}]", i + 1))
            .collect();
        db.execute(&format!(
            "(transact [{facts} [:n{} :blocked true]])",
            nodes - 1
        ))
        .unwrap();
        db.checkpoint().unwrap();
        let rules: &[&str] = if multistratum {
            &[
                "(rule [(reach ?x ?y) [?x :edge ?y]])",
                "(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])",
                "(rule [(blocked ?x) [?x :blocked true]])",
                "(rule [(allowed ?x ?y) (reach ?x ?y) (not (blocked ?y))])",
                "(rule [(eligible ?x ?y) (reach ?x ?y) (not (allowed ?x ?y))])",
            ]
        } else {
            &[
                "(rule [(reach ?x ?y) [?x :edge ?y]])",
                "(rule [(reach ?x ?z) (reach ?x ?y) (reach ?y ?z)])",
            ]
        };
        for rule in rules {
            db.execute(rule).unwrap();
        }
        let query = if multistratum {
            "(query [:find ?x ?y :where (eligible ?x ?y)])"
        } else {
            "(query [:find ?x ?y :where (reach ?x ?y)])"
        };
        let label = format!(
            "{}_{nodes}",
            if multistratum {
                "multistratum"
            } else {
                "nonlinear"
            }
        );
        let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
        let before_kb = proc_status_kb("VmRSS");
        let start = std::time::Instant::now();
        let mut cursor = db.query(query).unwrap();
        let mut rows = 0;
        while let Some(batch) = cursor.next_batch(1_000).unwrap() {
            rows += batch.len();
        }
        drop(cursor);
        let elapsed = start.elapsed();
        let peak_kb = proc_status_kb("VmHWM");
        let expected = if multistratum { nodes - 1 } else { pairs };
        assert_eq!(
            rows, expected,
            "complete closure or paths to the blocked terminal"
        );
        acceptance::check_rss(
            &label,
            reset,
            before_kb,
            peak_kb,
            (16 << 20) + 512 * derived,
        );
        assert!(
            elapsed < std::time::Duration::from_secs(60),
            "absolute recursive query budget"
        );
        if let Some(small) = small_time {
            assert!(
                elapsed <= small * 16 + std::time::Duration::from_secs(1),
                "doubling the chain stays within the scaling bound"
            );
        } else {
            small_time = Some(elapsed);
        }
        eprintln!(
            "{label}: nodes={nodes} edges={} closure_pairs={pairs} derived_bound={derived} rows={rows} elapsed={elapsed:?}",
            nodes - 1
        );
        drop(db);

        // The same shape must actually exhaust a finite derived-fact cap.
        let db = OpenOptions::new()
            .max_derived_facts(1_000)
            .path(&path)
            .open()
            .unwrap();
        for rule in rules {
            db.execute(rule).unwrap();
        }
        let err = db
            .execute(query)
            .err()
            .expect("derived relation exceeds the cap");
        assert_eq!(err.code(), "INT-020");
        eprintln!("{label}: derived_cap=1000 error=INT-020");
    }
}

#[test]
#[ignore]
fn nonlinear_closure_scaling() {
    closure_scaling(false);
}

#[test]
#[ignore]
fn multistratum_recursion_scaling() {
    closure_scaling(true);
}
