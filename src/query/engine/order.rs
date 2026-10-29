//! Snapshot-bound input-order proofs (#641). DICT order is never UUID/text
//! order. Unknown order retains the original index choice and buffering.

use super::plan::{
    Body, Grouping, JoinKind, Meta, Out, PartitionOrder, Plan, ScanSpec, Source, Step, Term, Vars,
    WindowMode,
};
use crate::graph::storage::Snapshot;
use crate::graph::types::Value;
use crate::query::datalog::functions::{AggImpl, Builtin, FunctionRegistry};
use crate::query::datalog::types::{BinOp, Expr, FindSpec, UnaryOp, WindowFunc};
use crate::storage::KeySource;
use crate::storage::keys::{self, Index, ValueKind};
use anyhow::Result;

mod arguments;
use arguments::Arguments;

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

/// Possible kinds of a constant on surviving rows. Unknown payload kinds stay
/// constant for Sort, but cannot certify a scalar composite grouping key.
#[derive(Clone, Copy)]
struct Kinds(u8);

impl Kinds {
    const INTEGER: Self = Self(4);
    const FLOAT: Self = Self(8);
    const BOOLEAN: Self = Self(2);
    const NULL: Self = Self(1);
    const OTHER: Self = Self(16);

    fn within(self, allowed: u8) -> bool {
        self.0 & !allowed == 0
    }
}

/// Leading scalar key and constants. A complete aggregate key is unique;
/// DICT entity suffixes never supply semantic UUID order.
struct Ordering {
    slot: Option<usize>,
    aliases: Vec<usize>,
    kind: Domain,
    constants: Vec<(usize, Kinds)>,
    unique: bool,
}

