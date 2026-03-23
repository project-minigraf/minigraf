# Minigraf Benchmarks

**Live benchmark history**: [bencher.dev/console/projects/minigraf/perf](https://bencher.dev/console/projects/minigraf/perf)

Benchmark results for Minigraf v0.8.0 (Phase 6.5 — on-disk B+tree indexes, file format v6).

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
| `single_fact` (transact one fact at a time) | 2.65 µs | 2.74 µs | 2.69 µs |
| `batch_100` (100 facts per transact call) | 317 µs | 318 µs | 315 µs |
| `explicit_tx` (WriteTransaction, single fact) | 2.69 µs | 2.70 µs | 2.83 µs |

Single-fact insert is constant across dataset sizes — the in-memory pending index is O(1) per insert.

### File-Backed Backend

| Benchmark | 1K facts | 10K facts | 100K facts |
|---|---|---|---|
| `single_fact` | 3.77 µs | 3.55 µs | 3.51 µs |
| `batch_100` | 210 µs | 212 µs | 221 µs |
| `explicit_tx` | 3.60 µs | 3.63 µs | 3.54 µs |

File-backed insert latency is constant — writes go to the WAL sidecar, not the `.graph` file directly, so insert cost is independent of database size.

---

## Query Latency

Measures single-query latency against databases pre-loaded with 1K / 10K / 100K / 1M facts.

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `point_entity` (query by entity + attribute) | 1.26 ms | 15.6 ms | 266 ms | 4.33 s |
| `point_attribute` (query by attribute only) | 1.16 ms | 14.7 ms | 258 ms | 4.29 s |
| `join_3pattern` (3-clause join) | 4.38 ms | 53.6 ms | 857 ms | 12.93 s |

Query performance scales linearly with dataset size. The query executor resolves committed facts via the on-disk B+tree range scan and page cache, then filters in memory. Range-scan selectivity is not yet exploited to skip non-matching facts — that optimisation is planned for Phase 7.

---

## Time-Travel Query Latency

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `as_of_counter` (`:as-of` by tx counter) | 1.27 ms | 16.2 ms | 276 ms | 4.49 s |
| `valid_at` (`:valid-at` timestamp) | 1.27 ms | 16.0 ms | 272 ms | 4.47 s |

Time-travel queries have the same cost profile as plain queries — temporal filtering adds negligible overhead.

---

## Recursive Rules

| Benchmark | Time |
|---|---|
| `chain/depth_10` (linear chain, 10 hops) | 2.75 ms |
| `chain/depth_100` (linear chain, 100 hops) | 16.27 s |
| `fanout/w10_d3` (fanout width=10, depth=3) | 5.12 s |

Recursive rule evaluation uses semi-naive fixed-point iteration. Deep chains scale super-linearly: each iteration must re-evaluate all intermediate facts. The semi-naive evaluator avoids redundant recomputation, but `chain/depth_100` still requires ~100 iterations of growing intermediate tables.

---

## Database Open / Replay

Measures cold-open latency (loading a committed `.graph` file) and WAL replay latency.

| Benchmark | 1K | 10K | 100K | 1M |
|---|---|---|---|---|
| `checkpointed` (open committed v6 file) | 7.24 ms | 12.20 ms | 118.9 ms | 1.314 s |
| `wal_replay` (replay uncommitted WAL) | 8.30 ms | 13.4 ms | — | — |

**Phase 6.5 improvement:** v6 open no longer loads indexes into RAM. At 1M facts, open time dropped from **3.14 s → 1.31 s** (2.4×). At 100K: **259 ms → 119 ms** (2.2×). The remaining cost is dominated by WAL check plus page-cache warming on the first query.

At small sizes (1K), v6 open is slower than v5 (7.2 ms vs 1.83 ms) — the per-open overhead (header I/O, B+tree root setup, WAL check) is not amortised enough at 1K facts to overcome the benefit of not loading a tiny index.

---

## Checkpoint

Measures time to flush the WAL to committed `.graph` pages (including B+tree rebuild for all four indexes).

| Benchmark | 1K | 10K |
|---|---|---|
| `checkpoint` | 1.25 ms | 11.80 ms |

Checkpoint now includes a merge-sort of committed + pending entries and a B+tree rebuild across all four indexes (EAVT, AEVT, AVET, VAET). At 10K facts this is **11.8 ms** — slightly faster than the v5 paged-blob serialisation (16.5 ms), as the B+tree writer makes fewer random-access passes.

---

## Concurrency (In-Memory)

Pre-loaded 10K-fact database. All threads operate concurrently.

| Benchmark | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| `readers` | — | 39.1 ms | 77.5 ms | 147.5 ms |
| `readers_plus_writer` | — | 33.5 ms | 66.7 ms | 141.6 ms |
| `serialized_writers` | 6.09 µs | 17.65 µs | 38.8 µs | 77.7 µs |

`serialized_writers` at ≥4 threads was previously OOM-killed on this machine. With v6, facts are cleared from RAM after each checkpoint, so accumulated memory is much lower and all thread counts now complete.

---

## Concurrency (File-Backed)

Pre-loaded 10K-fact database.

| Benchmark | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| `readers` | — | 41.5 ms | 87.7 ms | 152.8 ms |
| `readers_plus_writer` | — | 34.0 ms | 73.9 ms | 146.8 ms |
| `serialized_writers` | 10.98 µs | 25.9 µs | 56.4 µs | 112 µs |

---

## Concurrent B+Tree Range Scans (Phase 6.5, new)

Measures wall-clock latency of N simultaneous EAVT range scans against a committed B+tree with 10K facts.

| Threads | Median latency |
|---|---|
| 2 | 22.4 ms |
| 4 | 33.1 ms |
| 8 | 63.9 ms |

Scaling: 2→4 threads is ~1.5× (good), 4→8 threads is ~1.9× (improved from 2.2× before per-page locking). The backend `Mutex` is now held only for the duration of a single `read_page` call on a cache miss — on cache-warm pages no lock is acquired at all, allowing concurrent readers to proceed in parallel. Remaining contention at 8 threads reflects cold-page I/O serialisation, which is unavoidable and correct.

---

## Memory Usage (heaptrack)

Peak heap consumption during `examples/memory_profile` (insert N facts + one query + checkpoint). Measured with [heaptrack](https://github.com/KDE/heaptrack). These numbers reflect **v5** (Phase 6.4b); Phase 6.5 is expected to reduce peak heap by eliminating the in-memory index copy after checkpoint, but has not yet been re-profiled.

| Facts | Peak Heap | Peak RSS | Runtime |
|---|---|---|---|
| 10K | 14.4 MB | 22.5 MB | 0.26 s |
| 100K | 135.7 MB | 194.6 MB | 2.33 s |
| 1M | 1.33 GB | 2.04 GB | 26.8 s |

---

## Phase 6.4b → Phase 6.5 Summary

| Metric | Phase 6.4b (v5) | Phase 6.5 (v6) | Change |
|---|---|---|---|
| Open 100K facts | 259 ms | 119 ms | **2.2× faster** |
| Open 1M facts | 3.14 s | 1.31 s | **2.4× faster** |
| Checkpoint 10K | 16.5 ms | 11.8 ms | 1.4× faster |
| Query 1M (point) | 4.30 s | 4.33 s | ~same |
| `serialized_writers` ≥4T | OOM-killed | 17–78 µs | fixed |

---

## Known Limitations

- **Query scan is O(facts)**: Queries resolve all facts matching the range scan, then filter in memory. Phase 7 will enable index-based predicate pushdown for sub-linear lookups.
- **Backend mutex held on cache-cold page reads**: Concurrent B+tree scans serialise only when a page must be loaded from disk (cache miss). Cache-warm reads are fully parallel. Further per-page I/O parallelism is deferred to Phase 8.
- **1M recursion not benchmarked**: `chain/depth_100` takes 16 s; `chain/depth_1000` was not run.
- **Memory profile not re-run for v6**: heaptrack numbers above are from Phase 6.4b (v5). Phase 6.5's post-checkpoint RAM reduction is expected but not yet measured.

---

## Reproducing

```bash
# Run all Criterion benchmarks (HTML report in target/criterion/)
cargo bench

# Run a specific group
cargo bench -- "insert"
cargo bench -- "concurrent_btree_scan"

# Run heaptrack memory profile (requires heaptrack installed)
cargo build --release --example memory_profile
heaptrack ./target/release/examples/memory_profile 100000
heaptrack_print -f heaptrack.memory_profile.*.zst --merge-backtraces=0
```
