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

### Non-rules path (`execute_query`)

1. Temporal filtering (`filter_facts_for_query`)
2. Top-level `Pattern`/`RuleInvocation` clauses matched via `match_patterns`
3. **`apply_or_clauses` applied to the resulting bindings** ← new step
4. `not`/`not-join` post-filters applied
5. `Expr` clause filters/bindings applied (`apply_expr_clauses`)

`apply_or_clauses` is called after `matcher.match_patterns(...)` returns `bindings` and before the existing `not_clauses`/`not_join_clauses` collection and post-filter pass. There is no "main clause loop" — `apply_or_clauses` is inserted as an explicit pass between steps 2 and 4 above.

`Or`/`OrJoin` clauses are **not** fed through `query.get_patterns()`. That function filters for top-level `WhereClause::Pattern` only and will not include `Or`/`OrJoin`. Patterns inside branches are matched by `evaluate_branch` via `match_patterns_seeded` against the same `filtered_storage`.

### Rules path (mixed-rules loop in `StratifiedEvaluator`)

Same relative position: after `matcher.match_patterns(&positive_patterns)` returns `raw_candidates`, `apply_or_clauses` is applied to expand them, then `apply_expr_clauses_in_evaluator`, then not/not-join post-filters.

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

**Adding these variants makes existing exhaustive matches fail to compile.** The following locations need `Or`/`OrJoin` arms added:

- `WhereClause::rule_invocations()` — recurse into each branch, collect all `RuleInvocation`s
- `DatalogQuery::collect_rule_invocations_recursive()` — same treatment
- `DependencyGraph::from_rules` in `stratification.rs` — see Stratification section
- The mixed-rules `positive_patterns` filter_map in `evaluator.rs` (lines 624–645) — add `WhereClause::Or(_) | WhereClause::OrJoin { .. } => None` (or-expansion is handled separately)
- The mixed-rules `not_clauses` and `not_join_clauses` collectors in `evaluator.rs` (lines 647–665) — add `_ => None` already present; no change needed
- The `not_clauses`/`not_join_clauses` collectors in `execute_query` (`executor.rs` lines 198–216) — add `_ => None` already present; no change needed

---

## Parser — `src/query/datalog/parser.rs`

New parsing called from the `:where` clause parser.

### Safety checks (parse time, sequential binding tracking)

**`or`:**
- All branches must introduce the **same set of new variables** — variables not already bound by preceding clauses in the outer `:where` at the point where `or` appears.
- Variables bound by `Expr` clauses with `binding: Some(var)` inside a branch count toward that branch's "new variables" set.
- Mismatched new-variable sets across branches → parse error.

**`or-join`:**
- All `join_vars` must be **bound by preceding clauses** at the point where `or-join` appears.
- Unbound join var → parse error.
- Branch-private variables (in branches but not in `join_vars`) are existentially quantified — no cross-branch consistency requirement on them.

---

## Matcher — `src/query/datalog/matcher.rs`

One new independent method (not a delegation wrapper):

```rust
pub(crate) fn match_patterns_seeded(
    &self,
    patterns: &[Pattern],
    seed: Vec<Bindings>,
) -> Vec<Bindings>
```

Starts from `seed` bindings instead of a single empty binding. Iterates over each seed binding and joins each pattern in sequence, collecting all results. The existing `match_patterns` is left unchanged as its own implementation — both coexist.

`Bindings` here is `pub type Bindings = HashMap<String, Value>` from `matcher.rs` (already exported).

---

## Executor — `src/query/datalog/executor.rs`

Two new `pub(crate)` functions.

### Type alias note

