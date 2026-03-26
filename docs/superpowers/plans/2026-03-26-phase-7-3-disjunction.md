# Phase 7.3 — Disjunction (`or` / `or-join`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `or` / `or-join` disjunction to Minigraf's Datalog query language, allowing a `:where` clause to match if any one of several alternative branches succeeds.

**Architecture:** Post-pass execution model — top-level `Pattern`/`RuleInvocation` clauses are matched first, then `apply_or_clauses` expands `or`/`or-join` clauses, then existing `not`/`not-join`/`Expr` post-filters run. A new `pub(crate) evaluate_branch` helper in `executor.rs` handles branch evaluation and is called recursively for nesting; rules with `or`/`or-join` in their bodies route to the existing `mixed_rules` path in `StratifiedEvaluator`.

**Tech Stack:** Rust, `anyhow`, `HashMap<String, Value>` bindings, existing `PatternMatcher`/`FactStorage`/`RuleRegistry` APIs.

**Spec:** `docs/superpowers/specs/2026-03-26-phase-7-3-disjunction-design.md`

---

## File Map

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `Or`/`OrJoin` variants to `WhereClause`; fix `rule_invocations`, `has_negated_invocation`, `collect_rule_invocations_recursive` |
| `src/query/datalog/stratification.rs` | Add `collect_clause_deps` helper; replace inner match in `from_rules` |
| `src/query/datalog/matcher.rs` | Add `match_patterns_seeded` |
| `src/query/datalog/parser.rs` | Parse `or`/`or-join`/`and`; add safety checks in `check_expr_safety_with_bound`; update `outer_vars_from_clause` |
| `src/query/datalog/executor.rs` | Add `evaluate_branch`, `apply_or_clauses`; wire into `execute_query` and `execute_query_with_rules` |
| `src/query/datalog/evaluator.rs` | Change `rule_invocation_to_pattern` to `pub(crate)`; extend `has_not` closure; extend `positive_patterns` filter_map; wire `apply_or_clauses` in mixed-rules loop |
| `tests/disjunction_test.rs` | New integration test file |

---

## Task 1: Types — add Or/OrJoin variants and fix compile errors

**Files:**
- Modify: `src/query/datalog/types.rs`

The goal of this task is to add the new variants and fix the three exhaustive match compile errors that result. This task intentionally leaves `stratification.rs`, `parser.rs`, `executor.rs`, and `evaluator.rs` failing to compile — those are fixed in their own tasks. Run only the unit tests in `types.rs` in this task (via `-- types` filter).

- [ ] **Step 1: Write failing unit tests for Or/OrJoin in `types.rs`**

Add to `src/query/datalog/types.rs` inside the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn test_where_clause_or_variant_exists() {
    let branch1 = vec![WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":a".to_string()),
        EdnValue::Symbol("?v".to_string()),
    ))];
    let branch2 = vec![WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":b".to_string()),
        EdnValue::Symbol("?v".to_string()),
    ))];
    let or_clause = WhereClause::Or(vec![branch1, branch2]);
    assert!(matches!(or_clause, WhereClause::Or(_)));
}

#[test]
fn test_where_clause_or_join_variant_exists() {
    let branch = vec![WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":tag".to_string()),
        EdnValue::Symbol("?tag".to_string()),
    ))];
    let oj = WhereClause::OrJoin {
        join_vars: vec!["?e".to_string()],
        branches: vec![branch],
    };
    assert!(matches!(oj, WhereClause::OrJoin { .. }));
}

#[test]
fn test_rule_invocations_recurses_into_or_branches() {
    let branch = vec![WhereClause::RuleInvocation {
        predicate: "active".to_string(),
        args: vec![EdnValue::Symbol("?e".to_string())],
    }];
    let or_clause = WhereClause::Or(vec![branch]);
    assert_eq!(or_clause.rule_invocations(), vec!["active"]);
}

#[test]
fn test_has_negated_invocation_false_for_or() {
    let branch = vec![WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":a".to_string()),
        EdnValue::Boolean(true),
    ))];
    let or_clause = WhereClause::Or(vec![branch]);
    assert!(!or_clause.has_negated_invocation());
}

