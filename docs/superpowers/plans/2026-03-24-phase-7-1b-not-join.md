# Phase 7.1b: `not-join` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `(not-join [?join-vars…] clauses…)` to the Datalog engine — existentially-quantified negation that explicitly declares which variables are shared with the outer query context.

**Architecture:** `not-join` extends the existing `WhereClause::Not` infrastructure. A new `WhereClause::NotJoin { join_vars, clauses }` variant is added; the parser, stratification graph, `StratifiedEvaluator`, and both executor post-filters are updated to handle it. The key semantic difference from `not`: only the explicitly listed join variables are substituted from the outer binding; all other variables in the body are existentially quantified (treated as fresh/unbound and matched against the fact store).

**Tech Stack:** Rust, `anyhow`, existing `PatternMatcher`, `FactStorage`, `StratifiedEvaluator`, `substitute_pattern` (already `pub` in `evaluator.rs`).

---

## Semantic Reference

```
;; not: ALL variables in body must be pre-bound by outer clauses.
;;      No fresh variables allowed.
(query [:find ?e
        :where [?e :name ?n]
               (not [?e :banned true])])   ;; ?e must be bound — OK
               ;; (not [?e :tag ?tag]) would ERROR — ?tag unbound

;; not-join: only join_vars must be pre-bound.
;;           All other vars in body are existentially quantified.
(query [:find ?e
        :where [?e :name ?n]
               (not-join [?e]             ;; only ?e shared from outer
                         [?e :has-tag ?tag]   ;; ?tag is local/fresh
                         [?tag :is-bad true])])
;; Semantics: reject ?e for which ∃?tag s.t. (?e :has-tag ?tag) ∧ (?tag :is-bad true)
```

Safety rule: every variable in `join_vars` must be bound by an outer clause. Variables appearing only inside the `not-join` body (not in `join_vars`) are unconstrained — no error.

Evaluation rule: substitute only `join_vars` into the body patterns, then run `PatternMatcher::match_patterns` on the partially-substituted patterns. If any matches exist, the body is satisfiable → the outer binding is **rejected**.

Nesting rule: `(not-join ...)` cannot appear inside `(not ...)` or another `(not-join ...)`.

Stratification rule: `not-join` creates the same negative dependency edges as `not` — identical treatment in `DependencyGraph::from_rules`.

---

## File Map

| File | Change |
|------|--------|
| `src/query/datalog/types.rs` | Add `WhereClause::NotJoin { join_vars, clauses }` variant; update all helpers |
| `src/query/datalog/stratification.rs` | Traverse `NotJoin` bodies in `DependencyGraph::from_rules` |
| `src/query/datalog/parser.rs` | Parse `(not-join [?v…] clauses…)`; safety check for join vars |
| `src/query/datalog/evaluator.rs` | `StratifiedEvaluator` handles `NotJoin` in rule bodies; extract `evaluate_not_join` helper |
| `src/query/datalog/executor.rs` | Both not-post-filters handle `NotJoin` in query bodies |
| `tests/not_join_test.rs` | 10 integration tests |

---

## Task 1: Add `WhereClause::NotJoin` and update type helpers

**Files:**
- Modify: `src/query/datalog/types.rs`

### What to implement

Add a new variant to `WhereClause` and update all the helper methods that currently handle `Pattern`, `RuleInvocation`, and `Not` to also handle `NotJoin`.

- [ ] **Step 1.1: Write failing unit tests** for the new variant and helpers

Add inside the existing `#[cfg(test)]` block at the bottom of `types.rs`:

```rust
#[test]
fn test_where_clause_not_join_variant_exists() {
    let nj = WhereClause::NotJoin {
        join_vars: vec!["?e".to_string()],
        clauses: vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":tag".to_string()),
            EdnValue::Symbol("?tag".to_string()),
        ))],
    };
    assert!(matches!(nj, WhereClause::NotJoin { .. }));
}

#[test]
fn test_rule_invocations_recurses_into_not_join() {
    let nj = WhereClause::NotJoin {
        join_vars: vec!["?e".to_string()],
        clauses: vec![WhereClause::RuleInvocation {
            predicate: "blocked".to_string(),
            args: vec![EdnValue::Symbol("?e".to_string())],
        }],
    };
    let invocations = nj.rule_invocations();
    assert_eq!(invocations, vec!["blocked"]);
}

#[test]
fn test_has_negated_invocation_true_for_not_join_with_rule_invocation() {
    let nj = WhereClause::NotJoin {
        join_vars: vec!["?e".to_string()],
        clauses: vec![WhereClause::RuleInvocation {
            predicate: "blocked".to_string(),
            args: vec![EdnValue::Symbol("?e".to_string())],
        }],
    };
    assert!(nj.has_negated_invocation());
}

#[test]
fn test_collect_rule_invocations_recurses_into_not_join() {
    let query = DatalogQuery::new(
        vec!["?e".to_string()],
        vec![WhereClause::NotJoin {
            join_vars: vec!["?e".to_string()],
            clauses: vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?e".to_string())],
            }],
        }],
    );
    let invocations = query.get_rule_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].0, "blocked");
}

#[test]
fn test_get_top_level_rule_invocations_excludes_not_join_body() {
    // not-join body rule invocations are NOT top-level
    let query = DatalogQuery::new(
        vec!["?e".to_string()],
        vec![
            WhereClause::RuleInvocation {
                predicate: "reachable".to_string(),
                args: vec![
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
            },
            WhereClause::NotJoin {
                join_vars: vec!["?e".to_string()],
                clauses: vec![WhereClause::RuleInvocation {
                    predicate: "blocked".to_string(),
                    args: vec![EdnValue::Symbol("?e".to_string())],
                }],
            },
        ],
    );
    let top_level = query.get_top_level_rule_invocations();
    // Only "reachable" is top-level; "blocked" is inside not-join
    assert_eq!(top_level.len(), 1);
    assert_eq!(top_level[0].0, "reachable");
}
```

- [ ] **Step 1.2: Run tests to verify they fail**

```bash
cargo test --lib query::datalog::types -- not_join 2>&1 | grep -E "FAILED|error"
```

