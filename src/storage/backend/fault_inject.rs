//! Fault-injecting storage backend for reliability testing.
//!
//! Wraps any [`StorageBackend`] and injects `io::Error` on configurable call counts.
//! Used by WAL crash-recovery and durability tests.
//!
//! This module is only compiled in test builds (`#[cfg(test)]`).

use crate::storage::StorageBackend;
use anyhow::Result;
use std::io;
use std::sync::{Arc, Mutex};

/// Configuration for fault injection. All counts start at 0 and increment per call.
/// Set `fail_*_after` to `Some(N)` to inject an error after N successful calls.
/// `None` means never fail.
#[derive(Debug, Default)]
pub struct FaultConfig {
    /// Fail `write_page` after this many successful calls.
    pub fail_write_after: Option<u64>,
    /// Fail `sync` after this many successful calls.
    pub fail_sync_after: Option<u64>,
    /// Fail `close` after this many successful calls.
    pub fail_close_after: Option<u64>,
    /// When the write that `fail_write_after` fails is reached, first write the
    /// leading `n` bytes of the new page over the old contents (a torn write),
    /// then fail. `None` leaves the old page untouched.
    pub torn_write_bytes: Option<usize>,
    write_count: u64,
    sync_count: u64,
    close_count: u64,
}

impl FaultConfig {
    fn check_and_increment(count: &mut u64, limit: Option<u64>) -> Result<()> {
        if limit.is_some_and(|n| *count >= n) {
            return Err(anyhow::Error::new(io::Error::other(
                "fault injection: simulated I/O error",
            )));
        }
        *count += 1;
        Ok(())
    }
}

/// A storage backend wrapper that injects failures at configurable call counts.
pub struct FaultInjectingBackend<B: StorageBackend> {
    inner: B,
    config: Arc<Mutex<FaultConfig>>,
}

impl<B: StorageBackend> FaultInjectingBackend<B> {
    /// Convenience constructor: returns the backend AND a shared config handle.
    pub fn with_config(inner: B) -> (Self, Arc<Mutex<FaultConfig>>) {
        let config = Arc::new(Mutex::new(FaultConfig::default()));
        let backend = FaultInjectingBackend {
            inner,
            config: config.clone(),
        };
        (backend, config)
    }
}

impl<B: StorageBackend> StorageBackend for FaultInjectingBackend<B> {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        let limit = cfg.fail_write_after;
        let torn = cfg.torn_write_bytes;
        let result = FaultConfig::check_and_increment(&mut cfg.write_count, limit);
        drop(cfg);
        if let Err(e) = result {
            if let Some(n) = torn {
                let mut page = self
                    .inner
                    .read_page(page_id)
                    .unwrap_or_else(|_| vec![0u8; data.len()]);
                let n = n.min(data.len()).min(page.len());
                page[..n].copy_from_slice(&data[..n]);
                self.inner.write_page(page_id, &page)?;
            }
            return Err(e);
        }
        self.inner.write_page(page_id, data)
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.inner.read_page(page_id)
    }

    fn sync(&mut self) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        let limit = cfg.fail_sync_after;
        FaultConfig::check_and_increment(&mut cfg.sync_count, limit)?;
        drop(cfg);
        self.inner.sync()
    }

    fn page_count(&self) -> Result<u64> {
        self.inner.page_count()
    }

    fn holds_every_page(&self) -> bool {
        self.inner.holds_every_page()
    }

    fn close(&mut self) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        let limit = cfg.fail_close_after;
        FaultConfig::check_and_increment(&mut cfg.close_count, limit)?;
        drop(cfg);
        self.inner.close()
    }

    fn backend_name(&self) -> &'static str {
        "fault-injecting"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::PAGE_SIZE;
    use crate::storage::backend::MemoryBackend;

    fn make_page() -> Vec<u8> {
        vec![0xAB; PAGE_SIZE]
    }

    #[test]
    fn fault_injecting_backend_fails_on_nth_write() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        assert!(
            backend.write_page(0, &make_page()).is_ok(),
            "first write should succeed"
        );
        config.lock().unwrap().fail_write_after = Some(1);
        let result = backend.write_page(1, &make_page());
        assert!(
            result.is_err(),
            "second write should fail after fault injection"
        );
    }

    #[test]
    fn fault_injecting_backend_fails_on_nth_sync() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        assert!(backend.sync().is_ok(), "first sync should succeed");
        config.lock().unwrap().fail_sync_after = Some(1);
        let result = backend.sync();
        assert!(
            result.is_err(),
            "second sync should fail after fault injection"
        );
    }

    #[test]
    fn reads_are_never_faulted() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        backend.write_page(0, &make_page()).unwrap();
        config.lock().unwrap().fail_write_after = Some(0);
        assert!(
            backend.read_page(0).is_ok(),
            "reads should never be faulted"
        );
    }

    #[test]
    fn config_can_be_updated_mid_scenario() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        for i in 0..3 {
            assert!(backend.write_page(i, &make_page()).is_ok());
        }
        config.lock().unwrap().fail_write_after = Some(3);
        assert!(
            backend.write_page(3, &make_page()).is_err(),
            "4th write should fail"
        );
        config.lock().unwrap().fail_write_after = None;
        assert!(
            backend.write_page(4, &make_page()).is_ok(),
            "write should succeed after removing fault"
        );
    }

    #[test]
    fn torn_write_keeps_a_prefix_of_the_new_page() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        backend.write_page(0, &vec![0x11; PAGE_SIZE]).unwrap();
        {
            let mut cfg = config.lock().unwrap();
            cfg.fail_write_after = Some(1);
            cfg.torn_write_bytes = Some(100);
        }
        assert!(backend.write_page(0, &vec![0x22; PAGE_SIZE]).is_err());
        let page = backend.read_page(0).unwrap();
        assert!(page[..100].iter().all(|&b| b == 0x22), "new prefix");
        assert!(page[100..].iter().all(|&b| b == 0x11), "old suffix");
    }
}
