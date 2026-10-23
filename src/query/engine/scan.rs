//! `Scan` (#473 spec §6.1): one pattern over one key range of the snapshot.
//!
//! Every pending fact is newer than every committed one, so a triple with a
//! pending record in the snapshot (at or before `:as-of`) is decided by its
//! pending records alone, and a triple without one by its committed keys
//! (#379). `Scan` reads the committed ranges first, leaving out the triples a
//! pending record decides (the shadow set), then the pending range.
//!
//! In a rule query, a scan then reads the rows of the relations its pattern
//! names (#548 spec §5.2); a `Delta` read reads those rows only. A derived
//! row is visible at every time and has no metadata (`nil`).

use super::batch::Batch;
use super::cell::{Cell, JoinKey, Scalar};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, key_bytes, value_bytes};
use super::plan::{Meta, ScanSpec, Term, Valid, ValueConst, names};
use super::relation::Range as Rows;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::storage::PendingResume;
use crate::graph::types::{EntityId, Fact, Value};
use crate::storage::keys::{self, Index, KeyFact, KeyValue, SHORT_STRING_MAX, ValueKind};
use crate::storage::{KeySource, Scan as Range};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;

pub(crate) struct Scan {
    spec: ScanSpec,
    /// The entity of the current probe of a join's inner scan, by UUID and
    /// DICT id (`None` for an entity only pending facts hold).
    probe: Option<(EntityId, Option<u64>)>,
    /// The DICT ids of the attribute and value constants, resolved once for
    /// every probe; `Some(None)` when no committed key can match them.
    consts: Option<Option<Consts>>,
    state: State,
    keys_buf: Vec<KeyFact>,
    facts_buf: Vec<Fact>,
    /// What a value probe reads besides its key ranges (#721), from the
    /// query's [`Probes`], and the join key of the current probe.
    values: Option<Arc<Values>>,
    value_key: Option<JoinKey>,
    /// `rows_in` counts the live keys and pending facts read.
    counters: Counters,
}

/// What value probes read once for the whole query (#721), so that a rule
/// run in every round, or a body run per outer row, reads it once.
pub(crate) struct Probes {
    /// Per attribute, its pending facts and the committed triples they
    /// decide.
    values: HashMap<String, Arc<Values>>,
    /// Per attribute id, whether any of its live committed values is a
    /// keyword.
    keywords: HashMap<u32, bool>,
    /// The keyword that names an entity, where the query made the entity
    /// from the keyword (a rule head, #548 spec §5.3). A keyword's entity
    /// is a hash of its name, so no index leads back from the entity.
    named: HashMap<u64, u64>,
    held: Held,
}

impl Probes {
    pub(crate) fn new(meter: &Arc<Meter>) -> Self {
        Probes {
            values: HashMap::new(),
            keywords: HashMap::new(),
            named: HashMap::new(),
            held: Held::new(meter, "IndexNestedLoopJoin"),
        }
    }

    /// Record that the keyword with id `iid` names the entity `eid`.
    pub(crate) fn note(&mut self, eid: u64, iid: u64) -> Result<()> {
        if let std::collections::hash_map::Entry::Vacant(v) = self.named.entry(eid) {
            self.held
                .charge(HASH_ENTRY_OVERHEAD + size_of::<(u64, u64)>())?;
            v.insert(iid);
        }
        Ok(())
    }
}

/// An attribute's reads for value probes.
struct Values {
    /// Its live pending facts, by the join key of their value.
    pending: HashMap<JoinKey, Vec<Fact>>,
    /// The committed triples a pending record of it decides.
    shadow: Arc<HashSet<Vec<u8>>>,
}

enum State {
    Start,
    Committed(Committed),
    Pending(Option<PendingResume>),
    /// A value probe's pending facts.
    Bucket(Vec<Fact>),
    Derived(Derived),
    Done,
}

