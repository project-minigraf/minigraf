# #789 implementation plan

Status: Claude-approved (2026-10-29); maintainer approved implementation on 2026-10-29.
Spec: ../specs/2026-10-29-789-aggregate-values-design.md.
Worktree: `.worktrees/789-aggregate-values`, branch `feat/789-aggregate-values`.
Review session: `0a6c54d6-eea8-43f7-a2b8-4de1e99fb416`; both documents approved
with no blockers. Incorporates the review's documentation and test-location notes.

## 1. Establish the approved baseline

Reconcile with landed #777 before delivery, preserving ordinary avg and Sum
heap accounting. Read current aggregate.rs, engine aggregate/plan/tests,
existing ordinary/window suites, frozen answers and applicable guidance.
Use the bench machine for builds and tests whenever reachable and idle;
report any fallback. Record revision, clean baseline results and toolchain.
No implementation before design approval.

## 2. Write failing contract tests

Add shared tests in aggregate/tests.rs and a focused integration suite
`tests/aggregate_value_handling_test.rs`. Start with ordinary sum on
10, "unknown", 20 => Integer(30); mixed Integer/Float extrema; and
sum-distinct skipping repeated nonnumbers. Confirm they fail on the old
type errors before editing production code. Add the spec's explicit expected
all-variant, empty/group, zero/NaN, distinctness and permutation cases.
Keep ordinary avg implementation/tests owned by #777.

## 3. Make the narrow accumulator change

Remove Sum's deferred bad-type field and Extreme's kind validation. Keep
Sum arithmetic, value_order and window behavior unchanged. Filter sum-distinct
before both retention and memory charging. Update all enum patterns and #777
heap-accounting helpers. Preserve no-extremum `Ok(None)` and finish ordering.
Run the targeted red/green tests on the bench machine.

## 4. Verify execution and failure behavior

Extend engine unit tests for deterministic remaining error precedence,
memory charges, cleanup and plan independence. Replace obsolete tests that
expect INT-041/043/044 with successful explicit results; keep unrelated error
coverage intact. Audit `tests/answers` case by case before changing any golden.
Use injected upstream failures and UDF step/finalize failures to prove full
drain and find-order behavior, including empty min/max before/after a UDF.
In particular, replace the old type-error witnesses in
`src/query/engine/aggregate_order_tests.rs`, `tests/error_codes_qry_test.rs`
and `tests/aggregate_window_rules_test.rs`; preserve the precedence assertions
with real remaining failures rather than deleting their coverage.

Exercise prepared binds, multi-batch cursors and rewind/close, rules, temporal
views, transactions, pending and checkpointed data. Compare ordinary sums and
extrema with corresponding default and bounded windows over identical input
populations, with the explicit empty-extremum exception. Keep ordered
Aggregate eligibility unchanged and document the #781 handoff in the PR.

Run targeted engine meter tests for numeric-only sum-distinct retention,
canonical duplicates, unchanged count-distinct growth, QRY-012 thresholds and
cleanup. Recheck #777's Sum/Avg retained-heap tests after enum changes.

## 5. Synchronize documentation

Update the six repository status/reference documents listed in the spec using
actual final test counts. Document the stable error-code reservations and
the preserved empty/group behavior. Make a separate docs-site worktree from
the existing sibling checkout and version-scope v3 semantics. Audit the site's
other required pages and site.toml, preserving v2 and #777 changes. Run the
site's own checks and link validation; no version bump or release.

## 6. Verify and obtain final review

Run full `cargo test`, `cargo fmt --check`, and
`cargo clippy --all-features -- -D warnings`, plus the portable target/feature
and release-size checks used by current CI, on the bench machine. Preserve
logs and exact tested revisions; do not repeat expensive checks without a
new edit, failure or unresolved concern. Verify the final diff and test-count
documentation. Ask Claude to review complete core and docs diffs with the
approved spec, plan and verification evidence; resolve confirmed findings.
Open core PR targeting v3 and the docs-site PR only after final review. Do not
merge or release. Report remaining dependency or CI limitations accurately.

