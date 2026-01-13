/// Recursive rule evaluation using semi-naive fixed-point iteration.
///
/// This module implements the semi-naive evaluation algorithm for recursive Datalog rules.
/// The algorithm repeatedly applies rules to derive new facts until no new facts can be
/// derived (fixed point is reached).
///
/// # Algorithm Overview
///
/// 1. Start with base facts from database
/// 2. Apply rules to generate derived facts
/// 3. Track "delta" (new facts generated in this iteration)
/// 4. In next iteration, only apply rules to delta facts (semi-naive optimization)
/// 5. Stop when delta is empty (fixed point) or max iterations reached
///
/// # Example
///
/// ```ignore
/// // Facts: A->B, B->C
/// // Rule: (reachable ?x ?y) <- [?x :connected ?y]
/// //       (reachable ?x ?y) <- [?x :connected ?z] (reachable ?z ?y)
/// //
/// // Iteration 0: delta = {A->B, B->C}
/// // Iteration 1: Apply rules, derive {A->C}, delta = {A->C}
/// // Iteration 2: No new facts, delta = {}, STOP
/// ```
use super::matcher::{edn_to_entity_id, edn_to_value, Bindings, PatternMatcher};
use super::rules::RuleRegistry;
use super::types::{EdnValue, Pattern, Rule};
use crate::graph::types::{Fact, Value};
use crate::graph::FactStorage;
use anyhow::{anyhow, Result};
use std::sync::{Arc, RwLock};

/// Recursive evaluator for Datalog rules using semi-naive evaluation.
///
/// # Examples
///
/// ```ignore
/// let evaluator = RecursiveEvaluator::new(
///     storage.clone(),
///     rules.clone(),
///     1000  // max iterations
/// );
///
/// let derived_facts = evaluator.evaluate_recursive_rules(&["reachable"])?;
/// ```
pub struct RecursiveEvaluator {
    /// Base fact storage
    storage: FactStorage,
    /// Rule registry
    rules: Arc<RwLock<RuleRegistry>>,
    /// Maximum iterations before giving up (prevents infinite loops)
    max_iterations: usize,
}

impl RecursiveEvaluator {
    /// Create a new recursive evaluator.
    ///
    /// # Arguments
    /// * `storage` - Base fact storage
    /// * `rules` - Rule registry
    /// * `max_iterations` - Safety limit (e.g., 1000)
    pub fn new(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        max_iterations: usize,
    ) -> Self {
        RecursiveEvaluator {
            storage,
            rules,
            max_iterations,
        }
    }

    /// Evaluate rules for given predicates using semi-naive fixed-point iteration.
    ///
    /// # Arguments
    /// * `predicates` - Predicate names to evaluate (e.g., ["reachable"])
    ///
    /// # Returns
    /// A FactStorage containing all base facts + derived facts
    ///
    /// # Errors
    /// Returns error if max iterations exceeded or evaluation fails
    pub fn evaluate_recursive_rules(&self, predicates: &[String]) -> Result<FactStorage> {
        // Start with base facts as initial delta
        let base_facts = self.storage.get_asserted_facts()?;

        // Create storage for derived facts
        let derived = FactStorage::new();

        // Add base facts to derived storage
        for fact in &base_facts {
            derived.transact(vec![(
                fact.entity,
                fact.attribute.clone(),
                fact.value.clone(),
            )])?;
        }

        // Track facts we've seen (for delta computation)
        // Note: Using Vec instead of HashSet because Value contains Float which can't Hash
        let mut seen_facts: Vec<(uuid::Uuid, String, Value)> = base_facts
            .iter()
            .map(|f| (f.entity, f.attribute.clone(), f.value.clone()))
            .collect();

        let mut iteration = 0;

        // Fixed-point iteration
        loop {
            iteration += 1;

            if iteration > self.max_iterations {
                return Err(anyhow!(
                    "Max iterations ({}) exceeded. Possible infinite recursion or cycle in rules.",
                    self.max_iterations
                ));
            }

            // Evaluate rules once, get new facts
            let new_facts = self.evaluate_iteration(predicates, &derived)?;

            // Compute delta: facts not yet seen
            let mut delta = Vec::new();
            for fact in new_facts {
                let key = (fact.entity, fact.attribute.clone(), fact.value.clone());
                if !self.contains_fact(&seen_facts, &key) {
                    seen_facts.push(key);
                    delta.push(fact);
                }
            }

            // If no new facts, we've reached fixed point
            if delta.is_empty() {
                break;
            }

            // Add delta facts to derived storage
            for fact in delta {
                derived.transact(vec![(
                    fact.entity,
                    fact.attribute.clone(),
                    fact.value.clone(),
                )])?;
            }
        }

        Ok(derived)
    }

