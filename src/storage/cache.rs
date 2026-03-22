//! LRU page cache for bounded-memory page access.
//!
//! `PageCache` caches recently read pages. On a cache miss, the caller passes
//! a `&dyn StorageBackend` reference to load the page. Dirty pages (written
//! via `put_dirty`) are tracked and written back on `flush()`.
//!
//! Interior mutability: all methods take `&self` so the cache can be shared
//! across readers without requiring `&mut`.

use crate::storage::StorageBackend;
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
        // Fast path: check under write lock (we need write lock to touch anyway)
        {
            let mut inner = self.inner.write().unwrap();
            if inner.entries.contains_key(&page_id) {
                inner.touch(page_id);
                let data = inner.entries[&page_id].data.clone();
                return Ok(Arc::new(data));
            }
        }
        // Miss: load from backend (without holding the lock)
        let data = backend.read_page(page_id)?;
        let mut inner = self.inner.write().unwrap();
        // Double-check after acquiring write lock (another thread may have loaded it)
        if inner.entries.contains_key(&page_id) {
            inner.touch(page_id);
            return Ok(Arc::new(inner.entries[&page_id].data.clone()));
        }
        // Evict if at capacity
        while inner.entries.len() >= inner.capacity && inner.capacity > 0 {
            if let Some(id) = inner.order.pop_front() {
                inner.entries.remove(&id);
            }
        }
        inner.entries.insert(page_id, CacheEntry { data: data.clone(), dirty: false });
        inner.order.push_back(page_id);
        Ok(Arc::new(data))
    }

    /// Insert or update a page in the cache and mark it dirty.
    pub fn put_dirty(&self, page_id: u64, data: Vec<u8>) {
        let mut inner = self.inner.write().unwrap();
        // Evict if needed (only if not already present)
        if !inner.entries.contains_key(&page_id) {
            while inner.entries.len() >= inner.capacity && inner.capacity > 0 {
                if let Some(id) = inner.order.pop_front() {
                    inner.entries.remove(&id);
                }
            }
            inner.order.push_back(page_id);
        }
        inner.entries.insert(page_id, CacheEntry { data, dirty: true });
        // Touch to mark as MRU
        if let Some(pos) = inner.order.iter().rposition(|&id| id == page_id) {
            if pos != inner.order.len() - 1 {
                inner.order.remove(pos);
                inner.order.push_back(page_id);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::PAGE_SIZE;

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
