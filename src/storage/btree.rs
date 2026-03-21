//! B+tree page serialisation for covering index persistence.
//!
//! `write_*_index` writes a `BTreeMap<K, FactRef>` to B+tree leaf pages starting
//! at `start_page_id` on the backend. Returns the root page ID.
//! `read_*_index` is the inverse — reads all entries back into a BTreeMap.
//!
//! # Page layout
//!
//! Because `StorageBackend::write_page` / `read_page` require exactly
//! `PAGE_SIZE` (4096) bytes, this module uses a simple paged-blob strategy:
//!
//! * Each index is serialised to a single postcard byte-string.
//! * That byte-string is split into chunks of `DATA_BYTES_PER_PAGE` bytes.
//! * Each chunk is stored in one backend page with the following header:
//!
//! ```text
//! Offset  Size  Field
//! 0       1     page_type  (0x11 = index data page)
//! 1       4     total_byte_len  (u32 LE, only meaningful in the first page)
//! 5       4     chunk_byte_len  (u32 LE, number of valid bytes in this page)
//! 9       1     is_last_page    (0x00 = more pages follow, 0x01 = last page)
//! 10      4082  data bytes (padded with zeros if chunk_byte_len < 4082)
//! ```
//!
//! `total_byte_len` in page 0 tells the reader how many bytes to expect in
//! total so it can pre-allocate and know when it's done.
//!
//! Phase 6.1 uses these only for save/load. In-memory BTreeMaps handle queries.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::storage::index::{AevtKey, AvetKey, EavtKey, FactRef, VaetKey};
use crate::storage::{PAGE_SIZE, StorageBackend};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum entries per leaf page (reserved for Phase 6.2 true B+tree layout).
#[allow(dead_code)]
pub const LEAF_CAPACITY: usize = 50;
/// Maximum children per internal page (reserved for Phase 6.2 true B+tree layout).
#[allow(dead_code)]
pub const INTERNAL_CAPACITY: usize = 50;

/// Page type byte: index data page.
pub const PAGE_TYPE_INDEX: u8 = 0x11;

/// Header bytes per page: type(1) + total_len(4) + chunk_len(4) + is_last(1) = 10.
const PAGE_HEADER_SIZE: usize = 10;

/// Number of data bytes available per page.
const DATA_BYTES_PER_PAGE: usize = PAGE_SIZE - PAGE_HEADER_SIZE;

// ─── Low-level page I/O ───────────────────────────────────────────────────────

fn write_blob(blob: &[u8], backend: &mut dyn StorageBackend, start_page_id: u64) -> Result<u64> {
    if blob.len() > u32::MAX as usize {
        anyhow::bail!(
            "index blob too large to serialize: {} bytes (max {})",
            blob.len(),
            u32::MAX
        );
    }
    let total_len = blob.len() as u32;
    let num_pages = if blob.is_empty() {
        1 // always write at least one page
    } else {
        blob.len().div_ceil(DATA_BYTES_PER_PAGE)
    };

    for i in 0..num_pages {
        let offset = i * DATA_BYTES_PER_PAGE;
        let chunk = if offset < blob.len() {
            &blob[offset..blob.len().min(offset + DATA_BYTES_PER_PAGE)]
        } else {
            &[]
        };
        let chunk_len = chunk.len() as u32;
        let is_last: u8 = if i == num_pages - 1 { 0x01 } else { 0x00 };

        let mut page = vec![0u8; PAGE_SIZE];
        page[0] = PAGE_TYPE_INDEX;
        page[1..5].copy_from_slice(&total_len.to_le_bytes());
        page[5..9].copy_from_slice(&chunk_len.to_le_bytes());
        page[9] = is_last;
        if !chunk.is_empty() {
            page[PAGE_HEADER_SIZE..PAGE_HEADER_SIZE + chunk.len()].copy_from_slice(chunk);
        }

        backend.write_page(start_page_id + i as u64, &page)?;
    }

    // Return the page ID immediately after the last page written.
    Ok(start_page_id + num_pages as u64)
}

