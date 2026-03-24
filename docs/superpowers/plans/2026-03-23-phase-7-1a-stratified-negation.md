# Phase 7.1a — Stratified Negation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `not` (negation as failure) to Datalog query bodies and rule bodies, with full stratification support and hard rejection of unstratifiable programs.

**Architecture:** Six files are modified or created. `WhereClause::Not` is added to the type system first; then `Rule.body` migrates from `Vec<EdnValue>` to `Vec<WhereClause>` in a single coordinated commit. A new `stratification.rs` module handles dependency-graph construction and stratum assignment. `StratifiedEvaluator` replaces `RecursiveEvaluator` at the executor call-site and applies `not` filters between strata.

**Tech Stack:** Rust, `anyhow`, existing `PatternMatcher`, `RecursiveEvaluator`, `FactStorage`, `RuleRegistry` from this repo.

**Spec:** `docs/superpowers/specs/2026-03-23-phase-7-1a-stratified-negation-design.md`

---

### Task 1: `WhereClause::Not` variant and query helper updates (`types.rs`)

**Files:**
- Modify: `src/query/datalog/types.rs`

This is a non-breaking additive change. Existing code compiles unchanged; only new tests exercise the new variant.

- [ ] **Step 1: Write failing unit tests inside `types.rs`**

Add these tests at the bottom of the `#[cfg(test)]` block in `src/query/datalog/types.rs`:

```rust
#[test]
fn test_where_clause_not_variant_exists() {
    let not_clause = WhereClause::Not(vec![
        WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":banned".to_string()),
            EdnValue::Boolean(true),
        )),
    ]);
    assert!(matches!(not_clause, WhereClause::Not(_)));
}

#[test]
fn test_rule_invocations_pattern_returns_empty() {
    let clause = WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?x".to_string()),
        EdnValue::Keyword(":a".to_string()),
        EdnValue::Symbol("?v".to_string()),
    ));
    assert!(clause.rule_invocations().is_empty());
}

#[test]
fn test_rule_invocations_rule_invocation_returns_predicate() {
    let clause = WhereClause::RuleInvocation {
        predicate: "blocked".to_string(),
        args: vec![EdnValue::Symbol("?x".to_string())],
    };
    assert_eq!(clause.rule_invocations(), vec!["blocked"]);
}

#[test]
fn test_rule_invocations_recurses_into_not() {
    let clause = WhereClause::Not(vec![WhereClause::RuleInvocation {
        predicate: "blocked".to_string(),
        args: vec![EdnValue::Symbol("?x".to_string())],
    }]);
    assert_eq!(clause.rule_invocations(), vec!["blocked"]);
}

#[test]
fn test_has_negated_invocation_true_when_not_contains_rule_invocation() {
    let clause = WhereClause::Not(vec![WhereClause::RuleInvocation {
        predicate: "blocked".to_string(),
        args: vec![EdnValue::Symbol("?x".to_string())],
    }]);
    assert!(clause.has_negated_invocation());
}

#[test]
fn test_has_negated_invocation_false_when_not_contains_only_pattern() {
    let clause = WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
        EdnValue::Symbol("?x".to_string()),
        EdnValue::Keyword(":banned".to_string()),
        EdnValue::Boolean(true),
    ))]);
    assert!(!clause.has_negated_invocation());
}

#[test]
fn test_uses_rules_recurses_into_not_body() {
    let query = DatalogQuery::new(
        vec!["?person".to_string()],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?person".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::Symbol("?name".to_string()),
            )),
            WhereClause::Not(vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?person".to_string())],
            }]),
        ],
    );
    assert!(query.uses_rules());
}

#[test]
fn test_get_rule_invocations_recurses_into_not_body() {
    let query = DatalogQuery::new(
        vec!["?person".to_string()],
        vec![
            WhereClause::Not(vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?person".to_string())],
            }]),
        ],
    );
    let invocations = query.get_rule_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].0, "blocked");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test -p minigraf --lib query::datalog::types -- 2>&1 | tail -20
```

Expected: compile errors — `WhereClause::Not` does not exist; `rule_invocations` / `has_negated_invocation` methods don't exist.

- [ ] **Step 3: Add `WhereClause::Not` variant**

In `src/query/datalog/types.rs`, change the `WhereClause` enum (around line 167):

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum WhereClause {
    /// A fact pattern: [?e :person/name ?name]
    Pattern(Pattern),
    /// A rule invocation: (reachable ?from ?to)
    RuleInvocation {
        /// Predicate name (e.g., "reachable")
        predicate: String,
        /// Arguments (variables, constants, or UUIDs)
        args: Vec<EdnValue>,
    },
    /// Negation as failure: (not clause1 clause2 ...)
    /// Succeeds when none of the inner clauses match.
    Not(Vec<WhereClause>),
}
```

- [ ] **Step 4: Add helper methods on `WhereClause`**

Immediately after the `WhereClause` enum definition, add:

```rust
impl WhereClause {
    /// Collect all rule invocation predicate names, recursively (including inside Not bodies).
    pub fn rule_invocations(&self) -> Vec<&str> {
        match self {
            WhereClause::Pattern(_) => vec![],
            WhereClause::RuleInvocation { predicate, .. } => vec![predicate.as_str()],
            WhereClause::Not(clauses) => {
                clauses.iter().flat_map(|c| c.rule_invocations()).collect()
            }
        }
    }

    /// True if this clause is a Not containing at least one RuleInvocation.
    pub fn has_negated_invocation(&self) -> bool {
        matches!(self, WhereClause::Not(clauses) if
            clauses.iter().any(|c| matches!(c, WhereClause::RuleInvocation { .. })))
    }
}
```

- [ ] **Step 5: Update `DatalogQuery::uses_rules()` and `get_rule_invocations()`**

Replace the two methods in the `DatalogQuery` impl (around lines 244–261):

```rust
/// Helper: Get all rule invocations from where clauses, including inside Not bodies
pub fn get_rule_invocations(&self) -> Vec<(String, Vec<EdnValue>)> {
    let mut result = Vec::new();
    for clause in &self.where_clauses {
        match clause {
            WhereClause::RuleInvocation { predicate, args } => {
                result.push((predicate.clone(), args.clone()));
            }
            WhereClause::Not(inner) => {
                for inner_clause in inner {
                    if let WhereClause::RuleInvocation { predicate, args } = inner_clause {
                        result.push((predicate.clone(), args.clone()));
                    }
                }
            }
            WhereClause::Pattern(_) => {}
        }
    }
    result
}

/// Check if this query uses any rules (including inside Not bodies)
pub fn uses_rules(&self) -> bool {
    self.where_clauses.iter().any(|clause| match clause {
        WhereClause::RuleInvocation { .. } => true,
        WhereClause::Not(inner) => inner
            .iter()
            .any(|c| matches!(c, WhereClause::RuleInvocation { .. })),
        WhereClause::Pattern(_) => false,
    })
}
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cargo test -p minigraf --lib query::datalog::types -- 2>&1 | tail -30
```

Expected: all `types` unit tests pass (including the 8 new ones).

- [ ] **Step 7: Confirm full test suite still compiles and passes**

```bash
cargo test 2>&1 | tail -10
```

Expected: all 331 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add WhereClause::Not variant and recursive helper methods"
```

---

### Task 2: Stratification module (`stratification.rs`)

**Files:**
- Create: `src/query/datalog/stratification.rs`
- Modify: `src/query/datalog/mod.rs`

- [ ] **Step 1: Write failing unit tests directly in the new file**

Create `src/query/datalog/stratification.rs` with tests only first:

