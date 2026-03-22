# Phase 6.3 Benchmarks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Criterion 0.5 benchmark suite covering 9 groups (insert, insert_file, query, time_travel, recursion, open, checkpoint, concurrent, concurrent_file) and populate `README.md` with measured numbers.

**Architecture:** Single bench entry point (`benches/minigraf_bench.rs`) with a shared helpers module (`benches/helpers/mod.rs`). Each benchmark group is a standalone `fn bench_*(c: &mut Criterion)` registered via `criterion_group!`. The helpers module owns all DB fixture creation so benchmark functions stay focused on measurement.

**Tech Stack:** `criterion = "0.5"` (html_reports feature), `tempfile = "3"` (already present), `std::thread` + `std::sync::Barrier` for concurrent groups.

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `Cargo.toml` | Modify | Add `criterion` dev-dep + `[[bench]]` entry |
| `benches/minigraf_bench.rs` | Create | All 9 benchmark groups + criterion_group!/main! |
| `benches/helpers/mod.rs` | Create | Shared DB fixtures used across all groups |
| `README.md` | Modify (last task) | Add `## Performance` section with measured numbers |

---

## Data Model Used in Benchmarks

All helpers insert facts using two schemas:

**Value facts** (used by insert, insert_file, query, time_travel):
- Entity `:e{i}`, attribute `:val`, value `i` (integer)
- Query: `(query [:find ?v :where [:e0 :val ?v]])`

**Chain facts** (added on top of value facts for join benchmarks):
- Entity `:e{i}`, attribute `:next`, value `:e{i+1}` (keyword ref)
- Join query: `(query [:find ?v :where [:e0 :next ?m] [?m :next ?end] [?end :val ?v]])`

---

## Task 1: Add Criterion to Cargo.toml + create bench skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `benches/minigraf_bench.rs`
- Create: `benches/helpers/mod.rs` (stub)

- [ ] **Step 1: Add criterion dev-dep and bench entry to Cargo.toml**

In `[dev-dependencies]` section, add:
```toml
criterion = { version = "0.5", features = ["html_reports"] }
```

Add after existing `[dev-dependencies]`:
```toml
[[bench]]
name = "minigraf_bench"
harness = false
```

- [ ] **Step 2: Create benches/helpers/mod.rs stub**

```rust
// Shared benchmark fixtures. Included via `mod helpers;` in minigraf_bench.rs.
use minigraf::{Minigraf, OpenOptions};
use std::sync::Arc;

/// Placeholder — populated in Task 2.
pub fn populate_in_memory(_n: usize) -> Arc<Minigraf> {
    Arc::new(Minigraf::in_memory().unwrap())
}
```

- [ ] **Step 3: Create benches/minigraf_bench.rs skeleton**

```rust
mod helpers;

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert/single_fact");
    group.bench_function("stub", |b| b.iter(|| 1 + 1));
    group.finish();
}

criterion_group!(benches, bench_insert);
criterion_main!(benches);
```

- [ ] **Step 4: Verify the skeleton compiles and runs**

```bash
cargo bench --bench minigraf_bench -- bench_insert
```