fn read_blob(backend: &dyn StorageBackend, start_page_id: u64) -> Result<Vec<u8>> {
    // Read the first page to get total_len.
    let first_page = backend.read_page(start_page_id)?;
    if first_page[0] != PAGE_TYPE_INDEX {
        anyhow::bail!(
            "Expected index page type 0x{:02x}, got 0x{:02x}",
            PAGE_TYPE_INDEX,
            first_page[0]
        );
    }

    let total_len = u32::from_le_bytes(first_page[1..5].try_into().unwrap()) as usize;
    let mut blob = Vec::with_capacity(total_len);

    let mut page_id = start_page_id;
    loop {
        let page = backend.read_page(page_id)?;
        if page[0] != PAGE_TYPE_INDEX {
            anyhow::bail!("Expected index page at page {}", page_id);
        }
        let chunk_len = u32::from_le_bytes(page[5..9].try_into().unwrap()) as usize;
        let is_last = page[9] == 0x01;

        if chunk_len > 0 {
            blob.extend_from_slice(&page[PAGE_HEADER_SIZE..PAGE_HEADER_SIZE + chunk_len]);
        }

        if is_last {
            break;
        }
        page_id += 1;
    }

    Ok(blob)
}

// ─── Generic index write/read ─────────────────────────────────────────────────

