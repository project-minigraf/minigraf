//! `Filter` and `Bind` (#473 spec §6.1), and the pseudo-attribute patterns
//! on a scanned fact: the clauses after the scan, one batch at a time.
//!
//! An expression sees only the variables it reads, turned into values for
//! the row; a binding expression's result becomes a cell again.

use super::batch::Batch;
use super::cell::{Cell, join_eq};
use super::plan::{Entity, Step, Term, Vars};
use super::project::value;
use super::{Counters, Ctx, Operator};
use crate::query::datalog::executor::{eval_expr, is_truthy};
use crate::query::datalog::types::Expr;
use anyhow::Result;
use std::collections::HashMap;

/// One clause after the scan, over the batches of its child.
pub(crate) struct Apply {
    child: Box<dyn Operator>,
    step: Step,
    counters: Counters,
}

impl Apply {
    pub(crate) fn new(child: Box<dyn Operator>, step: Step) -> Self {
        let op = match step {
            Step::Pseudo { .. } => "Pseudo",
            Step::Filter(..) => "Filter",
            Step::Bind(..) => "Bind",
            Step::Join(_) => "Join",
        };
        Apply {
            child,
            step,
            counters: Counters::new(op),
        }
    }
}

impl Operator for Apply {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        while let Some(batch) = self.child.next_batch(ctx)? {
            self.counters.rows_in += batch.len as u64;
            let batch = apply(&self.step, batch, ctx)?;
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn close(&mut self) {
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.child.counters(out);
    }
}

/// Apply `step` to `batch`.
fn apply(step: &Step, mut batch: Batch, ctx: &mut Ctx<'_>) -> Result<Batch> {
    match step {
        Step::Pseudo {
            cols,
            entity,
            value,
        } => {
            let meta = pick(cols, entity.as_ref(), &batch, ctx)?;
            match value {
                Term::Any => {}
                Term::Bind(slot) => batch.set_col(*slot, meta),
                Term::Const(n) => {
                    let want = n.clone().map(Cell::Val);
                    let keep: Vec<bool> = meta.iter().map(|c| want.as_ref() == Some(c)).collect();
                    batch.retain(&keep);
                }
                Term::Eq(slot) => {
                    let other = batch.cols.get(*slot);
                    let keep: Vec<bool> = meta
                        .iter()
                        .enumerate()
                        .map(|(i, c)| other.and_then(|o| o.get(i)) == Some(c))
                        .collect();
                    batch.retain(&keep);
                }
            }
        }
        Step::Filter(expr, vars) => {
            let mut keep = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                keep.push(eval(expr, vars, &batch, i, ctx)?.is_some_and(|v| is_truthy(&v)));
            }
            batch.retain(&keep);
        }
        Step::Bind(expr, vars, slot) => {
            let mut keep = Vec::with_capacity(batch.len);
            let mut out = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                match eval(expr, vars, &batch, i, ctx)? {
                    Some(v) => {
                        out.push(Cell::of_value(ctx.ids, ctx.keys, &v)?);
                        keep.push(true);
                    }
                    None => keep.push(false),
                }
            }
            batch.retain(&keep);
            batch.set_col(*slot, out);
        }
        // Joins are operators of their own (`join::Join`).
        Step::Join(_) => {}
    }
    Ok(batch)
}

/// The metadata a pseudo-attribute pattern reads in each row of `batch`:
/// from the first of `cols` with no entity, or whose entity is join-equal to
/// `entity` (decision 4 of the #545 plan).
fn pick(
    cols: &[(Option<Entity>, usize)],
    entity: Option<&Entity>,
    batch: &Batch,
    ctx: &mut Ctx<'_>,
) -> Result<Vec<Cell>> {
    if let [(None, col)] = cols {
        return Ok(batch.cols.get(*col).cloned().unwrap_or_default());
    }
    let mut out = Vec::with_capacity(batch.len);
    for i in 0..batch.len {
        let want = match entity {
            Some(e) => entity_cell(e, batch, i, ctx)?,
            None => None,
        };
        for (e, col) in cols {
            let matched = match (e, &want) {
                (None, _) => true,
                (Some(e), Some(w)) => match entity_cell(e, batch, i, ctx)? {
                    Some(c) => join_eq(&c, w, true, ctx)?,
                    None => false,
                },
                (Some(_), None) => false,
            };
            if matched {
                if let Some(c) = batch.cols.get(*col).and_then(|c| c.get(i)) {
                    out.push(c.clone());
                }
                break;
            }
        }
    }
    Ok(out)
}

/// The cell of `entity` in row `i`.
fn entity_cell(
    entity: &Entity,
    batch: &Batch,
    i: usize,
    ctx: &mut Ctx<'_>,
) -> Result<Option<Cell>> {
    Ok(match entity {
        Entity::Slot(s) => batch.cols.get(*s).and_then(|c| c.get(i)).cloned(),
        Entity::Const(u) => Some(Cell::Eid(ctx.ids.eid(ctx.keys, u)?)),
    })
}

/// `expr` over row `i`, or `None` where the executor's evaluation fails.
fn eval(
    expr: &Expr,
    vars: &Vars,
    batch: &Batch,
    i: usize,
    ctx: &mut Ctx<'_>,
) -> Result<Option<crate::graph::types::Value>> {
    let mut binding = HashMap::with_capacity(vars.len());
    for (name, slot) in vars {
        if let Some(cell) = batch.cols.get(*slot).and_then(|c| c.get(i)) {
            binding.insert(name.clone(), value(cell, ctx)?);
        }
    }
    Ok(eval_expr(expr, &binding, Some(ctx.functions)).ok())
}
