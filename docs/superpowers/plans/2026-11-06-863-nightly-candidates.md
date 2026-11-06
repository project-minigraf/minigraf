# #863 plan: qualify the #841 nightly candidates

Spec: `docs/superpowers/specs/2026-11-06-863-nightly-candidates-design.md` (5f222758).
Branch `test/863-nightly-candidates` off `v3` at `921b3935`, worktree
`.worktrees/863-nightly-candidates`. Three PRs:

1. **v3 PR** (this branch): hooks, shared fixtures, Criterion cases, acceptance
   test, temporary qualification workflow (removed before merge), evidence, docs.
   Body: `Refs #863`. Closing keywords do not fire on `v3`; close #863 by hand
   after PRs 2 and 3 merge and the first scheduled run is green.
2. **v3 workflow PR** and 3. **main workflow PR**: identical `smoke.yml` and
   `.github/acceptance/` changes, after PR 1 merges.

Tests, coverage and timings run on the bench machine first (alt bench if busy).
Release size is measured on the laptop with its own `CARGO_TARGET_DIR`.

## Facts the plan relies on (checked at 921b3935)

- `Cursor`'s `Debug` prints the stream's operator counters (`operator_paths.rs`
  uses it). Operator names: `WindowPartitioned` / `Window` (`window.rs:96`),
  `Sort`, `Limit`, `HashJoin` / `IndexNestedLoopJoin` (`join.rs:103`; `Probe` and
  `ProbeValue` both print `IndexNestedLoopJoin`, hence the AVET event).
- `Cursor` drops its stream on the pull **after** the last rows
  (`cursor.rs:89`). Peaks must be read once the expected row count has been
  pulled, before the call that returns `None`.
- `Stream::meter` and `Meter::peak`/`peak_for` are `#[cfg(test)]`. `peak_for`
  tracks only `Window`, `Aggregate` and `IdSpace`; Sort has no per-operator peak,
  so N3 uses the query's total peak.
- Event sites: `recursive.rs:270` (`Ok(alternative)`), `scan.rs:241`
  (`ranges.push((Index::Avet, …))`).
- The #723 alternative needs an empty committed dictionary (pending fixture);
  the #721 AVET path needs a committed generation.
- `bench.yml` selects IDs by regex over `--list`; no ID inventory to update.
  New IDs under `^window/`, `^query/`, `^recursion/` run nightly on both
  branches' v3 schedule automatically.

## Step 1: hooks (TDD)

Files: `src/query/engine/mod.rs`, `recursive.rs`, `scan.rs`, `meter.rs`,
`src/cursor.rs`, `src/lib.rs`.

1. Test first (`src/query/engine/tests.rs`, unit): a pending width-10 tree with a
   single-leaf ancestor query records `ancestor_alternative_selected`; the same
   fixture checkpointed with 16 demanded leaves records `bound_value_avet_range`
   and not the alternative. A query with neither records nothing. Each test
   calls `take_path_events()` first to clear the thread's map.
2. `mod.rs`:
   ```rust
   #[cfg(any(test, feature = "test-hooks"))]
   thread_local! {
       static PATH_EVENTS: RefCell<BTreeMap<&'static str, u64>> = const { RefCell::new(BTreeMap::new()) };
   }
   #[inline]
   pub(crate) fn path_event(_name: &'static str) {
       #[cfg(any(test, feature = "test-hooks"))]
       PATH_EVENTS.with(|m| *m.borrow_mut().entry(_name).or_default() += 1);
   }
   #[cfg(any(test, feature = "test-hooks"))]
   pub(crate) fn take_path_events() -> BTreeMap<&'static str, u64> { PATH_EVENTS.with(|m| m.take()) }
   ```
   A no-op `#[inline]` call compiles out of release builds.
3. Call `path_event("ancestor_alternative_selected")` before `Ok(alternative)`
   and `path_event("bound_value_avet_range")` after the AVET `ranges.push`.
   `entity_probe_cache_hit` is not added (spec §3: only events in use).
4. `Stream::meter`: `#[cfg(any(test, feature = "test-hooks"))]`; same for
   `Meter::peak`, `tracked`, `peak_for`. Add
   `Stream::memory_peak(&self, op: Option<&str>) -> usize` (total or `peak_for`).
5. `cursor.rs`: under `test-hooks` only,
   `#[doc(hidden)] pub fn __memory_peak(&self, op: Option<&str>) -> Option<usize>`
   (`None` once the stream is gone or for an answer computed at open).
6. `lib.rs`: `#[cfg(feature = "test-hooks")] #[doc(hidden)] pub fn __path_events()
   -> BTreeMap<&'static str, u64>` with a doc comment like `__input_order`'s.
