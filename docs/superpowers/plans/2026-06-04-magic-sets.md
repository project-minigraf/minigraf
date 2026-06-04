# Magic Sets Rewriting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement demand-driven recursive rule evaluation via magic sets rewriting (#289), reducing O(N²) derivations to O(N) for point queries over recursive rules.

**Architecture:** New `magic_sets.rs` module with a single `pub(crate) fn rewrite(query, registry) -> Option<(RuleRegistry, Vec<SeedFact>)>` function. Called from `execute_query_with_rules` in `executor.rs` before `StratifiedEvaluator` is constructed. Rewrites positive recursive rules to add magic-predicate guards, emits seed facts for bound query args, propagates adornments through SCCs for mutual recursion. Mixed rules (containing `not`/`not-join`) are left untouched.

**Tech Stack:** Rust; `types::{DatalogQuery, Rule, WhereClause, EdnValue, Pattern}`; `rules::RuleRegistry`; `graph::types::{Fact, Value, EntityId}`; `query::datalog::matcher::{edn_to_entity_id, edn_to_value}`

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src/query/datalog/magic_sets.rs` | Create | Adornment, seed facts, rule transformation, `rewrite()` |
| `src/query/datalog/mod.rs` | Modify | Add `pub(crate) mod magic_sets;` |
| `src/query/datalog/executor.rs` | Modify | Wire `magic_sets::rewrite()` into `execute_query_with_rules` |
| `tests/magic_sets_test.rs` | Create | End-to-end correctness tests |
| `ROADMAP.md` | Modify | Negation limitation note (§9.6) |

---

### Task 1: Scaffold the module

**Files:**
- Create: `src/query/datalog/magic_sets.rs`
- Modify: `src/query/datalog/mod.rs`

- [ ] **Step 1: Create `magic_sets.rs` with stub and first test**

```rust
// src/query/datalog/magic_sets.rs
use crate::graph::types::{EntityId, Value};
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{DatalogQuery, EdnValue, WhereClause};
use std::collections::{HashMap, HashSet};

pub(crate) fn rewrite(
    _query: &DatalogQuery,
    _registry: &RuleRegistry,
) -> Option<(RuleRegistry, Vec<(EntityId, String, Value)>)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::FindSpec;

    #[test]
    fn test_rewrite_empty_query_returns_none() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![],
        );
        let registry = RuleRegistry::new();
        assert!(rewrite(&query, &registry).is_none());
    }
}
```

- [ ] **Step 2: Add module to `src/query/datalog/mod.rs`**

After the `pub mod optimizer;` line, add:

```rust
pub(crate) mod magic_sets;
```

- [ ] **Step 3: Run test to verify it passes**

```bash
cargo test magic_sets::tests::test_rewrite_empty_query_returns_none -- --nocapture
```
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/query/datalog/magic_sets.rs src/query/datalog/mod.rs
git commit -m "feat(magic-sets): scaffold module (#289)"
```

---

### Task 2: Adornment classification

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

- [ ] **Step 1: Write failing tests**

Add these helper functions and tests to the `#[cfg(test)]` block in `magic_sets.rs`:

```rust
    use crate::query::datalog::types::{AttributeSpec, Pattern, Rule, WhereClause};

    fn pat(entity: &str, attr: &str, value: &str) -> WhereClause {
        WhereClause::Pattern(Pattern::new(
            if entity.starts_with('?') {
                EdnValue::Symbol(entity.to_string())
            } else {
                EdnValue::Keyword(entity.to_string())
            },
            EdnValue::Keyword(attr.to_string()),
            if value.starts_with('?') {
                EdnValue::Symbol(value.to_string())
            } else {
                EdnValue::String(value.to_string())
            },
        ))
    }

    fn rule_inv(pred: &str, args: &[&str]) -> WhereClause {
        WhereClause::RuleInvocation {
            predicate: pred.to_string(),
            args: args
                .iter()
                .map(|a| {
                    if a.starts_with('?') {
                        EdnValue::Symbol(a.to_string())
                    } else {
                        EdnValue::String(a.to_string())
                    }
                })
                .collect(),
        }
    }

    fn make_rule(pred: &str, head_args: &[&str], body: Vec<WhereClause>) -> Rule {
        Rule {
            head: std::iter::once(EdnValue::Symbol(pred.to_string()))
                .chain(head_args.iter().map(|a| EdnValue::Symbol(a.to_string())))
                .collect(),
            body,
        }
    }

    #[test]
    fn test_literal_arg_is_bound() {
        let clauses = vec![rule_inv("ancestor", &["abc123", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['b', 'f']));
    }

    #[test]
    fn test_free_var_is_free() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['f', 'f']));
    }

    #[test]
    fn test_var_grounded_by_preceding_pattern() {
        // [?x :name "Alice"] (ancestor ?x ?y) → ?x grounded → bf
        let clauses = vec![pat("?x", ":name", "Alice"), rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['b', 'f']));
    }

    #[test]
    fn test_all_free_has_no_bound() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        let ad = adornments.get("ancestor").unwrap();
        assert!(!has_bound_arg(ad));
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test magic_sets::tests::test_literal_arg_is_bound -- --nocapture
```
Expected: FAIL — `cannot find function compute_query_adornments`

