//! Common header of every non-meta page in a format v8 file (spec §4.2).
//!
//! ```text
//! 0       page_type  u8
//! 1       reserved   u8 (0)
//! 2..4    count      u16 (entries, keys, records or ids, by type)
//! 4..8    crc32      u32 over the full page with this field zeroed
//! 8..16   page_id    u64, the page's own id
//! 16..24  generation u64, the checkpoint generation that wrote it
//! ```
//!
//! Type values: the high nibble is the page family, the low nibble the variant.
//! Values used by earlier formats are retired and never reassigned: 0x02/0x03
//! (v5–v7 fact pages), 0x11 (v5 index pages), 0x21/0x22 (v6/v7 B+tree).

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::PAGE_SIZE;
use anyhow::Result;

/// Size of the common page header.
pub const PAGE_HEADER_SIZE: usize = 24;

// 0x41 was the interim fact page of pre-release v8 builds. It is retired and
// never reused: such a page fails the known-type check.
/// Long-value page (spec §6.3).
pub const PAGE_TYPE_VALUE: u8 = 0x51;
/// Value overflow page. Reserved, never written.
#[allow(dead_code)]
pub const PAGE_TYPE_VALUE_OVERFLOW: u8 = 0x52;
/// B+tree leaf node.
pub const PAGE_TYPE_LEAF: u8 = 0x61;
/// B+tree internal node.
pub const PAGE_TYPE_INTERNAL: u8 = 0x62;
/// Free-list page.
pub const PAGE_TYPE_FREELIST: u8 = 0x81;

const CRC_RANGE: std::ops::Range<usize> = 4..8;
const ID_RANGE: std::ops::Range<usize> = 8..16;
const GEN_RANGE: std::ops::Range<usize> = 16..24;

fn is_known_type(t: u8) -> bool {
    matches!(
        t,
        PAGE_TYPE_VALUE
            | PAGE_TYPE_VALUE_OVERFLOW
            | PAGE_TYPE_LEAF
            | PAGE_TYPE_INTERNAL
            | PAGE_TYPE_FREELIST
    )
}

fn field_u64(page: &[u8], range: std::ops::Range<usize>) -> Result<u64> {
    let bytes = page
        .get(range)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "page shorter than its header"))?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
        err_coded!(ErrorCode::Int049, "page header field is not 8 bytes")
    })?))
}

/// CRC32 of `page` with the CRC field treated as zero.
fn page_crc(page: &[u8]) -> Result<u32> {
    let (head, rest) = (page.get(..CRC_RANGE.start), page.get(CRC_RANGE.end..));
    let (head, rest) = head
        .zip(rest)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "page shorter than its header"))?;
    let mut h = crc32fast::Hasher::new();
    h.update(head);
    h.update(&[0u8; 4]);
    h.update(rest);
    Ok(h.finalize())
}

/// The type byte of a page.
pub fn page_type(page: &[u8]) -> Option<u8> {
    page.first().copied()
}

/// The `count` field of a page.
pub fn page_count_field(page: &[u8]) -> Result<u16> {
    let bytes = page
        .get(2..4)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "page shorter than its header"))?;
    Ok(u16::from_le_bytes([
        *bytes.first().unwrap_or(&0),
        *bytes.get(1).unwrap_or(&0),
    ]))
}

/// The generation recorded in a page header.
pub fn page_generation(page: &[u8]) -> Result<u64> {
    field_u64(page, GEN_RANGE)
}

/// Write `page_id` and `generation` into the header, then the CRC over the page.
///
/// The caller has already written the type and count bytes and the body.
pub fn seal(page: &mut [u8], page_id: u64, generation: u64) -> Result<()> {
    if page.len() != PAGE_SIZE {
        bail_coded!(ErrorCode::Int051, page.len(), PAGE_SIZE);
    }
    let too_short = || err_coded!(ErrorCode::Int049, "page shorter than its header");
    page.get_mut(ID_RANGE)
        .ok_or_else(too_short)?
        .copy_from_slice(&page_id.to_le_bytes());
    page.get_mut(GEN_RANGE)
        .ok_or_else(too_short)?
        .copy_from_slice(&generation.to_le_bytes());
    let crc = page_crc(page)?;
    page.get_mut(CRC_RANGE)
        .ok_or_else(too_short)?
        .copy_from_slice(&crc.to_le_bytes());
    Ok(())
}

/// Run the read checks of spec §4.2 (1–4) and return the page type.
///
/// The type check runs first, so a legacy or foreign page fails with STG-013
/// before any CRC is computed. Checking the specific expected type (check 5)
/// is left to the caller.
pub fn verify(page: &[u8], expected_id: u64, max_generation: u64) -> Result<u8> {
    let t = match page_type(page) {
        Some(t) if is_known_type(t) && page.len() == PAGE_SIZE => t,
        _ => bail_coded!(ErrorCode::Stg013, expected_id),
    };
    let stored = page
        .get(CRC_RANGE)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "page shorter than its header"))?;
    if stored != page_crc(page)? {
        bail_coded!(ErrorCode::Stg029, expected_id);
    }
    let id = field_u64(page, ID_RANGE)?;
    if id != expected_id {
        bail_coded!(ErrorCode::Stg030, expected_id, id);
    }
    let generation = field_u64(page, GEN_RANGE)?;
    if generation > max_generation {
        bail_coded!(ErrorCode::Stg031, expected_id, generation, max_generation);
    }
    Ok(t)
}

