//! On-disk B+tree over byte keys (spec §4.3, §7).
//!
//! Each node is one page in the format of [`crate::storage::node`]. Entries are
//! `(key, value)` byte strings compared with memcmp; the four index trees use
//! empty values and DICT uses real ones. Leaves have no sibling pointers: scans
//! use [`LeafCursor`], which keeps the path from the root.
//!
//! [`build_btree`] bulk-builds a tree from sorted entries. [`cow_insert`] adds
//! entries copy-on-write, rewriting only the touched leaves and their paths and
//! splitting by balanced bytes. Every page comes from a [`PageAllocator`], so an
//! insert never writes a page the old tree uses.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::cache::PageCache;
use crate::storage::node::{
    Entry, Internal, decode_leaf, encode_internal, encode_leaf, internal_size, leaf_entry_sizes,
    leaf_get, leaf_size, shortest_separator,
};
use crate::storage::page::{PAGE_HEADER_SIZE, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PageAllocator};
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::sync::{Arc, Mutex};

/// Fill target for bulk-built and split leaves and internal nodes (~75%).
const PAGE_FILL_BYTES: usize = PAGE_SIZE * 3 / 4;

/// Deepest tree a reader descends. Every internal node has at least two
/// children, so a real tree this deep would need 2^32 leaves; anything deeper
/// is a cycle in the child pointers or corruption.
const MAX_TREE_DEPTH: usize = 32;

/// A written node: its page id and the first and last key below it.
struct NodeInfo {
    id: u64,
    first: Vec<u8>,
    last: Vec<u8>,
}

// ─── Packing ─────────────────────────────────────────────────────────────────

/// Split sorted `entries` into leaves. Entries that fit in one page stay in one
/// leaf. Otherwise they are cut into the fewest leaves of about
/// [`PAGE_FILL_BYTES`] each, balanced by size, so a leaf that overflows by one
/// entry splits roughly in half (#315).
fn pack_leaves(entries: Vec<Entry>) -> Result<Vec<Vec<Entry>>> {
    let sizes = leaf_entry_sizes(&entries);
    let total: usize = sizes.iter().sum();
    if PAGE_HEADER_SIZE + total <= PAGE_SIZE {
        return Ok(vec![entries]);
    }
    let usable = PAGE_FILL_BYTES - PAGE_HEADER_SIZE;
    let mut parts = total.div_ceil(usable).max(2);
    loop {
        // Group `g` ends where the running size reaches (g + 1) / parts of the
        // total, so cuts never drift and the last group is no larger than the
        // others. Cuts move restart points, so each group is checked exactly.
        let mut bounds = Vec::with_capacity(parts);
        let mut running = 0usize;
        for (i, size) in sizes.iter().enumerate() {
            running += size;
            let g = bounds.len();
            if g + 1 < parts && running * parts >= total * (g + 1) {
                bounds.push(i + 1);
            }
        }
        bounds.push(entries.len());
        let mut groups: Vec<&[Entry]> = Vec::with_capacity(parts);
        let mut start = 0;
        for end in bounds {
            if end > start {
                groups.push(entries.get(start..end).unwrap_or(&[]));
            }
            start = end;
        }
        if groups.iter().all(|g| leaf_size(g) <= PAGE_SIZE) {
            return Ok(groups.into_iter().map(<[Entry]>::to_vec).collect());
        }
        if parts >= entries.len() {
            bail_coded!(ErrorCode::Int049, "an entry does not fit in a leaf");
        }
        parts += 1;
    }
}

/// Write `entries` as one leaf at a new page id.
fn write_leaf(
    entries: &[Entry],
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
) -> Result<u64> {
    let page = encode_leaf(entries)?;
    let id = alloc.alloc(&*backend, cache)?;
    alloc.write(backend, cache, id, page)?;
    Ok(id)
}

/// Write sorted, non-empty `entries` as leaves; returns their infos.
fn emit_leaves(
    entries: Vec<Entry>,
    out: &mut Vec<NodeInfo>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
) -> Result<()> {
    for group in pack_leaves(entries)? {
        let (Some((first, _)), Some((last, _))) = (group.first(), group.last()) else {
            continue;
        };
        let (first, last) = (first.clone(), last.clone());
        let id = write_leaf(&group, backend, cache, alloc)?;
        out.push(NodeInfo { id, first, last });
    }
    Ok(())
}

/// Build internal levels bottom-up over `level` (nodes in key order) and return
/// the root page id. A single node is its own root.
fn build_internal_levels(
    mut level: Vec<NodeInfo>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
) -> Result<u64> {
    loop {
        match level.len() {
            0 => bail_coded!(ErrorCode::Int049, "build_internal_levels: no nodes"),
            1 => {
                return level
                    .pop()
                    .map(|n| n.id)
                    .ok_or_else(|| err_coded!(ErrorCode::Int049));
            }
            _ => {}
        }
        let mut next: Vec<NodeInfo> = Vec::new();
        let mut nodes = level.into_iter().peekable();
        while let Some(first) = nodes.next() {
            let mut children = vec![first.id];
            let mut seps: Vec<Vec<u8>> = Vec::new();
            let mut size = internal_size(std::iter::empty());
            let (node_first, mut node_last) = (first.first, first.last);
            while let Some(n) = nodes.peek() {
                let sep = shortest_separator(&node_last, &n.first)?;
                let grown = size + internal_size([sep.as_slice()]) - internal_size([]);
                if grown > PAGE_FILL_BYTES && !seps.is_empty() {
                    break;
                }
                let Some(n) = nodes.next() else { break };
                size = grown;
                seps.push(sep);
                children.push(n.id);
                node_last = n.last;
            }
            let page = encode_internal(&children, &seps)?;
            let id = alloc.alloc(&*backend, cache)?;
            alloc.write(backend, cache, id, page)?;
            next.push(NodeInfo {
                id,
                first: node_first,
                last: node_last,
            });
        }
        level = next;
    }
}

/// Build a B+tree from sorted entries with distinct keys. Pages come from
/// `alloc` and are stamped with its generation. An empty input is one empty
/// leaf. Returns the root page id.
pub fn build_btree(
    sorted: Vec<Entry>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
) -> Result<u64> {
    check_sorted(&sorted)?;
    if sorted.is_empty() {
        return write_leaf(&[], backend, cache, alloc);
    }
    let mut leaves = Vec::new();
    emit_leaves(sorted, &mut leaves, backend, cache, alloc)?;
    build_internal_levels(leaves, backend, cache, alloc)
}

