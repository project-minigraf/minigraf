# Phase 6.4b: Benchmarks + Light Publish Prep — Design Spec

## Goal

Run and document the existing Criterion benchmark suite, add memory profiling via heaptrack, write `BENCHMARKS.md` and a README "Performance" section, move `clap` out of library deps, complete `Cargo.toml` metadata, and enable GitHub Discussions. No crates.io publish — deferred to after Phase 6.5 (file format v6).

## Background

Phase 6.4a (retraction semantics + edge case tests) is complete with 298 tests passing. The Criterion benchmark suite already exists in `benches/minigraf_bench.rs` with 9 groups and `criterion 0.5` in `Cargo.toml`. Phase 6.4b validates the performance story and lays groundwork for eventual publish.

**Why defer publish to Phase 6.5?** Phase 6.5 will change the file format to v6 (on-disk B+tree indexes). Publishing at v0.8.0 would immediately force early adopters through a migration. Waiting until after v6 lands gives users a stable format from their first install.

---

## Section 1: Benchmark Execution

### Approach

Run the existing `benches/minigraf_bench.rs` suite using Criterion 0.5. The suite has 9 groups:

| Group | What it measures |
|---|---|
| `insert` | In-memory insert throughput (batches of 1, 10, 100 facts) |
| `insert_file` | File-backed WAL-write throughput (checkpoint suppressed) |
| `query` | Point-lookup and range-scan latency |
| `time_travel` | `:as-of` query overhead |
| `recursion` | Transitive-closure evaluation (chain + fanout graphs) |
| `open` | DB open latency (cold, WAL replay) |
| `checkpoint` | Checkpoint throughput |
| `concurrent` | Concurrent throughput (reads, read+write, serialized writes; in-memory) |
| `concurrent_file` | Concurrent throughput (reads, read+write, serialized writes; file-backed) |

### Scale Parameters

Scales vary by group — the bench code does not apply a uniform 10K/100K/1M grid across all groups:

- `insert/*`, `insert_file/*`: 1K, 10K, 100K facts
- `query/*`, `time_travel/*`, `open/checkpointed`: 1K, 10K, 100K, 1M facts
- `open/wal_replay`: 1K, 10K facts (1M excluded — WAL replay at that scale is prohibitively slow as a bench setup step)
- `recursion/*`: depth 10 and depth 100 (not fact counts)
- `checkpoint/*`: 1K, 10K facts
- `concurrent/*`, `concurrent_file/*`: fixed 10K DB; `readers` and `readers_plus_writer` use thread counts 4, 8, 16; `serialized_writers` uses 2, 4, 8, 16

The `open/checkpointed` group at 1M facts reads ~40K packed pages (~160MB) per iteration, well beyond the 256-page (1MB) LRU cache. Each open will cold-read from disk. If this group exceeds Criterion's default sample budget, extend `measurement_time` in the bench configuration for that group only.

### Execution

```bash
# Save a named baseline for future comparison
cargo bench --bench minigraf_bench -- --save-baseline main

# HTML reports land in target/criterion/
```

Criterion generates HTML reports automatically (via `html_reports` feature already enabled). We capture median latency and throughput per group/scale from the terminal output and HTML.

---

## Section 2: Memory Profiling

### Tool

**heaptrack** — Linux heap profiler, available on Manjaro. Captures peak heap, total allocations, and allocation site call stacks with minimal instrumentation overhead in release builds.

### Profiling Binary

Create `examples/memory_profile.rs`: a standalone binary that:
1. Creates a temp dir
2. Opens a file-backed DB (default 256-page cache)
3. Inserts N facts in batches of 100 (using the same `insert_val_facts` pattern as bench helpers)
4. Calls `db.checkpoint()` to flush to packed pages (WAL gone)
5. Runs one representative query: `(query [:find ?v :where [:e0 :val ?v]])`
6. Prints peak heap via a `heaptrack_api` annotation (or we just read from heaptrack output)

The binary accepts fact count as a positional argument, parsed with `std::env::args()` (no `clap` — consistent with the library not depending on it):

```rust
let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
```

Run for each scale:

```bash
heaptrack cargo run --example memory_profile --release -- 10000
heaptrack cargo run --example memory_profile --release -- 100000
heaptrack cargo run --example memory_profile --release -- 1000000
```

