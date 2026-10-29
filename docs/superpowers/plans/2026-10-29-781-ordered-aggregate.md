# #781 implementation plan

Date: 2026-10-29. Status: Claude conditionally approved (second review).
Maintainer approved implementation and the resource-error policy on 2026-10-29;
the later aggregate slice still requires the merged-#789 audit.
Spec: `../specs/2026-10-29-781-ordered-aggregate-design.md`.
Inspected base: `origin/v3` at `0fc97bd65`.
Worktree: `.worktrees/781-ordered-aggregate`, branch `perf/781-ordered-aggregate`.

## 1. Settle scope and baseline

1. Obtain Claude review of the spec and this plan against the actual engine,
   correcting unsupported proof and error claims. Explicitly review the finite
   scope: one varying grouping dimension, scalar extrema, integer sum-distinct.
   Present the reviewed design for maintainer approval, explicitly identifying
   the proposed #641-style ordering of common result-construction resource
   failures versus later source failures. Aggregate-specific finish errors get
   no exception to #781's precedence requirement. If the maintainer requires
   full-drain precedence for common output infrastructure too, revise the design.
2. Confirm #777's merged ordinary avg contract and #789's eventual merged
   implementation. Do not modify the active #789 worktree. Refresh/rebase v3
   before the dependent aggregate slice, recording exact revisions and changes
   in error/allocation behavior. No optimizer enablement based on unmerged code.
3. Verify the ignored isolated worktree and repository instructions. This
   planning worktree already exists; do not recreate or reset it. Run the
   implementation baseline tests/build on an idle bench checkout before
   production edits. No Cargo builds or large fixtures on the laptop.
4. Preserve current frozen answers and current release-size limit. Record
   baseline holder/IdSpace/query/RSS measurements on immutable revisions before
   interpreting improvements. Use task-specific scratch and preserve others'
   workloads/caches.

## 2. Proof and tuple slice (TDD)

Files: `src/query/engine/order.rs`, `order/tests.rs`, `plan.rs`,
`aggregate.rs`, `aggregate_order_tests.rs`.

5. Add failing tests for count/count-distinct/avg with a hidden sole grouping
   key, repeated plain variables, and literal scalar constant + integer key in
   either tuple position. Assert strategy selection, exact rows, projection
   positions and complete tuple order. Include empty count-only :with input:
   ordered execution must retain the existing synthesized zero row.
   Assert the actual source plan or pin textual order using the reorder hook;
   eligibility must not depend on an unasserted optimizer choice.
6. Add negative tests first for independently varying inner keys, a hidden
   unproved suffix, float-containing multi-component tuples, strings/sets,
   scrambled DICT IDs, unknown constant domains, all-constant tuples, and slot
   replacement. Existing single-float grouping/NaN/signed-zero behavior stays.
7. Keep a private explicit effective-key certificate and domain facts; normalize
   constants/repeated slots only for proof matching, never remove output slots
   from the actual group tuple. Enable Candidate for non-window :with queries,
   requiring the full tuple certificate before Ordered. Add plain_group_count
   to Step::Aggregate (or equivalent metadata). After Aggregate, remap the
   varying slot to its actual first output position and retain repeated-slot
   equivalences and constant output positions. Replace the hardcoded slot 0,
   cleared constants and unconditional unique flag. With :with, clear uniqueness
   and explicitly disable Sort omission; otherwise uniqueness follows the
   correctly remapped complete reduced key. Test a constant-first tuple with
   `:order-by [?k ?g :desc]` and hidden varying keys with visible count ties.
   Land step 8's complete-tuple comparison before or together with this
   enablement: the old first-component check rejects constant-first tuples.
   Add next_ordered's missing zero-row emission: track whether input was seen;
   at empty EOF with zero_row emit hidden nils then integer zeros exactly once,
   as an uncharged passing row, without relying on clamped uncharge of a row
   never charged. Share the constructor where practical. Reset on rewind and
   distinguish empty input from observed groups that all dropped.