7. Integration test `tests/path_events_test.rs` (`#![cfg(feature = "test-hooks")]`,
   not wasm32): the public `__path_events` and `__memory_peak` give the same
   result as step 1 through the public API, and `__memory_peak` is `None` after
   the stream ends.
8. Check: `cargo build --profile dist` size unchanged vs `921b3935` (laptop, own
   target dirs); `cargo clippy --all-targets --features test-hooks`; plain and
   hooks test runs.

## Step 2: shared shape fixtures

One source for benches and tests: `benches/helpers/shape.rs`, included by the
acceptance test with `#[path = "../benches/helpers/shape.rs"] mod shape;`.

Port from `scripts/shape-sweep/native/src/main.rs`, trimmed to the six
candidates: `entity`, `integer`, `part` (skew), `members` (skew), `records`
(window, set, scalar, recursion), `query`, the `Digest` (`mix`, `hash_value`,
`add`) and `oracle`, plus `build_committed` (LogWriter, fixed timestamps,
1,000 facts per tx) and `build_pending` (1,000-fact transacts,
`wal_checkpoint_threshold(usize::MAX)`) into a `TempDir`, and the two rules for
`anc`. Keep the harness's UUID base `0x8410…` and query text, so answers are the
#841 answers. The port drops the harness's `Case`/serde plumbing (plain
functions with explicit parameters), so it needs no new dev-dependency.

Unit-check the port in the acceptance file (step 4) by comparing each candidate's
drained digest with `oracle`.

## Step 3: Criterion cases

New `benches/minigraf_bench/nightly_candidates.rs`, registered in
`minigraf_bench.rs` (`criterion_group!`). One function per route group:

| ID | Group | Notes |
| --- | --- | --- |
| `window/nightly_candidates/skew_running_sum/100k` | `window/nightly_candidates` | n = 50,000 rows |
| `query/nightly_candidates/set_membership_k256/10k` | `query/nightly_candidates` | |
| `query/nightly_candidates/sort_offset_desc/100k`, `…/sort_offset_asc/100k` | same | `:offset 50000 :limit 1000` |
| `recursion/nightly_candidates/tree_ancestors_k1_pending/1110` | `recursion/nightly_candidates` | pending fixture |
| `recursion/nightly_candidates/tree_ancestors_k16_committed/1110` | same | committed |

As in `operator_paths.rs`: the fixture is built lazily inside the selected
`bench_function` closure (so `--list` and unselected filters pay nothing), the
answer digest is checked once against `oracle` before `b.iter`, and the timed
body opens the query and drains it with `next_batch(1000)`. `sample_size(10)`,
`measurement_time` sized after a local run so each case stays under ~60 s.
Path checks are not repeated here (they need `test-hooks`; the acceptance job
owns them), except the `Debug` operator-name checks, which are free.

N6 gets no Criterion case unless step 5 shows a need (spec §3).

Untimed check: `cargo bench --bench minigraf_bench -- --test 'nightly_candidates'`.

## Step 4: acceptance test

`tests/nightly_candidates_acceptance_test.rs`,
`#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]`, one
`#[test] #[ignore = "nightly acceptance (#863)"]` per candidate:

- `n1_skew_running_sum_streams_by_partition`: `Debug` has `WindowPartitioned`;
  digest = oracle; `__memory_peak(Some("Window"))` read after 50,000 rows. Control
  under `__input_order(false)`: `Debug` has `Window`, same digest. Assert
  partitioned peak ≤ control peak and ≥ 0.8 × control peak (the largest
  partition holds 90% of rows). Print `PEAK_CHECK n1 partitioned=… full=…`.
- `n2_set_membership_k256`: digest = oracle (row count 10,000).
- `n3_sort_heap_holds_offset_plus_limit`: descending `Debug` has `Sort`; total
  peak read after 1,000 rows; control without `:offset/:limit` (total peak after
  100,000 rows). Assert limited peak ≤ 0.6 × control (51% of rows plus fixed
  costs). Ascending: no `Sort` in `Debug`, digest = oracle. Print `PEAK_CHECK n3 …`.
- `n4_small_tree_takes_ancestor_alternative`: events cleared, query drained;
  `ancestor_alternative_selected ≥ 1`; digest = oracle.
- `n5_committed_tree_uses_bound_value_probes`: `bound_value_avet_range ≥ 1`,
  `ancestor_alternative_selected` absent, no `HashJoin` in `Debug` at open;
  digest = oracle.
- `n6_held_cursor_quarantine_and_reuse`: 10k-fact LogWriter fixture, file
  database, schedule 32 / 256 / 256 checkpoints, each retracting and asserting
  100 entities' `:pin/value`. The cursor opened at round 32 is read once at the
  end and must return the original 10,000 rows. File length (`fs::metadata`)
  must grow during the held phase, and stay constant over the last 128 rounds
  after release. `verify()` clean at the end.

