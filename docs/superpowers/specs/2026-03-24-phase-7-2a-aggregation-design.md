# Phase 7.2a: Aggregation Design

**Date**: 2026-03-24
**Phase**: 7.2a (split from 7.2 — arithmetic predicates deferred to 7.2b)
**Status**: Approved, pending implementation

---

## Overview

Add scalar aggregation to Minigraf's Datalog query engine: `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`, and the `:with` grouping clause. Aggregation is expressed in the `:find` clause as function calls over logic variables.

**Out of scope for 7.2a**: arithmetic filter predicates `[(> ?age 26)]` — deferred to Phase 7.2b.

---

## Syntax

```datalog
;; count — count all matching bindings
(query [:find (count ?e)
        :where [?e :person/name _]])

;; count-distinct — count unique values of ?e
(query [:find ?dept (count-distinct ?e)
        :where [?e :employee/dept ?dept]])

;; sum with :with to prevent over-deduplication
(query [:find ?dept (sum ?salary)
        :with ?e
        :where [?e :employee/dept ?dept]
               [?e :employee/salary ?salary]])

;; min / max
(query [:find (min ?ts)
        :where [?e :event/timestamp ?ts]])

;; sum-distinct — deduplicate values before summing
(query [:find (sum-distinct ?score)
        :where [?e :score ?score]])
```

Supported aggregate functions:

| Syntax | Description |
|---|---|
| `(count ?var)` | Count all bindings |
| `(count-distinct ?var)` | Count unique values |
| `(sum ?var)` | Sum (Integer or Float) |
| `(sum-distinct ?var)` | Sum unique values |
| `(min ?var)` | Minimum (Integer, Float, or String) |
| `(max ?var)` | Maximum (Integer, Float, or String) |

---

## Architecture

### 1. Types (`src/query/datalog/types.rs`)

