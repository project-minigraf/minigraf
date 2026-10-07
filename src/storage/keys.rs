//! Byte-comparable keys (spec §6).
//!
//! Every component is encoded so that comparing whole keys with memcmp gives
//! the logical order, and every encoding is prefix-free (its own bytes say where
//! it ends). Concatenating components therefore keeps the order of the tuple,
//! and keys are compared without decoding.
//!
//! Index keys (§5.2):
//!
//! | tree | key |
//! |---|---|
//! | EAVT | `e a v tx↓ vf vt op` |
//! | AEVT | `a e v tx↓ vf vt op` |
//! | AVET | `a v e tx↓ vf vt op` |
//! | VAET | `v a e tx↓ vf vt op` (`v` is the target eid; refs only) |
//!
//! DICT keys start with a tag byte (§5.2): 0x01 UUID → eid, 0x02 eid → UUID,
//! 0x03 ident → iid, 0x04 iid → ident, 0x05 tx_count → tx_id, 0x06 long-value
//! hash and ref (dedup).

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::VALID_TIME_FOREVER;
use crate::storage::PAGE_SIZE;
use crate::storage::page::PAGE_HEADER_SIZE;
use anyhow::Result;
use uuid::Uuid;

/// Longest string value, in bytes: one value page's payload (§6.3).
pub const MAX_VALUE_BYTES: usize = PAGE_SIZE - PAGE_HEADER_SIZE - 4;
/// Longest attribute name or keyword value, in bytes. The parser already caps
/// keywords at this length; it bounds DICT entries well inside a node.
pub const MAX_IDENT_BYTES: usize = 1024;
/// Strings up to this many bytes are stored inline in keys; longer ones go to
/// value pages (§6.2).
pub const SHORT_STRING_MAX: usize = 64;
/// Bytes of a long string kept in its key, before the marker and hash.
const LONG_PREFIX: usize = 32;

// ─── Integers (§6.1) ─────────────────────────────────────────────────────────

/// Type byte of zero. Positive values use `INT_ZERO + n` and negative values
/// `INT_ZERO - n`, where `n` is the length of the big-endian magnitude.
const INT_ZERO: u8 = 0x14;
/// `vt == VALID_TIME_FOREVER`: one byte that sorts after every finite time.
const FOREVER: u8 = 0xFF;

/// Minimal big-endian bytes of `m` (empty for 0).
fn magnitude_bytes(m: u64) -> ([u8; 8], usize) {
    let bytes = m.to_be_bytes();
    let skip = usize::try_from(m.leading_zeros() / 8).unwrap_or(8);
    (bytes, skip)
}

/// Append `v`, unsigned: type byte `0x14 + n`, then `n` big-endian bytes.
pub fn put_uint(out: &mut Vec<u8>, v: u64) {
    let (bytes, skip) = magnitude_bytes(v);
    let tail = bytes.get(skip..).unwrap_or(&[]);
    out.push(INT_ZERO + u8::try_from(tail.len()).unwrap_or(8));
    out.extend_from_slice(tail);
}

/// Append `v`, signed. Negative values store the one's complement of their
/// magnitude, so a larger magnitude sorts first.
pub fn put_int(out: &mut Vec<u8>, v: i64) {
    if v >= 0 {
        put_uint(out, v.unsigned_abs());
        return;
    }
    let m = v.unsigned_abs();
    let (_, skip) = magnitude_bytes(m);
    let comp = (!m).to_be_bytes();
    let tail = comp.get(skip..).unwrap_or(&[]);
    out.push(INT_ZERO - u8::try_from(tail.len()).unwrap_or(8));
    out.extend_from_slice(tail);
}

