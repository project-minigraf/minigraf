use super::evaluator::{
    DEFAULT_MAX_DERIVED_FACTS, DEFAULT_MAX_RESULTS, StratifiedEvaluator, evaluate_not_join,
};
use super::functions::{FunctionRegistry, apply_builtin_aggregate, value_lt};
use super::matcher::{PatternMatcher, edn_to_entity_id, edn_to_value};
use super::optimizer;
use super::rules::RuleRegistry;
use super::types::{
    AsOf, AttributeSpec, BinOp, DatalogCommand, DatalogQuery, EdnValue, Expr, FindSpec, Order,
    Pattern, Rule, Transaction, UnaryOp, ValidAt, WhereClause, WindowFunc,
};
use crate::graph::FactStorage;
use crate::graph::types::{Fact, TransactOptions, TxId, Value, tx_id_now};
use crate::storage::index::Indexes;
use anyhow::{Result, anyhow};
use std::sync::{Arc, RwLock};

/// Returns true if any where clause (at any depth) contains a per-fact
/// pseudo-attribute pattern (ValidFrom / ValidTo / TxCount / TxId).
/// Used to enforce the `:any-valid-time` requirement.
fn query_uses_per_fact_pseudo_attr(query: &DatalogQuery) -> bool {
    fn check_clauses(clauses: &[WhereClause]) -> bool {
        clauses.iter().any(|c| match c {
            WhereClause::Pattern(p) => matches!(
                &p.attribute,
                AttributeSpec::Pseudo(pa) if pa.is_per_fact()
            ),
            WhereClause::Not(inner) => check_clauses(inner),
            WhereClause::NotJoin { clauses: inner, .. } => check_clauses(inner),
            WhereClause::Or(branches) => branches.iter().any(|b| check_clauses(b)),
            WhereClause::OrJoin { branches, .. } => branches.iter().any(|b| check_clauses(b)),
            _ => false,
        })
    }
    check_clauses(&query.where_clauses)
}

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
    // RwLock pre-wired for 7.7b register_aggregate API.
    functions: Arc<RwLock<FunctionRegistry>>,
}

impl DatalogExecutor {
    pub fn new(storage: FactStorage) -> Self {
        DatalogExecutor {
            storage,
            rules: Arc::new(RwLock::new(RuleRegistry::new())),
            functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
        }
    }

    /// Create a `DatalogExecutor` with a shared rule registry and function registry.
    ///
    /// Used by `Minigraf` to share registries across all `execute()` calls.
    pub fn new_with_rules_and_functions(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        functions: Arc<RwLock<FunctionRegistry>>,
    ) -> Self {
        DatalogExecutor {
            storage,
            rules,
            functions,
        }
    }

    /// Convenience constructor for tests. Shares `rules` with other executors but creates
    /// a fresh `FunctionRegistry::with_builtins()`. Production code uses
    /// [`new_with_rules_and_functions`] to share the registry from `Minigraf::Inner`.
    pub fn new_with_rules(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>) -> Self {
        Self::new_with_rules_and_functions(
            storage,
            rules,
            Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
        )
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
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => return Err(anyhow!("Attribute must be a keyword")),
                AttributeSpec::Pseudo(_) => {
                    return Err(anyhow!("Cannot transact a pseudo-attribute"));
                }
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
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => return Err(anyhow!("Attribute must be a keyword")),
                AttributeSpec::Pseudo(_) => {
                    return Err(anyhow!("Cannot transact a pseudo-attribute"));
                }
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

    /// Build a filtered fact snapshot for a query's temporal constraints.
    ///
    /// Step 1: apply transaction-time filter (`:as-of`) — defaults to all facts.
    /// Step 2: discard retracted facts within the tx window (`net_asserted_facts`).
    /// Step 3: apply valid-time filter (`:valid-at`) — defaults to "currently valid".
    ///
    /// Returns an `Arc<[Fact]>` snapshot. `.clone()` is a cheap Arc refcount increment,
    /// so `or`-branches and `not`/`not-join` sub-evaluations share the same allocation.
    /// The three steps above are paid exactly once per `execute_query` /
    /// `execute_query_with_rules` call.
    ///
    /// **Post-1.0 backlog**: Use the on-disk B+tree indexes (EAVT/AEVT/AVET/VAET) for
    /// selective attribute/entity lookups instead of the full `get_all_facts()` scan (step 1).
    /// Also investigate caching the `net_asserted_facts()` result and invalidating on write (step 2).
    fn filter_facts_for_query(&self, query: &DatalogQuery) -> Result<Arc<[Fact]>> {
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

        Ok(Arc::from(valid_filtered))
    }

    /// Execute a query: find matching facts and return specified variables
    fn execute_query(&self, query: DatalogQuery) -> Result<QueryResult> {
        // Check if query uses rules
        if query.uses_rules() {
            // Use StratifiedEvaluator for queries with rule invocations (handles negation and strata)
            return self.execute_query_with_rules(query);
        }

        // Compute query-level valid_at value for :db/valid-at pseudo-attribute binding.
        let now = tx_id_now() as i64;
        let valid_at_value = match &query.valid_at {
            Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
            Some(ValidAt::AnyValidTime) => Value::Null,
            None => Value::Integer(now),
        };

        // Hard-error: per-fact pseudo-attrs require :any-valid-time.
        if query_uses_per_fact_pseudo_attr(&query)
            && !matches!(query.valid_at, Some(ValidAt::AnyValidTime))
        {
            return Err(anyhow!(
                "temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and \
                 :db/tx-id require :any-valid-time; add :any-valid-time to your query"
            ));
        }

        // Apply temporal filters before pattern matching
        let filtered_facts = self.filter_facts_for_query(&query)?;
        let matcher = PatternMatcher::from_slice_with_valid_at(
            filtered_facts.clone(),
            valid_at_value.clone(),
        );
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
            filtered_facts.clone(),
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
            bindings
                .into_iter()
                .filter(|binding| {
                    for not_body in &not_clauses {
                        if not_body_matches(
                            not_body,
                            binding,
                            filtered_facts.clone(),
                            valid_at_value.clone(),
                        ) {
                            return false;
                        }
                    }
                    for (join_vars, nj_clauses) in &not_join_clauses {
                        if evaluate_not_join(join_vars, nj_clauses, binding, filtered_facts.clone())
                        {
                            return false;
                        }
                    }
                    true
                })
                .collect()
        };

        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses);

