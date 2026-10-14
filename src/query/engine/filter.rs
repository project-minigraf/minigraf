//! `Filter` and `Bind` (#473 spec §6.1), and the pseudo-attribute patterns
//! on a scanned fact: the clauses after the scan, one batch at a time.
//!
//! An expression sees only the variables it reads, turned into values for
//! the row; a binding expression's result becomes a cell again.

use super::Ctx;
use super::batch::Batch;
use super::cell::{Cell, Scalar};
use super::plan::{Step, Term, Vars};
use super::project::value;
use crate::query::datalog::executor::{eval_expr, is_truthy};
use crate::query::datalog::types::Expr;
use anyhow::Result;
use std::collections::HashMap;

/// Apply `step` to `batch`.
pub(crate) fn apply(step: &Step, mut batch: Batch, ctx: &mut Ctx<'_>) -> Result<Batch> {
    match step {
        Step::Pseudo { col, value } => {
            let meta = batch.cols.get(*col).cloned().unwrap_or_default();
            match value {
                Term::Any => {}
                Term::Bind(slot) => batch.set_col(*slot, meta),
                Term::Const(n) => {
                    let want = n.map(|n| Cell::Val(Scalar::Int(n)));
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
                        out.push(Cell::of_value(&mut ctx.ids, ctx.keys, &v)?);
                        keep.push(true);
                    }
                    None => keep.push(false),
                }
            }
            batch.retain(&keep);
            batch.set_col(*slot, out);
        }
    }
    Ok(batch)
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
