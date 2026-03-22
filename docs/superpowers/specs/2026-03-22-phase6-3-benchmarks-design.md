# Phase 6.3 Benchmarks Design

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish a Criterion benchmark suite that discovers Minigraf's performance characteristics across all major operation types, then document findings in README and drive targeted query optimization.

**Approach:** Benchmarks-first. Run a comprehensive suite to identify where the actual bottlenecks are, then optimize those specifically. This is exploratory benchmarking — no prior performance data exists for Minigraf.

**Tech Stack:** Criterion 0.5 (with `html_reports` feature), `tempfile` (already present), standard `std::thread` for concurrency benchmarks.

---

## Benchmark Categories

Seven benchmark groups, all using Criterion's `BenchmarkGroup` API for structured HTML output.

### 1. `insert/` — Write throughput

Measures steady-state insertion cost. The DB is pre-populated to N facts before each benchmark; the benchmark measures the cost of inserting *one more unit* into a DB of that size. Pre-populated state uses an **in-memory** DB (via `populate_in_memory(n)`) to avoid file I/O in setup and to prevent memory pressure from multiple temp files during `iter_batched`.

Use `BatchSize::LargeInput` for all insert scenarios — Criterion generates one batch at a time, safe even at 100K pre-populated scale.

Scenarios:
- `single_fact/{1k,10k,100k}` — one fact per `execute("(transact [...])")` call
- `batch_100/{1k,10k,100k}` — 100 facts in a single transact
- `explicit_tx/{1k,10k,100k}` — single fact via `begin_write()` / `commit()`

### 2. `query/` — Read latency

DB pre-populated once per scale (outside the benchmark loop); queries run in tight Criterion `b.iter(|| ...)` loop.

Entities are stored as EDN keyword form: `":e{i}"` in Datalog source maps to `:e0`, `:e1`, etc. The `populate_in_memory(n)` helper uses `format!(":e{}", i)` as the entity field, so queries use `[:e0 :attr ?v]` (keyword literal, not string literal).

Scenarios:
- `point_entity/{1k,10k,100k,1m}` — `[:find ?v :where [:e0 :attr ?v]]` (EAVT range scan)
- `point_attribute/{1k,10k,100k,1m}` — `[:find ?e :where [?e :attr _]]` (AEVT range scan)
- `join_3pattern/{1k,10k,100k,1m}` — 3-clause join across two entity hops, e.g.:
  `[:find ?name :where [:e0 :attr ?mid] [?mid :attr ?v] [?v :attr ?name]]`
  (exercises optimizer join reordering)

### 3. `time_travel/` — Temporal query performance

Measures overhead of temporal filtering on top of base query cost. Includes 1M scale to allow direct comparison against `query/` at the same size and to fully characterize the temporal overhead curve.

Scenarios:
- `as_of_counter/{1k,10k,100k,1m}` — `:as-of N` (tx counter snapshot)
- `valid_at/{1k,10k,100k,1m}` — `:valid-at "TIMESTAMP"` (valid time filter)

### 4. `recursion/` — Rule evaluation

Measures semi-naive fixed-point iteration cost for transitive closure queries. All scenarios use in-memory DBs created by graph fixtures.

Scenarios:
- `chain/{depth_10,depth_100,depth_1k}` — linear chain graph (worst case for recursion depth)
- `fanout/{width_5_depth_5,width_10_depth_3}` — fan-out graph (tests delta size per iteration)

### 5. `open/` — Database open time

Two sub-scenarios:

**Cold open (checkpointed):** Pre-creates a `.graph` file with N facts, fully checkpointed (no WAL sidecar). Measures `OpenOptions::new().page_cache_size(DEFAULT).path(p).open()` — packed page header read + index load. Note: `page_cache_size()` must be called before `path()` in the builder chain.

**Open with WAL replay:** Pre-creates a `.graph` file with N facts in the WAL sidecar (using `wal_checkpoint_threshold: usize::MAX` during population to suppress auto-checkpoint). Measures open time when a WAL must be replayed (crash-recovery path).

Scenarios:
- `checkpointed/{1k,10k,100k,1m}` — clean open with no WAL
- `wal_replay/{1k,10k}` — open with N committed WAL entries pending replay (1M excluded; setup at that scale is impractical with iter_batched)

### 6. `checkpoint/` — WAL flush cost

Measures `db.checkpoint()` cost as a function of facts committed to the WAL sidecar but not yet flushed to packed pages. The setup phase opens the DB with `wal_checkpoint_threshold: usize::MAX` to suppress auto-checkpointing during the N-fact population phase. Facts in the WAL at checkpoint time are committed (durably written) — the checkpoint flushes them to packed pages and then deletes the WAL file.

Scenarios:
- `{1k,10k}` — checkpoint after N committed facts pending in WAL sidecar

### 7. `concurrent/` — Concurrency throughput

