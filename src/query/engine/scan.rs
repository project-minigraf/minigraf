//! `Scan` (#473 spec §6.1): one pattern over one key range of the snapshot.
//!
//! Every pending fact is newer than every committed one, so a triple with a
//! pending record in the snapshot (at or before `:as-of`) is decided by its
//! pending records alone, and a triple without one by its committed keys
//! (#379). `Scan` reads the committed range first, leaving out the triples a
//! pending record decides (the shadow set), then the pending range.

use super::batch::Batch;
use super::cell::{Cell, Scalar};
use super::plan::{ScanSpec, Term, Valid};
use super::{BATCH_ROWS, Ctx};
use crate::graph::storage::PendingResume;
use crate::graph::types::{Fact, Value};
use crate::storage::keys::{self, Index, KeyFact, KeyValue, SHORT_STRING_MAX};
use crate::storage::{KeySource, Scan as Range};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) struct Scan {
    spec: ScanSpec,
    state: State,
    keys_buf: Vec<KeyFact>,
    facts_buf: Vec<Fact>,
}

enum State {
    Start,
    Committed(Committed),
    Pending(Option<PendingResume>),
    Done,
}

/// The committed half of the read.
struct Committed {
    index: Index,
    prefix: Vec<u8>,
    resume: Option<Vec<u8>>,
    /// The value constant when the range does not fix it.
    value: Option<KeyValue>,
    /// Triples a pending record decides, as [`keys::entity_attribute_value_prefix`].
    shadow: HashSet<Vec<u8>>,
}

impl Scan {
    pub(crate) fn new(spec: ScanSpec) -> Self {
        Scan {
            spec,
            state: State::Start,
            keys_buf: Vec::new(),
            facts_buf: Vec::new(),
        }
    }

    pub(crate) fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        loop {
            let mut batch = Batch::new(self.spec.width);
            match &mut self.state {
                State::Start => self.state = open(&self.spec, ctx)?,
                State::Committed(c) => {
                    let Some(keys) = ctx.keys else {
                        self.state = State::Pending(None);
                        continue;
                    };
                    self.keys_buf.clear();
                    c.resume = keys.live_keys(
                        c.index,
                        &c.prefix,
                        c.resume.as_deref(),
                        self.spec.as_of,
                        BATCH_ROWS,
                        &mut self.keys_buf,
                    )?;
                    let valid = valid_at(self.spec.valid, ctx);
                    let mut row = vec![None; self.spec.width];
                    for kf in &self.keys_buf {
                        if !visible(valid, kf.vf, kf.vt)
                            || c.value.as_ref().is_some_and(|v| *v != kf.v)
                            || (!c.shadow.is_empty()
                                && c.shadow.contains(&keys::entity_attribute_value_prefix(
                                    kf.e, kf.a, &kf.v,
                                )))
                        {
                            continue;
                        }
                        if fill(&self.spec.e, &mut row, || Ok(Cell::Eid(kf.e)))?
                            && fill(&self.spec.a, &mut row, || Ok(Cell::Iid(u64::from(kf.a))))?
                            && fill(&self.spec.v, &mut row, || Ok(Cell::of_key(&kf.v)))?
                        {
                            batch.push(&mut row);
                        }
                        row.iter_mut().for_each(|c| *c = None);
                    }
                    if c.resume.is_none() {
                        self.state = State::Pending(None);
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
                    let valid = valid_at(self.spec.valid, ctx);
                    let mut row = vec![None; self.spec.width];
                    for f in &self.facts_buf {
                        if !visible(valid, f.valid_from, f.valid_to)
                            || matches!(&self.spec.v, Term::Const(v) if *v != f.value)
                        {
                            continue;
                        }
                        let keys = ctx.keys;
                        let ids = &mut ctx.ids;
                        if fill(&self.spec.e, &mut row, || {
                            Ok(Cell::Eid(ids.eid(keys, &f.entity)?))
                        })? && fill(&self.spec.a, &mut row, || {
                            Ok(Cell::Iid(ids.iid(keys, &f.attribute)?))
                        })? && fill(&self.spec.v, &mut row, || value_cell(ids, keys, &f.value))?
                        {
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
                return Ok(Some(batch));
            }
        }
    }
}

/// Resolve the committed range of `spec` and the pending triples that
/// shadow it.
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
    let v = match &spec.v {
        Term::Const(value) => match key_value(keys, value)? {
            Some(v) => Some(v),
            None => return Ok(pending),
        },
        _ => None,
    };
    let (index, prefix, value) = match (e, a, v) {
        (Some(e), Some(a), Some(v)) => (
            Index::Eavt,
            keys::entity_attribute_value_prefix(e, a, &v),
            None,
        ),
        (Some(e), Some(a), None) => (Index::Eavt, keys::entity_attribute_prefix(e, a), None),
        (Some(e), None, v) => (Index::Eavt, keys::entity_prefix(e), v),
        (None, Some(a), Some(v)) => (Index::Avet, keys::attribute_value_prefix(a, &v), None),
        (None, Some(a), None) => (Index::Aevt, keys::attribute_prefix(a), None),
        (None, None, v) => (Index::Eavt, Vec::new(), v),
    };
    let mut shadow = HashSet::new();
    for f in ctx.snapshot.pending_records(range(spec), spec.as_of)? {
        if let Some(t) = committed_triple(keys, &f)? {
            shadow.insert(t);
        }
    }
    Ok(State::Committed(Committed {
        index,
        prefix,
        resume: None,
        value,
        shadow,
    }))
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

/// The cell of a pending fact's value.
fn value_cell(
    ids: &mut super::ids::IdSpace,
    keys: Option<&dyn KeySource>,
    value: &Value,
) -> Result<Cell> {
    Ok(match value {
        Value::Null => Cell::Val(Scalar::Null),
        Value::Boolean(b) => Cell::Val(Scalar::Bool(*b)),
        Value::Integer(n) => Cell::Val(Scalar::Int(*n)),
        Value::Float(f) => Cell::Val(Scalar::Float(*f)),
        Value::String(s) => Cell::Val(Scalar::Str(Arc::from(s.as_str()))),
        Value::Keyword(k) => Cell::Iid(ids.iid(keys, k)?),
        Value::Ref(u) => Cell::Eid(ids.eid(keys, u)?),
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
