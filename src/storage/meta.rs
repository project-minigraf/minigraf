//! Format v8 meta pages (spec §4.1).
//!
//! Two alternating meta pages are the only commit point. Odd generations live in
//! page 0 (slot A), even generations in page 1 (slot B). Each covers the whole
//! page with its CRC, so the reserved tail must be zero.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::{FORMAT_VERSION, MAGIC_NUMBER, PAGE_SIZE};
use anyhow::Result;

/// Distinguishes a v8 meta page from a legacy header.
const META_MAGIC: [u8; 4] = *b"META";

/// File feature bits this version understands (spec §4.1.2). v3.0.0 defines none.
/// Bits are never reused; register new ones here with their meaning.
pub const KNOWN_FEATURES: u64 = 0;

const CRC_RANGE: std::ops::Range<usize> = 12..16;
/// Bytes 124.. are reserved and must be zero.
const FIELDS_END: usize = 124;

/// The contents of one meta page.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct MetaPage {
    pub generation: u64,
    pub page_count: u64,
    pub fact_count: u64,
    pub last_checkpointed_tx_count: u64,
    pub eavt_root: u64,
    pub aevt_root: u64,
    pub avet_root: u64,
    pub vaet_root: u64,
    pub dict_root: u64,
    pub freelist_head: u64,
    pub freelist_count: u64,
    pub next_eid: u64,
    pub required_features: u64,
    pub next_iid: u32,
}

/// How one meta slot reads.
pub enum SlotState {
    /// No `"MGRF"`/`"META"` magic: never written as a meta page.
    Empty,
    /// Magic, version and CRC check out.
    Valid(MetaPage),
    /// Magic present, but the CRC or a field fails.
    Damaged,
}

impl SlotState {
    /// True if this slot holds a valid meta of `generation`.
    #[cfg(test)]
    pub fn is_valid_generation(&self, generation: u64) -> bool {
        matches!(self, SlotState::Valid(m) if m.generation == generation)
    }
}

/// The page that holds generation `generation`'s meta: 0 for odd, 1 for even.
pub fn slot_page(generation: u64) -> u64 {
    generation.wrapping_sub(1) % 2
}

fn crc_of(page: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(page.get(..CRC_RANGE.start).unwrap_or(&[]));
    h.update(&[0u8; 4]);
    h.update(page.get(CRC_RANGE.end..).unwrap_or(&[]));
    h.finalize()
}

fn u64_at(page: &[u8], off: usize) -> u64 {
    page.get(off..off.saturating_add(8))
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}