fn check_sorted(entries: &[Entry]) -> Result<()> {
    for w in entries.windows(2) {
        if let [(a, _), (b, _)] = w
            && a >= b
        {
            bail_coded!(ErrorCode::Int049, "entries not sorted and distinct");
        }
    }
    Ok(())
}

/// Merge two sorted entry lists. An entry present in both (same key and value)
/// is kept once; the same key with different values is INT-049.
fn merge_entries(old: Vec<Entry>, new: Vec<Entry>) -> Result<Vec<Entry>> {
    let mut out = Vec::with_capacity(old.len() + new.len());
    let mut a = old.into_iter().peekable();
    let mut b = new.into_iter().peekable();
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (None, None) => return Ok(out),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some((ka, va)), Some((kb, vb))) => match ka.cmp(kb) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => {
                    if va != vb {
                        bail_coded!(ErrorCode::Int049, "one key with two values");
                    }
                    b.next();
                    true
                }
            },
        };
        let next = if take_a { a.next() } else { b.next() };
        out.extend(next);
    }
}

// ─── Copy-on-write insert (spec §8.3) ────────────────────────────────────────

/// A node written by a copy-on-write insert, standing in for one child slot of
/// its parent: the separator that goes before it in the parent (`None` for the
/// first piece, which keeps the parent's existing separator) and its page id.
struct Piece {
    sep: Option<Vec<u8>>,
    id: u64,
}

/// Write internal nodes for `elems`, `(separator before, child)` pairs in key
/// order (the first separator is ignored). One node if they fit in a page,
/// otherwise several of about [`PAGE_FILL_BYTES`] each, balanced by size; the
/// separator at each cut moves up as the next piece's separator.
fn write_internal_pieces(
    elems: Vec<(Option<Vec<u8>>, u64)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
) -> Result<Vec<Piece>> {
    let sep_size = |s: &Option<Vec<u8>>| {
        s.as_ref()
            .map_or(0, |s| internal_size([s.as_slice()]) - internal_size([]))
    };
    let sizes: Vec<usize> = elems
        .iter()
        .enumerate()
        .map(|(i, (s, _))| if i == 0 { 0 } else { sep_size(s) })
        .collect();
    let total: usize = sizes.iter().sum();
    let body = internal_size([]);
    let mut parts = if body + total <= PAGE_SIZE {
        1
    } else {
        total.div_ceil(PAGE_FILL_BYTES - body).max(2)
    };
    // Group `g` ends where the running size reaches (g + 1) / parts of the
    // total. A group's first separator moves up, so it does not count.
    let groups = loop {
        let mut bounds = Vec::with_capacity(parts);
        let mut running = 0usize;
        for (i, size) in sizes.iter().enumerate() {
            let g = bounds.len();
            if i > 0 && g + 1 < parts && (running + size) * parts > total * (g + 1) {
                bounds.push(i);
            }
            running += size;
        }
        bounds.push(elems.len());
        let mut start = 0;
        let mut fits = true;
        for &end in &bounds {
            let seps = sizes
                .get(start + 1..end)
                .unwrap_or(&[])
                .iter()
                .sum::<usize>();
            fits &= body + seps <= PAGE_SIZE;
            start = end;
        }
        if fits {
            let mut groups: Vec<Vec<(Option<Vec<u8>>, u64)>> = Vec::with_capacity(bounds.len());
            let mut it = elems.into_iter();
            let mut start = 0;
            for end in bounds {
                groups.push(it.by_ref().take(end - start).collect());
                start = end;
            }
            break groups;
        }
        if parts >= elems.len() {
            bail_coded!(ErrorCode::Int049, "a separator does not fit in a node");
        }
        parts += 1;
    };
    let mut pieces = Vec::with_capacity(groups.len());
    for (g, group) in groups.into_iter().enumerate() {
        let mut seps = Vec::with_capacity(group.len());
        let mut children = Vec::with_capacity(group.len());
        let mut up = None;
        for (i, (sep, child)) in group.into_iter().enumerate() {
            if i == 0 {
                up = sep;
            } else {
                seps.push(
                    sep.ok_or_else(|| err_coded!(ErrorCode::Int049, "child without separator"))?,
                );
            }
            children.push(child);
        }
        let page = encode_internal(&children, &seps)?;
        let id = alloc.alloc(&*backend, cache)?;
        alloc.write(backend, cache, id, page)?;
        pieces.push(Piece {
            sep: if g == 0 { None } else { up },
            id,
        });
    }
    Ok(pieces)
}

/// Insert sorted `entries` below `page_id`, copy-on-write. Returns the pieces
/// that replace the node; the node itself goes into `freed`.
fn insert_node(
    page_id: u64,
    entries: Vec<Entry>,
    depth: usize,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
    freed: &mut Vec<u64>,
) -> Result<Vec<Piece>> {
    if depth > MAX_TREE_DEPTH {
        bail_coded!(
            ErrorCode::Int049,
            format!("tree deeper than {MAX_TREE_DEPTH} at page_id={page_id}")
        );
    }
    let page = cache.get_or_load(page_id, &*backend)?;
    freed.push(page_id);
    match page.first().copied() {
        Some(PAGE_TYPE_LEAF) => {
            let merged = merge_entries(decode_leaf(&page[..])?, entries)?;
            let mut pieces = Vec::new();
            let mut prev_last: Option<Vec<u8>> = None;
            for group in pack_leaves(merged)? {
                let (Some((first, _)), Some((last, _))) = (group.first(), group.last()) else {
                    continue;
                };
                let sep = match &prev_last {
                    Some(p) => Some(shortest_separator(p, first)?),
                    None => None,
                };
                prev_last = Some(last.clone());
                let id = write_leaf(&group, backend, cache, alloc)?;
                pieces.push(Piece { sep, id });
            }
            Ok(pieces)
        }
        Some(PAGE_TYPE_INTERNAL) => {
            let node = Internal::new(&page[..])?;
            let count = node.count();
            // Route the entries: they are sorted, so each child takes a run.
            let mut runs: Vec<(usize, Vec<Entry>)> = Vec::new();
            let mut from = 0usize;
            for entry in entries {
                let c = node.route(&entry.0, from)?;
                from = c;
                match runs.last_mut() {
                    Some((last, run)) if *last == c => run.push(entry),
                    _ => runs.push((c, vec![entry])),
                }
            }
            let mut elems: Vec<(Option<Vec<u8>>, u64)> = Vec::with_capacity(count + runs.len() + 1);
            let mut runs = runs.into_iter().peekable();
            for i in 0..=count {
                let sep_before = if i == 0 {
                    None
                } else {
                    Some(node.sep(i - 1)?.to_vec())
                };
                let child = node.child(i)?;
                if runs.peek().is_some_and(|(c, _)| *c == i) {
                    let (_, run) = runs
                        .next()
                        .ok_or_else(|| err_coded!(ErrorCode::Int049, "routed run missing"))?;
                    let pieces = insert_node(child, run, depth + 1, backend, cache, alloc, freed)?;
                    for (k, piece) in pieces.into_iter().enumerate() {
                        let sep = if k == 0 {
                            sep_before.clone()
                        } else {
                            piece.sep
                        };
                        elems.push((sep, piece.id));
                    }
                } else {
                    elems.push((sep_before, child));
                }
            }
            write_internal_pieces(elems, backend, cache, alloc)
        }
        _ => bail_coded!(ErrorCode::Stg013, page_id),
    }
}