/// Where the read of derived rows is.
#[derive(Default)]
struct Derived {
    /// The relation read, as an index into the spec's.
    k: usize,
    /// The next position: in the relation's rows, or in its rows of the
    /// entity read.
    at: usize,
    /// The entity read (a probe's, or a constant's) and the cell of an
    /// exact value constant, resolved on the first pull.
    resolved: Option<(Option<u64>, Option<Cell>)>,
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
    shadow: Arc<HashSet<Vec<u8>>>,
}

/// The attribute and value constants of a scan, against the DICT.
#[derive(Clone)]
struct Consts {
    a: Option<u32>,
    test: Option<KeyTest>,
}

/// A value constant over committed key values, with ids resolved.
#[derive(Clone)]
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
            (KeyTest::Float(f), KeyValue::Float(ff)) => f == ff,
            _ => false,
        })
    }
}

impl Scan {
    pub(crate) fn new(spec: ScanSpec) -> Self {
        Scan {
            spec,
            probe: None,
            consts: None,
            state: State::Start,
            keys_buf: Vec::new(),
            facts_buf: Vec::new(),
            values: None,
            value_key: None,
            counters: Counters::new("Scan"),
        }
    }

    /// Start again, over the facts of the entity `uuid` (DICT id `eid`)
    /// only: one probe of an `IndexNestedLoopJoin`.
    pub(crate) fn probe(&mut self, uuid: EntityId, eid: Option<u64>) {
        self.probe = Some((uuid, eid));
        self.value_key = None;
        self.state = State::Start;
    }

    /// Start again, over the facts whose value may join-equal `cell` only:
    /// one probe of an `IndexNestedLoopJoin` on the value (#721). The
    /// committed facts come from the AVET ranges of the values equal to
    /// `cell` under the join rules, the pending ones from the attribute's
    /// facts with its key; the join checks every key, so these need only
    /// hold every match. `false`, and nothing to read, when the scan cannot
    /// find them: `cell` is an entity no keyword the query met names, and
    /// the attribute has committed keyword values, any of which may name it.
    pub(crate) fn probe_value(&mut self, cell: &Cell, ctx: &mut Ctx<'_>) -> Result<bool> {
        self.probe = None;
        self.value_key = None;
        self.state = State::Done;
        let Term::Const(attribute) = &self.spec.a else {
            return Ok(true);
        };
        if self.spec.never() {
            return Ok(true);
        }
        if self.consts.is_none()
            && let Some(keys) = ctx.keys
        {
            self.consts = Some(consts(&self.spec, keys)?);
        }
        if self.values.is_none() {
            self.values = Some(values(attribute, ctx)?);
        }
        let Some(key) = JoinKey::of(cell, true, ctx)? else {
            return Ok(true);
        };
        let a = match (&self.consts, ctx.keys) {
            (Some(Some(Consts { a: Some(a), .. })), Some(_)) => Some(*a),
            _ => None,
        };
        let ranges = match a {
            Some(a) => match value_ranges(cell, a, ctx)? {
                Some(r) => r,
                None => return Ok(false),
            },
            None => Vec::new(),
        };
        self.value_key = Some(key);
        self.state = if ranges.is_empty() {
            self.bucket()
        } else {
            let shadow = self
                .values
                .as_ref()
                .map_or_else(Arc::default, |v| v.shadow.clone());
            State::Committed(Committed {
                ranges,
                next: 0,
                resume: None,
                test: None,
                shadow,
            })
        };
        Ok(true)
    }

    /// After the committed ranges: a value probe's pending facts, or the
    /// pending range.
    fn bucket(&self) -> State {
        match (&self.value_key, &self.values) {
            (Some(k), Some(v)) => State::Bucket(v.pending.get(k).cloned().unwrap_or_default()),
            (Some(_), None) => State::Done,
            _ => State::Pending(None),
        }
    }
}

