use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// Synchronous in-memory page buffer with dirty-page tracking.
///
/// Implements `StorageBackend` so it can be used with `PersistentFactStorage`.
/// After `PersistentFactStorage::save()` writes updated pages here, call
/// `take_dirty()` to retrieve the page IDs that must be flushed to IndexedDB.
///
/// A page on the free list holds no bytes (#440): [`release`](Self::release)
/// drops them. Reading a released page is an error like any absent page; the
/// allocator always writes a free page before anything reads it.
pub struct BrowserBufferBackend {
    pages: HashMap<u64, Vec<u8>>,
    dirty: HashSet<u64>,
    /// Free pages whose bytes were dropped. Only export reads this, to write
    /// them as zeros.
    released: HashSet<u64>,
}

impl BrowserBufferBackend {
    /// Create an empty buffer (new database).
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            dirty: HashSet::new(),
            released: HashSet::new(),
        }
    }

    /// Load pages from an existing snapshot. Dirty set starts empty.
    /// Used during `BrowserDb::open()` after fetching all pages from IndexedDB.
    pub fn load_pages(pages: HashMap<u64, Vec<u8>>) -> Self {
        Self {
            pages,
            dirty: HashSet::new(),
            released: HashSet::new(),
        }
    }

    /// Load pages and mark every page dirty.
    /// Used during `BrowserDb::import_graph()` so all pages are flushed to IDB.
    pub fn load_pages_all_dirty(pages: HashMap<u64, Vec<u8>>) -> Self {
        let dirty: HashSet<u64> = pages.keys().copied().collect();
        Self {
            pages,
            dirty,
            released: HashSet::new(),
        }
    }

    /// Drain and return the set of page IDs written since the last call.
    /// Clears the dirty set. Call after `pfs.save()` to get pages to flush.
    pub fn take_dirty(&mut self) -> HashSet<u64> {
        std::mem::take(&mut self.dirty)
    }

    /// Drop the bytes of the free pages `ids` and forget any unflushed write to
    /// them. Returns the ids that held bytes: the IndexedDB keys to delete in
    /// the same transaction as the commit's meta page.
    pub fn release(&mut self, ids: impl IntoIterator<Item = u64>) -> Vec<u64> {
        let mut dropped = Vec::new();
        for id in ids {
            self.dirty.remove(&id);
            if self.pages.remove(&id).is_some() {
                dropped.push(id);
            }
            self.released.insert(id);
        }
        dropped
    }

    /// Page `page_id` for export: a released page is all zeros, any other
    /// absent page is an error.
    pub fn export_page(&self, page_id: u64) -> Result<Vec<u8>> {
        if !self.pages.contains_key(&page_id) && self.released.contains(&page_id) {
            return Ok(vec![0u8; PAGE_SIZE]);
        }
        self.read_page(page_id)
    }

    /// The ids of the pages holding bytes.
    pub fn page_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.pages.keys().copied()
    }

    /// The number of pages holding bytes.
    pub fn stored_page_count(&self) -> usize {
        self.pages.len()
    }

    /// Mark `page_id` dirty without writing it, for tests of the flush path.
    #[cfg(test)]
    pub(crate) fn mark_dirty_for_test(&mut self, page_id: u64) {
        self.dirty.insert(page_id);
    }
}

impl Default for BrowserBufferBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserBufferBackend {
    /// Read a page by ID (delegates to `StorageBackend::read_page`, usable without the trait).
    pub fn read_page_raw(&self, page_id: u64) -> anyhow::Result<Vec<u8>> {
        self.read_page(page_id)
    }

    /// Return the number of pages stored (delegates to `StorageBackend::page_count`, usable without the trait).
    pub fn page_count_raw(&self) -> anyhow::Result<u64> {
        self.page_count()
    }
}

impl StorageBackend for BrowserBufferBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        if data.len() != PAGE_SIZE {
            bail_coded!(ErrorCode::Int051, data.len(), PAGE_SIZE);
        }
        self.pages.insert(page_id, data.to_vec());
        self.dirty.insert(page_id);
        self.released.remove(&page_id);
        Ok(())
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.pages
            .get(&page_id)
            .cloned()
            .ok_or_else(|| err_coded!(ErrorCode::Int052, page_id))
    }

    fn sync(&mut self) -> Result<()> {
        Ok(()) // no-op: durability handled by IndexedDbBackend
    }

    /// One past the highest page id held (0 when empty), like a file's length
    /// in pages.
    fn page_count(&self) -> Result<u64> {
        Ok(self.pages.keys().max().map_or(0, |m| m.saturating_add(1)))
    }

    /// Released (free) pages are dropped, so the highest stored page can be
    /// below the meta's `page_count`.
    fn holds_every_page(&self) -> bool {
        false
    }

    fn close(&mut self) -> Result<()> {
        Ok(()) // no-op
    }

    fn backend_name(&self) -> &'static str {
        "browser-buffer"
    }
}

