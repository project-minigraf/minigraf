//! Planner v2 (#473 spec §6.3): which queries the engine runs, and their
//! operator trees.
//!
//! The engine runs a query whose `:where` holds patterns and expression
//! clauses, and whose `:find` holds plain variables (#545). Every other query
//! runs on the existing executor (spec §9).
//!
//! Clauses run in the order `optimizer::plan` gives the executor, so an
//! expression sees the same variables as it does there. A binding expression
//! whose variable is already bound keeps the rows where its result is equal
//! (#596). The first pattern is a
//! `Scan`; each later one a join: an `IndexNestedLoopJoin` when its entity is
//! a variable already bound, a `HashJoin` otherwise (decision 3 of the #545
//! plan).
//!
//! A per-fact pseudo-attribute pattern reads, as the executor does, the fact
//! of the last pattern with a stored attribute before it that matched the
//! same entity (decision 4 of the #545 plan): one of the metadata columns
//! those patterns' scans fill, chosen per row. The engine runs it when the
//! pattern's entity term is the entity term of such a pattern, so that one
//! always exists, or when no such pattern runs before it. A pseudo-attribute
//! pattern of the second kind, and every `:db/valid-at` pattern, joins with
//! every visible fact of its entity, as `[e _ _]` does.

use super::cell::Scalar;
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

/// A pseudo-attribute: fact metadata as an integer column, or the query's
/// valid time (`nil` under `:any-valid-time`) for `:db/valid-at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Meta {
    ValidFrom,
    ValidTo,
    TxCount,
    TxId,
    ValidAt,
}

/// The entity a pattern matched, in a row: a slot, or a constant.
#[derive(Debug, Clone)]
pub(crate) enum Entity {
    Slot(usize),
    Const(EntityId),
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
    /// A pseudo-attribute pattern: its value term over a metadata column.
    /// The row's column is the first of `cols` with no entity, or whose
    /// entity is join-equal to `entity`. A constant other than an integer or
    /// `nil` never matches.
    Pseudo {
        cols: Vec<(Option<Entity>, usize)>,
        entity: Option<Entity>,
        value: Term<Option<Scalar>>,
    },
    /// `[(pred …)]`: keeps the rows where the expression is truthy.
    Filter(Expr, Vars),
    /// `[(f …) ?out]`: binds the slot, or keeps the rows where the result is
    /// equal to its cell; drops the rows where the expression fails.
    Bind(Expr, Vars, usize, Out),
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

/// What a binding expression does with its slot (#596).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Out {
    /// The slot is new: the result fills it.
    New,
    /// The slot is bound: keep the rows where the result is join-equal.
    Check,
    /// As `Check`, and the result replaces the cell: the expression is
    /// earlier in the text than the clause that bound it (decision 2 of the
    /// #545 plan).
    Replace,
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
    let mut binders = Binders::new(&query.where_clauses, Vec::new());
    let (planned, _) = optimizer::plan(query.where_clauses.clone());
    let all: Vec<WhereClause> = planned.into_iter().map(|(c, _)| c).collect();
    let (forms, needs) = pseudo_forms(&all)?;