#[test]
fn test_collect_rule_invocations_recurses_into_or_branches() {
    let query = DatalogQuery::new(
        vec![FindSpec::Variable("?e".to_string())],
        vec![WhereClause::Or(vec![
            vec![WhereClause::RuleInvocation {
                predicate: "active".to_string(),
                args: vec![EdnValue::Symbol("?e".to_string())],
            }],
            vec![WhereClause::RuleInvocation {
                predicate: "pending".to_string(),
                args: vec![EdnValue::Symbol("?e".to_string())],
            }],
        ])],
    );
    let invocations = query.get_rule_invocations();
    assert_eq!(invocations.len(), 2);
    let pred_names: Vec<&str> = invocations.iter().map(|(p, _)| p.as_str()).collect();
    assert!(pred_names.contains(&"active"));
    assert!(pred_names.contains(&"pending"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/aditya/workspaces/rustrover/minigraf
cargo test -p minigraf --lib query::datalog::types 2>&1 | tail -20
```

Expected: compilation error about missing `Or`/`OrJoin` variants or test failures.

- [ ] **Step 3: Add Or/OrJoin variants to WhereClause**

In `src/query/datalog/types.rs`, inside the `WhereClause` enum (after `Expr`), add:

```rust
/// Disjunction: (or branch1 branch2 ...) — succeeds if any branch matches.
/// Each branch is a Vec<WhereClause>. A single clause is a one-element branch.
Or(Vec<Vec<WhereClause>>),
/// or-join: (or-join [?v1 ?v2] branch1 branch2 ...)
/// join_vars are visible to the outer query; branch-private vars are existential.
OrJoin {
    join_vars: Vec<String>,
    branches: Vec<Vec<WhereClause>>,
},
```

- [ ] **Step 4: Fix `rule_invocations()` — add Or/OrJoin arms**

In `WhereClause::rule_invocations()`, add before the closing `}`:

```rust
WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => branches
    .iter()
    .flat_map(|b| b.iter().flat_map(|c| c.rule_invocations()))
    .collect(),
```

- [ ] **Step 5: Fix `has_negated_invocation()` — add Or/OrJoin arm**

In `WhereClause::has_negated_invocation()`, extend the false arm:

```rust
WhereClause::Pattern(_)
| WhereClause::RuleInvocation { .. }
| WhereClause::Expr { .. }
| WhereClause::Or(_)
| WhereClause::OrJoin { .. } => false,
```

- [ ] **Step 6: Fix `collect_rule_invocations_recursive()` — add Or/OrJoin arm**

In `DatalogQuery::collect_rule_invocations_recursive()`, add inside the `for clause in clauses` loop's match, after `WhereClause::Expr`:

```rust
WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
    for branch in branches {
        result.extend(Self::collect_rule_invocations_recursive(branch));
    }
}
```

- [ ] **Step 7: Run types unit tests to verify they pass**

```bash
cargo test -p minigraf --lib query::datalog::types 2>&1 | tail -20
```

Expected: all `types` tests pass. Other modules may still fail to compile — that's expected.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add Or/OrJoin WhereClause variants with compile-error fixes"
```

---

## Task 2: Stratification — refactor from_rules with collect_clause_deps

**Files:**
- Modify: `src/query/datalog/stratification.rs`

- [ ] **Step 1: Write failing test for stratification with Or branch**

Add to `src/query/datalog/stratification.rs` in the `#[cfg(test)] mod tests` block (create one if it doesn't exist yet by checking the file):

First check the current test structure:
```bash
grep -n "mod tests" src/query/datalog/stratification.rs | head -5
```

Then add the test (either inside existing tests or create new block):

```rust
#[cfg(test)]
mod stratification_or_tests {
    use super::*;
    use crate::query::datalog::rules::RuleRegistry;
    use crate::query::datalog::types::{EdnValue, Pattern, Rule, WhereClause};

    #[test]
    fn test_from_rules_records_positive_dep_inside_or_branch() {
        // Rule: (p ?x) :- (or (active ?x) (pending ?x))
        // Should record positive edges: p → active, p → pending
        let mut registry = RuleRegistry::new();
        let rule = Rule::new(
            vec![EdnValue::Symbol("p".to_string()), EdnValue::Symbol("?x".to_string())],
            vec![WhereClause::Or(vec![
                vec![WhereClause::RuleInvocation {
                    predicate: "active".to_string(),
                    args: vec![EdnValue::Symbol("?x".to_string())],
                }],
                vec![WhereClause::RuleInvocation {
                    predicate: "pending".to_string(),
                    args: vec![EdnValue::Symbol("?x".to_string())],
                }],
            ])],
        );
        registry.register_rule_unchecked("p".to_string(), rule);
        let graph = DependencyGraph::from_rules(&registry);
        // Must stratify without error (positive-only: no negative cycle)
        let strata = graph.stratify();
        assert!(strata.is_ok(), "or with positive rule invocations should stratify");
    }

    #[test]
    fn test_from_rules_not_inside_or_branch_creates_negative_dep() {
        // (p ?x) :- (or (and (active ?x)) (and [?x :status :active]))
        // No rule invocations, so no negative deps — should stratify fine.
        let mut registry = RuleRegistry::new();
        let rule = Rule::new(
            vec![EdnValue::Symbol("p".to_string()), EdnValue::Symbol("?x".to_string())],
            vec![WhereClause::Or(vec![
                vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":status".to_string()),
                    EdnValue::Keyword(":active".to_string()),
                ))],
            ])],
        );
        registry.register_rule_unchecked("p".to_string(), rule);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_ok());
    }
}
```

- [ ] **Step 2: Run to see the compile error (from non-exhaustive match)**

```bash
cargo test -p minigraf --lib query::datalog::stratification 2>&1 | head -30
```

Expected: compile error — non-exhaustive patterns on `WhereClause`.

- [ ] **Step 3: Add `collect_clause_deps` helper and update `from_rules`**

In `src/query/datalog/stratification.rs`, replace the `from_rules` implementation. The full new implementation is:

```rust
fn collect_clause_deps(clause: &WhereClause, entry: &mut Vec<(String, bool)>) {
    match clause {
        WhereClause::RuleInvocation { predicate, .. } => {
            entry.push((predicate.clone(), false)); // positive edge
        }
        WhereClause::Not(inner) => {
            for inner_clause in inner {
                if let WhereClause::RuleInvocation { predicate, .. } = inner_clause {
                    entry.push((predicate.clone(), true)); // negative edge
                }
            }
        }
        WhereClause::NotJoin { clauses: inner, .. } => {
            for inner_clause in inner {
                if let WhereClause::RuleInvocation { predicate, .. } = inner_clause {
                    entry.push((predicate.clone(), true)); // negative edge
                }
            }
        }
        WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
            for branch in branches {
                for inner_clause in branch {
                    collect_clause_deps(inner_clause, entry); // recurse
                }
            }
        }
        WhereClause::Pattern(_) => {}
        WhereClause::Expr { .. } => {}
    }
}

impl DependencyGraph {
    pub fn from_rules(registry: &RuleRegistry) -> Self {
        let mut edges: HashMap<String, Vec<(String, bool)>> = HashMap::new();

        for (head_pred, rules) in registry.all_rules() {
            for rule in rules {
                let entry = edges.entry(head_pred.to_string()).or_default();
                for clause in &rule.body {
                    collect_clause_deps(clause, entry);
                }
            }
        }

        DependencyGraph { edges }
    }
    // ... rest of impl unchanged
```

Note: `collect_clause_deps` is a free function (not a method) placed before `impl DependencyGraph`.

- [ ] **Step 4: Run stratification tests**

```bash
cargo test -p minigraf --lib query::datalog::stratification 2>&1 | tail -20
```

Expected: all stratification tests pass (including existing ones and the new ones).

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/stratification.rs
git commit -m "feat(stratification): add collect_clause_deps, handle Or/OrJoin in dependency graph"
```

---

## Task 3: Matcher — add match_patterns_seeded

**Files:**
- Modify: `src/query/datalog/matcher.rs`

- [ ] **Step 1: Write failing unit test**

Add to `src/query/datalog/matcher.rs` inside `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_match_patterns_seeded_with_existing_bindings() {
    let storage = FactStorage::new();
    let alice_id = Uuid::new_v4();
    let bob_id = Uuid::new_v4();

    storage.transact(vec![
        (alice_id, ":person/age".to_string(), Value::Integer(30)),
        (bob_id, ":person/age".to_string(), Value::Integer(25)),
    ], None).unwrap();

    let matcher = PatternMatcher::new(storage);

    // Seed: ?e is already bound to alice_id
    let seed = vec![{
        let mut m = HashMap::new();
        m.insert("?e".to_string(), Value::Ref(alice_id));
        m
    }];

    // Pattern: [?e :person/age ?age]
    let pattern = Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":person/age".to_string()),
        EdnValue::Symbol("?age".to_string()),
    );

    let results = matcher.match_patterns_seeded(&[pattern], seed);
    // Should find age=30 for alice only (bob is not in seed)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("?age"), Some(&Value::Integer(30)));
}

#[test]
fn test_match_patterns_seeded_empty_seed_returns_empty() {
    let storage = FactStorage::new();
    let alice_id = Uuid::new_v4();
    storage.transact(vec![(alice_id, ":a".to_string(), Value::Integer(1))], None).unwrap();
    let matcher = PatternMatcher::new(storage);
    let pattern = Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":a".to_string()),
        EdnValue::Symbol("?v".to_string()),
    );
    let results = matcher.match_patterns_seeded(&[pattern], vec![]);
    assert!(results.is_empty());
}

