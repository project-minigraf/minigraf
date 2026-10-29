//! Kani proofs for key encoders (#627) and pure scalar decoders (#693).
//!
//! Round trips cover all integer, transaction, valid-to and float inputs plus
//! each scalar value tag. Malformed-input proofs cover slices up to 9 bytes
//! (10 with a value tag), including failure consumption. `Reader` error mapping,
//! strings, sets, packed pages and WAL remain under differential, property and
//! fuzz tests. A set-encoding harness once used 100 GB without finishing.

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

#[kani::proof]
#[kani::unwind(10)]
fn decode_uint_round_trips_with_suffix() {
    let value: u64 = kani::any();
    let mut bytes = enc_uint(value);
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok((decoded, consumed)) = decode_uint(&bytes) else {
        panic!("encoded unsigned integer must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_int_round_trips_with_suffix() {
    let value: i64 = kani::any();
    let mut bytes = enc_int(value);
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok((decoded, consumed)) = decode_int(&bytes) else {
        panic!("encoded signed integer must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_tx_desc_round_trips_with_suffix() {
    let value: u64 = kani::any();
    let mut bytes = Vec::new();
    put_tx_desc(&mut bytes, value);
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok((decoded, consumed)) = decode_tx_desc(&bytes) else {
        panic!("encoded transaction must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_valid_to_round_trips_with_suffix() {
    let value: i64 = kani::any();
    let mut bytes = Vec::new();
    put_valid_to(&mut bytes, value);
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok((decoded, consumed)) = decode_valid_to(&bytes) else {
        panic!("encoded valid-to must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
fn decode_float_round_trips_with_suffix() {
    let value: f64 = kani::any();
    let mut bytes = float_bits(value).to_be_bytes().to_vec();
    bytes.push(kani::any());
    let Ok((decoded, consumed)) = decode_float(&bytes) else {
        panic!("encoded float must decode");
    };
    let expected_bits = if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    };
    assert_eq!(decoded.to_bits(), expected_bits);
    assert_eq!(consumed, 8);
}

#[kani::proof]
#[kani::unwind(11)]
fn decode_uint_rejects_malformed_without_panicking() {
    let bytes: [u8; 9] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 9);
    let len = usize::from(len);
    match decode_uint(&bytes[..len]) {
        Ok((_, consumed)) => {
            let n = bytes.first().copied().unwrap_or(0).wrapping_sub(INT_ZERO);
            assert_eq!(consumed, 1 + usize::from(n));
            assert!(consumed <= len);
        }
        Err(failure) => {
            assert!(failure.consumed <= len);
            match failure.kind {
                DecodeErrorKind::EndsEarly => {
                    assert_eq!(failure.consumed, usize::from(len != 0));
                }
                DecodeErrorKind::BadUnsignedType(t) => {
                    assert_eq!(failure.consumed, 1);
                    assert_eq!(t, bytes[0]);
                }
                _ => panic!("unexpected unsigned error kind"),
            }
        }
    }
}

#[kani::proof]
#[kani::unwind(11)]
fn decode_int_rejects_malformed_without_panicking() {
    let bytes: [u8; 9] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 9);
    let len = usize::from(len);
    match decode_int(&bytes[..len]) {
        Ok((_, consumed)) => {
            let n = bytes.first().copied().unwrap_or(0).abs_diff(INT_ZERO);
            assert_eq!(consumed, 1 + usize::from(n));
            assert!(consumed <= len);
        }
        Err(failure) => {
            assert!(failure.consumed <= len);
            match failure.kind {
                DecodeErrorKind::EndsEarly => {
                    assert_eq!(failure.consumed, usize::from(len != 0));
                }
                DecodeErrorKind::BadIntegerType(t) => {
                    assert_eq!(failure.consumed, 1);
                    assert_eq!(t, bytes[0]);
                }
                DecodeErrorKind::IntegerOutOfRange => {
                    let n = bytes.first().copied().unwrap_or(0).abs_diff(INT_ZERO);
                    assert_eq!(failure.consumed, 1 + usize::from(n));
                }
                _ => panic!("unexpected integer error kind"),
            }
        }
    }
}

#[kani::proof]
#[kani::unwind(11)]
fn decode_tx_desc_rejects_malformed_without_panicking() {
    let bytes: [u8; 9] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 9);
    let len = usize::from(len);
    match decode_tx_desc(&bytes[..len]) {
        Ok((_, consumed)) => {
            let n = (!bytes.first().copied().unwrap_or(0)).wrapping_sub(INT_ZERO);
            assert_eq!(consumed, 1 + usize::from(n));
            assert!(consumed <= len);
        }
        Err(failure) => {
            assert_eq!(failure.consumed, 0);
            match failure.kind {
                DecodeErrorKind::EndsEarly => {}
                DecodeErrorKind::BadTxType(t) => assert_eq!(t, !bytes[0]),
                _ => panic!("unexpected transaction error kind"),
            }
        }
    }
}

#[kani::proof]
#[kani::unwind(11)]
fn decode_valid_to_rejects_malformed_without_panicking() {
    let bytes: [u8; 9] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 9);
    let len = usize::from(len);
    match decode_valid_to(&bytes[..len]) {
        Ok((_, consumed)) => {
            let t = bytes.first().copied().unwrap_or(0);
            let expected = if t == FOREVER {
                1
            } else {
                1 + usize::from(t.abs_diff(INT_ZERO))
            };
            assert_eq!(consumed, expected);
            assert!(consumed <= len);
        }
        Err(failure) => {
            assert!(failure.consumed <= len);
            match failure.kind {
                DecodeErrorKind::EndsEarly => {
                    assert_eq!(failure.consumed, usize::from(len != 0));
                }
                DecodeErrorKind::BadIntegerType(t) => {
                    assert_eq!(failure.consumed, 1);
                    assert_eq!(t, bytes[0]);
                }
                DecodeErrorKind::IntegerOutOfRange => {
                    let n = bytes.first().copied().unwrap_or(0).abs_diff(INT_ZERO);
                    assert_eq!(failure.consumed, 1 + usize::from(n));
                }
                _ => panic!("unexpected valid-to error kind"),
            }
        }
    }
}

#[kani::proof]
fn decode_float_rejects_short_payload_without_panicking() {
    let bytes: [u8; 9] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 9);
    let len = usize::from(len);
    match decode_float(&bytes[..len]) {
        Ok((_, consumed)) => {
            assert!(len >= 8);
            assert_eq!(consumed, 8);
        }
        Err(failure) => {
            assert!(len < 8);
            assert!(matches!(failure.kind, DecodeErrorKind::EndsEarly));
            assert_eq!(failure.consumed, 0);
        }
    }
}

#[kani::proof]
fn decode_null_value_round_trips() {
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Null);
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Null, consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded null must decode");
    };
    assert_eq!(consumed, len);
}

#[kani::proof]
fn decode_boolean_value_round_trips() {
    let value: bool = kani::any();
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Bool(value));
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Bool(decoded), consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded Boolean must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_integer_value_round_trips() {
    let value: i64 = kani::any();
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Int(value));
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Int(decoded), consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded integer must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
fn decode_float_value_round_trips() {
    let value: f64 = kani::any();
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Float(value));
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Float(decoded), consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded float must decode");
    };
    let expected_bits = if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    };
    assert_eq!(decoded.to_bits(), expected_bits);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_keyword_value_round_trips() {
    let value: u32 = kani::any();
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Keyword(value));
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Keyword(decoded), consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded keyword must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(10)]
fn decode_ref_value_round_trips() {
    let value: u64 = kani::any();
    let mut bytes = Vec::new();
    put_value(&mut bytes, &KeyValue::Ref(value));
    let len = bytes.len();
    bytes.push(kani::any());
    let Ok(Some((KeyValue::Ref(decoded), consumed))) = decode_scalar_value(&bytes) else {
        panic!("encoded ref must decode");
    };
    assert_eq!(decoded, value);
    assert_eq!(consumed, len);
}

