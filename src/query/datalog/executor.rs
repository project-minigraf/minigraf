use super::evaluator::{StratifiedEvaluator, evaluate_not_join};
use super::matcher::{PatternMatcher, edn_to_entity_id, edn_to_value};
use super::optimizer;
use super::rules::RuleRegistry;
use super::types::{
    AggFunc, AsOf, BinOp, DatalogCommand, DatalogQuery, EdnValue, Expr, FindSpec, Pattern, Rule,
    Transaction, UnaryOp, ValidAt, WhereClause,
};
use crate::graph::FactStorage;
use crate::graph::types::{Fact, TransactOptions, TxId, Value, tx_id_now};
use crate::storage::index::Indexes;
use anyhow::{Result, anyhow};
use regex_lite::Regex;
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
        // Transaction-level valid-time options (fallback when no per-fact override)
        let tx_opts = if tx.valid_from.is_some() || tx.valid_to.is_some() {
            Some(TransactOptions::new(tx.valid_from, tx.valid_to))
        } else {
            None
        };

        // Collect all facts into a single batch so they share one tx_count.
        // Each fact carries its own per-fact opts (or None to fall back to tx_opts).
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

            let per_fact_opts = if pattern.valid_from.is_some() || pattern.valid_to.is_some() {
                Some(TransactOptions::new(pattern.valid_from, pattern.valid_to))
            } else {
                None
            };

            fact_tuples.push((entity_id, attribute, value, per_fact_opts));
        }

        let tx_id = self
            .storage
            .transact_batch(fact_tuples, tx_opts)
            .map_err(|e| anyhow!("Transaction failed: {}", e))?;

        Ok(QueryResult::Transacted(tx_id))
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

        // Apply not-filter for WhereClause::Not and WhereClause::NotJoin clauses
        // (no rules involved — pure post-filter)
        let not_clauses: Vec<&Vec<WhereClause>> = query
            .where_clauses
            .iter()
            .filter_map(|c| match c {
                WhereClause::Not(inner) => Some(inner),
                _ => None,
            })
            .collect();

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

        let not_filtered: Vec<_> = if not_clauses.is_empty() && not_join_clauses.is_empty() {
            bindings
        } else {
            let not_storage = filtered_storage.clone();
            bindings
                .into_iter()
                .filter(|binding| {
                    for not_body in &not_clauses {
                        if not_body_matches(not_body, binding, &not_storage) {
                            return false;
                        }
                    }
                    for (join_vars, nj_clauses) in &not_join_clauses {
                        if evaluate_not_join(join_vars, nj_clauses, binding, &not_storage) {
                            return false;
                        }
                    }
                    true
                })
                .collect()
        };

        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses);

        // Extract requested variables from bindings (or aggregate)
        let has_aggregates = query
            .find
            .iter()
            .any(|s| matches!(s, FindSpec::Aggregate { .. }));
        let results = if has_aggregates {
            apply_aggregation(filtered_bindings, &query.find, &query.with_vars)?
        } else {
            extract_variables(filtered_bindings, &query.find)
        };

        Ok(QueryResult::QueryResults {
            vars: query.find.iter().map(|s| s.display_name()).collect(),
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

        // Apply not-post-filter for WhereClause::Not and WhereClause::NotJoin clauses
        // in the query body. (The StratifiedEvaluator handles `not`/`not-join` in rule
        // bodies; this handles them appearing directly in the query body alongside rule
        // invocations.)
        let not_clauses: Vec<&Vec<WhereClause>> = query
            .where_clauses
            .iter()
            .filter_map(|c| match c {
                WhereClause::Not(inner) => Some(inner),
                _ => None,
            })
            .collect();

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

        let not_filtered: Vec<_> = if not_clauses.is_empty() && not_join_clauses.is_empty() {
            bindings
        } else {
            let not_storage = derived_storage.clone();
            bindings
                .into_iter()
                .filter(|binding| {
                    for not_body in &not_clauses {
                        // Collect pattern and rule-invocation clauses into patterns.
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

                        // Compute not_bindings: if no patterns, seed with current binding.
                        let m = PatternMatcher::new(not_storage.clone());
                        let mut not_bindings: Vec<Binding> = if substituted.is_empty() {
                            vec![binding.clone()]
                        } else {
                            m.match_patterns(&substituted)
                                .into_iter()
                                .map(|mut nb| {
                                    for (k, v) in binding {
                                        nb.entry(k.clone()).or_insert_with(|| v.clone());
                                    }
                                    nb
                                })
                                .collect()
                        };

                        // Apply Expr clauses from the not body.
                        not_bindings = apply_expr_clauses(not_bindings, not_body);
                        if !not_bindings.is_empty() {
                            return false; // not condition violated
                        }
                    }
                    for (join_vars, nj_clauses) in &not_join_clauses {
                        if evaluate_not_join(join_vars, nj_clauses, binding, &not_storage) {
                            return false;
                        }
                    }
                    true
                })
                .collect()
        };

        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses);

        // Extract requested variables from bindings (or aggregate)
        let has_aggregates = query
            .find
            .iter()
            .any(|s| matches!(s, FindSpec::Aggregate { .. }));
        let results = if has_aggregates {
            apply_aggregation(filtered_bindings, &query.find, &query.with_vars)?
        } else {
            extract_variables(filtered_bindings, &query.find)
        };

        Ok(QueryResult::QueryResults {
            vars: query.find.iter().map(|s| s.display_name()).collect(),
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

/// Evaluate a `not` body against the current outer binding.
///
/// Returns true if the body "matches" (i.e., the outer binding should be excluded).
fn not_body_matches(
    not_body: &[WhereClause],
    outer: &Binding,
    storage: &crate::graph::FactStorage,
) -> bool {
    use crate::query::datalog::evaluator::substitute_pattern;

    let patterns: Vec<_> = not_body
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(substitute_pattern(p, outer)),
            // INVARIANT: not_body_matches is only called from execute_query, which is
            // only reached when query.uses_rules() is false. uses_rules() descends into
            // Not bodies via rule_invocations(), so any not body containing a
            // RuleInvocation is routed to execute_query_with_rules instead.
            // WhereClause::Expr clauses are handled by apply_expr_clauses below.
            _ => None,
        })
        .collect();

    let matcher = crate::query::datalog::matcher::PatternMatcher::new(storage.clone());
    let mut not_bindings: Vec<Binding> = if patterns.is_empty() {
        // Expr-only not body: start with the outer binding so variables resolve.
        vec![outer.clone()]
    } else {
        // Merge outer binding with pattern-match results.
        matcher
            .match_patterns(&patterns)
            .into_iter()
            .map(|mut nb| {
                for (k, v) in outer {
                    nb.entry(k.clone()).or_insert_with(|| v.clone());
                }
                nb
            })
            .collect()
    };

    // Apply Expr clauses from the not body.
    not_bindings = apply_expr_clauses(not_bindings, not_body);
    !not_bindings.is_empty()
}

/// Extract plain variable values from bindings (non-aggregate path).
fn extract_variables(
    bindings: Vec<std::collections::HashMap<String, Value>>,
    find_specs: &[FindSpec],
) -> Vec<Vec<Value>> {
    let mut results = Vec::new();
    for binding in bindings {
        let mut row = Vec::new();
        for spec in find_specs {
            if let Some(value) = binding.get(spec.var()) {
                row.push(value.clone());
            } else {
                break;
            }
        }
        if row.len() == find_specs.len() {
            results.push(row);
        }
    }
    results
}

/// Post-process bindings through aggregation.
/// Called only when find_specs contains at least one FindSpec::Aggregate.
fn apply_aggregation(
    bindings: Vec<std::collections::HashMap<String, Value>>,
    find_specs: &[FindSpec],
    with_vars: &[String],
) -> Result<Vec<Vec<Value>>> {
    let has_grouping_vars = find_specs
        .iter()
        .any(|s| matches!(s, FindSpec::Variable(_)));

    // Zero bindings: special case for pure count/count-distinct (no grouping vars)
    if bindings.is_empty() {
        let all_count = !has_grouping_vars
            && find_specs.iter().all(|s| {
                matches!(
                    s,
                    FindSpec::Aggregate {
                        func: AggFunc::Count | AggFunc::CountDistinct,
                        ..
                    }
                )
            });
        if all_count {
            let row = find_specs.iter().map(|_| Value::Integer(0)).collect();
            return Ok(vec![row]);
        }
        return Ok(vec![]);
    }

    // Grouping key = FindSpec::Variable vars (in find order) + with_vars
    let group_var_names: Vec<&str> = find_specs
        .iter()
        .filter_map(|s| match s {
            FindSpec::Variable(v) => Some(v.as_str()),
            FindSpec::Aggregate { .. } => None,
        })
        .chain(with_vars.iter().map(|s| s.as_str()))
        .collect();

    // Group using Vec + PartialEq scan (Value::Float doesn't implement Hash)
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(Vec<Value>, Vec<std::collections::HashMap<String, Value>>)> = Vec::new();
    for b in bindings {
        let key: Vec<Value> = group_var_names
            .iter()
            .map(|v| b.get(*v).cloned().unwrap_or(Value::Null))
            .collect();
        if let Some(pos) = groups.iter().position(|(k, _)| k == &key) {
            groups[pos].1.push(b);
        } else {
            groups.push((key.clone(), vec![b]));
        }
    }

    // Map of Variable spec name → its index in the group key Vec (only Variable specs, in order)
    let mut group_key_idx_for_var: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    {
        let mut var_pos = 0usize;
        for spec in find_specs {
            if let FindSpec::Variable(v) = spec {
                group_key_idx_for_var.insert(v.as_str(), var_pos);
                var_pos += 1;
            }
        }
    }

    // Build output rows (one per group)
    let mut results = Vec::new();
    for (key, group_bindings) in &groups {
        let mut row = Vec::new();
        let mut skip_row = false;
        for spec in find_specs {
            match spec {
                FindSpec::Variable(v) => {
                    let pos = *group_key_idx_for_var.get(v.as_str()).unwrap();
                    row.push(key[pos].clone());
                }
                FindSpec::Aggregate { func, var } => {
                    let non_null_values: Vec<&Value> = group_bindings
                        .iter()
                        .filter_map(|b| b.get(var.as_str()))
                        .filter(|v| !matches!(v, Value::Null))
                        .collect();
                    match apply_agg_func(func, &non_null_values) {
                        Ok(v) => row.push(v),
                        Err(e) => {
                            // min/max on all-null group: skip this group
                            let msg = e.to_string();
                            if msg.contains("no non-null values in group") {
                                skip_row = true;
                                break;
                            }
                            return Err(e);
                        }
                    }
                }
            }
        }
        if !skip_row {
            results.push(row);
        }
    }

    Ok(results)
}

/// Apply a single aggregate function to a slice of non-null values.
fn apply_agg_func(func: &AggFunc, values: &[&Value]) -> Result<Value> {
    match func {
        AggFunc::Count => Ok(Value::Integer(values.len() as i64)),

        AggFunc::CountDistinct => {
            let mut seen: Vec<&Value> = Vec::new();
            for v in values {
                if !seen.contains(v) {
                    seen.push(v);
                }
            }
            Ok(Value::Integer(seen.len() as i64))
        }

        AggFunc::Sum | AggFunc::SumDistinct => {
            let deduped: Vec<&Value> = if matches!(func, AggFunc::SumDistinct) {
                let mut seen: Vec<&Value> = Vec::new();
                for v in values {
                    if !seen.contains(v) {
                        seen.push(v);
                    }
                }
                seen
            } else {
                values.to_vec()
            };

            if deduped.is_empty() {
                return Ok(Value::Integer(0));
            }

            let has_float = deduped.iter().any(|v| matches!(v, Value::Float(_)));
            if has_float {
                let mut sum = 0.0_f64;
                for v in &deduped {
                    match v {
                        Value::Float(f) => sum += f,
                        Value::Integer(i) => sum += *i as f64,
                        other => {
                            return Err(anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ));
                        }
                    }
                }
                Ok(Value::Float(sum))
            } else {
                let mut sum = 0_i64;
                for v in &deduped {
                    match v {
                        Value::Integer(i) => sum += i,
                        other => {
                            return Err(anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ));
                        }
                    }
                }
                Ok(Value::Integer(sum))
            }
        }

        AggFunc::Min | AggFunc::Max => {
            if values.is_empty() {
                return Err(anyhow!("min/max: no non-null values in group"));
            }
            // Check all same type (no mixing Integer and Float)
            let first = values[0];
            for v in &values[1..] {
                if std::mem::discriminant(*v) != std::mem::discriminant(first) {
                    return Err(anyhow!(
                        "{}: cannot compare {} and {} values",
                        func.as_str(),
                        value_type_name(first),
                        value_type_name(v)
                    ));
                }
            }
            // Find min or max using PartialOrd
            let result = values.iter().try_fold((*values[0]).clone(), |acc, v| {
                let ordering = match (&acc, v) {
                    (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
                    (Value::Float(a), Value::Float(b)) => {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(a), Value::String(b)) => a.cmp(b),
                    (_, other) => {
                        return Err(anyhow!(
                            "{}: expected Integer, Float, String, or Null, got {}",
                            func.as_str(),
                            value_type_name(other)
                        ));
                    }
                };
                let replace = match func {
                    AggFunc::Min => ordering == std::cmp::Ordering::Greater,
                    AggFunc::Max => ordering == std::cmp::Ordering::Less,
                    _ => unreachable!(),
                };
                Ok::<Value, anyhow::Error>(if replace { (*v).clone() } else { acc })
            })?;
            Ok(result)
        }
    }
}

