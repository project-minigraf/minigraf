//! Planner v2 (#473 spec §6.3): which queries the engine runs, and their
//! operator trees.
//!
//! The engine runs a query whose `:where` holds patterns and expression
//! clauses, and whose `:find` holds plain variables (#545), or aggregates
//! with plain variables and a `:with` (#547). Every other query runs on the
//! existing executor (spec §9).
//!
//! Clauses run in the order `optimizer::plan` gives the executor, so an
//! expression sees the same variables as it does there. A binding expression
//! whose variable is already bound keeps the rows where its result is equal
//! (#596). The first pattern is a
//! `Scan`; each later one a join: an `IndexNestedLoopJoin` when its entity is
//! a variable already bound, a `HashJoin` otherwise (decision 3 of the #545
//! plan).
//!
//! A per-fact pseudo-attribute pattern reads the metadata of the fact its
//! source matched (#594): the parser names the source, which binds a hidden
//! variable per pseudo-attribute, and its scan or join fills that slot from
//! the fact's key. The pseudo-attribute pattern binds or checks its value
//! against the slot. A `:db/valid-at` pattern joins with every visible fact
//! of its entity, as `[e _ _]` does, with the query's valid time as a column.

use super::cell::Scalar;
use crate::graph::types::{EntityId, Value};
use crate::query::datalog::aggregate::named_vars;
use crate::query::datalog::executor::Binders;
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::optimizer;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogQuery, EdnValue, Expr, FindSpec, Pattern, PseudoAttr, UnaryOp,
    ValidAt, WhereClause,
};
use std::collections::{HashMap, HashSet};

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
    /// A pseudo-attribute pattern: its value term over the metadata column
    /// `col`. A constant other than an integer or `nil` never matches.
    Pseudo {
        col: usize,
        value: Term<Option<Scalar>>,
    },
    /// `[(pred …)]`: keeps the rows where the expression is truthy.
    Filter(Expr, Vars),
    /// `[(f …) ?out]`: binds the slot, or keeps the rows where the result is
    /// equal to its cell; drops the rows where the expression fails.
    Bind(Expr, Vars, usize, Out),
    /// A later pattern, joined with the rows so far.
    Join(JoinSpec),
    /// `not` / `not-join`: keeps the rows for which the body has no row
    /// (#546).
    Anti(Body),
    /// `or` / `or-join`: each row once per row of each branch, extended by
    /// the branch's cells of the new variables (#546 spec §5.1).
    Union(Vec<Branch>),
    /// Keeps the first of the rows whose cells at these slots are equal as
    /// the executor's values are (#546 spec §5.2).
    Distinct(Vec<usize>),
    /// The last step of an aggregate query (#547 spec §5): one row per group
    /// of the cells at `groups`, holding those cells and then the result of
    /// each aggregate `(function, argument slot)`.
    Aggregate {
        groups: Vec<usize>,
        aggs: Vec<(String, usize)>,
        /// Whether no input row gives one row of zeros: a query of `count`s
        /// and `count-distinct`s and no plain `:find` variable.
        zero_row: bool,
    },
    /// The last step of a query with windows (#547 spec §4): each row once,
    /// in input order, holding its cells at `keep` and then the value of
    /// each window of `find`, over the named variables at `named`.
    Window {
        find: Vec<FindSpec>,
        named: Vec<usize>,
        keep: Vec<usize>,
    },
}

/// A `not` / `not-join` body (#546 spec §4).
#[derive(Debug, Clone)]
pub(crate) enum Body {
    /// Run once per outer row, from a seed of its cells at `imports` (in
    /// seed slots `0..imports.len()`).
    Correlated {
        plan: Box<Plan>,
        imports: Vec<usize>,
    },
    /// Run once; a body row matches the outer rows whose cells at each
    /// `keys[k].outer` are join-equal to its cells at `keys[k].inner`.
    Hashed { plan: Box<Plan>, keys: Vec<Key> },
}

/// One branch of an `or` / `or-join` (#546 spec §5.1).
#[derive(Debug, Clone)]
pub(crate) struct Branch {
    pub(crate) body: Body,
    /// The body slots whose cells extend the outer row, in the order of the
    /// slots they fill; empty for an `or-join`.
    pub(crate) new: Vec<usize>,
}