```rust
use anyhow::Result;
use std::collections::HashMap;

use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{EdnValue, Rule, WhereClause, Pattern};

// ── Structs (to be implemented) ──────────────────────────────────────────────

pub struct DependencyGraph {
    /// head_predicate → Vec<(dependency_predicate, is_negative)>
    edges: HashMap<String, Vec<(String, bool)>>,
}

impl DependencyGraph {
    pub fn from_rules(registry: &RuleRegistry) -> Self {
        todo!()
    }

    pub fn stratify(&self) -> Result<HashMap<String, usize>> {
        todo!()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry_with_rules(rules: Vec<(&str, Rule)>) -> RuleRegistry {
        let mut registry = RuleRegistry::new();
        for (predicate, rule) in rules {
            registry.register_rule_unchecked(predicate.to_string(), rule);
        }
        registry
    }

    fn positive_rule(head_pred: &str, dep_pred: &str) -> (&'static str, Rule) {
        // (head_pred ?x) :- (dep_pred ?x)
        // head: [head_pred, ?x]
        // body: [RuleInvocation { dep_pred, [?x] }]
        let head = vec![
            EdnValue::Symbol(head_pred.to_string()),
            EdnValue::Symbol("?x".to_string()),
        ];
        let body = vec![WhereClause::RuleInvocation {
            predicate: dep_pred.to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }];
        // SAFETY: leaking head_pred str is fine for tests
        (Box::leak(head_pred.to_string().into_boxed_str()), Rule { head, body })
    }

    fn negative_rule(head_pred: &str, dep_pred: &str) -> (&'static str, Rule) {
        let head = vec![
            EdnValue::Symbol(head_pred.to_string()),
            EdnValue::Symbol("?x".to_string()),
        ];
        let body = vec![WhereClause::Not(vec![WhereClause::RuleInvocation {
            predicate: dep_pred.to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }])];
        (Box::leak(head_pred.to_string().into_boxed_str()), Rule { head, body })
    }

    fn base_rule(head_pred: &str) -> (&'static str, Rule) {
        let head = vec![
            EdnValue::Symbol(head_pred.to_string()),
            EdnValue::Symbol("?x".to_string()),
        ];
        let body = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":base".to_string()),
            EdnValue::Boolean(true),
        ))];
        (Box::leak(head_pred.to_string().into_boxed_str()), Rule { head, body })
    }

    #[test]
    fn test_positive_only_rules_all_stratum_zero() {
        // p depends positively on q; both at stratum 0
        let registry = make_registry_with_rules(vec![
            positive_rule("p", "q"),
            base_rule("q"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        let strata = graph.stratify().unwrap();
        assert_eq!(*strata.get("p").unwrap_or(&0), 0);
        assert_eq!(*strata.get("q").unwrap_or(&0), 0);
    }

    #[test]
    fn test_single_negative_edge_head_in_higher_stratum() {
        // eligible →⁻ rejected; rejected at 0, eligible at 1
        let registry = make_registry_with_rules(vec![
            negative_rule("eligible", "rejected"),
            base_rule("rejected"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        let strata = graph.stratify().unwrap();
        assert!(*strata.get("eligible").unwrap() > *strata.get("rejected").unwrap_or(&0));
    }

    #[test]
    fn test_two_stratum_chain() {
        // eligible →⁻ rejected →⁺ base
        // rejected = stratum 0, eligible = stratum 1
        let registry = make_registry_with_rules(vec![
            negative_rule("eligible", "rejected"),
            positive_rule("rejected", "base_fact"),
            base_rule("base_fact"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        let strata = graph.stratify().unwrap();
        let s_base = *strata.get("base_fact").unwrap_or(&0);
        let s_rejected = *strata.get("rejected").unwrap();
        let s_eligible = *strata.get("eligible").unwrap();
        assert!(s_rejected >= s_base);
        assert!(s_eligible > s_rejected);
    }

    #[test]
    fn test_negative_cycle_returns_error() {
        // p →⁻ q, q →⁻ p
        let registry = make_registry_with_rules(vec![
            negative_rule("p", "q"),
            negative_rule("q", "p"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_err());
    }

    #[test]
    fn test_self_negative_cycle_returns_error() {
        // p →⁻ p
        let registry = make_registry_with_rules(vec![
            negative_rule("p", "p"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_err());
    }

    #[test]
    fn test_disconnected_predicates_stratum_zero() {
        let registry = make_registry_with_rules(vec![
            base_rule("foo"),
            base_rule("bar"),
        ]);
        let graph = DependencyGraph::from_rules(&registry);
        let strata = graph.stratify().unwrap();
        assert_eq!(*strata.get("foo").unwrap_or(&0), 0);
        assert_eq!(*strata.get("bar").unwrap_or(&0), 0);
    }
}
```

- [ ] **Step 2: Add `register_rule_unchecked` to `RuleRegistry`**

The tests need a way to add rules without stratification checks (the checks come in Task 4). In `src/query/datalog/rules.rs`, add after `register_rule`:

```rust
/// Register a rule without stratification checks.
/// Used only in tests and internally before the stratification module is wired.
pub fn register_rule_unchecked(&mut self, predicate: String, rule: Rule) {
    self.rules.entry(predicate).or_default().push(rule);
}

/// Iterate all (predicate, rules) pairs.
pub fn all_rules(&self) -> impl Iterator<Item = (&str, &[Rule])> {
    self.rules.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
}
```

- [ ] **Step 3: Add `pub mod stratification;` to `mod.rs`**

In `src/query/datalog/mod.rs`, add:

```rust
pub mod stratification;
```

- [ ] **Step 4: Run tests to confirm they fail (not compile-error, but `todo!()` panic)**

```bash
cargo test -p minigraf --lib query::datalog::stratification 2>&1 | tail -20
```

Expected: tests compile but panic with `not yet implemented` (from `todo!()`).

- [ ] **Step 5: Implement `DependencyGraph::from_rules`**

Replace the `todo!()` body of `from_rules` with:

```rust
pub fn from_rules(registry: &RuleRegistry) -> Self {
    let mut edges: HashMap<String, Vec<(String, bool)>> = HashMap::new();

    for (head_pred, rules) in registry.all_rules() {
        for rule in rules {
            let entry = edges.entry(head_pred.to_string()).or_default();
            for clause in &rule.body {
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
                    WhereClause::Pattern(_) => {} // base facts: no predicate dependency
                }
            }
        }
    }

    DependencyGraph { edges }
}
```

- [ ] **Step 6: Implement `DependencyGraph::stratify`**

Replace the `todo!()` body of `stratify` with:

```rust
pub fn stratify(&self) -> Result<HashMap<String, usize>> {
    // Collect all predicates (both heads and dependencies)
    let mut all_predicates: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (head, deps) in &self.edges {
        all_predicates.insert(head.clone());
        for (dep, _) in deps {
            all_predicates.insert(dep.clone());
        }
    }
    let n = all_predicates.len();

    let mut strata: HashMap<String, usize> = all_predicates
        .into_iter()
        .map(|p| (p, 0))
        .collect();

    // Bellman-Ford-style constraint propagation
    let mut changed = true;
    while changed {
        changed = false;
        for (head, deps) in &self.edges {
            for (dep, is_negative) in deps {
                let dep_stratum = *strata.get(dep).unwrap_or(&0);
                let required = if *is_negative {
                    dep_stratum + 1
                } else {
                    dep_stratum
                };
                let head_stratum = strata.entry(head.clone()).or_insert(0);
                if required > *head_stratum {
                    *head_stratum = required;
                    changed = true;
                }
                // Cycle detection: stratum >= n means unstratifiable
                if *strata.get(head).unwrap_or(&0) >= n {
                    return Err(anyhow::anyhow!(
                        "unstratifiable: predicate '{}' is involved in a negative cycle through '{}'",
                        head,
                        dep
                    ));
                }
            }
        }
    }

    Ok(strata)
}
```

- [ ] **Step 7: Run stratification tests to confirm they pass**

```bash
cargo test -p minigraf --lib query::datalog::stratification 2>&1 | tail -20
```

Expected: all 6 stratification unit tests pass.

- [ ] **Step 8: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/query/datalog/stratification.rs src/query/datalog/mod.rs src/query/datalog/rules.rs
git commit -m "feat(stratification): add DependencyGraph and stratify() — Bellman-Ford stratum assignment"
```

---

### Task 3: Migrate `Rule.body` from `Vec<EdnValue>` to `Vec<WhereClause>`

**Files:**
- Modify: `src/query/datalog/types.rs` (Rule struct + Rule::new)
- Modify: `src/query/datalog/parser.rs` (parse_rule)
- Modify: `src/query/datalog/evaluator.rs` (evaluate_rule)
- Modify: `src/query/datalog/rules.rs` (test helper)

This is a coordinated multi-file change. All four files must be updated in one commit to keep the build green.

- [ ] **Step 1: Write a new test in `evaluator.rs` that verifies rule evaluation after migration**

In `src/query/datalog/evaluator.rs`, find the `#[cfg(test)]` block and add:

```rust
#[test]
fn test_evaluate_rule_with_where_clause_body() {
    // Build a rule: (reachable ?x ?y) :- [?x :connected ?y]
    // using Vec<WhereClause> body (post-migration shape)
    use crate::query::datalog::types::{Pattern, WhereClause};
    let mut storage = FactStorage::new();
    storage
        .transact(vec![crate::graph::types::Fact {
            entity: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            attribute: ":connected".to_string(),
            value: crate::graph::types::Value::Ref(
                uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            ),
            tx_id: uuid::Uuid::new_v4(),
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        }])
        .unwrap();

    let rule = Rule {
        head: vec![
            EdnValue::Symbol("reachable".to_string()),
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ],
        body: vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":connected".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ))],
    };

    let registry = Arc::new(RwLock::new(RuleRegistry::new()));
    let evaluator = RecursiveEvaluator::new(storage, registry, 10);
    // evaluate_rule is private; test via evaluate_recursive_rules
    let derived = evaluator
        .evaluate_recursive_rules(&["reachable".to_string()])
        .unwrap();
    // rule was not registered so result is just base facts, but the key check is no panic
    let _ = derived;
}
```