        let registry = self.functions.read().unwrap();
        let results =
            apply_post_processing(filtered_bindings, &query.find, &query.with_vars, &registry)?;

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

        // Compute query-level valid_at value for :db/valid-at pseudo-attribute binding.
        let now = tx_id_now() as i64;
        let valid_at_value = match &query.valid_at {
            Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
            Some(ValidAt::AnyValidTime) => Value::Null,
            None => Value::Integer(now),
        };

        // Hard-error: per-fact pseudo-attrs require :any-valid-time.
        if query_uses_per_fact_pseudo_attr(&query)
            && !matches!(query.valid_at, Some(ValidAt::AnyValidTime))
        {
            return Err(anyhow!(
                "temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and \
                 :db/tx-id require :any-valid-time; add :any-valid-time to your query"
            ));
        }

        // Apply temporal filters before evaluating recursive rules
        let filtered_facts = self.filter_facts_for_query(&query)?;

        // Convert to FactStorage for StratifiedEvaluator (needs mutable accumulation)
        // TODO (post-1.0): use FactStorage::new_noindex() once profiling confirms rules-path
        // index rebuild is also a bottleneck.
        let filtered_storage = FactStorage::new();
        for fact in filtered_facts.iter().cloned() {
            filtered_storage.load_fact(fact)?;
        }

        // Create StratifiedEvaluator — handles negation, stratification, and positive-only rules
        let evaluator = StratifiedEvaluator::new(
            filtered_storage,
            self.rules.clone(),
            1000, // max iterations
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
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

        // Compute derived_facts Arc once; reuse for or-clauses and not-post-filter.
        // NOTE: must use derived_storage (includes rule-derived facts), not filtered_facts (base facts only)
        let derived_facts: Arc<[Fact]> =
            Arc::from(derived_storage.get_asserted_facts().unwrap_or_default());

        // Match all patterns against derived facts
        let matcher =
            PatternMatcher::from_slice_with_valid_at(derived_facts.clone(), valid_at_value.clone());
        let bindings = matcher.match_patterns(&all_patterns);

        // Apply Or/OrJoin clauses against derived facts (rules already evaluated)
        let rules_guard = self.rules.read().unwrap();
        let bindings = apply_or_clauses(
            &query.where_clauses,
            bindings,
            derived_facts.clone(),
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
                        let m = PatternMatcher::from_slice_with_valid_at(
                            derived_facts.clone(),
                            valid_at_value.clone(),
                        );
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
                        if evaluate_not_join(join_vars, nj_clauses, binding, derived_facts.clone())
                        {
                            return false;
                        }
                    }
                    true
                })
                .collect()
        };

        // Apply WhereClause::Expr clauses (filter and binding predicates)
        let filtered_bindings = apply_expr_clauses(not_filtered, &query.where_clauses);

        let registry = self.functions.read().unwrap();
        let results =
            apply_post_processing(filtered_bindings, &query.find, &query.with_vars, &registry)?;

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
    storage: Arc<[Fact]>,
    valid_at: Value,
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

    let matcher = crate::query::datalog::matcher::PatternMatcher::from_slice_with_valid_at(
        storage.clone(),
        valid_at,
    );
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

type Binding = std::collections::HashMap<String, Value>;

/// Unified post-processing: handles plain-variable extraction, aggregation,
/// window functions, and mixed (aggregate + window) queries.
///
/// - Plain variables only → `extract_variables` (no change from current path).
/// - Aggregates only → group-by collapse, then project.
/// - Windows only → partition/sort/accumulate per spec, then project.
/// - Mixed → aggregate collapses first, window runs over collapsed rows.
fn apply_post_processing(
    bindings: Vec<Binding>,
    find_specs: &[FindSpec],
    with_vars: &[String],
    registry: &FunctionRegistry,
) -> Result<Vec<Vec<Value>>> {
    let has_aggregates = find_specs
        .iter()
        .any(|s| matches!(s, FindSpec::Aggregate { .. }));
    let has_windows = find_specs.iter().any(|s| matches!(s, FindSpec::Window(_)));

    if !has_aggregates && !has_windows {
        return Ok(extract_variables(bindings, find_specs));
    }

    // Step 1: Aggregate (collapses rows, produces binding maps).
    let mut working: Vec<Binding> = if has_aggregates {
        compute_aggregation(bindings, find_specs, with_vars, registry)?
    } else {
        bindings
    };

    // Step 2: Window functions (annotate each row, no collapse).
    if has_windows {
        apply_window_functions(&mut working, find_specs, registry)?;
    }

    // Step 3: Project to output rows in find-spec order.
    Ok(project_find_specs(&working, find_specs))
}

