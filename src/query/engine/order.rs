//! Snapshot-bound input-order proofs (#641). DICT order is never UUID/text
//! order. Unknown order retains the original index choice and buffering.

use super::plan::{Body, JoinKind, Meta, Out, Plan, ScanSpec, Source, Step, Term, Vars};
use crate::graph::storage::Snapshot;
use crate::query::datalog::functions::{AggImpl, FunctionRegistry};
use crate::query::datalog::types::{Expr, FindSpec, UnaryOp, WindowFunc};
use crate::storage::KeySource;
use crate::storage::keys::{self, Index, ValueKind};
use anyhow::Result;

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
thread_local! {
    /// Public write-transaction reads return rows rather than a cursor; tests
    /// use this marker to verify which root strategy they actually opened.
    pub(super) static OPTIMIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn enabled() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    return ENABLED.with(std::cell::Cell::get);
    #[cfg(not(any(test, feature = "test-hooks")))]
    true
}

/// Force the original strategy for differential tests only.
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn set_enabled(on: bool) -> InputOrderGuard {
    InputOrderGuard(ENABLED.with(|v| v.replace(on)))
}

#[cfg(any(test, feature = "test-hooks"))]
#[must_use = "input-order optimization is restored when the guard drops"]
/// Restores this thread's previous input-order test setting on drop.
pub struct InputOrderGuard(bool);

#[cfg(any(test, feature = "test-hooks"))]
impl Drop for InputOrderGuard {
    fn drop(&mut self) {
        ENABLED.with(|v| v.set(self.0));
    }
}

/// Only the leading scalar slot has a semantic comparator. The following
/// entity key is DICT order and cannot prove any Sort column in this slice.
struct Ordering {
    slot: Option<usize>,
    kind: ValueKind,
    constants: Vec<usize>,
}

impl Ordering {
    fn source(spec: &ScanSpec, kind: ValueKind) -> Self {
        Ordering {
            slot: match spec.v {
                Term::Bind(s) => Some(s),
                _ => None,
            },
            kind,
            constants: spec
                .meta
                .iter()
                .filter_map(|&(m, s)| (m == Meta::ValidAt).then_some(s))
                .collect(),
        }
    }

    fn invalidate(&mut self, slot: usize) {
        if self.slot == Some(slot) {
            self.slot = None;
        }
        self.constants.retain(|&s| s != slot);
    }

    fn constant(&self, expr: &Expr, vars: &Vars) -> bool {
        match expr {
            Expr::Lit(_) => true,
            Expr::Var(v) => vars
                .iter()
                .any(|(name, s)| name == v && self.constants.contains(s)),
            Expr::BinOp(_, a, b) => self.constant(a, vars) && self.constant(b, vars),
            Expr::UnaryOp(_, a) => self.constant(a, vars),
            Expr::Slot(_) | Expr::CollectSet { .. } => false,
        }
    }

    fn through(mut self, steps: &[Step]) -> Option<Self> {
        for step in steps {
            match step {
                Step::Filter(..) | Step::Anti(_) | Step::Distinct(_) | Step::Limit { .. } => {}
                Step::Bind(expr, vars, slot, mode) => {
                    let constant = self.constant(expr, vars);
                    if *mode == Out::Replace {
                        self.invalidate(*slot);
                    }
                    if *mode != Out::Check && constant {
                        self.constants.push(*slot);
                    }
                }
                Step::Pseudo { col, value } => {
                    if let Term::Bind(slot) = value
                        && self.constants.contains(col)
                    {
                        self.constants.push(*slot);
                    }
                }
                Step::Join(join) => {
                    if !matches!(
                        join.kind,
                        JoinKind::Probe(_)
                            | JoinKind::ProbeValue {
                                build_inner: true,
                                ..
                            }
                    ) {
                        return None;
                    }
                    for &(_, slot) in &join.replace {
                        self.invalidate(slot);
                    }
                }
                Step::DistinctFind(slots) => {
                    if self.kind == ValueKind::Float
                        && let Some(slot) = self.slot.filter(|s| slots.contains(s))
                    {
                        self.invalidate(slot);
                    }
                }
                Step::Union(_)
                | Step::CollectSet(_)
                | Step::Aggregate { .. }
                | Step::Window { .. }
                | Step::Sort { .. } => return None,
            }
        }
        Some(self)
    }

    fn satisfies(&self, cols: &[usize], order: &[(usize, bool)]) -> bool {
        let mut matched = false;
        for &(column, desc) in order {
            let Some(&slot) = cols.get(column) else {
                return false;
            };
            if self.constants.contains(&slot) || (matched && self.slot == Some(slot)) {
                continue;
            }
            if matched
                || desc
                || self.slot != Some(slot)
                || !matches!(
                    self.kind,
                    ValueKind::Integer | ValueKind::Float | ValueKind::Boolean
                )
            {
                return false;
            }
            matched = true;
        }
        matched
    }
}

fn safe_expr(expr: &Expr) -> bool {
    match expr {
        Expr::UnaryOp(UnaryOp::Udf(_), _) | Expr::CollectSet { .. } => false,
        Expr::UnaryOp(_, a) => safe_expr(a),
        Expr::BinOp(_, a, b) => safe_expr(a) && safe_expr(b),
        Expr::Lit(_) | Expr::Var(_) | Expr::Slot(_) => true,
    }
}

