//! `Distinct` (#546 spec §5.2): after an `or`'s `Union`, keeps the first of
//! the rows equal in every variable, as the executor's `Value`s compare:
//! floats by IEEE `==` (`-0.0` equals `0.0`, a NaN equals nothing, so a row
//! holding one is always kept), and a keyword never equal to a ref.

use super::batch::Batch;
use super::cell::{Cell, JoinKey};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, growth, keys_bytes};
use super::{Counters, Ctx, Operator};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) struct Distinct {
    child: Box<dyn Operator>,
    slots: Vec<usize>,
    /// The rows returned since the last rewind.
    seen: HashSet<Vec<JoinKey>>,
    held: Held,
    counters: Counters,
}

impl Distinct {
    pub(crate) fn new(child: Box<dyn Operator>, slots: Vec<usize>, meter: &Arc<Meter>) -> Self {
        Distinct {
            child,
            slots,
            seen: HashSet::new(),
            held: Held::new(meter, "Distinct"),
            counters: Counters::new("Distinct"),
        }
    }

    /// Whether row `i` of `batch` is the first of its kind.
    fn first(&mut self, batch: &Batch, i: usize, ctx: &mut Ctx<'_>) -> Result<bool> {
        let mut key = Vec::with_capacity(self.slots.len());
        for &slot in &self.slots {
            let Some(c) = batch.cols.get(slot).and_then(|c| c.get(i)) else {
                return Ok(true);
            };
            let Some(k) = JoinKey::of(c, false, ctx)? else {
                return Ok(true);
            };
            key.push(k);
        }
        if self.seen.contains(&key) {
            return Ok(false);
        }
        let n = keys_bytes(&key) + HASH_ENTRY_OVERHEAD;
        self.held.charge(n)?;
        let refused = growth(self.seen.capacity(), n);
        self.held.reserve(self.seen.try_reserve(1), n, refused)?;
        Ok(self.seen.insert(key))
    }
}

impl Operator for Distinct {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        while let Some(mut batch) = self.child.next_batch(ctx)? {
            self.counters.rows_in += batch.len as u64;
            let mut keep = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                keep.push(self.first(&batch, i, ctx)?);
            }
            batch.retain(&keep);
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.seen.clear();
        self.held.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.seen = HashSet::new();
        self.held.release();
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: self.held.bytes() as u64,
            ..self.counters
        });
        self.child.counters(out);
    }
}