Expected: Criterion output with one benchmark result for `insert/single_fact/stub`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml benches/
git commit -m "chore(bench): add criterion scaffold for Phase 6.3"
```

---

## Task 2: Implement helpers/mod.rs

**Files:**
- Create: `benches/helpers/mod.rs`

- [ ] **Step 1: Write the complete helpers module**

Replace `benches/helpers/mod.rs` with:

```rust
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
///   `:e{i} :next :e{i+1}` (chain ref, for i < n)
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
```

- [ ] **Step 2: Update minigraf_bench.rs to use helpers**

Replace the bench file with a version that imports the full helpers module and adds a smoke-test benchmark using `populate_in_memory`:

```rust
mod helpers;

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert/single_fact");
    group.bench_with_input(BenchmarkId::from_parameter("smoke"), &10usize, |b, &n| {
        let db = helpers::populate_in_memory(n);
        b.iter(|| db.execute("(transact [[:esmoke :val 0]])").unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_insert);
criterion_main!(benches);
```

- [ ] **Step 3: Verify helpers compile and fixture works**

```bash
cargo bench --bench minigraf_bench -- bench_insert
```

Expected: Criterion runs without panics and reports a time for `insert/single_fact/smoke`.

- [ ] **Step 4: Commit**

```bash
git add benches/helpers/mod.rs benches/minigraf_bench.rs
git commit -m "chore(bench): implement helpers fixtures (populate, chain, fanout)"
```

---

## Task 3: `insert/` benchmark group

Measures in-memory steady-state write cost. Each iteration gets a freshly-populated DB via `iter_batched`.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Replace bench_insert with full insert group**

```rust
fn bench_insert(c: &mut Criterion) {
    use criterion::BatchSize;
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("100k", 100_000)];

    // single_fact: insert one fact into a pre-populated in-memory DB
    {
        let mut group = c.benchmark_group("insert/single_fact");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                b.iter_batched(
                    || helpers::populate_in_memory(n),
                    |db| { db.execute("(transact [[:ebench :val 0]])").unwrap(); },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }

    // batch_100: insert 100 facts in a single transact
    {
        let mut group = c.benchmark_group("insert/batch_100");
        let batch_cmd: String = {
            let mut s = String::from("(transact [");
            for i in 0..100 { s.push_str(&format!("[:eb{} :val {}]", i, i)); }
            s.push(']'); s.push(')'); s
        };
        for &(label, n) in SCALES {
            let cmd = batch_cmd.clone();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let cmd = cmd.clone();
                b.iter_batched(
                    || helpers::populate_in_memory(n),
                    move |db| { db.execute(&cmd).unwrap(); },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }

    // explicit_tx: single fact via begin_write()/commit()
    {
        let mut group = c.benchmark_group("insert/explicit_tx");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                b.iter_batched(
                    || helpers::populate_in_memory(n),
                    |db| {
                        let mut tx = db.begin_write().unwrap();
                        tx.execute("(transact [[:ebench :val 0]])").unwrap();
                        tx.commit().unwrap();
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }
}
```

Register in criterion_group: `criterion_group!(benches, bench_insert);`

- [ ] **Step 2: Run insert group**

```bash
cargo bench --bench minigraf_bench -- "insert/"
```

Expected: 9 benchmarks (3 scenarios × 3 scales) complete without panics. Times roughly in µs range.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add insert/ benchmark group (in-memory write throughput)"
```

---

## Task 4: `insert_file/` benchmark group

Measures file-backed write throughput including WAL fsync. Uses `b.iter()` with a single accumulating DB (state mutates across iterations — acceptable and realistic).

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_insert_file function**

```rust
fn bench_insert_file(c: &mut Criterion) {
    use tempfile::NamedTempFile;
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("100k", 100_000)];

    // single_fact: one execute() per iter against growing file-backed DB
    {
        let mut group = c.benchmark_group("insert_file/single_fact");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| db.execute("(transact [[:ebench :val 0]])").unwrap());
                drop(tmp); // explicit: keep file alive for entire bench duration
            });
        }
        group.finish();
    }

    // batch_100: 100 facts per execute()
    {
        let mut group = c.benchmark_group("insert_file/batch_100");
        let batch_cmd: String = {
            let mut s = String::from("(transact [");
            for i in 0..100 { s.push_str(&format!("[:eb{} :val {}]", i, i)); }
            s.push(']'); s.push(')'); s
        };
        for &(label, n) in SCALES {
            let cmd = batch_cmd.clone();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| db.execute(&cmd).unwrap());
                drop(tmp);
            });
        }
        group.finish();
    }

    // explicit_tx: begin_write()/commit() per iter
    {
        let mut group = c.benchmark_group("insert_file/explicit_tx");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| {
                    let mut tx = db.begin_write().unwrap();
                    tx.execute("(transact [[:ebench :val 0]])").unwrap();
                    tx.commit().unwrap();
                });
                drop(tmp);
            });
        }
        group.finish();
    }
}
```

Add `bench_insert_file` to `criterion_group!`.

- [ ] **Step 2: Run insert_file group**

```bash
cargo bench --bench minigraf_bench -- "insert_file/"
```

Expected: 9 benchmarks complete. Times noticeably higher than `insert/` (WAL fsync overhead).

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add insert_file/ benchmark group (file-backed write + WAL fsync)"
```

---

## Task 5: `query/` benchmark group

Measures read latency. DB created once per scale outside the iter loop.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_query function**

```rust
fn bench_query(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[
        ("1k", 1_000), ("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000),
    ];

    // point_entity: EAVT range scan on a known entity
    {
        let mut group = c.benchmark_group("query/point_entity");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap());
            });
        }
        group.finish();
    }

    // point_attribute: AEVT scan — all entities with :val attribute
    {
        let mut group = c.benchmark_group("query/point_attribute");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute("(query [:find ?e :where [?e :val _]])").unwrap());
            });
        }
        group.finish();
    }

    // join_3pattern: 3-clause join across two :next hops
    // Uses populate_for_join which inserts both :val and :next facts.
    // Query: e0 -> e1 -> e2, return e2's :val
    {
        let mut group = c.benchmark_group("query/join_3pattern");
        for &(label, n) in SCALES {
            let db = helpers::populate_for_join(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?v :where [:e0 :next ?m] [?m :next ?end] [?end :val ?v]])"
                    ).unwrap()
                });
            });
        }
        group.finish();
    }
}
```

Add `bench_query` to `criterion_group!`.

- [ ] **Step 2: Run query group**

```bash
cargo bench --bench minigraf_bench -- "query/"
```

Expected: 12 benchmarks (3 scenarios × 4 scales). `point_entity` should be flat or near-flat across scales (EAVT index). `point_attribute` returns all N entities — expect time to scale with N. `join_3pattern` should be fast (indexed hop).

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add query/ benchmark group (point lookup + 3-pattern join)"
```