- [ ] **Step 3: Implement adornment helpers**

Add these functions to `magic_sets.rs` (above `rewrite`):

```rust
/// Classify each arg in rule invocations as bound ('b') or free ('f').
/// Single left-to-right pass; all variables in Pattern entity/value positions are
/// considered grounded after the pattern (Datalog: pattern binds all its variables).
pub(crate) fn compute_query_adornments(
    where_clauses: &[WhereClause],
) -> HashMap<String, Vec<char>> {
    let mut grounded: HashSet<String> = HashSet::new();
    let mut adornments: HashMap<String, Vec<char>> = HashMap::new();

    for clause in where_clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let Some(var) = p.entity.as_variable() {
                    grounded.insert(var.to_string());
                }
                if let Some(var) = p.value.as_variable() {
                    grounded.insert(var.to_string());
                }
            }
            WhereClause::RuleInvocation { predicate, args } => {
                let adornment: Vec<char> = args
                    .iter()
                    .map(|arg| {
                        if let Some(var) = arg.as_variable() {
                            if grounded.contains(var) { 'b' } else { 'f' }
                        } else {
                            'b' // literal
                        }
                    })
                    .collect();
                adornments.entry(predicate.clone()).or_insert(adornment);
            }
            _ => {}
        }
    }

    adornments
}

/// Returns true if at least one position in the adornment is bound.
pub(crate) fn has_bound_arg(adornment: &[char]) -> bool {
    adornment.contains(&'b')
}

/// Convert adornment to string: ['b','f'] → "bf".
pub(crate) fn adornment_string(adornment: &[char]) -> String {
    adornment.iter().collect()
}

/// Magic predicate name: "__magic_ancestor_bf".
pub(crate) fn magic_pred_name(pred: &str, adornment: &[char]) -> String {
    format!("__magic_{}_{}", pred, adornment_string(adornment))
}
```

- [ ] **Step 4: Run all unit tests**

```bash
cargo test magic_sets::tests -- --nocapture
```
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "feat(magic-sets): adornment classification (#289)"
```

---

### Task 3: Seed fact generation

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)]`:

```rust
    #[test]
    fn test_seed_fact_for_keyword_entity_arg() {
        // (ancestor :alice ?y) — arg0 bound (keyword alias)
        // seed: entity = edn_to_entity_id(:alice), attr = ":__magic_ancestor_bf", value = true
        let clauses = vec![WhereClause::RuleInvocation {
            predicate: "ancestor".to_string(),
            args: vec![
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ],
        }];
        let adornments = compute_query_adornments(&clauses);
        let seeds = build_seed_facts(&clauses, &adornments);
        assert_eq!(seeds.len(), 1);
        let (entity, attr, value) = &seeds[0];
        assert_eq!(attr, ":__magic_ancestor_bf");
        assert_eq!(value, &Value::Boolean(true));
        let expected = crate::query::datalog::matcher::edn_to_entity_id(
            &EdnValue::Keyword(":alice".to_string()),
        )
        .unwrap();
        assert_eq!(*entity, expected);
    }

    #[test]
    fn test_no_seed_for_all_free() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        let seeds = build_seed_facts(&clauses, &adornments);
        assert!(seeds.is_empty());
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test magic_sets::tests::test_seed_fact_for_keyword_entity_arg -- --nocapture
```
Expected: FAIL — `cannot find function build_seed_facts`

