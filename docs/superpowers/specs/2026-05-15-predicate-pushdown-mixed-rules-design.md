# Design: Predicate Push-down + Mixed Rule Optimization (Wave 2 PR 1)

**Issues**: #207 (predicate push-down for expression clauses), #206 (optimize mixed rule evaluation)  
**Date**: 2026-05-15  
**Branch**: single worktree + PR covering both issues

---

## Summary

Two related query execution improvements delivered in one PR:

- **#207**: Push `Expr` predicate clauses (e.g. `[(> ?age 30)]`) down to the earliest position in the execution plan where all their variables are bound, rather than applying them all in a post-pass after full pattern matching.
- **#206**: Route the mixed-rules path in `StratifiedEvaluator` through the updated planner so it benefits from both pattern selectivity ordering and predicate push-down — no separate implementation needed.

The unifying change is extending `optimizer::plan()` to accept both `Pattern` and `Expr` clauses and return an interleaved ordered plan.

---

## Current Behaviour

**Query execution (executor.rs)**:
```
where_clauses
  → extract Pattern variants
  → plan(patterns) → sorted [(Pattern, IndexHint)]   # selectivity order
  → PatternMatcher runs all patterns → full binding set
  → apply_expr_clauses(bindings) → post-pass filter  # all exprs at the end
```

**Mixed rules (evaluator.rs `StratifiedEvaluator`)**:
```
rule body
  → extract Pattern variants only (in original order, no plan() call)
  → PatternMatcher runs patterns → bindings
  → not/notjoin post-filter
```

Both paths leave `Expr` clauses to the end, potentially carrying large intermediate binding sets through every pattern join before any filter is applied.

---

## Design

### 1. `plan()` — new signature and algorithm

**File**: `src/query/datalog/optimizer.rs`

```rust
// Before
pub fn plan(
    patterns: Vec<Pattern>,
    indexes: &Indexes,
) -> Vec<(Pattern, IndexHint)>

// After
pub fn plan(
    clauses: Vec<WhereClause>,
    indexes: &Indexes,
) -> Vec<(WhereClause, Option<IndexHint>)>
```

Only `WhereClause::Pattern` and `WhereClause::Expr` variants are passed in. `Not`, `NotJoin`, `Or`, `OrJoin` continue to be extracted separately at call sites and applied as post-filters — their handling is unchanged.

**Algorithm**:

1. Separate input into patterns (assign `IndexHint` via `select_index`) and exprs.
2. `#[cfg(not(feature = "wasm"))]`: stable-sort patterns by selectivity descending — unchanged behaviour.
3. New helper `expr_vars(expr: &Expr) -> Vec<String>` walks the `Expr` tree and collects referenced variable names (symbols starting with `?`).
4. Walk sorted patterns left-to-right, tracking the set of variables bound so far. For each `Expr`, insert it at the position where all its variables first become bound.
5. Exprs with no variables, or variables never bound by any pattern, are placed at the end.

**`wasm` behaviour**: Pattern reordering remains gated behind `#[cfg(not(feature = "wasm"))]`. Expr push-down (step 4) is unconditional — it applies on all targets including WASM.

**Return type**: `Vec<(WhereClause, Option<IndexHint>)>` where `Pattern` entries carry `Some(IndexHint)` and `Expr` entries carry `None`.

---

### 2. Executor inner loop

**File**: `src/query/datalog/executor.rs`  
**Functions**: `execute_query()`, `execute_query_with_rules()`

Both functions currently use a two-phase structure: run all patterns, then call `apply_expr_clauses()`. After this change:

1. Collect top-level `Pattern` and `Expr` clauses into a `Vec<WhereClause>`.
2. Call `plan(clauses, indexes)` → interleaved `Vec<(WhereClause, Option<IndexHint>)>`.
3. Process in a single ordered loop:

```
bindings = [empty_binding]
for (clause, hint) in planned:
    WhereClause::Pattern(p) →
        bindings = PatternMatcher::match_with_hint(p, hint, facts, bindings)
    WhereClause::Expr { expr, binding: out } →
        bindings = filter_or_extend(bindings, expr, out)
```

`Not`, `NotJoin`, `Or`, `OrJoin` are extracted before the plan call and applied as post-filters afterward — no change to their handling. The **top-level** `apply_expr_clauses()` post-pass (executor.rs lines ~811, ~1094) is removed; its logic moves inline into the ordered loop. `apply_expr_clauses()` calls inside `not`/`not-join` body evaluation (lines ~1070, ~1216) are **unchanged** — those Expr clauses belong to the not-body scope and are not candidates for push-down.

---

### 3. Mixed-rules path

**File**: `src/query/datalog/evaluator.rs`  
**Function**: `StratifiedEvaluator::evaluate()` (mixed-rules loop, ~line 729)

Currently collects `positive_patterns: Vec<Pattern>` in original body order, bypassing the planner entirely.

After this change:
1. Collect `Pattern` and `Expr` `WhereClause` variants from the rule body.
2. Call `plan(clauses, indexes)` → interleaved plan.
3. Process with the same ordered loop as the executor.
4. `Not` and `NotJoin` filters continue to be applied as post-filters — no change.

Mixed rules gain both pattern selectivity ordering and predicate push-down with no separate implementation, satisfying #206.

---

### 4. Testing

**`src/query/datalog/optimizer.rs` unit tests** — `plan()` with mixed input:
- Expr with all vars bound by first pattern → positioned immediately after that pattern
- Expr with vars spread across multiple patterns → positioned after the last binding pattern
- Expr with no variables → placed at end
- Expr referencing a variable no pattern binds → placed at end
- Under `wasm` feature: patterns stay in user-written order, Exprs still pushed down

**`src/query/datalog/executor.rs` integration tests** — semantics preserved:
- Query with `[(> ?age 30)]` predicate produces same results before and after push-down
- Selective predicate on large fact set prunes intermediate bindings (verify via result count)

**`src/query/datalog/evaluator.rs` unit tests** — mixed-rule path:
- Rule with `not` + `Expr` in body: Expr pushed down correctly, not-filter still applied afterward
- Rule with `or` + `Expr` in body: same

**Benchmarks** (Criterion, per #207 acceptance criteria):
- `[(> ?age 30)]` selective predicate on large fact sets: throughput comparison before/after push-down

---

## Invariants Preserved

- **Correctness**: `eval_expr` returns `Err(())` for unbound variables (silently drops row). If push-down ever positions an Expr before all its vars are bound, the silent-drop behaviour acts as a safety net — results remain correct, just not optimally filtered. The `expr_vars()` helper must be correct to get the performance benefit.
- **Stratification safety**: `Not`/`NotJoin` handling is unchanged; push-down only applies to `Expr` clauses.
- **WASM portability**: Pattern reordering stays gated; Expr push-down is always on.
- **Backwards compatibility**: No change to query semantics, file format, or public API.

---

## Files Changed

| File | Change |
|------|--------|
| `src/query/datalog/optimizer.rs` | New `plan()` signature; new `expr_vars()` helper; push-down insertion algorithm |
| `src/query/datalog/executor.rs` | Updated call sites; single ordered processing loop; remove `apply_expr_clauses` post-pass |
| `src/query/datalog/evaluator.rs` | Mixed-rules path routes through `plan()` |
| `benches/` | New Criterion bench for selective predicate push-down |
