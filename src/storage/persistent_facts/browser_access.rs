//! Direct page access for the browser layer (`src/browser`), which flushes
//! the pages `save()` dirtied to IndexedDB. Built only for wasm32 with the
//! `browser` feature.

use super::PersistentFactStorage;
use crate::storage::StorageBackend;

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
}
