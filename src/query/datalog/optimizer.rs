//! Query optimizer: index selection and join ordering for Datalog patterns.
//!
//! `plan()` is the single entry point. It assigns an `IndexHint` to each
//! pattern and (outside the `wasm` feature) sorts patterns by selectivity.

use crate::query::datalog::types::{AttributeSpec, EdnValue, Expr, Pattern, WhereClause};

/// Which covering index to use for a given pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexHint {
    /// EAVT: entity-first scan. Also used when nothing is bound (full scan).
    Eavt,
    /// AEVT: attribute-first scan.
    Aevt,
    /// AVET: attribute + value equality / range lookup.
    Avet,
    /// VAET: reverse reference lookup (Ref value only, no attribute).
    Vaet,
}

/// Return true if the component is a logic variable (unbound).
fn is_variable(v: &EdnValue) -> bool {
    v.is_variable()
}

/// Return true if the value component is a bound entity literal (UUID/Ref).
fn is_entity_literal(v: &EdnValue) -> bool {
    matches!(v, EdnValue::Uuid(_))
}

/// Return true if the attribute is a bound (non-variable) real attribute.
/// Pseudo-attributes are never index-bound (they are not stored attributes).
fn attr_is_index_bound(a: &AttributeSpec) -> bool {
    match a {
        AttributeSpec::Real(edn) => !is_variable(edn),
        AttributeSpec::Pseudo(_) => false,
    }
}

/// Count the number of non-variable components in a pattern.
/// Higher score = more selective.
#[cfg(not(feature = "wasm"))]
fn selectivity_score(p: &Pattern) -> u8 {
    let e = !is_variable(&p.entity);
    let a = attr_is_index_bound(&p.attribute);
    let v = !is_variable(&p.value);
    (e as u8).saturating_add(a as u8).saturating_add(v as u8)
}

/// Select the most efficient index for a single pattern.
///
/// Selection table:
///   Entity bound (± anything)         → EAVT
///   Attribute + Value (any non-Var)    → AVET
///   Attribute only                     → AEVT
///   Value is entity literal, no attr   → VAET (reverse traversal)
///   Nothing bound                      → EAVT (full scan)
pub fn select_index(p: &Pattern) -> IndexHint {
    let e_bound = !is_variable(&p.entity);
    let a_bound = attr_is_index_bound(&p.attribute);
    let v_bound = !is_variable(&p.value);

    if e_bound {
        return IndexHint::Eavt;
    }
    if a_bound && v_bound {
        return IndexHint::Avet;
    }
    if a_bound {
        return IndexHint::Aevt;
    }
    if v_bound && is_entity_literal(&p.value) {
        return IndexHint::Vaet;
    }
    // Nothing bound: full scan through EAVT
    IndexHint::Eavt
}

/// Collect all logic-variable names (`?foo`) referenced in an Expr tree.
fn expr_vars(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::Var(s) => vec![s.clone()],
        Expr::Lit(_) | Expr::Slot(_) => vec![],
        Expr::BinOp(_, l, r) => {
            let mut vars = expr_vars(l);
            vars.extend(expr_vars(r));
            vars
        }
        Expr::UnaryOp(_, inner) => expr_vars(inner),
    }
}

/// Collect the logic-variable names bound (output) by a Pattern.
/// Only Symbol values starting with `?` count — literals never bind.
fn pattern_bound_vars(p: &Pattern) -> Vec<String> {
    let mut vars = Vec::new();
    if is_variable(&p.entity)
        && let EdnValue::Symbol(s) = &p.entity
    {
        vars.push(s.clone());
    }
    if let AttributeSpec::Real(attr) = &p.attribute
        && is_variable(attr)
        && let EdnValue::Symbol(s) = attr
    {
        vars.push(s.clone());
    }
    if is_variable(&p.value)
        && let EdnValue::Symbol(s) = &p.value
    {
        vars.push(s.clone());
    }
    vars
}

