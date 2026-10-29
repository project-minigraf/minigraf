//! Differential check against the pre-#693 Reader implementation.
//! Kept test-only so the production refactor can be checked on malformed keys.

use super::*;

struct LegacyReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> LegacyReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
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

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
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

    fn uint(&mut self) -> Result<u64> {
        let t = self.byte()?;
        match t.checked_sub(INT_ZERO) {
            Some(n @ 0..=8) => self.be(usize::from(n)),
            _ => bail_coded!(
                ErrorCode::Int049,
                format!("bad unsigned type byte {t:#04x}")
            ),
        }
    }

    fn int(&mut self) -> Result<i64> {
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

    fn tx_desc(&mut self) -> Result<u64> {
        let t = !self.peek()?;
        let n = match t.checked_sub(INT_ZERO) {
            Some(n @ 0..=8) => usize::from(n),
            _ => bail_coded!(ErrorCode::Int049, format!("bad tx type byte {t:#04x}")),
        };
        let raw: Vec<u8> = self.take(n + 1)?.iter().map(|b| !b).collect();
        LegacyReader::new(&raw).uint()
    }

    fn valid_to(&mut self) -> Result<i64> {
        if self.peek()? == FOREVER {
            self.pos += 1;
            return Ok(VALID_TIME_FOREVER);
        }
        self.int()
    }

    fn value(&mut self) -> Result<KeyValue> {
        let tag = self.byte()?;
        Ok(match tag {
            TAG_NULL => KeyValue::Null,
            TAG_BOOL => KeyValue::Bool(self.byte()? != 0),
            TAG_INT => KeyValue::Int(self.int()?),
            TAG_FLOAT => KeyValue::Float(float_from_bits(self.be(8)?)),
            TAG_STRING => match self.escaped()? {
                Escaped::Short(bytes) => KeyValue::Str(
                    String::from_utf8(bytes)
                        .map_err(|_| err_coded!(ErrorCode::Int049, "key string not UTF-8"))?,
                ),
                Escaped::Long { prefix, hash, vref } => KeyValue::LongStr { prefix, hash, vref },
            },
            TAG_SET => match self.escaped()? {
                Escaped::Short(bytes) => KeyValue::Set(bytes),
                Escaped::Long { prefix, hash, vref } => KeyValue::LongSet { prefix, hash, vref },
            },
            TAG_KEYWORD => KeyValue::Keyword(
                u32::try_from(self.uint()?)
                    .map_err(|_| err_coded!(ErrorCode::Int049, "iid out of range"))?,
            ),
            TAG_REF => KeyValue::Ref(self.uint()?),
            _ => bail_coded!(ErrorCode::Int049, format!("bad value tag {tag:#04x}")),
        })
    }

    fn escaped(&mut self) -> Result<Escaped> {
        let mut bytes = Vec::new();
        loop {
            let b = self.byte()?;
            if b != 0 {
                bytes.push(b);
                continue;
            }
            match self.byte()? {
                0xFF => bytes.push(0),
                0x00 => return Ok(Escaped::Short(bytes)),
                0x01 => {
                    let hash = self.be(8)?;
                    let page = self.be(8)?;
                    let slot = u16::try_from(self.be(2)?)
                        .map_err(|_| err_coded!(ErrorCode::Int049, "slot out of range"))?;
                    return Ok(Escaped::Long {
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

#[derive(Debug, PartialEq)]
struct Observation {
    value: Option<Vec<u8>>,
    error: Option<(String, String)>,
    pos: usize,
}

fn observe<T>(result: Result<T>, pos: usize, encode: fn(T) -> Vec<u8>) -> Observation {
    match result {
        Ok(value) => Observation {
            value: Some(encode(value)),
            error: None,
            pos,
        },
        Err(err) => {
            let message = err.to_string();
            let code = crate::error::MinigrafError::from(err).code().to_owned();
            Observation {
                value: None,
                error: Some((code, message)),
                pos,
            }
        }
    }
}

fn key_bytes(value: KeyValue) -> Vec<u8> {
    if let KeyValue::Float(f) = value {
        let mut bytes = vec![TAG_FLOAT];
        bytes.extend_from_slice(&f.to_bits().to_be_bytes());
        return bytes;
    }
    let mut bytes = Vec::new();
    put_value(&mut bytes, &value);
    bytes
}

fn compare_case(bytes: &[u8]) {
    let mut old = LegacyReader::new(bytes);
    let expected = observe(old.uint(), old.pos, |v| v.to_be_bytes().to_vec());
    let mut new = Reader::new(bytes);
    let actual = observe(new.uint(), new.pos, |v| v.to_be_bytes().to_vec());
    assert_eq!(actual, expected, "uint on {bytes:02x?}");

    let mut old = LegacyReader::new(bytes);
    let expected = observe(old.int(), old.pos, |v| v.to_be_bytes().to_vec());
    let mut new = Reader::new(bytes);
    let actual = observe(new.int(), new.pos, |v| v.to_be_bytes().to_vec());
    assert_eq!(actual, expected, "int on {bytes:02x?}");

    let mut old = LegacyReader::new(bytes);
    let expected = observe(old.tx_desc(), old.pos, |v| v.to_be_bytes().to_vec());
    let mut new = Reader::new(bytes);
    let actual = observe(new.tx_desc(), new.pos, |v| v.to_be_bytes().to_vec());
    assert_eq!(actual, expected, "tx_desc on {bytes:02x?}");

    let mut old = LegacyReader::new(bytes);
    let expected = observe(old.valid_to(), old.pos, |v| v.to_be_bytes().to_vec());
    let mut new = Reader::new(bytes);
    let actual = observe(new.valid_to(), new.pos, |v| v.to_be_bytes().to_vec());
    assert_eq!(actual, expected, "valid_to on {bytes:02x?}");

    let mut old = LegacyReader::new(bytes);
    let expected = observe(old.value(), old.pos, key_bytes);
    let mut new = Reader::new(bytes);
    let actual = observe(new.value(), new.pos, key_bytes);
    assert_eq!(actual, expected, "value on {bytes:02x?}");
}

#[test]
fn scalar_decoders_match_pre_refactor_reader() {
    compare_case(&[]);
    for value in [i64::MIN, i64::MAX] {
        let mut bytes = Vec::new();
        put_int(&mut bytes, value);
        compare_case(&bytes);
        bytes.push(0xA5);
        compare_case(&bytes);
    }
    compare_case(&[INT_ZERO + 8, 0x80, 0, 0, 0, 0, 0, 0, 0]);
    compare_case(&[INT_ZERO - 2, 0xFF, 0xFE]); // padded -1
    let mut finite_max = Vec::new();
    put_int(&mut finite_max, i64::MAX);
    compare_case(&finite_max); // valid_to also accepts the finite form
    compare_case(&[FOREVER]);
    let mut tx = Vec::new();
    put_tx_desc(&mut tx, u64::MAX);
    compare_case(&tx);
    let mut keyword_overflow = vec![TAG_KEYWORD];
    put_uint(&mut keyword_overflow, u64::from(u32::MAX) + 1);
    compare_case(&keyword_overflow);
    compare_case(&[TAG_BOOL, 0x7F]);
    for value in [
        KeyValue::Str(String::new()),
        KeyValue::Str("a\0é".to_owned()),
        KeyValue::Set(vec![0, 0xFF]),
        KeyValue::LongStr {
            prefix: vec![b'a'; LONG_PREFIX],
            hash: u64::MAX,
            vref: ValueRef { page: 7, slot: 2 },
        },
        KeyValue::LongSet {
            prefix: vec![0; LONG_PREFIX],
            hash: 0,
            vref: ValueRef { page: 9, slot: 3 },
        },
    ] {
        compare_case(&key_bytes(value));
    }
    compare_case(&[TAG_STRING, 0xFF, 0, 0]); // invalid UTF-8
    compare_case(&[TAG_SET, 0, 0x02]); // bad escape byte
    for bits in [(-0.0f64).to_bits(), 0.0f64.to_bits(), 0xFFF8_0000_0000_0001] {
        let mut bytes = vec![TAG_FLOAT];
        bytes.extend_from_slice(&bits.to_be_bytes());
        compare_case(&bytes);
    }
    for first in 0..=u8::MAX {
        compare_case(&[first]);
        for second in 0..=u8::MAX {
            compare_case(&[first, second]);
        }
    }
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for len in 3..=10 {
        for _ in 0..1024 {
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push(state as u8);
            }
            compare_case(&bytes);
        }
    }
}
