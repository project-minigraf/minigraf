# Phase 7.3 — Disjunction (`or` / `or-join`) Design

## Overview

Add `or` and `or-join` disjunction to Minigraf's Datalog query language. These allow a clause to match if any one of several alternative branches matches, completing the core Datomic-style query primitives alongside `not` / `not-join`.

---

## Syntax

```datalog
;; or — union over branches; each branch is a single clause or (and ...) group
(or clause1 clause2 ...)
(or (and clause1 clause2 ...) clause3 ...)

;; or-join — like or, but explicitly lists variables shared with outer query
(or-join [?v1 ?v2] branch1 branch2 ...)
(or-join [?v1 ?v2] (and clause1 clause2 ...) (and clause3 clause4 ...))
```

`(and ...)` is a grouping form only — it parses to a `Vec<WhereClause>` branch, never a standalone `WhereClause` variant.

---

## Execution Model

**Post-pass** (consistent with Datomic semantics):

1. Positive patterns (`Pattern`, `RuleInvocation`) matched first via `match_patterns_seeded`
2. `or`/`or-join` expansion applied (via `apply_or_clauses`)
3. `not`/`not-join` post-filters applied
4. `Expr` clause filters/bindings applied

This produces the same results as sequential evaluation for valid (safe) queries and is simpler to implement.

---

## Types — `src/query/datalog/types.rs`

Two new variants added to `WhereClause`:

```rust
Or(Vec<Vec<WhereClause>>),

OrJoin {
    join_vars: Vec<String>,
    branches: Vec<Vec<WhereClause>>,
},
```

Each branch is `Vec<WhereClause>`, permitting full nesting — any `WhereClause` variant (including `Or`/`OrJoin`) may appear inside a branch.

Helper methods updated with `Or`/`OrJoin` arms:
- `WhereClause::rule_invocations()` — recurse into each branch
- `DatalogQuery::collect_rule_invocations_recursive()` — same treatment

---

## Parser — `src/query/datalog/parser.rs`

New parsing called from the `:where` clause parser.

### Safety checks (parse time, sequential binding tracking)

**`or`:**
- All branches must introduce the **same set of new variables** (variables not already bound by preceding clauses in the outer `:where`).
- Mismatched new-variable sets across branches → parse error.

**`or-join`:**
- All `join_vars` must be **bound by preceding clauses** at the point where `or-join` appears.
- Unbound join var → parse error.
- Branch-private variables (in branches but not in `join_vars`) are existentially quantified — no cross-branch consistency requirement on them.

---

## Matcher — `src/query/datalog/matcher.rs`

One new method:

```rust
pub(crate) fn match_patterns_seeded(
    &self,
    patterns: &[Pattern],
    seed: Vec<Binding>,
) -> Vec<Binding>
```

Identical to `match_patterns` but starts from `seed` bindings instead of a single empty binding. Existing `match_patterns` is equivalent to `match_patterns_seeded(patterns, vec![HashMap::new()])` — both are kept for call-site clarity.

---

## Executor — `src/query/datalog/executor.rs`

Two new `pub(crate)` functions:

### `evaluate_branch`

Evaluates a single branch against incoming bindings:

```rust
pub(crate) fn evaluate_branch(
    branch: &[WhereClause],
    incoming: Vec<Binding>,
    storage: &FactStorage,
    rules: &RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> Result<Vec<Binding>, QueryError>
```

Processing order within a branch:
1. `Pattern` / `RuleInvocation` clauses → `match_patterns_seeded`
2. Nested `Or`/`OrJoin` → recursive `apply_or_clauses`
3. `Not`/`NotJoin` clauses → existing post-filter logic
4. `Expr` clauses → `apply_expr_clauses`

### `apply_or_clauses`

Wiring function called from `execute_query`'s main clause loop:

```rust
pub(crate) fn apply_or_clauses(
    clauses: &[WhereClause],
    bindings: Vec<Binding>,
    storage: &FactStorage,
    rules: &RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> Result<Vec<Binding>, QueryError>
```

For each `Or(branches)` or `OrJoin { join_vars, branches }`:
- Runs `evaluate_branch` on each branch with the current bindings
- Unions the results (deduplicating by full binding map)
- For `or-join`: projects results down to `join_vars` + outer-bound vars, stripping branch-private vars
- Passes merged bindings forward to the next clause

`execute_query` calls `apply_or_clauses` after pattern matching, before `not`/`not-join`/`Expr` post-filters.

---

## Evaluator — `src/query/datalog/evaluator.rs`

### Route `or`/`or-join` rules to the mixed-rules path

Extend `has_not` predicate:

```rust
fn has_not(clause: &WhereClause) -> bool {
    matches!(
        clause,
        WhereClause::Not(_)
            | WhereClause::NotJoin { .. }
            | WhereClause::Or(_)
            | WhereClause::OrJoin { .. }
    )
}
```

### Mixed-rules loop

Extract `Or`/`OrJoin` clauses from the rule body and apply them via `apply_or_clauses` (imported from `executor.rs`, following the existing `eval_expr`/`is_truthy` import pattern). Expansion occurs after positive pattern matching, before `not`/`not-join` post-filters.

`RecursiveEvaluator::evaluate_rule` is unchanged — `Or`/`OrJoin` never reach it because `has_not` routes them away first.

---

## Stratification — `src/query/datalog/stratification.rs`

`DependencyGraph::from_rules` needs `Or`/`OrJoin` arms that recurse into branches and collect **positive dependencies** (rule invocations inside branches are positive edges, same as top-level `RuleInvocation`).

`Or`/`OrJoin` never create negative edges directly. Any `Not`/`NotJoin` nested inside a branch creates its own negative edge when the recursion reaches it. No new stratification concepts; existing cycle detection and stratum assignment unchanged.

---

## Testing Strategy

### Core `or` tests
- Two-branch `or` — union of results from both branches
- Single-branch `or` — degenerates to simple filter
- `or` where only one branch matches — correct subset returned
- `or` with `not`/`Expr` inside branches
- Nested `or` inside `or`

### Core `or-join` tests
- Basic `or-join` — branch-private vars stripped from output
- `or-join` with multiple join vars
- `or-join` where branches introduce different private vars
- `or-join` inside a rule body

### Safety / parse error tests
- `or` branches with mismatched new-variable sets → parse error
- `or-join` with unbound join var → parse error
- Invalid regex inside a branch (`matches?`)

### Rule tests
- Rule with `or` in body — routes to mixed-rules path, correct results
- Rule with `or-join` in body
- Recursive rule with `or` (base and recursive cases)

### Bi-temporal tests
- `or` with `:as-of` and `:valid-at` — temporal filters apply correctly across branches

### Stratification tests
- `or` containing `not` that forms a negative cycle → stratification rejects it
