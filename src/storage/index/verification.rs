//! Kani proof harnesses for the canonical value encoding (#627).
//!
//! Each harness proves one variant's encoding for every value of that variant:
//! the type byte, and that byte order matches the variant's order. An order
//! that is exact on equality is also injective, and distinct type bytes order
//! and separate the variants. Strings and keywords are left out: CBMC does not
//! finish on variable-length encodings at a useful bound, so the fuzz and
//! property tests cover them.

use super::*;
use uuid::Uuid;

#[kani::proof]
fn encode_value_null_and_boolean() {
    let a: bool = kani::any();
    let b: bool = kani::any();
    let ea = encode_value(&Value::Boolean(a));
    let eb = encode_value(&Value::Boolean(b));
    assert_eq!(ea.first(), Some(&0x01));
    assert_eq!(a.cmp(&b), ea.cmp(&eb));
    assert_eq!(encode_value(&Value::Null), vec![0x00]);
}

#[kani::proof]
fn encode_value_integer_preserves_order() {
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    let ea = encode_value(&Value::Integer(a));
    let eb = encode_value(&Value::Integer(b));
    assert_eq!(ea.first(), Some(&0x02));
    assert_eq!(a.cmp(&b), ea.cmp(&eb));
}

/// Non-NaN floats keep IEEE total order, so `-0.0` sorts before `0.0` and the
/// two encode differently.
#[kani::proof]
fn encode_value_float_preserves_order() {
    let a: f64 = kani::any();
    let b: f64 = kani::any();
    kani::assume(!a.is_nan() && !b.is_nan());
    let ea = encode_value(&Value::Float(a));
    let eb = encode_value(&Value::Float(b));
    assert_eq!(ea.first(), Some(&0x03));
    assert_eq!(a.total_cmp(&b), ea.cmp(&eb));
}

/// Every NaN encodes to the same bytes.
#[kani::proof]
fn encode_value_nan_is_canonical() {
    let a: f64 = kani::any();
    let b: f64 = kani::any();
    kani::assume(a.is_nan() && b.is_nan());
    assert_eq!(
        encode_value(&Value::Float(a)),
        encode_value(&Value::Float(b))
    );
}

#[kani::proof]
fn encode_value_ref_preserves_order() {
    let a = Uuid::from_bytes(kani::any());
    let b = Uuid::from_bytes(kani::any());
    let ea = encode_value(&Value::Ref(a));
    let eb = encode_value(&Value::Ref(b));
    assert_eq!(ea.first(), Some(&0x06));
    assert_eq!(a.cmp(&b), ea.cmp(&eb));
}
