//! Planner v2 (#473 spec §6.3): the operator tree of every query (#551).
//! A query the planner cannot build fails with a coded error: the executor's
//! code for the same query (INT-022, INT-025, INT-028, INT-031, INT-054), or
//! INT-032 for an invariant the parser keeps.
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
use super::relation::{Derived, Range};
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::{EntityId, Value};
use crate::query::datalog::aggregate::named_vars;
use crate::query::datalog::evaluator::rule_invocation_to_pattern;
use crate::query::datalog::executor::Binders;
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::optimizer;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::stratification::DependencyGraph;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, Count, DatalogQuery, EdnValue, Expr, FindSpec, Order, Pattern, PseudoAttr,
    UnaryOp, ValidAt, WhereClause,
};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A planner invariant that does not hold (#551 plan, decision 2).
fn invariant(what: &str) -> anyhow::Error {
    err_coded!(ErrorCode::Int032, what)
}

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
    /// A constant no cell can equal (#551 plan, decision 7): an entity that
    /// is not a keyword or UUID, an attribute that is not a keyword, a
    /// vector, list or map value. A scan with one reads nothing.
    Never,
}

/// A constant in the value position, with the matching rules the executor
/// had.
#[derive(Debug, Clone)]
pub(crate) enum ValueConst {
    /// A string, integer, boolean, `nil` or set: equal values only.
    Exact(Value),
    /// A keyword: the same keyword, or a ref to the entity it names.
    Keyword(String, EntityId),
    /// A UUID: a ref to it, or a keyword that names it.
    Uuid(EntityId),
    /// A float: floats equal by IEEE `==`, so `0.0` matches `-0.0` (#706).
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
            (ValueConst::Float(f), Value::Float(ff)) => f == ff,
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
    pub(crate) valid: Valid,
    /// Number of slots the scan fills.
    pub(crate) width: usize,
    /// In a rule query, the derived rows read after the stored facts
    /// (#548 spec §5.2).
    pub(crate) derived: Option<Derived>,
}

impl ScanSpec {
    /// Whether a position is a constant no cell equals: the scan reads
    /// nothing.
    pub(crate) fn never(&self) -> bool {
        matches!(self.e, Term::Never)
            || matches!(self.a, Term::Never)
            || matches!(self.v, Term::Never)
    }
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
    /// `[(collect-set ?e :a) ?s]` (#316 spec §7.3).
    CollectSet(CollectSpec),
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
    /// After the last step: the `:find` columns at slots `cols`, sorted by
    /// `order`, `(column of cols, descending)` (the full order), holding at
    /// most `keep` rows (#306).
    Sort {
        cols: Vec<usize>,
        order: Vec<(usize, bool)>,
        keep: Option<usize>,
    },
    /// After the last step, or `Sort`: skips `offset` rows, then passes on at
    /// most `limit` (#306).
    Limit { offset: usize, limit: Option<usize> },
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
    /// `IndexNestedLoopJoin` on the value (#721): the inner pattern's
    /// attribute is a constant and its value is `keys[key]`; per outer row,
    /// the AVET ranges of the values that join-equal it, and the pending
    /// facts with its key. When a probe cannot find a row's matches, a
    /// `HashJoin` built on the side `build_inner` names.
    ProbeValue { key: usize, build_inner: bool },
    /// `HashJoin`: a table of the side with the smaller estimate, probed by
    /// the other.
    Hash { build_inner: bool },
}

