//! Planner v1 (#473 spec §6.3): which queries the engine runs, and their
//! operator trees.
//!
//! #544 runs a query whose `:where` is one pattern with a stored attribute,
//! followed by pseudo-attribute patterns on the same entity and expression
//! clauses, and whose `:find` holds plain variables. Every other query runs
//! on the existing executor (spec §9).
//!
//! Clauses run in the order `optimizer::plan` gives the executor, so an
//! expression sees the same variables, and a binding expression overwrites a
//! variable at the same point, as it does there.

use crate::graph::types::{EntityId, Value};
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::optimizer;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogQuery, EdnValue, Expr, FindSpec, PseudoAttr, ValidAt, WhereClause,
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

/// A constant in the value position, with the executor's matching rules
/// (`PatternMatcher::match_component`).
#[derive(Debug, Clone)]
pub(crate) enum ValueConst {
    /// A string, integer, boolean or `nil`: equal values only.
    Exact(Value),
    /// A keyword: the same keyword, or a ref to the entity it names.
    Keyword(String, EntityId),
    /// A UUID: a ref to it, or a keyword that names it.
    Uuid(EntityId),
    /// A float: floats within `f64::EPSILON`.
    Float(f64),
}

impl ValueConst {
    pub(crate) fn matches(&self, v: &Value) -> bool {
        match (self, v) {
            (ValueConst::Exact(c), v) => c == v,
            (ValueConst::Keyword(k, _), Value::Keyword(fk)) => k == fk,
            (ValueConst::Keyword(_, u), Value::Ref(fu)) | (ValueConst::Uuid(u), Value::Ref(fu)) => {
                u == fu
            }
            (ValueConst::Uuid(u), Value::Keyword(k)) => names(k, u),
            (ValueConst::Float(f), Value::Float(ff)) => (f - ff).abs() < f64::EPSILON,
            _ => false,
        }
    }
}

/// Whether the keyword `k` names the entity `u`.
pub(crate) fn names(k: &str, u: &EntityId) -> bool {
    edn_to_entity_id(&EdnValue::Keyword(k.to_string())).is_ok_and(|id| id == *u)
}

/// The valid-time test of a query.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Valid {
    /// No `:valid-at`: valid at the snapshot's "now".
    Now,
    At(i64),
    Any,
}

/// A per-fact pseudo-attribute: fact metadata as an integer column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Meta {
    ValidFrom,
    ValidTo,
    TxCount,
    TxId,
}

/// What a `Scan` reads.
#[derive(Debug, Clone)]
pub(crate) struct ScanSpec {
    pub(crate) e: Term<EntityId>,
    pub(crate) a: Term<String>,
    pub(crate) v: Term<ValueConst>,
    /// Metadata columns the scan fills, and their slots.
    pub(crate) meta: Vec<(Meta, usize)>,
    pub(crate) as_of: Option<u64>,
    pub(crate) valid: Valid,
    /// Number of slots the scan fills.
    pub(crate) width: usize,
}

/// A clause after the scan, in the executor's order.
#[derive(Debug, Clone)]
pub(crate) enum Step {
    /// A pseudo-attribute pattern on the scanned fact: its value term over
    /// the metadata column `col`. A constant other than an integer never
    /// matches.
    Pseudo {
        col: usize,
        value: Term<Option<i64>>,
    },
    /// `[(pred …)]`: keeps the rows where the expression is truthy.
    Filter(Expr, Vars),
    /// `[(f …) ?out]`: binds (or overwrites) the slot; drops the rows where
    /// the expression fails.
    Bind(Expr, Vars, usize),
}

/// The variables an expression reads that are bound when it runs, by slot.
pub(crate) type Vars = Vec<(String, usize)>;

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) scan: ScanSpec,
    pub(crate) steps: Vec<Step>,
    /// The slot of each `:find` variable.
    pub(crate) find: Vec<usize>,
    pub(crate) vars: Vec<String>,
}