type Binding = std::collections::HashMap<String, Value>;

/// Evaluate a single branch of an `or`/`or-join` against incoming bindings.
///
/// Processing order (mirrors top-level execute_query order):
/// 1. Pattern/RuleInvocation → match_patterns_seeded
/// 2. Nested Or/OrJoin → apply_or_clauses (recursive)
/// 3. Not/NotJoin → post-filter
/// 4. Expr → apply_expr_clauses
pub(crate) fn evaluate_branch(
    branch: &[WhereClause],
    incoming: Vec<Binding>,
    storage: &FactStorage,
    rules: &crate::query::datalog::rules::RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>> {
    use crate::query::datalog::evaluator::rule_invocation_to_pattern;
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
    let bindings = apply_or_clauses(
        branch,
        bindings,
        storage,
        rules,
        as_of.clone(),
        valid_at.clone(),
    )?;

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

/// Apply all Or/OrJoin clauses from `clauses` to `bindings` in sequence.
///
/// Non-Or/OrJoin clauses are ignored (handled elsewhere).
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
            WhereClause::OrJoin {
                join_vars,
                branches,
            } => {
                // outer_keys: all variable names present in the incoming bindings
                let outer_keys: std::collections::HashSet<String> =
                    bindings.iter().flat_map(|b| b.keys().cloned()).collect();

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
                        // Project to outer_keys (all variables bound before this or-join clause).
                        // This is equivalent to retaining join_vars because:
                        // (1) the parser enforces join_vars ⊆ outer_bound, so join_vars ⊆ outer_keys, and
                        // (2) retaining outer_keys is safe because those variables were stable before the or-join.
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

/// Returns true for Boolean(true), non-zero Integer, non-zero Float.
/// All other Value variants (String, Keyword, Ref, Null, Float(0.0)) → false.
/// Note: `Float(-0.0)` is falsy because `-0.0 == 0.0` in IEEE 754.
pub(crate) fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Boolean(b) => *b,
        Value::Integer(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        _ => false,
    }
}

/// Promote both values to f64 for numeric comparison / float arithmetic.
/// Returns Err(()) if either operand is not Integer or Float.
fn to_float_pair(l: &Value, r: &Value) -> Result<(f64, f64), ()> {
    let lf = match l {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        _ => return Err(()),
    };
    let rf = match r {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        _ => return Err(()),
    };
    Ok((lf, rf))
}

fn eval_binop(op: &BinOp, l: Value, r: Value) -> Result<Value, ()> {
    match op {
        // Structural equality — works for all Value variants; no type mismatch error.
        BinOp::Eq => return Ok(Value::Boolean(l == r)),
        BinOp::Neq => return Ok(Value::Boolean(l != r)),
        _ => {}
    }

    match op {
        // Numeric comparisons — require both numeric; int/float promotion via to_float_pair.
        BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
            let (lf, rf) = to_float_pair(&l, &r)?;
            Ok(Value::Boolean(match op {
                BinOp::Lt => lf < rf,
                BinOp::Gt => lf > rf,
                BinOp::Lte => lf <= rf,
                BinOp::Gte => lf >= rf,
                _ => unreachable!(),
            }))
        }

        // Arithmetic: integer-integer stays integer; any float promotes result to float.
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => match (&l, &r) {
            (Value::Integer(a), Value::Integer(b)) => match op {
                BinOp::Add => Ok(Value::Integer(a.wrapping_add(*b))),
                BinOp::Sub => Ok(Value::Integer(a.wrapping_sub(*b))),
                BinOp::Mul => Ok(Value::Integer(a.wrapping_mul(*b))),
                BinOp::Div => {
                    if *b == 0 {
                        Err(())
                    } else {
                        Ok(Value::Integer(a / b))
                    }
                }
                _ => unreachable!(),
            },
            _ => {
                let (lf, rf) = to_float_pair(&l, &r)?;
                match op {
                    BinOp::Add => {
                        let r = lf + rf;
                        if r.is_nan() {
                            Err(())
                        } else {
                            Ok(Value::Float(r))
                        }
                    }
                    BinOp::Sub => {
                        let r = lf - rf;
                        if r.is_nan() {
                            Err(())
                        } else {
                            Ok(Value::Float(r))
                        }
                    }
                    BinOp::Mul => {
                        let r = lf * rf;
                        if r.is_nan() {
                            Err(())
                        } else {
                            Ok(Value::Float(r))
                        }
                    }
                    BinOp::Div => {
                        if rf == 0.0 || rf.is_nan() {
                            Err(())
                        } else {
                            Ok(Value::Float(lf / rf))
                        }
                    }
                    _ => unreachable!(),
                }
            }
        },

        // String predicates — both operands must be String.
        BinOp::StartsWith => match (l, r) {
            (Value::String(s), Value::String(prefix)) => {
                Ok(Value::Boolean(s.starts_with(prefix.as_str())))
            }
            _ => Err(()),
        },
        BinOp::EndsWith => match (l, r) {
            (Value::String(s), Value::String(suffix)) => {
                Ok(Value::Boolean(s.ends_with(suffix.as_str())))
            }
            _ => Err(()),
        },
        BinOp::Contains => match (l, r) {
            (Value::String(s), Value::String(needle)) => {
                Ok(Value::Boolean(s.contains(needle.as_str())))
            }
            _ => Err(()),
        },
        BinOp::Matches => match (l, r) {
            (Value::String(s), Value::String(pattern)) => {
                // Pattern was validated at parse time; compile here.
                let re = Regex::new(&pattern).map_err(|_| ())?;
                Ok(Value::Boolean(re.is_match(&s)))
            }
            _ => Err(()),
        },

        // Eq/Neq handled above
        BinOp::Eq | BinOp::Neq => unreachable!(),
    }
}

/// Evaluate an Expr against a binding map.
///
/// Returns `Err(())` on: unbound variable, type mismatch, division by zero.
pub(crate) fn eval_expr(
    expr: &Expr,
    binding: &std::collections::HashMap<String, Value>,
) -> Result<Value, ()> {
    match expr {
        Expr::Var(v) => binding.get(v).cloned().ok_or(()),
        Expr::Lit(val) => Ok(val.clone()),
        Expr::UnaryOp(op, arg) => {
            let v = eval_expr(arg, binding)?;
            Ok(Value::Boolean(match op {
                UnaryOp::StringQ => matches!(v, Value::String(_)),
                UnaryOp::IntegerQ => matches!(v, Value::Integer(_)),
                UnaryOp::FloatQ => matches!(v, Value::Float(_)),
                UnaryOp::BooleanQ => matches!(v, Value::Boolean(_)),
                UnaryOp::NilQ => matches!(v, Value::Null),
            }))
        }
        Expr::BinOp(op, lhs, rhs) => {
            let l = eval_expr(lhs, binding)?;
            let r = eval_expr(rhs, binding)?;
            eval_binop(op, l, r)
        }
    }
}

/// Apply all WhereClause::Expr clauses from `where_clauses` to `bindings`.
///
/// Filter-form (`binding: None`) drops the row if the expr is not truthy or errors.
/// Binding-form (`binding: Some(var)`) extends the row with the computed value.
/// Type mismatches and errors silently drop the row.
pub(crate) fn apply_expr_clauses(
    mut bindings: Vec<Binding>,
    where_clauses: &[WhereClause],
) -> Vec<Binding> {
    for clause in where_clauses {
        if let WhereClause::Expr { expr, binding: out } = clause {
            bindings = bindings
                .into_iter()
                .filter_map(|mut b| match eval_expr(expr, &b) {
                    Ok(value) => match out {
                        None => {
                            if is_truthy(&value) {
                                Some(b)
                            } else {
                                None
                            }
                        }
                        Some(var) => {
                            b.insert(var.clone(), value);
                            Some(b)
                        }
                    },
                    Err(_) => None,
                })
                .collect();
        }
    }
    bindings
}

/// Human-readable type name for error messages.
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "String",
        Value::Integer(_) => "Integer",
        Value::Float(_) => "Float",
        Value::Boolean(_) => "Boolean",
        Value::Ref(_) => "Ref",
        Value::Keyword(_) => "Keyword",
        Value::Null => "Null",
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
                vec![FindSpec::Variable("?attr".to_string())],
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
                find: vec![FindSpec::Variable("?attr".to_string())],
                where_clauses: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Symbol("?attr".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                ))],
                as_of: Some(AsOf::Counter(1)),
                valid_at: Some(ValidAt::AnyValidTime),
                with_vars: Vec::new(),
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
                find: vec![FindSpec::Variable("?attr".to_string())],
                where_clauses: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Uuid(alice),
                    EdnValue::Symbol("?attr".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                ))],
                as_of: None,
                valid_at: Some(ValidAt::AnyValidTime),
                with_vars: Vec::new(),
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
            vec![FindSpec::Variable("?e".to_string())],
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
            vec![FindSpec::Variable("?x".to_string())],
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
    fn test_execute_query_not_join_basic() {
        // Query: find entities that have :submitted but NO blocked dependency
        // alice: submitted, has-dep dep1, dep1:blocked=true  -> excluded
        // bob:   submitted, no deps                          -> included
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
            vec![FindSpec::Variable("?x".to_string())],
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

        let executor = DatalogExecutor::new(storage);
        let result = executor
            .execute(crate::query::datalog::types::DatalogCommand::Query(query))
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "only bob should be returned");
            }
            _ => panic!("Expected QueryResults"),
        }
    }

    #[test]
    fn test_execute_query_with_rules_not_join_in_query_body() {
        // Rule: (reachable ?x ?y) :- [?x :edge ?y]
        // Query: find ?y reachable from root that do NOT have a blocked dep
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

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        {
            use crate::query::datalog::types::{Rule, WhereClause as WC};
            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("reachable".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Symbol("?y".to_string()),
                ],
                body: vec![WC::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":edge".to_string()),
                    EdnValue::Symbol("?y".to_string()),
                ))],
            };
            rules
                .write()
                .unwrap()
                .register_rule("reachable".to_string(), rule)
                .unwrap();
        }

        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?y".to_string())],
            vec![
                WhereClause::RuleInvocation {
                    predicate: "reachable".to_string(),
                    args: vec![EdnValue::Uuid(root), EdnValue::Symbol("?y".to_string())],
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

        let executor = DatalogExecutor::new_with_rules(storage, rules);
        let result = executor
            .execute(crate::query::datalog::types::DatalogCommand::Query(query))
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                // a is excluded (has a blocked dep); b passes
                assert_eq!(results.len(), 1, "only b should pass");
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

    // Helper: build a binding map from key-value pairs
    fn binding(pairs: &[(&str, Value)]) -> std::collections::HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_apply_aggregation_count_basic() {
        let bindings = vec![
            binding(&[("?e", Value::Integer(1))]),
            binding(&[("?e", Value::Integer(2))]),
            binding(&[("?e", Value::Integer(3))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Count,
            var: "?e".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0][0], Value::Integer(3));
    }

    #[test]
    fn test_apply_aggregation_count_with_grouping() {
        let bindings = vec![
            binding(&[
                ("?dept", Value::String("eng".to_string())),
                ("?e", Value::Integer(1)),
            ]),
            binding(&[
                ("?dept", Value::String("eng".to_string())),
                ("?e", Value::Integer(2)),
            ]),
            binding(&[
                ("?dept", Value::String("hr".to_string())),
                ("?e", Value::Integer(3)),
            ]),
        ];
        let find_specs = vec![
            FindSpec::Variable("?dept".to_string()),
            FindSpec::Aggregate {
                func: AggFunc::Count,
                var: "?e".to_string(),
            },
        ];
        let mut results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        results.sort_by_key(|r| match &r[0] {
            Value::String(s) => s.clone(),
            _ => String::new(),
        });
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            vec![Value::String("eng".to_string()), Value::Integer(2)]
        );
        assert_eq!(
            results[1],
            vec![Value::String("hr".to_string()), Value::Integer(1)]
        );
    }

    #[test]
    fn test_apply_aggregation_count_distinct() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(1))]),
            binding(&[("?v", Value::Integer(1))]), // duplicate
            binding(&[("?v", Value::Integer(2))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::CountDistinct,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(2));
    }

    #[test]
    fn test_apply_aggregation_count_empty_no_grouping_vars() {
        // count with no grouping vars + zero bindings → [[0]]
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Count,
            var: "?e".to_string(),
        }];
        let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
        assert_eq!(results.len(), 1, "should return one row with 0");
        assert_eq!(results[0][0], Value::Integer(0));
    }

    #[test]
    fn test_apply_aggregation_count_empty_with_grouping_var() {
        // count with grouping var + zero bindings → empty result
        let find_specs = vec![
            FindSpec::Variable("?dept".to_string()),
            FindSpec::Aggregate {
                func: AggFunc::Count,
                var: "?e".to_string(),
            },
        ];
        let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
        assert_eq!(results.len(), 0, "should return empty set");
    }

    #[test]
    fn test_apply_aggregation_sum_integers() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(10))]),
            binding(&[("?v", Value::Integer(20))]),
            binding(&[("?v", Value::Integer(30))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Sum,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(60));
    }

    #[test]
    fn test_apply_aggregation_sum_widens_to_float() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(10))]),
            binding(&[("?v", Value::Float(0.5))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Sum,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Float(10.5));
    }

    #[test]
    fn test_apply_aggregation_sum_distinct_deduplicates() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(5))]),
            binding(&[("?v", Value::Integer(5))]), // duplicate
            binding(&[("?v", Value::Integer(10))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::SumDistinct,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(15)); // 5 + 10, not 5 + 5 + 10
    }

    #[test]
    fn test_apply_aggregation_sum_type_error() {
        let bindings = vec![binding(&[("?v", Value::String("bad".to_string()))])];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Sum,
            var: "?v".to_string(),
        }];
        let result = apply_aggregation(bindings, &find_specs, &[]);
        assert!(result.is_err(), "sum of string should fail");
    }

    #[test]
    fn test_apply_aggregation_min_integers() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(30))]),
            binding(&[("?v", Value::Integer(10))]),
            binding(&[("?v", Value::Integer(20))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Min,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(10));
    }

    #[test]
    fn test_apply_aggregation_max_strings() {
        let bindings = vec![
            binding(&[("?v", Value::String("apple".to_string()))]),
            binding(&[("?v", Value::String("zebra".to_string()))]),
            binding(&[("?v", Value::String("mango".to_string()))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Max,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::String("zebra".to_string()));
    }

    #[test]
    fn test_apply_aggregation_min_type_error_boolean() {
        let bindings = vec![binding(&[("?v", Value::Boolean(true))])];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Min,
            var: "?v".to_string(),
        }];
        let result = apply_aggregation(bindings, &find_specs, &[]);
        assert!(result.is_err(), "min of boolean should fail");
    }

    #[test]
    fn test_apply_aggregation_min_mixed_int_float_error() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(1))]),
            binding(&[("?v", Value::Float(2.0))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Min,
            var: "?v".to_string(),
        }];
        let result = apply_aggregation(bindings, &find_specs, &[]);
        assert!(result.is_err(), "min of mixed Integer/Float should fail");
    }

    #[test]
    fn test_apply_aggregation_skips_nulls_in_sum() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(10))]),
            binding(&[("?v", Value::Null)]),
            binding(&[("?v", Value::Integer(20))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Sum,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(30));
    }

    #[test]
    fn test_apply_aggregation_skips_nulls_in_count() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(1))]),
            binding(&[("?v", Value::Null)]),
            binding(&[("?v", Value::Integer(2))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Count,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
        assert_eq!(results[0][0], Value::Integer(2)); // null not counted
    }

    #[test]
    fn test_apply_aggregation_sum_empty_bindings() {
        let find_specs = vec![FindSpec::Aggregate {
            func: AggFunc::Sum,
            var: "?v".to_string(),
        }];
        let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
        assert_eq!(results.len(), 0, "sum on empty should return empty set");
    }

    #[test]
    fn test_apply_aggregation_with_var_grouping() {
        // :with ?e adds ?e to the group key. Two entities with same dept but different ?e
        // form separate groups.
        let bindings = vec![
            binding(&[
                ("?dept", Value::String("eng".to_string())),
                ("?salary", Value::Integer(50)),
                ("?e", Value::Integer(1)),
            ]),
            binding(&[
                ("?dept", Value::String("eng".to_string())),
                ("?salary", Value::Integer(50)),
                ("?e", Value::Integer(2)),
            ]),
        ];
        let find_specs = vec![
            FindSpec::Variable("?dept".to_string()),
            FindSpec::Aggregate {
                func: AggFunc::Sum,
                var: "?salary".to_string(),
            },
        ];
        // Without :with: group key = ("eng",). Both bindings in one group → sum = 100.
        let results_no_with = apply_aggregation(bindings.clone(), &find_specs, &[]).unwrap();
        assert_eq!(results_no_with.len(), 1);
        assert_eq!(results_no_with[0][1], Value::Integer(100));
        // With :with ?e: group key = ("eng", e). Two separate groups → two rows, each sum = 50.
        let results_with = apply_aggregation(bindings, &find_specs, &["?e".to_string()]).unwrap();
        assert_eq!(results_with.len(), 2);
        assert_eq!(results_with[0][1], Value::Integer(50));
    }
}

