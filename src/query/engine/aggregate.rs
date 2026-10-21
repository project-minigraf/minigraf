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
use super::meter::{
    HASH_ENTRY_OVERHEAD, Held, Meter, cell_bytes, growth, key_bytes, keys_bytes, row_bytes,
    value_bytes,
};
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::{Agg, Groups, finish_groups, value_order};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;

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

    /// Take `cell`, counting a new member of a distinct set on `held`.
    fn step(&mut self, cell: &Cell, held: &mut Held, ctx: &mut Ctx<'_>) -> Result<()> {
        if matches!(cell, Cell::Val(Scalar::Null)) {
            return Ok(());
        }
        match self {
            Acc::Count(n) => *n = n.saturating_add(1),
            Acc::CountDistinct(set) => {
                let k = JoinKey::group(cell);
                if !set.contains(&k) {
                    let n = key_bytes(&k) + HASH_ENTRY_OVERHEAD;
                    held.charge(n)?;
                    let refused = growth(set.capacity(), n);
                    held.reserve(set.try_reserve(1), n, refused)?;
                    set.insert(k);
                }
            }
            Acc::Values(agg) => {
                let v = value(cell, ctx)?;
                if agg.would_add(&v) {
                    held.charge(value_bytes(&v) + HASH_ENTRY_OVERHEAD)?;
                }
                agg.step(&v);
            }
        }
        Ok(())
    }

    /// The accumulator as an [`Agg`], whose `finish` gives its result.
    fn into_agg(self) -> Agg {
        match self {
            Acc::Count(n) => Agg::Count(n),
            Acc::CountDistinct(set) => Agg::Count(i64::try_from(set.len()).unwrap_or(i64::MAX)),
            Acc::Values(agg) => agg,
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
    /// Each group's index in `rows`, by its key.
    table: HashMap<Vec<JoinKey>, usize>,
    rows: Vec<Group>,
    /// The finished rows, last first, once the input has ended.
    out: Option<Vec<Vec<Cell>>>,
    /// The groups (#549 spec §3.2).
    held: Held,
    /// The finished rows, released as they are returned.
    finished: Held,
    counters: Counters,
}

impl Aggregate {
    pub(crate) fn new(
        child: Box<dyn Operator>,
        groups: Vec<usize>,
        aggs: Vec<(String, usize)>,
        zero_row: bool,
        meter: &Arc<Meter>,
    ) -> Self {
        Aggregate {
            child,
            groups,
            aggs,
            zero_row,
            table: HashMap::new(),
            rows: Vec::new(),
            out: None,
            held: Held::new(meter, "Aggregate"),
            finished: Held::new(meter, "Aggregate"),
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
            let at = match self.table.get(&key) {
                Some(&at) => at,
                None => {
                    let accs = self
                        .aggs
                        .iter()
                        .map(|(func, _)| Acc::new(func, ctx))
                        .collect::<Result<Vec<_>>>()?;
                    let cells: Vec<Cell> = self
                        .groups
                        .iter()
                        .map(|&s| cell(s, i).unwrap_or(&null).clone())
                        .collect();
                    // The key, its index, the group: its cells and
                    // accumulators.
                    let entry = keys_bytes(&key) + size_of::<usize>() + HASH_ENTRY_OVERHEAD;
                    let group = size_of::<Group>()
                        + cells.iter().map(cell_bytes).sum::<usize>()
                        + accs.len() * size_of::<Acc>();
                    let n = entry + group;
                    self.held.charge(n)?;
                    let refused = growth(self.table.capacity(), entry)
                        .saturating_add(growth(self.rows.capacity(), size_of::<Group>()));
                    let reserved = self
                        .table
                        .try_reserve(1)
                        .and_then(|()| self.rows.try_reserve(1));
                    self.held.reserve(reserved, n, refused)?;
                    self.rows.push(Group { cells, accs });
                    self.table.insert(key, self.rows.len() - 1);
                    self.rows.len() - 1
                }
            };
            let Some(group) = self.rows.get_mut(at) else {
                continue;
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
                    acc.step(c, &mut self.held, ctx)?;
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
        // Sorted and finished as the executor's groups are.
        let mut groups = Groups::new();
        for group in std::mem::take(&mut self.rows) {
            let values = group
                .cells
                .iter()
                .map(|c| value(c, ctx))
                .collect::<Result<Vec<Value>>>()?;
            let accs = group.accs.into_iter().map(Acc::into_agg).collect();
            groups.insert(values.clone(), (values, accs));
        }
        self.table.clear();
        let funcs: Vec<&str> = self.aggs.iter().map(|(f, _)| f.as_str()).collect();
        let mut rows = Vec::new();
        for (shown, results) in finish_groups(groups, &funcs)? {
            let row = shown
                .iter()
                .chain(&results)
                .map(|v| Cell::of_value(ctx.ids, ctx.keys, v))
                .collect::<Result<Vec<Cell>>>()?;
            let n = row_bytes(&row);
            self.finished.charge(n)?;
            let refused = growth(rows.capacity(), size_of::<Vec<Cell>>());
            self.finished.reserve(rows.try_reserve(1), n, refused)?;
            rows.push(row);
        }
        self.held.release();
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
            self.finished.uncharge(row_bytes(&row));
            batch.push(&mut row.into_iter().map(Some).collect::<Vec<_>>());
        }
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.table.clear();
        self.rows.clear();
        self.out = None;
        self.held.release();
        self.finished.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.table = HashMap::new();
        self.rows = Vec::new();
        self.out = Some(Vec::new());
        self.held.release();
        self.finished.release();
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: (self.held.bytes() + self.finished.bytes()) as u64,
            ..self.counters
        });
        self.child.counters(out);
    }
}