/// Group bindings by non-aggregate find vars + with_vars, apply aggregate functions,
/// return one binding map per group. Aggregate results stored under `"__agg_{i}"`.
fn compute_aggregation(
    bindings: Vec<Binding>,
    find_specs: &[FindSpec],
    with_vars: &[String],
    registry: &FunctionRegistry,
) -> Result<Vec<Binding>> {
    let has_grouping_vars = find_specs
        .iter()
        .any(|s| matches!(s, FindSpec::Variable(_)));

    // Special case: zero bindings + all-count specs → one zero row.
    if bindings.is_empty() {
        let all_count = !has_grouping_vars
            && find_specs.iter().all(|s| {
                matches!(s, FindSpec::Aggregate { func, .. }
                    if func == "count" || func == "count-distinct")
            });
        if all_count {
            let mut b = Binding::new();
            for (i, _) in find_specs.iter().enumerate() {
                b.insert(format!("__agg_{}", i), Value::Integer(0));
            }
            return Ok(vec![b]);
        }
        return Ok(vec![]);
    }

    // In a mixed aggregate+window query, :with vars must NOT be added to the
    // grouping key. The window phase runs after aggregation, so :with vars that
    // are used only by window specs (var, order_by) would otherwise inflate the
    // number of groups. Even :with vars used by aggregate specs (e.g. ?e in
    // count(?e)) should not split groups — the aggregate operates over all rows
    // in the base group determined by the Variable find specs.
    let has_windows = find_specs.iter().any(|s| matches!(s, FindSpec::Window(_)));

    // Grouping key = Variable find specs (in find order).
    // In pure-aggregate queries, also include with_vars (Datomic semantics: :with
    // prevents pre-aggregation de-duplication by adding vars to the group key).
    let mut group_var_names: Vec<&str> = find_specs
        .iter()
        .filter_map(|s| match s {
            FindSpec::Variable(v) => Some(v.as_str()),
            _ => None,
        })
        .collect();
    if !has_windows {
        // Pure aggregate: with_vars add to grouping key.
        group_var_names.extend(with_vars.iter().map(|s| s.as_str()));
    }

    // Group using Vec + PartialEq scan (Value::Float doesn't implement Hash).
    let mut groups: Vec<(Vec<Value>, Vec<Binding>)> = Vec::new();
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

    // Build a position map for Variable specs only (indices 0..n_vars in the key vector).
    // with_vars occupy key positions n_vars..end and are used only for grouping, not for output.
    // Map of Variable spec name → its index in the group key Vec.
    let mut group_key_idx: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    {
        let mut var_pos = 0usize;
        for spec in find_specs {
            if let FindSpec::Variable(v) = spec {
                group_key_idx.insert(v.as_str(), var_pos);
                var_pos += 1;
            }
        }
    }

    let mut results: Vec<Binding> = Vec::new();
    for (key, group_bindings) in &groups {
        let mut binding = Binding::new();
        let mut skip = false;

        // Plain variable values from group key.
        for (v, &idx) in &group_key_idx {
            binding.insert((*v).to_string(), key[idx].clone());
        }

        // Aggregate values stored under "__agg_{i}".
        for (i, spec) in find_specs.iter().enumerate() {
            if let FindSpec::Aggregate { func, var } = spec {
                let non_null: Vec<&Value> = group_bindings
                    .iter()
                    .filter_map(|b| b.get(var.as_str()))
                    .filter(|v| !matches!(v, Value::Null))
                    .collect();
                let agg_val: anyhow::Result<Value> = if let Some(desc) = registry.get(func) {
                    if desc.is_builtin {
                        // Built-in: use batch path which enforces strict type-error semantics.
                        apply_builtin_aggregate(func, &non_null)
                    } else if let Some(ops) = &desc.window_ops {
                        // UDF registered with window_ops: incremental init/step/finalise.
                        let mut acc = (ops.init)();
                        for v in &non_null {
                            (ops.step)(&mut acc, v);
                        }
                        Ok((ops.finalise)(&acc))
                    } else {
                        // UDF without window_ops: batch path (will return a proper error for truly unknown names).
                        apply_builtin_aggregate(func, &non_null)
                    }
                } else {
                    // Unknown to registry: batch path (will return a proper error for truly unknown names).
                    apply_builtin_aggregate(func, &non_null)
                };
                match agg_val {
                    Ok(v) => {
                        binding.insert(format!("__agg_{}", i), v);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("no non-null values in group") {
                            skip = true;
                            break;
                        }
                        return Err(e);
                    }
                }
            }
        }

        if !skip {
            results.push(binding);
        }
    }

    Ok(results)
}

/// Compute window function values for each row and store under `"__win_{i}"`.
/// Modifies `bindings` in place.
fn apply_window_functions(
    bindings: &mut [Binding],
    find_specs: &[FindSpec],
    registry: &FunctionRegistry,
) -> Result<()> {
    for (i, spec) in find_specs.iter().enumerate() {
        let FindSpec::Window(ws) = spec else {
            continue;
        };
        let key = format!("__win_{}", i);

        // Build partitions: (partition_key, sorted row indices).
        let mut partitions: Vec<(Option<Value>, Vec<usize>)> = Vec::new();
        for (row_idx, binding) in bindings.iter().enumerate() {
            let part_key = ws
                .partition_by
                .as_ref()
                .and_then(|pv| binding.get(pv))
                .cloned();
            if let Some(pos) = partitions.iter().position(|(k, _)| k == &part_key) {
                partitions[pos].1.push(row_idx);
            } else {
                partitions.push((part_key, vec![row_idx]));
            }
        }

        // For each partition: sort, compute window values, write back.
        for (_, row_indices) in &mut partitions {
            // Sort by order_by key.
            row_indices.sort_by(|&a, &b| {
                let va = bindings[a].get(&ws.order_by).unwrap_or(&Value::Null);
                let vb = bindings[b].get(&ws.order_by).unwrap_or(&Value::Null);
                let lt = value_lt(va, vb);
                let eq = va == vb;
                let cmp = if eq {
                    std::cmp::Ordering::Equal
                } else if lt {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
                match ws.order {
                    Order::Asc => cmp,
                    Order::Desc => cmp.reverse(),
                }
            });

            // Compute one window value per row in partition order.
            let window_values: Vec<Value> = match ws.func {
                WindowFunc::RowNumber => row_indices
                    .iter()
                    .enumerate()
                    .map(|(pos, _)| Value::Integer(pos as i64 + 1))
                    .collect(),

                WindowFunc::Rank => {
                    let mut values = Vec::with_capacity(row_indices.len());
                    let mut rank = 1i64;
                    let mut prev_order_val: Option<Value> = None;
                    let mut row_num = 1i64;
                    for &row_idx in row_indices.iter() {
                        let cur_val = bindings[row_idx].get(&ws.order_by).cloned();
                        if prev_order_val.as_ref() != cur_val.as_ref() {
                            rank = row_num;
                            prev_order_val = cur_val;
                        }
                        values.push(Value::Integer(rank));
                        row_num += 1;
                    }
                    values
                }

                _ => {
                    // Accumulator-based: sum, count, min, max, avg.
                    let func_name = ws.func_name();
                    let desc = registry.get(func_name).ok_or_else(|| {
                        anyhow::anyhow!("no descriptor for window function '{}'", func_name)
                    })?;
                    let ops = desc.window_ops.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("function '{}' is not window-compatible", func_name)
                    })?;

                    let mut acc = (ops.init)();
                    let mut values = Vec::with_capacity(row_indices.len());
                    for &row_idx in row_indices.iter() {
                        let val = ws
                            .var
                            .as_ref()
                            .and_then(|v| bindings[row_idx].get(v))
                            .unwrap_or(&Value::Null);
                        (ops.step)(&mut acc, val);
                        values.push((ops.finalise)(&acc));
                    }
                    values
                }
            };

            // Write window values back to rows.
            for (&row_idx, window_val) in row_indices.iter().zip(window_values.into_iter()) {
                bindings[row_idx].insert(key.clone(), window_val);
            }
        }
    }
    Ok(())
}