/// Variable slots, in the order clauses bind them.
#[derive(Default)]
struct Slots {
    by_var: HashMap<String, usize>,
    len: usize,
}

impl Slots {
    fn add(&mut self) -> usize {
        self.len += 1;
        self.len - 1
    }
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
    if !query
        .where_clauses
        .iter()
        .all(|c| matches!(c, WhereClause::Pattern(_) | WhereClause::Expr { .. }))
    {
        return None;
    }
    let (planned, _) = optimizer::plan(query.where_clauses.clone());
    let mut planned = planned.into_iter().map(|(c, _)| c);
    let Some(WhereClause::Pattern(p)) = planned.next() else {
        return None;
    };
    let AttributeSpec::Real(attribute) = &p.attribute else {
        return None;
    };
    let rest: Vec<WhereClause> = planned.collect();

    let mut slots = Slots::default();
    let e = term(&p.entity, &mut slots, |c| match c {
        EdnValue::Keyword(_) | EdnValue::Uuid(_) => edn_to_entity_id(c).ok(),
        _ => None,
    })?;
    let a = term(attribute, &mut slots, |c| match c {
        EdnValue::Keyword(k) => Some(k.clone()),
        _ => None,
    })?;
    let v = term(&p.value, &mut slots, value_const)?;

    // A pseudo-attribute pattern reads the metadata of the fact the scan
    // matched only when its entity is the scan's entity term; any other
    // pseudo-attribute pattern joins over every fact (#545).
    let mut meta: Vec<(Meta, usize)> = Vec::new();
    for c in &rest {
        let WhereClause::Pattern(q) = c else {
            continue;
        };
        let m = match &q.attribute {
            AttributeSpec::Pseudo(pa) => meta_of(pa)?,
            AttributeSpec::Real(_) => return None,
        };
        if !same_entity(&p.entity, &q.entity) {
            return None;
        }
        if !meta.iter().any(|(mm, _)| *mm == m) {
            meta.push((m, slots.add()));
        }
    }
    // An expression that rebinds the entity variable would send a later
    // pseudo-attribute pattern to another entity.
    if !meta.is_empty()
        && rest.iter().any(|c| {
            matches!(c, WhereClause::Expr { binding: Some(out), .. }
                if matches!(&p.entity, EdnValue::Symbol(s) if s == out))
        })
    {
        return None;
    }
    let width = slots.len;

    let mut steps = Vec::with_capacity(rest.len());
    for c in rest {
        steps.push(match c {
            WhereClause::Pattern(q) => {
                let AttributeSpec::Pseudo(pa) = &q.attribute else {
                    return None;
                };
                let m = meta_of(pa)?;
                let col = meta.iter().find(|(mm, _)| *mm == m)?.1;
                let value = term(&q.value, &mut slots, |c| {
                    Some(match c {
                        EdnValue::Integer(n) => Some(*n),
                        _ => None,
                    })
                })?;
                Step::Pseudo { col, value }
            }
            WhereClause::Expr { expr, binding } => {
                let vars = expr_vars(&expr, &slots);
                match binding {
                    None => Step::Filter(expr, vars),
                    Some(out) => {
                        let slot = match slots.by_var.get(&out) {
                            Some(&s) => s,
                            None => {
                                let s = slots.add();
                                slots.by_var.insert(out, s);
                                s
                            }
                        };
                        Step::Bind(expr, vars, slot)
                    }
                }
            }
            _ => return None,
        });
    }

    let mut find = Vec::with_capacity(query.find.len());
    for spec in &query.find {
        let FindSpec::Variable(var) = spec else {
            return None;
        };
        // The parser rejects a variable no clause binds (PRS-080).
        find.push(*slots.by_var.get(var)?);
    }
    Some(Plan {
        scan: ScanSpec {
            e,
            a,
            v,
            meta,
            as_of,
            valid,
            width,
        },
        steps,
        find,
        vars: query.find.iter().map(FindSpec::display_name).collect(),
    })
}

