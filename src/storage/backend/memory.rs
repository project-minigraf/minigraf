/// In-memory storage backend for testing and embedded use.

use crate::storage::{StorageBackend, PAGE_SIZE};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// In-memory storage backend.
///
/// Stores pages in a HashMap for fast access. Suitable for:
/// - Testing
/// - Embedded systems with no filesystem
/// - Temporary graphs that don't need persistence
///
/// This is the same backend used in the PoC, now wrapped in the trait.
#[derive(Clone)]
pub struct MemoryBackend {
    pages: Arc<RwLock<HashMap<u64, Vec<u8>>>>,
}

impl MemoryBackend {
    /// Create a new in-memory storage backend.
    pub fn new() -> Self {
        MemoryBackend {
            pages: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of pages stored.
    fn page_count_internal(&self) -> u64 {
        self.pages.read().unwrap().len() as u64
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for MemoryBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        if data.len() != PAGE_SIZE {
            anyhow::bail!(
                "Invalid page size: {} bytes (expected {})",
                data.len(),
                PAGE_SIZE
            );
        }

        let mut pages = self.pages.write().unwrap();
        pages.insert(page_id, data.to_vec());
        Ok(())
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        let pages = self.pages.read().unwrap();
        pages
            .get(&page_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Page {} not found", page_id))
    }

    fn sync(&mut self) -> Result<()> {
        // No-op for in-memory storage
        Ok(())
    }

    fn page_count(&self) -> Result<u64> {
        Ok(self.page_count_internal())
    }

    fn close(&mut self) -> Result<()> {
        // No-op for in-memory storage
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_backend_write_read() {
        let mut backend = MemoryBackend::new();

        let data = vec![42u8; PAGE_SIZE];
        backend.write_page(0, &data).unwrap();

        let read_data = backend.read_page(0).unwrap();
        assert_eq!(data, read_data);
    }

    #[test]
    fn test_memory_backend_invalid_size() {
        let mut backend = MemoryBackend::new();

        let data = vec![42u8; 100]; // Wrong size
        let result = backend.write_page(0, &data);
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_backend_read_missing() {
        let backend = MemoryBackend::new();
        let result = backend.read_page(999);
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_backend_page_count() {
        let mut backend = MemoryBackend::new();
        assert_eq!(backend.page_count().unwrap(), 0);

        backend.write_page(0, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 1);

        backend.write_page(1, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 2);
    }
}
