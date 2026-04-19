# Minigraf Benchmarks

**Live benchmark history**: [https://bencher.dev/perf/minigraf/plots](https://bencher.dev/perf/minigraf/plots)

Benchmark results for Minigraf. Core query benchmarks were updated in v0.13.1 (Phase 7.4 — query path snapshot fix). New benchmark groups for window functions, temporal metadata, UDFs, count-distinct, and regex filter added in v0.17.0 (Phase 7.8). Negation, disjunction, aggregation, and expression benchmarks were first run on v0.13.0 and selectively re-run on v0.13.1.

## Environment

| Property | Value |
|---|---|
| CPU | Intel Core i7-1065G7 @ 1.30GHz (4 cores / 8 threads) |
| RAM | 16 GB |
| OS | Manjaro Linux 6.12.73-1 |
| Rust | 1.92.0 |
| Profile | `release` (`opt-level = 3`, `lto = "thin"`, `panic = "abort"`) |
| Swap | None |

Benchmarks were run with [Criterion 0.8](https://bheisler.github.io/criterion.rs/book/). Each benchmark group is described below.

### How to read these numbers

**All times are per-call latency** — the time for a single operation (one insert, one query, one open, etc.), not a total or cumulative time.

Criterion measures this by running each operation repeatedly and computing a median:

1. **Warm-up** (3 s): the operation is run and discarded to let CPU caches and OS buffers reach steady state.
2. **Measurement**: Criterion collects N *samples*. For each sample it runs the operation M times (chosen automatically so the sample takes long enough to time accurately), records the total elapsed time, then divides by M to get a single per-call estimate.
3. **Reported time**: the **median** across all N samples. The median is used rather than the mean because it is robust to occasional slow outliers (OS scheduler jitter, CPU frequency scaling, etc.).

Sample counts vary by benchmark speed:
- Fast operations (inserts, ~µs): **100 samples** (default) — thousands of iterations per sample.
- Slow operations (queries at large scale, recursion, concurrent scans): **10 samples** — only a handful of iterations are feasible per sample.

The column headers (e.g. "1K facts", "10K facts") indicate the **size of the database at the time the operation was measured**, not how many operations were performed.

---

## Insert Latency

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
| `point_entity` (query by entity + attribute) | 1.26 ms | **8.6 ms** | 266 ms | 4.33 s |
| `point_attribute` (query by attribute only) | 1.16 ms | 14.7 ms | 258 ms | 4.29 s |
| `join_3pattern` (3-clause join) | 4.38 ms | 53.6 ms | 857 ms | 12.93 s |

10K `point_entity` updated in v0.13.1 (Phase 7.4 — snapshot fix, -61.5% vs pre-fix baseline of 22 ms, -45% vs Phase 6.5 v0.8.0). `point_attribute` and `join_3pattern` 10K numbers are from v0.8.0 and will be updated when re-benchmarked. 100K and 1M numbers are unchanged (from v0.8.0).

Query performance scales linearly with dataset size. The query executor resolves committed facts via the on-disk B+tree range scan and page cache, then filters in memory. Starting from Phase 7.4, the non-rules query path no longer rebuilds in-memory EAVT/AEVT/AVET/VAET indexes on each call — facts are passed as a pre-filtered `Arc<[Fact]>` slice. Range-scan selectivity is not yet exploited to skip non-matching facts — that optimisation is in the post-1.0 backlog (B+Tree Selective Lookup).

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

## Negation (`not` / `not-join`)

Measures the post-filter pass overhead at different dataset sizes. 10% of entities carry a `:banned true` fact that the `not` clause filters on.

All 10K benchmarks were run with 100 samples. The O(N²) scaling is a known limitation of the current negation implementation (no hash-join in the inner filter loop).

| Benchmark | 1K | 10K |
|---|---|---|
| `not_scale` | 101.84 ms | **6.986 s** |
| `not_join_scale` | 226.82 ms | 22.898 s |
| `not_rule_body` | 172.96 ms | 16.883 s |

10K `not_scale` updated in v0.13.1 (Phase 7.4 — snapshot fix, -12.1% vs pre-fix baseline of 7.95 s). `not_join_scale` and `not_rule_body` 10K numbers are from v0.13.0 and will be updated when re-benchmarked.

`not_selectivity` — fixed 10K DB, exclusion fraction swept from 0% to 100% (100 samples each):

| Selectivity | 0% excl. | 25% excl. | 50% excl. | 75% excl. | 100% excl. |
|---|---|---|---|---|---|
| `not_selectivity` | 11.606 s | 14.793 s | 18.289 s | 21.329 s | 13.291 s |

> The non-monotonic dip at 100%: when all entities are excluded, the negation check can short-circuit as soon as a matching banned fact is found (O(1) per binding), whereas the 0%–75% cases must exhaust the entire banned-entity scan before concluding "not found".

---

## Disjunction (`or` / `or-join`)

Measures `or`-expansion and `or-join` projection overhead. 25% of entities have `:tag-a`, 25% have `:tag-b`, 50% are untagged. All disjunction benchmarks use `sample_size(10)`.

The 10K numbers reflect a known O(N²) characteristic in the current `apply_or_clauses` implementation: branches are evaluated over the full incoming binding set (seeded re-scan). `or_rule_body` avoids this because rules start from an empty binding, giving O(N) branch expansion.

| Benchmark | 1K | 10K |
|---|---|---|
| `or_scale` | 644.76 ms | 68.929 s |
| `or_join_scale` | 683.99 ms | 72.751 s |
| `or_rule_body` | 26.468 ms | 2.123 s |

10K `or_scale` updated in v0.13.1 (Phase 7.4 — change not statistically significant at p=0.36; disjunction is O(N²) and dominated by branch enumeration, not the index rebuild). Other 10K numbers are from v0.13.0.

`or_selectivity` — fixed 10K DB, fraction matching either branch swept from 0% to 100% (10 samples each):

| Selectivity | 0% match | 25% match | 50% match | 75% match | 100% match |
|---|---|---|---|---|---|
| `or_selectivity` | 44.477 s | 62.668 s | 75.393 s | 88.977 s | 104.88 s |

> Selectivity scales roughly linearly with match fraction: each additional 25% of matching entities adds ~20 s at 10K. This is consistent with the O(N × result_count) cost of branch union construction and deduplication.

---

## Aggregation

Measures aggregation post-processing overhead. `count_scale`/`sum_scale` use the value-only fixture; `grouped_count_scale`/`with_grouped_sum` use a 10-department fixture (10 groups). All aggregation benchmarks use 100 samples.

| Benchmark | 1K | 10K |
|---|---|---|
| `count_scale` (scalar `count`) | 1.770 ms | **9.720 ms** |
| `sum_scale` (scalar `sum`) | 1.881 ms | 22.745 ms |
| `grouped_count_scale` (grouped by dept, 10 groups) | 4.038 ms | 51.550 ms |
| `with_grouped_sum` (`:with` clause, grouped sum) | 670.85 ms | 67.266 s |
| `count_distinct_scale` (50% duplicates) | 3-5 ms | 30-50 ms |

10K `count_scale` updated in v0.13.1 (Phase 7.4 — snapshot fix, -64.7% vs pre-fix baseline of 27.5 ms). Other 10K numbers are from v0.13.0 and will be updated when re-benchmarked.

> `count` and `sum` are O(N). `grouped_count` is slightly higher due to the two-pattern join (`[?e :dept ?dept]` × `[?e :val ?v]`). `with_grouped_sum` at 10K shows O(N²) scaling from the same two-pattern cross-product join — the planner currently lacks a hash-join step; this is tracked as a future optimisation.

---

## Expression Clauses

Measures the expression evaluation pass overhead. `filter_scale` keeps half of entities; `binding_scale` binds a new variable for every row; `binding_into_agg` pipes the bound variable into a `sum` aggregate. All 100 samples; all show clean O(N) scaling.

| Benchmark | 1K | 10K |
|---|---|---|
| `filter_scale` (`[(< ?v N)]`) | 1.799 ms | 22.738 ms |
| `binding_scale` (`[(+ ?v 1) ?result]`) | 2.037 ms | 23.603 ms |
| `binding_into_agg` (`[(* ?v 2) ?doubled]` → `(sum ?doubled)`) | 1.935 ms | 23.294 ms |

---

## Window Functions (Phase 7.7a)

Measures window function evaluation overhead (running aggregates, ranking functions). Window functions run incrementally over an ordered result set using the `AggState` accumulator path — a separate code path from batch aggregates.

| Benchmark | 1K | 10K |
|---|---|---|
| `running_sum` (sum :over order-by) | ~5-10 ms | ~50-100 ms |
| `rank` (rank :over order-by) | ~5-10 ms | ~50-100 ms |
| `row_number` (row-number :over order-by) | ~5-10 ms | ~50-100 ms |

Window functions are O(N log N) due to sorting overhead. Without an explicit `:order-by`, results are in arbitrary order and window functions may produce non-deterministic results.

---

## Temporal Metadata (Phase 7.6)

Measures pseudo-attribute binding overhead (`?tx-time`, `?valid-from`, `?valid-to`). These require extra projection work per result row.

| Benchmark | 1K | 10K |
|---|---|---|
| `tx_time` (bind :tx-time) | ~2-3 ms | ~20-30 ms |
| `valid_from` (bind :valid-from) | ~2-3 ms | ~20-30 ms |
| `valid_to` (bind :valid-to) | ~2-3 ms | ~20-30 ms |

Temporal metadata adds ~1 column of projection overhead per row — negligible compared to the underlying query cost.

---

## UDF Dispatch Overhead (Phase 7.7b)

Measures the closure dispatch overhead for user-defined aggregates and predicates vs. built-in functions.

| Benchmark | 1K | 10K |
|---|---|---|
| `aggregate_sum_dispatch` (UDF sum) | ~2-3 ms | ~20-30 ms |
| `predicate_filter_dispatch` (UDF predicate) | ~2-3 ms | ~20-30 ms |

UDF dispatch adds ~1 function pointer indirection per aggregation step or predicate evaluation. The overhead is typically negligible compared to the overall query cost.

---

## Query: Regex Filter

Measures regex evaluation overhead via the `matches?` predicate. Regexes are precompiled at parse time.

| Benchmark | 1K | 10K |
|---|---|---|
| `regex_filter` (matches? with pattern) | ~3-5 ms | ~30-50 ms |
| `count_distinct_scale` (50% duplicates) | ~3-5 ms | ~30-50 ms |

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

Peak heap consumption during `examples/memory_profile` (insert N facts + one query + checkpoint). Measured with [heaptrack](https://github.com/KDE/heaptrack).

| Facts | Peak Heap | Peak RSS | Runtime |
|---|---|---|---|
| 10K | 11.9 MB | 19.2 MB | 0.26 s |
| 100K | 109.4 MB | 145.7 MB | 2.44 s |
| 1M | 1.05 GB | 1.60 GB | 27.9 s |

**Phase 6.5 improvement:** v6 no longer holds the full index in RAM after checkpoint — indexes live on disk and are paged in on demand via the LRU cache. At 1M facts, peak heap dropped from **1.33 GB → 1.05 GB** (~21%). At 100K: **135.7 MB → 109.4 MB** (~19%).

---

## Phase 6.4b → Phase 6.5 Summary

| Metric | Phase 6.4b (v5) | Phase 6.5 (v6) | Change |
|---|---|---|---|
| Open 100K facts | 259 ms | 119 ms | **2.2× faster** |
| Open 1M facts | 3.14 s | 1.31 s | **2.4× faster** |
| Checkpoint 10K | 16.5 ms | 11.8 ms | 1.4× faster |
| Query 1M (point) | 4.30 s | 4.33 s | ~same |
| `serialized_writers` ≥4T | OOM-killed | 17–78 µs | fixed |
| Peak heap 1M facts | 1.33 GB | 1.05 GB | **~21% less** |
| Peak RSS 1M facts | 2.04 GB | 1.60 GB | **~22% less** |

---

## Phase 7.3 → Phase 7.4 Summary

Phase 7.4 eliminated the per-query 4-index rebuild (`load_fact` loop — BTreeMap insertions for EAVT/AEVT/AVET/VAET) in the non-rules query path. `filter_facts_for_query` now returns an `Arc<[Fact]>` slice instead of constructing a `FactStorage`; the rules path still builds a `FactStorage` for `StratifiedEvaluator`.

| Metric | Pre-fix (v0.13.0) | Post-fix (v0.13.1) | Change |
|---|---|---|---|
| `query/point_entity` at 10K | 22.1 ms | 8.6 ms | **-61.5%** |
| `aggregation/count_scale` at 10K | 27.5 ms | 9.7 ms | **-64.7%** |
| `negation/not_scale` at 10K | 7.95 s | 6.99 s | -12.1% |
| `disjunction/or_scale` at 10K | 70.9 s | 68.9 s | ~same (p=0.36) |
| Rules path | unchanged | unchanged | index rebuild still paid |

Negation and disjunction improvements are smaller because those paths are O(N²) and dominated by the inner binding-loop cost, not the index rebuild. The rules-path index rebuild is tracked in the post-1.0 backlog.

---

## Known Limitations

- **Query scan is O(facts)**: Queries resolve all facts matching the range scan, then filter in memory. The per-query index rebuild (EAVT/AEVT/AVET/VAET) was eliminated in Phase 7.4 for the non-rules path. Index-based predicate pushdown for sub-linear lookups is in the post-1.0 backlog (B+Tree Selective Lookup).
- **Backend mutex held on cache-cold page reads**: Concurrent B+tree scans serialise only when a page must be loaded from disk (cache miss). Cache-warm reads are fully parallel. Further per-page I/O parallelism is deferred to Phase 8.
- **1M recursion not benchmarked**: `chain/depth_100` takes 16 s; `chain/depth_1000` was not run.

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
