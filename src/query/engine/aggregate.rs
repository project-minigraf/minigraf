//! `Aggregate` (#547 spec §5): one row per group of its input, with the
//! result of each aggregate of `:find`.
//!
//! Groups are keyed on their cells as `Value`'s `Ord` compares values (spec
//! §2), in a hash table: one entry for a query with no grouping variable, so
//! it runs in constant memory. Cells become values only where an
//! accumulator needs one: `count` reads only whether a cell is `nil`, and
//! `count-distinct` keeps the keys of its cells. When the input ends, the
//! groups are sorted by their values, as the executor's `BTreeMap` orders
//! them, and finished in that order (spec §3.3). A group returns, of the
//! floats its key finds equal (`-0.0` and `0.0`, NaNs), the least by
//! `value_order`, as the executor does.

use super::batch::Batch;
use super::cell::{Cell, JoinKey, Scalar};
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::{Agg, value_order};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// One aggregate of one group.
enum Acc {
    /// The number of cells that are not `nil`.
    Count(i64),
    /// The keys of the cells that are not `nil`.
    CountDistinct(HashSet<JoinKey>),
    /// Any other aggregate, over values.
    Values(Agg),
}

impl Acc {
    fn new(func: &str, ctx: &Ctx<'_>) -> Result<Acc> {
        Ok(match Agg::new(func, ctx.functions)? {
            Agg::Count(_) => Acc::Count(0),
            Agg::CountDistinct(_) => Acc::CountDistinct(HashSet::new()),
            agg => Acc::Values(agg),
        })
    }

    fn step(&mut self, cell: &Cell, ctx: &mut Ctx<'_>) -> Result<()> {
        if matches!(cell, Cell::Val(Scalar::Null)) {
            return Ok(());
        }
        match self {
            Acc::Count(n) => *n = n.saturating_add(1),
            Acc::CountDistinct(set) => {
                set.insert(JoinKey::group(cell));
            }
            Acc::Values(agg) => agg.step(&value(cell, ctx)?),
        }
        Ok(())
    }

    /// As [`Agg::finish`]: `None` drops the group.
    fn finish(self, func: &str) -> Result<Option<Value>> {
        match self {
            Acc::Count(n) => Ok(Some(Value::Integer(n))),
            Acc::CountDistinct(set) => Ok(Some(Value::Integer(
                i64::try_from(set.len()).unwrap_or(i64::MAX),
            ))),
            Acc::Values(agg) => agg.finish(func),
        }
    }
}

/// A group: its cells, and one accumulator per aggregate.
struct Group {
    cells: Vec<Cell>,
    accs: Vec<Acc>,
}

pub(crate) struct Aggregate {
    child: Box<dyn Operator>,
    /// The slots of the grouping variables.
    groups: Vec<usize>,
    /// Each aggregate's function and argument slot.
    aggs: Vec<(String, usize)>,
    zero_row: bool,
    table: HashMap<Vec<JoinKey>, Group>,
    /// The finished rows, last first, once the input has ended.
    out: Option<Vec<Vec<Cell>>>,
    counters: Counters,
}

impl Aggregate {
    pub(crate) fn new(
        child: Box<dyn Operator>,
        groups: Vec<usize>,
        aggs: Vec<(String, usize)>,
        zero_row: bool,
    ) -> Self {
        Aggregate {
            child,
            groups,
            aggs,
            zero_row,
            table: HashMap::new(),
            out: None,
            counters: Counters::new("Aggregate"),
        }
    }

    /// Step the group of each row of `batch`.
    fn take(&mut self, batch: &Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        let null = Cell::Val(Scalar::Null);
        // A slot a row lacks reads `nil`, as a missing binding does on the
        // executor.
        let cell = |slot: usize, i: usize| batch.cols.get(slot).and_then(|c| c.get(i));
        for i in 0..batch.len {
            let key: Vec<JoinKey> = self
                .groups
                .iter()
                .map(|&s| JoinKey::group(cell(s, i).unwrap_or(&null)))
                .collect();
            let group = match self.table.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let accs = self
                        .aggs
                        .iter()
                        .map(|(func, _)| Acc::new(func, ctx))
                        .collect::<Result<Vec<_>>>()?;
                    let cells = self
                        .groups
                        .iter()
                        .map(|&s| cell(s, i).unwrap_or(&null).clone())
                        .collect();
                    e.insert(Group { cells, accs })
                }
            };
            // Of the floats a key finds equal, the group returns the least
            // by `value_order`, as the executor does: the only cells that
            // are key-equal and not the same value.
            for (shown, &s) in group.cells.iter_mut().zip(&self.groups) {
                if let (Cell::Val(Scalar::Float(old)), Some(Cell::Val(Scalar::Float(new)))) =
                    (&*shown, cell(s, i))
                    && value_order(&Value::Float(*new), &Value::Float(*old)).is_lt()
                {
                    *shown = Cell::Val(Scalar::Float(*new));
                }
            }
            for (acc, (_, slot)) in group.accs.iter_mut().zip(&self.aggs) {
                if let Some(c) = cell(*slot, i) {
                    acc.step(c, ctx)?;
                }
            }
        }
        Ok(())
    }

    /// The rows of every group, in the order of their values, finished.
    fn finish(&mut self, ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Cell>>> {
        if self.table.is_empty() && self.zero_row {
            // `:with` cells, which `:find` does not return, then the zeros.
            let mut row = vec![Cell::Val(Scalar::Null); self.groups.len()];
            row.extend(vec![Cell::Val(Scalar::Int(0)); self.aggs.len()]);
            return Ok(vec![row]);
        }
        let mut groups = Vec::with_capacity(self.table.len());
        for (_, group) in std::mem::take(&mut self.table) {
            let values = group
                .cells
                .iter()
                .map(|c| value(c, ctx))
                .collect::<Result<Vec<Value>>>()?;
            groups.push((values, group));
        }
        groups.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut rows = Vec::with_capacity(groups.len());
        'group: for (_, group) in groups {
            let mut row = group.cells;
            for (acc, (func, _)) in group.accs.into_iter().zip(&self.aggs) {
                match acc.finish(func)? {
                    Some(v) => row.push(Cell::of_value(ctx.ids, ctx.keys, &v)?),
                    None => continue 'group,
                }
            }
            rows.push(row);
        }
        rows.reverse();
        Ok(rows)
    }
}

impl Operator for Aggregate {
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
        let width = self.groups.len() + self.aggs.len();
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
        self.table.clear();
        self.out = None;
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.table = HashMap::new();
        self.out = Some(Vec::new());
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.child.counters(out);
    }
}
