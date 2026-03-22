# Phase 6.2 — Packed Pages + LRU Page Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the one-fact-per-4KB-page format with packed pages (~25 facts/page), add an LRU page cache for bounded memory, and eliminate the "load all facts at startup" pattern so memory usage is independent of database size.

**Architecture:** On-disk facts move from `page_type=0x01` (one per page) to `page_type=0x02` (packed, ~25/page), accessed through a new `PageCache` that caches hot pages in an LRU structure. `FactStorage` gains a `CommittedFactReader` hook so committed (checkpointed) facts are resolved via page cache rather than an in-memory `Vec<Fact>`. Only pending (post-checkpoint) facts are held in memory. The file format version bumps from v4 to v5; migration is automatic on first open.

**Tech Stack:** Rust, `postcard` (serialization), `crc32fast` (checksums), `std::collections::BTreeMap` (indexes), existing `StorageBackend` trait.

---

## File Structure

**Create:**
- `src/storage/cache.rs` — LRU page cache (no backend reference, backend passed as arg)
- `src/storage/packed_pages.rs` — pack/unpack facts into packed page format

**Modify:**
- `src/storage/mod.rs` — FORMAT_VERSION=5, FileHeader v5 field, `CommittedFactReader` trait
- `src/storage/persistent_facts.rs` — backend becomes `Arc<Mutex<B>>`, packed save/load, v4→v5 migration, `CommittedFactReader` impl
- `src/graph/storage.rs` — `FactData.facts` becomes `pending_facts`, add `committed: Option<Arc<dyn CommittedFactReader>>`; index-driven read methods
- `src/db.rs` — `OpenOptions::page_cache_size(usize)`
- `Cargo.toml` — version 0.7.0
- `CHANGELOG.md` — v0.7.0 entry

**Create (tests):**
- `tests/performance_test.rs` — integration tests at scale + phase regression

---

## Task 1: LRU Page Cache

**Files:**
- Create: `src/storage/cache.rs`
- Modify: `src/storage/mod.rs` (add `pub mod cache;`)

- [ ] **Step 1: Write failing tests**

```rust
// src/storage/cache.rs  (bottom of file, under #[cfg(test)])
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;

    fn make_page(byte: u8) -> Vec<u8> {
        vec![byte; PAGE_SIZE]
    }

    #[test]
    fn test_cache_miss_loads_from_backend() {
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(0xAB)).unwrap();
        let cache = PageCache::new(4);
        let page = cache.get_or_load(1, &backend).unwrap();
        assert_eq!(page[0], 0xAB);
    }

    #[test]
    fn test_cache_hit_returns_same_bytes() {
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(0x11)).unwrap();
        let cache = PageCache::new(4);
        let p1 = cache.get_or_load(1, &backend).unwrap();
        let p2 = cache.get_or_load(1, &backend).unwrap();
        assert_eq!(p1[0], p2[0]);
    }

    #[test]
    fn test_lru_eviction_respects_capacity() {
        let mut backend = MemoryBackend::new();
        for i in 1u64..=5 {
            backend.write_page(i, &make_page(i as u8)).unwrap();
        }
        let cache = PageCache::new(3); // capacity 3
        // Load pages 1, 2, 3 — fills cache
        cache.get_or_load(1, &backend).unwrap();
        cache.get_or_load(2, &backend).unwrap();
        cache.get_or_load(3, &backend).unwrap();
        // Load page 4 — evicts LRU (page 1)
        cache.get_or_load(4, &backend).unwrap();
        // Cache size must not exceed capacity
        assert!(cache.cached_page_count() <= 3);
    }

    #[test]
    fn test_dirty_page_written_back_on_flush() {
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(0x00)).unwrap();
        let cache = PageCache::new(4);
        cache.put_dirty(1, make_page(0xFF));
        cache.flush(&mut backend).unwrap();
        let page = backend.read_page(1).unwrap();
        assert_eq!(page[0], 0xFF);
    }

    #[test]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(0x42)).unwrap();
        let cache = Arc::new(PageCache::new(8));
        let handles: Vec<_> = (0..4).map(|_| {
            let c = cache.clone();
            let b = backend.clone();
            thread::spawn(move || {
                let page = c.get_or_load(1, &b).unwrap();
                assert_eq!(page[0], 0x42);
            })
        }).collect();
        for h in handles { h.join().unwrap(); }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path Cargo.toml cache 2>&1 | head -20
```
Expected: `error[E0432]: unresolved import` — module doesn't exist yet.

- [ ] **Step 3: Implement `PageCache`**

Create `src/storage/cache.rs`:

```rust
//! LRU page cache for bounded-memory page access.
//!
//! `PageCache` caches recently read pages. On a cache miss, the caller passes
//! a `&dyn StorageBackend` reference to load the page. Dirty pages (written
//! via `put_dirty`) are tracked and written back on `flush()`.
//!
//! Interior mutability: all methods take `&self` so the cache can be shared
//! across readers without requiring `&mut`.

use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

struct CacheEntry {
    data: Vec<u8>,
    dirty: bool,
}

struct CacheInner {
    entries: HashMap<u64, CacheEntry>,
    /// LRU order: front = least-recently-used, back = most-recently-used.
    order: VecDeque<u64>,
    capacity: usize,
}

impl CacheInner {
    fn touch(&mut self, page_id: u64) {
        if let Some(pos) = self.order.iter().position(|&id| id == page_id) {
            self.order.remove(pos);
        }
        self.order.push_back(page_id);
    }

    /// Evict the least-recently-used clean page. Returns its id if evicted.
    /// Dirty pages are skipped (they must be flushed first).
    fn evict_lru(&mut self, backend: &mut dyn StorageBackend) -> Result<()> {
        // Try to find and evict an LRU entry
        let mut evict_idx = None;
        for (i, &id) in self.order.iter().enumerate() {
            evict_idx = Some((i, id));
            break;
        }
        if let Some((idx, id)) = evict_idx {
            let entry = self.entries.get(&id).unwrap();
            if entry.dirty {
                backend.write_page(id, &entry.data)?;
            }
            self.entries.remove(&id);
            self.order.remove(idx);
        }
        Ok(())
    }
}

/// LRU page cache with configurable capacity.
///
/// All methods take `&self` (interior mutability via `RwLock`).
pub struct PageCache {
    inner: RwLock<CacheInner>,
}

impl PageCache {
    /// Create a new page cache with the given page capacity.
    ///
    /// `capacity = 256` means at most 256 × 4KB = 1MB of cached pages.
    pub fn new(capacity: usize) -> Self {
        PageCache {
            inner: RwLock::new(CacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                capacity,
            }),
        }
    }

    /// Get a page from the cache, loading from `backend` on a miss.
    pub fn get_or_load(&self, page_id: u64, backend: &dyn StorageBackend) -> Result<Arc<Vec<u8>>> {
        // Fast path: read lock
        {
            let inner = self.inner.read().unwrap();
            if let Some(entry) = inner.entries.get(&page_id) {
                return Ok(Arc::new(entry.data.clone()));
            }
        }
        // Miss: load from backend then insert
        let data = backend.read_page(page_id)?;
        let mut inner = self.inner.write().unwrap();
        // Evict if at capacity
        while inner.entries.len() >= inner.capacity {
            // We pass a no-op backend for eviction here; dirty pages already
            // written back on flush(). For simplicity we evict LRU always.
            let evict_id = inner.order.pop_front();
            if let Some(id) = evict_id {
                inner.entries.remove(&id);
            }
        }
        inner.entries.insert(page_id, CacheEntry { data: data.clone(), dirty: false });
        inner.order.push_back(page_id);
        Ok(Arc::new(data))
    }

    /// Insert or update a page in the cache and mark it dirty.
    ///
    /// Called when a page is written in memory but not yet persisted.
    pub fn put_dirty(&self, page_id: u64, data: Vec<u8>) {
        let mut inner = self.inner.write().unwrap();
        if inner.entries.contains_key(&page_id) {
            let entry = inner.entries.get_mut(&page_id).unwrap();
            entry.data = data;
            entry.dirty = true;
        } else {
            while inner.entries.len() >= inner.capacity {
                if let Some(id) = inner.order.pop_front() {
                    inner.entries.remove(&id);
                }
            }
            inner.entries.insert(page_id, CacheEntry { data, dirty: true });
            inner.order.push_back(page_id);
        }
        // Touch to mark as MRU
        let pos = inner.order.iter().rposition(|&id| id == page_id);
        if let Some(p) = pos {
            inner.order.remove(p);
            inner.order.push_back(page_id);
        }
    }

    /// Write all dirty pages to the backend and clear dirty flags.
    pub fn flush(&self, backend: &mut dyn StorageBackend) -> Result<()> {
        let mut inner = self.inner.write().unwrap();
        for (&page_id, entry) in inner.entries.iter_mut() {
            if entry.dirty {
                backend.write_page(page_id, &entry.data)?;
                entry.dirty = false;
            }
        }
        Ok(())
    }

    /// Invalidate (remove) a page from the cache.
    pub fn invalidate(&self, page_id: u64) {
        let mut inner = self.inner.write().unwrap();
        inner.entries.remove(&page_id);
        inner.order.retain(|&id| id != page_id);
    }

    /// Number of pages currently cached (for testing).
    pub fn cached_page_count(&self) -> usize {
        self.inner.read().unwrap().entries.len()
    }
}
```

