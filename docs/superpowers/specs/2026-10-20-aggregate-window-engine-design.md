# Aggregation and window functions on the streaming engine — Design

**Issue:** #547 (part 5 of 9 of #473)
**Milestone:** v3.0.0, branch `v3` (no format change; breaking changes to query semantics)
**Builds on:** the streaming engine design (`2026-10-11-streaming-engine-design.md` §6.2,
§6.3, §9), plan-independent answers (`2026-10-15-plan-independent-answers-design.md`),
the #546 design (`2026-10-16-negation-disjunction-engine-design.md`)

## 1. Scope

The engine runs queries whose `:find` holds plain variables. This work moves aggregates,
window functions and `:with` to it:

- `Aggregate`, ungrouped in constant memory and hash-grouped (§5);
- `Window`, holding its input and sorting it (§6);
- one set of rules for aggregates, window ordering and ties, on the executor too, so that
  both engines agree and no answer depends on the plan (§3, §4).

Queries holding a rule invocation at any depth stay on the executor (#548). They take the
new rules too, since the executor's rule path (`execute_query_with_rules`) and its plain
path share `apply_post_processing`.

Settled with the maintainer on 2026-10-20:

1. **Window ties are SQL peers** (§4.2): rows with equal `:order-by` keys in one partition
   are peers; a running aggregate gives every peer the value at the last peer; `rank` is
   equal for peers; `row-number` breaks ties by the other variables `:find` names, then by
   input position. One total order of values (§3.1) is used for sorting, peers, `rank`
   and window `min`/`max`.
2. **Sums are exact** (§3.2): integers summed in `i128` and saturated to `i64` once, floats
   summed exactly and rounded once. One accumulator module serves both engines.
3. **Index order is not used in #547.** The engine design's sort skip and sort-grouped
   `Aggregate` (§6.2, §6.3 there) are deferred to a follow-up with `:order-by`/`:limit`
   (#306): nothing reads an order before that syntax exists, and an index's byte order
   matches value order only within one value type.
4. **A window in an aggregate query may read only grouping variables** (§4.4); anything
   else fails to parse with a new code, `PRS-082`.

## 2. Terms

- **Aggregate query.** A query whose `:find` holds at least one aggregate
  (`FindSpec::Aggregate`).
- **Grouping variables.** In an aggregate query, the plain `:find` variables, in `:find`
  order, followed by the `:with` variables when `:find` holds no window
  (`compute_aggregation`, `executor.rs:1640–1661`). Unchanged.
- **Group.** The input rows whose grouping variables are equal under `Value`'s `Ord`
  (today's `BTreeMap<Vec<Value>, _>` key): a keyword differs from a ref to the entity it
  names, `-0.0` equals `0.0`, a NaN equals every NaN. Unchanged.
- **Window input.** The rows a window reads: the query's rows in a window-only query,
  the groups' rows in an aggregate query.
- **Named variables.** In a query without aggregates, every variable `:find` names:
  plain variables and each window's argument, `:partition-by` and `:order-by`
  variables, in `:find` order, each once at its first place. In an aggregate query, the
  plain `:find` variables, in `:find` order: every group row has a distinct tuple of
  them, and every window reads only them (§4.4).

## 3. Values

### 3.1 The total order

`value_order(a, b)` orders every pair of values. It replaces `value_cmp`
(`functions.rs:367`) wherever this design orders values:

1. Classes, in this order: numbers (`Integer`, `Float`), strings, booleans, refs,
   keywords, `nil`.
2. Two numbers that are not NaN compare by their exact numeric values (an `i64` and an
   `f64` compared without rounding either). When equal, an `Integer` precedes a `Float`,
   and two floats compare by `f64::total_cmp` (so `-0.0` precedes `0.0`).
3. A NaN follows every other number; NaNs compare by `f64::total_cmp`.
4. Strings by bytes, booleans `false` first, refs by UUID bytes, keywords by their text.

Two values are **order-equal** when `value_order` returns `Equal`, which holds only for
identical values (for floats, identical bit patterns). Peers (§4.2) and `rank` ties use
order equality, so sorting and tie detection agree: `-0.0` and `0.0` are not peers, nor
are `1` and `1.0`.

Order equality is used only for window order and peers. Groups, partitions and the sets
of `count-distinct` / `sum-distinct` keep `Value`'s `Ord` equality (§2), under which
`-0.0` equals `0.0` and every NaN equals every NaN.

Today `value_cmp` returns `Equal` for any two keywords, refs or booleans, and for any
two values of different classes other than numbers, while `rank` detects ties with
`Value ==`, so the ranks follow the order the scan meets the rows. On `v3`

```
(transact [[:a :d :x] [:b :d :x] [:c :d :y] [:d :d :y]])
[:find ?e (rank :over (:order-by ?d)) :where [?e :d ?d]]
```

ranks the entities `1 2 3 4`, because the scan meets them in entity-id order, which
interleaves `:x` and `:y`; a scan meeting them as `:x :x :y :y` would rank `1 1 3 3`.
They now rank `1 1 3 3` on every plan.

### 3.2 Accumulators

One module, `query/datalog/aggregate.rs`, holds an accumulator per built-in function.
`compute_aggregation`, `apply_window_functions` and the engine's `Aggregate` and `Window`
use it; `apply_builtin_aggregate` and the built-in `WindowOps` of `FunctionRegistry` are
replaced by it. Every accumulator takes one value at a time, so `Aggregate` runs in one
pass, and gives the same result for every order of its input:

- **`count`**: the number of non-`nil` values, saturating at `i64::MAX`.
- **`sum`**: `nil` skipped. A value that is not a number makes the result `INT-041`
  (§3.3). No values: `Integer(0)`. When every value is an `Integer`, they are added in
  an `i128` (which cannot overflow for fewer than 2^64 values) and the total is
  saturated to `i64` once. Otherwise the result is a `Float`:
  - any NaN, or both `+inf` and `-inf`: NaN;
  - otherwise any infinity: that infinity;
  - otherwise the exact sum of every value, integers included, as a real number,
    rounded once to the nearest `f64` (ties to even); a magnitude beyond `f64::MAX`
    after rounding gives the infinity of its sign. A zero result is `-0.0` only when
    every value is `-0.0` (IEEE addition's rule), else `0.0`.

  The plan chooses the algorithm (Shewchuk's partials as Python's `math.fsum`, or a
  fixed-point accumulator wide enough for every `f64`); an `i64` enters it exactly, as
  `hi = x as f64` and `lo = (x as i128 - hi as i128) as f64`, both exact.
- **`avg`**: the exact sum (as `sum`, rounded once) divided by the count of numbers;
  values that are not numbers are skipped; no numbers: `nil`. Unchanged apart from the
  exact sum.
- **`min` / `max`** (aggregates): `nil` skipped; no values: the group is dropped
  (`INT-042`, caught, as today). The accumulator records the set of variant kinds it
  met. Two or more kinds: `INT-043` (as today, where `Integer` and `Float` are two
  kinds). One kind other than `Integer`, `Float` or `String`: `INT-044`. Otherwise the
  least or greatest by `value_order`, so for floats `min` prefers `-0.0` to `0.0`, `max`
  prefers `0.0`, `max` is NaN when any value is, and `min` is NaN only when every value
  is. Today the first of two values `partial_cmp` cannot order, or finds equal, wins.
- **`count-distinct` / `sum-distinct`**: a set of the non-`nil` values under `Value`'s
  `Ord`, each float stored canonically (`-0.0` as `0.0`, every NaN as one NaN), then
  counted or summed as `sum`. The set is the only accumulator state that grows with its
  input.
- **Window `min` / `max`**: `nil` skipped; the least or greatest by `value_order`, over
  values of any kind (today: `value_lt`, keeping the first of two values it cannot
  order). No values: `nil`.
- **Registered aggregates** (`register_aggregate`) keep their `init` / `step` /
  `finalise` contract and see their values in input order, which depends on the plan. A
  registered aggregate whose result depends on the order of its values may give
  different answers to equivalent queries; the documentation of `register_aggregate`
  says so.

### 3.3 Errors

No accumulator fails while taking values. Each records what it needs to decide its
error at the end: `sum` whether it met a value that is not a number (and the first such
value's type, for the message), `min`/`max` the set of kinds (§3.2). When a group's
accumulators finish, the first of them in `:find` order whose result is an error fails
the query, except `INT-042`, which drops the group. Groups finish in output order
(§5.3), so the error code a query fails with does not depend on the plan; the value type
an `INT-041` message names may. Today the executor also finishes groups in that order
(`compute_aggregation`'s `BTreeMap`), and fails on the first error.

### 3.4 Answers that change

All are breaking changes for v3.0.0, listed in `CHANGELOG.md`:

1. `sum` and `avg` over floats: exact, where today the rounding follows input order. Two
   plans of one query could differ in the last bits; now none do.
2. `sum` over integers that overflow: saturates once at the end, where today each
   addition saturated, so `i64::MAX + 1 + -1` gave `i64::MAX - 1` or `i64::MAX` by order.
3. Window `sum` over integers: summed exactly, where today they were added as `f64` and
   truncated, losing precision above 2^53.
4. `min`/`max` of floats with `-0.0` and `0.0`, or with NaN: by `value_order`, where
   today the first in input order won.
5. Window ordering, ties and `rank` (§4).
6. A window in an aggregate query that reads a non-grouping variable fails with
   `PRS-082` (§4.4).

## 4. Windows

### 4.1 Order of a partition

Each window partitions its input by its `:partition-by` variable (one partition when
absent; partitions by `Value`'s `Ord`, as today's `HashMap` key by `Value ==` except that
a NaN key is one partition, not one per row). Within a partition the rows are sorted by:

1. the `:order-by` value, by `value_order`, reversed under `:desc`;
2. then each named variable (§2), in `:find` order, by `value_order` ascending;
3. then input position.

Rows equal in 1 and 2 cannot be told apart by any column the query returns, so step 3
changes no answer; it makes every window of a query order such rows alike.

### 4.2 Peers and values

Rows of a partition order-equal in their `:order-by` value are **peers**. Over the
sorted partition:

- **`row-number`**: the position, from 1.
- **`rank`**: one more than the number of rows before the first peer.
- **`sum`, `count`, `min`, `max`, `avg`, registered window functions**: the running
  aggregate over every row up to and including the last peer of the row, so every peer
  gets the same value. A registered window function's `finalise` is called once per
  peer group, with the number of rows stepped so far, and steps its values in the
  order of §4.1.

Today a running aggregate gives each tied row the value at its own position, in input
order, so on `v3`, with `?s` of 1.5, 5, 10, 10, `(sum ?s :over (:order-by ?s))` gives
1.5, 6.5, 16.5, 26.5 with the 16.5 going to either tied entity; it now gives
1.5, 6.5, 26.5, 26.5.

### 4.3 Output

A window adds its column to every row of its input and returns the rows in input order,
as today. Several windows read the same input rows.

### 4.4 Windows in aggregate queries

The window runs over the groups, after aggregation, as today. Its argument,
`:partition-by` and `:order-by` must be plain `:find` variables; `:with` variables do
not count, since they do not group when `:find` holds a window. The parser fails with
`PRS-082` otherwise. This check is new: today the parser only requires these variables
to be bound in `:where` (`parser.rs:939–950`), and one that does not group reads `nil` on
every group row (`apply_window_functions` looks it up in the group's binding, which
holds only the grouping variables and the aggregate values).

## 5. `Aggregate` on the engine

### 5.1 Which queries

Today `plan::plan` accepts only a `:find` of plain variables (`plan.rs:316`). After PR 2
it accepts aggregates and a `:with`, and after PR 3 windows. It checks every function
name as `check_find_functions` does, before reading anything.

### 5.2 Ungrouped

With no grouping variables, `Aggregate` holds one accumulator per aggregate. It pulls
every input batch, steps each accumulator with the row's cell of its argument, and then
returns one row. Cells become values only where an accumulator needs one: `count` reads
only whether the cell is `nil`; `count-distinct` keys its set on the cell, compared as
the values of §2 compare (a long string by its content, a keyword never equal to a ref).
Memory is one batch and the accumulators. With no input rows it returns the zero row of
a `count`-only query, and otherwise no rows (`executor.rs:1625–1639`).

### 5.3 Grouped

With grouping variables, `Aggregate` holds a hash table from each group's key (its
grouping cells, compared as in §2) to its accumulators. When the input ends, it converts
each key to values, sorts the groups by those values under `Value`'s `Ord` (today's
`BTreeMap` order), finishes each group's accumulators (§3.3) and returns the rows in
batches. Memory is proportional to the number of groups, plus the distinct sets. #549
counts it.

### 5.4 Plan

`Aggregate` sits above the `:where` operators. With windows, the query's root is
`Window` (§6) over `Aggregate` or over the `:where` operators. The `:find` slots the
query returns are the grouping variables', aggregates' and windows' output slots.

## 6. `Window` on the engine

`Window` holds every input row's cells of the named variables, and the input position,
for all windows of the query. When its input ends it computes each window as §4 states,
over values, and returns the rows in input order, in batches. Memory is proportional to
the input; #549 counts it. The sort is part of `Window`: no other operator sorts until
`:order-by` (#306) adds `Sort`.

## 7. Testing

- The oracle (engine design §9) runs every aggregate and window query through both
  engines. Both use §3's accumulators, so the oracle checks grouping, peers and
  routing, not arithmetic.
- `aggregate.rs` unit tests: each accumulator against every input order of small
  inputs (exactness, saturation, `-0.0`/NaN, errors); `value_order` as a total order.
- `tests/aggregation_test.rs`, `tests/window_functions_test.rs`: §3.4's cases, each
  run with `optimizer::reorder()` on and off (`minigraf::__reorder`), asserting equal
  bags.
- Engine unit tests: `Aggregate` holds only its accumulators for an ungrouped query
  (counters), empty input, the zero row, an error in a later group.
- Acceptance (#547): `count` over 10M facts runs in constant memory, in the #394
  envelope at 100K, 1M and 10M, with peak RSS recorded; `count_grouped` recorded.

## 8. Delivery

One spec, three PRs to `v3`, none closing #473:

1. **Rules on the executor**: `value_order`, `aggregate.rs`, `compute_aggregation` and
   `apply_window_functions` on them, peers, `PRS-082`, the §3.4 tests, `CHANGELOG.md`,
   `docs/ERROR_REFERENCE.md`; docs-site PR for the window semantics.
2. **`Aggregate` on the engine**: plan routing of aggregates and `:with`, ungrouped and
   grouped `Aggregate`, envelope run.
3. **`Window` on the engine**: plan routing of windows, `Window`; #547 closed by hand.

Follow-up issue (filed with PR 1): sort skip on index order and sort-grouped
`Aggregate`, with #306.

## 9. Deviations from the engine design

- §6.2 there: `Sort` as an operator and top-k under `Limit` wait for #306 (decision 3);
  `Window` sorts within itself. Sort-grouped `Aggregate` waits likewise.
- §6.2 there: `Window` holds its whole input, not one partition: with no input order
  (decision 3), any partition's rows can arrive last.
- §6.2 there says `Aggregate` and `Window` reproduce today's executor. They follow the
  accumulator, order and peer rules of §3 and §4 instead, which change answers (§3.4).
