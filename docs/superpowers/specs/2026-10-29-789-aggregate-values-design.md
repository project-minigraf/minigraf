# #789: align ordinary aggregate values with windows

Date: 2026-10-29. Target: v3.0.0 only.
Status: Claude-approved (2026-10-29); maintainer approved implementation on 2026-10-29.
Issue: https://github.com/project-minigraf/minigraf/issues/789
Inspected base: `c2e862fe8` on v3.

## Scope and philosophy

Make ordinary `sum`, `sum-distinct`, `min` and `max` accept the same argument
domain as their corresponding window functions. Audit the count variants.
This follows PHILOSOPHY.md's reliability and simplicity principles, with no
dependency, configuration, syntax, public API or storage/WAL change. Changed
query semantics ship only in the next major release; v2 runtime is untouched.
Ordinary `avg` belongs to #777. Ordered-group expansion belongs to #781.

## Contract

| Function | Accepted values | Existing group with only nil | Existing group with only nonnumeric non-nil values |
| --- | --- | --- | --- |
| count | Every non-nil value | Integer(0) | Number of values, including repetitions |
| count-distinct | Every distinct non-nil value | Integer(0) | Number of distinct values |
| sum | Integer and Float only | Integer(0) | Integer(0) |
| sum-distinct | Distinct Integer and Float only | Integer(0) | Integer(0) |
| min / max | Every non-nil value | Drop the ordinary group, as today | Least / greatest value by the existing window total order |

