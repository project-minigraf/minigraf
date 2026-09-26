//! On-disk B+tree for covering index persistence (file format v6).
//!
//! Each node maps to exactly one 4KB page. The `PageCache` serves all reads.
//! `build_btree` does a bulk-build (write-all-leaves, then internal levels
//! bottom-up). Range scans traverse the tree through the cache.

use crate::error::{ErrorCode, bail_coded, err_coded};
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

// ─── Safe slice access helpers ───────────────────────────────────────────────

/// Read a u16 from 2 bytes at the given offset, returning an error if out of bounds.
fn read_u16_at(page: &[u8], offset: usize) -> Result<u16> {
    let bytes = page.get(offset..offset.saturating_add(2)).ok_or_else(|| {
        err_coded!(
            ErrorCode::Int049,
            format!("out of bounds: read_u16 at {offset} (len {})", page.len())
        )
    })?;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
        err_coded!(ErrorCode::Int049, format!("slice at {offset} not 2 bytes"))
    })?))
}

/// Read a u64 from 8 bytes at the given offset, returning an error if out of bounds.
fn read_u64_at(page: &[u8], offset: usize) -> Result<u64> {
    let bytes = page.get(offset..offset.saturating_add(8)).ok_or_else(|| {
        err_coded!(
            ErrorCode::Int049,
            format!("out of bounds: read_u64 at {offset} (len {})", page.len())
        )
    })?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
        err_coded!(ErrorCode::Int049, format!("slice at {offset} not 8 bytes"))
    })?))
}

// ─── Low-level page writers ───────────────────────────────────────────────────

/// Encode a leaf page: fixed header, slot directory, entries written end-to-start.
///
/// `entries`: each element is the postcard-serialised `(K, FactRef)` bytes for
/// one index entry, in sort order.
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
fn encode_leaf_page(entries: &[Vec<u8>], next_leaf: u64) -> Result<Vec<u8>> {
    let entry_count = u16::try_from(entries.len()).map_err(|_| {
        err_coded!(
            ErrorCode::Int049,
            format!("too many entries: {}", entries.len())
        )
    })?;
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
        let write_pos_u16 = u16::try_from(write_pos).map_err(|_| {
            err_coded!(
                ErrorCode::Int048,
                format!("write_pos {write_pos} exceeds u16")
            )
        })?;
        let entry_len_u16 = u16::try_from(entry.len()).map_err(|_| {
            err_coded!(
                ErrorCode::Int048,
                format!("entry len {} exceeds u16", entry.len())
            )
        })?;
        page[slot_off..slot_off + 2].copy_from_slice(&write_pos_u16.to_le_bytes());
        page[slot_off + 2..slot_off + 4].copy_from_slice(&entry_len_u16.to_le_bytes());
    }
    Ok(page)
}

/// Write a single leaf page and insert it into the cache.
fn write_leaf_page(
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    page_id: u64,
    entries: &[Vec<u8>],
    next_leaf: u64,
) -> Result<()> {
    let page = encode_leaf_page(entries, next_leaf)?;
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
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
fn write_internal_page(
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    page_id: u64,
    child_ids: &[u64],
    sep_bytes: &[Vec<u8>],
) -> Result<()> {
    debug_assert_eq!(child_ids.len(), sep_bytes.len() + 1);
    // Defensive check: empty child_ids would cause panic on .last()
    if child_ids.is_empty() {
        bail_coded!(ErrorCode::Stg011);
    }
    let key_count = u16::try_from(sep_bytes.len()).map_err(|_| {
        err_coded!(
            ErrorCode::Int049,
            format!("too many sep keys: {}", sep_bytes.len())
        )
    })?;
    let rightmost_child = *child_ids
        .last()
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "child_ids is empty".to_string()))?;

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
        let write_pos_u16 = u16::try_from(write_pos).map_err(|_| {
            err_coded!(
                ErrorCode::Int048,
                format!("write_pos {write_pos} exceeds u16")
            )
        })?;
        let sep_len_u16 = u16::try_from(sep.len()).map_err(|_| {
            err_coded!(
                ErrorCode::Int048,
                format!("sep len {} exceeds u16", sep.len())
            )
        })?;
        page[slot_off..slot_off + 2].copy_from_slice(&write_pos_u16.to_le_bytes());
        page[slot_off + 2..slot_off + 4].copy_from_slice(&sep_len_u16.to_le_bytes());
    }

    backend.write_page(page_id, &page)?;
    cache.put_dirty(page_id, page);
    Ok(())
}

