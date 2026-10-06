//! B+tree node codecs (spec §4.3).
//!
//! **Leaf (0x61).** After the common header, entries in key order, each
//! `varint shared ‖ varint suffix_len ‖ suffix ‖ varint value_len ‖ value`,
//! where `shared` is the length of the prefix shared with the previous key.
//! Every 16th entry is a restart point (`shared = 0`); their offsets are a `u16`
//! array at the end of the page. The header's `count` is the number of entries.
//!
//! **Internal (0x62).** `rightmost_child u64` at offset 24, then entries
//! `varint sep_len ‖ sep ‖ child u64` from offset 32, with a `u16` slot array at
//! the end of the page. Child `i` holds the keys below separator `i` (and at or
//! above separator `i − 1`); `rightmost_child` holds the keys at or above the
//! last separator. The header's `count` is the number of separators.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::PAGE_SIZE;
use crate::storage::page::{PAGE_HEADER_SIZE, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, new_page};
use anyhow::Result;

/// Entries between restart points.
pub const RESTART_INTERVAL: usize = 16;
/// Offset of an internal node's `rightmost_child`.
pub const RIGHTMOST_CHILD_OFFSET: usize = PAGE_HEADER_SIZE;
/// First byte of an internal node's entries.
pub const INTERNAL_BODY: usize = PAGE_HEADER_SIZE + 8;

/// A key and its value. The index trees use empty values.
pub type Entry = (Vec<u8>, Vec<u8>);

fn invalid(msg: impl Into<String>) -> anyhow::Error {
    err_coded!(ErrorCode::Int049, msg.into())
}

// ─── Varints and reads ───────────────────────────────────────────────────────

/// Append `v` as LEB128.
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(u8::try_from(v & 0x7F).unwrap_or(0) | 0x80);
        v >>= 7;
    }
    out.push(u8::try_from(v).unwrap_or(0));
}

/// Bytes `put_varint` writes for `v`.
pub fn varint_len(v: usize) -> usize {
    let mut n = 1;
    let mut v = v >> 7;
    while v > 0 {
        n += 1;
        v >>= 7;
    }
    n
}

/// Read a LEB128 value at `*pos`, advancing it.
fn get_varint(page: &[u8], pos: &mut usize) -> Result<usize> {
    let mut v: u64 = 0;
    for shift in (0..64).step_by(7) {
        let b = *page
            .get(*pos)
            .ok_or_else(|| invalid("varint runs past the page"))?;
        *pos += 1;
        v |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return usize::try_from(v).map_err(|_| invalid("varint overflows usize"));
        }
    }
    Err(invalid("varint too long"))
}

fn get_slice<'a>(page: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| invalid("length overflow"))?;
    let s = page
        .get(*pos..end)
        .ok_or_else(|| invalid("entry runs past the page"))?;
    *pos = end;
    Ok(s)
}

fn get_u16(page: &[u8], at: usize) -> Result<usize> {
    let b = page
        .get(at..at.saturating_add(2))
        .ok_or_else(|| invalid("u16 past the page"))?;
    Ok(usize::from(u16::from_le_bytes([
        b.first().copied().unwrap_or(0),
        b.get(1).copied().unwrap_or(0),
    ])))
}

fn get_u64(page: &[u8], at: usize) -> Result<u64> {
    let b = page
        .get(at..at.saturating_add(8))
        .ok_or_else(|| invalid("u64 past the page"))?;
    let arr: [u8; 8] = b.try_into().map_err(|_| invalid("u64 not 8 bytes"))?;
    Ok(u64::from_le_bytes(arr))
}