---

## Task 6: `time_travel/` benchmark group

Measures temporal filter overhead on top of base query cost. Uses the same `populate_in_memory` DB. Queries use `:as-of` and `:valid-at` with values that pass all facts (worst-case filter overhead).

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_time_travel function**

```rust
fn bench_time_travel(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[
        ("1k", 1_000), ("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000),
    ];

    // as_of_counter: :as-of with a large tx counter (all facts pass)
    // tx count after N facts inserted in batches of 100 is N/100.
    // Using 999999 ensures all real tx counts are ≤ this.
    {
        let mut group = c.benchmark_group("time_travel/as_of_counter");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?v :as-of 999999 :where [:e0 :val ?v]])"
                    ).unwrap()
                });
            });
        }
        group.finish();
    }

    // valid_at: :valid-at with a far-future timestamp (all facts valid)
    // Facts inserted without explicit valid-from default to tx time (~2026);
    // valid-to defaults to MAX (forever). "2099-01-01T00:00:00Z" is within that window.
    {
        let mut group = c.benchmark_group("time_travel/valid_at");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        r#"(query [:find ?v :valid-at "2099-01-01T00:00:00Z" :where [:e0 :val ?v]])"#
                    ).unwrap()
                });
            });
        }
        group.finish();
    }
}
```

Add `bench_time_travel` to `criterion_group!`.

- [ ] **Step 2: Run time_travel group**

```bash
cargo bench --bench minigraf_bench -- "time_travel/"
```

Expected: 8 benchmarks (2 scenarios × 4 scales). Compare with `query/point_entity` at each scale — the delta is the temporal filter overhead.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add time_travel/ benchmark group (:as-of and :valid-at overhead)"
```

---

## Task 7: `recursion/` benchmark group

Measures semi-naive fixed-point iteration cost for transitive closure queries.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_recursion function**

```rust
fn bench_recursion(c: &mut Criterion) {
    // chain: linear chain of depth N — worst case for iteration depth
    {
        let mut group = c.benchmark_group("recursion/chain");
        for &(label, depth) in &[("depth_10", 10usize), ("depth_100", 100), ("depth_1k", 1_000)] {
            let db = helpers::chain_graph(depth);
            group.bench_with_input(BenchmarkId::from_parameter(label), &depth, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?to :where (reach :n0 ?to)])").unwrap()
                });
            });
        }
        group.finish();
    }

    // fanout: fan-out tree — tests delta size per semi-naive iteration
    {
        let mut group = c.benchmark_group("recursion/fanout");
        // (width, depth) pairs: (5,5) ≈ 3905 nodes; (10,3) ≈ 1110 nodes
        for &(label, width, depth) in &[
            ("w5_d5", 5usize, 5usize),
            ("w10_d3", 10usize, 3usize),
        ] {
            let db = helpers::fanout_graph(width, depth);
            group.bench_with_input(BenchmarkId::from_parameter(label), &(width, depth), |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?to :where (reach :n0 ?to)])").unwrap()
                });
            });
        }
        group.finish();
    }
}
```

Add `bench_recursion` to `criterion_group!`.

- [ ] **Step 2: Run recursion group**

```bash
cargo bench --bench minigraf_bench -- "recursion/"
```

Expected: 5 benchmarks. Chain times should scale roughly linearly with depth. Fanout times depend on delta size per iteration.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add recursion/ benchmark group (transitive closure chain + fanout)"
```

