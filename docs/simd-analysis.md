# SIMD Benchmarking Analysis (Issue #229)

**Date:** 2026-05-16  
**Environment:** Intel Core i7-1065G7 @ 1.30GHz (Ice Lake, SSE4.2/AVX2/AVX512), Linux 6.12.85-1-MANJARO, rustc 1.94.0 (4a4ef493e 2026-03-02), wide 0.7.33  
**Branch:** feature/issue-229-simd-benchmarking

---

## Method

Both scalar and SIMD benchmarks operate on synthetic numeric slices (not live DB facts — `Fact` internals are `pub(crate)`). The benchmarks isolate the hot kernel loop; extraction overhead is identical for both paths (none). For full-query costs, see the `time_travel/` group results below.

Three benchmark groups:
- **`simd_temporal`** — valid-time range filter: `valid_from[i] <= ts && ts < valid_to[i]`, ~50% selectivity
- **`simd_as_of`** — tx-time as-of filter: `tx_count[i] <= threshold`, 50% selectivity, using `u64x4`
- **`simd_aggregate`** — i64 horizontal sum via `i64x4` 4-wide reduction

All sizes: 100, 1k, 10k, 100k, 1M elements. Criterion 10-sample runs, bench profile (LTO enabled).

---

## Results: simd_temporal (valid-time range filter)

`valid_from[i] <= ts && ts < valid_to[i]` — ~50% selectivity

| Size   | Scalar (ns) | SIMD (ns) | Speedup (scalar/simd) |
|--------|------------|-----------|----------------------|
| 100    | 92.9       | 126.5     | 0.73x (SIMD slower)  |
| 1k     | 670.9      | 1,092.6   | 0.61x (SIMD slower)  |
| 10k    | 7,439.8    | 10,856.0  | 0.69x (SIMD slower)  |
| 100k   | 73,833     | 108,850   | 0.68x (SIMD slower)  |
| 1M     | 860,100    | 1,175,500 | 0.73x (SIMD slower)  |

**Crossover point:** None observed at any tested size. Scalar is consistently ~30–40% faster than SIMD across all sizes.

**Analysis:** The range filter (`valid_from[i] <= ts && ts < valid_to[i]`) involves two `i64` comparisons per element. LLVM autovectorization (enabled via `lto = true` in the bench profile) already vectorizes the scalar iterator effectively. The `wide` crate's `i64x4` path introduces overhead — likely from the count-matches bookkeeping (the benchmark counts passing elements to prevent dead-code elimination) — that outweighs the SIMD lane benefit.

---

## Results: simd_as_of (tx-time as-of filter)

`tx_count[i] <= threshold` — 50% selectivity. Uses `u64x4` (wide 0.7 `u64x4`).

| Size   | Scalar (ns) | SIMD (ns) | Speedup (scalar/simd) |
|--------|------------|-----------|----------------------|
| 100    | 54.3       | 100.7     | 0.54x (SIMD slower)  |
| 1k     | 464.6      | 894.2     | 0.52x (SIMD slower)  |
| 10k    | 4,680.9    | 8,855.0   | 0.53x (SIMD slower)  |
| 100k   | 46,367     | 88,418    | 0.52x (SIMD slower)  |
| 1M     | 611,000    | 980,130   | 0.62x (SIMD slower)  |

**Crossover point:** None observed at any tested size. Scalar is consistently ~50% faster than SIMD across all sizes.

**Analysis:** The single-comparison filter (`tx_count[i] <= threshold`) is the simplest possible loop body. LLVM autovectorizes this aggressively. The `u64x4` path from the `wide` crate is consistently 1.9x slower than scalar — indicating the abstraction overhead of the `wide` API exceeds the SIMD lane gain for this operation on this architecture.

---

## Results: simd_aggregate (i64 horizontal sum)

Sum of N `i64` values via `i64x4` 4-wide reduction.

