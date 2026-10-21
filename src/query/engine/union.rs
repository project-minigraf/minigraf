//! `Union` (#546 spec §5.1): `or` and `or-join`. Each outer row, once per row
//! of each branch, extended by the branch's cells of the new variables; a
//! [`Distinct`](super::distinct::Distinct) follows. With no new variable (an
//! `or-join`, or an `or` whose branches bind nothing new) those rows would
//! all equal the outer row, so it is returned once, when some branch has a
//! row.
//!
//! A branch is planned as a `not` body is (decision 4 of the #546 plan): a
//! correlated branch runs once per outer row from a
//! [`Seed`](super::seed::Seed) of the cells it imports; a hashed one runs
//! once, into a table of its rows' new cells by their keys (their cells of
//! the imported variables, as [`JoinKey`]s).

use super::batch::Batch;
use super::cell::{Cell, JoinKey};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, growth, keys_bytes, row_bytes};
use super::plan::{Body as Spec, Branch, Key};
use super::{BATCH_ROWS, Counters, Ctx, Operator, build};
use anyhow::Result;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) struct Union {
    outer: Box<dyn Operator>,
    arms: Vec<Arm>,
    /// Number of new slots.
    width: usize,
    /// The outer batch being extended.
    at: Option<At>,
    /// The hashed branches' tables (#549 spec §3.2), counted as long as
    /// they are kept: until the operator drops.
    tables: Held,
    /// `At.rows`, released as they are returned.
    buffered: Held,
    /// `rows_in` counts the outer rows received.
    counters: Counters,
}

/// Where `Union` is in an outer batch.
struct At {
    batch: Batch,
    /// The next row whose branch rows are not computed.
    next: usize,
    /// The row `rows` extend, and the next of them to return.
    row: usize,
    rows: Vec<Vec<Cell>>,
    done: usize,
}

struct Arm {
    body: Body,
    /// The body slots of the new cells.
    new: Vec<usize>,
}

enum Body {
    Correlated {
        root: Box<dyn Operator>,
        imports: Vec<usize>,
        seed: Vec<Cell>,
    },
    Hashed {
        root: Box<dyn Operator>,
        keys: Vec<Key>,
        /// Built on the first pull, then kept: the branch does not depend on
        /// the outer row, so a rewind keeps it too.
        table: Option<HashMap<Vec<JoinKey>, Vec<Vec<Cell>>>>,
    },
}

impl Union {
    pub(crate) fn new(outer: Box<dyn Operator>, branches: Vec<Branch>, meter: &Arc<Meter>) -> Self {
        let width = branches.first().map_or(0, |b| b.new.len());
        let arms = branches
            .into_iter()
            .map(|Branch { body, new }| Arm {
                body: match body {
                    Spec::Correlated { plan, imports } => Body::Correlated {
                        root: build(plan.source, plan.steps, meter),
                        imports,
                        seed: Vec::new(),
                    },
                    Spec::Hashed { plan, keys } => Body::Hashed {
                        root: build(plan.source, plan.steps, meter),
                        keys,
                        table: None,
                    },
                },
                new,
            })
            .collect();
        Union {
            outer,
            arms,
            width,
            at: None,
            tables: Held::new(meter, "Union"),
            buffered: Held::new(meter, "Union"),
            counters: Counters::new("Union"),
        }
    }
}

/// The cell of row `i` in `slot`.
fn cell(batch: &Batch, slot: usize, i: usize) -> Option<&Cell> {
    batch.cols.get(slot).and_then(|c| c.get(i))
}

