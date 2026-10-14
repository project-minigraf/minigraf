//! Planner v1 (#473 spec §6.3): which queries the engine runs, and their
//! operator trees.
//!
//! #544 runs a query whose `:where` is one pattern with a stored attribute,
//! and whose `:find` holds plain variables. Every other query runs on the
//! existing executor (spec §9).

use crate::graph::types::{EntityId, Value};
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogQuery, EdnValue, FindSpec, ValidAt, WhereClause,
};
use std::collections::HashMap;

/// One position of a pattern.
#[derive(Debug, Clone)]
pub(crate) enum Term<C> {
    /// `_` or a `?_` variable: matches anything, binds nothing.
    Any,
    /// The first occurrence of a variable: binds its slot.
    Bind(usize),
    /// A later occurrence of a variable: must equal its slot.
    Eq(usize),
    Const(C),
}

/// The valid-time test of a query.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Valid {
    /// No `:valid-at`: valid at the snapshot's "now".
    Now,
    At(i64),
    Any,
}

/// What a `Scan` reads.
#[derive(Debug, Clone)]
pub(crate) struct ScanSpec {
    pub(crate) e: Term<EntityId>,
    pub(crate) a: Term<String>,
    pub(crate) v: Term<Value>,
    pub(crate) as_of: Option<u64>,
    pub(crate) valid: Valid,
    /// Number of variable slots.
    pub(crate) width: usize,
}

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) scan: ScanSpec,
    /// The slot of each `:find` variable.
    pub(crate) find: Vec<usize>,
    pub(crate) vars: Vec<String>,
}

/// The plan of `query`, or `None` when the engine does not run it yet.
pub(crate) fn plan(query: &DatalogQuery) -> Option<Plan> {
    let as_of = match &query.as_of {
        None => None,
        Some(AsOf::Counter(n)) => Some(*n),
        // A timestamp is not a key bound; a slot is resolved before this.
        Some(AsOf::Timestamp(_) | AsOf::Slot(_)) => return None,
    };
    let valid = match &query.valid_at {
        None => Valid::Now,
        Some(ValidAt::Timestamp(t)) => Valid::At(*t),
        Some(ValidAt::AnyValidTime) => Valid::Any,
        Some(ValidAt::Slot(_)) => return None,
    };
    let [WhereClause::Pattern(p)] = query.where_clauses.as_slice() else {
        return None;
    };
    let AttributeSpec::Real(attribute) = &p.attribute else {
        return None;
    };
    let mut slots = HashMap::new();
    let e = term(&p.entity, &mut slots, |c| match c {
        EdnValue::Keyword(_) | EdnValue::Uuid(_) => edn_to_entity_id(c).ok(),
        _ => None,
    })?;
    let a = term(attribute, &mut slots, |c| match c {
        EdnValue::Keyword(k) => Some(k.clone()),
        _ => None,
    })?;
    let v = term(&p.value, &mut slots, |c| match c {
        EdnValue::String(s) => Some(Value::String(s.clone())),
        EdnValue::Integer(n) => Some(Value::Integer(*n)),
        EdnValue::Boolean(b) => Some(Value::Boolean(*b)),
        EdnValue::Nil => Some(Value::Null),
        _ => None,
    })?;
    let mut find = Vec::with_capacity(query.find.len());
    for spec in &query.find {
        let FindSpec::Variable(var) = spec else {
            return None;
        };
        // The parser rejects a variable no clause binds (PRS-080).
        find.push(*slots.get(var)?);
    }
    Some(Plan {
        scan: ScanSpec {
            e,
            a,
            v,
            as_of,
            valid,
            width: slots.len(),
        },
        find,
        vars: query.find.iter().map(FindSpec::display_name).collect(),
    })
}

/// The term of one pattern position; `None` for a constant `konst` does not
/// take, or a symbol that is not a variable.
fn term<C>(
    edn: &EdnValue,
    slots: &mut HashMap<String, usize>,
    konst: impl Fn(&EdnValue) -> Option<C>,
) -> Option<Term<C>> {
    match edn {
        EdnValue::Symbol(s) if s == "_" || s.starts_with("?_") => Some(Term::Any),
        EdnValue::Symbol(s) if s.starts_with('?') => Some(match slots.get(s) {
            Some(&slot) => Term::Eq(slot),
            None => {
                let slot = slots.len();
                slots.insert(s.clone(), slot);
                Term::Bind(slot)
            }
        }),
        EdnValue::Symbol(_) => None,
        c => konst(c).map(Term::Const),
    }
}