/// A cursor over encoded bytes. Every read checks bounds and fails with
/// INT-049: a malformed key inside a verified page is an invariant violation.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Bytes read so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    fn byte(&mut self) -> Result<u8> {
        let b = self
            .buf
            .get(self.pos)
            .copied()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "key ends early"))?;
        self.pos += 1;
        Ok(b)
    }

    fn peek(&self) -> Result<u8> {
        self.buf
            .get(self.pos)
            .copied()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "key ends early"))
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "key length overflow"))?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "key ends early"))?;
        self.pos = end;
        Ok(s)
    }

    fn be(&mut self, n: usize) -> Result<u64> {
        let mut v = [0u8; 8];
        let src = self.take(n)?;
        let dst = v
            .get_mut(8usize.saturating_sub(n)..)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "integer longer than 8 bytes"))?;
        dst.copy_from_slice(src);
        Ok(u64::from_be_bytes(v))
    }

    pub fn uint(&mut self) -> Result<u64> {
        let t = self.byte()?;
        match t.checked_sub(INT_ZERO) {
            Some(n @ 0..=8) => self.be(usize::from(n)),
            _ => bail_coded!(
                ErrorCode::Int049,
                format!("bad unsigned type byte {t:#04x}")
            ),
        }
    }

    pub fn int(&mut self) -> Result<i64> {
        let t = self.byte()?;
        if t >= INT_ZERO {
            let n = t - INT_ZERO;
            if n > 8 {
                bail_coded!(ErrorCode::Int049, format!("bad integer type byte {t:#04x}"));
            }
            let m = self.be(usize::from(n))?;
            return i64::try_from(m)
                .map_err(|_| err_coded!(ErrorCode::Int049, "integer out of range"));
        }
        let n = INT_ZERO - t;
        if n > 8 {
            bail_coded!(ErrorCode::Int049, format!("bad integer type byte {t:#04x}"));
        }
        let n = usize::from(n);
        let comp = self.be(n)?;
        // The stored bytes are the low `n` bytes of !m; restore the high ones.
        let mask = if n == 8 {
            u64::MAX
        } else {
            (1u64 << (8 * n)) - 1
        };
        let m = !(comp | !mask);
        if m == 0 || m > i64::MIN.unsigned_abs() {
            bail_coded!(ErrorCode::Int049, "integer out of range");
        }
        Ok(0i64.checked_sub_unsigned(m).unwrap_or(i64::MIN))
    }

    /// `tx↓`: see [`put_tx_desc`].
    pub fn tx_desc(&mut self) -> Result<u64> {
        let t = !self.peek()?;
        let n = match t.checked_sub(INT_ZERO) {
            Some(n @ 0..=8) => usize::from(n),
            _ => bail_coded!(ErrorCode::Int049, format!("bad tx type byte {t:#04x}")),
        };
        let raw: Vec<u8> = self.take(n + 1)?.iter().map(|b| !b).collect();
        Reader::new(&raw).uint()
    }

    pub fn valid_to(&mut self) -> Result<i64> {
        if self.peek()? == FOREVER {
            self.pos += 1;
            return Ok(VALID_TIME_FOREVER);
        }
        self.int()
    }
}

/// Append `tx_count` so that newer transactions sort first: the encoding of
/// `tx_count` with every byte complemented. The encoding is prefix-free, so the
/// complement reverses its order exactly.
pub fn put_tx_desc(out: &mut Vec<u8>, tx_count: u64) {
    let start = out.len();
    put_uint(out, tx_count);
    if let Some(s) = out.get_mut(start..) {
        for b in s {
            *b = !*b;
        }
    }
}

/// Append a valid-to time; FOREVER is the single byte 0xFF.
pub fn put_valid_to(out: &mut Vec<u8>, vt: i64) {
    if vt == VALID_TIME_FOREVER {
        out.push(FOREVER);
    } else {
        put_int(out, vt);
    }
}

// ─── Values (§6.2) ───────────────────────────────────────────────────────────

/// Value type tags, in the cross-type order Null < Boolean < Integer < Float <
/// String < Keyword < Ref.
const TAG_NULL: u8 = 0x01;
const TAG_BOOL: u8 = 0x02;
const TAG_INT: u8 = 0x03;
const TAG_FLOAT: u8 = 0x04;
const TAG_STRING: u8 = 0x05;
const TAG_KEYWORD: u8 = 0x06;
const TAG_REF: u8 = 0x07;

/// Where a long string is stored: a record in a value page (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueRef {
    pub page: u64,
    pub slot: u16,
}

/// A value as it appears in a key: keywords and refs are ids, and a long
/// string is its value ref.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A string longer than [`SHORT_STRING_MAX`] bytes.
    LongStr {
        prefix: Vec<u8>,
        hash: u64,
        vref: ValueRef,
    },
    Keyword(u32),
    Ref(u64),
}

/// The order-preserving bit transform for floats, with NaN canonicalised.
fn float_bits(f: f64) -> u64 {
    if f.is_nan() {
        return 0xFFF8_0000_0000_0000;
    }
    let raw = f.to_bits();
    if raw >> 63 == 0 {
        raw ^ 0x8000_0000_0000_0000
    } else {
        !raw
    }
}

fn float_from_bits(bits: u64) -> f64 {
    if bits >> 63 == 1 {
        f64::from_bits(bits ^ 0x8000_0000_0000_0000)
    } else {
        f64::from_bits(!bits)
    }
}

/// Append `bytes` with 0x00 escaped as `00 FF`.
fn put_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for &b in bytes {
        out.push(b);
        if b == 0 {
            out.push(0xFF);
        }
    }
}