8. Replace next_ordered's first-cell monotonicity check with a complete tuple
   check. Inject (same leading key, decreasing later key) and recurrence across
   batches to prove INT-032. Keep group equality and representatives unchanged.
9. Admit Builtin::Avg through registry descriptors, not names. Test mixed
   numbers, skipped values, nil, no rows, large sums/means, NaNs/infinities,
   signed zero and Exact heap boundary accounting. One other ineligible
   aggregate still prevents streaming the entire Aggregate.
10. Run focused order/aggregate/avg/prepared/cursor/limit suites red then green
    on the bench. Review this independently useful slice before proceeding.

## 3. Argument proof and aggregate slice (after #789, TDD)

Files additionally: `src/query/datalog/aggregate.rs`,
`src/query/engine/meter.rs`, relevant shared aggregate tests.

11. Re-read #789's merged Agg::step/finish, retained-state helpers and tests.
    Document which fallible paths remain. Do not remove intentional fallback
    tests; update them to newly eligible versus still-unproved cases.
12. Add failing tests for sum on every input kind; scalar min/max over mixed
    integer/float/boolean/nil; integer sum-distinct with duplicates and a growing
    group. Compare expected answers and forced fallback. Test min/max dropping
    groups before a neighboring avg/count, with offset counting only emitted
    groups. No matching rows and existing nil groups must remain distinct.
13. Add argument domain proofs using raw per-attribute prefixes against the
    pinned KeySource. Seed source/metadata/constants; optionally certify only
    constant-attribute entity-probe outputs whose mapping is unchanged. Reject
    dynamic/replaced/derived values. Demand-drive and cache proofs per open.
    Test same source/argument attribute reuse and zero reads with no demand.
    Argument masks must include stored Null independently of group-order
    domains. Implement/test the spec's limited constant expression rules:
    literals, constant slot references, numeric arithmetic on surviving rows,
    Boolean comparisons/type predicates and Integer/Null ValidAt metadata.
    NoValue keeps dropping rows; Out::Check establishes no new constant/type.
    Extend candidate's no-I/O speculative demand pass before adding new proof
    reads, so unsupported/no-demand queries perform zero new kind-prefix seeks.
14. Add disqualifying kinds late in history, outside valid time and in pending
    or staged argument ranges; verify conservative fallback. Cover working
    large-transaction reader and rejection rollback. Optional proof-read I/O
    failure must preserve the complete original plan and real query error.
15. Admit sum, restricted min/max and integer-only sum-distinct only after all
    aggregate descriptors, arguments and group tuple qualify. Keep floating
    sum-distinct, arbitrary-value extrema and unproved argument expressions
    materialized. Never infer argument type from the group key or first row.
16. Audit finish_one's inline scalar assumptions and group-drop path. Preserve
    exact heap charging in step, integer distinct set charges, ownership during
    finish, and all existing checked reserves. Add no new recoverable
    accumulator-finish allocation to admitted cases. If required by the actual
    merged implementation, retain fallback and revise the spec before enablement.
17. Test error competition explicitly: earlier fallback finish failure versus
    later source I/O/QRY-012/INT-032; eligible step failures and output-buffer
    failures; full drain versus Limit and partial cursor pulls. Use local
    operator/allocation fixtures, not an unreliable real global OOM.
    Specifically verify the proposed infrastructure exception: an early
    finish_one scratch/row charge or reserve failure reports QRY-012 before a
    later child I/O error, whereas full input reports that I/O error. Use the
    existing count path as control. Do not apply this exception to any remaining
    aggregate-specific finalizer error; such a query remains ineligible.

## 4. Memory, lifecycle and excluded pipelines

18. Add failing Aggregate per-holder peak assertions, then private Meter peak
    tracking alongside Window/IdSpace. Aggregate's held and finished owners
    contribute together. Test exact charges around first finite float, a new
    integer distinct member versus duplicate, result transfer and close/error.
    Preserve the existing uncharged passing-batch convention: release a result
    row's charge when emit_one hands it to Batch. Test retained ownership, not
    an invented batch charge. Note the existing full-input variable-extremum
    payload accounting gap for separate tracking; scalar eligibility avoids it.
