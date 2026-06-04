use crate::graph::types::{EntityId, Value};
use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{DatalogQuery, EdnValue, Rule, WhereClause};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Return type for `rewrite()`: rewritten rule registry + seed facts to preload.
type RewriteResult = (RuleRegistry, Vec<(EntityId, String, Value)>);

/// Classify each arg in rule invocations as bound ('b') or free ('f').
/// Single left-to-right pass; all variables in Pattern entity/value positions are
/// considered grounded after the pattern (Datalog: pattern binds all its variables).
pub(crate) fn compute_query_adornments(
    where_clauses: &[WhereClause],
) -> HashMap<String, Vec<char>> {
    let mut grounded: HashSet<String> = HashSet::new();
    let mut adornments: HashMap<String, Vec<char>> = HashMap::new();

    for clause in where_clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let Some(var) = p.entity.as_variable() {
                    grounded.insert(var.to_string());
                }
                if let Some(var) = p.value.as_variable() {
                    grounded.insert(var.to_string());
                }
            }
            WhereClause::RuleInvocation { predicate, args } => {
                let adornment: Vec<char> = args
                    .iter()
                    .map(|arg| {
                        if let Some(var) = arg.as_variable() {
                            if grounded.contains(var) { 'b' } else { 'f' }
                        } else {
                            'b' // literal
                        }
                    })
                    .collect();
                adornments.entry(predicate.clone()).or_insert(adornment);
            }
            _ => {}
        }
    }

    adornments
}

/// Returns true if at least one position in the adornment is bound.
#[allow(dead_code)]
pub(crate) fn has_bound_arg(adornment: &[char]) -> bool {
    adornment.contains(&'b')
}

/// Convert adornment to string: ['b','f'] → "bf".
#[allow(dead_code)]
pub(crate) fn adornment_string(adornment: &[char]) -> String {
    adornment.iter().collect()
}

/// Magic predicate name: "__magic_ancestor_bf".
#[allow(dead_code)]
pub(crate) fn magic_pred_name(pred: &str, adornment: &[char]) -> String {
    format!("__magic_{}_{}", pred, adornment_string(adornment))
}

/// Build seed facts for adorned rule invocations with at least one bound arg.
///
/// Encoding:
///   arg0 bound: entity = edn_to_entity_id(arg0), attr = ":__magic_p_ad", value = Boolean(true)
///   arg1-only bound (fb): entity = Uuid::new_v4() (ephemeral carrier), value = edn_to_value(arg1)
#[allow(dead_code)]
pub(crate) fn build_seed_facts(
    where_clauses: &[WhereClause],
    adornments: &HashMap<String, Vec<char>>,
) -> Vec<(EntityId, String, Value)> {
    let mut seeds = Vec::new();

    for clause in where_clauses {
        let WhereClause::RuleInvocation { predicate, args } = clause else {
            continue;
        };
        let Some(adornment) = adornments.get(predicate) else {
            continue;
        };
        if !has_bound_arg(adornment) {
            continue;
        }

        let magic_attr = format!(":{}", magic_pred_name(predicate, adornment));

        if adornment.first() == Some(&'b') {
            // arg0 bound — dominant case
            if let Some(arg0) = args.first()
                && let Ok(entity) = edn_to_entity_id(arg0)
            {
                seeds.push((entity, magic_attr, Value::Boolean(true)));
            }
        } else if adornment.get(1) == Some(&'b') {
            // arg1-only bound (fb) — ephemeral carrier UUID
            if let Some(arg1) = args.get(1)
                && let Ok(value) = edn_to_value(arg1)
            {
                seeds.push((Uuid::new_v4(), magic_attr, value));
            }
        }
    }

    seeds
}