/// FNV-1a 64 of a long value: stable across versions and platforms. A
/// collision costs one extra comparison of full values, never a wrong match.
pub fn hash64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Append an encoded value.
pub fn put_value(out: &mut Vec<u8>, v: &KeyValue) {
    match v {
        KeyValue::Null => out.push(TAG_NULL),
        KeyValue::Bool(b) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*b));
        }
        KeyValue::Int(n) => {
            out.push(TAG_INT);
            put_int(out, *n);
        }
        KeyValue::Float(f) => {
            out.push(TAG_FLOAT);
            out.extend_from_slice(&float_bits(*f).to_be_bytes());
        }
        KeyValue::Str(s) => {
            out.push(TAG_STRING);
            put_escaped(out, s.as_bytes());
            out.extend_from_slice(&[0x00, 0x00]);
        }
        KeyValue::LongStr { prefix, hash, vref } => {
            out.push(TAG_STRING);
            put_escaped(out, prefix);
            out.extend_from_slice(&[0x00, 0x01]);
            out.extend_from_slice(&hash.to_be_bytes());
            out.extend_from_slice(&vref.page.to_be_bytes());
            out.extend_from_slice(&vref.slot.to_be_bytes());
        }
        KeyValue::Keyword(iid) => {
            out.push(TAG_KEYWORD);
            put_uint(out, u64::from(*iid));
        }
        KeyValue::Ref(eid) => {
            out.push(TAG_REF);
            put_uint(out, *eid);
        }
    }
}

/// The key form of a long string: its first [`LONG_PREFIX`] bytes and hash.
pub fn long_str(s: &str, vref: ValueRef) -> KeyValue {
    let bytes = s.as_bytes();
    KeyValue::LongStr {
        prefix: bytes.get(..LONG_PREFIX).unwrap_or(bytes).to_vec(),
        hash: hash64(bytes),
        vref,
    }
}

impl Reader<'_> {
    pub fn value(&mut self) -> Result<KeyValue> {
        let tag = self.byte()?;
        Ok(match tag {
            TAG_NULL => KeyValue::Null,
            TAG_BOOL => KeyValue::Bool(self.byte()? != 0),
            TAG_INT => KeyValue::Int(self.int()?),
            TAG_FLOAT => KeyValue::Float(float_from_bits(self.be(8)?)),
            TAG_STRING => self.string()?,
            TAG_KEYWORD => KeyValue::Keyword(
                u32::try_from(self.uint()?)
                    .map_err(|_| err_coded!(ErrorCode::Int049, "iid out of range"))?,
            ),
            TAG_REF => KeyValue::Ref(self.uint()?),
            _ => bail_coded!(ErrorCode::Int049, format!("bad value tag {tag:#04x}")),
        })
    }

    /// Move past one encoded value without decoding it.
    pub fn skip_value(&mut self) -> Result<()> {
        match self.byte()? {
            TAG_NULL => {}
            TAG_BOOL => {
                self.byte()?;
            }
            TAG_INT => {
                self.int()?;
            }
            TAG_FLOAT => {
                self.take(8)?;
            }
            TAG_KEYWORD | TAG_REF => {
                self.uint()?;
            }
            TAG_STRING => loop {
                if self.byte()? != 0 {
                    continue;
                }
                match self.byte()? {
                    0xFF => {}
                    0x00 => break,
                    // hash u64, page u64, slot u16
                    0x01 => {
                        self.take(18)?;
                        break;
                    }
                    other => {
                        bail_coded!(ErrorCode::Int049, format!("bad string escape {other:#04x}"))
                    }
                }
            },
            tag => bail_coded!(ErrorCode::Int049, format!("bad value tag {tag:#04x}")),
        }
        Ok(())
    }

    fn string(&mut self) -> Result<KeyValue> {
        let mut bytes = Vec::new();
        loop {
            let b = self.byte()?;
            if b != 0 {
                bytes.push(b);
                continue;
            }
            match self.byte()? {
                0xFF => bytes.push(0),
                0x00 => {
                    let s = String::from_utf8(bytes)
                        .map_err(|_| err_coded!(ErrorCode::Int049, "key string not UTF-8"))?;
                    return Ok(KeyValue::Str(s));
                }
                0x01 => {
                    let hash = self.be(8)?;
                    let page = self.be(8)?;
                    let slot = u16::try_from(self.be(2)?)
                        .map_err(|_| err_coded!(ErrorCode::Int049, "slot out of range"))?;
                    return Ok(KeyValue::LongStr {
                        prefix: bytes,
                        hash,
                        vref: ValueRef { page, slot },
                    });
                }
                other => bail_coded!(ErrorCode::Int049, format!("bad string escape {other:#04x}")),
            }
        }
    }
}

// ─── Index keys (§5.2) ───────────────────────────────────────────────────────

/// The four index trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Eavt,
    Aevt,
    Avet,
    Vaet,
}

impl Index {
    pub const ALL: [Index; 4] = [Index::Eavt, Index::Aevt, Index::Avet, Index::Vaet];
}

/// A fact with dictionary ids in place of UUIDs and idents: one index entry.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyFact {
    pub e: u64,
    pub a: u32,
    pub v: KeyValue,
    pub tx_count: u64,
    pub vf: i64,
    pub vt: i64,
    pub asserted: bool,
}