- [ ] **Step 3: Implement `build_seed_facts`**

Add to `magic_sets.rs`:

```rust
use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};

/// Build seed facts for adorned rule invocations with at least one bound arg.
///
/// Encoding:
///   arg0 bound: entity = edn_to_entity_id(arg0), attr = ":__magic_p_ad", value = Boolean(true)
///   arg1-only bound (fb): entity = Uuid::new_v4() (ephemeral carrier), value = edn_to_value(arg1)
pub(crate) fn build_seed_facts(
    where_clauses: &[WhereClause],
    adornments: &HashMap<String, Vec<char>>,
) -> Vec<(EntityId, String, Value)> {
    let mut seeds = Vec::new();

    for clause in where_clauses {
        let WhereClause::RuleInvocation { predicate, args } = clause else {
            continue;
        };
        let Some(adornment) = adornments.get(predicate) else {
            continue;
        };
        if !has_bound_arg(adornment) {
            continue;
        }

        let magic_attr = format!(":{}",  magic_pred_name(predicate, adornment));

        if adornment.first() == Some(&'b') {
            // arg0 bound — dominant case
            if let Some(arg0) = args.first() {
                if let Ok(entity) = edn_to_entity_id(arg0) {
                    seeds.push((entity, magic_attr, Value::Boolean(true)));
                }
            }
        } else if adornment.get(1) == Some(&'b') {
            // arg1-only bound (fb) — ephemeral carrier UUID
            if let Some(arg1) = args.get(1) {
                if let Ok(value) = edn_to_value(arg1) {
                    seeds.push((uuid::Uuid::new_v4(), magic_attr, value));
                }
            }
        }
    }

    seeds
}
```

- [ ] **Step 4: Run all unit tests**

```bash
cargo test magic_sets::tests -- --nocapture
```
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "feat(magic-sets): seed fact generation (#289)"
```

---

### Task 4: Magic guard injection

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)]`:

```rust
    #[test]
    fn test_magic_guard_prepended_to_positive_rule() {
        // (ancestor ?a ?c) :- [?a :parent ?b] (ancestor ?b ?c)
        // adornment bf → guard (__magic_ancestor_bf ?a) prepended
        let rule = make_rule(
            "ancestor",
            &["?a", "?c"],
            vec![pat("?a", ":parent", "?b"), rule_inv("ancestor", &["?b", "?c"])],
        );
        let adornment = vec!['b', 'f'];
        let rewritten = inject_magic_guard(&rule, "ancestor", &adornment);
        match rewritten.body.first().unwrap() {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, &magic_pred_name("ancestor", &adornment));
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
            }
            other => panic!("expected RuleInvocation guard, got {:?}", other),
        }
    }

    #[test]
    fn test_mixed_rule_not_touched_by_guard() {
        let rule = make_rule(
            "eligible",
            &["?x"],
            vec![
                pat("?x", ":applied", "true"),
                WhereClause::Not(vec![pat("?x", ":rejected", "true")]),
            ],
        );
        let rewritten = inject_magic_guard(&rule, "eligible", &['b']);
        // First body clause must still be Pattern (not injected guard)
        assert!(
            matches!(rewritten.body.first().unwrap(), WhereClause::Pattern(_)),
            "mixed rule must not have guard injected"
        );
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test magic_sets::tests::test_magic_guard_prepended_to_positive_rule -- --nocapture
```
Expected: FAIL — `cannot find function inject_magic_guard`

- [ ] **Step 3: Implement `inject_magic_guard`**

Add to `magic_sets.rs`:

```rust
use crate::query::datalog::types::Rule;

/// Prepend a magic-predicate guard to a positive rule's body.
/// Rules containing Not/NotJoin are returned unchanged.
///
/// The guard uses head[1] (the entity-position variable) as its argument,
/// which is always the bound position for the dominant `bf` adornment.
pub(crate) fn inject_magic_guard(rule: &Rule, predicate: &str, adornment: &[char]) -> Rule {
    let has_negation = rule
        .body
        .iter()
        .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }));
    if has_negation {
        return rule.clone();
    }

    let magic_name = magic_pred_name(predicate, adornment);
    let bound_head_arg = rule
        .head
        .get(1)
        .cloned()
        .unwrap_or(EdnValue::Symbol("?_".to_string()));
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: vec![bound_head_arg],
    };

    let mut new_body = Vec::with_capacity(rule.body.len() + 1);
    new_body.push(guard);
    new_body.extend(rule.body.iter().cloned());

    Rule { head: rule.head.clone(), body: new_body }
}
```

