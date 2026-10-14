//! `Scan` (#473 spec §6.1): one pattern over one key range of the snapshot.
//!
//! Every pending fact is newer than every committed one, so a triple with a
//! pending record in the snapshot (at or before `:as-of`) is decided by its
//! pending records alone, and a triple without one by its committed keys
//! (#379). `Scan` reads the committed ranges first, leaving out the triples a
//! pending record decides (the shadow set), then the pending range.

use super::batch::Batch;
use super::cell::{Cell, Scalar};
use super::plan::{Meta, ScanSpec, Term, Valid, ValueConst, names};
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::storage::PendingResume;
use crate::graph::types::{EntityId, Fact, Value};
use crate::storage::keys::{self, Index, KeyFact, KeyValue, SHORT_STRING_MAX, ValueKind};
use crate::storage::{KeySource, Scan as Range};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

pub(crate) struct Scan {
    spec: ScanSpec,
    state: State,
    keys_buf: Vec<KeyFact>,
    facts_buf: Vec<Fact>,
    /// `rows_in` counts the live keys and pending facts read.
    counters: Counters,
}

enum State {
    Start,
    Committed(Committed),
    Pending(Option<PendingResume>),
    Done,
}

/// The committed half of the read.
struct Committed {
    /// Key ranges as `(index, prefix)`, read in turn from `next`.
    ranges: Vec<(Index, Vec<u8>)>,
    next: usize,
    resume: Option<Vec<u8>>,
    /// The value constant when the ranges do not fix it.
    test: Option<KeyTest>,
    /// Triples a pending record decides, as [`keys::entity_attribute_value_prefix`].
    shadow: HashSet<Vec<u8>>,
}

/// A value constant over committed key values, with ids resolved.
enum KeyTest {
    Exact(KeyValue),
    /// A keyword constant: the keyword's ident id, the named entity's id.
    Keyword(Option<u32>, Option<u64>),
    /// A UUID constant: its entity id; whether each keyword names it.
    Uuid(EntityId, Option<u64>, HashMap<u32, bool>),
    Float(f64),
}

impl KeyTest {
    fn matches(&mut self, keys: &dyn KeySource, v: &KeyValue) -> Result<bool> {
        Ok(match (self, v) {
            (KeyTest::Exact(c), v) => c == v,
            (KeyTest::Keyword(Some(i), _), KeyValue::Keyword(fi)) => i == fi,
            (KeyTest::Keyword(_, Some(e)) | KeyTest::Uuid(_, Some(e), _), KeyValue::Ref(fe)) => {
                e == fe
            }
            (KeyTest::Uuid(u, _, seen), KeyValue::Keyword(i)) => match seen.get(i) {
                Some(&m) => m,
                None => {
                    let m = names(&keys.ident_of(*i)?, u);
                    seen.insert(*i, m);
                    m
                }
            },
            (KeyTest::Float(f), KeyValue::Float(ff)) => (*f - ff).abs() < f64::EPSILON,
            _ => false,
        })
    }
}

impl Scan {
    pub(crate) fn new(spec: ScanSpec) -> Self {
        Scan {
            spec,
            state: State::Start,
            keys_buf: Vec::new(),
            facts_buf: Vec::new(),
            counters: Counters::new("Scan"),
        }
    }
}

