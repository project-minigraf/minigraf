//! `IndexNestedLoopJoin` and `HashJoin` (#473 spec §6.2): the rows so far
//! (the outer side) joined with one more pattern (the inner side).
//!
//! The inner pattern is a [`Scan`] over slots of its own. Two cells join
//! when their [`JoinKey`]s are equal (spec §5.4): floats by IEEE `==`, a
//! keyword with a ref to the entity it names. A joined row keeps the outer
//! cell of each shared variable, except where the plan says the pattern
//! gives its value (`JoinSpec::replace`).

use super::batch::Batch;
use super::cell::{Cell, JoinKey, join_eq};
use super::plan::{JoinKind, JoinSpec, Key};
use super::scan::Scan;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use anyhow::Result;
use std::collections::HashMap;

type Row = Vec<Cell>;

pub(crate) struct Join {
    outer: Box<dyn Operator>,
    inner: Scan,
    kind: JoinKind,
    keys: Vec<Key>,
    new: Vec<usize>,
    replace: Vec<(usize, usize)>,
    outer_width: usize,
    state: State,
    /// `rows_in` counts the outer rows received.
    counters: Counters,
}

enum State {
    Start,
    Probe(Probe),
    Hash(Hash),
    Done,
}

/// `IndexNestedLoopJoin`: the rows of the current outer batch, the one
/// probed, whether its inner scan is open, and the inner rows read but not
/// yet joined.
#[derive(Default)]
struct Probe {
    rows: Vec<Row>,
    at: usize,
    probing: bool,
    matches: Vec<Row>,
    next: usize,
}

/// `HashJoin`: the table of the build side, the rows of the streamed side's
/// current batch, the one matched, and the next row of its bucket.
struct Hash {
    table: Table,
    rows: Vec<Row>,
    at: usize,
    bucket: Option<usize>,
    next: usize,
}

/// The build side of a `HashJoin`: its rows grouped by key.
#[derive(Default)]
struct Table {
    by_key: HashMap<Vec<JoinKey>, usize>,
    buckets: Vec<Vec<Row>>,
}

/// Which side a row comes from.
#[derive(Clone, Copy)]
enum Side {
    Outer,
    Inner,
}

impl Join {
    pub(crate) fn new(outer: Box<dyn Operator>, spec: JoinSpec) -> Self {
        let op = match spec.kind {
            JoinKind::Probe(_) => "IndexNestedLoopJoin",
            JoinKind::Hash { .. } => "HashJoin",
        };
        Join {
            outer,
            inner: Scan::new(spec.inner),
            kind: spec.kind,
            keys: spec.keys,
            new: spec.new,
            replace: spec.replace,
            outer_width: spec.outer_width,
            state: State::Start,
            counters: Counters::new(op),
        }
    }

    /// The key of `row`, from `side`, or `None` when a cell equals nothing.
    fn key(&self, row: &[Cell], side: Side, ctx: &mut Ctx<'_>) -> Result<Option<Vec<JoinKey>>> {
        let mut key = Vec::with_capacity(self.keys.len());
        for k in &self.keys {
            let slot = match side {
                Side::Outer => k.outer,
                Side::Inner => k.inner,
            };
            let Some(cell) = row.get(slot) else {
                return Ok(None);
            };
            match JoinKey::of(cell, k.normalise, ctx)? {
                Some(j) => key.push(j),
                None => return Ok(None),
            }
        }
        Ok(Some(key))
    }

    /// Append the join of `outer` and `inner` to `out`.
    fn emit(&self, outer: &[Cell], inner: &[Cell], out: &mut Batch) {
        let cells = outer
            .iter()
            .take(self.outer_width)
            .enumerate()
            .map(|(slot, cell)| {
                self.replace
                    .iter()
                    .find(|(_, o)| *o == slot)
                    .and_then(|(i, _)| inner.get(*i))
                    .unwrap_or(cell)
            })
            .chain(self.new.iter().filter_map(|i| inner.get(*i)));
        for (col, cell) in out.cols.iter_mut().zip(cells) {
            col.push(cell.clone());
        }
        out.len += 1;
    }

    /// The next rows of `side`, or `None` once it has ended.
    fn pull(&mut self, side: Side, ctx: &mut Ctx<'_>) -> Result<Option<Vec<Row>>> {
        let batch = match side {
            Side::Outer => self.outer.next_batch(ctx)?,
            Side::Inner => self.inner.next_batch(ctx)?,
        };
        Ok(batch.map(|b| {
            if let Side::Outer = side {
                self.counters.rows_in += b.len as u64;
            }
            rows(&b)
        }))
    }

    /// Read all of `side` into a table.
    fn build(&mut self, side: Side, ctx: &mut Ctx<'_>) -> Result<Table> {
        let mut table = Table::default();
        while let Some(rows) = self.pull(side, ctx)? {
            for row in rows {
                let Some(key) = self.key(&row, side, ctx)? else {
                    continue;
                };
                let next = table.buckets.len();
                let b = *table.by_key.entry(key).or_insert(next);
                if b == next {
                    table.buckets.push(Vec::new());
                }
                if let Some(bucket) = table.buckets.get_mut(b) {
                    bucket.push(row);
                }
            }
        }
        Ok(table)
    }

