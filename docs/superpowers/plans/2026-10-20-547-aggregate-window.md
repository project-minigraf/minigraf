# Aggregation and window functions on the streaming engine (#547) — Plan

Spec: `docs/superpowers/specs/2026-10-20-aggregate-window-engine-design.md`. PRs target
`v3`. None uses a closing keyword for #473; #547 closes by hand after PR 3.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–8: the rules on the executor (`value_order`, `aggregate.rs`, peers, `PRS-082`) | `feat/547-rules` |
| 2 | 9–12: `Aggregate` on the engine | `feat/547-aggregate` |
| 3 | 13–16: `Window` on the engine | `feat/547-window` |

Each PR leaves `v3` with no unused code, updates the test counts and `CHANGELOG.md` for
what it ships, and has a docs-site PR (`../minigraf-docs`) when it changes documented
behaviour (PR 1 does; PRs 2 and 3 change no answer).

## What PR 1 replaces

| Today (`v3`) | Used by | After PR 1 |
|---|---|---|
| `functions::AggState`, `WindowOps`, `AggImpl::Builtin(WindowOps)` | registry built-ins, `apply_window_functions` | `AggImpl::Builtin(Builtin)`, `Builtin` an enum of the seven built-ins; the arithmetic lives in `aggregate.rs` |
| `functions::apply_builtin_aggregate` | `compute_aggregation` | `aggregate::Agg` |
| `functions::value_lt` | built-in window `min`/`max` | `aggregate::value_order` |
| `functions::value_cmp` | window sort (`executor.rs:1799`) | `aggregate::value_order` |
| `FunctionRegistry::is_window_compatible`, `is_known` (`#[allow(dead_code)]`) | tests only | removed with their tests |

The parser already rejects `count-distinct`/`sum-distinct` in a window (`PRS-031`) and
`avg`/`rank`/`row-number` as aggregates (`PRS-027`). Registered aggregates (`AggImpl::Udf`)
are unchanged.

## Design decisions not fixed by the spec

1. **Window `sum` and `avg` skip values that are not numbers**, as the built-in
   `WindowOps` do today; only the aggregate `sum` fails with `INT-041` (spec §3.2 names
   the aggregate). Window `sum` is an `Integer` while every number so far was an
   integer, else a `Float`, as today, with the exact sum of spec §3.2. No numbers so
   far: `Integer(0)` for `sum`, `nil` for `avg`.
2. **Exact float sums use a fixed-point accumulator**, not Shewchuk partials: a signed
   integer of 36 `u64` limbs counting units of 2^-1074, which holds the sum of 2^64
   values of any finite `f64` without overflow. It is boxed and allocated on the first
   finite float, so integer-only and empty accumulators stay small. Integers are summed
   in an `i128` beside it and added in at the end. NaN, `+inf`, `-inf` and "every float
   was `-0.0`" are flags. Rounding to `f64` is round-half-even on the limbs. Reason: no
   intermediate overflow to handle, and the same code removes a value exactly (#183).
3. **`value_order` is a free function in `aggregate.rs`**, `pub(crate)`, used by the
   window sort, peers, `rank`, window and aggregate `min`/`max`. An `i64` and an `f64`
   compare exactly: a float outside `[-2^63, 2^63)` is beyond every integer; otherwise
   compare the integer with `f.floor()` as `i128`, then the fraction.
4. **`aggregate::Agg`** (one per aggregate per group): `Agg::new(func, registry)`,
   `step(&Value)`, `finish(self) -> Result<Option<Value>>`, where `Ok(None)` is
   spec §3.2's dropped group (`INT-042`, today's string match on "no non-null values in
   group" goes). Variants: `Count(i64)`, `Sum(Sum, first non-number's type)`,
   `Min/Max(Extreme)`, `CountDistinct/SumDistinct(BTreeSet<Value>)` with canonical
   floats, `Udf`. No `Avg`: `avg` is a window function only (`PRS-027`), so
   `Agg::new("avg")` is `INT-029`. `Extreme` records a bitset of kinds and the best
   value.
5. **`aggregate::Running`** (one per window per partition): `Running::new(func,
   registry)`, `step(&Value)`, `value(&self) -> Value` (a registered window's
   `finalise(acc, rows_stepped)`). `apply_window_functions` steps a whole peer group,
   then reads `value()` once for all its rows.
6. **Partitions by `Value`'s `Ord`**: a `BTreeMap<Value, Vec<usize>>` (a missing
   partition variable reads `nil`), replacing today's `HashMap<Option<Value>, _>`, so
   NaN keys share one partition (spec §4.1).
7. **Named variables** (spec §2) are computed once per query by
   `aggregate::named_vars(find)`; the sort reads them from each binding (`nil` when
   absent, which only happens for a missing grouping value).
