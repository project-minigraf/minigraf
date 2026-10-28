//! Snapshot-bound input-order proofs (#641). DICT order is never UUID/text
//! order. Unknown order retains the original index choice and buffering.

use super::plan::{
    Body, Grouping, JoinKind, Meta, Out, PartitionOrder, Plan, ScanSpec, Source, Step, Term, Vars,
    WindowMode,
};
use crate::graph::storage::Snapshot;
use crate::query::datalog::functions::{AggImpl, Builtin, FunctionRegistry};
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

/// Numeric tag ranges group in Value::Ord; only one scalar kind also
/// supplies the semantic Sort comparator.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Domain {
    Single(ValueKind),
    Numbers,
    Entity,
}

/// Leading scalar key and constants. A complete aggregate key is unique;
/// DICT entity suffixes never supply semantic UUID order.
struct Ordering {
    slot: Option<usize>,
    kind: Domain,
    constants: Vec<usize>,
    unique: bool,
}

impl Ordering {
    fn source(spec: &ScanSpec, kind: Domain) -> Self {
        Ordering {
            slot: match spec.v {
                Term::Bind(s) => Some(s),
                _ => None,
            },
            kind,
            unique: false,
            constants: spec
                .meta
                .iter()
                .filter_map(|&(m, s)| (m == Meta::ValidAt).then_some(s))
                .collect(),
        }
    }

    fn entity(spec: &ScanSpec) -> Option<Self> {
        let Term::Bind(slot) = spec.e else {
            return None;
        };
        if spec.index == Some(Index::Avet) {
            return None;
        }
        let mut proof = Self::source(spec, Domain::Entity);
        proof.slot = Some(slot);
        Some(proof)
    }

    fn group_ordered(&self) -> bool {
        matches!(
            self.kind,
            Domain::Single(ValueKind::Integer | ValueKind::Float | ValueKind::Boolean)
                | Domain::Numbers
        )
    }

    fn partition_order(&self) -> Option<PartitionOrder> {
        match self.kind {
            Domain::Entity | Domain::Single(ValueKind::Ref) => Some(PartitionOrder::Entity),
            Domain::Single(ValueKind::Keyword) => Some(PartitionOrder::Ident),
            _ if self.group_ordered() => Some(PartitionOrder::Values),
            _ => None,
        }
    }

    fn remap(&mut self, keep: &[usize]) {
        self.slot = self
            .slot
            .and_then(|slot| keep.iter().position(|&s| s == slot));
        self.constants = keep
            .iter()
            .enumerate()
            .filter_map(|(at, s)| self.constants.contains(s).then_some(at))
            .collect();
        self.unique = false;
    }