/// `[(collect-set ?e :a) ?s]`: per row, the values `inner` (`[e :a ?v]`,
/// probed by the cell at `entity`) finds, as one set bound to `out`.
#[derive(Debug, Clone)]
pub(crate) struct CollectSpec {
    pub(crate) entity: usize,
    pub(crate) attribute: String,
    pub(crate) inner: ScanSpec,
    pub(crate) out: usize,
    pub(crate) mode: Out,
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
    /// The query's `:as-of`; `None` for a body, which reads its query's.
    pub(crate) as_of: AsOfSpec,
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

/// A query's `:as-of`, resolved against the snapshot when the query opens
/// (`Snapshot::tx_bound`, #551 plan, decision 5).
#[derive(Debug, Clone, Copy)]
pub(crate) enum AsOfSpec {
    None,
    Counter(u64),
    /// Unix ms.
    Timestamp(i64),
}

/// The `:as-of` and valid-time test of `query`; a slot is resolved before
/// this.
fn temporal(query: &DatalogQuery) -> Result<(AsOfSpec, Valid)> {
    let as_of = match &query.as_of {
        None => AsOfSpec::None,
        Some(AsOf::Counter(n)) => AsOfSpec::Counter(*n),
        Some(AsOf::Timestamp(t)) => AsOfSpec::Timestamp(*t),
        // A prepared query substitutes every slot first.
        Some(AsOf::Slot(_)) => return Err(invariant("an :as-of slot reached the planner")),
    };
    let valid = match &query.valid_at {
        None => Valid::Now,
        Some(ValidAt::Timestamp(t)) => Valid::At(*t),
        Some(ValidAt::AnyValidTime) => Valid::Any,
        // As the executor reports one.
        Some(ValidAt::Slot(_)) => bail_coded!(ErrorCode::Int025),
    };
    Ok((as_of, valid))
}

/// The plan of `query`.
pub(crate) fn plan(query: &DatalogQuery) -> Result<Plan> {
    plan_with(query, None)
}

/// Add the steps of `clauses`, a `:where` or a rule body, to `b`: the
/// clauses `optimizer::plan` places, in its order, then the `or`s, then the
/// deferred ones. With `first`, that pattern runs first (a rule body's delta,
/// #548 spec §5.3).
fn clauses(
    b: &mut Builder,
    clauses: &[WhereClause],
    first: Option<&Pattern>,
    seeded: &HashSet<String>,
) -> Result<()> {
    if !clauses.iter().all(|c| {
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
        // A plain query has no rule invocation, and a rule body's are
        // patterns by now (`calls_as_patterns`).
        return Err(invariant("a rule invocation reached the planner"));
    }
    // With `first`, the rest are ordered after it, its variables bound, each
    // joining on a variable bound before it where one can (#694).
    let at = first.filter(|_| optimizer::reorder()).and_then(|p| {
        clauses
            .iter()
            .position(|c| matches!(c, WhereClause::Pattern(q) if q == p))
    });
    let (planned, deferred) = match at {
        Some(i) => {
            let mut rest = clauses.to_vec();
            let head = rest.remove(i);
            let seeded: HashSet<String> = match &head {
                WhereClause::Pattern(p) => optimizer::pattern_bound_vars(p).into_iter().collect(),
                _ => HashSet::new(),
            };
            let (mut planned, deferred) = optimizer::plan_after(rest, &seeded);
            planned.insert(0, (head, None));
            (planned, deferred)
        }
        None => optimizer::plan_seeded(clauses.to_vec(), seeded),
    };
    for (c, _) in planned {
        b.clause(c)?;
    }
    // `or` / `or-join` after the clauses the plan places, in text order, as
    // on the executor (spec §5.1).
    for c in clauses {
        if matches!(c, WhereClause::Or(_) | WhereClause::OrJoin { .. }) {
            b.clause(c.clone())?;
        }
    }
    // An expression or a `not` whose variables only an `or` binds runs
    // last, expressions first, as on the executor (#625).
    for c in sorted(deferred) {
        b.clause(c)?;
    }
    Ok(())
}

/// The plan of `query`, whose scans also read the relations `evaluated`
/// when it is a rule query (#548).
fn plan_with(query: &DatalogQuery, evaluated: Option<Arc<Evaluated>>) -> Result<Plan> {
    let (as_of, valid) = temporal(query)?;
    let mut b = Builder::new(
        Some(Binders::new(&query.where_clauses, Vec::new())),
        valid,
        evaluated,
    );
    clauses(&mut b, &query.where_clauses, None, &HashSet::new())?;

    // The parser rejects a variable no clause binds (PRS-080, PRS-022,
    // PRS-023).
    let slot = |var: &String| {
        b.slots
            .by_var
            .get(var)
            .copied()
            .ok_or_else(|| invariant("a :find variable without a slot"))
    };
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
                    .ok_or_else(|| invariant("a window's variable without a column"))
            } else {
                slot(var)
            }
        };
        let named = named_vars(&query.find)
            .iter()
            .map(column)
            .collect::<Result<Vec<usize>>>()?;
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
        find = cols
            .into_iter()
            .collect::<Option<Vec<usize>>>()
            .ok_or_else(|| invariant("a window outside a window query"))?;
    }
    let mut plan = b.finish()?;
    plan.find = find;
    // After `finish`, which may still add the steps of a clause list with
    // no pattern. `execute` rejects an unbound slot (API-010); a prepared
    // query fills it.
    let count = |c: &Option<Count>, option: &str| match c {
        None => Ok(None),
        Some(Count::Literal(n)) => Ok(Some(*n)),
        Some(Count::Slot(_)) => Err(invariant(&format!("a {option} slot reached the planner"))),
    };
    let (limit, offset) = (
        count(&query.limit, ":limit")?,
        count(&query.offset, ":offset")?,
    );
    let offset = offset.unwrap_or(0);
    if !query.order_by.is_empty() {
        // The full order (#306 spec §2): the keys, then every other `:find`
        // column ascending.
        let mut order = Vec::with_capacity(query.find.len());
        for (spec, dir) in &query.order_by {
            let col = query
                .find
                .iter()
                .position(|s| s == spec)
                .ok_or_else(|| invariant("an :order-by entry outside :find"))?;
            order.push((col, matches!(dir, Order::Desc)));
        }
        for col in 0..query.find.len() {
            if !order.iter().any(|&(c, _)| c == col) {
                order.push((col, false));
            }
        }
        plan.steps.push(Step::Sort {
            cols: std::mem::take(&mut plan.find),
            order,
            keep: limit.map(|l| offset.saturating_add(l)),
        });
        plan.find = (0..query.find.len()).collect();
    }
    if limit.is_some() || offset > 0 {
        plan.steps.push(Step::Limit { offset, limit });
    }
    plan.vars = query.find.iter().map(FindSpec::display_name).collect();
    plan.as_of = as_of;
    Ok(plan)
}