/// Prepend a magic-predicate guard to a positive rule's body.
/// Rules containing Not/NotJoin are returned unchanged.
///
/// The guard uses head[1] (the entity-position variable) as its argument,
/// which is always the bound position for the dominant `bf` adornment.
#[allow(dead_code)]
pub(crate) fn inject_magic_guard(rule: &Rule, predicate: &str, adornment: &[char]) -> Rule {
    let has_negation = rule
        .body
        .iter()
        .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }));
    if has_negation {
        return rule.clone();
    }

    debug_assert_eq!(
        adornment.len(),
        rule.head.len().saturating_sub(1),
        "adornment length must equal rule arity"
    );

    let magic_name = magic_pred_name(predicate, adornment);
    let bound_head_args: Vec<EdnValue> = adornment
        .iter()
        .enumerate()
        .filter(|&(_, &ch)| ch == 'b')
        // rule.head[0] is the predicate name; args start at index 1
        .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
        .collect();
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: bound_head_args,
    };

    let mut new_body = Vec::with_capacity(rule.body.len() + 1);
    new_body.push(guard);
    new_body.extend(rule.body.iter().cloned());

    Rule {
        head: rule.head.clone(),
        body: new_body,
    }
}

/// Build magic propagation rules for adorned recursive calls within a rule body.
///
/// For each `RuleInvocation` in the body that calls an adorned predicate,
/// emits a rule: (magic-p-ad ?bound_arg) :- (magic-p-ad ?head_bound_var) <preceding non-invocation clauses>
///
/// Returns Vec<(predicate_name, Rule)> to be registered with `register_rule_unchecked`.
#[allow(dead_code)]
pub(crate) fn build_propagation_rules(
    rule: &Rule,
    predicate: &str,
    adorned: &HashMap<String, Vec<char>>,
) -> Vec<(String, Rule)> {
    if rule
        .body
        .iter()
        .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }))
    {
        return vec![];
    }

    let Some(adornment) = adorned.get(predicate) else {
        return vec![];
    };
    let magic_name = magic_pred_name(predicate, adornment);

    // Bound-position variables from the rule head (same logic as inject_magic_guard).
    debug_assert_eq!(
        adornment.len(),
        rule.head.len().saturating_sub(1),
        "adornment length must equal rule arity"
    );
    let bound_head_vars: Vec<EdnValue> = adornment
        .iter()
        .enumerate()
        .filter(|&(_, &ch)| ch == 'b')
        // rule.head[0] is the predicate name; args start at index 1
        .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
        .collect();

    // Guard clause reused in every propagation rule body.
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: bound_head_vars,
    };

    let mut result = Vec::new();

    for (i, clause) in rule.body.iter().enumerate() {
        let WhereClause::RuleInvocation {
            predicate: called_pred,
            args: called_args,
        } = clause
        else {
            continue;
        };

        // Only emit propagation for calls to an adorned predicate.
        let Some(called_adornment) = adorned.get(called_pred.as_str()) else {
            continue;
        };
        let called_magic_name = magic_pred_name(called_pred, called_adornment);

        // New magic head args = bound-position args of the recursive call.
        debug_assert_eq!(
            called_adornment.len(),
            called_args.len(),
            "called adornment length must equal called args length"
        );

        // Fix 2: Skip all-free adorned predicates — no bound args means no magic head.
        if !has_bound_arg(called_adornment) {
            continue;
        }

        let new_magic_args: Vec<EdnValue> = called_adornment
            .iter()
            .enumerate()
            .filter(|&(_, &ch)| ch == 'b')
            // Fix 3: Rename inner `i` to `pos` to avoid shadowing the outer loop variable.
            .filter_map(|(pos, _)| called_args.get(pos).cloned())
            .collect();

        // Propagation body = guard + all non-RuleInvocation clauses before this call.
        let mut prop_body = vec![guard.clone()];
        for preceding in rule.body.iter().take(i) {
            if !matches!(preceding, WhereClause::RuleInvocation { .. }) {
                prop_body.push(preceding.clone());
            }
        }

        // Fix 1: Head must include ALL bound args, not just the first.
        let mut head = Vec::with_capacity(1 + new_magic_args.len());
        head.push(EdnValue::Symbol(called_magic_name.clone()));
        head.extend(new_magic_args);
        result.push((
            called_magic_name,
            Rule {
                head,
                body: prop_body,
            },
        ));
    }

    result
}

