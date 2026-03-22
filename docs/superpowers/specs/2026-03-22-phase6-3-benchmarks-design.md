# Phase 6.3 Benchmarks Design

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish a Criterion benchmark suite that discovers Minigraf's performance characteristics across all major operation types, then document findings in README and drive targeted query optimization.

**Approach:** Benchmarks-first. Run a comprehensive suite to identify where the actual bottlenecks are, then optimize those specifically. This is exploratory benchmarking — no prior performance data exists for Minigraf.

**Tech Stack:** Criterion 0.5 (with `html_reports` feature), `tempfile` (already present), standard `std::thread` for concurrency benchmarks.

---

## Benchmark Categories

Seven benchmark groups, all using Criterion's `BenchmarkGroup` API for structured HTML output.

### 1. `insert/` — Write throughput

Measures steady-state insertion cost. The DB is pre-populated to N facts before each benchmark; the benchmark measures the cost of inserting *one more unit* into a DB of that size. This avoids measuring cold-start cost.

Scenarios:
- `single_fact/{1k,10k,100k}` — one fact per `execute("(transact [...])")` call
- `batch_100/{1k,10k,100k}` — 100 facts in a single transact
- `explicit_tx/{1k,10k,100k}` — single fact via `begin_write()` / `commit()`

### 2. `query/` — Read latency

DB pre-populated once per scale; queries run in tight Criterion loop.

Scenarios:
- `point_entity/{1k,10k,100k,1m}` — `[:find ?v :where [:e0 :attr ?v]]` (EAVT range scan)
- `point_attribute/{1k,10k,100k,1m}` — `[:find ?e :where [?e :attr _]]` (AEVT range scan)
- `join_3pattern/{1k,10k,100k,1m}` — 3-clause join across two entity hops

### 3. `time_travel/` — Temporal query performance

Measures overhead of temporal filtering on top of base query cost.

Scenarios:
- `as_of_counter/{1k,10k,100k}` — `:as-of N` (tx counter snapshot)
- `valid_at/{1k,10k,100k}` — `:valid-at "TIMESTAMP"` (valid time filter)

### 4. `recursion/` — Rule evaluation

Measures semi-naive fixed-point iteration cost for transitive closure queries.

Scenarios:
- `chain/{depth_10,depth_100,depth_1k}` — linear chain graph (worst case for recursion depth)
- `fanout/{width_5_depth_5,width_10_depth_3}` — fan-out graph (tests delta size)

### 5. `open/` — Database open time

Pre-creates a `.graph` file with N facts (once, outside the benchmark loop); measures `OpenOptions::new().path(p).open()` round-trip including packed page header read and index load.

Scenarios:
- `{1k,10k,100k,1m}` — open an existing file at each scale

### 6. `checkpoint/` — WAL flush cost

Measures `db.checkpoint()` cost as a function of pending WAL entries.

Scenarios:
- `{1k,10k}` — checkpoint after N uncommitted facts in WAL

### 7. `concurrent/` — Concurrency throughput

Uses `std::thread::spawn` + `Arc<Minigraf>` with barrier synchronization to start all threads simultaneously. Measures total ops/sec under contention.

Scenarios:
- `readers/{4,8,16}` — N threads all querying simultaneously (read scalability)
- `readers_plus_writer/{4,8,16}` — N-1 reader threads + 1 writer (mixed workload)
- `writers/{2,4,8,16}` — N threads all calling `execute("(transact ...)")` (write contention / serialization overhead)

---

## File Structure

```
benches/
  minigraf_bench.rs    # single file; all 7 groups via criterion BenchmarkGroup
  helpers.rs           # shared fixtures (module included from minigraf_bench.rs)
```

### `helpers.rs` fixtures

| Function | Returns | Used by |
|---|---|---|
| `populate_in_memory(n)` | `Arc<Minigraf>` | query, time_travel, recursion, concurrent |
| `populate_file(n, path)` | `()` | open, insert |
| `chain_graph(depth)` | `Arc<Minigraf>` | recursion/chain |
| `fanout_graph(width, depth)` | `Arc<Minigraf>` | recursion/fanout |

All helpers use deterministic data (`":e{i}"`, `":attr"`, integer values) so results are reproducible.

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
| Read (query, open) | `b.iter(|| ...)` with DB created once in group setup | DB state doesn't change; safe to reuse |
| Write (insert, checkpoint) | `b.iter_batched(setup, routine, BatchSize::SmallInput)` | Each iteration needs fresh state |
| Concurrent | `b.iter_custom(...)` with thread spawn + barrier | Criterion's standard iter can't express multi-thread |

For 1M-fact scales: only `query/` and `open/` benchmarks run at 1M. Insert and checkpoint benchmarks stop at 100K — building 1M facts fresh per Criterion iteration is impractical even with `iter_batched`.

---

## README "Performance" Section

A new `## Performance` section added to `README.md` after the benchmark suite runs. Structure:

```markdown
## Performance

Benchmarks run with `cargo bench` on [hardware]. Full HTML reports in `target/criterion/`.

### Insert throughput
| Pre-populated | Single fact | Batch (100) | Explicit tx |
...

### Query latency
| DB size | Point lookup | 3-pattern join |
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

Values filled in after running the suite on the development machine. Prose summary notes any surprising findings (e.g. index lookup flatness across scales, write serialization cost).

---

## Success Criteria

- `cargo bench` completes without errors
- HTML reports generated in `target/criterion/`
- All 7 groups produce plausible, stable numbers (low Criterion noise)
- `README.md` has a populated `## Performance` section
- At least one optimization hypothesis identified from the data (to feed Phase 6.3b)
