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
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::windows;
use crate::query::datalog::types::FindSpec;
use anyhow::Result;

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
    counters: Counters,
}

impl Window {
    pub(crate) fn new(
        child: Box<dyn Operator>,
        find: Vec<FindSpec>,
        named: Vec<usize>,
        keep: Vec<usize>,
    ) -> Self {
        Window {
            child,
            find,
            named,
            keep,
            cells: Vec::new(),
            values: Vec::new(),
            out: None,
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
            self.cells
                .push(self.keep.iter().map(|&s| cell(s, i).clone()).collect());
            self.values.push(
                self.named
                    .iter()
                    .map(|&s| value(cell(s, i), ctx))
                    .collect::<Result<Vec<Value>>>()?,
            );
        }
        Ok(())
    }

    /// Every row, in input order, with its windows' cells.
    fn finish(&mut self, ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Cell>>> {
        let values = std::mem::take(&mut self.values);
        let mut rows = std::mem::take(&mut self.cells);
        for column in windows(&self.find, &values, ctx.functions)? {
            for (row, v) in rows.iter_mut().zip(&column) {
                row.push(Cell::of_value(ctx.ids, ctx.keys, v)?);
            }
        }
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
            batch.push(&mut row.into_iter().map(Some).collect::<Vec<_>>());
        }
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.cells.clear();
        self.values.clear();
        self.out = None;
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.cells = Vec::new();
        self.values = Vec::new();
        self.out = Some(Vec::new());
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.child.counters(out);
    }
}