/// A rule query on the engine (#548 spec §5): the relations, the rules of
/// each stratum in ascending order, and the query body.
#[derive(Debug, Clone)]
pub(crate) struct RulesPlan {
    pub(crate) evaluated: Arc<Evaluated>,
    /// Each stratum with rules: its relations, and its rules.
    pub(crate) strata: Vec<(Vec<usize>, Vec<RulePlan>)>,
    pub(crate) as_of: AsOfSpec,
    pub(crate) valid: Valid,
    pub(crate) body: Plan,
    /// The predicates the evaluated rules' bodies name, in `plan_rules`'
    /// predicate order, then text order (#551 plan, decision 9).
    pub(crate) predicates: Vec<String>,
}

/// One rule: its runs in round 1 and the rounds after (spec §5.3).
#[derive(Debug, Clone)]
pub(crate) struct RulePlan {
    /// The relation of its head.
    pub(crate) rel: usize,
    pub(crate) predicate: String,
    /// Every read `Full`: round 1, and every round of a naive rule.
    pub(crate) full: RuleRun,
    /// One run per top-level invocation of its stratum, for rounds after
    /// the first; empty for a rule without one, which runs in round 1 only.
    pub(crate) deltas: Vec<RuleRun>,
    pub(crate) naive: bool,
}

/// A rule body's plan and where its rows hold the head's arguments.
#[derive(Debug, Clone)]
pub(crate) struct RuleRun {
    pub(crate) plan: Plan,
    pub(crate) entity: HeadArg,
    /// `None` for a one-argument head, whose value is `true`.
    pub(crate) value: Option<HeadArg>,
}

/// A head argument.
#[derive(Debug, Clone)]
pub(crate) enum HeadArg {
    Slot(usize),
    /// A variable the body does not bind (`INT-022` once a row reaches it).
    Unbound(String),
    /// A constant: only the magic-set rewrite writes one (PRS-114 rejects
    /// it in a rule the user defines, #608).
    Const(EdnValue),
}

