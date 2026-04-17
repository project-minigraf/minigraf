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
use super::functions::FunctionRegistry;
use super::matcher::{Bindings, PatternMatcher, edn_to_entity_id, edn_to_value};
use super::rules::RuleRegistry;
use super::types::{AttributeSpec, EdnValue, Pattern, Rule, WhereClause};
use crate::graph::FactStorage;
use crate::graph::types::{Fact, Value};
use crate::storage::index::encode_value;
use anyhow::{Result, anyhow};
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Default maximum iterations for recursive evaluation (kept for reference).
#[allow(dead_code)]
pub const DEFAULT_MAX_ITERATIONS: usize = 1000;
/// Default maximum facts that can be derived per iteration
pub const DEFAULT_MAX_DERIVED_FACTS: usize = 100_000;
/// Default maximum total query results
pub const DEFAULT_MAX_RESULTS: usize = 1_000_000;

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
    /// Function registry for UDF predicates/aggregates
    functions: Arc<RwLock<FunctionRegistry>>,
    /// Maximum iterations before giving up (prevents infinite loops)
    max_iterations: usize,
    /// Maximum facts that can be derived per iteration
    max_derived_facts: usize,
    /// Maximum total query results
    max_results: usize,
}

impl RecursiveEvaluator {
    /// Create a new recursive evaluator.
    ///
    /// # Arguments
    /// * `storage` - Base fact storage
    /// * `rules` - Rule registry
    /// * `functions` - Function registry for UDF predicates/aggregates
    /// * `max_iterations` - Safety limit (e.g., 1000)
    /// * `max_derived_facts` - Maximum facts per iteration
    /// * `max_results` - Maximum total results
    pub fn new(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        functions: Arc<RwLock<FunctionRegistry>>,
        max_iterations: usize,
        max_derived_facts: usize,
        max_results: usize,
    ) -> Self {
        RecursiveEvaluator {
            storage,
            rules,
            functions,
            max_iterations,
            max_derived_facts,
            max_results,
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
            derived.transact(
                vec![(fact.entity, fact.attribute.clone(), fact.value.clone())],
                None,
            )?;
        }

        // Track facts we've seen (for delta computation) using BTreeSet with canonical encoding.
        let mut seen_facts: BTreeSet<(uuid::Uuid, String, Vec<u8>)> = base_facts
            .iter()
            .map(|f| {
                let encoded = encode_value(&f.value);
                (f.entity, f.attribute.clone(), encoded)
            })
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

            // Check per-iteration fact limit
            if new_facts.len() > self.max_derived_facts {
                return Err(anyhow!(
                    "Max derived facts per iteration ({}) exceeded. Rule may be generating too many facts.",
                    self.max_derived_facts
                ));
            }

            // Compute delta: facts not yet seen
            let mut delta = Vec::new();
            for fact in new_facts {
                let encoded = encode_value(&fact.value);
                let key = (fact.entity, fact.attribute.clone(), encoded);
                if !seen_facts.contains(&key) {
                    // Check total result limit
                    if seen_facts.len() >= self.max_results {
                        return Err(anyhow!(
                            "Max query results ({}) exceeded.",
                            self.max_results
                        ));
                    }
                    seen_facts.insert(key);
                    delta.push(fact);
                }
            }

            // If no new facts, we've reached fixed point
            if delta.is_empty() {
                break;
            }

            // Add delta facts to derived storage
            for fact in delta {
                derived.transact(
                    vec![(fact.entity, fact.attribute.clone(), fact.value.clone())],
                    None,
                )?;
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

        let registry = self.rules.read().expect("lock poisoned");

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

        // Separate Pattern/RuleInvocation clauses from Expr clauses
        let mut patterns = Vec::new();
        let mut expr_clauses: Vec<&WhereClause> = Vec::new();
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
                WhereClause::NotJoin { .. } => {
                    // NotJoin clauses are handled by StratifiedEvaluator, not here.
                    return Err(anyhow!(
                        "WhereClause::NotJoin in evaluate_rule: use StratifiedEvaluator for rules with negation"
                    ));
                }
                WhereClause::Expr { .. } => {
                    expr_clauses.push(clause);
                }
                WhereClause::Or(_) | WhereClause::OrJoin { .. } => {
                    // Or/OrJoin rules are routed to the mixed_rules path by StratifiedEvaluator before reaching here.
                    return Err(anyhow!(
                        "WhereClause::Or/OrJoin in evaluate_rule: not yet implemented"
                    ));
                }
            }
        }