Bounds 0.8 and 0.6 are first guesses: run each test once on the bench, record
the measured ratios in the plan's deviations, and keep a bound only if the
measured ratio has at least 25% margin from it; else derive a bound from the
measured value and say why in the test comment. No `{:?}` of values in assert
messages.

Command (also the smoke.yml one):
`cargo test --profile bench --features test-hooks --test nightly_candidates_acceptance_test -- --ignored --nocapture --test-threads=1`.

## Step 5: qualification (temporary)

1. `.github/workflows/qualify-863.yml` on the branch only:
   `pull_request` with `paths: [.github/workflows/qualify-863.yml]` plus
   `workflow_dispatch` (not usable from a branch, documentation only); matrix
   `run: [1, 2, 3]`, `ubuntu-latest`, `timeout-minutes: 60`. Steps (each with
   `date +%s` stamps written to `timings.txt`): checkout, stable Rust, cache-less
   `cargo bench --bench minigraf_bench --no-run`, then per candidate filter
   `cargo bench --bench minigraf_bench -- '<exact ID regex>'`, then the
   acceptance command per test (`--exact`). Upload `target/criterion/**/new/estimates.json`,
   `timings.txt`, `uname -a`, `nproc`, runner image version.
2. Push, open PR 1 as draft. Wait for all three runs. If a run fails, fix and
   rerun the whole matrix (all three runs must come from one commit).
3. `scripts/qualify_863.py` (stdlib only, kept in the PR) reads the three
   downloaded artifacts and writes `docs/benchmarks/863-nightly-candidates/summary.json`
   and the table in its `README.md`: per candidate, cost = Criterion step
   seconds + acceptance test seconds (max over runs), point estimates, spread
   `(max − min) / min`, verdict by spec D3 (≤ 600 s, ≤ 25%).
4. Commit the artifacts (`estimates.json` per ID per run, `timings.txt`) with a
   `SHA256SUMS`.
5. Rejected candidates: delete their Criterion case and acceptance test; keep
   the reason in the README. If N6's numbers suggest a Criterion case would add
   coverage beyond `query/cursor_lifecycle/held_checkpoint_*`, add it and rerun
   step 5 for it; otherwise record why not.
6. Last commit removes `qualify-863.yml`.

## Step 6: docs

- `docs/benchmarks/863-nightly-candidates/README.md`: protocol (spec §4), runner
  facts, the table, verdicts, limits (N2 has no path counter; a nightly route
  detects one representative's regression and does not re-establish the #841
  envelope; bench profile is opt-level z).
- `docs/bench-coverage-v3.md`: a dated follow-up paragraph at the top, like the
  #837/#840 ones, listing the new IDs with their suites and the acceptance suite;
  the #841 proposals resolved.
- `docs/benchmarks/841-shape-sweeps/README.md`: one line under "Nightly
  candidates" pointing to the #863 README.
- `CLAUDE.md` (counts, the #841 paragraph mentions #863), `docs/TEST_COVERAGE.md`
  (new files and counts, plain and hooks).
- `scripts/shape-sweep/README.md`: note that `benches/helpers/shape.rs` is a
  trimmed port; the harness stays authoritative for the sweep.

## Step 7: verification before PR 1 leaves draft

- Bench machine: `cargo test` plain and `--features test-hooks` (record counts);
  acceptance command; `cargo bench -- --test 'nightly_candidates'`.
- Coverage gates (`scripts/coverage_gates.py`) on the bench with the hooks build
  if the new `cfg` lines enter a gated file's branch counts.
- Laptop: `cargo build --profile dist` size equal to base; `cargo fmt --check`;
  clippy with and without `test-hooks`; `cargo doc` (doc-hidden items).
- CI green on PR 1 (semver check failing on v3 is expected).
- Codex review skipped while the quota is out (back 2026-11-11), noted in the PR.

## Step 8: workflow PRs (after PR 1 merges)

Branch `ci/863-acceptance` off `v3` and `ci/863-acceptance-main` off `main`, same
diff:

- `smoke.yml`: matrix entry `nightly-candidates` (timeout from step 5's max
  acceptance time × 2, at least 20 minutes) with the step 4 command; add
  `nightly_candidates` to the availability loop's file list.
- `.github/acceptance/verify.py`: `'nightly-candidates': ([<qualified test
  names>], [])`; `test_verify.py`: a passing and a missing-test case.
- Merge v3 first (merge commit), then main (squash). Dispatch `smoke.yml` on main
  with `ref: v3`; the `nightly-candidates` job and its verifier must pass. Then
  check the next scheduled v3 nightly bench run selects and uploads the new IDs.

## Deviations

(Recorded during implementation.)