    let mut slots = Slots::default();
    // Each clause placed so far.
    let mut done: Vec<Ran> = Vec::with_capacity(all.len());
    let mut outer_patterns: Vec<WhereClause> = Vec::new();
    let mut scan = None;
    let mut steps = Vec::with_capacity(all.len());
    for (i, c) in all.into_iter().enumerate() {
        let need = needs.get(i).map_or(&[][..], Vec::as_slice);
        let mut metas: Vec<(Meta, usize)> = Vec::new();
        let mut entity = None;
        match c {
            WhereClause::Pattern(q) => {
                let rebind = binders.pattern(&q);
                let form = forms.get(&i);
                if let Some(PseudoForm::Fast { cols }) = form {
                    let cols = cols
                        .iter()
                        .map(|&(j, compare)| {
                            let (metas, entity) = done.get(j)?;
                            let col = metas.iter().find(|(m, _)| Some(*m) == meta(&q))?.1;
                            let entity = if compare { Some(entity.clone()?) } else { None };
                            Some((entity, col))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let entity = entity_of(&q.entity, &slots);
                    let value = term(&q.value, &mut slots, OTHER, pseudo_const)?;
                    steps.push(Step::Pseudo {
                        cols,
                        entity,
                        value,
                    });
                    done.push((metas, None));
                    continue;
                }
                // A pattern with a stored attribute, or a pseudo-attribute
                // pattern read as `[e _ _]` and its metadata.
                let any = EdnValue::Symbol("_".to_string());
                let (attribute, scanned) = match &q.attribute {
                    AttributeSpec::Real(a) => (a.clone(), q.clone()),
                    AttributeSpec::Pseudo(_) => (
                        any.clone(),
                        Pattern::new(q.entity.clone(), any.clone(), any.clone()),
                    ),
                };
                let mut wants: Vec<Meta> = need.to_vec();
                if let Some(m) = form.and(meta(&q)) {
                    wants.push(m);
                }
                if scan.is_none() {
                    let e = term(&scanned.entity, &mut slots, OTHER, entity_const)?;
                    let a = term(&attribute, &mut slots, ATTRIBUTE, attribute_const)?;
                    let v = term(&scanned.value, &mut slots, OTHER, value_const)?;
                    metas = wants.iter().map(|&m| (m, slots.add(OTHER))).collect();
                    scan = Some(ScanSpec {
                        e,
                        a,
                        v,
                        meta: metas.clone(),
                        as_of,
                        valid,
                        width: slots.len(),
                    });
                } else {
                    let mut join = join(&scanned, &attribute, &mut slots, &rebind, as_of, valid)?;
                    for &m in &wants {
                        let local = join.inner.width;
                        join.inner.width += 1;
                        join.inner.meta.push((m, local));
                        join.new.push(local);
                        metas.push((m, slots.add(OTHER)));
                    }
                    if let JoinKind::Hash { build_inner } = &mut join.kind {
                        // With the test switch off, always the inner pattern
                        // (decision 3 of the #596 plan); `wasm` keeps the cost choice.
                        let cost = optimizer::reorder() || cfg!(feature = "wasm");
                        *build_inner = !cost
                            || optimizer::branch_cost(&[WhereClause::Pattern(q.clone())])
                                <= optimizer::branch_cost(&outer_patterns);
                    }
                    steps.push(Step::Join(join));
                }
                entity = entity_of(&q.entity, &slots);
                outer_patterns.push(WhereClause::Pattern(q.clone()));
                if form.is_some() {
                    let col = metas.last()?.1;
                    let value = term(&q.value, &mut slots, OTHER, pseudo_const)?;
                    steps.push(Step::Pseudo {
                        cols: vec![(None, col)],
                        entity: None,
                        value,
                    });
                }
            }
            WhereClause::Expr { expr, binding } => {
                // A clause before the first pattern sees no variable.
                let vars = expr_vars(&expr, &slots);
                let step = match binding {
                    None => Step::Filter(expr, vars),
                    Some(out) => {
                        let replace = binders.expression(&expr, &out);
                        let (slot, mode) = match slots.by_var.get(&out) {
                            Some(&s) if replace => {
                                if let Some(k) = slots.kinds.get_mut(s) {
                                    *k |= OTHER;
                                }
                                (s, Out::Replace)
                            }
                            Some(&s) => (s, Out::Check),
                            None => {
                                let s = slots.add(OTHER);
                                slots.by_var.insert(out, s);
                                (s, Out::New)
                            }
                        };
                        Step::Bind(expr, vars, slot, mode)
                    }
                };
                scan.as_ref()?;
                steps.push(step);
            }
            _ => return None,
        }
        done.push((metas, entity));
    }
    let scan = scan?;

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

/// The pseudo-attribute of `p`, or `None` for a stored attribute.
fn meta(p: &Pattern) -> Option<Meta> {
    let AttributeSpec::Pseudo(pa) = &p.attribute else {
        return None;
    };
    Some(match pa {
        PseudoAttr::ValidFrom => Meta::ValidFrom,
        PseudoAttr::ValidTo => Meta::ValidTo,
        PseudoAttr::TxCount => Meta::TxCount,
        PseudoAttr::TxId => Meta::TxId,
        PseudoAttr::ValidAt => Meta::ValidAt,
    })
}

/// A clause the planner has placed: the metadata slots its scan fills, and
/// its entity.
type Ran = (Vec<(Meta, usize)>, Option<Entity>);

/// The form of each pseudo-attribute pattern, by clause index.
type Forms = HashMap<usize, PseudoForm>;

/// How a pseudo-attribute pattern runs.
enum PseudoForm {
    /// It reads the metadata of an earlier pattern: of the first of `cols`
    /// (clause indexes, last first) that matched its entity, where the flag
    /// says whether the entity must be compared. The last one is the latest
    /// pattern with the same entity term, which always did.
    Fast { cols: Vec<(usize, bool)> },
    /// It joins with every visible fact of its entity.
    Join,
}

/// The form of each pseudo-attribute pattern of `all` (clauses in plan
/// order), by index, and the metadata each pattern's scan must fill; `None`
/// when one of them reads a pattern chosen by the row in a way the engine
/// does not reproduce, so the executor runs the query.
fn pseudo_forms(all: &[WhereClause]) -> Option<(Forms, Vec<Vec<Meta>>)> {
    let pattern = |c: &WhereClause| match c {
        WhereClause::Pattern(p) => Some(p.clone()),
        _ => None,
    };
    let patterns: Vec<Option<Pattern>> = all.iter().map(pattern).collect();
    let mut forms = HashMap::new();
    let mut needs = vec![Vec::new(); all.len()];
    for (i, q) in patterns.iter().enumerate() {
        let Some(m) = q.as_ref().and_then(meta) else {
            continue;
        };
        let Some(q) = q else { continue };
        // Earlier patterns with a stored attribute: the executor's per-entity
        // metadata of the fact each matched, the latest kept.
        let reals: Vec<(usize, &Pattern)> = patterns
            .iter()
            .take(i)
            .enumerate()
            .filter_map(|(j, p)| p.as_ref().map(|p| (j, p)))
            .filter(|(_, p)| matches!(p.attribute, AttributeSpec::Real(_)))
            .collect();
        if m == Meta::ValidAt || reals.is_empty() {
            forms.insert(i, PseudoForm::Join);
            continue;
        }
        let last = reals
            .iter()
            .rposition(|(_, p)| same_entity(&p.entity, &q.entity))?;
        let mut cols = Vec::new();
        for &(j, p) in reals.iter().skip(last + 1).rev() {
            match (&p.entity, &q.entity) {
                (EdnValue::Symbol(s), _) if s == "_" || s.starts_with("?_") => return None,
                (EdnValue::Symbol(_), _) | (_, EdnValue::Symbol(_)) => cols.push((j, true)),
                // Two constants naming different entities.
                _ => {}
            }
        }
        cols.push((reals.get(last)?.0, false));
        for &(j, _) in &cols {
            if let Some(n) = needs.get_mut(j)
                && !n.contains(&m)
            {
                n.push(m);
            }
        }
        forms.insert(i, PseudoForm::Fast { cols });
    }
    // An expression that rebinds an entity variable would send a later
    // pseudo-attribute pattern to the metadata of another entity.
    if !forms.is_empty()
        && all.iter().any(|c| {
            matches!(c, WhereClause::Expr { binding: Some(out), .. }
                if patterns.iter().flatten().any(
                    |p| matches!(&p.entity, EdnValue::Symbol(s) if s == out)))
        })
    {
        return None;
    }
    Some((forms, needs))
}

/// The entity `edn` gives a row: a bound variable's slot, or a constant.
fn entity_of(edn: &EdnValue, slots: &Slots) -> Option<Entity> {
    match edn {
        EdnValue::Symbol(s) => slots.by_var.get(s).map(|&s| Entity::Slot(s)),
        EdnValue::Keyword(_) | EdnValue::Uuid(_) => edn_to_entity_id(edn).ok().map(Entity::Const),
        _ => None,
    }
}

/// A constant in a pseudo-attribute pattern's value position: an integer, or
/// `nil` (the valid time under `:any-valid-time`); anything else never
/// matches.
fn pseudo_const(c: &EdnValue) -> Option<Option<Scalar>> {
    Some(match c {
        EdnValue::Integer(n) => Some(Scalar::Int(*n)),
        EdnValue::Nil => Some(Scalar::Null),
        _ => None,
    })
}

/// Whether two entity terms name the same entity in every row: one
/// variable, or constants naming one entity.
fn same_entity(a: &EdnValue, b: &EdnValue) -> bool {
    match (a, b) {
        (EdnValue::Symbol(s), EdnValue::Symbol(t)) => {
            s.starts_with('?') && !s.starts_with("?_") && s == t
        }
        (EdnValue::Keyword(_) | EdnValue::Uuid(_), EdnValue::Keyword(_) | EdnValue::Uuid(_)) => {
            edn_to_entity_id(a).ok() == edn_to_entity_id(b).ok()
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
