//! Packed fact page format (page_type = 0x02).
//!
//! Layout of a packed page:
//! ```text
//! [12-byte header]
//!   byte 0:    page_type  (0x02 = packed fact data)
//!   byte 1:    _reserved  (0x00)
//!   bytes 2-3: record_count  (u16 LE)
//!   bytes 4-11: next_page   (u64 LE, 0 = no overflow)
//!
//! [record directory: record_count × 4 bytes each]
//!   per entry: offset u16 LE | length u16 LE
//!   (offset measured from page start)
//!
//! [record data: variable-length postcard-serialised Facts]
//!   written from end of page backwards
//! ```
//!
//! Overflow pages (`page_type = 0x03`) are reserved for future use.
//! The `next_page` field is always written as 0 in Phase 6.2.

use crate::graph::types::Fact;
use crate::storage::index::FactRef;
use crate::storage::{StorageBackend, PAGE_SIZE};
use anyhow::Result;

/// Page type byte for packed fact pages.
pub const PAGE_TYPE_PACKED: u8 = 0x02;
/// Page type byte for overflow pages (reserved, not used in Phase 6.2).
pub const PAGE_TYPE_OVERFLOW: u8 = 0x03;

/// Packed page header size in bytes.
pub const PACKED_HEADER_SIZE: usize = 12;

/// Pack a slice of facts into packed pages.
///
/// Returns `(pages, fact_refs)` where:
/// - `pages[i]` is exactly `PAGE_SIZE` bytes of packed page data
/// - `fact_refs[j]` is the `FactRef { page_id, slot_index }` for `facts[j]`
///
/// `start_page_id` is assigned to `pages[0]`; subsequent pages get
/// `start_page_id + 1`, `start_page_id + 2`, etc.
pub fn pack_facts(facts: &[Fact], start_page_id: u64) -> Result<(Vec<Vec<u8>>, Vec<FactRef>)> {
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut fact_refs: Vec<FactRef> = Vec::with_capacity(facts.len());

    let mut current_page: Vec<u8> = new_packed_page();
    let mut current_record_count: u16 = 0;
    let mut dir_offset: usize = PACKED_HEADER_SIZE;
    let mut data_offset: usize = PAGE_SIZE; // data written from end backwards

    for fact in facts {
        let serialised = postcard::to_allocvec(fact)?;
        let len = serialised.len();
        let dir_entry_size = 4usize;

        // Check if this fact exceeds the maximum slot size.
        let max_slot_size = PAGE_SIZE - PACKED_HEADER_SIZE - 4; // 4 = one directory entry
        if len > max_slot_size {
            anyhow::bail!(
                "Fact serialised size {} bytes exceeds maximum slot size {} bytes",
                len,
                max_slot_size
            );
        }

        // Check if this fact fits on the current page.
        // Free space = data_offset - dir_offset - dir_entry_size (for the new dir entry).
        let free = data_offset.saturating_sub(dir_offset + dir_entry_size);
        if len > free || current_record_count == u16::MAX {
            // Flush current page and start a new one.
            write_record_count(&mut current_page, current_record_count);
            pages.push(current_page);
            current_page = new_packed_page();
            current_record_count = 0;
            dir_offset = PACKED_HEADER_SIZE;
            data_offset = PAGE_SIZE;
        }

        // Write data from end of page backwards.
        data_offset -= len;
        current_page[data_offset..data_offset + len].copy_from_slice(&serialised);

        // Write directory entry: offset (u16 LE) | length (u16 LE).
        let offset_u16 = data_offset as u16;
        let len_u16 = len as u16;
        current_page[dir_offset..dir_offset + 2].copy_from_slice(&offset_u16.to_le_bytes());
        current_page[dir_offset + 2..dir_offset + 4].copy_from_slice(&len_u16.to_le_bytes());
        dir_offset += 4;

        let page_id = start_page_id + pages.len() as u64;
        fact_refs.push(FactRef {
            page_id,
            slot_index: current_record_count,
        });
        current_record_count += 1;
    }

    // Always flush the last page (even if facts slice is empty).
    write_record_count(&mut current_page, current_record_count);
    pages.push(current_page);

    Ok((pages, fact_refs))
}

/// Read a single fact from a packed page at the given slot index.
pub fn read_slot(page: &[u8], slot: u16) -> Result<Fact> {
    if page.len() < PAGE_SIZE {
        anyhow::bail!(
            "Page too short: {} bytes (expected {})",
            page.len(),
            PAGE_SIZE
        );
    }
    if page[0] != PAGE_TYPE_PACKED {
        anyhow::bail!("Expected packed page (0x02), got 0x{:02x}", page[0]);
    }
    let record_count = u16::from_le_bytes([page[2], page[3]]);
    if slot >= record_count {
        anyhow::bail!(
            "Slot {} out of bounds (page has {} records)",
            slot,
            record_count
        );
    }
    let dir_base = PACKED_HEADER_SIZE + (slot as usize) * 4;
    let offset = u16::from_le_bytes([page[dir_base], page[dir_base + 1]]) as usize;
    let length = u16::from_le_bytes([page[dir_base + 2], page[dir_base + 3]]) as usize;
    if offset + length > PAGE_SIZE {
        anyhow::bail!("Record at slot {} extends beyond page boundary", slot);
    }
    let fact: Fact = postcard::from_bytes(&page[offset..offset + length])?;
    Ok(fact)
}