Add two new types:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum AggFunc {
    Count,
    CountDistinct,
    Sum,
    SumDistinct,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FindSpec {
    Variable(String),
    Aggregate { func: AggFunc, var: String },
}

impl FindSpec {
    /// Column header for QueryResults.vars.
    /// Variable("?name") → "?name"
    /// Aggregate { func: CountDistinct, var: "?e" } → "(count-distinct ?e)"
    /// Uses the hyphenated function name (e.g., "count-distinct", "sum-distinct").
    pub fn display_name(&self) -> String { ... }

    /// The logic variable this spec depends on.
    /// Variable("?name") → "?name"
    /// Aggregate { var: "?e", .. } → "?e"
    pub fn var(&self) -> &str { ... }
}
```

`display_name()` format for aggregates: `format!("({} {})", func_str, var)` where `func_str` is the hyphenated lowercase name (`"count"`, `"count-distinct"`, `"sum"`, `"sum-distinct"`, `"min"`, `"max"`).

Changes to `DatalogQuery`:
- `find: Vec<String>` → `find: Vec<FindSpec>`
- Add `with_vars: Vec<String>` (empty = no `:with` clause)
- `DatalogQuery::new` signature changes to accept `Vec<FindSpec>`; `with_vars` defaults to `Vec::new()`
- Struct literal construction sites (currently ~5) need `with_vars: Vec::new()` added

`QueryResult::QueryResults.vars` stays `Vec<String>`, populated from `FindSpec::display_name()`:
```rust
// executor.rs — at both QueryResult::QueryResults construction sites (lines ~269, ~454)
vars: query.find.iter().map(|s| s.display_name()).collect(),
```

No changes to `WhereClause`, `Pattern`, `Rule`, `Transaction`, `AsOf`, or `ValidAt`.

### 2. Parser (`src/query/datalog/parser.rs`)

**`:find` clause** — extend to accept aggregate lists alongside plain variables:

```
find-element = variable | aggregate-list
aggregate-list = ( func-name variable )
func-name = "count" | "count-distinct" | "sum" | "sum-distinct" | "min" | "max"
```

Current `:find` arm (lines 474–483) changes from variable-only to:
```rust
match &query_vector[i] {
    EdnValue::Symbol(s) if s.starts_with('?') =>
        find_specs.push(FindSpec::Variable(s.clone())),
    EdnValue::List(elems) =>
        find_specs.push(parse_aggregate(elems)?),
    other => return Err(format!("Expected variable or aggregate in :find, got ...")),
}
```

`parse_aggregate(elems)` validates:
1. Exactly 2 elements
2. First element is a known function name symbol (`"count"`, `"count-distinct"`, etc.)
3. Second element is a `?`-prefixed symbol (logic variable)

The local `find_vars: Vec<String>` (line 407) becomes `find_specs: Vec<FindSpec>`.

**`:with` clause** — new `":with"` arm in the keyword match, before the catch-all `_` arm:
- After seeing `:with`, advance `i` and loop while the next element is a `?`-prefixed symbol, collecting each into `with_vars: Vec<String>` and advancing `i` each time. Stop at the next keyword or end of vector.
- Multiple `:with` variables are allowed: `:with ?e ?f` collects both `"?e"` and `"?f"`
- `:with` may appear in any order relative to `:find`, `:where`, `:as-of`, `:valid-at`
- Aggregate expression in `:with` → parse error

**Validation at parse time:**

The existing `outer_vars_from_clause` function already includes variables from `WhereClause::RuleInvocation` args (parser.rs lines 779–790). So the safety check correctly handles variables bound by rule invocations — queries like `(query [:find (count ?x) :where (reachable :a ?x)])` are valid.

Checks added:
- Each `FindSpec::Aggregate` var must be in `outer_bound` (via `outer_vars_from_clause`)
- Each `:with` var must be in `outer_bound`
- `:with` present but no `FindSpec::Aggregate` in `:find` → parse error

### 3. Executor (`src/query/datalog/executor.rs`)

Aggregation is a post-processing step applied **after** bindings are collected and not-filtered. It is extracted into a private function shared by both execution paths (`execute_query` and `execute_query_with_rules`):

```rust
fn apply_aggregation(
    bindings: Vec<HashMap<String, Value>>,
    find_specs: &[FindSpec],
    with_vars: &[String],
) -> Result<Vec<Vec<Value>>>
```

**The variable-extraction loop also changes (both paths).** Currently:
```rust
for var in &query.find {  // find: Vec<String>
    binding.get(var) ...
}
```
After the type change, even the non-aggregate path becomes:
```rust
for spec in &query.find {  // find: Vec<FindSpec>
    binding.get(spec.var()) ...
}
```
The conditional split is: if `query.find` contains any `FindSpec::Aggregate`, call `apply_aggregation`; otherwise use the updated variable-extraction loop.

**`apply_aggregation` algorithm:**

**Grouping implementation:** use a `Vec<(Vec<Value>, Vec<HashMap<String, Value>>)>` where each entry is `(grouping_key_values, bindings_in_group)`. Membership check uses `PartialEq` linear scan — `O(n)` in the number of groups but correct and dependency-free (consistent with the evaluator's existing `seen_facts` approach, and with the fact that `Value::Float(f64)` does not implement `Hash`). Float values in grouping variables are compared with `PartialEq`; NaN values will each form their own group (NaN != NaN), which is the expected IEEE 754 behavior.

1. Determine grouping key = values of all `FindSpec::Variable` vars in `:find` order, followed by `:with` vars — used for grouping only, not for output
2. Group bindings into the `Vec` structure above, preserving insertion order of first occurrence
3. For each group, build one output row:
   - For each `FindSpec` in `:find` order:
     - `Variable(v)` → take value of `v` from the grouping key
     - `Aggregate { func, var }` → collect values of `var` across the group's bindings, apply `func`
4. `:with` variables are **not** included in output rows — they affect grouping only and do not appear in `QueryResult::QueryResults.vars` or any result row
5. Return rows in group-insertion order

**Zero bindings — `count`/`count-distinct` special case:** if the `:where` clause produces zero bindings and `:find` contains only `count` or `count-distinct` aggregates with no grouping variables (`FindSpec::Variable` in `:find`), return a single row with `Value::Integer(0)` for each aggregate. This matches SQL `COUNT` behavior and is always meaningful — "how many matched" has a well-defined answer of 0.

**Zero bindings — all other aggregates:** if the `:where` clause produces zero bindings, return an empty result set. This applies to `sum`, `sum-distinct`, `min`, and `max`. An empty set preserves the distinction between "no facts matched" and "facts matched but summed/min/max to zero."

If `:find` mixes `count`/`count-distinct` with grouping variables (e.g., `[:find ?dept (count ?e)]`), zero total bindings also yields an empty result — groups only form when bindings exist, so no group for `?dept` is created.

**Null handling:** `Value::Null` is silently skipped by all aggregate functions (SQL behavior). Each function operates only on non-null values in the group. If all values in a group are null: `count`/`count-distinct` return `0`; `sum`/`sum-distinct` return `0`; `min`/`max` return empty for that group (the group itself disappears from the result).

**Aggregate function semantics:**

| Function | Accepted types | Null values | Result type | Zero-binding result | Type mismatch |
|---|---|---|---|---|---|
| `count` | any | skipped | `Value::Integer` | `[[0]]` if no grouping vars; else empty | n/a |
| `count-distinct` | any | skipped | `Value::Integer` | `[[0]]` if no grouping vars; else empty | n/a |
| `sum` | `Integer`, `Float` | skipped | see widening rule | empty result set | `Err` |
| `sum-distinct` | `Integer`, `Float` | skipped | see widening rule | empty result set | `Err` |
| `min` | `Integer`, `Float`, `String` | skipped | same as input type | empty result set | `Err` |
| `max` | `Integer`, `Float`, `String` | skipped | same as input type | empty result set | `Err` |

**`sum`/`sum-distinct` widening rule:** if any non-null value in the group is `Value::Float`, the result is `Value::Float`; otherwise `Value::Integer`.

**`min`/`max` with mixed `Integer` and `Float` in the same group:** type error (`Err`). Callers must ensure the aggregated variable is uniformly typed across non-null values in the group.

**`min`/`max` on empty groups** cannot occur — groups only exist when at least one binding is present.

Non-null type errors return `Err` immediately (fail fast).

### 4. Evaluator (`src/query/datalog/evaluator.rs`)

No changes required. The evaluator operates on `Rule`, `WhereClause`, and `FactStorage` — it never receives a `DatalogQuery` and therefore never accesses `DatalogQuery.find`. The optimizer (`optimizer.rs`) likewise reads only `:where` patterns — no changes needed there.

---

## Migration Scope

The `find: Vec<String>` → `find: Vec<FindSpec>` change affects:

| Location | Change |
|---|---|
| `src/query/datalog/types.rs` | `DatalogQuery` struct field; `DatalogQuery::new` signature and body; `DatalogQuery::from_patterns`; ~15 unit test call sites using `DatalogQuery::new` or struct literal (add `with_vars: Vec::new()` to struct literals) |
| `src/query/datalog/parser.rs` | `find_vars: Vec<String>` local → `find_specs: Vec<FindSpec>`; `:find` arm extended; new `:with` arm; `DatalogQuery::new(find_specs, ...)` call at line ~520. Parser test sites asserting `q.find == vec!["?name"]` etc. (lines ~995, ~1045, ~1148, ~1170, ~1189) must change to compare against `vec![FindSpec::Variable("?name".to_string())]` etc. |
| `src/query/datalog/executor.rs` | Two `vars: query.find` assignments (lines ~269, ~454) → `vars: query.find.iter().map(|s| s.display_name()).collect()`; two variable-extraction loops → iterate over `&query.find` as `FindSpec` calling `.var()`; struct literal tests at lines ~1031 and ~1086 need `with_vars: Vec::new()` added |
| `src/query/datalog/evaluator.rs` | No changes — evaluator does not access `DatalogQuery.find` |
| All other `DatalogQuery { ... }` struct literal sites | Add `with_vars: Vec::new()` field |

---

## Error Handling

| Error | Kind | Message |
|---|---|---|
| Unknown aggregate function | Parse error | `"Unknown aggregate function: 'foo'"` |
| Non-variable as aggregate argument | Parse error | `"Aggregate argument must be a variable"` |
| Aggregate var not bound in `:where` | Parse error | `"Aggregate variable ?x not bound in :where"` |
| `:with` var not bound in `:where` | Parse error | `"':with' variable ?x not bound in :where"` |
| `:with` without aggregate in `:find` | Parse error | `"':with' clause requires at least one aggregate in :find"` |
| `sum`/`sum-distinct` on non-numeric, non-null | Runtime error | `"sum: expected Integer, Float, or Null, got String"` |
| `min`/`max` on incompatible type | Runtime error | `"min: expected Integer, Float, String, or Null, got Boolean"` |
| `min`/`max` on mixed Integer/Float | Runtime error | `"min: cannot compare Integer and Float values"` |

---

## Testing (`tests/aggregation_test.rs`)

New test file. All assertions follow the no-`{:?}`-on-Result/Fact/Value convention (CLAUDE.md).

| Test | Covers |
|---|---|
| `count_all` | `(count ?e)` over all matching entities |
| `count_with_grouping` | `[:find ?dept (count ?e) :where ...]` groups correctly |
| `count_distinct_deduplicates` | `count-distinct` vs `count` differ when values repeat |
| `sum_integers` | integer sum across group |
| `sum_floats` | float sum |
| `sum_mixed_widens_to_float` | `Integer` + `Float` in same group → `Float` result |
| `sum_distinct_deduplicates` | duplicate values excluded before summing |
| `min_max_integers` | boundary values correct |
| `min_max_strings` | lexicographic ordering |
| `with_prevents_overcollapse` | `sum` without `:with` vs with `:with ?e` differ when entity duplicates exist |
| `count_empty_result` | zero bindings, no grouping vars → `[[0]]` |
| `count_empty_with_grouping_var` | zero bindings, grouping var present → empty result set |
| `count_distinct_empty_result` | zero bindings, no grouping vars → `[[0]]` |
| `sum_empty_result` | zero bindings → empty result set |
| `sum_skips_nulls` | null values excluded from sum; non-null values summed correctly |
| `count_skips_nulls` | null values excluded from count |
| `sum_type_error` | `Err` when summing non-numeric non-null values (e.g., String) |
| `min_type_error` | `Err` on Boolean type |
| `min_mixed_int_float_error` | `Err` when Integer and Float in same group |
| `aggregate_after_nonrecursive_rule` | aggregation after simple rule evaluation |
| `aggregate_after_recursive_rule` | aggregation after recursive rule (e.g., count reachable nodes) |
| `aggregate_with_negation` | aggregation after `not`/`not-join` filtering |
| `aggregate_with_as_of` | aggregation on time-travel snapshot |
| `parse_error_with_without_aggregate` | `:with` without aggregate → parse error |
| `parse_error_unknown_func` | unknown aggregate name → parse error |
| `parse_error_agg_var_unbound` | aggregate var not in `:where` → parse error |

---

## Files Changed

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `AggFunc`, `FindSpec`; change `DatalogQuery.find`; add `DatalogQuery.with_vars` |
| `src/query/datalog/parser.rs` | Parse aggregate lists in `:find`; parse `:with` clause; new validations |
| `src/query/datalog/executor.rs` | Add `apply_aggregation`; update variable-extraction loops; update `vars:` assignments |
| `src/query/datalog/evaluator.rs` | Type propagation only (3–5 call sites) |
| `tests/aggregation_test.rs` | New — ~26 tests |

No changes to: `matcher.rs`, `optimizer.rs`, `stratification.rs`, `rules.rs`, `storage/`, `graph/`, `db.rs`, `wal.rs`, public API.

---

## Non-goals

- Arithmetic filter predicates (`[(> ?age 26)]`) — Phase 7.2b
- Collection aggregates (returning sets/vectors) — not planned
- User-defined aggregate functions — not planned
- Window functions — not planned