#[kani::proof]
#[kani::unwind(11)]
fn decode_scalar_value_rejects_malformed_without_panicking() {
    let bytes: [u8; 10] = kani::any();
    let len: u8 = kani::any();
    kani::assume(len <= 10);
    let len = usize::from(len);
    match decode_scalar_value(&bytes[..len]) {
        Ok(Some((_, consumed))) => {
            let tag = bytes.first().copied().unwrap_or(0);
            let type_byte = bytes.get(1).copied().unwrap_or(0);
            let expected = match tag {
                TAG_NULL => 1,
                TAG_BOOL => 2,
                TAG_FLOAT => 9,
                TAG_INT => 2 + usize::from(type_byte.abs_diff(INT_ZERO)),
                TAG_KEYWORD | TAG_REF => 2 + usize::from(type_byte.wrapping_sub(INT_ZERO)),
                _ => panic!("unexpected scalar tag"),
            };
            assert_eq!(consumed, expected);
            assert!(consumed <= len);
        }
        Ok(None) => {
            assert!(len > 0);
            assert!(bytes[0] == TAG_STRING || bytes[0] == TAG_SET);
        }
        Err(failure) => {
            assert!(failure.consumed <= len);
            match failure.kind {
                DecodeErrorKind::EndsEarly => {
                    let expected = if len == 0 {
                        0
                    } else if matches!(bytes[0], TAG_INT | TAG_KEYWORD | TAG_REF) && len >= 2 {
                        2
                    } else {
                        1
                    };
                    assert_eq!(failure.consumed, expected);
                }
                DecodeErrorKind::BadValueTag(t) => {
                    assert_eq!(failure.consumed, 1);
                    assert_eq!(t, bytes[0]);
                }
                DecodeErrorKind::BadUnsignedType(t) => {
                    assert!(matches!(bytes[0], TAG_KEYWORD | TAG_REF));
                    assert_eq!(failure.consumed, 2);
                    assert_eq!(t, bytes.get(1).copied().unwrap_or(0));
                }
                DecodeErrorKind::BadIntegerType(t) => {
                    assert_eq!(bytes[0], TAG_INT);
                    assert_eq!(failure.consumed, 2);
                    assert_eq!(t, bytes.get(1).copied().unwrap_or(0));
                }
                DecodeErrorKind::BadTxType(_) => panic!("transaction error in value"),
                DecodeErrorKind::IntegerOutOfRange => {
                    assert_eq!(bytes[0], TAG_INT);
                    let t = bytes.get(1).copied().unwrap_or(0);
                    assert_eq!(failure.consumed, 2 + usize::from(t.abs_diff(INT_ZERO)));
                }
                DecodeErrorKind::IidOutOfRange => {
                    assert_eq!(bytes[0], TAG_KEYWORD);
                    let t = bytes.get(1).copied().unwrap_or(0);
                    assert_eq!(failure.consumed, 2 + usize::from(t.wrapping_sub(INT_ZERO)));
                }
            }
        }
    }
}
