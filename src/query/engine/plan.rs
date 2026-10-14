//! Planner v2 (#473 spec §6.3): which queries the engine runs, and their
//! operator trees.
//!
//! The engine runs a query whose `:where` holds patterns with a stored
//! attribute and expression clauses, and whose `:find` holds plain
//! variables (#545); or one such pattern, pseudo-attribute patterns on its
//! entity and expression clauses (#544). Every other query runs on the
//! existing executor (spec §9).
//!
//! Clauses run in the order `optimizer::plan` gives the executor, so an
//! expression sees the same variables, and a binding expression overwrites a
//! variable at the same point, as it does there. The first pattern is a
//! `Scan`; each later one a join: an `IndexNestedLoopJoin` when its entity is
//! a variable already bound, a `HashJoin` otherwise (decision 3 of the #545
//! plan).

use crate::graph::types::{EntityId, Value};
use crate::query::datalog::executor::Binders;
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::optimizer;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogQuery, EdnValue, Expr, FindSpec, Pattern, PseudoAttr, ValidAt,
    WhereClause,
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
    /// A later pattern, joined with the rows so far.
    Join(JoinSpec),
}

/// A join of the rows so far (the outer side) with one pattern (spec §6.2).
#[derive(Debug, Clone)]
pub(crate) struct JoinSpec {
    /// The pattern, over slots of its own: its variables in order of first
    /// occurrence. A variable repeated inside it compares exactly, as in one
    /// pattern alone.
    pub(crate) inner: ScanSpec,
    pub(crate) kind: JoinKind,
    /// The variables both sides bind.
    pub(crate) keys: Vec<Key>,
    /// The inner slots of the variables the outer side lacks, in the order
    /// they are added after the outer slots.
    pub(crate) new: Vec<usize>,
    /// `(inner, outer)`: outer slots whose cell the inner one replaces, for a
    /// variable whose first binding in the query text is this pattern
    /// (decision 2 of the #545 plan).
    pub(crate) replace: Vec<(usize, usize)>,
    /// Number of slots of an outer row.
    pub(crate) outer_width: usize,
}

