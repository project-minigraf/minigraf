//! Format v7 packed fact pages, read only to migrate a v7 file (spec §9).
//!
//! Format v8 has no fact pages: the covering indexes hold every fact. A v7
//! fact page (page_type = 0x02) has a 12-byte header: type, reserved,
//! `record_count u16`, and a `next_page` field that was always 0. After it:
//! ```text
//! [record directory: record_count × 4 bytes each]
//!   per entry: offset u16 LE | length u16 LE (from page start)
//! [record data: postcard-serialised Facts, written from the end backwards]
//! ```

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::Fact;
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;

/// Page type byte for format v7 packed fact pages.
pub const PAGE_TYPE_PACKED_V7: u8 = 0x02;
/// Page type bytes of v2.x B+tree leaf and internal index pages.
const PAGE_TYPE_LEAF_V7: u8 = 0x21;
const PAGE_TYPE_INTERNAL_V7: u8 = 0x22;
/// Format v7 packed page header size in bytes.
const PACKED_HEADER_SIZE_V7: usize = 12;

fn u16_at(page: &[u8], at: usize) -> Result<u16> {
    page.get(at..at.saturating_add(2))
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| err_coded!(ErrorCode::Int024, format!("u16 at {at} out of bounds")))
}

/// Read the fact in `slot` of a v7 packed page.
pub fn read_slot_v7(page: &[u8], slot: u16) -> Result<Fact> {
    if page.len() < PAGE_SIZE {
        bail_coded!(
            ErrorCode::Int024,
            format!(
                "Page too short: {} bytes (expected {PAGE_SIZE})",
                page.len()
            )
        );
    }
    let page_type = page.first().copied().unwrap_or(0);
    if page_type != PAGE_TYPE_PACKED_V7 {
        bail_coded!(ErrorCode::Stg014, format!("{page_type:02x}"));
    }
    let record_count = u16_at(page, 2)?;
    if slot >= record_count {
        bail_coded!(
            ErrorCode::Int024,
            format!("Slot {slot} out of bounds (page has {record_count} records)")
        );
    }
    let dir = PACKED_HEADER_SIZE_V7.saturating_add(usize::from(slot).saturating_mul(4));
    let offset = usize::from(u16_at(page, dir)?);
    let length = usize::from(u16_at(page, dir.saturating_add(2))?);
    if offset.saturating_add(length) > PAGE_SIZE {
        bail_coded!(ErrorCode::Stg015, slot);
    }
    let bytes = page
        .get(offset..offset.saturating_add(length))
        .ok_or_else(|| err_coded!(ErrorCode::Int024, "record out of bounds"))?;
    Ok(postcard::from_bytes(bytes)?)
}

/// Read every fact from the `num_pages` v7 fact pages starting at
/// `first_page_id`.
///
/// A v2.x writer fills `1..=fact_page_count` with fact pages only (it always
/// writes at least one, even for no facts), so any other page type there is
/// damage: STG-014, never a skip (#496). When the count is `derived` from the
/// first index root instead, the range can also hold v2.x B+tree pages; only
/// those are skipped.
pub fn read_all_v7(
    backend: &dyn StorageBackend,
    first_page_id: u64,
    num_pages: u64,
    derived: bool,
) -> Result<Vec<Fact>> {
    let mut facts = Vec::new();
    for i in 0..num_pages {
        let page = backend.read_page(first_page_id.saturating_add(i))?;
        let page_type = page.first().copied().unwrap_or(0);
        if derived && matches!(page_type, PAGE_TYPE_LEAF_V7 | PAGE_TYPE_INTERNAL_V7) {
            continue;
        }
        if page_type != PAGE_TYPE_PACKED_V7 {
            bail_coded!(ErrorCode::Stg014, format!("{page_type:02x}"));
        }
        for slot in 0..u16_at(&page, 2)? {
            facts.push(read_slot_v7(&page, slot)?);
        }
    }
    Ok(facts)
}

