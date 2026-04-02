//! On-disk B+tree for covering index persistence (file format v6).
//!
//! Each node maps to exactly one 4KB page. The `PageCache` serves all reads.
//! `build_btree` does a bulk-build (write-all-leaves, then internal levels
//! bottom-up). Range scans traverse the tree through the cache.

use crate::storage::cache::PageCache;
use crate::storage::index::FactRef;
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::Mutex;

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

/// Serialize `(key, fact_ref)` pairs into the byte format expected by [`build_btree`].
///
/// Each item produces `(entry_bytes, key_bytes)` where:
/// - `entry_bytes` = postcard encoding of `(&key, &fact_ref)` — stored in leaf nodes
/// - `key_bytes`   = postcard encoding of `&key` alone — used as separator in internal nodes
///
/// Callers **must sort** entries before calling; this function preserves order.
/// Keeping serialisation in this small generic helper means `build_btree` itself
/// is monomorphised only once.
pub fn btree_entries<K: Serialize>(
    iter: impl Iterator<Item = (K, FactRef)>,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    iter.map(|(key, fact_ref)| {
        let entry_bytes = postcard::to_allocvec(&(&key, &fact_ref))?;
        let key_bytes = postcard::to_allocvec(&key)?;
        Ok((entry_bytes, key_bytes))
    })
    .collect()
}