/// True when adding an entry of `entry_len` bytes to a leaf that already holds
/// `n_entries` entries totalling `data_bytes` would exceed the fill threshold.
#[allow(clippy::arithmetic_side_effects)]
fn leaf_overflows(n_entries: usize, data_bytes: usize, entry_len: usize) -> bool {
    LEAF_HEADER_SIZE + (n_entries + 1) * SLOT_SIZE + data_bytes + entry_len > PAGE_FILL_BYTES
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
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
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
        if !cur_entries.is_empty()
            && leaf_overflows(cur_entries.len(), cur_data_bytes, entry_bytes.len())
        {
            write_leaf_page(backend, cache, next_page, &cur_entries, 0)?;
            let first_key = cur_first_key.take().ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int049,
                    "BUG: cur_first_key empty when writing leaf page".to_string()
                )
            })?;
            leaf_infos.push((next_page, first_key));
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
        let first_key = cur_first_key.take().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                "BUG: cur_first_key empty when flushing last leaf page".to_string()
            )
        })?;
        leaf_infos.push((next_page, first_key));
        next_page += 1;
    }

    // Patch next_leaf pointers: leaf[i].next_leaf = leaf[i+1].page_id
    for i in 0..leaf_infos.len() - 1 {
        let (pid, _) = leaf_infos
            .get(i)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, format!("leaf_infos[{i}] out of bounds")))?
            .clone();
        let (next_lid, _) = leaf_infos
            .get(i + 1)
            .ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int049,
                    format!("leaf_infos[{}] out of bounds", i + 1)
                )
            })?
            .clone();
        let cached = cache.get_or_load(pid, backend)?;
        let mut page = (*cached).clone();
        // Safety: page is PAGE_SIZE bytes; offset 4..12 is always valid
        page.get_mut(4..12)
            .ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int049,
                    "page too small to write next_leaf".to_string()
                )
            })?
            .copy_from_slice(&next_lid.to_le_bytes());
        backend.write_page(pid, &page)?;
        cache.put_dirty(pid, page);
    }

    build_internal_levels(leaf_infos, backend, cache, next_page)
}

/// Build internal levels bottom-up over `leaf_infos` (`(page_id, first_key_bytes)`
/// per leaf, in key order), writing nodes from `next_page` onward.
///
/// Returns `(root_page_id, next_free_page_id)`. A single leaf is its own root.
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
fn build_internal_levels(
    leaf_infos: Vec<(u64, Vec<u8>)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    next_page: u64,
) -> Result<(u64, u64)> {
    if leaf_infos.is_empty() {
        bail_coded!(
            ErrorCode::Int049,
            "build_internal_levels: no leaves".to_string()
        );
    }
    let mut next_page = next_page;
    let mut current_level = leaf_infos;

    loop {
        if current_level.len() == 1 {
            return Ok((
                current_level
                    .first()
                    .ok_or_else(|| {
                        err_coded!(
                            ErrorCode::Int049,
                            "current_level unexpectedly empty".to_string()
                        )
                    })?
                    .0,
                next_page,
            ));
        }

        let mut next_level: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut i = 0;

        while i < current_level.len() {
            let i_start = i;
            let first_entry = current_level.get(i).ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int049,
                    format!("current_level[{i}] out of bounds")
                )
            })?;
            let mut child_ids: Vec<u64> = vec![first_entry.0];
            let mut sep_bytes: Vec<Vec<u8>> = Vec::new();
            let mut sep_data_bytes: usize = 0;
            i += 1;

            while i < current_level.len() {
                let entry = current_level.get(i).ok_or_else(|| {
                    err_coded!(
                        ErrorCode::Int049,
                        format!("current_level[{i}] out of bounds")
                    )
                })?;
                let sep = entry.1.clone();
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
                child_ids.push(
                    current_level
                        .get(i)
                        .ok_or_else(|| {
                            err_coded!(
                                ErrorCode::Int049,
                                format!("current_level[{i}] out of bounds")
                            )
                        })?
                        .0,
                );
                i += 1;
            }

            let node_page_id = next_page;
            write_internal_page(backend, cache, node_page_id, &child_ids, &sep_bytes)?;
            next_page += 1;

            let first_key = current_level
                .get(i_start)
                .ok_or_else(|| {
                    err_coded!(
                        ErrorCode::Int049,
                        format!("current_level[{i_start}] out of bounds")
                    )
                })?
                .1
                .clone();
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
            if ai.peek() <= bi.peek() {
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

// ─── Incremental rebuild (#315) ───────────────────────────────────────────────

/// Collect the raw leaf pages of the B+tree at `root_page_id`, in key order.
///
/// `save()` calls this before writing anything: new fact pages overwrite the
/// start of the old index region, so the old leaves must be snapshotted first.
pub fn collect_leaf_pages(
    root_page_id: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<Arc<Vec<u8>>>> {
    let mut leaves = Vec::new();
    let mut leaf_id = find_leftmost_leaf(root_page_id, backend, cache)?;
    loop {
        let page = cache.get_or_load(leaf_id, backend)?;
        if page.first().copied() != Some(PAGE_TYPE_LEAF) {
            bail_coded!(
                ErrorCode::Int049,
                format!("collect_leaf_pages: expected leaf page at page_id={leaf_id}")
            );
        }
        let next_leaf = read_u64_at(&page[..], 4)?;
        leaves.push(page);
        if next_leaf == 0 {
            break;
        }
        leaf_id = next_leaf;
    }
    Ok(leaves)
}

/// Decode the key of a leaf's first entry; `None` for an empty leaf.
#[allow(clippy::arithmetic_side_effects)]
fn leaf_first_key<K>(page: &[u8]) -> Result<Option<K>>
where
    K: for<'de> Deserialize<'de>,
{
    if read_u16_at(page, 2)? == 0 {
        return Ok(None);
    }
    let offset = read_u16_at(page, LEAF_HEADER_SIZE)? as usize;
    let length = read_u16_at(page, LEAF_HEADER_SIZE + 2)? as usize;
    let slice = page
        .get(offset..offset.saturating_add(length))
        .ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("first entry out of bounds: offset={offset} len={length}")
            )
        })?;
    let (key, _): (K, FactRef) = postcard::from_bytes(slice)?;
    Ok(Some(key))
}

