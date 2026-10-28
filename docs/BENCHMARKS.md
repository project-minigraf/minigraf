# Minigraf Benchmarks

This document records reproducible local benchmark snapshots. Completed release-performance history belongs in the [CHANGELOG](../CHANGELOG.md); continuous CI history is available on [Bencher](https://bencher.dev/perf/minigraf/plots).

## One-group count and distinct count (#641, slice 2, unreleased)

**Date**: 2026-10-28 · **Parent**: slice 1 at `366f662df` · **Change**: #641,
second slice. The [build manifest](benchmarks/641-ordered-groups-build.json)
records the matched library, source, dependency-lock and probe fingerprints.
Both libraries use opt-level 3 (`--profile envelope`), frame pointers and Rust
1.99.1. The final appended native cache regression leaves the production library
byte-identical to the measured build.

Six balanced rounds run on an idle bench, with CPU and memory bound together
and the nodes and execution order swapped each round. Each query has its own
process and a 256 KiB worker stack. One warmup precedes 3–20,000 timed drains
within a one-second budget (at least three for slow queries). Tables report the
median of six process medians and the maximum of six process RSS peaks. Open and
fixture construction are outside timing; fixtures are prebuilt and read-only.
The existing envelope/history guards use their unchanged probe (three warmups,
at least five timed drains per process). All six process samples per case are retained, including each median, range
and run count.

The group fixture has exactly N committed facts: one integer `:group` per UUID
entity, `floor(i / 16)` for fixed groups. There are 6,250 / 62,500 / 625,000 groups
at 100K / 1M / 10M rows, each of size 16. Counts read `?e`; distinct counts read
its distinct entity ids. Each group-fixture probe checks the ordered group key and aggregate
value, as well as row count. The materialized `sum-distinct` control binds the
constant 1 and sums its distinct value per group, checking a result of 1.

| Committed rows | Count full drain, ms (parent → change) | Distinct full drain, ms | Count limit 10, ms | Distinct limit 10, ms |
|---:|---:|---:|---:|---:|
| 100K | 50.395 → 48.880 | 63.671 → 60.007 | 48.157 → 0.652 | 60.922 → 0.762 |
| 1M | 555.213 → 484.644 | 753.668 → 597.925 | 520.168 → 0.645 | 713.856 → 0.755 |
| 10M | 7,397.820 → 4,925.882 | 10,111.836 → 6,042.124 | 6,951.004 → 0.644 | 9,522.604 → 0.758 |

| Committed rows | Count full RSS, KiB (parent → change) | Distinct full RSS, KiB | Count limit RSS, KiB | Distinct limit RSS, KiB |
|---:|---:|---:|---:|---:|
| 100K | 10,080 → 5,072 | 13,416 → 5,052 | 9,912 → 3,840 | 13,460 → 3,816 |
| 1M | 59,664 → 5,040 | 87,888 → 5,008 | 56,648 → 3,848 | 87,972 → 3,812 |
| 10M | 500,376 → 5,188 | 904,552 → 5,204 | 506,456 → 3,816 | 894,320 → 3,860 |

The bounded limit 10M/100K time ratios are **0.988** and **0.994**, below the 2x gate. Across the four fixed-group shapes, the largest 10M RSS increase over 100K is **152 KiB**, within the 16 MiB allowance. At 10M, full-drain count is **33.41%** faster and distinct count **40.25%** faster. All [252 grouped records](benchmarks/641-ordered-groups.jsonl) are retained.

### Holder and early-stop gates

| Committed rows | Count peak | Distinct-count peak | Active groups | Scan rows, limit 10 |
|---:|---:|---:|---:|---:|
| 100K | 480 B | 1,120 B | 1 | 1,024 |
| 1M | 480 B | 1,120 B | 1 | 1,024 |
| 10M | 480 B | 1,120 B | 1 | 1,024 |

These are query-meter peaks, including IdSpace, from the
[24 diagnostic records](benchmarks/641-ordered-groups-meter.jsonl). Both count
modes release every completed group and retain no finished-results list. Zero
peak growth fits the 100K baseline plus two charged batches; the test computes
that allowance from `Batch`, `Vec<Cell>` and `cell_bytes` for the fixture's two
columns, rather than a guessed byte count. The requested groups contain 160 rows;
1,024 reads fit 160 + `BATCH_ROWS`. A probe-join regression checks the same bound
at Aggregate's input (the join may also hold an outer batch).

Input and output batches that pass within one operator pull follow the engine's
existing uncharged-batch convention. No batch is retained between calls by the
ordered Aggregate. The active group, distinct members, finish scratch and a
result row until it moves into the passing output batch remain charged. Internal
child-batch boundaries evict DICT lookup caches even when one group spans many
pulls; a targeted regression fails when that eviction is disabled.

### A single growing group

| Committed rows | Count peak | Distinct-count peak |
|---:|---:|---:|
| 100K | 480 B | 4,000,480 B |
| 1M | 480 B | 40,000,480 B |
| 10M | 480 B | 400,000,480 B |

The linear distinct-member control is expected. A limit cannot finish this one
group before it has read every member. Other holders, provisional ids and final
sorting can still dominate query memory; mixed integer/float grouping keeps its
numeric Sort, and fallible aggregates retain full-input validation.

| Committed rows | One-group count, ms (parent → change) | One-group distinct count, ms | Materialized sum-distinct control, ms |
|---:|---:|---:|---:|
| 100K | 39.487 → 44.752 | 53.622 → 58.114 | 61.088 → 62.489 |
| 1M | 401.147 → 447.729 | 639.518 → 674.656 | 679.843 → 690.527 |
| 10M | 4,066.914 → 4,519.806 | 6,921.283 → 7,348.321 | 7,344.431 → 7,553.146 |

For one growing group, count is **11.14–13.33%** slower and distinct count **5.49–8.38%** slower. The ordered path checks its active key on every input row, and this control releases no completed groups until EOF. No cost-based strategy selection is added. The materialized sum-distinct control changes by **+1.57–2.84%**; its memory still grows with the number of groups. These costs are separate from the bounded-limit and ordinary-point gates.

### Existing query guards and verification

The ordinary envelope entity/exact-value/hot-point deltas are **−1.16% to +1.60%**. The exact #379 history guards have a worst delta of **+0.68%**, below 5%; all 12 workload/depth combinations pass. Full scan, plain limit, UUID-tie and descending top-k controls span **−2.40% to +2.41%**. The earlier ordered value limit/offset controls span **-3.24% to +0.32%**. The existing ordered selective-point startup cost remains visible separately:

| Nominal envelope facts | Ordered point parent | Change |
|---:|---:|---:|
| 100K | 130.22 µs | 130.22 µs |
| 1M | 112.67 µs | 112.57 µs |
| 10M | 117.34 µs | 119.28 µs |

The eight optional whole-attribute proof seeks are unchanged from slice 1. All [360 envelope/control records](benchmarks/641-ordered-groups-guards.jsonl) and [144 history records](benchmarks/641-ordered-groups-history.jsonl) are retained. No ordinary query without an applicable demand makes proof reads.

The newly eligible grouped queries also pay that proof cost when there was no previous `:order-by`. On a one-row entity range:

| Nominal envelope facts | Grouped count parent → change | Grouped distinct count parent → change |
|---:|---:|---:|
| 100K | 23.19 → 133.54 µs | 24.03 → 134.88 µs |
| 1M | 22.94 → 118.55 µs | 24.80 → 121.45 µs |
| 10M | 22.99 → 120.19 µs | 24.49 → 121.67 µs |

These use `[:find ?a (count ?a) :where [:pM :age ?a]]` and its `count-distinct` variant, where M is the midpoint entity. EAVT remains selective, but the eight whole-attribute seeks add roughly **96–111 µs**. The [72 startup records](benchmarks/641-ordered-groups-startup.jsonl) use the unchanged guard probe (three warmups, at least five timed drains per process, six balanced process samples). A tiny group gets little benefit from releasing completed groups; there is no cost-based selection. This is reported separately from ordinary points with no applicable consumer demand.

The final default test set is 1,947 passing / 16 ignored (the full instrumented
suite plus rustdoc and the appended filtered cache regression); with test-hooks,
1,980 pass / 17 are ignored. The full default and hooks runs preceded the final
native-only cache test, which passed separately in both configurations and was
added to the preserved coverage report. All unchanged floors pass: **95.40%**
lines / **86.38%** branches. Native, browser and WASI clippy pass; WASI has 1,179
passing / 7 ignored tests, Firefox has 60 passing tests, and the WASI REPL builds
in release mode. Frozen answers are unchanged. The updated ignored million-group
acceptance test passes: ordered count fits its 32 MiB budget, while grouped `sum`
remains a materialized `QRY-012` control.

The default release binary is **1,259,368 bytes**, below the unchanged
**1,259,520-byte** cap. This slice changes no storage/key encoding, dependency,
public API or file format. Window buffering is addressed by the third slice.

### Reproduction and nightly routing

Compile the [fixture builder](../scripts/input_order_group_fixture.rs) against
the parent library and the same [group probe](../scripts/input_order_group_probe.rs)
against both libraries, using the rustc/envelope flags in the slice-1 recipe.
Run `group_fixture <new.graph> N 16`, then, for example:

```sh
group_probe groups.graph '(query [:find ?g (count ?e) :where [?e :group ?g] :order-by [?g] :limit 10])' 10 16
```

Use group size N instead of 16 for the single-growing-group control. Build
fixtures in separate processes and alternate CPU/memory binding and library
execution order over six rounds. Raw records contain every complete query.

All sixteen Criterion cases execute with
`cargo bench --bench minigraf_bench -- '^query/ordered_groups/' --test`:
`query/ordered_groups/{count,distinct,count_limit,distinct_limit}/{1k,10k,100k,1m}`.
The existing nightly `query` matrix's `^query/` route selects every new ID (#776).
Scheduled Bencher history is available after merge; none is claimed here.


## Proven input order and Sort omission (#641, slice 1, unreleased)

**Date**: 2026-10-28 · **Base**: `v3` at `2abf53df` · **Change**: #641,
first slice. The [build manifest](benchmarks/641-input-order-build.json) records
source, dependency-lock, library and probe fingerprints. Both libraries used
`--profile envelope` (opt-level 3) and `-C force-frame-pointers=yes`, with Rust
1.99.1 and the same dependency lock.

Six rounds ran on an idle benchmark machine, binding CPU and memory to the same
NUMA node and swapping nodes and execution order between base and change each
round. Every query ran in a fresh process on a prebuilt, checkpointed file opened
read-only, on a 256 KiB worker stack. Three warmups preceded 5–20,000 measured
cursor drains within a one-second budget (at least five even for slower queries).
Open, fixture construction and profiling are outside timing. Tables give the
median of six process p50s; raw records retain each p50, its per-process range,
run count and query-only process peak RSS. No samples were discarded.

The common envelope fixtures use `floor(N / 6)` ordinary entities and
approximately `N` currently visible facts, plus expired valid-time records and
hot-entity history. The
scale labels are nominal current-fact counts. The age queries read 16,666,
166,666 and 1,666,666 rows respectively. Every measured result checks its row
count; the ordered limit/offset probes also check that all returned ages are zero.

| Nominal current facts | Ordered limit: base | Change | Ordered offset: base | Change | Change limit max peak RSS |
|---|---:|---:|---:|---:|---:|
| 100K | 5.996 ms | 0.458 ms | 6.022 ms | 0.457 ms | 3,784 KiB |
| 1M | 62.260 ms | 0.455 ms | 62.739 ms | 0.457 ms | 3,776 KiB |
| 10M | 643.947 ms | 0.463 ms | 644.929 ms | 0.452 ms | 3,796 KiB |

The RSS column is the maximum of the six process peaks.

The limit query is `[:find ?a :where [?e :age ?a] :order-by [?a] :limit 10]`;
the offset query adds `:offset 13`. At 10M, the changed p50 ranges were
0.459–0.476 ms and 0.448–0.459 ms. The 10M/100K ratios are **1.012** and **0.989**,
within the 2x gate. Peak RSS increases by **12 KiB** and **40 KiB**, within the
16 MiB allowance. Internal tests separately verify Sort omission, restored
projection slots, input read at most `offset + limit + 1,024` rows on the positive
fixture, at most eight raw kind-prefix seeks, and zero proof reads for zero limit
including a nonzero offset.

### Startup cost on a selective ordered entity range

The entity-and-attribute range remains EAVT. Its order proof still makes eight
whole-attribute kind-prefix seeks, whose startup cost dominates a one-row query:

| Nominal current facts | Base | Change |
|---|---:|---:|
| 100K | 21.13 µs | 129.89 µs |
| 1M | 20.72 µs | 112.29 µs |
| 10M | 20.99 µs | 117.39 µs |

Query: `[:find ?a :where [:pM :age ?a] :order-by [?a] :limit 1]`, where
`M = floor(entity_count / 2)`. This cost is reported separately from the ordinary
point-query guards. There is no cost-based selection or cross-query proof cache
in this slice.

### Existing point and buffering controls

The envelope's ordinary entity point, exact attribute/value point and current
hot-history point changed by **−0.65% to +1.03%** across the three scales. The
attribute scan, plain limit, ascending UUID-tie Sort and descending top-k control
changed by **−3.28% to +2.00%**. All measurements are in the
[360 envelope records](benchmarks/641-input-order.jsonl).

The exact #323/#379 history fixture and four query shapes were also measured
through the same cursor probe: 2,000 filler facts, a never-changed sibling
attribute, and one live string value after `depth - 1` retract/reassert pairs.
The `:as-of 2002` query selects the first assertion with `:any-valid-time`.

| Guard | Depth | Base p50 | Change p50 | Delta |
|---|---:|---:|---:|---:|
| Churned attribute | 1 | 16.83 µs | 16.70 µs | −0.80% |
| | 500 | 151.83 µs | 153.69 µs | +1.23% |
| | 2,000 | 602.70 µs | 599.46 µs | −0.54% |
| Sibling attribute | 1 | 17.12 µs | 17.10 µs | −0.15% |
| | 500 | 22.53 µs | 22.58 µs | +0.24% |
| | 2,000 | 22.85 µs | 22.81 µs | −0.17% |
| Attribute scan | 1 | 17.26 µs | 17.35 µs | +0.50% |
| | 500 | 152.62 µs | 152.65 µs | +0.02% |
| | 2,000 | 595.38 µs | 603.65 µs | +1.39% |
| Churned attribute, as-of | 1 | 17.85 µs | 17.76 µs | −0.48% |
| | 500 | 141.32 µs | 141.59 µs | +0.19% |
| | 2,000 | 554.11 µs | 560.41 µs | +1.14% |

The worst guard delta is **+1.39%**, within the 5% gate. All
[144 history records](benchmarks/641-input-order-history.jsonl) are retained.

### Reproduction and nightly routing

Build the immutable base and the change once, then compile the same
[query probe](../scripts/input_order_probe.rs) against each library:

```sh
RUSTFLAGS='-C force-frame-pointers=yes' cargo build --profile envelope --example perf_envelope
rustc --edition=2024 -C opt-level=3 -C panic=abort -C force-frame-pointers=yes \
  -L dependency="$target_dir/envelope/deps" \
  --extern minigraf="$target_dir/envelope/deps/libminigraf.rlib" \
  scripts/input_order_probe.rs -o "$probe_out"
```

Build envelope fixtures separately with `perf_envelope __build <new.graph> N`.
Run `probe <fixture.graph> '(query [...])' <expected_rows> [zeros]` with alternating
CPU/memory binding as described above. Build the
[history fixture helper](../scripts/input_order_history_fixture.rs) against the
base library with the same rustc command; run it with `<new.graph> <depth>`.
Raw records contain each complete query.

All eight Criterion cases were executed on the bench with
`cargo bench --bench minigraf_bench -- '^query/ordered_values/' --test`:
`query/ordered_values/{index,uuid_ties}/{1k,10k,100k,1m}`. The existing `query`
nightly matrix uses `^query/`, which selects every new ID (#776); no workflow
change is needed. Scheduled Bencher history becomes available after merge.
`perf_envelope` also includes `ordered_values` beside its existing `top_k` control.

The ordinary release binary is **1,254,928 bytes**, below the unchanged
**1,259,520-byte** cap (base: 1,258,784). Sharing the existing set/window sort
comparators removes duplicate generated code while preserving their order.
At this snapshot, only Sort selection changed; Aggregate and Window still
buffered their full input. The later #641 sections describe the other slices.

## Projected UUID lookup batching (#530, unreleased)

**Date**: 2026-10-27 · **Baseline**: `v3` at `80120758` · **Change**: #530.
Two rounds on an idle benchmark server, CPU and memory bound to the same NUMA
node, running baseline then change and reversing the order in round two. Builds
and tests finished before timing. Both libraries used `--profile envelope`
(opt-level 3) and `-C force-frame-pointers=yes`. Queries opened checkpointed files
read-only and warmed their caches. Profiling ran separately. The tables show the
mean of each broad round's p50; [raw results](benchmarks/530-uuid-projection.jsonl)
retain run counts, maxima and the envelope's process-wide peak RSS, plus the
repeated-ref follow-up described below.

| Workload | Baseline p50 | Change p50 | Latency delta |
|---|---:|---:|---:|
| Attribute scan, 1M facts / 166,666 rows | 261.81 ms | 135.55 ms | -48.2% |
| Point lookup, 1M facts / 1 row | 19.08 µs | 18.23 µs | -4.4% |
| Sparse entities, 1,000 rows | 3.09 ms | 2.91 ms | -5.8% |
| Descending refs, 100,000 rows | 163.51 ms | 89.16 ms | -45.5% |
| Random refs, 100,000 rows | 388.97 ms | 279.13 ms | -28.2% |
| Repeated refs, 100,000 rows | 46.37 ms | 50.17 ms | +8.2% |
| Two entity columns, 100,000 rows | 524.05 ms | 474.02 ms | -9.5% |
| Duplicate columns, 100,000 rows | 396.10 ms | 284.36 ms | -28.2% |

The original 1M-fact envelope scan has **48.2% lower p50 latency**. Its process
peak RSS remains about 7 MiB in both versions, about 0.2 MiB higher in the changed
runs. Point lookups remain around 18–19 µs.
The broad repeated-ref rounds varied, including one slower changed run. Four
additional interleaved pairs measured 45.55 → 45.78 ms (+0.5%); those records have
`phase: repeated_refs_recheck` and `trial` instead of `round` in the raw file.
The initial +8.2% result remains in the table and raw rounds.

The reference queries use a separate 400,000-fact fixture: 100,000 UUID entities
with `:friend` pointing to `(i * 7919 + 1) % 100000`, `:boss` to `i / 100`,
`:reverse` to `99999 - i`, and `:bucket` equal to `i % 100`. Subject ids are assigned
and checked in order before any ref facts are added, making `:reverse` strictly
descending in internal-id order. Expected row counts are checked on every run.
Query workers use a 256 KiB stack, including the two-column and duplicate cases.

| Probe | Query |
|---|---|
| Sparse entities | `[:find ?e :where [?e :bucket 7]]` |
| Descending refs | `[:find ?f :where [?e :reverse ?f]]` |
| Random refs | `[:find ?f :where [?e :friend ?f]]` |
| Repeated refs | `[:find ?f :where [?e :boss ?f]]` |
| Two entity columns | `[:find ?e ?f :where [?e :friend ?f]]` |
| Duplicate columns | `[:find ?f ?f :where [?e :friend ?f]]` |

A separate 8,895-fact fixture assigns 8,192 subject ids before adding Boolean
markers for the cutoff probes. A chunk is dense at 64 or more distinct ids with
numeric span at most twice its count. Small/sparse chunks use scalar exact reads;
dense chunks walk leaves. The 64-id dense cutoff adds about 7.6 µs here; the small
and sparse cases improve. This is a measured cost of the fixed cutoff.

| Cutoff probe | Baseline p50 | Change p50 | Latency delta |
|---|---:|---:|---:|
| 63 ids, span 62 | 104.69 µs | 101.05 µs | -3.5% |
| 64 ids, span 128 (dense) | 105.62 µs | 113.27 µs | +7.2% |
| 64 ids, span 129 (sparse) | 116.87 µs | 115.45 µs | -1.2% |
| 512 ids, stride 16 (sparse) | 0.73 ms | 0.71 ms | -2.1% |

Separate user-cycle profiles of the 1M scan put scalar `DictReader::uuid_of` at
58.5% before the change and batched `DictReader::uuids_of` at 20.9% afterward.
Profile reports are retained locally.
Dense batches reuse the existing leaf cursor; sparse/small batches retain scalar
restart-point searches without a temporary per-id memo map. Sorted hints are
bounded to 1,024 distinct committed ids and keep cache/meter changes in projection
order. Joins, collect-set and sort/aggregate conversions retain their scalar paths.

To reproduce, build both revisions once with the envelope profile and frame
pointers. Compile the **same** [probe source](../scripts/uuid_projection_probe.rs)
against each revision's library (set `target_dir`, `probe_src` and `probe_out`
for that revision):

```bash
rustc --edition=2024 -C opt-level=3 -C panic=abort -C force-frame-pointers=yes \
  -L dependency="$target_dir/envelope/deps" \
  --extern minigraf="$target_dir/envelope/deps/libminigraf.rlib" \
  "$probe_src" -o "$probe_out"
```

Use a fresh scratch directory, place the two envelope/probe executables there as
`baseline-envelope`, `changed-envelope`, `baseline-probe` and `changed-probe`, and
build the common fixtures outside the timed phase. For example, with `run_dir`
pointing to that directory and `bench_node` selecting one NUMA node:

```bash
"$run_dir/baseline-envelope" __build "$run_dir/fixture-1m.graph" 1000000 \
  > "$run_dir/fixture-1m.json"
"$run_dir/baseline-probe" build "$run_dir/refs.graph" 100000
"$run_dir/baseline-probe" build-boundaries "$run_dir/boundaries.graph" 8192
numactl --cpunodebind="$bench_node" --membind="$bench_node" \
  "$run_dir/changed-envelope" __query "$run_dir/fixture-1m.graph" 2 8 60
numactl --cpunodebind="$bench_node" --membind="$bench_node" \
  "$run_dir/changed-probe" query "$run_dir/refs.graph" \
  '(query [:find ?f :where [?e :friend ?f]])' 100000
numactl --cpunodebind="$bench_node" --membind="$bench_node" \
  "$run_dir/changed-probe" query "$run_dir/boundaries.graph" \
  '(query [:find ?e :where [?e :dense-threshold true]])' 64
```

Run every case with both pairs sequentially, reversing the order for round two.
The envelope's arguments `2 8 60` select attribute scan, an eight-second budget
and a 60-second cap check; use `0 2 60` for point lookup. It warms three runs and
measures up to 200 runs or its budget, at least five. The reference probe warms
three runs and measures up to 20,000 runs or five seconds, at least five. The
higher cap gives small cutoff queries seconds of samples. For the repeated-ref
follow-up, run that query four additional pairs with alternating order. Remove
fixtures, executables and build outputs after retaining reports and CI passing.

## Bounded row frames (#183, unreleased)

**Date**: 2026-10-27 · **Branch**: `feat/183-row-frames` on `v3` · **Machine**: Intel Xeon E5-2696 v4, Linux 6.8, Rust 1.99.1, idle during timing. **Reproduction command**: `numactl --cpunodebind=1 --membind=1 cargo bench --bench minigraf_bench -- window_rows --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2 --nresamples 1000 --noplot`. The recorded run invoked the prebuilt Criterion binary directly with its `--bench` switch after checking the machine was idle. Criterion extended the measurement time for slow cases to obtain ten samples. The committed harness uses the same in-memory facts for a default-frame and each bounded-frame comparison. Each query scans and joins the facts, sorts window input, evaluates one aggregate, and materializes the result.

The [per-case CSV](benchmarks/183-row-frames.csv) contains the median and full range of the ten per-call samples, plus the peak bytes charged to the query engine's memory meter. The peak comes from the ignored `window_rows_benchmark_meter_peaks` diagnostic using identical fixtures and queries. It is a separate run from Criterion. The timed benchmark process's maximum RSS was 1,671,892 KiB across both fixture sizes; this is a process-wide high-water mark, not a per-query peak. The engine meter omits unrelated process and user-function allocations.

| Built-in gate | Worst observed | Limit |
|---|---:|---:|
| 100,000 rows: bounded/default median, same function and fixture (`min`, one partition, prefix) | 1.010× | 5× |
| 100,000/10,000 row median, same function, frame and partition shape (`sum`, 100 partitions, `2 preceding`) | 12.885× | 15× |

At 100,000 rows in one partition, a zero-width `min` frame peaked at 25,600,040 engine-meter bytes, only 8 bytes more than the default frame. Its prefix-covering frame peaked at 26,400,032 bytes, an 800,000-byte deque charge. The corresponding 100-partition deque charges were 8 and 8,000 bytes. This confirms deque memory follows the frame's maximum width rather than always reserving the whole partition.

The UDF benchmark registers an integer sum with a fresh accumulator per bounded frame. Its middle-sized frames deliberately use replay; a prefix-covering frame uses one running accumulator. Callback counts follow from the frame widths and partition sizes.

| Rows | Partitions | Frame | Callbacks | Median (ms) |
|---:|---:|---|---:|---:|
| 10,000 | 1 | `0 preceding` | 10,000 | 38.152 |
| 10,000 | 1 | `2 preceding` | 29,997 | 38.275 |
| 10,000 | 1 | `5,000 preceding` | 37,507,500 | 611.494 |
| 10,000 | 100 | `0 preceding` | 10,000 | 38.478 |
| 10,000 | 100 | `2 preceding` | 29,700 | 38.786 |
| 10,000 | 100 | `50 preceding` | 382,500 | 41.717 |
| 100,000 | 1 | prefix-covering | 100,000 | 459.714 |
| 100,000 | 100 | prefix-covering | 100,000 | 467.106 |

The harness omits the quadratic 100,000-row, half-partition UDF case. The large-case query scan and result materialization dominate the built-in timings; this snapshot checks relative growth of this complete query shape, not isolated deque or sum-step throughput.

## Performance Envelope (v3.0.0, #394)

**Version**: `v3` branch (format v8), unreleased · **Date**: 2026-10-11 · **Command**: `cargo run --profile envelope --example perf_envelope -- --scales 100k,1m,10m` · Intel Core i7-1065G7 (4 cores / 8 threads, 16 GB RAM), Rust 1.94.0, Linux 6.12.108, on AC power and otherwise idle; the `.graph` file on an SSD with the OS page cache warm.

This section lists the graph sizes and query shapes that are tested, with their p50 and p99 latency, and the ones that are not supported yet. Re-run it for each minor release (CONTRIBUTING.md).

### What is supported

Every query runs on the streaming engine since #551. The figures in this table are from the #551 A/B on an idle server (below); the storage and latency tables after it are the laptop run of 2026-10-11, before the engine.

| Workload | Tested up to | p50 / p99 at 10M facts | Notes |
|---|---:|---:|---|
| Point lookup: bound entity and attribute, including `:as-of` a counter and `:valid-at` | 10M facts | 18–20 µs / 22–27 µs | Flat from 100K to 10M |
| Point lookup `:as-of` a time | 10M facts | 0.92 ms / 0.93–0.96 ms | Resolving the time reads the DICT's transactions: 0.7 ms at 100K |
| Entity with a long history (1,000 versions), current and `:as-of` | 10M facts | 0.29–0.30 ms / 0.32–0.37 ms | Grows with that entity's history, not the graph |
| Attribute + value, joins that probe by entity or value | 10M facts | 17 µs–0.2 ms | `attribute_value`, `join_2`, `join_3_selective` |
| Open a checkpointed file | 10M facts | 0.13–0.23 ms, 3 MiB RSS | Indexes are paged in on demand |
| Checkpoint after 1 new fact | 10M facts | 1.1–1.5 ms / 2.2–2.9 ms | Copy-on-write: cost follows the change |
| Attribute scan, aggregates, `not`, `or`, joins that scan | 10M facts | see below | Linear in the facts they read, in bounded memory: 0.3–0.9 s for `not`/`or`/`count_grouped`, 2.5 s for a 1.67M-row scan in 7 MiB, 13 s for `join_3_scan` in 62 MiB |
| Rule queries (recursion) | 10M facts | see below | A rule with a constant (magic sets) takes 1.9 ms; a whole transitive closure grows with the facts it derives, about 5 µs and 250 bytes of RSS per derived fact |
| Window functions | 10M facts | 5.9–6.0 s | A window holds its whole input: 0.9 GiB for 1.67M rows |

**Not supported yet:**

- Graphs over 10M facts are not tested.
- Interactive latency for queries that read a whole attribute at 10M facts: 0.3–14 s. Their memory is bounded per operator, except windows and rule relations, which hold their rows; `OpenOptions::query_memory_limit` stops a query that passes it with `QRY-012` (#549).

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
| `limit` | `[:find ?e ?a :where [?e :age ?a] :limit 10]` (#306) | 307 µs / 502 µs | 317 µs / 348 µs | 318 µs / 391 µs |
| `join_2` | `[:p{m} :follows ?f] [?f :name ?n]` (1 row) | 54.0 ms / 63.0 ms | 606 ms / 610 ms | 6.50 s / 7.02 s |
| `join_3_selective` | `[:p{m} :manager ?b] [?s :manager ?b] [?s :name ?n]` (10 rows) | 117 ms / 125 ms | 1.24 s / 1.28 s | 13.5 s / 13.8 s |
| `join_3_scan` | `[?e :follows ?f] [?f :group 7] [?f :name ?n]` | 150 ms / 155 ms | 1.92 s / 1.96 s | 20.9 s / 21.2 s |
| `recursive_ancestors` | `(boss :p{leaf} ?b)` over `:manager` (5–7 rows) | 2.71 s / 2.72 s | 40.1 s / 40.8 s | > 60 s (abandoned) |

These are the executor's figures, before the streaming engine; the engine's are in the sections below, and every shape on today's code in [Executor removal (#551)](#executor-removal-551). `recursive_ancestors` at 1M was measured after the #528 fix, on the same machine and fixture. Below 100 runs, p99 is the slowest run. Shapes slower than about 75 ms ran fewer than 200 times; the full JSON report has run counts.

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

On the executor, before #544 (the executor's query path is gone since #551). `perf` with frame pointers on `attribute_scan` and `join_3_selective`:

- **Up-front materialization (50–60%).** Before matching, the executor turns every live fact of each pattern's attribute into a `Fact` (`selective_fact_fetch` → `live_scan`). A pattern without a bound entity is read in full, so a selective join costs as much as scanning its widest attribute. The streaming engine (#473: `Scan` and `IndexNestedLoopJoin` operators) replaces this.
- **DICT lookup per row (26–35%).** `uuid_of` does a B+tree `get` for every row's entity id (#530).
- **Bindings as `HashMap<String, Value>` (about 25%).** SipHash insert, lookup and drop per row; #473's columnar batches replace them.
- **`net_asserted_facts` over committed facts that `live_facts` already resolved (about 10%)** (#532). Superseded by #551's removal of the executor's query path; see the [source audit and fresh envelope/profiles](benchmarks/532-net-assert-audit.md). Pending records still need fact-level net-assert.
- **Keyword constants hashed to a UUID per fact (about 8% of the old join)** (#531). The streaming engine resolves pattern constants while planning; see the [post-#551 audit and profiles](benchmarks/531-keyword-constants.md). Expression keyword/ref equality retains a separate per-row conversion described there.
- `attribute_value` reads the whole attribute instead of seeking AVET (#518). Fixed by the streaming engine's `Scan` (#544): see below.

### Streaming engine (#544)

Single-pattern queries run on the streaming engine since #544 (with expressions and pseudo-attributes since its second PR); a cursor pulls one batch of up to 1,024 rows at a time. A/B on an otherwise idle two-socket server, each run pinned to its own NUMA node, base and change on different sockets and swapped between rounds; `v3` before #544 PR 3 (ce2e320) against #544 PR 3, two rounds each, p50 shown (the table above is an older run on a different machine).

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| `attribute_value` p50 (both sides, since PR 1) | 24 µs | 22 µs | 22 µs |
| `attribute_scan` p50, `v3` → PR 3 | 24.0 → 23.2 ms | 268 → 248 ms | 3.12 → 2.59 s |
| `attribute_scan` peak RSS, `v3` → PR 3 | 11 → 6 MiB | 58 → 7 MiB | 458 → 7 MiB |
| point, `:as-of`, `:valid-at`, hot entity | unchanged (16–26 µs; hot 0.3 ms) | unchanged | unchanged |

`attribute_value` reads only the AVET range of its value since #544 PR 1 (#518): 22–24 µs at every scale here, where the table above, before #544, has 24.5 ms, 288 ms and 3.16 s. An attribute scan drained with `next_batch(1000)` now holds one batch: peak RSS no longer grows with the number of rows (1.67M rows at 10M facts in 7 MiB, against a 4 MiB baseline). Before PR 3 the cursor held the whole answer, and in PR 3's first commit the query's id cache still held every entity it returned (189 MiB at 10M). Aggregates and rules still run on the executor and are unchanged (#546–#548). Joins: see below.

### Joins on the streaming engine (#545)

Queries of several patterns run on the engine since #545 PR 2: an `IndexNestedLoopJoin` when the next pattern's entity is bound, a `HashJoin` otherwise. A/B on the same idle server, unpinned, `v3` before PR 2 (eab15bb) and PR 2 run one after the other, two rounds each; p50 of the two rounds, peak RSS per shape.

| Shape | 100K | 1M |
|---|---:|---:|
| `join_2` p50, `v3` → PR 2 | 58.2 ms → 51 µs | 796 ms → 50 µs |
| `join_2` peak RSS | 38 → 4 MiB | 322 → 4 MiB |
| `join_3_selective` p50 | 125 ms → 10.6 ms | 1.67 s → 148 ms |
| `join_3_selective` peak RSS | 40–41 → 7 MiB | 392–462 → 14 MiB |
| `join_3_scan` p50 | 165 ms → 41.1 ms | 2.64 s → 1.12 s |
| `join_3_scan` peak RSS | 48–67 → 14 MiB | 468–480 → 42 MiB |
| point, `:as-of`, `:valid-at`, hot entity, attribute scan | unchanged | unchanged |

`join_2` probes one entity's range for its one outer row. `join_3_selective` joins on the value position, so it is a `HashJoin` that reads every `:manager` fact into its table: a value probe would need AVET over pending facts, which the pending store does not have (decision 3 of the #545 plan). `join_3_scan` scans the AVET range of `:group 7`, joins `:follows` on its value with a `HashJoin` built on those rows (it streams every `:follows` fact), then probes `:name` per row.

At 10M facts with PR 3 (same server, two rounds; the PR 2 code is unchanged for these shapes, and both builds agree within 2%): `join_2` 51 µs and 4 MiB; `join_3_selective` 2.48–2.52 s and 66 MiB, its `HashJoin` table holding the 10M-fact graph's 1.67M `:manager` facts; `join_3_scan` 13.6 s and 59–63 MiB. The 10M figures in the first table above, before #545 and on the laptop, are 6.50 s, 13.5 s and 20.9 s with 2.3–3.6 GiB of RSS.

#### Pseudo-attributes across joins (#545 PR 3)

`join_2_pseudo` is `[:p{m} :follows ?f] [?f :name ?n] [?f :db/tx-count ?t]` with `:valid-at :any-valid-time` (1 row). Before PR 3 a pseudo-attribute pattern in a query of several patterns sent the whole query to the executor, which materialises every fact. A/B on the same idle server, `v3` (86ad20b, with the envelope shape added) on one socket and PR 3 on the other, swapped for the second round; p50 of each round, peak RSS.

| `join_2_pseudo` | 100K | 1M | 10M |
|---|---:|---:|---:|
| p50, `v3` → PR 3 | 211 ms → 40–58 µs | 3.19 s → 55–56 µs | 33.8–34.1 s → 56 µs |
| peak RSS | 122 → 4 MiB | 857–858 → 4 MiB | 6.0–7.5 GiB → 4 MiB |
| every other shape | unchanged | unchanged | unchanged |

The pseudo-attribute pattern reads the `:db/tx-count` column that the `:name` probe fills for its one row.

### Aggregates on the streaming engine (#547 PR 2)

`count` is `[:find (count ?v) :where [?e ?a ?v]]` (every fact, 1 row); `count_grouped` is `[:find ?g (count ?e) :where [?e :group ?g]]` (100 rows). Before PR 2 every aggregate query ran on the executor, which holds every binding before grouping. A/B on the same idle server, `v3` (234cc3e, with the `count` shape added) then PR 2; p50 and peak RSS.

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| `count` p50, `v3` → PR 2 | 207 → 44 ms | 3.01 s → 457 ms | 29.6 s → 4.46 s |
| `count` peak RSS | 97 → 7 MiB | 728 → 7 MiB | 6,619 → 7 MiB |
| `count_grouped` p50 | 31.4 → 5.9 ms | 397 → 61 ms | 4.51 s → 614 ms |
| `count_grouped` peak RSS | 14 → 5 MiB | 97 → 7 MiB | 901 → 7 MiB |
| every other shape | unchanged | unchanged | unchanged |

An ungrouped aggregate holds one set of accumulators, and `count` reads no value, so memory is flat across scales; a grouped one holds one entry per group.

### Windows on the streaming engine (#547 PR 3)

`window` is `[:find ?e (rank :over (:order-by ?a)) :where [?e :age ?a]]` (every `:age` row: 16,666, 166,666 and 1,666,666 rows). Before PR 3 every window query ran on the executor. A/B on the same idle server, `v3` (58c19dd, with the `window` shape added) and PR 3 run in parallel on different sockets, swapping nodes between the two rounds; p50 and peak RSS, mean of the rounds.

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| `window` p50, `v3` → PR 3 | 41.5 → 32.8 ms | 601 → 434 ms | 7.89 → 5.65 s |
| `window` peak RSS | 19 → 16 MiB | 140 → 109 MiB | 1,257 → 1,030 MiB |
| every other shape | unchanged | unchanged | unchanged |

A window needs its whole partition, so `Window` holds every row and memory grows with the input. It turns each row's ids into values in its first pull, for the windows' sort; the stream keeps those DICT lookups until it ends, so returning the rows does not look them up again. Without that, the first measurement of PR 3 was 17% slower than the executor at 100K (48.2 ms).

### Rule queries on the streaming engine (#548)

`recursive_ancestors` is `(boss :p{leaf} ?b)` (5–7 rows, through the magic-set rewrite); `recursive` is `[:find ?x ?b :where (boss ?x ?b)]`, the whole transitive closure of `:manager` with no constant, so every entity's chain up to `:p0` is derived (72,219, 888,885 and 10,555,551 rows). `boss` is the two-rule closure over `:manager`; the envelope opens with `max_derived_facts` unlimited. A/B on the same idle server (2026-10-21), `v3` before PR 2 (84ac845, rule queries on the executor, with the `recursive` shape added) and PR 3 run in parallel on different sockets, swapped for the second round; p50 of either round (they agree within 2%) and peak RSS. The executor runs were stopped after 60 s at 10M.

| Shape | 100K | 1M | 10M |
|---|---:|---:|---:|
| `recursive_ancestors` p50, executor → engine | 2.71 s → 1.45 ms | 43.9 s → 2.14 ms | > 60 s → 3.04 ms |
| `recursive_ancestors` peak RSS | 253 → 4 MiB | 2,663 → 4 MiB | > 8.8 GiB → 4 MiB |
| `recursive` p50 | 3.04 s → 211 ms | 51.0 s → 3.21 s | > 60 s → 53.8 s |
| `recursive` peak RSS | 484 → 45 MiB | 4,786 → 318 MiB | > 8.8 GiB → 2,557 MiB |
| every other shape | unchanged | unchanged | unchanged |

The executor copied the database into a `FactStorage` for every rule query; the engine reads the stored facts where they are and holds only the derived relations (about 250 bytes per derived fact at 10M), so `recursive` memory follows its 10.6M derived facts and not the 10M stored ones. The point queries (#379) are bimodal per process on this server, about 20 µs or about 50 µs at 1M facts, in both builds: six fresh processes of each shape per build gave the same medians within 7 µs.

The acceptance test (`tests/recursive_acceptance_test.rs`, ignored; the nightly smoke workflow runs it) derives 4,882,500 `reach` facts over a layered graph of 155,000 edges (63 layers of 50 nodes, complete bipartite between neighbours) in 62 rounds: 226 s and 169 bytes of RSS per derived fact on a laptop. Each fact is derived through 50 middle nodes, so the rounds probe about 250M candidate rows; the time is in those entity probes and the identity set, not in memory.

### Query memory accounting (#549)

**Command**: `cargo test --profile bench --test memory_limit_acceptance_test -- --ignored --nocapture --test-threads=1` (the nightly smoke workflow runs it) and `cargo test --profile bench --lib meter_peak_against_rss -- --ignored --nocapture`, on a dedicated 88-vCPU Linux server, pinned to one NUMA node, three runs each.

| Shape | Rows | Time | Meter peak | Peak RSS added |
|---|---:|---:|---:|---:|
| 1M groups, `[:find ?g (count ?e) :where [?e :g ?g]]` | 1,000,000 | 3.5–3.6 s | 435 MiB | 542–549 MiB |
| #575 cartesian, 13 patterns sharing no variable over 3 facts | 1,594,323 | 2.6 s | 4 KiB | 9.6–9.7 MiB in a fresh process; under 50 KiB after another query |

The meter's estimate is within the spec's factor of two of the RSS a query adds (1.25× for the groups). The rest is allocator rounding, hash-table capacity beyond its entries, and the reader's pages, which no holder counts. The cartesian product held more than 5.7 GB on the executor (#575); on the engine every join builds on its 3-row inner pattern and the 1.6M rows stream through, so what a fresh process adds is the reader and the cursor, not the rows. With `query_memory_limit` at 32 MiB the groups query stops with `QRY-012` naming `Aggregate`, and at 64 bytes the product stops with `QRY-012` naming `HashJoin`.

Measuring this found that `Aggregate` built its sorted map of groups uncharged while its hash table was still alive: the meter read 282 MiB against 651 MiB of RSS. The map is now charged, and the table goes first, which also cut the real peak to the figure above.

### `:limit` (#306)

`limit` is `attribute_scan` with `:limit 10`: `Limit` closes the scan after its first internal batch, so the time does not grow with the facts (p50 307 µs, 317 µs and 318 µs at 100K, 1M and 10M, 5 MiB peak RSS each; `attribute_scan` reads the whole attribute in 43 ms, 462 ms and 4.83 s). Same idle server, one NUMA node, 2026-10-24. The acceptance test (`cargo test --profile bench --test limit_acceptance_test -- --ignored --nocapture`, nightly) runs `:limit 10` over 10M facts of one attribute in 0.8 ms under a 64 KiB `query_memory_limit`, adding 132 KiB of RSS.

`top_k` is `attribute_scan` with `:order-by [?a :desc] :limit 10`: `Sort` reads the whole attribute but holds ten rows, so its time follows the scan and its memory does not (p50 22.4 ms, 241 ms and 2.54 s at 100K, 1M and 10M against `attribute_scan`'s 23.1 ms, 250 ms and 2.63 s on the same run; 6–7 MiB peak RSS at every scale). Same server, one NUMA node, 2026-10-25. The acceptance test runs `:order-by [?v :desc] :limit 10` over the same 10M facts in 28.5 s (bench profile, opt-level `z`) under a 1 MiB `query_memory_limit`, adding no measurable RSS. Skipping the read when an index returns the rows in order is #641.

### Executor removal (#551)

Since #551 every query runs on the engine and the executor's query path is gone. Before it, the queries the engine could not plan ran on the executor, which reads every fact of the patterns' attributes into memory; the envelope's case is `:as-of` with a time. `as_of_ts` is the `as_of` point query with `:as-of "<time of transaction tx_half>"`; `as_of_ts_scan` is `[:find ?e ?a :as-of "<…>" :where [?e :age ?a]]` (8,400, 83,400 and 833,400 rows; 83,600 on the irregular copy). The fixture's `tx_id`s rise with `tx_count`, so the time becomes a counter bound. `1m-irregular` is a copy of the 1M fixture written with `LogWriter`, with each transaction's `tx_id` taken from a fixed permutation of the source's, so they fall and rise; there the engine filters on `tx_count` ranges instead.

A/B on the same idle server (2026-10-24), `v3` before #551 PR 1 (`1096e6a2`, with this PR's envelope shapes and fixture) and #551 PR 3, run in parallel on two sockets, swapped for the second round; `--scales 100k,1m,1m-irregular,10m`; p50, mean of the rounds (they agree within 9%, most within 2%), and peak RSS.

| Shape | 100K | 1M | 1M irregular | 10M |
|---|---:|---:|---:|---:|
| `as_of_ts` p50, `v3` → PR 3 | 111 ms → 693 µs | 1.65 s → 809 µs | 1.67 s → 656 µs | 17.2 s → 920 µs |
| `as_of_ts` peak RSS | 57 → 4 MiB | 435 → 4 MiB | 436 → 4 MiB | 3,623 → 5 MiB |
| `as_of_ts_scan` p50 | 110 → 13.4 ms | 1.72 s → 136 ms | 1.71 s → 136 ms | 17.8 → 1.40 s |
| `as_of_ts_scan` peak RSS | 57 → 5 MiB | 435 → 6 MiB | 435 → 6 MiB | 3,623 → 7 MiB |
| `recursive_ancestors` p50 (#721) | 100 ms → 1.14 ms | 1.81 s → 1.46 ms | 1.81 s → 1.39 ms | 35.7 s → 1.93 ms |
| `recursive_ancestors` peak RSS | 11 → 5 MiB | 69 → 5 MiB | 67 → 5 MiB | 628 → 5 MiB |
| `join_3_selective` p50 (#721) | 10.4 ms → 210 µs | 145 ms → 204 µs | 145 ms → 223 µs | 2.43 s → 195 µs |
| `join_3_selective` peak RSS | 7 → 5 MiB | 14 → 5 MiB | 14 → 4 MiB | 66 → 5 MiB |
| point lookups (`point_entity`, `attribute_value`, `as_of`, `valid_at`) | 17–19 → 18–21 µs | 16–19 → 17–20 µs | 17–19 → 18–20 µs | 16–19 → 17–20 µs |
| `join_2` | 37.5 → 38.6 µs | 35.7 → 37.3 µs | 36.0 → 36.9 µs | 35.7 → 37.2 µs |
| every other shape | within 2%, `not_probe` +4% | within 2% | within 3% | within 2% |

Every shape on PR 3, from the same run (before #726, see the notes; p50 of the two rounds, peak RSS):

| Shape | 100K | 1M | 10M | Peak RSS at 10M |
|---|---:|---:|---:|---:|
| `point_entity`, `as_of`, `valid_at` | 18–21 µs | 18–20 µs | 18–20 µs | 4–5 MiB |
| `attribute_value` | 18 µs | 17 µs | 17 µs | 5 MiB |
| `hot_current`, `hot_as_of` | 0.29–0.30 ms | 0.29–0.30 ms | 0.29–0.30 ms | 5 MiB |
| `join_2`, `join_2_pseudo` | 39–44 µs | 37–43 µs | 37–43 µs | 5 MiB |
| `join_3_selective` | 210 µs | 204 µs | 195 µs | 5 MiB |
| `recursive_ancestors` | 1.14 ms | 1.46 ms | 1.93 ms | 5 MiB |
| `as_of_ts` | 693 µs | 809 µs | 920 µs | 5 MiB |
| `not_probe` | 2.97 ms | 27.0 ms | 291 ms | 7 MiB |
| `count_grouped` | 5.92 ms | 60.8 ms | 610 ms | 7 MiB |
| `not_join_hash` | 5.88 ms | 52.9 ms | 569 ms | 9 MiB |
| `or_2` | 8.61 ms | 80.3 ms | 864 ms | 13 MiB |
| `as_of_ts_scan` | 13.4 ms | 136 ms | 1.40 s | 7 MiB |
| `attribute_scan` | 23.1 ms | 248 ms | 2.55 s | 7 MiB |
| `count` | 44.6 ms | 442 ms | 4.52 s | 7 MiB |
| `window` | 35.1 ms | 448 ms | 5.95 s | 893 MiB |
| `join_3_scan` | 41.4 ms | 1.13 s | 13.2 s | 61 MiB |
| `recursive` | 223 ms | 3.44 s | 57.4 s | 2,610 MiB |

Notes:

- **`:as-of` with a time** is the change #551 makes: the executor read every fact of the pattern's attribute, 3.6 GiB at 10M facts; the engine resolves the time once when the query opens and reads only what the pattern reads. Resolving it reads the DICT's transaction records, which is the 0.7–0.9 ms of `as_of_ts` against 18 µs for `as_of` with a counter. On the irregular copy, where the time selects `tx_count` ranges instead of a bound, both shapes take the same time as on the 1M fixture.
- **`recursive_ancestors` and `join_3_selective`** changed through #721, merged on `v3` after the baseline: a join on a value probes the AVET ranges of that value instead of hashing the whole attribute. `recursive_ancestors` had been 50–10,000× slower since #700 (#721); it is back to the #548 figures above.
- **Point lookups** and `join_2` are 2–7% slower than the baseline in this run (about 1 µs). The cause is #721 too: its per-query value-probe state made every query's `Stream` 1,056 bytes, past the 1,032 bytes glibc serves from its per-thread cache, so each query took the slower allocator path twice. #726 boxes that state and builds it only for queries that probe values (888 bytes); a loop of one point query per call at 100K takes 14.8–15.1 µs before #721, 15.7–16.6 µs after it and 15.3–15.8 µs after #726. The remaining 3% is #721's scan code; `stream_fits_the_allocator_thread_cache` keeps the size under the limit on 64-bit glibc.
- No other shape is more than 3% slower at any scale, except `not_probe` at 100K (2.84 → 2.97 ms, +4%; within 1% at 1M and 10M).

### Python FFI round trip

**Command**: `python bench/envelope.py <fixture> --report envelope.json --scale 1m` in [minigraf-python](https://github.com/project-minigraf/minigraf-python) (`v3`), on the 1M fixture and the PR 3 report of the #551 A/B, on the same server, pinned to one NUMA node, one process per shape. The binding is built with `maturin develop --release` against #551 PR 3. It runs the same queries through `db.query(...)`, drains them with `next_batch(1000)`, and decodes each JSON batch with `json.loads`. Python 3.12.3.

| Shape (1M facts) | Rows | Rust p50 | Python p50 / p99 | Rust peak RSS | Python peak RSS |
|---|---:|---:|---:|---:|---:|
| `point_entity` | 1 | 18 µs | 114 µs / 125 µs | 4 MiB | 17 MiB |
| `attribute_value` | 1 | 17 µs | 115 µs / 131 µs | 4 MiB | 17 MiB |
| `as_of` | 1 | 19 µs | 115 µs / 135 µs | 4 MiB | 17 MiB |
| `valid_at` | 1 | 20 µs | 129 µs / 157 µs | 4 MiB | 17 MiB |
| `join_2` | 1 | 38 µs | 145 µs / 157 µs | 4 MiB | 17 MiB |
| `join_2_pseudo` | 1 | 43 µs | 174 µs / 203 µs | 4 MiB | 17 MiB |
| `join_3_selective` | 10 | 203 µs | 327 µs / 348 µs | 5 MiB | 17 MiB |
| `hot_current` | 1 | 298 µs | 395 µs / 426 µs | 5 MiB | 17 MiB |
| `hot_as_of` | 1 | 289 µs | 399 µs / 432 µs | 5 MiB | 17 MiB |
| `as_of_ts` | 1 | 848 µs | 771 µs / 793 µs | 4 MiB | 17 MiB |
| `recursive_ancestors` | 6 | 1.43 ms | 1.54 ms / 1.96 ms | 5 MiB | 17 MiB |
| `not_probe` | 1,481 | 27.1 ms | 27.4 ms / 29.5 ms | 6 MiB | 19 MiB |
| `not_join_hash` | 740 | 53.1 ms | 54.7 ms / 61.7 ms | 7 MiB | 19 MiB |
| `count_grouped` | 100 | 61.0 ms | 65.4 ms / 73.1 ms | 6 MiB | 18 MiB |
| `or_2` | 1,112 | 81.5 ms | 84.0 ms / 94.7 ms | 7 MiB | 19 MiB |
| `as_of_ts_scan` | 83,400 | 136 ms | 192 ms / 205 ms | 6 MiB | 19 MiB |
| `attribute_scan` | 166,666 | 249 ms | 356 ms / 412 ms | 7 MiB | 19 MiB |
| `count` | 1 | 444 ms | 490 ms / 504 ms | 6 MiB | 18 MiB |
| `window` | 166,666 | 452 ms | 599 ms / 611 ms | 103 MiB | 81 MiB |
| `join_3_scan` | 1,667 | 1.13 s | 1.17 s / 1.18 s | 41 MiB | 49 MiB |
| `recursive` | 888,885 | 3.45 s | 4.29 s / 4.32 s | 345 MiB | 272 MiB |

The binding adds about 100 µs per query, plus about 0.6–1 µs per returned row for the JSON batches (0.1 s for `attribute_scan`'s 166,666 rows, 0.8 s for `recursive`'s 888,885). The interpreter adds about 13 MiB of RSS; past that, peak RSS is the query's own, and the cursor holds one batch: `attribute_scan` returns its 166,666 rows in 19 MiB, where the executor's cursor held the whole answer (169 MiB at 1M in the first tables). `window` and `recursive` hold their rows in both; the binding is its own build of the library (its release profile), which holds less for them here than the envelope's build. Built against `1096e6a2`, the same binding gave `attribute_scan` 355 ms / 19 MiB and `count_grouped` 64.9 ms / 18 MiB, against 358 ms / 19 MiB and 65.2 ms / 18 MiB on PR 3 (#473 acceptance: no change at the FFI).

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
- Criterion does not collect heap or RSS itself. The #183 snapshot adds a process-wide RSS high-water mark from `/usr/bin/time` and separate engine-meter peaks; the performance envelope reports RSS from `/proc` (Linux only). The weekly soak test (#392) also records RSS, file size, and open and checkpoint time at 10M facts.