#[test]
fn test_match_patterns_seeded_empty_patterns_returns_seed() {
    let storage = FactStorage::new();
    let matcher = PatternMatcher::new(storage);
    let seed = vec![{
        let mut m = HashMap::new();
        m.insert("?x".to_string(), Value::Integer(42));
        m
    }];
    let results = matcher.match_patterns_seeded(&[], seed.clone());
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get("?x"), Some(&Value::Integer(42)));
}
```

- [ ] **Step 2: Run to verify tests fail**

```bash
cargo test -p minigraf --lib query::datalog::matcher 2>&1 | tail -20
```

Expected: compile error — `match_patterns_seeded` not found.

- [ ] **Step 3: Implement `match_patterns_seeded`**

Add to `impl PatternMatcher` in `src/query/datalog/matcher.rs`, after `match_patterns`:

```rust
/// Match multiple patterns starting from existing seed bindings.
///
/// For each seed binding, extends it by matching all patterns in sequence.
/// Returns all extended bindings that satisfy every pattern.
/// If `seed` is empty, returns empty. If `patterns` is empty, returns `seed` unchanged.
pub(crate) fn match_patterns_seeded(
    &self,
    patterns: &[Pattern],
    seed: Vec<Bindings>,
) -> Vec<Bindings> {
    if seed.is_empty() {
        return vec![];
    }
    if patterns.is_empty() {
        return seed;
    }

    let mut results = seed;
    for pattern in patterns {
        results = self.join_with_pattern(results, pattern);
    }
    results
}
```

- [ ] **Step 4: Run matcher tests**

```bash
cargo test -p minigraf --lib query::datalog::matcher 2>&1 | tail -20
```

Expected: all matcher tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/matcher.rs
git commit -m "feat(matcher): add match_patterns_seeded for seeded branch evaluation"
```

---

## Task 4: Parser — parse or/or-join/and syntax with safety checks

**Files:**
- Modify: `src/query/datalog/parser.rs`

This is the largest task. It covers: parsing syntax, safety validation, and updating `outer_vars_from_clause` and `check_expr_safety_with_bound`.

- [ ] **Step 1: Write failing parse tests**

Add to `src/query/datalog/parser.rs` inside `#[cfg(test)] mod tests` (or a new submodule `or_tests`):

```rust
#[cfg(test)]
mod or_parse_tests {
    use super::*;

    #[test]
    fn test_parse_or_two_branches() {
        // (query [:find ?e :where [?e :a ?v] (or [?e :b ?v] [?e :c ?v])])
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :a ?v]
                              (or [?e :b ?v] [?e :c ?v])])"#,
        );
        assert!(cmd.is_ok(), "parse failed: {:?}", cmd.err());
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert_eq!(q.where_clauses.len(), 2);
            assert!(matches!(q.where_clauses[1], WhereClause::Or(_)));
        }
    }

    #[test]
    fn test_parse_or_with_and_grouping() {
        // (or (and [?e :a ?x] [?e :b ?y]) [?e :c ?z])
        // Here or introduces different new vars — this should fail safety
        // Let's use uniform new vars:
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or (and [?e :tag ?t]) [?e :label ?t])])"#,
        );
        assert!(cmd.is_ok(), "parse with and grouping failed: {:?}", cmd.err());
        if let Ok(DatalogCommand::Query(q)) = cmd {
            let or_clause = &q.where_clauses[1];
            if let WhereClause::Or(branches) = or_clause {
                assert_eq!(branches.len(), 2);
                // First branch has 1 clause (tag), second has 1 clause (label)
                assert_eq!(branches[0].len(), 1);
                assert_eq!(branches[1].len(), 1);
            } else {
                panic!("expected Or clause");
            }
        }
    }

    #[test]
    fn test_parse_or_join_basic() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or-join [?e]
                                [?e :tag :red]
                                [?e :tag :blue])])"#,
        );
        assert!(cmd.is_ok(), "or-join parse failed: {:?}", cmd.err());
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(q.where_clauses[1], WhereClause::OrJoin { .. }));
        }
    }

    #[test]
    fn test_parse_or_safety_mismatched_new_vars_is_error() {
        // Branch 1 introduces ?x, branch 2 introduces ?y → error
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or [?e :a ?x] [?e :b ?y])])"#,
        );
        assert!(cmd.is_err(), "should fail: branches introduce different vars");
        let err = cmd.unwrap_err();
        assert!(err.contains("same set of new variables"), "unexpected error: {}", err);
    }

    #[test]
    fn test_parse_or_join_unbound_join_var_is_error() {
        // ?unbound is not bound before or-join
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or-join [?unbound]
                                [?unbound :tag :red])])"#,
        );
        assert!(cmd.is_err(), "should fail: unbound join var");
        let err = cmd.unwrap_err();
        assert!(err.contains("not bound"), "unexpected error: {}", err);
    }
}
```

- [ ] **Step 2: Run to verify failures**

```bash
cargo test -p minigraf --lib query::datalog::parser::or_parse_tests 2>&1 | tail -30
```

Expected: compile errors (no `or`/`or-join` parse arms) or parse failures.

- [ ] **Step 3: Add `or`/`or-join`/`and` arms to `parse_list_as_where_clause`**

In `src/query/datalog/parser.rs`, inside `parse_list_as_where_clause`, add new arms for `"or"` and `"or-join"` before the final `EdnValue::Symbol(predicate)` arm:

```rust
EdnValue::Symbol(s) if s == "or" => {
    if list.len() < 2 {
        return Err("(or) requires at least one branch".to_string());
    }
    let mut branches: Vec<Vec<WhereClause>> = Vec::new();
    for item in &list[1..] {
        let branch = parse_or_branch(item)?;
        branches.push(branch);
    }
    Ok(WhereClause::Or(branches))
}
EdnValue::Symbol(s) if s == "or-join" => {
    if list.len() < 3 {
        return Err("(or-join) requires a join-vars vector and at least one branch".to_string());
    }
    let join_var_vec = match &list[1] {
        EdnValue::Vector(v) => v,
        _ => return Err("(or-join) first argument must be a vector of join variables".to_string()),
    };
    let join_vars: Vec<String> = join_var_vec
        .iter()
        .map(|v| match v {
            EdnValue::Symbol(s) if s.starts_with('?') => Ok(s.clone()),
            _ => Err(format!("(or-join) join variables must be logic variables, got {:?}", v)),
        })
        .collect::<Result<_, _>>()?;
    let mut branches: Vec<Vec<WhereClause>> = Vec::new();
    for item in &list[2..] {
        let branch = parse_or_branch(item)?;
        branches.push(branch);
    }
    Ok(WhereClause::OrJoin { join_vars, branches })
}
```

Add the `parse_or_branch` helper function (free function, placed near `parse_list_as_where_clause`):