/// Insert sorted, distinct `entries` into the tree at `root`, copy-on-write
/// (spec §8.3): only the touched leaves and their paths to the root are
/// rewritten, at ids from `alloc`; every page they replace goes into `freed`.
/// Untouched subtrees are shared with the old tree, which stays readable. A
/// root split adds a level. Root 0 (no tree) is a bulk build. Returns the new
/// root.
pub fn cow_insert(
    root: u64,
    entries: Vec<Entry>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    alloc: &mut PageAllocator,
    freed: &mut Vec<u64>,
) -> Result<u64> {
    check_sorted(&entries)?;
    if root == 0 {
        return build_btree(entries, backend, cache, alloc);
    }
    if entries.is_empty() {
        return Ok(root);
    }
    let mut pieces = insert_node(root, entries, 0, backend, cache, alloc, freed)?;
    while pieces.len() > 1 {
        let elems = pieces.into_iter().map(|p| (p.sep, p.id)).collect();
        pieces = write_internal_pieces(elems, backend, cache, alloc)?;
    }
    pieces
        .pop()
        .map(|p| p.id)
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "insert produced no root"))
}

// ─── Reading ─────────────────────────────────────────────────────────────────

/// Collect the leaf pages of the tree at `root`, in key order, each checked by
/// a full decode. Every node visited is also pushed onto `nodes_out` (when
/// given), so a checkpoint can free the whole old tree.
#[cfg(test)]
pub fn collect_leaf_pages(
    root: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
    mut nodes_out: Option<&mut Vec<u64>>,
) -> Result<Vec<Arc<Vec<u8>>>> {
    // A tree cannot have more nodes than the file has pages; more visits
    // means a cycle in the child pointers.
    let max_visits = backend.page_count()?;
    let mut visits: u64 = 0;
    let mut leaves = Vec::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((page_id, depth)) = stack.pop() {
        visits = visits.saturating_add(1);
        if depth > MAX_TREE_DEPTH || visits > max_visits {
            bail_coded!(
                ErrorCode::Int049,
                format!("collect_leaf_pages: cycle or corruption at page_id={page_id}")
            );
        }
        let page = cache.get_or_load(page_id, backend)?;
        if let Some(nodes) = nodes_out.as_deref_mut() {
            nodes.push(page_id);
        }
        match page.first().copied() {
            Some(PAGE_TYPE_LEAF) => {
                crate::storage::node::validate_leaf(&page[..])?;
                leaves.push(page);
            }
            Some(PAGE_TYPE_INTERNAL) => {
                let children = Internal::new(&page[..])?.children()?;
                stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
            }
            _ => bail_coded!(ErrorCode::Stg013, page_id),
        }
    }
    Ok(leaves)
}

/// One internal node on the cursor's path from the root.
struct Frame {
    page: Arc<Vec<u8>>,
    /// Index into the node's children of the subtree the cursor is in;
    /// `count` means `rightmost_child`.
    child_idx: usize,
    count: usize,
}

impl Frame {
    fn node(&self) -> Result<Internal<'_>> {
        Internal::new(&self.page[..])
    }
}

/// Forward cursor over a B+tree's entries in key order.
///
/// Holds the path from the root as a stack of internal nodes, so moving to the
/// next leaf or seeking forward re-reads only the pages below the lowest
/// ancestor that still covers the target.
pub(crate) struct LeafCursor<'a> {
    backend: &'a dyn StorageBackend,
    cache: &'a PageCache,
    stack: Vec<Frame>,
    entries: Vec<Entry>,
    pos: usize,
    /// Set once the cursor has moved past the last leaf.
    done: bool,
}

impl<'a> LeafCursor<'a> {
    /// A cursor positioned before the first entry `>= start` (`None`: the first).
    pub(crate) fn new(
        root: u64,
        start: Option<&[u8]>,
        backend: &'a dyn StorageBackend,
        cache: &'a PageCache,
    ) -> Result<Self> {
        let mut cursor = LeafCursor {
            backend,
            cache,
            stack: Vec::new(),
            entries: Vec::new(),
            pos: 0,
            done: false,
        };
        cursor.descend(root, start)?;
        Ok(cursor)
    }

    /// The next entry in key order, or `None` past the last one.
    pub(crate) fn next_ref(&mut self) -> Result<Option<&Entry>> {
        loop {
            if self.done {
                return Ok(None);
            }
            if self.pos < self.entries.len() {
                self.pos += 1;
                return Ok(self.entries.get(self.pos - 1));
            }
            if !self.advance_leaf()? {
                return Ok(None);
            }
        }
    }

    /// Like [`next_ref`](Self::next_ref), but clones the entry.
    #[cfg(test)]
    pub(crate) fn next(&mut self) -> Result<Option<Entry>> {
        Ok(self.next_ref()?.cloned())
    }

