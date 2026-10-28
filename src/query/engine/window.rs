//! `Window` (#547 spec §4): each input row once, in input order, with the
//! value of each window of `:find`.
//!
//! A window's value depends on every row of its partition, so `Window`
//! holds its input: each row's passed-on cells, and the values of its
//! named variables (spec §2), which the windows sort and partition by and
//! accumulate. When the input ends, the values come from
//! [`windows`](crate::query::datalog::aggregate::windows), the code the
//! executor runs. Snapshot-certified input instead completes one partition
//! at a time, with a charged lookahead batch. Completed partitions share a
//! passing output batch; input-batch boundaries stop unnecessary lookahead
//! when output is already available (#641).

use super::batch::Batch;
use super::cell::{Cell, JoinKey, Scalar};
use super::meter::{Held, Meter, cell_bytes, growth, row_bytes, value_bytes};
use super::plan::PartitionOrder;
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::error::{ErrorCode, err_coded};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::{WindowColumn, WindowColumnMeter, windows};
use crate::query::datalog::types::FindSpec;
use anyhow::Result;
use std::collections::TryReserveError;
use std::mem::size_of;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    pub(super) static PARTITION_MAX_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct Window {
    child: Box<dyn Operator>,
    find: Vec<FindSpec>,
    /// The slots of the named variables.
    named: Vec<usize>,
    /// The slots passed on, before the windows' columns.
    keep: Vec<usize>,
    /// A snapshot-certified partition key and its physical comparator.
    partition: Option<(usize, PartitionOrder)>,
    input: Option<Batch>,
    at: usize,
    key: Option<Cell>,
    ended: bool,
    /// Retained lookahead and the current scalar/id key.
    held_input: Held,
    /// Each input row's cells at `keep`, in input order.
    cells: Vec<Vec<Cell>>,
    /// Each input row's values of the named variables.
    values: Vec<Vec<Value>>,
    /// Finished rows, last first, for one partition or the full input.
    out: Option<Vec<Vec<Cell>>>,
    /// `cells`, then the finished rows, released as they are returned
    /// (#549 spec §3.2).
    held: Held,
    /// `values`, until the windows are computed.
    held_values: Held,
    /// Retained window result columns, including their outer vector.
    held_columns: Held,
    counters: Counters,
}

impl Window {
    pub(crate) fn new(
        child: Box<dyn Operator>,
        find: Vec<FindSpec>,
        named: Vec<usize>,
        keep: Vec<usize>,
        partition: Option<(usize, PartitionOrder)>,
        meter: &Arc<Meter>,
    ) -> Self {
        #[cfg(test)]
        if partition.is_some() {
            PARTITION_MAX_ROWS.with(|n| n.set(0));
        }
        Window {
            child,
            find,
            named,
            keep,
            partition,
            input: None,
            at: 0,
            key: None,
            ended: false,
            held_input: Held::new(meter, "Window"),
            cells: Vec::new(),
            values: Vec::new(),
            out: None,
            held: Held::new(meter, "Window"),
            held_values: Held::new(meter, "Window"),
            held_columns: Held::new(meter, "Window"),
            counters: Counters::new(if partition.is_some() {
                "WindowPartitioned"
            } else {
                "Window"
            }),
        }
    }

    /// Hold each row of `batch`.
    fn take(&mut self, batch: &Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        for i in 0..batch.len {
            self.take_row(batch, i, ctx)?;
        }
        Ok(())
    }

