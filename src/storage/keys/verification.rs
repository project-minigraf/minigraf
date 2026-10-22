//! Kani proof harnesses for the key encodings (#627).
//!
//! Each harness proves its property for every input of the given types. Only
//! the encoders are covered: CBMC does not finish on the `Reader` decoders
//! (their error paths build an `anyhow::Error`) or on variable-length strings,
//! so the fuzz and property tests cover those.

use super::*;

fn enc_int(v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    put_int(&mut out, v);
    out
}

fn enc_uint(v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    put_uint(&mut out, v);
    out
}

#[kani::proof]
#[kani::unwind(10)]
fn put_int_preserves_order() {
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    assert_eq!(a.cmp(&b), enc_int(a).cmp(&enc_int(b)));
}

#[kani::proof]
#[kani::unwind(10)]
fn put_uint_preserves_order() {
    let a: u64 = kani::any();
    let b: u64 = kani::any();
    assert_eq!(a.cmp(&b), enc_uint(a).cmp(&enc_uint(b)));
}

/// Non-NaN floats keep IEEE total order (`-0.0` before `0.0`).
#[kani::proof]
fn float_bits_preserves_order() {
    let a: f64 = kani::any();
    let b: f64 = kani::any();
    kani::assume(!a.is_nan() && !b.is_nan());
    assert_eq!(a.total_cmp(&b), float_bits(a).cmp(&float_bits(b)));
}

/// NaN is canonical and sorts after every other float.
#[kani::proof]
fn float_bits_nan_sorts_last() {
    let a: f64 = kani::any();
    let n: f64 = kani::any();
    kani::assume(n.is_nan());
    assert_eq!(float_bits(n), float_bits(f64::NAN));
    if !a.is_nan() {
        assert!(float_bits(a) < float_bits(n));
    }
}

#[kani::proof]
fn float_bits_round_trips() {
    let a: f64 = kani::any();
    kani::assume(!a.is_nan());
    assert_eq!(float_from_bits(float_bits(a)).to_bits(), a.to_bits());
}

/// Newer transactions sort first.
#[kani::proof]
#[kani::unwind(10)]
fn put_tx_desc_reverses_order() {
    let a: u64 = kani::any();
    let b: u64 = kani::any();
    let mut ea = Vec::new();
    let mut eb = Vec::new();
    put_tx_desc(&mut ea, a);
    put_tx_desc(&mut eb, b);
    assert_eq!(b.cmp(&a), ea.cmp(&eb));
}

/// Includes `VALID_TIME_FOREVER` (`i64::MAX`), encoded as the single byte 0xFF.
#[kani::proof]
#[kani::unwind(10)]
fn put_valid_to_preserves_order() {
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    let mut ea = Vec::new();
    let mut eb = Vec::new();
    put_valid_to(&mut ea, a);
    put_valid_to(&mut eb, b);
    assert_eq!(a.cmp(&b), ea.cmp(&eb));
}