/// Where a plan's rows start.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// The first pattern.
    Scan(ScanSpec),
    /// One row: the cells a correlated body imports, in its first slots;
    /// for a query whose first clause to run is an `or`, the empty row.
    Seed,
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

#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub(crate) source: Source,
    pub(crate) steps: Vec<Step>,
    /// The column of each `:find` element in the rows of the last step: a
    /// variable's slot, or a column of `Aggregate`; empty for a body.
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

    fn kind(&self, slot: usize) -> u8 {
        self.kinds.get(slot).copied().unwrap_or(OTHER)
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
    if !query.where_clauses.iter().all(|c| {
        matches!(
            c,
            WhereClause::Pattern(_)
                | WhereClause::Expr { .. }
                | WhereClause::Not(_)
                | WhereClause::NotJoin { .. }
                | WhereClause::Or(_)
                | WhereClause::OrJoin { .. }
        )
    }) {
        return None;
    }
    let mut b = Builder::new(
        Some(Binders::new(&query.where_clauses, Vec::new())),
        as_of,
        valid,
    );
    let (planned, deferred) = optimizer::plan(query.where_clauses.clone());
    for (c, _) in planned {
        b.clause(c)?;
    }
    // `or` / `or-join` after the clauses the plan places, in text order, as
    // on the executor (spec §5.1).
    for c in &query.where_clauses {
        if matches!(c, WhereClause::Or(_) | WhereClause::OrJoin { .. }) {
            b.clause(c.clone())?;
        }
    }
    // An expression or a `not` whose variables only an `or` binds runs
    // last, expressions first, as on the executor (#625).
    for c in sorted(deferred) {
        b.clause(c)?;
    }

    // The parser rejects a variable no clause binds (PRS-080, PRS-022,
    // PRS-023); one only some `or` branches bind has no slot here.
    let slot = |var: &String| b.slots.by_var.get(var).copied();
    // The column of each `:find` element that is not a window.
    let mut cols: Vec<Option<usize>> = Vec::with_capacity(query.find.len());
    let aggregate = query
        .find
        .iter()
        .any(|s| matches!(s, FindSpec::Aggregate { .. }));
    let window = query.find.iter().any(|s| matches!(s, FindSpec::Window(_)));
    if aggregate {
        // The rows of `Aggregate`: the grouping cells (the plain variables
        // in `:find` order, then `:with`), then the aggregates. With
        // windows, `:with` does not group, as on the executor.
        let plain: Vec<&String> = query
            .find
            .iter()
            .filter_map(|s| match s {
                FindSpec::Variable(v) => Some(v),
                _ => None,
            })
            .collect();
        let with: &[String] = if window { &[] } else { &query.with_vars };
        let mut groups = Vec::new();
        for var in plain.iter().copied().chain(with) {
            groups.push(slot(var)?);
        }
        let (mut next_plain, mut next_agg) = (0, groups.len());
        let mut aggs = Vec::new();
        for spec in &query.find {
            match spec {
                FindSpec::Variable(_) => {
                    cols.push(Some(next_plain));
                    next_plain += 1;
                }
                FindSpec::Aggregate { func, var } => {
                    cols.push(Some(next_agg));
                    next_agg += 1;
                    aggs.push((func.clone(), slot(var)?));
                }
                FindSpec::Window(_) => cols.push(None),
            }
        }
        let zero_row = plain.is_empty()
            && aggs
                .iter()
                .all(|(f, _)| f == "count" || f == "count-distinct");
        b.steps.push(Step::Aggregate {
            groups,
            aggs,
            zero_row,
        });
    } else {
        for spec in &query.find {
            cols.push(match spec {
                FindSpec::Variable(var) => Some(slot(var)?),
                _ => None,
            });
        }
    }
    let mut find = Vec::with_capacity(query.find.len());
    if window {
        // A named variable's column: in an aggregate query, the plain
        // variable's column of `Aggregate` (the parser allows no other,
        // PRS-082); otherwise its slot.
        let column = |var: &String| {
            if aggregate {
                query
                    .find
                    .iter()
                    .zip(&cols)
                    .find(|(s, _)| matches!(s, FindSpec::Variable(v) if v == var))
                    .and_then(|(_, c)| *c)
            } else {
                slot(var)
            }
        };
        let named = named_vars(&query.find)
            .iter()
            .map(column)
            .collect::<Option<Vec<usize>>>()?;
        let keep: Vec<usize> = cols.iter().flatten().copied().collect();
        let (mut next_kept, mut next_window) = (0, keep.len());
        for c in &cols {
            if c.is_some() {
                find.push(next_kept);
                next_kept += 1;
            } else {
                find.push(next_window);
                next_window += 1;
            }
        }
        b.steps.push(Step::Window {
            find: query.find.clone(),
            named,
            keep,
        });
    } else {
        find = cols.into_iter().collect::<Option<Vec<usize>>>()?;
    }
    let mut plan = b.finish()?;
    plan.find = find;
    plan.vars = query.find.iter().map(FindSpec::display_name).collect();
    Some(plan)
}

