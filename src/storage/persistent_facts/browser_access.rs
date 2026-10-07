//! Direct page access for the browser layer (`src/browser`), which flushes
//! the pages `save()` dirtied to IndexedDB. Built only for wasm32 with the
//! `browser` feature.

use super::PersistentFactStorage;
use crate::storage::{StorageBackend, freelist};
use anyhow::Result;

impl<B: StorageBackend + 'static> PersistentFactStorage<B> {
    /// Run a closure with read access to the underlying storage backend.
    ///
    /// Used by the browser WASM layer to read pages after `save()` without
    /// exposing the `Arc<Mutex<B>>` directly.
    pub(crate) fn with_backend<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&B) -> R,
    {
        // wasm32 aborts on panic, so the lock is never poisoned.
        let guard = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&*guard)
    }

    /// Run a closure with mutable access to the underlying storage backend.
    ///
    /// Used by the browser WASM layer to drain dirty pages after `save()`.
    pub(crate) fn with_backend_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut B) -> R,
    {
        let mut guard = self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut *guard)
    }

    /// The ids the last `save()` put on the free list, clearing the record.
    /// Pages already free before that commit are not repeated (#440).
    pub(crate) fn take_released(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released)
    }

    /// Every id on the active meta's free list, read from its chain. Used once
    /// at open; after that, [`take_released`](Self::take_released) gives the
    /// change per commit.
    pub(crate) fn free_page_ids(&self) -> Result<Vec<u64>> {
        if self.meta.freelist_head == 0 {
            return Ok(Vec::new());
        }
        let backend = self.lock()?;
        let (ids, _) = freelist::read_chain(
            self.meta.freelist_head,
            &*backend,
            &self.page_cache,
            self.meta.page_count,
        )?;
        Ok(ids)
    }

    /// The active meta's `page_count`: the length of the file it describes.
    pub(crate) fn committed_page_count(&self) -> u64 {
        self.meta.page_count
    }
}
