use super::evaluator::RecursiveEvaluator;
use super::matcher::{edn_to_entity_id, edn_to_value, PatternMatcher};
use super::rules::RuleRegistry;
use super::types::{DatalogCommand, DatalogQuery, EdnValue, Pattern, Rule, Transaction};
use crate::graph::types::{TxId, Value};
use crate::graph::FactStorage;
use anyhow::{anyhow, Result};
use std::sync::{Arc, RwLock};

/// Result of executing a Datalog query
#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    /// Transaction completed successfully with TX ID
    Transacted(TxId),
    /// Retraction completed successfully with TX ID
    Retracted(TxId),
    /// Query results: list of variable bindings
    QueryResults {
        vars: Vec<String>,
        results: Vec<Vec<Value>>,
    },
    /// Empty result (e.g., for future rule definitions)
    Ok,
}

/// Executor for Datalog commands
pub struct DatalogExecutor {
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
}

impl DatalogExecutor {
    pub fn new(storage: FactStorage) -> Self {
        DatalogExecutor {
            storage,
            rules: Arc::new(RwLock::new(RuleRegistry::new())),
        }
    }

    /// Execute a Datalog command
    pub fn execute(&self, command: DatalogCommand) -> Result<QueryResult> {
        match command {
            DatalogCommand::Transact(tx) => self.execute_transact(tx),
            DatalogCommand::Retract(tx) => self.execute_retract(tx),
            DatalogCommand::Query(query) => self.execute_query(query),
            DatalogCommand::Rule(rule) => self.execute_rule(rule),
        }
    }

    /// Execute a transact command: add facts to storage
    fn execute_transact(&self, tx: Transaction) -> Result<QueryResult> {
        let mut fact_tuples = Vec::new();

        for pattern in tx.facts {
            let entity_id = edn_to_entity_id(&pattern.entity)
                .map_err(|e| anyhow!("Invalid entity: {}", e))?;

            let attribute = match &pattern.attribute {
                EdnValue::Keyword(k) => k.clone(),
                _ => return Err(anyhow!("Attribute must be a keyword")),
            };

            let value =
                edn_to_value(&pattern.value).map_err(|e| anyhow!("Invalid value: {}", e))?;

            fact_tuples.push((entity_id, attribute, value));
        }

        let tx_id = self
            .storage
            .transact(fact_tuples, None)
            .map_err(|e| anyhow!("Transaction failed: {}", e))?;

        Ok(QueryResult::Transacted(tx_id))
    }

    /// Execute a retract command: retract facts from storage
    fn execute_retract(&self, tx: Transaction) -> Result<QueryResult> {
        let mut fact_tuples = Vec::new();

        for pattern in tx.facts {
            let entity_id = edn_to_entity_id(&pattern.entity)
                .map_err(|e| anyhow!("Invalid entity: {}", e))?;

            let attribute = match &pattern.attribute {
                EdnValue::Keyword(k) => k.clone(),
                _ => return Err(anyhow!("Attribute must be a keyword")),
            };

            let value =
                edn_to_value(&pattern.value).map_err(|e| anyhow!("Invalid value: {}", e))?;

            fact_tuples.push((entity_id, attribute, value));
        }

        let tx_id = self
            .storage
            .retract(fact_tuples)
            .map_err(|e| anyhow!("Retraction failed: {}", e))?;

        Ok(QueryResult::Retracted(tx_id))
    }

    /// Execute a query: find matching facts and return specified variables
    fn execute_query(&self, query: DatalogQuery) -> Result<QueryResult> {
        // Check if query uses rules
        if query.uses_rules() {
            // Use RecursiveEvaluator for queries with rule invocations
            return self.execute_query_with_rules(query);
        }

        // Simple pattern-only query
        let matcher = PatternMatcher::new(self.storage.clone());
        let patterns = query.get_patterns();

        // Match all patterns and get bindings
        let bindings = matcher.match_patterns(&patterns);

        // Extract requested variables from bindings
        let mut results = Vec::new();
        for binding in bindings {
            let mut row = Vec::new();
            for var in &query.find {
                if let Some(value) = binding.get(var) {
                    row.push(value.clone());
                } else {
                    // Variable not bound in this result - skip this result
                    // (This can happen if the variable wasn't mentioned in patterns)
                    continue;
                }
            }
            if row.len() == query.find.len() {
                results.push(row);
            }
        }

        Ok(QueryResult::QueryResults {
            vars: query.find,
            results,
        })
    }