```rust
/// Parse a single branch of an (or ...) or (or-join ...) clause.
///
/// A branch is either:
/// - A single clause: `[pattern]` or `(rule-invocation)` or `[(expr)]`
/// - A grouped list of clauses: `(and clause1 clause2 ...)`
fn parse_or_branch(item: &EdnValue) -> Result<Vec<WhereClause>, String> {
    match item {
        EdnValue::List(inner) if matches!(inner.first(), Some(EdnValue::Symbol(s)) if s == "and") => {
            // (and clause1 clause2 ...) — multi-clause branch
            if inner.len() < 2 {
                return Err("(and) inside or/or-join requires at least one clause".to_string());
            }
            let mut clauses = Vec::new();
            for clause_item in &inner[1..] {
                clauses.push(parse_or_branch_item(clause_item)?);
            }
            Ok(clauses)
        }
        other => {
            // Single-clause branch
            Ok(vec![parse_or_branch_item(other)?])
        }
    }
}

/// Parse a single clause item within an or branch.
fn parse_or_branch_item(item: &EdnValue) -> Result<WhereClause, String> {
    if let Some(vec) = item.as_vector() {
        if matches!(vec.first(), Some(EdnValue::List(_))) {
            parse_expr_clause(vec)
        } else {
            Ok(WhereClause::Pattern(Pattern::from_edn(vec)?))
        }
    } else if let Some(inner_list) = item.as_list() {
        // allow_not=true: or branches can contain not/not-join/or/or-join
        parse_list_as_where_clause(inner_list, true)
    } else {
        Err(format!("expected clause inside or branch, got {:?}", item))
    }
}
```

- [ ] **Step 4: Update `outer_vars_from_clause` to handle Or/OrJoin**

In `outer_vars_from_clause`, the current match has a catch-all. Add explicit arms for Or/OrJoin BEFORE the existing `WhereClause::Not(_) | WhereClause::NotJoin { .. }` arms:

```rust
WhereClause::Or(branches) => {
    if branches.is_empty() {
        return vec![];
    }
    // Variables available after `or` = intersection across all branches
    let branch_var_sets: Vec<std::collections::HashSet<String>> = branches
        .iter()
        .map(|branch| {
            branch
                .iter()
                .flat_map(outer_vars_from_clause)
                .collect::<std::collections::HashSet<_>>()
        })
        .collect();
    branch_var_sets[0]
        .iter()
        .filter(|v| branch_var_sets[1..].iter().all(|s| s.contains(*v)))
        .cloned()
        .collect()
}
WhereClause::OrJoin { join_vars, .. } => join_vars.clone(),
```

- [ ] **Step 5: Update `check_expr_safety_with_bound` to handle Or/OrJoin**

In `check_expr_safety_with_bound`, the `other` arm currently calls `outer_vars_from_clause(other)`. Add explicit arms for `Or` and `OrJoin` before the `other` arm:

```rust
WhereClause::Or(branches) => {
    if !branches.is_empty() {
        // Check each branch with a fresh copy of bound; collect new vars per branch
        let mut branch_new_var_sets: Vec<std::collections::HashSet<String>> = Vec::new();
        for branch in branches {
            let mut branch_bound = bound.clone();
            check_expr_safety_with_bound(branch, &mut branch_bound)?;
            let new_vars: std::collections::HashSet<String> =
                branch_bound.difference(bound).cloned().collect();
            branch_new_var_sets.push(new_vars);
        }
        // All branches must introduce the same set of new variables
        if branch_new_var_sets
            .windows(2)
            .any(|w| w[0] != w[1])
        {
            return Err(
                "all branches of (or ...) must introduce the same set of new variables"
                    .to_string(),
            );
        }
        // Add the uniform new variables to outer bound
        if let Some(new_vars) = branch_new_var_sets.first() {
            for var in new_vars {
                bound.insert(var.clone());
            }
        }
    }
}
WhereClause::OrJoin { join_vars, branches } => {
    // Safety: all join_vars must be pre-bound
    for var in join_vars {
        if !var.starts_with("?_") && !bound.contains(var) {
            return Err(format!(
                "join variable {} in (or-join ...) is not bound by any earlier clause",
                var
            ));
        }
    }
    // Check safety within each branch (branches see outer bound vars)
    for branch in branches {
        let mut branch_bound = bound.clone();
        check_expr_safety_with_bound(branch, &mut branch_bound)?;
    }
    // or-join does NOT add new variables to the outer bound
}
```

- [ ] **Step 6: Run parser tests**

```bash
cargo test -p minigraf --lib query::datalog::parser 2>&1 | tail -30
```