- [ ] **Step 2: Run the test to confirm it fails to compile (body is still `Vec<EdnValue>`)**

```bash
cargo test -p minigraf --lib query::datalog::evaluator::tests::test_evaluate_rule_with_where_clause_body 2>&1 | head -20
```

Expected: compile error — `Rule { body: Vec<EdnValue> }` does not accept `Vec<WhereClause>`.

- [ ] **Step 3: Change `Rule.body` type in `types.rs`**

In `src/query/datalog/types.rs`, replace the `Rule` struct and its `impl` (around lines 275–287):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The rule head: (predicate ?var1 ?var2)
    pub head: Vec<EdnValue>,
    /// The rule body: typed where clauses (patterns, rule invocations, not)
    pub body: Vec<WhereClause>,
}

impl Rule {
    pub fn new(head: Vec<EdnValue>, body: Vec<WhereClause>) -> Self {
        Rule { head, body }
    }
}
```

- [ ] **Step 4: Update `parse_rule` in `parser.rs` to produce `Vec<WhereClause>`**

In `src/query/datalog/parser.rs`, replace the body of `parse_rule` starting at the "Rest of body_vec are patterns or rule invocations" comment (around line 707):

```rust
// Rest of body_vec are patterns or rule invocations (not `not` yet — that comes in Task 5)
let mut body_clauses: Vec<WhereClause> = Vec::new();
for item in &body_vec[1..] {
    if let Some(vec) = item.as_vector() {
        let pattern = Pattern::from_edn(vec)?;
        body_clauses.push(WhereClause::Pattern(pattern));
    } else if let Some(list) = item.as_list() {
        if list.is_empty() {
            return Err("Rule invocation cannot be empty".to_string());
        }
        let predicate = match &list[0] {
            EdnValue::Symbol(s) => s.clone(),
            _ => return Err("Rule invocation must start with predicate name (symbol)".to_string()),
        };
        let args = list[1..].to_vec();
        body_clauses.push(WhereClause::RuleInvocation { predicate, args });
    } else {
        return Err(format!(
            "Rule body clause must be a vector (pattern) or list (rule invocation), got {:?}",
            item
        ));
    }
}

if body_clauses.is_empty() {
    return Err("Rule must have at least one pattern or rule invocation in body".to_string());
}

Ok(DatalogCommand::Rule(Rule {
    head: head_list.clone(),
    body: body_clauses,
}))
```

Replace the old code block from the comment "// Rest of body_vec are patterns or rule invocations" to the end of `parse_rule`. The old lines to remove are:

```rust
// Rest of body_vec are patterns or rule invocations
let body_clauses = body_vec[1..].to_vec();

if body_clauses.is_empty() {
    return Err("Rule must have at least one pattern or rule invocation in body".to_string());
}

Ok(DatalogCommand::Rule(Rule {
    head: head_list.clone(),
    body: body_clauses,
}))
```

- [ ] **Step 5: Update `evaluate_rule` in `evaluator.rs` to branch on `WhereClause` variants**

In `src/query/datalog/evaluator.rs`, replace `evaluate_rule` (lines 182–220):

```rust
fn evaluate_rule(&self, rule: &Rule, current_facts: &FactStorage) -> Result<Vec<Fact>> {
    let mut derived = Vec::new();

    // Build the list of patterns to match against, from WhereClause body
    let mut patterns = Vec::new();
    for clause in &rule.body {
        match clause {
            WhereClause::Pattern(p) => {
                patterns.push(p.clone());
            }
            WhereClause::RuleInvocation { predicate, args } => {
                // Convert (predicate arg0 arg1) → [arg0 :predicate arg1]
                let list: Vec<EdnValue> = std::iter::once(EdnValue::Symbol(predicate.clone()))
                    .chain(args.iter().cloned())
                    .collect();
                let pattern = self.rule_invocation_to_pattern(&list)?;
                patterns.push(pattern);
            }
            WhereClause::Not(_) => {
                // Not clauses are handled by StratifiedEvaluator, not here.
                return Err(anyhow!(
                    "WhereClause::Not in evaluate_rule: use StratifiedEvaluator for rules with negation"
                ));
            }
        }
    }

    if patterns.is_empty() {
        return Ok(derived);
    }

    let matcher = PatternMatcher::new(current_facts.clone());
    let bindings = matcher.match_patterns(&patterns);

    for binding in bindings {
        let fact = self.instantiate_head(&rule.head, &binding)?;
        derived.push(fact);
    }

    Ok(derived)
}
```

Also add the necessary import at the top of the `evaluator.rs` use block:

```rust
use crate::query::datalog::types::{EdnValue, Rule, WhereClause};
```

(Check if `WhereClause` is already imported; if `types::*` is already used, this is a no-op.)

- [ ] **Step 5b: Extend `rule_invocation_to_pattern` and `instantiate_head` for 1-arg rules**

The spec includes 1-arg predicates like `(blocked ?x)`. The current `rule_invocation_to_pattern` errors on `!= 3` elements and `instantiate_head` errors on `< 3` elements. Extend both to handle 1-arg rules:

In `evaluator.rs`, replace `rule_invocation_to_pattern` (around line 225):

```rust
fn rule_invocation_to_pattern(&self, list: &[EdnValue]) -> Result<Pattern> {
    if list.is_empty() {
        return Err(anyhow!("Rule invocation cannot be empty"));
    }
    let predicate = match &list[0] {
        EdnValue::Symbol(s) => s.clone(),
        _ => return Err(anyhow!("Rule invocation must start with predicate name (symbol)")),
    };

    match list.len() {
        2 => {
            // 1-arg: (blocked ?x)  →  [?x :blocked ?_rule_value]
            // ?_rule_value is a wildcard that matches any stored sentinel value.
            Ok(Pattern::new(
                list[1].clone(),
                EdnValue::Keyword(format!(":{}", predicate)),
                EdnValue::Symbol("?_rule_value".to_string()),
            ))
        }
        3 => {
            // 2-arg: (reachable ?from ?to)  →  [?from :reachable ?to]
            Ok(Pattern::new(
                list[1].clone(),
                EdnValue::Keyword(format!(":{}", predicate)),
                list[2].clone(),
            ))
        }
        n => Err(anyhow!(
            "Rule invocation '{}' must have 1 or 2 arguments, got {}",
            predicate,
            n - 1
        )),
    }
}
```

In the same file, replace `instantiate_head` (around line 265):

```rust
fn instantiate_head(&self, head: &[EdnValue], binding: &Bindings) -> Result<Fact> {
    if head.len() < 2 {
        return Err(anyhow!("Rule head must have at least 2 elements: (predicate ?arg1)"));
    }
    let predicate = match &head[0] {
        EdnValue::Symbol(s) => s.clone(),
        _ => return Err(anyhow!("Rule head must start with predicate name (symbol)")),
    };
    let entity_edn = self.substitute_variable(&head[1], binding)?;
    let entity = edn_to_entity_id(&entity_edn)
        .map_err(|e| anyhow!("Failed to convert entity: {}", e))?;

    let value = if head.len() >= 3 {
        // 2-arg head: (reachable ?from ?to) — value is head[2]
        let value_edn = self.substitute_variable(&head[2], binding)?;
        edn_to_value(&value_edn).map_err(|e| anyhow!("Failed to convert value: {}", e))?
    } else {
        // 1-arg head: (blocked ?x) — store a Boolean(true) sentinel
        crate::graph::types::Value::Boolean(true)
    };

    let attribute = format!(":{}", predicate);
    Ok(Fact::new(entity, attribute, value, 0))
}
```

- [ ] **Step 6: Fix the test helper in `rules.rs`**

In `src/query/datalog/rules.rs`, update `create_test_rule`:

```rust
fn create_test_rule(predicate: &str) -> Rule {
    // Create a simple test rule: (predicate ?x ?y) <- [?x :connected ?y]
    use crate::query::datalog::types::{Pattern, WhereClause};
    Rule {
        head: vec![
            EdnValue::Symbol(predicate.to_string()),
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ],
        body: vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":connected".to_string()),
            EdnValue::Symbol("?y".to_string()),
        ))],
    }
}
```

- [ ] **Step 7: Fix any remaining compile errors**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Fix any remaining call-sites that construct `Rule { body: vec![EdnValue::...] }` — there should be none outside of the test helper and parser.

- [ ] **Step 8: Run full test suite**

```bash
cargo test 2>&1 | tail -15
```

Expected: all 331+ tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/parser.rs src/query/datalog/evaluator.rs src/query/datalog/rules.rs
git commit -m "refactor(types): migrate Rule.body from Vec<EdnValue> to Vec<WhereClause>"
```