/// The plan of a rule query: `query` and every rule it evaluates in
/// `registry` (the magic-set rewrite's, when it ran), whose relations also
/// hold the attributes `seeds` (magic seeds).
pub(crate) fn plan_rules(
    query: &DatalogQuery,
    registry: &RuleRegistry,
    seeds: &[&str],
) -> Result<RulesPlan> {
    let (as_of, valid) = temporal(query)?;
    let strata = DependencyGraph::from_rules(registry).stratify()?;
    // Every predicate the query invokes, and those their rules invoke, in
    // the order the executor collected them.
    let mut preds: Vec<String> = Vec::new();
    for (p, _) in query.get_rule_invocations() {
        if !preds.contains(&p) {
            preds.push(p);
        }
    }
    let mut i = 0;
    while let Some(p) = preds.get(i).cloned() {
        for rule in registry.get_rules(&p) {
            for c in &rule.body {
                for dep in c.rule_invocations() {
                    if !preds.iter().any(|q| q == dep) {
                        preds.push(dep.to_string());
                    }
                }
            }
        }
        i += 1;
    }
    let mut predicates = Vec::new();
    for p in &preds {
        for rule in registry.get_rules(p) {
            udfs(&rule.body, &mut predicates);
        }
    }
    let mut attrs: Vec<String> = preds.iter().map(|p| format!(":{p}")).collect();
    for a in seeds {
        if !attrs.iter().any(|b| b == a) {
            attrs.push((*a).to_string());
        }
    }
    let evaluated = Arc::new(Evaluated {
        by_attr: attrs
            .iter()
            .enumerate()
            .map(|(i, a)| (a.clone(), i))
            .collect(),
        attrs,
    });
    let stratum = |p: &str| strata.get(p).copied().unwrap_or(0);
    let top = preds.iter().map(|p| stratum(p)).max().unwrap_or(0);
    let mut planned = Vec::new();
    for s in 0..=top {
        let in_stratum: HashSet<String> = preds
            .iter()
            .filter(|p| stratum(p) == s)
            .map(|p| format!(":{p}"))
            .collect();
        let mut rels = Vec::new();
        let mut rules = Vec::new();
        for (rel, p) in preds.iter().enumerate().filter(|(_, p)| stratum(p) == s) {
            rels.push(rel);
            for rule in registry.get_rules(p) {
                rules.push(rule_plan(
                    &rule.head,
                    &rule.body,
                    rel,
                    p,
                    &in_stratum,
                    valid,
                    &evaluated,
                )?);
            }
        }
        if !rels.is_empty() {
            planned.push((rels, rules));
        }
    }
    let mut body_query = query.clone();
    body_query.where_clauses = calls_as_patterns(&query.where_clauses)?;
    let body = plan_with(&body_query, Some(evaluated.clone()))?;
    Ok(RulesPlan {
        evaluated,
        strata: planned,
        as_of,
        valid,
        body,
        predicates,
    })
}

/// Append the predicates `clauses` name, at any depth, in text order.
pub(crate) fn udfs(clauses: &[WhereClause], out: &mut Vec<String>) {
    for c in clauses {
        match c {
            WhereClause::Expr {
                expr: Expr::UnaryOp(UnaryOp::Udf(name), _),
                ..
            } => out.push(name.clone()),
            WhereClause::Not(body) | WhereClause::NotJoin { clauses: body, .. } => udfs(body, out),
            WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
                for b in branches {
                    udfs(b, out);
                }
            }
            _ => {}
        }
    }
}

/// The plan of one rule of predicate `predicate` (relation `rel`), whose
/// stratum holds the relations of the attributes `stratum`.
#[allow(clippy::too_many_arguments)]
fn rule_plan(
    head: &[EdnValue],
    body: &[WhereClause],
    rel: usize,
    predicate: &str,
    stratum: &HashSet<String>,
    valid: Valid,
    evaluated: &Arc<Evaluated>,
) -> Result<RulePlan> {
    // As on the executor: a head without arguments is an error, and
    // arguments after the second are ignored.
    let Some(entity) = head.get(1) else {
        bail_coded!(
            ErrorCode::Int022,
            "Rule head must have at least 2 elements: (predicate ?arg1)"
        );
    };
    let naive = naive(body, stratum, false);
    let clauses = calls_as_patterns(body)?;
    // The top-level invocations of the stratum, in text order.
    let own: Vec<Pattern> = body
        .iter()
        .filter_map(|c| match c {
            WhereClause::RuleInvocation { predicate, args }
                if stratum.contains(&format!(":{predicate}")) =>
            {
                rule_invocation_to_pattern(predicate, args).ok()
            }
            _ => None,
        })
        .collect();
    let run = |ranges: Vec<(Pattern, Range)>, first: Option<&Pattern>| -> Result<RuleRun> {
        let mut b = Builder::new(None, valid, Some(evaluated.clone()));
        b.ranges = ranges;
        self::clauses(&mut b, &clauses, first, &HashSet::new())?;
        let slots = std::mem::take(&mut b.slots.by_var);
        let arg = |e: &EdnValue| match e {
            EdnValue::Symbol(v) if v.starts_with('?') => match slots.get(v) {
                Some(&s) => HeadArg::Slot(s),
                None => HeadArg::Unbound(v.clone()),
            },
            c => HeadArg::Const(c.clone()),
        };
        Ok(RuleRun {
            entity: arg(entity),
            value: head.get(2).map(arg),
            plan: b.finish()?,
        })
    };
    let full = run(Vec::new(), None)?;
    let mut deltas = Vec::new();
    if !naive {
        for (i, delta) in own.iter().enumerate() {
            let mut ranges: Vec<(Pattern, Range)> = own
                .iter()
                .enumerate()
                .map(|(j, p)| {
                    let range = match j.cmp(&i) {
                        std::cmp::Ordering::Less => Range::Old,
                        std::cmp::Ordering::Equal => Range::Delta,
                        std::cmp::Ordering::Greater => Range::Full,
                    };
                    (p.clone(), range)
                })
                .collect();
            // The delta's range first: the pattern planned first takes it
            // even when an equal invocation comes before it (#694).
            let d = ranges.remove(i);
            ranges.insert(0, d);
            deltas.push(run(ranges, Some(delta))?);
        }
    }
    Ok(RulePlan {
        rel,
        predicate: predicate.to_string(),
        full,
        deltas,
        naive,
    })
}