    /// Execute a query that uses recursive rules
    fn execute_query_with_rules(&self, query: DatalogQuery) -> Result<QueryResult> {
        // Extract predicates from rule invocations
        let rule_invocations = query.get_rule_invocations();
        let predicates: Vec<String> = rule_invocations
            .iter()
            .map(|(pred, _)| pred.clone())
            .collect();

        // Create evaluator and derive all facts for these predicates
        let evaluator = RecursiveEvaluator::new(
            self.storage.clone(),
            self.rules.clone(),
            1000, // max iterations
        );

        let derived_storage = evaluator.evaluate_recursive_rules(&predicates)?;

        // Convert rule invocations to patterns
        // (reachable ?x ?y) becomes [?x :reachable ?y]
        let mut all_patterns = query.get_patterns();

        for (predicate, args) in rule_invocations {
            if args.len() != 2 {
                return Err(anyhow!(
                    "Rule invocation '{}' must have exactly 2 arguments (entity and value), got {}",
                    predicate,
                    args.len()
                ));
            }

            // Create pattern: [entity :predicate value]
            let pattern = Pattern::new(
                args[0].clone(),
                EdnValue::Keyword(format!(":{}", predicate)),
                args[1].clone(),
            );
            all_patterns.push(pattern);
        }

        // Match all patterns against derived facts
        let matcher = PatternMatcher::new(derived_storage);
        let bindings = matcher.match_patterns(&all_patterns);

        // Extract requested variables from bindings
        let mut results = Vec::new();
        for binding in bindings {
            let mut row = Vec::new();
            for var in &query.find {
                if let Some(value) = binding.get(var) {
                    row.push(value.clone());
                } else {
                    continue;
                }
            }
            if row.len() == query.find.len() {
                results.push(row);
            }
        }

        Ok(QueryResult::QueryResults {
            vars: query.find,
            results,
        })
    }

    /// Execute a rule command: register the rule for later use
    fn execute_rule(&self, rule: Rule) -> Result<QueryResult> {
        // Extract predicate name from rule head
        // Head format: (predicate ?arg1 ?arg2 ...)
        let predicate = self.extract_predicate(&rule)?;

        // Register the rule
        self.rules
            .write()
            .unwrap()
            .register_rule(predicate, rule)?;

        Ok(QueryResult::Ok)
    }

    /// Extract the predicate name from a rule head
    fn extract_predicate(&self, rule: &Rule) -> Result<String> {
        if rule.head.is_empty() {
            return Err(anyhow!("Rule head cannot be empty"));
        }

        match &rule.head[0] {
            EdnValue::Symbol(s) => Ok(s.clone()),
            _ => Err(anyhow!(
                "Rule head must start with a symbol (predicate name)"
            )),
        }
    }

    /// Get the underlying storage (for testing)
    pub fn storage(&self) -> &FactStorage {
        &self.storage
    }