---

### Task 4: Stratification check in `register_rule`

**Files:**
- Modify: `src/query/datalog/rules.rs`

After this task, registering a negatively-cycling rule returns an error and the rule is not stored.

- [ ] **Step 1: Write a failing test in `rules.rs`**

Add to the `#[cfg(test)]` block in `src/query/datalog/rules.rs`:

```rust
#[test]
fn test_register_rule_rejects_negative_cycle() {
    use crate::query::datalog::types::{Pattern, WhereClause};
    let mut registry = RuleRegistry::new();

    // p :- not(q)   — p negatively depends on q
    let rule_p = Rule {
        head: vec![EdnValue::Symbol("p".to_string()), EdnValue::Symbol("?x".to_string())],
        body: vec![WhereClause::Not(vec![WhereClause::RuleInvocation {
            predicate: "q".to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }])],
    };
    // q :- not(p)   — q negatively depends on p  →  cycle
    let rule_q = Rule {
        head: vec![EdnValue::Symbol("q".to_string()), EdnValue::Symbol("?x".to_string())],
        body: vec![WhereClause::Not(vec![WhereClause::RuleInvocation {
            predicate: "p".to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }])],
    };

    // First rule registers fine
    registry.register_rule("p".to_string(), rule_p).unwrap();
    // Second rule creates a negative cycle → must fail
    let result = registry.register_rule("q".to_string(), rule_q);
    assert!(result.is_err(), "Expected stratification error for negative cycle");
    // The registry should NOT have stored the second rule
    assert!(registry.get_rules("q").is_empty());
}

#[test]
fn test_register_rule_accepts_stratifiable_negation() {
    use crate::query::datalog::types::{Pattern, WhereClause};
    let mut registry = RuleRegistry::new();

    // eligible :- not(rejected) — OK, one-way negative dependency
    let rule_eligible = Rule {
        head: vec![EdnValue::Symbol("eligible".to_string()), EdnValue::Symbol("?x".to_string())],
        body: vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":applied".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::Not(vec![WhereClause::RuleInvocation {
                predicate: "rejected".to_string(),
                args: vec![EdnValue::Symbol("?x".to_string())],
            }]),
        ],
    };

    registry.register_rule("eligible".to_string(), rule_eligible).unwrap();
    assert_eq!(registry.get_rules("eligible").len(), 1);
}
```

- [ ] **Step 2: Run tests to confirm the cycle-rejection test fails (register_rule currently always returns Ok)**

```bash
cargo test -p minigraf --lib query::datalog::rules::tests::test_register_rule_rejects_negative_cycle 2>&1 | tail -10
```

Expected: test fails — `result` is `Ok(())` but we asserted `is_err()`.

- [ ] **Step 3: Add stratification check to `register_rule`**

In `src/query/datalog/rules.rs`, add the import at the top:

```rust
use crate::query::datalog::stratification::DependencyGraph;
```

Replace `register_rule`:

```rust
pub fn register_rule(&mut self, predicate: String, rule: Rule) -> Result<()> {
    self.rules.entry(predicate.clone()).or_default().push(rule);

    // Check that the updated registry is still stratifiable
    let graph = DependencyGraph::from_rules(self);
    if let Err(e) = graph.stratify() {
        // Roll back: remove the rule we just added
        let rules = self.rules.get_mut(&predicate).unwrap();
        rules.pop();
        if rules.is_empty() {
            self.rules.remove(&predicate);
        }
        return Err(e);
    }

    Ok(())
}
```

- [ ] **Step 4: Run the new tests**

```bash
cargo test -p minigraf --lib query::datalog::rules 2>&1 | tail -20
```

Expected: all rules tests pass.

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/rules.rs
git commit -m "feat(rules): stratification check in register_rule — reject negative cycles"
```

---

### Task 5: Parse `(not ...)` clauses in queries and rule bodies

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing parser unit tests**

Add to the `#[cfg(test)]` block in `src/query/datalog/parser.rs`:

```rust
#[test]
fn test_parse_not_with_pattern_in_query() {
    let input = r#"(query [:find ?person :where [?person :name ?n] (not [?person :banned true])])"#;
    let cmd = parse_datalog_command(input).unwrap();
    match cmd {
        DatalogCommand::Query(q) => {
            assert_eq!(q.where_clauses.len(), 2);
            assert!(matches!(q.where_clauses[0], WhereClause::Pattern(_)));
            match &q.where_clauses[1] {
                WhereClause::Not(inner) => {
                    assert_eq!(inner.len(), 1);
                    assert!(matches!(inner[0], WhereClause::Pattern(_)));
                }
                other => panic!("Expected Not, got {:?}", other),
            }
        }
        _ => panic!("Expected Query"),
    }
}

#[test]
fn test_parse_not_with_rule_invocation_in_query() {
    let input = r#"(query [:find ?person :where [?person :name ?n] (not (blocked ?person))])"#;
    let cmd = parse_datalog_command(input).unwrap();
    match cmd {
        DatalogCommand::Query(q) => {
            match &q.where_clauses[1] {
                WhereClause::Not(inner) => {
                    assert!(matches!(inner[0], WhereClause::RuleInvocation { .. }));
                }
                other => panic!("Expected Not, got {:?}", other),
            }
        }
        _ => panic!("Expected Query"),
    }
}

#[test]
fn test_parse_not_in_rule_body() {
    let input = r#"(rule [(eligible ?x) [?x :applied true] (not (rejected ?x))])"#;
    let cmd = parse_datalog_command(input).unwrap();
    match cmd {
        DatalogCommand::Rule(rule) => {
            assert_eq!(rule.body.len(), 2);
            assert!(matches!(rule.body[0], WhereClause::Pattern(_)));
            assert!(matches!(rule.body[1], WhereClause::Not(_)));
        }
        _ => panic!("Expected Rule"),
    }
}

#[test]
fn test_parse_not_empty_body_is_error() {
    let input = r#"(query [:find ?x :where [?x :a ?v] (not)])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("requires at least one clause"), "got: {msg}");
}

#[test]
fn test_parse_nested_not_is_error() {
    let input = r#"(query [:find ?x :where [?x :a ?v] (not (not [?x :banned true]))])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("cannot appear inside another"), "got: {msg}");
}

#[test]
fn test_parse_not_unbound_variable_is_error() {
    // ?y is only in the not body, not in any outer clause
    let input = r#"(query [:find ?x :where [?x :a ?v] (not [?y :banned true])])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("not bound"), "got: {msg}");
}

#[test]
fn test_parse_not_unbound_variable_in_rule_body_is_error() {
    // ?y only in not, not in head or non-not body
    let input = r#"(rule [(eligible ?x) [?x :applied true] (not [?y :banned true])])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("not bound"), "got: {msg}");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test -p minigraf --lib query::datalog::parser -- test_parse_not 2>&1 | tail -20
```

Expected: failures — `(not ...)` is currently parsed as an unknown symbol or rule invocation.

- [ ] **Step 3: Extract a `parse_where_clause_list` helper (avoids duplication)**

In `src/query/datalog/parser.rs`, add a new helper function that parses a single list item in a `:where` context into a `WhereClause`, handling `not`:

```rust
/// Parse a list item (EDN List) appearing in a :where clause or rule body.
/// Returns Err if the list is empty, has an unknown form, or contains nested `not`.
fn parse_list_as_where_clause(
    list: &[EdnValue],
    allow_not: bool,
) -> Result<WhereClause, String> {
    if list.is_empty() {
        return Err("Empty list in :where clause".to_string());
    }
    match &list[0] {
        EdnValue::Symbol(s) if s == "not" => {
            if !allow_not {
                return Err("(not ...) cannot appear inside another (not ...)".to_string());
            }
            if list.len() < 2 {
                return Err("(not) requires at least one clause".to_string());
            }
            let mut inner = Vec::new();
            for item in &list[1..] {
                if let Some(vec) = item.as_vector() {
                    let pattern = Pattern::from_edn(vec)?;
                    inner.push(WhereClause::Pattern(pattern));
                } else if let Some(inner_list) = item.as_list() {
                    // Recurse with allow_not=false to reject nested not
                    let clause = parse_list_as_where_clause(inner_list, false)?;
                    inner.push(clause);
                } else {
                    return Err(format!(
                        "expected pattern or rule invocation inside (not), got {:?}",
                        item
                    ));
                }
            }
            Ok(WhereClause::Not(inner))
        }
        EdnValue::Symbol(predicate) => {
            let args = list[1..].to_vec();
            Ok(WhereClause::RuleInvocation {
                predicate: predicate.clone(),
                args,
            })
        }
        _ => Err(format!(
            "Rule invocation must start with predicate name (symbol), got {:?}",
            list[0]
        )),
    }
}
```