Expected: compile errors (variant doesn't exist yet).

- [ ] **Step 1.3: Add `NotJoin` variant to `WhereClause`**

In `src/query/datalog/types.rs`, extend the `WhereClause` enum (after the `Not` variant, around line 179):

```rust
pub enum WhereClause {
    Pattern(Pattern),
    RuleInvocation { predicate: String, args: Vec<EdnValue> },
    Not(Vec<WhereClause>),
    /// not-join: explicit join variables + existentially quantified body.
    /// Succeeds when no assignment to non-join variables satisfies all inner clauses
    /// when join variables are substituted from the outer binding.
    NotJoin {
        join_vars: Vec<String>,
        clauses: Vec<WhereClause>,
    },
}
```

- [ ] **Step 1.4: Update `WhereClause::rule_invocations()`**

In the `impl WhereClause` block, extend `rule_invocations()`:

```rust
pub fn rule_invocations(&self) -> Vec<&str> {
    match self {
        WhereClause::Pattern(_) => vec![],
        WhereClause::RuleInvocation { predicate, .. } => vec![predicate.as_str()],
        WhereClause::Not(clauses) => {
            clauses.iter().flat_map(|c| c.rule_invocations()).collect()
        }
        WhereClause::NotJoin { clauses, .. } => {
            clauses.iter().flat_map(|c| c.rule_invocations()).collect()
        }
    }
}
```

- [ ] **Step 1.5: Update `WhereClause::has_negated_invocation()`**

```rust
pub fn has_negated_invocation(&self) -> bool {
    match self {
        WhereClause::Not(clauses) | WhereClause::NotJoin { clauses, .. } => {
            clauses.iter().any(|c| matches!(c, WhereClause::RuleInvocation { .. }))
        }
        _ => false,
    }
}
```

- [ ] **Step 1.6: Update `collect_rule_invocations_recursive()` in `DatalogQuery`**

```rust
fn collect_rule_invocations_recursive(clauses: &[WhereClause]) -> Vec<(String, Vec<EdnValue>)> {
    let mut result = Vec::new();
    for clause in clauses {
        match clause {
            WhereClause::RuleInvocation { predicate, args } => {
                result.push((predicate.clone(), args.clone()));
            }
            WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
                result.extend(Self::collect_rule_invocations_recursive(inner));
            }
            WhereClause::Pattern(_) => {}
        }
    }
    result
}
```

- [ ] **Step 1.7: Update `get_top_level_rule_invocations()` in `DatalogQuery`**

This method already exists in types.rs — it must skip `NotJoin` bodies just like `Not` bodies:

```rust
pub fn get_top_level_rule_invocations(&self) -> Vec<(String, Vec<EdnValue>)> {
    self.where_clauses
        .iter()
        .filter_map(|c| match c {
            WhereClause::RuleInvocation { predicate, args } => {
                Some((predicate.clone(), args.clone()))
            }
            _ => None,
        })
        .collect()
}
```

(This method already excludes `Not` by only matching `RuleInvocation` — adding `NotJoin` requires no change here, but verify it compiles.)

- [ ] **Step 1.8: Run tests to verify they pass**

```bash
cargo test --lib query::datalog::types -- not_join 2>&1 | tail -5
```

Expected: all 5 new tests pass.

- [ ] **Step 1.9: Fix exhaustive match errors caused by the new variant**

Adding `WhereClause::NotJoin` will break every `match` on `WhereClause` that lacks a catch-all. Fix them all now so the build is never broken between tasks. The known sites are:

- `src/query/datalog/parser.rs` — `outer_vars_from_clause` (line ~713): add `WhereClause::NotJoin { .. } => vec![]`
- Any other `match self` / `match clause` on `WhereClause` in `evaluator.rs`, `executor.rs`, `stratification.rs` — add `WhereClause::NotJoin { .. } => ...` with the appropriate default (usually the same as `Not`'s arm or a no-op)

Run `cargo build` after each file fix to verify no residual compile errors remain.

- [ ] **Step 1.10: Run full test suite to confirm no regressions**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: all existing tests pass.

- [ ] **Step 1.11: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add WhereClause::NotJoin variant and update helpers"
```

---

## Task 2: Update stratification to traverse `NotJoin` bodies

**Files:**
- Modify: `src/query/datalog/stratification.rs`

`not-join` creates the same negative dependency edges as `not` — wherever a rule uses `(not-join [?x] ...)` to negate predicate `p`, there is a negative edge from the rule's head predicate to `p`.

- [ ] **Step 2.1: Write failing unit test**

Add inside the `#[cfg(test)]` block in `stratification.rs`:

```rust
#[test]
fn test_not_join_creates_negative_dependency_edge() {
    // Rule: (eligible ?x) :- [?x :applied true], (not-join [?x] [?x :dep ?d] [?d :status :rejected])
    // not-join body contains a pattern referencing no named predicate, so no negative edge to a rule.
    // Test a case where not-join body contains a RuleInvocation:
    // (eligible ?x) :- [?x :applied true], (not-join [?x] (blocked ?x))
    // => negative edge: eligible -> blocked
    use crate::query::datalog::types::{EdnValue, Rule, WhereClause};

    let rule = Rule::new(
        vec![
            EdnValue::Symbol("eligible".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":applied".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::NotJoin {
                join_vars: vec!["?x".to_string()],
                clauses: vec![WhereClause::RuleInvocation {
                    predicate: "blocked".to_string(),
                    args: vec![EdnValue::Symbol("?x".to_string())],
                }],
            },
        ],
    );

    let mut registry = RuleRegistry::new();
    registry.register_rule_unchecked("eligible".to_string(), rule);
    let graph = DependencyGraph::from_rules(&registry);
    let strata = graph.stratify().unwrap();

    // "eligible" must be in a higher stratum than "blocked"
    let eligible_stratum = *strata.get("eligible").unwrap_or(&0);
    let blocked_stratum = *strata.get("blocked").unwrap_or(&0);
    assert!(
        eligible_stratum > blocked_stratum,
        "eligible (stratum {}) must be above blocked (stratum {})",
        eligible_stratum,
        blocked_stratum
    );
}

#[test]
fn test_not_join_negative_cycle_rejected() {
    // p :- (not-join [?x] (q ?x))
    // q :- (not-join [?x] (p ?x))
    // This is a negative cycle and must be rejected.
    use crate::query::datalog::types::{EdnValue, Rule, WhereClause};

    let rule_p = Rule::new(
        vec![
            EdnValue::Symbol("p".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![WhereClause::NotJoin {
            join_vars: vec!["?x".to_string()],
            clauses: vec![WhereClause::RuleInvocation {
                predicate: "q".to_string(),
                args: vec![EdnValue::Symbol("?x".to_string())],
            }],
        }],
    );
    let rule_q = Rule::new(
        vec![
            EdnValue::Symbol("q".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![WhereClause::NotJoin {
            join_vars: vec!["?x".to_string()],
            clauses: vec![WhereClause::RuleInvocation {
                predicate: "p".to_string(),
                args: vec![EdnValue::Symbol("?x".to_string())],
            }],
        }],
    );
    let mut registry = RuleRegistry::new();
    registry.register_rule_unchecked("p".to_string(), rule_p);
    registry.register_rule_unchecked("q".to_string(), rule_q);
    let graph = DependencyGraph::from_rules(&registry);
    assert!(
        graph.stratify().is_err(),
        "negative cycle via not-join must be rejected"
    );
}
```

- [ ] **Step 2.2: Run tests to confirm they fail**

```bash
cargo test --lib query::datalog::stratification -- not_join 2>&1 | grep -E "FAILED|error"
```

Expected: compile error (`WhereClause::NotJoin` not handled in `from_rules`).

- [ ] **Step 2.3: Update `DependencyGraph::from_rules`**

Find the match arm in `from_rules` that handles `WhereClause::Not` and add a `NotJoin` arm alongside it. The logic is identical — collect rule invocation predicates from the body and add negative edges:

```rust
WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
    for inner_clause in inner {
        if let WhereClause::RuleInvocation { predicate: dep, .. } = inner_clause {
            graph.add_negative_edge(head_pred, dep);
        }
    }
}
```

- [ ] **Step 2.4: Run tests to confirm they pass**

```bash
cargo test --lib query::datalog::stratification -- not_join 2>&1 | tail -5
```

Expected: both new tests pass.

- [ ] **Step 2.5: Run full suite**

```bash
cargo test 2>&1 | grep -E "^test result"
```

- [ ] **Step 2.6: Commit**

```bash
git add src/query/datalog/stratification.rs
git commit -m "feat(stratification): traverse NotJoin bodies for dependency edges"
```

---

## Task 3: Parse `(not-join [?vars…] clauses…)` with safety validation

**Files:**
- Modify: `src/query/datalog/parser.rs`

### Safety rule for `not-join`

Every variable listed in `join_vars` must be bound by an outer clause. Variables that appear only inside the body but are **not** in `join_vars` are existentially quantified — no error.

- [ ] **Step 3.1: Write failing parser unit tests**

Add inside the `#[cfg(test)]` block in `parser.rs`:

```rust
#[test]
fn test_parse_not_join_basic() {
    // (query [:find ?e :where [?e :name ?n] (not-join [?e] [?e :banned true])])
    let result = parse("(query [:find ?e :where [?e :name ?n] (not-join [?e] [?e :banned true])])");
    assert!(result.is_ok(), "basic not-join must parse OK: {:?}", result);
    if let Ok(DatalogCommand::Query(q)) = result {
        assert_eq!(q.where_clauses.len(), 2);
        assert!(matches!(
            &q.where_clauses[1],
            WhereClause::NotJoin { join_vars, clauses }
            if join_vars == &["?e".to_string()] && clauses.len() == 1
        ));
    } else {
        panic!("expected Query");
    }
}

#[test]
fn test_parse_not_join_multiple_join_vars() {
    let result = parse(
        "(query [:find ?e :where [?e :name ?n] [?e :role ?r] \
         (not-join [?e ?r] [?e :has-role ?r] [?r :is-admin true])])",
    );
    assert!(result.is_ok(), "multi-join-var not-join must parse: {:?}", result);
    if let Ok(DatalogCommand::Query(q)) = result {
        if let WhereClause::NotJoin { join_vars, clauses } = &q.where_clauses[2] {
            assert_eq!(join_vars.len(), 2);
            assert_eq!(clauses.len(), 2);
        } else {
            panic!("expected NotJoin");
        }
    }
}

#[test]
fn test_parse_not_join_inner_var_need_not_be_outer_bound() {
    // ?tag appears only in the not-join body — this is legal
    let result = parse(
        "(query [:find ?e :where [?e :name ?n] \
         (not-join [?e] [?e :has-tag ?tag] [?tag :is-bad true])])",
    );
    assert!(
        result.is_ok(),
        "inner-only var ?tag must be allowed in not-join: {:?}",
        result
    );
}

#[test]
fn test_parse_not_join_unbound_join_var_rejected() {
    // ?role is in join_vars but not bound by any outer clause
    let result = parse(
        "(query [:find ?e :where [?e :name ?n] \
         (not-join [?role] [?e :has-role ?role])])",
    );
    assert!(
        result.is_err(),
        "unbound join var must be rejected: {:?}",
        result
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("?role") && msg.contains("not bound"),
        "error must name the offending variable: {}",
        msg
    );
}

#[test]
fn test_parse_not_join_missing_join_vars_vector_rejected() {
    // First arg is not a vector
    let result = parse("(query [:find ?e :where [?e :name ?n] (not-join ?e [?e :banned true])])");
    assert!(result.is_err(), "non-vector first arg must fail");
}

#[test]
fn test_parse_not_join_too_few_args_rejected() {
    // Only join-vars vector, no body clauses
    let result = parse("(query [:find ?e :where [?e :name ?n] (not-join [?e])])");
    assert!(result.is_err(), "not-join with no clauses must fail");
}

#[test]
fn test_parse_not_join_nested_inside_not_rejected() {
    let result = parse(
        "(query [:find ?e :where [?e :name ?n] \
         (not (not-join [?e] [?e :banned true]))])",
    );
    assert!(result.is_err(), "not-join nested inside not must fail");
}

#[test]
fn test_parse_not_join_in_rule_body() {
    let result = parse(
        "(rule [(eligible ?x) \
         [?x :applied true] \
         (not-join [?x] [?x :dep ?d] [?d :status :rejected])])",
    );
    assert!(result.is_ok(), "not-join in rule body must parse: {:?}", result);
    if let Ok(DatalogCommand::Rule(rule)) = result {
        assert_eq!(rule.body.len(), 2);
        assert!(matches!(&rule.body[1], WhereClause::NotJoin { join_vars, .. }
            if join_vars == &["?x".to_string()]));
    }
}

#[test]
fn test_parse_not_join_rule_body_unbound_join_var_rejected() {
    // ?dep is in join_vars but never bound by outer body
    let result = parse(
        "(rule [(eligible ?x) \
         [?x :applied true] \
         (not-join [?dep] [?x :dep ?dep])])",
    );
    assert!(
        result.is_err(),
        "unbound join var in rule body not-join must fail"
    );
}
```

- [ ] **Step 3.2: Run tests to confirm they fail**

```bash
cargo test --lib query::datalog::parser -- not_join 2>&1 | grep -E "FAILED|error"
```

Expected: all fail (parsing returns Ok or wrong error because `not-join` isn't handled yet).

- [ ] **Step 3.3: Add `not-join` parsing in `parse_list_as_where_clause`**

In `src/query/datalog/parser.rs`, inside `parse_list_as_where_clause`, add a new match arm after the `"not"` arm:

```rust
EdnValue::Symbol(s) if s == "not-join" => {
    if !allow_not {
        return Err(
            "(not-join ...) cannot appear inside another (not ...) or (not-join ...)".to_string(),
        );
    }
    // Syntax: (not-join [?v1 ?v2 ...] clause1 clause2 ...)
    if list.len() < 3 {
        return Err(
            "(not-join) requires a join-vars vector and at least one clause".to_string(),
        );
    }
    let join_var_vec = list[1]
        .as_vector()
        .ok_or_else(|| "(not-join) first argument must be a vector of join variables".to_string())?;
    let join_vars: Vec<String> = join_var_vec
        .iter()
        .map(|v| {
            v.as_variable()
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    format!("(not-join) join variables must be logic variables, got {:?}", v)
                })
        })
        .collect::<Result<_, _>>()?;
    let mut inner = Vec::new();
    for item in &list[2..] {
        if let Some(vec) = item.as_vector() {
            let pattern = Pattern::from_edn(vec)?;
            inner.push(WhereClause::Pattern(pattern));
        } else if let Some(inner_list) = item.as_list() {
            // allow_not=false to reject nested (not ...) or (not-join ...)
            let clause = parse_list_as_where_clause(inner_list, false)?;
            inner.push(clause);
        } else {
            return Err(format!(
                "expected pattern or rule invocation inside (not-join), got {:?}",
                item
            ));
        }
    }
    Ok(WhereClause::NotJoin {
        join_vars,
        clauses: inner,
    })
}
```

- [ ] **Step 3.4: Add `check_not_join_safety` helper**

After the existing `check_not_safety` function:

```rust
/// Validate not-join safety: every variable listed in join_vars must be bound
/// by an outer clause. Variables that appear only in the not-join body but are
/// NOT in join_vars are existentially quantified — no error.
fn check_not_join_safety(
    clauses: &[WhereClause],
    outer_bound: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for clause in clauses {
        if let WhereClause::NotJoin { join_vars, .. } = clause {
            for var in join_vars {
                if !var.starts_with("?_") && !outer_bound.contains(var) {
                    return Err(format!(
                        "join variable {} in (not-join ...) is not bound by any outer clause",
                        var
                    ));
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3.5: Call `check_not_join_safety` in `parse_query` and `parse_rule`**

In `parse_query`, after the existing `check_not_safety` call (around line 517):

```rust
check_not_safety(&where_clauses, &outer_bound)?;
check_not_join_safety(&where_clauses, &outer_bound)?;  // add this line
```

In `parse_rule`, after the existing `check_not_safety` call (around line 837):

```rust
check_not_safety(&body_clauses, &outer_bound)?;
check_not_join_safety(&body_clauses, &outer_bound)?;  // add this line
```

- [ ] **Step 3.6: Update `outer_vars_from_clause` to handle `NotJoin`**

`outer_vars_from_clause` collects variables from non-not clauses. `NotJoin` body variables are local, so they should NOT be collected here. However, if `not-join` is in a rule body, its `join_vars` are already bound by prior clauses — no change needed. Verify the function has an exhaustive match or a catch-all and won't panic:

```rust
fn outer_vars_from_clause(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Pattern(p) => { /* ... existing ... */ }
        WhereClause::RuleInvocation { args, .. } => { /* ... existing ... */ }
        WhereClause::Not(_) => vec![],
        WhereClause::NotJoin { .. } => vec![],  // add this arm
    }
}
```

- [ ] **Step 3.7: Update `vars_in_not` to handle `NotJoin`** (if it has a match)

Check if `vars_in_not` handles `WhereClause::NotJoin` — it only handles `Not`. Since `check_not_join_safety` is separate, no change to `vars_in_not` is needed. Confirm it has a catch-all or add `NotJoin { .. } => vec![]`.

- [ ] **Step 3.8: Run parser tests**

```bash
cargo test --lib query::datalog::parser -- not_join 2>&1 | tail -10
```

Expected: all 9 new tests pass.

- [ ] **Step 3.9: Run full test suite**

```bash
cargo test 2>&1 | grep -E "^test result"
```

- [ ] **Step 3.10: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): parse (not-join [vars] clauses) with safety validation"
```

---

## Task 4: Evaluator — `StratifiedEvaluator` handles `NotJoin` in rule bodies

**Files:**
- Modify: `src/query/datalog/evaluator.rs`

The key function is `StratifiedEvaluator::evaluate`. It already processes `not` clauses by collecting `not_clauses: Vec<Vec<WhereClause>>` and testing each one per binding. `not-join` needs the same treatment, but with a crucial difference: only the `join_vars` are substituted; other variables in the body are left unbound and matched existentially.

Two additions in this task:
1. Extract `rule_invocation_to_pattern` from `RecursiveEvaluator` as a `pub(super)` free function so that `evaluate_not_join` can call it.
2. Add a `pub fn evaluate_not_join` helper that handles both `Pattern` and `RuleInvocation` clauses in the body, used by both the evaluator and executor (Task 5).

`RuleInvocation` clauses in a `not-join` body are handled by converting them to patterns via `rule_invocation_to_pattern` — derived facts are already stored as regular EAV facts in `accumulated` (with `:predicate` as the attribute) by the time `evaluate_not_join` is called, so `PatternMatcher` finds them correctly.

- [ ] **Step 4.0: Extract `rule_invocation_to_pattern` as a free function**

In `src/query/datalog/evaluator.rs`, find `RecursiveEvaluator::rule_invocation_to_pattern` (currently a private method). Extract it as a `pub(super)` free function above `RecursiveEvaluator`:

```rust
/// Convert a rule invocation to a pattern for fact-store lookup.
///
/// Derived facts are stored as regular EAV facts with the predicate name as attribute.
/// Example: (blocked ?x)        → Pattern [?x :blocked ?_rule_value]
/// Example: (reachable ?from ?to) → Pattern [?from :reachable ?to]
pub(super) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
    match args.len() {
        1 => Ok(Pattern::new(
            args[0].clone(),
            EdnValue::Keyword(format!(":{}", predicate)),
            EdnValue::Symbol("?_rule_value".to_string()),
        )),
        2 => Ok(Pattern::new(
            args[0].clone(),
            EdnValue::Keyword(format!(":{}", predicate)),
            args[1].clone(),
        )),
        n => Err(anyhow!(
            "Rule invocation '{}' must have 1 or 2 arguments, got {}",
            predicate,
            n
        )),
    }
}
```

Update `RecursiveEvaluator::rule_invocation_to_pattern` to delegate to the free function:

```rust
fn rule_invocation_to_pattern(&self, list: &[EdnValue]) -> Result<Pattern> {
    if list.is_empty() {
        return Err(anyhow!("Rule invocation cannot be empty"));
    }
    let predicate = match &list[0] {
        EdnValue::Symbol(s) => s.as_str(),
        _ => return Err(anyhow!("Rule invocation must start with predicate name (symbol)")),
    };
    super::rule_invocation_to_pattern(predicate, &list[1..])
}
```

Run `cargo build` to verify no compile errors.

- [ ] **Step 4.1: Write failing evaluator unit tests**

Add inside the `#[cfg(test)]` `stratified_tests` module in `evaluator.rs`:

```rust
#[test]
fn test_not_join_rejects_entity_with_matching_inner_var() {
    // Rule: (clean ?x) :- [?x :submitted true], (not-join [?x] [?x :has-dep ?d] [?d :blocked true])
    // alice: submitted=true, has-dep=dep1, dep1:blocked=true  -> NOT clean
    // bob:   submitted=true                                    -> clean
    use uuid::Uuid;
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let dep1 = Uuid::new_v4();
    storage
        .transact(
            vec![
                (alice, ":submitted".to_string(), Value::Boolean(true)),
                (alice, ":has-dep".to_string(), Value::Ref(dep1)),
                (dep1, ":blocked".to_string(), Value::Boolean(true)),
                (bob, ":submitted".to_string(), Value::Boolean(true)),
            ],
            None,
        )
        .unwrap();

    let rule = Rule::new(
        vec![
            EdnValue::Symbol("clean".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":submitted".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::NotJoin {
                join_vars: vec!["?x".to_string()],
                clauses: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":has-dep".to_string()),
                        EdnValue::Symbol("?d".to_string()),
                    )),
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?d".to_string()),
                        EdnValue::Keyword(":blocked".to_string()),
                        EdnValue::Boolean(true),
                    )),
                ],
            },
        ],
    );

    let mut registry = RuleRegistry::new();
    registry.register_rule_unchecked("clean".to_string(), rule);
    let rules = Arc::new(RwLock::new(registry));
    let evaluator = StratifiedEvaluator::new(storage, rules, 100);
    let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
    let clean_facts: Vec<_> = result
        .get_facts_by_attribute(&":clean".to_string())
        .unwrap_or_default();
    assert_eq!(clean_facts.len(), 1, "only bob should be clean");
    assert_eq!(
        clean_facts[0].entity, bob,
        "the clean entity must be bob"
    );
}

#[test]
fn test_not_join_keeps_entity_when_inner_var_has_no_match() {
    // Only alice has submitted=true and NO has-dep at all -> clean
    let storage = FactStorage::new();
    let alice = uuid::Uuid::new_v4();
    storage
        .transact(
            vec![(alice, ":submitted".to_string(), Value::Boolean(true))],
            None,
        )
        .unwrap();

    let rule = Rule::new(
        vec![
            EdnValue::Symbol("clean".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":submitted".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::NotJoin {
                join_vars: vec!["?x".to_string()],
                clauses: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":has-dep".to_string()),
                    EdnValue::Symbol("?d".to_string()),
                ))],
            },
        ],
    );

    let mut registry = RuleRegistry::new();
    registry.register_rule_unchecked("clean".to_string(), rule);
    let rules = Arc::new(RwLock::new(registry));
    let evaluator = StratifiedEvaluator::new(storage, rules, 100);
    let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
    let clean_facts: Vec<_> = result
        .get_facts_by_attribute(&":clean".to_string())
        .unwrap_or_default();
    assert_eq!(clean_facts.len(), 1, "alice must be clean when no deps exist");
}
```

Now add a third test for `RuleInvocation` in the body:

```rust
#[test]
fn test_not_join_body_with_rule_invocation() {
    // Rule: (blocked ?x) :- [?x :status :banned]
    // Rule: (clean ?x) :- [?x :submitted true], (not-join [?x] (blocked ?x))
    // alice: submitted, blocked -> NOT clean
    // bob: submitted, not blocked -> clean
    use uuid::Uuid;
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    storage
        .transact(
            vec![
                (alice, ":submitted".to_string(), Value::Boolean(true)),
                (alice, ":status".to_string(), Value::Keyword(":banned".to_string())),
                (bob, ":submitted".to_string(), Value::Boolean(true)),
            ],
            None,
        )
        .unwrap();

    let rule_blocked = Rule::new(
        vec![
            EdnValue::Symbol("blocked".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":status".to_string()),
            EdnValue::Keyword(":banned".to_string()),
        ))],
    );
    let rule_clean = Rule::new(
        vec![
            EdnValue::Symbol("clean".to_string()),
            EdnValue::Symbol("?x".to_string()),
        ],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":submitted".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::NotJoin {
                join_vars: vec!["?x".to_string()],
                clauses: vec![WhereClause::RuleInvocation {
                    predicate: "blocked".to_string(),
                    args: vec![EdnValue::Symbol("?x".to_string())],
                }],
            },
        ],
    );
    let mut registry = RuleRegistry::new();
    registry.register_rule_unchecked("blocked".to_string(), rule_blocked);
    registry.register_rule_unchecked("clean".to_string(), rule_clean);
    let rules = Arc::new(RwLock::new(registry));
    let evaluator = StratifiedEvaluator::new(storage, rules, 100);
    let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
    let clean_facts: Vec<_> = result
        .get_facts_by_attribute(&":clean".to_string())
        .unwrap_or_default();
    assert_eq!(clean_facts.len(), 1, "only bob should be clean");
    assert_eq!(clean_facts[0].entity, bob, "the clean entity must be bob");
}
```

- [ ] **Step 4.2: Run tests to verify they fail**

```bash
cargo test --lib query::datalog::evaluator -- not_join 2>&1 | grep -E "FAILED|error"
```

Expected: compile errors because `StratifiedEvaluator` doesn't handle `NotJoin` yet.

- [ ] **Step 4.3: Add `evaluate_not_join` public helper to `evaluator.rs`**

Add after `substitute_pattern` (around line 370):

```rust
/// Test whether a `not-join` body is satisfiable given a current binding.
///
/// Returns `true` if the body IS satisfiable (i.e., the outer binding should be **rejected**).
/// Returns `false` if the body cannot be satisfied (i.e., the outer binding survives).
///
/// Algorithm:
/// 1. Build a partial binding containing only the join_vars entries.
/// 2. For each clause:
///    - Pattern → substitute join_vars via substitute_pattern.
///    - RuleInvocation → convert to Pattern via rule_invocation_to_pattern, then substitute.
///      Rule-derived facts are already in `storage` (accumulated) from lower strata.
/// 3. Run PatternMatcher::match_patterns on all resulting patterns against `storage`.
/// 4. Any complete match → body is satisfiable → return true (reject outer binding).
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: &FactStorage,
) -> bool {
    // Build a partial binding containing only the join variables
    let partial: Bindings = join_vars
        .iter()
        .filter_map(|v| binding.get(v.as_str()).map(|val| (v.clone(), val.clone())))
        .collect();

    // Convert all clauses to patterns: Pattern clauses are substituted directly;
    // RuleInvocation clauses are first converted to their EAV pattern equivalent
    // (derived facts are stored as regular facts in `storage`), then substituted.
    let substituted: Vec<Pattern> = clauses
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(substitute_pattern(p, &partial)),
            WhereClause::RuleInvocation { predicate, args } => {
                rule_invocation_to_pattern(predicate, args)
                    .ok()
                    .map(|p| substitute_pattern(&p, &partial))
            }
            _ => None,
        })
        .collect();

    if substituted.is_empty() {
        return false;
    }

    let matcher = PatternMatcher::new(storage.clone());
    !matcher.match_patterns(&substituted).is_empty()
}
```

- [ ] **Step 4.4: Update `StratifiedEvaluator::evaluate` to handle `NotJoin`**

Inside the `mixed_rules` processing loop, find where `not_clauses` is built and add `not_join_clauses` alongside it. The existing code pattern (around line 519):

```rust
// Collect not clauses (existing)
let not_clauses: Vec<Vec<WhereClause>> = rule
    .body
    .iter()
    .filter_map(|c| match c {
        WhereClause::Not(inner) => Some(inner.clone()),
        _ => None,
    })
    .collect();

// Add: collect not-join clauses
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
```

Then in the `'binding` loop (around line 536), after the existing `not` checks and before head instantiation:

```rust
// Check not-join clauses
for (join_vars, nj_clauses) in &not_join_clauses {
    if evaluate_not_join(join_vars, nj_clauses, &binding, &accumulated) {
        continue 'binding; // body satisfied → reject this binding
    }
}
```

Also update the `has_not` detection so that rules with `NotJoin` are also classified as `mixed_rules`:

```rust
let has_not = rule.body.iter().any(|c| {
    matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. })
});
```

And in the positive-patterns extraction filter, add `NotJoin` to the skip list:

```rust
WhereClause::Not(_) | WhereClause::NotJoin { .. } => None,
```

- [ ] **Step 4.5: Run evaluator tests**

```bash
cargo test --lib query::datalog::evaluator -- not_join 2>&1 | tail -10
```

Expected: all 3 new tests pass.

- [ ] **Step 4.6: Run full suite**

```bash
cargo test 2>&1 | grep -E "^test result"
```

- [ ] **Step 4.7: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "feat(evaluator): extract rule_invocation_to_pattern; add evaluate_not_join with RuleInvocation support; handle NotJoin in StratifiedEvaluator"
```

---

## Task 5: Executor — post-filters handle `NotJoin` in query bodies

**Files:**
- Modify: `src/query/datalog/executor.rs`

There are **two** not-post-filter sites in `executor.rs`:
1. `execute_query` — pure pattern queries (no rule invocations), around line 192
2. `execute_query_with_rules` — queries mixed with rule invocations, around line 318

Both need the same `NotJoin` handling. Use `evaluate_not_join` from `evaluator.rs`.

- [ ] **Step 5.1: Write failing executor unit tests**

Add inside the `#[cfg(test)]` block in `executor.rs`:

```rust
#[test]
fn test_execute_query_not_join_basic() {
    // Query: find entities that have :submitted but NO blocked dependency
    // alice: submitted, has-dep dep1, dep1:blocked=true  -> excluded
    // bob:   submitted, no deps                          -> included
    use uuid::Uuid;
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let dep1 = Uuid::new_v4();
    storage
        .transact(
            vec![
                (alice, ":submitted".to_string(), Value::Boolean(true)),
                (alice, ":has-dep".to_string(), Value::Ref(dep1)),
                (dep1, ":blocked".to_string(), Value::Boolean(true)),
                (bob, ":submitted".to_string(), Value::Boolean(true)),
            ],
            None,
        )
        .unwrap();

    let query = DatalogQuery::new(
        vec!["?x".to_string()],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":submitted".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::NotJoin {
                join_vars: vec!["?x".to_string()],
                clauses: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":has-dep".to_string()),
                        EdnValue::Symbol("?d".to_string()),
                    )),
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?d".to_string()),
                        EdnValue::Keyword(":blocked".to_string()),
                        EdnValue::Boolean(true),
                    )),
                ],
            },
        ],
    );

    let executor = DatalogExecutor::new(storage, Arc::new(RwLock::new(RuleRegistry::new())));
    let result = executor.execute_query(&query).unwrap();
    assert_eq!(result.len(), 1, "only bob should be returned");
}

#[test]
fn test_execute_query_with_rules_not_join_in_query_body() {
    // Rule: (reachable ?x ?y) :- [?x :edge ?y]
    // Query: find ?x reachable from root that do NOT have a blocked dep
    // Combines a rule invocation and a not-join clause in the query body.
    use uuid::Uuid;
    let storage = FactStorage::new();
    let root = Uuid::new_v4();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let dep1 = Uuid::new_v4();
    storage
        .transact(
            vec![
                (root, ":edge".to_string(), Value::Ref(a)),
                (root, ":edge".to_string(), Value::Ref(b)),
                (a, ":has-dep".to_string(), Value::Ref(dep1)),
                (dep1, ":blocked".to_string(), Value::Boolean(true)),
            ],
            None,
        )
        .unwrap();

    let rule = Rule::new(
        vec![
            EdnValue::Symbol("reachable".to_string()),
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ],
        vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":edge".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ))],
    );

    let mut registry = RuleRegistry::new();
    registry
        .register_rule("reachable".to_string(), rule)
        .unwrap();
    let rules = Arc::new(RwLock::new(registry));

    let query = DatalogQuery::new(
        vec!["?y".to_string()],
        vec![
            WhereClause::RuleInvocation {
                predicate: "reachable".to_string(),
                args: vec![
                    EdnValue::Uuid(root),
                    EdnValue::Symbol("?y".to_string()),
                ],
            },
            WhereClause::NotJoin {
                join_vars: vec!["?y".to_string()],
                clauses: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?y".to_string()),
                        EdnValue::Keyword(":has-dep".to_string()),
                        EdnValue::Symbol("?d".to_string()),
                    )),
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?d".to_string()),
                        EdnValue::Keyword(":blocked".to_string()),
                        EdnValue::Boolean(true),
                    )),
                ],
            },
        ],
    );

    let executor = DatalogExecutor::new(storage, rules);
    let result = executor.execute_query_with_rules(&query).unwrap();
    // a is excluded (has a blocked dep); b passes
    assert_eq!(result.len(), 1, "only b should pass: {}", result.len());
}
```

- [ ] **Step 5.2: Run tests to verify they fail**

```bash
cargo test --lib query::datalog::executor -- not_join 2>&1 | grep -E "FAILED|error"
```

Expected: fail (executor ignores `NotJoin`).

- [ ] **Step 5.3: Import `evaluate_not_join` in `executor.rs`**

Add to the imports at the top of `executor.rs`:

```rust
use crate::query::datalog::evaluator::{
    evaluate_not_join, substitute_pattern, value_to_edn, StratifiedEvaluator,
};
```

- [ ] **Step 5.4: Update `execute_query` not-post-filter**

Find the not-filter block in `execute_query` (around line 192). Collect `not_join_clauses` alongside `not_clauses`, then extend the early-exit guard and the filter closure:

```rust
// Collect not clauses (existing)
let not_clauses: Vec<&Vec<WhereClause>> = query
    .where_clauses
    .iter()
    .filter_map(|c| match c {
        WhereClause::Not(inner) => Some(inner),
        _ => None,
    })
    .collect();

// Collect not-join clauses (new)
let not_join_clauses: Vec<(Vec<String>, Vec<WhereClause>)> = query
    .where_clauses
    .iter()
    .filter_map(|c| match c {
        WhereClause::NotJoin { join_vars, clauses } => {
            Some((join_vars.clone(), clauses.clone()))
        }
        _ => None,
    })
    .collect();

// IMPORTANT: guard must check BOTH to avoid skipping not-join when not is empty
let filtered_bindings: Vec<_> = if not_clauses.is_empty() && not_join_clauses.is_empty() {
    bindings
} else {
    let not_storage = filtered_storage.clone();
    bindings
        .into_iter()
        .filter(|binding| {
            // Existing not check
            for not_body in &not_clauses {
                // ... existing substitution + match logic ...
            }
            // New not-join check
            for (join_vars, nj_clauses) in &not_join_clauses {
                if evaluate_not_join(join_vars, nj_clauses, binding, &not_storage) {
                    return false;
                }
            }
            true
        })
        .collect()
};
```

The critical change: `if not_clauses.is_empty()` → `if not_clauses.is_empty() && not_join_clauses.is_empty()`. Without this, a query with only `not-join` (no `not`) bypasses the filter entirely and returns unfiltered results.

- [ ] **Step 5.5: Update `execute_query_with_rules` not-post-filter**

Same pattern as Step 5.4, applied to the second not-post-filter site (around line 318). The guard `if not_clauses.is_empty()` at line ~327 must also become `if not_clauses.is_empty() && not_join_clauses.is_empty()`.

- [ ] **Step 5.6: Run executor tests**

```bash
cargo test --lib query::datalog::executor -- not_join 2>&1 | tail -10
```

Expected: both new tests pass.

- [ ] **Step 5.7: Run full suite**

```bash
cargo test 2>&1 | grep -E "^test result"
```

- [ ] **Step 5.8: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 5.9: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): handle NotJoin in both not-post-filters"
```

---

## Task 6: Integration tests

**Files:**
- Create: `tests/not_join_test.rs`

- [ ] **Step 6.1: Write 10 integration tests**

Create `tests/not_join_test.rs`:

```rust
//! Integration tests for Phase 7.1b: not-join (existential negation).

use minigraf::OpenOptions;

fn in_memory_db() -> minigraf::Minigraf {
    OpenOptions::new().open().unwrap()
}

/// 1. Basic not-join: single join var, inner var existentially quantified.
#[test]
fn test_not_join_basic_inner_var_excluded() {
    let db = in_memory_db();
    // alice: has-dep dep1, dep1 blocked -> excluded
    // bob: no deps -> included
    db.execute("(transact [[:alice :name \"Alice\"] [:alice :has-dep :dep1] [:dep1 :blocked true] [:bob :name \"Bob\"]])").unwrap();
    let result = db
        .execute("(query [:find ?x :where [?x :name ?n] (not-join [?x] [?x :has-dep ?d] [?d :blocked true])])")
        .unwrap();
    let s = format!("{:?}", result);
    // bob should appear, alice should not
    assert!(s.contains("Bob"), "bob must be in results");
    assert!(!s.contains("Alice"), "alice must be excluded");
}

/// 2. Multiple join vars.
#[test]
fn test_not_join_multiple_join_vars() {
    let db = in_memory_db();
    // Track (user, role) pairs where role has no :is-restricted flag
    db.execute("(transact [[:u1 :has-role :r1] [:u2 :has-role :r2] [:r1 :is-restricted true]])").unwrap();
    // Find users whose role is NOT restricted for that specific user+role combo
    let result = db
        .execute("(query [:find ?u :where [?u :has-role ?r] (not-join [?u ?r] [?r :is-restricted true])])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("u2"), "u2 with unrestricted role must appear");
    assert!(!s.contains("u1"), "u1 with restricted role must be excluded");
}

/// 3. Multi-clause not-join body (conjunction of patterns).
#[test]
fn test_not_join_multi_clause_body() {
    let db = in_memory_db();
    // Exclude entities that have BOTH :tag :sensitive AND :tag :critical
    db.execute("(transact [[:e1 :tag :sensitive] [:e1 :tag :critical] [:e2 :tag :sensitive] [:e3 :data true]])").unwrap();
    let result = db
        .execute("(query [:find ?e :where [?e :data true] (not-join [?e] [?e :tag :sensitive])])")
        .unwrap();
    // Only e3 has :data true and no :tag :sensitive
    let s = format!("{:?}", result);
    assert!(s.contains("e3"), "e3 must appear");
    assert!(!s.contains("e1"), "e1 must be excluded");
}

/// 4. not-join in a rule body.
#[test]
fn test_not_join_in_rule_body() {
    let db = in_memory_db();
    db.execute("(transact [[:alice :applied true] [:alice :dep :dep1] [:dep1 :status :rejected] [:bob :applied true]])").unwrap();
    db.execute("(rule [(eligible ?x) [?x :applied true] (not-join [?x] [?x :dep ?d] [?d :status :rejected])])").unwrap();
    let result = db
        .execute("(query [:find ?x :where (eligible ?x)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("bob"), "bob must be eligible");
    assert!(!s.contains("alice"), "alice must be ineligible");
}

/// 5. not-join in multi-stratum rule chain.
#[test]
fn test_not_join_multi_stratum_chain() {
    let db = in_memory_db();
    // stratum 0: base facts
    // stratum 1: (eligible ?x) :- [?x :applied true], (not-join [?x] [?x :dep ?d] [?d :blocked true])
    // stratum 2: (approved ?x) :- (eligible ?x), (not-join [?x] [?x :on-hold true])
    db.execute("(transact [[:alice :applied true] [:alice :dep :dep1] [:dep1 :blocked true] [:bob :applied true] [:bob :on-hold true] [:charlie :applied true]])").unwrap();
    db.execute("(rule [(eligible ?x) [?x :applied true] (not-join [?x] [?x :dep ?d] [?d :blocked true])])").unwrap();
    db.execute("(rule [(approved ?x) (eligible ?x) (not-join [?x] [?x :on-hold true])])").unwrap();
    let result = db
        .execute("(query [:find ?x :where (approved ?x)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("charlie"), "charlie must be approved");
    assert!(!s.contains("alice"), "alice blocked at eligible step");
    assert!(!s.contains("bob"), "bob blocked at approved step");
}

/// 6. not-join vs not semantic difference: inner var prevents not-join from erroring.
/// Equivalent not would fail at parse time (unbound ?d); not-join succeeds.
#[test]
fn test_not_join_allows_inner_var_not_would_reject() {
    let db = in_memory_db();
    // This query would be a parse error with (not):
    //   (not [?x :dep ?d] [?d :blocked true])  -- ?d is unbound, error
    // With not-join it's valid: ?d is existentially quantified.
    db.execute("(transact [[:alice :dep :dep1] [:dep1 :blocked true] [:bob :dep :dep2]])").unwrap();
    let result = db
        .execute("(query [:find ?x :where [?x :dep ?_d2] (not-join [?x] [?x :dep ?d] [?d :blocked true])])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("bob"), "bob must appear (dep not blocked)");
    assert!(!s.contains("alice"), "alice must be excluded (dep blocked)");
}

/// 7. not-join combined with :as-of time travel.
#[test]
fn test_not_join_with_as_of() {
    let db = in_memory_db();
    db.execute("(transact [[:alice :applied true]])").unwrap(); // tx 1
    db.execute("(transact [[:dep1 :blocked true] [:alice :dep :dep1]])").unwrap(); // tx 2 — dep added later
    // At tx 1, alice has no dep → passes not-join
    let result_tx1 = db
        .execute("(query [:find ?x :as-of 1 :where [?x :applied true] (not-join [?x] [?x :dep ?d] [?d :blocked true])])")
        .unwrap();
    let s1 = format!("{:?}", result_tx1);
    assert!(s1.contains("alice"), "alice must pass at tx 1 (no dep yet)");

    // At tx 2, alice has a blocked dep → excluded
    let result_tx2 = db
        .execute("(query [:find ?x :as-of 2 :where [?x :applied true] (not-join [?x] [?x :dep ?d] [?d :blocked true])])")
        .unwrap();
    let s2 = format!("{:?}", result_tx2);
    assert!(!s2.contains("alice"), "alice must be excluded at tx 2");
}

/// 8. Unbound join var rejected at parse time.
#[test]
fn test_not_join_unbound_join_var_parse_error() {
    let db = in_memory_db();
    db.execute("(transact [[:e1 :name \"test\"]])").unwrap();
    let result = db.execute(
        "(query [:find ?e :where [?e :name ?n] (not-join [?unbound] [?e :dep ?unbound])])",
    );
    assert!(result.is_err(), "unbound join var must produce a parse error");
}

/// 9. Nested not-join inside not is rejected.
#[test]
fn test_not_join_nested_inside_not_rejected() {
    let db = in_memory_db();
    db.execute("(transact [[:e1 :data true]])").unwrap();
    let result = db.execute(
        "(query [:find ?e :where [?e :data true] (not (not-join [?e] [?e :flag true]))])",
    );
    assert!(
        result.is_err(),
        "not-join nested inside not must be a parse error"
    );
}

/// 10. not-join body contains a RuleInvocation — derived rule facts in accumulated
///     are correctly negated end-to-end through Minigraf::execute.
#[test]
fn test_not_join_body_with_rule_invocation_end_to_end() {
    let db = in_memory_db();
    // Rule: (banned ?x) :- [?x :status :banned]
    // Rule: (eligible ?x) :- [?x :applied true], (not-join [?x] (banned ?x))
    // alice: applied, banned -> NOT eligible
    // bob: applied, not banned -> eligible
    db.execute("(transact [[:alice :applied true] [:alice :status :banned] [:bob :applied true]])").unwrap();
    db.execute("(rule [(banned ?x) [?x :status :banned]])").unwrap();
    db.execute("(rule [(eligible ?x) [?x :applied true] (not-join [?x] (banned ?x))])").unwrap();
    let result = db
        .execute("(query [:find ?x :where (eligible ?x)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("bob"), "bob must be eligible");
    assert!(!s.contains("alice"), "alice must be excluded (banned rule fires)");
}
```

- [ ] **Step 6.2: Run integration tests**

```bash
cargo test --test not_join_test -- --nocapture 2>&1 | tail -20
```

Expected: all 10 tests pass.

- [ ] **Step 6.3: Run full test suite**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: all tests pass (341 baseline + ~25 new unit tests + 10 integration tests).

- [ ] **Step 6.4: Run clippy and format check**

```bash
cargo clippy -- -D warnings && cargo fmt --check
```

Expected: clean.

- [ ] **Step 6.5: Commit**

```bash
git add tests/not_join_test.rs
git commit -m "test(not-join): add 10 integration tests for Phase 7.1b"
```

---

## Implementation Notes

### Why `evaluate_not_join` is in `evaluator.rs`
`substitute_pattern` and `PatternMatcher` are both accessible from `evaluator.rs`, and the executor already imports from that module. Co-locating `evaluate_not_join` there avoids circular dependencies and keeps the existential matching logic in one place.

### Rule invocations inside `not-join` bodies
`evaluate_not_join` fully supports `RuleInvocation` clauses. Derived facts are stored as regular EAV facts in `accumulated` with `:predicate` as the attribute, so `rule_invocation_to_pattern` converts a `RuleInvocation` to its equivalent `Pattern` and `PatternMatcher` finds the derived facts the same way it finds base facts. The prerequisite is that `rule_invocation_to_pattern` is extracted as a `pub(super)` free function in Step 4.0 so that `evaluate_not_join` (outside `RecursiveEvaluator`) can call it.

### Empty `join_vars`
`(not-join [] [?any :lock true])` is valid. With no join vars, no substitution happens. The existential check asks: "does there exist any entity with `:lock true`?" This is a global existence check — if any fact matches, ALL outer bindings are rejected. The `evaluate_not_join` helper already handles this correctly: `partial_binding` is empty, patterns are left fully unbound, and `match_patterns` searches all facts.