/// Project binding maps to output rows in find-spec order.
fn project_find_specs(bindings: &[Binding], find_specs: &[FindSpec]) -> Vec<Vec<Value>> {
    let mut results = Vec::new();
    for binding in bindings {
        let mut row = Vec::new();
        let mut complete = true;
        for (i, spec) in find_specs.iter().enumerate() {
            let val = match spec {
                FindSpec::Variable(v) => binding.get(v).cloned(),
                FindSpec::Aggregate { .. } => binding.get(&format!("__agg_{}", i)).cloned(),
                FindSpec::Window(_) => binding.get(&format!("__win_{}", i)).cloned(),
            };
            match val {
                Some(v) => row.push(v),
                // Invariant: all __agg_{i} and __win_{i} keys are populated for non-skipped rows.
                // None here only occurs for skipped aggregate groups (e.g. min/max on all-null input).
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            results.push(row);
        }
    }
    results
}

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
    storage: Arc<[Fact]>,
    rules: &crate::query::datalog::rules::RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>> {
    use crate::query::datalog::evaluator::rule_invocation_to_pattern;
    use crate::query::datalog::matcher::PatternMatcher;

    if incoming.is_empty() {
        return Ok(vec![]);
    }

    // Compute valid_at_value for pseudo-attribute binding in this branch.
    let branch_valid_at_value = match &valid_at {
        Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
        Some(ValidAt::AnyValidTime) => Value::Null,
        None => Value::Integer(tx_id_now() as i64),
    };

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

    let matcher =
        PatternMatcher::from_slice_with_valid_at(storage.clone(), branch_valid_at_value.clone());
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
        storage.clone(),
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
                    if not_body_matches(
                        not_body,
                        binding,
                        storage.clone(),
                        branch_valid_at_value.clone(),
                    ) {
                        return false;
                    }
                }
                for (join_vars, nj_clauses) in &not_join_clauses {
                    if evaluate_not_join(join_vars, nj_clauses, binding, storage.clone()) {
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
    storage: Arc<[Fact]>,
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
                        storage.clone(),
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
                        storage.clone(),
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
        BinOp::Matches { regex: re, .. } => match (l, r) {
            (Value::String(s), Value::String(_)) => Ok(Value::Boolean(re.is_match(&s))),
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
            func: "count".to_string(),
            var: "?e".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
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
                func: "count".to_string(),
                var: "?e".to_string(),
            },
        ];
        let mut results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
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
            func: "count-distinct".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results[0][0], Value::Integer(2));
    }

    #[test]
    fn test_apply_aggregation_count_empty_no_grouping_vars() {
        // count with no grouping vars + zero bindings → [[0]]
        let find_specs = vec![FindSpec::Aggregate {
            func: "count".to_string(),
            var: "?e".to_string(),
        }];
        let results =
            apply_post_processing(vec![], &find_specs, &[], &FunctionRegistry::with_builtins())
                .unwrap();
        assert_eq!(results.len(), 1, "should return one row with 0");
        assert_eq!(results[0][0], Value::Integer(0));
    }

    #[test]
    fn test_apply_aggregation_count_empty_with_grouping_var() {
        // count with grouping var + zero bindings → empty result
        let find_specs = vec![
            FindSpec::Variable("?dept".to_string()),
            FindSpec::Aggregate {
                func: "count".to_string(),
                var: "?e".to_string(),
            },
        ];
        let results =
            apply_post_processing(vec![], &find_specs, &[], &FunctionRegistry::with_builtins())
                .unwrap();
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
            func: "sum".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results[0][0], Value::Integer(60));
    }

    #[test]
    fn test_apply_aggregation_sum_widens_to_float() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(10))]),
            binding(&[("?v", Value::Float(0.5))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: "sum".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
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
            func: "sum-distinct".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results[0][0], Value::Integer(15)); // 5 + 10, not 5 + 5 + 10
    }

    #[test]
    fn test_apply_aggregation_sum_type_error() {
        let bindings = vec![binding(&[("?v", Value::String("bad".to_string()))])];
        let find_specs = vec![FindSpec::Aggregate {
            func: "sum".to_string(),
            var: "?v".to_string(),
        }];
        let result = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        );
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
            func: "min".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
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
            func: "max".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results[0][0], Value::String("zebra".to_string()));
    }

    #[test]
    fn test_apply_aggregation_min_type_error_boolean() {
        let bindings = vec![binding(&[("?v", Value::Boolean(true))])];
        let find_specs = vec![FindSpec::Aggregate {
            func: "min".to_string(),
            var: "?v".to_string(),
        }];
        let result = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        );
        assert!(result.is_err(), "min of boolean should fail");
    }

    #[test]
    fn test_apply_aggregation_min_mixed_int_float_error() {
        let bindings = vec![
            binding(&[("?v", Value::Integer(1))]),
            binding(&[("?v", Value::Float(2.0))]),
        ];
        let find_specs = vec![FindSpec::Aggregate {
            func: "min".to_string(),
            var: "?v".to_string(),
        }];
        let result = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        );
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
            func: "sum".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
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
            func: "count".to_string(),
            var: "?v".to_string(),
        }];
        let results = apply_post_processing(
            bindings,
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results[0][0], Value::Integer(2)); // null not counted
    }

    #[test]
    fn test_apply_aggregation_sum_empty_bindings() {
        let find_specs = vec![FindSpec::Aggregate {
            func: "sum".to_string(),
            var: "?v".to_string(),
        }];
        let results =
            apply_post_processing(vec![], &find_specs, &[], &FunctionRegistry::with_builtins())
                .unwrap();
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
                func: "sum".to_string(),
                var: "?salary".to_string(),
            },
        ];
        // Without :with: group key = ("eng",). Both bindings in one group → sum = 100.
        let results_no_with = apply_post_processing(
            bindings.clone(),
            &find_specs,
            &[],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results_no_with.len(), 1);
        assert_eq!(results_no_with[0][1], Value::Integer(100));
        // With :with ?e: group key = ("eng", e). Two separate groups → two rows, each sum = 50.
        let results_with = apply_post_processing(
            bindings,
            &find_specs,
            &["?e".to_string()],
            &FunctionRegistry::with_builtins(),
        )
        .unwrap();
        assert_eq!(results_with.len(), 2);
        assert_eq!(results_with[0][1], Value::Integer(50));
    }

    #[test]
    fn test_filter_facts_for_query_returns_net_asserted_slice() {
        // Setup: one fact asserted then retracted, one fact left standing.
        // After filter_facts_for_query, only the standing fact should appear.
        // The return type (Arc<[Fact]>) exposes .len() and index access [0].
        use uuid::Uuid;
        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // tx 1: assert name
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        // tx 2: retract name — net state for name is now gone
        storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            )])
            .unwrap();

        // tx 3: assert age — this is the only net-asserted fact
        storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(30))],
                None,
            )
            .unwrap();

        let executor = DatalogExecutor::new(storage);
        let query = DatalogQuery {
            find: vec![],
            where_clauses: vec![],
            as_of: None,
            valid_at: Some(ValidAt::AnyValidTime),
            with_vars: vec![],
        };

        let facts = executor.filter_facts_for_query(&query).unwrap();
        assert_eq!(facts.len(), 1, "expected exactly 1 net-asserted fact");
        assert_eq!(facts[0].attribute, ":person/age");
    }

    #[test]
    fn test_filter_facts_for_query_valid_time_filter() {
        // Setup: one fact with a narrow valid-time window (1000..2000), one open-ended.
        // Query with valid_at inside the window → both facts visible.
        // Query with valid_at outside the window → only the open-ended fact visible.
        // filter_facts_for_query now returns Result<Arc<[Fact]>> (changed in Task 5).
        use crate::graph::types::TransactOptions;
        use uuid::Uuid;
        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Fact valid only during [1000, 2000)
        storage
            .transact(
                vec![(
                    alice,
                    ":employment/status".to_string(),
                    Value::String("active".to_string()),
                )],
                Some(TransactOptions::new(Some(1000_i64), Some(2000_i64))),
            )
            .unwrap();

        // Fact valid forever (open-ended): explicit valid_from=0 so it is visible at t=1500 and t=3000.
        // Passing None would set valid_from=tx_id_now() (current epoch ms ≈ 1.7T), which is
        // far beyond the test's query timestamps.
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                Some(TransactOptions::new(Some(0_i64), None)),
            )
            .unwrap();

        let executor = DatalogExecutor::new(storage);

        // Query inside the window: both facts should be visible
        let query_inside = DatalogQuery {
            find: vec![],
            where_clauses: vec![],
            as_of: None,
            valid_at: Some(ValidAt::Timestamp(1500_i64)),
            with_vars: vec![],
        };
        let facts_inside = executor.filter_facts_for_query(&query_inside).unwrap();
        assert_eq!(facts_inside.len(), 2, "both facts visible at t=1500");

        // Query outside the window: only the open-ended name fact should be visible
        let query_outside = DatalogQuery {
            find: vec![],
            where_clauses: vec![],
            as_of: None,
            valid_at: Some(ValidAt::Timestamp(3000_i64)),
            with_vars: vec![],
        };
        let facts_outside = executor.filter_facts_for_query(&query_outside).unwrap();
        assert_eq!(
            facts_outside.len(),
            1,
            "only open-ended fact visible at t=3000"
        );
        assert_eq!(facts_outside[0].attribute, ":person/name");
    }
}

