# Semantic permutations and natural-plan control limits (#842)

This report covers [#842](https://github.com/project-minigraf/minigraf/issues/842),
X07 in the [#776 close-out](../../bench-coverage-v3.md#remaining-exclusions-and-follow-ups).
It maps the four excluded permutation families (numeric, bind, cross-kind join
and forced planner strategy) to correctness tests at a pinned revision. It
states what the representative nightly controls establish and what they do
not, and it records the standing exclusion decision.

**Decision:** X07 stays **EXCLUDED**. Exhaustive semantic, bind, join and
forced-strategy permutations belong to `cargo test`, which CI runs on every PR
in both the plain and `test-hooks` builds (`rust.yml`). No nightly benchmark ID
is added or removed. No production code, public API, test, or nightly route
changes. Timing is unaffected, so nothing was measured.

This follows PHILOSOPHY.md's reliability ("do less, do it perfectly"),
performance-through-simplicity and documentation principles: correctness is
verified once, in tests, rather than duplicated as a Cartesian product of
timed cases.

## Pinned inventory

| Input | Value |
| --- | --- |
| Core, tests, benchmarks and workflows | `v3` at `5d85c36f9a4ec3056d59a14982ff5bd3490a2263` (2026-11-05, after #860) |
| Toolchain | rustc 1.94.0 (4a4ef493e 2026-03-02), cargo 1.94.0 |
| Plain inventory | `cargo test -- --list --format terse` → 2,062 entries ([list-plain.txt](evidence/list-plain.txt)) |
| `test-hooks` inventory | `cargo test --features test-hooks -- --list --format terse` → 2,099 entries: 2,079 Rust tests in `src/` and `tests/`, 19 doctests, 1 example test ([list-hooks.txt](evidence/list-hooks.txt)) |

The lists were taken with stderr included (`2>&1`), so every binary's `Running`
line precedes its tests. Local paths are replaced by `<repo>`. The enumeration
is a build plus `--list`, not a timing run, so it ran on the maintainer
workstation. The bench host was busy with other work, and nothing here depends
on idle hardware.

### Reproduce

```bash
git checkout 5d85c36f9a4ec3056d59a14982ff5bd3490a2263
cargo test -- --list --format terse > list-plain.txt 2>&1
cargo test --features test-hooks -- --list --format terse > list-hooks.txt 2>&1
python3 -I docs/benchmarks/842-semantic-permutations/map.py . list-hooks.txt list-plain.txt out \
  docs/benchmarks/842-semantic-permutations/targets.csv
```

From the committed lists, [map.py](map.py) (standard library only) writes
[tests.csv](evidence/tests.csv), [summary.json](evidence/summary.json) and
[targets.json](evidence/targets.json) byte for byte (checked). It exits 1 if
any curated target is missing.

## Method

There are two layers:

1. **Curated targets** ([targets.csv](targets.csv), 130 rows in 29 contract
   cells; 128 distinct tests, two of which serve two cells). Each test was chosen by reading it. `map.py` checks that each one is
   exactly one listed test defined in the named file, and records whether it
   exists only in the `test-hooks` build. All 130 resolve; 19 rows (17 distinct tests) are
   hooks-only.
2. **Lexical index** ([tests.csv](evidence/tests.csv), all 2,079 Rust tests).
   Each test is tagged by regexes over its body, plus the bodies of helper
   functions in the same file that it calls, transitively. String and comment
   contents are skipped when matching braces. In `src/`, only test code is
   read, so production functions do not lend their tags to the tests that call
   them. Family counts are: numeric 229, bind 151, cross-kind 419, forced 81.
   These are lexical evidence for review, not proof that a test asserts a
   contract. Parser tests, for example, carry cross-kind tags because they
   parse `not`/`or` syntax.

## Family → correctness targets

Counts are curated targets; the full list with test paths is in
[targets.json](evidence/targets.json).

### Numeric (C20)

| Cell | Tests | Representative targets |
| --- | ---: | --- |
| N1 ordinary avg | 8 | `ordinary_avg_test.rs` (float result, nonnumber skip with `:with` multiplicity, window parity, rules, cursor batches, pending/checkpointed/reopened); `aggregate::tests::ordinary_avg_uses_exact_unsaturated_totals_in_every_order` |
| N2 exact sums | 5 | `aggregate::tests::{exact_sum_is_the_same_in_every_order, exact_sum_rounds_once, integer_sum_saturates_once, …}` |
| N3 mixed sums, sum-distinct | 4 | `aggregate_value_handling_test.rs::sum_skips_nonnumbers_and_distinct_keeps_numeric_identity`, nil/nonnumeric groups, `:with`/join multiplicity, rules and temporal queries |
| N4 extrema | 4 | `extrema_accept_mixed_numbers_and_every_nonnil_class`; `min_max_mixed_values_do_not_depend_on_order`; certified ordered extrema |
| N5 tuple fallbacks | 5 | `aggregate_order_tests::generalized_{unproved_tuple_components_stay_materialized, constant_and_repeated_group_columns_keep_projection_and_sort_order, hidden_groups_stream_but_keep_visible_tie_sorting}` |
| N6 signed zero / NaN, including set elements | 9 | `exact_sum_signs_of_zero_and_specials`, `min_max_of_signed_zeros_and_nan`, `value_set_{holds_one_nan, keeps_negative_zero_whatever_the_order}`, `sets_with_a_zero_of_either_sign_are_equal`, `nan_is_one_element`, `typed_values_nan_and_sets_use_group_equality_and_canonical_output`, `unify_{negative_zero_equals_zero, int_never_equals_float}` |
| N7 numerics under every forced plan | 3 | `ordinary_avg_is_independent_of_plan_choices` (reorder × input order × subplans), `value_handling_is_independent_of_join_and_negation_plans`, `ordered_results_are_independent_of_order_modes_and_physical_plans` |

### Bind (C34; binding hosts in C36)

| Cell | Tests | Representative targets |
| --- | ---: | --- |
| B1 value/entity/expression slots | 8 | `prepared_statements_test.rs::prepare_and_execute_{entity,value,expr}_slot`, `…_combined`, `plan_reused_across_executions`; `prepared::tests::test_substitute_*` |
| B2 time slots | 10 | `prepare_and_execute_{as_of_counter, as_of_timestamp, valid_at, valid_at_any}`, `prepared_literal_as_of_and_valid_at_are_kept`; substitution unit tests |
| B3 count slots | 4 | `prepared::tests::{limit_slot_takes_a_non_negative_integer, offset_slot_is_collected_and_resolved}`; `limit_order_test.rs::{limit,offset}_slot_errors` |
| B4 Entity versus `Val(Ref)`, rule arguments | 1 | `entity_and_ref_bind_values_fit_each_others_slots` (includes `(friend-of ?x $f)`) |
| B5 keyword/ref equality through a slot | 2 | `join_rules_test.rs::expression_equality_equates_keyword_and_ref{,_after_checkpoint}`: both operand orders, `=`/`!=`, alternating keywords (#774 no-leak) |
| B6 set binds | 3 | `set_in_{operand,value}_slot_reused_across_executions`; `collect_set_test.rs::predicates_over_the_result_and_prepared_slots` |
| B7 bind failures | 3 | API-022/023/024 in every position (`missing_bind_value_is_api_022_in_every_position`, …) |
| B8 prepared over snapshots and committed data | 5 | `grouped_cursor_and_prepared_queries_survive_checkpoint_and_reopen`, `prepared_cursor_pins_like_a_query_cursor`, `prepared_queries_recheck_each_snapshot_and_rules_keep_original_strategy` |

`PreparedQuery::execute`/`query` substitute the bind values into the parsed
query and only then plan it as an ordinary query
(`src/query/datalog/prepared.rs`, `bind`). A forced-plan matrix over bind slots
would therefore repeat the literal-query matrices (F1–F4); the B1–B3
substitution unit tests check each slot's substituted value. Four-host set round trips are in the
binding repositories' own suites (C36), outside this core inventory.

### Cross-kind joins (C15, C16, C35)

| Cell | Tests | Representative targets |
| --- | ---: | --- |
| J1 entity probe | 4 | `engine::tests::{planner_chooses_the_join, entity_probe_reuses_repeated_entities, cached_entity_probe_normalizes_keyword_and_ref, entity_probe_cache_evicts_without_losing_rows}` |
| J2 value probe and hash fallback | 3 | `value_probe_reads_only_the_probed_keys` (asserts operator and keys read), `value_probe_matches_as_joins_do` (keyword/ref, ±0.0, long strings, `:as-of`, valid time; in memory, all and half checkpointed), `value_probe_falls_back_mid_batch` |
| J3 hash joins, variable attributes | 3 | `engine_matches_executor_{in_memory, across_checkpoints_and_reopen}` (frozen answers include `[?e :person/age ?a] [?f :person/age ?a] [?f :db/tx-count …]`), `planner_builds_a_keyless_join_on_the_smaller_product` |
| J4 pending/committed/mixed | 4 | `committed_and_in_memory_histories_answer_alike`, `collect_set_test.rs::pending_committed_and_mixed`, `committed_and_pending_facts_combine_in_every_lookup`, `matrix_file_backed_after_checkpoint` |
| J5 per-fact pseudo metadata source | 6 | `pseudo_source_test.rs` (nearest earlier source, variable attribute, `not`/`or`/rule scoping), `pseudo_attribute_pattern_never_replaces_a_value` |
| J6 keyword/ref join equality | 7 | `join_rules_test.rs::keyword_or_ref_*`, `imported_keyword_equals_ref_to_its_entity`, `branch_ref_meets_outer_keyword{,_after_checkpoint}` |
| J7 float join equality | 4 | `float_join_pairs_zero_with_negative_zero`, IEEE equality in `not` bodies and `or` branches, after checkpoint |
| J8 anti and union operators | 4 | `engine::tests::{negation_matches_executor, disjunction_matches_executor, not_bodies_plan_as_clause_lists, planner_chooses_the_body_form}` |
| J9 correlated rule bodies | 3 | `or_branch_expression_reads_outer_variable_in_rule_body`, `expression_reads_variable_bound_only_by_or_with_rules`, `expression_equality_in_rule_bodies_and_negation` |

### Forced planner strategies

The `test-hooks` switches are `minigraf::__reorder(bool)`,
`minigraf::__subplans(Choose | Hash | Correlated)` and
`minigraf::__input_order(bool)`.

| Cell | Tests | Representative targets |
| --- | ---: | --- |
| F1 clause reordering on/off | 5 | optimizer switch tests; `order_by_on_committed_and_pending_facts`, `bounded_rows_answer_bag_is_plan_independent`, `set_queries_are_plan_independent` |
| F2 subplans | 3 | `subplans_switch_is_per_thread_and_restored`, `repeated_join_variable_matches_single`, `distinct_is_independent_of_join_and_subplan_choices` |
| F3 input order on/off | 2 | `ordered_results_are_independent_of_order_modes_and_physical_plans`, `forced_fallback_restores_sort_and_gives_the_same_ordered_list` |
| F4 full matrix | 6 | `plan_independence_test.rs::{matrix_in_memory, matrix_file_backed_after_checkpoint, issue_596_example}`: 3 subplan modes × reorder on/off × execute/cursor against the frozen answer; `property_test.rs::query_matches_reference_{in_memory,file_backed}`; `model_based_test.rs::file_backed_db_matches_model` |
| F5 join kind | 2 | No switch exists. Join kinds vary through query shape and F1 reordering; `value_probe_runtime_fallback_preserves_order_only_when_it_streams_the_outer`, `filters_entity_probes_and_anti_joins_preserve_unchanged_order_columns` |

The 19 hooks-only rows (17 distinct tests) are in test files or functions
gated on `test-hooks`: the F1–F4 matrices, the numeric and join targets that
run inside them, and `limit_order_test.rs` (B3). They run in CI's `test-hooks`
job, not in a plain `cargo test`.

## Missing contract coverage

Every cell above has at least one correctness test at the pin. No missing
**answer** contract was found. The review found three limits, all in how far
benchmarks attribute their cost, not in what answers are checked:

1. **No join-kind switch (F5).** The hooks force body strategies, clause order
   and input order, but not entity probe versus value probe versus hash join
   for one fixed query. Equivalence across join kinds rests on shape variants
   (J1–J3), F1 reordering and the runtime value-probe fallback (J2). This is
   sufficient for the plan-independence contract, which covers every plan the
   planner can produce. Adding a join-kind switch would be a production test
   hook needing its own review; it is not proposed here.
2. **The operator preflight cannot tell the two probes apart.** Both
   `query/operator_paths/entity_*` and `value_*` assert `IndexNestedLoopJoin`
   and the absence of `HashJoin`. A value probe (#721) reports the same
   operator name as an entity probe (`value_probe_reads_only_the_probed_keys`
   asserts exactly that). The entity/value labels follow from query shape and
   the planner rules in CLAUDE.md, not from a counter.
3. **Most v3 controls check answers, not strategies.** Only the six join
   controls inspect operator counters.
   - `anti_committed` and `union_committed` check a row count; they do not
     check which body form (hashed or correlated) ran.
   - `aggregation/v3_controls/*` and `expr/keyword_equality/*` check full
     answers against an oracle.
   - The `tuple_*` controls cite the engine tests whose strategy checks they
     mirror, but do not repeat those checks.

   If a planner change moved one of these controls to another strategy, its
   timing would silently represent a different path. The answer would still be
   checked, both by the control and by J8/N5.

## What the representative nightly IDs establish

| Nightly IDs | Preflight before timing | Establishes | Does not establish |
| --- | --- | --- | --- |
| `query/operator_paths/{entity,value,hash}_{pending,committed}/10k` (6) | Operator counters (probe vs `HashJoin`); every row checked: entity, value and right-hand source tx-count | Latency of three natural join shapes over paired 20k-fact fixtures, 10k rows each, in memory and committed | Isolated hash-join or probe speedups; entity vs value probe by counter; cross-kind values (all integers); forced plans |
| `query/operator_paths/{anti,union}_committed/10k` (2) | Row count | One query with a correlated `not` plus a hashed `not-join`; `or` branches with nested outer-variable expressions | Forced body strategies; which strategy ran; pending data |
| `aggregation/v3_controls/*` (10) | Full answer vs oracle | Four hash aggregates over mixed literals at 10k; six 100k skew/tuple pages (committed, one pending) | Every numeric kind combination; signed-zero/NaN; sets; ordered-strategy selection |
| `prepared/*` (12) | Answer checks; each path has an unprepared control | Preparation, execute and cursor cost; count, time and entity slots | Every slot-kind combination; set binds; Entity versus `Val(Ref)`; bind errors (X08) |
| `expr/keyword_equality/*` (6) | Full answer vs oracle; prepared alternates targets 5, 6, 5 | Literal, dynamic, nested and prepared keyword/ref expression cost on one 10k pending fixture | An isolated cache-off switch; committed data; keyword/ref joins (J6) |

## Confounders next to comparisons

These belong next to any comparison that uses the controls (also stated in
C15 and C35 of the close-out):

- **Hash controls scan more.** `hash_*` reads `[?f ?a ?v]`, every attribute's
  facts (20k, twice the 10k `:op/right` facts the probes join), then filters
  `(= ?a :op/right)`. A `hash_*` versus `entity_*`/`value_*` ratio mixes join
  strategy with scan width and the expression filter. **Do not report it as a
  hash-join or probe speedup.**
- **Dynamic keyword controls do more work.** `dynamic_*` reads the keyword from
  a bound variable: per row it does a binding lookup and clones a value, and it
  does not use the literal cache. A `literal_*` versus `dynamic_*` ratio
  therefore bounds the cache's benefit plus that work. **It is not an isolated
  cache measurement.** `prepared_keyword_eq` also includes substitution and
  per-execution cache construction.
- **Pending versus committed** pairs differ in storage path (in-memory pending
  runs versus file pages and DICT lookups), not only in data placement.

## Standing exclusion and new representatives

- X07 remains a deliberate standing exclusion. Correctness for these families
  is owned by the 128 curated tests above and by the wider suite, all run on
  every PR.
- No new nightly representative is proposed. A future one must state:
  - a distinct cost hypothesis that no existing ID covers (not a semantic
    permutation of one);
  - the operator or strategy it is meant to time, checked before timing
    (counter preflight, as in the join controls);
  - a bounded nightly budget. For comparison, every current X07 control uses
    Criterion sample size 10, 1 s warm-up and 3 s measurement per ID.
- Optional, not required by any gap above: extending the anti/union and tuple
  control preflights to assert their strategies would make limit 3 visible.
  It would change benchmark code, so it would need its own PR.

## Files

| File | Purpose |
| --- | --- |
| [map.py](map.py) | Reproduces the index and verifies targets (standard library) |
| [targets.csv](targets.csv) | 130 curated targets in 29 cells |
| [evidence/list-plain.txt](evidence/list-plain.txt), [evidence/list-hooks.txt](evidence/list-hooks.txt) | Pinned inventories |
| [evidence/tests.csv](evidence/tests.csv), [evidence/summary.json](evidence/summary.json) | Lexical index and family counts |
| [evidence/targets.json](evidence/targets.json) | Each target's listed path, tags and hooks-only flag |