/// The live pending facts of `attribute` by the join key of their value,
/// and the committed triples its pending records decide (#721): read once
/// per query.
fn values(attribute: &str, ctx: &mut Ctx<'_>) -> Result<Arc<Values>> {
    if let Some(v) = ctx.ids.probes.values.get(attribute) {
        return Ok(v.clone());
    }
    let mut held = Held::new(ctx.ids.probes.held.meter(), "IndexNestedLoopJoin");
    let mut pending: HashMap<JoinKey, Vec<Fact>> = HashMap::new();
    let mut resume = None;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let more = ctx.snapshot.pending_live(
            Range::Attribute(attribute),
            ctx.bound,
            &mut resume,
            BATCH_ROWS,
            &mut buf,
        )?;
        for f in buf.drain(..) {
            let cell = Cell::of_value(ctx.ids, ctx.keys, &f.value)?;
            let Some(k) = JoinKey::of(&cell, true, ctx)? else {
                continue;
            };
            held.charge(
                HASH_ENTRY_OVERHEAD
                    + key_bytes(&k)
                    + size_of::<Fact>()
                    + f.entity.as_bytes().len()
                    + f.attribute.len()
                    + value_bytes(&f.value),
            )?;
            pending.entry(k).or_default().push(f);
        }
        if !more {
            break;
        }
    }
    let mut shadow = HashSet::new();
    if let Some(keys) = ctx.keys {
        for f in ctx
            .snapshot
            .pending_records(Range::Attribute(attribute), ctx.bound)?
        {
            if let Some(t) = committed_triple(keys, &f)? {
                held.charge(HASH_ENTRY_OVERHEAD + size_of::<Vec<u8>>() + t.len())?;
                shadow.insert(t);
            }
        }
    }
    let v = Arc::new(Values {
        pending,
        shadow: Arc::new(shadow),
    });
    held.charge(HASH_ENTRY_OVERHEAD + size_of::<(String, Arc<Values>)>() + attribute.len())?;
    let probes = &mut ctx.ids.probes;
    probes.held.absorb(held);
    probes.values.insert(attribute.to_string(), v.clone());
    Ok(v)
}

/// Key ranges as `(index, prefix)`.
type Ranges = Vec<(Index, Vec<u8>)>;

/// The AVET ranges of attribute `a` holding every committed value that
/// join-equals `cell`, or `None` when they cannot be found (see
/// [`Scan::probe_value`]).
fn value_ranges(cell: &Cell, a: u32, ctx: &mut Ctx<'_>) -> Result<Option<Ranges>> {
    let Some(keys) = ctx.keys else {
        return Ok(Some(Vec::new()));
    };
    let mut kvs = Vec::new();
    match cell {
        // A keyword, and a ref to the entity it names.
        Cell::Iid(i) => {
            if let Some(di) = ctx.ids.dict_iid(*i) {
                kvs.push(KeyValue::Keyword(di));
            }
            let e = ctx.ids.keyword_eid(ctx.keys, *i)?;
            if let Some(de) = ctx.ids.dict_eid(e) {
                kvs.push(KeyValue::Ref(de));
            }
        }
        // A ref, and the keyword that names the entity.
        Cell::Eid(e) => {
            if let Some(de) = ctx.ids.dict_eid(*e) {
                kvs.push(KeyValue::Ref(de));
            }
            match ctx.ids.probes.named.get(e).copied() {
                Some(i) => kvs.extend(ctx.ids.dict_iid(i).map(KeyValue::Keyword)),
                None if has_keywords(a, ctx)? => return Ok(None),
                None => {}
            }
        }
        // `-0.0` equals `0.0`; NaN has no key and never gets here.
        Cell::Val(Scalar::Float(f)) => {
            kvs.push(KeyValue::Float(*f));
            if *f == 0.0 {
                kvs.push(KeyValue::Float(-*f));
            }
        }
        Cell::Val(_) => {
            let v = super::project::value(cell, ctx)?;
            kvs.extend(key_value(keys, &v)?);
        }
    }
    Ok(Some(
        kvs.iter()
            .map(|v| (Index::Avet, keys::attribute_value_prefix(a, v)))
            .collect(),
    ))
}

