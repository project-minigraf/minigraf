use super::functions::FunctionRegistry;
use super::matcher::{edn_to_entity_id, edn_to_value};
use super::rules::RuleRegistry;
use super::types::{
    AttributeSpec, BinOp, DatalogCommand, DatalogQuery, EdnValue, Expr, FindSpec, Pattern, Rule,
    Transaction, UnaryOp, ValidAt, WhereClause, WindowFunc,
};
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::FactStorage;
use crate::graph::storage::Snapshot;
use crate::graph::types::{TransactOptions, TxId, Value};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// `INT-026`: per-fact pseudo-attributes require `:any-valid-time`.
fn check_per_fact_pseudo_attrs(query: &DatalogQuery) -> Result<()> {
    if query_uses_per_fact_pseudo_attr(query)
        && !matches!(query.valid_at, Some(ValidAt::AnyValidTime))
    {
        return Err(err_coded!(ErrorCode::Int026));
    }
    Ok(())
}

/// `QRY-007` for the first of `names` that `registry` lacks: the engine
/// resolves every predicate a query names, at any depth, when it opens
/// (#551 spec §3.2).
fn check_predicates<'a>(
    names: impl IntoIterator<Item = &'a String>,
    registry: &FunctionRegistry,
) -> Result<()> {
    for name in names {
        if registry.get_predicate(name).is_none() {
            bail_coded!(ErrorCode::Qry007, name);
        }
    }
    Ok(())
}

/// The clause checks for a query the streaming engine runs: the executor's
/// pseudo-attribute check, then every predicate it names (#473, #551).
fn check_clauses(query: &DatalogQuery, registry: &FunctionRegistry) -> Result<()> {
    check_per_fact_pseudo_attrs(query)?;
    let mut names = Vec::new();
    crate::query::engine::udfs(&query.where_clauses, &mut names);
    check_predicates(&names, registry)
}

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

/// What [`DatalogExecutor::open`] gives a cursor: a whole result, or a
/// query on the streaming engine to pull rows from.
pub(crate) enum Answer {
    Result(QueryResult),
    Stream(Box<crate::query::engine::Stream>),
}

/// The result of executing a Datalog command via [`crate::db::Minigraf::execute`].
///
/// Pattern-match on this to distinguish query results from write confirmations:
///
/// ```
/// # use minigraf::{Minigraf, QueryResult};
/// # let db = Minigraf::in_memory().unwrap();
/// # db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// match db.execute("(query [:find ?name :where [?e :person/name ?name]])").unwrap() {
///     QueryResult::QueryResults { vars, results } => {
///         for row in &results {
///             println!("{}: {:?}", vars[0], row[0]);
///         }
///     }
///     QueryResult::Transacted(tx_id) => println!("tx {}", tx_id),
///     QueryResult::Retracted(tx_id) => println!("retracted tx {}", tx_id),
///     QueryResult::Ok => {}
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    /// Transaction completed successfully. The inner value is the transaction ID
    /// (Unix milliseconds). Use [`crate::db::Minigraf::current_tx_count`] to retrieve
    /// the monotonic counter (`:as-of N` value) after a write.
    Transacted(TxId),
    /// Retraction completed successfully. The inner value is the transaction ID
    /// (Unix milliseconds). Use [`crate::db::Minigraf::current_tx_count`] to retrieve
    /// the monotonic counter (`:as-of N` value) after a write.
    Retracted(TxId),
    /// Query results: list of variable bindings
    QueryResults {
        /// The variable names in the order they appear in the `:find` clause.
        vars: Vec<String>,
        /// Each inner `Vec<Value>` is one result row, aligned with `vars`.
        results: Vec<Vec<Value>>,
    },
    /// Acknowledgement for commands that produce no data (e.g. rule definitions
    /// inside a [`crate::db::WriteTransaction`]).
    Ok,
}

/// Executor for Datalog commands
pub struct DatalogExecutor {
    storage: FactStorage,
    /// The state every read of this executor sees; when `None`, each read
    /// takes its own snapshot of `storage`.
    snapshot: Option<Snapshot>,
    rules: Arc<RwLock<RuleRegistry>>,
    // RwLock pre-wired for 7.7b register_aggregate API.
    functions: Arc<RwLock<FunctionRegistry>>,
    /// The most facts a rule query may derive, base facts not counted (#553).
    max_derived_facts: usize,
    /// The most bytes an engine query may hold (#549).
    query_memory_limit: usize,
}

impl DatalogExecutor {
    #[allow(dead_code)]
    pub fn new(storage: FactStorage) -> Self {
        DatalogExecutor {
            storage,
            snapshot: None,
            rules: Arc::new(RwLock::new(RuleRegistry::new())),
            functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
            max_derived_facts: crate::query::datalog::evaluator::DEFAULT_MAX_DERIVED_FACTS,
            query_memory_limit: usize::MAX,
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
            snapshot: None,
            rules,
            functions,
            max_derived_facts: crate::query::datalog::evaluator::DEFAULT_MAX_DERIVED_FACTS,
            query_memory_limit: usize::MAX,
        }
    }

    /// Convenience constructor for tests. Shares `rules` with other executors but creates
    /// a fresh `FunctionRegistry::with_builtins()`. Production code uses
    /// [`new_with_rules_and_functions`] to share the registry from `Minigraf::Inner`.
    #[allow(dead_code)]
    pub fn new_with_rules(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>) -> Self {
        Self::new_with_rules_and_functions(
            storage,
            rules,
            Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
        )
    }

    /// Create a `DatalogExecutor` with custom complexity limits.
    ///
    /// Used by `Minigraf` when `OpenOptions` specifies non-default limits.
    #[allow(dead_code)]
    pub fn new_with_limits(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        functions: Arc<RwLock<FunctionRegistry>>,
        max_derived_facts: usize,
    ) -> Self {
        DatalogExecutor {
            storage,
            snapshot: None,
            rules,
            functions,
            max_derived_facts,
            query_memory_limit: usize::MAX,
        }
    }

    /// Set the most facts a rule query may derive, base facts not counted
    /// (`OpenOptions::max_derived_facts`, #553), and the most bytes a query
    /// on the engine may hold (`OpenOptions::query_memory_limit`, #549).
    pub fn set_limits(&mut self, max_derived_facts: usize, query_memory_limit: usize) {
        self.max_derived_facts = max_derived_facts;
        self.query_memory_limit = query_memory_limit;
    }

