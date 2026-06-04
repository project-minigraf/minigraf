use crate::graph::types::{EntityId, Value};
use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{DatalogQuery, EdnValue, Rule, WhereClause};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

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
            if let Some(arg0) = args.first() {
                if let Ok(entity) = edn_to_entity_id(arg0) {
                    seeds.push((entity, magic_attr, Value::Boolean(true)));
                }
            }
        } else if adornment.get(1) == Some(&'b') {
            // arg1-only bound (fb) — ephemeral carrier UUID
            if let Some(arg1) = args.get(1) {
                if let Ok(value) = edn_to_value(arg1) {
                    seeds.push((Uuid::new_v4(), magic_attr, value));
                }
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

    let magic_name = magic_pred_name(predicate, adornment);
    let bound_head_arg = rule
        .head
        .get(1)
        .cloned()
        .unwrap_or(EdnValue::Symbol("?_".to_string()));
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: vec![bound_head_arg],
    };

    let mut new_body = Vec::with_capacity(rule.body.len() + 1);
    new_body.push(guard);
    new_body.extend(rule.body.iter().cloned());

    Rule { head: rule.head.clone(), body: new_body }
}

#[allow(dead_code)]
pub(crate) fn rewrite(
    _query: &DatalogQuery,
    _registry: &RuleRegistry,
) -> Option<(RuleRegistry, Vec<(EntityId, String, Value)>)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{FindSpec, Pattern, Rule, WhereClause};

    #[test]
    fn test_rewrite_empty_query_returns_none() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![],
        );
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

    #[allow(dead_code)]
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
        let clauses = vec![pat("?x", ":name", "Alice"), rule_inv("ancestor", &["?x", "?y"])];
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
        let expected = crate::query::datalog::matcher::edn_to_entity_id(
            &EdnValue::Keyword(":alice".to_string()),
        )
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
            vec![pat("?a", ":parent", "?b"), rule_inv("ancestor", &["?b", "?c"])],
        );
        let adornment = vec!['b', 'f'];
        let rewritten = inject_magic_guard(&rule, "ancestor", &adornment);
        match rewritten.body.first().unwrap() {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, &magic_pred_name("ancestor", &adornment));
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], EdnValue::Symbol("?a".to_string()));
            }
            other => panic!("expected RuleInvocation guard, got {:?}", other),
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
}