8. **`PRS-082`** text: `"{} in {} must be a plain :find variable when the query has aggregates"`
   (variable, the window's display name). Checked in `parse_query` after `PRS-080`.

## PR 1 — the rules on the executor

1. **`aggregate.rs` order and sums.** `value_order`; the fixed-point `Exact`
   accumulator (add `f64`, add `i64`, round). Unit tests: `value_order` is a total
   order consistent with itself on a grid of values (every class, `±0.0`, NaNs with
   both signs, `i64::MIN/MAX` against nearby floats, `2^53 + 1` against `2^53` as float);
   `Exact` against hand-computed results (cancellation `1e308 + 1e308 - 1e308`,
   `0.1` ten times, subnormals, `-0.0` only, overflow to `inf`, `i64::MAX` plus `0.5`)
   and against every permutation of 6-element inputs.
2. **`Agg` and `Running`** (decisions 1, 4, 5). Unit tests per function: empty input,
   `nil` only, saturation, errors decided at `finish` whatever the order (`min` over
   `[kw, kw, 1]` and `[1, kw, kw]` both `INT-043`), canonical distinct floats, a
   registered aggregate's `n`.
3. **Registry.** `AggImpl::Builtin(Builtin)`; drop `AggState`, `WindowOps`,
   `apply_builtin_aggregate`, `value_lt`, `value_cmp`, `is_window_compatible`,
   `is_known` and their tests (`functions.rs:520–800`), moving each still-meaningful
   case to step 1 or 2. Update the `eval_binop` comment that cites `value_lt`/`value_cmp`.
4. **`compute_aggregation`** on `Agg`: groups stay a `BTreeMap<Vec<Value>, _>`, now of
   accumulators instead of bindings (one pass); finish groups in key order with spec
   §3.3's error rule. The zero row for `count`-only queries is unchanged.
5. **`apply_window_functions`** on decisions 5–7: partition, sort by (order key with
   direction, named variables, input index), walk peer groups. Output stays in input
   order.
6. **`PRS-082`** (decision 8): `ErrorCode::Prs082` in the four places `Prs081`
   appears in `error.rs`, `docs/ERROR_REFERENCE.md` (table row and section with cause,
   resolution, example), parser check, parser test.
7. **Integration tests** (`tests/aggregation_test.rs`, `tests/window_functions_test.rs`):
   every case of spec §3.4 and the §3.1 / §4.2 examples, each run with
   `minigraf::__reorder` on and off under `test-oracle` (as `plan_independence_test.rs`
   does), plus ungated copies of the plain assertions so the coverage job (which runs
   without `test-oracle`) counts them. Fix existing tests that asserted input-order
   ties; list each in the PR body.
8. **Docs and counts.** `CHANGELOG.md` (v3.0.0 breaking: spec §3.4 items 1–6),
   `register_aggregate` doc comment (order dependence, spec §3.2), CLAUDE.md module list
   (`aggregate.rs`) and test count, `docs/TEST_COVERAGE.md`, `ROADMAP.md` if it lists
   #547 status. Docs-site PR: `content/datalog-reference.md` window section (peers,
   value order, frame sentence), aggregate section (exact sums, `min`/`max` of floats),
   `@since v3.0.0` / `@until v3.0.0` around the changed text. File the follow-up issue
   (index-order sort skip and sort-grouped `Aggregate`, with #306) and link it from
   the PR.

## PR 2 — `Aggregate` on the engine

9. **Routing.** `plan::plan` accepts `FindSpec::Aggregate` and `:with` (windows still
   return `None`). It calls `check_find_functions` first (made `pub(crate)`). `Plan`
   gets `find: Vec<Out>` describing each output column (a slot, or an aggregate
   result), built by a new `Step::Aggregate { groups: Vec<usize>, aggs: Vec<(String,
   usize)> }` at the end of `steps`.
10. **`engine/aggregate.rs`.** Ungrouped: one `Agg` per aggregate; `count` steps only on
    whether the cell is `Val(Null)`; `count-distinct` keys on the cell (`JoinKey::of`
    with keyword-to-ref off, long strings by content, floats canonical) and converts
    to `Value` only at finish; other functions convert the argument cell to a `Value`
    (`project::value`). Grouped: `HashMap<Vec<GroupKey>, Vec<Agg>>` where `GroupKey`
    compares as `Value`'s `Ord` (as `JoinKey` but NaN equal to NaN, keyword never a
    ref); at the end, convert keys, sort by `Value`'s `Ord`, finish (spec §3.3), emit
    batches whose cells are the group cells and the results as `Cell::Val`. A result
    that is a `Ref` or `Keyword` (from `min`/`max` this cannot happen; from a
    registered aggregate it can) needs a cell: add `Cell::Value(Box<Value>)`, projected
    as is, if a registered aggregate can return one; otherwise restrict to scalars and
    check in `project`.
11. **Tests.** Engine unit tests: counters show `Aggregate` held no rows for an
    ungrouped `count`; empty input; zero row; error in a later group; `:with`. Routing
    test (`RUNS`) that aggregate queries run on the engine. The oracle covers the
    integration suites.
12. **Measurement.** Envelope `count` and `count_grouped` at 100K, 1M, 10M on the bench
    machine (idle check first), peak RSS for `count` flat across scales; numbers in the
    PR body and `docs/BENCHMARKS.md`.

## PR 3 — `Window` on the engine

13. **Routing.** `plan::plan` accepts windows. `Step::Window(Vec<WindowSpec>, slots)`
    after `Step::Aggregate` (mixed) or after the `:where` steps.
14. **`engine/window.rs`.** Holds every input row's cells of the named variables and
    the window arguments, converted to `Value`s on arrival (the sort compares values),
    plus input position; at end of input computes each window with the PR 1 code
    (factor `apply_window_functions`' per-partition core into `aggregate.rs` so both
    engines call it over `&[Vec<Value>]`), emits rows in input order.
15. **Tests.** Routing; mixed aggregate + window; oracle over `window_functions_test.rs`.
16. **Close.** CLAUDE.md engine paragraph, counts; close #547 by hand; docs-site PR only
    if the architecture page lists engine operators.