fn safe_find(find: &[FindSpec], functions: &FunctionRegistry) -> bool {
    find.iter().all(|f| match f {
        FindSpec::Variable(_) => true,
        FindSpec::Aggregate { func, .. } => functions
            .get(func)
            .is_some_and(|d| matches!(d.impl_, AggImpl::Builtin(_))),
        FindSpec::Window(w) => !matches!(w.func, WindowFunc::Udf(_)),
    })
}

fn safe_body(body: &Body, functions: &FunctionRegistry) -> bool {
    match body {
        Body::Correlated { plan, .. } | Body::Hashed { plan, .. } => {
            safe_steps(&plan.steps, functions)
        }
    }
}

fn safe_steps(steps: &[Step], functions: &FunctionRegistry) -> bool {
    steps.iter().all(|s| match s {
        Step::Filter(expr, _) | Step::Bind(expr, ..) => safe_expr(expr),
        Step::CollectSet(_) => false,
        Step::Anti(body) => safe_body(body, functions),
        Step::Union(branches) => branches.iter().all(|b| safe_body(&b.body, functions)),
        Step::Aggregate { aggs, .. } => aggs.iter().all(|(func, _)| {
            functions
                .get(func)
                .is_some_and(|d| matches!(d.impl_, AggImpl::Builtin(_)))
        }),
        Step::Window { find, .. } => safe_find(find, functions),
        Step::Pseudo { .. }
        | Step::Join(_)
        | Step::Distinct(_)
        | Step::DistinctFind(_)
        | Step::Sort { .. }
        | Step::Limit { .. } => true,
    })
}

/// At most eight raw seeks through the exact reader the Stream will scan.
fn scalar_kind(keys: &dyn KeySource, attribute: u32) -> Result<Option<ValueKind>> {
    let mut found = None;
    for kind in ValueKind::ALL {
        if keys.any_key(Index::Avet, &keys::attribute_kind_prefix(attribute, kind))? {
            if found.is_some() {
                return Ok(None);
            }
            found = Some(kind);
        }
    }
    Ok(found.filter(|k| {
        matches!(
            k,
            ValueKind::Integer | ValueKind::Float | ValueKind::Boolean
        )
    }))
}

fn candidate(
    snapshot: &Snapshot,
    keys: &dyn KeySource,
    plan: &Plan,
) -> Result<Option<(usize, bool)>> {
    let Source::Scan(spec) = &plan.source else {
        return Ok(None);
    };
    let (Term::Const(attribute), Term::Bind(_)) = (&spec.a, &spec.v) else {
        return Ok(None);
    };
    if spec.derived.is_some() || spec.never() || matches!(spec.e, Term::Eq(_)) {
        return Ok(None);
    }
    let Some(at) = plan
        .steps
        .iter()
        .position(|s| matches!(s, Step::Sort { .. }))
    else {
        return Ok(None);
    };
    let Some(Step::Sort { cols, order, keep }) = plan.steps.get(at) else {
        return Ok(None);
    };
    if *keep == Some(0) {
        return Ok(None);
    }
    let Some(before) = plan.steps.get(..at) else {
        return Ok(None);
    };
    // Reject impossible full orders before any speculative storage reads.
    if !Ordering::source(spec, ValueKind::Integer)
        .through(before)
        .is_some_and(|p| p.satisfies(cols, order))
    {
        return Ok(None);
    }
    let entity = match &spec.e {
        Term::Const(e) => Some(e),
        _ => None,
    };
    if snapshot.has_pending(attribute, entity)? {
        return Ok(None);
    }
    let Some(a) = keys.iid_of(attribute)? else {
        return Ok(None);
    };
    let Some(kind) = scalar_kind(keys, a)? else {
        return Ok(None);
    };
    if !Ordering::source(spec, kind)
        .through(before)
        .is_some_and(|p| p.satisfies(cols, order))
    {
        return Ok(None);
    }
    Ok(Some((at, !matches!(spec.e, Term::Const(_)))))
}

/// Only a plain query's root pipeline is resolved; bodies and rules keep their
/// original strategies. Failed optional proof reads retain the original plan.
pub(super) fn resolve(
    snapshot: &Snapshot,
    keys: Option<&dyn KeySource>,
    functions: &FunctionRegistry,
    plan: &mut Plan,
) {
    // With an offset, Sort.keep can be nonzero even though Limit pulls no row.
    if !enabled()
        || matches!(plan.steps.last(), Some(Step::Limit { limit: Some(0), .. }))
        || !safe_steps(&plan.steps, functions)
    {
        return;
    }
    let Some(keys) = keys else {
        return;
    };
    let Ok(Some((at, avet))) = candidate(snapshot, keys, plan) else {
        return;
    };
    if !matches!(plan.steps.get(at), Some(Step::Sort { .. })) {
        return;
    }
    if let Step::Sort { cols, .. } = plan.steps.remove(at) {
        plan.find = cols;
    }
    if avet && let Source::Scan(spec) = &mut plan.source {
        spec.index = Some(Index::Avet);
    }
    #[cfg(test)]
    OPTIMIZATIONS.with(|n| n.set(n.get() + 1));
}

#[cfg(test)]
mod tests;