    fn take_row(&mut self, batch: &Batch, i: usize, ctx: &mut Ctx<'_>) -> Result<()> {
        let null = Cell::Val(Scalar::Null);
        let cell = |slot: usize| batch.cols.get(slot).and_then(|c| c.get(i)).unwrap_or(&null);
        let cells: Vec<Cell> = self.keep.iter().map(|&s| cell(s).clone()).collect();
        let n = row_bytes(&cells);
        self.held.charge(n)?;
        let refused = growth(self.cells.capacity(), size_of::<Vec<Cell>>());
        self.held.reserve(self.cells.try_reserve(1), n, refused)?;
        let values = self
            .named
            .iter()
            .map(|&s| value(cell(s), ctx))
            .collect::<Result<Vec<Value>>>()?;
        let v = size_of::<Vec<Value>>() + values.iter().map(value_bytes).sum::<usize>();
        self.held_values.charge(v)?;
        let refused = growth(self.values.capacity(), size_of::<Vec<Value>>());
        self.held_values
            .reserve(self.values.try_reserve(1), v, refused)?;
        self.cells.push(cells);
        self.values.push(values);
        #[cfg(test)]
        if self.partition.is_some() {
            PARTITION_MAX_ROWS.with(|n| n.set(n.get().max(self.cells.len())));
        }
        Ok(())
    }

    fn input_bytes(batch: &Batch) -> usize {
        size_of::<Batch>()
            + batch.cols.len() * size_of::<Vec<Cell>>()
            + batch.cols.iter().flatten().map(cell_bytes).sum::<usize>()
    }

    fn release_key(&mut self) {
        if let Some(key) = self.key.take() {
            self.held_input.uncharge(cell_bytes(&key));
        }
    }

    fn increasing(
        order: PartitionOrder,
        previous: &Cell,
        next: &Cell,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        Ok(match (order, previous, next) {
            (PartitionOrder::Entity, Cell::Eid(a), Cell::Eid(b)) => b > a,
            (PartitionOrder::Ident, Cell::Iid(a), Cell::Iid(b)) => b > a,
            (PartitionOrder::Values, a, b) => value(b, ctx)?.cmp(&value(a, ctx)?).is_gt(),
            _ => false,
        })
    }

    /// Load one complete partition. A boundary row remains in the charged
    /// lookahead until this partition's results have all been returned.
    fn load_partition(
        &mut self,
        slot: usize,
        order: PartitionOrder,
        stop_at_batch_end: bool,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        loop {
            let batch = match self.input.take() {
                Some(batch) => batch,
                None => match self.child.next_batch(ctx)? {
                    Some(batch) => {
                        debug_assert!(batch.len <= BATCH_ROWS);
                        self.held_input.charge(Self::input_bytes(&batch))?;
                        self.counters.rows_in += batch.len as u64;
                        batch
                    }
                    None => {
                        self.ended = true;
                        self.release_key();
                        self.out = Some(if self.cells.is_empty() {
                            Vec::new()
                        } else {
                            self.finish(ctx)?
                        });
                        return Ok(true);
                    }
                },
            };
            while self.at < batch.len {
                let key = batch
                    .cols
                    .get(slot)
                    .and_then(|c| c.get(self.at))
                    .ok_or_else(|| {
                        err_coded!(ErrorCode::Int032, "window partition key is missing")
                    })?
                    .clone();
                if let Some(previous) = &self.key {
                    if JoinKey::group(previous) != JoinKey::group(&key) {
                        if !Self::increasing(order, previous, &key, ctx)? {
                            return Err(err_coded!(
                                ErrorCode::Int032,
                                "window partition input is not increasing"
                            ));
                        }
                        self.input = Some(batch);
                        self.release_key();
                        self.out = Some(self.finish(ctx)?);
                        return Ok(true);
                    }
                } else {
                    self.held_input.charge(cell_bytes(&key))?;
                    self.key = Some(key);
                }
                self.take_row(&batch, self.at, ctx)?;
                self.at += 1;
            }
            let bytes = Self::input_bytes(&batch);
            drop(batch);
            self.held_input.uncharge(bytes);
            self.at = 0;
            // DICT ids and value-page references remain valid under the pin.
            // Provisional ids survive this eviction.
            ctx.ids.end_pull();
            // Already completed rows can be returned without reading another
            // child batch just to finish an unneeded later partition.
            if stop_at_batch_end {
                return Ok(false);
            }
        }
    }