/// Plan a list of where clauses: assign index hints to Pattern entries, push Expr
/// entries to the earliest position where all their variables are bound by preceding
/// patterns, and (non-wasm) sort patterns by selectivity.
///
/// Only `WhereClause::Pattern` and `WhereClause::Expr` variants should be passed in.
/// `Not`, `NotJoin`, `Or`, `OrJoin`, and `RuleInvocation` variants are handled by
/// the executor/evaluator and must not appear here.
///
/// Returns an interleaved `Vec<(WhereClause, Option<IndexHint>)>` where Pattern entries
/// carry `Some(hint)` and Expr entries carry `None`.
pub fn plan(
    clauses: Vec<WhereClause>,
    _indexes: &crate::storage::index::Indexes,
) -> Vec<(WhereClause, Option<IndexHint>)> {
    // Separate into patterns (with hints) and exprs.
    let mut patterns: Vec<(WhereClause, IndexHint)> = Vec::new();
    let mut exprs: Vec<WhereClause> = Vec::new();

    for clause in clauses {
        match &clause {
            WhereClause::Pattern(p) => {
                let hint = select_index(p);
                patterns.push((clause, hint));
            }
            WhereClause::Expr { .. } => exprs.push(clause),
            // Other variants must not be passed to plan(); silently skip.
            _ => {}
        }
    }

    // Stable sort patterns by selectivity descending (non-wasm only).
    // Preserves original order for ties, ensuring deterministic output.
    #[cfg(not(feature = "wasm"))]
    patterns.sort_by_key(|(clause, _)| {
        if let WhereClause::Pattern(p) = clause {
            std::cmp::Reverse(selectivity_score(p))
        } else {
            std::cmp::Reverse(0u8)
        }
    });

    // Start with sorted patterns only.
    let mut result: Vec<(WhereClause, Option<IndexHint>)> = patterns
        .into_iter()
        .map(|(clause, hint)| (clause, Some(hint)))
        .collect();

    // Push each Expr to the earliest position where all its variables are bound.
    for expr_clause in exprs {
        let vars: std::collections::HashSet<String> =
            if let WhereClause::Expr { expr, .. } = &expr_clause {
                expr_vars(expr).into_iter().collect()
            } else {
                Default::default()
            };

        let mut bound: std::collections::HashSet<String> = Default::default();
        // Default: append at end (covers no-var Exprs and vars never bound by any pattern).
        let mut insert_pos = result.len();

        if !vars.is_empty() {
            for (pos, (clause, _)) in result.iter().enumerate() {
                if let WhereClause::Pattern(p) = clause {
                    bound.extend(pattern_bound_vars(p));
                    if vars.is_subset(&bound) {
                        insert_pos = pos + 1;
                        break;
                    }
                }
            }
        }

        result.insert(insert_pos, (expr_clause, None));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::datalog::types::{BinOp, EdnValue, Expr, Pattern, WhereClause};
    use uuid::Uuid;

    fn make_pattern(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Pattern {
        Pattern::new(entity, attribute, value)
    }

    fn var(s: &str) -> EdnValue {
        EdnValue::Symbol(format!("?{s}"))
    }
    fn kw(s: &str) -> EdnValue {
        EdnValue::Keyword(s.to_string())
    }
    fn str_val(s: &str) -> EdnValue {
        EdnValue::String(s.to_string())
    }
    fn entity_lit() -> EdnValue {
        EdnValue::Uuid(Uuid::new_v4())
    }

    #[test]
    fn test_entity_bound_selects_eavt() {
        let p = make_pattern(entity_lit(), var("a"), var("v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[test]
    fn test_entity_and_attr_bound_selects_eavt() {
        let p = make_pattern(entity_lit(), kw(":name"), var("v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[test]
    fn test_attr_and_value_bound_selects_avet() {
        let p = make_pattern(var("e"), kw(":name"), str_val("Alice"));
        assert_eq!(select_index(&p), IndexHint::Avet);
    }

    #[test]
    fn test_attr_and_ref_bound_selects_avet() {
        // A UUID value with a bound attribute → AVET (not VAET, because attr is bound)
        let p = make_pattern(var("e"), kw(":friend"), entity_lit());
        assert_eq!(select_index(&p), IndexHint::Avet);
    }

    #[test]
    fn test_attr_only_selects_aevt() {
        let p = make_pattern(var("e"), kw(":name"), var("v"));
        assert_eq!(select_index(&p), IndexHint::Aevt);
    }

    #[test]
    fn test_ref_only_selects_vaet() {
        // UUID value but no bound attribute → VAET
        let p = make_pattern(var("e"), var("a"), entity_lit());
        assert_eq!(select_index(&p), IndexHint::Vaet);
    }

    #[test]
    fn test_nothing_bound_selects_eavt_full_scan() {
        let p = make_pattern(var("e"), var("a"), var("v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_join_ordering_moves_selective_pattern_first() {
        use crate::storage::index::Indexes;
        let p1 = make_pattern(var("e"), kw(":age"), var("a")); // selectivity 1 (attr only)
        let p2 = make_pattern(entity_lit(), kw(":name"), var("v")); // selectivity 2 (entity + attr)
        let p1_attr = p1.attribute.clone();
        let p2_attr = p2.attribute.clone();
        let planned = plan(
            vec![WhereClause::Pattern(p1), WhereClause::Pattern(p2)],
            &Indexes::new(),
        );
        let first_attr = match &planned[0].0 {
            WhereClause::Pattern(p) => p.attribute.clone(),
            _ => panic!("expected Pattern at index 0"),
        };
        let second_attr = match &planned[1].0 {
            WhereClause::Pattern(p) => p.attribute.clone(),
            _ => panic!("expected Pattern at index 1"),
        };
        assert_ne!(
            first_attr, p1_attr,
            "Lower-selectivity pattern must not be first"
        );
        assert_eq!(
            first_attr, p2_attr,
            "Higher-selectivity pattern must be first"
        );
        assert_eq!(
            second_attr, p1_attr,
            "Lower-selectivity pattern must be second"
        );
    }

    // ── expr_vars() ──────────────────────────────────────────────────────────

    #[test]
    fn test_expr_vars_var() {
        let e = Expr::Var("?age".to_string());
        assert_eq!(expr_vars(&e), vec!["?age".to_string()]);
    }

    #[test]
    fn test_expr_vars_lit_is_empty() {
        let e = Expr::Lit(Value::Integer(42));
        assert!(expr_vars(&e).is_empty());
    }

    #[test]
    fn test_expr_vars_binop() {
        let e = Expr::BinOp(
            BinOp::Gt,
            Box::new(Expr::Var("?age".to_string())),
            Box::new(Expr::Lit(Value::Integer(30))),
        );
        assert_eq!(expr_vars(&e), vec!["?age".to_string()]);
    }

    #[test]
    fn test_expr_vars_nested_binop_collects_all() {
        // (> (+ ?a ?b) ?c)
        let e = Expr::BinOp(
            BinOp::Gt,
            Box::new(Expr::BinOp(
                BinOp::Add,
                Box::new(Expr::Var("?a".to_string())),
                Box::new(Expr::Var("?b".to_string())),
            )),
            Box::new(Expr::Var("?c".to_string())),
        );
        let vars = expr_vars(&e);
        assert!(vars.contains(&"?a".to_string()));
        assert!(vars.contains(&"?b".to_string()));
        assert!(vars.contains(&"?c".to_string()));
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn test_expr_vars_unary_op() {
        use crate::query::datalog::types::UnaryOp;
        let e = Expr::UnaryOp(UnaryOp::IntegerQ, Box::new(Expr::Var("?v".to_string())));
        assert_eq!(expr_vars(&e), vec!["?v".to_string()]);
    }

    // ── plan() — new signature and push-down ─────────────────────────────────

    #[test]
    fn test_plan_pattern_carries_some_hint() {
        #[cfg(not(feature = "wasm"))]
        {
            use crate::storage::index::Indexes;
            let p = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
            let planned = plan(vec![p], &Indexes::new());
            assert!(
                planned[0].1.is_some(),
                "Pattern entry must carry Some(IndexHint)"
            );
        }
    }

    #[test]
    fn test_plan_expr_carries_none_hint() {
        #[cfg(not(feature = "wasm"))]
        {
            use crate::storage::index::Indexes;
            let p = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
            let expr = WhereClause::Expr {
                expr: Expr::Lit(Value::Boolean(true)),
                binding: None,
            };
            let planned = plan(vec![p, expr], &Indexes::new());
            let expr_entry = planned
                .iter()
                .find(|(c, _)| matches!(c, WhereClause::Expr { .. }));
            assert!(expr_entry.is_some());
            assert!(
                expr_entry.unwrap().1.is_none(),
                "Expr entry must carry None hint"
            );
        }
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_pushed_after_binding_pattern() {
        use crate::storage::index::Indexes;
        // Three patterns with equal selectivity (1 attr bound each) — stable sort preserves
        // original order: [p1, p2, p3]. Expr needs ?v, bound by p2 (pos 1).
        // Expected output: [p1, p2, expr, p3].
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":name"), var("n")));
        let p2 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let p3 = WhereClause::Pattern(make_pattern(var("e"), kw(":dept"), var("d")));
        let expr = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Gt,
                Box::new(Expr::Var("?v".to_string())),
                Box::new(Expr::Lit(Value::Integer(30))),
            ),
            binding: None,
        };
        let planned = plan(vec![p1, p2, p3, expr], &Indexes::new());
        assert_eq!(planned.len(), 4);
        // Item at index 2 must be the Expr (pushed after p2 which binds ?v at index 1).
        assert!(
            matches!(planned[2].0, WhereClause::Expr { .. }),
            "Expr must be at index 2"
        );
        // Item at index 3 must be a Pattern (p3).
        assert!(
            matches!(planned[3].0, WhereClause::Pattern(_)),
            "p3 must be at index 3"
        );
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_no_vars_goes_to_end() {
        use crate::storage::index::Indexes;
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let expr = WhereClause::Expr {
            expr: Expr::Lit(Value::Boolean(true)),
            binding: None,
        };
        let planned = plan(vec![p1, expr], &Indexes::new());
        assert_eq!(planned.len(), 2);
        assert!(
            matches!(planned[1].0, WhereClause::Expr { .. }),
            "no-var Expr must be last"
        );
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_expr_unbound_var_goes_to_end() {
        use crate::storage::index::Indexes;
        // ?x is never bound by any pattern
        let p1 = WhereClause::Pattern(make_pattern(var("e"), kw(":val"), var("v")));
        let expr = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Gt,
                Box::new(Expr::Var("?x".to_string())),
                Box::new(Expr::Lit(Value::Integer(0))),
            ),
            binding: None,
        };
        let planned = plan(vec![p1, expr], &Indexes::new());
        assert_eq!(planned.len(), 2);
        assert!(
            matches!(planned[1].0, WhereClause::Expr { .. }),
            "Expr with unbound var must be last"
        );
    }
}
