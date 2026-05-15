# Wave 2 PR 1: Predicate Push-down + Mixed Rule Optimization

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `optimizer::plan()` to accept and interleave `Expr` predicate clauses at the earliest position where their variables are bound, and route the mixed-rules evaluator path through the updated planner.

**Architecture:** `plan()` signature changes from `Vec<Pattern> → Vec<(Pattern, IndexHint)>` to `Vec<WhereClause> → Vec<(WhereClause, Option<IndexHint>)>`. The executor's inner join loop processes clauses in plan order (Pattern → join bindings, Expr → filter/extend inline). The `StratifiedEvaluator` mixed-rules path is updated to use the same planner. Three files modified: `optimizer.rs`, `executor.rs`, `evaluator.rs`. One new method added to `matcher.rs`. One new benchmark group.

**Tech Stack:** Rust stable, `std::collections::HashSet` for variable tracking, Criterion for benchmarks.

---

## File Map

| File | Change |
|------|--------|
| `src/query/datalog/optimizer.rs` | New `expr_vars()`, `pattern_bound_vars()` helpers; `plan()` signature → `Vec<WhereClause>`; update existing test |
| `src/query/datalog/matcher.rs` | New `pub(crate) match_with_hint_seeded()` method |
| `src/query/datalog/executor.rs` | Updated `execute_query()` and `execute_query_with_rules()`: inline ordered loop, registry acquired earlier, top-level `apply_expr_clauses` post-passes removed |
| `src/query/datalog/evaluator.rs` | `StratifiedEvaluator` mixed-rules loop routes through `plan()` |
| `benches/minigraf_bench.rs` | New `query/predicate_pushdown` benchmark group |

---

## Task 1: Create git worktree

**Files:**
- No file changes — shell commands only

- [ ] **Step 1: Invoke the using-git-worktrees skill**

```
Use superpowers:using-git-worktrees to create a worktree for this feature.
Branch name: feature/issues-207-206-predicate-pushdown
Worktree dir: .worktrees/wave2-pr1-predicate-pushdown
```

- [ ] **Step 2: Verify worktree is active and clean**

```bash
git worktree list
git status
```

Expected: worktree listed, status shows clean working tree on `feature/issues-207-206-predicate-pushdown`.

---

## Task 2: Failing tests for `expr_vars()` and new `plan()` signature (optimizer.rs)

**Files:**
- Modify: `src/query/datalog/optimizer.rs` (test module only)

These tests compile-fail (new functions don't exist yet) and will pass after Task 3.

- [ ] **Step 1: Add imports and test helpers to the test module**

In `src/query/datalog/optimizer.rs`, extend the existing `#[cfg(test)] mod tests` block. Replace the current imports block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::datalog::types::{BinOp, EdnValue, Expr, Pattern, WhereClause};
    #[cfg(not(feature = "wasm"))]
    use crate::storage::index::Indexes;
    use uuid::Uuid;

    fn make_pattern(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Pattern {
        Pattern::new(entity, attribute, value)
    }

    fn var(s: &str) -> EdnValue {
        EdnValue::Symbol(format!("?{s}"))
    }
    fn kw(s: &str) -> EdnValue {
        EdnValue::Keyword(s.to_string())
    }
    fn str_val(s: &str) -> EdnValue {
        EdnValue::String(s.to_string())
    }
    fn entity_lit() -> EdnValue {
        EdnValue::Uuid(Uuid::new_v4())
    }
```

- [ ] **Step 2: Add failing tests for `expr_vars()`**

Append to the test module:

```rust
    // ── expr_vars() ──────────────────────────────────────────────────────────

    #[test]
    fn test_expr_vars_var() {
        let e = Expr::Var("?age".to_string());
        assert_eq!(expr_vars(&e), vec!["?age".to_string()]);
    }

    #[test]
    fn test_expr_vars_lit_is_empty() {
        let e = Expr::Lit(Value::Integer(42));
        assert!(expr_vars(&e).is_empty());
    }

    #[test]
    fn test_expr_vars_binop() {
        let e = Expr::BinOp(
            BinOp::Gt,
            Box::new(Expr::Var("?age".to_string())),
            Box::new(Expr::Lit(Value::Integer(30))),
        );
        assert_eq!(expr_vars(&e), vec!["?age".to_string()]);
    }

    #[test]
    fn test_expr_vars_nested_binop_collects_all() {
        // (> (+ ?a ?b) ?c)
        let e = Expr::BinOp(
            BinOp::Gt,
            Box::new(Expr::BinOp(
                BinOp::Add,
                Box::new(Expr::Var("?a".to_string())),
                Box::new(Expr::Var("?b".to_string())),
            )),
            Box::new(Expr::Var("?c".to_string())),
        );
        let vars = expr_vars(&e);
        assert!(vars.contains(&"?a".to_string()));
        assert!(vars.contains(&"?b".to_string()));
        assert!(vars.contains(&"?c".to_string()));
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn test_expr_vars_unary_op() {
        use crate::query::datalog::types::UnaryOp;
        let e = Expr::UnaryOp(UnaryOp::IntegerQ, Box::new(Expr::Var("?v".to_string())));
        assert_eq!(expr_vars(&e), vec!["?v".to_string()]);
    }
```

- [ ] **Step 3: Add failing tests for new `plan()` return type and push-down**

Append to the test module:

```rust
    // ── plan() — new signature and push-down ─────────────────────────────────

    #[test]
    fn test_plan_pattern_carries_some_hint() {
        #[cfg(not(feature = "wasm"))]
        use crate::storage::index::Indexes;
        #[cfg(not(feature = "wasm"))]
        {
            let p = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
            let planned = plan(vec![p], &Indexes::new());
            assert!(planned[0].1.is_some(), "Pattern entry must carry Some(IndexHint)");
        }
    }

    #[test]
    fn test_plan_expr_carries_none_hint() {
        #[cfg(not(feature = "wasm"))]
        use crate::storage::index::Indexes;
        #[cfg(not(feature = "wasm"))]
        {
            let p = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
            let expr = WhereClause::Expr {
                expr: Expr::Lit(Value::Boolean(true)),
                binding: None,
            };
            let planned = plan(vec![p, expr], &Indexes::new());
            let expr_entry = planned.iter().find(|(c, _)| matches!(c, WhereClause::Expr { .. }));
            assert!(expr_entry.is_some());
            assert!(expr_entry.unwrap().1.is_none(), "Expr entry must carry None hint");
        }
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_pushed_after_binding_pattern() {
        // Three patterns with equal selectivity (1 attr bound each) — stable sort preserves
        // original order: [p1, p2, p3]. Expr needs ?v, bound by p2 (pos 1).
        // Expected output: [p1, p2, expr, p3].
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":name"), var("n")));
        let p2 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let p3 = WhereClause::Pattern(make_pattern(var("e"), kw(":dept"), var("d")));
        let expr = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Gt,
                Box::new(Expr::Var("?v".to_string())),
                Box::new(Expr::Lit(Value::Integer(30))),
            ),
            binding: None,
        };
        let planned = plan(vec![p1, p2, p3, expr], &Indexes::new());
        assert_eq!(planned.len(), 4);
        // Item at index 2 must be the Expr (pushed after p2 which binds ?v at index 1).
        assert!(
            matches!(planned[2].0, WhereClause::Expr { .. }),
            "Expr must be at index 2"
        );
        // Item at index 3 must be a Pattern (p3, not yet seen when ?v first bound).
        assert!(
            matches!(planned[3].0, WhereClause::Pattern(_)),
            "p3 must be at index 3"
        );
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_no_vars_goes_to_end() {
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let expr = WhereClause::Expr {
            expr: Expr::Lit(Value::Boolean(true)),
            binding: None,
        };
        let planned = plan(vec![p1, expr], &Indexes::new());
        assert_eq!(planned.len(), 2);
        assert!(
            matches!(planned[1].0, WhereClause::Expr { .. }),
            "no-var Expr must be last"
        );
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_unbound_var_goes_to_end() {
        // ?x is never bound by any pattern
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let expr = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Gt,
                Box::new(Expr::Var("?x".to_string())),
                Box::new(Expr::Lit(Value::Integer(0))),
            ),
            binding: None,
        };
        let planned = plan(vec![p1, expr], &Indexes::new());
        assert_eq!(planned.len(), 2);
        assert!(
            matches!(planned[1].0, WhereClause::Expr { .. }),
            "Expr with unbound var must be last"
        );
    }