/// Builds the operator steps of one clause list: a query's `:where`, or a
/// body.
struct Builder {
    slots: Slots,
    /// Which pattern or expression gives a variable its value; `None` in a
    /// body, where no join or expression replaces a cell (decision 10 of the
    /// #546 plan).
    binders: Option<Binders>,
    as_of: Option<u64>,
    valid: Valid,
    source: Option<Source>,
    steps: Vec<Step>,
    /// `not`s planned before the first pattern (they read no variable): they
    /// run right after it, which does not change the rows (decision 9).
    front: Vec<Step>,
    /// The stored-attribute patterns so far, for the `HashJoin` build side.
    outer_patterns: Vec<WhereClause>,
}

impl Builder {
    fn new(binders: Option<Binders>, as_of: Option<u64>, valid: Valid) -> Self {
        Builder {
            slots: Slots::default(),
            binders,
            as_of,
            valid,
            source: None,
            steps: Vec::new(),
            front: Vec::new(),
            outer_patterns: Vec::new(),
        }
    }

    fn finish(self) -> Option<Plan> {
        if !self.front.is_empty() {
            return None;
        }
        Some(Plan {
            source: self.source?,
            steps: self.steps,
            find: Vec::new(),
            vars: Vec::new(),
        })
    }

    /// Add the steps of `c`; `None` when the engine cannot run it.
    fn clause(&mut self, c: WhereClause) -> Option<()> {
        let (as_of, valid) = (self.as_of, self.valid);
        match c {
            WhereClause::Pattern(q) => {
                let rebind = match &mut self.binders {
                    Some(b) => b.pattern(&q),
                    None => Vec::new(),
                };
                let pseudo = meta(&q);
                if pseudo.is_some_and(|m| m != Meta::ValidAt) {
                    // Reads the hidden variable its source's scan filled.
                    let source = q.pseudo_source.as_ref()?.as_variable()?;
                    let col = *self.slots.by_var.get(source)?;
                    let value = term(&q.value, &mut self.slots, OTHER, pseudo_const)?;
                    self.steps.push(Step::Pseudo { col, value });
                    return Some(());
                }
                // A pattern with a stored attribute, or `:db/valid-at` read
                // as `[e _ _]` and the query's valid time.
                let any = EdnValue::Symbol("_".to_string());
                let (attribute, scanned) = match &q.attribute {
                    AttributeSpec::Real(a) => (a.clone(), q.clone()),
                    AttributeSpec::Pseudo(_) => (
                        any.clone(),
                        Pattern::new(q.entity.clone(), any.clone(), any.clone()),
                    ),
                };
                // The metadata columns: the hidden variables of a source, or
                // the valid time.
                let mut wants: Vec<(Meta, Option<&str>)> = q
                    .meta
                    .iter()
                    .map(|(a, v)| (meta_of(a), Some(v.as_str())))
                    .collect();
                if pseudo.is_some() {
                    wants.push((Meta::ValidAt, None));
                }
                let slots = &mut self.slots;
                let mut metas: Vec<(Meta, usize)> = Vec::new();
                if self.source.is_none() {
                    let e = term(&scanned.entity, slots, OTHER, entity_const)?;
                    let a = term(&attribute, slots, ATTRIBUTE, attribute_const)?;
                    let v = term(&scanned.value, slots, OTHER, value_const)?;
                    for &(m, var) in &wants {
                        let slot = slots.add(OTHER);
                        if let Some(var) = var {
                            slots.by_var.insert(var.to_string(), slot);
                        }
                        metas.push((m, slot));
                    }
                    self.source = Some(Source::Scan(ScanSpec {
                        e,
                        a,
                        v,
                        meta: metas.clone(),
                        as_of,
                        valid,
                        width: slots.len(),
                    }));
                    self.steps.append(&mut self.front);
                } else {
                    let mut join = join(&scanned, &attribute, slots, &rebind, as_of, valid)?;
                    for &(m, var) in &wants {
                        let local = join.inner.width;
                        join.inner.width += 1;
                        join.inner.meta.push((m, local));
                        join.new.push(local);
                        let slot = slots.add(OTHER);
                        if let Some(var) = var {
                            slots.by_var.insert(var.to_string(), slot);
                        }
                        metas.push((m, slot));
                    }
                    if let JoinKind::Hash { build_inner } = &mut join.kind {
                        // With the test switch off, always the inner pattern
                        // (decision 3 of the #596 plan). After a seed and no
                        // pattern, the seed (`branch_cost` of nothing is 0).
                        *build_inner = !optimizer::reorder()
                            || optimizer::branch_cost(&[WhereClause::Pattern(q.clone())])
                                <= optimizer::branch_cost(&self.outer_patterns);
                    }
                    self.steps.push(Step::Join(join));
                }
                self.outer_patterns.push(WhereClause::Pattern(q.clone()));
                if pseudo.is_some() {
                    let col = metas.last()?.1;
                    let value = term(&q.value, &mut self.slots, OTHER, pseudo_const)?;
                    self.steps.push(Step::Pseudo { col, value });
                }
            }
            WhereClause::Expr { expr, binding } => {
                // A clause before the first pattern sees no variable.
                self.source.as_ref()?;
                let vars = expr_vars(&expr, &self.slots);
                let step = match binding {
                    None => Step::Filter(expr, vars),
                    Some(out) => {
                        let replace = match &mut self.binders {
                            Some(b) => b.expression(&expr, &out),
                            None => false,
                        };
                        let (slot, mode) = match self.slots.by_var.get(&out) {
                            Some(&s) if replace => {
                                if let Some(k) = self.slots.kinds.get_mut(s) {
                                    *k |= OTHER;
                                }
                                (s, Out::Replace)
                            }
                            Some(&s) => (s, Out::Check),
                            None => {
                                let s = self.slots.add(OTHER);
                                self.slots.by_var.insert(out, s);
                                (s, Out::New)
                            }
                        };
                        Step::Bind(expr, vars, slot, mode)
                    }
                };
                self.steps.push(step);
            }
            WhereClause::Not(body) => {
                // Every variable the body names that the rows bind; the
                // others are the body's own (its sources' hidden variables).
                let imports = optimizer::clause_ref_vars(&body);
                let step = self.anti(&body, imports.into_iter().collect())?;
                self.push_anti(step);
            }
            WhereClause::NotJoin { join_vars, clauses } => {
                let step = self.anti(&clauses, join_vars)?;
                self.push_anti(step);
            }
            WhereClause::Or(branches) => self.union(&branches, None)?,
            WhereClause::OrJoin {
                join_vars,
                branches,
            } => self.union(&branches, Some(join_vars))?,
            _ => return None,
        }
        Some(())
    }

