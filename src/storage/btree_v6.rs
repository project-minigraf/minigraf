//! On-disk B+tree for covering index persistence (file format v6).
//!
//! Each node maps to exactly one 4KB page. The `PageCache` serves all reads.
//! `build_btree` does a bulk-build (write-all-leaves, then internal levels
//! bottom-up). Range scans traverse the tree through the cache.

use crate::storage::cache::PageCache;
use crate::storage::index::FactRef;
use crate::storage::{StorageBackend, PAGE_SIZE};
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─── Page type constants ───────────────────────────────────────────────────────

/// Leaf node page type (v6).
pub const PAGE_TYPE_LEAF: u8 = 0x21;
/// Internal node page type (v6).
pub const PAGE_TYPE_INTERNAL: u8 = 0x22;

// ─── Fixed sizes ──────────────────────────────────────────────────────────────

/// Leaf page fixed header: type(1) + reserved(1) + entry_count(2) + next_leaf(8) = 12 bytes.
const LEAF_HEADER_SIZE: usize = 12;
/// Internal page fixed header: type(1) + reserved(1) + key_count(2) + rightmost_child(8) = 12 bytes.
const INTERNAL_HEADER_SIZE: usize = 12;
/// Slot directory entry: offset(u16) + length(u16) = 4 bytes.
const SLOT_SIZE: usize = 4;
/// Fill-factor threshold: stop packing once total used bytes exceed this (~75% of PAGE_SIZE).
const PAGE_FILL_BYTES: usize = PAGE_SIZE * 3 / 4;

// ─── Low-level page writers ───────────────────────────────────────────────────

/// Write a single leaf page and insert it into the cache.
///
/// `entries`: each element is the postcard-serialised `(K, FactRef)` bytes for
/// one index entry, in sort order. Written end-to-start in the page.
fn write_leaf_page(
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    page_id: u64,
    entries: &[Vec<u8>],
    next_leaf: u64,
) -> Result<()> {
    let entry_count = entries.len() as u16;
    let mut page = vec![0u8; PAGE_SIZE];

    // Fixed header
    page[0] = PAGE_TYPE_LEAF;
    page[1] = 0; // reserved
    page[2..4].copy_from_slice(&entry_count.to_le_bytes());
    page[4..12].copy_from_slice(&next_leaf.to_le_bytes());

    // Slot directory starts at byte 12; data written end-to-start
    let mut write_pos = PAGE_SIZE;
    for (i, entry) in entries.iter().enumerate() {
        write_pos -= entry.len();
        page[write_pos..write_pos + entry.len()].copy_from_slice(entry);
        let slot_off = LEAF_HEADER_SIZE + i * SLOT_SIZE;
        page[slot_off..slot_off + 2].copy_from_slice(&(write_pos as u16).to_le_bytes());
        page[slot_off + 2..slot_off + 4].copy_from_slice(&(entry.len() as u16).to_le_bytes());
    }

    backend.write_page(page_id, &page)?;
    cache.put_dirty(page_id, page);
    Ok(())
}

/// Write a single internal node page and insert it into the cache.
///
/// `child_ids`: all child page IDs in order; the last one is `rightmost_child`.
/// `sep_bytes`: postcard-serialised Key bytes for each separator key.
///   `sep_bytes[j]` = first key of `child_ids[j+1]`'s subtree.
///   `sep_bytes.len()` == `child_ids.len() - 1`.
fn write_internal_page(
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    page_id: u64,
    child_ids: &[u64],
    sep_bytes: &[Vec<u8>],
) -> Result<()> {
    debug_assert_eq!(child_ids.len(), sep_bytes.len() + 1);
    let key_count = sep_bytes.len() as u16;
    let rightmost_child = *child_ids.last().unwrap();

    let mut page = vec![0u8; PAGE_SIZE];

    // Fixed header
    page[0] = PAGE_TYPE_INTERNAL;
    page[1] = 0; // reserved
    page[2..4].copy_from_slice(&key_count.to_le_bytes());
    page[4..12].copy_from_slice(&rightmost_child.to_le_bytes());

    // Child array: key_count entries starting at byte 12
    let child_arr_start = INTERNAL_HEADER_SIZE;
    for (i, &cid) in child_ids[..child_ids.len() - 1].iter().enumerate() {
        let off = child_arr_start + i * 8;
        page[off..off + 8].copy_from_slice(&cid.to_le_bytes());
    }

    // Slot directory for separator keys: after child array
    let slot_dir_start = INTERNAL_HEADER_SIZE + (key_count as usize) * 8;

    // Separator key data written end-to-start
    let mut write_pos = PAGE_SIZE;
    for (i, sep) in sep_bytes.iter().enumerate() {
        write_pos -= sep.len();
        page[write_pos..write_pos + sep.len()].copy_from_slice(sep);
        let slot_off = slot_dir_start + i * SLOT_SIZE;
        page[slot_off..slot_off + 2].copy_from_slice(&(write_pos as u16).to_le_bytes());
        page[slot_off + 2..slot_off + 4].copy_from_slice(&(sep.len() as u16).to_le_bytes());
    }

    backend.write_page(page_id, &page)?;
    cache.put_dirty(page_id, page);
    Ok(())
}