/// Read all facts from a contiguous range of packed fact pages.
///
/// `first_page_id` is the backend page ID of the first packed fact page.
/// `num_pages` is the number of pages to read.
/// Non-packed pages (e.g., index pages) are silently skipped.
pub fn read_all_from_pages(
    backend: &dyn StorageBackend,
    first_page_id: u64,
    num_pages: u64,
) -> Result<Vec<Fact>> {
    let mut facts = Vec::new();
    for i in 0..num_pages {
        let page = backend.read_page(first_page_id + i)?;
        if page.len() < PAGE_SIZE || page[0] != PAGE_TYPE_PACKED {
            continue;
        }
        let record_count = u16::from_le_bytes([page[2], page[3]]);
        for slot in 0..record_count {
            facts.push(read_slot(&page, slot)?);
        }
    }
    Ok(facts)
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn new_packed_page() -> Vec<u8> {
    let mut page = vec![0u8; PAGE_SIZE];
    page[0] = PAGE_TYPE_PACKED;
    // byte 1: reserved = 0x00 (already zero from vec initialisation)
    // bytes 2-3: record_count = 0 (written later via write_record_count)
    // bytes 4-11: next_page = 0 (already zero)
    page
}

fn write_record_count(page: &mut [u8], count: u16) {
    page[2..4].copy_from_slice(&count.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Fact, VALID_TIME_FOREVER, Value};
    use uuid::Uuid;

    fn make_fact(n: u64) -> Fact {
        Fact::with_valid_time(
            Uuid::from_u128(n as u128),
            ":attr".to_string(),
            Value::Integer(n as i64),
            n as u64,
            n,
            0,
            VALID_TIME_FOREVER,
        )
    }

    #[test]
    fn test_single_fact_roundtrip() {
        let facts = vec![make_fact(1)];
        let (pages, refs) = pack_facts(&facts, 1).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].page_id, 1);
        assert_eq!(refs[0].slot_index, 0);
        let recovered = read_slot(&pages[0], 0).unwrap();
        assert_eq!(recovered.entity, facts[0].entity);
        assert_eq!(recovered.tx_count, facts[0].tx_count);
    }

    #[test]
    fn test_multiple_facts_pack_fewer_pages() {
        let facts: Vec<Fact> = (0..50).map(make_fact).collect();
        let (pages, refs) = pack_facts(&facts, 1).unwrap();
        assert!(
            pages.len() < 50,
            "packed pages ({}) should be < 50",
            pages.len()
        );
        assert_eq!(refs.len(), 50);
    }

    #[test]
    fn test_slot_index_roundtrip() {
        let facts: Vec<Fact> = (0..30).map(make_fact).collect();
        let (pages, refs) = pack_facts(&facts, 1).unwrap();
        for (i, fact) in facts.iter().enumerate() {
            let r = &refs[i];
            let page = &pages[(r.page_id - 1) as usize]; // page_id is 1-based, pages vec is 0-based
            let recovered = read_slot(page, r.slot_index).unwrap();
            assert_eq!(recovered.entity, fact.entity, "fact {} mismatched", i);
        }
    }

    #[test]
    fn test_page_type_byte_is_0x02() {
        let facts = vec![make_fact(1)];
        let (pages, _) = pack_facts(&facts, 1).unwrap();
        assert_eq!(pages[0][0], PAGE_TYPE_PACKED);
    }

    #[test]
    fn test_read_all_from_pages_roundtrip() {
        use crate::storage::backend::MemoryBackend;
        let facts: Vec<Fact> = (0..60).map(make_fact).collect();
        let (pages, _refs) = pack_facts(&facts, 1).unwrap();
        let mut backend = MemoryBackend::new();
        for (i, page) in pages.iter().enumerate() {
            backend.write_page((i + 1) as u64, page).unwrap();
        }
        let recovered = read_all_from_pages(&backend, 1, pages.len() as u64).unwrap();
        assert_eq!(recovered.len(), 60);
        for (orig, rec) in facts.iter().zip(recovered.iter()) {
            assert_eq!(orig.entity, rec.entity);
        }
    }

    #[test]
    fn test_oversized_fact_returns_error() {
        // Create a fact with a very large string value (>4080 bytes)
        let big_string = "x".repeat(5000);
        let fact = Fact::with_valid_time(
            Uuid::from_u128(999),
            ":big".to_string(),
            Value::String(big_string),
            1,
            1,
            0,
            VALID_TIME_FOREVER,
        );
        let result = pack_facts(&[fact], 1);
        assert!(result.is_err(), "oversized fact must return Err, not panic");
    }
}
