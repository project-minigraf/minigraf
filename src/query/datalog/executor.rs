use super::matcher::{edn_to_entity_id, edn_to_value, PatternMatcher};
use super::types::{DatalogCommand, DatalogQuery, EdnValue, Transaction};
use crate::graph::types::{TxId, Value};
use crate::graph::FactStorage;
use anyhow::{anyhow, Result};

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
}

impl DatalogExecutor {
    pub fn new(storage: FactStorage) -> Self {
        DatalogExecutor { storage }
    }

    /// Execute a Datalog command
    pub fn execute(&self, command: DatalogCommand) -> Result<QueryResult> {
        match command {
            DatalogCommand::Transact(tx) => self.execute_transact(tx),
            DatalogCommand::Retract(tx) => self.execute_retract(tx),
            DatalogCommand::Query(query) => self.execute_query(query),
            DatalogCommand::Rule(_rule) => {
                // TODO: Implement rule execution in later phase
                Err(anyhow!("Rule execution not yet implemented"))
            }
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
            .transact(fact_tuples)
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
        let matcher = PatternMatcher::new(self.storage.clone());

        // Match all patterns and get bindings
        let bindings = matcher.match_patterns(&query.patterns);

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

    /// Get the underlying storage (for testing)
    pub fn storage(&self) -> &FactStorage {
        &self.storage
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
            ])
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
            ])
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
            )])
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
}