/// Propagate adornments transitively through rule bodies (handles mutual recursion).
/// Starting from `initial`, expands until fixed point.
pub(crate) fn propagate_adornments(
    initial: &HashMap<String, Vec<char>>,
    registry: &RuleRegistry,
) -> HashMap<String, Vec<char>> {
    let mut adorned = initial.clone();
    let mut changed = true;

    while changed {
        changed = false;
        // Collect to avoid borrowing issues in the loop
        let preds_with_adornments: Vec<(String, Vec<char>)> = adorned
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        for (pred, adornment) in &preds_with_adornments {
            for rule in registry.get_rules(pred) {
                if rule
                    .body
                    .iter()
                    .any(|c| matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. }))
                {
                    continue;
                }
                // Seed grounded vars with bound-position vars from the rule head
                let mut grounded: HashSet<String> = HashSet::new();
                for (i, &ch) in adornment.iter().enumerate() {
                    if ch == 'b'
                        && let Some(v) = rule.head.get(i + 1).and_then(|e| e.as_variable())
                    {
                        grounded.insert(v.to_string());
                    }
                }
                for clause in &rule.body {
                    match clause {
                        WhereClause::Pattern(p) => {
                            if let Some(v) = p.entity.as_variable() {
                                grounded.insert(v.to_string());
                            }
                            if let Some(v) = p.value.as_variable() {
                                grounded.insert(v.to_string());
                            }
                        }
                        WhereClause::RuleInvocation {
                            predicate: called,
                            args,
                        } => {
                            let call_adornment: Vec<char> = args
                                .iter()
                                .map(|a| match a.as_variable() {
                                    Some(v) if grounded.contains(v) => 'b',
                                    Some(_) => 'f',
                                    None => 'b', // literals are always bound
                                })
                                .collect();
                            // First-writer-wins: if a predicate is already adorned from another call site,
                            // we keep the first adornment. In programs with multiple call sites for the same
                            // predicate, this may miss more-permissive adornments from later call sites.
                            // This is an acceptable simplification — worst case is less pruning, not wrong results.
                            if has_bound_arg(&call_adornment)
                                && !adorned.contains_key(called.as_str())
                            {
                                adorned.insert(called.clone(), call_adornment);
                                changed = true;
                            }
                            // After a RuleInvocation, the output vars become grounded
                            for a in args {
                                if let Some(v) = a.as_variable() {
                                    grounded.insert(v.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    adorned
}

/// Build rewritten registry: copy all rules with magic guards injected for adorned
/// predicates, then register magic propagation rules for all adorned predicates.
/// Uses `register_rule_unchecked` because magic rules are self-recursive (positive cycle).
fn build_rewritten_registry(
    registry: &RuleRegistry,
    adorned: &HashMap<String, Vec<char>>,
) -> RuleRegistry {
    let mut new_reg = RuleRegistry::new();

    // Copy existing rules, injecting magic guards where adorned.
    for (pred, rules) in registry.all_rules() {
        for rule in rules {
            let rewritten = if let Some(adornment) = adorned.get(pred) {
                inject_magic_guard(rule, pred, adornment)
            } else {
                rule.clone()
            };
            new_reg.register_rule_unchecked(pred.to_string(), rewritten);
        }
    }

    // Emit magic propagation rules for adorned predicates.
    for (pred, rules) in registry.all_rules() {
        if adorned.contains_key(pred) {
            for rule in rules {
                for (magic_pred, prop_rule) in build_propagation_rules(rule, pred, adorned) {
                    new_reg.register_rule_unchecked(magic_pred, prop_rule);
                }
            }
        }
    }

    new_reg
}

pub(crate) fn rewrite(query: &DatalogQuery, registry: &RuleRegistry) -> Option<RewriteResult> {
    let initial = compute_query_adornments(&query.where_clauses);
    let initial_bound: HashMap<String, Vec<char>> = initial
        .into_iter()
        .filter(|(_, ad)| has_bound_arg(ad))
        .collect();

    if initial_bound.is_empty() {
        return None;
    }

    // Fast-path: magic sets only helps when at least one adorned predicate has a free
    // position. Fully-bound adornments (e.g. "bb") provide no demand-driven benefit
    // and the current seed encoding only supports arg0-bound or arg1-only-bound cases.
    // NOTE: this guard is redundant with the downstream `seeds.is_empty()` check, but
    // is retained as an early exit that avoids the `propagate_adornments` traversal.
    let any_free = initial_bound.values().any(|ad| ad.contains(&'f'));
    if !any_free {
        return None;
    }

    let adorned = propagate_adornments(&initial_bound, registry);
    let seeds = build_seed_facts(&query.where_clauses, &adorned);
    if seeds.is_empty() {
        return None;
    }

    Some((build_rewritten_registry(registry, &adorned), seeds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::rules::RuleRegistry;
    use crate::query::datalog::types::{FindSpec, Pattern, Rule, WhereClause};

    #[test]
    fn test_rewrite_empty_query_returns_none() {
        let query = DatalogQuery::new(vec![FindSpec::Variable("?x".to_string())], vec![]);
        let registry = RuleRegistry::new();
        assert!(rewrite(&query, &registry).is_none());
    }

    fn pat(entity: &str, attr: &str, value: &str) -> WhereClause {
        WhereClause::Pattern(Pattern::new(
            if entity.starts_with('?') {
                EdnValue::Symbol(entity.to_string())
            } else {
                EdnValue::Keyword(entity.to_string())
            },
            EdnValue::Keyword(attr.to_string()),
            if value.starts_with('?') {
                EdnValue::Symbol(value.to_string())
            } else {
                EdnValue::String(value.to_string())
            },
        ))
    }

    fn rule_inv(pred: &str, args: &[&str]) -> WhereClause {
        WhereClause::RuleInvocation {
            predicate: pred.to_string(),
            args: args
                .iter()
                .map(|a| {
                    if a.starts_with('?') {
                        EdnValue::Symbol(a.to_string())
                    } else {
                        EdnValue::String(a.to_string())
                    }
                })
                .collect(),
        }
    }

    fn make_rule(pred: &str, head_args: &[&str], body: Vec<WhereClause>) -> Rule {
        Rule {
            head: std::iter::once(EdnValue::Symbol(pred.to_string()))
                .chain(head_args.iter().map(|a| EdnValue::Symbol(a.to_string())))
                .collect(),
            body,
        }
    }

    #[test]
    fn test_literal_arg_is_bound() {
        let clauses = vec![rule_inv("ancestor", &["abc123", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['b', 'f']));
    }

    #[test]
    fn test_free_var_is_free() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['f', 'f']));
    }

    #[test]
    fn test_var_grounded_by_preceding_pattern() {
        // [?x :name "Alice"] (ancestor ?x ?y) → ?x grounded → bf
        let clauses = vec![
            pat("?x", ":name", "Alice"),
            rule_inv("ancestor", &["?x", "?y"]),
        ];
        let adornments = compute_query_adornments(&clauses);
        assert_eq!(adornments.get("ancestor"), Some(&vec!['b', 'f']));
    }

    #[test]
    fn test_all_free_has_no_bound() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        let ad = adornments.get("ancestor").unwrap();
        assert!(!has_bound_arg(ad));
    }

    #[test]
    fn test_seed_fact_for_keyword_entity_arg() {
        // (ancestor :alice ?y) — arg0 bound (keyword alias)
        // seed: entity = edn_to_entity_id(:alice), attr = ":__magic_ancestor_bf", value = true
        let clauses = vec![WhereClause::RuleInvocation {
            predicate: "ancestor".to_string(),
            args: vec![
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::Symbol("?y".to_string()),
            ],
        }];
        let adornments = compute_query_adornments(&clauses);
        let seeds = build_seed_facts(&clauses, &adornments);
        assert_eq!(seeds.len(), 1);
        let (entity, attr, value) = &seeds[0];
        assert_eq!(attr, ":__magic_ancestor_bf");
        assert_eq!(value, &Value::Boolean(true));
        let expected = crate::query::datalog::matcher::edn_to_entity_id(&EdnValue::Keyword(
            ":alice".to_string(),
        ))
        .unwrap();
        assert_eq!(*entity, expected);
    }

    #[test]
    fn test_no_seed_for_all_free() {
        let clauses = vec![rule_inv("ancestor", &["?x", "?y"])];
        let adornments = compute_query_adornments(&clauses);
        let seeds = build_seed_facts(&clauses, &adornments);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_magic_guard_prepended_to_positive_rule() {
        // (ancestor ?a ?c) :- [?a :parent ?b] (ancestor ?b ?c)
        // adornment bf → guard (__magic_ancestor_bf ?a) prepended
        let rule = make_rule(
            "ancestor",
            &["?a", "?c"],
            vec![
                pat("?a", ":parent", "?b"),
                rule_inv("ancestor", &["?b", "?c"]),
            ],
        );
        let adornment = vec!['b', 'f'];
        let rewritten = inject_magic_guard(&rule, "ancestor", &adornment);
        match rewritten.body.first().unwrap() {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, &magic_pred_name("ancestor", &adornment));
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
            }
            _ => panic!("expected RuleInvocation guard"),
        }
    }

    #[test]
    fn test_magic_guard_bb_adornment() {
        // Rule: (reachable ?a ?b) :- [?a :edge/to ?b]
        // adornment bb — both args bound → guard gets both args
        let rule = make_rule(
            "reachable",
            &["?a", "?b"],
            vec![pat("?a", ":edge/to", "?b")],
        );
        let ad = vec!['b', 'b'];
        let result = inject_magic_guard(&rule, "reachable", &ad);
        let guard = result
            .body
            .first()
            .expect("guard should be first body clause");
        match guard {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, "__magic_reachable_bb");
                assert_eq!(args.len(), 2, "bb adornment should produce 2 guard args");
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
                assert_eq!(args[1], EdnValue::Symbol("?b".to_string()));
            }
            _ => panic!("expected RuleInvocation guard"),
        }
    }

    #[test]
    fn test_mixed_rule_not_touched_by_guard() {
        let rule = make_rule(
            "eligible",
            &["?x"],
            vec![
                pat("?x", ":applied", "true"),
                WhereClause::Not(vec![pat("?x", ":rejected", "true")]),
            ],
        );
        let rewritten = inject_magic_guard(&rule, "eligible", &['b']);
        // First body clause must still be Pattern (not an injected guard)
        assert!(
            matches!(rewritten.body.first().unwrap(), WhereClause::Pattern(_)),
            "mixed rule must not have guard injected"
        );
    }

    #[test]
    fn test_propagation_rule_emitted_for_recursive_call() {
        // (ancestor ?a ?c) :- [?a :parent ?b] (ancestor ?b ?c)
        // bf → (__magic_ancestor_bf ?b) :- (__magic_ancestor_bf ?a) [?a :parent ?b]
        let rule = make_rule(
            "ancestor",
            &["?a", "?c"],
            vec![
                pat("?a", ":parent", "?b"),
                rule_inv("ancestor", &["?b", "?c"]),
            ],
        );
        let adorned: HashMap<String, Vec<char>> = [("ancestor".to_string(), vec!['b', 'f'])]
            .into_iter()
            .collect();
        let prop_rules = build_propagation_rules(&rule, "ancestor", &adorned);
        assert_eq!(prop_rules.len(), 1);

        let (pred, prop) = &prop_rules[0];
        assert_eq!(
            pred.as_str(),
            magic_pred_name("ancestor", &['b', 'f']).as_str()
        );
        // Head: [Symbol("__magic_ancestor_bf"), Symbol("?b")]
        assert_eq!(prop.head.len(), 2);
        assert_eq!(prop.head[1], EdnValue::Symbol("?b".to_string()));
        // Body: [magic_guard(?a), [?a :parent ?b]]
        assert_eq!(prop.body.len(), 2);
        match &prop.body[0] {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, &magic_pred_name("ancestor", &['b', 'f']));
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
            }
            _ => panic!("expected magic guard in propagation body"),
        }
    }

    #[test]
    fn test_propagation_rule_bb_adornment() {
        // (reachable ?a ?b) :- (reachable ?a ?c) [?c :edge/to ?b]
        // bb → propagation rule head must have BOTH bound args
        let rule = make_rule(
            "reachable",
            &["?a", "?b"],
            vec![
                rule_inv("reachable", &["?a", "?c"]),
                pat("?c", ":edge/to", "?b"),
            ],
        );
        let adorned: HashMap<String, Vec<char>> = [("reachable".to_string(), vec!['b', 'b'])]
            .into_iter()
            .collect();
        let prop_rules = build_propagation_rules(&rule, "reachable", &adorned);
        assert_eq!(prop_rules.len(), 1);
        let (_, prop) = &prop_rules[0];
        // Head must have predicate name + 2 args for bb
        assert_eq!(
            prop.head.len(),
            3,
            "bb propagation rule head must have 3 elements (name + 2 args)"
        );
        assert_eq!(prop.head[1], EdnValue::Symbol("?a".to_string()));
        assert_eq!(prop.head[2], EdnValue::Symbol("?c".to_string()));
    }

    #[test]
    fn test_no_propagation_for_non_recursive_rule() {
        // (ancestor ?a ?b) :- [?a :parent ?b]  — no recursive call
        let rule = make_rule("ancestor", &["?a", "?b"], vec![pat("?a", ":parent", "?b")]);
        let adorned: HashMap<String, Vec<char>> = [("ancestor".to_string(), vec!['b', 'f'])]
            .into_iter()
            .collect();
        let prop_rules = build_propagation_rules(&rule, "ancestor", &adorned);
        assert!(prop_rules.is_empty());
    }

    #[test]
    fn test_scc_peer_adornment_propagates() {
        // even ?n :- (odd ?n)
        // odd  ?n :- (even ?n)
        // Initial: even adorned 'b' → odd should also be adorned via propagation
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "even".to_string(),
            make_rule("even", &["?n"], vec![rule_inv("odd", &["?n"])]),
        );
        registry.register_rule_unchecked(
            "odd".to_string(),
            make_rule("odd", &["?n"], vec![rule_inv("even", &["?n"])]),
        );
        let initial: HashMap<String, Vec<char>> =
            [("even".to_string(), vec!['b'])].into_iter().collect();
        let propagated = propagate_adornments(&initial, &registry);
        assert!(
            propagated.contains_key("odd"),
            "odd should be adorned via SCC"
        );
    }

    #[test]
    fn test_rewrite_none_when_all_free() {
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule("reach", &["?a", "?b"], vec![pat("?a", ":edge", "?b")]),
        );
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![rule_inv("reach", &["?a", "?b"])],
        );
        assert!(rewrite(&query, &registry).is_none());
    }

    #[test]
    fn test_rewrite_some_when_bound_arg() {
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule("reach", &["?a", "?b"], vec![pat("?a", ":edge", "?b")]),
        );
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule(
                "reach",
                &["?a", "?c"],
                vec![rule_inv("reach", &["?a", "?b"]), pat("?b", ":edge", "?c")],
            ),
        );
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![WhereClause::RuleInvocation {
                predicate: "reach".to_string(),
                args: vec![
                    EdnValue::Keyword(":start".to_string()),
                    EdnValue::Symbol("?x".to_string()),
                ],
            }],
        );
        let result = rewrite(&query, &registry);
        assert!(result.is_some(), "should rewrite when arg0 is literal");
        let (rewritten_reg, seeds) = result.unwrap();
        assert!(!seeds.is_empty(), "should produce seed facts");
        let magic_name = magic_pred_name("reach", &['b', 'f']);
        assert!(
            !rewritten_reg.get_rules(&magic_name).is_empty(),
            "magic propagation rules should be registered"
        );
    }

    #[test]
    fn test_rewritten_registry_has_magic_guard_in_rules() {
        let mut registry = RuleRegistry::new();
        registry.register_rule_unchecked(
            "reach".to_string(),
            make_rule("reach", &["?a", "?b"], vec![pat("?a", ":edge", "?b")]),
        );
        let adorned: HashMap<String, Vec<char>> = [("reach".to_string(), vec!['b', 'f'])]
            .into_iter()
            .collect();
        let new_reg = build_rewritten_registry(&registry, &adorned);
        let rules = new_reg.get_rules("reach");
        assert!(
            !rules.is_empty(),
            "rewritten registry should contain reach rules"
        );
        let first_body = rules[0].body.first().expect("rule should have body");
        assert!(
            matches!(first_body, WhereClause::RuleInvocation { .. }),
            "first body clause of adorned rule should be magic guard"
        );
    }
}
