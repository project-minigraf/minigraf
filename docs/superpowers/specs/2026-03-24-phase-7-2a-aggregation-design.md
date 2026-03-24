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
    /// Column header for QueryResults.vars
    pub fn display_name(&self) -> String;
    /// The logic variable this spec references
    pub fn var(&self) -> &str;
}
```

Changes to `DatalogQuery`:
- `find: Vec<String>` → `find: Vec<FindSpec>`
- Add `with_vars: Vec<String>` (empty = no `:with` clause)

`QueryResult::QueryResults.vars` stays `Vec<String>`, populated from `FindSpec::display_name()` (e.g., `"?dept"`, `"(count ?e)"`).

No changes to `WhereClause`, `Pattern`, `Rule`, `Transaction`, `AsOf`, or `ValidAt`.

### 2. Parser (`src/query/datalog/parser.rs`)

**`:find` clause** — extend to accept aggregate lists alongside plain variables:

```
find-element = variable | aggregate-list
aggregate-list = ( func-name variable )
func-name = "count" | "count-distinct" | "sum" | "sum-distinct" | "min" | "max"
```

When the parser sees an `EdnValue::List` in the `:find` position, it delegates to `parse_aggregate(elems)` which validates:
1. Exactly 2 elements
2. First element is a known function name symbol
3. Second element is a logic variable (`?`-prefixed symbol)

**`:with` clause** — new keyword arm in the query vector parser:
- Collects plain variables only (aggregate in `:with` → parse error)

**Validation at parse time:**
- Aggregate vars must appear in `:where` (same safety check as `not`/`not-join`)
- `:with` vars must appear in `:where`
- `:with` without any aggregate in `:find` → parse error (meaningless combination)

### 3. Executor (`src/query/datalog/executor.rs`)

Aggregation is a post-processing step applied **after** bindings are collected and not-filtered. It is extracted into a private function shared by both execution paths:

```rust
fn apply_aggregation(
    bindings: Vec<HashMap<String, Value>>,
    find_specs: &[FindSpec],
    with_vars: &[String],
) -> Result<Vec<Vec<Value>>>
```

**Algorithm:**

1. Determine grouping key = values of all `FindSpec::Variable` vars + `:with` vars, in order
2. Group bindings by grouping key (preserve insertion order of first occurrence)
3. For each group, for each `FindSpec` in order:
   - `Variable` → take the group key value directly
   - `Aggregate` → collect values of `agg.var` across all bindings in the group, apply `agg.func`
4. Return rows in group-insertion order

**Aggregate function semantics:**

| Function | Accepted types | Result type | Empty group result | Type mismatch |
|---|---|---|---|---|
| `count` | any | `Value::Integer` | `0` | n/a |
| `count-distinct` | any | `Value::Integer` | `0` | n/a |
| `sum` | `Integer`, `Float` | `Integer` or `Float` (widening if mixed) | `0` | `Err` |
| `sum-distinct` | `Integer`, `Float` | same | `0` | `Err` |
| `min` | `Integer`, `Float`, `String` | same as input | cannot occur | `Err` |
| `max` | `Integer`, `Float`, `String` | same as input | cannot occur | `Err` |

Notes:
- `sum`/`sum-distinct` with mixed `Integer`/`Float` values widens to `Float`
- `min`/`max` on empty groups cannot occur — groups only exist when there is at least one binding
- Type errors return `Err` immediately (fail fast, no silent skipping)

The existing variable-extraction loop (no aggregates) is unchanged — `apply_aggregation` is only called when `find_specs` contains at least one `FindSpec::Aggregate`.

### 4. Evaluator (`src/query/datalog/evaluator.rs`)

Type propagation only — no logic changes. Anywhere `DatalogQuery.find` is accessed as `Vec<String>`, update to call `.var()` or `.display_name()` on `FindSpec` as appropriate. Estimated 3–5 call sites.

---

## Error Handling

| Error | Kind | Message |
|---|---|---|
| Unknown aggregate function | Parse error | `"Unknown aggregate function: 'foo'"` |
| Non-variable as aggregate argument | Parse error | `"Aggregate argument must be a variable"` |
| Aggregate var not bound in :where | Parse error | `"Aggregate variable ?x not bound in :where"` |
| `:with` var not bound in :where | Parse error | `"':with' variable ?x not bound in :where"` |
| `:with` without aggregate in :find | Parse error | `"':with' clause requires at least one aggregate in :find"` |
| `sum`/`sum-distinct` on non-numeric | Runtime error | `"sum: expected Integer or Float, got String"` |
| `min`/`max` on incompatible type | Runtime error | `"min: expected Integer, Float, or String, got Boolean"` |

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
| `sum_mixed_widens_to_float` | `Integer` + `Float` → `Float` result |
| `sum_distinct_deduplicates` | duplicate values excluded before summing |
| `min_max_integers` | boundary values correct |
| `min_max_strings` | lexicographic ordering |
| `with_prevents_overcollapse` | `sum` without `:with` vs with `:with ?e` differ |
| `count_empty_result` | returns `0` when no bindings |
| `sum_empty_result` | returns `0` when no bindings |
| `sum_type_error` | `Err` when summing strings |
| `min_type_error` | `Err` on incompatible type |
| `aggregate_with_rules` | aggregation after recursive rule evaluation |
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
| `src/query/datalog/executor.rs` | Add `apply_aggregation`; call it from both execution paths |
| `src/query/datalog/evaluator.rs` | Type propagation only (3–5 call sites) |
| `tests/aggregation_test.rs` | New — ~20 tests |

No changes to: `matcher.rs`, `optimizer.rs`, `stratification.rs`, `rules.rs`, `storage/`, `graph/`, `db.rs`, `wal.rs`, public API.

---

## Non-goals

- Arithmetic filter predicates (`[(> ?age 26)]`) — Phase 7.2b
- Collection aggregates (returning sets/vectors) — not planned
- User-defined aggregate functions — not planned
- Window functions — not planned
