//! Bind values from JavaScript for `BrowserPreparedQuery` (#181).
//!
//! A binding is a plain value or a typed form `{ type, value }`:
//!
//! | JavaScript | Bind value |
//! |---|---|
//! | `true` / `false` | `Val(Boolean)` |
//! | a `number` that `Number.isInteger` accepts, a `bigint` | `Val(Integer)` |
//! | any other `number` | `Val(Float)` |
//! | a `string` | `Val(String)` |
//! | `null` | `Val(Null)` |
//! | `{ type: 'string' \| 'integer' \| 'float' \| 'boolean' \| 'ref' \| 'keyword' \| 'null', value }` | `Val` of that type |
//! | `{ type: 'entity', value: uuid }` | `Entity` |
//! | `{ type: 'txCount', value }` | `TxCount` (an integer `>= 0`) |
//! | `{ type: 'timestamp', value }` | `Timestamp` (Unix ms) |
//! | `{ type: 'anyValidTime' }` | `AnyValidTime` |
//!
//! Any other JavaScript value is a `TypeError`. A typed form whose value does
//! not fit its type (a bad UUID, a negative tx count, an integer outside
//! `i64`) is `API-017`.

use crate::MinigrafError;
use crate::graph::types::Value;
use crate::query::datalog::prepared::BindValue;
use js_sys::{Array, Number, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The bindings object `bindings` as `(slot name, bind value)` pairs.
/// `undefined` and `null` mean no bindings.
pub(super) fn read_bindings(bindings: &JsValue) -> Result<Vec<(String, BindValue)>, JsValue> {
    if bindings.is_undefined() || bindings.is_null() {
        return Ok(Vec::new());
    }
    let Some(object) = bindings
        .dyn_ref::<Object>()
        .filter(|_| !Array::is_array(bindings))
    else {
        return Err(type_error(
            "bindings must be an object of slot name to bind value",
        ));
    };
    Object::entries(object)
        .iter()
        .map(|entry| {
            let pair: Array = entry.unchecked_into();
            let name = pair.get(0).as_string().unwrap_or_default();
            let value = read_bind_value(&name, &pair.get(1))?;
            Ok((name, value))
        })
        .collect()
}

fn read_bind_value(name: &str, v: &JsValue) -> Result<BindValue, JsValue> {
    if let Some(b) = v.as_bool() {
        return Ok(BindValue::Val(Value::Boolean(b)));
    }
    if v.is_null() {
        return Ok(BindValue::Val(Value::Null));
    }
    if let Some(s) = v.as_string() {
        return Ok(BindValue::Val(Value::String(s)));
    }
    if v.is_bigint() || v.as_f64().is_some() {
        return Ok(BindValue::Val(number(name, v)?));
    }
    if v.is_object()
        && !Array::is_array(v)
        && let Some(kind) = Reflect::get(v, &JsValue::from_str("type"))?.as_string()
        && let Some(bind) = read_typed(name, &kind, v)?
    {
        return Ok(bind);
    }
    Err(type_error(&format!(
        "slot ${name}: a bind value is a boolean, number, bigint, string, null or {{type, value}} object"
    )))
}

/// A typed form, or `None` if `kind` names no type.
fn read_typed(name: &str, kind: &str, v: &JsValue) -> Result<Option<BindValue>, JsValue> {
    let value = Reflect::get(v, &JsValue::from_str("value"))?;
    let what = || format!("slot ${name} of type {kind:?}");
    let string = || value.as_string().ok_or_else(|| mismatch(&what()));
    Ok(Some(match kind {
        "string" => BindValue::Val(Value::String(string()?)),
        "keyword" => BindValue::Val(Value::Keyword(string()?)),
        "boolean" => BindValue::Val(Value::Boolean(
            value.as_bool().ok_or_else(|| mismatch(&what()))?,
        )),
        "integer" => BindValue::Val(Value::Integer(integer(&what(), &value)?)),
        "float" => BindValue::Val(Value::Float(
            value.as_f64().ok_or_else(|| mismatch(&what()))?,
        )),
        "null" => BindValue::Val(Value::Null),
        "ref" => BindValue::Val(Value::Ref(uuid(&what(), &string()?)?)),
        "entity" => BindValue::Entity(uuid(&what(), &string()?)?),
        "txCount" => {
            let n = integer(&what(), &value)?;
            BindValue::TxCount(
                u64::try_from(n).map_err(|_| invalid(&format!("{} {n} is negative", what())))?,
            )
        }
        "timestamp" => BindValue::Timestamp(integer(&what(), &value)?),
        "anyValidTime" => BindValue::AnyValidTime,
        _ => return Ok(None),
    }))
}

/// A plain `number` or `bigint`: an integer when it is whole, else a float.
fn number(name: &str, v: &JsValue) -> Result<Value, JsValue> {
    if v.is_bigint() || Number::is_integer(v) {
        integer(&format!("slot ${name}"), v).map(Value::Integer)
    } else {
        Ok(Value::Float(v.as_f64().unwrap_or(f64::NAN)))
    }
}

/// A `bigint`, or a `number` that is a whole number, as an `i64`.
fn integer(what: &str, v: &JsValue) -> Result<i64, JsValue> {
    if v.is_bigint() {
        return i64::try_from(v.clone())
            .map_err(|_| invalid(&format!("{what}: {} is outside i64", big_text(v))));
    }
    // -2^63 is exact as an f64, and 2^63 is the first whole f64 past i64::MAX.
    const RANGE: std::ops::Range<f64> = (i64::MIN as f64)..-(i64::MIN as f64);
    match v.as_f64() {
        Some(f) if Number::is_integer(v) && RANGE.contains(&f) => {
            // Whole and in range, so the cast is exact.
            #[allow(clippy::cast_possible_truncation)]
            Ok(f as i64)
        }
        Some(f) if Number::is_integer(v) => Err(invalid(&format!("{what}: {f} is outside i64"))),
        _ => Err(mismatch(what)),
    }
}

fn big_text(v: &JsValue) -> String {
    js_sys::BigInt::from(v.clone())
        .to_string(10)
        .map(String::from)
        .unwrap_or_default()
}

fn uuid(what: &str, s: &str) -> Result<uuid::Uuid, JsValue> {
    uuid::Uuid::parse_str(s).map_err(|_| invalid(&format!("{what}: {s:?} is not a UUID")))
}

fn mismatch(what: &str) -> JsValue {
    invalid(&format!("{what}: the value does not match the type"))
}

/// `API-017`, as the native bindings report a bad argument.
fn invalid(msg: &str) -> JsValue {
    JsValue::from_str(&MinigrafError::invalid_argument(msg).to_string())
}

fn type_error(msg: &str) -> JsValue {
    js_sys::TypeError::new(msg).into()
}