/// Writes leaf pages at consecutive page ids, linking each to the next.
///
/// One page is held back so its `next_leaf` can be set to the following page
/// id, or to 0 once it is known to be the last leaf.
struct LeafEmitter<'a> {
    backend: &'a mut dyn StorageBackend,
    cache: &'a PageCache,
    next_page: u64,
    held: Option<(u64, Vec<u8>)>,
    infos: Vec<(u64, Vec<u8>)>,
}

impl LeafEmitter<'_> {
    #[allow(clippy::arithmetic_side_effects)]
    fn push(&mut self, page: Vec<u8>, first_key_bytes: Vec<u8>) -> Result<()> {
        let page_id = self.next_page;
        self.next_page += 1;
        self.release_held(page_id)?;
        self.held = Some((page_id, page));
        self.infos.push((page_id, first_key_bytes));
        Ok(())
    }

    fn release_held(&mut self, next_leaf: u64) -> Result<()> {
        if let Some((page_id, mut page)) = self.held.take() {
            page.get_mut(4..12)
                .ok_or_else(|| {
                    err_coded!(
                        ErrorCode::Int049,
                        "page too small to write next_leaf".to_string()
                    )
                })?
                .copy_from_slice(&next_leaf.to_le_bytes());
            self.backend.write_page(page_id, &page)?;
            self.cache.put_dirty(page_id, page);
        }
        Ok(())
    }

    /// Write the last held leaf and return `(leaf_infos, next_free_page_id)`.
    fn finish(mut self) -> Result<(Vec<(u64, Vec<u8>)>, u64)> {
        self.release_held(0)?;
        Ok((self.infos, self.next_page))
    }
}

/// Serialise sorted entries and emit them as leaves using `build_btree`'s fill rule.
#[allow(clippy::arithmetic_side_effects)]
fn emit_packed<K: Serialize>(
    emitter: &mut LeafEmitter<'_>,
    entries: impl Iterator<Item = (K, FactRef)>,
) -> Result<()> {
    let mut cur: Vec<Vec<u8>> = Vec::new();
    let mut cur_bytes = 0usize;
    let mut cur_first: Option<Vec<u8>> = None;
    for (key, fact_ref) in entries {
        let entry = postcard::to_allocvec(&(&key, &fact_ref))?;
        if !cur.is_empty() && leaf_overflows(cur.len(), cur_bytes, entry.len()) {
            let first = cur_first.take().ok_or_else(|| {
                err_coded!(ErrorCode::Int049, "BUG: leaf without first key".to_string())
            })?;
            emitter.push(encode_leaf_page(&cur, 0)?, first)?;
            cur.clear();
            cur_bytes = 0;
        }
        if cur_first.is_none() {
            cur_first = Some(postcard::to_allocvec(&key)?);
        }
        cur_bytes += entry.len();
        cur.push(entry);
    }
    if let Some(first) = cur_first {
        emitter.push(encode_leaf_page(&cur, 0)?, first)?;
    }
    Ok(())
}