impl Operator for Scan {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        loop {
            let mut batch = Batch::new(self.spec.width);
            match &mut self.state {
                State::Start => self.state = open(&self.spec, ctx)?,
                State::Committed(c) => {
                    let Some(keys) = ctx.keys else {
                        self.state = State::Pending(None);
                        continue;
                    };
                    let Some((index, prefix)) = c.ranges.get(c.next) else {
                        self.state = State::Pending(None);
                        continue;
                    };
                    self.keys_buf.clear();
                    c.resume = keys.live_keys(
                        *index,
                        prefix,
                        c.resume.as_deref(),
                        self.spec.as_of,
                        BATCH_ROWS,
                        &mut self.keys_buf,
                    )?;
                    if c.resume.is_none() {
                        c.next += 1;
                    }
                    self.counters.rows_in += self.keys_buf.len() as u64;
                    let valid = valid_at(self.spec.valid, ctx);
                    let mut row = vec![None; self.spec.width];
                    for kf in &self.keys_buf {
                        if !visible(valid, kf.vf, kf.vt)
                            || (!c.shadow.is_empty()
                                && c.shadow.contains(&keys::entity_attribute_value_prefix(
                                    kf.e, kf.a, &kf.v,
                                )))
                        {
                            continue;
                        }
                        if let Some(t) = &mut c.test
                            && !t.matches(keys, &kf.v)?
                        {
                            continue;
                        }
                        if fill(&self.spec.e, &mut row, || Ok(Cell::Eid(kf.e)))?
                            && fill(&self.spec.a, &mut row, || Ok(Cell::Iid(u64::from(kf.a))))?
                            && fill(&self.spec.v, &mut row, || Ok(Cell::of_key(&kf.v)))?
                        {
                            for &(m, slot) in &self.spec.meta {
                                let n = match m {
                                    Meta::ValidFrom => kf.vf,
                                    Meta::ValidTo => kf.vt,
                                    Meta::TxCount => kf.tx_count.cast_signed(),
                                    Meta::TxId => keys.tx_id_of(kf.tx_count)?.cast_signed(),
                                };
                                set(&mut row, slot, n);
                            }
                            batch.push(&mut row);
                        }
                        row.iter_mut().for_each(|c| *c = None);
                    }
                }
                State::Pending(resume) => {
                    self.facts_buf.clear();
                    let more = ctx.snapshot.pending_live(
                        range(&self.spec),
                        self.spec.as_of,
                        resume,
                        BATCH_ROWS,
                        &mut self.facts_buf,
                    )?;
                    self.counters.rows_in += self.facts_buf.len() as u64;
                    let valid = valid_at(self.spec.valid, ctx);
                    let mut row = vec![None; self.spec.width];
                    for f in &self.facts_buf {
                        if !visible(valid, f.valid_from, f.valid_to)
                            || matches!(&self.spec.v, Term::Const(v) if !v.matches(&f.value))
                        {
                            continue;
                        }
                        let keys = ctx.keys;
                        let ids = &mut *ctx.ids;
                        if fill(&self.spec.e, &mut row, || {
                            Ok(Cell::Eid(ids.eid(keys, &f.entity)?))
                        })? && fill(&self.spec.a, &mut row, || {
                            Ok(Cell::Iid(ids.iid(keys, &f.attribute)?))
                        })? && fill(&self.spec.v, &mut row, || {
                            Cell::of_value(ids, keys, &f.value)
                        })? {
                            for &(m, slot) in &self.spec.meta {
                                let n = match m {
                                    Meta::ValidFrom => f.valid_from,
                                    Meta::ValidTo => f.valid_to,
                                    Meta::TxCount => f.tx_count.cast_signed(),
                                    Meta::TxId => f.tx_id.cast_signed(),
                                };
                                set(&mut row, slot, n);
                            }
                            batch.push(&mut row);
                        }
                        row.iter_mut().for_each(|c| *c = None);
                    }
                    if !more {
                        self.state = State::Done;
                    }
                }
                State::Done => return Ok(None),
            }
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
    }

    fn close(&mut self) {
        self.state = State::Done;
        self.keys_buf = Vec::new();
        self.facts_buf = Vec::new();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
    }
}

fn set(row: &mut [Option<Cell>], slot: usize, n: i64) {
    if let Some(s) = row.get_mut(slot) {
        *s = Some(Cell::Val(Scalar::Int(n)));
    }
}

/// Resolve the committed ranges of `spec` and the pending triples that
/// shadow them.
fn open(spec: &ScanSpec, ctx: &mut Ctx<'_>) -> Result<State> {
    let Some(keys) = ctx.keys else {
        return Ok(State::Pending(None));
    };
    let pending = State::Pending(None);
    let e = match &spec.e {
        Term::Const(u) => match keys.eid_of(u)? {
            Some(e) => Some(e),
            None => return Ok(pending),
        },
        _ => None,
    };
    let a = match &spec.a {
        Term::Const(name) => match keys.iid_of(name)? {
            Some(a) => Some(a),
            None => return Ok(pending),
        },
        _ => None,
    };
    let test = match &spec.v {
        Term::Const(c) => match key_test(keys, c)? {
            Some(t) => Some(t),
            None => return Ok(pending),
        },
        _ => None,
    };
    let (ranges, test) = match (e, a, test) {
        (Some(e), Some(a), Some(KeyTest::Exact(v))) => (
            vec![(Index::Eavt, keys::entity_attribute_value_prefix(e, a, &v))],
            None,
        ),
        (Some(e), Some(a), t) => (vec![(Index::Eavt, keys::entity_attribute_prefix(e, a))], t),
        (Some(e), None, t) => (vec![(Index::Eavt, keys::entity_prefix(e))], t),
        (None, Some(a), Some(t)) => avet(a, t),
        (None, Some(a), None) => (vec![(Index::Aevt, keys::attribute_prefix(a))], None),
        (None, None, t) => (vec![(Index::Eavt, Vec::new())], t),
    };
    let mut shadow = HashSet::new();
    for f in ctx.snapshot.pending_records(range(spec), spec.as_of)? {
        if let Some(t) = committed_triple(keys, &f)? {
            shadow.insert(t);
        }
    }
    Ok(State::Committed(Committed {
        ranges,
        next: 0,
        resume: None,
        test,
        shadow,
    }))
}