#[cfg(test)]
mod expr_eval_tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::datalog::types::{BinOp, Expr, UnaryOp};
    use std::collections::HashMap;

    fn b(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_eval_lit() {
        let e = Expr::Lit(Value::Integer(42));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Integer(42)));
    }

    #[test]
    fn test_eval_var_bound() {
        let e = Expr::Var("?x".to_string());
        let binding = b(&[("?x", Value::Integer(10))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Integer(10)));
    }

    #[test]
    fn test_eval_var_unbound_is_err() {
        let e = Expr::Var("?x".to_string());
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_lt_true() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let binding = b(&[("?v", Value::Integer(50))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_lt_false() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let binding = b(&[("?v", Value::Integer(150))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_add_integers() {
        let e = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Var("?a".to_string())),
            Box::new(Expr::Var("?b".to_string())),
        );
        let binding = b(&[("?a", Value::Integer(3)), ("?b", Value::Integer(4))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Integer(7)));
    }

    #[test]
    fn test_eval_add_int_float_promotes() {
        let e = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Lit(Value::Integer(1))),
            Box::new(Expr::Lit(Value::Float(1.5))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Float(2.5)));
    }

    #[test]
    fn test_eval_div_integer_truncates() {
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Integer(5))),
            Box::new(Expr::Lit(Value::Integer(2))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Integer(2)));
    }

    #[test]
    fn test_eval_div_by_zero_is_err() {
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Integer(5))),
            Box::new(Expr::Lit(Value::Integer(0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_eq_strings() {
        let e = Expr::BinOp(
            BinOp::Eq,
            Box::new(Expr::Lit(Value::String("Alice".to_string()))),
            Box::new(Expr::Lit(Value::String("Alice".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_eq_int_float_false() {
        // Different Value variants → structural inequality
        let e = Expr::BinOp(
            BinOp::Eq,
            Box::new(Expr::Lit(Value::Integer(1))),
            Box::new(Expr::Lit(Value::Float(1.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_type_mismatch_comparison_is_err() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Lit(Value::String("hello".to_string()))),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_string_q_true() {
        let e = Expr::UnaryOp(
            UnaryOp::StringQ,
            Box::new(Expr::Lit(Value::String("hi".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_string_q_false() {
        let e = Expr::UnaryOp(UnaryOp::StringQ, Box::new(Expr::Lit(Value::Integer(1))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_starts_with_true() {
        let e = Expr::BinOp(
            BinOp::StartsWith,
            Box::new(Expr::Lit(Value::String("foobar".to_string()))),
            Box::new(Expr::Lit(Value::String("foo".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_ends_with_true() {
        let e = Expr::BinOp(
            BinOp::EndsWith,
            Box::new(Expr::Lit(Value::String("foobar".to_string()))),
            Box::new(Expr::Lit(Value::String("bar".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_contains_true() {
        let e = Expr::BinOp(
            BinOp::Contains,
            Box::new(Expr::Lit(Value::String("engineer at co".to_string()))),
            Box::new(Expr::Lit(Value::String("engineer".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_matches_true() {
        let e = Expr::BinOp(
            BinOp::Matches,
            Box::new(Expr::Lit(Value::String("test@example.com".to_string()))),
            Box::new(Expr::Lit(Value::String("^[^@]+@[^@]+$".to_string()))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_is_truthy() {
        assert!(is_truthy(&Value::Boolean(true)));
        assert!(!is_truthy(&Value::Boolean(false)));
        assert!(is_truthy(&Value::Integer(1)));
        assert!(!is_truthy(&Value::Integer(0)));
        assert!(is_truthy(&Value::Float(0.1)));
        assert!(!is_truthy(&Value::Float(0.0)));
        assert!(!is_truthy(&Value::Null));
        assert!(!is_truthy(&Value::String("hi".to_string())));
    }

    #[test]
    fn test_apply_expr_filter_keeps_truthy() {
        // [(< ?v 100)] — keeps row where ?v < 100
        use crate::query::datalog::types::WhereClause;
        let expr = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let clauses = vec![WhereClause::Expr {
            expr,
            binding: None,
        }];
        let bindings = vec![
            b(&[("?v", Value::Integer(50))]),
            b(&[("?v", Value::Integer(150))]),
        ];
        let result = apply_expr_clauses(bindings, &clauses);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].get("?v"), Some(&Value::Integer(50)));
    }

    #[test]
    fn test_apply_expr_binding_extends_row() {
        // [(+ ?a ?b) ?sum] — binds ?sum
        use crate::query::datalog::types::WhereClause;
        let expr = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Var("?a".to_string())),
            Box::new(Expr::Var("?b".to_string())),
        );
        let clauses = vec![WhereClause::Expr {
            expr,
            binding: Some("?sum".to_string()),
        }];
        let bindings = vec![b(&[("?a", Value::Integer(3)), ("?b", Value::Integer(4))])];
        let result = apply_expr_clauses(bindings, &clauses);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].get("?sum"), Some(&Value::Integer(7)));
    }

    #[test]
    fn test_apply_expr_type_mismatch_drops_row() {
        // [(< ?v 100)] where ?v = "hello" — type mismatch silently drops row
        use crate::query::datalog::types::WhereClause;
        let expr = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let clauses = vec![WhereClause::Expr {
            expr,
            binding: None,
        }];
        let bindings = vec![b(&[("?v", Value::String("hello".to_string()))])];
        let result = apply_expr_clauses(bindings, &clauses);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_execute_expr_filter_lt() {
        use crate::graph::storage::FactStorage;
        use crate::query::datalog::rules::RuleRegistry;
        use std::sync::{Arc, RwLock};

        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let executor = DatalogExecutor::new_with_rules(storage.clone(), rules);

        // Transact two items with different prices
        executor
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(transact [[:item1 :item/price 50] [:item2 :item/price 150]])",
                )
                .unwrap(),
            )
            .unwrap();

        // Query: find items where price < 100
        let result = executor.execute(
            crate::query::datalog::parser::parse_datalog_command(
                "(query [:find ?e :where [?e :item/price ?p] [(< ?p 100)]])",
            )
            .unwrap(),
        );

        assert!(result.is_ok(), "expr filter query failed");
        match result.unwrap() {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "expected exactly one result");
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_apply_or_clauses_union_from_two_branches() {
        // e1 has :color :red, e2 has :color :blue.
        // or-only where clause: (or [?e :color :red] [?e :color :blue])
        // Without apply_or_clauses, get_patterns() returns [] → match_patterns returns
        // [{}] (one empty binding) → extract_variables finds no ?e binding → 0 results.
        // With apply_or_clauses, both entities are returned → 2 results.
        use uuid::Uuid;
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (e1, ":color".to_string(), Value::Keyword(":red".to_string())),
                    (
                        e2,
                        ":color".to_string(),
                        Value::Keyword(":blue".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        let executor = DatalogExecutor::new(storage.clone());
        let cmd = crate::query::datalog::parser::parse_datalog_command(
            r#"(query [:find ?e
                       :where (or [?e :color :red] [?e :color :blue])])"#,
        )
        .unwrap();
        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 2, "both entities should match via or");
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_apply_or_clauses_deduplication() {
        // e1 has :color :red AND :shape :circle.
        // or clause: (or [?e :color :red] [?e :shape :circle])
        // Without apply_or_clauses: or is skipped → 0 results (no non-or patterns).
        // With apply_or_clauses: e1 is returned by both branches → deduplicated to 1 result.
        use uuid::Uuid;
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (e1, ":color".to_string(), Value::Keyword(":red".to_string())),
                    (
                        e1,
                        ":shape".to_string(),
                        Value::Keyword(":circle".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        let executor = DatalogExecutor::new(storage.clone());
        let cmd = crate::query::datalog::parser::parse_datalog_command(
            r#"(query [:find ?e
                       :where (or [?e :color :red] [?e :shape :circle])])"#,
        )
        .unwrap();
        let result = executor.execute(cmd).unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    1,
                    "one entity matched by both branches → deduplicated"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }
}