`executor.rs` defines a module-private `type Binding = std::collections::HashMap<String, Value>` at line 785. `evaluator.rs` uses `type Bindings = HashMap<String, Value>` (from `matcher.rs`). These are the same concrete type under different aliases. New functions in `executor.rs` use the existing `Binding` alias; calling code in `evaluator.rs` uses `Bindings` — this compiles cleanly with no conversion needed.

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
) -> anyhow::Result<Vec<Binding>>
```

Processing order within a branch:
1. `Pattern` / `RuleInvocation` clauses → `match_patterns_seeded`
2. Nested `Or`/`OrJoin` → recursive `apply_or_clauses`
3. `Not`/`NotJoin` clauses → existing post-filter logic
4. `Expr` clauses → `apply_expr_clauses`

`evaluate_branch` does not call back into `evaluator.rs` — it uses only functions in `executor.rs` and `matcher.rs`. No circular module dependency.

### `apply_or_clauses`

Applied as a pass over the full `where_clauses` list, operating on a bindings vector:

```rust
pub(crate) fn apply_or_clauses(
    clauses: &[WhereClause],
    bindings: Vec<Binding>,
    storage: &FactStorage,
    rules: &RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>>
```

For each `Or(branches)` or `OrJoin { join_vars, branches }` encountered in `clauses`:
- Runs `evaluate_branch` on each branch with the current bindings
- Unions the results (deduplicating by full binding map)
- For `or-join`: projects each result down to `join_vars` + outer-bound vars, stripping branch-private vars. **Outer-bound vars** are the set of variable names already present as keys in the incoming binding before this `or-join` clause is processed. Projection keeps any key that is either in `join_vars` or was already a key in the incoming binding; removes any key introduced by a branch that is not in `join_vars`.
- Passes the merged bindings forward to the next clause in `clauses`

In `execute_query`: called after `matcher.match_patterns(...)` and before the `not_clauses`/`not_join_clauses` post-filter pass.

In `evaluator.rs` mixed-rules loop: called after `matcher.match_patterns(&positive_patterns)` and before `apply_expr_clauses_in_evaluator`. Import via `use crate::query::datalog::executor::{..., apply_or_clauses, evaluate_branch}` (same direction as existing `eval_expr`/`is_truthy` import on line 476).

---

## Evaluator — `src/query/datalog/evaluator.rs`

### Route `or`/`or-join` rules to the mixed-rules path

The routing predicate is an inline closure at lines 584–587:

```rust
let has_not = rule
    .body
    .iter()
    .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }));
```

Extend to include `Or`/`OrJoin`:

```rust
let has_not = rule
    .body
    .iter()
    .any(|c| matches!(
        c,
        WhereClause::Not(_)
            | WhereClause::NotJoin { .. }
            | WhereClause::Or(_)
            | WhereClause::OrJoin { .. }
    ));
```

### Mixed-rules loop

In the `positive_patterns` filter_map (lines 624–645), add an explicit arm for `Or`/`OrJoin` that returns `None` — these are handled by `apply_or_clauses`, not extracted as patterns:

```rust
WhereClause::Or(_) | WhereClause::OrJoin { .. } => None,
```

After `matcher.match_patterns(&positive_patterns)` returns `raw_candidates`, call `apply_or_clauses` to expand them before `apply_expr_clauses_in_evaluator`.

`RecursiveEvaluator::evaluate_rule` is unchanged — `Or`/`OrJoin` never reach it because the extended `has_not` predicate routes them to the mixed-rules path first.

---

## Stratification — `src/query/datalog/stratification.rs`

`DependencyGraph::from_rules` has an exhaustive match on `WhereClause` (lines 22–36). Adding `Or`/`OrJoin` variants will cause a **compile error** — non-exhaustive match. New arms must be added:

```rust
WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
    for branch in branches {
        for inner_clause in branch {
            if let WhereClause::RuleInvocation { predicate, .. } = inner_clause {
                entry.push((predicate.clone(), false)); // positive edge
            }
            // recurse into nested Or/OrJoin within branches
        }
    }
}
```

`Or`/`OrJoin` never create negative edges directly. Any `Not`/`NotJoin` nested inside a branch creates its own negative edge when recursion reaches it. Full recursion into branches (not just one level) is needed to handle nested `or`.

---

## Testing Strategy

### Core `or` tests
- Two-branch `or` — union of results from both branches
- Single-branch `or` — degenerates to simple filter
- `or` where only one branch matches — correct subset returned
- Both branches produce overlapping results — deduplication is correct (one result, not two)
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
- `or` branches with mismatched `Expr`-bound variable sets → parse error
- Invalid regex inside a branch (`matches?`)

### Rule tests
- Rule with `or` in body — routes to mixed-rules path, correct results
- Rule with `or-join` in body
- Recursive rule with `or` (base and recursive cases)

### Bi-temporal tests
- `or` with `:as-of` and `:valid-at` — temporal filters apply correctly across branches

### Stratification tests
- `or` containing `not` that forms a negative cycle → stratification rejects it
- `or` containing `RuleInvocation` — positive dependency edge recorded correctly