#[cfg(test)]
mod tests {
    // The browser module only builds for wasm32, where the test runner runs
    // `#[wasm_bindgen_test]` functions, not `#[test]` ones.
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn page(byte: u8) -> Vec<u8> {
        vec![byte; PAGE_SIZE]
    }

    #[wasm_bindgen_test]
    fn write_marks_dirty() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(1)).unwrap();
        let dirty = buf.take_dirty();
        assert!(dirty.contains(&0));
    }

    #[wasm_bindgen_test]
    fn take_dirty_clears_set() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(1)).unwrap();
        let _ = buf.take_dirty();
        assert!(buf.take_dirty().is_empty());
    }

    #[wasm_bindgen_test]
    fn read_after_write_returns_same_bytes() {
        let mut buf = BrowserBufferBackend::new();
        let p = page(42);
        buf.write_page(3, &p).unwrap();
        assert_eq!(buf.read_page(3).unwrap(), p);
    }

    #[wasm_bindgen_test]
    fn page_count_is_high_water_mark() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(0, &page(0)).unwrap();
        buf.write_page(1, &page(1)).unwrap();
        buf.write_page(0, &page(2)).unwrap(); // overwrite
        assert_eq!(buf.page_count().unwrap(), 2);
        buf.write_page(5, &page(3)).unwrap(); // sparse
        assert_eq!(buf.page_count().unwrap(), 6);
        // Free pages are dropped, so open does not check a meta's page_count
        // against this one (#497).
        assert!(!buf.holds_every_page());
    }

    #[wasm_bindgen_test]
    fn load_pages_starts_with_no_dirty() {
        let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
        let mut buf = BrowserBufferBackend::load_pages(pages);
        assert!(buf.take_dirty().is_empty());
    }

    #[wasm_bindgen_test]
    fn load_pages_all_dirty_marks_all() {
        let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
        let mut buf = BrowserBufferBackend::load_pages_all_dirty(pages);
        let dirty = buf.take_dirty();
        assert!(dirty.contains(&0));
        assert!(dirty.contains(&1));
    }

    #[wasm_bindgen_test]
    fn empty_buffer_has_no_pages() {
        assert_eq!(BrowserBufferBackend::new().page_count().unwrap(), 0);
    }

    #[wasm_bindgen_test]
    fn wrong_page_size_errors() {
        let mut buf = BrowserBufferBackend::new();
        assert!(buf.write_page(0, &[0u8; 100]).is_err());
    }

    #[wasm_bindgen_test]
    fn release_drops_bytes_and_dirty_and_reports_stored_ids() {
        let mut buf = BrowserBufferBackend::load_pages(HashMap::from([(2u64, page(2))]));
        buf.write_page(3, &page(3)).unwrap();
        let mut dropped = buf.release([2, 3, 4]);
        dropped.sort_unstable();
        assert_eq!(dropped, vec![2, 3], "page 4 held no bytes");
        assert!(
            buf.take_dirty().is_empty(),
            "a released write is not flushed"
        );
        assert!(buf.read_page(2).is_err(), "a released page reads as absent");
        assert_eq!(buf.stored_page_count(), 0);
        assert!(
            buf.release([2]).is_empty(),
            "releasing twice deletes nothing"
        );
    }

    #[wasm_bindgen_test]
    fn export_page_zeroes_released_pages_only() {
        let mut buf = BrowserBufferBackend::new();
        buf.write_page(2, &page(9)).unwrap();
        let _ = buf.release([2]);
        assert_eq!(buf.export_page(2).unwrap(), page(0));
        assert!(
            buf.export_page(5).is_err(),
            "an absent page that is not free"
        );
        buf.write_page(2, &page(7)).unwrap();
        assert_eq!(
            buf.export_page(2).unwrap(),
            page(7),
            "rewritten after reuse"
        );
    }

    #[wasm_bindgen_test]
    fn read_missing_page_errors() {
        let buf = BrowserBufferBackend::new();
        assert!(buf.read_page(99).is_err());
    }
}
