//! `Aggregate` (#547 spec §5): one row per group of its input, with the
//! result of each aggregate of `:find`.
//!
//! Groups are keyed on their cells as `Value`'s `Ord` compares values (spec
//! §2), in a hash table: one entry for a query with no grouping variable, so
//! it runs in constant memory. Cells become values only where an
//! accumulator needs one: `count` reads only whether a cell is `nil`, and
//! `count-distinct` keeps the keys of its cells. When the input ends, the
//! groups are sorted by their values, as the executor's `BTreeMap` orders
//! them, and finished in that order (spec §3.3). A snapshot-proved scalar
//! group key with only built-in count/count-distinct instead holds one group,
//! finishing it at the next key and returning a bounded batch (#641).
//! Other aggregates keep full-drain validation, including under Limit.
//! A group returns, of the
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
use crate::error::{ErrorCode, err_coded};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::{Agg, Groups, finish_group, finish_groups, value_order};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    pub(super) static ORDERED_MAX_GROUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
                agg.step(&v)?;
            }
        }
        Ok(())
    }

    /// The accumulator as an [`Agg`], whose `finish` gives its result.
    /// The bytes `step` charged for the members of a set that stays in the
    /// `Agg`: `sum-distinct`'s. A `count-distinct` set ends as its count.
    fn member_bytes(&self) -> usize {
        match self {
            Acc::Values(Agg::SumDistinct(set)) => set
                .iter()
                .map(|v| value_bytes(v) + HASH_ENTRY_OVERHEAD)
                .sum(),
            _ => 0,
        }
    }

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
    ordered: bool,
    ended: bool,
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
        ordered: bool,
        meter: &Arc<Meter>,
    ) -> Self {
        #[cfg(test)]
        if ordered {
            ORDERED_MAX_GROUPS.with(|n| n.set(0));
        }
        Aggregate {
            child,
            groups,
            aggs,
            zero_row,
            ordered,
            ended: false,
            table: HashMap::new(),
            rows: Vec::new(),
            out: None,
            held: Held::new(meter, "Aggregate"),
            finished: Held::new(meter, "Aggregate"),
            counters: Counters::new(if ordered {
                "AggregateOrdered"
            } else {
                "Aggregate"
            }),
        }
    }

    fn key(&self, batch: &Batch, i: usize) -> Vec<JoinKey> {
        let null = Cell::Val(Scalar::Null);
        self.groups
            .iter()
            .map(|&s| JoinKey::group(batch.cols.get(s).and_then(|c| c.get(i)).unwrap_or(&null)))
            .collect()
    }

    /// Step the group of each row of `batch`.
    fn take(&mut self, batch: &Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        for i in 0..batch.len {
            self.step(self.key(batch, i), batch, i, ctx)?;
        }
        Ok(())
    }

    fn step(
        &mut self,
        key: Vec<JoinKey>,
        batch: &Batch,
        i: usize,
        ctx: &mut Ctx<'_>,
    ) -> Result<()> {
        let null = Cell::Val(Scalar::Null);
        let cell = |slot: usize, i: usize| batch.cols.get(slot).and_then(|c| c.get(i));
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
                #[cfg(test)]
                if self.ordered {
                    ORDERED_MAX_GROUPS.with(|n| n.set(n.get().max(self.rows.len())));
                }
                self.table.insert(key, self.rows.len() - 1);
                self.rows.len() - 1
            }
        };
        let Some(group) = self.rows.get_mut(at) else {
            return Ok(());
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
        Ok(())
    }

    /// Only scalar keys and infallible count accumulators reach this path.
    /// Result rows stay charged until moved into a passing output batch.
    fn finish_one(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Vec<Cell>>> {
        self.table.clear();
        let Some(group) = self.rows.pop() else {
            return Ok(None);
        };
        let scratch = 4 * size_of::<Vec<Value>>()
            + group.cells.len() * size_of::<Value>()
            + group.accs.len() * (size_of::<Agg>() + size_of::<Value>() + size_of::<&str>());
        self.finished.charge(scratch)?;
        let shown = group
            .cells
            .iter()
            .map(|c| value(c, ctx))
            .collect::<Result<Vec<_>>>()?;
        let accs = group.accs.into_iter().map(Acc::into_agg).collect();
        let funcs: Vec<&str> = self.aggs.iter().map(|(f, _)| f.as_str()).collect();
        let results = finish_group(accs, &funcs)?;
        self.held.release();
        let Some(results) = results else {
            self.finished.uncharge(scratch);
            return Ok(None);
        };
        let width = shown.len() + results.len();
        let bytes = size_of::<Vec<Cell>>() + width * size_of::<Cell>();
        self.finished.charge(bytes)?;
        let mut row = Vec::new();
        self.finished.reserve(
            row.try_reserve_exact(width),
            bytes,
            width * size_of::<Cell>(),
        )?;
        for v in shown.iter().chain(&results) {
            row.push(Cell::of_value(ctx.ids, ctx.keys, v)?);
        }
        self.finished.uncharge(scratch);
        Ok(Some(row))
    }

    fn emit_one(&mut self, out: &mut Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        if let Some(row) = self.finish_one(ctx)? {
            self.finished.uncharge(row_bytes(&row));
            out.push(&mut row.into_iter().map(Some).collect::<Vec<_>>());
        }
        Ok(())
    }

    fn next_ordered(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        if self.ended {
            return Ok(None);
        }
        loop {
            let mut out = Batch::new(self.groups.len() + self.aggs.len());
            if let Some(batch) = self.child.next_batch(ctx)? {
                debug_assert!(batch.len <= BATCH_ROWS);
                self.counters.rows_in += batch.len as u64;
                for i in 0..batch.len {
                    let key = self.key(&batch, i);
                    if !self.table.is_empty() && !self.table.contains_key(&key) {
                        let previous = self.rows.last().and_then(|g| g.cells.first());
                        let next = self
                            .groups
                            .first()
                            .and_then(|s| batch.cols.get(*s))
                            .and_then(|c| c.get(i));
                        let (Some(previous), Some(next)) = (previous, next) else {
                            return Err(err_coded!(
                                ErrorCode::Int032,
                                "ordered aggregate without its group key"
                            ));
                        };
                        if !value(next, ctx)?.cmp(&value(previous, ctx)?).is_gt() {
                            return Err(err_coded!(
                                ErrorCode::Int032,
                                "ordered aggregate input is not increasing"
                            ));
                        }
                        self.emit_one(&mut out, ctx)?;
                    }
                    self.step(key, &batch, i, ctx)?;
                }
                // Every child caps batches at BATCH_ROWS. At most one group
                // completes per input row, so this needs no retained lookahead.
                ctx.ids.end_pull();
            } else {
                self.ended = true;
                self.emit_one(&mut out, ctx)?;
            }
            if out.len > 0 {
                self.counters.out(&out);
                return Ok(Some(out));
            }
            if self.ended {
                return Ok(None);
            }
        }
    }

    /// The rows of every group, in the order of their values, finished.
    fn finish(&mut self, ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Cell>>> {
        if self.table.is_empty() && self.zero_row {
            // `:with` cells, which `:find` does not return, then the zeros.
            let mut row = vec![Cell::Val(Scalar::Null); self.groups.len()];
            row.extend(vec![Cell::Val(Scalar::Int(0)); self.aggs.len()]);
            return Ok(vec![row]);
        }
        // Sorted and finished as the executor's groups are. The sorted map
        // holds each group's values twice and its accumulators, with their
        // `sum-distinct` members, until every row is built, so it is charged
        // as `finished` too.
        self.table = HashMap::new();
        let mut groups = Groups::new();
        let mut sorted = 0;
        for group in std::mem::take(&mut self.rows) {
            let values = group
                .cells
                .iter()
                .map(|c| value(c, ctx))
                .collect::<Result<Vec<Value>>>()?;
            let members: usize = group.accs.iter().map(Acc::member_bytes).sum();
            let n = 2 * (size_of::<Vec<Value>>() + values.iter().map(value_bytes).sum::<usize>())
                + size_of::<Vec<Agg>>()
                + group.accs.len() * size_of::<Agg>()
                + members
                + HASH_ENTRY_OVERHEAD;
            // The members move from `held` to `finished`, so they are not
            // counted twice.
            self.held.uncharge(members);
            self.finished.charge(n)?;
            sorted += n;
            let accs = group.accs.into_iter().map(Acc::into_agg).collect();
            groups.insert(values.clone(), (values, accs));
        }
        self.held.release();
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
        self.finished.uncharge(sorted);
        rows.reverse();
        Ok(rows)
    }
}

impl Operator for Aggregate {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        if self.ordered {
            return self.next_ordered(ctx);
        }
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
        self.ended = false;
        self.table.clear();
        self.rows.clear();
        self.out = None;
        self.held.release();
        self.finished.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.ended = true;
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