---

## Task 8: `open/` benchmark group

Measures DB open time: clean open (no WAL) and crash-recovery open (WAL replay).

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_open function**

```rust
fn bench_open(c: &mut Criterion) {
    use tempfile::NamedTempFile;

    // checkpointed: open a fully-checkpointed .graph file (no WAL replay)
    {
        let mut group = c.benchmark_group("open/checkpointed");
        for &(label, n) in &[
            ("1k", 1_000usize), ("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000),
        ] {
            // Create the pre-populated file ONCE (outside iter loop).
            let tmp = NamedTempFile::new().unwrap();
            let path = tmp.path().to_str().unwrap().to_string();
            helpers::populate_file(n, &path);
            // Checkpoint is already done by populate_file; WAL sidecar absent.

            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                let path = path.clone();
                b.iter(|| {
                    // Open and immediately drop — measures full open time.
                    let _db = OpenOptions::new().page_cache_size(256).path(&path).open().unwrap();
                });
            });
            drop(tmp);
        }
        group.finish();
    }

    // wal_replay: open with N WAL entries pending (crash-recovery path)
    {
        let mut group = c.benchmark_group("open/wal_replay");
        for &(label, n) in &[("1k", 1_000usize), ("10k", 10_000)] {
            let tmp = NamedTempFile::new().unwrap();
            let path = tmp.path().to_str().unwrap().to_string();
            // populate_file_no_checkpoint leaves all facts in the WAL (not checkpointed).
            helpers::populate_file_no_checkpoint(n, &path);

            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                let path = path.clone();
                b.iter(|| {
                    // Each open replays the WAL. WAL is NOT consumed (no checkpoint during bench).
                    let _db = OpenOptions::new().page_cache_size(256).path(&path).open().unwrap();
                });
            });
            drop(tmp);
        }
        group.finish();
    }
}
```

Add `use minigraf::OpenOptions;` at the top of the bench file.

Add `bench_open` to `criterion_group!`.

- [ ] **Step 2: Run open group**

```bash
cargo bench --bench minigraf_bench -- "open/"
```