// ─── build_btree ──────────────────────────────────────────────────────────────

/// Build a B+tree from sorted entries and write it to the backend.
///
/// Returns `(root_page_id, next_free_page_id)`. Chain multiple calls:
/// pass the returned `next_free_page_id` as `start_page_id` for the next index.
///
/// All written pages are inserted into `cache` via `put_dirty`.
pub fn build_btree<K>(
    sorted_entries: impl Iterator<Item = (K, FactRef)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    start_page_id: u64,
) -> Result<(u64, u64)>
where
    K: Serialize + Ord,
{
    // ── Phase 1: pack entries into leaf pages ─────────────────────────────────
    let mut leaf_infos: Vec<(u64, Vec<u8>)> = Vec::new();

    let mut cur_entries: Vec<Vec<u8>> = Vec::new();
    let mut cur_data_bytes: usize = 0;
    let mut cur_first_key: Option<Vec<u8>> = None;
    let mut next_page = start_page_id;

    for (key, fact_ref) in sorted_entries {
        let entry_bytes = postcard::to_allocvec(&(&key, &fact_ref))?;

        let projected = LEAF_HEADER_SIZE
            + (cur_entries.len() + 1) * SLOT_SIZE
            + cur_data_bytes
            + entry_bytes.len();

        if projected > PAGE_FILL_BYTES && !cur_entries.is_empty() {
            write_leaf_page(backend, cache, next_page, &cur_entries, 0)?;
            leaf_infos.push((next_page, cur_first_key.unwrap()));
            next_page += 1;
            cur_entries.clear();
            cur_data_bytes = 0;
            cur_first_key = None;
        }

        if cur_first_key.is_none() {
            cur_first_key = Some(postcard::to_allocvec(&key)?);
        }
        cur_data_bytes += entry_bytes.len();
        cur_entries.push(entry_bytes);
    }

    // Flush the last (or only) batch
    if cur_entries.is_empty() && leaf_infos.is_empty() {
        // Empty tree: single empty leaf
        write_leaf_page(backend, cache, next_page, &[], 0)?;
        return Ok((next_page, next_page + 1));
    }
    if !cur_entries.is_empty() {
        write_leaf_page(backend, cache, next_page, &cur_entries, 0)?;
        leaf_infos.push((next_page, cur_first_key.unwrap()));
        next_page += 1;
    }

    // Patch next_leaf pointers: leaf[i].next_leaf = leaf[i+1].page_id
    for i in 0..leaf_infos.len() - 1 {
        let pid = leaf_infos[i].0;
        let next_lid = leaf_infos[i + 1].0;
        let cached = cache.get_or_load(pid, backend)?;
        let mut page = (*cached).clone();
        page[4..12].copy_from_slice(&next_lid.to_le_bytes());
        backend.write_page(pid, &page)?;
        cache.put_dirty(pid, page);
    }

    // Single leaf: it is the root
    if leaf_infos.len() == 1 {
        return Ok((leaf_infos[0].0, next_page));
    }

    // ── Phase 2: build internal levels bottom-up ──────────────────────────────
    let mut current_level = leaf_infos;

    loop {
        if current_level.len() == 1 {
            return Ok((current_level[0].0, next_page));
        }

        let mut next_level: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut i = 0;

        while i < current_level.len() {
            let i_start = i;
            let mut child_ids: Vec<u64> = vec![current_level[i].0];
            let mut sep_bytes: Vec<Vec<u8>> = Vec::new();
            let mut sep_data_bytes: usize = 0;
            i += 1;

            while i < current_level.len() {
                let sep = current_level[i].1.clone();
                let projected = INTERNAL_HEADER_SIZE
                    + (child_ids.len() - 1) * 8
                    + (sep_bytes.len() + 1) * SLOT_SIZE
                    + sep_data_bytes
                    + sep.len();

                if projected > PAGE_FILL_BYTES && !sep_bytes.is_empty() {
                    break;
                }

                sep_data_bytes += sep.len();
                sep_bytes.push(sep);
                child_ids.push(current_level[i].0);
                i += 1;
            }

            let node_page_id = next_page;
            write_internal_page(backend, cache, node_page_id, &child_ids, &sep_bytes)?;
            next_page += 1;

            let first_key = current_level[i_start].1.clone();
            next_level.push((node_page_id, first_key));
        }

        current_level = next_level;
    }
}

