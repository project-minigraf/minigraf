//! Free-list pages (spec §4.4).
//!
//! A singly linked chain of 0x81 pages holding ids of pages the active meta does
//! not reference. Body after the common header: `next u64`, then `count` page
//! ids (u64 each). A checkpoint reads chain pages only as it needs free ids and
//! pushes new head pages in front of the unread tail
//! ([`crate::storage::page::PageAllocator`]), so its cost is set by the pages it
//! allocates and frees, never by the length of the list.

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

/// Write `ids` into the chain pages `pages` (already allocated, in order), the
/// last one linking to `tail` (0: end of chain). `pages` must hold every id.
pub fn write_pages(
    ids: &[u64],
    pages: &[u64],
    tail: u64,
    alloc: &PageAllocator,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
) -> Result<()> {
    if ids.len().div_ceil(IDS_PER_PAGE) > pages.len() {
        bail_coded!(ErrorCode::Int049, "free-list pages too few for their ids");
    }
    for (i, &page_id) in pages.iter().enumerate() {
        let chunk = ids
            .get(i.saturating_mul(IDS_PER_PAGE)..)
            .unwrap_or(&[])
            .iter()
            .take(IDS_PER_PAGE)
            .copied()
            .collect::<Vec<u64>>();
        let count = u16::try_from(chunk.len())
            .map_err(|_| err_coded!(ErrorCode::Int048, "free-list page count exceeds u16"))?;
        let mut page = new_page(PAGE_TYPE_FREELIST, count);
        let next = pages.get(i.saturating_add(1)).copied().unwrap_or(tail);
        if let Some(dst) = page.get_mut(NEXT_OFFSET..IDS_OFFSET) {
            dst.copy_from_slice(&next.to_le_bytes());
        }
        for (j, id) in chunk.iter().enumerate() {
            let off = IDS_OFFSET.saturating_add(j.saturating_mul(8));
            if let Some(dst) = page.get_mut(off..off.saturating_add(8)) {
                dst.copy_from_slice(&id.to_le_bytes());
            }
        }
        alloc.write(backend, cache, page_id, page)?;
    }
    Ok(())
}

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
    let mut pages = Vec::with_capacity(ids.len().div_ceil(IDS_PER_PAGE));
    for _ in 0..ids.len().div_ceil(IDS_PER_PAGE) {
        pages.push(alloc.alloc(&*backend, cache)?);
    }
    write_pages(ids, &pages, 0, alloc, backend, cache)?;
    Ok((pages.first().copied().unwrap_or(0), pages))
}

