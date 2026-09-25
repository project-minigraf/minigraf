//! Small non-cryptographic hasher for internal hot-path hash maps (#323).
//!
//! Same multiply-rotate scheme as rustc's `FxHasher`. Not HashDoS-resistant;
//! only use it for maps keyed by data the embedding application already
//! controls (facts in its own database), never for untrusted network input.

use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Word-at-a-time multiply-rotate hasher.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(chunk);
            self.add_to_hash(u64::from_le_bytes(buf));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            for (dst, src) in buf.iter_mut().zip(rem) {
                *dst = *src;
            }
            self.add_to_hash(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `BuildHasher` for [`FxHasher`]; use with `HashMap::default()`.
pub(crate) type FxBuildHasher = BuildHasherDefault<FxHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::hash::BuildHasher;

    #[test]
    fn equal_inputs_hash_equal() {
        let b = FxBuildHasher::default();
        assert_eq!(b.hash_one(("abc", 1u64)), b.hash_one(("abc", 1u64)));
    }

    #[test]
    fn remainder_bytes_affect_hash() {
        let b = FxBuildHasher::default();
        // 9 bytes: one full word + 1 remainder byte that differs.
        assert_ne!(b.hash_one(b"abcdefghX"), b.hash_one(b"abcdefghY"));
        assert_ne!(b.hash_one(":a"), b.hash_one(":ab"));
    }

    #[test]
    fn usable_as_map_and_set_hasher() {
        let mut m: HashMap<(&str, u64), u64, FxBuildHasher> = HashMap::default();
        m.insert((":x", 1), 10);
        m.insert((":x", 2), 20);
        assert_eq!(m.get(&(":x", 2)).copied(), Some(20));
        let s: HashSet<u64, FxBuildHasher> = (0..1000).collect();
        assert_eq!(s.len(), 1000);
    }
}