fn u32_at(page: &[u8], off: usize) -> u32 {
    page.get(off..off.saturating_add(4))
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

impl MetaPage {
    /// The u64 fields in on-disk order, starting at byte 16.
    fn u64_fields(&self) -> [u64; 13] {
        [
            self.generation,
            self.page_count,
            self.fact_count,
            self.last_checkpointed_tx_count,
            self.eavt_root,
            self.aevt_root,
            self.avet_root,
            self.vaet_root,
            self.dict_root,
            self.freelist_head,
            self.freelist_count,
            self.next_eid,
            self.required_features,
        ]
    }

    /// Encode as a full page with its CRC.
    pub fn encode(&self) -> Vec<u8> {
        let mut page = vec![0u8; PAGE_SIZE];
        let mut put = |off: usize, bytes: &[u8]| {
            if let Some(dst) = page.get_mut(off..off.saturating_add(bytes.len())) {
                dst.copy_from_slice(bytes);
            }
        };
        put(0, &MAGIC_NUMBER);
        put(4, &FORMAT_VERSION.to_le_bytes());
        put(8, &META_MAGIC);
        for (i, v) in self.u64_fields().iter().enumerate() {
            put(
                16usize.saturating_add(i.saturating_mul(8)),
                &v.to_le_bytes(),
            );
        }
        put(120, &self.next_iid.to_le_bytes());
        let crc = crc_of(&page);
        if let Some(dst) = page.get_mut(CRC_RANGE) {
            dst.copy_from_slice(&crc.to_le_bytes());
        }
        page
    }

    /// Classify a page read from a meta slot.
    pub fn decode(page: &[u8]) -> SlotState {
        if page.len() != PAGE_SIZE
            || page.get(0..4) != Some(&MAGIC_NUMBER[..])
            || page.get(8..12) != Some(&META_MAGIC[..])
        {
            return SlotState::Empty;
        }
        if u32_at(page, 4) != FORMAT_VERSION
            || u32_at(page, CRC_RANGE.start) != crc_of(page)
            || page
                .get(FIELDS_END..)
                .is_some_and(|r| r.iter().any(|&b| b != 0))
        {
            return SlotState::Damaged;
        }
        let f = |i: usize| u64_at(page, 16usize.saturating_add(i.saturating_mul(8)));
        let meta = MetaPage {
            generation: f(0),
            page_count: f(1),
            fact_count: f(2),
            last_checkpointed_tx_count: f(3),
            eavt_root: f(4),
            aevt_root: f(5),
            avet_root: f(6),
            vaet_root: f(7),
            dict_root: f(8),
            freelist_head: f(9),
            freelist_count: f(10),
            next_eid: f(11),
            required_features: f(12),
            next_iid: u32_at(page, 120),
        };
        // Generation 0 is never committed, and pages 0 and 1 always exist.
        if meta.generation == 0 || meta.page_count < 2 {
            return SlotState::Damaged;
        }
        SlotState::Valid(meta)
    }

    /// Fail with STG-032 for a pre-release v8 file: one written before covering
    /// keys, whose trees have no dictionary. A committed file always has a DICT
    /// tree once it has index trees.
    pub fn check_layout(&self) -> Result<()> {
        if self.eavt_root != 0 && self.dict_root == 0 {
            bail_coded!(ErrorCode::Stg032);
        }
        Ok(())
    }

    /// Fail with STG-034 if the file needs a feature this version lacks.
    pub fn check_features(&self) -> Result<()> {
        let unknown = self.required_features & !KNOWN_FEATURES;
        if unknown != 0 {
            bail_coded!(ErrorCode::Stg034, format!("{unknown:#x}"));
        }
        Ok(())
    }

    /// All five tree roots: the four indexes, then DICT.
    pub fn tree_roots(&self) -> [u64; 5] {
        [
            self.eavt_root,
            self.aevt_root,
            self.avet_root,
            self.vaet_root,
            self.dict_root,
        ]
    }

    /// An empty database: no trees, no free pages, pages 0 and 1 allocated.
    pub fn empty(generation: u64) -> Self {
        MetaPage {
            generation,
            page_count: 2,
            ..MetaPage::default()
        }
    }

    /// The next generation, or an error on overflow.
    pub fn next_generation(&self) -> Result<u64> {
        self.generation
            .checked_add(1)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "generation overflow"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MetaPage {
        MetaPage {
            generation: 7,
            page_count: 900,
            fact_count: 1234,
            last_checkpointed_tx_count: 55,
            eavt_root: 10,
            aevt_root: 11,
            avet_root: 12,
            vaet_root: 13,
            dict_root: 14,
            freelist_head: 15,
            freelist_count: 16,
            next_eid: 17,
            required_features: 0,
            next_iid: 18,
        }
    }

    #[test]
    fn round_trip() {
        let m = sample();
        match MetaPage::decode(&m.encode()) {
            SlotState::Valid(d) => assert!(d == m, "decoded meta differs"),
            _ => panic!("expected a valid meta"),
        }
    }

    #[test]
    fn field_offsets_match_the_spec_table() {
        let p = sample().encode();
        let at = |o: usize| u64::from_le_bytes(p[o..o + 8].try_into().unwrap());
        assert_eq!(&p[0..4], b"MGRF");
        assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), 8);
        assert_eq!(&p[8..12], b"META");
        assert_eq!(at(16), 7);
        assert_eq!(at(24), 900);
        assert_eq!(at(32), 1234);
        assert_eq!(at(40), 55);
        assert_eq!(at(48), 10);
        assert_eq!(at(56), 11);
        assert_eq!(at(64), 12);
        assert_eq!(at(72), 13);
        assert_eq!(at(80), 14);
        assert_eq!(at(88), 15);
        assert_eq!(at(96), 16);
        assert_eq!(at(104), 17);
        assert_eq!(at(112), 0);
        assert_eq!(u32::from_le_bytes(p[120..124].try_into().unwrap()), 18);
        assert!(p[124..].iter().all(|&b| b == 0));
    }

    #[test]
    fn damaged_and_empty_are_told_apart() {
        let good = sample().encode();

        let mut flipped = good.clone();
        flipped[30] ^= 1;
        assert!(matches!(MetaPage::decode(&flipped), SlotState::Damaged));

        // A non-zero reserved byte is damage even with a matching CRC.
        let mut reserved = sample();
        reserved.generation = 7;
        let mut p = reserved.encode();
        p[2000] = 1;
        let crc = crc_of(&p);
        p[12..16].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(MetaPage::decode(&p), SlotState::Damaged));

        assert!(matches!(
            MetaPage::decode(&vec![0u8; PAGE_SIZE]),
            SlotState::Empty
        ));

        // A v7 header has "MGRF" but no "META".
        let mut v7 = vec![0u8; PAGE_SIZE];
        v7[0..4].copy_from_slice(b"MGRF");
        v7[4..8].copy_from_slice(&7u32.to_le_bytes());
        assert!(matches!(MetaPage::decode(&v7), SlotState::Empty));

        // A torn write that kept the magic but lost the tail.
        let mut torn = good.clone();
        torn[16..].fill(0);
        assert!(matches!(MetaPage::decode(&torn), SlotState::Damaged));
    }

    #[test]
    fn unknown_feature_bit_is_stg_034() {
        let mut m = sample();
        m.required_features = 1 << 5;
        let err = m.check_features().unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-034");
        assert!(sample().check_features().is_ok());
    }

    #[test]
    fn odd_generations_go_to_page_0() {
        assert_eq!(slot_page(1), 0);
        assert_eq!(slot_page(2), 1);
        assert_eq!(slot_page(3), 0);
        assert_eq!(slot_page(u64::MAX), 0);
    }
}
