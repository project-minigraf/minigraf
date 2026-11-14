//! The storage under a `BrowserDb`: the in-memory page buffer (in-memory and
//! IndexedDB mode) or an OPFS file (OPFS mode, #441).

use crate::browser::buffer::BrowserBufferBackend;
use crate::browser::opfs::OpfsBackend;
use crate::storage::StorageBackend;
use anyhow::Result;
use std::collections::HashSet;

/// One `BrowserDb`'s backend. The buffer-only methods give an OPFS file no
/// dirty pages and nothing to release: it is written in place, and its free
/// pages are left on the file for the allocator to reuse.
pub(crate) enum BrowserBackend {
    Buffer(BrowserBufferBackend),
    Opfs(OpfsBackend),
}

impl BrowserBackend {
    /// See [`BrowserBufferBackend::take_dirty`].
    pub(crate) fn take_dirty(&mut self) -> HashSet<u64> {
        match self {
            BrowserBackend::Buffer(b) => b.take_dirty(),
            BrowserBackend::Opfs(_) => HashSet::new(),
        }
    }

    /// See [`BrowserBufferBackend::is_dirty`].
    pub(crate) fn is_dirty(&self, page_id: u64) -> bool {
        match self {
            BrowserBackend::Buffer(b) => b.is_dirty(page_id),
            BrowserBackend::Opfs(_) => false,
        }
    }

    /// See [`BrowserBufferBackend::release`].
    pub(crate) fn release(&mut self, ids: impl IntoIterator<Item = u64>) -> Vec<u64> {
        match self {
            BrowserBackend::Buffer(b) => b.release(ids),
            BrowserBackend::Opfs(_) => Vec::new(),
        }
    }

    /// The buffer, unless this is an OPFS file.
    pub(crate) fn buffer(&self) -> Option<&BrowserBufferBackend> {
        match self {
            BrowserBackend::Buffer(b) => Some(b),
            BrowserBackend::Opfs(_) => None,
        }
    }

    /// See [`BrowserBufferBackend::mark_dirty_for_test`].
    #[cfg(test)]
    pub(crate) fn mark_dirty_for_test(&mut self, page_id: u64) {
        if let BrowserBackend::Buffer(b) = self {
            b.mark_dirty_for_test(page_id);
        }
    }
}

impl StorageBackend for BrowserBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        match self {
            BrowserBackend::Buffer(b) => b.write_page(page_id, data),
            BrowserBackend::Opfs(b) => b.write_page(page_id, data),
        }
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        match self {
            BrowserBackend::Buffer(b) => b.read_page(page_id),
            BrowserBackend::Opfs(b) => b.read_page(page_id),
        }
    }

    fn sync(&mut self) -> Result<()> {
        match self {
            BrowserBackend::Buffer(b) => b.sync(),
            BrowserBackend::Opfs(b) => b.sync(),
        }
    }

    fn page_count(&self) -> Result<u64> {
        match self {
            BrowserBackend::Buffer(b) => b.page_count(),
            BrowserBackend::Opfs(b) => b.page_count(),
        }
    }

    fn holds_every_page(&self) -> bool {
        match self {
            BrowserBackend::Buffer(b) => b.holds_every_page(),
            BrowserBackend::Opfs(b) => b.holds_every_page(),
        }
    }

    fn partial_page(&self) -> Result<Option<Vec<u8>>> {
        match self {
            BrowserBackend::Buffer(b) => b.partial_page(),
            BrowserBackend::Opfs(b) => b.partial_page(),
        }
    }

    fn close(&mut self) -> Result<()> {
        match self {
            BrowserBackend::Buffer(b) => b.close(),
            BrowserBackend::Opfs(b) => b.close(),
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            BrowserBackend::Buffer(b) => b.backend_name(),
            BrowserBackend::Opfs(b) => b.backend_name(),
        }
    }
}
