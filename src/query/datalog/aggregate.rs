//! Aggregates and the order of values (#547 spec §3): one accumulator per
//! built-in function, used by the executor and the engine, whose result
//! does not depend on the order of its input.

use super::functions::{AggImpl, Builtin, FunctionRegistry, UdfFinaliseFn, UdfStepFn};
use super::types::{FindSpec, WindowSpec};
use crate::error::{ErrorCode, err_coded};
use crate::graph::types::Value;
use anyhow::Result;
use std::any::Any;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The class of a value in [`value_order`]: numbers, strings, booleans,
/// refs, keywords, `nil`.
fn class(v: &Value) -> u8 {
    match v {
        Value::Integer(_) | Value::Float(_) => 0,
        Value::String(_) => 1,
        Value::Boolean(_) => 2,
        Value::Ref(_) => 3,
        Value::Keyword(_) => 4,
        Value::Null => 5,
    }
}

/// The total order of values (spec §3.1): numbers by exact numeric value
/// (an `Integer` before an equal `Float`, floats then by `f64::total_cmp`,
/// NaN after every other number), then strings, booleans, refs, keywords
/// and `nil`. `Equal` only for identical values.
pub(crate) fn value_order(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => match (x.is_nan(), y.is_nan()) {
            (false, false) => x
                .partial_cmp(y)
                .unwrap_or(Ordering::Equal)
                .then_with(|| x.total_cmp(y)),
            (true, true) => x.total_cmp(y),
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
        },
        (Value::Integer(i), Value::Float(f)) => int_float(*i, *f).then(Ordering::Less),
        (Value::Float(f), Value::Integer(i)) => int_float(*i, *f).reverse().then(Ordering::Greater),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Boolean(x), Value::Boolean(y)) => x.cmp(y),
        (Value::Ref(x), Value::Ref(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Keyword(x), Value::Keyword(y)) => x.cmp(y),
        (Value::Null, Value::Null) => Ordering::Equal,
        _ => class(a).cmp(&class(b)),
    }
}

/// `i` against `f` by exact numeric value; a NaN `f` is greater.
fn int_float(i: i64, f: f64) -> Ordering {
    if f.is_nan() {
        return Ordering::Less;
    }
    // 2^63: every float at or above it exceeds every i64, every float
    // below -2^63 is less than every i64.
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    if f >= TWO_63 {
        return Ordering::Less;
    }
    if f < -TWO_63 {
        return Ordering::Greater;
    }
    let floor = f.floor();
    // In [-2^63, 2^63), so the floor is an exact integer in i128 range.
    #[allow(clippy::cast_possible_truncation)]
    let whole = floor as i128;
    i128::from(i).cmp(&whole).then(if f > floor {
        Ordering::Less
    } else {
        Ordering::Equal
    })
}

/// Limbs of [`Exact`]: 36 × 64 bits hold the sum of 2^64 values of any
/// finite `f64` and the `i128` integer total, in units of 2^-1074.
const LIMBS: usize = 36;

/// The bit of 2^0 in [`Exact`]'s units of 2^-1074.
const ONE: u32 = 1074;

/// An exact sum of finite floats and integers: a two's complement integer
/// counting units of 2^-1074, the smallest positive `f64`.
#[derive(Clone)]
struct Exact {
    limbs: [u64; LIMBS],
}

impl Exact {
    fn new() -> Self {
        Exact { limbs: [0; LIMBS] }
    }

    /// Add or subtract `mag << shift`.
    fn add_shifted(&mut self, mag: u64, shift: u32, neg: bool) {
        let idx = (shift / 64) as usize;
        let wide = u128::from(mag) << (shift % 64);
        let parts = [low(wide), low(wide >> 64)];
        let mut carry = false;
        for (i, limb) in self.limbs.iter_mut().enumerate().skip(idx) {
            let part = parts.get(i - idx).copied().unwrap_or(0);
            if part == 0 && !carry && i > idx + 1 {
                break;
            }
            if neg {
                let (d, b1) = limb.overflowing_sub(part);
                let (d, b2) = d.overflowing_sub(u64::from(carry));
                *limb = d;
                carry = b1 || b2;
            } else {
                let (s, c1) = limb.overflowing_add(part);
                let (s, c2) = s.overflowing_add(u64::from(carry));
                *limb = s;
                carry = c1 || c2;
            }
        }
    }