/// A zeroed page with the type and count bytes set.
pub fn new_page(page_type: u8, count: u16) -> Vec<u8> {
    let mut page = vec![0u8; PAGE_SIZE];
    if let Some(b) = page.first_mut() {
        *b = page_type;
    }
    if let Some(c) = page.get_mut(2..4) {
        c.copy_from_slice(&count.to_le_bytes());
    }
    page
}

/// Hands out page ids for one checkpoint generation and writes sealed pages.
///
/// `alloc` takes an id from the active meta's free list first, then appends at
/// the high-water mark; `alloc_append` always appends. Every id it hands out is
/// unreferenced by the active meta (spec §8.1), so writing it can never damage
/// the committed state.
///
/// The free list comes either as a list of ids ([`PageAllocator::new`]) or as
/// the meta's on-disk chain ([`PageAllocator::from_chain`]), whose pages are
/// read one at a time, only when another id is needed (spec §4.4). A chain page
/// that has been read is replaced: [`PageAllocator::finish_free_list`] pushes it,
/// its unused ids and every freed page as new head pages in front of the unread
/// tail, which is shared unchanged.
pub struct PageAllocator {
    /// Free ids ready to hand out, in reverse order (`pop` yields the next).
    free: Vec<u64>,
    /// First chain page not read yet (0: none).
    chain_next: u64,
    /// Ids in the chain pages not read yet.
    chain_tail_count: u64,
    /// Chain pages read so far; each is free once the new meta commits.
    chain_read: Vec<u64>,
    /// Bound for chain page ids (the active meta's `page_count`).
    chain_bound: u64,
    next_append: u64,
    generation: u64,
}

impl PageAllocator {
    /// An allocator over `free` (any order, lowest handed out first) that
    /// appends from `next_append`.
    pub fn new(mut free: Vec<u64>, next_append: u64, generation: u64) -> Self {
        free.sort_unstable_by(|a, b| b.cmp(a));
        PageAllocator {
            free,
            chain_next: 0,
            chain_tail_count: 0,
            chain_read: Vec::new(),
            chain_bound: next_append,
            next_append,
            generation,
        }
    }

    /// An allocator over the free-list chain at `head` holding `count` ids, in a
    /// file whose committed `page_count` is `next_append`.
    pub fn from_chain(head: u64, count: u64, next_append: u64, generation: u64) -> Self {
        PageAllocator {
            free: Vec::new(),
            chain_next: head,
            chain_tail_count: count,
            chain_read: Vec::new(),
            chain_bound: next_append,
            next_append,
            generation,
        }
    }

    /// The generation every page written through this allocator is stamped with.
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The high-water mark: one past the highest id handed out or inherited.
    pub fn next_append(&self) -> u64 {
        self.next_append
    }

