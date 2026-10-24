//! `CollectSet` (#316 spec §7.3): `[(collect-set ?e :a) ?s]`, per row the
//! set of the values of `[e :a ?v]` under the query's view.
//!
//! The values come from a one-entity probe of the pattern's `Scan`, the
//! probe an `IndexNestedLoopJoin` makes for a bound entity. A cell that is
//! no entity, or a keyword naming none, gives `#{}`; the row is kept.

use super::batch::Batch;
use super::cell::{Cell, JoinKey, Scalar, join_eq};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, growth, value_bytes};
use super::plan::{CollectSpec, Out};
use super::project::value;
use super::scan::Scan;
use super::{Counters, Ctx, Operator};
use crate::error::{ErrorCode, bail_coded};
use crate::graph::types::{Value, ValueSet};
use anyhow::Result;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) struct CollectSet {
    child: Box<dyn Operator>,
    inner: Scan,
    entity: usize,
    attribute: String,
    out: usize,
    mode: Out,
    /// The values of the set being built (#549).
    held: Held,
    counters: Counters,
}

impl CollectSet {
    pub(crate) fn new(child: Box<dyn Operator>, spec: CollectSpec, meter: &Arc<Meter>) -> Self {
        CollectSet {
            child,
            inner: Scan::new(spec.inner),
            entity: spec.entity,
            attribute: spec.attribute,
            out: spec.out,
            mode: spec.mode,
            held: Held::new(meter, "CollectSet"),
            counters: Counters::new("CollectSet"),
        }
    }

    /// The set of row `i`'s entity.
    fn set(&mut self, batch: &Batch, i: usize, ctx: &mut Ctx<'_>) -> Result<Cell> {
        let mut values: Vec<Value> = Vec::new();
        let cell = batch.cols.get(self.entity).and_then(|c| c.get(i));
        let entity = match cell {
            Some(cell) => JoinKey::of(cell, true, ctx)?,
            None => None,
        };
        if let Some(JoinKey::Eid(e)) = entity {
            let uuid = ctx.ids.uuid(ctx.keys, e)?;
            self.inner.probe(uuid, ctx.ids.dict_eid(e));
            while let Some(found) = self.inner.next_batch(ctx)? {
                for v in found.cols.get(1).into_iter().flatten() {
                    let v = value(v, ctx)?;
                    if let Value::Set(_) = v {
                        bail_coded!(ErrorCode::Qry015, &self.attribute);
                    }
                    let n = value_bytes(&v) + HASH_ENTRY_OVERHEAD;
                    self.held.charge(n)?;
                    let refused = growth(values.capacity(), size_of::<Value>());
                    self.held.reserve(values.try_reserve(1), n, refused)?;
                    values.push(v);
                }
            }
        }
        let set = ValueSet::new(values)?;
        self.held.release();
        Ok(Cell::Val(Scalar::Set(Arc::new(set))))
    }
}

impl Operator for CollectSet {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        while let Some(mut batch) = self.child.next_batch(ctx)? {
            self.counters.rows_in += batch.len as u64;
            let mut keep = Vec::with_capacity(batch.len);
            let mut out = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                let cell = self.set(&batch, i, ctx)?;
                let old = match self.mode {
                    Out::New => None,
                    Out::Check | Out::Replace => batch.cols.get(self.out).and_then(|c| c.get(i)),
                };
                // As `Bind` (#596).
                match old {
                    Some(old) if !join_eq(old, &cell, true, ctx)? => keep.push(false),
                    Some(old) if self.mode == Out::Check => {
                        out.push(old.clone());
                        keep.push(true);
                    }
                    _ => {
                        out.push(cell);
                        keep.push(true);
                    }
                }
            }
            batch.retain(&keep);
            batch.set_col(self.out, out);
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.held.release();
        self.child.rewind(seed);
        self.inner.rewind(&[]);
    }

    fn close(&mut self) {
        self.held.release();
        self.child.close();
        self.inner.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.inner.counters(out);
        self.child.counters(out);
    }
}