- [ ] **Step 4: Use the helper in query `:where` parsing**

In the `Some(":where")` branch inside `parse_query` (around line 491), replace the `else if let Some(rule_list)` block:

```rust
} else if let Some(rule_list) = query_vector[i].as_list() {
    let clause = parse_list_as_where_clause(rule_list, true)?;
    where_clauses.push(clause);
}
```

- [ ] **Step 5: Use the helper in `parse_rule`**

In `parse_rule`, replace the body-parsing loop:

```rust
let mut body_clauses: Vec<WhereClause> = Vec::new();
for item in &body_vec[1..] {
    if let Some(vec) = item.as_vector() {
        let pattern = Pattern::from_edn(vec)?;
        body_clauses.push(WhereClause::Pattern(pattern));
    } else if let Some(list) = item.as_list() {
        let clause = parse_list_as_where_clause(list, true)?;
        body_clauses.push(clause);
    } else {
        return Err(format!(
            "Rule body clause must be a vector (pattern) or list (rule invocation / not), got {:?}",
            item
        ));
    }
}
```

- [ ] **Step 6: Add safety validation — collect outer-bound variables then check `not` bodies**

After parsing the full `body_clauses` list in both `parse_query` and `parse_rule`, add a safety check. Because query parsing and rule parsing are different functions, add a shared helper:

```rust
/// Collect all variable names that appear in a where clause (non-recursively into Not).
fn outer_vars_from_clause(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Pattern(p) => {
            let mut vars = Vec::new();
            for v in [&p.entity, &p.attribute, &p.value] {
                if let Some(name) = v.as_variable() {
                    vars.push(name.to_string());
                }
            }
            vars
        }
        WhereClause::RuleInvocation { args, .. } => args
            .iter()
            .filter_map(|a| a.as_variable().map(|s| s.to_string()))
            .collect(),
        WhereClause::Not(_) => vec![], // not counted as "outer"
    }
}

/// Collect all variable names that appear inside a Not clause.
fn vars_in_not(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Not(inner) => inner
            .iter()
            .flat_map(|c| outer_vars_from_clause(c))
            .collect(),
        _ => vec![],
    }
}

/// Validate safety: every variable in a (not ...) body must be bound by an outer clause.
/// `outer_bound`: set of variable names already bound outside the not.
fn check_not_safety(
    clauses: &[WhereClause],
    outer_bound: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for clause in clauses {
        if let WhereClause::Not(_) = clause {
            for var in vars_in_not(clause) {
                if !outer_bound.contains(&var) {
                    return Err(format!(
                        "variable {} in (not ...) is not bound by any outer clause",
                        var
                    ));
                }
            }
        }
    }
    Ok(())
}
```

In `parse_query`, after building `where_clauses`, add:

```rust
// Safety check: all variables in (not ...) must be bound by outer clauses
let outer_bound: std::collections::HashSet<String> = where_clauses
    .iter()
    .flat_map(outer_vars_from_clause)
    .collect();
check_not_safety(&where_clauses, &outer_bound)?;
```

In `parse_rule`, after building `body_clauses`, add:

```rust
// Safety check: variables in (not ...) must be bound by the rule head or outer body clauses
let mut outer_bound: std::collections::HashSet<String> = std::collections::HashSet::new();
// Head args count as binding sites
for v in &head_list[1..] {
    if let Some(name) = v.as_variable() {
        outer_bound.insert(name.to_string());
    }
}
// Non-not body clauses
for clause in &body_clauses {
    for var in outer_vars_from_clause(clause) {
        outer_bound.insert(var);
    }
}
check_not_safety(&body_clauses, &outer_bound)?;
```

- [ ] **Step 7: Run parser tests**

```bash
cargo test -p minigraf --lib query::datalog::parser -- test_parse_not 2>&1 | tail -20
```

Expected: all 7 new parser tests pass.

- [ ] **Step 8: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): parse (not ...) clauses with safety validation and nested-not rejection"
```

---

### Task 6: `StratifiedEvaluator`

**Files:**
- Modify: `src/query/datalog/evaluator.rs`
- Modify: `src/query/datalog/mod.rs` (re-export)

- [ ] **Step 1: Write failing unit tests for `StratifiedEvaluator`**

Add to the `#[cfg(test)]` block in `src/query/datalog/evaluator.rs`:

```rust
mod stratified_tests {
    use super::*;
    use crate::graph::types::{Fact, Value};
    use crate::query::datalog::types::{Pattern, WhereClause};
    use uuid::Uuid;

    fn alice() -> Uuid { Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap() }
    fn bob()   -> Uuid { Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap() }

    fn make_fact(entity: Uuid, attr: &str, value: Value, tx: u64) -> Fact {
        Fact {
            entity,
            attribute: attr.to_string(),
            value,
            tx_id: Uuid::new_v4(),
            tx_count: tx,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        }
    }

    #[test]
    fn test_stratified_no_negation_same_as_recursive() {
        // StratifiedEvaluator with only positive rules must produce the same result
        // as RecursiveEvaluator.
        let mut storage = FactStorage::new();
        storage.transact(vec![
            make_fact(alice(), ":connected", Value::Ref(bob()), 1),
        ]).unwrap();

        let rule = Rule {
            head: vec![
                EdnValue::Symbol("reachable".to_string()),
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ],
            body: vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":connected".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ))],
        };
        let mut registry = RuleRegistry::new();
        registry.register_rule("reachable".to_string(), rule).unwrap();
        let rules = Arc::new(RwLock::new(registry));

        let evaluator = StratifiedEvaluator::new(storage, rules, 100);
        let result = evaluator.evaluate(&["reachable".to_string()]).unwrap();
        // get_facts_by_attribute returns Result<Vec<Fact>>
        let reachable_facts: Vec<_> = result
            .get_facts_by_attribute(":reachable")
            .unwrap()
            .into_iter()
            .filter(|f| f.asserted)
            .collect();
        assert_eq!(reachable_facts.len(), 1);
    }

    #[test]
    fn test_not_filter_removes_binding_when_body_satisfied() {
        // eligible :- [?x :applied true], not([?x :rejected true])
        // alice applied=true, rejected=true → NOT eligible
        let mut storage = FactStorage::new();
        storage.transact(vec![
            make_fact(alice(), ":applied", Value::Boolean(true), 1),
            make_fact(alice(), ":rejected", Value::Boolean(true), 2),
        ]).unwrap();

        let rule = Rule {
            head: vec![
                EdnValue::Symbol("eligible".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            body: vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":applied".to_string()),
                    EdnValue::Boolean(true),
                )),
                WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":rejected".to_string()),
                    EdnValue::Boolean(true),
                ))]),
            ],
        };
        let mut registry = RuleRegistry::new();
        registry.register_rule("eligible".to_string(), rule).unwrap();
        let rules = Arc::new(RwLock::new(registry));

        let evaluator = StratifiedEvaluator::new(storage, rules, 100);
        let result = evaluator.evaluate(&["eligible".to_string()]).unwrap();
        let eligible_facts: Vec<_> = result
            .get_facts_by_attribute(":eligible")
            .unwrap()
            .into_iter()
            .filter(|f| f.asserted)
            .collect();
        assert_eq!(eligible_facts.len(), 0, "alice should NOT be eligible");
    }

    #[test]
    fn test_not_filter_keeps_binding_when_body_not_satisfied() {
        // eligible :- [?x :applied true], not([?x :rejected true])
        // alice applied=true only → eligible
        let mut storage = FactStorage::new();
        storage.transact(vec![
            make_fact(alice(), ":applied", Value::Boolean(true), 1),
        ]).unwrap();

        let rule = Rule {
            head: vec![
                EdnValue::Symbol("eligible".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            body: vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":applied".to_string()),
                    EdnValue::Boolean(true),
                )),
                WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":rejected".to_string()),
                    EdnValue::Boolean(true),
                ))]),
            ],
        };
        let mut registry = RuleRegistry::new();
        registry.register_rule("eligible".to_string(), rule).unwrap();
        let rules = Arc::new(RwLock::new(registry));

        let evaluator = StratifiedEvaluator::new(storage, rules, 100);
        let result = evaluator.evaluate(&["eligible".to_string()]).unwrap();
        let eligible_facts: Vec<_> = result
            .get_facts_by_attribute(":eligible")
            .unwrap()
            .into_iter()
            .filter(|f| f.asserted)
            .collect();
        assert_eq!(eligible_facts.len(), 1, "alice should be eligible");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail to compile**

```bash
cargo test -p minigraf --lib query::datalog::evaluator::stratified_tests 2>&1 | head -20
```

Expected: compile error — `StratifiedEvaluator` does not exist.

- [ ] **Step 3: Implement `StratifiedEvaluator`**

Add the following to `src/query/datalog/evaluator.rs` (after the `RecursiveEvaluator` impl block):

```rust
/// Evaluates Datalog rules with stratified negation support.
///
/// Strata are evaluated in ascending order. Within each stratum, positive-only
/// rules are handled by RecursiveEvaluator; rules containing `not` clauses are
/// handled by an inner loop that applies `not` filters to candidate bindings.
pub struct StratifiedEvaluator {
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    max_iterations: usize,
}