    fn push_anti(&mut self, step: Step) {
        if self.source.is_some() {
            self.steps.push(step);
        } else {
            self.front.push(step);
        }
    }

    /// The `AntiJoin` of `body`, whose variables `vars` the outer row gives
    /// where it binds them (spec §2).
    fn anti(&self, body: &[WhereClause], vars: Vec<String>) -> Option<Step> {
        if !body
            .iter()
            .all(|c| matches!(c, WhereClause::Pattern(_) | WhereClause::Expr { .. }))
        {
            return None;
        }
        Some(Step::Anti(self.sub(body, vars)?.0))
    }

    /// The `Union` and `Distinct` of an `or` (`join` is `None`) or an
    /// `or-join` (spec §5).
    fn union(&mut self, branches: &[Vec<WhereClause>], join: Option<Vec<String>>) -> Option<()> {
        if self.source.is_none() {
            // Nothing before the `or` binds a variable: its outer side is
            // the empty row.
            self.source = Some(Source::Seed);
            self.steps.append(&mut self.front);
        }
        let mut subs = Vec::with_capacity(branches.len());
        for branch in branches {
            let vars: Vec<String> = match &join {
                // The parser requires every join variable bound before
                // (INT-031 on the executor otherwise).
                Some(j) if j.iter().all(|v| self.slots.by_var.contains_key(v)) => j.clone(),
                Some(_) => return None,
                None => optimizer::clause_ref_vars(branch).into_iter().collect(),
            };
            let imports: HashSet<String> = vars
                .iter()
                .filter(|v| self.slots.by_var.contains_key(*v))
                .cloned()
                .collect();
            if !runs_branch(branch, &imports) {
                return None;
            }
            subs.push(self.sub(branch, vars)?);
        }
        // The variables a branch binds that the rows do not. Every branch of
        // an `or` binds the same (PRS-057), but their sources' hidden
        // variables differ; the executor keeps those, so such an `or` stays
        // there. An `or-join` keeps none.
        let mut names: Vec<String> = Vec::new();
        if join.is_none() {
            for (k, (_, slots)) in subs.iter().enumerate() {
                let mut own: Vec<String> = slots
                    .by_var
                    .keys()
                    .filter(|v| !self.slots.by_var.contains_key(*v))
                    .cloned()
                    .collect();
                own.sort();
                if k == 0 {
                    names = own;
                } else if own != names {
                    return None;
                }
            }
        }
        let mut arms = Vec::with_capacity(subs.len());
        let mut kinds = vec![0u8; names.len()];
        for (body, slots) in subs {
            let mut new = Vec::with_capacity(names.len());
            for (n, kind) in names.iter().zip(kinds.iter_mut()) {
                let inner = *slots.by_var.get(n)?;
                *kind |= slots.kind(inner);
                new.push(inner);
            }
            arms.push(Branch { body, new });
        }
        for (n, kind) in names.into_iter().zip(kinds) {
            let s = self.slots.add(kind);
            self.slots.by_var.insert(n, s);
        }
        self.steps.push(Step::Union(arms));
        let mut slots: Vec<usize> = self.slots.by_var.values().copied().collect();
        slots.sort_unstable();
        self.steps.push(Step::Distinct(slots));
        Some(())
    }