### Metrics to Record

Per scale:
- Peak heap (MB)
- Total bytes allocated
- Page cache ceiling: `256 pages × 4KB = 1MB` (theoretical; actual depends on hit rate)

---

## Section 3: Documentation

### `BENCHMARKS.md` (new file, repo root)

Structure:

```
# Minigraf Benchmarks

## Environment
CPU / RAM / OS / Rust version / minigraf version

## Criterion Results

### Insert Throughput
[table: scale × batch_size → median ns/op, ops/sec]

### File-Backed Insert (WAL write)
[table: scale × batch_size → median ns/op]

### Query Latency
[table: scale × query_type → median ns/op]

### Time Travel (`:as-of`)
[table: scale → median ns/op vs. non-temporal baseline]

### Recursive Rules (Transitive Closure)
[table: chain_depth / fanout → median ms]

### DB Open Latency
[table: scenario (cold / WAL replay) → median ms]

### Checkpoint Throughput
[table: scale → median ms, MB/s]

### Concurrent Throughput
[table: thread_count × scale → ops/sec]

## Memory Profiles
[table: scale → peak heap MB, total alloc MB, page cache ceiling]

## Interpretation
[narrative: key findings, scaling characteristics]

### Known Limitations

- **Recursion is O(depth²) on chain graphs** — semi-naive evaluation recomputes delta sets per stratum; depth_100 is the practical ceiling in the bench suite.
- **Concurrent benchmark numbers are wall-clock max across threads**, not aggregate throughput — not directly comparable to single-threaded latency figures.
- **`open/checkpointed` at 1M facts measures cold-read latency** — the 256-page LRU cache (1MB) is far smaller than the dataset (~160MB packed), so every open reads from disk. This is intentional: it measures worst-case open cost, not cached access.
```

### `README.md` — "Performance" Section

Added after the feature list, before the API reference. Contains:
- A two-row headline table (10K and 100K facts): insert throughput + point-query latency
- One sentence on memory usage
- Link: "See [BENCHMARKS.md](BENCHMARKS.md) for full results"

---

## Section 4: Light Publish Prep

### 4a — Remove Dead `clap` Dependency

`clap` is listed in `[dependencies]` in `Cargo.toml` but is never imported anywhere in `src/`. `src/main.rs` uses `std::env::args()` directly. The entry is dead weight that leaks into every consumer's dependency graph via `cargo add minigraf`.

Fix: delete the `clap` line from `[dependencies]`. No feature flag, no `required-features`, no source changes needed.

```toml
# Remove this line from [dependencies]:
clap = { version = "4.5", features = ["derive"] }
```

Verify after: `cargo build` and `cargo test` must still pass.

### 4b — `Cargo.toml` Metadata

Add to `[package]`:

```toml
repository = "https://github.com/project-minigraf/minigraf"
keywords = ["graph", "datalog", "bitemporal", "embedded", "database"]
categories = ["database", "embedded"]
readme = "README.md"
documentation = "https://docs.rs/minigraf"
```

---

## Section 5: GitHub Discussions

Enable via GitHub API:

```bash
gh api repos/project-minigraf/minigraf -X PATCH -f has_discussions=true
```

No content to create. Existing `CONTRIBUTING.md`, issue templates, and `CODE_OF_CONDUCT.md` already provide contributor context.

---

## Out of Scope (deferred to Phase 6.5)

- `lib.rs` export narrowing (API surface audit)
- rustdoc sweep on all public items
- `cargo clippy -- -D warnings` clean pass
- `unwrap()`/`expect()` audit in library code
- `cargo test` on macOS and Windows
- Actual crates.io publish
- Checkpoint-during-crash edge case test
- Error-path coverage sweep (~82% → ≥90%)

---

## Deliverables

1. `BENCHMARKS.md` — full Criterion + heaptrack results with machine spec header
2. `README.md` — "Performance" section with headline numbers
3. `examples/memory_profile.rs` — heaptrack profiling binary
4. `Cargo.toml` — dead `clap` dep removed; metadata fields complete
5. GitHub Discussions enabled on the repo
6. `CHANGELOG.md` — v0.8.0 entry added (required by CLAUDE.md doc-sync rule)

## Version

Ships as **v0.8.0**.