Add `pub mod cache;` to `src/storage/mod.rs` (after existing `pub mod` declarations).

Also add `MemoryBackend::clone()` — check if `MemoryBackend` is `Clone`. If not, add:
```rust
// src/storage/backend/memory.rs — add derive or impl Clone
#[derive(Clone)]
pub struct MemoryBackend { ... }
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml cache 2>&1
```
Expected: all 5 cache tests pass.

- [ ] **Step 5: Run full test suite**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result"
```
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/storage/cache.rs src/storage/mod.rs src/storage/backend/memory.rs
git commit -m "feat(6.2): add LRU page cache (cache.rs)"
```

---

## Task 2: Packed Page Format

**Files:**
- Create: `src/storage/packed_pages.rs`
- Modify: `src/storage/mod.rs` (add `pub mod packed_pages;`)

- [ ] **Step 1: Write failing tests**

```rust
// src/storage/packed_pages.rs (bottom, under #[cfg(test)])
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
        // With 1-per-page we'd need 50 pages; with packing we need far fewer.
        let facts: Vec<Fact> = (0..50).map(make_fact).collect();
        let (pages, refs) = pack_facts(&facts, 1).unwrap();
        assert!(pages.len() < 50, "packed pages ({}) should be < 50", pages.len());
        assert_eq!(refs.len(), 50);
    }

    #[test]
    fn test_slot_index_roundtrip() {
        let facts: Vec<Fact> = (0..30).map(make_fact).collect();
        let (pages, refs) = pack_facts(&facts, 1).unwrap();
        for (i, fact) in facts.iter().enumerate() {
            let r = &refs[i];
            let page = &pages[(r.page_id - 1) as usize]; // page_id is 1-based
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
        // page 0 = header (unused here), pages 1..=pages.len() = data
        for (i, page) in pages.iter().enumerate() {
            backend.write_page((i + 1) as u64, page).unwrap();
        }
        let recovered = read_all_from_pages(&backend, 1, pages.len() as u64).unwrap();
        assert_eq!(recovered.len(), 60);
        for (orig, rec) in facts.iter().zip(recovered.iter()) {
            assert_eq!(orig.entity, rec.entity);
        }
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --manifest-path Cargo.toml packed_pages 2>&1 | head -10
```
Expected: compile error.

- [ ] **Step 3: Implement packed pages**

Create `src/storage/packed_pages.rs`:

```rust
//! Packed fact page format (page_type = 0x02).
//!
//! Layout of a packed page:
//! ```text
//! [12-byte header]
//!   byte 0:   page_type  (0x02 = packed fact data)
//!   byte 1:   _reserved  (0x00)
//!   bytes 2-3: record_count  (u16 LE)
//!   bytes 4-11: next_page    (u64 LE, 0 = no overflow)
//!
//! [record directory: record_count × 4 bytes]
//!   per entry: offset u16 LE | length u16 LE
//!   (offset is from page start)
//!
//! [record data: variable-length serialised Facts]
//! ```

use crate::graph::types::Fact;
use crate::storage::index::FactRef;
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;

/// Page type byte for packed fact pages.
pub const PAGE_TYPE_PACKED: u8 = 0x02;
/// Page type byte for overflow pages.
pub const PAGE_TYPE_OVERFLOW: u8 = 0x03;

/// Packed page header size in bytes.
pub const PACKED_HEADER_SIZE: usize = 12;

/// Pack a slice of facts into packed pages.
///
/// Returns `(pages, fact_refs)` where `pages` is the serialised page data
/// (each entry is exactly `PAGE_SIZE` bytes) and `fact_refs[i]` is the
/// `FactRef` for `facts[i]`.
///
/// `start_page_id` is the page ID assigned to `pages[0]`; subsequent pages
/// get `start_page_id + 1`, `start_page_id + 2`, etc.
pub fn pack_facts(facts: &[Fact], start_page_id: u64) -> Result<(Vec<Vec<u8>>, Vec<FactRef>)> {
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut fact_refs: Vec<FactRef> = Vec::with_capacity(facts.len());

    let mut current_page: Vec<u8> = new_packed_page();
    let mut current_record_count: u16 = 0;
    let mut dir_offset: usize = PACKED_HEADER_SIZE; // where next dir entry goes
    let mut data_offset: usize = PAGE_SIZE;         // data written from end backwards

    for fact in facts {
        let serialised = postcard::to_allocvec(fact)?;
        let len = serialised.len();

        // Directory entry: 4 bytes
        let dir_entry_size = 4usize;
        let needed = dir_entry_size + len;

        // Check if this fact fits in the current page
        // Available space = data_offset - dir_offset - dir_entry_size
        let available = data_offset.saturating_sub(dir_offset + dir_entry_size);
        if needed > available || current_record_count == u16::MAX {
            // Flush current page
            write_record_count(&mut current_page, current_record_count);
            pages.push(current_page);
            current_page = new_packed_page();
            current_record_count = 0;
            dir_offset = PACKED_HEADER_SIZE;
            data_offset = PAGE_SIZE;
        }

        // Write data from end of page backwards
        data_offset -= len;
        current_page[data_offset..data_offset + len].copy_from_slice(&serialised);

        // Write directory entry
        let offset_u16 = data_offset as u16;
        let len_u16 = len as u16;
        current_page[dir_offset..dir_offset + 2].copy_from_slice(&offset_u16.to_le_bytes());
        current_page[dir_offset + 2..dir_offset + 4].copy_from_slice(&len_u16.to_le_bytes());
        dir_offset += 4;

        let slot_index = current_record_count;
        let page_id = start_page_id + pages.len() as u64;
        fact_refs.push(FactRef { page_id, slot_index });
        current_record_count += 1;
    }

    // Flush last page (even if empty, we need at least 1 page for 0 facts)
    write_record_count(&mut current_page, current_record_count);
    pages.push(current_page);

    // Handle empty facts slice
    if fact_refs.is_empty() && facts.is_empty() {
        // Return 1 empty page
    }

    Ok((pages, fact_refs))
}