/// Read chain page `id`: its free ids and the next page id. A page id or free
/// id below 2 or at or past `page_count` is STG-035.
pub fn read_page(
    id: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
    page_count: u64,
) -> Result<(Vec<u64>, u64)> {
    if id < 2 || id >= page_count {
        bail_coded!(ErrorCode::Stg035, format!("chain page {id} out of range"));
    }
    let page = cache.get_or_load(id, backend)?;
    if page.first().copied() != Some(PAGE_TYPE_FREELIST) {
        bail_coded!(ErrorCode::Stg013, id);
    }
    let count = usize::from(crate::storage::page::page_count_field(&page)?);
    if count > IDS_PER_PAGE {
        bail_coded!(ErrorCode::Stg035, format!("page {id} count {count}"));
    }
    let mut ids = Vec::with_capacity(count);
    for j in 0..count {
        let free = read_u64(&page, IDS_OFFSET.saturating_add(j.saturating_mul(8)))?;
        if free < 2 || free >= page_count {
            bail_coded!(ErrorCode::Stg035, format!("free id {free} out of range"));
        }
        ids.push(free);
    }
    Ok((ids, read_u64(&page, NEXT_OFFSET)?))
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
        // A chain longer than the file has pages must loop.
        if u64::try_from(chain.len()).unwrap_or(u64::MAX) >= page_count {
            bail_coded!(ErrorCode::Stg035, "chain loops");
        }
        let (page_ids, after) = read_page(next, backend, cache, page_count)?;
        chain.push(next);
        ids.extend(page_ids);
        next = after;
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

    /// Pop `take` ids from a chain of `n` free ids, free `freed` more pages,
    /// and push. Space is conserved, nothing freed is handed out, unread chain
    /// pages are shared, and the count matches the new chain.
    fn pop_then_push(n: u64, take: usize, freed_n: u64) {
        use std::collections::BTreeSet;
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        // Free ids 2..2+n; old chain pages appended after them, in generation 1.
        let old_ids: Vec<u64> = (2..2 + n).collect();
        let mut alloc1 = PageAllocator::new(Vec::new(), 2 + n, 1);
        let (head, old_chain) = write_chain(&old_ids, &mut alloc1, &mut backend, &cache).unwrap();
        let page_count = alloc1.next_append();
        // Pages "freed" by generation 2: ids past the file, standing for tree pages.
        let freed: Vec<u64> = (page_count..page_count + freed_n).collect();
        let bound = page_count + freed_n;

        let mut alloc = PageAllocator::from_chain(head, n, bound, 2);
        let handed: Vec<u64> = (0..take)
            .map(|_| alloc.alloc(&backend, &cache).unwrap())
            .collect();
        let (new_head, count) = alloc
            .finish_free_list(freed.clone(), &mut backend, &cache)
            .unwrap();
        let (new_ids, new_chain) =
            read_chain(new_head, &backend, &cache, alloc.next_append()).unwrap();

        assert_eq!(count, new_ids.len() as u64, "count matches the chain");
        let set = |v: &[u64]| v.iter().copied().collect::<BTreeSet<u64>>();
        let (h, ni, nc) = (set(&handed), set(&new_ids), set(&new_chain));
        assert_eq!(h.len(), handed.len(), "no id handed out twice");
        assert_eq!(ni.len(), new_ids.len(), "no duplicate free id");
        assert!(h.is_disjoint(&ni) && h.is_disjoint(&nc) && ni.is_disjoint(&nc));
        assert!(
            h.is_disjoint(&set(&freed)),
            "freed pages are not reused this generation"
        );
        let before: BTreeSet<u64> = set(&old_ids)
            .union(&set(&old_chain))
            .chain(freed.iter())
            .copied()
            .collect();
        let appended: BTreeSet<u64> = (bound..alloc.next_append()).collect();
        let after: BTreeSet<u64> = h.union(&ni).chain(nc.iter()).copied().collect();
        let expected: BTreeSet<u64> = before.union(&appended).copied().collect();
        assert!(after == expected, "every page is accounted for");
        // Chain pages never read are shared unchanged (same ids, generation 1).
        let read_pages = take.div_ceil(IDS_PER_PAGE).max(usize::from(take > 0));
        for &id in old_chain.iter().skip(read_pages + 1) {
            assert!(nc.contains(&id), "unread tail page is shared");
            let p = backend.read_page(id).unwrap();
            assert_eq!(crate::storage::page::page_generation(&p).unwrap(), 1);
        }
    }

    #[test]
    fn lazy_pop_and_push_conserve_space() {
        for (n, take, freed) in [
            (0, 0, 0),
            (0, 5, 3),
            (1, 1, 0),
            (10, 3, 2),
            (508, 508, 0),
            (509, 508, 1),
            (1200, 600, 10),
            (1200, 1300, 700),
            (5000, 1, 0),
            (5000, 2000, 2000),
        ] {
            pop_then_push(n, take, freed);
        }
    }

    #[test]
    fn untouched_chain_reads_no_page() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let ids: Vec<u64> = (2..3002).collect();
        let mut a1 = PageAllocator::new(Vec::new(), 3002, 1);
        let (head, chain) = write_chain(&ids, &mut a1, &mut backend, &cache).unwrap();
        // No allocation and nothing freed: the list is unchanged.
        let mut alloc = PageAllocator::from_chain(head, 3000, a1.next_append(), 2);
        let (h, count) = alloc
            .finish_free_list(Vec::new(), &mut backend, &cache)
            .unwrap();
        assert_eq!((h, count), (head, 3000));
        assert_eq!(alloc.next_append(), a1.next_append(), "nothing written");
        assert_eq!(chain.len(), 6);
    }

    #[test]
    fn wrong_count_or_looping_chain_is_stg_035() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let mut a1 = PageAllocator::new(Vec::new(), 12, 1);
        let (head, _) = write_chain(
            &(2..12).collect::<Vec<u64>>(),
            &mut a1,
            &mut backend,
            &cache,
        )
        .unwrap();
        let mut alloc = PageAllocator::from_chain(head, 3, a1.next_append(), 2);
        let err = (0..20)
            .map(|_| alloc.alloc(&backend, &cache))
            .find_map(Result::err)
            .unwrap();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-035");
    }
}