impl Ordering {
    fn source(spec: &ScanSpec, kind: Domain) -> Self {
        let mut constants = Vec::new();
        for &(m, slot) in &spec.meta {
            if m == Meta::ValidAt {
                constants.push((slot, Kinds(Kinds::INTEGER.0 | Kinds::NULL.0)));
            }
        }
        Ordering {
            slot: match spec.v {
                Term::Bind(s) => Some(s),
                _ => None,
            },
            kind,
            aliases: Vec::new(),
            unique: false,
            constants,
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
        let mut slot = None;
        let mut aliases = Vec::new();
        let mut constants = Vec::new();
        for (at, &s) in keep.iter().enumerate() {
            if self.ordered_slot(s) {
                if slot.is_none() {
                    slot = Some(at);
                } else {
                    aliases.push(at);
                }
            }
            if let Some(kind) = self.constant_kind(s) {
                constants.push((at, kind));
            }
        }
        self.slot = slot;
        self.aliases = aliases;
        self.constants = constants;
        self.unique = false;
    }

    fn invalidate(&mut self, slot: usize) {
        if self.ordered_slot(slot) {
            self.slot = None;
            self.aliases.clear();
            self.unique = false;
        }
        self.constants.retain(|&(s, _)| s != slot);
    }

    fn ordered_slot(&self, slot: usize) -> bool {
        self.slot == Some(slot) || self.aliases.contains(&slot)
    }

    fn constant_kind(&self, slot: usize) -> Option<Kinds> {
        self.constants
            .iter()
            .find_map(|&(s, kind)| (s == slot).then_some(kind))
    }

    fn constant(&self, expr: &Expr, vars: &Vars) -> Option<Kinds> {
        match expr {
            Expr::Lit(v) => Some(match v {
                Value::Integer(_) => Kinds::INTEGER,
                Value::Float(_) => Kinds::FLOAT,
                Value::Boolean(_) => Kinds::BOOLEAN,
                Value::Null => Kinds::NULL,
                _ => Kinds::OTHER,
            }),
            Expr::Var(v) => vars
                .iter()
                .find_map(|(name, s)| (name == v).then(|| self.constant_kind(*s)).flatten()),
            Expr::BinOp(op, a, b) => {
                let a = self.constant(a, vars)?;
                let b = self.constant(b, vars)?;
                Some(match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        let kinds = Kinds(a.0 | b.0);
                        if kinds.within(Kinds::INTEGER.0 | Kinds::FLOAT.0) {
                            kinds
                        } else {
                            Kinds::OTHER
                        }
                    }
                    _ => Kinds::BOOLEAN,
                })
            }
            Expr::UnaryOp(UnaryOp::Udf(_), _) => None,
            Expr::UnaryOp(_, a) => self.constant(a, vars).map(|_| Kinds::BOOLEAN),
            Expr::Slot(_) | Expr::CollectSet { .. } => None,
        }
    }

    /// The entire effective tuple must reduce to the one ascending source
    /// slot. A repeated float component is conservatively a composite too.
    fn tuple_ordered(&self, groups: &[usize]) -> bool {
        if !self.group_ordered()
            || (groups.len() != 1
                && !matches!(
                    self.kind,
                    Domain::Single(ValueKind::Integer | ValueKind::Boolean)
                ))
        {
            return false;
        }
        let mut varying = false;
        for &slot in groups {
            if self.ordered_slot(slot) {
                varying = true;
            } else if !self.constant_kind(slot).is_some_and(|kind| {
                kind.within(Kinds::INTEGER.0 | Kinds::BOOLEAN.0 | Kinds::NULL.0)
            }) {
                return false;
            }
        }
        varying
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
                    if *mode != Out::Check
                        && let Some(kind) = constant
                    {
                        self.constants.push((*slot, kind));
                    }
                }
                Step::Pseudo { col, value } => {
                    if let Term::Bind(slot) = value
                        && let Some(kind) = self.constant_kind(*col)
                    {
                        self.constants.push((*slot, kind));
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
                    ) && let Some(slot) = slots.iter().copied().find(|&s| self.ordered_slot(s))
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
            if self.constant_kind(slot).is_some() || (matched && self.ordered_slot(slot)) {
                continue;
            }
            if matched
                || desc
                || !self.ordered_slot(slot)
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
/// Both grouping-order and argument-domain proofs share this classifier.
fn value_kinds(keys: &dyn KeySource, attribute: u32) -> Result<Kinds> {
    let mut found = Kinds(0);
    for kind in ValueKind::ALL {
        if keys.any_key(Index::Avet, &keys::attribute_kind_prefix(attribute, kind))? {
            found.0 |= Kinds::stored(kind).0;
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
    arguments: &mut Arguments<'_>,
) -> Result<Option<Choice>> {
    let mut choice = Choice {
        aggregate: None,
        window: None,
        sort: None,
        avet,
    };
    let mut allow_sort = true;
    for (at, step) in plan.steps.iter().enumerate() {
        let Some(prefix) = plan.steps.get(..at) else {
            return Ok(None);
        };
        match step {
            Step::Aggregate {
                groups,
                plain_group_count,
                aggs,
                grouping: Grouping::Candidate,
                ..
            } if proof.tuple_ordered(groups)
                && arguments.accepts(aggs, &plan.source, prefix, &proof, functions)? =>
            {
                choice.aggregate = Some(at);
                proof.remap(groups);
                allow_sort = *plain_group_count == groups.len();
                proof.unique = allow_sort;
            }
            Step::Window {
                keep,
                mode: WindowMode::Candidate(slot),
                ..
            } => {
                if proof.slot != Some(*slot) || proof.constant_kind(*slot).is_some() {
                    break;
                }
                let Some(order) = proof.partition_order() else {
                    break;
                };
                choice.window = Some((at, *slot, order));
                proof.remap(keep);
            }
            Step::Sort { cols, order, keep } => {
                if allow_sort && *keep != Some(0) && proof.satisfies(cols, order) {
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
    Ok(
        (choice.aggregate.is_some() || choice.window.is_some() || choice.sort.is_some())
            .then_some(choice),
    )
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
    let mut speculative = Arguments {
        reader: None,
        source: (0, Kinds::INTEGER),
        cache: Vec::new(),
    };
    let entity_choice = match Ordering::entity(spec) {
        Some(proof) => choices(plan, proof, false, functions, &mut speculative)?,
        None => None,
    };
    let avet = !matches!(spec.e, Term::Const(_));
    if entity_choice.is_none()
        && choices(
            plan,
            Ordering::source(spec, Domain::Single(ValueKind::Integer)),
            avet,
            functions,
            &mut speculative,
        )?
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
    if let Some(choice) = entity_choice {
        // Entity adjacency is only a window proof, never an aggregate proof
        // derived from the optimistic no-I/O argument pass.
        debug_assert!(choice.aggregate.is_none());
        return Ok(Some(choice));
    }
    let Some(a) = keys.iid_of(attribute)? else {
        return Ok(None);
    };
    let kinds = value_kinds(keys, a)?;
    let Some(kind) = kinds.group_domain() else {
        return Ok(None);
    };
    let mut arguments = Arguments {
        reader: Some((snapshot, keys)),
        source: (a, kinds),
        cache: Vec::new(),
    };
    choices(
        plan,
        Ordering::source(spec, kind),
        avet,
        functions,
        &mut arguments,
    )
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