/// Build a B+tree from pre-serialised sorted entries and write it to the backend.
///
/// Each item in `sorted_entries` is `(entry_bytes, key_bytes)` as produced by
/// [`btree_entries`]. Entries **must already be sorted** by key.
///
/// Returns `(root_page_id, next_free_page_id)`. Chain multiple calls:
/// pass the returned `next_free_page_id` as `start_page_id` for the next index.
///
/// All written pages are inserted into `cache` via `put_dirty`.
pub fn build_btree(
    sorted_entries: impl Iterator<Item = (Vec<u8>, Vec<u8>)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    start_page_id: u64,
) -> Result<(u64, u64)> {
    // ── Phase 1: pack entries into leaf pages ─────────────────────────────────
    let mut leaf_infos: Vec<(u64, Vec<u8>)> = Vec::new();

    let mut cur_entries: Vec<Vec<u8>> = Vec::new();
    let mut cur_data_bytes: usize = 0;
    let mut cur_first_key: Option<Vec<u8>> = None;
    let mut next_page = start_page_id;

    for (entry_bytes, key_bytes) in sorted_entries {
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
            cur_first_key = Some(key_bytes);
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

// ─── Leaf traversal helpers ───────────────────────────────────────────────────

/// Traverse internal nodes from `root` to find the leftmost (first) leaf page.
fn find_leftmost_leaf(root: u64, backend: &dyn StorageBackend, cache: &PageCache) -> Result<u64> {
    let mut page_id = root;
    loop {
        let page = cache.get_or_load(page_id, backend)?;
        match page[0] {
            PAGE_TYPE_LEAF => return Ok(page_id),
            PAGE_TYPE_INTERNAL => {
                let key_count = u16::from_le_bytes(page[2..4].try_into().unwrap()) as usize;
                if key_count == 0 {
                    page_id = u64::from_le_bytes(page[4..12].try_into().unwrap());
                } else {
                    page_id = u64::from_le_bytes(
                        page[INTERNAL_HEADER_SIZE..INTERNAL_HEADER_SIZE + 8]
                            .try_into()
                            .unwrap(),
                    );
                }
            }
            t => anyhow::bail!(
                "find_leftmost_leaf: unexpected page type 0x{:02x} at page_id={}",
                t,
                page_id
            ),
        }
    }
}

/// Traverse from `root` to the leaf that would contain `key`.
fn find_leaf_for_key<K>(
    root: u64,
    key: &K,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<u64>
where
    K: for<'de> Deserialize<'de> + Ord,
{
    let mut page_id = root;
    loop {
        let page = cache.get_or_load(page_id, backend)?;
        match page[0] {
            PAGE_TYPE_LEAF => return Ok(page_id),
            PAGE_TYPE_INTERNAL => {
                let key_count = u16::from_le_bytes(page[2..4].try_into().unwrap()) as usize;
                let rightmost_child = u64::from_le_bytes(page[4..12].try_into().unwrap());
                let child_arr_start = INTERNAL_HEADER_SIZE;
                let slot_dir_start = INTERNAL_HEADER_SIZE + key_count * 8;

                let mut descended = false;
                for i in 0..key_count {
                    let slot_off = slot_dir_start + i * SLOT_SIZE;
                    let sep_offset =
                        u16::from_le_bytes(page[slot_off..slot_off + 2].try_into().unwrap())
                            as usize;
                    let sep_length =
                        u16::from_le_bytes(page[slot_off + 2..slot_off + 4].try_into().unwrap())
                            as usize;
                    let sep_key: K =
                        postcard::from_bytes(&page[sep_offset..sep_offset + sep_length])?;

                    if *key < sep_key {
                        let child_off = child_arr_start + i * 8;
                        page_id =
                            u64::from_le_bytes(page[child_off..child_off + 8].try_into().unwrap());
                        descended = true;
                        break;
                    }
                }
                if !descended {
                    page_id = rightmost_child;
                }
            }
            t => anyhow::bail!(
                "find_leaf_for_key: unexpected page type 0x{:02x} at page_id={}",
                t,
                page_id
            ),
        }
    }
}

/// Read all `(K, FactRef)` entries from a leaf page's slot directory.
fn read_leaf_entries<K>(page: &[u8]) -> Result<Vec<(K, FactRef)>>
where
    K: for<'de> Deserialize<'de>,
{
    let entry_count = u16::from_le_bytes(page[2..4].try_into().unwrap()) as usize;
    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let slot_off = LEAF_HEADER_SIZE + i * SLOT_SIZE;
        let offset = u16::from_le_bytes(page[slot_off..slot_off + 2].try_into().unwrap()) as usize;
        let length =
            u16::from_le_bytes(page[slot_off + 2..slot_off + 4].try_into().unwrap()) as usize;
        let (k, fr): (K, FactRef) = postcard::from_bytes(&page[offset..offset + length])?;
        entries.push((k, fr));
    }
    Ok(entries)
}

// ─── stream_all_entries ───────────────────────────────────────────────────────

/// Stream all `(K, FactRef)` entries from a B+tree in sorted order.
pub fn stream_all_entries<K>(
    root_page_id: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<(K, FactRef)>>
where
    K: for<'de> Deserialize<'de> + Ord,
{
    let first_leaf = find_leftmost_leaf(root_page_id, backend, cache)?;
    let mut result = Vec::new();
    let mut leaf_id = first_leaf;

    loop {
        let page = cache.get_or_load(leaf_id, backend)?;
        if page[0] != PAGE_TYPE_LEAF {
            anyhow::bail!(
                "stream_all_entries: expected leaf page at page_id={}",
                leaf_id
            );
        }
        let next_leaf = u64::from_le_bytes(page[4..12].try_into().unwrap());
        result.extend(read_leaf_entries::<K>(&page)?);

        if next_leaf == 0 {
            break;
        }
        leaf_id = next_leaf;
    }

    Ok(result)
}

// ─── range_scan ───────────────────────────────────────────────────────────────

/// Scan the B+tree for all `FactRef`s whose key is in `[start, end]`.
///
/// `end: None` means unbounded (scan to last leaf).
pub fn range_scan<K>(
    root_page_id: u64,
    start: &K,
    end: Option<&K>,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<FactRef>>
where
    K: Serialize + for<'de> Deserialize<'de> + Ord,
{
    let start_leaf = find_leaf_for_key(root_page_id, start, backend, cache)?;
    let mut result = Vec::new();
    let mut leaf_id = start_leaf;

    'outer: loop {
        let page = cache.get_or_load(leaf_id, backend)?;
        if page[0] != PAGE_TYPE_LEAF {
            anyhow::bail!("range_scan: expected leaf at page_id={}", leaf_id);
        }
        let next_leaf = u64::from_le_bytes(page[4..12].try_into().unwrap());
        let entries: Vec<(K, FactRef)> = read_leaf_entries(&page)?;

        for (k, fr) in entries {
            if k < *start {
                continue;
            }
            if let Some(e) = end
                && k >= *e
            {
                break 'outer;
            }
            result.push(fr);
        }

        if next_leaf == 0 {
            break;
        }
        leaf_id = next_leaf;
    }

    Ok(result)
}

// ─── MutexStorageBackend ──────────────────────────────────────────────────────

/// Read-only [`StorageBackend`] adapter that locks `Arc<Mutex<B>>` only for the
/// duration of a single [`StorageBackend::read_page`] call.
///
/// Used exclusively by [`OnDiskIndexReader::range_scan_*`] so that the backend
/// mutex is held only while reading one cold page from disk, rather than for the
/// entire range scan. On a cache hit [`PageCache::get_or_load`] never calls
/// `read_page`, so no lock is acquired at all. All methods other than `read_page`
/// are unimplemented and will panic if called.
struct MutexStorageBackend<B>(Arc<Mutex<B>>);

impl<B: StorageBackend> StorageBackend for MutexStorageBackend<B> {
    fn read_page(&self, page_id: u64) -> anyhow::Result<Vec<u8>> {
        self.0.lock().unwrap().read_page(page_id)
    }

    fn write_page(&mut self, _page_id: u64, _data: &[u8]) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; write_page must not be called")
    }

    fn sync(&mut self) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; sync must not be called")
    }

    fn page_count(&self) -> anyhow::Result<u64> {
        unimplemented!("MutexStorageBackend is read-only; page_count must not be called")
    }

    fn close(&mut self) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; close must not be called")
    }

    fn backend_name(&self) -> &'static str {
        unimplemented!("MutexStorageBackend is read-only; backend_name must not be called")
    }

    fn is_new(&self) -> bool {
        self.0.lock().unwrap().is_new()
    }
}