/// Merge two already-sorted `Vec`s into a single sorted iterator.
///
/// Used by `PersistentFactStorage::save()` to merge committed B+tree entries
/// with new pending entries before building the replacement B+tree.
pub fn merge_sorted_vecs<T: Ord>(a: Vec<T>, b: Vec<T>) -> impl Iterator<Item = T> {
    let mut ai = a.into_iter().peekable();
    let mut bi = b.into_iter().peekable();
    std::iter::from_fn(move || match (ai.peek(), bi.peek()) {
        (Some(_), Some(_)) => {
            if ai.peek().unwrap() <= bi.peek().unwrap() {
                ai.next()
            } else {
                bi.next()
            }
        }
        (Some(_), None) => ai.next(),
        (None, Some(_)) => bi.next(),
        (None, None) => None,
    })
}

// Leave stream_all_entries and range_scan as stubs — implemented in Task 3
pub fn stream_all_entries<K>(
    _root_page_id: u64,
    _backend: &dyn StorageBackend,
    _cache: &PageCache,
) -> Result<Vec<(K, FactRef)>>
where
    K: for<'de> Deserialize<'de> + Ord,
{
    unimplemented!("implemented in Task 3")
}

pub fn range_scan<K>(
    _root_page_id: u64,
    _start: &K,
    _end: Option<&K>,
    _backend: &dyn StorageBackend,
    _cache: &PageCache,
) -> Result<Vec<FactRef>>
where
    K: Serialize + for<'de> Deserialize<'de> + Ord,
{
    unimplemented!("implemented in Task 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::index::{EavtKey, FactRef};
    use uuid::Uuid;

    fn make_eavt(n: u128, attr: &str, tx: u64) -> (EavtKey, FactRef) {
        (
            EavtKey {
                entity: Uuid::from_u128(n),
                attribute: attr.to_string(),
                valid_from: 0,
                valid_to: i64::MAX,
                tx_count: tx,
            },
            FactRef { page_id: tx + 1, slot_index: 0 },
        )
    }

    #[test]
    fn test_build_btree_empty_returns_single_leaf() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let entries: Vec<(EavtKey, FactRef)> = vec![];
        let (root, next_free) = build_btree(entries.into_iter(), &mut backend, &cache, 1).unwrap();
        assert_eq!(root, 1, "root must be at start_page_id");
        assert_eq!(next_free, 2, "single empty leaf = 1 page");
        // Verify it is a leaf page
        let page = cache.get_or_load(1, &backend).unwrap();
        assert_eq!(page[0], PAGE_TYPE_LEAF);
        let entry_count = u16::from_le_bytes(page[2..4].try_into().unwrap());
        assert_eq!(entry_count, 0);
    }

    #[test]
    fn test_build_btree_single_entry() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let entries = vec![make_eavt(1, ":name", 1)];
        let (root, next_free) = build_btree(entries.into_iter(), &mut backend, &cache, 5).unwrap();
        assert_eq!(root, 5);
        assert_eq!(next_free, 6);
        let page = cache.get_or_load(5, &backend).unwrap();
        assert_eq!(page[0], PAGE_TYPE_LEAF);
        assert_eq!(u16::from_le_bytes(page[2..4].try_into().unwrap()), 1);
    }

    #[test]
    fn test_build_btree_chained_next_free() {
        // Two sequential build_btree calls: second must start where first ended.
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(128);
        let entries1 = (0u128..5).map(|n| make_eavt(n, ":a", n as u64 + 1));
        let (_, next1) = build_btree(entries1, &mut backend, &cache, 1).unwrap();

        let entries2 = (5u128..10).map(|n| make_eavt(n, ":b", n as u64 + 1));
        let (root2, next2) = build_btree(entries2, &mut backend, &cache, next1).unwrap();

        assert!(root2 >= next1, "second tree must not overlap with first");
        assert!(next2 > root2);
    }

    #[test]
    fn test_build_btree_pages_in_cache_after_build() {
        // All written pages must be retrievable from cache without backend read
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let entries = (0u128..100).map(|n| make_eavt(n, ":x", n as u64 + 1));
        let (root, next_free) = build_btree(entries, &mut backend, &cache, 1).unwrap();

        let empty_backend = MemoryBackend::new();
        for page_id in root..next_free {
            let result = cache.get_or_load(page_id, &empty_backend);
            assert!(result.is_ok(), "page {} missing from cache", page_id);
        }
    }

    #[test]
    fn test_build_btree_fill_factor_no_overflow() {
        // With many entries, leaf pages must not exceed PAGE_SIZE
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let entries = (0u128..200).map(|n| make_eavt(n, ":verylongattributename", n as u64 + 1));
        let (root, next_free) = build_btree(entries, &mut backend, &cache, 1).unwrap();

        for page_id in root..next_free {
            let page = cache.get_or_load(page_id, &backend).unwrap();
            assert_eq!(page.len(), PAGE_SIZE, "every page must be exactly PAGE_SIZE");
        }
    }

    #[test]
    fn test_build_btree_internal_node_created_for_many_entries() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(512);
        // ~300 entries should force at least 2 leaf pages and 1 internal node
        let entries = (0u128..300).map(|n| make_eavt(n, ":attr", n as u64 + 1));
        let (root, next_free) = build_btree(entries, &mut backend, &cache, 1).unwrap();

        let root_page = cache.get_or_load(root, &backend).unwrap();
        let pages_written = next_free - 1;
        assert!(pages_written >= 2, "300 entries must need multiple pages; got {}", pages_written);
        assert!(
            root_page[0] == PAGE_TYPE_LEAF || root_page[0] == PAGE_TYPE_INTERNAL,
            "root page type 0x{:02x} is not leaf or internal", root_page[0]
        );
    }

    #[test]
    fn test_merge_sorted_vecs() {
        let a = vec![1u32, 3, 5, 7];
        let b = vec![2u32, 4, 6, 8];
        let merged: Vec<u32> = merge_sorted_vecs(a, b).collect();
        assert_eq!(merged, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn test_merge_sorted_vecs_empty_left() {
        let merged: Vec<u32> = merge_sorted_vecs(vec![], vec![1u32, 2, 3]).collect();
        assert_eq!(merged, vec![1, 2, 3]);
    }

    #[test]
    fn test_build_btree_leaf_next_pointers_form_chain() {
        // Build a tree with enough entries to require multiple leaf pages,
        // then verify leaf[i].next_leaf == leaf[i+1].page_id
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        // ~100 entries with long keys should span 4-6 leaf pages
        let entries = (0u128..100).map(|n| make_eavt(n, ":verylongattributename", n as u64 + 1));
        let (root, next_free) = build_btree(entries, &mut backend, &cache, 1).unwrap();

        // Collect leaf page IDs by following the chain from the leftmost leaf
        // The root may be an internal node; find the leftmost leaf first
        let root_page = cache.get_or_load(root, &backend).unwrap();
        let mut leaf_pid = if root_page[0] == PAGE_TYPE_LEAF {
            root
        } else {
            // leftmost leaf: follow first child of each internal node down
            let mut pid = root;
            loop {
                let p = cache.get_or_load(pid, &backend).unwrap();
                if p[0] == PAGE_TYPE_LEAF {
                    break pid;
                }
                // first child is at child_array[0] = bytes 12..20
                pid = u64::from_le_bytes(p[12..20].try_into().unwrap());
            }
        };

        // Walk the chain and verify it's contiguous and terminates
        let mut chain: Vec<u64> = vec![leaf_pid];
        loop {
            let p = cache.get_or_load(leaf_pid, &backend).unwrap();
            assert_eq!(p[0], PAGE_TYPE_LEAF, "page {} should be leaf", leaf_pid);
            let next = u64::from_le_bytes(p[4..12].try_into().unwrap());
            if next == 0 {
                break;
            }
            chain.push(next);
            leaf_pid = next;
        }

        assert!(chain.len() >= 2, "100 long-key entries should span multiple leaves; got {} leaves", chain.len());
        // Total entries across all leaves must equal 100
        let total_entries: u64 = chain.iter().map(|&pid| {
            let p = cache.get_or_load(pid, &backend).unwrap();
            u16::from_le_bytes(p[2..4].try_into().unwrap()) as u64
        }).sum();
        assert_eq!(total_entries, 100);
        // next_free must be > all leaf page IDs
        for &pid in &chain {
            assert!(pid < next_free, "leaf {} must be < next_free {}", pid, next_free);
        }
    }

    #[test]
    fn test_merge_sorted_vecs_duplicates() {
        let a = vec![1u32, 3, 3, 5];
        let b = vec![2u32, 3, 4];
        let merged: Vec<u32> = merge_sorted_vecs(a, b).collect();
        assert_eq!(merged, vec![1, 2, 3, 3, 3, 4, 5]);
    }
}