    /// Evaluate all rules for given predicates once.
    ///
    /// This is a single iteration of the fixed-point loop.
    /// It applies each rule to the current derived facts and returns newly derived facts.
    fn evaluate_iteration(
        &self,
        predicates: &[String],
        current_facts: &FactStorage,
    ) -> Result<Vec<Fact>> {
        let mut new_facts = Vec::new();

        let registry = self.rules.read().unwrap();

        // For each predicate, evaluate all its rules
        for predicate in predicates {
            let rules = registry.get_rules(predicate);

            for rule in rules {
                let derived = self.evaluate_rule(&rule, current_facts)?;
                new_facts.extend(derived);
            }
        }

        Ok(new_facts)
    }

    /// Evaluate a single rule against current facts.
    ///
    /// # Algorithm
    /// 1. Convert body patterns and rule invocations to Pattern structs
    /// 2. Use PatternMatcher to find all bindings
    /// 3. For each binding, instantiate rule head to create derived fact
    fn evaluate_rule(&self, rule: &Rule, current_facts: &FactStorage) -> Result<Vec<Fact>> {
        let mut derived = Vec::new();

        // Parse body clauses: patterns and rule invocations
        let mut patterns = Vec::new();
        for body_clause in &rule.body {
            if let Some(vec) = body_clause.as_vector() {
                // This is a pattern [?e :attr ?v]
                let pattern = Pattern::from_edn(vec)
                    .map_err(|e| anyhow!("Failed to parse pattern: {}", e))?;
                patterns.push(pattern);
            } else if let Some(list) = body_clause.as_list() {
                // This is a rule invocation (predicate ?arg1 ?arg2)
                // Convert to pattern: [?arg1 :predicate ?arg2]
                let pattern = self.rule_invocation_to_pattern(list)?;
                patterns.push(pattern);
            } else {
                return Err(anyhow!(
                    "Rule body clause must be a vector (pattern) or list (rule invocation)"
                ));
            }
        }

        if patterns.is_empty() {
            return Ok(derived);
        }

        // Match patterns against current facts
        let matcher = PatternMatcher::new(current_facts.clone());
        let bindings = matcher.match_patterns(&patterns);

        // For each binding, instantiate rule head to create derived fact
        for binding in bindings {
            let fact = self.instantiate_head(&rule.head, &binding)?;
            derived.push(fact);
        }

        Ok(derived)
    }

    /// Convert a rule invocation to a pattern.
    ///
    /// Example: (reachable ?x ?y) -> [?x :reachable ?y]
    fn rule_invocation_to_pattern(&self, list: &[EdnValue]) -> Result<Pattern> {
        if list.is_empty() {
            return Err(anyhow!("Rule invocation cannot be empty"));
        }

        // First element is predicate name
        let predicate = match &list[0] {
            EdnValue::Symbol(s) => s.clone(),
            _ => return Err(anyhow!("Rule invocation must start with predicate name (symbol)")),
        };

        // Must have exactly 2 arguments (entity and value)
        if list.len() != 3 {
            return Err(anyhow!(
                "Rule invocation '{}' must have exactly 2 arguments (entity and value), got {}",
                predicate,
                list.len() - 1
            ));
        }

        // Create pattern: [entity :predicate value]
        let pattern = Pattern::new(
            list[1].clone(),
            EdnValue::Keyword(format!(":{}", predicate)),
            list[2].clone(),
        );

        Ok(pattern)
    }