    /// The plan of a `not` body or an `or` branch whose variables `vars`
    /// the outer row gives where it binds them (spec §2), hashed or
    /// correlated (spec §4.4), and its slots.
    fn sub(&self, body: &[WhereClause], vars: Vec<String>) -> Option<(Body, Slots)> {
        let mut imports: Vec<(String, usize)> = vars
            .into_iter()
            .filter_map(|v| self.slots.by_var.get(&v).map(|&s| (v, s)))
            .collect();
        imports.sort_by_key(|&(_, s)| s);
        let seeded: HashSet<String> = imports.iter().map(|(v, _)| v.clone()).collect();
        let (seeded_order, seeded_deferred) = optimizer::plan_seeded(body.to_vec(), &seeded);
        // A probe per outer row holds nothing: correlated when the first
        // pattern's entity is imported.
        let probe = seeded_order
            .iter()
            .find_map(|(c, _)| match c {
                WhereClause::Pattern(p) if !per_fact(p) => Some(p),
                _ => None,
            })
            .and_then(|p| p.entity.as_variable())
            .is_some_and(|e| seeded.contains(e));
        let hashable = closed(body, &seeded);
        let hash = match optimizer::subplans() {
            optimizer::Subplans::Choose => hashable && !probe,
            optimizer::Subplans::Hash => hashable,
            optimizer::Subplans::Correlated => false,
        };
        let (as_of, valid) = (self.as_of, self.valid);
        if hash {
            let mut b = Builder::new(None, as_of, valid);
            let (planned, deferred) = optimizer::plan(body.to_vec());
            for (c, _) in planned {
                b.clause(c)?;
            }
            for c in sorted(deferred) {
                b.clause(c)?;
            }
            let keys = imports
                .iter()
                .filter_map(|(v, outer)| {
                    let inner = *b.slots.by_var.get(v)?;
                    let both =
                        self.slots.kind(*outer) == ATTRIBUTE && b.slots.kind(inner) == ATTRIBUTE;
                    Some(Key {
                        inner,
                        outer: *outer,
                        normalise: !both,
                    })
                })
                .collect();
            let slots = std::mem::take(&mut b.slots);
            let plan = Box::new(b.finish()?);
            return Some((Body::Hashed { plan, keys }, slots));
        }
        let mut b = Builder::new(None, as_of, valid);
        for (v, outer) in &imports {
            let s = b.slots.add(self.slots.kind(*outer));
            b.slots.by_var.insert(v.clone(), s);
        }
        b.source = Some(Source::Seed);
        for (c, _) in seeded_order {
            b.clause(c)?;
        }
        for c in sorted(seeded_deferred) {
            b.clause(c)?;
        }
        let slots = std::mem::take(&mut b.slots);
        let plan = Box::new(b.finish()?);
        Some((
            Body::Correlated {
                plan,
                imports: imports.into_iter().map(|(_, s)| s).collect(),
            },
            slots,
        ))
    }
}

