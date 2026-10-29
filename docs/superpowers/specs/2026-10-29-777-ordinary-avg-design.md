# #777: ordinary average and accurate v2 documentation

Date: 2026-10-29. Target: v3.0.0, matching #777's milestone.
Status: Claude-approved (second review, 2026-10-29); maintainer selected matching window semantics
on 2026-10-29. Implementation is on feat/777-avg.
Issue: https://github.com/project-minigraf/minigraf/issues/777
Base: v3, rebased onto `c2e862fe8` (#774); v2 documentation is a separate main-based change.

## Scope and philosophy

Expose `(avg ?v)` through v3's existing shared aggregate path. Correct v2's
misleading documentation without changing its parser or runtime. This follows
PHILOSOPHY.md's reliability, simplicity, portability and documentation principles:
no dependencies, configuration, API or storage/WAL format changes.

## Ordinary avg contract (v3.0.0)

- Accept one variable, using the existing ordinary aggregate syntax and binding
  checks. Plain `:find` variables and `:with` retain their existing grouping
  behavior. Feed avg precisely the same rows an existing aggregate receives;
  do not add distinctness or change row multiplicity.
- Take Integer and Float values; skip nil and every nonnumeric type (including
  strings, booleans, keywords, refs and sets). Count only numbers. This follows
  the existing window avg. The maintainer explicitly requires consistency
  between ordinary and window forms; this supersedes the briefly selected
  nonnumeric-rejection option. Other functions' changes belong to #789, not this
  implementation. Numeric count differs from count, which includes nonnumbers.
- A nonempty numeric population always yields Float, including integer-only
  populations and exact integral results. An existing group with no numeric
  values yields nil and is not dropped. With no matching bindings, grouped and
  ungrouped queries containing avg yield no rows, preserving the existing
  non-count aggregate empty-input rule (`engine/plan.rs`'s `zero_row`). This
  includes `(count ?v) (avg ?v)` and queries with `:with`. Distinguish this from
  a constructed empty accumulator, whose `mean` is nil.
- Use the existing `Sum::mean` unchanged: accumulate integers and finite floats
  exactly, round the total once to nearest f64 (ties to even), then divide by
  the numeric count converted to f64. Integer totals are not saturated before
  division. This is an order-independent floating-point mean, not a promise of
  a correctly rounded exact rational mean. The final division rounds separately.
- In particular, a finite total exceeding f64 range becomes signed infinity
  before division, even when the mathematical mean would be finite (e.g. two
  f64::MAX values). Preserve that existing window behavior and document it.
- Any numeric NaN or both infinity signs produce NaN. A single infinity sign
  produces that infinity. All numeric inputs equal to -0.0 produce -0.0; exact
  cancellation or a zero total with another numeric value produces +0.0.
  A nonzero negative total can underflow to -0.0 on division (e.g. the smallest
  negative subnormal plus two +0.0 values, divided by three). Test both signs
  of underflow, zero signs via float bits, and NaNs with is_nan.
- Execution order and legal plan choice cannot change avg's result. This covers
  mixed integer/float cancellation and integer values above 2^53. Storage's
  existing normalization of values is unchanged.
- Other aggregate failures and group dropping keep existing find-order/group
  precedence. Adding avg must not suppress a neighboring aggregate's error.

## Implementation boundary

Add `Agg::Avg(Sum)`; construct it for Builtin::Avg, step it with `Sum::add`
(ignoring nonnumeric values), and finish it with `Sum::mean`. The engine's
`Acc::Values` already delegates to Agg. Keep Sum's arithmetic, Running,
SlidingSum, and window evaluation unchanged. Do not generalize the ordered
Aggregate optimizer.

Account for the retained Box<Exact> (36 u64 limbs) used by Sum-backed ordinary
accumulators. Add private Agg helpers for the additional heap bytes a step
would allocate and the heap bytes already retained. Before the first finite
float is stepped into Sum or Avg, charge that allocation once through Held;
integers, NaN, infinities and subsequent floats add no such charge. Transfer
this charge with the accumulator when building the sorted finishing map,
alongside existing sum-distinct member charges, and release it on finish,
failure, rewind and close. No arithmetic or input-type behavior of other
aggregates changes. This closes the same pre-existing Sum accounting gap rather
than adding an uncharged allocation for Avg. Temporary arithmetic scratch is
outside the existing retained-state accounting model. Test exact charge deltas
and a QRY-012 boundary on the first float, including cleanup.

Remove avg from parser WINDOW_ONLY, preserving rank/row-number restrictions
and current lag/lead restrictions. Replace the invalid avg fixture with a
genuinely window-only expression, and add valid ordinary avg fixtures. The
structural grammar already accepts generic aggregate names; do not specialize it.

Prepared queries, cursors, rule queries and write-transaction queries must use
the same semantics. Aggregate/window combinations retain PRS-082 restrictions.

## Required evidence

Before production edits, add failing tests for parser acceptance, shared Agg,
and the public API reproduction (10 and 20 => Float(15)). Test the contract's
empty/nil/nonnumeric, mixed, large integer, special float and zero cases. Run
all permutations of small adversarial cancellation populations against explicit
expected values; include a rounding-sensitive integer case and finite-sum
overflow. Preserve and run existing window tests, including sliding frames.

Integration coverage must include grouped/ungrouped/:with multiplicity,
coexisting aggregates and their errors, prepared bind execution, multi-batch
cursor reads, rules, and legal plan alternatives on pending versus checkpointed
data. Assert semantic outcomes, not just agreement between two implementations.
Run parser fixture/grammar checks and existing frozen-answer engine tests
(tests/answers/; the old reference executor was removed in #551). New avg tests
use explicit expectations and plan comparisons, not generated snapshots.
Run the full default test suite, wasm feature checks, fmt and the repository's
clippy gate on the bench machine; inspect binary-size impact without changing
the budget. Capture baseline and final results on exact revisions.

## Documentation and delivery

1. A main-based documentation PR removes ordinary avg claims from PRS-021,
   PRS-024, INT-029 and any other current documentation. Name the six ordinary
   built-ins: count, count-distinct, sum, sum-distinct, min, max. State that v2
   avg requires :over. Correct PRS-027's unsupported function list and use a
   real PRS-027 example (rank ?score); bare (rank) gives PRS-024.
   No runtime changes and no new release/version tag.
2. A v3 PR implements ordinary avg, tests it, and synchronizes
   docs/ERROR_REFERENCE.md, README.md, ROADMAP.md, CHANGELOG.md, CLAUDE.md and
   docs/TEST_COVERAGE.md. Record the feature in unreleased v3.0.0 only. Audit
   related current references; do not rewrite historical release claims.
   Fix v3 PRS-027 to name rank/row-number only; QRY-010's existing avg list
   becomes accurate. Explicitly record that ordinary avg no longer raises
   PRS-027. Use the v3.0.0 milestone in current release-direction references
   after retiring #383; preserve historical records.
3. A docs-site PR scopes compatibility tables and aggregation reference with
   @until v3.0.0 / @since v3.0.0; preserve v2's six built-ins. Document ordinary
   avg type, skipping, empty and rounding behavior. Audit architecture,
   comparison and use-cases for affected claims; update only where relevant.
   Check site.toml's existing v3 entry; no invented release or version bump.
4. #781 receives the reviewed contract: avg is an infallible, fixed-state
   aggregate over decoded values, skipping nonnumbers. Its ordered eligibility
   still requires #781's pipeline/order/memory analysis. No optimizer expansion
   in #777. Keep the issue open until both branch scopes and site docs land.

Claude spec and implementation-plan reviews precede implementation. The
maintainer directed the avg-only implementation with matching window handling;
#789 tracks other functions as a v3.0.0 requirement. Claude reviews the final
diff before PRs are opened. No merge/release without explicit authorization.