// ─── OnDiskIndexReader ────────────────────────────────────────────────────────

/// Implements `CommittedIndexReader` by delegating to `range_scan` on
/// on-disk B+tree pages via the page cache.
pub struct OnDiskIndexReader<B: StorageBackend + 'static> {
    backend: Arc<Mutex<B>>,
    cache: Arc<PageCache>,
    pub(crate) eavt_root: u64,
    pub(crate) aevt_root: u64,
    pub(crate) avet_root: u64,
    pub(crate) vaet_root: u64,
}

impl<B: StorageBackend + 'static> OnDiskIndexReader<B> {
    pub fn new(
        backend: Arc<Mutex<B>>,
        cache: Arc<PageCache>,
        eavt_root: u64,
        aevt_root: u64,
        avet_root: u64,
        vaet_root: u64,
    ) -> Self {
        OnDiskIndexReader {
            backend,
            cache,
            eavt_root,
            aevt_root,
            avet_root,
            vaet_root,
        }
    }
}

impl<B: StorageBackend + 'static> crate::storage::CommittedIndexReader for OnDiskIndexReader<B> {
    fn range_scan_eavt(
        &self,
        start: &crate::storage::index::EavtKey,
        end: Option<&crate::storage::index::EavtKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>> {
        if self.eavt_root == 0 {
            return Ok(vec![]);
        }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.eavt_root, start, end, &adapter, &self.cache)
    }

    fn range_scan_aevt(
        &self,
        start: &crate::storage::index::AevtKey,
        end: Option<&crate::storage::index::AevtKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>> {
        if self.aevt_root == 0 {
            return Ok(vec![]);
        }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.aevt_root, start, end, &adapter, &self.cache)
    }

    fn range_scan_avet(
        &self,
        start: &crate::storage::index::AvetKey,
        end: Option<&crate::storage::index::AvetKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>> {
        if self.avet_root == 0 {
            return Ok(vec![]);
        }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.avet_root, start, end, &adapter, &self.cache)
    }

    fn range_scan_vaet(
        &self,
        start: &crate::storage::index::VaetKey,
        end: Option<&crate::storage::index::VaetKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>> {
        if self.vaet_root == 0 {
            return Ok(vec![]);
        }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.vaet_root, start, end, &adapter, &self.cache)
    }
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
            FactRef {
                page_id: tx + 1,
                slot_index: 0,
            },
        )
    }

    #[test]
    fn test_build_btree_empty_returns_single_leaf() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let entries: Vec<(EavtKey, FactRef)> = vec![];
        let ser = btree_entries(entries.into_iter()).unwrap();
        let (root, next_free) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
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
        let ser = btree_entries(entries.into_iter()).unwrap();
        let (root, next_free) = build_btree(ser.into_iter(), &mut backend, &cache, 5).unwrap();
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
        let entries1 = btree_entries((0u128..5).map(|n| make_eavt(n, ":a", n as u64 + 1))).unwrap();
        let (_, next1) = build_btree(entries1.into_iter(), &mut backend, &cache, 1).unwrap();

        let entries2 =
            btree_entries((5u128..10).map(|n| make_eavt(n, ":b", n as u64 + 1))).unwrap();
        let (root2, next2) =
            build_btree(entries2.into_iter(), &mut backend, &cache, next1).unwrap();

        assert!(root2 >= next1, "second tree must not overlap with first");
        assert!(next2 > root2);
    }

    #[test]
    fn test_build_btree_pages_in_cache_after_build() {
        // All written pages must be retrievable from cache without backend read
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let entries =
            btree_entries((0u128..100).map(|n| make_eavt(n, ":x", n as u64 + 1))).unwrap();
        let (root, next_free) = build_btree(entries.into_iter(), &mut backend, &cache, 1).unwrap();

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
        let entries = btree_entries(
            (0u128..200).map(|n| make_eavt(n, ":verylongattributename", n as u64 + 1)),
        )
        .unwrap();
        let (root, next_free) = build_btree(entries.into_iter(), &mut backend, &cache, 1).unwrap();

        for page_id in root..next_free {
            let page = cache.get_or_load(page_id, &backend).unwrap();
            assert_eq!(
                page.len(),
                PAGE_SIZE,
                "every page must be exactly PAGE_SIZE"
            );
        }
    }

    #[test]
    fn test_build_btree_internal_node_created_for_many_entries() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(512);
        // ~300 entries should force at least 2 leaf pages and 1 internal node
        let entries = (0u128..300).map(|n| make_eavt(n, ":attr", n as u64 + 1));
        let ser = btree_entries(entries).unwrap();
        let (root, next_free) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let root_page = cache.get_or_load(root, &backend).unwrap();
        let pages_written = next_free - 1;
        assert!(
            pages_written >= 2,
            "300 entries must need multiple pages; got {}",
            pages_written
        );
        // With 300 entries at 75% fill factor (~3072 bytes/leaf), we always get multiple
        // leaf pages, so the root MUST be an internal node.
        assert_eq!(
            root_page[0], PAGE_TYPE_INTERNAL,
            "300 entries should produce an internal node root, got page type 0x{:02x}",
            root_page[0]
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
        let ser = btree_entries(entries).unwrap();
        let (root, next_free) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

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

        assert!(
            chain.len() >= 2,
            "100 long-key entries should span multiple leaves; got {} leaves",
            chain.len()
        );
        // Total entries across all leaves must equal 100
        let total_entries: u64 = chain
            .iter()
            .map(|&pid| {
                let p = cache.get_or_load(pid, &backend).unwrap();
                u16::from_le_bytes(p[2..4].try_into().unwrap()) as u64
            })
            .sum();
        assert_eq!(total_entries, 100);
        // next_free must be > all leaf page IDs
        for &pid in &chain {
            assert!(
                pid < next_free,
                "leaf {} must be < next_free {}",
                pid,
                next_free
            );
        }
    }

    #[test]
    fn test_merge_sorted_vecs_duplicates() {
        let a = vec![1u32, 3, 3, 5];
        let b = vec![2u32, 3, 4];
        let merged: Vec<u32> = merge_sorted_vecs(a, b).collect();
        assert_eq!(merged, vec![1, 2, 3, 3, 3, 4, 5]);
    }

    #[test]
    fn test_stream_all_entries_roundtrip() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let input: Vec<(EavtKey, FactRef)> = (0u128..50)
            .map(|n| make_eavt(n, ":name", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let output: Vec<(EavtKey, FactRef)> = stream_all_entries(root, &backend, &cache).unwrap();

        assert_eq!(output.len(), 50);
        for w in output.windows(2) {
            assert!(w[0].0 <= w[1].0, "entries must be in sorted order");
        }
        for (original, recovered) in input.iter().zip(output.iter()) {
            assert_eq!(original.1, recovered.1);
        }
    }

    #[test]
    fn test_stream_all_entries_empty_tree() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(16);
        let entries: Vec<(EavtKey, FactRef)> = vec![];
        let ser = btree_entries(entries.into_iter()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        let out: Vec<(EavtKey, FactRef)> = stream_all_entries(root, &backend, &cache).unwrap();
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn test_range_scan_exact_match() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let input: Vec<(EavtKey, FactRef)> = (0u128..100)
            .map(|n| make_eavt(n, ":v", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let target_entity = Uuid::from_u128(42);
        let start = EavtKey {
            entity: target_entity,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let next_entity = Uuid::from_u128(43);
        let end = EavtKey {
            entity: next_entity,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };

        let refs = range_scan(root, &start, Some(&end), &backend, &cache).unwrap();
        assert_eq!(refs.len(), 1, "exactly one entry for entity 42");
        // make_eavt(42, ":v", 43) → FactRef { page_id: 43+1=44, slot_index: 0 }
        assert_eq!(
            refs[0],
            FactRef {
                page_id: 44,
                slot_index: 0
            }
        );
    }

    #[test]
    fn test_range_scan_empty_range() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let input: Vec<(EavtKey, FactRef)> = (0u128..50)
            .map(|n| make_eavt(n, ":v", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let start = EavtKey {
            entity: Uuid::from_u128(999),
            attribute: String::new(),
            valid_from: 0,
            valid_to: 0,
            tx_count: 0,
        };
        let refs = range_scan::<EavtKey>(root, &start, None, &backend, &cache).unwrap();
        assert_eq!(refs.len(), 0);
    }

    #[test]
    fn test_range_scan_unbounded_end() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let input: Vec<(EavtKey, FactRef)> = (0u128..10)
            .map(|n| make_eavt(n, ":v", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let start = EavtKey {
            entity: Uuid::from_u128(5),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let refs = range_scan::<EavtKey>(root, &start, None, &backend, &cache).unwrap();
        assert_eq!(refs.len(), 5, "entities 5..9 = 5 entries");
    }

    #[test]
    fn test_range_scan_multi_leaf_span() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(512);
        let input: Vec<(EavtKey, FactRef)> = (0u128..500)
            .map(|n| make_eavt(n, ":a", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let start = EavtKey {
            entity: Uuid::from_u128(100),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let end = EavtKey {
            entity: Uuid::from_u128(200),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let refs = range_scan(root, &start, Some(&end), &backend, &cache).unwrap();
        // NOTE: The end key has attribute="" which sorts BEFORE ":a". So entity 200's
        // actual entry {200, ":a", ...} sorts AFTER the end key and is EXCLUDED.
        // Result: entities 100..199 = 100 entries.
        assert_eq!(
            refs.len(),
            100,
            "entities 100..199 (end key excludes entity 200's entry since its attr ':a' > '')"
        );
    }

    #[test]
    fn test_on_disk_index_reader_range_scan_eavt() {
        use crate::storage::CommittedIndexReader;
        use std::sync::Arc;

        let mut backend = MemoryBackend::new();
        let cache = Arc::new(PageCache::new(256));
        let input: Vec<(EavtKey, FactRef)> = (0u128..20)
            .map(|n| make_eavt(n, ":x", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (eavt_root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let reader =
            OnDiskIndexReader::new(Arc::new(Mutex::new(backend)), cache, eavt_root, 0, 0, 0);

        let start = EavtKey {
            entity: Uuid::from_u128(5),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let end = EavtKey {
            entity: Uuid::from_u128(10),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let refs = reader.range_scan_eavt(&start, Some(&end)).unwrap();
        // Same exclusion logic: entity 10's entry {10, ":x", ...} > end {10, "", ...}
        // So entities 5..9 = 5 entries
        assert_eq!(refs.len(), 5, "entities 5..9 (end excludes entity 10)");
    }

    #[test]
    fn test_concurrent_range_scans_correctness() {
        use crate::storage::CommittedIndexReader;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let mut backend = MemoryBackend::new();
        // build_btree takes &PageCache (not Arc), so construct without Arc first
        let cache = PageCache::new(256);
        // 50 entries — enough to span multiple leaf pages
        let input: Vec<(EavtKey, FactRef)> = (0u128..50)
            .map(|n| make_eavt(n, ":x", n as u64 + 1))
            .collect();
        let ser = btree_entries(input.iter().cloned()).unwrap();
        let (eavt_root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        // Wrap in Arc after build_btree is done — OnDiskIndexReader requires Arc<PageCache>
        let reader = Arc::new(OnDiskIndexReader::new(
            Arc::new(Mutex::new(backend)),
            Arc::new(cache),
            eavt_root,
            0,
            0,
            0,
        ));

        // Scan entities 10..19 (10 entries expected)
        let start = EavtKey {
            entity: Uuid::from_u128(10),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let end = EavtKey {
            entity: Uuid::from_u128(20),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };

        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = Arc::clone(&reader);
                let b = Arc::clone(&barrier);
                let s = start.clone();
                let e = end.clone();
                thread::spawn(move || {
                    b.wait(); // all 8 threads start simultaneously
                    r.range_scan_eavt(&s, Some(&e)).unwrap()
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let expected_len = results[0].len();
        assert_eq!(expected_len, 10, "expected 10 entries for entities 10..19");
        for (i, res) in results.iter().enumerate() {
            assert_eq!(
                res.len(),
                expected_len,
                "thread {} returned {} refs, expected {}",
                i,
                res.len(),
                expected_len
            );
        }
    }
}
