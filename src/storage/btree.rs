//! Legacy v5 B+tree serialisation — kept for v5→v7 file format migration only.
//! The write_* functions are no longer called in production; they exist
//! only to support tests that verify round-trip serialisation.
// Suppress dead-code warnings: this entire module is legacy v5 code; the write_*
// functions are only invoked by the unit tests in this file.
#![allow(dead_code)]
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
    // Safe: guarded by the > u32::MAX check above.
    let total_len = u32::try_from(blob.len())
        .map_err(|_| anyhow::anyhow!("blob.len() overflows u32 (len={})", blob.len()))?;
    let num_pages = if blob.is_empty() {
        1usize // always write at least one page
    } else {
        blob.len().div_ceil(DATA_BYTES_PER_PAGE)
    };

    for i in 0..num_pages {
        let offset = i
            .checked_mul(DATA_BYTES_PER_PAGE)
            .ok_or_else(|| anyhow::anyhow!("btree write_blob: page offset overflow at i={i}"))?;
        let chunk = if offset < blob.len() {
            let end = blob.len().min(
                offset
                    .checked_add(DATA_BYTES_PER_PAGE)
                    .ok_or_else(|| anyhow::anyhow!("btree write_blob: end offset overflow"))?,
            );
            blob.get(offset..end).ok_or_else(|| {
                anyhow::anyhow!("btree write_blob: slice {offset}..{end} out of bounds")
            })?
        } else {
            &[]
        };
        let chunk_len = u32::try_from(chunk.len())
            .map_err(|_| anyhow::anyhow!("chunk.len() overflows u32 (len={})", chunk.len()))?;
        // num_pages >= 1 is guaranteed: write_blob always writes at least one page.
        let is_last: u8 = if i == num_pages - 1 { 0x01 } else { 0x00 };

        let mut page = vec![0u8; PAGE_SIZE];
        *page
            .get_mut(0)
            .ok_or_else(|| anyhow::anyhow!("btree: page index 0 out of bounds"))? = PAGE_TYPE_INDEX;
        page.get_mut(1..5)
            .ok_or_else(|| anyhow::anyhow!("btree: page slice 1..5 out of bounds"))?
            .copy_from_slice(&total_len.to_le_bytes());
        page.get_mut(5..9)
            .ok_or_else(|| anyhow::anyhow!("btree: page slice 5..9 out of bounds"))?
            .copy_from_slice(&chunk_len.to_le_bytes());
        *page
            .get_mut(9)
            .ok_or_else(|| anyhow::anyhow!("btree: page index 9 out of bounds"))? = is_last;
        if !chunk.is_empty() {
            let end = PAGE_HEADER_SIZE
                .checked_add(chunk.len())
                .ok_or_else(|| anyhow::anyhow!("btree write_blob: data end overflow"))?;
            page.get_mut(PAGE_HEADER_SIZE..end)
                .ok_or_else(|| {
                    anyhow::anyhow!("btree: page slice {PAGE_HEADER_SIZE}..{end} out of bounds")
                })?
                .copy_from_slice(chunk);
        }

        let page_offset = u64::try_from(i)
            .map_err(|_| anyhow::anyhow!("btree write_blob: page index {i} overflows u64"))?;
        backend.write_page(
            start_page_id
                .checked_add(page_offset)
                .ok_or_else(|| anyhow::anyhow!("btree write_blob: page_id overflow"))?,
            &page,
        )?;
    }

    // Return the page ID immediately after the last page written.
    let num_pages_u64 = u64::try_from(num_pages)
        .map_err(|_| anyhow::anyhow!("btree write_blob: num_pages {num_pages} overflows u64"))?;
    start_page_id
        .checked_add(num_pages_u64)
        .ok_or_else(|| anyhow::anyhow!("btree write_blob: final page_id overflow"))
}

fn read_blob(backend: &dyn StorageBackend, start_page_id: u64) -> Result<Vec<u8>> {
    // Read the first page to get total_len.
    let first_page = backend.read_page(start_page_id)?;
    let first_byte = *first_page
        .first()
        .ok_or_else(|| anyhow::anyhow!("btree read_blob: first_page index 0 out of bounds"))?;
    if first_byte != PAGE_TYPE_INDEX {
        anyhow::bail!(
            "Expected index page type 0x{:02x}, got 0x{:02x}",
            PAGE_TYPE_INDEX,
            first_byte
        );
    }

    let total_len = u32::from_le_bytes(
        first_page
            .get(1..5)
            .ok_or_else(|| anyhow::anyhow!("btree read_blob: first_page slice 1..5 out of bounds"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("btree read_blob: slice 1..5 not exactly 4 bytes"))?,
    ) as usize;
    let mut blob = Vec::with_capacity(total_len);

    let mut page_id = start_page_id;
    loop {
        let page = backend.read_page(page_id)?;
        let page_byte = *page.first().ok_or_else(|| {
            anyhow::anyhow!("btree read_blob: page index 0 out of bounds (page {page_id})")
        })?;
        if page_byte != PAGE_TYPE_INDEX {
            anyhow::bail!("Expected index page at page {}", page_id);
        }
        let chunk_len = u32::from_le_bytes(
            page.get(5..9)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "btree read_blob: page slice 5..9 out of bounds (page {page_id})"
                    )
                })?
                .try_into()
                .map_err(|_| anyhow::anyhow!("btree read_blob: slice 5..9 not exactly 4 bytes"))?,
        ) as usize;
        let is_last = *page.get(9).ok_or_else(|| {
            anyhow::anyhow!("btree read_blob: page index 9 out of bounds (page {page_id})")
        })? == 0x01;

        if chunk_len > 0 {
            let end = PAGE_HEADER_SIZE.checked_add(chunk_len).ok_or_else(|| {
                anyhow::anyhow!("btree read_blob: data end overflow (chunk_len={chunk_len})")
            })?;
            blob.extend_from_slice(page.get(PAGE_HEADER_SIZE..end).ok_or_else(|| {
                anyhow::anyhow!(
                    "btree read_blob: page slice {PAGE_HEADER_SIZE}..{end} out of bounds"
                )
            })?);
        }

        if is_last {
            break;
        }
        page_id = page_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("btree read_blob: page_id overflow"))?;
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