fn write_index_generic<K>(
    map: &BTreeMap<K, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<u64>
where
    K: serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone,
{
    let entries: Vec<(&K, &FactRef)> = map.iter().collect();
    let blob = postcard::to_allocvec(&entries)?;
    write_blob(&blob, backend, start_page_id)
}

fn read_index_generic<K>(
    root_page_id: u64,
    backend: &dyn StorageBackend,
) -> Result<BTreeMap<K, FactRef>>
where
    K: serde::Serialize + for<'de> serde::Deserialize<'de> + Ord + Clone,
{
    let blob = read_blob(backend, root_page_id)?;
    let entries: Vec<(K, FactRef)> = postcard::from_bytes(&blob)?;
    Ok(entries.into_iter().collect())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Write the EAVT index to pages starting at `start_page_id`.
///
/// Returns the next free page ID (i.e., `start_page_id + pages_written`).
pub fn write_eavt_index(
    map: &BTreeMap<EavtKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<u64> {
    write_index_generic(map, backend, start_page_id)
}

/// Read the EAVT index from pages starting at `root_page_id`.
pub fn read_eavt_index(
    root_page_id: u64,
    backend: &dyn StorageBackend,
) -> Result<BTreeMap<EavtKey, FactRef>> {
    read_index_generic(root_page_id, backend)
}

/// Write the AEVT index. Returns next free page ID.
pub fn write_aevt_index(
    map: &BTreeMap<AevtKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<u64> {
    write_index_generic(map, backend, start_page_id)
}

/// Read the AEVT index.
pub fn read_aevt_index(
    root_page_id: u64,
    backend: &dyn StorageBackend,
) -> Result<BTreeMap<AevtKey, FactRef>> {
    read_index_generic(root_page_id, backend)
}

/// Write the AVET index. Returns next free page ID.
pub fn write_avet_index(
    map: &BTreeMap<AvetKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<u64> {
    write_index_generic(map, backend, start_page_id)
}

/// Read the AVET index.
pub fn read_avet_index(
    root_page_id: u64,
    backend: &dyn StorageBackend,
) -> Result<BTreeMap<AvetKey, FactRef>> {
    read_index_generic(root_page_id, backend)
}

/// Write the VAET index. Returns next free page ID.
pub fn write_vaet_index(
    map: &BTreeMap<VaetKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<u64> {
    write_index_generic(map, backend, start_page_id)
}

/// Read the VAET index.
pub fn read_vaet_index(
    root_page_id: u64,
    backend: &dyn StorageBackend,
) -> Result<BTreeMap<VaetKey, FactRef>> {
    read_index_generic(root_page_id, backend)
}

/// Write all four indexes to pages starting at `start_page_id`.
///
/// Returns `(eavt_root, aevt_root, avet_root, vaet_root)` — the start page ID
/// of each index blob. Each `*_root` value is the first page of that index's
/// blob; subsequent pages follow contiguously.
pub fn write_all_indexes(
    eavt: &std::collections::BTreeMap<crate::storage::index::EavtKey, FactRef>,
    aevt: &std::collections::BTreeMap<crate::storage::index::AevtKey, FactRef>,
    avet: &std::collections::BTreeMap<crate::storage::index::AvetKey, FactRef>,
    vaet: &std::collections::BTreeMap<crate::storage::index::VaetKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<(u64, u64, u64, u64)> {
    let eavt_root = start_page_id;
    let after_eavt = write_eavt_index(eavt, backend, eavt_root)?;

    let aevt_root = after_eavt;
    let after_aevt = write_aevt_index(aevt, backend, aevt_root)?;

    let avet_root = after_aevt;
    let after_avet = write_avet_index(avet, backend, avet_root)?;

    let vaet_root = after_avet;
    write_vaet_index(vaet, backend, vaet_root)?;

    Ok((eavt_root, aevt_root, avet_root, vaet_root))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::memory::MemoryBackend;
    use crate::storage::index::{EavtKey, FactRef};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn eavt_entry(entity_lo: u128, attr: &str, tx: u64) -> (EavtKey, FactRef) {
        (
            EavtKey {
                entity: Uuid::from_u128(entity_lo),
                attribute: attr.to_string(),
                valid_from: 0,
                valid_to: i64::MAX,
                tx_count: tx,
            },
            FactRef {
                page_id: tx,
                slot_index: 0,
            },
        )
    }

    #[test]
    fn test_empty_eavt_roundtrip() {
        let mut backend = MemoryBackend::new();
        let map: BTreeMap<EavtKey, FactRef> = BTreeMap::new();
        let next = write_eavt_index(&map, &mut backend, 1).unwrap();
        // Empty serialization should still write exactly one page.
        assert_eq!(next, 2, "empty index should consume exactly 1 page");
        let recovered = read_eavt_index(1, &backend).unwrap();
        assert_eq!(recovered.len(), 0);
    }

    #[test]
    fn test_small_eavt_roundtrip() {
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..10 {
            let (k, v) = eavt_entry(i, ":name", i as u64 + 1);
            map.insert(k, v);
        }
        let next = write_eavt_index(&map, &mut backend, 1).unwrap();
        assert!(next >= 2, "should write at least one page");
        let recovered = read_eavt_index(1, &backend).unwrap();
        assert_eq!(recovered.len(), 10);
        for (k, v) in &map {
            assert_eq!(recovered.get(k), Some(v), "missing key {:?}", k);
        }
    }

    #[test]
    fn test_large_eavt_roundtrip_multi_leaf() {
        // Force multi-page: insert more entries than LEAF_CAPACITY (50).
        // With 150 entries × ~50 bytes each ≈ 7500 bytes > DATA_BYTES_PER_PAGE (4086),
        // so this will span at least 2 pages.
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..150 {
            let (k, v) = eavt_entry(i, ":attr", i as u64 + 1);
            map.insert(k, v);
        }
        let next = write_eavt_index(&map, &mut backend, 1).unwrap();
        assert!(next > 2, "150 entries must span multiple pages");
        let recovered = read_eavt_index(1, &backend).unwrap();
        assert_eq!(recovered.len(), 150);
        for (k, v) in &map {
            assert_eq!(recovered.get(k), Some(v));
        }
    }

    #[test]
    fn test_eavt_preserves_sort_order() {
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..100 {
            let (k, v) = eavt_entry(i, ":x", i as u64 + 1);
            map.insert(k, v);
        }
        let root = write_eavt_index(&map, &mut backend, 1).unwrap();
        let recovered = read_eavt_index(1, &backend).unwrap();
        let orig_keys: Vec<_> = map.keys().collect();
        let rec_keys: Vec<_> = recovered.keys().collect();
        assert_eq!(orig_keys, rec_keys, "Sort order must be preserved");
        // Ensure the return value is sane.
        assert!(root >= 2);
    }
}