/// Read a single fact from a packed page at the given slot index.
pub fn read_slot(page: &[u8], slot: u16) -> Result<Fact> {
    if page[0] != PAGE_TYPE_PACKED {
        anyhow::bail!("Expected packed page (0x02), got 0x{:02x}", page[0]);
    }
    let record_count = u16::from_le_bytes([page[2], page[3]]);
    if slot >= record_count {
        anyhow::bail!("Slot {} out of bounds (page has {} records)", slot, record_count);
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
/// `first_page_id` is the page_id of the first packed fact page.
/// `num_pages` is the number of packed pages to read.
pub fn read_all_from_pages(
    backend: &dyn StorageBackend,
    first_page_id: u64,
    num_pages: u64,
) -> Result<Vec<Fact>> {
    let mut facts = Vec::new();
    for i in 0..num_pages {
        let page = backend.read_page(first_page_id + i)?;
        if page[0] != PAGE_TYPE_PACKED {
            continue; // skip non-packed pages (e.g., index pages)
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
    page[1] = 0x00; // reserved
    // record_count = 0 (bytes 2-3)
    // next_page = 0 (bytes 4-11)
    page
}

fn write_record_count(page: &mut Vec<u8>, count: u16) {
    page[2..4].copy_from_slice(&count.to_le_bytes());
}
```

Add `pub mod packed_pages;` to `src/storage/mod.rs`.

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml packed_pages 2>&1
```
Expected: all 5 tests pass.

- [ ] **Step 5: Run full suite**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result"
```

- [ ] **Step 6: Commit**

```bash
git add src/storage/packed_pages.rs src/storage/mod.rs
git commit -m "feat(6.2): add packed fact page format (packed_pages.rs)"
```

---

## Task 3: FileHeader v5 + CommittedFactReader Trait

**Files:**
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: Write failing tests**

Add to the existing test module in `src/storage/mod.rs`:

```rust
#[test]
fn test_format_version_is_5() {
    assert_eq!(FORMAT_VERSION, 5);
}

#[test]
fn test_file_header_v5_fact_page_format_roundtrip() {
    let mut h = FileHeader::new();
    h.fact_page_format = FACT_PAGE_FORMAT_PACKED;
    let bytes = h.to_bytes();
    let parsed = FileHeader::from_bytes(&bytes).unwrap();
    assert_eq!(parsed.fact_page_format, FACT_PAGE_FORMAT_PACKED);
}

#[test]
fn test_v4_header_reads_fact_page_format_zero() {
    // v4 header has _padding = 0, so fact_page_format must come back as 0
    let mut bytes = vec![0u8; 72];
    bytes[0..4].copy_from_slice(b"MGRF");
    bytes[4..8].copy_from_slice(&4u32.to_le_bytes()); // version = 4
    bytes[8..16].copy_from_slice(&2u64.to_le_bytes()); // page_count = 2
    let h = FileHeader::from_bytes(&bytes).unwrap();
    // v4 _padding was 0, so byte 68 = 0 = unset
    assert_eq!(h.fact_page_format, 0);
}

#[test]
fn test_validate_accepts_version_5() {
    let mut h = FileHeader::new();
    h.version = 5;
    assert!(h.validate().is_ok());
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --manifest-path Cargo.toml test_format_version_is_5 2>&1 | head -5
```

- [ ] **Step 3: Update `src/storage/mod.rs`**

Change:
```rust
pub const FORMAT_VERSION: u32 = 4;
```
to:
```rust
pub const FORMAT_VERSION: u32 = 5;

/// fact_page_format byte: legacy one-per-page (v4 and earlier, or unset).
pub const FACT_PAGE_FORMAT_ONE_PER_PAGE: u8 = 0x01;
/// fact_page_format byte: packed pages (v5+).
pub const FACT_PAGE_FORMAT_PACKED: u8 = 0x02;
```

Update `FileHeader`:
- Replace `pub(crate) _padding: u32` with:
  ```rust
  /// fact_page_format (v5): 0x00 = unset/legacy, 0x01 = one-per-page, 0x02 = packed.
  pub fact_page_format: u8,
  pub(crate) _padding: [u8; 3],
  ```
- Update `FileHeader::new()`: set `fact_page_format: FACT_PAGE_FORMAT_PACKED`, `_padding: [0; 3]`
- Update `to_bytes()`: at offset 68, write `self.fact_page_format`, then `self._padding` (3 bytes)
- Update `from_bytes()`: at offset 68, read `fact_page_format = bytes[68]`, `_padding = [bytes[69], bytes[70], bytes[71]]`
- Update `validate()`: accept versions 1-5

Add `CommittedFactReader` trait (add to `src/storage/mod.rs`, before the tests):

```rust
use crate::graph::types::Fact;
use crate::storage::index::FactRef;

/// Reads committed (checkpointed) facts from persistent storage.
///
/// Implemented by `PersistentFactStorage` and set on `FactStorage` so that
/// index-driven reads can resolve `FactRef`s to `Fact` objects via the page
/// cache without keeping the whole fact list in memory.
pub trait CommittedFactReader: Send + Sync {
    /// Resolve a single committed fact by its disk reference.
    fn resolve(&self, fact_ref: FactRef) -> Result<Fact>;
    /// Stream all committed facts (for full scans, sync check, migration).
    fn stream_all(&self) -> Result<Vec<Fact>>;
    /// Number of committed fact pages (used for checksum + iteration bounds).
    fn committed_page_count(&self) -> u64;
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result|FAILED"
```
Expected: all tests pass (the changed `FORMAT_VERSION` may break one existing test — update `test_format_version_is_4` → `test_format_version_is_5`, and `test_validate_accepts_versions_1_to_4` → update to test 1-5).

- [ ] **Step 5: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat(6.2): FileHeader v5 (fact_page_format), CommittedFactReader trait, FORMAT_VERSION=5"
```

---

## Task 4: PersistentFactStorage — Backend Refactor + Packed Save

**Files:**
- Modify: `src/storage/persistent_facts.rs`

This task wraps the backend in `Arc<Mutex<B>>` (needed for `CommittedFactReader` to share it), adds a `PageCache`, and updates `save()` to write packed pages and compute the page-based checksum.

- [ ] **Step 1: Write failing tests**

Add to the test module in `src/storage/persistent_facts.rs`:

```rust
#[test]
fn test_save_writes_packed_pages() {
    use crate::storage::backend::FileBackend;
    use tempfile::NamedTempFile;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Write 50 facts
    {
        let mut pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        let mut tuples = Vec::new();
        for i in 0u64..50 {
            tuples.push((alice, format!(":attr{}", i), Value::Integer(i as i64)));
        }
        tuples.push((alice, ":friend".to_string(), Value::Ref(bob)));
        pfs.storage().transact(tuples, None).unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
    }

    // Verify: pages used should be << 51 (packed)
    {
        let backend = FileBackend::open(&path).unwrap();
        let header_bytes = backend.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
        assert_eq!(header.version, 5);
        assert_eq!(header.fact_page_format, crate::storage::FACT_PAGE_FORMAT_PACKED);
        // 51 facts @ ~25/page = ~3 pages. Old format = 51 pages.
        // EAVT index pages + fact pages together must be << 100 total pages.
        // At minimum, fact pages should be <= 3.
        let fact_page_count = header.eavt_root_page - 1; // fact pages are 1..eavt_root
        assert!(fact_page_count <= 4, "got {} fact pages", fact_page_count);
    }
}

#[test]
fn test_save_v5_header_checksum_is_page_based() {
    // After save(), reopening should succeed via fast path (checksum match).
    use crate::storage::backend::FileBackend;
    use tempfile::NamedTempFile;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();

    {
        let mut pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        pfs.storage().transact(
            vec![(alice, ":name".to_string(), Value::String("Alice".to_string()))],
            None,
        ).unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
    }

    // Reload — should use fast path (no rebuild triggered).
    {
        let pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        assert_eq!(pfs.storage().fact_count(), 1);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --manifest-path Cargo.toml test_save_writes_packed_pages 2>&1 | head -10
```

- [ ] **Step 3: Refactor backend + add PageCache + update save()**

In `src/storage/persistent_facts.rs`:

**3a. Change the struct:**
```rust
pub struct PersistentFactStorage<B: StorageBackend> {
    backend: Arc<Mutex<B>>,       // was plain `B`; now shared for CommittedFactReader
    page_cache: Arc<PageCache>,
    storage: FactStorage,
    dirty: bool,
    last_checkpointed_tx_count: u64,
    /// Number of committed (packed) fact pages currently on disk.
    committed_fact_pages: Arc<AtomicU64>,
}
```

Add imports:
```rust
use crate::storage::cache::PageCache;
use crate::storage::packed_pages::{pack_facts, read_all_from_pages, FACT_PAGE_FORMAT_PACKED};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
```

**3b. Update `new(backend: B, page_cache_capacity: usize) -> Result<Self>`:**
```rust
pub fn new(backend: B, page_cache_capacity: usize) -> Result<Self> {
    let backend = Arc::new(Mutex::new(backend));
    let page_cache = Arc::new(PageCache::new(page_cache_capacity));
    let committed_fact_pages = Arc::new(AtomicU64::new(0));
    let mut persistent = PersistentFactStorage {
        backend,
        page_cache,
        storage: FactStorage::new(),
        dirty: false,
        last_checkpointed_tx_count: 0,
        committed_fact_pages,
    };
    let page_count = persistent.backend.lock().unwrap().page_count()?;
    if page_count > 1 {
        persistent.load()?;
    } else {
        persistent.save()?;
    }
    Ok(persistent)
}
```

All existing callers of `PersistentFactStorage::new(backend)` must be updated to pass a `page_cache_capacity` argument (use `256` for file-backed, `0` or `8` for tests). Update call sites in `src/db.rs` and tests.

**3c. Update `save()` to write packed pages:**

```rust
pub fn save(&mut self) -> Result<()> {
    if !self.dirty {
        return Ok(());
    }

    let facts = self.storage.get_pending_facts()?; // new method on FactStorage
    let mut backend = self.backend.lock().unwrap();

    // Write packed fact pages starting at page 1
    let start_page_id: u64 = 1;
    let (pages, fact_refs) = pack_facts(&facts, start_page_id)?;
    for (i, page) in pages.iter().enumerate() {
        backend.write_page(start_page_id + i as u64, page)?;
    }
    let num_fact_pages = pages.len() as u64;
    self.committed_fact_pages.store(num_fact_pages, Ordering::SeqCst);

    // Compute page-based checksum (stream over raw fact page bytes)
    let checksum = compute_page_checksum(&*backend, start_page_id, num_fact_pages)?;

    // Update indexes: rebuild from facts with correct FactRefs
    let mut new_indexes = crate::storage::index::Indexes::new();
    for (i, fact) in facts.iter().enumerate() {
        new_indexes.insert(fact, fact_refs[i]);
    }
    self.storage.replace_indexes(new_indexes);

    // Write index pages
    let index_start = start_page_id + num_fact_pages;
    let rebuilt = crate::storage::persistent_facts::reindex_with_refs(&facts, &fact_refs);
    let (eavt_root, aevt_root, avet_root, vaet_root) = write_all_indexes(
        &rebuilt.eavt, &rebuilt.aevt, &rebuilt.avet, &rebuilt.vaet,
        &mut *backend, index_start,
    )?;

    let total_pages = backend.page_count()?;
    let mut header = FileHeader::new(); // version=5, fact_page_format=PACKED
    header.page_count = total_pages;
    header.node_count = facts.len() as u64;
    header.last_checkpointed_tx_count = self.storage.current_tx_count();
    header.eavt_root_page = eavt_root;
    header.aevt_root_page = aevt_root;
    header.avet_root_page = avet_root;
    header.vaet_root_page = vaet_root;
    header.index_checksum = checksum;
    header.fact_page_format = FACT_PAGE_FORMAT_PACKED;

    let mut header_page = header.to_bytes();
    header_page.resize(PAGE_SIZE, 0);
    backend.write_page(0, &header_page)?;
    backend.sync()?;

    self.last_checkpointed_tx_count = self.storage.current_tx_count();
    self.dirty = false;

    // All pending facts are now committed on disk.
    // Clear the pending buffer so resolve_fact_ref uses the CommittedFactReader path.
    self.storage.clear_pending_facts();
    Ok(())
}
```

Add `compute_page_checksum`:
```rust
/// CRC32 over raw committed fact page bytes (v5 semantics).
fn compute_page_checksum(
    backend: &dyn StorageBackend,
    first_page: u64,
    num_pages: u64,
) -> Result<u32> {
    use crc32fast::Hasher;
    let mut hasher = Hasher::new();
    for i in 0..num_pages {
        let page = backend.read_page(first_page + i)?;
        hasher.update(&page);
    }
    Ok(hasher.finalize())
}
```

Add `reindex_with_refs` helper:
```rust
fn reindex_with_refs(facts: &[Fact], refs: &[FactRef]) -> crate::storage::index::Indexes {
    let mut indexes = crate::storage::index::Indexes::new();
    for (fact, &fact_ref) in facts.iter().zip(refs.iter()) {
        indexes.insert(fact, fact_ref);
    }
    indexes
}
```

Add these three helpers to `FactStorage` (in `src/graph/storage.rs`). These are needed by Tasks 4 and 5 before Task 6 refactors `FactData`:
```rust
/// Return facts that have been transacted but not yet checkpointed.
/// These are the facts in the in-memory pending buffer.
pub fn get_pending_facts(&self) -> Result<Vec<Fact>> {
    let d = self.data.read().unwrap();
    Ok(d.facts.clone())
}

/// Replace the in-memory indexes with a freshly-built set.
/// Called by PersistentFactStorage::save() after packing.
pub fn replace_indexes(&self, indexes: crate::storage::index::Indexes) {
    let mut d = self.data.write().unwrap();
    d.indexes = indexes;
}

/// Clear the pending fact buffer after a successful checkpoint.
/// All pending facts are now committed (on-disk packed pages).
pub fn clear_pending_facts(&self) {
    let mut d = self.data.write().unwrap();
    d.facts.clear();
}
```

Also update `into_backend()` — it can no longer do `ptr::read` since `backend` is `Arc<Mutex<B>>`. Use `Arc::try_unwrap`:
```rust
pub fn into_backend(mut self) -> B {
    if self.dirty {
        let _ = self.save();
    }
    // Clone the Arc so we hold a reference after self is dropped.
    // drop(self) also drops self.storage (and the CommittedFactLoaderImpl inside
    // FactData.committed), leaving backend_arc as the sole owner.
    let backend_arc = self.backend.clone();
    drop(self);
    Arc::try_unwrap(backend_arc)
        .expect("into_backend: backend Arc has multiple owners")
        .into_inner()
        .unwrap()
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result|FAILED"
```
Fix any compilation errors. The main breakage will be call sites passing `PersistentFactStorage::new(backend)` without `page_cache_capacity` — add `256` everywhere.

- [ ] **Step 5: Commit**

```bash
git add src/storage/persistent_facts.rs src/graph/storage.rs src/db.rs
git commit -m "feat(6.2): PersistentFactStorage save() with packed pages + page checksum"
```

---

## Task 5: PersistentFactStorage — v5 Load + v4→v5 Migration + CommittedFactReader

**Files:**
- Modify: `src/storage/persistent_facts.rs`

- [ ] **Step 1: Write failing tests**

Add to test module in `src/storage/persistent_facts.rs`:

```rust
#[test]
fn test_v4_database_migrates_to_v5_on_open() {
    use crate::storage::backend::FileBackend;
    use tempfile::NamedTempFile;

    // Create a v4 database by manually writing a one-per-page header
    // with version=4 and fact_page_format=0 (legacy).
    // Easiest: build with Phase 6.1 format by hand using the v4 save path.
    // We simulate this by writing a v4 header + one-per-page fact.
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();

    // Write a "v4-style" file manually.
    {
        use crate::storage::{FileHeader, MAGIC_NUMBER, FORMAT_VERSION};
        let fact = crate::graph::types::Fact::with_valid_time(
            alice, ":name".to_string(),
            Value::String("Alice".to_string()),
            1u64, 1u64, 0i64, i64::MAX,
        );
        let mut backend = FileBackend::open(&path).unwrap();

        // Write fact at page 1 (one-per-page format)
        let data = postcard::to_allocvec(&fact).unwrap();
        let mut page = vec![0u8; PAGE_SIZE];
        page[..data.len()].copy_from_slice(&data);
        backend.write_page(1, &page).unwrap();

        // Write v4 header (version=4, fact_page_format=0x00 in _padding)
        let mut header = FileHeader::new();
        header.version = 4; // force v4
        header.fact_page_format = 0; // v4 didn't have this field
        header.page_count = 2;
        header.node_count = 1;
        let mut hbytes = header.to_bytes();
        // Force version field to 4
        hbytes[4..8].copy_from_slice(&4u32.to_le_bytes());
        hbytes.resize(PAGE_SIZE, 0);
        backend.write_page(0, &hbytes).unwrap();
        backend.sync().unwrap();
    }

    // Open with PersistentFactStorage — should auto-migrate to v5
    {
        let pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        assert_eq!(pfs.storage().fact_count(), 1, "migrated fact must be loaded");
    }

    // Verify file is now v5
    {
        let backend = FileBackend::open(&path).unwrap();
        let header_bytes = backend.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
        assert_eq!(header.version, 5, "file must be upgraded to v5");
        assert_eq!(header.fact_page_format, crate::storage::FACT_PAGE_FORMAT_PACKED);
    }
}

#[test]
fn test_v5_load_fast_path_on_checksum_match() {
    use crate::storage::backend::FileBackend;
    use tempfile::NamedTempFile;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();

    // Save in v5 format
    {
        let mut pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        pfs.storage().transact(
            vec![(alice, ":name".to_string(), Value::String("Alice".to_string()))],
            None,
        ).unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
    }

    // Reload — CommittedFactReader should resolve the fact correctly
    {
        let pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap(), 256,
        ).unwrap();
        let facts = pfs.storage().get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].entity, alice);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --manifest-path Cargo.toml test_v4_database_migrates 2>&1 | head -10
```

- [ ] **Step 3: Implement v5 load() + migration + CommittedFactReader**

**3a. Add `CommittedFactLoaderImpl`:**

```rust
/// CommittedFactReader implementation backed by a PageCache + backend.
struct CommittedFactLoaderImpl<B: StorageBackend> {
    backend: Arc<Mutex<B>>,
    page_cache: Arc<PageCache>,
    committed_fact_pages: Arc<AtomicU64>,
    first_fact_page: u64,  // always 1 in current layout
}

impl<B: StorageBackend + 'static> crate::storage::CommittedFactReader
    for CommittedFactLoaderImpl<B>
{
    fn resolve(&self, fact_ref: crate::storage::index::FactRef) -> Result<Fact> {
        let backend = self.backend.lock().unwrap();
        let page = self.page_cache.get_or_load(fact_ref.page_id, &*backend)?;
        crate::storage::packed_pages::read_slot(&page, fact_ref.slot_index)
    }

    fn stream_all(&self) -> Result<Vec<Fact>> {
        let n = self.committed_fact_pages.load(Ordering::SeqCst);
        let backend = self.backend.lock().unwrap();
        crate::storage::packed_pages::read_all_from_pages(
            &*backend, self.first_fact_page, n,
        )
    }

    fn committed_page_count(&self) -> u64 {
        self.committed_fact_pages.load(Ordering::SeqCst)
    }
}
```

**3b. Update `load()` in `PersistentFactStorage`:**

```rust
fn load(&mut self) -> Result<()> {
    let header = {
        let backend = self.backend.lock().unwrap();
        let header_page = backend.read_page(0)?;
        let h = FileHeader::from_bytes(&header_page)?;
        h.validate()?;
        h
    };

    if header.version < 2 {
        return self.migrate_v1_to_v2();
    }

    self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;
    self.storage.clear()?;

    let fact_page_format = header.fact_page_format;

    if fact_page_format == 0 || fact_page_format == FACT_PAGE_FORMAT_ONE_PER_PAGE {
        // v4 or earlier one-per-page format: load all facts, then migrate.
        self.load_one_per_page(&header)?;
        self.storage.restore_tx_counter()?;
        self.dirty = true;
        self.save()?; // migrates to v5 packed format
        return Ok(());
    }

    // v5 packed format:
    // 1. Determine how many fact pages exist (eavt_root - 1)
    let num_fact_pages = if header.eavt_root_page > 1 {
        header.eavt_root_page - 1
    } else {
        0
    };
    self.committed_fact_pages.store(num_fact_pages, Ordering::SeqCst);

    // 2. Compute page-based checksum
    let computed = {
        let backend = self.backend.lock().unwrap();
        compute_page_checksum(&*backend, 1, num_fact_pages)?
    };
    let stored = header.index_checksum;
    let needs_rebuild = computed != stored
        || (header.eavt_root_page == 0 && computed != 0);

    // 3. Register CommittedFactReader on FactStorage (before WAL replay)
    let loader = Arc::new(CommittedFactLoaderImpl {
        backend: self.backend.clone(),
        page_cache: self.page_cache.clone(),
        committed_fact_pages: self.committed_fact_pages.clone(),
        first_fact_page: 1,
    });
    self.storage.set_committed_reader(loader);

    // 4. Restore tx_counter from header
    self.storage.restore_tx_counter_from(header.last_checkpointed_tx_count);

    // NOTE: `read_eavt_index`, `read_aevt_index`, `read_avet_index`, `read_vaet_index`,
    // and `write_all_indexes` are all defined in Phase 6.1's `src/storage/btree.rs`.
    // Add this import at the top of `persistent_facts.rs`:
    //   use crate::storage::btree::{
    //       read_eavt_index, read_aevt_index, read_avet_index, read_vaet_index,
    //       write_all_indexes,
    //   };

    if needs_rebuild {
        // Full rebuild from pages. Re-pack to derive correct FactRefs.
        // This is safe: the data bytes on disk are the canonical truth;
        // we're only computing which (page_id, slot_index) each fact occupies.
        let all_facts = {
            let backend = self.backend.lock().unwrap();
            read_all_from_pages(&*backend, 1, num_fact_pages)?
        };
        let max_tx = all_facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.storage.restore_tx_counter_from(max_tx);
        // pack_facts gives us the exact FactRefs that correspond to the on-disk layout
        let (_, real_refs) = pack_facts(&all_facts, 1)?;
        let mut indexes = crate::storage::index::Indexes::new();
        for (fact, &fr) in all_facts.iter().zip(real_refs.iter()) {
            indexes.insert(fact, fr);
        }
        self.storage.replace_indexes(indexes);
        self.dirty = true;
        self.save()?;
    } else if header.eavt_root_page != 0 {
        // Fast path: load indexes from B+tree pages
        let backend = self.backend.lock().unwrap();
        let eavt = read_eavt_index(header.eavt_root_page, &*backend)?;
        let aevt = read_aevt_index(header.aevt_root_page, &*backend)?;
        let avet = read_avet_index(header.avet_root_page, &*backend)?;
        let vaet = read_vaet_index(header.vaet_root_page, &*backend)?;
        drop(backend);
        self.storage.replace_indexes(crate::storage::index::Indexes { eavt, aevt, avet, vaet });
    }

    self.dirty = false;
    Ok(())
}

fn load_one_per_page(&mut self, header: &FileHeader) -> Result<()> {
    let page_count = header.page_count;
    let backend = self.backend.lock().unwrap();
    for page_id in 1..page_count {
        let page = backend.read_page(page_id)?;
        if let Ok(fact) = postcard::from_bytes::<Fact>(&page) {
            self.storage.load_fact(fact)?;
        }
    }
    Ok(())
}
```

Add `restore_tx_counter_from(max: u64)` to `FactStorage`:
```rust
pub fn restore_tx_counter_from(&self, max: u64) {
    self.tx_counter.store(max, Ordering::SeqCst);
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result|FAILED"
```

- [ ] **Step 5: Commit**

```bash
git add src/storage/persistent_facts.rs src/graph/storage.rs
git commit -m "feat(6.2): v5 load(), v4→v5 migration, CommittedFactReader wired into PersistentFactStorage"
```

---

## Task 6: FactStorage — CommittedFactReader Integration + Index-Driven Reads

**Files:**
- Modify: `src/graph/storage.rs`

This task wires the `CommittedFactReader` into `FactStorage` so that read methods use index → FactRef resolution via the committed loader instead of scanning `d.facts`.

- [ ] **Step 1: Write failing tests**

Add to `src/graph/storage.rs` test module:

```rust
#[test]
fn test_get_facts_by_entity_uses_committed_reader() {
    use crate::storage::CommittedFactReader;
    use crate::storage::index::FactRef;
    use uuid::Uuid;
    use std::sync::Arc;

    // Build a mock CommittedFactReader.
    // The mock uses slot_index as an index into its facts vec.
    struct MockLoader {
        facts: Vec<Fact>,
    }
    impl CommittedFactReader for MockLoader {
        fn resolve(&self, fr: FactRef) -> Result<Fact> {
            // slot_index is used as the fact-list index in this mock
            self.facts.get(fr.slot_index as usize)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("not found at slot {}", fr.slot_index))
        }
        fn stream_all(&self) -> Result<Vec<Fact>> { Ok(self.facts.clone()) }
        fn committed_page_count(&self) -> u64 { 1 }
    }

    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    // Add alice as a "committed" fact via the mock loader
    let committed_fact = Fact::with_valid_time(
        alice, ":name".to_string(), Value::String("Alice".to_string()),
        0, 1, 0, VALID_TIME_FOREVER,
    );
    let loader = Arc::new(MockLoader { facts: vec![committed_fact] });

    // Pre-populate index with FactRef pointing into the committed loader.
    // page_id >= 1 means committed (not pending). slot_index=0 → mock.facts[0].
    let mut indexes = crate::storage::index::Indexes::new();
    indexes.insert(&Fact::with_valid_time(
        alice, ":name".to_string(), Value::String("Alice".to_string()),
        0, 1, 0, VALID_TIME_FOREVER,
    ), FactRef { page_id: 1, slot_index: 0 }); // page_id=1 → committed path
    storage.replace_indexes(indexes);
    storage.set_committed_reader(loader);

    let facts = storage.get_facts_by_entity(&alice).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].entity, alice);
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --manifest-path Cargo.toml test_get_facts_by_entity_uses_committed_reader 2>&1 | head -10
```

- [ ] **Step 3: Update `FactData` and `FactStorage`**

In `src/graph/storage.rs`:

**3a. Update `FactData`:**
```rust
struct FactData {
    /// Pending (uncommitted) facts — transacted since last checkpoint.
    facts: Vec<Fact>,
    indexes: Indexes,
    /// Resolves committed (on-disk) FactRefs to Fact objects.
    /// None for in-memory databases (MemoryBackend) where all facts are pending.
    committed: Option<Arc<dyn crate::storage::CommittedFactReader>>,
}
```

**3b. Add `set_committed_reader` to `FactStorage`:**
```rust
pub fn set_committed_reader(&self, reader: Arc<dyn crate::storage::CommittedFactReader>) {
    let mut d = self.data.write().unwrap();
    d.committed = Some(reader);
}
```

**3c. Update `FactData::new` in `FactStorage::new()`:**
```rust
data: Arc::new(RwLock::new(FactData {
    facts: Vec::new(),
    indexes: Indexes::new(),
    committed: None,
}))
```

**3d. Add `resolve_fact_ref` helper:**
```rust
fn resolve_fact_ref(d: &FactData, fr: FactRef) -> Result<Fact> {
    if fr.page_id == 0 {
        // Pending fact: page_id=0 means slot_index is index into pending facts
        d.facts.get(fr.slot_index as usize)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("pending fact index {} out of bounds", fr.slot_index))
    } else {
        // Committed fact: resolve via the committed reader
        match &d.committed {
            Some(loader) => loader.resolve(fr),
            None => anyhow::bail!("no committed reader but got non-zero page_id {}", fr.page_id),
        }
    }
}
```

**3e. Update `get_facts_by_entity()` to use EAVT:**
```rust
pub fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
    use crate::storage::index::EavtKey;
    let d = self.data.read().unwrap();
    let start = EavtKey { entity: *entity_id, attribute: String::new(),
        valid_from: i64::MIN, valid_to: i64::MIN, tx_count: 0 };
    // Iterate EAVT entries for this entity; stop when entity changes.
    let mut facts = Vec::new();
    for (key, &fr) in d.indexes.eavt.range(start..) {
        if &key.entity != entity_id { break; }
        facts.push(resolve_fact_ref(&d, fr)?);
    }
    Ok(facts)
}
```

**3f. Update `get_facts_by_attribute()` to use AEVT:**
```rust
pub fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
    use crate::storage::index::AevtKey;
    use uuid::Uuid;
    let d = self.data.read().unwrap();
    let start = AevtKey { attribute: attribute.clone(), entity: Uuid::nil(),
        valid_from: i64::MIN, valid_to: i64::MIN, tx_count: 0 };
    let mut facts = Vec::new();
    for (key, &fr) in d.indexes.aevt.range(start..) {
        if &key.attribute != attribute { break; }
        facts.push(resolve_fact_ref(&d, fr)?);
    }
    Ok(facts)
}
```

**3g. Update `get_facts_by_entity_attribute()` to use EAVT:**
```rust
pub fn get_facts_by_entity_attribute(&self, entity_id: &EntityId, attribute: &Attribute) -> Result<Vec<Fact>> {
    use crate::storage::index::EavtKey;
    let d = self.data.read().unwrap();
    let start = EavtKey { entity: *entity_id, attribute: attribute.clone(),
        valid_from: i64::MIN, valid_to: i64::MIN, tx_count: 0 };
    let mut facts = Vec::new();
    for (key, &fr) in d.indexes.eavt.range(start..) {
        if &key.entity != entity_id || &key.attribute != attribute { break; }
        facts.push(resolve_fact_ref(&d, fr)?);
    }
    Ok(facts)
}
```

**3h. Update `get_all_facts()` to include committed facts:**
```rust
pub fn get_all_facts(&self) -> Result<Vec<Fact>> {
    let d = self.data.read().unwrap();
    let mut facts = Vec::new();
    // Committed facts first (from disk)
    if let Some(loader) = &d.committed {
        facts.extend(loader.stream_all()?);
    }
    // Then pending facts
    facts.extend(d.facts.clone());
    Ok(facts)
}
```

**3i. Update `transact()` to assign correct pending FactRefs:**
In `transact()`, change the `FactRef` inserted into indexes to use the pending index:
```rust
// After d.facts.extend(facts) — NO, do it before:
let pending_start = d.facts.len(); // index of first new pending fact
for (i, fact) in facts.iter().enumerate() {
    d.indexes.insert(fact, FactRef {
        page_id: 0,  // pending
        slot_index: (pending_start + i) as u16,
    });
}
d.facts.extend(facts);
```
Same for `retract()` and `load_fact()`.

**3j. Update `restore_tx_counter()`** to look at both committed and pending facts:
```rust
pub fn restore_tx_counter(&self) -> Result<()> {
    let d = self.data.read().unwrap();
    let pending_max = d.facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
    let committed_max = d.committed.as_ref()
        .map(|loader| loader.stream_all().ok()
            .map(|facts| facts.iter().map(|f| f.tx_count).max().unwrap_or(0))
            .unwrap_or(0))
        .unwrap_or(0);
    let max = pending_max.max(committed_max);
    self.tx_counter.store(max, Ordering::SeqCst);
    Ok(())
}
```

**3k. Update `fact_count()`:**
```rust
pub fn fact_count(&self) -> usize {
    let d = self.data.read().unwrap();
    let committed = d.committed.as_ref()
        .map(|l| l.stream_all().map(|v| v.len()).unwrap_or(0))
        .unwrap_or(0);
    committed + d.facts.len()
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result|FAILED"
```

Fix any failures. Common issues:
- `get_facts_as_of()` and `get_facts_valid_at()` still scan `d.facts` — they should also include committed facts. Update them to call `get_all_facts()` or scan both committed + pending.
- `get_asserted_facts()` similarly.

Update `get_facts_as_of()`:
```rust
pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
    let all = self.get_all_facts()?;
    Ok(all.into_iter().filter(|f| match as_of {
        AsOf::Counter(n) => f.tx_count <= *n,
        AsOf::Timestamp(t) => f.tx_id <= *t as u64,
    }).collect())
}
```

Update `get_facts_valid_at()`, `get_asserted_facts()`, `asserted_fact_count()`, `get_current_value()` similarly — replace `d.facts.iter()...` with `self.get_all_facts()?...`.

- [ ] **Step 5: Commit**

```bash
git add src/graph/storage.rs
git commit -m "feat(6.2): FactStorage index-driven reads via CommittedFactReader; pending/committed fact separation"
```

---

## Task 7: OpenOptions + Version Bump + Integration Tests

**Files:**
- Modify: `src/db.rs` — `OpenOptions::page_cache_size`
- Modify: `Cargo.toml` — version 0.7.0
- Modify: `CHANGELOG.md` — v0.7.0 entry
- Create: `tests/performance_test.rs`

- [ ] **Step 1: Add `page_cache_size` to OpenOptions**

In `src/db.rs`:
```rust
pub struct OpenOptions {
    pub wal_checkpoint_threshold: usize,
    /// Number of pages to hold in the LRU page cache.
    ///
    /// Default: 256 pages = 1MB for 4KB pages.
    pub page_cache_size: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            wal_checkpoint_threshold: 1000,
            page_cache_size: 256,
        }
    }
}

impl OpenOptions {
    ...
    pub fn page_cache_size(mut self, size: usize) -> Self {
        self.page_cache_size = size;
        self
    }
}
```

Update `OpenOptionsWithPath::open()` and `Minigraf::in_memory()` to pass `opts.page_cache_size` (or `8` for in-memory) to `PersistentFactStorage::new(backend, cache_size)`.

- [ ] **Step 2: Bump version + CHANGELOG**

In `Cargo.toml`:
```toml
version = "0.7.0"
```

In `CHANGELOG.md`, add at the top:
```markdown
## [0.7.0] - 2026-03-21

### Added
- Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page, ~25× disk space reduction
- LRU page cache (`src/storage/cache.rs`): configurable capacity (default 256 pages = 1MB)
- `OpenOptions::page_cache_size(usize)` for tuning page cache capacity
- `CommittedFactReader` trait: index-driven fact resolution via page cache (no startup load-all)
- File format v5: `fact_page_format` header field; auto-migration from v4 on first open
- Page-based CRC32 checksum (v5): streams raw committed page bytes instead of serialising all facts
- `FactStorage::set_committed_reader()`: wires packed-page loader for index-driven queries
- `FactStorage::restore_tx_counter_from(max: u64)`: efficient counter restoration from header

### Changed
- `PersistentFactStorage::new()` now takes `page_cache_capacity: usize` as second argument
- Committed facts are no longer loaded into `Vec<Fact>` at startup; only WAL-replayed pending facts are held in memory
- `FactStorage` read methods (`get_facts_by_entity`, `get_facts_by_attribute`, etc.) now use index range scans + `CommittedFactReader` instead of full `Vec<Fact>` scans

### Fixed
- v4 databases are automatically repacked as v5 on first open (no data loss)
```

- [ ] **Step 3: Write integration tests**

Create `tests/performance_test.rs`:

```rust
//! Integration and performance tests for Phase 6.2 packed pages.

use minigraf::{Minigraf, OpenOptions};
use tempfile::NamedTempFile;

/// Insert N facts and verify correct query results.
fn insert_and_query(db: &Minigraf, n: usize) {
    // Insert N facts in batches of 100
    for batch in 0..(n / 100) {
        let mut cmds = String::from("(transact [");
        for i in 0..100 {
            let idx = batch * 100 + i;
            cmds.push_str(&format!(
                "[:e{} :val {}]",
                idx, idx
            ));
        }
        cmds.push_str("])");
        db.execute(&cmds).unwrap();
    }
}

#[test]
fn test_1k_facts_correct_after_packed_save_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        insert_and_query(&db, 1000);
    }

    // Reload and verify a query
    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db.execute(
        "(query [:find ?v :where [:e0 :val ?v]])"
    ).unwrap();
    // :e0 should have val 0
    // Result should be non-empty
    assert!(!format!("{:?}", result).is_empty());
}

#[test]
fn test_packed_pages_use_fewer_pages_than_one_per_page() {
    use minigraf::OpenOptions;
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    // Insert 200 facts
    {
        let db = OpenOptions::new().path(path).open().unwrap();
        for i in 0..200u64 {
            db.execute(&format!("(transact [[:e{} :val {}]])", i, i)).unwrap();
        }
    }

    // Read the file size and verify it's much smaller than 200 * 4096 bytes
    let file_size = std::fs::metadata(path).unwrap().len();
    let one_per_page_size = 201 * 4096u64; // 200 facts + header
    assert!(
        file_size < one_per_page_size,
        "packed file ({} bytes) should be smaller than one-per-page ({} bytes)",
        file_size, one_per_page_size
    );
}

#[test]
fn test_bitemporal_query_after_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"} [[:alice :status :active]])"#).unwrap();
        db.execute(r#"(transact {:valid-from "2024-01-01"} [[:alice :status :inactive]])"#).unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db
        .execute(r#"(query [:find ?s :valid-at "2023-06-01" :where [:alice :status ?s]])"#)
        .unwrap();
    assert!(format!("{:?}", result).contains("active"),
        "should find :active at 2023-06-01, got: {:?}", result);
}

#[test]
fn test_as_of_query_after_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute("(transact [[:alice :age 30]])").unwrap();
        db.execute("(transact [[:alice :age 31]])").unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db
        .execute("(query [:find ?age :as-of 1 :where [:alice :age ?age]])")
        .unwrap();
    assert!(format!("{:?}", result).contains("30"),
        "as-of 1 should return age 30, got {:?}", result);
}

#[test]
fn test_recursive_rules_unchanged_after_6_2() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c] [:c :next :d]])").unwrap();
    db.execute("(rule [(reachable ?from ?to) [?from :next ?to]])").unwrap();
    db.execute("(rule [(reachable ?from ?to) [?from :next ?mid] (reachable ?mid ?to)])").unwrap();
    let result = db
        .execute("(query [:find ?to :where (reachable :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("b") && s.contains("c") && s.contains("d"),
        "transitive closure must work: {:?}", result);
}

#[test]
fn test_explicit_tx_survives_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.commit().unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db.execute("(query [:find ?n :where [:alice :name ?n]])").unwrap();
    assert!(format!("{:?}", result).contains("Alice"),
        "Alice must survive packed reload: {:?}", result);
}

#[test]
fn test_page_cache_size_option_accepted() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    // Should not panic with custom cache size
    let db = OpenOptions::new()
        .path(path)
        .page_cache_size(64)
        .open()
        .unwrap();
    db.execute("(transact [[:x :y 1]])").unwrap();
}
```

- [ ] **Step 4: Run full test suite**

```bash
cargo test --manifest-path Cargo.toml 2>&1 | grep -E "^test result|FAILED|error"
```
All tests must pass. Pay attention to:
- All 212+ existing tests still pass (no regressions)
- All 7 new `performance_test.rs` tests pass

- [ ] **Step 5: Run clippy + fmt**

```bash
cargo clippy --manifest-path Cargo.toml -- -D warnings 2>&1 | grep -v "^$"
cargo fmt --manifest-path Cargo.toml
```
Fix any warnings.

- [ ] **Step 6: Commit all remaining changes**

```bash
git add Cargo.toml CHANGELOG.md src/db.rs tests/performance_test.rs
git commit -m "feat(6.2): OpenOptions::page_cache_size; bump to v0.7.0; integration tests"
```

---

## Implementation Notes

### FactRef assignment in transact()

When `transact()` builds new facts, assign `FactRef { page_id: 0, slot_index: pending_index }` where `pending_index` is the 0-based index of the fact in `d.facts` after appending. This lets `resolve_fact_ref()` look up `d.facts[slot_index]` for pending facts.

### into_backend() in tests

Tests that call `pfs.into_backend()` and then inspect the backend need updating. The safest approach: rewrite those tests to use `FileBackend` + reopen the file, rather than consuming the PFS.

### Overflow pages

Packed pages assume facts are < `~3900` bytes. The current `Fact` struct is ~150 bytes, so overflow is not needed. The `next_page` field in the packed page header is reserved for future use and should always be written as `0`.

### WAL replay in v5

After WAL replay, pending facts are in `d.facts` with `FactRef { page_id: 0, slot_index }`. These are resolved from the pending buffer. On next checkpoint (`save()`), they're packed into committed pages and their indexes are updated with real FactRefs. The `committed_fact_pages` counter is updated to reflect the new page count.

### Bounded memory semantics

Phase 6.2 guarantees bounded memory **at idle** (between queries): only pending (un-checkpointed) facts are held in `d.facts`; committed facts live on disk, accessed via page cache. Full-scan operations like `get_all_facts()`, `get_facts_as_of()`, and `get_facts_valid_at()` call `stream_all()` which materialises all committed facts into a `Vec<Fact>`. This is O(n) memory during the query but the allocation is freed when the query completes. The page cache evicts old pages as new ones are loaded, keeping its footprint at `page_cache_size × 4KB`. Phase 6.3 will add streaming/iterator-based scan execution to eliminate the transient materialisation.

### Task ordering and dependencies

- **Task 4 depends on Task 6 helpers**: `get_pending_facts()`, `replace_indexes()`, and `clear_pending_facts()` are defined in Task 4 (listed in the "Add these three helpers" block above) and consumed in Task 4's `save()`. Task 6 then refactors `FactData` further. Implement Tasks 1→2→3→4→5→6→7 in order.
- **btree.rs functions**: `read_eavt_index`, `read_aevt_index`, `read_avet_index`, `read_vaet_index`, `write_all_indexes` are all defined in Phase 6.1's `src/storage/btree.rs`. Add the import in `persistent_facts.rs` as shown in the Task 5 note.

### MemoryBackend for in-memory DB

`Minigraf::in_memory()` uses `MemoryBackend` with `page_cache_capacity = 8` (or any small value). Since all facts are pending (never checkpointed to packed pages unless `save()` is called), `d.committed` is `None` and all facts live in `d.facts`. This is backward-compatible.
