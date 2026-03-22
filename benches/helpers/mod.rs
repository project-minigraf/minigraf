//! Shared benchmark fixtures.
//!
//! Data model:
//! - Value facts:  `:e{i} :val {i}` for i in 0..n
//! - Chain facts:  `:e{i} :next :e{i+1}` for i in 0..n-1  (only in chain/join fixtures)
//!
//! Builder chain for file-backed DBs: `OpenOptions::new().page_cache_size(256).path(p).open()`
//! — `page_cache_size()` must precede `path()` due to type-state design.

use minigraf::{Minigraf, OpenOptions};
use std::sync::Arc;

// ── Value-only fixture ────────────────────────────────────────────────────────

/// In-memory DB with `n` value facts: `:e{i} :val {i}` for i in 0..n.
/// Inserted in batches of 100.
pub fn populate_in_memory(n: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    Arc::new(db)
}

/// File-backed DB with `n` value facts, fully checkpointed (no WAL sidecar).
pub fn populate_file(n: usize, path: &str) {
    let db = OpenOptions::new().page_cache_size(256).path(path).open().unwrap();
    insert_val_facts(&db, n);
    db.checkpoint().unwrap();
}

/// File-backed DB with `n` value facts, WAL NOT checkpointed.
/// Uses `wal_checkpoint_threshold: usize::MAX` to suppress auto-checkpoint.
/// Facts are committed (WAL-written) but not flushed to packed pages.
pub fn populate_file_no_checkpoint(n: usize, path: &str) {
    let db = OpenOptions {
        wal_checkpoint_threshold: usize::MAX,
        ..Default::default()
    }
    .path(path)
    .open()
    .unwrap();
    insert_val_facts(&db, n);
    // Do NOT checkpoint — WAL entries remain pending.
}

/// Open an existing file-backed DB with auto-checkpoint suppressed.
/// Used by insert_file and concurrent_file groups so WAL fsyncs are
/// not interrupted by checkpoint spikes during measurement.
pub fn open_file_no_checkpoint(path: &str) -> Arc<Minigraf> {
    let db = OpenOptions {
        wal_checkpoint_threshold: usize::MAX,
        ..Default::default()
    }
    .path(path)
    .open()
    .unwrap();
    Arc::new(db)
}

// ── Graph fixtures ────────────────────────────────────────────────────────────

/// In-memory DB containing a linear chain of `depth` nodes plus transitive-closure rules.
///
/// Nodes: `:n0 :next :n1`, `:n1 :next :n2`, …, `:n{depth-1} :next :n{depth}`.
/// Rules: `(reach ?from ?to)` — transitive closure over `:next`.
/// Query: `(query [:find ?to :where (reach :n0 ?to)])` returns all reachable nodes.
pub fn chain_graph(depth: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    // Insert edges in batches of 200
    for chunk_start in (0..depth).step_by(200) {
        let chunk_end = (chunk_start + 200).min(depth);
        let mut cmd = String::from("(transact [");
        for i in chunk_start..chunk_end {
            cmd.push_str(&format!("[:n{} :next :n{}]", i, i + 1));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    db.execute("(rule [(reach ?from ?to) [?from :next ?to]])").unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])").unwrap();
    Arc::new(db)
}

/// In-memory DB containing a fan-out tree plus transitive-closure rules.
///
/// Each node at depth d has `width` children. Root is `:n0`.
/// Rules: same `(reach ?from ?to)` transitive closure.
pub fn fanout_graph(width: usize, depth: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut current_level = vec![0usize];
    let mut next_id = 1usize;
    for _ in 0..depth {
        let mut next_level = Vec::new();
        for &parent in &current_level {
            for _ in 0..width {
                edges.push((parent, next_id));
                next_level.push(next_id);
                next_id += 1;
            }
        }
        current_level = next_level;
    }
    for chunk in edges.chunks(200) {
        let mut cmd = String::from("(transact [");
        for (from, to) in chunk {
            cmd.push_str(&format!("[:n{} :next :n{}]", from, to));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    db.execute("(rule [(reach ?from ?to) [?from :next ?to]])").unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])").unwrap();
    Arc::new(db)
}

/// In-memory DB with n value facts AND n-1 chain reference facts.
/// Total ≈ 2n facts. Used for join_3pattern benchmarks.
///
/// Schema:
///   `:e{i} :val {i}` (value fact)
///   `:e{i} :next :e{i+1}` (chain ref, for i < n-1)
/// Query: `(query [:find ?v :where [:e0 :next ?m] [?m :next ?end] [?end :val ?v]])`
pub fn populate_for_join(n: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    for batch_start in (0..n).step_by(50) {
        let batch_end = (batch_start + 50).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :val {}]", i, i));
            if i + 1 < n {
                cmd.push_str(&format!("[:e{} :next :e{}]", i, i + 1));
            }
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    Arc::new(db)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn insert_val_facts(db: &Minigraf, n: usize) {
    const BATCH: usize = 100;
    for batch_start in (0..n).step_by(BATCH) {
        let batch_end = (batch_start + BATCH).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :val {}]", i, i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
}

/// Human-readable scale label: 1_000 → "1k", 10_000 → "10k", etc.
pub fn scale_label(n: usize) -> &'static str {
    match n {
        1_000 => "1k",
        10_000 => "10k",
        100_000 => "100k",
        1_000_000 => "1m",
        _ => "?",
    }
}