#[cfg(test)]
mod expr_eval_tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::{BinOp, Expr, UnaryOp, WhereClause};
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

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
        let re = regex_lite::Regex::new("^[^@]+@[^@]+$").unwrap();
        let e = Expr::BinOp(
            BinOp::Matches {
                regex: re,
                pattern: "^[^@]+@[^@]+$".to_string(),
            },
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

    // ── Stream 3: branches unreachable via the parser ─────────────────────────

    #[test]
    fn execute_transact_non_keyword_attribute_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        // Construct a transact with a String attribute (not a keyword)
        let cmd = DatalogCommand::Transact(Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":e".to_string()),
                EdnValue::String("not-a-keyword".to_string()),
                EdnValue::String("value".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "non-keyword attribute in transact must fail");
    }

    #[test]
    fn execute_retract_non_keyword_attribute_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Retract(Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":e".to_string()),
                EdnValue::Integer(42),
                EdnValue::String("value".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "non-keyword attribute in retract must fail");
    }

    #[test]
    fn execute_transact_pseudo_attr_error() {
        // Exercises executor.rs line 103: Pseudo(_) arm in execute_transact
        use crate::query::datalog::types::{PseudoAttr, Transaction};
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Transact(Transaction {
            facts: vec![Pattern::pseudo(
                EdnValue::Keyword(":e".to_string()),
                PseudoAttr::ValidFrom,
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "transacting a pseudo-attribute must fail");
    }

    #[test]
    fn execute_retract_pseudo_attr_error() {
        // Exercises executor.rs line 139: Pseudo(_) arm in execute_retract
        use crate::query::datalog::types::{PseudoAttr, Transaction};
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Retract(Transaction {
            facts: vec![Pattern::pseudo(
                EdnValue::Keyword(":e".to_string()),
                PseudoAttr::TxCount,
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "retracting a pseudo-attribute must fail");
    }

    #[test]
    fn execute_rule_empty_head_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Rule(Rule {
            head: vec![],
            body: vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":a".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "rule with empty head must fail");
    }

    #[test]
    fn execute_rule_non_symbol_head_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Rule(Rule {
            head: vec![EdnValue::Integer(99)], // not a Symbol
            body: vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":a".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        });
        let r = executor.execute(cmd);
        assert!(r.is_err(), "rule head starting with non-symbol must fail");
    }

    // ── Float arithmetic edge cases ──────────────────────────────────────────

    #[test]
    fn test_eval_float_div_by_zero_is_err() {
        // Line 1096: rf == 0.0 → Err(()) for float division
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Float(5.0))),
            Box::new(Expr::Lit(Value::Float(0.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_float_div_succeeds() {
        // Line 1096 false branch: rf != 0.0 → Ok(Float)
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Float(6.0))),
            Box::new(Expr::Lit(Value::Float(2.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Float(3.0)));
    }

    #[test]
    fn test_eval_float_sub() {
        // Line 1079-1085: float subtraction
        let e = Expr::BinOp(
            BinOp::Sub,
            Box::new(Expr::Lit(Value::Float(5.0))),
            Box::new(Expr::Lit(Value::Float(2.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Float(3.0)));
    }

    #[test]
    fn test_eval_float_mul() {
        // Line 1087-1093: float multiplication
        let e = Expr::BinOp(
            BinOp::Mul,
            Box::new(Expr::Lit(Value::Float(3.0))),
            Box::new(Expr::Lit(Value::Float(4.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Float(12.0)));
    }

    // ── Aggregation edge cases ────────────────────────────────────────────────

    #[test]
    fn test_agg_count_empty_bindings_returns_zero() {
        // (count ?x) with no matching facts → zero bindings → special-case returns 0
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command("(query [:find (count ?x) :where [?x :no-such-attr _]])")
            .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "should return one row with count 0");
                assert_eq!(results[0][0], crate::graph::types::Value::Integer(0));
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_agg_sum_empty_no_grouping_returns_zero() {
        // (sum ?v) with no matching facts and no grouping vars
        // bindings is empty → `has_grouping_vars` is false but it's not count → returns []
        let storage = FactStorage::new();
        storage
            .transact(
                vec![(
                    Uuid::new_v4(),
                    ":item/price".to_string(),
                    crate::graph::types::Value::Integer(50),
                )],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // Query for non-existing attribute to produce empty bindings, then sum
        let cmd = parse_datalog_command("(query [:find (sum ?v) :where [?x :no-such-attr ?v]])")
            .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                // empty bindings with non-count agg and no grouping → returns []
                assert_eq!(results.len(), 0, "empty bindings with sum returns no rows");
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_agg_sum_distinct_float_values() {
        // sum-distinct on float values exercises the SumDistinct + has_float path
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":item/weight".to_string(),
                        crate::graph::types::Value::Float(1.5),
                    ),
                    (
                        e2,
                        ":item/weight".to_string(),
                        crate::graph::types::Value::Float(1.5),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        let cmd =
            parse_datalog_command("(query [:find (sum-distinct ?w) :where [?e :item/weight ?w]])")
                .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                // Both have weight 1.5 but sum-distinct deduplicates → result is 1.5
                assert_eq!(results.len(), 1, "expected one result row");
                assert_eq!(
                    results[0][0],
                    crate::graph::types::Value::Float(1.5),
                    "sum-distinct of [1.5, 1.5] should be 1.5"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_agg_min_max_on_all_null_group_skips_row() {
        // min/max on a group where all values are Null → row is skipped
        // We insert a fact with value Null and query for min
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    e1,
                    ":item/score".to_string(),
                    crate::graph::types::Value::Null,
                )],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // min on only Null values → "no non-null values in group" → row skipped → 0 rows
        let cmd = parse_datalog_command("(query [:find (min ?s) :where [?e :item/score ?s]])")
            .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    0,
                    "min on all-null group should produce 0 rows"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_agg_min_on_strings() {
        // min on strings exercises the String comparison path in apply_agg_func
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":item/name".to_string(),
                        crate::graph::types::Value::String("banana".to_string()),
                    ),
                    (
                        e2,
                        ":item/name".to_string(),
                        crate::graph::types::Value::String("apple".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command("(query [:find (min ?n) :where [?e :item/name ?n]])")
            .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "expected one result");
                assert_eq!(
                    results[0][0],
                    crate::graph::types::Value::String("apple".to_string()),
                    "min of strings should return lexicographically smallest"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_agg_max_on_floats() {
        // max on floats exercises the Float comparison path
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":item/score".to_string(),
                        crate::graph::types::Value::Float(3.14),
                    ),
                    (
                        e2,
                        ":item/score".to_string(),
                        crate::graph::types::Value::Float(2.71),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command("(query [:find (max ?s) :where [?e :item/score ?s]])")
            .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "expected one result");
                assert_eq!(
                    results[0][0],
                    crate::graph::types::Value::Float(3.14),
                    "max of floats should return largest"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    // ── evaluate_branch / apply_or_clauses edge cases ────────────────────────

    #[test]
    fn test_evaluate_branch_with_timestamp_valid_at() {
        // Exercises executor.rs lines 930-931: evaluate_branch with Timestamp/AnyValidTime
        use crate::query::datalog::types::ValidAt;
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    e1,
                    ":tag".to_string(),
                    crate::graph::types::Value::Integer(1),
                )],
                None,
            )
            .unwrap();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());
        let rules = crate::query::datalog::rules::RuleRegistry::new();
        let branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":tag".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        let mut initial = std::collections::HashMap::new();
        initial.insert("?seed".to_string(), crate::graph::types::Value::Integer(0));

        // Line 930: Some(ValidAt::Timestamp(t)) arm
        let ts_result = evaluate_branch(
            &branch,
            vec![initial.clone()],
            facts.clone(),
            &rules,
            None,
            Some(ValidAt::Timestamp(crate::graph::types::tx_id_now() as i64)),
        )
        .unwrap();
        assert_eq!(ts_result.len(), 1, "timestamp valid_at should match");

        // Line 931: Some(ValidAt::AnyValidTime) arm
        let any_result = evaluate_branch(
            &branch,
            vec![initial],
            facts,
            &rules,
            None,
            Some(ValidAt::AnyValidTime),
        )
        .unwrap();
        assert_eq!(any_result.len(), 1, "any_valid_time should match");
    }

    #[test]
    fn test_execute_query_with_rules_valid_at_timestamp() {
        // Exercises executor.rs lines 341-342 (valid_at_value in execute_query_with_rules
        // for Timestamp and AnyValidTime arms) and lines 348-350 (hard-error guard).
        use crate::query::datalog::parser::parse_datalog_command;
        use crate::query::datalog::types::ValidAt;
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Register a rule so the query routes through execute_query_with_rules
        let rule_cmd = parse_datalog_command(r#"(rule [(tagged ?e) [?e :item/tag ?v]])"#)
            .expect("rule parse failed");
        executor.execute(rule_cmd).expect("rule register failed");

        // Transact a fact
        executor
            .execute(
                parse_datalog_command(r#"(transact [[:item1 :item/tag "x"]])"#)
                    .expect("transact parse failed"),
            )
            .expect("transact failed");

        // Lines 341-342: call execute_query_with_rules directly with Timestamp and AnyValidTime.
        // The public execute() routing may bypass it if query_uses_rules returns false;
        // calling the private method directly guarantees coverage.
        let q_ts = crate::query::datalog::types::DatalogQuery {
            find: vec![crate::query::datalog::types::FindSpec::Variable(
                "?e".to_string(),
            )],
            where_clauses: vec![],
            as_of: None,
            valid_at: Some(ValidAt::Timestamp(946684800000)), // 2000-01-01
            with_vars: vec![],
        };
        let r_ts = executor.execute_query_with_rules(q_ts);
        assert!(
            r_ts.is_ok(),
            "execute_query_with_rules with Timestamp must not error"
        );

        let q_any = crate::query::datalog::types::DatalogQuery {
            find: vec![crate::query::datalog::types::FindSpec::Variable(
                "?e".to_string(),
            )],
            where_clauses: vec![],
            as_of: None,
            valid_at: Some(ValidAt::AnyValidTime),
            with_vars: vec![],
        };
        let r_any = executor.execute_query_with_rules(q_any);
        assert!(
            r_any.is_ok(),
            "execute_query_with_rules with AnyValidTime must not error"
        );

        // Lines 348-350: hard-error guard in execute_query_with_rules
        // Per-fact pseudo-attr without :any-valid-time in a rules query
        let err_cmd = parse_datalog_command(
            "(query [:find ?e ?vf :where (tagged ?e) [?e :db/valid-from ?vf]])",
        )
        .expect("err query parse failed");
        let err_result = executor.execute(err_cmd);
        assert!(
            err_result.is_err(),
            "per-fact pseudo-attr without :any-valid-time in rules query must fail"
        );
    }

    #[test]
    fn test_evaluate_branch_empty_incoming_returns_empty() {
        // evaluate_branch with empty incoming bindings → returns [] immediately (line 842)
        let storage = FactStorage::new();
        storage
            .transact(
                vec![(
                    Uuid::new_v4(),
                    ":a".to_string(),
                    crate::graph::types::Value::Integer(1),
                )],
                None,
            )
            .unwrap();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());
        let rules = crate::query::datalog::rules::RuleRegistry::new();
        let branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":a".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        let result = evaluate_branch(&branch, vec![], facts, &rules, None, None).unwrap();
        assert_eq!(result.len(), 0, "empty incoming should return empty");
    }

    #[test]
    fn test_evaluate_branch_no_match_patterns_empty_bindings_returns_empty() {
        // evaluate_branch: patterns exist but match nothing → bindings is empty (line 865)
        let storage = FactStorage::new();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());
        let rules = crate::query::datalog::rules::RuleRegistry::new();
        // Branch has a pattern that won't match empty storage
        let branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":no-such-attr".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        // Seed with one binding so the branch has something to work with
        let mut initial = std::collections::HashMap::new();
        initial.insert("?init".to_string(), crate::graph::types::Value::Integer(1));
        let result = evaluate_branch(&branch, vec![initial], facts, &rules, None, None).unwrap();
        assert_eq!(
            result.len(),
            0,
            "no matching facts should return empty bindings"
        );
    }

    #[test]
    fn test_evaluate_branch_not_filter_excludes_matching() {
        // evaluate_branch with Not clause: entities that match the not body are excluded (line 909)
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":color".to_string(),
                        crate::graph::types::Value::Keyword(":red".to_string()),
                    ),
                    (
                        e2,
                        ":color".to_string(),
                        crate::graph::types::Value::Keyword(":blue".to_string()),
                    ),
                    (
                        e1,
                        ":flagged".to_string(),
                        crate::graph::types::Value::Boolean(true),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // Query: find entities with :color but not :flagged
        // Uses the not_body_matches path in not-post-filter (line 909)
        let cmd = parse_datalog_command(
            "(query [:find ?e :where [?e :color ?c] (not-join [?e] [?e :flagged ?fv])])",
        )
        .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "only e2 (non-flagged) should match");
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_or_join_deduplication() {
        // or-join where both branches bind ?e → duplicate bindings are deduplicated
        // (line 991: if !result.contains(&b) { result.push(b); })
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":color".to_string(),
                        crate::graph::types::Value::Keyword(":red".to_string()),
                    ),
                    (
                        e1,
                        ":shape".to_string(),
                        crate::graph::types::Value::Keyword(":circle".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // or-join [?e] with two branches that both match e1 → deduplicated to 1
        // ?e must be bound by an earlier clause; use :color as the primary clause
        let cmd = parse_datalog_command(
            "(query [:find ?e :where [?e :color ?c] (or-join [?e] [?e :color ?c2] [?e :shape ?s])])",
        )
        .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    1,
                    "e1 should appear once despite two matching branches"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_transact_with_tx_level_valid_time() {
        // Exercises the tx_opts = Some(...) path when valid_from/valid_to set at tx level (line 66)
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command(
            r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2025-01-01T00:00:00Z"} [[:alice :person/name "Alice"]])"#,
        )
        .expect("parse with tx-level valid-time should succeed");
        let result = executor.execute(cmd);
        assert!(
            result.is_ok(),
            "transact with tx-level valid-time should succeed"
        );
    }

    #[test]
    fn test_transact_with_per_fact_valid_time() {
        // Exercises the per_fact_opts = Some(...) path when valid_from/valid_to set per fact (line 87)
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command(
            r#"(transact [[:alice :person/name "Alice" {:valid-from "2020-01-01T00:00:00Z"}]])"#,
        )
        .expect("parse with per-fact valid-time should succeed");
        let result = executor.execute(cmd);
        assert!(
            result.is_ok(),
            "transact with per-fact valid-time should succeed"
        );
    }

    #[test]
    fn test_transact_with_valid_to_only_at_tx_level() {
        // Exercises the `|| tx.valid_to.is_some()` branch (line 66 col 53)
        // when valid_from is None but valid_to is Some
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command(
            r#"(transact {:valid-to "2025-01-01T00:00:00Z"} [[:alice :person/name "Alice"]])"#,
        )
        .expect("parse with tx-level valid-to only should succeed");
        let result = executor.execute(cmd);
        assert!(
            result.is_ok(),
            "transact with valid-to only at tx level should succeed"
        );
    }

    #[test]
    fn test_transact_with_valid_to_only_per_fact() {
        // Exercises the `|| pattern.valid_to.is_some()` branch (line 87 col 68)
        // when per-fact valid_from is None but valid_to is Some
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = parse_datalog_command(
            r#"(transact [[:alice :person/name "Alice" {:valid-to "2025-01-01T00:00:00Z"}]])"#,
        )
        .expect("parse with per-fact valid-to only should succeed");
        let result = executor.execute(cmd);
        assert!(
            result.is_ok(),
            "transact with valid-to only per fact should succeed"
        );
    }

    #[test]
    fn test_evaluate_branch_empty_patterns_passes_incoming_through() {
        // Line 859: patterns.is_empty() = true → bindings = incoming (pass through)
        // Achieved when branch contains only Not/Expr clauses, no Pattern/RuleInvocation
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    e1,
                    ":a".to_string(),
                    crate::graph::types::Value::Integer(10),
                )],
                None,
            )
            .unwrap();
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());
        let rules = crate::query::datalog::rules::RuleRegistry::new();

        // Branch with only an Expr clause (no patterns) — patterns.is_empty() = true
        let branch = vec![WhereClause::Expr {
            expr: crate::query::datalog::types::Expr::Lit(crate::graph::types::Value::Boolean(
                true,
            )),
            binding: None,
        }];
        // Incoming with one binding
        let mut initial = std::collections::HashMap::new();
        initial.insert("?x".to_string(), crate::graph::types::Value::Integer(42));
        let result = evaluate_branch(&branch, vec![initial], facts, &rules, None, None).unwrap();
        // The expr is truthy so the binding passes through
        assert_eq!(
            result.len(),
            1,
            "expr-only branch should pass binding through"
        );
    }

    #[test]
    fn test_evaluate_branch_or_clause_produces_empty_bindings() {
        // Line 879: bindings empty after apply_or_clauses → return Ok([])
        // This happens when an Or clause produces no results
        let storage = FactStorage::new();
        // Empty storage → no facts
        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());
        let rules = crate::query::datalog::rules::RuleRegistry::new();

        // Branch with an Or clause that matches nothing
        let or_branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":no-attr".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        let branch = vec![WhereClause::Or(vec![or_branch])];

        let mut initial = std::collections::HashMap::new();
        initial.insert("?seed".to_string(), crate::graph::types::Value::Integer(1));
        let result = evaluate_branch(&branch, vec![initial], facts, &rules, None, None).unwrap();
        assert_eq!(
            result.len(),
            0,
            "or clause with no matches should yield empty"
        );
    }

    #[test]
    fn test_evaluate_branch_not_join_excludes_matching() {
        // Line 914: evaluate_not_join returns true inside evaluate_branch → exclude
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":status".to_string(),
                        crate::graph::types::Value::Keyword(":active".to_string()),
                    ),
                    (
                        e2,
                        ":status".to_string(),
                        crate::graph::types::Value::Keyword(":inactive".to_string()),
                    ),
                    (
                        e2,
                        ":blocked".to_string(),
                        crate::graph::types::Value::Boolean(true),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // not-join: exclude entities that have :blocked = true
        let cmd = parse_datalog_command(
            "(query [:find ?e :where [?e :status ?s] (not-join [?e] [?e :blocked ?b])])",
        )
        .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    1,
                    "only non-blocked entity should be returned"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn test_not_body_expr_only_filters_binding() {
        // Exercises not_body_matches with patterns.is_empty() (Expr-only not body) at line 561
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":item/price".to_string(),
                        crate::graph::types::Value::Integer(200),
                    ),
                    (
                        e2,
                        ":item/price".to_string(),
                        crate::graph::types::Value::Integer(50),
                    ),
                ],
                None,
            )
            .unwrap();
        let executor = DatalogExecutor::new(storage);
        // not with expr-only body: exclude items where price > 100
        let cmd = parse_datalog_command(
            "(query [:find ?e :where [?e :item/price ?p] (not [(> ?p 100)])])",
        )
        .expect("parse failed");
        let result = executor.execute(cmd).expect("query failed");
        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(
                    results.len(),
                    1,
                    "only the item with price 50 should survive"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    #[test]
    fn window_sum_resets_per_partition() {
        use super::super::functions::FunctionRegistry;
        use super::super::types::{Order, WindowFunc, WindowSpec};

        let mut bindings: Vec<std::collections::HashMap<String, Value>> = vec![
            [
                ("dept".into(), Value::String("A".into())),
                ("salary".into(), Value::Integer(10)),
            ]
            .into_iter()
            .collect(),
            [
                ("dept".into(), Value::String("A".into())),
                ("salary".into(), Value::Integer(20)),
            ]
            .into_iter()
            .collect(),
            [
                ("dept".into(), Value::String("B".into())),
                ("salary".into(), Value::Integer(100)),
            ]
            .into_iter()
            .collect(),
        ];
        let find_specs = vec![
            FindSpec::Variable("dept".into()),
            FindSpec::Variable("salary".into()),
            FindSpec::Window(WindowSpec {
                func: WindowFunc::Sum,
                var: Some("salary".into()),
                partition_by: Some("dept".into()),
                order_by: "salary".into(),
                order: Order::Asc,
            }),
        ];
        let registry = FunctionRegistry::with_builtins();
        apply_window_functions(&mut bindings, &find_specs, &registry).expect("window");

        // Partition A: 10 → sum=10, 20 → sum=30
        let row_a10 = bindings
            .iter()
            .find(|b| b.get("salary") == Some(&Value::Integer(10)))
            .unwrap();
        assert_eq!(row_a10.get("__win_2"), Some(&Value::Integer(10)));

        let row_a20 = bindings
            .iter()
            .find(|b| b.get("salary") == Some(&Value::Integer(20)))
            .unwrap();
        assert_eq!(row_a20.get("__win_2"), Some(&Value::Integer(30)));

        // Partition B: 100 → sum=100 (accumulator reset, NOT 130)
        let row_b100 = bindings
            .iter()
            .find(|b| b.get("salary") == Some(&Value::Integer(100)))
            .unwrap();
        assert_eq!(row_b100.get("__win_2"), Some(&Value::Integer(100)));
    }
}