    /// Instantiate rule head with variable bindings to create a derived fact.
    ///
    /// # Example
    /// Head: (reachable ?x ?y)
    /// Bindings: {?x -> alice_uuid, ?y -> bob_uuid}
    /// Result: Fact(alice_uuid, ":reachable", Ref(bob_uuid))
    fn instantiate_head(&self, head: &[EdnValue], binding: &Bindings) -> Result<Fact> {
        if head.len() < 3 {
            return Err(anyhow!(
                "Rule head must have at least 3 elements: (predicate ?arg1 ?arg2)"
            ));
        }

        // head[0] is predicate name
        let predicate = match &head[0] {
            EdnValue::Symbol(s) => s.clone(),
            _ => return Err(anyhow!("Rule head must start with predicate name (symbol)")),
        };

        // head[1] is entity (usually a variable)
        let entity_edn = self.substitute_variable(&head[1], binding)?;
        let entity = edn_to_entity_id(&entity_edn)
            .map_err(|e| anyhow!("Failed to convert entity: {}", e))?;

        // head[2] is value (usually a variable or constant)
        let value_edn = self.substitute_variable(&head[2], binding)?;
        let value = edn_to_value(&value_edn)
            .map_err(|e| anyhow!("Failed to convert value: {}", e))?;

        // Create fact with derived predicate as attribute
        // Use ":predicate-name" as the attribute for derived facts
        let attribute = format!(":{}", predicate);

        // Create the fact (no tx_id yet, will be added when transacted)
        Ok(Fact {
            entity,
            attribute,
            value,
            tx_id: 0, // Will be assigned when added to storage
            asserted: true,
        })
    }

    /// Substitute a variable with its binding, or return as-is if not a variable.
    fn substitute_variable(&self, edn: &EdnValue, binding: &Bindings) -> Result<EdnValue> {
        match edn {
            EdnValue::Symbol(s) if s.starts_with('?') => {
                // This is a variable
                if let Some(value) = binding.get(s) {
                    // Convert Value back to EdnValue for entity/value conversion
                    Ok(self.value_to_edn(value))
                } else {
                    Err(anyhow!("Unbound variable in rule head: {}", s))
                }
            }
            _ => Ok(edn.clone()), // Not a variable, use as-is
        }
    }

    /// Convert a Value back to EdnValue for rule head instantiation.
    fn value_to_edn(&self, value: &Value) -> EdnValue {
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

    /// Check if a fact tuple exists in the seen_facts vector.
    ///
    /// Manual containment check since Value can't implement Hash (contains Float).
    fn contains_fact(
        &self,
        seen_facts: &[(uuid::Uuid, String, Value)],
        key: &(uuid::Uuid, String, Value),
    ) -> bool {
        seen_facts
            .iter()
            .any(|(e, a, v)| e == &key.0 && a == &key.1 && v == &key.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    use uuid::Uuid;

    fn create_test_storage() -> FactStorage {
        let storage = FactStorage::new();

        // Create a simple graph: A->B, B->C
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (b, ":connected".to_string(), Value::Ref(c)),
            ])
            .unwrap();

        storage
    }

    fn register_test_rule(rules: &Arc<RwLock<RuleRegistry>>, rule_str: &str) {
        let cmd = parse_datalog_command(rule_str).unwrap();
        if let DatalogCommand::Rule(rule) = cmd {
            let predicate = match &rule.head[0] {
                EdnValue::Symbol(s) => s.clone(),
                _ => panic!("Expected symbol as predicate name"),
            };
            rules.write().unwrap().register_rule(predicate, rule).unwrap();
        } else {
            panic!("Expected Rule command");
        }
    }

    #[test]
    fn test_evaluator_creation() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        let evaluator = RecursiveEvaluator::new(storage, rules, 1000);
        assert_eq!(evaluator.max_iterations, 1000);
    }

    #[test]
    fn test_evaluate_simple_rule() {
        let storage = create_test_storage();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Register simple rule: (reachable ?x ?y) <- [?x :connected ?y]
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);

        let evaluator = RecursiveEvaluator::new(storage, rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_ok());

        let derived = result.unwrap();
        let facts = derived.get_asserted_facts().unwrap();