impl StratifiedEvaluator {
    pub fn new(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        max_iterations: usize,
    ) -> Self {
        StratifiedEvaluator { storage, rules, max_iterations }
    }

    /// Derive all facts for the given predicates, respecting stratification order.
    pub fn evaluate(&self, predicates: &[String]) -> Result<FactStorage> {
        use crate::query::datalog::stratification::DependencyGraph;

        let registry = self.rules.read().unwrap();

        // Build dependency graph and stratify (defensive — should always succeed post-registration)
        let graph = DependencyGraph::from_rules(&*registry);
        let strata = graph.stratify()?;

        // Collect transitive dependencies of requested predicates
        let mut all_preds: Vec<String> = predicates.to_vec();
        {
            let mut i = 0;
            while i < all_preds.len() {
                let pred = all_preds[i].clone();
                for rule in registry.get_rules(&pred) {
                    for clause in &rule.body {
                        for dep in clause.rule_invocations() {
                            if !all_preds.contains(&dep.to_string()) {
                                all_preds.push(dep.to_string());
                            }
                        }
                    }
                }
                i += 1;
            }
        }

        // Group predicates by stratum
        let max_stratum = all_preds
            .iter()
            .map(|p| *strata.get(p).unwrap_or(&0))
            .max()
            .unwrap_or(0);

        drop(registry); // release read lock before recursive calls

        let mut accumulated = self.storage.clone();

        for stratum in 0..=max_stratum {
            let registry = self.rules.read().unwrap();
            let stratum_preds: Vec<String> = all_preds
                .iter()
                .filter(|p| *strata.get(*p).unwrap_or(&0) == stratum)
                .cloned()
                .collect();

            if stratum_preds.is_empty() {
                continue;
            }

            // Partition rules into positive-only and mixed (containing Not)
            let mut positive_rules: Vec<(String, Rule)> = Vec::new();
            let mut mixed_rules: Vec<(String, Rule)> = Vec::new();

            for pred in &stratum_preds {
                for rule in registry.get_rules(pred) {
                    let has_not = rule.body.iter().any(|c| matches!(c, WhereClause::Not(_)));
                    if has_not {
                        mixed_rules.push((pred.clone(), rule));
                    } else {
                        positive_rules.push((pred.clone(), rule));
                    }
                }
            }
            drop(registry);

            // Evaluate positive-only rules via RecursiveEvaluator
            if !positive_rules.is_empty() {
                let mut sub_registry = RuleRegistry::new();
                for (pred, rule) in &positive_rules {
                    sub_registry.register_rule_unchecked(pred.clone(), rule.clone());
                }
                let sub_rules = Arc::new(RwLock::new(sub_registry));
                let sub_eval = RecursiveEvaluator::new(
                    accumulated.clone(),
                    sub_rules,
                    self.max_iterations,
                );
                let derived = sub_eval.evaluate_recursive_rules(&stratum_preds)?;
                // Merge new facts into accumulated.
                // get_asserted_facts() returns Result<Vec<Fact>>; propagate errors with ?.
                for fact in derived.get_asserted_facts()? {
                    let _ = accumulated.load_fact(fact);
                }
            }

            // Evaluate mixed rules (with not-filter)
            for (pred, rule) in mixed_rules {
                let positive_patterns: Vec<Pattern> = rule
                    .body
                    .iter()
                    .filter_map(|c| {
                        match c {
                            WhereClause::Pattern(p) => Some(p.clone()),
                            WhereClause::RuleInvocation { predicate, args } => {
                                // Convert invocation to pattern.
                                // 1-arg: (blocked ?x)  →  [?x :blocked ?_rule_value]
                                // 2-arg: (reachable ?a ?b)  →  [?a :reachable ?b]
                                match args.len() {
                                    1 => Some(Pattern::new(
                                        args[0].clone(),
                                        EdnValue::Keyword(format!(":{}", predicate)),
                                        EdnValue::Symbol("?_rule_value".to_string()),
                                    )),
                                    2 => Some(Pattern::new(
                                        args[0].clone(),
                                        EdnValue::Keyword(format!(":{}", predicate)),
                                        args[1].clone(),
                                    )),
                                    _ => None, // unsupported arity — skip
                                }
                            }
                            WhereClause::Not(_) => None,
                        }
                    })
                    .collect();

                let not_clauses: Vec<&Vec<WhereClause>> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Not(inner) => Some(inner),
                        _ => None,
                    })
                    .collect();

                let matcher = PatternMatcher::new(accumulated.clone());
                let candidates = matcher.match_patterns(&positive_patterns);

                'binding: for binding in candidates {
                    for not_body in &not_clauses {
                        // Substitute bound variables into not_body patterns
                        let substituted: Vec<Pattern> = not_body
                            .iter()
                            .filter_map(|c| match c {
                                WhereClause::Pattern(p) => {
                                    Some(substitute_pattern(p, &binding))
                                }
                                WhereClause::RuleInvocation { predicate, args } => {
                                    let subst_args: Vec<EdnValue> = args
                                        .iter()
                                        .map(|a| substitute_value(a, &binding))
                                        .collect();
                                    match subst_args.len() {
                                        1 => Some(Pattern::new(
                                            subst_args[0].clone(),
                                            EdnValue::Keyword(format!(":{}", predicate)),
                                            EdnValue::Symbol("?_rule_value".to_string()),
                                        )),
                                        2 => Some(Pattern::new(
                                            subst_args[0].clone(),
                                            EdnValue::Keyword(format!(":{}", predicate)),
                                            subst_args[1].clone(),
                                        )),
                                        _ => None, // unsupported arity — skip
                                    }
                                }
                                WhereClause::Not(_) => None,
                            })
                            .collect();

                        let not_matcher = PatternMatcher::new(accumulated.clone());
                        let not_matches = not_matcher.match_patterns(&substituted);
                        if !not_matches.is_empty() {
                            // Not condition satisfied → discard this binding
                            continue 'binding;
                        }
                    }

                    // All Not conditions held → derive head fact
                    let registry = self.rules.read().unwrap();
                    // Reconstruct a temporary evaluator to call instantiate_head
                    let temp_eval = RecursiveEvaluator::new(
                        accumulated.clone(),
                        Arc::new(RwLock::new(registry.clone())),
                        1,
                    );
                    drop(registry);
                    if let Ok(fact) = temp_eval.instantiate_head_public(&rule.head, &binding) {
                        let _ = accumulated.load_fact(fact);
                    }
                }
            }
        }

        Ok(accumulated)
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn substitute_pattern(pattern: &Pattern, binding: &Bindings) -> Pattern {
    Pattern::new(
        substitute_value(&pattern.entity, binding),
        substitute_value(&pattern.attribute, binding),
        substitute_value(&pattern.value, binding),
    )
}