    /// Every row, in input order, with its windows' cells.
    fn finish(&mut self, ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Cell>>> {
        let result = self.finish_rows(ctx);
        if result.is_err() {
            self.held.release();
            self.held_values.release();
            self.held_columns.release();
        }
        result
    }

    fn finish_rows(&mut self, ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Cell>>> {
        let values = std::mem::take(&mut self.values);
        let mut rows = std::mem::take(&mut self.cells);
        let mut columns = windows(&self.find, &values, ctx.functions, &mut self.held_columns)?;
        drop(values);
        self.held_values.release();
        let outer = columns.len() * size_of::<WindowColumn>();
        for column in columns.drain(..) {
            for (at, (row, v)) in rows.iter_mut().zip(&column.values).enumerate() {
                let c = Cell::of_value(ctx.ids, ctx.keys, v)?;
                let n = cell_bytes(&c);
                self.held.charge(n)?;
                let refused = growth(row.capacity(), size_of::<Cell>());
                self.held.reserve(row.try_reserve(1), n, refused)?;
                row.push(c);
                if self.partition.is_some() && (at + 1) % BATCH_ROWS == 0 {
                    ctx.ids.end_pull();
                }
            }
            // Ref/keyword-valued results can populate the lookup maps again
            // even after input ended. Bound those conversions too, and release
            // a completed partition before coalescing another into the output.
            if self.partition.is_some() {
                ctx.ids.end_pull();
            }
            let bytes = column.bytes;
            drop(column);
            self.held_columns.uncharge(bytes);
        }
        drop(columns);
        self.held_columns.uncharge(outer);
        rows.reverse();
        Ok(rows)
    }
}

impl Operator for Window {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        let width = self.keep.len()
            + self
                .find
                .iter()
                .filter(|s| matches!(s, FindSpec::Window(_)))
                .count();
        let mut batch = Batch::new(width);
        loop {
            if self.out.as_ref().is_none_or(Vec::is_empty) {
                if let Some((slot, order)) = self.partition {
                    self.out = None;
                    if self.ended || !self.load_partition(slot, order, batch.len > 0, ctx)? {
                        break;
                    }
                } else if self.out.is_none() {
                    while let Some(input) = self.child.next_batch(ctx)? {
                        self.counters.rows_in += input.len as u64;
                        self.take(&input, ctx)?;
                    }
                    self.out = Some(self.finish(ctx)?);
                } else {
                    break;
                }
            }
            let Some(rows) = self.out.as_mut().filter(|r| !r.is_empty()) else {
                break;
            };
            while batch.len < BATCH_ROWS
                && let Some(row) = rows.pop()
            {
                self.held.uncharge(row_bytes(&row));
                batch.push(&mut row.into_iter().map(Some).collect::<Vec<_>>());
            }
            if batch.len == BATCH_ROWS || self.partition.is_none() {
                break;
            }
        }
        if batch.len == 0 {
            return Ok(None);
        }
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.input = None;
        self.at = 0;
        self.key = None;
        self.ended = false;
        self.held_input.release();
        self.cells.clear();
        self.values.clear();
        self.out = None;
        self.held.release();
        self.held_values.release();
        self.held_columns.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.input = None;
        self.key = None;
        self.at = 0;
        self.ended = true;
        self.held_input.release();
        self.cells = Vec::new();
        self.values = Vec::new();
        self.out = Some(Vec::new());
        self.held.release();
        self.held_values.release();
        self.held_columns.release();
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: (self.held.bytes()
                + self.held_values.bytes()
                + self.held_columns.bytes()
                + self.held_input.bytes()) as u64,
            ..self.counters
        });
        self.child.counters(out);
    }
}

impl WindowColumnMeter for Held {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        Held::charge(self, bytes)
    }

    fn uncharge(&mut self, bytes: usize) {
        Held::uncharge(self, bytes);
    }

    fn reserve(
        &mut self,
        result: std::result::Result<(), TryReserveError>,
        charged: usize,
        refused: usize,
    ) -> Result<()> {
        Held::reserve(self, result, charged, refused)
    }

    fn charge_payload(&mut self, value: &Value) -> Result<usize> {
        let bytes = value_bytes(value) - size_of::<Value>();
        Held::charge(self, bytes)?;
        Ok(bytes)
    }
}
