//! Free-list pages (spec §4.4).
//!
//! A singly linked chain of 0x81 pages holding ids of pages the active meta does
//! not reference. Body after the common header: `next u64`, then `count` page
//! ids (u64 each). This version writes the whole chain afresh on every
//! checkpoint; the format is the same one copy-on-write push/pop will use.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::PAGE_SIZE;
use crate::storage::StorageBackend;
use crate::storage::cache::PageCache;
use crate::storage::page::{PAGE_HEADER_SIZE, PAGE_TYPE_FREELIST, PageAllocator, new_page};
use anyhow::Result;

const NEXT_OFFSET: usize = PAGE_HEADER_SIZE;
const IDS_OFFSET: usize = PAGE_HEADER_SIZE + 8;
/// Page ids per free-list page: (4096 − 24 − 8) / 8.
pub const IDS_PER_PAGE: usize = (PAGE_SIZE - IDS_OFFSET) / 8;

/// Write `ids` as a new chain and return `(head, chain_page_ids)`.
///
/// Chain pages come from `alloc`, so the chain never lists its own pages.
/// An empty list writes nothing and returns head 0.
pub fn write_chain(
    ids: &[u64],
    alloc: &mut PageAllocator,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
) -> Result<(u64, Vec<u64>)> {
    let chunks: Vec<&[u64]> = ids.chunks(IDS_PER_PAGE).collect();
    let mut page_ids = Vec::with_capacity(chunks.len());
    for _ in &chunks {
        page_ids.push(alloc.alloc()?);
    }
    for (i, chunk) in chunks.iter().enumerate() {
        let count = u16::try_from(chunk.len())
            .map_err(|_| err_coded!(ErrorCode::Int048, "free-list page count exceeds u16"))?;
        let mut page = new_page(PAGE_TYPE_FREELIST, count);
        let next = page_ids.get(i.saturating_add(1)).copied().unwrap_or(0);
        if let Some(dst) = page.get_mut(NEXT_OFFSET..IDS_OFFSET) {
            dst.copy_from_slice(&next.to_le_bytes());
        }
        for (j, id) in chunk.iter().enumerate() {
            let off = IDS_OFFSET.saturating_add(j.saturating_mul(8));
            if let Some(dst) = page.get_mut(off..off.saturating_add(8)) {
                dst.copy_from_slice(&id.to_le_bytes());
            }
        }
        let page_id = *page_ids
            .get(i)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "free-list page id missing"))?;
        alloc.write(backend, cache, page_id, page)?;
    }
    Ok((page_ids.first().copied().unwrap_or(0), page_ids))
}

/// Read the chain at `head` and return `(free_ids, chain_page_ids)`.
///
/// Every page is verified through `cache`. A cycle, or an id below 2 or at or
/// past `page_count`, is STG-035.
pub fn read_chain(
    head: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
    page_count: u64,
) -> Result<(Vec<u64>, Vec<u64>)> {
    let mut ids = Vec::new();
    let mut chain = Vec::new();
    let mut next = head;
    while next != 0 {
        if next < 2 || next >= page_count {
            bail_coded!(ErrorCode::Stg035, format!("chain page {next} out of range"));
        }
        // A chain longer than the file has pages must loop.
        if u64::try_from(chain.len()).unwrap_or(u64::MAX) >= page_count {
            bail_coded!(ErrorCode::Stg035, "chain loops");
        }
        let page = cache.get_or_load(next, backend)?;
        if page.first().copied() != Some(PAGE_TYPE_FREELIST) {
            bail_coded!(ErrorCode::Stg013, next);
        }
        chain.push(next);
        let count = usize::from(crate::storage::page::page_count_field(&page)?);
        if count > IDS_PER_PAGE {
            bail_coded!(ErrorCode::Stg035, format!("page {next} count {count}"));
        }
        for j in 0..count {
            let off = IDS_OFFSET.saturating_add(j.saturating_mul(8));
            let id = read_u64(&page, off)?;
            if id < 2 || id >= page_count {
                bail_coded!(ErrorCode::Stg035, format!("free id {id} out of range"));
            }
            ids.push(id);
        }
        next = read_u64(&page, NEXT_OFFSET)?;
    }
    Ok((ids, chain))
}

fn read_u64(page: &[u8], off: usize) -> Result<u64> {
    page.get(off..off.saturating_add(8))
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "free-list page too short"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;

    fn round_trip(n: u64) {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(16);
        // Free ids 2..2+n; chain pages are appended after them.
        let ids: Vec<u64> = (2..2 + n).collect();
        let mut alloc = PageAllocator::new(Vec::new(), 2 + n, 1);
        let (head, chain) = write_chain(&ids, &mut alloc, &mut backend, &cache).unwrap();
        assert_eq!(chain.len(), (n as usize).div_ceil(IDS_PER_PAGE));
        let (read, read_chain_ids) =
            read_chain(head, &backend, &PageCache::new(0), alloc.next_append()).unwrap();
        assert_eq!(read, ids, "ids round trip");
        assert_eq!(read_chain_ids, chain);
    }

    #[test]
    fn chain_round_trips_at_page_boundaries() {
        for n in [0, 1, 508, 509, 5_000] {
            round_trip(n);
        }
        assert_eq!(IDS_PER_PAGE, 508);
    }

    #[test]
    fn looping_chain_is_stg_035() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let alloc = PageAllocator::new(Vec::new(), 3, 1);
        let mut page = new_page(PAGE_TYPE_FREELIST, 0);
        page[NEXT_OFFSET..IDS_OFFSET].copy_from_slice(&2u64.to_le_bytes()); // points at itself
        alloc.write(&mut backend, &cache, 2, page).unwrap();
        let err = read_chain(2, &backend, &cache, 3).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-035");
    }

    #[test]
    fn out_of_range_id_is_stg_035() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let mut alloc = PageAllocator::new(Vec::new(), 2, 1);
        let (head, _) = write_chain(&[1], &mut alloc, &mut backend, &cache).unwrap();
        let err = read_chain(head, &backend, &cache, alloc.next_append()).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-035");
    }
}