- [ ] **Step 4: Run all unit tests**

```bash
cargo test magic_sets::tests -- --nocapture
```
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "feat(magic-sets): magic guard injection into positive rules (#289)"
```

---

### Task 5: Magic propagation rules

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)]`:

```rust
    #[test]
    fn test_propagation_rule_emitted_for_recursive_call() {
        // (ancestor ?a ?c) :- [?a :parent ?b] (ancestor ?b ?c)
        // bf → (__magic_ancestor_bf ?b) :- (__magic_ancestor_bf ?a) [?a :parent ?b]
        let rule = make_rule(
            "ancestor",
            &["?a", "?c"],
            vec![pat("?a", ":parent", "?b"), rule_inv("ancestor", &["?b", "?c"])],
        );
        let adorned: HashMap<String, Vec<char>> =
            [("ancestor".to_string(), vec!['b', 'f'])].into_iter().collect();
        let prop_rules = build_propagation_rules(&rule, "ancestor", &adorned);
        assert_eq!(prop_rules.len(), 1);

        let (pred, prop) = &prop_rules[0];
        assert_eq!(pred, &magic_pred_name("ancestor", &['b', 'f']));
        // Head: [Symbol("__magic_ancestor_bf"), Symbol("?b")]
        assert_eq!(prop.head.len(), 2);
        assert_eq!(prop.head[1], EdnValue::Symbol("?b".to_string()));
        // Body: [magic_guard(?a), [?a :parent ?b]]
        assert_eq!(prop.body.len(), 2);
        match &prop.body[0] {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, &magic_pred_name("ancestor", &['b', 'f']));
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
            }
            other => panic!("expected magic guard, got {:?}", other),
        }
    }

    #[test]
    fn test_no_propagation_for_non_recursive_rule() {
        // (ancestor ?a ?b) :- [?a :parent ?b]  — no recursive call
        let rule = make_rule("ancestor", &["?a", "?b"], vec![pat("?a", ":parent", "?b")]);
        let adorned: HashMap<String, Vec<char>> =
            [("ancestor".to_string(), vec!['b', 'f'])].into_iter().collect();
        let prop_rules = build_propagation_rules(&rule, "ancestor", &adorned);
        assert!(prop_rules.is_empty());
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test magic_sets::tests::test_propagation_rule_emitted_for_recursive_call -- --nocapture
```
Expected: FAIL — `cannot find function build_propagation_rules`

- [ ] **Step 3: Implement `build_propagation_rules`**

Add to `magic_sets.rs`:

```rust
/// Build magic propagation rules for adorned recursive calls within a rule body.
///
/// For each `RuleInvocation` in the body that calls an adorned predicate,
/// emits a rule: (magic-p-ad ?bound_arg) :- (magic-p-ad ?head_bound_var) <preceding clauses>
///
/// Returns Vec<(predicate_name, Rule)> to be registered with `register_rule_unchecked`.
pub(crate) fn build_propagation_rules(
    rule: &Rule,
    predicate: &str,
    adorned: &HashMap<String, Vec<char>>,
) -> Vec<(String, Rule)> {
    if rule
        .body
        .iter()
        .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }))
    {
        return vec![];
    }

    let Some(adornment) = adorned.get(predicate) else {
        return vec![];
    };
    let magic_name = magic_pred_name(predicate, adornment);

    // The bound-position variable from the rule head (head[1] = entity arg).
    let bound_head_var = rule
        .head
        .get(1)
        .cloned()
        .unwrap_or(EdnValue::Symbol("?_".to_string()));

    // Guard clause reused in every propagation rule body.
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name.clone(),
        args: vec![bound_head_var],
    };

    let mut result = Vec::new();

    for (i, clause) in rule.body.iter().enumerate() {
        let WhereClause::RuleInvocation {
            predicate: called_pred,
            args: called_args,
        } = clause
        else {
            continue;
        };

        // Only emit propagation for calls to an adorned predicate.
        let Some(called_adornment) = adorned.get(called_pred.as_str()) else {
            continue;
        };
        let called_magic_name = magic_pred_name(called_pred, called_adornment);

        // New magic head arg = bound-position arg of the recursive call (arg0 for bf).
        let new_magic_arg = called_args
            .first()
            .cloned()
            .unwrap_or(EdnValue::Symbol("?_".to_string()));

        // Propagation body = guard + all non-RuleInvocation clauses before this call.
        let mut prop_body = vec![guard.clone()];
        for preceding in &rule.body[..i] {
            if !matches!(preceding, WhereClause::RuleInvocation { .. }) {
                prop_body.push(preceding.clone());
            }
        }

        result.push((
            called_magic_name.clone(),
            Rule {
                head: vec![EdnValue::Symbol(called_magic_name), new_magic_arg],
                body: prop_body,
            },
        ));
    }

    result
}
```