impl KeyFact {
    /// The key of this fact in `index`, or `None` for VAET unless the value is a ref.
    pub fn key(&self, index: Index) -> Option<Vec<u8>> {
        let mut k = Vec::with_capacity(48);
        let a = u64::from(self.a);
        match index {
            Index::Eavt => {
                put_uint(&mut k, self.e);
                put_uint(&mut k, a);
                put_value(&mut k, &self.v);
            }
            Index::Aevt => {
                put_uint(&mut k, a);
                put_uint(&mut k, self.e);
                put_value(&mut k, &self.v);
            }
            Index::Avet => {
                put_uint(&mut k, a);
                put_value(&mut k, &self.v);
                put_uint(&mut k, self.e);
            }
            Index::Vaet => {
                let KeyValue::Ref(target) = self.v else {
                    return None;
                };
                put_uint(&mut k, target);
                put_uint(&mut k, a);
                put_uint(&mut k, self.e);
            }
        }
        put_tx_desc(&mut k, self.tx_count);
        put_int(&mut k, self.vf);
        put_valid_to(&mut k, self.vt);
        k.push(u8::from(self.asserted));
        Some(k)
    }

    /// Decode an index key of `index`.
    pub fn decode(index: Index, key: &[u8]) -> Result<KeyFact> {
        let mut r = Reader::new(key);
        let (e, a, v) = match index {
            Index::Eavt => {
                let e = r.uint()?;
                let a = r.uint()?;
                (e, a, r.value()?)
            }
            Index::Aevt => {
                let a = r.uint()?;
                let e = r.uint()?;
                (e, a, r.value()?)
            }
            Index::Avet => {
                let a = r.uint()?;
                let v = r.value()?;
                (r.uint()?, a, v)
            }
            Index::Vaet => {
                let target = r.uint()?;
                let a = r.uint()?;
                (r.uint()?, a, KeyValue::Ref(target))
            }
        };
        let tx_count = r.tx_desc()?;
        let vf = r.int()?;
        let vt = r.valid_to()?;
        let asserted = match r.byte()? {
            0 => false,
            1 => true,
            b => bail_coded!(ErrorCode::Int049, format!("bad op byte {b:#04x}")),
        };
        if !r.is_empty() {
            bail_coded!(ErrorCode::Int049, "trailing bytes after index key");
        }
        Ok(KeyFact {
            e,
            a: u32::try_from(a).map_err(|_| err_coded!(ErrorCode::Int049, "iid out of range"))?,
            v,
            tx_count,
            vf,
            vt,
            asserted,
        })
    }
}

/// Length of the leading triple of an `index` key: its three id/value
/// components, everything before `tx↓`. Entries with the same triple bytes are
/// the history of one `(e, a, v)`, newest first. The value is skipped, not decoded.
pub fn triple_len(index: Index, key: &[u8]) -> Result<usize> {
    let mut r = Reader::new(key);
    match index {
        Index::Eavt | Index::Aevt => {
            r.uint()?;
            r.uint()?;
            r.skip_value()?;
        }
        Index::Avet => {
            r.uint()?;
            r.skip_value()?;
            r.uint()?;
        }
        Index::Vaet => {
            r.uint()?;
            r.uint()?;
            r.uint()?;
        }
    }
    Ok(r.position())
}

/// `(e, a, tx_count)` of an EAVT or AEVT key, read without decoding the value.
pub fn entity_attribute_tx(index: Index, key: &[u8]) -> Result<(u64, u64, u64)> {
    let mut r = Reader::new(key);
    let (e, a) = match index {
        Index::Eavt => {
            let e = r.uint()?;
            (e, r.uint()?)
        }
        Index::Aevt => {
            let a = r.uint()?;
            (r.uint()?, a)
        }
        Index::Avet | Index::Vaet => {
            bail_coded!(ErrorCode::Int049, "entity_attribute_tx: EAVT or AEVT only")
        }
    };
    r.skip_value()?;
    Ok((e, a, r.tx_desc()?))
}

/// Key prefix of every EAVT entry of entity `e`.
pub fn entity_prefix(e: u64) -> Vec<u8> {
    let mut k = Vec::new();
    put_uint(&mut k, e);
    k
}

/// Key prefix of every EAVT entry of `(e, a)`.
pub fn entity_attribute_prefix(e: u64, a: u32) -> Vec<u8> {
    let mut k = entity_prefix(e);
    put_uint(&mut k, u64::from(a));
    k
}

/// Key prefix of every AEVT (or AVET) entry of attribute `a`.
pub fn attribute_prefix(a: u32) -> Vec<u8> {
    let mut k = Vec::new();
    put_uint(&mut k, u64::from(a));
    k
}

// ─── DICT keys (§5.2) ────────────────────────────────────────────────────────