Expected: all parser tests pass (including the new `or_parse_tests`).

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): parse or/or-join/and syntax with safety checks"
```

---

## Task 5: Evaluator — expose rule_invocation_to_pattern and wire mixed-rules loop

**Files:**
- Modify: `src/query/datalog/evaluator.rs`

This task has two parts: (a) make `rule_invocation_to_pattern` accessible from `executor.rs`, and (b) wire `apply_or_clauses` into the mixed-rules loop and extend `has_not`. Task 5b depends on Task 6 (which adds `apply_or_clauses`), so do 5a now and 5b after Task 6.

**Part 5a: Change visibility of `rule_invocation_to_pattern`**

- [ ] **Step 1: Change `pub(super)` to `pub(crate)` on `rule_invocation_to_pattern`**

In `src/query/datalog/evaluator.rs`, line 376:

```rust
// Before:
pub(super) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {

// After:
pub(crate) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
```

- [ ] **Step 2: Run all tests to verify nothing broke**

```bash
cargo test -p minigraf --lib 2>&1 | tail -20
```

Expected: all lib tests still pass (the change is purely visibility, no behavior change).

- [ ] **Step 3: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "refactor(evaluator): expose rule_invocation_to_pattern as pub(crate)"
```

---

## Task 6: Executor — add evaluate_branch and apply_or_clauses, wire into execute_query

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write failing unit tests in executor.rs**

Add to `src/query/datalog/executor.rs` inside `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_apply_or_clauses_union_from_two_branches() {
    use crate::query::datalog::types::{DatalogQuery, FindSpec};
    let storage = FactStorage::new();
    let e1 = Uuid::new_v4();
    let e2 = Uuid::new_v4();
    storage.transact(vec![
        (e1, ":tag".to_string(), Value::Keyword(":red".to_string())),
        (e2, ":tag".to_string(), Value::Keyword(":blue".to_string())),
    ], None).unwrap();

    let executor = DatalogExecutor::new(storage.clone());
    let result = executor.execute(
        r#"(query [:find ?e
                   :where [?e :tag ?_t]
                          (or [?e :tag :red] [?e :tag :blue])])"#,
    ).unwrap();
    match result {
        crate::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 2, "both entities should match via or");
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn test_apply_or_clauses_deduplication() {
    // Both branches match the same entity — result should appear once
    let storage = FactStorage::new();
    let e1 = Uuid::new_v4();
    storage.transact(vec![
        (e1, ":tag".to_string(), Value::Keyword(":red".to_string())),
        (e1, ":label".to_string(), Value::Keyword(":primary".to_string())),
    ], None).unwrap();

    let executor = DatalogExecutor::new(storage.clone());
    let result = executor.execute(
        r#"(query [:find ?e
                   :where [?e :tag ?_t]
                          (or [?e :tag :red] [?e :label :primary])])"#,
    ).unwrap();
    match result {
        crate::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "one entity matched by both branches → deduplicated");
        }
        _ => panic!("expected QueryResults"),
    }
}
```

- [ ] **Step 2: Run to verify the tests fail**

```bash
cargo test -p minigraf --lib query::datalog::executor 2>&1 | tail -30
```

Expected: compile or test failures (or/or-join not yet handled in executor).

- [ ] **Step 3: Add `evaluate_branch` function**

Add before the `type Binding = ...` line (around line 785) in `src/query/datalog/executor.rs`:

```rust
/// Evaluate a single branch of an `or`/`or-join` against incoming bindings.
///
/// Processing order (mirrors top-level execute_query order):
/// 1. Pattern/RuleInvocation → match_patterns_seeded
/// 2. Nested Or/OrJoin → apply_or_clauses
/// 3. Not/NotJoin → post-filter
/// 4. Expr → apply_expr_clauses
///
/// `storage` must already contain any rule-derived facts (caller's responsibility).
pub(crate) fn evaluate_branch(
    branch: &[WhereClause],
    incoming: Vec<Binding>,
    storage: &FactStorage,
    rules: &crate::query::datalog::rules::RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>> {
    use crate::query::datalog::evaluator::{
        evaluate_not_join, rule_invocation_to_pattern,
    };
    use crate::query::datalog::matcher::PatternMatcher;

    if incoming.is_empty() {
        return Ok(vec![]);
    }

    // Step 1: Collect Pattern and RuleInvocation clauses
    let patterns: Vec<Pattern> = branch
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(p.clone()),
            WhereClause::RuleInvocation { predicate, args } => {
                rule_invocation_to_pattern(predicate, args).ok()
            }
            _ => None,
        })
        .collect();

    let matcher = PatternMatcher::new(storage.clone());
    let bindings = if patterns.is_empty() {
        incoming
    } else {
        matcher.match_patterns_seeded(&patterns, incoming)
    };

    if bindings.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: Nested Or/OrJoin
    let bindings = apply_or_clauses(branch, bindings, storage, rules, as_of.clone(), valid_at.clone())?;

    if bindings.is_empty() {
        return Ok(vec![]);
    }

    // Step 3: Not/NotJoin post-filter
    let not_clauses: Vec<&Vec<WhereClause>> = branch
        .iter()
        .filter_map(|c| match c {
            WhereClause::Not(inner) => Some(inner),
            _ => None,
        })
        .collect();

    let not_join_clauses: Vec<(Vec<String>, Vec<WhereClause>)> = branch
        .iter()
        .filter_map(|c| match c {
            WhereClause::NotJoin { join_vars, clauses } => {
                Some((join_vars.clone(), clauses.clone()))
            }
            _ => None,
        })
        .collect();

    let bindings = if not_clauses.is_empty() && not_join_clauses.is_empty() {
        bindings
    } else {
        bindings
            .into_iter()
            .filter(|binding| {
                for not_body in &not_clauses {
                    if not_body_matches(not_body, binding, storage) {
                        return false;
                    }
                }
                for (join_vars, nj_clauses) in &not_join_clauses {
                    if evaluate_not_join(join_vars, nj_clauses, binding, storage) {
                        return false;
                    }
                }
                true
            })
            .collect()
    };

    // Step 4: Expr clauses
    let bindings = apply_expr_clauses(bindings, branch);

    Ok(bindings)
}
```

- [ ] **Step 4: Add `apply_or_clauses` function**

Add immediately after `evaluate_branch` in `executor.rs`:

```rust
/// Apply all Or/OrJoin clauses from `clauses` to `bindings` in sequence.
///
/// Non-Or/OrJoin clauses are ignored (they are handled elsewhere).
/// For `Or`: union results from all branches (deduplicated by full binding map).
/// For `OrJoin`: union results, then project out branch-private variables.
pub(crate) fn apply_or_clauses(
    clauses: &[WhereClause],
    mut bindings: Vec<Binding>,
    storage: &FactStorage,
    rules: &crate::query::datalog::rules::RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>> {
    for clause in clauses {
        match clause {
            WhereClause::Or(branches) => {
                let mut result: Vec<Binding> = Vec::new();
                for branch in branches {
                    let branch_result = evaluate_branch(
                        branch,
                        bindings.clone(),
                        storage,
                        rules,
                        as_of.clone(),
                        valid_at.clone(),
                    )?;
                    for b in branch_result {
                        if !result.contains(&b) {
                            result.push(b);
                        }
                    }
                }
                bindings = result;
            }
            WhereClause::OrJoin { join_vars, branches } => {
                // Compute outer_keys: all variable names in the incoming bindings
                // (join_vars are a subset of these, since safety check ensures they are pre-bound)
                let outer_keys: std::collections::HashSet<String> = bindings
                    .iter()
                    .flat_map(|b| b.keys().cloned())
                    .collect();

                let mut result: Vec<Binding> = Vec::new();
                for branch in branches {
                    let branch_result = evaluate_branch(
                        branch,
                        bindings.clone(),
                        storage,
                        rules,
                        as_of.clone(),
                        valid_at.clone(),
                    )?;
                    for mut b in branch_result {
                        // Drop partial bindings (missing any join_var)
                        if !join_vars.iter().all(|v| b.contains_key(v)) {
                            continue;
                        }
                        // Project: keep only outer_keys (strips branch-private vars)
                        b.retain(|k, _| outer_keys.contains(k));
                        if !result.contains(&b) {
                            result.push(b);
                        }
                    }
                }
                bindings = result;
            }
            _ => {} // Other clause types handled elsewhere
        }
    }
    Ok(bindings)
}
```

- [ ] **Step 5: Wire `apply_or_clauses` into `execute_query`**

In `execute_query`, after the `let bindings = matcher.match_patterns(...)` call and before the `not_clauses` collection, insert:

```rust
// Apply Or/OrJoin clauses (post-pass: after pattern matching, before not/expr)
let rules_guard = self.rules.read().unwrap();
let bindings = apply_or_clauses(
    &query.where_clauses,
    bindings,
    &filtered_storage,
    &rules_guard,
    query.as_of.clone(),
    query.valid_at.clone(),
)?;
drop(rules_guard);
```

- [ ] **Step 6: Wire `apply_or_clauses` into `execute_query_with_rules`**

In `execute_query_with_rules`, after `let bindings = matcher.match_patterns(&all_patterns)` and before the `not_clauses` collection:

```rust
// Apply Or/OrJoin clauses against derived_storage (rules already evaluated)
let rules_guard = self.rules.read().unwrap();
let bindings = apply_or_clauses(
    &query.where_clauses,
    bindings,
    &derived_storage,
    &rules_guard,
    query.as_of.clone(),
    query.valid_at.clone(),
)?;
drop(rules_guard);
```

- [ ] **Step 7: Run executor unit tests**

```bash
cargo test -p minigraf --lib query::datalog::executor 2>&1 | tail -30
```

Expected: new executor tests pass; all existing executor tests still pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): add evaluate_branch, apply_or_clauses; wire into execute_query paths"
```

---

## Task 7: Evaluator — extend has_not and mixed-rules loop

**Files:**
- Modify: `src/query/datalog/evaluator.rs`

This is Part 5b (deferred from Task 5). `apply_or_clauses` is now available from Task 6.

- [ ] **Step 1: Write failing test — rule with or in body**

Add to `src/query/datalog/evaluator.rs` unit tests:

```rust
#[test]
fn test_stratified_evaluator_routes_or_rule_to_mixed_path() {
    // Rule with or in body must route to mixed_rules (not RecursiveEvaluator)
    // and produce correct results.
    let storage = FactStorage::new();
    let e1 = Uuid::new_v4();
    let e2 = Uuid::new_v4();
    storage.transact(vec![
        (e1, ":a".to_string(), Value::Boolean(true)),
        (e2, ":b".to_string(), Value::Boolean(true)),
    ], None).unwrap();

    let mut registry = RuleRegistry::new();
    // Rule: (p ?x) :- (or [?x :a true] [?x :b true])
    let rule = Rule::new(
        vec![
            EdnValue::Symbol("p".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![WhereClause::Or(vec![
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":a".to_string()),
                EdnValue::Boolean(true),
            ))],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":b".to_string()),
                EdnValue::Boolean(true),
            ))],
        ])],
    );
    registry.register_rule_unchecked("p".to_string(), rule);
    let rules = Arc::new(RwLock::new(registry));
    let evaluator = StratifiedEvaluator::new(storage, rules, 1000);
    let derived = evaluator.evaluate(&["p".to_string()]).unwrap();
    let facts = derived.get_asserted_facts().unwrap();
    let p_facts: Vec<_> = facts.iter().filter(|f| f.attribute == ":p").collect();
    assert_eq!(p_facts.len(), 2, "both e1 and e2 should be derived as :p");
}
```

- [ ] **Step 2: Run to verify the test fails**

```bash
cargo test -p minigraf --lib query::datalog::evaluator 2>&1 | tail -30
```

Expected: test fails (rule with Or routes to `positive_rules`, not `mixed_rules`, causing incorrect results or panic).

- [ ] **Step 3: Extend the `has_not` inline closure in `StratifiedEvaluator::evaluate`**

In `src/query/datalog/evaluator.rs`, at lines 584–587, replace:

```rust
let has_not = rule
    .body
    .iter()
    .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }));