        if patterns.is_empty() && expr_clauses.is_empty() {
            return Ok(derived);
        }

        let matcher = PatternMatcher::new(current_facts.clone());
        let bindings = if patterns.is_empty() {
            // Expr-only rule body: seed with empty binding
            vec![Bindings::new()]
        } else {
            matcher.match_patterns(&patterns)
        };

        // Apply Expr clauses to filter/extend bindings
        let bindings = apply_expr_clauses_in_evaluator(
            bindings,
            &expr_clauses,
            &self.functions.read().expect("lock poisoned"),
        );

        for binding in bindings {
            let fact = self.instantiate_head(&rule.head, &binding)?;
            derived.push(fact);
        }

        Ok(derived)
    }

    /// Convert a rule invocation to a pattern.
    ///
    /// Example: (reachable ?x ?y) -> [?x :reachable ?y]
    /// Example: (blocked ?x) -> [?x :blocked ?_rule_value]
    fn rule_invocation_to_pattern(&self, list: &[EdnValue]) -> Result<Pattern> {
        if list.is_empty() {
            return Err(anyhow!("Rule invocation cannot be empty"));
        }
        let predicate = match &list[0] {
            EdnValue::Symbol(s) => s.clone(),
            _ => {
                return Err(anyhow!(
                    "Rule invocation must start with predicate name (symbol)"
                ));
            }
        };
        let args: Vec<EdnValue> = list[1..].to_vec();
        rule_invocation_to_pattern(&predicate, &args)
    }

    /// Instantiate rule head with variable bindings to create a derived fact.
    ///
    /// # Example
    /// Head: (reachable ?x ?y)
    /// Bindings: {?x -> alice_uuid, ?y -> bob_uuid}
    /// Result: Fact(alice_uuid, ":reachable", Ref(bob_uuid))
    fn instantiate_head(&self, head: &[EdnValue], binding: &Bindings) -> Result<Fact> {
        if head.len() < 2 {
            return Err(anyhow!(
                "Rule head must have at least 2 elements: (predicate ?arg1)"
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

        let value = if head.len() >= 3 {
            // 2-arg head: (reachable ?from ?to) — value is head[2]
            let value_edn = self.substitute_variable(&head[2], binding)?;
            edn_to_value(&value_edn).map_err(|e| anyhow!("Failed to convert value: {}", e))?
        } else {
            // 1-arg head: (blocked ?x) — store a Boolean(true) sentinel
            crate::graph::types::Value::Boolean(true)
        };

        // Create fact with derived predicate as attribute
        // Use ":predicate-name" as the attribute for derived facts
        let attribute = format!(":{}", predicate);

        // Create the fact (no tx_id yet, will be added when transacted)
        Ok(Fact::new(entity, attribute, value, 0))
    }

    /// Substitute a variable with its binding, or return as-is if not a variable.
    fn substitute_variable(&self, edn: &EdnValue, binding: &Bindings) -> Result<EdnValue> {
        match edn {
            EdnValue::Symbol(s) if s.starts_with('?') => {
                // This is a variable
                if let Some(value) = binding.get(s) {
                    // Convert Value back to EdnValue for entity/value conversion
                    Ok(value_to_edn(value))
                } else {
                    Err(anyhow!("Unbound variable in rule head: {}", s))
                }
            }
            _ => Ok(edn.clone()), // Not a variable, use as-is
        }
    }

    /// Public version of instantiate_head for use by StratifiedEvaluator.
    pub fn instantiate_head_public(&self, head: &[EdnValue], binding: &Bindings) -> Result<Fact> {
        self.instantiate_head(head, binding)
    }
}

/// Convert a stored Value back to EdnValue.
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

/// Substitute bound variables in a Pattern, returning a new Pattern with concrete values.
pub fn substitute_pattern(pattern: &Pattern, binding: &Bindings) -> Pattern {
    let attribute = match &pattern.attribute {
        AttributeSpec::Real(edn) => AttributeSpec::Real(substitute_value(edn, binding)),
        AttributeSpec::Pseudo(p) => AttributeSpec::Pseudo(p.clone()),
    };
    Pattern {
        entity: substitute_value(&pattern.entity, binding),
        attribute,
        value: substitute_value(&pattern.value, binding),
        valid_from: pattern.valid_from,
        valid_to: pattern.valid_to,
    }
}

/// Substitute a single value: if it's a bound variable, replace it; otherwise clone.
pub fn substitute_value(value: &EdnValue, binding: &Bindings) -> EdnValue {
    if let Some(var) = value.as_variable() {
        binding
            .get(var)
            .map(value_to_edn)
            .unwrap_or_else(|| value.clone())
    } else {
        value.clone()
    }
}

/// Convert a predicate name + argument list to a Pattern.
///
/// 1-arg: `(blocked ?x)` → `[?x :blocked ?_rule_value]`
/// 2-arg: `(reachable ?from ?to)` → `[?from :reachable ?to]`
pub(crate) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
    match args.len() {
        1 => Ok(Pattern::new(
            args[0].clone(),
            EdnValue::Keyword(format!(":{}", predicate)),
            EdnValue::Symbol("?_rule_value".to_string()),
        )),
        2 => Ok(Pattern::new(
            args[0].clone(),
            EdnValue::Keyword(format!(":{}", predicate)),
            args[1].clone(),
        )),
        n => Err(anyhow!(
            "Rule invocation '{}' must have 1 or 2 arguments, got {}",
            predicate,
            n
        )),
    }
}

/// Test whether a `not-join` body is satisfiable given a current binding.
///
/// Returns `true` if the body IS satisfiable → outer binding should be **rejected**.
/// Returns `false` if the body cannot be satisfied → outer binding survives.
///
/// Algorithm:
/// 1. Build a partial binding containing only the join_vars entries.
/// 2. For each clause:
///    - Pattern → substitute join_vars via substitute_pattern.
///    - RuleInvocation → convert to Pattern via rule_invocation_to_pattern, then substitute.
///      Rule-derived facts are already present in `storage` (accumulated) from lower strata.
/// 3. Run PatternMatcher::match_patterns on all resulting patterns against `storage`.
/// 4. Any complete match → body is satisfiable → return true (reject outer binding).
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: Arc<[Fact]>,
    functions: &FunctionRegistry,
) -> bool {
    // Build a partial binding containing only the join variables
    let partial: Bindings = join_vars
        .iter()
        .filter_map(|v| binding.get(v.as_str()).map(|val| (v.clone(), val.clone())))
        .collect();

    // Convert Pattern and RuleInvocation clauses to patterns (excluding Expr)
    let substituted: Vec<Pattern> = clauses
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(substitute_pattern(p, &partial)),
            WhereClause::RuleInvocation { predicate, args } => {
                rule_invocation_to_pattern(predicate, args)
                    .ok()
                    .map(|p| substitute_pattern(&p, &partial))
            }
            _ => None,
        })
        .collect();

    // Collect Expr clauses from the not-join body
    let expr_clauses: Vec<&WhereClause> = clauses
        .iter()
        .filter(|c| matches!(c, WhereClause::Expr { .. }))
        .collect();

    let matcher = PatternMatcher::from_slice(storage.clone());
    let mut not_bindings: Vec<Bindings> = if substituted.is_empty() {
        // Expr-only not-join body: seed with the partial binding so variables resolve.
        vec![partial.clone()]
    } else {
        matcher
            .match_patterns(&substituted)
            .into_iter()
            .map(|mut nb| {
                for (k, v) in &partial {
                    nb.entry(k.clone()).or_insert_with(|| v.clone());
                }
                nb
            })
            .collect()
    };

    // Apply Expr clauses to filter not_bindings
    not_bindings = apply_expr_clauses_in_evaluator(not_bindings, &expr_clauses, functions);
    !not_bindings.is_empty()
}