```

- [ ] **Step 4: Run to verify tests fail to compile**

```bash
cargo test --lib -p minigraf -- query::datalog::optimizer 2>&1 | head -30
```

Expected: compile errors — `expr_vars`, `pattern_bound_vars` not found; `plan()` type mismatch.

---

## Task 3: Implement `expr_vars()`, `pattern_bound_vars()`, and new `plan()` (optimizer.rs)

**Files:**
- Modify: `src/query/datalog/optimizer.rs`

- [ ] **Step 1: Update imports at the top of optimizer.rs**

Replace the existing import line:

```rust
use crate::query::datalog::types::{AttributeSpec, EdnValue, Pattern};
```

with:

```rust
use crate::query::datalog::types::{AttributeSpec, EdnValue, Expr, Pattern, WhereClause};
```

- [ ] **Step 2: Add `expr_vars()` and `pattern_bound_vars()` helpers after the existing helpers**

Add after the `attr_is_index_bound` function (around line 38):

```rust
/// Collect all logic-variable names (`?foo`) referenced in an Expr tree.
fn expr_vars(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::Var(s) => vec![s.clone()],
        Expr::Lit(_) | Expr::Slot(_) => vec![],
        Expr::BinOp(_, l, r) => {
            let mut vars = expr_vars(l);
            vars.extend(expr_vars(r));
            vars
        }
        Expr::UnaryOp(_, inner) => expr_vars(inner),
    }
}

