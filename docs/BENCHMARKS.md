# Minigraf Benchmarks

This document records reproducible local benchmark snapshots. Completed release-performance history belongs in the [CHANGELOG](../CHANGELOG.md); continuous CI history is available on [Bencher](https://bencher.dev/perf/minigraf/plots).

## Performance Envelope (v3.0.0, #394)

**Version**: `v3` branch (format v8), unreleased · **Date**: 2026-10-11 · **Command**: `cargo run --profile envelope --example perf_envelope -- --scales 100k,1m,10m` · Intel Core i7-1065G7 (4 cores / 8 threads, 16 GB RAM), Rust 1.94.0, Linux 6.12.108, on AC power and otherwise idle; the `.graph` file on an SSD with the OS page cache warm.

This section lists the graph sizes and query shapes that are tested, with their p50 and p99 latency, and the ones that are not supported yet. Re-run it for each minor release (CONTRIBUTING.md).

### What is supported

| Workload | Tested up to | p50 / p99 at 10M facts | Notes |
|---|---:|---:|---|
| Point lookup: bound entity and attribute, including `:as-of` and `:valid-at` | 10M facts | 12–14 µs / 20–48 µs | Flat from 100K to 10M |
| Entity with a long history (1,000 versions), current and `:as-of` | 10M facts | 0.26 ms / 0.30 ms | Grows with that entity's history, not the graph |
| Open a checkpointed file | 10M facts | 0.12 ms, 3 MiB RSS | Indexes are paged in on demand |
| Checkpoint after 1 new fact | 10M facts | 3.8 ms / 21 ms | Copy-on-write: cost follows the change |
| Attribute scan, attribute + value, grouped aggregation, joins | 1M facts | see below | Linear in the facts of the attributes they touch: about 0.1–0.5 s at 1M facts, seconds at 10M |
| Rule queries (recursion) | 1M facts | 2.7 s at 100K, 40 s at 1M | Read the whole database (#529); not interactive past 100K |

**Not supported yet:**

- Interactive rule queries past 100K facts. `recursive_ancestors` takes 40 s and 3.2 GiB of RSS at 1M facts because rule evaluation reads the whole database (#529); before #528 was fixed it failed there with `INT-020`.
- Graphs over 10M facts are not tested.
- Interactive latency for scans and joins at 10M facts. They take 3–21 s and materialize every fact of the attributes they touch, 0.7–3.6 GiB of RSS.

### Storage

| Metric | 100K | 1M | 10M |
|---|---:|---:|---:|
| File size | 10 MiB | 120 MiB | 1,202 MiB |
| Build time (200-entity transactions, checkpoint every 200K facts) | 3.2 s | 12.7 s | 195 s |
| Build peak RSS | 185 MiB | 403 MiB | 421 MiB |
| Open | 120 µs | 136 µs | 124 µs |
| RSS after open | 3 MiB | 3 MiB | 3 MiB |
| Checkpoint, 1 new fact (p50 / p99) | 3.1 ms / 5.1 ms | 2.9 ms / 4.3 ms | 3.8 ms / 20.5 ms |
| Checkpoint, 10K new facts | 41 ms | 83 ms | 722 ms |
| Peak RSS (open + 21 checkpoints) | 20 MiB | 21 MiB | 21 MiB |

### Query latency (p50 / p99)

| Shape | Query | 100K | 1M | 10M |
|---|---|---:|---:|---:|
| `point_entity` | `[:p{m} :age ?a]` | 13 µs / 16 µs | 12 µs / 48 µs | 12 µs / 46 µs |
| `as_of` | `:as-of {tx}`, `[:p{m/4} :age ?a]` | 14 µs / 36 µs | 14 µs / 20 µs | 14 µs / 30 µs |
| `valid_at` | `:valid-at "2021-06-01…"`, `[:p{m} :status ?s]` | 15 µs / 49 µs | 14 µs / 20 µs | 14 µs / 23 µs |
| `hot_current` | `[:hot :counter ?v]`, 1,000 versions | 263 µs / 301 µs | 256 µs / 293 µs | 268 µs / 300 µs |
| `hot_as_of` | same, `:as-of` halfway | 257 µs / 284 µs | 262 µs / 293 µs | 264 µs / 304 µs |
| `attribute_value` | `[?e :name "p{m}"]` (1 row) | 24.5 ms / 25.6 ms | 288 ms / 297 ms | 3.16 s / 3.16 s |
| `attribute_scan` | `[?e :age ?a]` (N/6 rows) | 43.4 ms / 49.4 ms | 462 ms / 484 ms | 4.83 s / 5.18 s |
| `count_grouped` | `[:find ?g (count ?e) :where [?e :group ?g]]` | 46.3 ms / 48.6 ms | 521 ms / 524 ms | 5.66 s / 6.20 s |
| `join_2` | `[:p{m} :follows ?f] [?f :name ?n]` (1 row) | 54.0 ms / 63.0 ms | 606 ms / 610 ms | 6.50 s / 7.02 s |
| `join_3_selective` | `[:p{m} :manager ?b] [?s :manager ?b] [?s :name ?n]` (10 rows) | 117 ms / 125 ms | 1.24 s / 1.28 s | 13.5 s / 13.8 s |
| `join_3_scan` | `[?e :follows ?f] [?f :group 7] [?f :name ?n]` | 150 ms / 155 ms | 1.92 s / 1.96 s | 20.9 s / 21.2 s |
| `recursive_ancestors` | `(boss :p{leaf} ?b)` over `:manager` (5–7 rows) | 2.71 s / 2.72 s | 40.1 s / 40.8 s | > 60 s (abandoned) |

`recursive_ancestors` at 1M was measured after the #528 fix, on the same machine and fixture. Below 100 runs, p99 is the slowest run. Shapes slower than about 75 ms ran fewer than 200 times; the full JSON report has run counts.

Peak RSS per shape, each in its own process (baseline 4 MiB):

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| point lookups, `:as-of`, `:valid-at`, hot entity | 4 MiB | 4 MiB | 4 MiB |
| `attribute_value` | 22 MiB | 137 MiB | 729 MiB |
| `attribute_scan` | 22 MiB | 169 MiB | 1,665 MiB |
| `count_grouped` | 24 MiB | 198 MiB | 1,783 MiB |
| `join_2` | 33 MiB | 307 MiB | 2,342 MiB |
| `join_3_selective` | 45 MiB | 376 MiB | 2,856 MiB |
| `join_3_scan` | 59 MiB | 456 MiB | 3,610 MiB |
| `recursive_ancestors` | 375 MiB | 3,259 MiB | 4,832 MiB (at abandonment) |

### Fixture

`N` facts over `N/6` entities `:p{i}`, six facts each: `:name "p{i}"`, `:age i%90`, `:group i%100`, `:follows :p{(7919·i+1) mod E}`, `:manager :p{i/10}` (a tree with fan-out 10), and `:status` in two valid-time windows (`"a"` for 2020–2023, `"b"` from 2023). Plus `:hot :counter`, retracted and reasserted 1,000 times. `m = E/2`. Every query drains its cursor with `next_batch(1000)`. Shapes run in their own process on the fixture opened read-only. The storage rows use a copy.

The harness builds with `--profile envelope`: `release` at opt-level 3, which is how an application's release build compiles the library. `release` and `bench` (which inherits it) are opt-level "z" for the REPL binary's size, and run these queries about 1.5× slower (`attribute_scan` at 1M facts: 703 ms against 462 ms). The Criterion numbers elsewhere in this file are opt-level "z".

### Where the time goes (1M facts)

`perf` with frame pointers on `attribute_scan` and `join_3_selective`:

- **Up-front materialization (50–60%).** Before matching, the executor turns every live fact of each pattern's attribute into a `Fact` (`selective_fact_fetch` → `live_scan`). A pattern without a bound entity is read in full, so a selective join costs as much as scanning its widest attribute. The streaming engine (#473: `Scan` and `IndexNestedLoopJoin` operators) replaces this.
- **DICT lookup per row (26–35%).** `uuid_of` does a B+tree `get` for every row's entity id (#530).
- **Bindings as `HashMap<String, Value>` (about 25%).** SipHash insert, lookup and drop per row; #473's columnar batches replace them.
- **`net_asserted_facts` over committed facts that `live_facts` already resolved (about 10%)** (#532).
- **Keyword constants hashed to a UUID per fact (about 8% of the join)** (#531).
- `attribute_value` reads the whole attribute instead of seeking AVET (#518). Fixed by the streaming engine's `Scan` (#544): see below.

### Streaming engine (#544)

Single-pattern queries run on the streaming engine since #544 (with expressions and pseudo-attributes since its second PR); a cursor pulls one batch of up to 1,024 rows at a time. A/B on an otherwise idle two-socket server, each run pinned to its own NUMA node, base and change on different sockets and swapped between rounds; `v3` before #544 PR 3 (ce2e320) against #544 PR 3, two rounds each, p50 shown (the table above is an older run on a different machine).

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| `attribute_value` p50 (both sides, since PR 1) | 24 µs | 22 µs | 22 µs |
| `attribute_scan` p50, `v3` → PR 3 | 24.0 → 23.2 ms | 268 → 248 ms | 3.12 → 2.59 s |
| `attribute_scan` peak RSS, `v3` → PR 3 | 11 → 6 MiB | 58 → 7 MiB | 458 → 7 MiB |
| point, `:as-of`, `:valid-at`, hot entity | unchanged (16–26 µs; hot 0.3 ms) | unchanged | unchanged |

`attribute_value` reads only the AVET range of its value since #544 PR 1 (#518): 22–24 µs at every scale here, where the table above, before #544, has 24.5 ms, 288 ms and 3.16 s. An attribute scan drained with `next_batch(1000)` now holds one batch: peak RSS no longer grows with the number of rows (1.67M rows at 10M facts in 7 MiB, against a 4 MiB baseline). Before PR 3 the cursor held the whole answer, and in PR 3's first commit the query's id cache still held every entity it returned (189 MiB at 10M). Joins, aggregates and rules still run on the executor and are unchanged (#545–#548).

### Python FFI round trip

**Command**: `python bench/envelope.py <fixture> --report envelope.json --scale <scale>` in [minigraf-python](https://github.com/project-minigraf/minigraf-python) (`v3`), on the same fixtures and report. The binding is built with `maturin develop --release` against this commit. It runs the same queries through `db.query(...)`, drains them with `next_batch(1000)`, and decodes each JSON batch with `json.loads`. Python 3.14.7.

| Shape | Rust p50 (100K / 1M / 10M) | Python p50 (100K / 1M / 10M) | Python peak RSS at 10M |
|---|---:|---:|---:|
| `point_entity` | 13 µs / 12 µs / 12 µs | 77 µs / 84 µs / 88 µs | 21 MiB |
| `hot_current` | 263 µs / 256 µs / 268 µs | 330 µs / 326 µs / 328 µs | 21 MiB |
| `attribute_scan` | 43.4 ms / 462 ms / 4.83 s | 49.4 ms / 537 ms / 5.73 s | 1,601 MiB |
| `count_grouped` | 46.3 ms / 521 ms / 5.66 s | 46.0 ms / 506 ms / 5.45 s | 1,736 MiB |
| `join_3_scan` | 150 ms / 1.92 s / 20.9 s | 150 ms / 1.92 s / 21.3 s | 3,372 MiB |

The binding adds about 70 µs per query, plus about 0.5 µs per returned row for the JSON batches (0.9 s for 1.67M rows). The interpreter adds about 16 MiB of RSS. Peak RSS is otherwise the cursor's: the answer is computed when the cursor opens. #473 re-measures this table as an acceptance test for the streaming engine.

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
# The performance envelope (#394): builds and caches fixtures in target/perf-envelope
# (10M facts: about 1.2 GiB on disk, 3 minutes to build, up to 5 GiB RSS while measuring).
cargo run --profile envelope --example perf_envelope -- --scales 100k,1m,10m --json envelope.json

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
- The Criterion snapshots do not include heap or RSS; the performance envelope reports RSS from `/proc` (Linux only). The weekly soak test (#392) also records RSS, file size, and open and checkpoint time at 10M facts.