/// One variable both sides of a join bind.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Key {
    pub(crate) inner: usize,
    pub(crate) outer: usize,
    /// Whether a keyword equals a ref to the entity it names here: everywhere
    /// but between two attribute positions.
    pub(crate) normalise: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum JoinKind {
    /// `IndexNestedLoopJoin`: the inner pattern's entity is `keys[k]`; one
    /// entity range per outer row.
    Probe(usize),
    /// `HashJoin`: a table of the side with the smaller estimate, probed by
    /// the other.
    Hash { build_inner: bool },
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

/// Positions a slot's cells come from.
const ATTRIBUTE: u8 = 1;
const OTHER: u8 = 2;

/// Variable slots, in the order clauses bind them.
#[derive(Default)]
struct Slots {
    by_var: HashMap<String, usize>,
    /// For each slot: the positions its cells may come from.
    kinds: Vec<u8>,
}

impl Slots {
    fn add(&mut self, kind: u8) -> usize {
        self.kinds.push(kind);
        self.kinds.len() - 1
    }

    fn len(&self) -> usize {
        self.kinds.len()
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
    // Pseudo-attribute patterns read the fact of the one real pattern; across
    // joins they wait for #545's last part.
    let real = query
        .where_clauses
        .iter()
        .filter(|c| matches!(c, WhereClause::Pattern(p) if matches!(p.attribute, AttributeSpec::Real(_))))
        .count();
    let pseudo = query.where_clauses.iter().any(
        |c| matches!(c, WhereClause::Pattern(p) if matches!(p.attribute, AttributeSpec::Pseudo(_))),
    );
    if real > 1 && pseudo {
        return None;
    }
    let mut binders = Binders::new(&query.where_clauses, Vec::new());
    let (planned, _) = optimizer::plan(query.where_clauses.clone());
    let mut planned = planned.into_iter().map(|(c, _)| c);
    let Some(WhereClause::Pattern(p)) = planned.next() else {
        return None;
    };
    let AttributeSpec::Real(attribute) = &p.attribute else {
        return None;
    };
    binders.pattern(&p);
    let rest: Vec<WhereClause> = planned.collect();

    let mut slots = Slots::default();
    let e = term(&p.entity, &mut slots, OTHER, entity_const)?;
    let a = term(attribute, &mut slots, ATTRIBUTE, attribute_const)?;
    let v = term(&p.value, &mut slots, OTHER, value_const)?;

    // A pseudo-attribute pattern reads the metadata of the fact the scan
    // matched only when its entity is the scan's entity term; any other
    // pseudo-attribute pattern joins over every fact (#545).
    let mut meta: Vec<(Meta, usize)> = Vec::new();
    for c in &rest {
        let WhereClause::Pattern(q) = c else {
            continue;
        };
        let AttributeSpec::Pseudo(pa) = &q.attribute else {
            continue;
        };
        let m = meta_of(pa)?;
        if !same_entity(&p.entity, &q.entity) {
            return None;
        }
        if !meta.iter().any(|(mm, _)| *mm == m) {
            meta.push((m, slots.add(OTHER)));
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
    let scan = ScanSpec {
        e,
        a,
        v,
        meta: meta.clone(),
        as_of,
        valid,
        width: slots.len(),
    };

    let mut outer_patterns = vec![WhereClause::Pattern(p.clone())];
    let mut steps = Vec::with_capacity(rest.len());
    for c in rest {
        steps.push(match c {
            WhereClause::Pattern(q) => match &q.attribute {
                AttributeSpec::Pseudo(pa) => {
                    binders.pattern(&q);
                    let m = meta_of(pa)?;
                    let col = meta.iter().find(|(mm, _)| *mm == m)?.1;
                    let value = term(&q.value, &mut slots, OTHER, |c| {
                        Some(match c {
                            EdnValue::Integer(n) => Some(*n),
                            _ => None,
                        })
                    })?;
                    Step::Pseudo { col, value }
                }
                AttributeSpec::Real(attribute) => {
                    let rebind = binders.pattern(&q);
                    let mut join = join(&q, attribute, &mut slots, &rebind, as_of, valid)?;
                    if let JoinKind::Hash { build_inner } = &mut join.kind {
                        *build_inner = optimizer::branch_cost(&[WhereClause::Pattern(q.clone())])
                            <= optimizer::branch_cost(&outer_patterns);
                    }
                    outer_patterns.push(WhereClause::Pattern(q));
                    Step::Join(join)
                }
            },
            WhereClause::Expr { expr, binding } => {
                let vars = expr_vars(&expr, &slots);
                match binding {
                    None => Step::Filter(expr, vars),
                    Some(out) => {
                        binders.expression(&out);
                        let slot = match slots.by_var.get(&out) {
                            Some(&s) => {
                                if let Some(k) = slots.kinds.get_mut(s) {
                                    *k |= OTHER;
                                }
                                s
                            }
                            None => {
                                let s = slots.add(OTHER);
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
        scan,
        steps,
        find,
        vars: query.find.iter().map(FindSpec::display_name).collect(),
    })
}

/// The join of the rows so far, whose variables are `slots`, with the
/// pattern `q`; adds the slots of its new variables. `rebind` names the
/// variables whose value `q` gives (decision 2 of the #545 plan).
fn join(
    q: &Pattern,
    attribute: &EdnValue,
    slots: &mut Slots,
    rebind: &[String],
    as_of: Option<u64>,
    valid: Valid,
) -> Option<JoinSpec> {
    let outer_width = slots.len();
    let mut local = Slots::default();
    let e = term(&q.entity, &mut local, OTHER, entity_const)?;
    let a = term(attribute, &mut local, ATTRIBUTE, attribute_const)?;
    let v = term(&q.value, &mut local, OTHER, value_const)?;

    let mut keys = Vec::new();
    let mut new = Vec::new();
    let mut replace = Vec::new();
    let mut probe = None;
    // Each variable at its first position in the pattern.
    for (position, (edn, kind)) in [
        (&q.entity, OTHER),
        (attribute, ATTRIBUTE),
        (&q.value, OTHER),
    ]
    .into_iter()
    .enumerate()
    {
        let EdnValue::Symbol(var) = edn else {
            continue;
        };
        let Some(&inner) = local.by_var.get(var) else {
            continue;
        };
        if keys.iter().any(|k: &Key| k.inner == inner) || new.contains(&inner) {
            continue;
        }
        match slots.by_var.get(var) {
            Some(&outer) => {
                let held = slots.kinds.get(outer).copied().unwrap_or(OTHER);
                if position == 0 {
                    probe = Some(keys.len());
                }
                keys.push(Key {
                    inner,
                    outer,
                    normalise: !(kind == ATTRIBUTE && held == ATTRIBUTE),
                });
                if rebind.contains(var) {
                    replace.push((inner, outer));
                    if let Some(k) = slots.kinds.get_mut(outer) {
                        *k |= kind;
                    }
                }
            }
            None => {
                new.push(inner);
                let s = slots.add(kind);
                slots.by_var.insert(var.clone(), s);
            }
        }
    }
    Some(JoinSpec {
        inner: ScanSpec {
            e,
            a,
            v,
            meta: Vec::new(),
            as_of,
            valid,
            width: local.len(),
        },
        kind: match probe {
            Some(k) => JoinKind::Probe(k),
            None => JoinKind::Hash { build_inner: true },
        },
        keys,
        new,
        replace,
        outer_width,
    })
}

fn entity_const(c: &EdnValue) -> Option<EntityId> {
    match c {
        EdnValue::Keyword(_) | EdnValue::Uuid(_) => edn_to_entity_id(c).ok(),
        _ => None,
    }
}

fn attribute_const(c: &EdnValue) -> Option<String> {
    match c {
        EdnValue::Keyword(k) => Some(k.clone()),
        _ => None,
    }
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
    kind: u8,
    konst: impl Fn(&EdnValue) -> Option<C>,
) -> Option<Term<C>> {
    match edn {
        EdnValue::Symbol(s) if s == "_" || s.starts_with("?_") => Some(Term::Any),
        EdnValue::Symbol(s) if s.starts_with('?') => Some(match slots.by_var.get(s) {
            Some(&slot) => Term::Eq(slot),
            None => {
                let slot = slots.add(kind);
                slots.by_var.insert(s.clone(), slot);
                Term::Bind(slot)
            }
        }),
        EdnValue::Symbol(_) => None,
        c => konst(c).map(Term::Const),
    }
}