```

With:

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

- [ ] **Step 4: Add `Or`/`OrJoin => None` arm to `positive_patterns` filter_map**

In the mixed-rules loop (around line 624–645), in the `positive_patterns` filter_map, add before the closing `_ => None`:

```rust
WhereClause::Or(_) | WhereClause::OrJoin { .. } => None,
```

(These are handled by `apply_or_clauses`, not extracted as patterns.)

- [ ] **Step 5: Wire `apply_or_clauses` into the mixed-rules loop**

Import `apply_or_clauses` alongside the existing import:

```rust
use crate::query::datalog::executor::{eval_expr, is_truthy, apply_or_clauses};
```

In the mixed-rules loop, replace:

```rust
let raw_candidates = matcher.match_patterns(&positive_patterns);

// Apply top-level Expr clauses to filter/extend candidates
let candidates = apply_expr_clauses_in_evaluator(raw_candidates, &body_expr_clauses);
```

With:

```rust
let raw_candidates = matcher.match_patterns(&positive_patterns);

// Apply Or/OrJoin clauses before Expr (mirrors top-level execute_query order)
let registry_guard = self.rules.read().unwrap();
let or_expanded = apply_or_clauses(
    &rule.body,
    raw_candidates,
    &accumulated,
    &registry_guard,
    None, // no temporal filtering in rule evaluation
    None,
)?;
drop(registry_guard);

// Apply top-level Expr clauses to filter/extend candidates
let candidates = apply_expr_clauses_in_evaluator(or_expanded, &body_expr_clauses);
```

Note: the `?` operator propagates errors — the containing `for` loop needs to return a `Result`. Check if the mixed-rules block already handles `Result`s (it uses `?` via the outer `evaluate` function's `Result<FactStorage>` return). If not, you may need to wrap the or_expanded call in an `anyhow::Result` propagation — adjust as needed based on what the compiler says.

- [ ] **Step 6: Run evaluator tests**

```bash
cargo test -p minigraf --lib query::datalog::evaluator 2>&1 | tail -30
```

Expected: all evaluator tests pass including the new one.

- [ ] **Step 7: Run full lib test suite**

```bash
cargo test -p minigraf --lib 2>&1 | tail -20
```

Expected: all lib tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "feat(evaluator): route or/or-join rules to mixed_rules path, wire apply_or_clauses"
```

---

## Task 8: Integration tests — core `or` tests

**Files:**
- Create: `tests/disjunction_test.rs`

- [ ] **Step 1: Create the test file with helper**

```rust
//! Integration tests for Phase 7.3: or / or-join disjunction.

use minigraf::{Minigraf, OpenOptions, QueryResult};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

fn result_count(r: &QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    }
}

fn result_values(r: &QueryResult) -> Vec<Vec<minigraf::Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results.clone(),
        _ => panic!("expected QueryResults"),
    }
}
```

- [ ] **Step 2: Write test — two-branch `or` unions results**

```rust
/// (1) Two-branch or: entities with :tag :red OR :tag :blue both appear.
#[test]
fn test_or_union_two_branches() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue] [:e3 :tag :green]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 2, "red and blue entities must both appear");
}
```

- [ ] **Step 3: Run to verify it passes**

