use anyhow::Result;
use std::collections::HashMap;

use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::WhereClause;

// ── Helpers ───────────────────────────────────────────────────────────────────

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

// ── Structs ───────────────────────────────────────────────────────────────────

pub struct DependencyGraph {
    /// head_predicate → Vec<(dependency_predicate, is_negative)>
    edges: HashMap<String, Vec<(String, bool)>>,
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

    pub fn stratify(&self) -> Result<HashMap<String, usize>> {
        // Collect all predicates (both heads and dependencies)
        let mut all_predicates: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for (head, deps) in &self.edges {
            all_predicates.insert(head.clone());
            for (dep, _) in deps {
                all_predicates.insert(dep.clone());
            }
        }
        let n = all_predicates.len();

        let mut strata: HashMap<String, usize> =
            all_predicates.into_iter().map(|p| (p, 0)).collect();

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
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{EdnValue, Pattern, Rule};

    fn make_registry_with_rules(rules: Vec<(&str, Rule)>) -> RuleRegistry {
        let mut registry = RuleRegistry::new();
        for (predicate, rule) in rules {
            registry.register_rule_unchecked(predicate.to_string(), rule);
        }
        registry
    }

    fn positive_rule(head_pred: &str, dep_pred: &str) -> (&'static str, Rule) {
        // (head_pred ?x) :- (dep_pred ?x)
        let head = vec![
            EdnValue::Symbol(head_pred.to_string()),
            EdnValue::Symbol("?x".to_string()),
        ];
        let body = vec![WhereClause::RuleInvocation {
            predicate: dep_pred.to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }];
        (
            Box::leak(head_pred.to_string().into_boxed_str()),
            Rule { head, body },
        )
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
        (
            Box::leak(head_pred.to_string().into_boxed_str()),
            Rule { head, body },
        )
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
        (
            Box::leak(head_pred.to_string().into_boxed_str()),
            Rule { head, body },
        )
    }

    #[test]
    fn test_positive_only_rules_all_stratum_zero() {
        // p depends positively on q; both at stratum 0
        let registry = make_registry_with_rules(vec![positive_rule("p", "q"), base_rule("q")]);
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
        // eligible →⁻ rejected →⁺ base_fact
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
        let registry =
            make_registry_with_rules(vec![negative_rule("p", "q"), negative_rule("q", "p")]);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_err());
    }

    #[test]
    fn test_self_negative_cycle_returns_error() {
        // p →⁻ p
        let registry = make_registry_with_rules(vec![negative_rule("p", "p")]);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_err());
    }

    #[test]
    fn test_disconnected_predicates_stratum_zero() {
        let registry = make_registry_with_rules(vec![base_rule("foo"), base_rule("bar")]);
        let graph = DependencyGraph::from_rules(&registry);
        let strata = graph.stratify().unwrap();
        assert_eq!(*strata.get("foo").unwrap_or(&0), 0);
        assert_eq!(*strata.get("bar").unwrap_or(&0), 0);
    }

    #[test]
    fn test_not_join_creates_negative_dependency_edge() {
        // Rule: (eligible ?x) :- [?x :applied true], (not-join [?x] (blocked ?x))
        // => negative edge: eligible -> blocked
        use crate::query::datalog::types::{EdnValue, Pattern, Rule, WhereClause};

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
}

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
            vec![
                EdnValue::Symbol("p".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
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
        assert!(
            strata.is_ok(),
            "or with positive rule invocations should stratify"
        );
    }

    #[test]
    fn test_from_rules_or_pattern_only_no_neg_dep() {
        // Or branch with only a Pattern — no rule invocations → no deps
        let mut registry = RuleRegistry::new();
        let rule = Rule::new(
            vec![
                EdnValue::Symbol("p".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            vec![WhereClause::Or(vec![vec![WhereClause::Pattern(
                Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":status".to_string()),
                    EdnValue::Keyword(":active".to_string()),
                ),
            )]])],
        );
        registry.register_rule_unchecked("p".to_string(), rule);
        let graph = DependencyGraph::from_rules(&registry);
        assert!(graph.stratify().is_ok());
    }
}
