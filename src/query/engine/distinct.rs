//! `Distinct` (#546 spec §5.2): after an `or`'s `Union`, keeps the first of
//! the rows equal in every variable, as the executor's `Value`s compare:
//! floats by IEEE `==` (`-0.0` equals `0.0`, a NaN equals nothing, so a row
//! holding one is always kept), and a keyword never equal to a ref.
//!
//! Projection mode (`:distinct`, #493) instead uses aggregate-group equality:
//! all NaNs are one value. Returned floats, including set elements, use
//! canonical `-0.0`/NaN so encounter order never decides their representation.

use super::batch::Batch;
use super::cell::{Cell, JoinKey, Scalar};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, growth, keys_bytes};
use super::{Counters, Ctx, Operator};
use crate::graph::types::{Value, ValueSet};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) struct Distinct {
    child: Box<dyn Operator>,
    slots: Vec<usize>,
    projected: bool,
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
            projected: false,
            seen: HashSet::new(),
            held: Held::new(meter, "Distinct"),
            counters: Counters::new("Distinct"),
        }
    }

    pub(crate) fn projected(
        child: Box<dyn Operator>,
        slots: Vec<usize>,
        meter: &Arc<Meter>,
    ) -> Self {
        Self {
            projected: true,
            ..Self::new(child, slots, meter)
        }
    }

    /// Whether row `i` of `batch` is the first of its kind.
    fn first(&mut self, batch: &Batch, i: usize, ctx: &mut Ctx<'_>) -> Result<bool> {
        let mut key = Vec::with_capacity(self.slots.len());
        for &slot in &self.slots {
            let Some(c) = batch.cols.get(slot).and_then(|c| c.get(i)) else {
                return Ok(true);
            };
            let k = if self.projected {
                JoinKey::group(c)
            } else {
                let Some(k) = JoinKey::of(c, false, ctx)? else {
                    return Ok(true);
                };
                k
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
                let first = self.first(&batch, i, ctx)?;
                if first && self.projected {
                    for &slot in &self.slots {
                        if let Some(c) = batch.cols.get_mut(slot).and_then(|col| col.get_mut(i)) {
                            canonical(c)?;
                        }
                    }
                }
                keep.push(first);
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

/// Canonical representatives without buffering the entire distinct answer.
fn canonical(cell: &mut Cell) -> Result<()> {
    fn float(f: f64) -> f64 {
        if f == 0.0 {
            -0.0
        } else if f.is_nan() {
            f64::NAN
        } else {
            f
        }
    }
    match cell {
        Cell::Val(Scalar::Float(f)) => *f = float(*f),
        Cell::Val(Scalar::Set(set))
            if set
                .iter()
                .any(|v| matches!(v, Value::Float(f) if f.to_bits() != float(*f).to_bits())) =>
        {
            *set = Arc::new(ValueSet::new(set.iter().map(|v| match v {
                Value::Float(f) => Value::Float(float(*f)),
                other => other.clone(),
            }))?);
        }
        _ => {}
    }
    Ok(())
}
