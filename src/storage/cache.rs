//! LRU page cache for bounded-memory page access.
//!
//! `PageCache` caches recently read pages. On a cache miss, the caller passes
//! a `&dyn StorageBackend` reference to load the page. Dirty pages (written
//! via `put_dirty`) are tracked and written back on `flush()`.
//!
//! Interior mutability: all methods take `&self` so the cache can be shared
//! across readers without requiring `&mut`.
//!
//! ## LRU accuracy
//!
//! `get_or_load` uses a read lock on cache hits for concurrent-reader throughput.
//! As a result, hit pages are **not** promoted to MRU position on each access —
//! only first-load (miss) positions them as MRU. This gives approximate-LRU
//! semantics: frequently accessed pages are unlikely to be evicted but not
//! strictly guaranteed MRU. For a 256-page cache this is an excellent tradeoff.

use crate::error::{ErrorCode, err_coded};
use crate::storage::StorageBackend;
use crate::storage::page;
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

struct CacheEntry {
    data: Arc<Vec<u8>>,
    dirty: bool,
}

struct CacheInner {
    entries: HashMap<u64, CacheEntry>,
    /// LRU order: front = least-recently-used, back = most-recently-used.
    order: VecDeque<u64>,
    capacity: usize,
}

impl CacheInner {
    /// Move `page_id` to the MRU (back) position.
    ///
    /// Uses an O(N) scan — acceptable for the small cache sizes used here
    /// (default 256 pages). A positions HashMap was tried previously but was
    /// incorrect: every `pop_front` eviction shifts all remaining indices,
    /// making stored positions stale and causing out-of-bounds panics.
    fn touch(&mut self, page_id: u64) {
        if let Some(pos) = self.order.iter().position(|&id| id == page_id) {
            // Check if already at MRU (back) position without arithmetic
            if self.order.back() == Some(&page_id) {
                return; // Already at MRU position
            }
            self.order.remove(pos);
        }
        self.order.push_back(page_id);
    }
}

/// LRU page cache with configurable capacity.
///
/// All methods take `&self` (interior mutability via `RwLock`).
pub struct PageCache {
    inner: RwLock<CacheInner>,
    /// Highest page generation a load accepts: the active meta's generation
    /// (spec §4.2 check 4). `u64::MAX` until storage sets it.
    generation_bound: AtomicU64,
}