    /// Add a finite float.
    fn add_float(&mut self, f: f64) {
        let bits = f.to_bits();
        let neg = bits >> 63 == 1;
        let exp = u32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
        let frac = bits & ((1 << 52) - 1);
        if exp == 0 {
            self.add_shifted(frac, 0, neg);
        } else {
            self.add_shifted(frac | (1 << 52), exp - 1, neg);
        }
    }

    /// Add an integer.
    fn add_int(&mut self, i: i128) {
        let mag = i.unsigned_abs();
        let neg = i < 0;
        self.add_shifted(low(mag), ONE, neg);
        self.add_shifted(low(mag >> 64), ONE + 64, neg);
    }

    fn bit(mag: &[u64; LIMBS], p: u32) -> bool {
        mag.get((p / 64) as usize)
            .is_some_and(|l| (l >> (p % 64)) & 1 == 1)
    }

    /// Whether any bit of `mag` below `p` is set.
    fn any_below(mag: &[u64; LIMBS], p: u32) -> bool {
        let full = (p / 64) as usize;
        mag.iter().take(full).any(|&l| l != 0)
            || mag
                .get(full)
                .is_some_and(|l| !p.is_multiple_of(64) && l & ((1u64 << (p % 64)) - 1) != 0)
    }

    /// The sum rounded once to the nearest `f64`, ties to even; `None`
    /// when it is zero.
    fn round(&self) -> Option<f64> {
        let neg = self.limbs.last().is_some_and(|l| l >> 63 == 1);
        let mut mag = self.limbs;
        if neg {
            let mut carry = true;
            for limb in &mut mag {
                let (s, c) = (!*limb).overflowing_add(u64::from(carry));
                *limb = s;
                carry = c;
            }
        }
        let top = mag.iter().rposition(|&l| l != 0)?;
        let top_limb = mag.get(top).copied().unwrap_or(0);
        let h = u32::try_from(top).unwrap_or(0) * 64 + (63 - top_limb.leading_zeros());
        let value = if h < 53 {
            // Below 2^53 units: a multiple of 2^-1074 under 2^-1021, exact.
            let m = mag.first().copied().unwrap_or(0);
            #[allow(clippy::cast_precision_loss)]
            let m = m as f64;
            m * f64::from_bits(1)
        } else {
            let s = h - 52;
            let mut m: u64 = 0;
            for p in (s..=h).rev() {
                m = (m << 1) | u64::from(Self::bit(&mag, p));
            }
            let half = Self::bit(&mag, s - 1);
            let sticky = Self::any_below(&mag, s - 1);
            let mut s = s;
            if half && (sticky || m & 1 == 1) {
                m += 1;
                if m == 1 << 53 {
                    m >>= 1;
                    s += 1;
                }
            }
            // The value is m · 2^(s-1074) with m in [2^52, 2^53): biased
            // exponent s + 1.
            let biased = u64::from(s) + 1;
            if biased >= 0x7ff {
                f64::INFINITY
            } else {
                f64::from_bits((biased << 52) | (m & ((1 << 52) - 1)))
            }
        };
        Some(if neg { -value } else { value })
    }
}

/// The low 64 bits of `v`.
#[allow(clippy::cast_possible_truncation)]
fn low(v: u128) -> u64 {
    v as u64
}

/// An exact sum of numbers (spec §3.2).
#[derive(Clone, Default)]
pub(crate) struct Sum {
    ints: i128,
    /// The exact sum of the finite floats, allocated on the first one.
    floats: Option<Box<Exact>>,
    /// Numbers taken.
    count: u64,
    any_float: bool,
    nan: bool,
    pos_inf: bool,
    neg_inf: bool,
    /// Whether some number taken is not `-0.0`.
    not_neg_zero: bool,
}