fn substitute_value(value: &EdnValue, binding: &Bindings) -> EdnValue {
    if let Some(var) = value.as_variable() {
        // binding values are owned Value; value_to_edn takes &Value, so use a closure.
        binding.get(var).map(|v| value_to_edn(v)).unwrap_or_else(|| value.clone())
    } else {
        value.clone()
    }
}
```

- [ ] **Step 4: Expose `instantiate_head` as `pub` on `RecursiveEvaluator`**

`StratifiedEvaluator` needs to call `instantiate_head`. In `evaluator.rs`, change the visibility of `instantiate_head` and add a public alias:

```rust
/// Public version of instantiate_head for use by StratifiedEvaluator.
pub fn instantiate_head_public(
    &self,
    head: &[EdnValue],
    binding: &Bindings,
) -> Result<Fact> {
    self.instantiate_head(head, binding)
}
```

Add this method to the `RecursiveEvaluator` impl block.

- [ ] **Step 5: Extract `value_to_edn` as a free function**

`substitute_value` (added in Step 3) calls `value_to_edn` as a free function pointer: `binding.get(var).cloned().map(value_to_edn)`. The current `value_to_edn` is a method on `RecursiveEvaluator` (`fn value_to_edn(&self, value: &Value) -> EdnValue`) — making it `pub` alone doesn't fix this since the `&self` receiver prevents it being used as a plain function pointer.

The body of `value_to_edn` does not use `self`, so extract it from the impl block:

1. Remove `fn value_to_edn(&self, value: &Value) -> EdnValue { ... }` from the `impl RecursiveEvaluator` block.
2. Add it as a free function (outside any impl) in `evaluator.rs`:

```rust
/// Convert a stored Value back to EdnValue. Used in rule head instantiation and substitution.
pub fn value_to_edn(value: &Value) -> EdnValue {
    match value {
        Value::String(s) => EdnValue::String(s.clone()),
        Value::Integer(i) => EdnValue::Integer(*i),
        Value::Float(f) => EdnValue::Float(*f),
        Value::Boolean(b) => EdnValue::Boolean(*b),
        Value::Ref(uuid) => EdnValue::Uuid(*uuid),
        Value::Keyword(k) => EdnValue::Keyword(k.clone()),
        Value::Null => EdnValue::Symbol("nil".to_string()),
    }
}
```

3. Update the one internal call site in `substitute_variable` (inside `RecursiveEvaluator`):

```rust
// was: Ok(self.value_to_edn(value))
Ok(value_to_edn(value))
```

After this, `substitute_value` (the free function added in Step 3) can call `value_to_edn` directly as a function pointer.

- [ ] **Step 6: Run new evaluator tests**

```bash
cargo test -p minigraf --lib query::datalog::evaluator::stratified_tests 2>&1 | tail -20
```

Expected: all 3 new tests pass.

- [ ] **Step 7: Export `StratifiedEvaluator` from `mod.rs`**

In `src/query/datalog/mod.rs`, add:

```rust
pub use evaluator::StratifiedEvaluator;
```

- [ ] **Step 8: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/query/datalog/evaluator.rs src/query/datalog/mod.rs
git commit -m "feat(evaluator): add StratifiedEvaluator with not-filter for mixed rules"
```

---

### Task 7: Wire `executor.rs` — switch to `StratifiedEvaluator`, add `not` post-filter

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write a failing integration-style unit test for `execute_query` with `not`**

Add to the `#[cfg(test)]` block in `src/query/datalog/executor.rs`:

```rust
#[test]
fn test_execute_query_not_as_pure_filter() {
    // Query: [:find ?e :where [?e :applied true] (not [?e :rejected true])]
    // No rule invocations — pure not-filter path in execute_query.
    use crate::query::datalog::types::{Pattern, WhereClause};
    use crate::graph::types::{Fact, Value};
    let mut storage = FactStorage::new();
    let alice = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let bob   = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    storage.transact(vec![
        Fact { entity: alice, attribute: ":applied".to_string(), value: Value::Boolean(true), tx_id: uuid::Uuid::new_v4(), tx_count: 1, valid_from: 0, valid_to: i64::MAX, asserted: true },
        Fact { entity: alice, attribute: ":rejected".to_string(), value: Value::Boolean(true), tx_id: uuid::Uuid::new_v4(), tx_count: 2, valid_from: 0, valid_to: i64::MAX, asserted: true },
        Fact { entity: bob,   attribute: ":applied".to_string(), value: Value::Boolean(true), tx_id: uuid::Uuid::new_v4(), tx_count: 3, valid_from: 0, valid_to: i64::MAX, asserted: true },
    ]).unwrap();

    let query = DatalogQuery::new(
        vec!["?e".to_string()],
        vec![
            WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":applied".to_string()),
                EdnValue::Boolean(true),
            )),
            WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":rejected".to_string()),
                EdnValue::Boolean(true),
            ))]),
        ],
    );

    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let executor = DatalogExecutor::new(storage, rules);
    let result = executor.execute(crate::query::datalog::types::DatalogCommand::Query(query)).unwrap();

    match result {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "only bob should pass (alice is rejected)");
        }
        _ => panic!("Expected QueryResults"),
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p minigraf --lib query::datalog::executor::tests::test_execute_query_not_as_pure_filter 2>&1 | tail -10
```

Expected: test fails — `not` clauses are currently ignored in `execute_query`.

- [ ] **Step 3: Switch `execute_query_with_rules` to use `StratifiedEvaluator`**

In `src/query/datalog/executor.rs`, add the import:

```rust
use crate::query::datalog::evaluator::StratifiedEvaluator;
```

Replace the evaluator creation in `execute_query_with_rules` (around line 227):

```rust
// Create StratifiedEvaluator — handles negation, stratification, and positive-only rules
let evaluator = StratifiedEvaluator::new(
    filtered_storage,
    self.rules.clone(),
    1000, // max iterations
);

let derived_storage = evaluator.evaluate(&predicates)?;
```

- [ ] **Step 4: Add `not` post-filter to `execute_query` (pure-pattern path)**

In `execute_query`, after `let bindings = matcher.match_patterns(...)`, add a not-filter before extracting results:

```rust
// Apply not-filter for WhereClause::Not clauses (no rules involved — pure post-filter)
let not_clauses: Vec<&Vec<WhereClause>> = query
    .where_clauses
    .iter()
    .filter_map(|c| match c {
        WhereClause::Not(inner) => Some(inner),
        _ => None,
    })
    .collect();

let filtered_bindings: Vec<_> = if not_clauses.is_empty() {
    bindings
} else {
    // filtered_storage is already cloned at the PatternMatcher::new call (see step 4a below),
    // so it is still valid here.
    let not_storage = filtered_storage.clone();
    bindings
        .into_iter()
        .filter(|binding| {
            for not_body in &not_clauses {
                let substituted: Vec<Pattern> = not_body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Pattern(p) => Some(crate::query::datalog::evaluator::substitute_pattern(p, binding)),
                        _ => None,
                    })
                    .collect();
                let m = PatternMatcher::new(not_storage.clone());
                if !m.match_patterns(&substituted).is_empty() {
                    return false; // not condition violated
                }
            }
            true
        })
        .collect()
};
```

**Step 4a (prerequisite):** At line ~174 in `execute_query`, the existing code passes `filtered_storage` by value into `PatternMatcher::new`, consuming it. Before making the changes above, change that line to clone:

```rust
// was: let matcher = PatternMatcher::new(filtered_storage);
let matcher = PatternMatcher::new(filtered_storage.clone()); // keep filtered_storage for not-filter
```

After this clone, `filtered_storage` remains valid for use as `not_storage` in the block above. Then use `filtered_bindings` instead of `bindings` for the variable extraction loop.

- [ ] **Step 5: Make `substitute_pattern` pub in evaluator.rs**

The `substitute_pattern` helper added in Task 6 needs to be `pub` so executor.rs can use it. In `evaluator.rs`:

```rust
pub fn substitute_pattern(pattern: &Pattern, binding: &Bindings) -> Pattern {
```

- [ ] **Step 6: Run the new executor test**

```bash
cargo test -p minigraf --lib query::datalog::executor::tests::test_execute_query_not_as_pure_filter 2>&1 | tail -10
```

Expected: test passes.

- [ ] **Step 7: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/executor.rs src/query/datalog/evaluator.rs
git commit -m "feat(executor): switch to StratifiedEvaluator; add not-filter in pure-query path"
```

---

### Task 8: Integration tests (`tests/negation_test.rs`)

**Files:**
- Create: `tests/negation_test.rs`

- [ ] **Step 1: Create the test file with all 10 integration tests**

Create `tests/negation_test.rs`:

```rust
//! Integration tests for stratified negation (Phase 7.1a).
//! Covers the 10 scenarios from the spec testing plan.

use minigraf::{Minigraf, OpenOptions};

fn in_memory_db() -> Minigraf {
    OpenOptions::new().open().unwrap()
}

// ── Test 1: Simple not on base fact ──────────────────────────────────────────

#[test]
fn test_not_excludes_base_fact() {
    let db = in_memory_db();

    db.execute(r#"(transact [[:alice :person/name "Alice"]
                              [:bob   :person/name "Bob"]
                              [:alice :person/banned true]])"#).unwrap();

    let result = db.execute(r#"
        (query [:find ?person
                :where [?person :person/name ?n]
                       (not [?person :person/banned true])])
    "#).unwrap();

    let result_str = format!("{:?}", result);
    assert!(!result_str.contains("alice"), "alice is banned, should not appear");
    assert!(result_str.contains("bob") || result_str.contains("Bob"),
            "bob is not banned, should appear");
}