// NOTE: This mirrors apply_expr_clauses in executor.rs but uses the evaluator's
// Bindings type alias (HashMap<String, Value> from matcher.rs). The duplication
// is structural — both type aliases resolve to the same concrete type but are
// defined separately. If expression evaluation semantics change, both must be
// updated in sync. TODO: unify into a shared module (e.g., expr.rs) in a future cleanup.
/// Apply WhereClause::Expr clauses to a list of bindings.
///
/// Filter-form (`binding: None`) drops rows where the expr is not truthy or errors.
/// Binding-form (`binding: Some(var)`) extends the row with the computed value.
fn apply_expr_clauses_in_evaluator(
    bindings: Vec<Bindings>,
    expr_clauses: &[&WhereClause],
    registry: &FunctionRegistry,
) -> Vec<Bindings> {
    use crate::query::datalog::executor::{eval_expr, is_truthy};
    bindings
        .into_iter()
        .filter_map(|mut b| {
            for clause in expr_clauses {
                if let WhereClause::Expr { expr, binding: out } = clause {
                    match eval_expr(expr, &b, Some(registry)) {
                        Ok(value) => match out {
                            None => {
                                if !is_truthy(&value) {
                                    return None;
                                }
                            }
                            Some(var) => {
                                b.insert(var.clone(), value);
                            }
                        },
                        Err(_) => return None,
                    }
                }
            }
            Some(b)
        })
        .collect()
}