| Size   | Scalar (ns) | SIMD (ns) | Speedup (scalar/simd) |
|--------|------------|-----------|----------------------|
| 100    | 47.3       | 21.8      | **2.17x**            |
| 1k     | 419.9      | 151.5     | **2.77x**            |
| 10k    | 4,125.9    | 1,538.4   | **2.68x**            |
| 100k   | 40,863     | 16,507    | **2.47x**            |
| 1M     | 493,980    | 256,820   | **1.92x**            |

**Crossover point:** SIMD wins at all tested sizes (100 through 1M). The speedup is consistent and significant — approximately 2–2.8x across the range.

**Analysis:** Horizontal sum is where SIMD excels. The `i64x4` reduction performs 4 additions per cycle vs 1 for scalar. LLVM's autovectorizer does not fully capture the same gain here because the horizontal reduction pattern is harder to autovectorize than a simple comparison filter. The `wide` crate's explicit SIMD path achieves a reliable 2–2.8x improvement at all practical sizes.

---

## Post-#208 Residual Analysis

After the B+Tree selective lookup introduced in #208, queries with bound entity or entity+attribute patterns route through index-backed lookups rather than full fact scans. This significantly reduces the number of facts reaching the temporal filter:

- **Selective lookup queries** (entity or entity+attribute bound): residual fact counts are typically O(attributes per entity) or O(facts per entity+attribute), often 1–100 facts. At these sizes, the temporal filter benchmarks show scalar is already sub-microsecond — SIMD overhead would make things worse.
- **Unbound queries** (full scans): all N facts reach the temporal filter. These are the cases where SIMD temporal filtering would theoretically help, but the benchmark results show scalar autovectorization already wins.
- **Aggregate queries** (e.g., summing numeric values across many facts): the `simd_aggregate` results are directly applicable. A 2–2.8x improvement in the summation kernel is achievable when aggregate operations are applied to large result sets.

The practical conclusion: for the current array-of-structs `Fact` storage layout, SIMD temporal filtering is not beneficial. SIMD aggregation is beneficial but requires extracting numeric values into a contiguous slice first — an O(N) copy that partially offsets the gain.

---

## Full-Query Context

From `time_travel/as_of_counter` and `time_travel/valid_at` (full Minigraf query stack, including parsing, transaction overhead, and fact materialization):

| Benchmark                          | 1k facts   | 10k facts  | 100k facts | 1M facts   |
|------------------------------------|------------|------------|------------|------------|
| `time_travel/as_of_counter`        | 867.1 µs   | 9.99 ms    | 109.3 ms   | 1,729 ms   |
| `time_travel/valid_at`             | 1,194 µs   | 12.60 ms   | 108.5 ms   | 356.9 ms   |

The full-query cost is orders of magnitude above the raw filter kernel. For example, the `simd_as_of` scalar kernel at 10k facts takes 4.7 µs, but the full `as_of_counter` query at 10k facts costs ~10 ms — a 2000x overhead from transaction, parsing, and fact materialization layers. This confirms that optimizing the filter kernel alone would have negligible impact on end-to-end query latency.

---

## Recommendation

**Skip (filter kernels) / Revisit with SoA layout (aggregates).**

**For temporal filtering (`simd_temporal`, `simd_as_of`):** SIMD is slower than scalar at all tested sizes (100 to 1M). LLVM autovectorization with `lto = true` already captures the available vectorization opportunity in the scalar iterator path. Integrating the `wide`-based filter helpers into production code would be a net regression. Recommend closing the filter SIMD work in #229 without further action.

**For aggregation (`simd_aggregate`):** SIMD wins at all sizes with a consistent 2–2.8x improvement. However, the current array-of-structs `Fact` storage layout means that reaching the `i64` values requires either:
1. An O(N) extraction pass to build a contiguous slice (partially negating the gain), or
2. A struct-of-arrays storage redesign (a much larger change).

The `pub(crate)` field restriction in `Fact` also means the SIMD helpers cannot access field data directly from production code paths without an API change.

**Action:** Close #229 with this analysis. Open a separate issue for struct-of-arrays storage layout if aggregate performance becomes a measured bottleneck. The `wide` dependency and `simd_helpers` module introduced in this branch provide a validated foundation for future SoA work.