```bash
cargo test -p minigraf --test disjunction_test test_or_union_two_branches 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 4: Write test — single-branch `or` degenerates to filter**

```rust
/// (2) Single-branch or behaves like an inline pattern filter.
#[test]
fn test_or_single_branch_acts_as_filter() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red])])"#).unwrap();
    assert_eq!(result_count(&r), 1);
}
```

- [ ] **Step 5: Write test — only one branch matches**

```rust
/// (3) Only the first branch matches; second branch finds nothing.
#[test]
fn test_or_only_first_branch_matches() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 1);
}
```

- [ ] **Step 6: Write test — deduplication when both branches match same entity**

```rust
/// (4) Both branches match the same entity — result must appear exactly once.
#[test]
fn test_or_deduplication_both_branches_match() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true] [:e1 :b true]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :a ?_a]
                       (or [?e :a true] [?e :b true])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "deduplicated: e1 must appear once");
}
```

- [ ] **Step 7: Write test — `or` with `not` inside a branch**

```rust
/// (5) Or branch containing not: e1 has :a and NOT :banned; e2 has :b.
#[test]
fn test_or_with_not_inside_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true] [:e2 :b true] [:e1 :banned true]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :a ?_a]
                       (or (and [?e :a true]
                               (not [?e :banned true]))
                           [?e :b true])])"#).unwrap();
    // e1 has :a AND :banned → excluded by not in branch 1; doesn't have :b → excluded from branch 2 too
    // e2 has :b → passes branch 2; but does e2 have :a? No — it won't even pass the outer [?e :a ?_a] pattern
    // Actually outer pattern [?e :a ?_a] requires :a. Let's fix: use no outer constraint
    // Re-think: without outer constraint, or matches e1 (branch 1 fails due to banned, branch 2 fails: no :b on e1)
    // and e2 (branch 1: no :a; branch 2: has :b). So only e2 passes.
    assert_eq!(result_count(&r), 0, "e1 has :a outer but is banned in branch 1 and no :b; e2 has :b but fails outer :a");
    // This test is tricky. Let's fix the data setup:
}
```

Wait — let me rethink this test properly:

```rust
/// (5) Or with not inside branch.
#[test]
fn test_or_with_not_inside_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :status :active]
                              [:e2 :status :active]
                              [:e1 :banned true]])"#).unwrap();
    // or: (e has :status :active AND NOT :banned) OR (e has :status :active AND some other condition)
    // Simpler: find all active that are NOT banned (branch 1) OR that have :vip (branch 2)
    db.execute(r#"(transact [[:e3 :vip true] [:e3 :status :active]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :status :active]
                       (or (and (not [?e :banned true]))
                           [?e :vip true])])"#).unwrap();
    // e1: active + banned → branch 1 fails (not [e1 :banned] = false since e1 IS banned); branch 2 fails (no :vip)
    // e2: active + not banned → branch 1 passes; result: e2
    // e3: active + vip → branch 1 passes (not banned) AND branch 2 passes → deduplicated once
    // Result: e2 and e3
    assert_eq!(result_count(&r), 2);
}
```

- [ ] **Step 8: Write test — nested `or` inside `or`**

```rust
/// (6) Nested or inside or.
#[test]
fn test_or_nested() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true] [:e2 :b true] [:e3 :c true]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :_attr ?_v]
                       (or (or [?e :a true] [?e :b true])
                           [?e :c true])])"#).unwrap();
    // Matches e1 (via :a), e2 (via :b), e3 (via :c)
    // But outer [?e :_attr ?_v] is a wildcard — doesn't exist. Let's use a real attribute.
    assert_eq!(result_count(&r), 3);
}
```

Actually we should use a real setup without wildcards in attribute position (wildcards are for values with `?_`). Let me revise:

```rust
/// (6) Nested or inside or.
#[test]
fn test_or_nested() {
    let db = db();
    db.execute(r#"(transact [[:e1 :kind :a] [:e2 :kind :b] [:e3 :kind :c]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :kind ?_k]
                       (or (or [?e :kind :a] [?e :kind :b])
                           [?e :kind :c])])"#).unwrap();
    assert_eq!(result_count(&r), 3);
}
```

- [ ] **Step 9: Run all disjunction tests so far**

```bash
cargo test -p minigraf --test disjunction_test 2>&1 | tail -30
```

Expected: all pass.

- [ ] **Step 10: Commit**

```bash
git add tests/disjunction_test.rs
git commit -m "test(disjunction): core or tests — union, filter, dedup, not inside branch, nested"
```

---

## Task 9: Integration tests — `or-join`, safety errors, rules, bi-temporal, stratification

**Files:**
- Modify: `tests/disjunction_test.rs`

- [ ] **Step 1: Write `or-join` tests**

Add to `tests/disjunction_test.rs`:

```rust
// ── or-join tests ─────────────────────────────────────────────────────────────

/// (7) Basic or-join: branch-private vars stripped from output.
#[test]
fn test_or_join_strips_branch_private_vars() {
    let db = db();
    db.execute(r#"(transact [[:e1 :name "Alice"] [:e1 :tag :red]
                              [:e2 :name "Bob"]  [:e2 :badge :gold]])"#).unwrap();
    // or-join [?e]: branch 1 checks :tag :red (private ?t); branch 2 checks :badge :gold (private ?b)
    // Only ?e is in the result.
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?_n]
                       (or-join [?e]
                         [?e :tag :red]
                         [?e :badge :gold])])"#).unwrap();
    assert_eq!(result_count(&r), 2, "both Alice (red tag) and Bob (gold badge) should match");
}

/// (8) or-join with multiple join vars.
#[test]
fn test_or_join_multiple_join_vars() {
    let db = db();
    db.execute(r#"(transact [[:e1 :dept :eng] [:e1 :level :senior]
                              [:e2 :dept :eng] [:e2 :role :lead]
                              [:e3 :dept :hr]])"#).unwrap();
    // or-join [?e ?dept]: branch 1 has :level :senior; branch 2 has :role :lead
    // Both e1 and e2 are in :eng; e3 is not in either branch
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :dept ?dept]
                       (or-join [?e ?dept]
                         (and [?e :dept ?dept] [?e :level :senior])
                         (and [?e :dept ?dept] [?e :role :lead]))])"#).unwrap();
    assert_eq!(result_count(&r), 2);
}

/// (9) or-join where branches introduce different private vars — result has only join vars.
#[test]
fn test_or_join_different_private_vars_per_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red] [:e2 :shape :circle]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :color ?_c]
                       (or-join [?e]
                         (and [?e :color ?private_color])
                         (and [?e :shape ?private_shape]))])"#).unwrap();
    // e1 matches branch 1 (has :color), e2 doesn't have :color so fails outer [?e :color ?_c]
    // Only e1 should appear
    assert_eq!(result_count(&r), 1);
}
```

- [ ] **Step 2: Write safety error tests**

```rust
// ── Safety / parse error tests ─────────────────────────────────────────────────

/// (10) or branches with mismatched new variables → parse error.
#[test]
fn test_or_safety_mismatched_vars_error() {
    let db = db();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or [?e :a ?x] [?e :b ?y])])"#);
    assert!(r.is_err());
    let err = r.unwrap_err().to_string();
    assert!(err.contains("same set of new variables"), "error: {}", err);
}

/// (11) or-join with unbound join var → parse error.
#[test]
fn test_or_join_unbound_join_var_error() {
    let db = db();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or-join [?x] [?x :tag :red])])"#);
    assert!(r.is_err());
    let err = r.unwrap_err().to_string();
    assert!(err.contains("not bound"), "error: {}", err);
}
```

- [ ] **Step 3: Write rule tests**

```rust
// ── Rule tests ─────────────────────────────────────────────────────────────────

