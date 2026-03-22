# Minigraf Benchmarks

Benchmark results for Minigraf v0.8.0 (Phase 6.4b).

## Environment

| Property | Value |
|---|---|
| CPU | Intel Core i7-1065G7 @ 1.30GHz (4 cores / 8 threads) |
| RAM | 16 GB |
| OS | Manjaro Linux 6.12.73-1 |
| Rust | 1.92.0 |
| Profile | `release` (`opt-level = 3`, `lto = "thin"`, `panic = "abort"`) |
| Swap | None |

Benchmarks were run with [Criterion 0.5](https://bheisler.github.io/criterion.rs/book/). Each benchmark group is described below. Times shown are the median of 100 samples unless noted otherwise.

---

## Insert Throughput

Measures per-fact insert latency at three dataset sizes (1K / 10K / 100K facts in the database at insert time).

### In-Memory Backend

| Benchmark | 1K facts | 10K facts | 100K facts |
|---|---|---|---|
| `single_fact` (transact one fact at a time) | 2.39 µs | 2.38 µs | 2.35 µs |
| `batch_100` (100 facts per transact call) | 268 µs | 260 µs | 275 µs |
| `explicit_tx` (WriteTransaction, single fact) | 2.28 µs | 2.24 µs | 2.24 µs |

Single-fact insert is constant across dataset sizes — the in-memory index is O(1) per insert.

### File-Backed Backend

| Benchmark | 1K facts | 10K facts | 100K facts |
|---|---|---|---|
| `single_fact` | 3.34 µs | 3.36 µs | 3.36 µs |
| `batch_100` | 207 µs | 210 µs | 214 µs |
| `explicit_tx` | 3.35 µs | 3.36 µs | 3.39 µs |

File-backed insert latency is also constant — writes go to the WAL sidecar, not the `.graph` file directly, so insert cost is independent of database size.

---

## Query Latency

Measures single-query latency against databases pre-loaded with 1K / 10K / 100K / 1M facts.

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `point_entity` (query by entity + attribute) | 1.17 ms | 14.9 ms | 257 ms | 4.30 s |
| `point_attribute` (query by attribute only) | 1.06 ms | 13.8 ms | 243 ms | 4.11 s |
| `join_3pattern` (3-clause join) | 4.05 ms | 50.5 ms | 815 ms | 12.3 s |

Query performance scales linearly with dataset size. This is expected for the current in-memory index implementation: the query executor scans all facts and filters by pattern. Phase 6.5 (on-disk B+tree indexes) will bring this to sub-linear for attribute- and entity-selective queries.

---

## Time-Travel Query Latency

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `as_of_counter` (`:as-of` by tx counter) | 1.16 ms | 14.7 ms | 254 ms | 4.27 s |
| `valid_at` (`:valid-at` timestamp) | 1.16 ms | 14.6 ms | 250 ms | 4.20 s |

Time-travel queries have the same cost profile as plain queries — temporal filtering adds negligible overhead on top of the scan.

---

## Recursive Rules

| Benchmark | Time |
|---|---|
| `chain/depth_10` (linear chain, 10 hops) | 2.53 ms |
| `chain/depth_100` (linear chain, 100 hops) | 15.7 s |
| `fanout/w10_d3` (fanout width=10, depth=3) | 4.84 s |

Recursive rule evaluation uses semi-naive fixed-point iteration. Deep chains scale super-linearly: each iteration must re-evaluate all intermediate facts. The semi-naive evaluator avoids redundant recomputation, but chain/100 still requires ~100 iterations of growing intermediate tables.

---

## Database Open / Replay

Measures cold-open latency (loading a committed `.graph` file) and WAL replay latency.

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `checkpointed` (open committed file) | 1.83 ms | 21.8 ms | 259 ms | 3.14 s |
| `wal_replay` (replay uncommitted WAL) | 1.66 ms | 19.6 ms | — | — |

Open time includes deserialising packed fact pages and rebuilding the in-memory index. Phase 6.5 (on-disk B+tree) will decouple open time from fact count.

---

## Checkpoint

Measures time to flush the WAL to committed `.graph` pages.

| Benchmark | 1K | 10K |
|---|---|---|
| `checkpoint` | 1.40 ms | 16.5 ms |

---

## Concurrency (In-Memory)

Pre-loaded 10K-fact database. All threads operate concurrently.

| Benchmark | 4 threads | 8 threads | 16 threads |
|---|---|---|---|
| `readers` (concurrent read queries) | 32.2 ms | 67.3 ms | 125 ms |
| `readers_plus_writer` (reads + 1 writer) | 26.4 ms | 58.2 ms | 120 ms |
| `serialized_writers` (sequential writes, N threads) | 4.76 µs (2 threads) | — | — |

`serialized_writers` at ≥4 threads triggered OOM kills on this machine (no swap, memory exhausted by the millions of in-memory facts accumulated across ~343K benchmark iterations). See **Known Limitations** below.

---

## Concurrency (File-Backed)

Pre-loaded 10K-fact database.

| Benchmark | 4 threads | 8 threads | 16 threads |
|---|---|---|---|
| `readers` | 31.8 ms | 64.4 ms | 126 ms |
| `readers_plus_writer` | 25.6 ms | 57.2 ms | 119 ms |
| `serialized_writers` | 9.66 µs (2 threads) | — | — |

---

## Memory Usage (heaptrack)

Peak heap consumption during `examples/memory_profile` (insert N facts + one query + checkpoint). Measured with [heaptrack](https://github.com/KDE/heaptrack).

| Facts | Peak Heap | Peak RSS | Runtime |
|---|---|---|---|
| 10K | 14.4 MB | 22.5 MB | 0.26 s |
| 100K | 135.7 MB | 194.6 MB | 2.33 s |
| 1M | 1.33 GB | 2.04 GB | 26.8 s |

Memory scales linearly with fact count (~135 bytes/fact peak heap at 100K, ~1.35 KB/fact at 1M due to in-memory index structures). Phase 6.5 will bring index memory to O(cache_pages) by moving to on-disk B+tree indexes.

Raw heaptrack data files (`.zst`) are available locally for detailed flamegraph analysis.

---

## Known Limitations

- **`serialized_writers` at ≥4 threads OOM-kills**: The Criterion benchmark accumulates millions of in-memory facts across hundreds of thousands of iterations. On systems without swap and with limited free RAM (~4.8 GB available), the Linux OOM killer terminates the benchmark process. Only the 2-thread result is reported.

- **Query scan is O(facts)**: All query benchmarks perform a full scan of the in-memory index. Phase 6.5 will add on-disk B+tree indexes that enable sub-linear lookups for entity- and attribute-selective queries.

- **Open time is O(facts)**: Database open deserialises all packed pages into memory. Phase 6.5 will allow lazy loading via the page cache.

- **1M recursion not benchmarked**: The recursion group uses depth/fanout counts, not fact counts. `chain/depth_100` already takes 15.7 s; `chain/depth_1000` was not run.

---

## Reproducing

```bash
# Run all Criterion benchmarks (HTML report in target/criterion/)
cargo bench

# Run a specific group
cargo bench -- "insert"

# Run heaptrack memory profile (requires heaptrack installed)
cargo build --release --example memory_profile
heaptrack ./target/release/examples/memory_profile 100000
heaptrack_print -f heaptrack.memory_profile.*.zst --merge-backtraces=0
```
