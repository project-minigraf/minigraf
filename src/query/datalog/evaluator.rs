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
use super::matcher::{Bindings, PatternMatcher, edn_to_entity_id, edn_to_value};
use super::rules::RuleRegistry;
use super::types::{EdnValue, Pattern, Rule, WhereClause};
use crate::graph::FactStorage;
use crate::graph::types::{Fact, Value};
use anyhow::{Result, anyhow};
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
            derived.transact(
                vec![(fact.entity, fact.attribute.clone(), fact.value.clone())],
                None,
            )?;
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
                    // TODO: phase-7-3 — or/or-join in rules not yet supported
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
        let bindings = apply_expr_clauses_in_evaluator(bindings, &expr_clauses);

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
    Pattern::new(
        substitute_value(&pattern.entity, binding),
        substitute_value(&pattern.attribute, binding),
        substitute_value(&pattern.value, binding),
    )
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
pub(super) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
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
    storage: &FactStorage,
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

    let matcher = PatternMatcher::new(storage.clone());
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
    not_bindings = apply_expr_clauses_in_evaluator(not_bindings, &expr_clauses);
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
) -> Vec<Bindings> {
    use crate::query::datalog::executor::{eval_expr, is_truthy};
    bindings
        .into_iter()
        .filter_map(|mut b| {
            for clause in expr_clauses {
                if let WhereClause::Expr { expr, binding: out } = clause {
                    match eval_expr(expr, &b) {
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
    max_iterations: usize,
}

impl StratifiedEvaluator {
    pub fn new(
        storage: FactStorage,
        rules: Arc<RwLock<RuleRegistry>>,
        max_iterations: usize,
    ) -> Self {
        StratifiedEvaluator {
            storage,
            rules,
            max_iterations,
        }
    }

    /// Derive all facts for the given predicates, respecting stratification order.
    pub fn evaluate(&self, predicates: &[String]) -> Result<FactStorage> {
        use crate::query::datalog::stratification::DependencyGraph;

        let registry = self.rules.read().unwrap();

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
            let registry = self.rules.read().unwrap();
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
                    let has_not = rule
                        .body
                        .iter()
                        .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }));
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
                let sub_eval =
                    RecursiveEvaluator::new(accumulated.clone(), sub_rules, self.max_iterations);
                let derived = sub_eval.evaluate_recursive_rules(&stratum_preds)?;
                // Snapshot existing fact keys so we only load truly new (derived) facts
                let existing: Vec<(uuid::Uuid, String, Value)> = accumulated
                    .get_asserted_facts()?
                    .into_iter()
                    .map(|f| (f.entity, f.attribute, f.value))
                    .collect();
                for fact in derived.get_asserted_facts()? {
                    let key = (fact.entity, fact.attribute.clone(), fact.value.clone());
                    if !existing.iter().any(|e| e == &key) {
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
                        WhereClause::Or(_) | WhereClause::OrJoin { .. } => None, // TODO: phase-7-3
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

                let matcher = PatternMatcher::new(accumulated.clone());
                let raw_candidates = matcher.match_patterns(&positive_patterns);

                // Apply top-level Expr clauses to filter/extend candidates
                let candidates =
                    apply_expr_clauses_in_evaluator(raw_candidates, &body_expr_clauses);

                // Build temp_eval once per rule (outside the binding loop);
                // instantiate_head_public only uses storage, not the registry.
                let temp_eval =
                    RecursiveEvaluator::new(accumulated.clone(), Arc::clone(&self.rules), 1);

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
                                WhereClause::Or(_) | WhereClause::OrJoin { .. } => None, // TODO: phase-7-3
                            })
                            .collect();

                        let not_matcher = PatternMatcher::new(accumulated.clone());
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
                        not_bindings =
                            apply_expr_clauses_in_evaluator(not_bindings, &not_body_expr_clauses);
                        if !not_bindings.is_empty() {
                            continue 'binding; // not condition violated -> discard binding
                        }
                    }

                    for (join_vars, nj_clauses) in &not_join_clauses {
                        if evaluate_not_join(join_vars, nj_clauses, &binding, &accumulated) {
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
            .transact(
                vec![
                    (n1, ":connected".to_string(), Value::Ref(n2)),
                    (n2, ":connected".to_string(), Value::Ref(n3)),
                    (n3, ":connected".to_string(), Value::Ref(n4)),
                    (n4, ":connected".to_string(), Value::Ref(n5)),
                ],
                None,
            )
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
            .transact(
                vec![
                    (a, ":connected".to_string(), Value::Ref(b)),
                    (b, ":connected".to_string(), Value::Ref(c)),
                    (c, ":connected".to_string(), Value::Ref(a)),
                ],
                None,
            )
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
    fn test_evaluate_rule_with_where_clause_body() {
        // Build a rule: (reachable ?x ?y) :- [?x :connected ?y]
        // using Vec<WhereClause> body (post-migration shape)
        use crate::query::datalog::types::{Pattern, WhereClause};
        let storage = FactStorage::new();
        storage
            .transact(
                vec![(
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                    ":connected".to_string(),
                    crate::graph::types::Value::Ref(
                        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                    ),
                )],
                None,
            )
            .unwrap();

        let rule = Rule {
            head: vec![
                EdnValue::Symbol("reachable".to_string()),
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ],
            body: vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?x".to_string()),
                EdnValue::Keyword(":connected".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ))],
        };

        let registry = Arc::new(RwLock::new(RuleRegistry::new()));
        // Register the rule so evaluate_recursive_rules actually calls evaluate_rule
        registry
            .write()
            .unwrap()
            .register_rule("reachable".to_string(), rule)
            .unwrap();

        let evaluator = RecursiveEvaluator::new(storage, registry, 10);
        let derived = evaluator
            .evaluate_recursive_rules(&["reachable".to_string()])
            .unwrap();

        // The rule [?x :connected ?y] -> (reachable ?x ?y) should derive a :reachable fact
        // entity 1 has :connected ref(entity 2), so (reachable entity1 entity2) should be derived
        let entity1 = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let reachable_facts = derived.get_facts_by_entity(&entity1).unwrap();
        assert!(
            reachable_facts.iter().any(|f| f.attribute == ":reachable"),
            "Expected :reachable fact to be derived from :connected base fact"
        );
    }

    #[test]
    fn test_recursive_convergence_iterations() {
        let storage = FactStorage::new();

        // Simple chain: A->B
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        storage
            .transact(vec![(a, ":connected".to_string(), Value::Ref(b))], None)
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

    mod stratified_tests {
        use super::*;
        use crate::graph::types::Value;
        use crate::query::datalog::types::{Pattern, WhereClause};
        use uuid::Uuid;

        fn alice() -> Uuid {
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
        }
        fn bob() -> Uuid {
            Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()
        }

        #[test]
        fn test_stratified_no_negation_same_as_recursive() {
            // StratifiedEvaluator with only positive rules must produce the same result
            // as RecursiveEvaluator.
            let storage = FactStorage::new();
            storage
                .transact(
                    vec![(alice(), ":connected".to_string(), Value::Ref(bob()))],
                    None,
                )
                .unwrap();

            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("reachable".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Symbol("?y".to_string()),
                ],
                body: vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":connected".to_string()),
                    EdnValue::Symbol("?y".to_string()),
                ))],
            };
            let mut registry = RuleRegistry::new();
            registry
                .register_rule("reachable".to_string(), rule)
                .unwrap();
            let rules = Arc::new(RwLock::new(registry));

            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["reachable".to_string()]).unwrap();
            let reachable_facts: Vec<_> = result
                .get_facts_by_attribute(&":reachable".to_string())
                .unwrap()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(reachable_facts.len(), 1);
        }

        #[test]
        fn test_not_filter_removes_binding_when_body_satisfied() {
            // eligible :- [?x :applied true], not([?x :rejected true])
            // alice applied=true, rejected=true -> NOT eligible
            let storage = FactStorage::new();
            storage
                .transact(
                    vec![
                        (alice(), ":applied".to_string(), Value::Boolean(true)),
                        (alice(), ":rejected".to_string(), Value::Boolean(true)),
                    ],
                    None,
                )
                .unwrap();

            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("eligible".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                body: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":applied".to_string()),
                        EdnValue::Boolean(true),
                    )),
                    WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":rejected".to_string()),
                        EdnValue::Boolean(true),
                    ))]),
                ],
            };
            let mut registry = RuleRegistry::new();
            registry
                .register_rule("eligible".to_string(), rule)
                .unwrap();
            let rules = Arc::new(RwLock::new(registry));

            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["eligible".to_string()]).unwrap();
            let eligible_facts: Vec<_> = result
                .get_facts_by_attribute(&":eligible".to_string())
                .unwrap()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(eligible_facts.len(), 0, "alice should NOT be eligible");
        }

        #[test]
        fn test_not_filter_keeps_binding_when_body_not_satisfied() {
            // eligible :- [?x :applied true], not([?x :rejected true])
            // alice applied=true only -> eligible
            let storage = FactStorage::new();
            storage
                .transact(
                    vec![(alice(), ":applied".to_string(), Value::Boolean(true))],
                    None,
                )
                .unwrap();

            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("eligible".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                body: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":applied".to_string()),
                        EdnValue::Boolean(true),
                    )),
                    WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":rejected".to_string()),
                        EdnValue::Boolean(true),
                    ))]),
                ],
            };
            let mut registry = RuleRegistry::new();
            registry
                .register_rule("eligible".to_string(), rule)
                .unwrap();
            let rules = Arc::new(RwLock::new(registry));

            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["eligible".to_string()]).unwrap();
            let eligible_facts: Vec<_> = result
                .get_facts_by_attribute(&":eligible".to_string())
                .unwrap()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(eligible_facts.len(), 1, "alice should be eligible");
        }

        #[test]
        fn test_not_join_rejects_entity_with_matching_inner_var() {
            // Rule: (clean ?x) :- [?x :submitted true], (not-join [?x] [?x :has-dep ?d] [?d :blocked true])
            // alice: submitted=true, has-dep=dep1, dep1:blocked=true  -> NOT clean
            // bob:   submitted=true                                    -> clean
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

            let rule = Rule::new(
                vec![
                    EdnValue::Symbol("clean".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
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

            let mut registry = RuleRegistry::new();
            registry.register_rule_unchecked("clean".to_string(), rule);
            let rules = Arc::new(RwLock::new(registry));
            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
            let clean_facts: Vec<_> = result
                .get_facts_by_attribute(&":clean".to_string())
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(clean_facts.len(), 1, "only bob should be clean");
            assert_eq!(clean_facts[0].entity, bob, "the clean entity must be bob");
        }

        #[test]
        fn test_not_join_keeps_entity_when_inner_var_has_no_match() {
            // Only alice has submitted=true and NO has-dep at all -> clean
            let storage = FactStorage::new();
            let alice = Uuid::new_v4();
            storage
                .transact(
                    vec![(alice, ":submitted".to_string(), Value::Boolean(true))],
                    None,
                )
                .unwrap();

            let rule = Rule::new(
                vec![
                    EdnValue::Symbol("clean".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":submitted".to_string()),
                        EdnValue::Boolean(true),
                    )),
                    WhereClause::NotJoin {
                        join_vars: vec!["?x".to_string()],
                        clauses: vec![WhereClause::Pattern(Pattern::new(
                            EdnValue::Symbol("?x".to_string()),
                            EdnValue::Keyword(":has-dep".to_string()),
                            EdnValue::Symbol("?d".to_string()),
                        ))],
                    },
                ],
            );

            let mut registry = RuleRegistry::new();
            registry.register_rule_unchecked("clean".to_string(), rule);
            let rules = Arc::new(RwLock::new(registry));
            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
            let clean_facts: Vec<_> = result
                .get_facts_by_attribute(&":clean".to_string())
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(
                clean_facts.len(),
                1,
                "alice must be clean when no deps exist"
            );
        }

        #[test]
        fn test_not_join_body_with_rule_invocation() {
            // Rule: (blocked ?x) :- [?x :status :banned]
            // Rule: (clean ?x) :- [?x :submitted true], (not-join [?x] (blocked ?x))
            // alice: submitted, banned -> NOT clean
            // bob: submitted, not banned -> clean
            let storage = FactStorage::new();
            let alice = Uuid::new_v4();
            let bob = Uuid::new_v4();
            storage
                .transact(
                    vec![
                        (alice, ":submitted".to_string(), Value::Boolean(true)),
                        (
                            alice,
                            ":status".to_string(),
                            Value::Keyword(":banned".to_string()),
                        ),
                        (bob, ":submitted".to_string(), Value::Boolean(true)),
                    ],
                    None,
                )
                .unwrap();

            let rule_blocked = Rule::new(
                vec![
                    EdnValue::Symbol("blocked".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                vec![WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?x".to_string()),
                    EdnValue::Keyword(":status".to_string()),
                    EdnValue::Keyword(":banned".to_string()),
                ))],
            );
            let rule_clean = Rule::new(
                vec![
                    EdnValue::Symbol("clean".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":submitted".to_string()),
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
            registry.register_rule_unchecked("blocked".to_string(), rule_blocked);
            registry.register_rule_unchecked("clean".to_string(), rule_clean);
            let rules = Arc::new(RwLock::new(registry));
            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["clean".to_string()]).unwrap();
            let clean_facts: Vec<_> = result
                .get_facts_by_attribute(&":clean".to_string())
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(clean_facts.len(), 1, "only bob should be clean");
            assert_eq!(clean_facts[0].entity, bob, "the clean entity must be bob");
        }

        #[test]
        fn test_not_filter_with_multiple_not_clauses() {
            // eligible :- [?x :status "active"], not([?x :role "admin"]), not([?x :banned true])
            // alice: status="active"                         -> eligible (passes both not-clauses)
            // bob:   status="active", role="admin"           -> NOT eligible (fails first not-clause)
            let storage = FactStorage::new();
            storage
                .transact(
                    vec![
                        (
                            alice(),
                            ":status".to_string(),
                            Value::String("active".to_string()),
                        ),
                        (
                            bob(),
                            ":status".to_string(),
                            Value::String("active".to_string()),
                        ),
                        (
                            bob(),
                            ":role".to_string(),
                            Value::String("admin".to_string()),
                        ),
                    ],
                    None,
                )
                .unwrap();

            let rule = Rule {
                head: vec![
                    EdnValue::Symbol("eligible".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
                body: vec![
                    WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":status".to_string()),
                        EdnValue::String("active".to_string()),
                    )),
                    WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":role".to_string()),
                        EdnValue::String("admin".to_string()),
                    ))]),
                    WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
                        EdnValue::Symbol("?x".to_string()),
                        EdnValue::Keyword(":banned".to_string()),
                        EdnValue::Boolean(true),
                    ))]),
                ],
            };
            let mut registry = RuleRegistry::new();
            registry.register_rule_unchecked("eligible".to_string(), rule);
            let rules = Arc::new(RwLock::new(registry));

            let evaluator = StratifiedEvaluator::new(storage, rules, 100);
            let result = evaluator.evaluate(&["eligible".to_string()]).unwrap();
            let eligible_facts: Vec<_> = result
                .get_facts_by_attribute(&":eligible".to_string())
                .unwrap()
                .into_iter()
                .filter(|f| f.asserted)
                .collect();
            assert_eq!(eligible_facts.len(), 1, "only alice should be eligible");
            assert_eq!(
                eligible_facts[0].entity,
                alice(),
                "the eligible entity should be alice"
            );
        }
    }
}