    /// A free id if any is left, otherwise a new one at the end. Reads the next
    /// free-list chain page when the ids read so far are used up.
    pub fn alloc(
        &mut self,
        backend: &dyn crate::storage::StorageBackend,
        cache: &crate::storage::cache::PageCache,
    ) -> Result<u64> {
        loop {
            if let Some(id) = self.free.pop() {
                return Ok(id);
            }
            if self.chain_next == 0 {
                return self.alloc_append();
            }
            if u64::try_from(self.chain_read.len()).unwrap_or(u64::MAX) >= self.chain_bound {
                bail_coded!(ErrorCode::Stg035, "chain loops");
            }
            let id = self.chain_next;
            let (mut ids, next) =
                crate::storage::freelist::read_page(id, backend, cache, self.chain_bound)?;
            self.chain_tail_count = self
                .chain_tail_count
                .checked_sub(u64::try_from(ids.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| err_coded!(ErrorCode::Stg035, "freelist_count too small"))?;
            self.chain_read.push(id);
            self.chain_next = next;
            ids.reverse();
            self.free = ids;
        }
    }

    /// A new id at the end of the file.
    pub fn alloc_append(&mut self) -> Result<u64> {
        let id = self.next_append;
        self.next_append = id
            .checked_add(1)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "page id overflow"))?;
        Ok(id)
    }

    /// Free ids not handed out, in hand-out order. Chain pages not read yet are
    /// not included.
    #[cfg(test)]
    pub fn take_unused_free(&mut self) -> Vec<u64> {
        let mut v = std::mem::take(&mut self.free);
        v.reverse();
        v
    }

    /// Write the new free list and return `(head, count)`.
    ///
    /// The list is `freed` (pages the new meta no longer references), every
    /// chain page read and the ids read but not handed out, as new head pages in
    /// front of the unread tail. The head pages themselves are allocated here,
    /// which may read more of the chain, so this repeats until the page count
    /// covers the ids. Nothing listed here is handed out in this generation.
    pub fn finish_free_list(
        &mut self,
        freed: Vec<u64>,
        backend: &mut dyn crate::storage::StorageBackend,
        cache: &crate::storage::cache::PageCache,
    ) -> Result<(u64, u64)> {
        use crate::storage::freelist::IDS_PER_PAGE;
        let mut pages: Vec<u64> = Vec::new();
        let ids = loop {
            let mut ids = freed.clone();
            ids.extend(self.chain_read.iter().copied());
            ids.extend(self.free.iter().rev().copied());
            // Use the first `k` allocated pages for the chain and list the rest
            // as free: the smallest `k` whose pages hold every listed id.
            let n = pages.len();
            let mut k = 0usize;
            while k <= n && (ids.len() + n - k).div_ceil(IDS_PER_PAGE) > k {
                k += 1;
            }
            if k > n {
                pages.push(self.alloc(&*backend, cache)?);
                continue;
            }
            ids.extend(pages.get(k..).unwrap_or(&[]).iter().copied());
            pages.truncate(k);
            break ids;
        };
        let tail = self.chain_next;
        let count = self
            .chain_tail_count
            .checked_add(u64::try_from(ids.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "free-list count overflow"))?;
        crate::storage::freelist::write_pages(&ids, &pages, tail, self, backend, cache)?;
        Ok((pages.first().copied().unwrap_or(tail), count))
    }

    /// Seal `page` as `page_id` in this generation, write it, and put it in the
    /// cache so that a reused id never serves stale bytes.
    pub fn write(
        &self,
        backend: &mut dyn crate::storage::StorageBackend,
        cache: &crate::storage::cache::PageCache,
        page_id: u64,
        mut page: Vec<u8>,
    ) -> Result<()> {
        seal(&mut page, page_id, self.generation)?;
        backend.write_page(page_id, &page)?;
        cache.put_dirty(page_id, page);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(e: anyhow::Error) -> &'static str {
        crate::error::MinigrafError::from(e).code()
    }

    fn sealed(id: u64, generation: u64) -> Vec<u8> {
        let mut p = new_page(PAGE_TYPE_LEAF, 3);
        if let Some(b) = p.get_mut(100) {
            *b = 0xAB;
        }
        seal(&mut p, id, generation).unwrap();
        p
    }

    #[test]
    fn seal_then_verify_round_trips() {
        let p = sealed(7, 3);
        assert_eq!(verify(&p, 7, 3).unwrap(), PAGE_TYPE_LEAF);
        assert_eq!(page_count_field(&p).unwrap(), 3);
        assert_eq!(page_generation(&p).unwrap(), 3);
    }

    #[test]
    fn flipped_body_bit_is_stg_029() {
        let mut p = sealed(7, 3);
        if let Some(b) = p.get_mut(4000) {
            *b ^= 1;
        }
        assert_eq!(code_of(verify(&p, 7, 3).unwrap_err()), "STG-029");
    }

    #[test]
    fn wrong_page_id_is_stg_030() {
        let p = sealed(7, 3);
        assert_eq!(code_of(verify(&p, 8, 3).unwrap_err()), "STG-030");
    }

    #[test]
    fn future_generation_is_stg_031() {
        let p = sealed(7, 4);
        assert_eq!(code_of(verify(&p, 7, 3).unwrap_err()), "STG-031");
    }

    #[test]
    fn legacy_type_fails_before_crc() {
        // A v7 B+tree leaf (0x21) with garbage where the CRC would be.
        let mut p = vec![0u8; PAGE_SIZE];
        p[0] = 0x21;
        p[5] = 0x99;
        assert_eq!(code_of(verify(&p, 7, 3).unwrap_err()), "STG-013");
        for retired in [0x02u8, 0x03, 0x11, 0x22, 0x00] {
            p[0] = retired;
            assert_eq!(code_of(verify(&p, 7, 3).unwrap_err()), "STG-013");
        }
    }

    #[test]
    fn short_page_is_rejected() {
        let p = vec![PAGE_TYPE_LEAF; 10];
        assert!(verify(&p, 0, 0).is_err());
        let mut q = vec![0u8; 10];
        assert!(seal(&mut q, 0, 0).is_err());
    }

    #[test]
    fn allocator_takes_lowest_free_then_appends() {
        let b = crate::storage::backend::MemoryBackend::new();
        let c = crate::storage::cache::PageCache::new(0);
        let mut a = PageAllocator::new(vec![9, 4, 7], 20, 2);
        assert_eq!(a.alloc(&b, &c).unwrap(), 4);
        assert_eq!(a.alloc_append().unwrap(), 20);
        assert_eq!(a.alloc(&b, &c).unwrap(), 7);
        assert_eq!(a.take_unused_free(), vec![9]);
        assert_eq!(a.alloc(&b, &c).unwrap(), 21);
        assert_eq!(a.next_append(), 22);
        assert_eq!(a.generation(), 2);
    }
}