impl Arm {
    /// Append the new cells of the branch's rows for row `i` of `batch` to
    /// `out`, counted on `buffered`; with `one`, at most one row. A hashed
    /// branch's table is counted on `tables`.
    #[allow(clippy::too_many_arguments)]
    fn rows(
        &mut self,
        batch: &Batch,
        i: usize,
        one: bool,
        out: &mut Vec<Vec<Cell>>,
        tables: &mut Held,
        buffered: &mut Held,
        ctx: &mut Ctx<'_>,
    ) -> Result<()> {
        let new = &self.new;
        match &mut self.body {
            Body::Correlated {
                root,
                imports,
                seed,
            } => {
                seed.clear();
                for &slot in imports.iter() {
                    // Every imported slot is filled.
                    if let Some(c) = cell(batch, slot, i) {
                        seed.push(c.clone());
                    }
                }
                root.rewind(seed);
                'pull: while let Some(b) = root.next_batch(ctx)? {
                    for j in 0..b.len {
                        buffer(
                            out,
                            new.iter()
                                .filter_map(|&s| cell(&b, s, j).cloned())
                                .collect(),
                            buffered,
                        )?;
                        if one {
                            break 'pull;
                        }
                    }
                }
                root.close();
            }
            Body::Hashed { root, keys, table } => {
                if table.is_none() {
                    *table = Some(collect(root.as_mut(), keys, new, one, tables, ctx)?);
                }
                let Some(table) = table else {
                    return Ok(());
                };
                let mut key = Vec::with_capacity(keys.len());
                for k in keys.iter() {
                    let Some(c) = cell(batch, k.outer, i) else {
                        return Ok(());
                    };
                    // A NaN equals nothing a branch row holds.
                    let Some(j) = JoinKey::of(c, k.normalise, ctx)? else {
                        return Ok(());
                    };
                    key.push(j);
                }
                if let Some(rows) = table.get(&key) {
                    let rows = match rows.first() {
                        Some(first) if one => std::slice::from_ref(first),
                        _ => rows.as_slice(),
                    };
                    for r in rows {
                        buffer(out, r.clone(), buffered)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn root(&self) -> &dyn Operator {
        match &self.body {
            Body::Correlated { root, .. } | Body::Hashed { root, .. } => root.as_ref(),
        }
    }

    fn root_mut(&mut self) -> &mut dyn Operator {
        match &mut self.body {
            Body::Correlated { root, .. } | Body::Hashed { root, .. } => root.as_mut(),
        }
    }
}

/// Append `row` to `out`, counted on `held`.
fn buffer(out: &mut Vec<Vec<Cell>>, row: Vec<Cell>, held: &mut Held) -> Result<()> {
    let n = row_bytes(&row);
    held.charge(n)?;
    let refused = growth(out.capacity(), size_of::<Vec<Cell>>());
    held.reserve(out.try_reserve(1), n, refused)?;
    out.push(row);
    Ok(())
}

/// Every row of a hashed branch: its new cells by the keys of `keys`' inner
/// slots, counted on `held`. With `one`, one row per key.
fn collect(
    root: &mut dyn Operator,
    keys: &[Key],
    new: &[usize],
    one: bool,
    held: &mut Held,
    ctx: &mut Ctx<'_>,
) -> Result<HashMap<Vec<JoinKey>, Vec<Vec<Cell>>>> {
    let mut table: HashMap<Vec<JoinKey>, Vec<Vec<Cell>>> = HashMap::new();
    while let Some(b) = root.next_batch(ctx)? {
        'row: for i in 0..b.len {
            let mut key = Vec::with_capacity(keys.len());
            for k in keys {
                let Some(c) = cell(&b, k.inner, i) else {
                    continue 'row;
                };
                // A row with a NaN key equals no outer row.
                let Some(j) = JoinKey::of(c, k.normalise, ctx)? else {
                    continue 'row;
                };
                key.push(j);
            }
            let found = table.contains_key(&key);
            if one && found {
                continue;
            }
            let row: Vec<Cell> = new
                .iter()
                .filter_map(|&s| cell(&b, s, i).cloned())
                .collect();
            // A new key's entry: the key and its rows' header; then the row.
            let entry = keys_bytes(&key) + size_of::<Vec<Vec<Cell>>>() + HASH_ENTRY_OVERHEAD;
            let n = row_bytes(&row) + if found { 0 } else { entry };
            held.charge(n)?;
            if !found {
                let refused = growth(table.capacity(), entry);
                held.reserve(table.try_reserve(1), n, refused)?;
            }
            let rows = table.entry(key).or_default();
            let refused = growth(rows.capacity(), size_of::<Vec<Cell>>());
            held.reserve(rows.try_reserve(1), n, refused)?;
            rows.push(row);
        }
    }
    root.close();
    Ok(table)
}

impl Operator for Union {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        let one = self.width == 0;
        let mut out: Option<Batch> = None;
        loop {
            if self.at.is_none() {
                let Some(batch) = self.outer.next_batch(ctx)? else {
                    break;
                };
                self.counters.rows_in += batch.len as u64;
                self.at = Some(At {
                    batch,
                    next: 0,
                    row: 0,
                    rows: Vec::new(),
                    done: 0,
                });
            }
            let Some(at) = self.at.as_mut() else {
                break;
            };
            let out = out.get_or_insert_with(|| Batch::new(at.batch.cols.len() + self.width));
            while at.done < at.rows.len() && out.len < BATCH_ROWS {
                if let Some(new) = at.rows.get(at.done) {
                    self.buffered.uncharge(row_bytes(new));
                }
                let mut row: Vec<Option<Cell>> = at
                    .batch
                    .cols
                    .iter()
                    .map(|c| c.get(at.row).cloned())
                    .collect();
                if let Some(new) = at.rows.get(at.done) {
                    row.extend(new.iter().cloned().map(Some));
                }
                out.push(&mut row);
                at.done += 1;
            }
            if out.len >= BATCH_ROWS {
                break;
            }
            if at.next >= at.batch.len {
                self.at = None;
                continue;
            }
            at.row = at.next;
            at.next += 1;
            at.rows.clear();
            at.done = 0;
            self.buffered.release();
            for arm in &mut self.arms {
                arm.rows(
                    &at.batch,
                    at.row,
                    one,
                    &mut at.rows,
                    &mut self.tables,
                    &mut self.buffered,
                    ctx,
                )?;
                if one && !at.rows.is_empty() {
                    break;
                }
            }
        }
        match out {
            Some(b) if b.len > 0 => {
                self.counters.out(&b);
                Ok(Some(b))
            }
            _ => Ok(None),
        }
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.at = None;
        self.buffered.release();
        self.outer.rewind(seed);
    }

    fn close(&mut self) {
        self.at = None;
        self.buffered.release();
        self.outer.close();
        for arm in &mut self.arms {
            arm.root_mut().close();
        }
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: (self.tables.bytes() + self.buffered.bytes()) as u64,
            ..self.counters
        });
        for arm in &self.arms {
            arm.root().counters(out);
        }
        self.outer.counters(out);
    }
}