/// Whether a rule body reads a relation of its stratum (the attributes
/// `stratum`) other than through a top-level invocation, outside `not` and
/// `not-join` (spec §5.3): it is then evaluated in full every round.
fn naive(body: &[WhereClause], stratum: &HashSet<String>, in_or: bool) -> bool {
    body.iter().any(|c| match c {
        WhereClause::Pattern(p) => match &p.attribute {
            AttributeSpec::Real(EdnValue::Keyword(a)) => stratum.contains(a),
            AttributeSpec::Real(_) => true,
            AttributeSpec::Pseudo(pa) => !pa.is_per_fact(),
        },
        WhereClause::RuleInvocation { predicate, .. } => {
            in_or && stratum.contains(&format!(":{predicate}"))
        }
        WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
            branches.iter().any(|b| naive(b, stratum, true))
        }
        WhereClause::Not(_) | WhereClause::NotJoin { .. } | WhereClause::Expr { .. } => false,
    })
}

/// `clauses` with every rule invocation, at any depth, read as its pattern
/// (`rule_invocation_to_pattern`), as the executor matches it.
fn calls_as_patterns(clauses: &[WhereClause]) -> Result<Vec<WhereClause>> {
    let all = |branches: &[Vec<WhereClause>]| {
        branches
            .iter()
            .map(|b| calls_as_patterns(b))
            .collect::<Result<Vec<_>>>()
    };
    clauses
        .iter()
        .map(|c| {
            Ok(match c {
                WhereClause::RuleInvocation { predicate, args } => {
                    WhereClause::Pattern(rule_invocation_to_pattern(predicate, args)?)
                }
                WhereClause::Not(body) => WhereClause::Not(calls_as_patterns(body)?),
                WhereClause::NotJoin { join_vars, clauses } => WhereClause::NotJoin {
                    join_vars: join_vars.clone(),
                    clauses: calls_as_patterns(clauses)?,
                },
                WhereClause::Or(branches) => WhereClause::Or(all(branches)?),
                WhereClause::OrJoin {
                    join_vars,
                    branches,
                } => WhereClause::OrJoin {
                    join_vars: join_vars.clone(),
                    branches: all(branches)?,
                },
                c => c.clone(),
            })
        })
        .collect()
}

/// Builds the operator steps of one clause list: a query's `:where`, or a
/// body.
struct Builder {
    slots: Slots,
    /// Which pattern or expression gives a variable its value; `None` in a
    /// body, where no join or expression replaces a cell (decision 10 of the
    /// #546 plan).
    binders: Option<Binders>,
    valid: Valid,
    source: Option<Source>,
    steps: Vec<Step>,
    /// `not`s planned before the first pattern (they read no variable): they
    /// run right after it, which does not change the rows (decision 9).
    front: Vec<Step>,
    /// The stored-attribute patterns so far, for the `HashJoin` build side.
    outer_patterns: Vec<WhereClause>,
    /// In a rule query, the evaluated predicates every scan also reads
    /// (#548 spec §5.2); bodies inherit them.
    evaluated: Option<Arc<Evaluated>>,
    /// The ranges of this clause list's own patterns that do not read
    /// `Full`, taken by the first equal pattern planned (a rule body's
    /// top-level invocations, spec §5.3). Equal patterns may take each
    /// other's range: a join does not depend on their order.
    ranges: Vec<(Pattern, Range)>,
}