/// The AVET ranges of attribute `a` that hold every value `t` matches, and
/// the test left for the values they read.
fn avet(a: u32, t: KeyTest) -> (Vec<(Index, Vec<u8>)>, Option<KeyTest>) {
    let value = |v: KeyValue| (Index::Avet, keys::attribute_value_prefix(a, &v));
    let kind = |k: ValueKind| (Index::Avet, keys::attribute_kind_prefix(a, k));
    match t {
        KeyTest::Exact(v) => (vec![value(v)], None),
        KeyTest::Keyword(i, e) => {
            let ranges = i
                .map(|i| value(KeyValue::Keyword(i)))
                .into_iter()
                .chain(e.map(|e| value(KeyValue::Ref(e))))
                .collect();
            (ranges, None)
        }
        KeyTest::Uuid(u, e, seen) => {
            let ranges = e
                .map(|e| value(KeyValue::Ref(e)))
                .into_iter()
                .chain([kind(ValueKind::Keyword)])
                .collect();
            (ranges, Some(KeyTest::Uuid(u, e, seen)))
        }
        KeyTest::Float(f) => (vec![kind(ValueKind::Float)], Some(KeyTest::Float(f))),
    }
}

/// The test of a value constant over committed keys, or `None` when no
/// committed key can match it.
fn key_test(keys: &dyn KeySource, c: &ValueConst) -> Result<Option<KeyTest>> {
    Ok(match c {
        ValueConst::Exact(v) => key_value(keys, v)?.map(KeyTest::Exact),
        ValueConst::Keyword(k, u) => match (keys.iid_of(k)?, keys.eid_of(u)?) {
            (None, None) => None,
            (i, e) => Some(KeyTest::Keyword(i, e)),
        },
        ValueConst::Uuid(u) => Some(KeyTest::Uuid(*u, keys.eid_of(u)?, HashMap::new())),
        ValueConst::Float(f) => Some(KeyTest::Float(*f)),
    })
}

/// The pending range of `spec`.
fn range(spec: &ScanSpec) -> Range<'_> {
    match (&spec.e, &spec.a) {
        (Term::Const(e), Term::Const(a)) => Range::EntityAttribute(e, a),
        (Term::Const(e), _) => Range::Entity(e),
        (_, Term::Const(a)) => Range::Attribute(a),
        _ => Range::All,
    }
}

/// The triple bytes of `f` as committed keys hold them, or `None` when no
/// committed fact can have its triple (an entity, ident or long string the
/// generation does not hold).
fn committed_triple(keys: &dyn KeySource, f: &Fact) -> Result<Option<Vec<u8>>> {
    let (Some(e), Some(a)) = (keys.eid_of(&f.entity)?, keys.iid_of(&f.attribute)?) else {
        return Ok(None);
    };
    Ok(key_value(keys, &f.value)?.map(|v| keys::entity_attribute_value_prefix(e, a, &v)))
}

/// `value` as committed keys hold it, or `None` when no committed key can.
fn key_value(keys: &dyn KeySource, value: &Value) -> Result<Option<KeyValue>> {
    Ok(match value {
        Value::Null => Some(KeyValue::Null),
        Value::Boolean(b) => Some(KeyValue::Bool(*b)),
        Value::Integer(n) => Some(KeyValue::Int(*n)),
        Value::Float(f) => Some(KeyValue::Float(*f)),
        Value::String(s) if s.len() <= SHORT_STRING_MAX => Some(KeyValue::Str(s.clone())),
        Value::String(s) => keys.long_value_ref(s)?.map(|vref| keys::long_str(s, vref)),
        Value::Keyword(k) => keys.iid_of(k)?.map(KeyValue::Keyword),
        Value::Ref(u) => keys.eid_of(u)?.map(KeyValue::Ref),
    })
}

/// The valid time a fact must cover, `None` for any.
fn valid_at(valid: Valid, ctx: &Ctx<'_>) -> Option<i64> {
    match valid {
        Valid::Now => Some(ctx.snapshot.now()),
        Valid::At(t) => Some(t),
        Valid::Any => None,
    }
}

fn visible(valid: Option<i64>, vf: i64, vt: i64) -> bool {
    valid.is_none_or(|t| vf <= t && t < vt)
}

/// Apply one position to `row`: bind its slot, or check it against the slot
/// an earlier position bound. Returns false when the row does not match.
fn fill(
    term: &Term<impl Sized>,
    row: &mut [Option<Cell>],
    cell: impl FnOnce() -> Result<Cell>,
) -> Result<bool> {
    match term {
        Term::Any | Term::Const(_) => Ok(true),
        Term::Bind(slot) => {
            if let Some(s) = row.get_mut(*slot) {
                *s = Some(cell()?);
            }
            Ok(true)
        }
        Term::Eq(slot) => Ok(row.get(*slot).and_then(Option::as_ref) == Some(&cell()?)),
    }
}