/// Evaluates Datalog rules with stratified negation support.
///
/// Strata are evaluated in ascending order. Within each stratum, positive-only
/// rules are handled by RecursiveEvaluator; rules containing `not` clauses are
/// handled by an inner loop that applies `not` filters to candidate bindings.
pub struct StratifiedEvaluator {
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
    max_iterations: usize,
    max_derived_facts: usize,
    max_results: usize,
}

impl StratifiedEvaluator {
    pub fn new(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        functions: Arc<RwLock<FunctionRegistry>>,
        max_iterations: usize,
        max_derived_facts: usize,
        max_results: usize,
    ) -> Self {
        StratifiedEvaluator {
            storage,
            rules,
            functions,
            max_iterations,
            max_derived_facts,
            max_results,
        }
    }

    /// Derive all facts for the given predicates, respecting stratification order.
    pub fn evaluate(&self, predicates: &[String]) -> Result<FactStorage> {
        use crate::query::datalog::stratification::DependencyGraph;

        let registry = self.rules.read().expect("lock poisoned");

        // Build dependency graph and stratify
        let graph = DependencyGraph::from_rules(&registry);
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

        let accumulated = self.storage.clone();

        for stratum in 0..=max_stratum {
            let registry = self.rules.read().expect("lock poisoned");
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
                    let has_not = rule.body.iter().any(|c| {
                        matches!(
                            c,
                            WhereClause::Not(_)
                                | WhereClause::NotJoin { .. }
                                | WhereClause::Or(_)
                                | WhereClause::OrJoin { .. }
                        )
                    });
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
                    self.functions.clone(),
                    self.max_iterations,
                    self.max_derived_facts,
                    self.max_results,
                );
                let derived = sub_eval.evaluate_recursive_rules(&stratum_preds)?;
                // Snapshot existing fact keys so we only load truly new (derived) facts
                let existing: HashSet<(uuid::Uuid, String, Vec<u8>)> = accumulated
                    .get_asserted_facts()?
                    .into_iter()
                    .map(|f| {
                        let encoded = encode_value(&f.value);
                        (f.entity, f.attribute, encoded)
                    })
                    .collect();
                for fact in derived.get_asserted_facts()? {
                    let key = (
                        fact.entity,
                        fact.attribute.clone(),
                        encode_value(&fact.value),
                    );
                    if !existing.contains(&key) {
                        let _ = accumulated.load_fact(fact);
                    }
                }
                accumulated.restore_tx_counter()?;
            }

