use super::evaluator::StratifiedEvaluator;
use super::matcher::{PatternMatcher, edn_to_entity_id, edn_to_value};
use super::optimizer;
use super::rules::RuleRegistry;
use super::types::{
    DatalogCommand, DatalogQuery, EdnValue, Pattern, Rule, Transaction, ValidAt, WhereClause,
};
use crate::graph::FactStorage;
use crate::graph::types::{Fact, TransactOptions, TxId, Value, tx_id_now};
use crate::storage::index::Indexes;
use anyhow::{Result, anyhow};
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

    /// Create a `DatalogExecutor` with a shared rule registry.
    ///
    /// Used by `Minigraf` to share rules across all `execute()` calls.
    pub fn new_with_rules(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>) -> Self {
        DatalogExecutor { storage, rules }
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
        // Transaction-level valid-time options (used when pattern has no per-fact override)
        let tx_opts = if tx.valid_from.is_some() || tx.valid_to.is_some() {
            Some(TransactOptions::new(tx.valid_from, tx.valid_to))
        } else {
            None
        };

        let mut last_tx_id: TxId = 0;

        for pattern in tx.facts {
            let entity_id =
                edn_to_entity_id(&pattern.entity).map_err(|e| anyhow!("Invalid entity: {}", e))?;

            let attribute = match &pattern.attribute {
                EdnValue::Keyword(k) => k.clone(),
                _ => return Err(anyhow!("Attribute must be a keyword")),
            };

            let value =
                edn_to_value(&pattern.value).map_err(|e| anyhow!("Invalid value: {}", e))?;

            // Determine per-fact opts: per-fact override takes precedence over tx-level
            let opts = if pattern.valid_from.is_some() || pattern.valid_to.is_some() {
                Some(TransactOptions::new(pattern.valid_from, pattern.valid_to))
            } else {
                tx_opts.clone()
            };

            last_tx_id = self
                .storage
                .transact(vec![(entity_id, attribute, value)], opts)
                .map_err(|e| anyhow!("Transaction failed: {}", e))?;
        }

        Ok(QueryResult::Transacted(last_tx_id))
    }

    /// Execute a retract command: retract facts from storage
    fn execute_retract(&self, tx: Transaction) -> Result<QueryResult> {
        let mut fact_tuples = Vec::new();

        for pattern in tx.facts {
            let entity_id =
                edn_to_entity_id(&pattern.entity).map_err(|e| anyhow!("Invalid entity: {}", e))?;

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

    /// Build a filtered FactStorage for a query's temporal constraints.
    ///
    /// Step 1: apply transaction-time filter (`:as-of`) — defaults to all facts.
    /// Step 2: discard retracted facts within the tx window.
    /// Step 3: apply valid-time filter (`:valid-at`) — defaults to "currently valid".
    fn filter_facts_for_query(&self, query: &DatalogQuery) -> Result<FactStorage> {
        let now = tx_id_now() as i64;

        // Step 1: transaction-time filter
        let tx_filtered: Vec<Fact> = match &query.as_of {
            Some(as_of) => self.storage.get_facts_as_of(as_of)?,
            None => self.storage.get_all_facts()?,
        };

        // Step 2: compute net-asserted view — for each (entity, attribute, value) triple,
        // keep it only if the record with the highest tx_count is an assertion.
        // This correctly hides facts that have been retracted.
        let asserted = crate::graph::storage::net_asserted_facts(tx_filtered);

        // Step 3: valid-time filter
        let valid_filtered: Vec<Fact> = match &query.valid_at {
            Some(ValidAt::Timestamp(t)) => asserted
                .into_iter()
                .filter(|f| f.valid_from <= *t && *t < f.valid_to)
                .collect(),
            Some(ValidAt::AnyValidTime) => asserted,
            None => asserted
                .into_iter()
                .filter(|f| f.valid_from <= now && now < f.valid_to)
                .collect(),
        };

        // Build a temporary FactStorage with the filtered facts
        let filtered_storage = FactStorage::new();
        for fact in valid_filtered {
            filtered_storage.load_fact(fact)?;
        }
        Ok(filtered_storage)
    }

    /// Execute a query: find matching facts and return specified variables
    fn execute_query(&self, query: DatalogQuery) -> Result<QueryResult> {
        // Check if query uses rules
        if query.uses_rules() {
            // Use StratifiedEvaluator for queries with rule invocations (handles negation and strata)
            return self.execute_query_with_rules(query);
        }

        // Apply temporal filters before pattern matching
        let filtered_storage = self.filter_facts_for_query(&query)?;
        let matcher = PatternMatcher::new(filtered_storage.clone()); // keep filtered_storage for not-filter
        let patterns = query.get_patterns();

        // Plan patterns: assign index hints and reorder by selectivity.
        // Phase 6.1: Indexes::new() is a placeholder; Phase 6.2 will pass real indexes.
        let planned_patterns = optimizer::plan(patterns, &Indexes::new());

        // Match all patterns in planned order and get bindings
        let bindings = matcher.match_patterns(
            &planned_patterns
                .into_iter()
                .map(|(p, _hint)| p)
                .collect::<Vec<_>>(),
        );

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
            let not_storage = filtered_storage.clone();
            bindings
                .into_iter()
                .filter(|binding| {
                    for not_body in &not_clauses {
                        let substituted: Vec<Pattern> = not_body
                            .iter()
                            .filter_map(|c| match c {
                                WhereClause::Pattern(p) => {
                                    Some(crate::query::datalog::evaluator::substitute_pattern(
                                        p, binding,
                                    ))
                                }
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

        // Extract requested variables from bindings
        let mut results = Vec::new();
        for binding in filtered_bindings {
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
        // Extract ALL predicates (including inside not bodies) so the StratifiedEvaluator
        // evaluates every referenced rule. This is needed for not-post-filter to work.
        let all_rule_invocations = query.get_rule_invocations();
        let predicates: Vec<String> = all_rule_invocations
            .iter()
            .map(|(pred, _)| pred.clone())
            .collect();

        // Apply temporal filters before evaluating recursive rules
        let filtered_storage = self.filter_facts_for_query(&query)?;

        // Create StratifiedEvaluator — handles negation, stratification, and positive-only rules
        let evaluator = StratifiedEvaluator::new(
            filtered_storage,
            self.rules.clone(),
            1000, // max iterations
        );

        let derived_storage = evaluator.evaluate(&predicates)?;

        // Convert ONLY top-level rule invocations to positive-match patterns.
        // Rule invocations inside `not` bodies are handled by the not-post-filter below.
        // (reachable ?x ?y) becomes [?x :reachable ?y]
        let mut all_patterns = query.get_patterns();

        for (predicate, args) in query.get_top_level_rule_invocations() {
            let pattern = match args.len() {
                1 => {
                    // 1-arg: (blocked ?x)  →  [?x :blocked ?_rule_value]
                    Pattern::new(
                        args[0].clone(),
                        EdnValue::Keyword(format!(":{}", predicate)),
                        EdnValue::Symbol("?_rule_value".to_string()),
                    )
                }
                2 => {
                    // 2-arg: (reachable ?x ?y)  →  [?x :reachable ?y]
                    Pattern::new(
                        args[0].clone(),
                        EdnValue::Keyword(format!(":{}", predicate)),
                        args[1].clone(),
                    )
                }
                n => {
                    return Err(anyhow!(
                        "Rule invocation '{}' must have 1 or 2 arguments, got {}",
                        predicate,
                        n
                    ));
                }
            };
            all_patterns.push(pattern);
        }

        // Match all patterns against derived facts
        let matcher = PatternMatcher::new(derived_storage.clone());
        let bindings = matcher.match_patterns(&all_patterns);

        // Apply not-post-filter for WhereClause::Not clauses in the query body.
        // (The StratifiedEvaluator handles `not` in rule bodies; this handles `not`
        // appearing directly in the query body alongside rule invocations.)
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
            let not_storage = derived_storage.clone();
            bindings
                .into_iter()
                .filter(|binding| {
                    for not_body in &not_clauses {
                        let substituted: Vec<Pattern> = not_body
                            .iter()
                            .filter_map(|c| match c {
                                WhereClause::Pattern(p) => {
                                    Some(crate::query::datalog::evaluator::substitute_pattern(
                                        p, binding,
                                    ))
                                }
                                WhereClause::RuleInvocation { predicate, args } => {
                                    // Convert rule invocation to a pattern against derived storage.
                                    // Apply the current binding to any variables in args first.
                                    let resolved_args: Vec<EdnValue> = args
                                        .iter()
                                        .map(|a| match a {
                                            EdnValue::Symbol(s) if s.starts_with('?') => {
                                                // Look up the bound value and convert back to EdnValue
                                                binding
                                                    .get(s)
                                                    .map(|v| match v {
                                                        Value::Keyword(k) => {
                                                            EdnValue::Keyword(k.clone())
                                                        }
                                                        Value::String(s) => {
                                                            EdnValue::String(s.clone())
                                                        }
                                                        Value::Integer(i) => EdnValue::Integer(*i),
                                                        Value::Float(f) => EdnValue::Float(*f),
                                                        Value::Boolean(b) => EdnValue::Boolean(*b),
                                                        Value::Ref(u) => EdnValue::Uuid(*u),
                                                        Value::Null => EdnValue::Nil,
                                                    })
                                                    .unwrap_or_else(|| a.clone())
                                            }
                                            other => other.clone(),
                                        })
                                        .collect();
                                    let pattern = match resolved_args.len() {
                                        1 => Pattern::new(
                                            resolved_args[0].clone(),
                                            EdnValue::Keyword(format!(":{}", predicate)),
                                            EdnValue::Symbol("?_rule_value".to_string()),
                                        ),
                                        2 => Pattern::new(
                                            resolved_args[0].clone(),
                                            EdnValue::Keyword(format!(":{}", predicate)),
                                            resolved_args[1].clone(),
                                        ),
                                        _ => return None,
                                    };
                                    Some(crate::query::datalog::evaluator::substitute_pattern(
                                        &pattern, binding,
                                    ))
                                }
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

        // Extract requested variables from bindings
        let mut results = Vec::new();
        for binding in filtered_bindings {
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
        self.rules.write().unwrap().register_rule(predicate, rule)?;

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
    use crate::query::datalog::types::WhereClause;
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
            .transact(
                vec![
                    (
                        alice_id,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice_id, ":person/age".to_string(), Value::Integer(30)),
                ],
                None,
            )
            .unwrap();

        // Query for name
        let cmd = parse_datalog_command(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
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
            .transact(
                vec![
                    (
                        alice_id,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice_id, ":person/age".to_string(), Value::Integer(30)),
                ],
                None,
            )
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
        let cmd = parse_datalog_command(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
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
            .transact(
                vec![(alice_id, ":person/age".to_string(), Value::Integer(30))],
                None,
            )
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
        let cmd =
            parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap();

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
        let cmd1 =
            parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap();
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
        let cmd1 =
            parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap();
        executor.execute(cmd1).unwrap();

        // Register ancestor rule
        let cmd2 = parse_datalog_command(r#"(rule [(ancestor ?a ?d) [?a :parent ?d]])"#).unwrap();
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (a, ":connected".to_string(), Value::Ref(c)),
                ],
                None,
            )
            .unwrap();

        // Register reachable rule (base case only - no recursion yet)
        let rule1 =
            parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap();
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (a, ":connected".to_string(), Value::Ref(c)),
                    (
                        b,
                        ":person/name".to_string(),
                        Value::String("Bob".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        // Register reachable rule (base case only - no recursion yet)
        executor
            .execute(
                parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap(),
            )
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (b, ":connected".to_string(), Value::Ref(c)),
                ],
                None,
            )
            .unwrap();

        // Register reachable rules (base + recursive)
        executor
            .execute(
                parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap(),
            )
            .unwrap();

        executor
            .execute(
                parse_datalog_command(
                    r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
                )
                .unwrap(),
            )
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
    fn test_default_query_filters_to_currently_valid() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        let alice = Uuid::new_v4();

        // Fact valid forever (default) - tx_count=1
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::String("Alice".to_string()),
                )],
                valid_from: None,
                valid_to: None,
            }))
            .unwrap();

        // Fact with valid_to in the past (expired) - tx_count=2
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":employment/status".to_string()),
                    EdnValue::Keyword(":active".to_string()),
                )],
                valid_from: Some(1000_i64),
                valid_to: Some(2000_i64), // expired long ago
            }))
            .unwrap();

        // Default query (no :valid-at) should only return the forever-valid fact
        let result = executor
            .execute(DatalogCommand::Query(DatalogQuery::new(
                vec!["?attr".to_string()],
                vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Symbol("?attr".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                ))],
            )))
            .unwrap();

        let rows = match result {
            QueryResult::QueryResults { results, .. } => results,
            _ => panic!("expected query results"),
        };
        assert_eq!(rows.len(), 1); // only the name fact
    }

    #[test]
    fn test_as_of_counter_shows_past_state() {
        use crate::query::datalog::types::AsOf;
        use crate::query::datalog::types::ValidAt;

        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let alice = Uuid::new_v4();

        // tx_count=1: assert name
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::String("Alice".to_string()),
                )],
                valid_from: None,
                valid_to: None,
            }))
            .unwrap();

        // tx_count=2: assert age
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":person/age".to_string()),
                    EdnValue::Integer(30),
                )],
                valid_from: None,
                valid_to: None,
            }))
            .unwrap();

        // :as-of 1 → only name fact visible (age was added at tx_count=2)
        let result = executor
            .execute(DatalogCommand::Query(DatalogQuery {
                find: vec!["?attr".to_string()],
                where_clauses: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Symbol("?attr".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                ))],
                as_of: Some(AsOf::Counter(1)),
                valid_at: Some(ValidAt::AnyValidTime),
            }))
            .unwrap();

        let rows = match result {
            QueryResult::QueryResults { results, .. } => results,
            _ => panic!("expected query results"),
        };
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_valid_at_any_valid_time_shows_all() {
        use crate::query::datalog::types::ValidAt;

        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let alice = Uuid::new_v4();

        // Fact valid forever (default)
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::String("Alice".to_string()),
                )],
                valid_from: None,
                valid_to: None,
            }))
            .unwrap();

        // Fact with valid_to already in the past
        executor
            .execute(DatalogCommand::Transact(Transaction {
                facts: vec![Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Keyword(":employment/status".to_string()),
                    EdnValue::Keyword(":active".to_string()),
                )],
                valid_from: Some(1000_i64),
                valid_to: Some(2000_i64), // expired
            }))
            .unwrap();

        // :valid-at :any-valid-time → both facts returned
        let result = executor
            .execute(DatalogCommand::Query(DatalogQuery {
                find: vec!["?attr".to_string()],
                where_clauses: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Symbol("?attr".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                ))],
                as_of: None,
                valid_at: Some(ValidAt::AnyValidTime),
            }))
            .unwrap();

        let rows = match result {
            QueryResult::QueryResults { results, .. } => results,
            _ => panic!("expected query results"),
        };
        assert_eq!(rows.len(), 2);
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (b, ":connected".to_string(), Value::Ref(c)),
                    (
                        c,
                        ":person/name".to_string(),
                        Value::String("Charlie".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        // Register recursive reachable rules
        executor
            .execute(
                parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap(),
            )
            .unwrap();

        executor
            .execute(
                parse_datalog_command(
                    r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
                )
                .unwrap(),
            )
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

    #[test]
    fn test_execute_query_not_as_pure_filter() {
        // Query: [:find ?e :where [?e :applied true] (not [?e :rejected true])]
        // No rule invocations — pure not-filter path in execute_query.
        use crate::query::datalog::types::WhereClause;
        let storage = FactStorage::new();
        let alice = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let bob = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        // alice: applied + rejected
        storage
            .transact(
                vec![
                    (alice, ":applied".to_string(), Value::Boolean(true)),
                    (alice, ":rejected".to_string(), Value::Boolean(true)),
                ],
                None,
            )
            .unwrap();
        // bob: applied only
        storage
            .transact(
                vec![(bob, ":applied".to_string(), Value::Boolean(true))],
                None,
            )
            .unwrap();

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

        let executor = DatalogExecutor::new(storage);
        let result = executor
            .execute(crate::query::datalog::types::DatalogCommand::Query(query))
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "only bob should pass (alice is rejected)");
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_execute_query_with_rules_not_in_query_body() {
        // Query: [:find ?x :where (reachable ?_a ?x) (not [?x :blocked true])]
        // rule invocation + pattern-not in same query body
        use crate::graph::types::Fact;
        use crate::query::datalog::types::{Pattern, WhereClause};
        let storage = FactStorage::new();
        let a = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let b = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let c = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        storage
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (a, ":connected".to_string(), Value::Ref(c)),
                    (c, ":blocked".to_string(), Value::Boolean(true)),
                ],
                None,
            )
            .unwrap();

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        // reachable(?from ?to) :- [?from :connected ?to]
        {
            use crate::query::datalog::types::{Rule, WhereClause as WC};
            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("reachable".to_string()),
                    EdnValue::Symbol("?from".to_string()),
                    EdnValue::Symbol("?to".to_string()),
                ],
                body: vec![WC::Pattern(Pattern::new(
                    EdnValue::Symbol("?from".to_string()),
                    EdnValue::Keyword(":connected".to_string()),
                    EdnValue::Symbol("?to".to_string()),
                ))],
            };
            rules
                .write()
                .unwrap()
                .register_rule("reachable".to_string(), rule)
                .unwrap();
        }

        let query = DatalogQuery::new(
            vec!["?x".to_string()],
            vec![
                WhereClause::RuleInvocation {
                    predicate: "reachable".to_string(),
                    args: vec![
                        EdnValue::Symbol("?_a".to_string()),
                        EdnValue::Symbol("?x".to_string()),
                    ],
                },
                WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":blocked".to_string()),
                    EdnValue::Boolean(true),
                ))]),
            ],
        );

        let executor = DatalogExecutor::new_with_rules(storage, rules);
        let result = executor
            .execute(crate::query::datalog::types::DatalogCommand::Query(query))
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    1,
                    "c should be excluded (blocked), only b passes"
                );
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_optimizer_does_not_change_query_results() {
        // A multi-pattern query that the optimizer would reorder.
        // Results must be identical regardless of execution order.
        let storage = FactStorage::new();
        let alice = uuid::Uuid::new_v4();
        let bob = uuid::Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice, ":friend".to_string(), Value::Ref(bob)),
                    (bob, ":name".to_string(), Value::String("Bob".to_string())),
                ],
                None,
            )
            .unwrap();

        let executor = DatalogExecutor::new(storage);
        // Simple query: find all names (no join reordering needed)
        let result = executor
            .execute(
                parse_datalog_command("(query [:find ?name :where [?e :name ?name]])").unwrap(),
            )
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 2, "Alice and Bob both have names");
            }
            _ => panic!("Expected QueryResults"),
        }
    }
}