- [ ] **Step 4: Run all unit tests**

```bash
cargo test magic_sets::tests -- --nocapture
```
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "feat(magic-sets): magic propagation rule generation (#289)"
```

---

### Task 6: SCC adornment propagation + assemble `rewrite()`

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)]`:

```rust
    #[test]
    fn test_scc_peer_adornment_propagates() {
        // even ?n :- (odd ?n)
        // odd  ?n :- (even ?n)
        // Initial: even adorned 'b' → odd should also be adorned via propagation
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "even".to_string(),
            make_rule("even", &["?n"], vec![rule_inv("odd", &["?n"])]),
        );
        registry.register_rule_unchecked(
            "odd".to_string(),
            make_rule("odd", &["?n"], vec![rule_inv("even", &["?n"])]),
        );
        let initial: HashMap<String, Vec<char>> =
            [("even".to_string(), vec!['b'])].into_iter().collect();
        let propagated = propagate_adornments(&initial, &registry);
        assert!(propagated.contains_key("odd"), "odd should be adorned via SCC");
    }

    #[test]
    fn test_rewrite_none_when_all_free() {
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule("reach", &["?a", "?b"], vec![pat("?a", ":edge", "?b")]),
        );
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![rule_inv("reach", &["?a", "?b"])],
        );
        assert!(rewrite(&query, &registry).is_none());
    }

    #[test]
    fn test_rewrite_some_when_bound_arg() {
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule("reach", &["?a", "?b"], vec![pat("?a", ":edge", "?b")]),
        );
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule(
                "reach",
                &["?a", "?c"],
                vec![rule_inv("reach", &["?a", "?b"]), pat("?b", ":edge", "?c")],
            ),
        );
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![WhereClause::RuleInvocation {
                predicate: "reach".to_string(),
                args: vec![
                    EdnValue::Keyword(":start".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
            }],
        );
        let result = rewrite(&query, &registry);
        assert!(result.is_some(), "should rewrite when arg0 is literal");
        let (rewritten_reg, seeds) = result.unwrap();
        assert!(!seeds.is_empty(), "should produce seed facts");
        let magic_name = magic_pred_name("reach", &['b', 'f']);
        assert!(
            !rewritten_reg.get_rules(&magic_name).is_empty(),
            "magic propagation rules should be registered"
        );
    }
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test magic_sets::tests::test_rewrite_some_when_bound_arg -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Implement `propagate_adornments`, `build_rewritten_registry`, and update `rewrite()`**

Replace the current stub `rewrite()` and add these functions to `magic_sets.rs`:

```rust
/// Propagate adornments transitively through rule bodies (handles mutual recursion).
/// Starting from `initial`, expands until fixed point.
pub(crate) fn propagate_adornments(
    initial: &HashMap<String, Vec<char>>,
    registry: &RuleRegistry,
) -> HashMap<String, Vec<char>> {
    let mut adorned = initial.clone();
    let mut changed = true;

    while changed {
        changed = false;
        for (pred, rules) in registry.all_rules() {
            let Some(adornment) = adorned.get(pred).cloned() else {
                continue;
            };
            for rule in rules {
                if rule
                    .body
                    .iter()
                    .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }))
                {
                    continue;
                }
                // Seed grounded vars with the head's bound-position var.
                let mut grounded: HashSet<String> = HashSet::new();
                if adornment.first() == Some(&'b') {
                    if let Some(EdnValue::Symbol(v)) = rule.head.get(1) {
                        grounded.insert(v.clone());
                    }
                }
                for clause in &rule.body {
                    match clause {
                        WhereClause::Pattern(p) => {
                            if let Some(v) = p.entity.as_variable() {
                                grounded.insert(v.to_string());
                            }
                            if let Some(v) = p.value.as_variable() {
                                grounded.insert(v.to_string());
                            }
                        }
                        WhereClause::RuleInvocation {
                            predicate: called,
                            args,
                        } => {
                            let call_adornment: Vec<char> = args
                                .iter()
                                .map(|a| {
                                    if let Some(v) = a.as_variable() {
                                        if grounded.contains(v) { 'b' } else { 'f' }
                                    } else {
                                        'b'
                                    }
                                })
                                .collect();
                            if has_bound_arg(&call_adornment)
                                && !adorned.contains_key(called.as_str())
                            {
                                adorned.insert(called.clone(), call_adornment);
                                changed = true;
                            }
                            for a in args {
                                if let Some(v) = a.as_variable() {
                                    grounded.insert(v.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    adorned
}

/// Build rewritten registry: copy all rules with magic guards injected for adorned
/// predicates, then register magic propagation rules for all adorned predicates.
/// Uses `register_rule_unchecked` because magic rules are self-recursive (positive cycle).
fn build_rewritten_registry(
    registry: &RuleRegistry,
    adorned: &HashMap<String, Vec<char>>,
) -> RuleRegistry {
    let mut new_reg = RuleRegistry::new();

    // Copy existing rules, injecting magic guards where adorned.
    for (pred, rules) in registry.all_rules() {
        for rule in rules {
            let rewritten = if let Some(adornment) = adorned.get(pred) {
                inject_magic_guard(rule, pred, adornment)
            } else {
                rule.clone()
            };
            new_reg.register_rule_unchecked(pred.to_string(), rewritten);
        }
    }

    // Emit magic propagation rules for adorned predicates.
    for (pred, rules) in registry.all_rules() {
        if let Some(adornment) = adorned.get(pred) {
            for rule in rules {
                for (magic_pred, prop_rule) in build_propagation_rules(rule, pred, adorned) {
                    new_reg.register_rule_unchecked(magic_pred, prop_rule);
                }
            }
        }
    }

    new_reg
}

pub(crate) fn rewrite(
    query: &DatalogQuery,
    registry: &RuleRegistry,
) -> Option<(RuleRegistry, Vec<(EntityId, String, Value)>)> {
    let initial = compute_query_adornments(&query.where_clauses);
    let initial_bound: HashMap<String, Vec<char>> = initial
        .into_iter()
        .filter(|(_, ad)| has_bound_arg(ad))
        .collect();

    if initial_bound.is_empty() {
        return None;
    }

    let adorned = propagate_adornments(&initial_bound, registry);
    let seeds = build_seed_facts(&query.where_clauses, &adorned);
    if seeds.is_empty() {
        return None;
    }

    Some((build_rewritten_registry(registry, &adorned), seeds))
}
```

- [ ] **Step 4: Run all unit tests**

```bash
cargo test magic_sets::tests -- --nocapture
```
Expected: all PASS

- [ ] **Step 5: Run full suite (no regressions before wiring)**

```bash
cargo test
```
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "feat(magic-sets): SCC propagation and complete rewrite() (#289)"
```

---

### Task 7: Wire into `execute_query_with_rules`

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Replace the StratifiedEvaluator construction block**

In `executor.rs`, locate the comment `// Apply temporal filters before evaluating recursive rules` (around line 922). Replace everything from that comment down to (and including) `let derived_storage = evaluator.evaluate(&predicates)?;` with:

```rust
        // Apply temporal filters before evaluating recursive rules
        let filtered_facts = self.filter_facts_for_query(&query)?;

        // Convert to FactStorage for StratifiedEvaluator (needs mutable accumulation)
        // TODO (post-1.0): use FactStorage::new_noindex() once profiling confirms rules-path
        // index rebuild is also a bottleneck.
        let filtered_storage = FactStorage::new();
        for fact in filtered_facts.iter().cloned() {
            filtered_storage.load_fact(fact)?;
        }

        // Compute effective limits: per-query override takes precedence over executor default.
        let effective_max_derived = query.max_derived_facts.unwrap_or(self.max_derived_facts);
        let effective_max_results = query.max_results.unwrap_or(self.max_results);

        // Apply magic sets rewriting for demand-driven recursive evaluation.
        // Returns None for all-free queries — zero overhead path.
        let rewritten = {
            let reg = self
                .rules
                .read()
                .map_err(|_| anyhow!("rule registry lock poisoned"))?;
            crate::query::datalog::magic_sets::rewrite(&query, &reg)
        };
        let (eval_rules, seed_facts) = match rewritten {
            Some((rewritten_registry, seeds)) => (Arc::new(RwLock::new(rewritten_registry)), seeds),
            None => (self.rules.clone(), vec![]),
        };
        for (entity, attribute, value) in seed_facts {
            filtered_storage.load_fact(Fact::new(entity, attribute, value, 0))?;
        }

        // Create StratifiedEvaluator — handles negation, stratification, and positive-only rules
        let evaluator = StratifiedEvaluator::new(
            filtered_storage,
            eval_rules,
            self.functions.clone(),
            1000, // max iterations
            effective_max_derived,
            effective_max_results,
        );

        let derived_storage = evaluator.evaluate(&predicates)?;
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test
```
Expected: all existing tests PASS — magic sets returns `None` for most queries (no bound args), so behaviour is unchanged for them.

- [ ] **Step 3: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(magic-sets): wire rewrite() into execute_query_with_rules (#289)"
```

---

### Task 8: Integration tests

**Files:**
- Create: `tests/magic_sets_test.rs`

- [ ] **Step 1: Create the test file**

```rust
//! Integration tests for magic sets rewriting (#289).
//!
//! Asserts result *correctness* only — magic sets must never change query results.

use minigraf::Minigraf;

fn open_db() -> Minigraf {
    Minigraf::open(":memory:").expect("open db")
}

fn exec(db: &Minigraf, cmd: &str) -> String {
    db.execute(cmd).expect("execute").to_string()
}

/// Transitive closure with bound start: only reachable nodes returned.
#[test]
fn test_bound_start_transitive_closure() {
    let db = open_db();
    exec(&db, "(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d]])");
    exec(&db, "(rule [(reach ?x ?y) [?x :edge ?y]])");
    exec(&db, "(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])");

    let result = exec(&db, "(query [:find ?y :where (reach :a ?y)])");
    assert!(result.contains(":b"), "should reach :b");
    assert!(result.contains(":c"), "should reach :c");
    assert!(result.contains(":d"), "should reach :d");
    assert!(!result.contains(":a"), "should not reach :a (no self-loop)");
}

/// All-free transitive closure: magic sets skipped, full result returned correctly.
#[test]
fn test_all_free_transitive_closure() {
    let db = open_db();
    exec(&db, "(transact [[:a :edge :b] [:b :edge :c]])");
    exec(&db, "(rule [(reach ?x ?y) [?x :edge ?y]])");
    exec(&db, "(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])");

    let result = exec(&db, "(query [:find ?x ?y :where (reach ?x ?y)])");
    assert!(result.contains(":a"));
    assert!(result.contains(":b"));
    assert!(result.contains(":c"));
}

/// Bound result is a subset of all-free result — no extra nodes returned.
#[test]
fn test_bound_result_subset_of_all_free() {
    let db = open_db();
    exec(&db, "(transact [[:x :link :y] [:y :link :z] [:p :link :q]])");
    exec(&db, "(rule [(conn ?a ?b) [?a :link ?b]])");
    exec(&db, "(rule [(conn ?a ?c) (conn ?a ?b) [?b :link ?c]])");

    let bound = exec(&db, "(query [:find ?b :where (conn :x ?b)])");
    let all_free = exec(&db, "(query [:find ?a ?b :where (conn ?a ?b)])");

    assert!(bound.contains(":y"));
    assert!(bound.contains(":z"));
    assert!(!bound.contains(":q"), ":q is unreachable from :x");
    assert!(all_free.contains(":q"), ":q is reachable from :p in full closure");
}

/// Multi-hop: 4 levels of recursion with a bound start.
#[test]
fn test_multi_hop_recursion_with_bound_start() {
    let db = open_db();
    exec(&db, "(transact [[:a :hop :b] [:b :hop :c] [:c :hop :d] [:d :hop :e]])");
    exec(&db, "(rule [(path ?x ?y) [?x :hop ?y]])");
    exec(&db, "(rule [(path ?x ?z) (path ?x ?y) [?y :hop ?z]])");

    let result = exec(&db, "(query [:find ?y :where (path :a ?y)])");
    assert!(result.contains(":b"));
    assert!(result.contains(":c"));
    assert!(result.contains(":d"));
    assert!(result.contains(":e"));
}

/// Mutual recursion: even/odd distance from a seeded node.
#[test]
fn test_mutual_recursion_even_odd_distance() {
    let db = open_db();
    // Chain: n0 → n1 → n2 → n3 → n4; mark n0 as the even-distance seed
    exec(&db, "(transact [
        [:n0 :is-start true]
        [:n0 :next :n1]
        [:n1 :next :n2]
        [:n2 :next :n3]
        [:n3 :next :n4]
    ])");
    exec(&db, "(rule [(even-d ?x) [?x :is-start true]])");
    exec(&db, "(rule [(even-d ?y) (odd-d ?x) [?x :next ?y]])");
    exec(&db, "(rule [(odd-d ?y) (even-d ?x) [?x :next ?y]])");

    let evens = exec(&db, "(query [:find ?x :where (even-d ?x)])");
    let odds  = exec(&db, "(query [:find ?x :where (odd-d ?x)])");

    assert!(evens.contains(":n0"), "n0 should be even-distance");
    assert!(evens.contains(":n2"), "n2 should be even-distance");
    assert!(evens.contains(":n4"), "n4 should be even-distance");
    assert!(!evens.contains(":n1"), "n1 should not be even-distance");

    assert!(odds.contains(":n1"), "n1 should be odd-distance");
    assert!(odds.contains(":n3"), "n3 should be odd-distance");
    assert!(!odds.contains(":n0"), "n0 should not be odd-distance");
}
```

- [ ] **Step 2: Run integration tests**

```bash
cargo test --test magic_sets_test -- --nocapture
```
Expected: all 5 PASS

- [ ] **Step 3: Run full test suite**

```bash
cargo test
```
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git add tests/magic_sets_test.rs
git commit -m "test(magic-sets): integration tests for result correctness (#289)"
```

---

### Task 9: ROADMAP negation note

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Add §9.6 after the §9.5 branching section**

Find the `### 9.5 Database Branching / Forking (Exploratory)` section in `ROADMAP.md`. After the closing paragraph of that section (and before whatever comes next), add:

```markdown
### 9.6 Magic Sets with Stratified Negation (Exploratory)

Magic sets rewriting (#289) is not applied to mixed rules containing `not`/`not-join` — those continue to use full semi-naive evaluation. Extending to negation is well-studied (Beeri & Ramakrishnan 1991, §5) but adds significant complexity: negated subgoals require care to avoid unsound propagation across stratification boundaries.

**Only worth pursuing if**: profiling shows that negation-heavy recursive rules are a bottleneck in a real workload. No issue is tracked — investigate if and when the need arises.
```

- [ ] **Step 2: Update the "Last Updated" footer**

Find the last line of `ROADMAP.md`:
```
**Last Updated**: May 2026 — 962 tests passing, v1.1.1; all post-1.0 work complete; #185 deferred to 2.0; ecosystem, developer tools, and integration examples fully transferred to external repos
```

Replace with:
```
**Last Updated**: June 2026 — magic sets rewriting (#289) implemented; #185 deferred to 2.0; ecosystem, developer tools, and integration examples fully transferred to external repos
```

(Update the test count after Task 8 confirms all tests passing.)

- [ ] **Step 3: Commit**

```bash
git add ROADMAP.md
git commit -m "docs: add magic sets negation limitation note to ROADMAP §9.6 (#289)"
```