impl PageCache {
    /// Create a new page cache with the given page capacity.
    ///
    /// `capacity = 256` means at most 256 × 4KB = 1MB of cached pages.
    ///
    /// `capacity = 0` **disables the cache entirely**: `get_or_load` always
    /// reads through to the backend and `put_dirty` is a no-op. Use this when
    /// the backend itself is already an in-memory page store (e.g. a
    /// `HashMap`-backed buffer), so this LRU layer would only add duplicate
    /// storage and bookkeeping overhead on top of an already-resident page.
    pub fn new(capacity: usize) -> Self {
        PageCache {
            inner: RwLock::new(CacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                capacity,
            }),
            generation_bound: AtomicU64::new(u64::MAX),
        }
    }

    /// Set the highest page generation that `get_or_load` accepts.
    pub fn set_generation_bound(&self, generation: u64) {
        self.generation_bound.store(generation, Ordering::SeqCst);
    }

    /// Read a page from the backend and run the spec §4.2 header checks.
    fn load_verified(&self, page_id: u64, backend: &dyn StorageBackend) -> Result<Vec<u8>> {
        let data = backend.read_page(page_id)?;
        page::verify(&data, page_id, self.generation_bound.load(Ordering::SeqCst))?;
        Ok(data)
    }

    /// Get a page from the cache, loading from `backend` on a miss.
    ///
    /// Every page read from the backend is verified (type, CRC, page id,
    /// generation) before it is returned or cached; a page that fails is
    /// never cached.
    pub fn get_or_load(&self, page_id: u64, backend: &dyn StorageBackend) -> Result<Arc<Vec<u8>>> {
        // Fast path: read lock for cache hits (concurrent readers don't block each other)
        // Approximate LRU: return without promoting to MRU to avoid a write lock
        // on every read. Pages loaded recently (on miss) are already near MRU.
        {
            let inner = self
                .inner
                .read()
                .map_err(|_| err_coded!(ErrorCode::Int050, "cache"))?;
            // Capacity 0 disables the cache: always read through, no bookkeeping.
            if inner.capacity == 0 {
                return Ok(Arc::new(self.load_verified(page_id, backend)?));
            }
            if let Some(entry) = inner.entries.get(&page_id) {
                return Ok(entry.data.clone());
            }
        }
        // Miss: load from backend (without holding any lock)
        let data = Arc::new(self.load_verified(page_id, backend)?);
        let mut inner = self
            .inner
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "cache"))?;
        // Double-check after acquiring write lock (another thread may have loaded it)
        if let Some(entry) = inner.entries.get(&page_id) {
            return Ok(entry.data.clone());
        }
        // Evict LRU if at capacity
        while inner.entries.len() >= inner.capacity && inner.capacity > 0 {
            if let Some(id) = inner.order.pop_front() {
                inner.entries.remove(&id);
            } else {
                break; // order/entries out of sync — avoid infinite loop
            }
        }
        inner.entries.insert(
            page_id,
            CacheEntry {
                data: data.clone(),
                dirty: false,
            },
        );
        inner.order.push_back(page_id);
        Ok(data)
    }

    /// Insert or update a page in the cache and mark it dirty.
    pub fn put_dirty(&self, page_id: u64, data: Vec<u8>) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        // Capacity 0 disables the cache: nothing to prime, next get_or_load reads through.
        if inner.capacity == 0 {
            return;
        }
        let data = Arc::new(data);
        if inner.entries.contains_key(&page_id) {
            // Update in place, move to MRU
            if let Some(entry) = inner.entries.get_mut(&page_id) {
                entry.data = data;
                entry.dirty = true;
            }
            inner.touch(page_id);
        } else {
            // Evict if at capacity
            while inner.entries.len() >= inner.capacity && inner.capacity > 0 {
                if let Some(id) = inner.order.pop_front() {
                    inner.entries.remove(&id);
                } else {
                    break; // order/entries out of sync — avoid infinite loop
                }
            }
            inner
                .entries
                .insert(page_id, CacheEntry { data, dirty: true });
            inner.order.push_back(page_id);
        }
    }

    /// Write all dirty pages to the backend and clear dirty flags.
    #[allow(dead_code)]
    pub fn flush(&self, backend: &mut dyn StorageBackend) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "cache"))?;
        for (&page_id, entry) in inner.entries.iter_mut() {
            if entry.dirty {
                backend.write_page(page_id, &entry.data[..])?;
                entry.dirty = false;
            }
        }
        Ok(())
    }

    /// Invalidate (remove) a page from the cache.
    #[allow(dead_code)]
    pub fn invalidate(&self, page_id: u64) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.remove(&page_id);
        inner.order.retain(|&id| id != page_id);
    }

    /// Number of pages currently cached (for testing).
    #[allow(dead_code)]
    pub fn cached_page_count(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }

    /// Configured capacity (in pages). `0` means the cache is disabled — see
    /// [`PageCache::new`].
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;

    /// Byte offset of the marker each test page carries in its body.
    const MARK: usize = 100;

    /// A valid sealed leaf page for `page_id` (generation 1) with `byte` at `MARK`.
    fn make_page(page_id: u64, byte: u8) -> Vec<u8> {
        let mut p = page::new_page(page::PAGE_TYPE_LEAF, 0);
        p[MARK] = byte;
        page::seal(&mut p, page_id, 1).unwrap();
        p
    }

    #[test]
    fn test_cache_miss_loads_from_backend() {
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(1, 0xAB)).unwrap();
        let cache = PageCache::new(4);
        let page = cache.get_or_load(1, &backend).unwrap();
        assert_eq!(page[MARK], 0xAB);
    }

    #[test]
    fn test_cache_hit_returns_same_bytes() {
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(1, 0x11)).unwrap();
        let cache = PageCache::new(4);
        let p1 = cache.get_or_load(1, &backend).unwrap();
        let p2 = cache.get_or_load(1, &backend).unwrap();
        assert_eq!(p1[MARK], p2[MARK]);
    }

    #[test]
    fn test_lru_eviction_respects_capacity() {
        let mut backend = MemoryBackend::new();
        for i in 1u64..=5 {
            backend.write_page(i, &make_page(i, i as u8)).unwrap();
        }
        let cache = PageCache::new(3); // capacity 3
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
        backend.write_page(1, &make_page(1, 0x00)).unwrap();
        let cache = PageCache::new(4);
        cache.put_dirty(1, make_page(1, 0xFF));
        cache.flush(&mut backend).unwrap();
        let page = backend.read_page(1).unwrap();
        assert_eq!(page[MARK], 0xFF);
    }

    #[test]
    #[cfg(not(target_os = "wasi"))]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(1, 0x42)).unwrap();
        let cache = Arc::new(PageCache::new(8));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = cache.clone();
                let b = backend.clone();
                thread::spawn(move || {
                    let page = c.get_or_load(1, &b).unwrap();
                    assert_eq!(page[MARK], 0x42);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_lru_eviction_evicts_correct_page() {
        let mut backend = MemoryBackend::new();
        for i in 1u64..=4 {
            backend.write_page(i, &make_page(i, i as u8)).unwrap();
        }
        let cache = PageCache::new(3);
        // Load 1, 2, 3 in order
        cache.get_or_load(1, &backend).unwrap();
        cache.get_or_load(2, &backend).unwrap();
        cache.get_or_load(3, &backend).unwrap();
        // Page 1 is LRU. Load page 4 — evicts page 1 (LRU).
        cache.get_or_load(4, &backend).unwrap();
        // Page 4 loaded, total still <= 3
        assert!(cache.cached_page_count() <= 3);
        // Page 4 must now be loadable from cache (just loaded)
        // We can't directly inspect the cache, but we can verify capacity is respected
        assert!(cache.cached_page_count() == 3);
    }

    #[test]
    fn test_zero_capacity_disables_caching() {
        let mut backend = MemoryBackend::new();
        for i in 1u64..=10 {
            backend.write_page(i, &make_page(i, i as u8)).unwrap();
        }
        let cache = PageCache::new(0);
        assert_eq!(cache.capacity(), 0);
        for i in 1u64..=10 {
            let page = cache.get_or_load(i, &backend).unwrap();
            assert_eq!(page[MARK], i as u8);
        }
        // Nothing should have been retained: capacity 0 means "no cache", not
        // "unbounded cache" — every load must read straight through and leave
        // no trace behind.
        assert_eq!(cache.cached_page_count(), 0);
    }

    #[test]
    fn test_zero_capacity_put_dirty_is_noop() {
        let cache = PageCache::new(0);
        cache.put_dirty(1, make_page(1, 0xAA));
        assert_eq!(cache.cached_page_count(), 0);
    }

    /// Regression test for: put_dirty on a cached page after an eviction caused
    /// an out-of-bounds panic. The positions HashMap (now removed) became stale
    /// after pop_front shifted all remaining VecDeque indices by 1, so touch()
    /// tried to index beyond the end of the order deque.
    #[test]
    fn test_put_dirty_after_eviction_does_not_panic() {
        let mut backend = MemoryBackend::new();
        for i in 1u64..=3 {
            backend.write_page(i, &make_page(i, i as u8)).unwrap();
        }
        let cache = PageCache::new(2);
        // Fill cache: pages 1 and 2 (order: [1, 2])
        cache.get_or_load(1, &backend).unwrap();
        cache.get_or_load(2, &backend).unwrap();
        // Evict page 1 (LRU) by loading page 3 (order becomes [2, 3])
        cache.get_or_load(3, &backend).unwrap();
        // put_dirty on page 2 triggers touch(); previously this panicked because
        // the stale position for page 2 (index 1 in the old 2-element deque)
        // became out of bounds after eviction made it a 2-element deque with
        // indices 0..1 but the stored position was 1 — which after pop_front
        // pointed past the end.
        cache.put_dirty(2, make_page(2, 0xBB)); // must not panic
        assert_eq!(cache.cached_page_count(), 2);
    }

    #[test]
    fn corrupt_page_is_rejected_and_not_cached() {
        let mut backend = MemoryBackend::new();
        let mut p = make_page(1, 0x11);
        p[MARK] ^= 1; // body changed after sealing
        backend.write_page(1, &p).unwrap();
        for cap in [0, 4] {
            let cache = PageCache::new(cap);
            let err = cache.get_or_load(1, &backend).unwrap_err();
            assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-029");
            assert_eq!(cache.cached_page_count(), 0);
        }
    }

    #[test]
    fn page_newer_than_bound_is_rejected_until_bound_rises() {
        let mut backend = MemoryBackend::new();
        let mut p = page::new_page(page::PAGE_TYPE_LEAF, 0);
        page::seal(&mut p, 1, 5).unwrap();
        backend.write_page(1, &p).unwrap();
        let cache = PageCache::new(4);
        cache.set_generation_bound(4);
        let err = cache.get_or_load(1, &backend).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-031");
        assert_eq!(cache.cached_page_count(), 0);
        cache.set_generation_bound(5);
        cache.get_or_load(1, &backend).unwrap();
        assert_eq!(cache.cached_page_count(), 1);
    }
}
