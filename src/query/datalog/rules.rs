/// Rule registry for storing and retrieving Datalog rules.
///
/// Rules are indexed by their predicate name (head). Multiple rules can share
/// the same predicate (for defining base cases and recursive cases separately).
use crate::query::datalog::stratification::DependencyGraph;
use crate::query::datalog::types::Rule;
use anyhow::Result;
use std::collections::HashMap;

/// In-memory registry for Datalog rules.
///
/// # Examples
/// ```
/// use minigraf::query::datalog::rules::RuleRegistry;
/// use minigraf::query::datalog::types::Rule;
///
/// let mut registry = RuleRegistry::new();
///
/// // Register a rule for the 'reachable' predicate
/// // registry.register_rule("reachable".to_string(), rule)?;
///
/// // Retrieve all rules for 'reachable'
/// let rules = registry.get_rules("reachable");
/// ```
#[derive(Debug, Clone)]
pub struct RuleRegistry {
    /// Map from predicate name to list of rules
    /// Multiple rules can have the same head predicate (e.g., base case + recursive case)
    rules: HashMap<String, Vec<Rule>>,
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleRegistry {
    /// Create a new empty rule registry.
    pub fn new() -> Self {
        RuleRegistry {
            rules: HashMap::new(),
        }
    }

    /// Register a rule under the given predicate name.
    ///
    /// Multiple rules can be registered for the same predicate.
    /// This allows defining base cases and recursive cases separately.
    ///
    /// # Arguments
    /// * `predicate` - The predicate name (e.g., "reachable", "ancestor")
    /// * `rule` - The rule to register
    ///
    /// # Examples
    /// ```ignore
    /// let mut registry = RuleRegistry::new();
    /// registry.register_rule("reachable".to_string(), base_rule)?;
    /// registry.register_rule("reachable".to_string(), recursive_rule)?;
    /// ```
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

    /// Get all rules for a given predicate.
    ///
    /// Returns an empty vector if no rules are registered for the predicate.
    ///
    /// # Arguments
    /// * `predicate` - The predicate name to query
    ///
    /// # Returns
    /// A vector of rules (may be empty if predicate not found)
    pub fn get_rules(&self, predicate: &str) -> Vec<Rule> {
        self.rules.get(predicate).cloned().unwrap_or_default()
    }

    /// Check if any rules are registered for a predicate.
    ///
    /// # Arguments
    /// * `predicate` - The predicate name to check
    ///
    /// # Returns
    /// `true` if at least one rule is registered, `false` otherwise
    pub fn has_rule(&self, predicate: &str) -> bool {
        self.rules.contains_key(predicate)
    }

    /// Get the total number of rules registered across all predicates.
    pub fn rule_count(&self) -> usize {
        self.rules.values().map(|v| v.len()).sum()
    }

    /// Get the number of unique predicates with registered rules.
    pub fn predicate_count(&self) -> usize {
        self.rules.len()
    }

    /// Clear all rules from the registry.
    ///
    /// This is primarily useful for testing.
    pub fn clear(&mut self) {
        self.rules.clear();
    }

    /// Get all predicate names that have registered rules.
    pub fn predicate_names(&self) -> Vec<String> {
        self.rules.keys().cloned().collect()
    }

    /// Register a rule without stratification checks.
    /// Used only in tests and internally before the stratification module is wired.
    #[allow(dead_code)]
    pub(crate) fn register_rule_unchecked(&mut self, predicate: String, rule: Rule) {
        self.rules.entry(predicate).or_default().push(rule);
    }

    /// Iterate all (predicate, rules) pairs.
    pub fn all_rules(&self) -> impl Iterator<Item = (&str, &[Rule])> {
        self.rules.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::EdnValue;

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

    #[test]
    fn test_rule_registry_new() {
        let registry = RuleRegistry::new();
        assert_eq!(registry.rule_count(), 0);
        assert_eq!(registry.predicate_count(), 0);
    }

    #[test]
    fn test_register_single_rule() {
        let mut registry = RuleRegistry::new();
        let rule = create_test_rule("reachable");

        registry
            .register_rule("reachable".to_string(), rule)
            .unwrap();

        assert_eq!(registry.rule_count(), 1);
        assert_eq!(registry.predicate_count(), 1);
        assert!(registry.has_rule("reachable"));
    }

    #[test]
    fn test_register_multiple_rules_same_predicate() {
        let mut registry = RuleRegistry::new();

        // Register two rules for the same predicate (e.g., base case + recursive case)
        let base_rule = create_test_rule("reachable");
        let recursive_rule = create_test_rule("reachable");

        registry
            .register_rule("reachable".to_string(), base_rule)
            .unwrap();
        registry
            .register_rule("reachable".to_string(), recursive_rule)
            .unwrap();

        assert_eq!(registry.rule_count(), 2);
        assert_eq!(registry.predicate_count(), 1); // Still just one predicate
        assert_eq!(registry.get_rules("reachable").len(), 2);
    }

    #[test]
    fn test_register_rules_different_predicates() {
        let mut registry = RuleRegistry::new();

        let rule1 = create_test_rule("reachable");
        let rule2 = create_test_rule("ancestor");

        registry
            .register_rule("reachable".to_string(), rule1)
            .unwrap();
        registry
            .register_rule("ancestor".to_string(), rule2)
            .unwrap();

        assert_eq!(registry.rule_count(), 2);
        assert_eq!(registry.predicate_count(), 2);
        assert!(registry.has_rule("reachable"));
        assert!(registry.has_rule("ancestor"));
    }

    #[test]
    fn test_get_rules_empty() {
        let registry = RuleRegistry::new();
        let rules = registry.get_rules("nonexistent");
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn test_get_rules_returns_all() {
        let mut registry = RuleRegistry::new();

        let rule1 = create_test_rule("reachable");
        let rule2 = create_test_rule("reachable");
        let rule3 = create_test_rule("reachable");

        registry
            .register_rule("reachable".to_string(), rule1)
            .unwrap();
        registry
            .register_rule("reachable".to_string(), rule2)
            .unwrap();
        registry
            .register_rule("reachable".to_string(), rule3)
            .unwrap();

        let rules = registry.get_rules("reachable");
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn test_has_rule() {
        let mut registry = RuleRegistry::new();

        assert!(!registry.has_rule("reachable"));

        let rule = create_test_rule("reachable");
        registry
            .register_rule("reachable".to_string(), rule)
            .unwrap();

        assert!(registry.has_rule("reachable"));
        assert!(!registry.has_rule("ancestor"));
    }

    #[test]
    fn test_clear() {
        let mut registry = RuleRegistry::new();

        let rule1 = create_test_rule("reachable");
        let rule2 = create_test_rule("ancestor");

        registry
            .register_rule("reachable".to_string(), rule1)
            .unwrap();
        registry
            .register_rule("ancestor".to_string(), rule2)
            .unwrap();

        assert_eq!(registry.rule_count(), 2);

        registry.clear();

        assert_eq!(registry.rule_count(), 0);
        assert_eq!(registry.predicate_count(), 0);
        assert!(!registry.has_rule("reachable"));
        assert!(!registry.has_rule("ancestor"));
    }

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

    #[test]
    fn test_predicate_names() {
        let mut registry = RuleRegistry::new();

        let rule1 = create_test_rule("reachable");
        let rule2 = create_test_rule("ancestor");
        let rule3 = create_test_rule("reachable"); // Duplicate predicate

        registry
            .register_rule("reachable".to_string(), rule1)
            .unwrap();
        registry
            .register_rule("ancestor".to_string(), rule2)
            .unwrap();
        registry
            .register_rule("reachable".to_string(), rule3)
            .unwrap();

        let mut names = registry.predicate_names();
        names.sort(); // HashMap order is not guaranteed

        assert_eq!(names.len(), 2);
        assert!(names.contains(&"reachable".to_string()));
        assert!(names.contains(&"ancestor".to_string()));
    }
}