/// (12) Rule with or in body routes to mixed_rules and produces correct results.
#[test]
fn test_rule_with_or_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :score 90] [:e2 :score 70] [:e3 :score 50]])"#).unwrap();
    // Rule: (high-or-mid ?e) :- (or [?e :score ?s] [(>= ?s 70)])
    // Wait — we can't use or with Expr like that as-is. Let's use pure pattern branches:
    db.execute(r#"(transact [[:e1 :tier :gold] [:e2 :tier :silver]])"#).unwrap();
    db.execute(r#"(rule [(valuable ?e) (or [?e :tier :gold] [?e :tier :silver])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (valuable ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 2, "e1 (gold) and e2 (silver) are valuable");
}

/// (13) or-join in a rule body.
#[test]
fn test_rule_with_or_join_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red] [:e2 :shape :circle] [:e3 :size :large]])"#).unwrap();
    db.execute(r#"(rule [(interesting ?e)
                         [?e :color ?_c]
                         (or-join [?e]
                           [?e :color :red]
                           [?e :color :blue])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (interesting ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 1, "only e1 (red) is interesting");
}
```

- [ ] **Step 4: Write bi-temporal test**

```rust
// ── Bi-temporal tests ──────────────────────────────────────────────────────────

/// (14) or with :as-of — temporal filter applies across branches.
#[test]
fn test_or_with_as_of() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red]])"#).unwrap();
    // tx_count is now 1; add blue at tx 2
    db.execute(r#"(transact [[:e2 :tag :blue]])"#).unwrap();
    // As-of tx 1 — only e1 (red) was present
    let r = db.execute(r#"
        (query [:find ?e
                :as-of 1
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "only e1 (red) was present at tx 1");
}
```

- [ ] **Step 5: Write stratification test**

```rust
// ── Stratification tests ───────────────────────────────────────────────────────

/// (15) or containing not that would form a negative cycle → error at rule registration.
#[test]
fn test_or_with_not_cycle_rejected() {
    let db = db();
    // Define p in terms of not(q) and q in terms of not(p) → negative cycle
    db.execute(r#"(rule [(p ?x) (or (and (not (q ?x))) [?x :a true])])"#).unwrap();
    let result = db.execute(r#"(rule [(q ?x) (or (and (not (p ?x))) [?x :b true])])"#);
    assert!(result.is_err(), "negative cycle through or should be rejected");
}

/// (16) or containing RuleInvocation → positive dep edge, correct stratification.
#[test]
fn test_or_with_rule_invocation_positive_dep() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true]])"#).unwrap();
    db.execute(r#"(rule [(base ?x) [?x :a true]])"#).unwrap();
    db.execute(r#"(rule [(derived ?x) (or (base ?x) [?x :b true])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (derived ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 1, "e1 matches via base → derived");
}
```

- [ ] **Step 6: Run all disjunction tests**

```bash
cargo test -p minigraf --test disjunction_test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 7: Run full test suite**

```bash
cargo test -p minigraf 2>&1 | tail -30
```

Expected: all tests pass (should be ≥527 + new tests).

- [ ] **Step 8: Commit**

```bash
git add tests/disjunction_test.rs
git commit -m "test(disjunction): or-join, safety errors, rules, bi-temporal, stratification tests"
```

---

## Task 10: Docs, CHANGELOG, ROADMAP, and wiki

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`
- Modify: `.wiki/Datalog-Reference.md`

- [ ] **Step 1: Update CHANGELOG.md**

Add a new version entry at the top of `CHANGELOG.md`:

```markdown
## [0.13.0] — 2026-03-26

### Added
- **Disjunction (`or` / `or-join`)**: queries and rule bodies can now use `(or branch1 branch2 ...)` and `(or-join [?v...] branch1 branch2 ...)` where-clauses. Branches support all other clause types including `not`, `not-join`, `Expr`, and nested `or`/`or-join`. `(and ...)` groups multiple clauses into a single branch.
- `match_patterns_seeded` on `PatternMatcher` for seeded branch evaluation.
- `evaluate_branch` and `apply_or_clauses` as `pub(crate)` helpers in `executor.rs`.

### Technical
- `WhereClause` enum gains `Or(Vec<Vec<WhereClause>>)` and `OrJoin { join_vars, branches }` variants.
- `DependencyGraph::from_rules` refactored with recursive `collect_clause_deps` helper; `Or`/`OrJoin` branches contribute positive dependency edges.
- Rules with `or`/`or-join` in their bodies route to the `mixed_rules` path in `StratifiedEvaluator`.
```

- [ ] **Step 2: Update ROADMAP.md status line**

Find the status line (around line 1502) and update:

```markdown
Phase 7.3 Complete — Phase 7.4 Next (...)
```

(Check what Phase 7.4 is in the roadmap and use the appropriate label.)

- [ ] **Step 3: Update `.wiki/Datalog-Reference.md`**

Add an `## Disjunction` section after `## Negation`:

```markdown
## Disjunction

### `or` — match any branch

```datalog
;; Succeeds if any branch matches.
(or clause1 clause2 ...)

;; Use (and ...) to group multiple clauses into one branch:
(or (and clause1 clause2 ...) clause3 ...)
```

All branches must introduce the same set of new variables.

### `or-join` — existentially-quantified disjunction

```datalog
;; join_vars are shared with the outer query.
;; Variables inside branches but not in join_vars are private (existential).
(or-join [?v1 ?v2] branch1 branch2 ...)
(or-join [?v1] (and [?v1 :a ?priv1]) (and [?v1 :b ?priv2]))
```

All `join_vars` must be bound by preceding clauses. Branch-private variables do not appear in query results.

### Branch contents

Each branch (or a single clause) may contain any `WhereClause`: `Pattern`, `RuleInvocation`, `not`, `not-join`, `Expr`, and nested `or`/`or-join`.

### Safety

- `or`: all branches must introduce the same set of new variable names. Mismatched sets → parse error.
- `or-join`: all `join_vars` must be bound by a preceding clause → parse error if not.
```

- [ ] **Step 4: Commit docs**

```bash
git add CHANGELOG.md ROADMAP.md .wiki/Datalog-Reference.md
git commit -m "docs: update CHANGELOG, ROADMAP, and Datalog-Reference for Phase 7.3 or/or-join"
```

---

## Final Verification

- [ ] **Run full test suite one last time**

```bash
cargo test -p minigraf 2>&1 | tail -30
```

Expected: all tests pass with no failures or warnings.

- [ ] **Update version in Cargo.toml**

Change `version = "0.12.0"` to `version = "0.13.0"` in `Cargo.toml`.

```bash
git add Cargo.toml
git commit -m "chore: bump version to 0.13.0 (Phase 7.3 disjunction)"
```
