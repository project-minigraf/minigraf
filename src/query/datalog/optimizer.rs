//! Query optimizer: index selection and join ordering for Datalog patterns.
//!
//! `plan()` is the single entry point. It assigns an `IndexHint` to each
//! pattern and (outside the `wasm` feature) sorts patterns by selectivity.

use crate::query::datalog::types::{EdnValue, Pattern};

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

/// Count the number of non-variable components in a pattern.
/// Higher score = more selective.
#[cfg(not(feature = "wasm"))]
fn selectivity_score(p: &Pattern) -> u8 {
    let e = !is_variable(&p.entity);
    let a = !is_variable(&p.attribute);
    let v = !is_variable(&p.value);
    e as u8 + a as u8 + v as u8
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
    let a_bound = !is_variable(&p.attribute);
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

/// Plan a list of patterns: assign index hints and (non-wasm) reorder by selectivity.
///
/// `_indexes` is reserved for statistics-based optimization in a future phase;
/// in Phase 6.1 selectivity is estimated purely from bound-variable counts.
///
/// Under the `wasm` feature flag, patterns execute in user-written order
/// (index selection still applies, join reordering is skipped).
pub fn plan(
    patterns: Vec<Pattern>,
    _indexes: &crate::storage::index::Indexes,
) -> Vec<(Pattern, IndexHint)> {
    let planned: Vec<(Pattern, IndexHint)> = patterns
        .into_iter()
        .map(|p| {
            let h = select_index(&p);
            (p, h)
        })
        .collect();

    // Stable sort preserves original order for ties.
    // Under `wasm`, patterns execute in user-written order (join reordering skipped).
    #[cfg(not(feature = "wasm"))]
    let planned = {
        let mut v = planned;
        v.sort_by_key(|(p, _)| std::cmp::Reverse(selectivity_score(p)));
        v
    };

    planned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{EdnValue, Pattern};
    use crate::storage::index::Indexes;
    use uuid::Uuid;

    fn make_pattern(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Pattern {
        Pattern {
            entity,
            attribute,
            value,
            valid_from: None,
            valid_to: None,
        }
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
        let p1 = make_pattern(var("e"), kw(":age"), var("a")); // selectivity 1 (attr only)
        let p2 = make_pattern(entity_lit(), kw(":name"), var("v")); // selectivity 2 (entity + attr)
        let p1_attr = p1.attribute.clone();
        let p2_attr = p2.attribute.clone();
        let planned = plan(vec![p1, p2], &Indexes::new());
        assert_ne!(
            planned[0].0.attribute, p1_attr,
            "Lower-selectivity pattern must not be first"
        );
        assert_eq!(
            planned[0].0.attribute, p2_attr,
            "Higher-selectivity pattern must be first"
        );
    }
}