/// Collect the logic-variable names bound (output) by a Pattern.
/// Only Symbol values starting with `?` count — literals never bind.
fn pattern_bound_vars(p: &Pattern) -> Vec<String> {
    let mut vars = Vec::new();
    if is_variable(&p.entity) {
        if let EdnValue::Symbol(s) = &p.entity {
            vars.push(s.clone());
        }
    }
    if let AttributeSpec::Real(attr) = &p.attribute {
        if is_variable(attr) {
            if let EdnValue::Symbol(s) = attr {
                vars.push(s.clone());
            }
        }
    }
    if is_variable(&p.value) {
        if let EdnValue::Symbol(s) = &p.value {
            vars.push(s.clone());
        }
    }
    vars
}
```

- [ ] **Step 3: Replace `plan()` with the new signature and push-down algorithm**

Replace the entire `plan()` function (currently lines 86–108):

```rust
/// Plan a list of where clauses: assign index hints to Pattern entries, push Expr
/// entries to the earliest position where all their variables are bound by preceding
/// patterns, and (non-wasm) sort patterns by selectivity.
///
/// Only `WhereClause::Pattern` and `WhereClause::Expr` variants should be passed in.
/// `Not`, `NotJoin`, `Or`, `OrJoin`, and `RuleInvocation` variants are handled by
/// the executor/evaluator and must not appear here.
///
/// Returns an interleaved `Vec<(WhereClause, Option<IndexHint>)>` where Pattern entries
/// carry `Some(hint)` and Expr entries carry `None`.
pub fn plan(
    clauses: Vec<WhereClause>,
    _indexes: &crate::storage::index::Indexes,
) -> Vec<(WhereClause, Option<IndexHint>)> {
    // Separate into patterns (with hints) and exprs.
    let mut patterns: Vec<(WhereClause, IndexHint)> = Vec::new();
    let mut exprs: Vec<WhereClause> = Vec::new();

    for clause in clauses {
        match &clause {
            WhereClause::Pattern(p) => {
                let hint = select_index(p);
                patterns.push((clause, hint));
            }
            WhereClause::Expr { .. } => exprs.push(clause),
            // Other variants must not be passed to plan(); silently skip.
            _ => {}
        }
    }

    // Stable sort patterns by selectivity descending (non-wasm only).
    // Preserves original order for ties, ensuring deterministic output.
    #[cfg(not(feature = "wasm"))]
    patterns.sort_by_key(|(clause, _)| {
        if let WhereClause::Pattern(p) = clause {
            std::cmp::Reverse(selectivity_score(p))
        } else {
            std::cmp::Reverse(0u8)
        }
    });

    // Start with sorted patterns only.
    let mut result: Vec<(WhereClause, Option<IndexHint>)> = patterns
        .into_iter()
        .map(|(clause, hint)| (clause, Some(hint)))
        .collect();

    // Push each Expr to the earliest position where all its variables are bound.
    for expr_clause in exprs {
        let vars: std::collections::HashSet<String> =
            if let WhereClause::Expr { expr, .. } = &expr_clause {
                expr_vars(expr).into_iter().collect()
            } else {
                Default::default()
            };

        let mut bound: std::collections::HashSet<String> = Default::default();
        // Default: append at end (covers no-var Exprs and vars never bound by any pattern).
        let mut insert_pos = result.len();

        if !vars.is_empty() {
            for (pos, (clause, _)) in result.iter().enumerate() {
                if let WhereClause::Pattern(p) = clause {
                    bound.extend(pattern_bound_vars(p));
                    if vars.is_subset(&bound) {
                        insert_pos = pos + 1;
                        break;
                    }
                }
            }
        }

        result.insert(insert_pos, (expr_clause, None));
    }

    result
}
```

- [ ] **Step 4: Update the existing join-ordering test to use the new signature**

Find and replace `test_join_ordering_moves_selective_pattern_first` in the test module:

```rust
    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_join_ordering_moves_selective_pattern_first() {
        let p1 = make_pattern(var("e"), kw(":age"), var("a")); // selectivity 1 (attr only)
        let p2 = make_pattern(entity_lit(), kw(":name"), var("v")); // selectivity 2 (entity + attr)
        let p1_attr = p1.attribute.clone();
        let p2_attr = p2.attribute.clone();
        let planned = plan(
            vec![
                WhereClause::Pattern(p1),
                WhereClause::Pattern(p2),
            ],
            &Indexes::new(),
        );
        // planned[0].0 is WhereClause::Pattern — extract the inner Pattern.
        let first_attr = match &planned[0].0 {
            WhereClause::Pattern(p) => p.attribute.clone(),
            _ => panic!("expected Pattern at index 0"),
        };
        let second_attr = match &planned[1].0 {
            WhereClause::Pattern(p) => p.attribute.clone(),
            _ => panic!("expected Pattern at index 1"),
        };
        assert_ne!(first_attr, p1_attr, "Lower-selectivity pattern must not be first");
        assert_eq!(first_attr, p2_attr, "Higher-selectivity pattern must be first");
        assert_eq!(second_attr, p1_attr, "Lower-selectivity pattern must be second");
    }
