# #781: extend ordered Aggregate with bounded, proved cases

Date: 2026-10-29. Target: v3.0.0, no v2 runtime backport.
Status: Claude conditionally approved (second review, 2026-10-29).
All first-review design blockers resolved. Maintainer approved implementation
and the common result-construction error policy on 2026-10-29. The later
aggregate slice still requires the audit of merged #789.
Issue: https://github.com/project-minigraf/minigraf/issues/781
Inspected base: `0fc97bd65` on v3 (#777 merged).
Plan: `../plans/2026-10-29-781-ordered-aggregate.md`.

## Scope and dependencies

Extend #641's one-group Aggregate without changing answers, aggregate value
semantics, callbacks, storage, syntax, configuration or dependencies. This
follows PHILOSOPHY.md §§4–8: optimize established embedded execution paths,
with reliable fallback taking precedence over optimization coverage.

#777 is complete: ordinary avg skips nil and every nonnumber, counts numbers,
and has no aggregate data error. The briefly selected rejection contract is
superseded. #789 is still in progress in a separate worktree: it makes sum and
sum-distinct skip nonnumbers and min/max accept the window value domain. Do not
edit or copy that worktree's implementation. Rebase the aggregate-expansion
slice onto its merged v3 revision and recheck its final contract before enabling
sum/min/max. The proof/tuple work and avg can be developed independently.

This delivery deliberately has finite coverage:

- Existing count/count-distinct plus ordinary avg and sum.
- Min/max when their arguments are proved fixed-size scalar values.
- Sum-distinct when its argument is proved Integer or nil, avoiding its
  finish-time floating-point heap allocation.
- Effective grouping tuples that reduce to one proved varying scalar slot
  after removing proved constants and repeated slots, including hidden :with
  keys. No new proof of two independent varying columns.

Every other case keeps full-input Aggregate. Classifying a built-in as a
fallback is an explicit outcome, not an assumption that it is unsupported.
Floating sum-distinct, arbitrary-value extrema and independent composite keys
require a separate reviewed extension; do not mark them implemented in docs.

## Current code and semantic constraints

`engine/plan.rs` constructs the effective group tuple from plain :find variables
in find order, followed by :with variables. It marks only non-window queries
without :with as candidates. `engine/order.rs` proves a single committed scalar
source slot, follows unchanged outer slots through selected probes, and enables
only count/count-distinct. `engine/aggregate.rs::next_ordered` already retains
one active group and emits at most one bounded batch per child batch. Its
monotonicity assertion currently compares only the first grouping component.

Full-input Aggregate drains/steps all input before finishing groups in
`Value::Ord` tuple order, then aggregate find order. Min/max with no non-nil
argument drops the group and prevents later finalizers in that group from
running. Preserve that behavior, including when avg or count shares the group.
Keep the planner's `zero_row` predicate unchanged: no plain find variable and
exclusively count variants can synthesize a zero row, including the existing
:with case. The ordered operator does not implement that case today; add its
empty-input emission before making :with eligible (see Holders and lifecycle).
Other empty queries produce no row. Ungrouped execution stays unchanged.

Group equality/order uses `Value::Ord`; extrema and query Sort use
`aggregate::value_order`. Numeric values of different variants remain distinct
groups. Never substitute one comparator for the other.

## Aggregate classification

The following describes the post-#789 contract. Before that dependency lands,
sum/sum-distinct can raise INT-041 and min/max INT-043/044; those functions stay
on full drain in this work until their merged semantics are verified.

| Built-in | Result and edge cases | Ordered eligibility in this delivery |
| --- | --- | --- |
| count | Counts non-nil values, saturates at i64 max; nil-only group gives 0 | Existing behavior |
| count-distinct | Distinct non-nil values under existing group equality; nil-only group gives 0; state grows with distinct population | Existing behavior, extended safe tuple shapes |
| avg | Numbers only; no numbers gives nil in an existing group; Float result; fixed-size exact accumulator | Eligible with the complete group-order proof |
| sum | Numbers only; no numbers gives Integer(0); integer-only sum saturates once; any numeric Float promotes result | Eligible after #789 with the complete group-order proof |
| min/max | Skip nil; no value drops the group; use the existing window total order | Eligible after #789 only for proved Integer/Float/Boolean/nil arguments; other domains retain full drain |
| sum-distinct | Distinct numbers only; no numbers gives Integer(0); canonical zeros/NaNs, Integer(1) distinct from Float(1.0) | Eligible after #789 only for proved Integer/nil arguments; any possible Float or unproved domain retains full drain |
| Registered aggregate | Existing callbacks/errors, including empty handling | Always full-input |

Sum/avg reuse `Sum` without arithmetic changes: exact finite accumulation;
NaN or opposing infinities gives NaN; one infinity sign survives. All numeric
-0.0 gives -0.0, otherwise zero/cancellation follows the existing accumulator.
Avg rounds the sum before division: even a finite mathematical mean may become
infinity if the intermediate rounded sum overflows. Integer totals beyond 2^53,
i64 saturation, subnormal mean underflow and both zero signs keep #777 behavior.
Min/max preserve `value_order`'s mixed numeric and NaN/zero rules. Float argument
values are permitted for extrema even though float *composite group components*
are excluded below. No normalization beyond existing aggregate behavior.

An eligible query must qualify for every aggregate in its find list. One
unproved/fallible aggregate keeps the entire Aggregate on full drain, with no
early result or Limit stop. Do not emit the valid prefix of a query that could
later fail aggregate data validation.

## Snapshot proofs and complete grouping tuples

Reuse the actual pinned Stream snapshot, KeySource, TxBound and registry.
Prepared queries recompute proofs on every open. No proof survives into another
query or reads the live mutable store instead of the captured view.

Keep #641's bounded raw AVET kind-prefix existence seeks, conservative historical
over-approximation, single-range source restriction and pending/staged exclusion.
Unrelated pending attributes do not disqualify a source; in-range pending
assertions/retractions do. Large transactions must use their working reader.
Optional proof I/O failure abandons the candidate atomically and lets the
ordinary query determine the real error. No first-row sampling or full scan to
establish a type/order. No broader scan replacing a selective entity prefix.

Represent the effective tuple explicitly in the choice, keeping original output
positions. A tuple qualifies if:

1. Every component has a permitted representation and `JoinKey::group` equality
   agrees with `Value::Ord` equality for that representation.
2. Removing proved constant slots and repeated occurrences of the same slot
   leaves exactly the existing ascending source slot. An all-constant tuple or
   an empty tuple remains on the existing path; no extra global-group strategy.
3. No operator changes a certified slot or invalidates the source order.

For a single component, retain #641's Integer, Boolean, Float and mixed numeric
group domains. For tuples of more than one component, initially permit only
Integer/Boolean/nil components, including proved constants; reject every Float,
string, set, ref/entity or keyword component even if it appears constant. This
is conservative and avoids representing constant payloads with unsafe equality
or ordering assumptions. The nonconstant source remains Integer or Boolean.
No arithmetic monotonicity inference, string encoding proof or DICT semantic
ordering proof is added. A constant claim alone is not a domain claim: record
or conservatively infer its scalar domain, otherwise reject the tuple.

Examples that should qualify on committed Integer :bucket data:

```clojure
[:find ?g (avg ?v) :where [?e :bucket ?g] [?e :item ?v]]
[:find (count ?e) :with ?g :where [?e :bucket ?g]]
[:find ?g ?k (count ?e) :where [?e :bucket ?g] [(+ 1 1) ?k]]
[:find ?k (count ?e) :with ?g :where [?e :bucket ?g] [(+ 1 1) ?k]]
```

These examples assume the physical plan scans :bucket first and preserves that
slot through subsequent probes; tests must assert that plan or use the existing
reorder hook. The latter two need the binding's constant scalar domain proved from existing
expression semantics; unsupported constant expressions may conservatively
fall back. Prefer literal bindings and existing fixed :db/valid-at metadata
where syntax allows. Repeated plain variables must preserve their output
positions. A tuple (?g, ?v) from a probe remains unproved even if each ?g's
rows are contiguous: the inner values are not proved ascending. Likewise, a
prefix whose hidden :with suffix varies independently is not sufficient.

Generalize the boundary assertion to compare the *whole* decoded effective tuple
under `Value::Ord`. It must increase when the group key changes; a decrease or
reappearance yields INT-032, never fallback after output. Equal tuples remain
one group across batch boundaries. This assertion is defense, not a substitute
for a source proof. Single-column float representatives retain today's rule.

## Argument-domain proofs

Group order and aggregate argument domain are separate facts. Avg/sum do not
need a numeric input-domain proof after their semantics are merged. Min/max
need the fixed-scalar proof and sum-distinct the Integer/nil proof above.

Introduce a small private slot-domain mask independent of the order domain.
Seed it from the same bounded raw kind probes for the committed source value,
integer metadata, known nil/integer/boolean constants, and narrowly supported
constant expressions. For an argument introduced by an outer-preserving entity
probe on a constant attribute, a whole-attribute kind proof is a conservative
superset. Check pending/staged facts for that attribute, using the same captured
snapshot. Initially reject derived sources, dynamic attributes, replaced slots,
unproved join outputs and dynamic expression results. Cache each proof once per
attribute per open, never across snapshots. Unsupported kinds simply yield no
certificate. Domain-only probes must not change the join's chosen access path.

The domain mask includes stored `ValueKind::Null` as nil. Keep this separate
from the existing source-order classifier, which rejects mixed Null ranges:
accepting Integer/Null as an *argument* domain does not certify their index
order as a grouping domain. A domain with no raw keys is an empty set of
possible values, not evidence that a row exists.

Initially infer expression domains only for constant expressions already
recognized by Ordering::constant: a literal has its exact Value kind; a
reference copies its proved constant slot domain; arithmetic over exclusively
Integer operands yields Integer on surviving rows, numeric arithmetic with
Float possibilities conservatively yields Integer/Float; built-in comparisons
and type predicates yield Boolean on surviving rows. Non-numeric arithmetic
and invalid operations can yield NoValue and drop a row; do not evaluate them
early, invent a row or treat NoValue as nil. Reject unsupported expressions,
unsubstituted bind slots and UDFs. Fixed ValidAt metadata is Integer/Null;
other time/count metadata is Integer. Out::Check proves no new constant/domain
because join equality can accept a different representation. Out::Replace
invalidates prior facts before any new constant fact is installed. No inference
of monotonicity or domains for dynamic expressions is added in this delivery.

Inspect the actual join output/replacement mapping; a type claim about an inner
value cannot be attached to a different outer slot. Preserve only facts known
through all possible ProbeValue fallback orientations. No new join planning or
order-preservation rule is introduced. Queries with no eligible demand perform
no new proof reads. Limit zero keeps the existing no-proof/no-input behavior.
Extend candidate's existing no-I/O speculative pass to include the new
aggregate/argument-domain demands before doing any kind-prefix reads; do not
make proof I/O the way to discover that no eligible consumer exists.

## Finishing, errors and Limit

For newly eligible accumulators, finishing decoded state must produce only an
inline scalar or a group drop. Sum/avg's Exact allocation occurs while stepping,
with #777's charge-before-growth. Restricted min/max retains a scalar and needs
no dictionary lookup, long-value read or variable payload clone at finish.
Integer-only sum-distinct constructs an integer Sum without allocating Exact.

The exclusions are material: general min/max may require retained strings/sets
and `Cell::of_value` reads/interning at finish. Floating sum-distinct creates
temporary Exact scratch through infallible Box::new at finish; today that is
unmetered on both paths, not a recoverable allocation error. Its exclusion is
conservative allocation/accounting scope, not a claim that it currently returns
QRY-012 there. If that allocation becomes checked/charged, its error must not
move ahead of later input failures. Do not move it into step just to enable the
optimization. General extrema also have a pre-existing uncharged retained-value
payload on the full-input path; this issue does not expand that path to ordered
execution or claim to repair its accounting. Track that independently during
implementation review. Restricted scalar extrema avoid the variable payload
and result-conversion I/O/interning concerns.

Separate common result-construction failures from accumulator finish failures
in tests and documentation. finish_one still performs a scratch charge, a row
charge and a checked row reserve. A QRY-012 at those sites can precede a later
source I/O error which full-input Aggregate would encounter first. This is an
explicit proposed preservation of #641's lazy resource-error contract for the
newly eligible paths, not a claim of identical error ordering for every query.
The maintainer explicitly approved this distinction on 2026-10-29. It does not
waive full-drain precedence for aggregate-specific finalizer errors.

After auditing #789's actual merged code, each admitted accumulator must have
no recoverable aggregate-specific finish error to reorder against later input
I/O, QRY-012 or INT-032. That requirement is unconditional; the whitelist is
conditional on the audit confirming it. No exception is made for a newly
checked/charged sum-distinct finalizer. The existing count path's infrastructure
error behavior is the comparison for the proposed resource-error contract.
Unchecked global allocator OOM can abort the process; this proposal does not
claim to turn Rust's infallible Box/BTreeSet allocations into recoverable errors.
Preserve all existing checked reserve and meter failures. If implementation
introduces a recoverable finish error, the affected path is ineligible until
its full-drain precedence is explicitly preserved and reviewed.

Full-drain differential tests must compare any remaining competing errors,
especially a fallback finish failure versus a later upstream error. Tests should
inject errors at their actual step/finish boundary, not pretend a global OOM is
a Result. A normally fallible function on the inspected base cannot be enabled
on the strength of #789's unmerged design.

Limit/Offset counts emitted groups, after min/max drops. Dropped groups consume
input but no offset; an arbitrarily long run of dropped groups gives no bounded
startup guarantee. Eligible cursors may expose completed batches before a later
storage/resource error, and Limit may leave unread input, as in #641. A retained
Sort still drains its child; no early-stop promise when aggregate-result order
requires Sort. Preserve all existing no-pull behavior for limit zero.

Do not assume that a complete *effective* grouping tuple is unique in the
visible projection. Hidden :with keys can distinguish groups with identical
visible keys; an order by visible keys may require aggregate-value tie sorting.
Add `plain_group_count` (or equivalent private metadata) to Step::Aggregate;
the ordered resolver currently cannot distinguish plain from hidden keys.
After Aggregate, map the certified varying source slot to its first actual
output group-column position, retain equivalent positions for repetitions, and
map each constant to its output position. Do not keep today's hardcoded slot 0
or discard all constants. With no :with keys, mark the complete reduced group
key unique only after correctly remapping it. With :with keys, clear uniqueness
and explicitly disable Sort omission for this query, even if repeated/constant
hidden keys could theoretically be proved harmless.

Only remove Sort when all nonconstant effective grouping dimensions are matched
through actual output columns and the full existing Sort comparator is proved.
Mixed numeric group order remains insufficient for numeric Sort. Initially keep
Sort for every query with :with; this isolates tuple support from a new hidden-key
uniqueness proof. Preserve plan.find/Sort.cols remapping for existing omissions.

## Holders and lifecycle

Reuse one Group, its accumulators, and bounded input/output batches. No list of
finished groups, deferred output spool or second validation scan. Extend
finish_one only where its scalar output assumption holds. Preserve zero-row
synthesis by adding explicit empty-input handling to next_ordered: on EOF
without ever stepping an input row, if zero_row is true, emit exactly one row
of groups.len() nil cells followed by aggs.len() integer zeros. Share the
full-input constructor where practical; its one synthetic passing row is
uncharged, so do not attempt to uncharge it or rely on Held's clamp. Project
away hidden keys normally. Reset the seen-input/zero-emitted state on
rewind and never emit a zero row merely because all observed groups dropped.

Preserve #549's accounting convention: group keys/cells, distinct members,
Exact's 36 limbs, existing finishing scratch and held result rows are charged;
bounded passing input/output batches and the current batch inside an operator
are not. emit_one uncharges a result row when moving it into the outgoing Batch,
as the full-input path does. This proposal does not introduce batch metering.
Charge retained growth before allocation and preserve checked reserve failures
as QRY-012. A sum-distinct set grows only with distinct eligible numbers.
Release group state only after finishing and charge retained payload transfers
once until the existing handoff to the uncharged passing batch. Audit actual
batch overlap separately in RSS/memory measurements.

Add Aggregate to Meter's private per-holder peaks, aggregating its held and
finished owners just as Window/IdSpace peaks do. Record Aggregate, IdSpace and
total-query peaks separately; counters.bytes_held after drain is not a peak.
Keep private instrumentation out of public APIs and release-size overhead.

Continue IdSpace::end_pull at internal child-batch boundaries, including pulls
that emit nothing. Never evict provisional IDs while retained cells use them.
For fixed committed fixtures, both Aggregate and IdSpace must be independent of
completed group count. A bounded Aggregate is not a guarantee that an upstream
join or provisional-ID population is bounded; measure and label those controls.
One growing count-distinct or integer sum-distinct group is intentionally linear.

Rewind, close, drop, errors, partially consumed batches, prepared reopening and
concurrent checkpoint/page reuse must release meters and preserve pinned values.

## Exclusions and regression matrix

Keep existing materialization/access order for any user callback (including
registered aggregates and nested expressions/not/or bodies), collect-set anywhere,
aggregate+window queries, and rule-query pipelines. Retain the recursive
descriptor-based detection; no name-based approximation. No new optimization
inside nested bodies. Compare callback counts, failures and rows explicitly.
Built-in registry names cannot be overridden: duplicate registration returns
INT-040. Test that invariant, not an impossible built-in override.

Use the existing private forced-full-input switch on identical pinned data;
add a narrower test hook only if needed to isolate source order from holder
strategy. Compare ordered lists/type/float bits for deterministic order and bags
where order is unspecified. Add explicit expected results, not merely agreement
between paths. Never regenerate frozen answers for this optimization.

Required cases include every built-in, all-nil/nonnumber/existing/absent groups,
mixed numbers, overflow, NaNs/infinities/zeros, distinct duplicates, >batch groups,
boundaries at batch edges, all effective tuple positions, hidden-only groups,
repeated columns, and all specified fallback key/argument kinds. Test no-data
count-only :with zero-row compatibility. Include prepared binds before/after
checkpoint, pending and staged argument/source ranges, unrelated pending data,
temporal/history disqualifiers, large-transaction working roots and rollback,
cursor pinning, and every invalidating operator/fallback orientation.

Inject out-of-order complete tuples, later source I/O, meter exhaustion and
checked allocation refusal. Verify no path with a remaining aggregate data
error emits early. Preserve full-input UDF/drop precedence. For arbitrary-value
min/max and floating sum-distinct, assert fallback and identical surviving error
precedence rather than only comparing successful rows. Meter boundary tests
cover the first finite float Exact allocation, distinct insertion, finish/output
overlap, and cleanup after every error.
Separately inject a common finish_one scratch/row-charge or checked-reserve
failure in an early group and a later child I/O failure: assert the documented
ordered QRY-012 versus full-input I/O outcome, using existing count as a control.
Keep this test distinct from aggregate-specific finish errors, whose precedence
must not change. Include constant-first ascending/descending Sort regressions
and hidden-only :with groups with visible aggregate ties.

## Measurements, portability and delivery

Run builds/tests and performance work on the designated bench machine, not the
developer laptop. Baseline against immutable merged v3 including #789; record
revision, toolchain, commands, workload, medians/ranges and machine conditions.

At 100K/1M/10M committed facts measure fixed-size groups separately for avg,
sum, eligible extrema, integer sum-distinct, count variants and safe tuple/:with
shapes. Compare optimized and forced-full-input strategies. Include selective
entity-prefix startup, raw historical kind proofs, no-demand ordinary queries,
Limit/Offset, and fallback controls. Build fixtures outside query subprocesses.

For each bounded fixture, Aggregate and IdSpace peaks at 1M/10M must each be at
most their 100K baseline plus a numerical allowance of two maximally sized
batches computed from the fixture's row/cell estimators. Those batches are not
actually meter-charged; the allowance is benchmark slack, not a definition of
meter ownership. Explain Aggregate peak as one active group plus finish scratch
and a temporarily charged result row. Report total meter and query-only RSS too:
10M RSS at most the 100K RSS baseline plus max(16 MiB, 15% of that baseline).
Count-distinct and integer sum-distinct
single-growing-group controls must show and explain linear state. Batch width
and aggregate Exact charges must be reflected in each allowance.

Positive limited fixtures without dropped groups/retained Sort must consume
at most the rows of offset + limit groups plus BATCH_ROWS child rows: the pull
finishes the entire batch containing the first row after the last requested
group. Express this at Aggregate's child boundary; joins can change source-row
counts. EOF without a following group needs no extra row. Measure at least five idle-machine
A/B samples after warmup; 10M limited-query median at most twice 100K. Retain
#379 point-query and existing ordinary-query guards (5% median regression gate,
investigating noise). Verify zero new proof reads for no-demand queries.

Run full default tests, targeted test-hooks/frozen-answer suites, fmt, repository
clippy, small-stack tests, wasm feature and WASI/browser committed-reader tests
where runners are available. Current v3 binary cap is **1,263,616 bytes
(1,234 KiB)** in binary-size.yml; do not use #641's historical 1,230 KiB number
or raise the cap. Use current CI target commands rather than inventing gates.

Synchronize affected current CLAUDE.md, ROADMAP.md, README.md, CHANGELOG.md and
docs/TEST_COVERAGE.md only when implementation changes their claims/counts.
Audit ERROR_REFERENCE for accurate fallback/error claims, preserving #789's
stable deprecated codes. Separate docs-site PR with @since v3.0.0 architecture
scope; audit datalog-reference/comparison/use-cases and site.toml, preserve v2,
and add no release/version/tag. The v3 milestone is the current work list;
#383 references are historical context, not a resurrected tracker.

Claude reviews spec/plan before implementation and code/evidence before PRs.
Report implemented and fallback cases separately in delivery. Do not claim the
issue's benchmarks passed from a spec review. No merge/release is authorized by
this document; remove task-created bench scratch after its last use.