/// Rebuild a B+tree from its old leaves plus sorted `pending` entries.
///
/// Leaves that receive no pending entry are copied verbatim (only `next_leaf`
/// is patched); only leaves that do are decoded, merged and repacked. Internal
/// levels are rebuilt from scratch. Pending keys route to leaf `i` when
/// `first_key[i] <= key < first_key[i + 1]`; keys below the first leaf's first
/// key go to leaf 0. With no non-empty old leaves this is a plain bulk build.
///
/// `old_leaves` must be snapshotted (see [`collect_leaf_pages`]) before any
/// page in `start_page_id..` is written. Returns `(root_page_id, next_free_page_id)`.
pub fn rebuild_btree_incremental<K>(
    old_leaves: Vec<Arc<Vec<u8>>>,
    pending: Vec<(K, FactRef)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    start_page_id: u64,
) -> Result<(u64, u64)>
where
    K: Serialize + for<'de> Deserialize<'de> + Ord,
{
    let mut leaves: Vec<(Arc<Vec<u8>>, K)> = Vec::with_capacity(old_leaves.len());
    for page in old_leaves {
        if let Some(first_key) = leaf_first_key::<K>(&page[..])? {
            leaves.push((page, first_key));
        }
    }
    if leaves.is_empty() {
        return build_btree(
            btree_entries(pending.into_iter())?.into_iter(),
            backend,
            cache,
            start_page_id,
        );
    }

    let mut emitter = LeafEmitter {
        backend: &mut *backend,
        cache,
        next_page: start_page_id,
        held: None,
        infos: Vec::new(),
    };
    let mut pending = pending.into_iter().peekable();
    for (i, (page, first_key)) in leaves.iter().enumerate() {
        let upper = leaves.get(i.saturating_add(1)).map(|(_, k)| k);
        let mut batch = Vec::new();
        while let Some((key, _)) = pending.peek() {
            if upper.is_some_and(|u| key >= u) {
                break;
            }
            if let Some(entry) = pending.next() {
                batch.push(entry);
            }
        }
        if batch.is_empty() {
            emitter.push((**page).clone(), postcard::to_allocvec(first_key)?)?;
        } else {
            let old = read_leaf_entries::<K>(&page[..])?;
            emit_packed(&mut emitter, merge_sorted_vecs(old, batch))?;
        }
    }
    let (leaf_infos, next_page) = emitter.finish()?;
    build_internal_levels(leaf_infos, backend, cache, next_page)
}

// ─── Leaf traversal helpers ───────────────────────────────────────────────────

/// Traverse internal nodes from `root` to find the leftmost (first) leaf page.
#[allow(clippy::arithmetic_side_effects)]
fn find_leftmost_leaf(root: u64, backend: &dyn StorageBackend, cache: &PageCache) -> Result<u64> {
    let mut page_id = root;
    loop {
        let page = cache.get_or_load(page_id, backend)?;
        let page_type = page.first().copied().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("empty page at page_id={page_id}")
            )
        })?;
        match page_type {
            PAGE_TYPE_LEAF => return Ok(page_id),
            PAGE_TYPE_INTERNAL => {
                let key_count = read_u16_at(&page[..], 2)? as usize;
                if key_count == 0 {
                    page_id = read_u64_at(&page[..], 4)?;
                } else {
                    page_id = read_u64_at(&page[..], INTERNAL_HEADER_SIZE)?;
                }
            }
            t => bail_coded!(
                ErrorCode::Int049,
                format!(
                    "find_leftmost_leaf: unexpected page type 0x{:02x} at page_id={}",
                    t, page_id
                )
            ),
        }
    }
}