```

- [ ] **Step 5: Run optimizer tests to verify they pass**

```bash
cargo test --lib -p minigraf -- query::datalog::optimizer 2>&1
```

Expected: all optimizer tests pass, no compile errors.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/optimizer.rs
git commit -m "feat(optimizer): extend plan() to accept Expr clauses with push-down positioning

Adds expr_vars() and pattern_bound_vars() helpers. plan() now accepts
Vec<WhereClause> (Pattern + Expr) and returns Vec<(WhereClause, Option<IndexHint>)>.
Each Expr is inserted at the earliest position after all its variables are
bound by preceding patterns. No-variable Exprs and Exprs with unbound
variables are appended at the end.

Closes part of #207.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Add `match_with_hint_seeded()` to PatternMatcher (matcher.rs)

**Files:**
- Modify: `src/query/datalog/matcher.rs`

- [ ] **Step 1: Write a failing test for `match_with_hint_seeded()`**

Add to the test module in `src/query/datalog/matcher.rs`:

```rust
    #[test]
    fn test_match_with_hint_seeded_unit_seed_uses_hint_path() {
        // Unit seed (one empty map) — behaves like the first pattern in a query.
        // Any non-empty result confirms the method returns matches (not empty).
        use crate::query::datalog::optimizer::IndexHint;
        let storage = FactStorage::new();
        storage.load_fact(crate::graph::types::Fact {
            entity: uuid::Uuid::new_v4(),
            attribute: ":val".to_string(),
            value: crate::graph::types::Value::Integer(1),
            tx_id: 1,
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        }).unwrap();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap());
        let matcher = PatternMatcher::from_slice(facts);
        let p = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":val".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        let unit_seed = vec![HashMap::new()];
        let results = matcher.match_with_hint_seeded(unit_seed, &p, &IndexHint::Aevt);
        assert_eq!(results.len(), 1, "unit seed must produce one result per matching fact");
    }

    #[test]
    fn test_match_with_hint_seeded_real_bindings_uses_join() {
        // Non-unit seed — behaves like a subsequent pattern (join path).
        use crate::query::datalog::optimizer::IndexHint;
        use uuid::Uuid;
        let storage = FactStorage::new();
        let e = Uuid::new_v4();
        storage.load_fact(crate::graph::types::Fact {
            entity: e,
            attribute: ":val".to_string(),
            value: crate::graph::types::Value::Integer(42),
            tx_id: 1,
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        }).unwrap();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap());
        let matcher = PatternMatcher::from_slice(facts);
        let p = Pattern::new(
            EdnValue::Uuid(e),
            EdnValue::Keyword(":val".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        // Seed with a binding that already has some data (non-unit).
        let seed = vec![{
            let mut m = HashMap::new();
            m.insert("?other".to_string(), crate::graph::types::Value::Integer(99));
            m
        }];
        let results = matcher.match_with_hint_seeded(seed, &p, &IndexHint::Eavt);
        assert_eq!(results.len(), 1, "join path must unify with existing binding");
        assert_eq!(
            results[0].get("?v"),
            Some(&crate::graph::types::Value::Integer(42))
        );
    }
```

- [ ] **Step 2: Run to confirm tests fail to compile**

```bash
cargo test --lib -p minigraf -- query::datalog::matcher 2>&1 | head -20
```

Expected: compile error — `match_with_hint_seeded` not found.

- [ ] **Step 3: Implement `match_with_hint_seeded()`**

Add the following method to the `impl PatternMatcher` block in `src/query/datalog/matcher.rs`, after `match_patterns_with_hints`:

```rust
    /// Match a single pattern against existing bindings, using an index hint when the
    /// bindings are the unit seed (one empty map — i.e., this is the first pattern).
    ///
    /// When `seed` is the unit binding (`[{}]`), delegates to `match_pattern_with_hint`
    /// for an indexed lookup. For all other seeds, delegates to `join_with_pattern`
    /// (hash-join path) which already picks the right strategy.
    ///
    /// Used by the executor and evaluator incremental plan loops introduced in #207.
    pub(crate) fn match_with_hint_seeded(
        &self,
        seed: Vec<Bindings>,
        pattern: &Pattern,
        hint: &crate::query::datalog::optimizer::IndexHint,
    ) -> Vec<Bindings> {
        if seed.len() == 1 && seed[0].is_empty() {
            // First pattern in the plan — use the index hint for a targeted lookup.
            self.match_pattern_with_hint(pattern, hint)
        } else {
            // Subsequent pattern — join_with_pattern uses hash-join when possible.
            self.join_with_pattern(seed, pattern)
        }
    }
```

- [ ] **Step 4: Run matcher tests to confirm they pass**

```bash
cargo test --lib -p minigraf -- query::datalog::matcher 2>&1
```

Expected: all matcher tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/matcher.rs
git commit -m "feat(matcher): add match_with_hint_seeded() for incremental plan execution

Used by the executor and evaluator plan loops added in #207: applies
an index hint for the first pattern (unit seed) and falls back to the
hash-join path for subsequent patterns.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Update `execute_query()` in executor.rs

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write integration tests that assert semantics are preserved**

Add to the `#[cfg(test)]` module in `src/query/datalog/executor.rs`:

```rust
    #[test]
    fn test_expr_pushdown_preserves_query_results() {
        // Query with a selective Expr predicate: results must be identical before and after
        // the push-down refactor. This test serves as a regression guard.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute("(transact [[:e1 :val 10] [:e2 :val 20] [:e3 :val 30]])")
            .unwrap();
        let result = executor
            .execute("(query [:find ?e ?v :where [?e :val ?v] [(> ?v 15)]])")
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "only :e2 and :e3 have :val > 15");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_expr_pushdown_multi_pattern_preserves_results() {
        // Multi-pattern query with Expr in the middle: [?e :val ?v] [?e :name ?n] [(> ?v 10)]
        // Expr should be pushed between the two patterns.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute(r#"(transact [[:e1 :val 5] [:e1 :name "a"] [:e2 :val 20] [:e2 :name "b"]])"#)
            .unwrap();
        let result = executor
            .execute(r#"(query [:find ?e ?n :where [?e :val ?v] [?e :name ?n] [(> ?v 10)]])"#)
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 1, "only :e2 passes the predicate");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_expr_binding_form_preserves_results() {
        // Binding-form Expr: [(* ?v 2) ?doubled] — should still work after refactor.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute("(transact [[:e1 :val 5] [:e2 :val 10]])")
            .unwrap();
        let result = executor
            .execute("(query [:find ?e ?doubled :where [?e :val ?v] [(* ?v 2) ?doubled]])")
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "both entities must appear");
        } else {
            panic!("expected QueryResults");
        }
    }
```

- [ ] **Step 2: Run to confirm tests pass against the current (pre-refactor) code**

```bash
cargo test --lib -p minigraf -- query::datalog::executor::tests::test_expr_pushdown 2>&1
```

Expected: all three new tests PASS (they test semantics, not the push-down path).

- [ ] **Step 3: Refactor `execute_query()` — move registry acquisition and replace pattern/expr handling**

In `execute_query()`, make the following changes:

**3a.** Move registry acquisition to BEFORE the pattern matching block. Replace the current sequence (starting around line 493):

```rust
        let patterns = query.get_patterns();

        // Plan patterns: assign index hints and reorder by selectivity.
        let planned_patterns = optimizer::plan(patterns, &self.indexes);

        // Match all patterns in planned order and get bindings
        let bindings = matcher.match_patterns_with_hints(&planned_patterns);

        // Acquire the function registry once; used by apply_or_clauses, not_body_matches,
        // apply_expr_clauses, and apply_post_processing below.
        let registry = self
            .functions
            .read()
            .map_err(|_| anyhow!("functions lock poisoned"))?;
```

with:

```rust
        // Acquire function registry before the plan loop — needed for inline Expr evaluation.
        let registry = self
            .functions
            .read()
            .map_err(|_| anyhow!("functions lock poisoned"))?;

        // Pre-validate UDF predicate names: surface unknown predicates as errors before
        // processing any rows (matches the behaviour of the former apply_expr_clauses post-pass).
        for clause in &query.where_clauses {
            if let WhereClause::Expr {
                expr: Expr::UnaryOp(UnaryOp::Udf(name), _),
                ..
            } = clause
            {
                if registry.get_predicate(name).is_none() {
                    anyhow::bail!("unknown predicate: '{}'", name);
                }
            }
        }

        // Collect Pattern and Expr top-level clauses for the planner.
        // Not/NotJoin/Or/OrJoin are extracted separately below and applied as post-filters.
        let plan_clauses: Vec<WhereClause> = query
            .where_clauses
            .iter()
            .filter(|c| matches!(c, WhereClause::Pattern(_) | WhereClause::Expr { .. }))
            .cloned()
            .collect();

        let planned = optimizer::plan(plan_clauses, &self.indexes);

        // Process planned clauses in order: Pattern → expand bindings, Expr → filter/extend.
        let mut bindings: Vec<std::collections::HashMap<String, crate::graph::types::Value>> =
            vec![std::collections::HashMap::new()];
        for (clause, hint) in planned {
            match clause {
                WhereClause::Pattern(p) => {
                    bindings = matcher.match_with_hint_seeded(
                        bindings,
                        &p,
                        hint.as_ref().unwrap_or(&optimizer::IndexHint::Eavt),
                    );
                }
                WhereClause::Expr { expr, binding: out } => {
                    bindings = bindings
                        .into_iter()
                        .filter_map(|mut b| {
                            match eval_expr(&expr, &b, Some(&registry)) {
                                Ok(v) => {
                                    if let Some(var) = &out {
                                        b.insert(var.clone(), v);
                                        Some(b)
                                    } else if is_truthy(&v) {
                                        Some(b)
                                    } else {
                                        None
                                    }
                                }
                                Err(_) => None,
                            }
                        })
                        .collect();
                }
                _ => {}
            }
        }
```

**3b.** Remove the top-level `apply_expr_clauses` post-pass. Find and delete (around line 810):

```rust
        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses, &registry)?;
```

Replace the reference to `filtered_bindings` in the line immediately after with `not_filtered`:

```rust
        let results =
            apply_post_processing(not_filtered, &query.find, &query.with_vars, &registry)?;
```

- [ ] **Step 4: Run tests to confirm they still pass**

```bash
cargo test --lib -p minigraf -- query::datalog::executor 2>&1
```

Expected: all executor tests pass, including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): inline Expr push-down in execute_query() plan loop