Uses `std::thread::spawn` + `Arc<Minigraf>` with `std::sync::Barrier` synchronization to start all threads simultaneously. Measures total ops/sec under contention. Uses `b.iter_custom(...)` — Criterion's standard `iter` cannot express multi-thread workloads.

The shared DB is pre-populated with 10K facts before threads are spawned. An empty DB would measure only parser overhead; 10K gives a realistic working set where index scans and lock contention are meaningful.

**Note on `serialized_writers`:** Minigraf serializes writes via an exclusive `Mutex`. The `serialized_writers` scenario does not measure parallel write throughput (there is none by design — only one writer proceeds at a time). It measures lock-contention overhead and scheduling cost as writer thread count increases. Expect total throughput to remain flat or decrease slightly with more threads; this is correct behavior, not a bug.

Scenarios:
- `readers/{4,8,16}` — N threads all querying simultaneously (read scalability via RwLock)
- `readers_plus_writer/{4,8,16}` — N-1 reader threads + 1 writer (mixed workload, RwLock vs Mutex interaction)
- `serialized_writers/{2,4,8,16}` — N threads competing for the write Mutex; measures serialization overhead, not write parallelism

---

## File Structure

```
benches/
  minigraf_bench.rs      # single entry point; all 7 groups via BenchmarkGroup
  helpers/
    mod.rs               # shared fixtures; included via `mod helpers;` in minigraf_bench.rs
```

Cargo treats each file directly under `benches/` as a bench entry point. Subdirectory modules (`helpers/mod.rs`) are not treated as entry points and must be included via `mod helpers;` inside `minigraf_bench.rs`.

### `helpers/mod.rs` fixtures

| Function | Returns | Used by |
|---|---|---|
| `populate_in_memory(n: usize)` | `Arc<Minigraf>` | query, time_travel, recursion, concurrent, insert |
| `populate_file(n: usize, path: &str)` | `()` | open/checkpointed |
| `populate_file_no_checkpoint(n: usize, path: &str)` | `()` | open/wal_replay, checkpoint (opens with `wal_checkpoint_threshold: usize::MAX`) |
| `chain_graph(depth: usize)` | `Arc<Minigraf>` | recursion/chain |
| `fanout_graph(width: usize, depth: usize)` | `Arc<Minigraf>` | recursion/fanout |

All helpers use deterministic data — entity keyword form `:e{i}`, attribute `:attr`, integer value `i` — so results are reproducible across runs.

Builder chain order for file-backed helpers: `OpenOptions::new().page_cache_size(256).path(path).open()`. The `page_cache_size()` call must precede `path()` due to the builder's type-state design.

### `Cargo.toml` additions

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "minigraf_bench"
harness = false
```

---

## Measurement Strategy

| Benchmark type | Criterion pattern | Rationale |
|---|---|---|
| Read (query, time_travel, recursion, open) | `b.iter(|| ...)` with DB created once in group setup | DB state does not change; safe to reuse across iterations |
| Write (insert, checkpoint) | `b.iter_batched(setup, routine, BatchSize::LargeInput)` | Each iteration needs fresh state; LargeInput generates one batch at a time to avoid memory pressure at large scales |
| Concurrent | `b.iter_custom(...)` with thread spawn + barrier | Criterion's standard iter cannot express multi-thread workloads |

For 1M-fact scales: only `query/`, `time_travel/`, and `open/checkpointed` run at 1M. Insert, checkpoint, and WAL-replay benchmarks stop at 100K (insert, checkpoint) or 10K (wal_replay) — setup at larger scales with `iter_batched` is impractical.

---

## README "Performance" Section

A new `## Performance` section added to `README.md` after the benchmark suite runs. Structure:

```markdown
## Performance

Benchmarks run with `cargo bench` on [hardware summary]. Full HTML reports: `target/criterion/`.

### Insert throughput
| Pre-populated | Single fact | Batch (100) | Explicit tx |
|---|---|---|---|
| 1K  | X µs | X µs | X µs |
| 10K | X µs | X µs | X µs |
| 100K| X µs | X µs | X µs |

### Query latency
| DB size | Point lookup (entity) | Point lookup (attr) | 3-pattern join |
...

### Time travel overhead
...

### Transitive closure
...

### Concurrent throughput
...

### Database open time
...
```

Values filled in after running the suite on the development machine. Prose summary notes any surprising findings (e.g. index lookup flatness across scales, write serialization cost at high thread counts, WAL replay cost vs. clean open).

---

## Success Criteria

- `cargo bench` completes without panics or Criterion errors
- HTML reports generated in `target/criterion/` with stable measurements (Criterion noise < 5% for most benchmarks)
- All 7 groups produce plausible numbers across all defined scales
- `README.md` has a populated `## Performance` section with actual numbers from the run
- A written Phase 6.3b plan document referencing specific benchmark numbers identifies concrete optimization targets (e.g. "join_3pattern at 1M shows 50ms — optimizer improvement target")