Expected: 6 benchmarks. `wal_replay` should be slower than `checkpointed` at same scale. `checkpointed/1m` will take a few seconds to set up.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add open/ benchmark group (cold open + WAL replay)"
```

---

## Task 9: `checkpoint/` benchmark group

Measures WAL flush cost as a function of pending WAL entries. Setup opens DB with `wal_checkpoint_threshold: usize::MAX`, inserts N facts (all stay in WAL), then times `db.checkpoint()`.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_checkpoint function**

```rust
fn bench_checkpoint(c: &mut Criterion) {
    use criterion::BatchSize;
    use tempfile::NamedTempFile;

    let mut group = c.benchmark_group("checkpoint");
    for &(label, n) in &[("1k", 1_000usize), ("10k", 10_000)] {
        group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Setup: file DB with n WAL-committed facts, no checkpoint yet.
                    let tmp = NamedTempFile::new().unwrap();
                    let path = tmp.path().to_str().unwrap().to_string();
                    helpers::populate_file_no_checkpoint(n, &path);
                    // Re-open to get a fresh handle (populate drops its handle).
                    let db = helpers::open_file_no_checkpoint(&path);
                    (db, tmp) // keep tmp alive
                },
                |(db, _tmp)| {
                    // Routine: flush WAL to packed pages, delete WAL sidecar.
                    db.checkpoint().unwrap();
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}
```

Add `bench_checkpoint` to `criterion_group!`.

- [ ] **Step 2: Run checkpoint group**

```bash
cargo bench --bench minigraf_bench -- "checkpoint/"
```

Expected: 2 benchmarks. Times in ms range (disk I/O dominant). `10k` measurably slower than `1k`.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add checkpoint/ benchmark group (WAL flush cost)"
```

---

## Task 10: `concurrent/` benchmark group (in-memory)

Measures concurrency throughput using `b.iter_custom` with thread spawning and barrier synchronization.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_concurrent function**

```rust
fn bench_concurrent(c: &mut Criterion) {
    use std::sync::{Arc as StdArc, Barrier};
    use std::time::Instant;

    // DB pre-populated with 10K facts (in-memory).
    let db = helpers::populate_in_memory(10_000);

    // readers: N threads all querying simultaneously
    {
        let mut group = c.benchmark_group("concurrent/readers");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait(); // release all threads simultaneously
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // readers_plus_writer: (N-1) readers + 1 writer
    {
        let mut group = c.benchmark_group("concurrent/readers_plus_writer");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let n_readers = n_threads - 1;
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        // readers
                        for _ in 0..n_readers {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        // 1 writer
                        {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // serialized_writers: N threads competing for the write Mutex.
    // NOTE: Measures lock-contention overhead, NOT write parallelism.
    // Writes are serialized by design. Throughput expected to stay flat or decrease slightly.
    {
        let mut group = c.benchmark_group("concurrent/serialized_writers");
        for &(label, n_threads) in &[("2", 2usize), ("4", 4), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }
}
```

Add `bench_concurrent` to `criterion_group!`.

- [ ] **Step 2: Run concurrent group**

```bash
cargo bench --bench minigraf_bench -- "concurrent/"
```

Expected: 10 benchmarks (3+3+4). `readers` throughput should improve or hold steady as threads increase (reads are parallel via RwLock). `serialized_writers` throughput should hold flat or decrease slightly (serialized by design).

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add concurrent/ benchmark group (in-memory concurrency)"
```

---

## Task 11: `concurrent_file/` benchmark group (file-backed)

Mirrors `concurrent/` but uses a file-backed DB with WAL writes. Setup creates a pre-populated file DB once.

**Files:**
- Modify: `benches/minigraf_bench.rs`

- [ ] **Step 1: Add bench_concurrent_file function**

```rust
fn bench_concurrent_file(c: &mut Criterion) {
    use std::sync::{Arc as StdArc, Barrier};
    use std::time::Instant;
    use tempfile::NamedTempFile;

    // Pre-create the file-backed DB once for all concurrent_file benchmarks.
    let tmp = Box::new(NamedTempFile::new().unwrap()); // Box to keep alive
    let path = tmp.path().to_str().unwrap().to_string();
    helpers::populate_file(10_000, &path);
    let db = helpers::open_file_no_checkpoint(&path);

    // readers (file-backed): concurrent page-cache reads under RwLock
    {
        let mut group = c.benchmark_group("concurrent_file/readers");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // readers_plus_writer (file-backed): readers + 1 WAL-writing thread
    {
        let mut group = c.benchmark_group("concurrent_file/readers_plus_writer");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let n_readers = n_threads - 1;
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_readers {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // serialized_writers (file-backed): N WAL-writing threads queuing on Mutex
    {
        let mut group = c.benchmark_group("concurrent_file/serialized_writers");
        for &(label, n_threads) in &[("2", 2usize), ("4", 4), ("8", 8), ("16", 16)] {
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label), &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles.into_iter().map(|h| h.join().unwrap()).max().unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    drop(tmp); // clean up temp file
}
```

Add `bench_concurrent_file` to `criterion_group!`.

- [ ] **Step 2: Run concurrent_file group**

```bash
cargo bench --bench minigraf_bench -- "concurrent_file/"
```

Expected: 10 benchmarks. Compare `concurrent_file/serialized_writers` vs `concurrent/serialized_writers` — the delta is WAL fsync cost under contention.

- [ ] **Step 3: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "feat(bench): add concurrent_file/ benchmark group (file-backed concurrency)"
```

---

## Task 12: Run full benchmark suite and capture output

- [ ] **Step 1: Run the complete suite**

```bash
cargo bench --bench minigraf_bench 2>&1 | tee /tmp/bench_results.txt
```

Expected: All 9 groups run. Criterion generates HTML reports in `target/criterion/`. No panics.

- [ ] **Step 2: Open HTML reports to review**

```bash
xdg-open target/criterion/report/index.html
# or on macOS: open target/criterion/report/index.html
```

Review each group for:
- No unexpected outliers or high noise (>10% CV)
- Plausible ordering (insert_file > insert; wal_replay open > checkpointed open)
- Concurrent results: readers scale flat or better; serialized_writers degrade gracefully

- [ ] **Step 3: Note any anomalies or unexpected results for README prose**

Record observations (e.g., whether query latency is flat across scales, whether WAL fsync dominates write cost, how concurrent readers scale).

- [ ] **Step 4: Commit any minor fixes found during the full run**

```bash
git add benches/
git commit -m "fix(bench): address any issues found during full benchmark run"
# (skip this commit if no fixes are needed)
```

---

## Task 13: Add `## Performance` section to README.md

- [ ] **Step 1: Add Performance section to README.md**

Add the following section to `README.md`, replacing placeholder values with actual numbers from the benchmark run. Insert before the `## Roadmap` section.

```markdown
## Performance

Benchmarks run with `cargo bench` on [CPU model, RAM size, OS]. Full HTML reports: `cargo bench` → `target/criterion/report/index.html`.

### Insert throughput (in-memory)

Steady-state cost of inserting into a pre-populated in-memory database.

| Pre-populated | Single fact | Batch (100 facts) | Explicit tx |
|---|---|---|---|
| 1K  | X µs | X µs | X µs |
| 10K | X µs | X µs | X µs |
| 100K| X µs | X µs | X µs |

### Insert throughput (file-backed, includes WAL fsync)

| Pre-populated | Single fact | Batch (100 facts) | Explicit tx |
|---|---|---|---|
| 1K  | X µs | X µs | X µs |
| 10K | X µs | X µs | X µs |
| 100K| X µs | X µs | X µs |

WAL fsync overhead per single fact: Xµs (delta between tables above at same scale).

### Query latency

| DB size | Point lookup (entity) | Point lookup (attribute) | 3-pattern join |
|---|---|---|---|
| 1K   | X µs | X µs | X µs |
| 10K  | X µs | X µs | X µs |
| 100K | X µs | X µs | X µs |
| 1M   | X µs | X µs | X µs |

### Time travel overhead

`:as-of` and `:valid-at` temporal filter overhead on top of base query cost.

| DB size | Base (point entity) | `:as-of` | `:valid-at` |
|---|---|---|---|
| 1K   | X µs | X µs | X µs |
| 10K  | X µs | X µs | X µs |
| 100K | X µs | X µs | X µs |
| 1M   | X µs | X µs | X µs |

### Transitive closure

Semi-naive fixed-point evaluation of recursive reachability rules.

| Graph | Nodes | Query time |
|---|---|---|
| Chain depth 10  | 11   | X µs |
| Chain depth 100 | 101  | X µs |
| Chain depth 1K  | 1001 | X ms |
| Fanout w5/d5   | ~3905 | X ms |
| Fanout w10/d3  | ~1110 | X ms |

### Concurrent throughput (in-memory, 10K fact DB)

Reported as wall-clock time per iteration (lower = better). Concurrent readers use RwLock (parallel). Writers use Mutex (serialized by design).

| Scenario | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| Readers only          | — | X µs | X µs | X µs |
| Readers + 1 writer    | — | X µs | X µs | X µs |
| Writers only (serial) | X µs | X µs | X µs | X µs |

### Concurrent throughput (file-backed, 10K fact DB, WAL writes)

| Scenario | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| Readers only          | — | X µs | X µs | X µs |
| Readers + 1 writer    | — | X µs | X µs | X µs |
| Writers only (serial) | X µs | X µs | X µs | X µs |

### Database open time

| DB size | Checkpointed (no WAL) | With WAL replay |
|---|---|---|
| 1K   | X µs | X µs |
| 10K  | X µs | X µs |
| 100K | X ms | — |
| 1M   | X ms | — |

### Findings

[Prose summary: note anything surprising — e.g. EAVT index keeps point_entity flat across all scales; WAL fsync cost dominates single-fact writes; write serialization degrades gracefully under thread contention.]
```

- [ ] **Step 2: Verify README renders correctly**

```bash
cargo doc --no-deps 2>&1 | head -20
```

(Just checking compilation isn't broken — README is not compiled but ensures no other regressions.)

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: add Performance section to README with Phase 6.3 benchmark results"
```

---

## Final Verification

- [ ] **Run full test suite to confirm benchmarks didn't break anything**

```bash
cargo test
```

Expected: all 280 tests pass.

- [ ] **Run full benchmark suite one more time for clean numbers**

```bash
cargo bench --bench minigraf_bench
```

Expected: all 9 groups complete, HTML reports in `target/criterion/`.

- [ ] **Push to remote**

```bash
git push
```
