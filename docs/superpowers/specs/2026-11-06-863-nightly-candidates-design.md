# #863: qualify the #841 shape-sweep nightly candidates

Issue: [#863](https://github.com/project-minigraf/minigraf/issues/863), a follow-up of
#841. Base: `v3` at `921b3935`. No production behavior, API, file format or
release-binary change.

## 1. Goal

The [#841 report](../../benchmarks/841-shape-sweeps/README.md#nightly-candidates-proposals-only)
proposed six small representatives, each guarding one behavior the sweep found.
For each, this work either:

- **qualifies** it on the GitHub runner and runs it nightly, asserting its answer,
  its path and (where that is the behavior) its charged query memory, as well as
  its time; or
- **rejects** it with a recorded reason (cost, variance, or a fixture too large
  for the runner).

A nightly route detects a regression in one representative. It does not
re-establish the #841 envelope, and the report says so.

## 2. Decisions (user, 2026-11-06)

| # | Decision |
| --- | --- |
| D1 | **Path evidence:** read the cursor's existing `Debug` operator counters (operator names, rows, bytes held) wherever they distinguish the path. Add thread-local event counters only for the events `Debug` cannot show, under the `test-hooks` feature (`cfg(any(test, feature = "test-hooks"))`). The default and `dist` builds do not change. |
| D2 | **Route:** split. Time goes in a Criterion case selected by an existing nightly `bench.yml` filter (Bencher latency, t-test alerts, bench profile). Answer, path and memory go in a named, ignored acceptance test in `smoke.yml`'s v3 acceptance matrix, built with `--features test-hooks`. A path regression fails the acceptance job. A slower runner can only raise a Bencher alert. |
| D3 | **Qualification bar:** at least 3 runs on separate `ubuntu-latest` runners. A candidate is rejected if fixture construction plus measurement takes more than **10 job-minutes**, or its run-to-run spread is over **25%**: `(max − min) / min` of the per-run Criterion point estimates. |

## 3. Candidates

"Committed" fixtures are built with `LogWriter` (fixed timestamps), as #841 did;
"pending" fixtures are file-backed with `wal_checkpoint_threshold(usize::MAX)`, so
every transaction is replayed from the WAL. Entity UUIDs, fixture records and
queries are the #841 harness's (`scripts/shape-sweep/native/src/main.rs`,
`records` and `query`), so the answers can be compared with the archived #841
digests.

| ID | Candidate | Criterion ID (filter) | Path evidence | Memory evidence |
| --- | --- | --- | --- | --- |
| N1 | 50,000 rows (100k facts), 90%-skewed partitions, running `sum`, committed | `window/nightly_candidates/skew_running_sum/100k` (`^window/`) | `Debug` has `WindowPartitioned`, not `Window` | Window's charged peak is at least the largest partition's rows × the per-row estimate and below 2× that; a full-input control (`__input_order(false)`) charges more |
| N2 | 10k entities, `K=256` skewed sets, six membership predicates, committed | `query/nightly_candidates/set_membership_k256/10k` (`^query/`) | Answer only. No event identifies the long value-page path (attribution limit, recorded) | none |
| N3 | 100k facts, descending `:offset 50,000 :limit 1,000`, committed; ascending pair | `query/nightly_candidates/sort_offset_desc/100k` and `…/sort_offset_asc/100k` (`^query/`) | Descending: `Debug` has `Sort`. Ascending: no `Sort` (input-order omission) | Sort's charged peak is bounded by `offset + limit` = 51,000 rows, not 100,000 |
| N4 | 1,110-node width-10 tree, ancestors of one leaf (`k=1`), pending | `recursion/nightly_candidates/tree_ancestors_k1_pending/1110` (`^recursion/`) | Event `ancestor_alternative_selected` ≥ 1 | none |
| N5 | 1,110-node tree, ancestors of 16 demanded leaves (`k=16`), committed | `recursion/nightly_candidates/tree_ancestors_k16_committed/1110` (`^recursion/`) | Event `bound_value_avet_range` ≥ 1; `Debug` has no `HashJoin` | none |
| N6 | 10k facts, one cursor held across the churn schedule (32 baseline, 256 held, 256 after release; 100 entities retracted/asserted per checkpoint) | Acceptance only, unless qualification shows a Criterion case adds coverage that `query/cursor_lifecycle/held_checkpoint_*` lacks | Public API only: the held cursor's answer does not change; the file grows while it is held; after release the file length stops growing within the 256 checkpoints (freed pages reused); `verify()` is clean | none |

The exact bounds in N1 and N3 (the per-row estimate and factor) are fixed in the
plan from the meter's per-entry estimates, then confirmed against a measured run.
A bound only a broken path can exceed is enough; the acceptance test is not a
precise memory model.

### Event counters (D1)

One thread-local `BTreeMap<&'static str, u64>` in `query::engine`, incremented at
three sites, the same as `scripts/shape-sweep/events.py`:

| Event | Site |
| --- | --- |
| `ancestor_alternative_selected` | `recursive.rs`: a round picks the small in-memory ancestor alternative (#723) |
| `bound_value_avet_range` | `scan.rs`: an AVET range for a bound value is pushed (#721) |
| `entity_probe_cache_hit` | `join.rs`: an entity probe reuses a cached entry |

`entity_probe_cache_hit` is not checked by any candidate. It is included only if a
candidate's plan turns out to need it; otherwise it is left out (only events in use
are added).

Exported as `#[doc(hidden)] pub fn __path_events() -> BTreeMap<&'static str, u64>`
(take and reset), next to `__input_order` in `lib.rs`. A `test-hooks` doc-hidden
accessor for one cursor's per-operator meter peak (`Meter::peak_for`) serves N1
and N3. Queries and cursor pulls run on the calling thread, so the thread-local
maps see the query's own events.

## 4. Qualification protocol

1. The v3 PR adds the Criterion cases and the acceptance test.
2. A **temporary** workflow on the PR branch (`pull_request`, path-filtered)
   runs a 3-way matrix. Each job, on its own runner, builds with the bench
   profile, times the untimed setup that `--list` and selection pay, then runs
   `cargo bench --bench minigraf_bench -- '<candidate filter>'`, and uploads the
   Criterion estimates and step timings. It also runs the acceptance test once
   and records its wall time. Nothing is uploaded to Bencher.
3. Evidence is committed under `docs/benchmarks/863-nightly-candidates/`: raw
   Criterion `estimates.json`, step timings, runner image, source SHA, and a
   summary table per candidate (cost, three point estimates, spread, verdict).
4. The temporary workflow is removed in the PR's last commit, before merge.
5. A rejected candidate's Criterion case and acceptance test are removed; the
   reason stays in the evidence README.

Cost per candidate = its fixture construction + warm-up + measurement inside the
Criterion job, plus its acceptance test's wall time. The shared compile step is
not counted per candidate (it already runs nightly).

## 5. Wiring

- **Criterion:** new group files under `benches/minigraf_bench/`, registered in
  `benches/minigraf_bench.rs`. Fixtures and answer checks run lazily inside the
  selected callback, outside `iter`, as `operator_paths.rs` does. The answer is
  checked once before timing (row count and the #841 digest). `bench.yml` needs no
  change: selection is by filter.
- **Acceptance:** `tests/nightly_candidates_acceptance_test.rs`, one `#[ignore]`d
  test per qualified candidate (`#![cfg(not(target_arch = "wasm32"))]`,
  `#![cfg(feature = "test-hooks")]`). Shared CI files, as separate PRs after the
  v3 PR merges, identical on both branches:
  - `smoke.yml`: matrix entry `nightly-candidates`
    (`cargo test --profile bench --features test-hooks --test nightly_candidates_acceptance_test -- --ignored --nocapture --test-threads=1`)
    and the file in the acceptance-availability loop;
  - `.github/acceptance/verify.py` and its test: `SUITES` entry with the exact test
    names (no RSS records).

  The v3 PR goes first, so the scheduled v3 run never sees the availability check
  without the test file.

## 6. Documentation

- `docs/bench-coverage-v3.md`: the new routes, and the #841 proposals marked
  resolved (qualified or rejected, linking the evidence).
- `docs/benchmarks/841-shape-sweeps/README.md`: a pointer from "Nightly
  candidates" to the #863 evidence (the original table is kept).
- `CLAUDE.md`, `docs/TEST_COVERAGE.md`: test counts (new ignored tests; test-hooks
  counts).
- No CHANGELOG entry and no docs-site change: nothing users see changes.

## 7. Acceptance criteria (from #863)

- [ ] Each candidate is qualified with a recorded cost and variance on the
      runner, or rejected with a reason.
- [ ] Qualified candidates run nightly, asserting answers, path counters and
      (N1, N3) charged memory as well as time.
- [ ] `docs/bench-coverage-v3.md` lists the new routes and marks the #841
      proposals resolved.

Also: the release binary size is unchanged (the hooks compile only under
`test-hooks`), and every new test follows the Testing Conventions (no `{:?}` of
values in assertion messages).

## 8. Out of scope

- Re-running the #841 sweep, or any envelope-profile route (D2).
- Public path counters or a query profiler (#185).
- A Bencher memory metric.