fn value_const(c: &EdnValue) -> Option<ValueConst> {
    Some(match c {
        EdnValue::String(s) => ValueConst::Exact(Value::String(s.clone())),
        EdnValue::Integer(n) => ValueConst::Exact(Value::Integer(*n)),
        EdnValue::Boolean(b) => ValueConst::Exact(Value::Boolean(*b)),
        EdnValue::Nil => ValueConst::Exact(Value::Null),
        // A keyword whose name parses as a UUID names that UUID.
        EdnValue::Keyword(k) => ValueConst::Keyword(k.clone(), edn_to_entity_id(c).ok()?),
        EdnValue::Uuid(u) => ValueConst::Uuid(*u),
        EdnValue::Float(f) => ValueConst::Float(*f),
        _ => return None,
    })
}

/// The metadata column of a per-fact pseudo-attribute; `None` for
/// `:db/valid-at`, which the executor matches against every fact of the
/// entity (a join, #545).
fn meta_of(pa: &PseudoAttr) -> Option<Meta> {
    match pa {
        PseudoAttr::ValidFrom => Some(Meta::ValidFrom),
        PseudoAttr::ValidTo => Some(Meta::ValidTo),
        PseudoAttr::TxCount => Some(Meta::TxCount),
        PseudoAttr::TxId => Some(Meta::TxId),
        PseudoAttr::ValidAt => None,
    }
}

/// Whether a pseudo-attribute pattern's entity `pseudo` resolves to the
/// entity of the fact the scan pattern's entity `scan` matched.
fn same_entity(scan: &EdnValue, pseudo: &EdnValue) -> bool {
    match (scan, pseudo) {
        (EdnValue::Symbol(s), EdnValue::Symbol(t)) => {
            s.starts_with('?') && !s.starts_with("?_") && s == t
        }
        (EdnValue::Keyword(_) | EdnValue::Uuid(_), EdnValue::Keyword(_) | EdnValue::Uuid(_)) => {
            edn_to_entity_id(scan).ok() == edn_to_entity_id(pseudo).ok()
        }
        _ => false,
    }
}

/// The variables of `expr` bound so far, with their slots. A variable bound
/// later, or never, is left out: the expression fails on it, as it does in
/// the executor.
fn expr_vars(expr: &Expr, slots: &Slots) -> Vars {
    fn walk(expr: &Expr, slots: &Slots, out: &mut Vars) {
        match expr {
            Expr::Var(v) => {
                if let Some(&s) = slots.by_var.get(v)
                    && !out.iter().any(|(n, _)| n == v)
                {
                    out.push((v.clone(), s));
                }
            }
            Expr::Lit(_) | Expr::Slot(_) => {}
            Expr::BinOp(_, l, r) => {
                walk(l, slots, out);
                walk(r, slots, out);
            }
            Expr::UnaryOp(_, inner) => walk(inner, slots, out),
        }
    }
    let mut out = Vec::new();
    walk(expr, slots, &mut out);
    out
}

/// The term of one pattern position; `None` for a constant `konst` does not
/// take, or a symbol that is not a variable.
fn term<C>(
    edn: &EdnValue,
    slots: &mut Slots,
    konst: impl Fn(&EdnValue) -> Option<C>,
) -> Option<Term<C>> {
    match edn {
        EdnValue::Symbol(s) if s == "_" || s.starts_with("?_") => Some(Term::Any),
        EdnValue::Symbol(s) if s.starts_with('?') => Some(match slots.by_var.get(s) {
            Some(&slot) => Term::Eq(slot),
            None => {
                let slot = slots.add();
                slots.by_var.insert(s.clone(), slot);
                Term::Bind(slot)
            }
        }),
        EdnValue::Symbol(_) => None,
        c => konst(c).map(Term::Const),
    }
}