/// Traverse from `root` to the leaf that would contain `key`.
// Called by range_scan which is called by OnDiskIndexReader::range_scan_*.
#[allow(dead_code)]
#[allow(clippy::arithmetic_side_effects)]
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
        let page_type = page.first().copied().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("empty page at page_id={page_id}")
            )
        })?;
        match page_type {
            PAGE_TYPE_LEAF => return Ok(page_id),
            PAGE_TYPE_INTERNAL => {
                let key_count = read_u16_at(&page[..], 2)? as usize;
                let rightmost_child = read_u64_at(&page[..], 4)?;
                let child_arr_start = INTERNAL_HEADER_SIZE;
                let slot_dir_start = INTERNAL_HEADER_SIZE + key_count * 8;

                let mut descended = false;
                for i in 0..key_count {
                    let slot_off = slot_dir_start + i * SLOT_SIZE;
                    let sep_offset = read_u16_at(&page[..], slot_off)? as usize;
                    let sep_length = read_u16_at(&page[..], slot_off + 2)? as usize;
                    let sep_slice = page
                        .get(sep_offset..sep_offset.saturating_add(sep_length))
                        .ok_or_else(|| {
                            err_coded!(ErrorCode::Int049, format!(
                                "sep slice out of bounds: offset={sep_offset} len={sep_length} page_len={}",
                                page.len()
                            ))
                        })?;
                    let sep_key: K = postcard::from_bytes(sep_slice)?;

                    if *key < sep_key {
                        let child_off = child_arr_start + i * 8;
                        page_id = read_u64_at(&page[..], child_off)?;
                        descended = true;
                        break;
                    }
                }
                if !descended {
                    page_id = rightmost_child;
                }
            }
            t => bail_coded!(
                ErrorCode::Int049,
                format!(
                    "find_leaf_for_key: unexpected page type 0x{:02x} at page_id={}",
                    t, page_id
                )
            ),
        }
    }
}

/// Read all `(K, FactRef)` entries from a leaf page's slot directory.
#[allow(clippy::arithmetic_side_effects)]
fn read_leaf_entries<K>(page: &[u8]) -> Result<Vec<(K, FactRef)>>
where
    K: for<'de> Deserialize<'de>,
{
    let entry_count = read_u16_at(page, 2)? as usize;
    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let slot_off = LEAF_HEADER_SIZE + i * SLOT_SIZE;
        let offset = read_u16_at(page, slot_off)? as usize;
        let length = read_u16_at(page, slot_off + 2)? as usize;
        let slice = page
            .get(offset..offset.saturating_add(length))
            .ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int049,
                    format!(
                        "entry slice out of bounds: offset={offset} len={length} page_len={}",
                        page.len()
                    )
                )
            })?;
        let (k, fr): (K, FactRef) = postcard::from_bytes(slice)?;
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
        let page_type = page.first().copied().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("empty page at page_id={leaf_id}")
            )
        })?;
        if page_type != PAGE_TYPE_LEAF {
            bail_coded!(
                ErrorCode::Int049,
                format!(
                    "stream_all_entries: expected leaf page at page_id={}",
                    leaf_id
                )
            );
        }
        let next_leaf = read_u64_at(&page[..], 4)?;
        result.extend(read_leaf_entries::<K>(&page[..])?);

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
// Called by OnDiskIndexReader::range_scan_* (via trait object dispatch).
#[allow(dead_code)]
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
        let page_type = page.first().copied().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("empty page at page_id={leaf_id}")
            )
        })?;
        if page_type != PAGE_TYPE_LEAF {
            bail_coded!(ErrorCode::Stg013, leaf_id);
        }
        let next_leaf = read_u64_at(&page[..], 4)?;
        let entries: Vec<(K, FactRef)> = read_leaf_entries(&page[..])?;

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
/// Used by [`OnDiskIndexReader::range_scan_*`] and [`crate::storage::persistent_facts`]
/// so that the backend mutex is held only while reading one cold page from disk,
/// rather than for the entire operation. On a cache hit [`PageCache::get_or_load`]
/// never calls `read_page`, so no lock is acquired at all. All methods other than
/// `read_page` are unimplemented and will panic if called.
pub(crate) struct MutexStorageBackend<B>(pub(crate) Arc<Mutex<B>>);