## #777 integration check

#777 merged as core PR #791 while verification was running. This worktree
was advanced to its actual v3 merge, `0fc97bd65`, and reconciled using the
previously reviewed and tested scratch combination. It preserves `Agg::Avg`,
its constructor/step/mean, and #777's engine accounting. Both
`exact_heap_growth` and `exact_heap_bytes` now match `Agg::Sum(s)`.
The avg integration test now expects `[nil, 0]` / `[0, nil]` for avg beside sum
on a nonnumeric population, retains the empty-min group-drop assertion, and
is named `ordinary_avg_matches_sum_results_and_preserves_dropped_groups`.
#789's UDF tests protect the remaining finish-error precedence. The obsolete
unit-test error-code helper was removed after merging the avg tests.

The reconciled base passes 64 aggregate unit/engine tests (one ignored existing
diagnostic) and all 15 avg/value-handling API tests. Before the merge, the
scratch combination also passed the dedicated Sum/Avg retained-heap test.
The docs branch was advanced to the merged #777 site PR #86 (`3f65810`),
preserving its avg content and version guards.

## Delivery verification

Final-base platform checks use Rust/Cargo 1.99.1. SHA-256 comparison confirms every
changed Rust source/test file matches the verification snapshot. Native
all-feature clippy, browser clippy, WASI clippy (all with `-D warnings`),
formatting and WASI lib/new-suite compilation pass. WASI tests were compiled,
not executed; no runner was installed. The release binary is 1,261,736 bytes,
below the unchanged 1,263,616-byte cap.

The focused `test-hooks` run passes 34 tests across `aggregate_value_handling_test`,
`ordinary_avg_test`, `plan_independence_test`, and `input_order_test`.
The dedicated `sum_and_avg_charge_exact_heap_once_and_release_it` test passes
on the final base. All 26 aggregation API tests and eight error-handling tests
pass after replacing six legacy type-error assertions found by the broader
runs with explicit results. Unrelated error-path checks remain intact.

All 16 unchanged storage fault-matrix cases passed in the pre-rebase full unit
run (1,167 passed, five ignored). A first attempt was interrupted after severe
filesystem contention; the completed matrix used a task-owned fixture directory.
The final-base default suite excludes only this already completed matrix.
No storage source changed in the rebase. Local fallback checks while the bench
was busy used Rust 1.94.0; the final checks above use the designated bench toolchain.

The reconciled docs site builds all 13 versions and 368 pages with link validation.
The build uses a temporary source snapshot containing the final core changelog
and error reference. Rendered v2 retains the old type restrictions, and v3
contains both the new aggregate value rules and the merged avg documentation.

A bench default rerun hit the crash-kill test's 30-second child-progress deadline
while other compilation and another task's full suite contended for I/O.
The same recorded seed (`1793252894809935624`) passes locally in 0.37 seconds,
with the unchanged test and timeout. The final default rerun uses the approved
local fallback (Rust 1.94.0), excluding only the completed storage fault matrix.
The deadline failure and the earlier obsolete-expectation failures are retained
in the verification logs; they are not counted as passing runs.

The final `cargo test --offline -j 2 --no-fail-fast -- --skip fault_matrix::`
completed successfully: 1,997 passed, zero failed, 17 ignored. Together with
the 16 completed unchanged fault tests, this covers all 2,013 active default
tests. Cargo inventories verify 2,030 default tests (17 ignored) and 2,066
`test-hooks` tests (18 ignored); only the 34 focused hooks tests were executed
for final verification. The final review uses this explicit split evidence.

Final Claude review (`fbe4d289-e7b3-4ef6-8b7d-546d51aa975d`, 2026-10-29):
**approved, no blockers for either PR**. The reviewer accepted the split default
verification for opening the PRs, confirmed no remaining obsolete aggregate
error assertions, and checked the docs/inventory consistency. Both PRs must
pass CI before merge, including full hooks coverage, final-base storage fault
tests and the docs-site link check. No merge or release is part of this task.