Replaces the two-phase pattern-match-then-expr-filter with a single
ordered loop that processes Pattern (join) and Expr (filter/extend)
clauses in plan order. Registry is acquired before the loop for inline
Expr evaluation. Removes the top-level apply_expr_clauses post-pass.
Not/NotJoin/Or/OrJoin handling is unchanged.

Closes part of #207.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 6: Update `execute_query_with_rules()` in executor.rs

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write integration tests for the rules path**

Add to the test module in `src/query/datalog/executor.rs`:

```rust
    #[test]
    fn test_expr_pushdown_with_rules_preserves_results() {
        // Query that uses rules AND an Expr predicate — exercises execute_query_with_rules.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute("(transact [[:e1 :val 5] [:e2 :val 20] [:e3 :val 30]])")
            .unwrap();
        executor
            .execute("(rule [(high ?e) [?e :val ?v] [(> ?v 15)]])")
            .unwrap();
        let result = executor
            .execute("(query [:find ?e :where (high ?e)])")
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "only :e2 and :e3 qualify");
        } else {
            panic!("expected QueryResults");
        }
    }
```

- [ ] **Step 2: Run to confirm test passes against current code**

```bash
cargo test --lib -p minigraf -- query::datalog::executor::tests::test_expr_pushdown_with_rules 2>&1
```

Expected: PASS (semantics test; push-down not yet applied here).

- [ ] **Step 3: Refactor `execute_query_with_rules()` — replace pattern/expr handling**

In `execute_query_with_rules()`, replace the current sequence (around lines 881–924):

```rust
        // Convert ONLY top-level rule invocations to positive-match patterns.
        // Rule invocations inside `not` bodies are handled by the not-post-filter below.
        // (reachable ?x ?y) becomes [?x :reachable ?y]
        let mut all_patterns = query.get_patterns();

        for (predicate, args) in query.get_top_level_rule_invocations() {
            let pattern = match args.len() {
                1 => { ... }
                2 => { ... }
                n => { return Err(...); }
            };
            all_patterns.push(pattern);
        }

        // Compute derived_facts Arc once; reuse for or-clauses and not-post-filter.
        let derived_facts: Arc<[Fact]> = ...;

        // Match all patterns against derived facts
        let matcher = PatternMatcher::from_slice_with_valid_at(derived_facts.clone(), valid_at_value.clone());
        let bindings = matcher.match_patterns(&all_patterns);

        // Acquire the function registry once; ...
        let registry = self.functions.read()...;
```

with:

