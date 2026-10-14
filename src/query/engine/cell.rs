//! Cells: the values operators exchange (#473 spec §5.2).

use super::ids::IdSpace;
use crate::graph::types::Value;
use crate::storage::KeySource;
use crate::storage::keys::{KeyValue, ValueRef};
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

    /// The cell of a value, with ids from `ids`.
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
            Value::String(s) => Cell::Val(Scalar::Str(Arc::from(s.as_str()))),
            Value::Keyword(k) => Cell::Iid(ids.iid(keys, k)?),
            Value::Ref(u) => Cell::Eid(ids.eid(keys, u)?),
        })
    }
}