/// Whether any live committed value of attribute `a` is a keyword: read
/// once per query.
fn has_keywords(a: u32, ctx: &mut Ctx<'_>) -> Result<bool> {
    if let Some(&k) = ctx.ids.probes.keywords.get(&a) {
        return Ok(k);
    }
    let Some(keys) = ctx.keys else {
        return Ok(false);
    };
    let mut buf = Vec::new();
    keys.live_keys(
        Index::Avet,
        &keys::attribute_kind_prefix(a, ValueKind::Keyword),
        None,
        ctx.bound,
        1,
        &mut buf,
    )?;
    let k = !buf.is_empty();
    let probes = &mut ctx.ids.probes;
    probes
        .held
        .charge(HASH_ENTRY_OVERHEAD + size_of::<(u32, bool)>())?;
    probes.keywords.insert(a, k);
    Ok(k)
}

impl Operator for Scan {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        loop {
            let mut batch = Batch::new(self.spec.width);
            match &mut self.state {
                State::Start if self.spec.never() => self.state = State::Done,
                State::Start
                    if self
                        .spec
                        .derived
                        .as_ref()
                        .is_some_and(|d| d.range == Rows::Delta) =>
                {
                    self.state = State::Derived(Derived::default());
                }
                State::Start => {
                    if self.consts.is_none()
                        && let Some(keys) = ctx.keys
                    {
                        self.consts = Some(consts(&self.spec, keys)?);
                    }
                    self.state = open(&self.spec, self.probe.as_ref(), self.consts.as_ref(), ctx)?;
                }
                State::Committed(c) => {
                    let Some(keys) = ctx.keys else {
                        self.state = self.bucket();
                        continue;
                    };
                    let Some((index, prefix)) = c.ranges.get(c.next) else {
                        self.state = self.bucket();
                        continue;
                    };
                    self.keys_buf.clear();
                    c.resume = keys.live_keys(
                        *index,
                        prefix,
                        c.resume.as_deref(),
                        ctx.bound,
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
                                    Meta::ValidFrom => Some(kf.vf),
                                    Meta::ValidTo => Some(kf.vt),
                                    Meta::TxCount => Some(kf.tx_count.cast_signed()),
                                    Meta::TxId => Some(keys.tx_id_of(kf.tx_count)?.cast_signed()),
                                    Meta::ValidAt => valid,
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
                        range(&self.spec, self.probe.as_ref()),
                        ctx.bound,
                        resume,
                        BATCH_ROWS,
                        &mut self.facts_buf,
                    )?;
                    self.counters.rows_in += self.facts_buf.len() as u64;
                    push_facts(&self.spec, &self.facts_buf, &mut batch, ctx)?;
                    if !more {
                        self.state = if self.spec.derived.is_some() {
                            State::Derived(Derived::default())
                        } else {
                            State::Done
                        };
                    }
                }
                State::Bucket(facts) => {
                    let rest = facts.split_off(facts.len().min(BATCH_ROWS));
                    let now = std::mem::replace(facts, rest);
                    self.counters.rows_in += now.len() as u64;
                    push_facts(&self.spec, &now, &mut batch, ctx)?;
                    if now.is_empty() {
                        self.state = State::Done;
                    }
                }
                State::Derived(d) => {
                    if !derived(&self.spec, self.probe.as_ref(), d, &mut batch, ctx)? {
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

    /// Start again; the constants resolved against the DICT are kept.
    fn rewind(&mut self, _seed: &[Cell]) {
        self.probe = None;
        self.value_key = None;
        self.state = State::Start;
    }

    fn close(&mut self) {
        self.state = State::Done;
        self.keys_buf = Vec::new();
        self.facts_buf = Vec::new();
        self.values = None;
        self.value_key = None;
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
    }
}

/// Push the rows of pending `facts` that match `spec` and are visible.
fn push_facts(spec: &ScanSpec, facts: &[Fact], batch: &mut Batch, ctx: &mut Ctx<'_>) -> Result<()> {
    let valid = valid_at(spec.valid, ctx);
    let mut row = vec![None; spec.width];
    for f in facts {
        if !visible(valid, f.valid_from, f.valid_to)
            || matches!(&spec.v, Term::Const(v) if !v.matches(&f.value))
        {
            continue;
        }
        let keys = ctx.keys;
        let ids = &mut *ctx.ids;
        if fill(&spec.e, &mut row, || {
            Ok(Cell::Eid(ids.eid(keys, &f.entity)?))
        })? && fill(&spec.a, &mut row, || {
            Ok(Cell::Iid(ids.iid(keys, &f.attribute)?))
        })? && fill(&spec.v, &mut row, || Cell::of_value(ids, keys, &f.value))?
        {
            for &(m, slot) in &spec.meta {
                let n = match m {
                    Meta::ValidFrom => Some(f.valid_from),
                    Meta::ValidTo => Some(f.valid_to),
                    Meta::TxCount => Some(f.tx_count.cast_signed()),
                    Meta::TxId => Some(f.tx_id.cast_signed()),
                    Meta::ValidAt => valid,
                };
                set(&mut row, slot, n);
            }
            batch.push(&mut row);
        }
        row.iter_mut().for_each(|c| *c = None);
    }
    Ok(())
}

/// Read derived rows of `spec` into `batch` until it is full; `false` once
/// every relation has been read.
fn derived(
    spec: &ScanSpec,
    probe: Option<&(EntityId, Option<u64>)>,
    d: &mut Derived,
    batch: &mut Batch,
    ctx: &mut Ctx<'_>,
) -> Result<bool> {
    let (Some(read), Some(rels)) = (&spec.derived, ctx.derived) else {
        return Ok(false);
    };
    let (target, exact) = match &d.resolved {
        Some(r) => r.clone(),
        None => {
            let target = match (probe, &spec.e) {
                (Some((u, _)), _) | (None, Term::Const(u)) => Some(ctx.ids.eid(ctx.keys, u)?),
                _ => None,
            };
            let exact = match &spec.v {
                Term::Const(ValueConst::Exact(v)) => Some(Cell::of_value(ctx.ids, ctx.keys, v)?),
                _ => None,
            };
            d.resolved = Some((target, exact.clone()));
            (target, exact)
        }
    };
    let valid = valid_at(spec.valid, ctx);
    let mut row = vec![None; spec.width];
    while batch.len < BATCH_ROWS {
        let Some(&rel) = read.rels.get(d.k) else {
            return Ok(false);
        };
        let span = rels.span(rel, read.range);
        let pos = match target {
            Some(e) => rels.of_entity(rel, e).get(d.at).copied(),
            None => Some(span.start + d.at).filter(|p| *p < span.end),
        };
        let Some(pos) = pos else {
            d.k += 1;
            d.at = 0;
            continue;
        };
        d.at += 1;
        let (Some(r), Some(iid)) = (rels.row(rel, pos), rels.iid(rel)) else {
            continue;
        };
        if !span.contains(&pos) {
            continue;
        }
        let keeps = match (&spec.v, &exact) {
            (_, Some(c)) => r[1] == *c,
            (Term::Const(c), None) => cell_matches(c, &r[1], ctx)?,
            _ => true,
        };
        if keeps
            && fill(&spec.e, &mut row, || Ok(r[0].clone()))?
            && fill(&spec.a, &mut row, || Ok(Cell::Iid(iid)))?
            && fill(&spec.v, &mut row, || Ok(r[1].clone()))?
        {
            for &(m, slot) in &spec.meta {
                set(
                    &mut row,
                    slot,
                    if m == Meta::ValidAt { valid } else { None },
                );
            }
            // `set` gives `nil` for `None`; the valid time under
            // `:any-valid-time` is `nil` too.
            batch.push(&mut row);
        }
        row.iter_mut().for_each(|c| *c = None);
    }
    Ok(true)
}

/// Whether the derived cell `cell` matches the value constant `c`, as
/// [`ValueConst::matches`] matches its value.
fn cell_matches(c: &ValueConst, cell: &Cell, ctx: &mut Ctx<'_>) -> Result<bool> {
    Ok(match (c, cell) {
        (ValueConst::Exact(v), cell) => Cell::of_value(ctx.ids, ctx.keys, v)? == *cell,
        (ValueConst::Keyword(k, _), Cell::Iid(i)) => *ctx.ids.name(ctx.keys, *i)? == **k,
        (ValueConst::Keyword(_, u) | ValueConst::Uuid(u), Cell::Eid(e)) => {
            ctx.ids.uuid(ctx.keys, *e)? == *u
        }
        (ValueConst::Uuid(u), Cell::Iid(i)) => names(&ctx.ids.name(ctx.keys, *i)?, u),
        (ValueConst::Float(f), Cell::Val(Scalar::Float(ff))) => f == ff,
        _ => false,
    })
}

/// Fill a metadata slot: an integer, or `nil` for the valid time under
/// `:any-valid-time`.
fn set(row: &mut [Option<Cell>], slot: usize, n: Option<i64>) {
    if let Some(s) = row.get_mut(slot) {
        *s = Some(Cell::Val(n.map_or(Scalar::Null, Scalar::Int)));
    }
}

/// The DICT ids of the constants of `spec`, or `None` when no committed key
/// can match them.
fn consts(spec: &ScanSpec, keys: &dyn KeySource) -> Result<Option<Consts>> {
    let a = match &spec.a {
        Term::Const(name) => match keys.iid_of(name)? {
            Some(a) => Some(a),
            None => return Ok(None),
        },
        _ => None,
    };
    let test = match &spec.v {
        Term::Const(c) => match key_test(keys, c)? {
            Some(t) => Some(t),
            None => return Ok(None),
        },
        _ => None,
    };
    Ok(Some(Consts { a, test }))
}

/// Resolve the committed ranges of `spec` (of the entity `probe`, when
/// given) and the pending triples that shadow them.
fn open(
    spec: &ScanSpec,
    probe: Option<&(EntityId, Option<u64>)>,
    consts: Option<&Option<Consts>>,
    ctx: &mut Ctx<'_>,
) -> Result<State> {
    let pending = State::Pending(None);
    let (Some(keys), Some(Some(Consts { a, test }))) = (ctx.keys, consts) else {
        return Ok(pending);
    };
    let (a, test) = (*a, test.clone());
    let e = match (probe, &spec.e) {
        (Some((_, Some(e))), _) => Some(*e),
        (Some((_, None)), _) => return Ok(pending),
        (None, Term::Const(u)) => match keys.eid_of(u)? {
            Some(e) => Some(e),
            None => return Ok(pending),
        },
        (None, _) => None,
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
    for f in ctx
        .snapshot
        .pending_records(range(spec, probe), ctx.bound)?
    {
        if let Some(t) = committed_triple(keys, &f)? {
            shadow.insert(t);
        }
    }
    Ok(State::Committed(Committed {
        ranges,
        next: 0,
        resume: None,
        test,
        shadow: Arc::new(shadow),
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

/// The pending range of `spec` (of the entity `probe`, when given).
fn range<'a>(spec: &'a ScanSpec, probe: Option<&'a (EntityId, Option<u64>)>) -> Range<'a> {
    let e = match (probe, &spec.e) {
        (Some((u, _)), _) | (None, Term::Const(u)) => Some(u),
        _ => None,
    };
    match (e, &spec.a) {
        (Some(e), Term::Const(a)) => Range::EntityAttribute(e, a),
        (Some(e), _) => Range::Entity(e),
        (None, Term::Const(a)) => Range::Attribute(a),
        (None, _) => Range::All,
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
        Term::Never => Ok(false),
        Term::Bind(slot) => {
            if let Some(s) = row.get_mut(*slot) {
                *s = Some(cell()?);
            }
            Ok(true)
        }
        Term::Eq(slot) => Ok(row.get(*slot).and_then(Option::as_ref) == Some(&cell()?)),
    }
}