```rust
        // Compute derived_facts Arc once; reuse for plan loop, or-clauses and not-post-filter.
        // Must use derived_storage (includes rule-derived facts), not filtered_facts (base only).
        let derived_facts: Arc<[Fact]> =
            Arc::from(derived_storage.get_asserted_facts().unwrap_or_default());

        let matcher =
            PatternMatcher::from_slice_with_valid_at(derived_facts.clone(), valid_at_value.clone());

        // Acquire function registry before the plan loop — needed for inline Expr evaluation.
        let registry = self
            .functions
            .read()
            .map_err(|_| anyhow!("functions lock poisoned"))?;

        // Pre-validate UDF predicate names.
        for clause in &query.where_clauses {
            if let WhereClause::Expr {
                expr: Expr::UnaryOp(UnaryOp::Udf(name), _),
                ..
            } = clause
            {
                if registry.get_predicate(name).is_none() {
                    anyhow::bail!("unknown predicate: '{}'", name);
                }
            }
        }

        // Collect Pattern and Expr top-level clauses for the planner.
        // Rule invocations are converted to WhereClause::Pattern against derived_storage.
        let mut plan_clauses: Vec<WhereClause> = query
            .where_clauses
            .iter()
            .filter(|c| matches!(c, WhereClause::Pattern(_) | WhereClause::Expr { .. }))
            .cloned()
            .collect();

        for (predicate, args) in query.get_top_level_rule_invocations() {
            let pattern = match args.len() {
                1 => {
                    #[allow(clippy::indexing_slicing)]
                    let entity = args[0].clone();
                    Pattern::new(
                        entity,
                        EdnValue::Keyword(format!(":{}", predicate)),
                        EdnValue::Symbol("?_rule_value".to_string()),
                    )
                }
                2 => {
                    #[allow(clippy::indexing_slicing)]
                    let entity = args[0].clone();
                    #[allow(clippy::indexing_slicing)]
                    let value = args[1].clone();
                    Pattern::new(entity, EdnValue::Keyword(format!(":{}", predicate)), value)
                }
                n => {
                    return Err(anyhow!(
                        "Rule invocation '{}' must have 1 or 2 arguments, got {}",
                        predicate,
                        n
                    ));
                }
            };
            plan_clauses.push(WhereClause::Pattern(pattern));
        }

        let planned = optimizer::plan(plan_clauses, &self.indexes);

        // Process planned clauses in order: Pattern → expand, Expr → filter/extend.
        let mut bindings: Vec<std::collections::HashMap<String, crate::graph::types::Value>> =
            vec![std::collections::HashMap::new()];
        for (clause, hint) in planned {
            match clause {
                WhereClause::Pattern(p) => {
                    bindings = matcher.match_with_hint_seeded(
                        bindings,
                        &p,
                        hint.as_ref().unwrap_or(&optimizer::IndexHint::Eavt),
                    );
                }
                WhereClause::Expr { expr, binding: out } => {
                    bindings = bindings
                        .into_iter()
                        .filter_map(|mut b| {
                            match eval_expr(&expr, &b, Some(&registry)) {
                                Ok(v) => {
                                    if let Some(var) = &out {
                                        b.insert(var.clone(), v);
                                        Some(b)
                                    } else if is_truthy(&v) {
                                        Some(b)
                                    } else {
                                        None
                                    }
                                }
                                Err(_) => None,
                            }
                        })
                        .collect();
                }
                _ => {}
            }
        }
```

**3b.** Remove the top-level `apply_expr_clauses` post-pass in this function. Find and delete (around line 1094):

```rust
        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses, &registry)?;
```

Replace the `filtered_bindings` reference immediately after with `not_filtered`:

```rust
        let results =
            apply_post_processing(not_filtered, &query.find, &query.with_vars, &registry)?;
```

- [ ] **Step 4: Run all executor tests**

```bash
cargo test --lib -p minigraf -- query::datalog::executor 2>&1
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): inline Expr push-down in execute_query_with_rules() plan loop

Mirrors Task 5 for the rule-backed query path. Rule invocations are
converted to WhereClause::Pattern before the plan call so they
participate in selectivity ordering. Removes second apply_expr_clauses
post-pass.

Closes part of #207.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 7: Update evaluator mixed-rules path (evaluator.rs) — closes #206

**Files:**
- Modify: `src/query/datalog/evaluator.rs`

- [ ] **Step 1: Write integration test for mixed-rules Expr push-down**

Add to the test module in `src/query/datalog/evaluator.rs`:

```rust
    #[test]
    fn test_mixed_rule_with_expr_preserves_semantics() {
        // Rule with not + Expr: (eligible ?x) :- [?x :val ?v] [(> ?v 10)] (not [?x :blocked true])
        // Expr should be pushed after [?x :val ?v] in the mixed-rules plan.
        // Result must be the same as before the refactor.
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));

        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let e3 = Uuid::new_v4();

        storage
            .transact(
                vec![
                    (e1, ":val".to_string(), Value::Integer(5)),
                    (e2, ":val".to_string(), Value::Integer(20)),
                    (e3, ":val".to_string(), Value::Integer(30)),
                    (e3, ":blocked".to_string(), Value::Boolean(true)),
                ],
                None,
            )
            .unwrap();

        register_test_rule(
            &rules,
            r#"(rule [(eligible ?x) [?x :val ?v] [(> ?v 10)] (not [?x :blocked true])])"#,
        );

        let evaluator = StratifiedEvaluator::new(
            storage,
            Arc::clone(&rules),
            Arc::clone(&functions),
            DEFAULT_MAX_ITERATIONS,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );

        let derived = evaluator.evaluate(&["eligible".to_string()]).unwrap();
        let facts = derived.get_asserted_facts().unwrap();
        let eligible: Vec<_> = facts
            .iter()
            .filter(|f| f.attribute.contains("eligible"))
            .collect();
        assert_eq!(eligible.len(), 1, "only e2 is eligible (e3 is blocked)");
    }