pub const DICT_UUID_TO_EID: u8 = 0x01;
pub const DICT_EID_TO_UUID: u8 = 0x02;
pub const DICT_IDENT_TO_IID: u8 = 0x03;
pub const DICT_IID_TO_IDENT: u8 = 0x04;
pub const DICT_TX: u8 = 0x05;
pub const DICT_LONG_VALUE: u8 = 0x06;

pub fn dict_uuid_key(uuid: &Uuid) -> Vec<u8> {
    let mut k = vec![DICT_UUID_TO_EID];
    k.extend_from_slice(uuid.as_bytes());
    k
}

pub fn dict_eid_key(eid: u64) -> Vec<u8> {
    let mut k = vec![DICT_EID_TO_UUID];
    put_uint(&mut k, eid);
    k
}

pub fn dict_ident_key(ident: &str) -> Vec<u8> {
    let mut k = vec![DICT_IDENT_TO_IID];
    k.extend_from_slice(ident.as_bytes());
    k
}

pub fn dict_iid_key(iid: u32) -> Vec<u8> {
    let mut k = vec![DICT_IID_TO_IDENT];
    put_uint(&mut k, u64::from(iid));
    k
}

pub fn dict_tx_key(tx_count: u64) -> Vec<u8> {
    let mut k = vec![DICT_TX];
    put_uint(&mut k, tx_count);
    k
}

/// Prefix of every DICT 0x06 entry for long values with this hash.
pub fn dict_long_value_prefix(hash: u64) -> Vec<u8> {
    let mut k = vec![DICT_LONG_VALUE];
    k.extend_from_slice(&hash.to_be_bytes());
    k
}

pub fn dict_long_value_key(hash: u64, vref: ValueRef) -> Vec<u8> {
    let mut k = dict_long_value_prefix(hash);
    k.extend_from_slice(&vref.page.to_be_bytes());
    k.extend_from_slice(&vref.slot.to_be_bytes());
    k
}

/// The value ref at the end of a DICT 0x06 key.
pub fn dict_long_value_ref(key: &[u8]) -> Result<ValueRef> {
    let mut r = Reader::new(key);
    if r.byte()? != DICT_LONG_VALUE {
        bail_coded!(ErrorCode::Int049, "not a long-value DICT key");
    }
    r.be(8)?;
    let page = r.be(8)?;
    let slot =
        u16::try_from(r.be(2)?).map_err(|_| err_coded!(ErrorCode::Int049, "slot out of range"))?;
    if !r.is_empty() {
        bail_coded!(ErrorCode::Int049, "trailing bytes after DICT key");
    }
    Ok(ValueRef { page, slot })
}

/// Encode a dictionary value that is an integer id or timestamp.
pub fn uint_bytes(v: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(9);
    put_uint(&mut out, v);
    out
}