impl<B: StorageBackend> StorageBackend for MutexStorageBackend<B> {
    fn read_page(&self, page_id: u64) -> anyhow::Result<Vec<u8>> {
        self.0
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Int050, "MutexStorageBackend"))?
            .read_page(page_id)
    }

    #[allow(clippy::unimplemented)]
    fn write_page(&mut self, _page_id: u64, _data: &[u8]) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; write_page must not be called")
    }

    #[allow(clippy::unimplemented)]
    fn sync(&mut self) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; sync must not be called")
    }

    #[allow(clippy::unimplemented)]
    fn page_count(&self) -> anyhow::Result<u64> {
        unimplemented!("MutexStorageBackend is read-only; page_count must not be called")
    }

    #[allow(clippy::unimplemented)]
    fn close(&mut self) -> anyhow::Result<()> {
        unimplemented!("MutexStorageBackend is read-only; close must not be called")
    }

    #[allow(clippy::unimplemented)]
    fn backend_name(&self) -> &'static str {
        unimplemented!("MutexStorageBackend is read-only; backend_name must not be called")
    }

    fn is_new(&self) -> bool {
        self.0.lock().map(|g| g.is_new()).unwrap_or(false)
    }
}

// ─── OnDiskIndexReader ────────────────────────────────────────────────────────