```

- [ ] **Step 2: Run to confirm test passes against current code**

```bash
cargo test --lib -p minigraf -- query::datalog::evaluator::tests::test_mixed_rule_with_expr 2>&1
```

Expected: PASS (semantics test).

- [ ] **Step 3: Refactor the mixed-rules loop in `StratifiedEvaluator::evaluate()`**

In `src/query/datalog/evaluator.rs`, inside the mixed-rules loop (around line 729), replace the `positive_patterns` + `body_expr_clauses` extraction and the `raw_candidates` + `candidates` computation:

**Current code to replace** (roughly lines 729–827):

```rust
                let positive_patterns: Vec<Pattern> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Pattern(p) => Some(p.clone()),
                        WhereClause::RuleInvocation { predicate, args } => { ... }
                        WhereClause::Not(_) | WhereClause::NotJoin { .. } => None,
                        WhereClause::Expr { .. } => None,
                        WhereClause::Or(_) | WhereClause::OrJoin { .. } => None,
                    })
                    .collect();
                // ... not_clauses / not_join_clauses extraction ...
                let body_expr_clauses: Vec<&WhereClause> = rule.body.iter()
                    .filter(|c| matches!(c, WhereClause::Expr { .. })).collect();
                let accumulated_facts: Arc<[Fact]> = ...;
                let matcher = PatternMatcher::from_slice(accumulated_facts.clone());
                let raw_candidates = matcher.match_patterns(&positive_patterns);
                // apply_or_clauses → or_expanded
                let candidates =
                    apply_expr_clauses_in_evaluator(or_expanded, &body_expr_clauses, &fn_guard);
```

**New code**:

```rust
                // Collect Pattern and Expr clauses for the planner.
                // RuleInvocations are converted to Pattern; Not/NotJoin/Or/OrJoin extracted separately below.
                let mut plan_clauses: Vec<WhereClause> = Vec::new();
                for c in &rule.body {
                    match c {
                        WhereClause::Pattern(_) | WhereClause::Expr { .. } => {
                            plan_clauses.push(c.clone());
                        }
                        WhereClause::RuleInvocation { predicate, args } => {
                            let pattern = match args.len() {
                                1 => args.first().map(|a0| {
                                    Pattern::new(
                                        a0.clone(),
                                        EdnValue::Keyword(format!(":{}", predicate)),
                                        EdnValue::Symbol("?_rule_value".to_string()),
                                    )
                                }),
                                2 => args.first().and_then(|a0| {
                                    args.get(1).map(|a1| {
                                        Pattern::new(
                                            a0.clone(),
                                            EdnValue::Keyword(format!(":{}", predicate)),
                                            a1.clone(),
                                        )
                                    })
                                }),
                                _ => None,
                            };
                            if let Some(p) = pattern {
                                plan_clauses.push(WhereClause::Pattern(p));
                            }
                        }
                        WhereClause::Not(_)
                        | WhereClause::NotJoin { .. }
                        | WhereClause::Or(_)
                        | WhereClause::OrJoin { .. } => {}
                    }
                }

                let not_clauses: Vec<Vec<WhereClause>> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Not(inner) => Some(inner.clone()),
                        _ => None,
                    })
                    .collect();

                let not_join_clauses: Vec<(Vec<String>, Vec<WhereClause>)> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::NotJoin { join_vars, clauses } => {
                            Some((join_vars.clone(), clauses.clone()))
                        }
                        _ => None,
                    })
                    .collect();

                // Compute once; reuse for plan loop, apply_or_clauses, not-body matching.
                let accumulated_facts: Arc<[Fact]> =
                    Arc::from(accumulated.get_asserted_facts().unwrap_or_default());

                let matcher = PatternMatcher::from_slice(accumulated_facts.clone());

                // Plan: assigns index hints + pushes Expr to earliest binding position.
                let planned = crate::query::datalog::optimizer::plan(
                    plan_clauses,
                    &crate::storage::index::Indexes::new(),
                );

                // Process planned clauses in order: Pattern → join, Expr → filter/extend.
                let fn_guard = self
                    .functions
                    .read()
                    .map_err(|_| anyhow!("function registry lock poisoned"))?;
                let mut candidates: Vec<Bindings> = vec![Bindings::new()];
                for (clause, hint) in planned {
                    match clause {
                        WhereClause::Pattern(p) => {
                            candidates = matcher.match_with_hint_seeded(
                                candidates,
                                &p,
                                hint.as_ref()
                                    .unwrap_or(&crate::query::datalog::optimizer::IndexHint::Eavt),
                            );
                        }
                        WhereClause::Expr { expr, binding: out } => {
                            use crate::query::datalog::executor::{eval_expr, is_truthy};
                            candidates = candidates
                                .into_iter()
                                .filter_map(|mut b| {
                                    match eval_expr(&expr, &b, Some(&fn_guard)) {
                                        Ok(v) => {
                                            if let Some(var) = &out {
                                                b.insert(var.clone(), v);
                                                Some(b)
                                            } else if is_truthy(&v) {
                                                Some(b)
                                            } else {
                                                None
                                            }
                                        }
                                        Err(_) => None,
                                    }
                                })
                                .collect();
                        }
                        _ => {}
                    }
                }
                drop(fn_guard);

                // Apply Or/OrJoin clauses (mirrors top-level execute_query order).
                let candidates = {
                    use crate::query::datalog::executor::apply_or_clauses;
                    let registry_guard = self
                        .rules
                        .read()
                        .map_err(|_| anyhow!("rule registry lock poisoned"))?;
                    let fn_registry = crate::query::datalog::functions::FunctionRegistry::with_builtins();
                    let expanded = apply_or_clauses(
                        &rule.body,
                        candidates,
                        accumulated_facts.clone(),
                        &registry_guard,
                        None,
                        None,
                        &fn_registry,
                    )?;
                    drop(registry_guard);
                    expanded
                };