impl Sum {
    /// Take `v`; `false` when it is not a number.
    pub(crate) fn add(&mut self, v: &Value) -> bool {
        match v {
            Value::Integer(i) => {
                self.ints += i128::from(*i);
                self.not_neg_zero = true;
            }
            Value::Float(f) => {
                self.any_float = true;
                if f.is_nan() {
                    self.nan = true;
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        self.pos_inf = true;
                    } else {
                        self.neg_inf = true;
                    }
                } else {
                    if f.to_bits() != (-0.0f64).to_bits() {
                        self.not_neg_zero = true;
                    }
                    self.floats
                        .get_or_insert_with(|| Box::new(Exact::new()))
                        .add_float(*f);
                }
            }
            _ => return false,
        }
        self.count += 1;
        true
    }

    /// The sum as a float, rounded once.
    fn float(&self) -> f64 {
        if self.nan || (self.pos_inf && self.neg_inf) {
            return f64::NAN;
        }
        if self.pos_inf {
            return f64::INFINITY;
        }
        if self.neg_inf {
            return f64::NEG_INFINITY;
        }
        let mut exact = self.floats.as_deref().cloned().unwrap_or_else(Exact::new);
        exact.add_int(self.ints);
        match exact.round() {
            Some(f) => f,
            None if self.count > 0 && !self.not_neg_zero => -0.0,
            None => 0.0,
        }
    }

    /// `Integer` while every number was an integer, saturated; else
    /// `Float`. No numbers: `Integer(0)`.
    pub(crate) fn value(&self) -> Value {
        if self.any_float {
            Value::Float(self.float())
        } else {
            let clamped = self.ints.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
            Value::Integer(i64::try_from(clamped).unwrap_or(i64::MAX))
        }
    }

    /// The mean of the numbers, or `nil` without any.
    pub(crate) fn mean(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = self.count as f64;
        Value::Float(self.float() / n)
    }
}

/// `v` with `-0.0` as `0.0` and every NaN as one NaN, so equal values under
/// `Value`'s `Ord` are stored alike.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Float(f) if f.is_nan() => Value::Float(f64::NAN),
        Value::Float(f) if *f == 0.0 => Value::Float(0.0),
        other => other.clone(),
    }
}

/// The type name used in error messages.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "String",
        Value::Integer(_) => "Integer",
        Value::Float(_) => "Float",
        Value::Boolean(_) => "Boolean",
        Value::Ref(_) => "Ref",
        Value::Keyword(_) => "Keyword",
        Value::Null => "Null",
    }
}

/// Variant kinds, in `Value`'s declaration order, for `INT-043`.
const KINDS: [&str; 7] = [
    "String", "Integer", "Float", "Boolean", "Ref", "Keyword", "Null",
];

fn kind(v: &Value) -> u8 {
    match v {
        Value::String(_) => 0,
        Value::Integer(_) => 1,
        Value::Float(_) => 2,
        Value::Boolean(_) => 3,
        Value::Ref(_) => 4,
        Value::Keyword(_) => 5,
        Value::Null => 6,
    }
}

/// The least or greatest value by [`value_order`], and the kinds met.
#[derive(Clone, Default)]
pub(crate) struct Extreme {
    best: Option<Value>,
    kinds: u8,
}

impl Extreme {
    fn add(&mut self, v: &Value, max: bool) {
        if matches!(v, Value::Null) {
            return;
        }
        self.kinds |= 1 << kind(v);
        let better = match &self.best {
            None => true,
            Some(b) => {
                let o = value_order(v, b);
                if max {
                    o == Ordering::Greater
                } else {
                    o == Ordering::Less
                }
            }
        };
        if better {
            self.best = Some(v.clone());
        }
    }
}

/// A registered aggregate's state.
pub(crate) struct Udf {
    acc: Box<dyn Any + Send>,
    step: UdfStepFn,
    finalise: UdfFinaliseFn,
    /// Values stepped.
    n: usize,
}

impl Udf {
    fn of(registry: &FunctionRegistry, name: &str) -> Option<Udf> {
        match &registry.get(name)?.impl_ {
            AggImpl::Udf(ops) => Some(Udf {
                acc: (ops.init)(),
                step: ops.step.clone(),
                finalise: ops.finalise.clone(),
                n: 0,
            }),
            AggImpl::Builtin(_) => None,
        }
    }

    fn add(&mut self, v: &Value) {
        (self.step)(&mut self.acc, v);
        self.n += 1;
    }

