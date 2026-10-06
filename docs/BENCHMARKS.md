# Minigraf Benchmarks

This document records reproducible local benchmark snapshots. Completed release-performance history belongs in the [CHANGELOG](../CHANGELOG.md); continuous CI history is available on [Bencher](https://bencher.dev/perf/minigraf/plots).

## Current Local Snapshot

**Version**: v2.0.0

**Date**: 2026-08-26

**Command**: `cargo bench -- 'query/(point_entity|point_attribute|join_3pattern)'`

| Property | Value |
|---|---|
| CPU | Intel Core i7-1065G7 @ 1.30 GHz (4 cores / 8 threads) |
| OS | Manjaro Linux 6.12.101-1 |
| Rust | 1.94.0 |
| Benchmark framework | Criterion 0.8 |
| Profile | `bench` (optimized) |

### Query Latency

Each value is Criterion's estimated per-query latency; the range is its 95% confidence interval. The current query fixture covers 1K and 10K facts.

| Benchmark | 1K facts | 10K facts |
|---|---:|---:|
| `point_entity` (bound entity + attribute) | 6.05 µs (5.91–6.25) | 5.88 µs (5.86–5.89) |
| `point_attribute` (bound attribute) | 2.12 ms (2.11–2.14) | 26.09 ms (25.68–26.34) |
| `join_3pattern` (three-clause join) | 5.94 ms (5.90–6.00) | 75.88 ms (71.22–82.39) |

`point_entity` uses a selective index-backed lookup. `point_attribute` and `join_3pattern` return larger result sets and therefore scale with the number of matching facts. Do not compare these local measurements directly with CI runs or earlier releases: host load, CPU-frequency policy, toolchain, and implementation all affect the result.

### Point Query vs. Version-Chain Depth (#323)

**Date**: 2026-09-26 · **Command**: `cargo bench --bench minigraf_bench -- point_query_chain_depth` · same host as above, on AC power and otherwise idle.

One entity's `:hash` is retracted and reasserted `depth` times (exactly one live value), plus a never-changed `:other` on the same entity and 2,000 filler facts; checkpointed file database. "Before" is v2.0.1; "after" is the #323 change.

| Query | Depth | Before | After |
|---|---:|---:|---:|
| `[:e/hot :hash ?v]` (churned attribute) | 1 | 19.5 µs | 18.3 µs |
| | 500 | 1.18 ms | 855 µs |
| | 2000 | 4.64 ms | 3.36 ms |
| `[:e/hot :other ?v]` (sibling attribute) | 1 | 20.0 µs | 19.2 µs |
| | 500 | 1.17 ms | 20.6 µs |
| | 2000 | 4.58 ms | 19.0 µs |
| `[?e :hash ?v]` (attribute scan) | 1 | 17.5 µs | 17.8 µs |
| | 500 | 1.14 ms | 841 µs |
| | 2000 | 4.49 ms | 3.33 ms |

The churned attribute still scales with its own history on v2.x; see #379 for the v3.0.0 fix.

### Net-Assert on Index Keys (#379, v3.0.0)

**Date**: 2026-10-06 · **Command**: `cargo bench --bench minigraf_bench -- point_query_chain_depth --warm-up-time 1 --measurement-time 3` · same fixture as above, on format v8. "Before" is `v3` at `74310ee`; "after" decides net-assert on index keys and translates only surviving entries. The `:as-of` row is new: `[:find ?v :as-of 2002 :valid-at :any-valid-time :where [:e/hot :hash ?v]]`, which used to scan every fact in the file.

| Query | Depth | Before | After |
|---|---:|---:|---:|
| `[:e/hot :hash ?v]` (churned attribute) | 1 | 19.9 µs | 21.1 µs |
| | 500 | 1.10 ms | 262 µs |
| | 2000 | 4.61 ms | 1.03 ms |
| `[:e/hot :other ?v]` (sibling attribute) | 1 | 21.6 µs | 22.6 µs |
| | 2000 | 28.8 µs | 30.1 µs |
| `[?e :hash ?v]` (attribute scan) | 1 | 22.7 µs | 23.5 µs |
| | 500 | 1.20 ms | 294 µs |
| | 2000 | 5.87 ms | 1.24 ms |
| `:as-of 2002` on the churned attribute | 1 | 5.59 ms | 23.4 µs |
| | 500 | 6.30 ms | 216 µs |
| | 2000 | 8.55 ms | 845 µs |

Depth-1 differences are within run-to-run noise: two back-to-back A/B runs measured 19.0–19.3 µs before and 19.8–19.9 µs after for the churned attribute, and 22.6 µs before and 21.2–21.5 µs after for the sibling attribute. In this fixture every retraction and re-assertion uses a new value, so each history entry is its own two-entry triple. The remaining cost is the walk over those keys. A triple with a long history of its own is skipped with one seek.

### Checkpoint After One Dirty Fact (#315)

**Date**: 2026-09-26 · **Command**: `cargo bench --bench minigraf_bench -- checkpoint/after_1_fact` · same host as above; file on tmpfs, so fsync cost is excluded.

An already-checkpointed file database of `n` facts receives one new fact, then `checkpoint()` runs; every iteration has exactly one dirty fact. "Before" is v2.0.1, which decoded and re-encoded every index entry. "After" copies index leaves that receive no new entries.

| Facts | Before | After |
|---:|---:|---:|
| 10k | 23.9 ms | 4.01 ms |
| 100k | 246 ms | 26.3 ms |

Cost still grows with graph size, because index pages are copied on every checkpoint. Checkpoints proportional to the change alone need copy-on-write pages, tracked with #374 for v3.0.0.

## Measurement Method

Criterion warms up each benchmark, collects ten samples for this query group, and estimates per-call latency from repeated iterations. The reported confidence intervals describe measurement uncertainty on this host; they are not cross-machine performance guarantees.

For changes that may affect performance, record the exact command, version, toolchain, host characteristics, and fixture size alongside the result. Use the Bencher history for trend and regression analysis across CI runs.

## Reproducing

```bash
# Run the current documented query snapshot.
cargo bench -- 'query/(point_entity|point_attribute|join_3pattern)'

# Run every Criterion benchmark (the full suite includes slow large-fixture cases).
cargo bench

# Run an individual group.
cargo bench -- 'insert'
cargo bench -- 'concurrent_btree_scan'
```

Criterion writes HTML reports to `target/criterion/`. A benchmark result should be refreshed after changes to the query, storage, synchronization, or compiler/toolchain paths it exercises.

## Scope and Caveats

- Results are measurements, not service-level guarantees.
- The full suite intentionally includes expensive large-fixture and quadratic workloads; run focused groups during development and the complete suite for a release-level performance review.
- Heap and RSS measurements require a separate profiler run and are not included in this snapshot.