/// Deferred `not`s, cheapest first when reordering, as on the executor.
fn sorted(mut deferred: Vec<WhereClause>) -> Vec<WhereClause> {
    if optimizer::reorder() {
        deferred.sort_by_key(optimizer::clause_cost);
    }
    deferred
}

/// Whether the engine runs `branch` of an `or` whose rows give `imports`
/// (decision 11 of the #546 plan). It holds patterns, expressions and
/// `not`s only (a rule invocation or a nested `or` stays on the executor),
/// and no predicate the registry may lack: the executor reports that only
/// when a row reaches it. The executor applies a branch's `not`s before its
/// expressions, so a `not` must not read a variable only an expression of
/// the branch binds.
fn runs_branch(branch: &[WhereClause], imports: &HashSet<String>) -> bool {
    let mut by_pattern: HashSet<String> = imports.clone();
    let mut by_expr: HashSet<&str> = HashSet::new();
    for c in branch {
        match c {
            WhereClause::Pattern(p) => by_pattern.extend(optimizer::pattern_bound_vars(p)),
            WhereClause::Expr {
                expr: Expr::UnaryOp(UnaryOp::Udf(_), _),
                ..
            } => return false,
            WhereClause::Expr { binding, .. } => by_expr.extend(binding.as_deref()),
            WhereClause::Not(_) | WhereClause::NotJoin { .. } => {}
            _ => return false,
        }
    }
    branch.iter().all(|c| {
        let reads: Vec<String> = match c {
            WhereClause::Not(body) => optimizer::clause_ref_vars(body).into_iter().collect(),
            WhereClause::NotJoin { join_vars, .. } => join_vars.clone(),
            _ => return true,
        };
        reads
            .iter()
            .all(|v| by_pattern.contains(v) || !by_expr.contains(v.as_str()))
    })
}

/// Whether `p` is a per-fact pseudo-attribute pattern (it scans nothing).
fn per_fact(p: &Pattern) -> bool {
    matches!(&p.attribute, AttributeSpec::Pseudo(a) if a.is_per_fact())
}

/// Whether `body` has the same rows for every outer row (spec §2): it has a
/// pattern to scan, every variable its expressions and per-fact
/// pseudo-attributes read is bound by one of its other patterns, and every
/// variable of `imports` a nested `not` reads is too (decision 6 of the #546
/// plan).
fn closed(body: &[WhereClause], imports: &HashSet<String>) -> bool {
    let bound: HashSet<String> = body
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) if !per_fact(p) => Some(optimizer::pattern_bound_vars(p)),
            _ => None,
        })
        .flatten()
        .collect();
    body.iter()
        .any(|c| matches!(c, WhereClause::Pattern(p) if !per_fact(p)))
        && body.iter().all(|c| match c {
            WhereClause::Expr { expr, .. } => {
                optimizer::expr_vars(expr).iter().all(|v| bound.contains(v))
            }
            WhereClause::Pattern(p) if per_fact(p) => p
                .pseudo_source
                .as_ref()
                .and_then(EdnValue::as_variable)
                .is_some_and(|v| bound.contains(v)),
            WhereClause::Not(inner) => optimizer::clause_ref_vars(inner)
                .iter()
                .all(|v| !imports.contains(v) || bound.contains(v)),
            WhereClause::NotJoin { join_vars, .. } => join_vars
                .iter()
                .all(|v| !imports.contains(v) || bound.contains(v)),
            _ => true,
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
    Some(meta_of(pa))
}

fn meta_of(pa: &PseudoAttr) -> Meta {
    match pa {
        PseudoAttr::ValidFrom => Meta::ValidFrom,
        PseudoAttr::ValidTo => Meta::ValidTo,
        PseudoAttr::TxCount => Meta::TxCount,
        PseudoAttr::TxId => Meta::TxId,
        PseudoAttr::ValidAt => Meta::ValidAt,
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
