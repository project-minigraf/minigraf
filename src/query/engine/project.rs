//! `Project` (#473 spec §6.1): the `:find` columns of a batch, as values.
//!
//! Only rows that are returned are converted, so a query reads ids until
//! the end (#530).

use super::Ctx;
use super::batch::Batch;
use super::cell::{Cell, Scalar};
use crate::graph::types::Value;
use anyhow::Result;

/// The rows of `batch`, with the cells of the slots `find` as values.
pub(crate) fn project(batch: &Batch, find: &[usize], ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Value>>> {
    let mut rows = Vec::with_capacity(batch.len);
    for i in 0..batch.len {
        let mut row = Vec::with_capacity(find.len());
        for &slot in find {
            let cell = batch.cols.get(slot).and_then(|c| c.get(i));
            row.push(match cell {
                Some(cell) => value(cell, ctx)?,
                None => Value::Null,
            });
        }
        rows.push(row);
    }
    Ok(rows)
}

/// The value of `cell`.
pub(crate) fn value(cell: &Cell, ctx: &mut Ctx<'_>) -> Result<Value> {
    Ok(match cell {
        Cell::Eid(e) => Value::Ref(ctx.ids.uuid(ctx.keys, *e)?),
        Cell::Iid(i) => Value::Keyword(ctx.ids.name(ctx.keys, *i)?.to_string()),
        Cell::Val(Scalar::Null) => Value::Null,
        Cell::Val(Scalar::Bool(b)) => Value::Boolean(*b),
        Cell::Val(Scalar::Int(n)) => Value::Integer(*n),
        Cell::Val(Scalar::Float(f)) => Value::Float(*f),
        Cell::Val(Scalar::Str(s)) => Value::String(s.to_string()),
        Cell::Val(Scalar::Long(vref)) => match ctx.keys {
            Some(k) => Value::String(k.long_value(*vref)?),
            None => {
                return Err(crate::error::err_coded!(
                    crate::error::ErrorCode::Int049,
                    "query engine: long value without a committed reader"
                ));
            }
        },
    })
}