/// The relations of a rule query, by attribute.
#[derive(Debug, Default)]
pub(crate) struct Evaluated {
    /// The relation of each evaluated predicate `p`, by `:p`.
    pub(crate) by_attr: HashMap<String, usize>,
    /// The attributes of the relations, in relation order.
    pub(crate) attrs: Vec<String>,
}

impl Evaluated {
    /// What a scan of `attribute` in pattern `q` reads of the relations.
    fn derived(
        &self,
        q: &Pattern,
        attribute: &EdnValue,
        ranges: &mut Vec<(Pattern, Range)>,
    ) -> Option<Derived> {
        match attribute {
            EdnValue::Keyword(a) => {
                let rel = *self.by_attr.get(a)?;
                let range = match ranges.iter().position(|(p, _)| p == q) {
                    Some(i) => ranges.swap_remove(i).1,
                    None => Range::Full,
                };
                Some(Derived {
                    rels: vec![rel],
                    range,
                })
            }
            _ => Some(Derived {
                rels: (0..self.attrs.len()).collect(),
                range: Range::Full,
            }),
        }
    }
}

impl Builder {
    fn new(binders: Option<Binders>, valid: Valid, evaluated: Option<Arc<Evaluated>>) -> Self {
        Builder {
            slots: Slots::default(),
            binders,
            valid,
            source: None,
            steps: Vec::new(),
            front: Vec::new(),
            outer_patterns: Vec::new(),
            evaluated,
            ranges: Vec::new(),
        }
    }

    fn finish(mut self) -> Result<Plan> {
        // `not`s and no pattern: they run on the empty row (#551).
        self.seed();
        let source = self.source.unwrap_or(Source::Seed);
        Ok(Plan {
            source,
            steps: self.steps,
            find: Vec::new(),
            vars: Vec::new(),
            as_of: AsOfSpec::None,
        })
    }