fn put_u16_at(page: &mut [u8], at: usize, v: usize) -> Result<()> {
    let v = u16::try_from(v).map_err(|_| invalid("offset exceeds u16"))?;
    page.get_mut(at..at.saturating_add(2))
        .ok_or_else(|| invalid("u16 write past the page"))?
        .copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// The entry count in a node's header.
pub fn node_count(page: &[u8]) -> Result<usize> {
    get_u16(page, 2)
}

fn shared_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

// ─── Leaves ──────────────────────────────────────────────────────────────────

/// Encoded size of one leaf entry.
fn leaf_entry_len(shared: usize, key: &[u8], value: &[u8]) -> usize {
    let suffix = key.len().saturating_sub(shared);
    varint_len(shared) + varint_len(suffix) + suffix + varint_len(value.len()) + value.len()
}

/// Bytes a leaf holding `entries` needs, header and restart array included.
pub fn leaf_size(entries: &[Entry]) -> usize {
    let mut size = PAGE_HEADER_SIZE;
    let mut prev: &[u8] = &[];
    for (i, (k, v)) in entries.iter().enumerate() {
        let restart = i % RESTART_INTERVAL == 0;
        let shared = if restart { 0 } else { shared_len(prev, k) };
        size += leaf_entry_len(shared, k, v);
        if restart {
            size += 2;
        }
        prev = k;
    }
    size
}

/// Encode a leaf. Fails with INT-049 if the entries do not fit in a page.
pub fn encode_leaf(entries: &[Entry]) -> Result<Vec<u8>> {
    let size = leaf_size(entries);
    if size > PAGE_SIZE {
        bail_coded!(
            ErrorCode::Int049,
            format!("leaf entries need {size} bytes, more than a page")
        );
    }
    let count = u16::try_from(entries.len()).map_err(|_| invalid("too many entries for a leaf"))?;
    let restarts = entries.len().div_ceil(RESTART_INTERVAL);
    let mut body = Vec::with_capacity(size);
    let mut offsets = Vec::with_capacity(restarts);
    let mut prev: &[u8] = &[];
    for (i, (k, v)) in entries.iter().enumerate() {
        let shared = if i % RESTART_INTERVAL == 0 {
            offsets.push(PAGE_HEADER_SIZE + body.len());
            0
        } else {
            shared_len(prev, k)
        };
        let suffix = k.get(shared..).unwrap_or(&[]);
        put_varint(&mut body, u64::try_from(shared).unwrap_or(0));
        put_varint(&mut body, u64::try_from(suffix.len()).unwrap_or(0));
        body.extend_from_slice(suffix);
        put_varint(&mut body, u64::try_from(v.len()).unwrap_or(0));
        body.extend_from_slice(v);
        prev = k;
    }
    let mut page = new_page(PAGE_TYPE_LEAF, count);
    page.get_mut(PAGE_HEADER_SIZE..PAGE_HEADER_SIZE + body.len())
        .ok_or_else(|| invalid("leaf body past the page"))?
        .copy_from_slice(&body);
    let array = PAGE_SIZE - 2 * restarts;
    for (j, off) in offsets.into_iter().enumerate() {
        put_u16_at(&mut page, array + 2 * j, off)?;
    }
    Ok(page)
}

/// Decode one entry at `*pos` given the previous key; checks `shared`.
fn read_leaf_entry(page: &[u8], pos: &mut usize, prev: &[u8], end: usize) -> Result<Entry> {
    let shared = get_varint(page, pos)?;
    if shared > prev.len() {
        bail_coded!(
            ErrorCode::Int049,
            "leaf entry shares more than its predecessor"
        );
    }
    let suffix_len = get_varint(page, pos)?;
    let suffix = get_slice(page, pos, suffix_len)?;
    let value_len = get_varint(page, pos)?;
    let value = get_slice(page, pos, value_len)?;
    if *pos > end {
        bail_coded!(ErrorCode::Int049, "leaf entry overlaps the restart array");
    }
    let mut key = Vec::with_capacity(shared + suffix_len);
    key.extend_from_slice(prev.get(..shared).unwrap_or(&[]));
    key.extend_from_slice(suffix);
    Ok((key, value.to_vec()))
}

/// Decode every entry of a leaf, checking the restart array and key order.
pub fn decode_leaf(page: &[u8]) -> Result<Vec<Entry>> {
    if page.first().copied() != Some(PAGE_TYPE_LEAF) {
        bail_coded!(ErrorCode::Int049, "not a leaf page");
    }
    let count = node_count(page)?;
    let restarts = count.div_ceil(RESTART_INTERVAL);
    let end = PAGE_SIZE
        .checked_sub(2 * restarts)
        .filter(|&e| e >= PAGE_HEADER_SIZE)
        .ok_or_else(|| invalid("restart array overflows the leaf"))?;
    let mut entries: Vec<Entry> = Vec::with_capacity(count);
    let mut pos = PAGE_HEADER_SIZE;
    for i in 0..count {
        if i % RESTART_INTERVAL == 0 && get_u16(page, end + 2 * (i / RESTART_INTERVAL))? != pos {
            bail_coded!(ErrorCode::Int049, "leaf restart offset mismatch");
        }
        // A restart entry stores its whole key, so it shares nothing.
        let prev: &[u8] = if i % RESTART_INTERVAL == 0 {
            &[]
        } else {
            entries.last().map_or(&[], |(k, _)| k.as_slice())
        };
        let entry = read_leaf_entry(page, &mut pos, prev, end)?;
        if entries.last().is_some_and(|(k, _)| *k >= entry.0) {
            bail_coded!(ErrorCode::Int049, "leaf keys out of order");
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// The value stored under `key` in a leaf, found by binary search over the
/// restart points and a scan of at most [`RESTART_INTERVAL`] entries.
pub fn leaf_get(page: &[u8], key: &[u8]) -> Result<Option<Vec<u8>>> {
    let count = node_count(page)?;
    if count == 0 {
        return Ok(None);
    }
    let restarts = count.div_ceil(RESTART_INTERVAL);
    let end = PAGE_SIZE
        .checked_sub(2 * restarts)
        .ok_or_else(|| invalid("restart array overflows the leaf"))?;
    // The last restart whose key is <= `key`.
    let (mut lo, mut hi) = (0usize, restarts);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let mut pos = get_u16(page, end + 2 * mid)?;
        let (k, _) = read_leaf_entry(page, &mut pos, &[], end)?;
        if k.as_slice() <= key {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mut pos = get_u16(page, end + 2 * lo)?;
    let first = lo * RESTART_INTERVAL;
    let last = (first + RESTART_INTERVAL).min(count);
    let mut prev: Vec<u8> = Vec::new();
    for _ in first..last {
        let (k, v) = read_leaf_entry(page, &mut pos, &prev, end)?;
        match k.as_slice().cmp(key) {
            std::cmp::Ordering::Equal => return Ok(Some(v)),
            std::cmp::Ordering::Greater => return Ok(None),
            std::cmp::Ordering::Less => prev = k,
        }
    }
    Ok(None)
}

/// The first key of a leaf, or `None` if it is empty.
pub fn leaf_first_key(page: &[u8]) -> Result<Option<Vec<u8>>> {
    if node_count(page)? == 0 {
        return Ok(None);
    }
    let mut pos = PAGE_HEADER_SIZE;
    let (k, _) = read_leaf_entry(page, &mut pos, &[], PAGE_SIZE)?;
    Ok(Some(k))
}

// ─── Internal nodes ──────────────────────────────────────────────────────────

/// The shortest `s` with `left_last < s <= right_first`: the shortest prefix of
/// `right_first` that is greater than `left_last`.
pub fn shortest_separator(left_last: &[u8], right_first: &[u8]) -> Result<Vec<u8>> {
    if left_last >= right_first {
        bail_coded!(ErrorCode::Int049, "separator bounds out of order");
    }
    let p = shared_len(left_last, right_first);
    Ok(right_first
        .get(..p.saturating_add(1))
        .unwrap_or(right_first)
        .to_vec())
}

/// Bytes an internal node with these separators needs.
pub fn internal_size<'a>(seps: impl IntoIterator<Item = &'a [u8]>) -> usize {
    seps.into_iter()
        .map(|s| varint_len(s.len()) + s.len() + 8 + 2)
        .sum::<usize>()
        + INTERNAL_BODY
}

/// Encode an internal node. `children` has one more element than `seps`; its
/// last element is `rightmost_child`.
pub fn encode_internal(children: &[u64], seps: &[Vec<u8>]) -> Result<Vec<u8>> {
    if children.len() != seps.len().saturating_add(1) {
        bail_coded!(
            ErrorCode::Int049,
            "internal node needs one more child than separators"
        );
    }
    let size = internal_size(seps.iter().map(Vec::as_slice));
    if size > PAGE_SIZE {
        bail_coded!(
            ErrorCode::Int049,
            format!("internal node needs {size} bytes, more than a page")
        );
    }
    let count = u16::try_from(seps.len()).map_err(|_| invalid("too many separators"))?;
    let mut page = new_page(PAGE_TYPE_INTERNAL, count);
    let rightmost = children
        .last()
        .copied()
        .ok_or_else(|| invalid("internal node without children"))?;
    page.get_mut(RIGHTMOST_CHILD_OFFSET..INTERNAL_BODY)
        .ok_or_else(|| invalid("internal header past the page"))?
        .copy_from_slice(&rightmost.to_le_bytes());
    let mut body = Vec::new();
    let slots = PAGE_SIZE - 2 * seps.len();
    for (i, (sep, child)) in seps.iter().zip(children).enumerate() {
        put_u16_at(&mut page, slots + 2 * i, INTERNAL_BODY + body.len())?;
        put_varint(&mut body, u64::try_from(sep.len()).unwrap_or(0));
        body.extend_from_slice(sep);
        body.extend_from_slice(&child.to_le_bytes());
    }
    page.get_mut(INTERNAL_BODY..INTERNAL_BODY + body.len())
        .ok_or_else(|| invalid("internal body past the page"))?
        .copy_from_slice(&body);
    Ok(page)
}

/// Read access to an internal node without decoding every separator.
pub struct Internal<'a> {
    page: &'a [u8],
    count: usize,
    slots: usize,
}

impl<'a> Internal<'a> {
    pub fn new(page: &'a [u8]) -> Result<Self> {
        if page.first().copied() != Some(PAGE_TYPE_INTERNAL) {
            bail_coded!(ErrorCode::Int049, "not an internal page");
        }
        let count = node_count(page)?;
        let slots = PAGE_SIZE
            .checked_sub(2 * count)
            .filter(|&s| s >= INTERNAL_BODY)
            .ok_or_else(|| invalid("slot array overflows the internal node"))?;
        Ok(Internal { page, count, slots })
    }

    /// Number of separators; there are `count() + 1` children.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Separator `i` and the child below it.
    fn entry(&self, i: usize) -> Result<(&'a [u8], u64)> {
        let mut pos = get_u16(self.page, self.slots + 2 * i)?;
        if pos < INTERNAL_BODY {
            bail_coded!(ErrorCode::Int049, "internal slot points into the header");
        }
        let len = get_varint(self.page, &mut pos)?;
        let sep = get_slice(self.page, &mut pos, len)?;
        if pos.saturating_add(8) > self.slots {
            bail_coded!(ErrorCode::Int049, "internal entry overlaps the slot array");
        }
        Ok((sep, get_u64(self.page, pos)?))
    }

    pub fn sep(&self, i: usize) -> Result<&'a [u8]> {
        Ok(self.entry(i)?.0)
    }

    /// Child `i`; `i == count()` is `rightmost_child`.
    pub fn child(&self, i: usize) -> Result<u64> {
        if i == self.count {
            get_u64(self.page, RIGHTMOST_CHILD_OFFSET)
        } else {
            Ok(self.entry(i)?.1)
        }
    }

    /// All children in key order, `rightmost_child` last.
    pub fn children(&self) -> Result<Vec<u64>> {
        (0..=self.count).map(|i| self.child(i)).collect()
    }

    /// The child to take for `key`, searching from child `from`: `from` plus the
    /// number of separators from `from` on that are `<= key`.
    pub fn route(&self, key: &[u8], from: usize) -> Result<usize> {
        let (mut lo, mut hi) = (from, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.sep(mid)? <= key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> usize {
            usize::try_from(self.next() % n).unwrap()
        }
    }

    /// Sorted, distinct random keys that share prefixes often.
    fn random_entries(rng: &mut Rng, n: usize, value_max: u64) -> Vec<Entry> {
        let mut keys: Vec<Vec<u8>> = (0..n)
            .map(|_| {
                let len = 1 + rng.below(40);
                (0..len)
                    .map(|_| u8::try_from(rng.below(4)).unwrap() * 60)
                    .collect()
            })
            .collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .map(|k| {
                let v = (0..rng.below(value_max + 1))
                    .map(|_| u8::try_from(rng.below(256)).unwrap())
                    .collect();
                (k, v)
            })
            .collect()
    }

    /// The longest prefix of `entries` that fits in one leaf.
    fn fitting(entries: &[Entry]) -> &[Entry] {
        let mut n = entries.len();
        while leaf_size(&entries[..n]) > PAGE_SIZE {
            n -= 1;
        }
        &entries[..n]
    }

    #[test]
    fn varint_round_trip() {
        for v in [0u64, 1, 127, 128, 300, 16_383, 16_384, u64::from(u32::MAX)] {
            let mut out = Vec::new();
            put_varint(&mut out, v);
            assert_eq!(out.len(), varint_len(usize::try_from(v).unwrap()));
            let mut pos = 0;
            assert_eq!(
                get_varint(&out, &mut pos).unwrap(),
                usize::try_from(v).unwrap()
            );
        }
    }

    #[test]
    fn leaf_round_trip_and_lookup_agree_with_linear_scan() {
        let mut rng = Rng(17);
        for round in 0..60 {
            let all = random_entries(&mut rng, 300, if round % 2 == 0 { 0 } else { 20 });
            let entries = fitting(&all);
            let page = encode_leaf(entries).unwrap();
            assert_eq!(page.len(), PAGE_SIZE);
            assert_eq!(decode_leaf(&page).unwrap(), entries, "round trip");
            assert_eq!(
                leaf_first_key(&page).unwrap(),
                entries.first().map(|(k, _)| k.clone())
            );
            for (k, v) in entries {
                assert_eq!(leaf_get(&page, k).unwrap().as_ref(), Some(v), "present key");
            }
            for probe in random_entries(&mut rng, 50, 0) {
                let want = entries.iter().find(|(k, _)| *k == probe.0).map(|(_, v)| v);
                assert_eq!(leaf_get(&page, &probe.0).unwrap().as_ref(), want, "probe");
            }
        }
    }

    #[test]
    fn prefix_compression_shrinks_shared_keys() {
        let entries: Vec<Entry> = (0u8..200)
            .map(|i| {
                let mut k = vec![7u8; 30];
                k.push(i);
                (k, Vec::new())
            })
            .collect();
        let size = leaf_size(&entries);
        assert!(
            size < 200 * 10,
            "shared prefixes are stored once per restart"
        );
        let page = encode_leaf(&entries).unwrap();
        assert_eq!(decode_leaf(&page).unwrap(), entries);
    }

    #[test]
    fn empty_leaf() {
        let page = encode_leaf(&[]).unwrap();
        assert!(decode_leaf(&page).unwrap().is_empty());
        assert_eq!(leaf_get(&page, b"x").unwrap(), None);
        assert_eq!(leaf_first_key(&page).unwrap(), None);
    }

    #[test]
    fn overflowing_nodes_are_int_049() {
        let big: Vec<Entry> = (0u8..3).map(|i| (vec![i; 1500], Vec::new())).collect();
        let err = encode_leaf(&big).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-049");
        let seps: Vec<Vec<u8>> = (0u8..3).map(|i| vec![i; 1500]).collect();
        let err = encode_internal(&[1, 2, 3, 4], &seps).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "INT-049");
    }

    #[test]
    fn corrupt_leaf_is_rejected() {
        let entries: Vec<Entry> = (0u8..40).map(|i| (vec![1, i], vec![i])).collect();
        let page = encode_leaf(&entries).unwrap();
        let mut bad = page.clone();
        bad[2] = 200; // count far beyond the data
        assert!(decode_leaf(&bad).is_err());
        let mut bad = page.clone();
        bad[PAGE_HEADER_SIZE] = 9; // first entry claims a shared prefix
        assert!(decode_leaf(&bad).is_err());
        let mut bad = page;
        let last = PAGE_SIZE - 2;
        bad[last] ^= 1; // restart offset
        assert!(decode_leaf(&bad).is_err());
    }

    #[test]
    fn separators_are_shortest_and_bounded() {
        let mut rng = Rng(99);
        let entries = random_entries(&mut rng, 2000, 0);
        for w in entries.windows(2) {
            let (l, r) = (&w[0].0, &w[1].0);
            let s = shortest_separator(l, r).unwrap();
            assert!(l.as_slice() < s.as_slice() && s.as_slice() <= r.as_slice());
            assert!(r.starts_with(&s));
            assert!(
                s.len() == 1 || l.as_slice() >= &s[..s.len() - 1],
                "no shorter prefix of r separates"
            );
        }
        assert!(shortest_separator(b"b", b"a").is_err());
        assert_eq!(shortest_separator(b"ab", b"abc").unwrap(), b"abc".to_vec());
    }

    #[test]
    fn internal_round_trip_and_route() {
        let mut rng = Rng(5);
        let keys = random_entries(&mut rng, 400, 0);
        // Separators from consecutive key pairs; children numbered.
        let seps: Vec<Vec<u8>> = keys
            .windows(2)
            .step_by(4)
            .map(|w| shortest_separator(&w[0].0, &w[1].0).unwrap())
            .take(150)
            .collect();
        let children: Vec<u64> = (100..100 + u64::try_from(seps.len()).unwrap() + 1).collect();
        let page = encode_internal(&children, &seps).unwrap();
        let node = Internal::new(&page).unwrap();
        assert_eq!(node.count(), seps.len());
        assert_eq!(node.children().unwrap(), children);
        for (i, s) in seps.iter().enumerate() {
            assert_eq!(node.sep(i).unwrap(), s.as_slice());
        }
        for (k, _) in &keys {
            let want = seps.iter().filter(|s| s.as_slice() <= k.as_slice()).count();
            assert_eq!(node.route(k, 0).unwrap(), want, "route = separators <= key");
        }
    }
}