    /// Move forward to the first entry `>= key`. A key at or before the current
    /// position leaves the cursor where it is.
    // Only tests call this until the streaming engine's joins (#432) do.
    #[allow(dead_code)]
    pub(crate) fn seek(&mut self, key: &[u8]) -> Result<()> {
        if self.done {
            return Ok(());
        }
        if let Some((next_key, _)) = self.entries.get(self.pos)
            && key <= next_key.as_slice()
        {
            return Ok(());
        }
        // Find the lowest node whose range covers `key`. The child a frame is
        // in ends at that frame's separator `child_idx`; a rightmost child
        // inherits its parent's bound. `reroute` is the frame to route `key` in
        // again; `stack.len()` means the current leaf covers it.
        let mut reroute = self.stack.len();
        for (d, f) in self.stack.iter().enumerate().rev() {
            if f.child_idx < f.count {
                if key < f.node()?.sep(f.child_idx)? {
                    break;
                }
                reroute = d;
            }
        }
        if reroute == self.stack.len() {
            let p = self.entries.partition_point(|(k, _)| k.as_slice() < key);
            self.pos = self.pos.max(p);
            return Ok(());
        }
        self.stack.truncate(reroute + 1);
        let top = self
            .stack
            .last_mut()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "seek: empty cursor stack"))?;
        let (c, child) = {
            let node = top.node()?;
            let c = node.route(key, top.child_idx)?;
            (c, node.child(c)?)
        };
        top.child_idx = c;
        self.descend(child, Some(key))
    }

    /// Move to the start of the next leaf. Returns false past the last leaf.
    fn advance_leaf(&mut self) -> Result<bool> {
        loop {
            let Some(top) = self.stack.last_mut() else {
                self.done = true;
                self.entries.clear();
                self.pos = 0;
                return Ok(false);
            };
            if top.child_idx < top.count {
                top.child_idx += 1;
                let child = top.node()?.child(top.child_idx)?;
                self.descend(child, None)?;
                return Ok(true);
            }
            self.stack.pop();
        }
    }

    /// Descend from `page_id` to the leaf that holds `key`, or to the leftmost
    /// leaf when `key` is `None`.
    fn descend(&mut self, page_id: u64, key: Option<&[u8]>) -> Result<()> {
        let mut page_id = page_id;
        loop {
            if self.stack.len() >= MAX_TREE_DEPTH {
                bail_coded!(
                    ErrorCode::Int049,
                    format!("tree deeper than {MAX_TREE_DEPTH} at page_id={page_id}")
                );
            }
            let page = self.cache.get_or_load(page_id, self.backend)?;
            match page.first().copied() {
                Some(PAGE_TYPE_LEAF) => {
                    self.entries = decode_leaf(&page[..])?;
                    self.pos = key.map_or(0, |k| {
                        self.entries.partition_point(|(e, _)| e.as_slice() < k)
                    });
                    return Ok(());
                }
                Some(PAGE_TYPE_INTERNAL) => {
                    let node = Internal::new(&page[..])?;
                    let child_idx = match key {
                        Some(k) => node.route(k, 0)?,
                        None => 0,
                    };
                    let count = node.count();
                    page_id = node.child(child_idx)?;
                    self.stack.push(Frame {
                        page,
                        child_idx,
                        count,
                    });
                }
                _ => bail_coded!(ErrorCode::Stg013, page_id),
            }
        }
    }
}