    fn value(&self) -> Value {
        (self.finalise)(&self.acc, self.n)
    }
}

/// One aggregate of one group (spec §3.2). Takes values one at a time and
/// never fails while taking them; [`Agg::finish`] decides the result.
pub(crate) enum Agg {
    Count(i64),
    /// The sum, and the type of the first value that is not a number.
    Sum(Sum, Option<&'static str>),
    Min(Extreme),
    Max(Extreme),
    CountDistinct(BTreeSet<Value>),
    SumDistinct(BTreeSet<Value>),
    Udf(Udf),
}

impl Agg {
    /// The accumulator of the aggregate `func`, or `INT-029` when it is
    /// neither built in nor registered.
    pub(crate) fn new(func: &str, registry: &FunctionRegistry) -> Result<Agg> {
        let desc = registry
            .get(func)
            .ok_or_else(|| err_coded!(ErrorCode::Int029, func))?;
        Ok(match &desc.impl_ {
            AggImpl::Builtin(b) => match b {
                Builtin::Count => Agg::Count(0),
                Builtin::Sum => Agg::Sum(Sum::default(), None),
                // `avg` is a window function only (`PRS-027`).
                Builtin::Avg => return Err(err_coded!(ErrorCode::Int029, func)),
                Builtin::Min => Agg::Min(Extreme::default()),
                Builtin::Max => Agg::Max(Extreme::default()),
                Builtin::CountDistinct => Agg::CountDistinct(BTreeSet::new()),
                Builtin::SumDistinct => Agg::SumDistinct(BTreeSet::new()),
            },
            AggImpl::Udf(_) => Agg::Udf(
                Udf::of(registry, func).ok_or_else(|| err_coded!(ErrorCode::Int029, func))?,
            ),
        })
    }

    /// Take one value; `nil` is skipped by every aggregate.
    pub(crate) fn step(&mut self, v: &Value) {
        if matches!(v, Value::Null) {
            return;
        }
        match self {
            Agg::Count(n) => *n = n.saturating_add(1),
            Agg::Sum(s, bad) => {
                if !s.add(v) && bad.is_none() {
                    *bad = Some(type_name(v));
                }
            }
            Agg::Min(e) => e.add(v, false),
            Agg::Max(e) => e.add(v, true),
            Agg::CountDistinct(set) | Agg::SumDistinct(set) => {
                set.insert(canonical(v));
            }
            Agg::Udf(u) => u.add(v),
        }
    }

    /// The result: `Ok(None)` when the group is dropped (`min`/`max` with no
    /// value, `INT-042`), or the error the values make (spec §3.3).
    pub(crate) fn finish(self, func: &str) -> Result<Option<Value>> {
        Ok(Some(match self {
            Agg::Count(n) => Value::Integer(n),
            Agg::Sum(_, Some(bad)) => return Err(err_coded!(ErrorCode::Int041, bad)),
            Agg::Sum(s, None) => s.value(),
            Agg::Min(e) | Agg::Max(e) => {
                let Some(best) = e.best else {
                    return Ok(None);
                };
                if e.kinds.count_ones() > 1 {
                    let mut names = KINDS
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| e.kinds & (1 << i) != 0)
                        .map(|(_, n)| *n);
                    let first = names.next().unwrap_or("?");
                    let second = names.next().unwrap_or("?");
                    return Err(err_coded!(ErrorCode::Int043, func, first, second));
                }
                if !matches!(best, Value::Integer(_) | Value::Float(_) | Value::String(_)) {
                    return Err(err_coded!(ErrorCode::Int044, func, type_name(&best)));
                }
                best
            }
            Agg::CountDistinct(set) => Value::Integer(i64::try_from(set.len()).unwrap_or(i64::MAX)),
            Agg::SumDistinct(set) => {
                let mut s = Sum::default();
                for v in &set {
                    if !s.add(v) {
                        return Err(err_coded!(ErrorCode::Int041, type_name(v)));
                    }
                }
                s.value()
            }
            Agg::Udf(u) if u.n == 0 => Value::Null,
            Agg::Udf(u) => u.value(),
        }))
    }
}