    /// Get the rule registry (for testing)
    #[cfg(test)]
    pub fn rules(&self) -> Arc<RwLock<RuleRegistry>> {
        self.rules.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::parser::parse_datalog_command;
    use uuid::Uuid;

    #[test]
    fn test_execute_transact() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        let cmd = parse_datalog_command(
            r#"(transact [[:alice :person/name "Alice"]
                          [:alice :person/age 30]])"#,
        )
        .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::Transacted(tx_id) => {
                assert!(tx_id > 0);
            }
            _ => panic!("Expected Transacted result"),
        }

        // Verify facts were added
        let facts = executor.storage().get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 2);
    }

    #[test]
    fn test_execute_simple_query() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Add some facts
        let alice_id = Uuid::new_v4();
        storage
            .transact(vec![
                (
                    alice_id,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                ),
                (alice_id, ":person/age".to_string(), Value::Integer(30)),
            ], None)
            .unwrap();

        // Query for name
        let cmd =
            parse_datalog_command(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
                .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?name"]);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String("Alice".to_string()));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_execute_multi_pattern_query() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Add some facts
        let alice_id = Uuid::new_v4();
        storage
            .transact(vec![
                (
                    alice_id,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                ),
                (alice_id, ":person/age".to_string(), Value::Integer(30)),
            ], None)
            .unwrap();

        // Query for both name and age
        let cmd = parse_datalog_command(
            r#"(query [:find ?name ?age
                       :where [?e :person/name ?name]
                              [?e :person/age ?age]])"#,
        )
        .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?name", "?age"]);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String("Alice".to_string()));
                assert_eq!(results[0][1], Value::Integer(30));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_execute_query_no_results() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Query with no matching facts
        let cmd =
            parse_datalog_command(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
                .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?name"]);
                assert_eq!(results.len(), 0);
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_execute_retract() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Add a fact
        let alice_id = Uuid::new_v4();
        storage
            .transact(vec![(
                alice_id,
                ":person/age".to_string(),
                Value::Integer(30),
            )], None)
            .unwrap();

        // Verify it exists
        let current_value = storage
            .get_current_value(&alice_id, &":person/age".to_string())
            .unwrap();
        assert_eq!(current_value, Some(Value::Integer(30)));

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(2));

        // Retract it using UUID-based entity reference
        let cmd = parse_datalog_command(
            format!(
                r#"(retract [[#uuid "{}" :person/age 30]])"#,
                alice_id.to_string()
            )
            .as_str(),
        )
        .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::Retracted(tx_id) => {
                assert!(tx_id > 0);
            }
            _ => panic!("Expected Retracted result"),
        }

        // Verify it's retracted (current value should be None)
        let current_value = storage
            .get_current_value(&alice_id, &":person/age".to_string())
            .unwrap();
        assert_eq!(current_value, None);
    }

    #[test]
    fn test_transact_with_keyword_entity() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Transact with keyword-based entity (will be converted to deterministic UUID)
        let cmd = parse_datalog_command(
            r#"(transact [[:alice :person/name "Alice"]
                          [:alice :person/age 30]])"#,
        )
        .unwrap();

        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::Transacted(_) => {}
            _ => panic!("Expected Transacted result"),
        }

        // Query to verify both facts share the same entity
        let query_cmd = parse_datalog_command(
            r#"(query [:find ?name ?age
                       :where [?e :person/name ?name]
                              [?e :person/age ?age]])"#,
        )
        .unwrap();

        let result = executor.execute(query_cmd).unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String("Alice".to_string()));
                assert_eq!(results[0][1], Value::Integer(30));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_register_rule() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Parse and execute a rule command
        let cmd = parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#,
        )
        .unwrap();

        let result = executor.execute(cmd).unwrap();
        assert_eq!(result, QueryResult::Ok);

        // Verify rule was registered
        let registry = executor.rules();
        let rules = registry.read().unwrap().get_rules("reachable");
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn test_register_multiple_rules_same_predicate() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Register base case
        let cmd1 = parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#,
        )
        .unwrap();
        executor.execute(cmd1).unwrap();

        // Register recursive case
        let cmd2 = parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        )
        .unwrap();
        executor.execute(cmd2).unwrap();

        // Verify both rules registered
        let registry = executor.rules();
        let rules = registry.read().unwrap().get_rules("reachable");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn test_register_rules_different_predicates() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Register reachable rule
        let cmd1 = parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#,
        )
        .unwrap();
        executor.execute(cmd1).unwrap();

        // Register ancestor rule
        let cmd2 = parse_datalog_command(
            r#"(rule [(ancestor ?a ?d) [?a :parent ?d]])"#,
        )
        .unwrap();
        executor.execute(cmd2).unwrap();

        // Verify both predicates have rules
        let registry = executor.rules();
        let reg_read = registry.read().unwrap();
        assert!(reg_read.has_rule("reachable"));
        assert!(reg_read.has_rule("ancestor"));
        assert_eq!(reg_read.predicate_count(), 2);
    }

    #[test]
    fn test_query_with_rule_invocation() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Create graph: A->B, A->C
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (a, ":connected".to_string(), Value::Ref(c)),
            ], None)
            .unwrap();

        // Register reachable rule (base case only - no recursion yet)
        let rule1 = parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#,
        )
        .unwrap();
        executor.execute(rule1).unwrap();

        // Query using rule invocation: find all nodes reachable from A
        let query_str = format!(
            r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
            a.to_string()
        );
        let query_cmd = parse_datalog_command(&query_str).unwrap();

        let result = executor.execute(query_cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?to"]);
                // Should find B and C (direct connections)
                assert_eq!(results.len(), 2);

                // Collect result UUIDs
                let result_uuids: Vec<Uuid> = results
                    .iter()
                    .map(|row| match &row[0] {
                        Value::Ref(uuid) => *uuid,
                        _ => panic!("Expected Ref value"),
                    })
                    .collect();

                assert!(result_uuids.contains(&b));
                assert!(result_uuids.contains(&c));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_query_mixed_pattern_and_rule() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Create graph with names: A->B, A->C, and give B a name
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (a, ":connected".to_string(), Value::Ref(c)),
                (
                    b,
                    ":person/name".to_string(),
                    Value::String("Bob".to_string()),
                ),
            ], None)
            .unwrap();

        // Register reachable rule (base case only - no recursion yet)
        executor
            .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap())
            .unwrap();

        // Query: find names of nodes reachable from A
        let query_str = format!(
            r#"(query [:find ?name :where (reachable #uuid "{}" ?to) [?to :person/name ?name]])"#,
            a.to_string()
        );
        let query_cmd = parse_datalog_command(&query_str).unwrap();

        let result = executor.execute(query_cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?name"]);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String("Bob".to_string()));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_query_with_recursive_transitive_closure() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Create graph: A->B->C
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (b, ":connected".to_string(), Value::Ref(c)),
            ], None)
            .unwrap();

        // Register reachable rules (base + recursive)
        executor
            .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap())
            .unwrap();

        executor
            .execute(parse_datalog_command(
                r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
            ).unwrap())
            .unwrap();

        // Query: find all nodes reachable from A
        let query_str = format!(
            r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
            a.to_string()
        );
        let query_cmd = parse_datalog_command(&query_str).unwrap();

        let result = executor.execute(query_cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?to"]);
                // Should find B and C via transitive closure
                assert_eq!(results.len(), 2);

                // Collect result UUIDs
                let result_uuids: Vec<Uuid> = results
                    .iter()
                    .map(|row| match &row[0] {
                        Value::Ref(uuid) => *uuid,
                        _ => panic!("Expected Ref value"),
                    })
                    .collect();

                assert!(result_uuids.contains(&b));
                assert!(result_uuids.contains(&c));
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_query_recursive_with_mixed_patterns() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());

        // Create graph: A->B->C, give C a name
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        storage
            .transact(vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (b, ":connected".to_string(), Value::Ref(c)),
                (
                    c,
                    ":person/name".to_string(),
                    Value::String("Charlie".to_string()),
                ),
            ], None)
            .unwrap();

        // Register recursive reachable rules
        executor
            .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap())
            .unwrap();

        executor
            .execute(parse_datalog_command(
                r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
            ).unwrap())
            .unwrap();

        // Query: find names of nodes transitively reachable from A
        let query_str = format!(
            r#"(query [:find ?name :where (reachable #uuid "{}" ?to) [?to :person/name ?name]])"#,
            a.to_string()
        );
        let query_cmd = parse_datalog_command(&query_str).unwrap();

        let result = executor.execute(query_cmd).unwrap();
        match result {
            QueryResult::QueryResults { vars, results } => {
                assert_eq!(vars, vec!["?name"]);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String("Charlie".to_string()));
            }
            _ => panic!("Expected QueryResults"),
        }
    }
}
