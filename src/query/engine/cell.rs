//! Cells: the values operators exchange (#473 spec §5.2).

use super::Ctx;
use super::ids::IdSpace;
use crate::graph::types::Value;
use crate::storage::KeySource;
use crate::storage::keys::{KeyValue, SHORT_STRING_MAX, ValueRef};
use anyhow::Result;
use std::sync::Arc;

/// One value in a row. Entities and keywords are ids in the query's
/// [`IdSpace`](super::ids::IdSpace); everything else is a [`Scalar`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cell {
    /// An entity: a DICT entity id, or a provisional id.
    Eid(u64),
    /// An ident used as a value (a keyword, or an attribute): a DICT ident
    /// id, or a provisional id.
    Iid(u64),
    Val(Scalar),
}

/// A value that is neither an entity nor a keyword.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Scalar {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A committed string of up to 64 bytes, or a pending string.
    Str(Arc<str>),
    /// A committed string over 64 bytes, by where it is stored.
    Long(ValueRef),
}

impl Cell {
    /// The cell of a committed key value.
    pub(crate) fn of_key(v: &KeyValue) -> Cell {
        match v {
            KeyValue::Null => Cell::Val(Scalar::Null),
            KeyValue::Bool(b) => Cell::Val(Scalar::Bool(*b)),
            KeyValue::Int(n) => Cell::Val(Scalar::Int(*n)),
            KeyValue::Float(f) => Cell::Val(Scalar::Float(*f)),
            KeyValue::Str(s) => Cell::Val(Scalar::Str(Arc::from(s.as_str()))),
            KeyValue::LongStr { vref, .. } => Cell::Val(Scalar::Long(*vref)),
            KeyValue::Keyword(iid) => Cell::Iid(u64::from(*iid)),
            KeyValue::Ref(eid) => Cell::Eid(*eid),
        }
    }

    /// The cell of a value, with ids from `ids`. A string over 64 bytes
    /// that the committed value pages hold is `Long`, as a committed key
    /// holds it, so that equal strings are equal cells (#545).
    pub(crate) fn of_value(
        ids: &mut IdSpace,
        keys: Option<&dyn KeySource>,
        value: &Value,
    ) -> Result<Cell> {
        Ok(match value {
            Value::Null => Cell::Val(Scalar::Null),
            Value::Boolean(b) => Cell::Val(Scalar::Bool(*b)),
            Value::Integer(n) => Cell::Val(Scalar::Int(*n)),
            Value::Float(f) => Cell::Val(Scalar::Float(*f)),
            Value::String(s) if s.len() > SHORT_STRING_MAX => {
                match keys.map(|k| k.long_value_ref(s)).transpose()?.flatten() {
                    Some(vref) => Cell::Val(Scalar::Long(vref)),
                    None => Cell::Val(Scalar::Str(Arc::from(s.as_str()))),
                }
            }
            Value::String(s) => Cell::Val(Scalar::Str(Arc::from(s.as_str()))),
            Value::Keyword(k) => Cell::Iid(ids.iid(keys, k)?),
            Value::Ref(u) => Cell::Eid(ids.eid(keys, u)?),
        })
    }
}

/// A cell as a join compares it (#473 spec §5.4): equal keys are
/// join-equal cells.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum JoinKey {
    Eid(u64),
    Iid(u64),
    Null,
    Bool(bool),
    Int(i64),
    /// A float's bits, `-0.0` as `0.0`; a NaN only in a [`JoinKey::group`]
    /// key, as one NaN.
    Float(u64),
    Str(Arc<str>),
    Long(ValueRef),
}

impl JoinKey {
    /// The key of `cell`, or `None` for a NaN, which equals nothing. With
    /// `normalise`, a keyword is the entity it names, so that it equals a
    /// ref to that entity; without, it equals the same keyword only.
    pub(crate) fn of(cell: &Cell, normalise: bool, ctx: &mut Ctx<'_>) -> Result<Option<JoinKey>> {
        Ok(Some(match cell {
            Cell::Eid(e) => JoinKey::Eid(*e),
            Cell::Iid(i) if normalise => JoinKey::Eid(ctx.ids.keyword_eid(ctx.keys, *i)?),
            Cell::Iid(i) => JoinKey::Iid(*i),
            Cell::Val(Scalar::Null) => JoinKey::Null,
            Cell::Val(Scalar::Bool(b)) => JoinKey::Bool(*b),
            Cell::Val(Scalar::Int(n)) => JoinKey::Int(*n),
            Cell::Val(Scalar::Float(f)) if f.is_nan() => return Ok(None),
            Cell::Val(Scalar::Float(f)) => JoinKey::Float(if *f == 0.0 { 0 } else { f.to_bits() }),
            Cell::Val(Scalar::Str(s)) => JoinKey::Str(s.clone()),
            Cell::Val(Scalar::Long(vref)) => JoinKey::Long(*vref),
        }))
    }
}

impl JoinKey {
    /// The key of `cell` as `Value`'s `Ord` compares values, for groups and
    /// distinct sets (#547 spec §2): as [`JoinKey::of`] without
    /// normalising, except that every NaN is one key.
    pub(crate) fn group(cell: &Cell) -> JoinKey {
        match cell {
            Cell::Eid(e) => JoinKey::Eid(*e),
            Cell::Iid(i) => JoinKey::Iid(*i),
            Cell::Val(Scalar::Null) => JoinKey::Null,
            Cell::Val(Scalar::Bool(b)) => JoinKey::Bool(*b),
            Cell::Val(Scalar::Int(n)) => JoinKey::Int(*n),
            Cell::Val(Scalar::Float(f)) if f.is_nan() => JoinKey::Float(f64::NAN.to_bits()),
            Cell::Val(Scalar::Float(f)) => JoinKey::Float(if *f == 0.0 { 0 } else { f.to_bits() }),
            Cell::Val(Scalar::Str(s)) => JoinKey::Str(s.clone()),
            Cell::Val(Scalar::Long(vref)) => JoinKey::Long(*vref),
        }
    }
}

/// Whether `a` and `b` are join-equal (#473 spec §5.4).
pub(crate) fn join_eq(a: &Cell, b: &Cell, normalise: bool, ctx: &mut Ctx<'_>) -> Result<bool> {
    Ok(
        match (
            JoinKey::of(a, normalise, ctx)?,
            JoinKey::of(b, normalise, ctx)?,
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
    )
}