19. Measure many fixed groups versus one growing distinct group independently
    of result collection. Avoid retaining the full public result in a purported
    bounded-query measurement. Evict only transient DICT caches at internal
    input-batch boundaries; never provisional IDs. Test id-heavy arguments and
    joins as separate controls with honest whole-query bounds.
20. Run prepared execution before/after writes/checkpoint, partial cursor close,
    rewind/drop, checkpoint/page-reuse pinning, pending/staged/large transaction
    cases. Require meter.used == 0 after every lifecycle terminator.
21. Prove callbacks, registered aggregates, nested UDFs, collect-set in nested
    bodies, aggregate+window and rules keep original strategies. Assert callback
    counts, group-drop/finalizer ordering, late errors and answers under limit.
    Keep existing plan-independent frozen answers unchanged.
    Duplicate built-in registration must remain INT-040; no override is possible.

## 5. Bench evidence and release gates

22. Extend existing `scripts/input_order_group_fixture.rs` and probes or add
    narrowly scoped companions. Existing group probe hard-codes count answers;
    change validation per workload, not only labels. Compile probes against
    baseline and implementation; use private test instrumentation for holder
    peaks and real release subprocesses for timings/RSS.
23. Generate 100K/1M/10M committed fixtures once outside timed processes. Cover
    fixed-group avg/sum/extrema/integer-distinct and count variants, safe :with
    and constant tuple shapes, selective EAVT entity prefixes, raw historical
    proofs, no-demand ordinary queries, limit/offset, and fallback controls.
24. Record Aggregate and IdSpace peaks separately, their 100K-plus-two-batch
    numerical allowance using fixture-specific row/cell estimates, total
    meter/RSS, rows read and timing samples. Passing batches are uncharged;
    the allowance is measurement slack. Explain the retained bound as one group,
    finish scratch and a temporarily charged row. Record Exact bytes per accumulator. Test linear
    count-distinct and integer sum-distinct growing-group controls separately.
25. Use established idle-machine NUMA A/B protocol with immutable revisions,
    warmup and at least five samples. Apply the spec's limited startup/RSS gates,
    existing #379 point-query and ordinary-query regression guards. Investigate
    noise instead of changing thresholds. Record actual benchmark filters/IDs;
    adding a workload is not evidence that nightly CI executes it.
26. Run current required default/test-hooks/frozen-answer tests, fmt, clippy,
    small-stack and portable-target checks from CI on the bench. Exercise a
    real committed MemoryBackend optimized path in WASI/browser runners where
    available. Build release and compare with 1,263,616-byte cap; never raise it.
    Report unavailable gates explicitly. Run additional suites only for relevant
    failures/new changes. No storage encoder changes, so no new Kani harnesses.

## 6. Documentation and review

27. Update current core docs only to advertise the delivered whitelist and
    fallback scope; synchronize CLAUDE, ROADMAP, README, CHANGELOG and
    TEST_COVERAGE. Cross-check ERROR_REFERENCE. Do not alter historical #641 or
    #777 design contracts, released v2 documentation, version numbers or tags.
28. Prepare a separate docs-site PR with version-scoped architecture/performance
    details. Audit Datalog reference, comparison, use cases and site.toml;
    explain hidden :with grouping and limit/drop caveats where relevant. No new
    syntax or aggregate semantics are introduced by this optimization.
29. Obtain Claude final code/evidence review and resolve findings before core
    and docs PR creation. Split core delivery by proof/tuple + avg and remaining
    aggregate eligibility if useful, with each independently verified against
    its recorded base. No merge or release without explicit authorization.
30. Report passed gates, residual fallbacks and any missing evidence separately.
    Remove task-created remote scratch after last use; preserve shared caches
    and preexisting fixtures. Only complete #781 acceptance after its required
    evidence and docs are delivered, not after the planning stage.