/// Pack facts into v7 fact pages, as v2.x wrote them (test fixtures only).
#[cfg(test)]
pub fn pack_facts_v7(facts: &[Fact]) -> Vec<Vec<u8>> {
    let new_page = || {
        let mut p = vec![0u8; PAGE_SIZE];
        p[0] = PAGE_TYPE_PACKED_V7;
        p
    };
    let mut pages = Vec::new();
    let mut page = new_page();
    let (mut count, mut dir, mut data) = (0u16, PACKED_HEADER_SIZE_V7, PAGE_SIZE);
    for fact in facts {
        let rec = postcard::to_allocvec(fact).unwrap();
        if rec.len() + 4 > data - dir {
            page[2..4].copy_from_slice(&count.to_le_bytes());
            pages.push(std::mem::replace(&mut page, new_page()));
            (count, dir, data) = (0, PACKED_HEADER_SIZE_V7, PAGE_SIZE);
        }
        data -= rec.len();
        page[data..data + rec.len()].copy_from_slice(&rec);
        page[dir..dir + 2].copy_from_slice(&u16::try_from(data).unwrap().to_le_bytes());
        page[dir + 2..dir + 4].copy_from_slice(&u16::try_from(rec.len()).unwrap().to_le_bytes());
        dir += 4;
        count += 1;
    }
    page[2..4].copy_from_slice(&count.to_le_bytes());
    pages.push(page);
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{VALID_TIME_FOREVER, Value};
    use crate::storage::backend::MemoryBackend;
    use uuid::Uuid;

    fn make_fact(n: u64) -> Fact {
        Fact::with_valid_time(
            Uuid::from_u128(u128::from(n)),
            ":attr".to_string(),
            Value::Integer(i64::try_from(n).unwrap()),
            n,
            n,
            0,
            VALID_TIME_FOREVER,
        )
    }

    #[test]
    fn v7_pages_read_back_every_fact() {
        let facts: Vec<Fact> = (0..300).map(make_fact).collect();
        let pages = pack_facts_v7(&facts);
        assert!(pages.len() > 1, "fixture spans pages");
        let mut backend = MemoryBackend::new();
        for (i, p) in pages.iter().enumerate() {
            backend
                .write_page(u64::try_from(i).unwrap() + 1, p)
                .unwrap();
        }
        let read = read_all_v7(&backend, 1, u64::try_from(pages.len()).unwrap(), false).unwrap();
        assert_eq!(read.len(), 300);
        assert!(
            read.iter().zip(&facts).all(|(a, b)| a == b),
            "facts round trip"
        );
    }

    #[test]
    fn wrong_type_is_stg_014_and_overlong_record_is_stg_015() {
        let mut pages = pack_facts_v7(&[make_fact(1)]);
        let mut p = pages[0].clone();
        p[0] = 0x01;
        let err = read_slot_v7(&p, 0).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-014");
        pages[0][14] = 0xFF;
        pages[0][15] = 0xFF;
        let err = read_slot_v7(&pages[0], 0).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-015");
    }

    fn backend_with(pages: &[Vec<u8>]) -> MemoryBackend {
        let mut backend = MemoryBackend::new();
        for (i, p) in pages.iter().enumerate() {
            backend
                .write_page(u64::try_from(i).unwrap() + 1, p)
                .unwrap();
        }
        backend
    }

    /// A page in the counted fact range with another type byte is damaged:
    /// STG-014, never skipped (#496). Covers v2.x B+tree types too, and an
    /// all-zero page.
    #[test]
    fn damaged_type_in_counted_range_is_stg_014() {
        let pages = pack_facts_v7(&(0..300).map(make_fact).collect::<Vec<_>>());
        let n = u64::try_from(pages.len()).unwrap();
        for at in [0, pages.len() - 1] {
            for ty in [0x00, 0x01, 0x21, 0x22, 0xFF] {
                let mut damaged = pages.clone();
                damaged[at][0] = ty;
                let err = read_all_v7(&backend_with(&damaged), 1, n, false).unwrap_err();
                assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-014");
            }
        }
        let mut zeroed = pages.clone();
        zeroed[0] = vec![0u8; PAGE_SIZE];
        let err = read_all_v7(&backend_with(&zeroed), 1, n, false).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-014");
    }

    /// A derived range (no `fact_page_count`) ends at the first index root, so
    /// it can hold v2.x B+tree pages below it; those are skipped. Any other
    /// type is still damage.
    #[test]
    fn derived_range_skips_only_v7_btree_pages() {
        let mut pages = pack_facts_v7(&[make_fact(1), make_fact(2)]);
        for ty in [0x21, 0x22] {
            let mut p = vec![0u8; PAGE_SIZE];
            p[0] = ty;
            pages.push(p);
        }
        let n = u64::try_from(pages.len()).unwrap();
        let read = read_all_v7(&backend_with(&pages), 1, n, true).unwrap();
        assert_eq!(read.len(), 2);
        pages[1][0] = 0x01;
        let err = read_all_v7(&backend_with(&pages), 1, n, true).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-014");
    }
}
