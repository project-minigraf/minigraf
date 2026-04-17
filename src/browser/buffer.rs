use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// Synchronous in-memory page buffer with dirty-page tracking.
///
/// Implements `StorageBackend` so it can be used with `PersistentFactStorage`.
/// After `PersistentFactStorage::save()` writes updated pages here, call
/// `take_dirty()` to retrieve the page IDs that must be flushed to IndexedDB.
pub struct BrowserBufferBackend {
    pages: HashMap<u64, Vec<u8>>,
    dirty: HashSet<u64>,
}

impl BrowserBufferBackend {
    /// Create an empty buffer (new database).
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            dirty: HashSet::new(),
        }
    }

    /// Load pages from an existing snapshot. Dirty set starts empty.
    /// Used during `BrowserDb::open()` after fetching all pages from IndexedDB.
    pub fn load_pages(pages: HashMap<u64, Vec<u8>>) -> Self {
        Self {
            pages,
            dirty: HashSet::new(),
        }
    }

    /// Load pages and mark every page dirty.
    /// Used during `BrowserDb::import_graph()` so all pages are flushed to IDB.
    pub fn load_pages_all_dirty(pages: HashMap<u64, Vec<u8>>) -> Self {
        let dirty: HashSet<u64> = pages.keys().copied().collect();
        Self { pages, dirty }
    }

    /// Drain and return the set of page IDs written since the last call.
    /// Clears the dirty set. Call after `pfs.save()` to get pages to flush.
    pub fn take_dirty(&mut self) -> HashSet<u64> {
        std::mem::take(&mut self.dirty)
    }
}

impl Default for BrowserBufferBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for BrowserBufferBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        if data.len() != PAGE_SIZE {
            anyhow::bail!(
                "Invalid page size: {} bytes (expected {})",
                data.len(),
                PAGE_SIZE
            );
        }
        self.pages.insert(page_id, data.to_vec());
        self.dirty.insert(page_id);
        Ok(())
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.pages
            .get(&page_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Page {} not found", page_id))
    }

    fn sync(&mut self) -> Result<()> {
        Ok(()) // no-op: durability handled by IndexedDbBackend
    }

    fn page_count(&self) -> Result<u64> {
        Ok(self.pages.len() as u64)
    }

    fn close(&mut self) -> Result<()> {
        Ok(()) // no-op
    }

    fn backend_name(&self) -> &'static str {
        "browser-buffer"
    }

    fn is_new(&self) -> bool {
        self.pages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(byte: u8) -> Vec<u8> {
        vec![byte; PAGE_SIZE]
    }

    #[test]
    fn write_marks_dirty() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(1)).unwrap();
        let dirty = buf.take_dirty();
        assert!(dirty.contains(&0));
    }

    #[test]
    fn take_dirty_clears_set() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(1)).unwrap();
        let _ = buf.take_dirty();
        assert!(buf.take_dirty().is_empty());
    }

    #[test]
    fn read_after_write_returns_same_bytes() {
        let mut buf = BrowserBufferBackend::new();
        let p = page(42);
        buf.write_page(3, &p).unwrap();
        assert_eq!(buf.read_page(3).unwrap(), p);
    }

    #[test]
    fn page_count_reflects_distinct_ids() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(0)).unwrap();
        buf.write_page(1, &page(1)).unwrap();
        buf.write_page(0, &page(2)).unwrap(); // overwrite
        assert_eq!(buf.page_count().unwrap(), 2);
    }

    #[test]
    fn load_pages_starts_with_no_dirty() {
        let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
        let mut buf = BrowserBufferBackend::load_pages(pages);
        assert!(buf.take_dirty().is_empty());
    }

    #[test]
    fn load_pages_all_dirty_marks_all() {
        let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
        let mut buf = BrowserBufferBackend::load_pages_all_dirty(pages);
        let dirty = buf.take_dirty();
        assert!(dirty.contains(&0));
        assert!(dirty.contains(&1));
    }

    #[test]
    fn is_new_true_when_empty() {
        assert!(BrowserBufferBackend::new().is_new());
    }

    #[test]
    fn is_new_false_after_write() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(0)).unwrap();
        assert!(!buf.is_new());
    }

    #[test]
    fn wrong_page_size_errors() {
        let mut buf = BrowserBufferBackend::new();
        assert!(buf.write_page(0, &[0u8; 100]).is_err());
    }

    #[test]
    fn read_missing_page_errors() {
        let buf = BrowserBufferBackend::new();
        assert!(buf.read_page(99).is_err());
    }
}