    /// Add the steps of `c`.
    fn clause(&mut self, c: WhereClause) -> Result<()> {
        let valid = self.valid;
        match c {
            WhereClause::Pattern(q) => {
                let rebind = match &mut self.binders {
                    Some(b) => b.pattern(&q),
                    None => Vec::new(),
                };
                let pseudo = meta(&q);
                if pseudo.is_some_and(|m| m != Meta::ValidAt) {
                    // Reads the hidden variable its source's scan filled; the
                    // parser names one (PRS-081 otherwise).
                    let col = q
                        .pseudo_source
                        .as_ref()
                        .and_then(EdnValue::as_variable)
                        .and_then(|source| self.slots.by_var.get(source))
                        .copied()
                        .ok_or_else(|| invariant("a pseudo-attribute without its source"))?;
                    let value = term(&q.value, &mut self.slots, OTHER, pseudo_const)?;
                    self.steps.push(Step::Pseudo { col, value });
                    return Ok(());
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
                let derived = match &self.evaluated {
                    Some(e) => e.derived(&q, &attribute, &mut self.ranges),
                    None => None,
                };
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
                        valid,
                        width: slots.len(),
                        derived,
                    }));
                    self.steps.append(&mut self.front);
                } else {
                    let mut join = join(&scanned, &attribute, slots, &rebind, valid)?;
                    // Relations are indexed by entity only: a value join on
                    // one hashes (#721).
                    if derived.is_some() && matches!(join.kind, JoinKind::ProbeValue { .. }) {
                        join.kind = JoinKind::Hash { build_inner: true };
                    }
                    join.inner.derived = derived;
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
                    if let JoinKind::Hash { build_inner }
                    | JoinKind::ProbeValue { build_inner, .. } = &mut join.kind
                    {
                        // With the test switch off, always the inner pattern
                        // (decision 3 of the #596 plan). After a seed and no
                        // pattern, the seed (both costs of nothing are 0).
                        // With no key, the outer side is up to the product
                        // of its patterns (#549 spec §6.1).
                        let outer = if join.keys.is_empty() {
                            optimizer::product_cost(&self.outer_patterns)
                        } else {
                            optimizer::branch_cost(&self.outer_patterns)
                        };
                        *build_inner = !optimizer::reorder()
                            || optimizer::branch_cost(&[WhereClause::Pattern(q.clone())]) <= outer;
                    }
                    self.steps.push(Step::Join(join));
                }
                self.outer_patterns.push(WhereClause::Pattern(q.clone()));
                if pseudo.is_some() {
                    let col = metas
                        .last()
                        .ok_or_else(|| invariant("a valid-at column without a slot"))?
                        .1;
                    let value = term(&q.value, &mut self.slots, OTHER, pseudo_const)?;
                    self.steps.push(Step::Pseudo { col, value });
                }
            }
            WhereClause::Expr { expr, binding } => {
                // A clause before the first pattern sees no variable: it
                // runs once, on the empty row (#551).
                self.seed();
                let vars = expr_vars(&expr, &self.slots);
                let step = match binding {
                    None => Step::Filter(expr, vars),
                    Some(out) => {
                        let collect = match &expr {
                            Expr::CollectSet { entity, attribute } => Some((
                                self.slots.by_var.get(entity).copied().ok_or_else(|| {
                                    invariant("a collect-set entity bound by no earlier clause")
                                })?,
                                attribute.clone(),
                            )),
                            _ => None,
                        };
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
                        match collect {
                            // Stored facts only, never a rule's relation
                            // (spec §7.1).
                            Some((entity, attribute)) => Step::CollectSet(CollectSpec {
                                entity,
                                inner: ScanSpec {
                                    e: Term::Bind(0),
                                    a: Term::Const(attribute.clone()),
                                    v: Term::Bind(1),
                                    meta: Vec::new(),
                                    valid,
                                    width: 2,
                                    derived: None,
                                },
                                attribute,
                                out: slot,
                                mode,
                            }),
                            None => Step::Bind(expr, vars, slot, mode),
                        }
                    }
                };
                self.steps.push(step);
            }
            WhereClause::Not(body) => {
                // Every variable the body names that the rows bind; the
                // others are the body's own (its sources' hidden variables).
                let imports = reads(&body);
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
            WhereClause::RuleInvocation { .. } => {
                return Err(invariant("a rule invocation reached the planner"));
            }
        }
        Ok(())
    }

    /// Start from the empty row if nothing was planned to scan first, and
    /// run the `not`s planned before it.
    fn seed(&mut self) {
        if self.source.is_none() {
            self.source = Some(Source::Seed);
            self.steps.append(&mut self.front);
        }
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
    fn anti(&self, body: &[WhereClause], vars: Vec<String>) -> Result<Step> {
        Ok(Step::Anti(self.sub(body, vars)?.0))
    }

    /// The `Union` and `Distinct` of an `or` (`join` is `None`) or an
    /// `or-join` (spec §5).
    fn union(&mut self, branches: &[Vec<WhereClause>], join: Option<Vec<String>>) -> Result<()> {
        // Nothing before the `or` binds a variable: its outer side is the
        // empty row.
        self.seed();
        let mut subs = Vec::with_capacity(branches.len());
        for branch in branches {
            let vars: Vec<String> = match &join {
                // The parser requires every join variable bound before
                // (INT-031 on the executor otherwise).
                Some(j) if j.iter().all(|v| self.slots.by_var.contains_key(v)) => j.clone(),
                // The parser rejects it first (INT-018).
                Some(j) => {
                    let v = j.iter().find(|v| !self.slots.by_var.contains_key(*v));
                    bail_coded!(ErrorCode::Int031, v.map_or("?", String::as_str));
                }
                None => reads(branch).into_iter().collect(),
            };
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
                    // The parser rejects it first (PRS-057).
                    return Err(invariant("or branches bind different variables"));
                }
            }
        }
        let mut arms = Vec::with_capacity(subs.len());
        let mut kinds = vec![0u8; names.len()];
        for (body, slots) in subs {
            let mut new = Vec::with_capacity(names.len());
            for (n, kind) in names.iter().zip(kinds.iter_mut()) {
                let inner = *slots
                    .by_var
                    .get(n)
                    .ok_or_else(|| invariant("a branch without an `or` variable"))?;
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
        Ok(())
    }

    /// The plan of a `not` body or an `or` branch whose variables `vars`
    /// the outer row gives where it binds them (spec §2), hashed or
    /// correlated (spec §4.4), and its slots.
    fn sub(&self, body: &[WhereClause], vars: Vec<String>) -> Result<(Body, Slots)> {
        let mut imports: Vec<(String, usize)> = vars
            .into_iter()
            .filter_map(|v| self.slots.by_var.get(&v).map(|&s| (v, s)))
            .collect();
        imports.sort_by_key(|&(_, s)| s);
        let seeded: HashSet<String> = imports.iter().map(|(v, _)| v.clone()).collect();
        let (seeded_order, _) = optimizer::plan_seeded(body.to_vec(), &seeded);
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
        let valid = self.valid;
        if hash {
            let mut b = Builder::new(None, valid, self.evaluated.clone());
            clauses(&mut b, body, None, &HashSet::new())?;
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
            return Ok((Body::Hashed { plan, keys }, slots));
        }
        let mut b = Builder::new(None, valid, self.evaluated.clone());
        for (v, outer) in &imports {
            let s = b.slots.add(self.slots.kind(*outer));
            b.slots.by_var.insert(v.clone(), s);
        }
        b.source = Some(Source::Seed);
        clauses(&mut b, body, None, &seeded)?;
        let slots = std::mem::take(&mut b.slots);
        let plan = Box::new(b.finish()?);
        Ok((
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

/// Every variable `clauses` name, at any depth: an `or-join` names its join
/// variables only. What a `not` body or `or` branch may import.
fn reads(clauses: &[WhereClause]) -> HashSet<String> {
    let mut vars = HashSet::new();
    for c in clauses {
        match c {
            WhereClause::Or(branches) => vars.extend(branches.iter().flat_map(|b| reads(b))),
            WhereClause::OrJoin { join_vars, .. } => vars.extend(join_vars.iter().cloned()),
            WhereClause::Not(body) => vars.extend(reads(body)),
            WhereClause::NotJoin { join_vars, .. } => vars.extend(join_vars.iter().cloned()),
            c => vars.extend(optimizer::clause_ref_vars(std::slice::from_ref(c))),
        }
    }
    vars
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
            WhereClause::Not(inner) => reads(inner)
                .iter()
                .all(|v| !imports.contains(v) || bound.contains(v)),
            WhereClause::NotJoin { join_vars, .. } => join_vars
                .iter()
                .all(|v| !imports.contains(v) || bound.contains(v)),
            c @ (WhereClause::Or(_) | WhereClause::OrJoin { .. }) => reads(std::slice::from_ref(c))
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
    valid: Valid,
) -> Result<JoinSpec> {
    let outer_width = slots.len();
    let mut local = Slots::default();
    let e = term(&q.entity, &mut local, OTHER, entity_const)?;
    let a = term(attribute, &mut local, ATTRIBUTE, attribute_const)?;
    let v = term(&q.value, &mut local, OTHER, value_const)?;

    let mut keys = Vec::new();
    let mut new = Vec::new();
    let mut replace = Vec::new();
    let mut probe = None;
    let mut probe_value = None;
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
                } else if position == 2
                    && matches!(a, Term::Const(_))
                    && !matches!(e, Term::Const(_))
                {
                    probe_value = Some(keys.len());
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
    Ok(JoinSpec {
        inner: ScanSpec {
            e,
            a,
            v,
            meta: Vec::new(),
            valid,
            width: local.len(),
            derived: None,
        },
        kind: match (probe, probe_value) {
            (Some(k), _) => JoinKind::Probe(k),
            (None, Some(k)) => JoinKind::ProbeValue {
                key: k,
                build_inner: true,
            },
            (None, None) => JoinKind::Hash { build_inner: true },
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
        // Equal sets only: no keyword-ref or float rule inside a set (#316).
        EdnValue::Set(_) => {
            ValueConst::Exact(crate::query::datalog::matcher::edn_to_value(c).ok()?)
        }
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
            Expr::CollectSet { entity, .. } => walk(&Expr::Var(entity.clone()), slots, out),
        }
    }
    let mut out = Vec::new();
    walk(expr, slots, &mut out);
    out
}

/// The term of one pattern position: a variable, a constant `konst` takes,
/// or `Never` for one it does not.
fn term<C>(
    edn: &EdnValue,
    slots: &mut Slots,
    kind: u8,
    konst: impl Fn(&EdnValue) -> Option<C>,
) -> Result<Term<C>> {
    match edn {
        EdnValue::Symbol(s) if s == "_" || s.starts_with("?_") => Ok(Term::Any),
        EdnValue::Symbol(s) if s.starts_with('?') => Ok(match slots.by_var.get(s) {
            Some(&slot) => Term::Eq(slot),
            None => {
                let slot = slots.add(kind);
                slots.by_var.insert(s.clone(), slot);
                Term::Bind(slot)
            }
        }),
        // The parser reads no other symbol in a pattern.
        EdnValue::Symbol(_) => Err(invariant("a symbol that is not a variable")),
        c => Ok(konst(c).map_or(Term::Never, Term::Const)),
    }
}
