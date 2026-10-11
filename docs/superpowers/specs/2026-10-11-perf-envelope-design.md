# Performance Envelope (#394) — Design

**Date**: 2026-10-11 · **Issue**: #394 · **Branch**: `v3` (format v8)

## Goal

Publish the graph sizes and query shapes Minigraf supports and tests, with p50/p99
latency, open time, checkpoint time and RSS at 100K, 1M and 10M facts. State the
sizes that are not supported yet. Find the top costs of the attribute scan and the
3-clause join at 1M facts and file follow-up issues.

## Decisions

1. **Envelope binary plus Criterion.** `examples/perf_envelope.rs` builds and caches
   the fixtures, times every query shape, reads RSS from `/proc/self/status`, and prints
   a markdown table and JSON. Criterion reports a mean with a confidence interval but no
   p99 and no RSS, and 10M facts would not fit the nightly time budget. The nightly
   `query/*` Criterion groups gain 100K and 1M scales for regression tracking. No new
   dependencies (`serde_json` and `tempfile` are already dev-dependencies).
2. **Where it runs.** 10M numbers come from local runs. `.github/workflows/envelope.yml`
   runs 100K and 1M on `workflow_dispatch` and uploads the table; on PRs that touch the
   harness it runs a 10K smoke pass. Nothing is scheduled.
3. **Python FFI round trip** (carried over from #462): `bench/envelope.py` in
   `minigraf-python` (`v3`), run against the fixture file the core harness builds,
   drains `db.query(...)` with `next_batch(1000)` and reports wall time and RSS. The
   numbers go into `docs/BENCHMARKS.md` here. Separate PR to the binding repo. #473
   re-measures it as an acceptance test for the streaming engine.

## Fixture

`N` facts over `E = N / 6` entities `:p{i}`, six facts each, built in transactions of
200 entities with an explicit checkpoint every 200K facts so RSS stays bounded:

| Attribute | Value | Purpose |
|---|---|---|
| `:name` | `"p{i}"` | string; attribute-value lookup |
| `:age` | `i % 90` | integer; point entity, attribute scan |
| `:group` | `i % 100` | integer; grouped aggregation |
| `:follows` | `:p{(i·7919 + 1) mod E}` | ref; 2-clause join |
| `:manager` | `:p{i / 10}` | ref tree (fan-out 10); 3-clause join, recursion |
| `:status` | `"a"` valid 2020–2023, `"b"` valid from 2023 | two valid-time windows |

Plus one hot entity `:hot` whose `:counter` is retracted and reasserted 1,000 times
(#323, #379), so its history is the same at every scale.

The fixture is cached at `target/perf-envelope/fixture-v{version}-{scale}.graph`, with
its build metadata (entity count, the `:as-of` counters, build time and peak RSS) in a
`.json` next to it. Bump the version when the fixture changes.

## Measurements

The harness builds with `--profile envelope`: `release` at opt-level 3, which is how
an application's release build compiles the library. `release` (and `bench`, which
inherits it) is opt-level "z" for the REPL binary's size and runs these queries about
1.5× slower.

The build, the storage measurements and every query shape run in their own child
process, so each peak RSS (VmHWM) is that step's alone. Opening costs about 0.1 ms,
so a process per shape is cheap.

- **Open**: `Minigraf::open` on a copy of the fixture (OS page cache warm; a cold open
  needs root to drop caches), RSS after open.
- **Checkpoint**: after 1 new fact (20 runs, p50/p99) and after 10K new facts, on the
  same copy.
- **Queries**: the fixture opened read-only. The first run checks the row count, and is
  abandoned after `--cap-secs` (default 60): the shape is reported as capped. A query
  error (such as INT-020) is reported as the shape's result. Then two more warm-up runs
  if a run takes under a second, then runs until 200 or a 15 s budget, at least 5.
  Every run drains the cursor with `next_batch(1000)`. Reported: p50, p99 (nearest
  rank; the maximum below 100 runs), max, run count, rows, peak RSS.

Query shapes: point entity; attribute + value; attribute scan; selective 2- and
3-clause joins; whole-graph 3-clause join; recursive rule from a leaf (demand-driven);
`:as-of`; `:valid-at`; grouped count; hot entity current value and `:as-of`.

## Envelope

`docs/BENCHMARKS.md` gets the full table per scale; the README gets a short summary:
which shapes are supported at which sizes, p50/p99, and that sizes over 10M facts are
not tested and not supported yet. The release checklist (CONTRIBUTING.md) re-runs the
envelope for each minor release.

## Profiling

`perf record` on the attribute scan and the 3-clause join at 1M facts; one follow-up
issue per top cost, linked from #394.