/// Implements `CommittedIndexReader` by delegating to `range_scan` on
/// on-disk B+tree pages via the page cache.
// Fields are read by range_scan_* methods of the CommittedIndexReader impl.
#[allow(dead_code)]
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

    /// Deterministic xorshift64 so randomized tests are reproducible without deps.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    /// `n` EAVT entries with random entities; `counter` keeps keys unique.
    fn random_eavt(rng: &mut Rng, n: usize, counter: &mut u64) -> Vec<(EavtKey, FactRef)> {
        let mut v: Vec<(EavtKey, FactRef)> = (0..n)
            .map(|_| {
                *counter += 1;
                let entity = (u128::from(rng.next()) << 64) | u128::from(*counter);
                make_eavt(entity, ":attr", *counter)
            })
            .collect();
        v.sort();
        v
    }

    fn sorted_union(a: &[(EavtKey, FactRef)], b: &[(EavtKey, FactRef)]) -> Vec<(EavtKey, FactRef)> {
        let mut v: Vec<_> = a.iter().chain(b.iter()).cloned().collect();
        v.sort();
        v
    }

    /// Build `committed` at page 1, then rebuild with `pending` starting at `start`.
    fn build_then_incremental(
        committed: &[(EavtKey, FactRef)],
        pending: &[(EavtKey, FactRef)],
        start: u64,
    ) -> (MemoryBackend, PageCache, u64, u64) {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let ser = btree_entries(committed.iter().cloned()).unwrap();
        let (old_root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        let leaves = collect_leaf_pages(old_root, &backend, &cache).unwrap();
        let (root, next_free) =
            rebuild_btree_incremental(leaves, pending.to_vec(), &mut backend, &cache, start)
                .unwrap();
        (backend, cache, root, next_free)
    }

    /// Stream equality, a next_leaf chain of non-empty leaves, and range_scan
    /// agreement on random bounds.
    fn assert_tree_exact(
        root: u64,
        backend: &MemoryBackend,
        cache: &PageCache,
        expected: &[(EavtKey, FactRef)],
        rng: &mut Rng,
    ) {
        let got: Vec<(EavtKey, FactRef)> = stream_all_entries(root, backend, cache).unwrap();
        assert_eq!(got.len(), expected.len(), "entry count differs");
        assert!(got == expected, "streamed entries differ from expected");

        let leaves = collect_leaf_pages(root, backend, cache).unwrap();
        if expected.is_empty() {
            assert_eq!(leaves.len(), 1, "empty tree is one empty leaf");
        } else {
            for page in &leaves {
                assert!(read_u16_at(&page[..], 2).unwrap() > 0, "empty leaf in tree");
            }
        }

        for _ in 0..20 {
            if expected.is_empty() {
                break;
            }
            let a = (rng.next() as usize) % expected.len();
            let b = (rng.next() as usize) % expected.len();
            let (lo, hi) = (a.min(b), a.max(b));
            let start = &expected[lo].0;
            let end = &expected[hi].0;
            let want: Vec<FactRef> = expected[lo..hi].iter().map(|(_, r)| *r).collect();
            let refs = range_scan(root, start, Some(end), backend, cache).unwrap();
            assert!(refs == want, "range_scan differs from expected slice");
        }
    }

    #[test]
    fn test_incremental_no_pending_copies_leaves_verbatim() {
        let mut rng = Rng(0x315);
        let mut ctr = 0;
        let committed = random_eavt(&mut rng, 2000, &mut ctr);
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let ser = btree_entries(committed.iter().cloned()).unwrap();
        let (old_root, old_next) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        let old_leaves = collect_leaf_pages(old_root, &backend, &cache).unwrap();
        let (root, _) = rebuild_btree_incremental(
            old_leaves.clone(),
            Vec::<(EavtKey, FactRef)>::new(),
            &mut backend,
            &cache,
            old_next,
        )
        .unwrap();
        let new_leaves = collect_leaf_pages(root, &backend, &cache).unwrap();
        assert_eq!(new_leaves.len(), old_leaves.len(), "leaf count changed");
        for (o, n) in old_leaves.iter().zip(new_leaves.iter()) {
            assert!(o[..4] == n[..4], "leaf header prefix changed");
            assert!(o[12..] == n[12..], "leaf body changed");
        }
        assert_tree_exact(root, &backend, &cache, &committed, &mut rng);
    }

    #[test]
    fn test_incremental_matches_merge_random() {
        let mut rng = Rng(0xC0FFEE);
        let mut ctr = 0;
        for _ in 0..40 {
            let n_committed = (rng.next() % 3000) as usize;
            let committed = random_eavt(&mut rng, n_committed, &mut ctr);
            let n_pending = (rng.next() % 200) as usize;
            let pending = random_eavt(&mut rng, n_pending, &mut ctr);
            let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
            let expected = sorted_union(&committed, &pending);
            assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
        }
    }

    #[test]
    fn test_incremental_pending_outside_old_range() {
        let mut rng = Rng(7);
        let committed: Vec<_> = (1000u128..3000)
            .map(|n| make_eavt(n, ":a", n as u64))
            .collect();
        let pending = vec![
            make_eavt(1, ":a", 1),
            make_eavt(2, ":a", 2),
            make_eavt(9_000, ":a", 9_000),
            make_eavt(9_001, ":a", 9_001),
        ];
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
        let expected = sorted_union(&committed, &pending);
        assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
    }

    #[test]
    fn test_incremental_many_pending_into_one_leaf_splits() {
        let mut rng = Rng(11);
        // Committed entities are 0, 10_000, 20_000, …: every pending 0 < e < 10_000
        // routes to leaf 0, which must split into several leaves.
        let committed: Vec<_> = (0u128..2000)
            .map(|n| make_eavt(n * 10_000, ":a", n as u64 + 1))
            .collect();
        let pending: Vec<_> = (1u128..3000)
            .map(|n| make_eavt(n, ":a", 100_000 + n as u64))
            .collect();
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
        let expected = sorted_union(&committed, &pending);
        assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
    }

    #[test]
    fn test_incremental_empty_old_tree_falls_back_to_bulk_build() {
        let mut rng = Rng(13);
        let mut ctr = 0;
        let pending = random_eavt(&mut rng, 500, &mut ctr);
        let (backend, cache, root, _) = build_then_incremental(&[], &pending, 5000);
        assert_tree_exact(root, &backend, &cache, &pending, &mut rng);
        // Nothing old, nothing new: still a valid single empty leaf.
        let (backend, cache, root, next) = build_then_incremental(&[], &[], 5000);
        assert_eq!(next, root + 1, "empty result is one page");
        assert_tree_exact(root, &backend, &cache, &[], &mut rng);
    }

    #[test]
    fn test_incremental_new_tree_overlaps_old_pages() {
        // save() writes the new tree over the old tree's pages; old leaves are
        // snapshotted first, so the result must still be exact.
        let mut rng = Rng(17);
        let mut ctr = 0;
        let committed = random_eavt(&mut rng, 3000, &mut ctr);
        let pending = random_eavt(&mut rng, 100, &mut ctr);
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 3);
        let expected = sorted_union(&committed, &pending);
        assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
    }

    #[test]
    fn test_incremental_repeated_rounds() {
        // Each round consumes the previous round's incrementally built tree,
        // writing the new tree over the old pages like save() does.
        let mut rng = Rng(19);
        let mut ctr = 0;
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let mut expected = random_eavt(&mut rng, 1500, &mut ctr);
        let ser = btree_entries(expected.iter().cloned()).unwrap();
        let (mut root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        for round in 0..30u64 {
            let n_pending = 1 + (rng.next() % 150) as usize;
            let pending = random_eavt(&mut rng, n_pending, &mut ctr);
            let leaves = collect_leaf_pages(root, &backend, &cache).unwrap();
            let start = 1 + round % 3;
            let (r, _) =
                rebuild_btree_incremental(leaves, pending.clone(), &mut backend, &cache, start)
                    .unwrap();
            root = r;
            expected = sorted_union(&expected, &pending);
            assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
        }
    }

    #[test]
    fn test_read_u16_at_oob_rejected() {
        let page = vec![0u8; 4];
        assert!(read_u16_at(&page, 3).is_err());
        assert!(read_u16_at(&page, 4).is_err());
    }

    #[test]
    fn test_read_u64_at_oob_rejected() {
        let page = vec![0u8; 4];
        assert!(read_u64_at(&page, 0).is_err());
        assert!(read_u64_at(&page, 1).is_err());
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
        let entry_count = read_u16_at(&page[..], 2).unwrap();
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
        assert_eq!(read_u16_at(&page[..], 2).unwrap(), 1);
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
                pid = read_u64_at(&p[..], 12).unwrap();
            }
        };

        // Walk the chain and verify it's contiguous and terminates
        let mut chain: Vec<u64> = vec![leaf_pid];
        loop {
            let p = cache.get_or_load(leaf_pid, &backend).unwrap();
            assert_eq!(p[0], PAGE_TYPE_LEAF, "page {} should be leaf", leaf_pid);
            let next = read_u64_at(&p[..], 4).unwrap();
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
                read_u16_at(&p[..], 2).unwrap() as u64
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

    // ══ #359: STG-0xx regression tests ═══════════════════════════════════
    //
    // STG-011 ("internal page has no children") is guarded by
    // `debug_assert_eq!(child_ids.len(), sep_bytes.len() + 1)` immediately
    // above the `bail_coded!` call site in `write_internal_page`, and no
    // combination of arguments satisfies that assertion when `child_ids` is
    // empty (`sep_bytes.len() + 1` can never be `0`). So under `cargo
    // test`'s debug build, the `debug_assert!` always panics first,
    // preempting the bail entirely -- this call site is only live as a
    // release-build defensive guard and isn't reachable from a normal test
    // build. No dedicated regression test for STG-011 as a result.

    /// A leaf's `next_leaf` chain pointer corrupted to point at a non-leaf
    /// page must surface as the coded STG-013 when `range_scan` follows it,
    /// not a generic error. `find_leaf_for_key`'s own traversal only
    /// validates the *first* leaf it lands on; a corrupted `next_leaf` on a
    /// later page in a multi-leaf scan is caught by `range_scan`'s own
    /// per-page check.
    #[test]
    fn range_scan_corrupted_next_leaf_pointer_returns_stg_013() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(512);
        // Enough entries with long keys to force a multi-leaf tree with an
        // internal-node root (mirrors test_build_btree_leaf_next_pointers_form_chain).
        let entries = (0u128..100).map(|n| make_eavt(n, ":verylongattributename", n as u64 + 1));
        let ser = btree_entries(entries).unwrap();
        let (root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();

        let root_page = cache.get_or_load(root, &backend).unwrap();
        assert_eq!(
            root_page[0], PAGE_TYPE_INTERNAL,
            "100 long-key entries must produce an internal-node root for this test to be valid"
        );

        // Find the leftmost leaf by following first children down from root.
        let mut leaf_pid = root;
        loop {
            let p = cache.get_or_load(leaf_pid, &backend).unwrap();
            if p[0] == PAGE_TYPE_LEAF {
                break;
            }
            leaf_pid = read_u64_at(&p[..], 12).unwrap();
        }

        // Corrupt the leftmost leaf's next_leaf pointer (bytes 4..12) to
        // point at the internal-node root instead of the next real leaf.
        let mut leaf_bytes = backend.read_page(leaf_pid).unwrap();
        leaf_bytes[4..12].copy_from_slice(&root.to_le_bytes());
        backend.write_page(leaf_pid, &leaf_bytes).unwrap();
        cache.invalidate(leaf_pid);

        let start = EavtKey {
            entity: Uuid::from_u128(0),
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let err = range_scan::<EavtKey>(root, &start, None, &backend, &cache)
            .expect_err("following a next_leaf pointer into a non-leaf page must fail");
        let coded: crate::error::MinigrafError = err.into();
        assert_eq!(coded.code(), "STG-013");
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
    #[cfg(not(target_os = "wasi"))]
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