    /// Read every fact from `snapshot`, so the queries of this executor see
    /// one state of the store and pin its committed generation (#543).
    pub(crate) fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.snapshot = Some(snapshot);
    }

    /// The snapshot reads go to: the executor's, or a new one of `storage`.
    fn read_snapshot(&self) -> Result<Snapshot> {
        match &self.snapshot {
            Some(s) => Ok(s.clone()),
            None => self.storage.snapshot(),
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
                edn_to_entity_id(&pattern.entity).map_err(|e| err_coded!(ErrorCode::Qry001, e))?;

            let attribute = match &pattern.attribute {
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => bail_coded!(ErrorCode::Qry002),
                AttributeSpec::Pseudo(_) => {
                    bail_coded!(ErrorCode::Qry003);
                }
            };

            let value =
                edn_to_value(&pattern.value).map_err(|e| err_coded!(ErrorCode::Qry004, e))?;

            let per_fact_opts = if pattern.valid_from.is_some() || pattern.valid_to.is_some() {
                Some(TransactOptions::new(pattern.valid_from, pattern.valid_to))
            } else {
                None
            };

            fact_tuples.push((entity_id, attribute, value, per_fact_opts));
        }

        let (tx_id, _tx_count) = self
            .storage
            .transact_batch(fact_tuples, tx_opts)
            .map_err(|e| err_coded!(ErrorCode::Qry005, e))?;

        Ok(QueryResult::Transacted(tx_id))
    }

    /// Execute a retract command: retract facts from storage
    fn execute_retract(&self, tx: Transaction) -> Result<QueryResult> {
        let mut fact_tuples = Vec::new();

        for pattern in tx.facts {
            let entity_id =
                edn_to_entity_id(&pattern.entity).map_err(|e| err_coded!(ErrorCode::Qry001, e))?;

            let attribute = match &pattern.attribute {
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => bail_coded!(ErrorCode::Qry002),
                AttributeSpec::Pseudo(_) => {
                    bail_coded!(ErrorCode::Qry003);
                }
            };

            let value =
                edn_to_value(&pattern.value).map_err(|e| err_coded!(ErrorCode::Qry004, e))?;

            fact_tuples.push((entity_id, attribute, value));
        }

        let (tx_id, _tx_count) = self
            .storage
            .retract(fact_tuples)
            .map_err(|e| err_coded!(ErrorCode::Qry006, e))?;

        Ok(QueryResult::Retracted(tx_id))
    }

    /// Execute a query: find matching facts and return specified variables
    fn execute_query(&self, query: DatalogQuery) -> Result<QueryResult> {
        match self.open_query(query)? {
            Answer::Result(result) => Ok(result),
            Answer::Stream(stream) => stream.collect(),
        }
    }

    /// Run `command` for a cursor: a query the streaming engine runs is
    /// opened, not run; anything else runs as [`Self::execute`] runs it.
    pub(crate) fn open(&self, command: DatalogCommand) -> Result<Answer> {
        match command {
            DatalogCommand::Query(query) => self.open_query(query),
            command => self.execute(command).map(Answer::Result),
        }
    }

    fn open_query(&self, query: DatalogQuery) -> Result<Answer> {
        // Resolve :find functions up front so an unknown name fails whether or
        // not any row matches (#414).
        {
            let registry = self
                .functions
                .read()
                .map_err(|_| err_coded!(ErrorCode::Qry008))?;
            check_find_functions(&query.find, &registry)?;
        }

        // A rule query (#548).
        if query.uses_rules() {
            return self.open_rule_query(&query);
        }

        // Warn about queries with no binding mechanism
        if !query.has_binding_mechanism() {
            return Err(err_coded!(ErrorCode::Int027));
        }

        // The streaming engine runs every query (#473, #551). A planner
        // error comes after the pseudo-attribute check (#473 spec §5.5).
        let plan = match crate::query::engine::plan(&query) {
            Ok(plan) => plan,
            Err(e) => {
                check_per_fact_pseudo_attrs(&query)?;
                return Err(e);
            }
        };
        {
            let registry = self
                .functions
                .read()
                .map_err(|_| err_coded!(ErrorCode::Qry008))?;
            check_clauses(&query, &registry)?;
        }
        let snapshot = self.read_snapshot()?;
        let stream = crate::query::engine::open(
            snapshot.clone(),
            plan,
            self.functions.clone(),
            self.query_memory_limit,
        )?;
        #[cfg(test)]
        crate::query::engine::RUNS.with(|r| r.set(r.get() + 1));
        Ok(Answer::Stream(Box::new(stream)))
    }

    /// A rule query on the streaming engine (#548). Its errors come in the
    /// executor's order: pseudo-attributes, then the rules, then unknown
    /// predicates.
    fn open_rule_query(&self, query: &DatalogQuery) -> Result<Answer> {
        let plan = {
            let registry = self
                .rules
                .read()
                .map_err(|_| err_coded!(ErrorCode::Qry009))?;
            match crate::query::datalog::magic_sets::rewrite(query, &registry) {
                Some((rewritten, seeds)) => {
                    let attrs: Vec<&str> = seeds.iter().map(|(_, a, _)| a.as_str()).collect();
                    crate::query::engine::plan_rules(query, &rewritten, &attrs)
                        .map(|plan| (plan, seeds))
                }
                None => {
                    crate::query::engine::plan_rules(query, &registry, &[]).map(|p| (p, vec![]))
                }
            }
        };
        let (plan, seeds) = match plan {
            Ok(planned) => planned,
            Err(e) => {
                check_per_fact_pseudo_attrs(query)?;
                return Err(e);
            }
        };
        check_per_fact_pseudo_attrs(query)?;
        let limits = crate::query::engine::Limits {
            max_iterations: crate::query::datalog::evaluator::DEFAULT_MAX_ITERATIONS,
            max_derived_facts: self.derived_limit(query),
            query_memory: self.query_memory_limit,
        };
        let snapshot = self.read_snapshot()?;
        // The keywords the query's rule calls name as entities (#721).
        let keywords: Vec<String> = query
            .where_clauses
            .iter()
            .filter_map(|c| match c {
                WhereClause::RuleInvocation { args, .. } => match args.first() {
                    Some(EdnValue::Keyword(k)) => Some(k.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // The query's predicates, then its rules' (#551 plan, decision 9).
        let mut names = Vec::new();
        crate::query::engine::udfs(&query.where_clauses, &mut names);
        names.extend(plan.predicates.iter().cloned());
        let open = |plan| -> Result<crate::query::engine::Stream> {
            let stream = crate::query::engine::open_rules(
                snapshot.clone(),
                plan,
                &seeds,
                &keywords,
                self.functions.clone(),
                limits,
            )?;
            let registry = self
                .functions
                .read()
                .map_err(|_| err_coded!(ErrorCode::Qry008))?;
            check_predicates(&names, &registry)?;
            Ok(stream)
        };
        let stream = open(plan)?;
        #[cfg(test)]
        crate::query::engine::RUNS.with(|r| r.set(r.get() + 1));
        Ok(Answer::Stream(Box::new(stream)))
    }

    /// The most facts `query`'s rules may derive: a per-query limit replaces
    /// the executor's; `:max-results` is a deprecated alias of
    /// `:max-derived-facts`, and the lower of the two applies (#553).
    fn derived_limit(&self, query: &DatalogQuery) -> usize {
        match (query.max_derived_facts, query.max_results) {
            (None, None) => self.max_derived_facts,
            (a, b) => a.unwrap_or(usize::MAX).min(b.unwrap_or(usize::MAX)),
        }
    }

    /// Execute a rule command: register the rule for later use
    fn execute_rule(&self, rule: Rule) -> Result<QueryResult> {
        // Extract predicate name from rule head
        // Head format: (predicate ?arg1 ?arg2 ...)
        let predicate = self.extract_predicate(&rule)?;

        // Register the rule
        self.rules
            .write()
            .map_err(|_| err_coded!(ErrorCode::Qry009))?
            .register_rule(predicate, rule)?;

        Ok(QueryResult::Ok)
    }

    /// Extract the predicate name from a rule head
    fn extract_predicate(&self, rule: &Rule) -> Result<String> {
        if rule.head.is_empty() {
            return Err(err_coded!(ErrorCode::Int032, "Rule head cannot be empty"));
        }

        // Safety: is_empty() check above guarantees index 0 exists.
        #[allow(clippy::indexing_slicing)]
        match &rule.head[0] {
            EdnValue::Symbol(s) => Ok(s.clone()),
            _ => Err(err_coded!(
                ErrorCode::Int032,
                "Rule head must start with a symbol (predicate name)"
            )),
        }
    }

    /// Get the underlying storage (for testing)
    #[allow(dead_code)]
    pub fn storage(&self) -> &FactStorage {
        &self.storage
    }

    /// Get the rule registry (for testing)
    #[cfg(test)]
    pub fn rules(&self) -> Arc<RwLock<RuleRegistry>> {
        self.rules.clone()
    }
}

/// Which clause of a query's top-level `:where` gave each variable its value
/// (#545, #473 spec §5.4).
///
/// A variable that several patterns bind takes its value from the first of
/// them in the query text, whatever order the planner runs them in: a
/// keyword and the ref of the entity it names are equal in a join, but are
/// returned as they were bound. An expression binding is a binder like a
/// pattern: when the variable is already bound it keeps the rows where its
/// result is equal (#596), and gives the value when it is first in the text.
/// A pseudo-attribute pattern never gives a variable its value: a real
/// pattern that binds it replaces its value, wherever it is (#606).
pub(crate) struct Binders {
    /// Each top-level pattern (rule calls as patterns) and its position in
    /// `:where`; taken as the planner hands them out.
    text: Vec<(Pattern, usize)>,
    /// Each top-level binding expression, its variable and its position.
    exprs: Vec<(Expr, String, usize)>,
    /// The position of the clause each variable's value came from.
    by_var: HashMap<String, usize>,
}

impl Binders {
    pub(crate) fn new(where_clauses: &[WhereClause], rule_patterns: Vec<(Pattern, usize)>) -> Self {
        let mut text: Vec<(Pattern, usize)> = where_clauses
            .iter()
            .enumerate()
            .filter_map(|(i, c)| match c {
                WhereClause::Pattern(p) => Some((p.clone(), i)),
                _ => None,
            })
            .collect();
        text.extend(rule_patterns);
        let exprs = where_clauses
            .iter()
            .enumerate()
            .filter_map(|(i, c)| match c {
                WhereClause::Expr {
                    expr,
                    binding: Some(var),
                } => Some((expr.clone(), var.clone(), i)),
                _ => None,
            })
            .collect();
        Binders {
            text,
            exprs,
            by_var: HashMap::new(),
        }
    }

    /// Record the pattern `p`, run next, and return the variables whose
    /// value it replaces.
    pub(crate) fn pattern(&mut self, p: &Pattern) -> Vec<String> {
        let real = matches!(p.attribute, AttributeSpec::Real(_));
        let at = match self.text.iter().position(|(q, _)| q == p) {
            // A pseudo-attribute pattern never gives a variable its value: it
            // ranks after every real pattern, whatever runs first (#606).
            Some(i) if real => self.text.swap_remove(i).1,
            Some(i) => {
                self.text.swap_remove(i);
                usize::MAX
            }
            None => usize::MAX,
        };
        let attribute = match &p.attribute {
            AttributeSpec::Real(a) => Some(a),
            AttributeSpec::Pseudo(_) => None,
        };
        let mut rebind = Vec::new();
        for c in [Some(&p.entity), attribute, Some(&p.value)]
            .into_iter()
            .flatten()
        {
            let EdnValue::Symbol(v) = c else { continue };
            if !v.starts_with('?') || v.starts_with("?_") {
                continue;
            }
            match self.by_var.get(v) {
                None => {
                    self.by_var.insert(v.clone(), at);
                }
                Some(b) if real && at < *b => {
                    self.by_var.insert(v.clone(), at);
                    if !rebind.contains(v) {
                        rebind.push(v.clone());
                    }
                }
                Some(_) => {}
            }
        }
        rebind
    }

    /// Record the expression `expr` binding `var`, run next, and return
    /// whether its value replaces the one a clause already gave `var`: when
    /// that clause is later in the text.
    pub(crate) fn expression(&mut self, expr: &Expr, var: &str) -> bool {
        let at = match self
            .exprs
            .iter()
            .position(|(e, v, _)| e == expr && v == var)
        {
            Some(i) => self.exprs.swap_remove(i).2,
            None => usize::MAX,
        };
        match self.by_var.get(var) {
            None => {
                self.by_var.insert(var.to_string(), at);
                false
            }
            Some(b) if at < *b => {
                self.by_var.insert(var.to_string(), at);
                true
            }
            Some(_) => false,
        }
    }
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
        // Ordering comparisons. Strings order lexicographically, matching
        // aggregate::value_order — which min/max and :order-by use — so the same two
        // strings order the same way whether compared by a predicate or by an aggregate.
        // Numbers order numerically with int/float promotion via to_float_pair.
        // Mixed types (string vs number) remain a type error.
        BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
            let ordering = match (&l, &r) {
                (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
                // None when either operand is NaN: every ordering against NaN is false.
                _ => {
                    let (lf, rf) = to_float_pair(&l, &r)?;
                    lf.partial_cmp(&rf)
                }
            };
            Ok(Value::Boolean(match ordering {
                None => false,
                Some(ord) => match op {
                    BinOp::Lt => ord.is_lt(),
                    BinOp::Gt => ord.is_gt(),
                    BinOp::Lte => ord.is_le(),
                    BinOp::Gte => ord.is_ge(),
                    #[allow(clippy::unreachable)]
                    _ => unreachable!(),
                },
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
                #[allow(clippy::unreachable)]
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
                    #[allow(clippy::unreachable)]
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
/// Rejects aggregate and window functions in `:find` that are neither
/// built-in nor registered. Runs before execution, so the result does not
/// depend on the data (#414).
fn check_find_functions(find: &[FindSpec], registry: &FunctionRegistry) -> Result<()> {
    for spec in find {
        match spec {
            FindSpec::Aggregate { func, .. } => {
                if registry.get(func).is_none() {
                    bail_coded!(ErrorCode::Qry010, func);
                }
            }
            FindSpec::Window(ws) => {
                if !matches!(ws.func, WindowFunc::Rank | WindowFunc::RowNumber) {
                    let name = ws.func_name();
                    if registry.get(&name).is_none() {
                        bail_coded!(ErrorCode::Qry011, name);
                    }
                }
            }
            FindSpec::Variable(_) => {}
        }
    }
    Ok(())
}

/// Why [`eval_expr`] gives no value.
#[derive(Debug)]
pub(crate) enum EvalError {
    /// Unbound variable, type mismatch, division by zero, unknown UDF
    /// predicate: the expression has no value, and its row is dropped.
    NoValue,
    /// A user predicate failed (`QRY-014`, #180): the query fails.
    Udf(anyhow::Error),
}

#[cfg(test)]
impl PartialEq for EvalError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// The value of `expr` under `binding`; see [`EvalError`] for its failures.
pub(crate) fn eval_expr(
    expr: &Expr,
    binding: &std::collections::HashMap<String, Value>,
    registry: Option<&FunctionRegistry>,
) -> Result<Value, EvalError> {
    match expr {
        Expr::Var(v) => binding.get(v).cloned().ok_or(EvalError::NoValue),
        Expr::Lit(val) => Ok(val.clone()),
        Expr::UnaryOp(op, arg) => {
            let v = eval_expr(arg, binding, registry)?;
            match op {
                UnaryOp::StringQ => Ok(Value::Boolean(matches!(v, Value::String(_)))),
                UnaryOp::IntegerQ => Ok(Value::Boolean(matches!(v, Value::Integer(_)))),
                UnaryOp::FloatQ => Ok(Value::Boolean(matches!(v, Value::Float(_)))),
                UnaryOp::BooleanQ => Ok(Value::Boolean(matches!(v, Value::Boolean(_)))),
                UnaryOp::NilQ => Ok(Value::Boolean(matches!(v, Value::Null))),
                UnaryOp::Udf(name) => {
                    let desc = registry
                        .and_then(|r| r.get_predicate(name))
                        .ok_or(EvalError::NoValue)?;
                    match (desc.f)(&v) {
                        Ok(b) => Ok(Value::Boolean(b)),
                        Err(e) => Err(EvalError::Udf(err_coded!(
                            ErrorCode::Qry014,
                            name,
                            e.message()
                        ))),
                    }
                }
            }
        }
        Expr::BinOp(op, lhs, rhs) => {
            let l = eval_expr(lhs, binding, registry)?;
            let r = eval_expr(rhs, binding, registry)?;
            eval_binop(op, l, r).map_err(|()| EvalError::NoValue)
        }
        Expr::Slot(_) => {
            // Unsubstituted bind slot — treat as eval error (unbound variable equivalent).
            Err(EvalError::NoValue)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::functions::FunctionRegistry;
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::rules::RuleRegistry;
    use crate::query::datalog::types::WhereClause;
    use std::sync::{Arc, RwLock};
    use uuid::Uuid;

    /// The entities of a one-column query answer, sorted.
    fn entities(result: Result<QueryResult>) -> Vec<Uuid> {
        let QueryResult::QueryResults { results, .. } = result.expect("query") else {
            panic!("expected query results");
        };
        let mut out: Vec<Uuid> = results
            .iter()
            .map(|r| match r.first() {
                Some(Value::Ref(u)) => *u,
                _ => panic!("expected an entity"),
            })
            .collect();
        out.sort();
        out
    }

    fn keyword(k: &str) -> Uuid {
        crate::query::datalog::matcher::edn_to_entity_id(&EdnValue::Keyword(k.to_string()))
            .expect("keyword entity")
    }

    /// Answer change 1 (#551 spec §4): a `not` in an `or` branch sees the
    /// variable an expression of the branch binds; the executor applied the
    /// branch's `not` before its expression.
    #[test]
    fn branch_not_sees_its_expression_binding() {
        let executor = DatalogExecutor::new(FactStorage::new());
        executor
            .execute(parse_datalog_command("(transact [[:x :n 1] [:y :n 2] [:z :n 5]])").unwrap())
            .unwrap();
        let q = "(query [:find ?e :where [?e :n ?v] (or-join [?e ?v] (and [(+ ?v 1) ?w] (not [?f :n ?w])) [?e :n 5])])";
        let before = crate::query::engine::RUNS.with(std::cell::Cell::get);
        let engine = entities(executor.execute(parse_datalog_command(q).unwrap()));
        assert_eq!(
            crate::query::engine::RUNS.with(std::cell::Cell::get),
            before + 1,
            "the engine runs it"
        );
        assert!(
            engine == {
                let mut v = vec![keyword(":y"), keyword(":z")];
                v.sort();
                v
            },
            "the engine: :y (no :n 3) and :z"
        );
        // The executor returned :z only.
    }

    /// Answer change 2 (#551 spec §4): an unknown predicate in an `or`
    /// branch is `QRY-007` on the engine whether or not a row reaches it,
    /// in a plain query and in a rule body; the executor answered when none
    /// did. In a `not` body too (plan deviation 4): the executor's predicate
    /// failed on every body row, so the `not` kept every outer row.
    #[test]
    fn unknown_branch_predicate_fails_eagerly() {
        let executor = DatalogExecutor::new(FactStorage::new());
        for cmd in [
            "(transact [[:x :n 1] [:z :n 5]])",
            "(rule [(odd ?e) [?e :n ?v] (or (and [?e :n 99] [(no-such? ?v)]) [?e :n 5])])",
        ] {
            executor
                .execute(parse_datalog_command(cmd).unwrap())
                .unwrap();
        }
        let plain =
            "(query [:find ?e :where [?e :n ?v] (or (and [?e :n 99] [(no-such? ?v)]) [?e :n 5])])";
        let rule = "(query [:find ?e :where (odd ?e)])";
        let negated = "(query [:find ?e :where [?e :n ?v] (not [?e :n ?v] [(no-such? ?v)])])";
        for q in [plain, rule, negated] {
            let err = executor
                .execute(parse_datalog_command(q).unwrap())
                .expect_err("the engine resolves every predicate");
            assert_eq!(crate::error::MinigrafError::from(err).code(), "QRY-007");
        }
        // The executor returned :z for `plain` and `rule`, and both outer
        // rows for `negated`.
    }

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
            format!(r#"(retract [[#uuid "{}" :person/age 30]])"#, alice_id).as_str(),
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
            a
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
            a
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
            a
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
                max_derived_facts: None,
                max_results: None,
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
                max_derived_facts: None,
                max_results: None,
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
            a
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
    fn test_rule_query_not_in_query_body() {
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
    fn test_rule_query_not_join_in_query_body() {
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

    #[test]
    fn test_per_query_max_derived_facts_overrides_executor_default() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let mut executor = DatalogExecutor::new_with_rules_and_functions(
            storage,
            rules.clone(),
            functions.clone(),
        );
        executor.set_limits(1_000_000, usize::MAX);

        executor
            .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :edge ?y]])"#).unwrap())
            .unwrap();
        executor
            .execute(
                parse_datalog_command(
                    r#"(rule [(reachable ?x ?z) [?x :edge ?y] (reachable ?y ?z)])"#,
                )
                .unwrap(),
            )
            .unwrap();
        executor
            .execute(parse_datalog_command(r#"(transact [[:a :edge :b] [:b :edge :c]])"#).unwrap())
            .unwrap();

        // Per-query limit of 1 — too tight, must fail
        let result = executor.execute(
            parse_datalog_command(
                "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1])",
            )
            .unwrap(),
        );
        assert!(result.is_err(), "per-query limit of 1 should fail");

        // Per-query limit of 1M — should succeed
        let result = executor.execute(
            parse_datalog_command(
                "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1000000])",
            )
            .unwrap(),
        );
        assert!(result.is_ok(), "per-query limit of 1M should succeed");
    }

    #[test]
    fn test_per_query_limit_does_not_bleed_into_next_query() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let executor = DatalogExecutor::new_with_rules_and_functions(
            storage,
            rules.clone(),
            functions.clone(),
        );

        executor
            .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :edge ?y]])"#).unwrap())
            .unwrap();
        executor
            .execute(parse_datalog_command(r#"(transact [[:a :edge :b]])"#).unwrap())
            .unwrap();

        // First query: tight limit, expect failure (ignore result)
        let _ = executor.execute(
            parse_datalog_command(
                "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1])",
            )
            .unwrap(),
        );

        // Second query: no per-query limit — must use executor default (1M) and succeed
        let result = executor.execute(
            parse_datalog_command("(query [:find ?x ?y :where (reachable ?x ?y)])").unwrap(),
        );
        assert!(
            result.is_ok(),
            "next query should not inherit the tight per-query limit"
        );
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
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Integer(42)));
    }

    #[test]
    fn test_eval_var_bound() {
        let e = Expr::Var("?x".to_string());
        let binding = b(&[("?x", Value::Integer(10))]);
        assert_eq!(eval_expr(&e, &binding, None), Ok(Value::Integer(10)));
    }

    #[test]
    fn test_eval_var_unbound_is_err() {
        let e = Expr::Var("?x".to_string());
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Err(EvalError::NoValue)
        );
    }

    #[test]
    fn test_eval_lt_true() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let binding = b(&[("?v", Value::Integer(50))]);
        assert_eq!(eval_expr(&e, &binding, None), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_lt_false() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        let binding = b(&[("?v", Value::Integer(150))]);
        assert_eq!(eval_expr(&e, &binding, None), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_add_integers() {
        let e = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Var("?a".to_string())),
            Box::new(Expr::Var("?b".to_string())),
        );
        let binding = b(&[("?a", Value::Integer(3)), ("?b", Value::Integer(4))]);
        assert_eq!(eval_expr(&e, &binding, None), Ok(Value::Integer(7)));
    }

    #[test]
    fn test_eval_add_int_float_promotes() {
        let e = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Lit(Value::Integer(1))),
            Box::new(Expr::Lit(Value::Float(1.5))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Float(2.5)));
    }

    #[test]
    fn test_eval_div_integer_truncates() {
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Integer(5))),
            Box::new(Expr::Lit(Value::Integer(2))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Integer(2)));
    }

    #[test]
    fn test_eval_div_by_zero_is_err() {
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Integer(5))),
            Box::new(Expr::Lit(Value::Integer(0))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Err(EvalError::NoValue)
        );
    }

    #[test]
    fn test_eval_eq_strings() {
        let e = Expr::BinOp(
            BinOp::Eq,
            Box::new(Expr::Lit(Value::String("Alice".to_string()))),
            Box::new(Expr::Lit(Value::String("Alice".to_string()))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_eq_int_float_false() {
        // Different Value variants → structural inequality
        let e = Expr::BinOp(
            BinOp::Eq,
            Box::new(Expr::Lit(Value::Integer(1))),
            Box::new(Expr::Lit(Value::Float(1.0))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(false))
        );
    }

    #[test]
    fn test_eval_type_mismatch_comparison_is_err() {
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Lit(Value::String("hello".to_string()))),
            Box::new(Expr::Lit(Value::Integer(100))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Err(EvalError::NoValue)
        );
    }

    /// Evaluate `a <op> b` for two string literals.
    fn eval_str_cmp(op: BinOp, a: &str, b: &str) -> Result<Value, EvalError> {
        eval_expr(
            &Expr::BinOp(
                op,
                Box::new(Expr::Lit(Value::String(a.to_string()))),
                Box::new(Expr::Lit(Value::String(b.to_string()))),
            ),
            &HashMap::new(),
            None,
        )
    }

    #[test]
    fn test_eval_string_comparison_is_lexicographic() {
        assert_eq!(
            eval_str_cmp(BinOp::Lt, "apple", "banana"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            eval_str_cmp(BinOp::Lt, "banana", "apple"),
            Ok(Value::Boolean(false))
        );
        assert_eq!(
            eval_str_cmp(BinOp::Gt, "banana", "apple"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_string_comparison_equal_operands() {
        // Strict comparisons are false and non-strict are true for equal strings.
        assert_eq!(
            eval_str_cmp(BinOp::Lt, "same", "same"),
            Ok(Value::Boolean(false))
        );
        assert_eq!(
            eval_str_cmp(BinOp::Gt, "same", "same"),
            Ok(Value::Boolean(false))
        );
        assert_eq!(
            eval_str_cmp(BinOp::Lte, "same", "same"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            eval_str_cmp(BinOp::Gte, "same", "same"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_string_comparison_totality() {
        // Regression: both directions returned no match, so a range filter silently
        // dropped every row instead of partitioning them. Exactly one must hold.
        let lt = eval_str_cmp(BinOp::Lt, "2025-05-06", "2026-01-01");
        let gte = eval_str_cmp(BinOp::Gte, "2025-05-06", "2026-01-01");
        assert_eq!(lt, Ok(Value::Boolean(true)), "earlier date sorts first");
        assert_eq!(gte, Ok(Value::Boolean(false)), "and not the other way");
    }

    #[test]
    fn test_eval_nan_comparison_is_false() {
        // Unchanged by the string arm: every ordering against NaN stays false.
        let e = Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Lit(Value::Float(f64::NAN))),
            Box::new(Expr::Lit(Value::Float(1.0))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(false))
        );

        let e = Expr::BinOp(
            BinOp::Gte,
            Box::new(Expr::Lit(Value::Float(f64::NAN))),
            Box::new(Expr::Lit(Value::Float(1.0))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(false))
        );
    }

    #[test]
    fn test_eval_string_q_true() {
        let e = Expr::UnaryOp(
            UnaryOp::StringQ,
            Box::new(Expr::Lit(Value::String("hi".to_string()))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_string_q_false() {
        let e = Expr::UnaryOp(UnaryOp::StringQ, Box::new(Expr::Lit(Value::Integer(1))));
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(false))
        );
    }

    #[test]
    fn test_eval_starts_with_true() {
        let e = Expr::BinOp(
            BinOp::StartsWith,
            Box::new(Expr::Lit(Value::String("foobar".to_string()))),
            Box::new(Expr::Lit(Value::String("foo".to_string()))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_ends_with_true() {
        let e = Expr::BinOp(
            BinOp::EndsWith,
            Box::new(Expr::Lit(Value::String("foobar".to_string()))),
            Box::new(Expr::Lit(Value::String("bar".to_string()))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn test_eval_contains_true() {
        let e = Expr::BinOp(
            BinOp::Contains,
            Box::new(Expr::Lit(Value::String("engineer at co".to_string()))),
            Box::new(Expr::Lit(Value::String("engineer".to_string()))),
        );
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
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
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Ok(Value::Boolean(true))
        );
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
    fn test_binders_expression_replaces_only_when_earlier_in_text() {
        use crate::query::datalog::types::WhereClause;
        let expr = Expr::Lit(Value::Integer(1));
        let bind = WhereClause::Expr {
            expr: expr.clone(),
            binding: Some("?x".to_string()),
        };
        let pat = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":score".to_string()),
            EdnValue::Symbol("?x".to_string()),
        );
        // The expression is first in the text and runs after the pattern.
        let mut binders = Binders::new(&[bind.clone(), WhereClause::Pattern(pat.clone())], vec![]);
        assert!(binders.pattern(&pat).is_empty());
        assert!(binders.expression(&expr, "?x"), "earlier in the text");
        // The pattern is first in the text: the expression only checks.
        let mut binders = Binders::new(&[WhereClause::Pattern(pat.clone()), bind.clone()], vec![]);
        assert!(binders.pattern(&pat).is_empty());
        assert!(!binders.expression(&expr, "?x"), "later in the text");
        // An expression that runs first binds; a later-run, text-earlier
        // pattern then gives the value.
        let mut binders = Binders::new(&[WhereClause::Pattern(pat.clone()), bind], vec![]);
        assert!(!binders.expression(&expr, "?x"));
        assert_eq!(binders.pattern(&pat), vec!["?x".to_string()]);
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
    fn test_or_union_from_two_branches() {
        // e1 has :color :red, e2 has :color :blue.
        // or-only where clause: (or [?e :color :red] [?e :color :blue])
        // Both entities are returned → 2 results.
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
    fn test_or_deduplication() {
        // e1 has :color :red AND :shape :circle.
        // or clause: (or [?e :color :red] [?e :shape :circle])
        // e1 is returned by both branches → deduplicated to 1 result.
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
    //
    // `execute_transact`/`execute_retract` (below) carry the QRY-001..006
    // codes, but `Minigraf::execute()`'s write path (src/db.rs `execute_inner`)
    // routes `transact`/`retract` commands through `Minigraf::materialize_transaction`/
    // `materialize_retraction` instead, for WAL-first-ordering reasons — it
    // never calls these methods. That makes these branches unreachable from
    // `db.execute()` today (confirmed empirically, not just by inspection), so
    // they are exercised here via direct `DatalogExecutor` construction, same
    // as the rest of this "unreachable via the parser" stream. `db.rs`'s own
    // near-duplicate messages are already earmarked as API-003/API-004 in
    // ERROR_REFERENCE.md for the API+INT category PR (#277 step 6); QRY-001/
    // 004/005/006 currently have no `db.rs`-side equivalent at all.

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
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-002");
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
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-002");
    }

    #[test]
    fn execute_transact_pseudo_attr_error() {
        // Exercises the Pseudo(_) arm in execute_transact
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
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-003");
    }

    #[test]
    fn execute_retract_pseudo_attr_error() {
        // Exercises the Pseudo(_) arm in execute_retract
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
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-003");
    }

    #[test]
    fn execute_transact_invalid_entity_error() {
        // Entity is neither a Keyword nor a Uuid — edn_to_entity_id fails.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Transact(Transaction {
            facts: vec![Pattern::new(
                EdnValue::String("not-a-uuid".to_string()),
                EdnValue::Keyword(":name".to_string()),
                EdnValue::String("Alice".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-001");
    }

    #[test]
    fn execute_retract_invalid_entity_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Retract(Transaction {
            facts: vec![Pattern::new(
                EdnValue::Integer(42),
                EdnValue::Keyword(":name".to_string()),
                EdnValue::String("Alice".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-001");
    }

    #[test]
    fn execute_transact_invalid_value_error() {
        // An unbound `?variable` symbol cannot be converted to a stored Value.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Transact(Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":e".to_string()),
                EdnValue::Keyword(":name".to_string()),
                EdnValue::Symbol("?unbound".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-004");
    }

    #[test]
    fn execute_retract_invalid_value_error() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);
        let cmd = DatalogCommand::Retract(Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":e".to_string()),
                EdnValue::Keyword(":name".to_string()),
                EdnValue::Symbol("?unbound".to_string()),
            )],
            valid_from: None,
            valid_to: None,
        });
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-004");
    }

    // NOTE: QRY-005 (Transaction failed) and QRY-006 (Retraction failed) wrap
    // `FactStorage::transact_batch`/`retract`'s only failure mode — its
    // private `data: RwLock<FactData>` (src/graph/storage.rs) being poisoned.
    // That field isn't reachable from this module (or from any of the other
    // four files in this issue's scope) without a test-only hook inside
    // `src/graph/storage.rs`, which is outside this PR's scope (issue #361
    // covers only executor.rs/evaluator.rs/optimizer.rs/matcher.rs/
    // magic_sets.rs). The call sites are migrated and their templates are
    // verified against ERROR_REFERENCE.md by `error::tests::
    // registry_is_a_subset_of_error_reference_doc`; a genuine end-to-end
    // trigger is left to whichever PR next touches src/graph/storage.rs.

    /// Poisons `lock` with a real panic in another thread while the write
    /// guard is held, then joins (discarding the panic payload). Unlike
    /// `FactStorage`'s internal `data` lock, `rules`/`functions` are
    /// constructor parameters of `DatalogExecutor`, so they can be poisoned
    /// from outside — no test hook needed in another file.
    fn poison_rwlock<T>(lock: &Arc<RwLock<T>>)
    where
        T: Send + Sync + 'static,
    {
        let lock = lock.clone();
        let _ = std::thread::spawn(move || {
            let _guard = lock.write().unwrap();
            panic!("deliberate poison for a QRY-008/QRY-009 regression test");
        })
        .join();
    }

    #[test]
    #[cfg_attr(
        target_os = "wasi",
        ignore = "poisons the lock from a spawned thread; WASI has no threads"
    )]
    fn execute_query_functions_lock_poisoned_error() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        poison_rwlock(&functions);

        let executor = DatalogExecutor::new_with_rules_and_functions(storage, rules, functions);
        let cmd = parse_datalog_command(r#"(query [:find ?e :where [?e :name ?n]])"#).unwrap();
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-008");
    }

    #[test]
    #[cfg_attr(
        target_os = "wasi",
        ignore = "poisons the lock from a spawned thread; WASI has no threads"
    )]
    fn execute_rule_rules_lock_poisoned_error() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        poison_rwlock(&rules);

        let executor = DatalogExecutor::new_with_rules_and_functions(storage, rules, functions);
        let cmd = parse_datalog_command(r#"(rule [(p ?x) [?x :a true]])"#).unwrap();
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-009");
    }

    #[test]
    #[cfg_attr(
        target_os = "wasi",
        ignore = "poisons the lock from a spawned thread; WASI has no threads"
    )]
    fn rule_query_rules_lock_poisoned_error() {
        // Covers the magic-sets rewrite's rules.read() in a rule query
        // (distinct call site from execute_rule's rules.write()).
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        {
            // Register a rule normally first so the query recognises it as
            // rule-backed (uses_rules()) and takes the rules path.
            let mut reg = rules.write().unwrap();
            reg.register_rule(
                "p".to_string(),
                Rule {
                    head: vec![
                        EdnValue::Symbol("p".to_string()),
                        EdnValue::Symbol("?x".to_string()),
                    ],
                    body: vec![WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":a".to_string()),
                        EdnValue::Boolean(true),
                    ))],
                },
            )
            .unwrap();
        }
        poison_rwlock(&rules);

        let executor = DatalogExecutor::new_with_rules_and_functions(storage, rules, functions);
        let cmd = parse_datalog_command(r#"(query [:find ?x :where (p ?x)])"#).unwrap();
        let r = executor.execute(cmd);
        let err: crate::error::MinigrafError = r.unwrap_err().into();
        assert_eq!(err.code(), "QRY-009");
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
        assert_eq!(
            eval_expr(&e, &HashMap::new(), None),
            Err(EvalError::NoValue)
        );
    }

    #[test]
    fn test_eval_float_div_succeeds() {
        // Line 1096 false branch: rf != 0.0 → Ok(Float)
        let e = Expr::BinOp(
            BinOp::Div,
            Box::new(Expr::Lit(Value::Float(6.0))),
            Box::new(Expr::Lit(Value::Float(2.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Float(3.0)));
    }

    #[test]
    fn test_eval_float_sub() {
        // Line 1079-1085: float subtraction
        let e = Expr::BinOp(
            BinOp::Sub,
            Box::new(Expr::Lit(Value::Float(5.0))),
            Box::new(Expr::Lit(Value::Float(2.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Float(3.0)));
    }

    #[test]
    fn test_eval_float_mul() {
        // Line 1087-1093: float multiplication
        let e = Expr::BinOp(
            BinOp::Mul,
            Box::new(Expr::Lit(Value::Float(3.0))),
            Box::new(Expr::Lit(Value::Float(4.0))),
        );
        assert_eq!(eval_expr(&e, &HashMap::new(), None), Ok(Value::Float(12.0)));
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
                        crate::graph::types::Value::Float(3.5),
                    ),
                    (
                        e2,
                        ":item/score".to_string(),
                        crate::graph::types::Value::Float(2.5),
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
                    crate::graph::types::Value::Float(3.5),
                    "max of floats should return largest"
                );
            }
            _ => panic!("expected QueryResults"),
        }
    }

    // ── or branch edge cases ─────────────────────────────────────────────────

    #[test]
    fn rule_query_per_fact_pseudo_attr_needs_any_valid_time() {
        use crate::query::datalog::parser::parse_datalog_command;
        let executor = DatalogExecutor::new(FactStorage::new());
        for cmd in [
            r#"(rule [(tagged ?e) [?e :item/tag ?v]])"#,
            r#"(transact [[:item1 :item/tag "x"]])"#,
        ] {
            executor
                .execute(parse_datalog_command(cmd).unwrap())
                .unwrap();
        }
        let err_cmd = parse_datalog_command(
            "(query [:find ?e ?vf :where (tagged ?e) [?e ?a _] [?e :db/valid-from ?vf]])",
        )
        .expect("err query parse failed");
        let err_result = executor.execute(err_cmd);
        assert!(
            err_result.is_err(),
            "per-fact pseudo-attr without :any-valid-time in rules query must fail"
        );
    }

    #[test]
    fn test_or_branch_not_excludes_matching() {
        // Entities that match the not body are excluded.
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
            r#"(transact {:valid-to "2125-01-01T00:00:00Z"} [[:alice :person/name "Alice"]])"#,
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
            r#"(transact [[:alice :person/name "Alice" {:valid-to "2125-01-01T00:00:00Z"}]])"#,
        )
        .expect("parse with per-fact valid-to only should succeed");
        let result = executor.execute(cmd);
        assert!(
            result.is_ok(),
            "transact with valid-to only per fact should succeed"
        );
    }

    #[test]
    fn test_or_branch_not_join_excludes_matching() {
        // An entity the not-join body matches is excluded.
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
        // An expression-only not body.
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
}

#[cfg(test)]
mod selective_lookup_tests {
    use crate::graph::FactStorage;
    use crate::query::datalog::executor::DatalogExecutor;

    fn make_db_with_entities(n: usize) -> DatalogExecutor {
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(50) {
            let batch_end = (batch_start + 50).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!(r#"[:e{i} :name "entity{i}"]"#, i = i));
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        exec
    }

    #[test]
    fn entity_bound_query_returns_correct_results() {
        let exec = make_db_with_entities(100);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    r#"(query [:find ?n :where [:e5 :name ?n]])"#,
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 1, "expected exactly 1 result for entity :e5");
            assert_eq!(
                results[0][0],
                crate::graph::types::Value::String("entity5".to_string())
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn attribute_bound_query_returns_correct_results() {
        let exec = make_db_with_entities(100);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e ?v :where [?e :val ?v]])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(
                results.len(),
                100,
                "expected 100 results for :val attribute scan"
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn as_of_query_still_works_after_change() {
        let exec = make_db_with_entities(10);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?n :where [?e :name ?n] :as-of 1])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert!(!results.is_empty(), "expected results from as-of 1 query");
        } else {
            panic!("expected QueryResults");
        }
    }
}

#[cfg(test)]
mod not_hash_join_tests {
    use crate::graph::FactStorage;
    use crate::query::datalog::executor::DatalogExecutor;

    fn make_not_db(n: usize, excluded: usize) -> DatalogExecutor {
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(100) {
            let batch_end = (batch_start + 100).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        for batch_start in (0..excluded).step_by(100) {
            let batch_end = (batch_start + 100).min(excluded);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :banned true]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        exec
    }

    #[test]
    fn not_filter_returns_correct_count() {
        let n = 1_000;
        let excluded = n / 10; // 100 banned
        let exec = make_not_db(n, excluded);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e :where [?e :val ?v] (not [?e :banned true])])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(
                results.len(),
                n - excluded,
                "expected {} results after not-filter",
                n - excluded
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn not_join_filter_returns_correct_count() {
        let n = 1_000;
        let excluded = n / 10;
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(100) {
            let batch_end = (batch_start + 100).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        exec.execute(
            crate::query::datalog::parser::parse_datalog_command(
                "(transact [[:d-bad :status :bad]])",
            )
            .unwrap(),
        )
        .unwrap();
        for batch_start in (0..excluded).step_by(100) {
            let batch_end = (batch_start + 100).min(excluded);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :dep :d-bad]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e :where [?e :val ?v] \
                     (not-join [?e] [?e :dep ?d] [?d :status :bad])])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(
                results.len(),
                n - excluded,
                "expected {} results after not-join-filter",
                n - excluded
            );
        } else {
            panic!("expected QueryResults");
        }
    }
}

#[cfg(test)]
mod or_hash_join_tests {
    use crate::graph::FactStorage;
    use crate::query::datalog::executor::DatalogExecutor;

    fn make_or_db(n: usize, a_count: usize, b_count: usize) -> DatalogExecutor {
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(100) {
            let batch_end = (batch_start + 100).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        for batch_start in (0..a_count).step_by(100) {
            let batch_end = (batch_start + 100).min(a_count);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :tag-a true]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        let b_start = n.saturating_sub(b_count);
        for batch_start in (b_start..n).step_by(100) {
            let batch_end = (batch_start + 100).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :tag-b true]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        exec
    }

    #[test]
    fn or_clause_returns_correct_count() {
        // 1000 entities, first 250 have :tag-a, last 250 have :tag-b, no overlap
        let n = 1_000;
        let a = n / 4;
        let b = n / 4;
        let exec = make_or_db(n, a, b);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e :where [?e :val ?v] \
                     (or [?e :tag-a true] [?e :tag-b true])])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(
                results.len(),
                a + b,
                "expected {} results from or (a={} b={} no overlap)",
                a + b,
                a,
                b
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn or_join_clause_returns_correct_count() {
        let n = 1_000;
        let a = n / 4;
        let b = n / 4;
        let exec = make_or_db(n, a, b);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e :where [?e :val ?v] \
                     (or-join [?e] [?e :tag-a true] [?e :tag-b true])])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(
                results.len(),
                a + b,
                "expected {} results from or-join",
                a + b
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn or_clause_with_overlap_deduplicates() {
        // 100 entities, all have both :tag-a and :tag-b → result should be 100, not 200
        let n = 100;
        let exec = make_or_db(n, n, n);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e :where [?e :val ?v] \
                     (or [?e :tag-a true] [?e :tag-b true])])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), n, "expected {} deduplicated results", n);
        } else {
            panic!("expected QueryResults");
        }
    }
}

#[cfg(test)]
mod pushdown_tests {
    use crate::graph::FactStorage;
    use crate::graph::types::Value;
    use crate::query::datalog::executor::{DatalogExecutor, QueryResult};
    use crate::query::datalog::parser::parse_datalog_command;

    #[test]
    fn test_expr_pushdown_preserves_query_results() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute(
                parse_datalog_command("(transact [[:e1 :val 10] [:e2 :val 20] [:e3 :val 30]])")
                    .unwrap(),
            )
            .unwrap();
        let result = executor
            .execute(
                parse_datalog_command("(query [:find ?e ?v :where [?e :val ?v] [(> ?v 15)]])")
                    .unwrap(),
            )
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "only :e2 and :e3 have :val > 15");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_expr_pushdown_multi_pattern_preserves_results() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute(
                parse_datalog_command(
                    r#"(transact [[:e1 :val 5] [:e1 :name "a"] [:e2 :val 20] [:e2 :name "b"]])"#,
                )
                .unwrap(),
            )
            .unwrap();
        let result = executor
            .execute(
                parse_datalog_command(
                    r#"(query [:find ?e ?n :where [?e :val ?v] [?e :name ?n] [(> ?v 10)]])"#,
                )
                .unwrap(),
            )
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 1, "only :e2 passes the predicate");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_expr_binding_form_preserves_results() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute(parse_datalog_command("(transact [[:e1 :val 5] [:e2 :val 10]])").unwrap())
            .unwrap();
        let result = executor
            .execute(
                parse_datalog_command(
                    "(query [:find ?e ?doubled :where [?e :val ?v] [(* ?v 2) ?doubled]])",
                )
                .unwrap(),
            )
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "both entities must appear");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_expr_pushdown_with_rules_preserves_results() {
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage.clone());
        executor
            .execute(
                parse_datalog_command("(transact [[:e1 :val 5] [:e2 :val 20] [:e3 :val 30]])")
                    .unwrap(),
            )
            .unwrap();
        executor
            .execute(parse_datalog_command("(rule [(high ?e) [?e :val ?v] [(> ?v 15)]])").unwrap())
            .unwrap();
        let result = executor
            .execute(parse_datalog_command("(query [:find ?e :where (high ?e)])").unwrap())
            .unwrap();
        if let QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 2, "only :e2 and :e3 qualify");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn test_not_clause_ordering_correctness() {
        // Two `not` clauses given in expensive-first source order; results must be
        // identical regardless of which order the optimizer chooses to evaluate them.
        // This is a correctness regression guard — semantics must not change.
        let storage = FactStorage::new();
        let executor = DatalogExecutor::new(storage);

        // Transact 3 items: widget, gadget, doohickey
        executor
            .execute(
                parse_datalog_command(
                    r#"(transact [[:item1 :item/name "widget"]
                                  [:item2 :item/name "gadget"]
                                  [:item3 :item/name "doohickey"]])"#,
                )
                .unwrap(),
            )
            .unwrap();

        // Query: find items that are NOT "widget" AND NOT "gadget"
        // Clauses are given in expensive-first order to exercise the cost-based sort.
        let result = executor
            .execute(
                parse_datalog_command(
                    r#"(query [:find ?name
                               :where [?e :item/name ?name]
                                      (not [?e :item/name "gadget"])
                                      (not [?e :item/name "widget"])])"#,
                )
                .unwrap(),
            )
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "only doohickey should pass");
                if let Value::String(ref s) = results[0][0] {
                    assert_eq!(s.as_str(), "doohickey", "result should be doohickey");
                } else {
                    panic!("expected a String value for the result");
                }
            }
            _ => panic!("expected QueryResults"),
        }
    }
}