The all-nil min/max rule is an explicit preservation of ordinary result
structure: windows emit nil for each affected source row, whereas an ordinary
group with no extremum is omitted. Do not turn that into an error or fabricate
a nil ordinary result. An all-nonnumeric group has an extremum whenever at
least one value is non-nil, including an empty set.
Ordinary avg (#777) and an unstepped registered aggregate instead return nil
for an existing all-nil group. That intentional difference remains; do not
document empty-group handling as uniform across functions.

No matching rows remains distinct from an existing group. Preserve the exact
`engine/plan.rs` `zero_row` predicate: only a query with no plain find variable
and exclusively count/count-distinct aggregates synthesizes a zero row. This
also preserves current `:with` behavior rather than inventing an empty group
rule. Any sum, sum-distinct, min or max in an otherwise empty query means no
rows. Windows over no rows emit no rows. A constructed empty accumulator is
not evidence that a query should synthesize a row.

Skipping an argument never filters a source row, changes grouping or `:with`,
deduplicates ordinary sum/count, or removes a bounded-window position. Keep
the existing aggregate/window syntax restrictions. Registered aggregates and
their nil/callback behavior are unchanged.

### Numbers, ordering and distinctness

Reuse `Sum::add` and `Sum::value` unchanged. Integer-only totals saturate once
to i64; any numeric Float makes the result Float. The exact accumulator gives
input-order-independent results. NaN or opposing infinities produce NaN;
one infinity sign produces that infinity. Numeric -0.0 alone yields -0.0;
another numeric zero or exact cancellation produces +0.0. Skipped values
neither promote the result type nor affect the zero sign.

Reuse `Extreme::add` / `value_order` for min/max, without type validation.
Numbers compare by exact numeric value, Integer before an equal Float; float
ties use total_cmp, and NaNs follow all other numbers. The remaining classes
are strings, booleans, refs, keywords, nil, sets. Nil is skipped. Sets compare
lexicographically by elements sorted using this same order. This is the
existing window order, NOT `Value::Ord` (which still governs group equality
and distinctness). No comparator changes are part of #789.

Keep current `Value::Ord` distinctness and `canonical`: Integer(1) and
Float(1.0) are distinct; float signed zeros collapse to +0.0; all float NaNs
collapse to one canonical NaN. Thus sum-distinct of only -0.0 is +0.0, unlike
sum of those rows. Compare sum-distinct with sum over the explicitly
canonicalized distinct numeric population, not over the original rows.

Filter sum-distinct before insertion AND before `would_add` memory charging.
Nonnumeric values do not grow its retained set. Count-distinct retains every
non-nil type and its current accounting. Upstream decoding, row storage and
IdSpace costs still apply even when an aggregate ignores an argument.

## Implementation and errors

In `src/query/datalog/aggregate.rs`:

- Change `Agg::Sum(Sum, bad_type)` to `Agg::Sum(Sum)` and ignore false from add.
- Remove Extreme's kinds bitmask and its now-unused kind/KINDS helpers;
  preserve best-value selection and ordinary `Ok(None)` on no extremum.
- Separate sum-distinct from count-distinct in step and would_add, with the
  identical numeric predicate for the former. Finish sums only retained
  numeric members through the existing Sum accumulator.
- Retain type_name if other callers use it. Update stale comments and enum
  pattern matches, including any #777 heap-accounting helpers.

INT-041, INT-043 and INT-044 cease to be emitted by these built-in aggregate
paths. Keep their public ErrorCode variants, registry entries and stable text;
mark their reference table titles "(deprecated)" following the existing
INT-033..035 convention, and explain the v3 behavior. INT-042 already has
no emission site on the inspected v3 base: it describes the ordinary min/max
group-drop condition, not an error returned to a caller. Preserve that rule
and clarify the reference rather than claiming a newly removed error.

Keep full-drain evaluation, group order and find-order finishing unchanged.
An earlier empty min/max can still drop a group before a later UDF finalizer;
a preceding failing UDF finalizer still wins. Step failures, decoding/storage
errors, QRY-012 budgets/checked allocation failures, INT-032 pipeline errors
and remaining UDF errors remain
observable through their existing paths. Data that formerly caused a removed
type error now produces a result or the next applicable real error. Tests
must assert this explicitly, including `:limit` / `:offset`.

For #781: sum and min/max no longer have aggregate type errors, but this does
not prove streaming eligibility. Preserve the count/count-distinct-only
ordered path and every fallback. Sum-distinct remains growing state. Whole
pipeline order, error timing and memory proofs remain #781's responsibility.

## Coordination with #777

The local #777 worktree has in-progress ordinary avg and Sum heap accounting.
Do not copy or edit it. Reconcile against its landed v3 revision before code
delivery; preserve Avg, its parser/grammar work, and exact-accumulator charges.
If #777 is still pending, keep changes independently scoped and explicitly
review the shared enum/helper conflict before integration. This spec does not
authorize silently dropping either change. Test both contracts together when
the dependency is available.

## Evidence and acceptance

1. Shared-accumulator tests cover all eight Value variants, mixed classes,
   sets with mixed scalar elements, nil, signed zeros, multiple signed/payload NaNs, infinities,
   i64 extrema, integers beyond 2^53, and cancellation. Exhaust small
   adversarial permutations. Explicit expected values accompany comparisons
   with Running and bounded windows; use bits for zero and NaN ordering.
2. Compare each ordinary aggregate with its corresponding full-partition
   window population, including grouped/partitioned cases. Default cumulative
   windows compare at the final row; bounded windows compare each selected
   argument slice with an ordinary accumulator. For all-nil extrema, assert
   the deliberate ordinary-drop/window-nil structural distinction. Distinct
   variants have no built-in windows; assert exact distinct counts and compare
   sum-distinct to the canonical numeric population.
3. Public queries cover grouped/ungrouped results, all-nil/nonnumber/empty
   cases, mixed find lists, :with, repeated values, multiplicity, prepared
   binds, cursors across batches, recursive/rule queries, temporal selections,
   staged write transactions and pending/checkpointed/reopened snapshots.
   Preserve min/max group drops even beside count/sum/avg.
4. Force legal plan alternatives with existing engine test facilities and
   compare values, type/zero/NaN details and surviving errors. Cover error
   order, cursor close/rewind, memory cleanup and Limit/Offset. Frozen answers
   change only for the intentionally removed aggregate type restrictions.
5. Meter tests prove sum-distinct's skipped types add no retained members,
   accepted distinct numbers are charged once, duplicates add no charge,
   count-distinct retains its charges, and release/failed-step cleanup works.
   Test QRY-012 boundaries without claiming upstream memory is free.
   Sum-distinct's finishing Sum uses temporary Exact scratch, already outside
   the retained-state accounting model. Do not expand that model in #789.
6. On the bench machine, verify baseline then red/green targeted tests, full
   default suite, clippy's repository gate, fmt, portable feature/target checks
   and release binary size against the unchanged cap. Capture exact revisions
   and commands. No new performance claim or broad benchmark project.

## Documentation and delivery

Synchronize current README, ROADMAP, CHANGELOG (unreleased v3), CLAUDE,
docs/TEST_COVERAGE and docs/ERROR_REFERENCE. Audit current aggregate comments
and references. Preserve historical design/release records; this spec supersedes
the ordinary type restrictions in #547 without rewriting its history.

Use the existing sibling minigraf-docs checkout for a separate PR. Scope the
aggregate table, examples, nil/empty behavior and error descriptions using
`@until v3.0.0` / `@since v3.0.0`. Preserve v2's restrictions and avg behavior;
coordinate with #777's site changes. Audit architecture, comparison and
use-cases for affected claims; update only relevant text. Inspect site.toml's
existing v3 entry; do not invent a release or bump versions.

Claude reviews this spec and the accompanying plan before implementation;
maintainer design approval is required. Claude reviews the final implementation
before core/docs PRs are raised. No merge, release or v2 runtime backport.

## Implementation clarifications

The approved test matrix's reference to recursive sets was corrected to sets
with mixed scalar elements: ValueSet rejects nested sets (API-021). #789 does
not add nested-set support. All other approved semantic boundaries are unchanged.
