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
    let db = OpenOptions::new()
        .page_cache_size(256)
        .path(path)
        .open()
        .unwrap();
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
    db.execute("(rule [(reach ?from ?to) [?from :next ?to]])")
        .unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])")
        .unwrap();
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
    db.execute("(rule [(reach ?from ?to) [?from :next ?to]])")
        .unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])")
        .unwrap();
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

// ── Negation fixtures ─────────────────────────────────────────────────────────

/// In-memory DB with `n` value facts plus `excluded` banned entities.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :banned true` for i in 0..excluded  (first `excluded` entities are banned)
///
/// Used for `not` benchmarks:
///   `(query [:find ?e :where [?e :val ?v] (not [?e :banned true])])`
pub fn populate_with_not_exclusion(n: usize, excluded: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    // Insert banned markers in batches of 100
    for batch_start in (0..excluded).step_by(100) {
        let batch_end = (batch_start + 100).min(excluded);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :banned true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    Arc::new(db)
}

/// In-memory DB with `n` value facts plus `excluded` entities that have a
/// dependency on a "bad" dependency entity.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :dep :d-bad` for i in 0..excluded
///   `:d-bad :status :bad`
///
/// Used for `not-join` benchmarks:
///   `(query [:find ?e :where [?e :val ?v] (not-join [?e] [?e :dep ?d] [?d :status :bad])])`
pub fn populate_with_not_join_exclusion(n: usize, excluded: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    // Mark the "bad" dependency
    db.execute("(transact [[:d-bad :status :bad]])").unwrap();
    // Insert dep edges in batches of 100
    for batch_start in (0..excluded).step_by(100) {
        let batch_end = (batch_start + 100).min(excluded);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :dep :d-bad]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    Arc::new(db)
}

/// In-memory DB with `n` value facts, `excluded` blocked entities, and a
/// `(eligible ?x)` rule that uses `not` to exclude blocked entities.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :blocked true` for i in 0..excluded
/// Rule:
///   `(eligible ?x) :- [?x :val ?v] (not [?x :blocked true])`
///
/// Used for rule-body negation benchmarks:
///   `(query [:find ?e :where (eligible ?e)])`
pub fn populate_with_not_rule(n: usize, excluded: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    for batch_start in (0..excluded).step_by(100) {
        let batch_end = (batch_start + 100).min(excluded);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :blocked true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    db.execute("(rule [(eligible ?x) [?x :val ?v] (not [?x :blocked true])])")
        .unwrap();
    Arc::new(db)
}

// ── Disjunction fixtures ──────────────────────────────────────────────────────

/// In-memory DB with `n` value facts plus `a_count` entities with `:tag-a` and
/// `b_count` entities with `:tag-b` (counts from opposite ends; some may overlap).
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :tag-a true` for i in 0..a_count
///   `:e{i} :tag-b true` for i in (n-b_count)..n
///
/// Used for `or` benchmarks:
///   `(query [:find ?e :where [?e :val ?v] (or [?e :tag-a true] [?e :tag-b true])])`
pub fn populate_with_or_tags(n: usize, a_count: usize, b_count: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    // Insert :tag-a for first a_count entities
    for batch_start in (0..a_count).step_by(100) {
        let batch_end = (batch_start + 100).min(a_count);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :tag-a true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    // Insert :tag-b for last b_count entities
    let b_start = n.saturating_sub(b_count);
    for batch_start in (b_start..n).step_by(100) {
        let batch_end = (batch_start + 100).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :tag-b true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    Arc::new(db)
}

/// In-memory DB with `n` value facts and a `(tagged ?x)` rule using `or` in its body.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :tag-a true` for i in 0..n/2
///   `:e{i} :tag-b true` for i in n/2..n
/// Rule:
///   `(tagged ?x) :- (or [?x :tag-a true] [?x :tag-b true])`
///
/// Used for rule-body `or` benchmarks:
///   `(query [:find ?e :where (tagged ?e)])`
pub fn populate_with_or_rule(n: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    insert_val_facts(&db, n);
    let half = n / 2;
    for batch_start in (0..half).step_by(100) {
        let batch_end = (batch_start + 100).min(half);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :tag-a true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    for batch_start in (half..n).step_by(100) {
        let batch_end = (batch_start + 100).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :tag-b true]", i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    db.execute("(rule [(tagged ?x) (or [?x :tag-a true] [?x :tag-b true])])")
        .unwrap();
    Arc::new(db)
}

// ── Aggregation fixtures ──────────────────────────────────────────────────────

/// In-memory DB with `n` value facts, each entity having a `:dept` keyword cycling
/// over `dept_count` departments.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :dept :d{i % dept_count}` for i in 0..n
///
/// Used for aggregation benchmarks, e.g.:
///   `(query [:find (count ?e) :where [?e :val ?v]])`
///   `(query [:find ?dept (count ?e) :where [?e :dept ?dept]])`
pub fn populate_with_dept(n: usize, dept_count: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    for batch_start in (0..n).step_by(50) {
        let batch_end = (batch_start + 50).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :val {}]", i, i));
            cmd.push_str(&format!("[:e{} :dept :d{}]", i, i % dept_count));
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