/// Every entry whose key starts with `prefix`, in key order.
pub fn prefix_scan(
    root: u64,
    prefix: &[u8],
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<Entry>> {
    let mut cursor = LeafCursor::new(root, Some(prefix), backend, cache)?;
    let mut out = Vec::new();
    while let Some((k, v)) = cursor.next_ref()? {
        if !k.starts_with(prefix) {
            break;
        }
        out.push((k.clone(), v.clone()));
    }
    Ok(out)
}

/// The value stored under `key`, if any.
pub fn get(
    root: u64,
    key: &[u8],
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Option<Vec<u8>>> {
    let mut page_id = root;
    for _ in 0..MAX_TREE_DEPTH {
        let page = cache.get_or_load(page_id, backend)?;
        match page.first().copied() {
            Some(PAGE_TYPE_LEAF) => return leaf_get(&page[..], key),
            Some(PAGE_TYPE_INTERNAL) => {
                let node = Internal::new(&page[..])?;
                page_id = node.child(node.route(key, 0)?)?;
            }
            _ => bail_coded!(ErrorCode::Stg013, page_id),
        }
    }
    bail_coded!(
        ErrorCode::Int049,
        format!("tree deeper than {MAX_TREE_DEPTH} below page_id={root}")
    )
}

/// Every entry of the tree, in key order.
#[cfg(test)]
pub fn stream_all_entries(
    root: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<Entry>> {
    prefix_scan(root, &[], backend, cache)
}

// ─── MutexStorageBackend ─────────────────────────────────────────────────────

/// Read-only [`StorageBackend`] adapter that locks `Arc<Mutex<B>>` only for the
/// duration of one [`StorageBackend::read_page`] call, so readers hold the
/// backend lock only while a cold page is read. Every other method fails.
pub(crate) struct MutexStorageBackend<B>(pub(crate) Arc<Mutex<B>>);

impl<B: StorageBackend> StorageBackend for MutexStorageBackend<B> {
    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.0
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Int050, "MutexStorageBackend"))?
            .read_page(page_id)
    }

    fn write_page(&mut self, _page_id: u64, _data: &[u8]) -> Result<()> {
        bail_coded!(ErrorCode::Int049, "MutexStorageBackend is read-only")
    }

    fn sync(&mut self) -> Result<()> {
        bail_coded!(ErrorCode::Int049, "MutexStorageBackend is read-only")
    }

    fn page_count(&self) -> Result<u64> {
        self.0
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Int050, "MutexStorageBackend"))?
            .page_count()
    }

    fn close(&mut self) -> Result<()> {
        bail_coded!(ErrorCode::Int049, "MutexStorageBackend is read-only")
    }

    fn backend_name(&self) -> &'static str {
        "mutex-adapter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::keys::{Index, KeyFact, KeyValue};
    use crate::storage::node::RIGHTMOST_CHILD_OFFSET;

    /// An EAVT key for entity `n`: real-shaped keys with shared prefixes.
    fn key(n: u64, tx: u64) -> Vec<u8> {
        KeyFact {
            e: n,
            a: 3,
            v: KeyValue::Int(i64::try_from(n % 1000).unwrap()),
            tx_count: tx,
            vf: 1_700_000_000_000,
            vt: i64::MAX,
            asserted: true,
        }
        .key(Index::Eavt)
        .unwrap()
    }

    fn entry(n: u64, tx: u64) -> Entry {
        (key(n, tx), Vec::new())
    }

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
        fn below(&mut self, n: u64) -> usize {
            usize::try_from(self.next() % n).unwrap()
        }
    }

    /// `n` entries with random entities; `counter` keeps keys unique.
    fn random_entries(rng: &mut Rng, n: usize, counter: &mut u64) -> Vec<Entry> {
        let mut v: Vec<Entry> = (0..n)
            .map(|_| {
                *counter += 1;
                entry(rng.next() >> 20, *counter)
            })
            .collect();
        v.sort();
        v.dedup_by(|a, b| a.0 == b.0);
        v
    }

    fn sorted_union(a: &[Entry], b: &[Entry]) -> Vec<Entry> {
        let mut v: Vec<Entry> = a.iter().chain(b).cloned().collect();
        v.sort();
        v.dedup();
        v
    }

    fn build_at(
        entries: Vec<Entry>,
        backend: &mut dyn StorageBackend,
        cache: &PageCache,
        start: u64,
    ) -> (u64, u64) {
        let mut alloc = PageAllocator::new(Vec::new(), start, 1);
        let root = build_btree(entries, backend, cache, &mut alloc).unwrap();
        (root, alloc.next_append())
    }

    /// Stream equality, no empty leaf in a non-empty tree, and prefix scans
    /// that agree with the expected set.
    fn assert_tree_exact(
        root: u64,
        backend: &MemoryBackend,
        cache: &PageCache,
        expected: &[Entry],
        rng: &mut Rng,
    ) {
        let got = stream_all_entries(root, backend, cache).unwrap();
        assert_eq!(got.len(), expected.len(), "entry count differs");
        assert!(got == expected, "streamed entries differ from expected");
        let leaves = collect_leaf_pages(root, backend, cache, None).unwrap();
        if expected.is_empty() {
            assert_eq!(leaves.len(), 1, "empty tree is one empty leaf");
        } else {
            for page in &leaves {
                assert!(
                    !decode_leaf(&page[..]).unwrap().is_empty(),
                    "empty leaf in tree"
                );
            }
        }
        for _ in 0..20 {
            if expected.is_empty() {
                break;
            }
            let (k, v) = &expected[rng.below(u64::try_from(expected.len()).unwrap())];
            assert_eq!(
                get(root, k, backend, cache).unwrap().as_ref(),
                Some(v),
                "get"
            );
            let prefix = &k[..k.len().min(3)];
            let want: Vec<Entry> = expected
                .iter()
                .filter(|(e, _)| e.starts_with(prefix))
                .cloned()
                .collect();
            assert!(
                prefix_scan(root, prefix, backend, cache).unwrap() == want,
                "prefix scan differs"
            );
        }
    }

    fn leaf_ids(root: u64, backend: &dyn StorageBackend, cache: &PageCache) -> Vec<u64> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let page = cache.get_or_load(id, backend).unwrap();
            if page[0] == PAGE_TYPE_LEAF {
                ids.push(id);
            } else {
                let children = Internal::new(&page[..]).unwrap().children().unwrap();
                stack.extend(children.into_iter().rev());
            }
        }
        ids
    }

    fn leaf_keys(id: u64, backend: &dyn StorageBackend, cache: &PageCache) -> Vec<Vec<u8>> {
        let page = cache.get_or_load(id, backend).unwrap();
        decode_leaf(&page[..])
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    fn multi_leaf_tree() -> (MemoryBackend, PageCache, u64, Vec<u64>) {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries: Vec<Entry> = (0..2000).map(|n| entry(n, n)).collect();
        let (root, _) = build_at(entries, &mut backend, &cache, 2);
        let ids = leaf_ids(root, &backend, &cache);
        assert!(ids.len() >= 3, "fixture needs several leaves");
        (backend, cache, root, ids)
    }

    /// Replace page `id` with `page`, resealed so only its content is wrong.
    fn overwrite_page(backend: &mut MemoryBackend, cache: &PageCache, id: u64, page: Vec<u8>) {
        let mut page = page;
        crate::storage::page::seal(&mut page, id, 1).unwrap();
        backend.write_page(id, &page).unwrap();
        cache.put_dirty(id, page);
    }

    #[test]
    fn build_empty_and_single() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let (root, next) = build_at(Vec::new(), &mut backend, &cache, 2);
        assert_eq!((root, next), (2, 3), "empty tree is one leaf");
        assert!(
            stream_all_entries(root, &backend, &cache)
                .unwrap()
                .is_empty()
        );
        let (root, _) = build_at(vec![entry(1, 1)], &mut backend, &cache, 3);
        assert_eq!(
            stream_all_entries(root, &backend, &cache).unwrap(),
            vec![entry(1, 1)]
        );
    }

    #[test]
    fn build_spans_levels_and_seals_every_page() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let entries: Vec<Entry> = (0..60_000).map(|n| entry(n * 7, n)).collect();
        let (root, next) = build_at(entries.clone(), &mut backend, &cache, 2);
        let root_page = backend.read_page(root).unwrap();
        let below = Internal::new(&root_page).unwrap().child(0).unwrap();
        assert_eq!(
            backend.read_page(below).unwrap()[0],
            PAGE_TYPE_INTERNAL,
            "60k entries need depth 3"
        );
        for id in 2..next {
            let page = backend.read_page(id).unwrap();
            crate::storage::page::verify(&page, id, 1).unwrap();
            assert!(page.len() == PAGE_SIZE);
        }
        assert!(stream_all_entries(root, &backend, &cache).unwrap() == entries);
    }

    #[test]
    fn build_rejects_unsorted_or_duplicate_input() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(0);
        let mut alloc = PageAllocator::new(Vec::new(), 2, 1);
        let dup = vec![entry(1, 1), entry(1, 1)];
        assert!(build_btree(dup, &mut backend, &cache, &mut alloc).is_err());
        let unsorted = vec![entry(2, 1), entry(1, 1)];
        assert!(build_btree(unsorted, &mut backend, &cache, &mut alloc).is_err());
    }

    #[test]
    fn bulk_leaves_are_filled_to_target() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries: Vec<Entry> = (0..20_000).map(|n| entry(n, n)).collect();
        let (root, _) = build_at(entries, &mut backend, &cache, 2);
        let leaves = collect_leaf_pages(root, &backend, &cache, None).unwrap();
        for page in &leaves[..leaves.len() - 1] {
            let size = leaf_size(&decode_leaf(&page[..]).unwrap());
            assert!(size <= PAGE_SIZE, "leaf fits");
            assert!(size >= PAGE_FILL_BYTES / 2, "leaf at least half the target");
        }
    }

    #[test]
    fn collect_leaf_pages_rejects_cycles_and_corrupt_leaves() {
        let (mut backend, cache, root, ids) = multi_leaf_tree();
        let mut page = cache.get_or_load(root, &backend).unwrap().to_vec();
        assert_eq!(page[0], PAGE_TYPE_INTERNAL, "fixture root must be internal");
        page[RIGHTMOST_CHILD_OFFSET..RIGHTMOST_CHILD_OFFSET + 8]
            .copy_from_slice(&root.to_le_bytes());
        overwrite_page(&mut backend, &cache, root, page);
        assert!(
            collect_leaf_pages(root, &backend, &cache, None).is_err(),
            "cycle"
        );
        assert!(
            stream_all_entries(root, &backend, &cache).is_err(),
            "cursor cycle"
        );

        let (mut backend, cache, root, ids2) = multi_leaf_tree();
        let mut page = cache.get_or_load(ids2[1], &backend).unwrap().to_vec();
        page[2..4].copy_from_slice(&u16::MAX.to_le_bytes());
        overwrite_page(&mut backend, &cache, ids2[1], page);
        assert!(
            collect_leaf_pages(root, &backend, &cache, None).is_err(),
            "a count beyond the data is rejected before the leaf is copied"
        );
        assert!(ids.len() >= 3);
    }

    #[test]
    fn child_pointing_at_non_btree_page_is_stg_013() {
        let (mut backend, cache, root, ids) = multi_leaf_tree();
        let mut page = cache.get_or_load(ids[0], &backend).unwrap().to_vec();
        page[0] = crate::storage::page::PAGE_TYPE_FREELIST;
        overwrite_page(&mut backend, &cache, ids[0], page);
        let err = stream_all_entries(root, &backend, &cache).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-013");
    }

    #[test]
    fn dict_shaped_entries_with_values_round_trip() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries: Vec<Entry> = (0u64..3000)
            .map(|n| {
                let mut k = vec![0x04];
                crate::storage::keys::put_uint(&mut k, n);
                (k, vec![b'x'; usize::try_from(n % 1024).unwrap()])
            })
            .collect();
        let (root, _) = build_at(entries.clone(), &mut backend, &cache, 2);
        assert!(stream_all_entries(root, &backend, &cache).unwrap() == entries);
        for (k, v) in entries.iter().step_by(97) {
            assert_eq!(get(root, k, &backend, &cache).unwrap().as_ref(), Some(v));
        }
        assert_eq!(get(root, &[0x05], &backend, &cache).unwrap(), None);
    }

    // ─── LeafCursor ──────────────────────────────────────────────────────────

    /// `MemoryBackend` that counts `read_page` calls, for seek cost tests.
    struct CountingBackend {
        inner: MemoryBackend,
        reads: std::sync::atomic::AtomicU64,
    }

    impl CountingBackend {
        fn reads(&self) -> u64 {
            self.reads.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn reset(&self) {
            self.reads.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl StorageBackend for CountingBackend {
        fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
            self.inner.write_page(page_id, data)
        }
        fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.read_page(page_id)
        }
        fn sync(&mut self) -> Result<()> {
            self.inner.sync()
        }
        fn page_count(&self) -> Result<u64> {
            self.inner.page_count()
        }
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }
        fn backend_name(&self) -> &'static str {
            "counting"
        }
    }

    /// Entities `0, 2, 4, ...` so every odd entity is absent.
    fn even_entries(n: u64) -> Vec<Entry> {
        (0..n).map(|i| entry(i * 2, 1)).collect()
    }

    fn key_for(entity: u64) -> Vec<u8> {
        key(entity, 1)
    }

    fn drain(cursor: &mut LeafCursor<'_>) -> Vec<Entry> {
        let mut v = Vec::new();
        while let Some(e) = cursor.next().unwrap() {
            v.push(e);
        }
        v
    }

    #[test]
    fn cursor_full_scan_matches_input() {
        for n in [0u64, 1, 10, 500, 20_000] {
            let mut backend = MemoryBackend::new();
            let cache = PageCache::new(4096);
            let entries = even_entries(n);
            let (root, _) = build_at(entries.clone(), &mut backend, &cache, 2);
            let mut cursor = LeafCursor::new(root, None, &backend, &cache).unwrap();
            assert!(
                drain(&mut cursor) == entries,
                "cursor scan differs from input"
            );
        }
    }

    #[test]
    fn cursor_start_key_positions_at_first_ge() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries = even_entries(5_000);
        let (root, _) = build_at(entries.clone(), &mut backend, &cache, 2);
        let leaves = leaf_ids(root, &backend, &cache);
        assert!(leaves.len() >= 3, "fixture needs several leaves");
        let second_leaf_first = leaf_keys(leaves[1], &backend, &cache)[0].clone();
        let starts = [
            key_for(0),
            key_for(1),
            key_for(500),
            key_for(501),
            second_leaf_first,
            key_for(9_998),
            key_for(9_999),
        ];
        for start in &starts {
            let mut cursor = LeafCursor::new(root, Some(start), &backend, &cache).unwrap();
            let want = entries
                .get(entries.partition_point(|(e, _)| e < start))
                .cloned();
            assert!(
                cursor.next().unwrap() == want,
                "first entry after start differs"
            );
        }
    }

    #[test]
    fn cursor_seek_matches_fresh_descent_random() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let (root, _) = build_at(even_entries(20_000), &mut backend, &cache, 2);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut targets: Vec<u64> = (0..2_000).map(|_| rng.next() % 40_010).collect();
        targets.sort_unstable();
        let mut cursor = LeafCursor::new(root, None, &backend, &cache).unwrap();
        for t in targets {
            let k = key_for(t);
            cursor.seek(&k).unwrap();
            let got = cursor.next().unwrap();
            let want = LeafCursor::new(root, Some(&k), &backend, &cache)
                .unwrap()
                .next()
                .unwrap();
            assert!(got == want, "seek differs from a fresh descent");
            cursor = LeafCursor::new(root, Some(&k), &backend, &cache).unwrap();
        }
    }

    #[test]
    fn cursor_seek_sequence_without_reset_visits_ascending_entries() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries = even_entries(20_000);
        let (root, _) = build_at(entries.clone(), &mut backend, &cache, 2);
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let mut cursor = LeafCursor::new(root, None, &backend, &cache).unwrap();
        let mut consumed = 0usize;
        let mut target = 0u64;
        while consumed < entries.len() {
            target += rng.next() % 200;
            let k = key_for(target);
            cursor.seek(&k).unwrap();
            let expect = consumed.max(entries.partition_point(|(e, _)| *e < k));
            assert!(
                cursor.next().unwrap() == entries.get(expect).cloned(),
                "seek/next differs"
            );
            consumed = expect + 1;
        }
        assert!(
            cursor.next().unwrap().is_none(),
            "cursor ends after the last entry"
        );
    }

    #[test]
    fn cursor_seek_within_leaf_sibling_across_subtrees_gap_and_end() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let entries = even_entries(20_000);
        let (root, _) = build_at(entries, &mut backend, &cache, 2);
        let leaves = leaf_ids(root, &backend, &cache);
        assert!(leaves.len() > 20, "fixture needs many leaves");
        let l0 = leaf_keys(leaves[0], &backend, &cache);
        let sibling = leaf_keys(leaves[1], &backend, &cache)[0].clone();
        let far = leaf_keys(leaves[leaves.len() - 2], &backend, &cache)[0].clone();
        let mut cursor = LeafCursor::new(root, Some(&l0[0]), &backend, &cache).unwrap();
        for target in [l0.last().unwrap(), &sibling, &far] {
            cursor.seek(target).unwrap();
            assert!(
                cursor.next().unwrap().unwrap().0 == *target,
                "seek lands on the target"
            );
        }

        // A key in the gap after a leaf's last entry lands on the next leaf.
        let l2_last = leaf_keys(leaves[2], &backend, &cache)
            .last()
            .unwrap()
            .clone();
        let l3_first = leaf_keys(leaves[3], &backend, &cache)[0].clone();
        let mut gap = l2_last.clone();
        gap.push(0);
        assert!(gap > l2_last && gap < l3_first);
        let mut cursor = LeafCursor::new(root, None, &backend, &cache).unwrap();
        cursor.seek(&gap).unwrap();
        assert!(cursor.next().unwrap().unwrap().0 == l3_first, "gap seek");

        // Backwards is a no-op; past the end stays at the end.
        cursor.seek(&key_for(12_000)).unwrap();
        cursor.seek(&key_for(10)).unwrap();
        assert!(
            cursor.next().unwrap().unwrap().0 == key_for(12_000),
            "backward seek"
        );
        cursor.seek(&key_for(1_000_000)).unwrap();
        assert!(cursor.next().unwrap().is_none(), "past the end");
        cursor.seek(&key_for(2_000_000)).unwrap();
        assert!(cursor.next().unwrap().is_none(), "still at the end");
    }

    #[test]
    fn cursor_seek_to_next_leaf_reads_one_page() {
        let mut backend = CountingBackend {
            inner: MemoryBackend::new(),
            reads: std::sync::atomic::AtomicU64::new(0),
        };
        let cache = PageCache::new(0);
        let (root, _) = build_at(even_entries(20_000), &mut backend, &cache, 2);
        let leaves = leaf_ids(root, &backend, &cache);
        let first = leaf_keys(leaves[0], &backend, &cache)[0].clone();
        let next_first = leaf_keys(leaves[1], &backend, &cache)[0].clone();
        let mut cursor = LeafCursor::new(root, Some(&first), &backend, &cache).unwrap();
        backend.reset();
        cursor.seek(&next_first).unwrap();
        assert_eq!(
            backend.reads(),
            1,
            "seek to the sibling reads only that leaf"
        );
        assert!(cursor.next().unwrap().unwrap().0 == next_first);
    }

    #[test]
    #[cfg(not(target_os = "wasi"))]
    fn concurrent_prefix_scans_agree() {
        use std::sync::Barrier;
        use std::thread;
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(16);
        let entries = even_entries(5_000);
        let (root, _) = build_at(entries, &mut backend, &cache, 2);
        let shared = Arc::new(Mutex::new(backend));
        let cache = Arc::new(cache);
        let barrier = Arc::new(Barrier::new(8));
        let prefix = crate::storage::keys::entity_prefix(4_000);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (s, c, b, p) = (
                    Arc::clone(&shared),
                    Arc::clone(&cache),
                    Arc::clone(&barrier),
                    prefix.clone(),
                );
                thread::spawn(move || {
                    b.wait();
                    let adapter = MutexStorageBackend(s);
                    prefix_scan(root, &p, &adapter, &c).unwrap().len()
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), 1, "entity 4000 has one entry");
        }
    }

    #[test]
    fn largest_entries_fit() {
        // The largest DICT entry (tag + 1024-byte ident key) beside others.
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let entries: Vec<Entry> = (0u8..20)
            .map(|i| {
                let mut k = vec![0x03, i];
                k.extend(vec![b'k'; crate::storage::keys::MAX_IDENT_BYTES - 1]);
                (k, crate::storage::keys::uint_bytes(u64::from(i)))
            })
            .collect();
        let (root, _) = build_at(entries.clone(), &mut backend, &cache, 2);
        assert!(stream_all_entries(root, &backend, &cache).unwrap() == entries);
    }

    // ─── Copy-on-write insert ────────────────────────────────────────────────

    /// Every node id of the tree at `root`.
    fn node_ids(root: u64, backend: &dyn StorageBackend, cache: &PageCache) -> BTreeSet<u64> {
        let mut nodes = Vec::new();
        collect_leaf_pages(root, backend, cache, Some(&mut nodes)).unwrap();
        nodes.into_iter().collect()
    }

    use std::collections::BTreeSet;

    /// Build `committed`, then insert `pending` copy-on-write, and check: the
    /// result is the sorted union; the old root still streams `committed`
    /// unchanged; `freed` is exactly the old nodes the new tree no longer uses;
    /// and the new tree writes no page of the old tree.
    fn cow_check(committed: &[Entry], pending: &[Entry], rng: &mut Rng) {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let (old_root, next) = build_at(committed.to_vec(), &mut backend, &cache, 2);
        let old_nodes = node_ids(old_root, &backend, &cache);
        let mut alloc = PageAllocator::new(Vec::new(), next, 2);
        let mut freed = Vec::new();
        let root = cow_insert(
            old_root,
            pending.to_vec(),
            &mut backend,
            &cache,
            &mut alloc,
            &mut freed,
        )
        .unwrap();
        let expected = sorted_union(committed, pending);
        assert_tree_exact(root, &backend, &cache, &expected, rng);
        assert!(
            stream_all_entries(old_root, &backend, &cache).unwrap() == committed,
            "the old tree is unchanged"
        );
        let new_nodes = node_ids(root, &backend, &cache);
        let freed: BTreeSet<u64> = freed.into_iter().collect();
        let dropped: BTreeSet<u64> = old_nodes.difference(&new_nodes).copied().collect();
        if pending.is_empty() {
            assert_eq!(root, old_root);
            assert!(freed.is_empty());
        } else {
            assert!(freed == dropped, "freed is exactly the replaced old nodes");
        }
        for id in new_nodes.difference(&old_nodes) {
            assert!(*id >= next, "new nodes are new pages");
        }
    }

    #[test]
    fn cow_insert_matches_merge_random() {
        let mut rng = Rng(0xC0FE);
        let mut ctr = 0;
        for _ in 0..40 {
            let (nc, np) = (rng.below(6000), rng.below(400));
            let committed = random_entries(&mut rng, nc, &mut ctr);
            let pending = random_entries(&mut rng, np, &mut ctr);
            cow_check(&committed, &pending, &mut rng);
        }
    }

    #[test]
    fn cow_insert_edges_splits_and_root_growth() {
        let mut rng = Rng(77);
        // Below the first key and above the last.
        let committed: Vec<Entry> = (1000..3000).map(|n| entry(n, n)).collect();
        let pending = vec![entry(1, 1), entry(2, 2), entry(9_000, 9), entry(9_001, 9)];
        cow_check(&committed, &pending, &mut rng);
        // Many entries into one leaf: it splits into balanced leaves.
        let committed: Vec<Entry> = (0..2000).map(|n| entry(n * 10_000, n)).collect();
        let pending: Vec<Entry> = (1..3000).map(|n| entry(n, 100_000 + n)).collect();
        cow_check(&committed, &pending, &mut rng);
        // A single-leaf root grows to depth 3; an empty tree is a bulk build.
        let pending: Vec<Entry> = (0..60_000).map(|n| entry(n * 3, n)).collect();
        cow_check(&[entry(1, 1)], &pending, &mut rng);
        cow_check(&[], &pending[..10], &mut rng);
        cow_check(&committed, &[], &mut rng);
        // Shared entries are kept once.
        cow_check(&committed, &committed[5..50], &mut rng);
    }

    #[test]
    fn cow_insert_into_root_zero_is_a_bulk_build() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(64);
        let mut alloc = PageAllocator::new(Vec::new(), 2, 1);
        let mut freed = Vec::new();
        let entries: Vec<Entry> = (0..500).map(|n| entry(n, n)).collect();
        let root = cow_insert(
            0,
            entries.clone(),
            &mut backend,
            &cache,
            &mut alloc,
            &mut freed,
        )
        .unwrap();
        assert!(freed.is_empty());
        assert!(stream_all_entries(root, &backend, &cache).unwrap() == entries);
    }

    #[test]
    fn cow_insert_large_separators_split_internal_nodes() {
        // DICT-shaped keys of about 1 KB: internal nodes hold few separators
        // and split often.
        let key = |n: u64| {
            let mut k = vec![0x03];
            k.extend(format!("{n:08}").into_bytes());
            k.extend(vec![b'k'; 1000]);
            k
        };
        let mut rng = Rng(5);
        let committed: Vec<Entry> = (0..300).map(|n| (key(n * 2), vec![1])).collect();
        let pending: Vec<Entry> = (0..300).map(|n| (key(n * 2 + 1), vec![2])).collect();
        cow_check(&committed, &pending, &mut rng);
    }

    #[test]
    fn cow_single_inserts_do_not_fragment_leaves() {
        let mut rng = Rng(23);
        let mut ctr = 0;
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let mut expected = random_entries(&mut rng, 3000, &mut ctr);
        let (mut root, next) = build_at(expected.clone(), &mut backend, &cache, 2);
        let bulk = collect_leaf_pages(root, &backend, &cache, None)
            .unwrap()
            .len();
        let mut alloc = PageAllocator::new(Vec::new(), next, 2);
        for _ in 0..600 {
            let pending = random_entries(&mut rng, 1, &mut ctr);
            let mut freed = Vec::new();
            root = cow_insert(
                root,
                pending.clone(),
                &mut backend,
                &cache,
                &mut alloc,
                &mut freed,
            )
            .unwrap();
            expected = sorted_union(&expected, &pending);
        }
        assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
        let leaves = collect_leaf_pages(root, &backend, &cache, None)
            .unwrap()
            .len();
        assert!(
            leaves <= bulk * 5 / 2,
            "leaf count grew from {bulk} to {leaves}"
        );
    }

    #[test]
    fn cow_insert_writes_only_the_touched_paths() {
        // One entry into a depth-3 tree writes one leaf and two internal nodes.
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let committed: Vec<Entry> = (0..60_000).map(|n| entry(n * 2, n)).collect();
        let (root, next) = build_at(committed, &mut backend, &cache, 2);
        let mut alloc = PageAllocator::new(Vec::new(), next, 2);
        let mut freed = Vec::new();
        cow_insert(
            root,
            vec![entry(77_777, 1)],
            &mut backend,
            &cache,
            &mut alloc,
            &mut freed,
        )
        .unwrap();
        assert_eq!(alloc.next_append() - next, 3, "leaf + 2 internal nodes");
        assert_eq!(freed.len(), 3);
    }
}