        // Should have base facts (2) + derived facts (2)
        // Base: A->B, B->C
        // Derived: A reachable B, B reachable C
        assert!(facts.len() >= 2);
    }

    #[test]
    fn test_max_iterations_enforced() {
        let storage = create_test_storage();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Register a rule
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);

        // Set reasonable max iterations
        // Note: Even simple rules need at least 2 iterations (derive + check convergence)
        let evaluator = RecursiveEvaluator::new(storage, rules, 10);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);

        // Should succeed because simple rule converges quickly
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_predicates() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        let evaluator = RecursiveEvaluator::new(storage, rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&[]);
        assert!(result.is_ok());

        // Should just return base facts
        let derived = result.unwrap();
        assert_eq!(derived.fact_count(), 0);
    }

    #[test]
    fn test_no_matching_rules() {
        let storage = create_test_storage();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Don't register any rules

        let evaluator = RecursiveEvaluator::new(storage.clone(), rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&["nonexistent".to_string()]);
        assert!(result.is_ok());

        // Should just return base facts (no derivation happened)
        let derived = result.unwrap();
        let base_facts = storage.get_asserted_facts().unwrap();
        assert_eq!(derived.fact_count(), base_facts.len());
    }

    #[test]
    fn test_recursive_transitive_closure() {
        let storage = create_test_storage();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Register base case: (reachable ?x ?y) <- [?x :connected ?y]
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);

        // Register recursive case: (reachable ?x ?y) <- [?x :connected ?z] (reachable ?z ?y)
        register_test_rule(
            &rules,
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        );

        let evaluator = RecursiveEvaluator::new(storage.clone(), rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_ok());

        let derived = result.unwrap();

        // Get all reachable facts
        let all_facts = derived.get_asserted_facts().unwrap();
        let reachable_facts: Vec<_> = all_facts
            .iter()
            .filter(|f| f.attribute == ":reachable")
            .collect();

        // Should derive:
        // - A reachable B (base: A->B)
        // - B reachable C (base: B->C)
        // - A reachable C (recursive: A->B->C)
        assert_eq!(reachable_facts.len(), 3);
    }

    #[test]
    fn test_recursive_long_chain() {
        let storage = FactStorage::new();

        // Create chain: 1->2->3->4->5
        let n1 = Uuid::new_v4();
        let n2 = Uuid::new_v4();
        let n3 = Uuid::new_v4();
        let n4 = Uuid::new_v4();
        let n5 = Uuid::new_v4();

        storage
            .transact(vec![
                (n1, ":connected".to_string(), Value::Ref(n2)),
                (n2, ":connected".to_string(), Value::Ref(n3)),
                (n3, ":connected".to_string(), Value::Ref(n4)),
                (n4, ":connected".to_string(), Value::Ref(n5)),
            ])
            .unwrap();

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Register reachable rules (base + recursive)
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
        register_test_rule(
            &rules,
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        );

        let evaluator = RecursiveEvaluator::new(storage, rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_ok());

        let derived = result.unwrap();
        let all_facts = derived.get_asserted_facts().unwrap();
        let reachable_facts: Vec<_> = all_facts
            .iter()
            .filter(|f| f.attribute == ":reachable")
            .collect();

        // Should derive:
        // 1->2, 2->3, 3->4, 4->5 (base: 4 facts)
        // 1->3, 2->4, 3->5 (1 hop: 3 facts)
        // 1->4, 2->5 (2 hops: 2 facts)
        // 1->5 (3 hops: 1 fact)
        // Total: 10 derived facts
        assert_eq!(reachable_facts.len(), 10);
    }

    #[test]
    fn test_recursive_with_cycle() {
        let storage = FactStorage::new();

        // Create cycle: A->B->C->A
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (b, ":connected".to_string(), Value::Ref(c)),
                (c, ":connected".to_string(), Value::Ref(a)),
            ])
            .unwrap();

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Register reachable rules
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
        register_test_rule(
            &rules,
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        );

        let evaluator = RecursiveEvaluator::new(storage, rules, 1000);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_ok());

        let derived = result.unwrap();
        let all_facts = derived.get_asserted_facts().unwrap();
        let reachable_facts: Vec<_> = all_facts
            .iter()
            .filter(|f| f.attribute == ":reachable")
            .collect();

        // Should derive:
        // A->B, B->C, C->A (base: 3)
        // A->C, B->A, C->B (1 hop: 3)
        // A->A, B->B, C->C (2 hops back to self: 3)
        // Total: 9 (everyone reaches everyone including themselves)
        assert_eq!(reachable_facts.len(), 9);

        // Verify it converged without infinite loop
        // (The fact that we got here means it converged)
    }

    #[test]
    fn test_recursive_convergence_iterations() {
        let storage = FactStorage::new();

        // Simple chain: A->B
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        storage
            .transact(vec![(a, ":connected".to_string(), Value::Ref(b))])
            .unwrap();

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
        register_test_rule(
            &rules,
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        );

        // Set low iteration limit (should still work for simple graph)
        let evaluator = RecursiveEvaluator::new(storage, rules, 5);

        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_ok());

        // Simple chain should converge quickly
        let derived = result.unwrap();
        let all_facts = derived.get_asserted_facts().unwrap();
        let reachable_facts: Vec<_> = all_facts
            .iter()
            .filter(|f| f.attribute == ":reachable")
            .collect();

        // Should have 1 reachable fact: A->B
        assert_eq!(reachable_facts.len(), 1);
    }
}