    /// The entity of an outer row to probe: its UUID and DICT id, or `None`
    /// when the cell is no entity (a scalar never equals one).
    fn probe_of(
        &self,
        row: &[Cell],
        k: usize,
        ctx: &mut Ctx<'_>,
    ) -> Result<Option<(crate::graph::types::EntityId, Option<u64>)>> {
        let Some(key) = self.keys.get(k) else {
            return Ok(None);
        };
        let Some(cell) = row.get(key.outer) else {
            return Ok(None);
        };
        let Some(JoinKey::Eid(e)) = JoinKey::of(cell, true, ctx)? else {
            return Ok(None);
        };
        let uuid = ctx.ids.uuid(ctx.keys, e)?;
        Ok(Some((uuid, ctx.ids.dict_eid(e))))
    }

    /// Whether `outer` and `inner` agree on every key but the probed one.
    fn agree(
        &self,
        outer: &[Cell],
        inner: &[Cell],
        probed: usize,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        for (n, k) in self.keys.iter().enumerate() {
            if n == probed {
                continue;
            }
            let (Some(o), Some(i)) = (outer.get(k.outer), inner.get(k.inner)) else {
                return Ok(false);
            };
            if !join_eq(o, i, k.normalise, ctx)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Join outer rows into `out` until it is full; `false` once the outer
    /// side has ended.
    fn next_probe(
        &mut self,
        p: &mut Probe,
        probed: usize,
        out: &mut Batch,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        while out.len < BATCH_ROWS {
            if let Some(o) = p.rows.get(p.at) {
                if let Some(m) = p.matches.get(p.next) {
                    p.next += 1;
                    if self.agree(o, m, probed, ctx)? {
                        self.emit(o, m, out);
                    }
                } else if p.probing {
                    match self.inner.next_batch(ctx)? {
                        Some(b) => p.matches = rows(&b),
                        None => {
                            p.matches.clear();
                            p.probing = false;
                            p.at += 1;
                        }
                    }
                    p.next = 0;
                } else {
                    match self.probe_of(o, probed, ctx)? {
                        Some((uuid, eid)) => {
                            self.inner.probe(uuid, eid);
                            p.probing = true;
                        }
                        None => p.at += 1,
                    }
                }
                continue;
            }
            match self.pull(Side::Outer, ctx)? {
                Some(r) => {
                    p.rows = r;
                    p.at = 0;
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Join streamed rows into `out` until it is full; `false` once the
    /// streamed side has ended.
    fn next_hash(
        &mut self,
        h: &mut Hash,
        build_inner: bool,
        out: &mut Batch,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        let streamed = if build_inner {
            Side::Outer
        } else {
            Side::Inner
        };
        while out.len < BATCH_ROWS {
            if let Some(s) = h.rows.get(h.at) {
                let matched = h
                    .bucket
                    .and_then(|b| h.table.buckets.get(b))
                    .and_then(|r| r.get(h.next));
                if let Some(m) = matched {
                    h.next += 1;
                    if build_inner {
                        self.emit(s, m, out);
                    } else {
                        self.emit(m, s, out);
                    }
                } else if h.bucket.is_some() {
                    h.bucket = None;
                    h.at += 1;
                } else {
                    let key = self.key(s, streamed, ctx)?;
                    match key.and_then(|k| h.table.by_key.get(&k).copied()) {
                        Some(b) => {
                            h.bucket = Some(b);
                            h.next = 0;
                        }
                        None => h.at += 1,
                    }
                }
                continue;
            }
            match self.pull(streamed, ctx)? {
                Some(r) => {
                    h.rows = r;
                    h.at = 0;
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }
}

impl Operator for Join {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        let mut out = Batch::new(self.outer_width + self.new.len());
        if let State::Start = self.state {
            self.state = match self.kind {
                JoinKind::Probe(_) => State::Probe(Probe::default()),
                JoinKind::Hash { build_inner } => {
                    let built = if build_inner {
                        Side::Inner
                    } else {
                        Side::Outer
                    };
                    let table = self.build(built, ctx)?;
                    if table.buckets.is_empty() {
                        State::Done
                    } else {
                        State::Hash(Hash {
                            table,
                            rows: Vec::new(),
                            at: 0,
                            bucket: None,
                            next: 0,
                        })
                    }
                }
            };
        }
        // Taken out while it runs; an error leaves the join done.
        let more = match (std::mem::replace(&mut self.state, State::Done), self.kind) {
            (State::Probe(mut p), JoinKind::Probe(k)) => {
                let more = self.next_probe(&mut p, k, &mut out, ctx)?;
                self.state = State::Probe(p);
                more
            }
            (State::Hash(mut h), JoinKind::Hash { build_inner }) => {
                let more = self.next_hash(&mut h, build_inner, &mut out, ctx)?;
                self.state = State::Hash(h);
                more
            }
            _ => false,
        };
        if !more {
            self.close();
        }
        if out.len == 0 {
            return Ok(None);
        }
        self.counters.out(&out);
        Ok(Some(out))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.state = State::Start;
        self.outer.rewind(seed);
        self.inner.rewind(&[]);
    }

    fn close(&mut self) {
        self.state = State::Done;
        self.outer.close();
        self.inner.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.inner.counters(out);
        self.outer.counters(out);
    }
}

/// The rows of `batch`.
fn rows(batch: &Batch) -> Vec<Row> {
    (0..batch.len)
        .map(|i| {
            batch
                .cols
                .iter()
                .filter_map(|c| c.get(i).cloned())
                .collect()
        })
        .collect()
}