// ── Test 2: not with multiple clauses (conjunction) ──────────────────────────

#[test]
fn test_not_multiple_clauses_conjunction() {
    let db = in_memory_db();

    db.execute(r#"(transact [[:alice :role :admin]
                              [:alice :active false]
                              [:bob   :role :admin]
                              [:bob   :active true]])"#).unwrap();

    // Exclude entities that are BOTH admin AND active=false
    let result = db.execute(r#"
        (query [:find ?person
                :where [?person :role :admin]
                       (not [?person :role :admin]
                            [?person :active false])])
    "#).unwrap();

    let result_str = format!("{:?}", result);
    assert!(!result_str.contains("alice"), "alice matches both conditions, should be excluded");
    assert!(result_str.contains("bob"), "bob is active=true, should not be excluded");
}

// ── Test 3: not negating a derived rule ──────────────────────────────────────

#[test]
fn test_not_negates_derived_rule() {
    let db = in_memory_db();

    db.execute(r#"(rule [(blocked ?x) [?x :status :blocked]])"#).unwrap();

    db.execute(r#"(transact [[:alice :person/name "Alice"]
                              [:bob   :person/name "Bob"]
                              [:alice :status :blocked]])"#).unwrap();

    let result = db.execute(r#"
        (query [:find ?person
                :where [?person :person/name ?n]
                       (not (blocked ?person))])
    "#).unwrap();

    let result_str = format!("{:?}", result);
    assert!(!result_str.contains("alice"), "alice is blocked");
    assert!(result_str.contains("bob") || result_str.contains("Bob"),
            "bob is not blocked");
}

// ── Test 4: Multi-stratum chain ───────────────────────────────────────────────

#[test]
fn test_multi_stratum_not_on_derived_predicate() {
    let db = in_memory_db();

    // rejected is derived, eligible uses not(rejected)
    db.execute(r#"(rule [(rejected ?x) [?x :score :low]])"#).unwrap();
    db.execute(r#"(rule [(eligible ?x) [?x :applied true] (not (rejected ?x))])"#).unwrap();

    db.execute(r#"(transact [[:alice :applied true]
                              [:alice :score :low]
                              [:bob   :applied true]
                              [:bob   :score :high]])"#).unwrap();

    let result = db.execute(r#"
        (query [:find ?x :where (eligible ?x)])
    "#).unwrap();

    let result_str = format!("{:?}", result);
    assert!(!result_str.contains("alice"), "alice has low score → rejected → not eligible");
    assert!(result_str.contains("bob"), "bob has high score → not rejected → eligible");
}

// ── Test 5: not combined with :as-of ─────────────────────────────────────────

#[test]
fn test_not_with_as_of_time_travel() {
    let db = in_memory_db();

    // tx 1: alice applied
    db.execute(r#"(transact [[:alice :applied true]])"#).unwrap();
    // tx 2: alice gets rejected
    db.execute(r#"(transact [[:alice :rejected true]])"#).unwrap();

    // As of tx 1, alice was not yet rejected → eligible
    let result_tx1 = db.execute(r#"
        (query [:find ?x
                :as-of 1
                :where [?x :applied true]
                       (not [?x :rejected true])])
    "#).unwrap();
    let r1 = format!("{:?}", result_tx1);
    assert!(r1.contains("alice"), "at tx1 alice was not yet rejected");

    // As of tx 2, alice is rejected → not eligible
    let result_tx2 = db.execute(r#"
        (query [:find ?x
                :as-of 2
                :where [?x :applied true]
                       (not [?x :rejected true])])
    "#).unwrap();
    let r2 = format!("{:?}", result_tx2);
    assert!(!r2.contains("alice"), "at tx2 alice is rejected");
}

// ── Test 6: not combined with :valid-at ──────────────────────────────────────

#[test]
fn test_not_with_valid_at() {
    let db = in_memory_db();

    // alice employed 2023, banned from 2024
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2025-01-01"}
                            [[:alice :employed true]])"#).unwrap();
    db.execute(r#"(transact {:valid-from "2024-01-01"}
                            [[:alice :banned true]])"#).unwrap();

    // In 2023, alice was employed and not yet banned
    let result_2023 = db.execute(r#"
        (query [:find ?x
                :valid-at "2023-06-01"
                :where [?x :employed true]
                       (not [?x :banned true])])
    "#).unwrap();
    let r = format!("{:?}", result_2023);
    assert!(r.contains("alice"), "in 2023 alice was not banned");

    // In 2024, alice is both employed and banned → excluded
    let result_2024 = db.execute(r#"
        (query [:find ?x
                :valid-at "2024-06-01"
                :where [?x :employed true]
                       (not [?x :banned true])])
    "#).unwrap();
    let r2 = format!("{:?}", result_2024);
    assert!(!r2.contains("alice"), "in 2024 alice is banned");
}

// ── Test 7: Negative cycle at rule registration → error ──────────────────────

#[test]
fn test_negative_cycle_rejected_at_registration() {
    let db = in_memory_db();

    // Register first rule fine
    db.execute(r#"(rule [(p ?x) [?x :base true] (not (q ?x))])"#).unwrap();

    // Second rule creates negative cycle
    let result = db.execute(r#"(rule [(q ?x) [?x :base true] (not (p ?x))])"#);
    assert!(result.is_err(), "negative cycle must be rejected");

    // q must not be registered
    let query_result = db.execute(r#"
        (query [:find ?x :where (q ?x)])
    "#);
    // Either returns empty or errors (predicate unknown) — either is acceptable
    // but it must NOT panic
    let _ = query_result;
}

// ── Test 8: Recursive rule + not coexist for different predicates ─────────────

#[test]
fn test_recursive_rule_and_not_coexist() {
    let db = in_memory_db();

    // reachable is recursive (positive)
    db.execute(r#"(rule [(reachable ?a ?b) [?a :connected ?b]])"#).unwrap();
    db.execute(r#"(rule [(reachable ?a ?b) [?a :connected ?m] (reachable ?m ?b)])"#).unwrap();

    // blocked uses not on a base fact
    db.execute(r#"(rule [(accessible ?a ?b)
                         (reachable ?a ?b)
                         (not [?b :blocked true])])"#).unwrap();

    db.execute(r#"(transact [[:a :connected :b]
                              [:b :connected :c]
                              [:c :blocked true]])"#).unwrap();

    let result = db.execute(r#"
        (query [:find ?b :where (accessible :a ?b)])
    "#).unwrap();

    let r = format!("{:?}", result);
    assert!(r.contains("b") || r.contains(":b"), "b is reachable and not blocked");
    assert!(!r.contains(":c"), "c is blocked");
}

// ── Test 9: not in a rule body (rule-level) ───────────────────────────────────

#[test]
fn test_not_in_rule_body() {
    let db = in_memory_db();

    db.execute(r#"(rule [(safe ?x) [?x :checked true] (not [?x :flagged true])])"#).unwrap();

    db.execute(r#"(transact [[:a :checked true]
                              [:b :checked true]
                              [:b :flagged true]])"#).unwrap();

    let result = db.execute(r#"
        (query [:find ?x :where (safe ?x)])
    "#).unwrap();

    let r = format!("{:?}", result);
    assert!(r.contains(":a") || r.contains("a"), ":a is safe");
    assert!(!r.contains(":b"), ":b is flagged");
}

// ── Test 10: Safety violation → parse error ───────────────────────────────────

#[test]
fn test_safety_violation_unbound_variable_in_not() {
    let db = in_memory_db();

    // ?y is only in (not ...), never in an outer clause
    let result = db.execute(r#"
        (query [:find ?x
                :where [?x :a ?v]
                       (not [?y :banned true])])
    "#);

    assert!(result.is_err(), "unbound variable in not should be a parse error");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("not bound") || msg.contains("unbound"),
            "error should mention unbound variable, got: {msg}");
}
```

- [ ] **Step 2: Run integration tests to confirm the file compiles and most pass**

```bash
cargo test --test negation_test 2>&1 | tail -30
```

Expected: most tests pass. Investigate and fix any failures before proceeding.

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass (331 + new negation tests).

- [ ] **Step 4: Commit**

```bash
git add tests/negation_test.rs
git commit -m "test(negation): add 10 integration tests for Phase 7.1a stratified negation"
```

---

### Final verification

- [ ] **Run full test suite one last time**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -30
```

Fix any warnings before declaring done.

- [ ] **Final commit (if clippy fixes needed)**

```bash
git add -p
git commit -m "fix(clippy): resolve warnings after Phase 7.1a implementation"
```