    fn invalidate(&mut self, slot: usize) {
        if self.slot == Some(slot) {
            self.slot = None;
            self.unique = false;
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
                    self.unique = false;
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
                    if matches!(
                        self.kind,
                        Domain::Single(ValueKind::Float) | Domain::Numbers
                    ) && let Some(slot) = self.slot.filter(|s| slots.contains(s))
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
            if matched && self.unique {
                continue;
            }
            if self.constants.contains(&slot) || (matched && self.slot == Some(slot)) {
                continue;
            }
            if matched
                || desc
                || self.slot != Some(slot)
                || !matches!(
                    self.kind,
                    Domain::Single(ValueKind::Integer | ValueKind::Float | ValueKind::Boolean)
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
fn value_domain(keys: &dyn KeySource, attribute: u32) -> Result<Option<Domain>> {
    let mut found = None;
    for kind in ValueKind::ALL {
        if keys.any_key(Index::Avet, &keys::attribute_kind_prefix(attribute, kind))? {
            found = match (found, kind) {
                (
                    None,
                    ValueKind::Integer
                    | ValueKind::Float
                    | ValueKind::Boolean
                    | ValueKind::Ref
                    | ValueKind::Keyword,
                ) => Some(Domain::Single(kind)),
                (Some(Domain::Single(ValueKind::Integer)), ValueKind::Float) => {
                    Some(Domain::Numbers)
                }
                _ => return Ok(None),
            };
        }
    }
    Ok(found)
}

/// The choices are applied together only after the snapshot proof succeeds.
struct Choice {
    aggregate: Option<usize>,
    window: Option<(usize, usize, PartitionOrder)>,
    sort: Option<usize>,
    avet: bool,
}

fn choices(
    plan: &Plan,
    mut proof: Ordering,
    avet: bool,
    functions: &FunctionRegistry,
) -> Option<Choice> {
    let mut choice = Choice {
        aggregate: None,
        window: None,
        sort: None,
        avet,
    };
    for (at, step) in plan.steps.iter().enumerate() {
        match step {
            Step::Aggregate {
                groups,
                aggs,
                grouping: Grouping::Candidate,
                ..
            } if proof.group_ordered()
                && groups.len() == 1
                && proof.slot == groups.first().copied()
                && aggs.iter().all(|(func, _)| {
                    functions.get(func).is_some_and(|d| {
                        matches!(
                            d.impl_,
                            AggImpl::Builtin(Builtin::Count | Builtin::CountDistinct)
                        )
                    })
                }) =>
            {
                choice.aggregate = Some(at);
                proof.slot = Some(0);
                proof.constants.clear();
                proof.unique = true;
            }
            Step::Window {
                keep,
                mode: WindowMode::Candidate(slot),
                ..
            } => {
                if proof.slot != Some(*slot) || proof.constants.contains(slot) {
                    break;
                }
                let Some(order) = proof.partition_order() else {
                    break;
                };
                choice.window = Some((at, *slot, order));
                proof.remap(keep);
            }
            Step::Sort { cols, order, keep } => {
                if *keep != Some(0) && proof.satisfies(cols, order) {
                    choice.sort = Some(at);
                }
                break;
            }
            _ => {
                let Some(next) = proof.through(std::slice::from_ref(step)) else {
                    break;
                };
                proof = next;
            }
        }
    }
    (choice.aggregate.is_some() || choice.window.is_some() || choice.sort.is_some())
        .then_some(choice)
}

fn candidate(
    snapshot: &Snapshot,
    keys: &dyn KeySource,
    plan: &Plan,
    functions: &FunctionRegistry,
) -> Result<Option<Choice>> {
    let Source::Scan(spec) = &plan.source else {
        return Ok(None);
    };
    let (Term::Const(attribute), Term::Bind(_)) = (&spec.a, &spec.v) else {
        return Ok(None);
    };
    if spec.derived.is_some() || spec.never() || matches!(spec.e, Term::Eq(_)) {
        return Ok(None);
    }
    // Prefer the existing entity prefix when it already partitions windows;
    // no kind seeks or broader index are needed for dictionary-id adjacency.
    let entity_choice =
        Ordering::entity(spec).and_then(|proof| choices(plan, proof, false, functions));
    let avet = !matches!(spec.e, Term::Const(_));
    if entity_choice.is_none()
        && choices(
            plan,
            Ordering::source(spec, Domain::Single(ValueKind::Integer)),
            avet,
            functions,
        )
        .is_none()
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
    if entity_choice.is_some() {
        return Ok(entity_choice);
    }
    let Some(a) = keys.iid_of(attribute)? else {
        return Ok(None);
    };
    let Some(kind) = value_domain(keys, a)? else {
        return Ok(None);
    };
    Ok(choices(plan, Ordering::source(spec, kind), avet, functions))
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
    let Ok(Some(choice)) = candidate(snapshot, keys, plan, functions) else {
        return;
    };
    if let Some(at) = choice.aggregate
        && let Some(Step::Aggregate { grouping, .. }) = plan.steps.get_mut(at)
    {
        *grouping = Grouping::Ordered;
    }
    if let Some((at, slot, order)) = choice.window
        && let Some(Step::Window { mode, .. }) = plan.steps.get_mut(at)
    {
        *mode = WindowMode::Partitioned { slot, order };
    }
    if let Some(at) = choice.sort
        && let Step::Sort { cols, .. } = plan.steps.remove(at)
    {
        plan.find = cols;
    }
    if choice.avet
        && let Source::Scan(spec) = &mut plan.source
    {
        spec.index = Some(Index::Avet);
    }
    #[cfg(test)]
    OPTIMIZATIONS.with(|n| n.set(n.get() + 1));
}

#[cfg(test)]
mod tests;