            // Evaluate mixed rules (with not-filter)
            for (_pred, rule) in &mixed_rules {
                let positive_patterns: Vec<Pattern> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Pattern(p) => Some(p.clone()),
                        WhereClause::RuleInvocation { predicate, args } => match args.len() {
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
                            _ => None,
                        },
                        WhereClause::Not(_) | WhereClause::NotJoin { .. } => None,
                        WhereClause::Expr { .. } => None,
                        WhereClause::Or(_) | WhereClause::OrJoin { .. } => None, // Or/OrJoin handled by apply_or_clauses below, not extracted as positive patterns.
                    })
                    .collect();

                let not_clauses: Vec<Vec<WhereClause>> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Not(inner) => Some(inner.clone()),
                        _ => None,
                    })
                    .collect();

                let not_join_clauses: Vec<(Vec<String>, Vec<WhereClause>)> = rule
                    .body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::NotJoin { join_vars, clauses } => {
                            Some((join_vars.clone(), clauses.clone()))
                        }
                        _ => None,
                    })
                    .collect();

                // Collect top-level Expr clauses from the rule body
                let body_expr_clauses: Vec<&WhereClause> = rule
                    .body
                    .iter()
                    .filter(|c| matches!(c, WhereClause::Expr { .. }))
                    .collect();

                // Compute once; reuse for matcher, apply_or_clauses, not-body matching, and evaluate_not_join.
                // Declared at loop body scope so it remains in scope for all four usages below.
                let accumulated_facts: Arc<[Fact]> =
                    Arc::from(accumulated.get_asserted_facts().unwrap_or_default());

                let matcher = PatternMatcher::from_slice(accumulated_facts.clone());
                let raw_candidates = matcher.match_patterns(&positive_patterns);

                // Apply Or/OrJoin clauses before Expr (mirrors top-level execute_query order)
                let or_expanded = {
                    use crate::query::datalog::executor::apply_or_clauses;
                    use crate::query::datalog::functions::FunctionRegistry;
                    let registry_guard = self.rules.read().expect("lock poisoned");
                    // Rule bodies in the semi-naive evaluator don't have access to a
                    // FunctionRegistry (UDF registration happens at the db layer). Use the
                    // built-in-only registry so or-branches can still use built-in predicates.
                    let fn_registry = FunctionRegistry::with_builtins();
                    let expanded = apply_or_clauses(
                        &rule.body,
                        raw_candidates,
                        accumulated_facts.clone(),
                        &registry_guard,
                        None,
                        None,
                        &fn_registry,
                    )?;
                    drop(registry_guard);
                    expanded
                };

                // Apply top-level Expr clauses to filter/extend candidates
                let candidates = apply_expr_clauses_in_evaluator(
                    or_expanded,
                    &body_expr_clauses,
                    &self.functions.read().expect("lock poisoned"),
                );

                // Build temp_eval once per rule (outside the binding loop);
                // instantiate_head_public only uses storage, not the registry.
                let temp_eval = RecursiveEvaluator::new(
                    accumulated.clone(),
                    Arc::clone(&self.rules),
                    Arc::clone(&self.functions),
                    1,
                    self.max_derived_facts,
                    self.max_results,
                );

                'binding: for binding in candidates {
                    for not_body in &not_clauses {
                        let substituted: Vec<Pattern> = not_body
                            .iter()
                            .filter_map(|c| match c {
                                WhereClause::Pattern(p) => Some(substitute_pattern(p, &binding)),
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
                                        _ => None,
                                    }
                                }
                                WhereClause::Not(_) | WhereClause::NotJoin { .. } => None,
                                WhereClause::Expr { .. } => None,
                                WhereClause::Or(_) | WhereClause::OrJoin { .. } => None, // Or/OrJoin inside not bodies: rejected at parse time (see parser.rs).
                            })
                            .collect();

                        let not_matcher = PatternMatcher::from_slice(accumulated_facts.clone());
                        let mut not_bindings: Vec<Bindings> = if substituted.is_empty() {
                            vec![binding.clone()]
                        } else {
                            not_matcher
                                .match_patterns(&substituted)
                                .into_iter()
                                .map(|mut nb| {
                                    for (k, v) in &binding {
                                        nb.entry(k.clone()).or_insert_with(|| v.clone());
                                    }
                                    nb
                                })
                                .collect()
                        };

                        // Apply Expr clauses from the not body
                        let not_body_expr_clauses: Vec<&WhereClause> = not_body
                            .iter()
                            .filter(|c| matches!(c, WhereClause::Expr { .. }))
                            .collect();
                        not_bindings = apply_expr_clauses_in_evaluator(
                            not_bindings,
                            &not_body_expr_clauses,
                            &self.functions.read().expect("lock poisoned"),
                        );
                        if !not_bindings.is_empty() {
                            continue 'binding; // not condition violated -> discard binding
                        }
                    }

                    for (join_vars, nj_clauses) in &not_join_clauses {
                        if evaluate_not_join(
                            join_vars,
                            nj_clauses,
                            &binding,
                            accumulated_facts.clone(),
                            &self.functions.read().expect("lock poisoned"),
                        ) {
                            continue 'binding;
                        }
                    }

                    // All Not / NotJoin conditions held -> derive head fact
                    if let Ok(fact) = temp_eval.instantiate_head_public(&rule.head, &binding) {
                        // Use transact (not load_fact) so derived facts get a proper
                        // tx_id and incremented tx_count, matching spec step (d).
                        let _ = accumulated
                            .transact(vec![(fact.entity, fact.attribute, fact.value)], None);
                    }
                }
            }
        }

        Ok(accumulated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::functions::FunctionRegistry;
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (b, ":connected".to_string(), Value::Ref(c)),
                ],
                None,
            )
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
            rules
                .write()
                .unwrap()
                .register_rule(predicate, rule)
                .unwrap();
        } else {
            panic!("Expected Rule command");
        }
    }

    #[test]
    fn test_evaluator_creation() {
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));

        let _evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        // Note: RecursiveEvaluator uses evaluate_recursive_rules, not evaluate
        // let derived = _evaluator.evaluate(&["p".to_string()]).unwrap();
    }

    // ── Additional targeted branch coverage ───────────────────────────────────

    #[test]
    fn test_max_iterations_exceeded_returns_error() {
        // Line 113: iteration > self.max_iterations → returns Err
        // Use a recursive rule that keeps deriving new facts beyond max_iterations=0
        let storage = create_test_storage();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        // Transitive closure rule — requires multiple iterations
        register_test_rule(&rules, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
        register_test_rule(
            &rules,
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        );

        // max_iterations=0 means it will exceed the limit on the very first iteration
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions.clone(),
            0,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        let result = evaluator.evaluate_recursive_rules(&["reachable".to_string()]);
        assert!(result.is_err(), "should fail when max iterations exceeded");
    }

    #[test]
    fn test_rule_invocation_empty_list_error() {
        // Line 253: rule_invocation_to_pattern with empty list → Err
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        let result = evaluator.rule_invocation_to_pattern(&[]);
        assert!(
            result.is_err(),
            "empty rule invocation list should return error"
        );
    }

    #[test]
    fn test_head_requires_args() {
        // Line 233: head.len() != arg_count → return Err
        let storage = FactStorage::new();
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        let head = vec![EdnValue::Symbol("predicate".to_string())]; // only 1 element
        let binding = std::collections::HashMap::new();
        let result = evaluator.instantiate_head(&head, &binding);
        assert!(
            result.is_err(),
            "head with only predicate and no arg should fail"
        );
    }

    #[test]
    fn test_evaluate_rule_empty_body_returns_empty_derived() {
        // Line 225 TRUE: patterns.is_empty() && expr_clauses.is_empty() → return Ok(derived)
        // Achieved by a rule with an empty body (no clauses)
        use crate::query::datalog::types::Rule;

        let storage = FactStorage::new();
        let mut registry = RuleRegistry::new();
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        // Rule with empty body: no patterns, no expr_clauses
        let rule = Rule {
            head: vec![
                EdnValue::Symbol("empty-body-pred".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            body: vec![], // empty body
        };
        registry.register_rule_unchecked("empty-body-pred".to_string(), rule);
        let rules = Arc::new(RwLock::new(registry));
        let evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        let result = evaluator.evaluate_recursive_rules(&["empty-body-pred".to_string()]);
        // Should succeed (returns base facts only, derives nothing)
        assert!(result.is_ok(), "empty body rule should not fail");
    }

    #[test]
    fn test_evaluate_rule_expr_only_body() {
        // Line 230 TRUE: patterns.is_empty() but expr_clauses not empty → seed with empty binding
        // Achieved by a rule with only Expr clauses in body
        use crate::query::datalog::types::{Expr, Rule, WhereClause};

        let storage = FactStorage::new();
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![(e1, ":item/value".to_string(), Value::Integer(42))],
                None,
            )
            .unwrap();

        // Rule with only an Expr that always evaluates to truthy
        // Since there are no patterns, it seeds with ONE empty binding
        // The Expr filters it (or not), then instantiate_head would fail if ?x is unbound
        // Let's just test that evaluate_rule handles this path without crashing
        let mut registry = RuleRegistry::new();
        let rule = Rule {
            head: vec![
                EdnValue::Symbol("expr-only-pred".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            body: vec![WhereClause::Expr {
                expr: Expr::Lit(Value::Boolean(false)), // always false → no bindings pass
                binding: None,
            }],
        };
        registry.register_rule_unchecked("expr-only-pred".to_string(), rule);
        let rules = Arc::new(RwLock::new(registry));
        let evaluator = RecursiveEvaluator::new(
            storage,
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        // This exercises the expr-only path. The expr evaluates to false → no facts derived.
        let result = evaluator.evaluate_recursive_rules(&["expr-only-pred".to_string()]);
        assert!(result.is_ok(), "expr-only rule body should not fail");
        let derived = result.unwrap();
        let facts = derived.get_asserted_facts().unwrap();
        let pred_facts: Vec<_> = facts
            .iter()
            .filter(|f| f.attribute == ":expr-only-pred")
            .collect();
        assert_eq!(
            pred_facts.len(),
            0,
            "expr evaluating to false should derive no facts"
        );
    }

    #[test]
    fn test_evaluate_not_join_expr_only_body() {
        // Line 448: evaluate_not_join with substituted.is_empty() (Expr-only not-join body)
        // Uses an Expr-only clause in the not-join body to hit the "seed with partial binding" path
        use crate::graph::types::Value;
        use crate::query::datalog::types::{BinOp, Expr, WhereClause};

        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(vec![(e1, ":score".to_string(), Value::Integer(100))], None)
            .unwrap();

        let facts: Arc<[crate::graph::types::Fact]> =
            Arc::from(storage.get_asserted_facts().unwrap().as_slice());

        // Build a not-join with an Expr-only body: (not-join [?x] [(> ?x 50)])
        // join_vars = ["?x"], clause = Expr that evaluates to true (100 > 50)
        // → substituted is empty → seeds with partial binding → not_bindings has 1 entry
        // → expr evaluates to true (100 > 50) → not_bindings not empty → returns true (reject)
        let mut binding = std::collections::HashMap::new();
        binding.insert("?x".to_string(), Value::Integer(100));

        let expr_clause = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Gt,
                Box::new(Expr::Var("?x".to_string())),
                Box::new(Expr::Lit(Value::Integer(50))),
            ),
            binding: None,
        };

        let result = evaluate_not_join(
            &["?x".to_string()],
            &[expr_clause],
            &binding,
            facts,
            &FunctionRegistry::with_builtins(),
        );
        assert!(
            result,
            "not-join with expr (100 > 50) should return true (reject)"
        );
    }

    #[test]
    fn test_stratified_evaluator_empty_stratum_skipped() {
        // Line 580: stratum_preds.is_empty() → continue
        // This is hit when a predicate has a stratum but no rules match
        // We can trigger it by evaluating predicates that exist in strata but have no rules
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        storage
            .transact(
                vec![(e1, ":x".to_string(), crate::graph::types::Value::Integer(1))],
                None,
            )
            .unwrap();

        let rules = Arc::new(RwLock::new(RuleRegistry::new()));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        // Register a rule for pred "a" — "b" has no rule, creating a gap in strata
        register_test_rule(&rules, r#"(rule [(a ?x) [?x :x ?v]])"#);

        let evaluator = StratifiedEvaluator::new(
            storage.clone(),
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        // Ask for both "a" and "nonexistent" — nonexistent has no rules → stratum_preds may be empty
        let result = evaluator.evaluate(&["a".to_string(), "nonexistent".to_string()]);
        assert!(
            result.is_ok(),
            "evaluation with missing predicate should succeed"
        );
    }

    #[test]
    fn test_stratified_evaluator_not_body_empty_patterns() {
        // Line 750: in the mixed-rule loop, not-body with substituted.is_empty()
        // (Expr-only not body in a stratified mixed rule)
        // This exercises the path where not-body has no patterns → seed with current binding
        let storage = FactStorage::new();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        storage
            .transact(
                vec![
                    (
                        e1,
                        ":val".to_string(),
                        crate::graph::types::Value::Integer(200),
                    ),
                    (
                        e2,
                        ":val".to_string(),
                        crate::graph::types::Value::Integer(50),
                    ),
                ],
                None,
            )
            .unwrap();

        // Rule with not + Expr-only not body: (big ?x) <- [?x :val ?v] (not [(< ?v 100)])
        // This rule uses not with an expr-only body: the not body has no Pattern clauses
        use crate::graph::types::Value;
        use crate::query::datalog::types::{BinOp, Expr, Pattern, Rule, WhereClause};

        let mut registry = RuleRegistry::new();
        let rule = Rule {
            head: vec![
                EdnValue::Symbol("big".to_string()),
                EdnValue::Symbol("?x".to_string()),
            ],
            body: vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":val".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                )),
                WhereClause::Not(vec![WhereClause::Expr {
                    expr: Expr::BinOp(
                        BinOp::Lt,
                        Box::new(Expr::Var("?v".to_string())),
                        Box::new(Expr::Lit(Value::Integer(100))),
                    ),
                    binding: None,
                }]),
            ],
        };
        registry.register_rule_unchecked("big".to_string(), rule);
        let rules = Arc::new(RwLock::new(registry));
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let evaluator = StratifiedEvaluator::new(
            storage.clone(),
            rules,
            functions,
            1000,
            DEFAULT_MAX_DERIVED_FACTS,
            DEFAULT_MAX_RESULTS,
        );
        let result = evaluator.evaluate(&["big".to_string()]);
        assert!(
            result.is_ok(),
            "stratified evaluation with expr-only not body should succeed"
        );
        let derived = result.unwrap();
        let facts = derived.get_asserted_facts().unwrap();
        let big_facts: Vec<_> = facts.iter().filter(|f| f.attribute == ":big").collect();
        // Only e1 with val=200 should be derived (e2 with val=50 is excluded by not [(< ?v 100)])
        assert_eq!(
            big_facts.len(),
            1,
            "only entity with val=200 should be 'big'"
        );
    }

    #[test]
    fn test_stratified_max_results_limit() {
        let storage = create_test_storage();
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        register_test_rule(&rules, "(rule [(all ?x) [?x :connected _]])");

        let evaluator = StratifiedEvaluator::new(
            storage,
            rules,
            functions,
            100,
            DEFAULT_MAX_DERIVED_FACTS,
            1, // very low max_results
        );

        let result = evaluator.evaluate(&["all".to_string()]);
        assert!(
            result.is_err(),
            "stratified should error when max_results exceeded"
        );
    }

    #[test]
    fn test_stratified_max_derived_facts_limit() {
        let storage = create_test_storage();
        let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        register_test_rule(&rules, "(rule [(all ?x) [?x :connected _]])");

        let evaluator = StratifiedEvaluator::new(
            storage,
            rules,
            functions,
            100,
            1, // very low max_derived_facts
            DEFAULT_MAX_RESULTS,
        );

        let result = evaluator.evaluate(&["all".to_string()]);
        assert!(
            result.is_err(),
            "stratified should error when max_derived_facts exceeded"
        );
    }
}