```

The remaining `'binding: for binding in candidates { ... }` loop (not-body filtering + head instantiation) is **unchanged**.

- [ ] **Step 4: Run evaluator tests**

```bash
cargo test --lib -p minigraf -- query::datalog::evaluator 2>&1
```

Expected: all tests pass.

- [ ] **Step 5: Run all library tests**

```bash
cargo test --lib -p minigraf 2>&1
```

Expected: all tests pass. Fix any compile errors from call-site changes.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "feat(evaluator): route mixed-rules path through plan() for Expr push-down

StratifiedEvaluator mixed-rules loop now collects Pattern + Expr clauses
and calls optimizer::plan(), processing them in the same ordered loop as
the executor. Expr clauses are pushed down to the earliest valid position
in the rule body. Not/NotJoin filtering is unchanged.

Closes #206.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 8: Add predicate push-down benchmark

**Files:**
- Modify: `benches/minigraf_bench.rs`
- Modify: `benches/helpers/mod.rs` (optional — `populate_with_names` already exists)

- [ ] **Step 1: Add a new `bench_predicate_pushdown` function in `benches/minigraf_bench.rs`**

Add after the last existing bench function, before `criterion_group!`:

```rust
// ── Task 8: query/predicate_pushdown ─────────────────────────────────────────

fn bench_predicate_pushdown(c: &mut Criterion) {
    // Fixture: n entities each with :val (integer) and :name (string).
    // Uses helpers::populate_with_names(n).
    //
    // Query under test: multi-pattern with a selective Expr predicate.
    //   (query [:find ?e ?n :where [?e :val ?v] [?e :name ?n] [(> ?v <threshold>)]])
    //
    // Predicate selects 10% of entities. With push-down, [?e :name ?n] is only
    // joined for the 10% that pass the filter; without push-down it would join all.
    const SCALES: &[(&str, usize)] = &[
        ("1k", 1_000),
        ("10k", 10_000),
        ("100k", 100_000),
    ];

    let mut group = c.benchmark_group("query/predicate_pushdown");
    group.sample_size(10);

    for &(label, n) in SCALES {
        let db = helpers::populate_with_names(n);
        // Threshold = 90th percentile value → 10% of entities pass the predicate.
        let threshold = (n as i64) * 9 / 10;
        let query = format!(
            "(query [:find ?e ?n :where [?e :val ?v] [?e :name ?n] [(> ?v {})]])",
            threshold
        );
        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &(db, query),
            |b, (db, q)| {
                b.iter(|| db.execute(q).unwrap());
            },
        );
    }

    group.finish();
}
```

- [ ] **Step 2: Register the benchmark in `criterion_group!`**

Find the `criterion_group!` macro and add `bench_predicate_pushdown`:

```rust
criterion_group!(
    benches,
    bench_insert,
    bench_insert_file,
    // ... existing entries ...
    bench_predicate_pushdown,
);
```

- [ ] **Step 3: Verify the benchmark compiles and runs**

```bash
cargo bench --bench minigraf_bench -- query/predicate_pushdown 2>&1 | tail -20
```

Expected: benchmark runs and prints throughput numbers, no panics.

- [ ] **Step 4: Commit**

```bash
git add benches/minigraf_bench.rs
git commit -m "bench: add query/predicate_pushdown benchmark group

Measures multi-pattern query with a selective [(> ?v N)] predicate at
1K/10K/100K fact scales. Threshold set at 90th percentile (10% pass rate).
Provides baseline for evaluating push-down gains.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 9: Full test suite, final cleanup, open PR

**Files:**
- No new file changes — verification and PR creation

- [ ] **Step 1: Run the full test suite**

```bash
cargo test 2>&1
```

Expected: all tests pass (unit + integration + doc). Fix any remaining compile errors.

- [ ] **Step 2: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1
```

Expected: no warnings. Fix any clippy lints before continuing.

- [ ] **Step 3: Run the full benchmark suite to confirm no panics**

```bash
cargo bench --bench minigraf_bench 2>&1 | tail -30
```

Expected: all benchmarks complete without panic.

- [ ] **Step 4: Open the PR**

```bash
gh pr create \
  --repo project-minigraf/minigraf \
  --title "perf: predicate push-down for Expr clauses + mixed rule optimization (#207, #206)" \
  --body "$(cat <<'EOF'
## Summary

- **#207**: Extends `optimizer::plan()` to accept `Vec<WhereClause>` (Pattern + Expr) and return an interleaved plan. Each `Expr` predicate clause is positioned at the earliest point where all its variables are bound by preceding patterns, rather than being applied as a post-pass over the full binding set.
- **#206**: Routes the `StratifiedEvaluator` mixed-rules path through the updated `plan()`, giving mixed rules both selectivity-based pattern ordering and Expr push-down with no separate implementation.

## New helper

`PatternMatcher::match_with_hint_seeded()` — used by both executor and evaluator to process the plan incrementally (first pattern uses index hint; subsequent patterns use hash-join).

## Test plan

- [ ] `cargo test` — all tests pass
- [ ] `cargo clippy -- -D warnings` — no warnings
- [ ] `cargo bench --bench minigraf_bench -- query/predicate_pushdown` — new benchmark runs without panic

## Follow-up

- Not/NotJoin push-down (same mechanism, more complex safety rules): project-minigraf/minigraf#248

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Monitor CI and fix any failures**

```bash
gh pr checks --watch
```

Expected: all checks green. If any fail, diagnose and fix before merging.