/// Decode a dictionary value written by [`uint_bytes`].
pub fn read_uint_bytes(bytes: &[u8]) -> Result<u64> {
    let mut r = Reader::new(bytes);
    let v = r.uint()?;
    if !r.is_empty() {
        bail_coded!(ErrorCode::Int049, "trailing bytes after integer");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    /// xorshift64*: deterministic randomness without a dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn uint_enc(v: u64) -> Vec<u8> {
        let mut o = Vec::new();
        put_uint(&mut o, v);
        o
    }

    fn int_enc(v: i64) -> Vec<u8> {
        let mut o = Vec::new();
        put_int(&mut o, v);
        o
    }

    fn interesting_ints(rng: &mut Rng) -> Vec<i64> {
        let mut v = vec![
            0,
            1,
            -1,
            255,
            256,
            -255,
            -256,
            -257,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
            i64::MAX - 1,
            1 << 41,
            -(1 << 41),
        ];
        for _ in 0..2000 {
            let bits = rng.below(64);
            let m = rng.next() >> (63 - bits);
            v.push(m.cast_signed());
            v.push(m.cast_signed().wrapping_neg());
        }
        v
    }

    #[test]
    fn signed_integers_sort_and_round_trip() {
        let mut rng = Rng(7);
        let mut vals = interesting_ints(&mut rng);
        vals.sort_unstable();
        vals.dedup();
        let encs: Vec<Vec<u8>> = vals.iter().map(|&v| int_enc(v)).collect();
        for w in encs.windows(2) {
            assert!(w[0] < w[1], "encoding order follows integer order");
        }
        for (v, e) in vals.iter().zip(&encs) {
            let mut r = Reader::new(e);
            assert_eq!(r.int().unwrap(), *v);
            assert!(r.is_empty());
        }
        assert_eq!(int_enc(0).len(), 1, "zero takes one byte");
        assert_eq!(int_enc(255).len(), 2);
        assert_eq!(int_enc(1 << 41).len(), 7, "a ms timestamp takes 7 bytes");
    }

    #[test]
    fn unsigned_integers_sort_and_round_trip() {
        let mut rng = Rng(11);
        let mut vals: Vec<u64> = vec![0, 1, 255, 256, u64::MAX, u64::MAX - 1];
        for _ in 0..2000 {
            vals.push(rng.next() >> rng.below(64));
        }
        vals.sort_unstable();
        vals.dedup();
        let encs: Vec<Vec<u8>> = vals.iter().map(|&v| uint_enc(v)).collect();
        for w in encs.windows(2) {
            assert!(w[0] < w[1], "encoding order follows integer order");
        }
        for (v, e) in vals.iter().zip(&encs) {
            assert_eq!(Reader::new(e).uint().unwrap(), *v);
        }
    }

    #[test]
    fn tx_desc_reverses_order_and_is_short() {
        let mut rng = Rng(3);
        let mut vals: Vec<u64> = vec![0, 1, 255, 256, 70_000, u64::MAX];
        for _ in 0..1000 {
            vals.push(rng.next() >> rng.below(64));
        }
        vals.sort_unstable();
        vals.dedup();
        let encs: Vec<Vec<u8>> = vals
            .iter()
            .map(|&v| {
                let mut o = Vec::new();
                put_tx_desc(&mut o, v);
                o
            })
            .collect();
        for w in encs.windows(2) {
            assert!(w[0] > w[1], "newer transactions sort first");
        }
        for (v, e) in vals.iter().zip(&encs) {
            let mut r = Reader::new(e);
            assert_eq!(r.tx_desc().unwrap(), *v);
            assert!(r.is_empty());
        }
        let mut o = Vec::new();
        put_tx_desc(&mut o, 70_000);
        assert_eq!(o.len(), 4, "a typical tx_count costs a few bytes");
    }

    #[test]
    fn forever_sorts_after_every_finite_valid_to() {
        let mut forever = Vec::new();
        put_valid_to(&mut forever, VALID_TIME_FOREVER);
        assert_eq!(forever, vec![0xFF]);
        let mut rng = Rng(5);
        for v in interesting_ints(&mut rng) {
            if v == VALID_TIME_FOREVER {
                continue;
            }
            let mut e = Vec::new();
            put_valid_to(&mut e, v);
            assert!(e < forever, "finite time sorts before FOREVER");
            assert_eq!(Reader::new(&e).valid_to().unwrap(), v);
        }
        assert_eq!(
            Reader::new(&forever).valid_to().unwrap(),
            VALID_TIME_FOREVER
        );
    }

    fn enc_value(v: &KeyValue) -> Vec<u8> {
        let mut o = Vec::new();
        put_value(&mut o, v);
        o
    }

    fn rank(v: &KeyValue) -> u8 {
        match v {
            KeyValue::Null => 0,
            KeyValue::Bool(_) => 1,
            KeyValue::Int(_) => 2,
            KeyValue::Float(_) => 3,
            KeyValue::Str(_) | KeyValue::LongStr { .. } => 4,
            KeyValue::Keyword(_) => 5,
            KeyValue::Ref(_) => 6,
        }
    }

    /// Logical order of values whose order is exact (all but long strings).
    fn logical_cmp(a: &KeyValue, b: &KeyValue) -> Ordering {
        rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (KeyValue::Bool(x), KeyValue::Bool(y)) => x.cmp(y),
            (KeyValue::Int(x), KeyValue::Int(y)) => x.cmp(y),
            (KeyValue::Float(x), KeyValue::Float(y)) => {
                // NaN is canonicalised to one value that sorts after +inf.
                let c = |f: f64| if f.is_nan() { f64::NAN } else { f };
                c(*x).total_cmp(&c(*y))
            }
            (KeyValue::Str(x), KeyValue::Str(y)) => x.as_bytes().cmp(y.as_bytes()),
            (KeyValue::Keyword(x), KeyValue::Keyword(y)) => x.cmp(y),
            (KeyValue::Ref(x), KeyValue::Ref(y)) => x.cmp(y),
            _ => Ordering::Equal,
        })
    }

    fn random_int(rng: &mut Rng) -> i64 {
        let m = (rng.next() >> rng.below(64)).cast_signed();
        if rng.below(2) == 0 {
            m
        } else {
            m.wrapping_neg()
        }
    }

    fn random_string(rng: &mut Rng, max_len: u64) -> String {
        let len = rng.below(max_len + 1);
        (0..len)
            .map(|_| match rng.below(4) {
                0 => '\0',
                1 => 'a',
                2 => 'b',
                _ => char::from(u8::try_from(rng.below(0x7f)).unwrap()),
            })
            .collect()
    }

    fn random_value(rng: &mut Rng) -> KeyValue {
        match rng.below(8) {
            0 => KeyValue::Null,
            1 => KeyValue::Bool(rng.below(2) == 1),
            2 => KeyValue::Int(random_int(rng)),
            3 => KeyValue::Float(match rng.below(6) {
                0 => f64::NAN,
                1 => 0.0,
                2 => -0.0,
                3 => f64::INFINITY,
                4 => f64::NEG_INFINITY,
                _ => f64::from_bits(rng.next()),
            }),
            4 | 5 => KeyValue::Str(random_string(rng, 64)),
            6 => KeyValue::Keyword(u32::try_from(rng.below(1 << 20)).unwrap()),
            _ => KeyValue::Ref(rng.next() >> rng.below(64)),
        }
    }

    #[test]
    fn value_encoding_order_matches_logical_order() {
        let mut rng = Rng(42);
        let mut vals: Vec<KeyValue> = (0..3000).map(|_| random_value(&mut rng)).collect();
        vals.push(KeyValue::Str("a".repeat(64)));
        vals.push(KeyValue::Str(String::new()));
        vals.push(KeyValue::Str("\0".into()));
        vals.push(KeyValue::Str("\0\0".into()));
        let mut by_logic = vals.clone();
        by_logic.sort_by(logical_cmp);
        let mut by_bytes = vals;
        by_bytes.sort_by_key(enc_value);
        let a: Vec<Vec<u8>> = by_logic.iter().map(enc_value).collect();
        let b: Vec<Vec<u8>> = by_bytes.iter().map(enc_value).collect();
        assert!(a == b, "memcmp order equals logical order");
    }

    #[test]
    fn values_round_trip() {
        let mut rng = Rng(9);
        for _ in 0..3000 {
            let v = random_value(&mut rng);
            let e = enc_value(&v);
            let mut r = Reader::new(&e);
            let back = r.value().unwrap();
            assert!(r.is_empty(), "value is self-delimiting");
            assert_eq!(enc_value(&back), e, "round trip");
        }
        let long = "x\0y".repeat(40);
        let v = long_str(&long, ValueRef { page: 77, slot: 3 });
        let e = enc_value(&v);
        assert_eq!(Reader::new(&e).value().unwrap(), v);
    }

    #[test]
    fn long_strings_order_by_prefix_when_prefixes_differ() {
        let r = ValueRef { page: 9, slot: 1 };
        let a = enc_value(&long_str(&format!("a{}", "z".repeat(100)), r));
        let b = enc_value(&long_str(&format!("b{}", "a".repeat(100)), r));
        let short_b = enc_value(&KeyValue::Str("b".into()));
        let short_a = enc_value(&KeyValue::Str("a".repeat(SHORT_STRING_MAX)));
        assert!(a < b, "first 32 bytes decide");
        assert!(
            short_a < a && a < short_b,
            "short and long interleave by prefix"
        );
    }

    fn random_fact(rng: &mut Rng) -> KeyFact {
        let ref_target = rng.below(2) == 0;
        KeyFact {
            e: rng.below(50),
            a: u32::try_from(rng.below(10)).unwrap(),
            v: if ref_target {
                KeyValue::Ref(rng.below(50))
            } else {
                random_value(rng)
            },
            tx_count: rng.below(40),
            vf: random_int(rng),
            vt: if rng.below(2) == 0 {
                VALID_TIME_FOREVER
            } else {
                (rng.next() >> 2).cast_signed() - (1 << 61)
            },
            asserted: rng.below(2) == 0,
        }
    }

    fn fact_cmp(index: Index, x: &KeyFact, y: &KeyFact) -> Ordering {
        let tail = |x: &KeyFact, y: &KeyFact| {
            y.tx_count
                .cmp(&x.tx_count)
                .then(x.vf.cmp(&y.vf))
                .then(x.vt.cmp(&y.vt))
                .then(x.asserted.cmp(&y.asserted))
        };
        let head = match index {
            Index::Eavt => {
                x.e.cmp(&y.e)
                    .then(x.a.cmp(&y.a))
                    .then_with(|| logical_cmp(&x.v, &y.v))
            }
            Index::Aevt => {
                x.a.cmp(&y.a)
                    .then(x.e.cmp(&y.e))
                    .then_with(|| logical_cmp(&x.v, &y.v))
            }
            Index::Avet => {
                x.a.cmp(&y.a)
                    .then_with(|| logical_cmp(&x.v, &y.v))
                    .then(x.e.cmp(&y.e))
            }
            Index::Vaet => logical_cmp(&x.v, &y.v)
                .then(x.a.cmp(&y.a))
                .then(x.e.cmp(&y.e)),
        };
        head.then_with(|| tail(x, y))
    }

    #[test]
    fn index_keys_sort_logically_and_round_trip() {
        let mut rng = Rng(1234);
        let facts: Vec<KeyFact> = (0..3000).map(|_| random_fact(&mut rng)).collect();
        for index in Index::ALL {
            let mut subset: Vec<&KeyFact> = facts
                .iter()
                .filter(|f| index != Index::Vaet || matches!(f.v, KeyValue::Ref(_)))
                .collect();
            let mut by_bytes = subset.clone();
            by_bytes.sort_by_key(|f| f.key(index).unwrap());
            subset.sort_by(|x, y| fact_cmp(index, x, y));
            let a: Vec<Vec<u8>> = subset.iter().map(|f| f.key(index).unwrap()).collect();
            let b: Vec<Vec<u8>> = by_bytes.iter().map(|f| f.key(index).unwrap()).collect();
            assert!(a == b, "memcmp order equals logical order");
            for f in &subset {
                let k = f.key(index).unwrap();
                let back = KeyFact::decode(index, &k).unwrap();
                assert_eq!(back.key(index).unwrap(), k, "round trip");
            }
        }
        let non_ref = KeyFact {
            v: KeyValue::Int(1),
            ..random_fact(&mut rng)
        };
        assert!(non_ref.key(Index::Vaet).is_none(), "VAET holds refs only");
    }

    /// `triple_len` ends exactly where `tx↓` starts, for every value type and
    /// index, and the byte after the triple is below 0xFF, so `triple ‖ 0xFF`
    /// sorts after the triple's whole history.
    #[test]
    fn triple_len_ends_where_the_transaction_starts() {
        let mut rng = Rng(4321);
        for _ in 0..3000 {
            let f = random_fact(&mut rng);
            let mut tail = Vec::new();
            put_tx_desc(&mut tail, f.tx_count);
            put_int(&mut tail, f.vf);
            put_valid_to(&mut tail, f.vt);
            tail.push(u8::from(f.asserted));
            for index in Index::ALL {
                let Some(k) = f.key(index) else { continue };
                let n = triple_len(index, &k).unwrap();
                assert_eq!(n, k.len() - tail.len(), "triple length");
                assert!(k[n] < 0xFF, "tx byte sorts below 0xFF");
            }
        }
        let long = long_str(&"x\0y".repeat(40), ValueRef { page: 7, slot: 2 });
        let f = KeyFact {
            e: 1,
            a: 2,
            v: long,
            tx_count: 9,
            vf: -3,
            vt: VALID_TIME_FOREVER,
            asserted: false,
        };
        let k = f.key(Index::Eavt).unwrap();
        let mut r = Reader::new(&k);
        r.uint().unwrap();
        r.uint().unwrap();
        r.value().unwrap();
        assert_eq!(triple_len(Index::Eavt, &k).unwrap(), r.position());
    }

    #[test]
    fn prefixes_bound_exactly_their_entity_or_attribute() {
        let f = KeyFact {
            e: 300,
            a: 7,
            v: KeyValue::Int(1),
            tx_count: 5,
            vf: 0,
            vt: VALID_TIME_FOREVER,
            asserted: true,
        };
        let eavt = f.key(Index::Eavt).unwrap();
        assert!(eavt.starts_with(&entity_prefix(300)));
        assert!(eavt.starts_with(&entity_attribute_prefix(300, 7)));
        assert!(!eavt.starts_with(&entity_prefix(3)));
        assert!(!eavt.starts_with(&entity_attribute_prefix(300, 70)));
        let aevt = f.key(Index::Aevt).unwrap();
        assert!(aevt.starts_with(&attribute_prefix(7)));
        assert!(!aevt.starts_with(&attribute_prefix(700)));
    }

    #[test]
    fn dict_keys_round_trip() {
        let r = ValueRef {
            page: 123_456,
            slot: 17,
        };
        let k = dict_long_value_key(0xDEAD_BEEF, r);
        assert!(k.starts_with(&dict_long_value_prefix(0xDEAD_BEEF)));
        assert_eq!(dict_long_value_ref(&k).unwrap(), r);
        assert_eq!(read_uint_bytes(&uint_bytes(1 << 41)).unwrap(), 1 << 41);
        assert!(dict_eid_key(1) < dict_eid_key(256), "eids sort in order");
        assert!(dict_tx_key(9) < dict_tx_key(10));
        assert_eq!(
            hash64(b"a"),
            0xaf63_dc4c_8601_ec8c,
            "FNV-1a 64 reference value"
        );
    }

    #[test]
    fn malformed_keys_are_int_049() {
        for bad in [&[][..], &[0x30][..], &[0x15][..], &[TAG_STRING, b'a'][..]] {
            let err = Reader::new(bad).value().unwrap_err();
            assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-049");
        }
        let err = KeyFact::decode(Index::Eavt, &[0x14, 0x14, TAG_NULL]).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-049");
    }

    #[test]
    fn size_limits() {
        assert_eq!(MAX_VALUE_BYTES, 4068);
        assert_eq!(MAX_IDENT_BYTES, 1024);
    }
}
