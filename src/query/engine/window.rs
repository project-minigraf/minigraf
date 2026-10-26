//! `Window` (#547 spec §4): each input row once, in input order, with the
//! value of each window of `:find`.
//!
//! A window's value depends on every row of its partition, so `Window`
//! holds its input: each row's passed-on cells, and the values of its
//! named variables (spec §2), which the windows sort and partition by and
//! accumulate. When the input ends, the values come from
//! [`windows`](crate::query::datalog::aggregate::windows), the code the
//! executor runs.

use super::batch::Batch;
use super::cell::{Cell, Scalar};
use super::meter::{Held, Meter, cell_bytes, growth, row_bytes, value_bytes};
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::{WindowColumn, WindowColumnMeter, windows};
use crate::query::datalog::types::FindSpec;
use anyhow::Result;
use std::collections::TryReserveError;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) struct Window {
    child: Box<dyn Operator>,
    find: Vec<FindSpec>,
    /// The slots of the named variables.
    named: Vec<usize>,
    /// The slots passed on, before the windows' columns.
    keep: Vec<usize>,
    /// Each input row's cells at `keep`, in input order.
    cells: Vec<Vec<Cell>>,
    /// Each input row's values of the named variables.
    values: Vec<Vec<Value>>,
    /// The finished rows, last first, once the input has ended.
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
        meter: &Arc<Meter>,
    ) -> Self {
        Window {
            child,
            find,
            named,
            keep,
            cells: Vec::new(),
            values: Vec::new(),
            out: None,
            held: Held::new(meter, "Window"),
            held_values: Held::new(meter, "Window"),
            held_columns: Held::new(meter, "Window"),
            counters: Counters::new("Window"),
        }
    }

    /// Hold each row of `batch`.
    fn take(&mut self, batch: &Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        let null = Cell::Val(Scalar::Null);
        // A slot a row lacks reads `nil`, as a missing binding does on the
        // executor.
        let cell =
            |slot: usize, i: usize| batch.cols.get(slot).and_then(|c| c.get(i)).unwrap_or(&null);
        for i in 0..batch.len {
            let cells: Vec<Cell> = self.keep.iter().map(|&s| cell(s, i).clone()).collect();
            let n = row_bytes(&cells);
            self.held.charge(n)?;
            let refused = growth(self.cells.capacity(), size_of::<Vec<Cell>>());
            self.held.reserve(self.cells.try_reserve(1), n, refused)?;
            let values = self
                .named
                .iter()
                .map(|&s| value(cell(s, i), ctx))
                .collect::<Result<Vec<Value>>>()?;
            let v = size_of::<Vec<Value>>() + values.iter().map(value_bytes).sum::<usize>();
            self.held_values.charge(v)?;
            let refused = growth(self.values.capacity(), size_of::<Vec<Value>>());
            self.held_values
                .reserve(self.values.try_reserve(1), v, refused)?;
            self.cells.push(cells);
            self.values.push(values);
        }
        Ok(())
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
            for (row, v) in rows.iter_mut().zip(&column.values) {
                let c = Cell::of_value(ctx.ids, ctx.keys, v)?;
                let n = cell_bytes(&c);
                self.held.charge(n)?;
                let refused = growth(row.capacity(), size_of::<Cell>());
                self.held.reserve(row.try_reserve(1), n, refused)?;
                row.push(c);
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
        if self.out.is_none() {
            while let Some(batch) = self.child.next_batch(ctx)? {
                self.counters.rows_in += batch.len as u64;
                self.take(&batch, ctx)?;
            }
            self.out = Some(self.finish(ctx)?);
        }
        let Some(rows) = self.out.as_mut().filter(|r| !r.is_empty()) else {
            return Ok(None);
        };
        let width = self.keep.len()
            + self
                .find
                .iter()
                .filter(|s| matches!(s, FindSpec::Window(_)))
                .count();
        let mut batch = Batch::new(width);
        while batch.len < BATCH_ROWS
            && let Some(row) = rows.pop()
        {
            self.held.uncharge(row_bytes(&row));
            batch.push(&mut row.into_iter().map(Some).collect::<Vec<_>>());
        }
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.cells.clear();
        self.values.clear();
        self.out = None;
        self.held.release();
        self.held_values.release();
        self.held_columns.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
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
            bytes_held: (self.held.bytes() + self.held_values.bytes() + self.held_columns.bytes())
                as u64,
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