/// The groups of an aggregate query, by their grouping values as `Value`'s
/// `Ord` compares them: the values each group returns, and one accumulator
/// per aggregate, in `:find` order.
pub(crate) type Groups = BTreeMap<Vec<Value>, (Vec<Value>, Vec<Agg>)>;

/// Finish `groups` in key order (spec §3.3): each group's values and its
/// aggregates' results. A group whose aggregate gives `INT-042` is dropped;
/// any other error, the first in `:find` order of the first group, fails.
/// `funcs` names each aggregate, for the messages.
pub(crate) fn finish_groups(
    groups: Groups,
    funcs: &[&str],
) -> Result<Vec<(Vec<Value>, Vec<Value>)>> {
    let mut out = Vec::with_capacity(groups.len());
    'group: for (shown, accs) in groups.into_values() {
        let mut results = Vec::with_capacity(accs.len());
        for (acc, func) in accs.into_iter().zip(funcs) {
            match acc.finish(func)? {
                Some(v) => results.push(v),
                None => continue 'group,
            }
        }
        out.push((shown, results));
    }
    Ok(out)
}

/// A window function's running value over a sorted partition (spec §4.2).
pub(crate) enum Running {
    Count(i64),
    /// Numbers only; other values are skipped.
    Sum(Sum),
    Avg(Sum),
    Min(Extreme),
    Max(Extreme),
    /// Steps every value, `nil` included.
    Udf(Udf),
}

impl Running {
    /// The accumulator of `ws`, or `INT-030` when its function is unknown.
    /// Not for `rank` or `row-number`.
    pub(crate) fn new(ws: &WindowSpec, registry: &FunctionRegistry) -> Result<Running> {
        let name = ws.func_name();
        let unknown = || err_coded!(ErrorCode::Int030, name.clone());
        let desc = registry.get(&name).ok_or_else(unknown)?;
        Ok(match &desc.impl_ {
            AggImpl::Builtin(Builtin::Count) => Running::Count(0),
            AggImpl::Builtin(Builtin::Sum) => Running::Sum(Sum::default()),
            AggImpl::Builtin(Builtin::Avg) => Running::Avg(Sum::default()),
            AggImpl::Builtin(Builtin::Min) => Running::Min(Extreme::default()),
            AggImpl::Builtin(Builtin::Max) => Running::Max(Extreme::default()),
            AggImpl::Builtin(Builtin::CountDistinct | Builtin::SumDistinct) => {
                return Err(unknown());
            }
            AggImpl::Udf(_) => Running::Udf(Udf::of(registry, &name).ok_or_else(unknown)?),
        })
    }

    pub(crate) fn step(&mut self, v: &Value) {
        match self {
            Running::Count(n) => {
                if !matches!(v, Value::Null) {
                    *n = n.saturating_add(1);
                }
            }
            Running::Sum(s) | Running::Avg(s) => {
                s.add(v);
            }
            Running::Min(e) => e.add(v, false),
            Running::Max(e) => e.add(v, true),
            Running::Udf(u) => u.add(v),
        }
    }

    pub(crate) fn value(&self) -> Value {
        match self {
            Running::Count(n) => Value::Integer(*n),
            Running::Sum(s) => s.value(),
            Running::Avg(s) => s.mean(),
            Running::Min(e) | Running::Max(e) => e.best.clone().unwrap_or(Value::Null),
            Running::Udf(u) => u.value(),
        }
    }
}

/// The variables a window's tie-break reads (spec §2, "named variables"),
/// in `:find` order, each once.
pub(crate) fn named_vars(find: &[FindSpec]) -> Vec<String> {
    let aggregate = find.iter().any(|s| matches!(s, FindSpec::Aggregate { .. }));
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: &String| {
        if !out.contains(v) {
            out.push(v.clone());
        }
    };
    for spec in find {
        match spec {
            FindSpec::Variable(v) => push(v),
            FindSpec::Window(ws) if !aggregate => {
                ws.var.iter().for_each(&mut push);
                ws.partition_by.iter().for_each(&mut push);
                push(&ws.order_by);
            }
            FindSpec::Window(_) | FindSpec::Aggregate { .. } => {}
        }
    }
    out
}

#[cfg(test)]
mod tests;
