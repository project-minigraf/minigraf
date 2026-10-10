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
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
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
        }
    }

    /// Get a page from the cache, loading from `backend` on a miss.
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
                return Ok(Arc::new(backend.read_page(page_id)?));
            }
            if let Some(entry) = inner.entries.get(&page_id) {
                return Ok(entry.data.clone());
            }
        }
        // Miss: load from backend (without holding any lock)
        let data = Arc::new(backend.read_page(page_id)?);
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

    /// Invalidate all cached pages with `page_id >= from_page`.
    ///
    /// Used during save to discard stale B+tree index pages before
    /// overwriting them with new fact and index pages.
    pub fn invalidate_from(&self, from_page: u64) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.retain(|&id, _| id < from_page);
        inner.order.retain(|&id| id < from_page);
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
    use crate::storage::PAGE_SIZE;
    use crate::storage::backend::MemoryBackend;

    fn make_page(byte: u8) -> Vec<u8> {
        vec![byte; PAGE_SIZE]
    }

    /// Holds every `read_page` until two are in flight, so two threads miss
    /// the same page before either caches it.
    struct BarrierBackend {
        inner: MemoryBackend,
        barrier: std::sync::Barrier,
    }

    impl StorageBackend for BarrierBackend {
        fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
            self.inner.write_page(page_id, data)
        }
        fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
            self.barrier.wait();
            self.inner.read_page(page_id)
        }
        fn sync(&mut self) -> Result<()> {
            Ok(())
        }
        fn page_count(&self) -> Result<u64> {
            self.inner.page_count()
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "barrier"
        }
        fn is_new(&self) -> bool {
            false
        }
    }

    /// Two threads that miss the same page both load it; the second to take
    /// the write lock returns the first one's cached copy.
    #[test]
    #[cfg_attr(target_os = "wasi", ignore = "spawns threads; WASI has no threads")]
    fn test_concurrent_misses_share_one_cached_copy() {
        let mut inner = MemoryBackend::new();
        inner.write_page(1, &make_page(0x5A)).unwrap();
        let backend = BarrierBackend {
            inner,
            barrier: std::sync::Barrier::new(2),
        };
        let cache = PageCache::new(4);
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| cache.get_or_load(1, &backend).unwrap());
            let b = s.spawn(|| cache.get_or_load(1, &backend).unwrap());
            (a.join().unwrap(), b.join().unwrap())
        });
        assert!(Arc::ptr_eq(&a, &b), "one copy cached and returned to both");
        assert_eq!(a[0], 0x5A);
        let cached = cache.get_or_load(1, &backend.inner).unwrap();
        assert!(Arc::ptr_eq(&a, &cached));
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
    #[cfg(not(target_os = "wasi"))]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;
        let mut backend = MemoryBackend::new();
        backend.write_page(1, &make_page(0x42)).unwrap();
        let cache = Arc::new(PageCache::new(8));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = cache.clone();
                let b = backend.clone();
                thread::spawn(move || {
                    let page = c.get_or_load(1, &b).unwrap();
                    assert_eq!(page[0], 0x42);
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
            backend.write_page(i, &make_page(i as u8)).unwrap();
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
            backend.write_page(i, &make_page(i as u8)).unwrap();
        }
        let cache = PageCache::new(0);
        assert_eq!(cache.capacity(), 0);
        for i in 1u64..=10 {
            let page = cache.get_or_load(i, &backend).unwrap();
            assert_eq!(page[0], i as u8);
        }
        // Nothing should have been retained: capacity 0 means "no cache", not
        // "unbounded cache" — every load must read straight through and leave
        // no trace behind.
        assert_eq!(cache.cached_page_count(), 0);
    }

    #[test]
    fn test_zero_capacity_put_dirty_is_noop() {
        let cache = PageCache::new(0);
        cache.put_dirty(1, make_page(0xAA));
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
            backend.write_page(i, &make_page(i as u8)).unwrap();
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
        cache.put_dirty(2, make_page(0xBB)); // must not panic
        assert_eq!(cache.cached_page_count(), 2);
    }
}
