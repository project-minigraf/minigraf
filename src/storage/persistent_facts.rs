use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::FactStorage;
/// Persistent fact storage that integrates StorageBackend with Datalog facts.
///
/// This module bridges the gap between high-level fact operations and
/// low-level page-based storage backends.
use crate::graph::types::Fact;
use crate::storage::FACT_PAGE_FORMAT_PACKED;
use crate::storage::btree_v6::{
    MutexStorageBackend, OnDiskIndexReader, btree_entries, build_btree, merge_sorted_vecs,
    stream_all_entries,
};
use crate::storage::cache::PageCache;
use crate::storage::index::{AevtKey, AvetKey, EavtKey, FactRef, VaetKey, encode_value};
use crate::storage::packed_pages::pack_facts;
use crate::storage::{FileHeader, PAGE_SIZE, StorageBackend};
use anyhow::Result;
use crc32fast::Hasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Compute the CRC32 sync checksum over all facts (used in tests only).
///
/// Sorts facts by `(tx_count, entity_bytes, attribute)` before hashing to
/// produce a stable total order independent of Vec insertion order.
#[cfg(all(test, not(target_arch = "wasm32")))]
fn compute_index_checksum(facts: &[Fact]) -> u32 {
    let mut sorted: Vec<&Fact> = facts.iter().collect();
    sorted.sort_by(|a, b| {
        a.tx_count
            .cmp(&b.tx_count)
            .then_with(|| a.entity.as_bytes().cmp(b.entity.as_bytes()))
            .then_with(|| a.attribute.as_str().cmp(b.attribute.as_str()))
    });
    let mut hasher = Hasher::new();
    for fact in sorted {
        let bytes = postcard::to_allocvec(fact)
            .expect("BUG: failed to serialize Fact for index checksum; this should never happen");
        hasher.update(&bytes);
    }
    hasher.finalize()
}

/// CommittedFactReader backed by a PageCache + shared backend.
///
/// Resolves FactRefs to Fact objects by reading packed pages from the backend
/// through the page cache. Used after loading (or migrating) a v5/v6 file so that indexes can
/// resolve committed facts without keeping the entire fact list in memory.
// Fields are read inside CommittedFactReader trait methods that are accessed via
// dyn dispatch — Rust's dead-code lint does not track trait-impl field reads
// when the impl is behind dyn dispatch.
struct CommittedFactLoaderImpl<B: StorageBackend> {
    #[allow(dead_code)]
    backend: Arc<Mutex<B>>,
    #[allow(dead_code)]
    page_cache: Arc<PageCache>,
    /// Pre-built adapter reused on every `resolve()` call.
    /// Avoids an `Arc::clone` per call: the adapter is constructed once and
    /// holds the `Arc<Mutex<B>>` for the lifetime of this reader.
    /// Backend mutex is still only acquired on cache misses (see `MutexStorageBackend`).
    backend_adapter: MutexStorageBackend<B>,
    committed_fact_pages: Arc<AtomicU64>,
    #[allow(dead_code)]
    first_fact_page: u64, // always 1 in current layout
}

impl<B: StorageBackend + 'static> crate::storage::CommittedFactReader
    for CommittedFactLoaderImpl<B>
{
    fn resolve(
        &self,
        fact_ref: crate::storage::index::FactRef,
    ) -> anyhow::Result<crate::graph::types::Fact> {
        // backend_adapter is pre-built at construction time — no Arc::clone per call.
        // Backend mutex is only acquired inside adapter.read_page() on a cache miss.
        let page = self
            .page_cache
            .get_or_load(fact_ref.page_id, &self.backend_adapter)?;
        crate::storage::packed_pages::read_slot(&page, fact_ref.slot_index)
    }

    fn stream_all(&self) -> anyhow::Result<Vec<crate::graph::types::Fact>> {
        let n = self.committed_fact_pages.load(Ordering::SeqCst);
        let backend = self
            .backend
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Stg016))?;
        crate::storage::packed_pages::read_all_from_pages(&*backend, 1, n)
    }

    fn committed_page_count(&self) -> u64 {
        self.committed_fact_pages.load(Ordering::SeqCst)
    }
}

/// Persistent fact storage with serialization support.
///
/// Architecture:
/// - Page 0: File header (metadata)
/// - Page 1+: Serialized facts (one fact per page, for simplicity)
///
/// # Storage Strategy (Phase 3-5)
///
/// Current implementation uses a simple "load all, save all" approach:
/// - On open: Deserialize all facts into memory (FactStorage)
/// - All operations: Work on in-memory `Vec<Fact>`
/// - On save: Serialize all facts back to disk
///
/// **Trade-offs:**
/// - ✅ Simple, correct, easy to reason about
/// - ✅ Fast queries (no disk I/O)
/// - ✅ Good for embedded use cases with small-medium datasets
/// - ❌ Memory usage = entire database size
/// - ❌ Not scalable to very large datasets
///
/// **Scalability:**
/// - Works well for <100K facts (typical use case)
/// - Memory footprint: ~100-200 bytes per fact
/// - Example: 100K facts ≈ 10-20MB memory (acceptable for embedded)
///
/// # Future: Phase 6 (Performance)
///
/// Phase 6 will introduce page-based access with indexes:
/// - EAVT, AEVT, AVET, VAET indexes (in-memory B-trees)
/// - On-demand fact loading from disk
/// - LRU cache for hot pages
/// - Memory-mapped file access (optional)
/// - Target: Scale to millions of facts with bounded memory
///
/// The page-based backend (StorageBackend) is designed to support this
/// future architecture without breaking changes.
pub struct PersistentFactStorage<B: StorageBackend + 'static> {
    backend: Arc<Mutex<B>>,
    page_cache: Arc<PageCache>,
    storage: FactStorage,
    dirty: bool,
    last_checkpointed_tx_count: u64,
    committed_fact_pages: Arc<AtomicU64>,
}

impl<B: StorageBackend + 'static> PersistentFactStorage<B> {
    /// Create a new persistent storage with the given backend.
    ///
    /// If the backend already contains data, loads it.
    /// Otherwise, initializes a new empty fact storage.
    ///
    /// `page_cache_capacity` controls the LRU page cache size (in pages).
    /// A value of 256 means at most 256 x 4KB = 1MB of cached pages.
    pub fn new(backend: B, page_cache_capacity: usize) -> Result<Self> {
        let backend = Arc::new(Mutex::new(backend));
        let page_cache = Arc::new(PageCache::new(page_cache_capacity));
        let committed_fact_pages = Arc::new(AtomicU64::new(0));
        let mut persistent = PersistentFactStorage {
            backend,
            page_cache,
            storage: FactStorage::new(),
            dirty: false,
            last_checkpointed_tx_count: 0,
            committed_fact_pages,
        };

        // Try to load existing data.
        //
        // The load condition combines two checks:
        // - `!is_new`: FileBackend reports false when the file existed on disk
        //   (even with only a header page, page_count == 1). This ensures
        //   `load()` runs (and its version check rejects pre-v7 files) even
        //   for a file that has no fact pages.
        // - `page_count > 1`: catches MemoryBackend, which always reports
        //   is_new == true; page count > 1 means facts were previously saved.
        let (is_new_backend, page_count) = {
            let b = persistent
                .backend
                .lock()
                .map_err(|_| err_coded!(ErrorCode::Stg016))?;
            (b.is_new(), b.page_count()?)
        };
        if !is_new_backend || page_count > 1 {
            persistent.load()?;
        } else {
            // New database: FileBackend already wrote the initial header;
            // MemoryBackend starts empty. Nothing to save yet.
        }

        Ok(persistent)
    }

    /// The LRU page cache capacity this storage was constructed with (for testing).
    #[allow(dead_code)]
    pub(crate) fn page_cache_capacity(&self) -> usize {
        self.page_cache.capacity()
    }

    /// Load all facts from the backend into memory.
    fn load(&mut self) -> Result<()> {
        let (header, raw_header_bytes) = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| err_coded!(ErrorCode::Stg016))?;
            let header_page = backend.read_page(0)?;
            let h = FileHeader::from_bytes(&header_page)?;
            h.validate()?;
            (h, header_page)
        };

        // For v7+ files, validate header checksum using raw bytes from disk
        if header.header_checksum != 0 {
            let computed = compute_header_checksum_from_bytes(&raw_header_bytes);
            if header.header_checksum != computed {
                bail_coded!(ErrorCode::Int053);
            }
        }

        // Store last_checkpointed_tx_count from header (0 for v2 files)
        self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;

        // Clear existing storage
        self.storage.clear()?;

        // v6 packed format
        let num_fact_pages = if header.fact_page_count > 0 {
            header.fact_page_count
        } else {
            let first_index_page = [
                header.eavt_root_page,
                header.aevt_root_page,
                header.avet_root_page,
                header.vaet_root_page,
            ]
            .iter()
            .filter(|&&p| p > 0)
            .copied()
            .min()
            .unwrap_or(header.page_count);
            first_index_page.saturating_sub(1)
        };
        self.committed_fact_pages
            .store(num_fact_pages, Ordering::SeqCst);

        // Compute page-based checksum to verify data integrity.
        // New files (post-fix): checksum covers ALL pages (facts + indexes).
        // Old files (pre-fix): checksum covers only fact pages.
        // Try full checksum first; fall back to fact-only for backwards compat.
        let needs_rebuild = if num_fact_pages == 0 || header.eavt_root_page == 0 {
            num_fact_pages > 0 // rebuild if facts exist but no index root
        } else {
            let backend = self
                .backend
                .lock()
                .map_err(|_| err_coded!(ErrorCode::Stg016))?;
            let stored = header.index_checksum;
            // Total data pages: pages 1 through page_count-1 (everything except header)
            let total_data_pages = header.page_count.saturating_sub(1);
            let full_checksum = compute_page_checksum(&*backend, 1, total_data_pages)?;
            if full_checksum == stored {
                false // new-style checksum matches: facts + indexes verified
            } else {
                // Fall back: old files stored checksum over fact pages only
                let fact_checksum = compute_page_checksum(&*backend, 1, num_fact_pages)?;
                if fact_checksum == stored {
                    false // old-style checksum matches: facts verified, indexes unprotected
                } else {
                    true // neither matches: corruption detected, rebuild
                }
            }
        };

        // Register CommittedFactReader on FactStorage (before WAL replay)
        let loader: std::sync::Arc<dyn crate::storage::CommittedFactReader> =
            std::sync::Arc::new(CommittedFactLoaderImpl {
                backend: self.backend.clone(),
                backend_adapter: MutexStorageBackend(self.backend.clone()),
                page_cache: self.page_cache.clone(),
                committed_fact_pages: self.committed_fact_pages.clone(),
                first_fact_page: 1,
            });
        self.storage.set_committed_reader(loader);

        // Restore tx_counter from header
        self.storage
            .restore_tx_counter_from(header.last_checkpointed_tx_count);

        if needs_rebuild {
            // Checksum mismatch: rebuild indexes by re-reading all packed facts
            // FactRefs must come from the actual on-disk layout: each save() starts a
            // fresh page, so re-packing all facts contiguously would yield refs that
            // point at the wrong page/slot (#370).
            let (all_facts, real_refs) = {
                let backend = self
                    .backend
                    .lock()
                    .map_err(|_| err_coded!(ErrorCode::Stg016))?;
                crate::storage::packed_pages::read_all_with_refs(&*backend, 1, num_fact_pages)?
            };

            // Build sorted index entries
            let (eavt_entries, aevt_entries, avet_entries, vaet_entries) =
                build_sorted_index_entries(&all_facts, &real_refs);

            // Fix up tx_counter from actual facts
            let max_tx = all_facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
            self.storage.restore_tx_counter_from(max_tx);

            // Build v6 B+tree indexes directly
            let index_start = 1u64
                .checked_add(num_fact_pages)
                .ok_or_else(|| err_coded!(ErrorCode::Stg017))?;
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| err_coded!(ErrorCode::Stg016))?;
            let (eavt_root, next1) = build_btree(
                btree_entries(eavt_entries.into_iter())?.into_iter(),
                &mut *backend,
                &self.page_cache,
                index_start,
            )?;
            let (aevt_root, next2) = build_btree(
                btree_entries(aevt_entries.into_iter())?.into_iter(),
                &mut *backend,
                &self.page_cache,
                next1,
            )?;
            let (avet_root, next3) = build_btree(
                btree_entries(avet_entries.into_iter())?.into_iter(),
                &mut *backend,
                &self.page_cache,
                next2,
            )?;
            let (vaet_root, next4) = build_btree(
                btree_entries(vaet_entries.into_iter())?.into_iter(),
                &mut *backend,
                &self.page_cache,
                next3,
            )?;

            // Write header with full-coverage checksum (facts + indexes)
            let total_data_pages = next4.saturating_sub(1);
            let full_checksum = compute_page_checksum(&*backend, 1, total_data_pages)?;

            let mut new_header = FileHeader::new();
            new_header.page_count = next4;
            new_header.node_count = all_facts.len() as u64;
            new_header.last_checkpointed_tx_count = max_tx;
            new_header.eavt_root_page = eavt_root;
            new_header.aevt_root_page = aevt_root;
            new_header.avet_root_page = avet_root;
            new_header.vaet_root_page = vaet_root;
            new_header.index_checksum = full_checksum;
            new_header.fact_page_format = FACT_PAGE_FORMAT_PACKED;
            new_header.fact_page_count = num_fact_pages;

            let write_checksum = compute_header_checksum(&new_header);
            new_header.header_checksum = write_checksum;

            let mut header_page = new_header.to_bytes();
            header_page.resize(PAGE_SIZE, 0);
            backend.write_page(0, &header_page)?;
            backend.sync()?;
            drop(backend);

            self.last_checkpointed_tx_count = max_tx;

            // Wire OnDiskIndexReader
            let index_reader: std::sync::Arc<dyn crate::storage::CommittedIndexReader> =
                std::sync::Arc::new(OnDiskIndexReader::new(
                    self.backend.clone(),
                    self.page_cache.clone(),
                    eavt_root,
                    aevt_root,
                    avet_root,
                    vaet_root,
                ));
            self.storage.set_committed_index_reader(index_reader);
        } else {
            // No rebuild needed - validate header checksum for v7+ files
            // Re-read header from disk to get any updates from rebuild path
            if header.header_checksum != 0 {
                let backend = self
                    .backend
                    .lock()
                    .map_err(|_| err_coded!(ErrorCode::Stg016))?;
                let current_header_bytes = backend.read_page(0)?;
                let current_header = FileHeader::from_bytes(&current_header_bytes)?;
                let computed = compute_header_checksum_from_bytes(&current_header_bytes);
                if current_header.header_checksum != computed {
                    bail_coded!(ErrorCode::Int053);
                }
            }

            if header.eavt_root_page != 0 {
                // Fast path: v6 — wire OnDiskIndexReader from header roots, no RAM index load
                let index_reader: std::sync::Arc<dyn crate::storage::CommittedIndexReader> =
                    std::sync::Arc::new(OnDiskIndexReader::new(
                        self.backend.clone(),
                        self.page_cache.clone(),
                        header.eavt_root_page,
                        header.aevt_root_page,
                        header.avet_root_page,
                        header.vaet_root_page,
                    ));
                self.storage.set_committed_index_reader(index_reader);
            }
        }
        // else: empty DB — indexes are empty by default, nothing to do.

        self.dirty = false;
        Ok(())
    }

    /// Consume this storage and return the underlying backend.
    ///
    /// Useful in tests to inspect or reuse the backend after saving.
    /// Any dirty (unsaved) changes are saved before the backend is returned.
    ///
    /// Returns an error if the backend Arc has multiple references.
    #[allow(dead_code)]
    pub fn into_backend(mut self) -> Result<B> {
        // Save pending changes before giving up ownership
        if self.dirty {
            let _ = self.save();
        }
        let backend_arc = self.backend.clone();
        // Suppress the Drop impl so we don't double-save.
        self.dirty = false;
        drop(self);
        match Arc::try_unwrap(backend_arc) {
            Ok(mutex) => Ok(mutex
                .into_inner()
                .map_err(|_| err_coded!(ErrorCode::Stg016))?),
            Err(_) => Err(err_coded!(ErrorCode::Int047)),
        }
    }

    /// Save all facts from memory to the backend using packed pages and v6 on-disk B+tree indexes.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        // ── Step A: read current header + stream old B+tree entries BEFORE overwriting ──
        let pending_facts = self.storage.get_pending_facts();
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Stg016))?;

        let old_fact_page_count = self.committed_fact_pages.load(Ordering::SeqCst);
        let new_fact_start = 1u64
            .checked_add(old_fact_page_count)
            .ok_or_else(|| err_coded!(ErrorCode::Stg019))?;

        let curr_header = match backend.read_page(0) {
            Ok(bytes) => FileHeader::from_bytes(&bytes)?,
            Err(_) if backend.is_new() => FileHeader::new(),
            Err(e) => bail_coded!(ErrorCode::Stg010, e),
        };

        // Stream committed B+tree entries BEFORE writing new pages that may overlap
        let committed_eavt: Vec<(EavtKey, FactRef)> = if curr_header.eavt_root_page != 0 {
            stream_all_entries(curr_header.eavt_root_page, &*backend, &self.page_cache)?
        } else {
            Vec::new()
        };
        let committed_aevt: Vec<(AevtKey, FactRef)> = if curr_header.aevt_root_page != 0 {
            stream_all_entries(curr_header.aevt_root_page, &*backend, &self.page_cache)?
        } else {
            Vec::new()
        };
        let committed_avet: Vec<(AvetKey, FactRef)> = if curr_header.avet_root_page != 0 {
            stream_all_entries(curr_header.avet_root_page, &*backend, &self.page_cache)?
        } else {
            Vec::new()
        };
        let committed_vaet: Vec<(VaetKey, FactRef)> = if curr_header.vaet_root_page != 0 {
            stream_all_entries(curr_header.vaet_root_page, &*backend, &self.page_cache)?
        } else {
            Vec::new()
        };

        // Invalidate cached pages that will be overwritten (old index pages)
        self.page_cache.invalidate_from(new_fact_start);

        // ── Step B: pack pending facts as new appended pages ────────────────────
        let (new_pages, new_fact_refs) = pack_facts(&pending_facts, new_fact_start)?;
        for (i, page_data) in new_pages.iter().enumerate() {
            let page_offset = u64::try_from(i).map_err(|_| err_coded!(ErrorCode::Stg023, i))?;
            let page_id = new_fact_start
                .checked_add(page_offset)
                .ok_or_else(|| err_coded!(ErrorCode::Stg022))?;
            backend.write_page(page_id, page_data)?;
        }
        let new_pages_len = u64::try_from(new_pages.len())
            .map_err(|_| err_coded!(ErrorCode::Int048, "new page count exceeds u64::MAX"))?;
        let new_total_fact_pages = old_fact_page_count
            .checked_add(new_pages_len)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "fact page count overflow"))?;

        // Sync fact pages to disk before building indexes on top of them.
        // Without this, a crash during index build could leave partially-flushed
        // fact pages that the old header's index roots would try to traverse.
        backend.sync()?;

        // ── Step C: build sorted index entries for pending facts ────────────────
        let (pending_eavt, pending_aevt, pending_avet, pending_vaet) =
            build_sorted_index_entries(&pending_facts, &new_fact_refs);

        // ── Step D: merge committed + pending entries, build new B+trees ─────────
        let index_start = 1u64
            .checked_add(new_total_fact_pages)
            .ok_or_else(|| err_coded!(ErrorCode::Stg017))?;

        let eavt_ser = if !committed_eavt.is_empty() {
            btree_entries(merge_sorted_vecs(committed_eavt, pending_eavt))?
        } else {
            btree_entries(pending_eavt.into_iter())?
        };
        let (eavt_root, next1) = build_btree(
            eavt_ser.into_iter(),
            &mut *backend,
            &self.page_cache,
            index_start,
        )?;

        let aevt_ser = if !committed_aevt.is_empty() {
            btree_entries(merge_sorted_vecs(committed_aevt, pending_aevt))?
        } else {
            btree_entries(pending_aevt.into_iter())?
        };
        let (aevt_root, next2) =
            build_btree(aevt_ser.into_iter(), &mut *backend, &self.page_cache, next1)?;

        let avet_ser = if !committed_avet.is_empty() {
            btree_entries(merge_sorted_vecs(committed_avet, pending_avet))?
        } else {
            btree_entries(pending_avet.into_iter())?
        };
        let (avet_root, next3) =
            build_btree(avet_ser.into_iter(), &mut *backend, &self.page_cache, next2)?;

        let vaet_ser = if !committed_vaet.is_empty() {
            btree_entries(merge_sorted_vecs(committed_vaet, pending_vaet))?
        } else {
            btree_entries(pending_vaet.into_iter())?
        };
        let (vaet_root, next4) =
            build_btree(vaet_ser.into_iter(), &mut *backend, &self.page_cache, next3)?;

        // Sync index pages to disk before writing the header.
        // The header update is the atomic commit point: once it's durable,
        // recovery uses the new root pages. All data those roots reference
        // must already be on stable storage.
        backend.sync()?;

        // CRC32 over ALL data pages (facts + indexes), excluding page 0 (header).
        // This detects corruption in both fact pages and B+tree index pages.
        let total_data_pages = next4.saturating_sub(1);
        let checksum = compute_page_checksum(&*backend, 1, total_data_pages)?;

        // ── Step E: write header (last write = crash-safe boundary) ─────────────
        let mut header = FileHeader::new(); // version=7
        header.page_count = next4;
        let pending_len =
            u64::try_from(pending_facts.len()).map_err(|_| err_coded!(ErrorCode::Stg024))?;
        header.node_count = curr_header
            .node_count
            .checked_add(pending_len)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "node_count overflow"))?;
        header.last_checkpointed_tx_count = self.storage.current_tx_count();
        header.eavt_root_page = eavt_root;
        header.aevt_root_page = aevt_root;
        header.avet_root_page = avet_root;
        header.vaet_root_page = vaet_root;
        header.index_checksum = checksum;
        header.fact_page_format = FACT_PAGE_FORMAT_PACKED;
        header.fact_page_count = new_total_fact_pages;
        header.header_checksum = compute_header_checksum(&header);

        let mut header_page = header.to_bytes();
        header_page.resize(PAGE_SIZE, 0);
        backend.write_page(0, &header_page)?;
        backend.sync()?;
        drop(backend);

        self.committed_fact_pages
            .store(new_total_fact_pages, Ordering::SeqCst);
        self.last_checkpointed_tx_count = self.storage.current_tx_count();
        self.dirty = false;

        // ── Step F: wire CommittedFactReader and CommittedIndexReader ────────────
        let loader: Arc<dyn crate::storage::CommittedFactReader> =
            Arc::new(CommittedFactLoaderImpl {
                backend: self.backend.clone(),
                backend_adapter: MutexStorageBackend(self.backend.clone()),
                page_cache: self.page_cache.clone(),
                committed_fact_pages: self.committed_fact_pages.clone(),
                first_fact_page: 1,
            });
        self.storage.set_committed_reader(loader);

        let index_reader: Arc<dyn crate::storage::CommittedIndexReader> =
            Arc::new(OnDiskIndexReader::new(
                self.backend.clone(),
                self.page_cache.clone(),
                eavt_root,
                aevt_root,
                avet_root,
                vaet_root,
            ));
        self.storage.set_committed_index_reader(index_reader);

        // Clear pending — all data now on disk
        self.storage.post_checkpoint_clear();

        Ok(())
    }

    /// Get a reference to the underlying fact storage
    pub fn storage(&self) -> &FactStorage {
        &self.storage
    }

    /// The `last_checkpointed_tx_count` recorded in the on-disk header.
    ///
    /// Used by WAL replay to skip entries already present in the main file.
    pub fn last_checkpointed_tx_count(&self) -> u64 {
        self.last_checkpointed_tx_count
    }

    /// Mark storage as dirty (needs saving)
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Force the dirty flag to true regardless of current state.
    ///
    /// Used by checkpoint to ensure save() always writes even if no new
    /// facts have been added since the last save.
    pub fn force_dirty(&mut self) {
        self.mark_dirty();
    }

    /// Check if storage has unsaved changes
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Capacity (in pages) of the internal LRU page cache passed to `new()`.
    ///
    /// Exposed only for the browser WASM layer's tests, which verify that
    /// `BrowserDb` constructors pass `0` (cache disabled — see #275) since
    /// `BrowserBufferBackend` is already an in-memory page store and the LRU
    /// layer on top of it would be redundant.
    #[cfg(all(target_arch = "wasm32", feature = "browser", test))]
    pub(crate) fn page_cache_capacity(&self) -> usize {
        self.page_cache.capacity()
    }

    /// Run a closure with read access to the underlying storage backend.
    ///
    /// Used by the browser WASM layer to read pages after `save()` without
    /// exposing the `Arc<Mutex<B>>` directly.
    #[cfg(all(target_arch = "wasm32", feature = "browser"))]
    pub(crate) fn with_backend<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&B) -> R,
    {
        let guard = self.backend.lock().unwrap();
        f(&*guard)
    }

    /// Run a closure with mutable access to the underlying storage backend.
    ///
    /// Used by the browser WASM layer to drain dirty pages after `save()`.
    #[cfg(all(target_arch = "wasm32", feature = "browser"))]
    pub(crate) fn with_backend_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut B) -> R,
    {
        let mut guard = self.backend.lock().unwrap();
        f(&mut *guard)
    }
}

impl<B: StorageBackend + 'static> Drop for PersistentFactStorage<B> {
    fn drop(&mut self) {
        // Auto-save on drop
        if self.dirty {
            let _ = self.save();
        }
    }
}

/// Compute CRC32 checksum over a range of pages on the backend.
fn compute_page_checksum(
    backend: &dyn StorageBackend,
    first_page: u64,
    num_pages: u64,
) -> Result<u32> {
    let mut hasher = Hasher::new();
    for i in 0..num_pages {
        let page_id = first_page
            .checked_add(i)
            .ok_or_else(|| err_coded!(ErrorCode::Stg021))?;
        let page = backend.read_page(page_id)?;
        hasher.update(&page);
    }
    Ok(hasher.finalize())
}

/// Compute CRC32 checksum over header bytes 0-79 (header_checksum field zeroed).
pub fn compute_header_checksum(header: &FileHeader) -> u32 {
    let mut bytes = header.to_bytes();
    // Zero out bytes 80–83 (the header_checksum field) before hashing.
    // The header is exactly 84 bytes (guaranteed by FileHeader::to_bytes).
    if let Some(b) = bytes.get_mut(80) {
        *b = 0;
    }
    if let Some(b) = bytes.get_mut(81) {
        *b = 0;
    }
    if let Some(b) = bytes.get_mut(82) {
        *b = 0;
    }
    if let Some(b) = bytes.get_mut(83) {
        *b = 0;
    }
    let mut hasher = Hasher::new();
    if let Some(slice) = bytes.get(..80) {
        hasher.update(slice);
    }
    hasher.finalize()
}

/// Compute CRC32 checksum over raw header bytes 0-79 (bytes 80-83 zeroed).
fn compute_header_checksum_from_bytes(bytes: &[u8]) -> u32 {
    let mut data = bytes.to_vec();
    if data.len() < 84 {
        data.resize(84, 0);
    }
    // Zero out bytes 80–83 (the header_checksum field) before hashing.
    if let Some(b) = data.get_mut(80) {
        *b = 0;
    }
    if let Some(b) = data.get_mut(81) {
        *b = 0;
    }
    if let Some(b) = data.get_mut(82) {
        *b = 0;
    }
    if let Some(b) = data.get_mut(83) {
        *b = 0;
    }
    let mut hasher = Hasher::new();
    if let Some(slice) = data.get(..80) {
        hasher.update(slice);
    }
    hasher.finalize()
}

/// Build sorted index entry vecs for a slice of facts and their corresponding FactRefs.
///
/// Returns `(eavt_entries, aevt_entries, avet_entries, vaet_entries)`, each sorted by their
/// respective key type. The `vaet` vec only contains entries whose value is a `Value::Ref`.
#[allow(clippy::type_complexity)]
fn build_sorted_index_entries(
    facts: &[Fact],
    refs: &[FactRef],
) -> (
    Vec<(EavtKey, FactRef)>,
    Vec<(AevtKey, FactRef)>,
    Vec<(AvetKey, FactRef)>,
    Vec<(VaetKey, FactRef)>,
) {
    let mut eavt: Vec<(EavtKey, FactRef)> = facts
        .iter()
        .zip(refs.iter())
        .map(|(f, &fr)| {
            (
                EavtKey {
                    entity: f.entity,
                    attribute: f.attribute.clone(),
                    valid_from: f.valid_from,
                    valid_to: f.valid_to,
                    tx_count: f.tx_count,
                },
                fr,
            )
        })
        .collect();
    eavt.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut aevt: Vec<(AevtKey, FactRef)> = facts
        .iter()
        .zip(refs.iter())
        .map(|(f, &fr)| {
            (
                AevtKey {
                    attribute: f.attribute.clone(),
                    entity: f.entity,
                    valid_from: f.valid_from,
                    valid_to: f.valid_to,
                    tx_count: f.tx_count,
                },
                fr,
            )
        })
        .collect();
    aevt.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut avet: Vec<(AvetKey, FactRef)> = facts
        .iter()
        .zip(refs.iter())
        .map(|(f, &fr)| {
            (
                AvetKey {
                    attribute: f.attribute.clone(),
                    value_bytes: encode_value(&f.value),
                    valid_from: f.valid_from,
                    valid_to: f.valid_to,
                    entity: f.entity,
                    tx_count: f.tx_count,
                },
                fr,
            )
        })
        .collect();
    avet.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut vaet: Vec<(VaetKey, FactRef)> = facts
        .iter()
        .zip(refs.iter())
        .filter_map(|(f, &fr)| {
            if let crate::graph::types::Value::Ref(target) = &f.value {
                Some((
                    VaetKey {
                        ref_target: *target,
                        attribute: f.attribute.clone(),
                        valid_from: f.valid_from,
                        valid_to: f.valid_to,
                        source_entity: f.entity,
                        tx_count: f.tx_count,
                    },
                    fr,
                ))
            } else {
                None
            }
        })
        .collect();
    vaet.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    (eavt, aevt, avet, vaet)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::storage::backend::MemoryBackend;
    use std::io::Write;
    use uuid::Uuid;

    #[test]
    fn test_page_cache_capacity_reflects_constructed_value() {
        let zero_cap = PersistentFactStorage::new(MemoryBackend::new(), 0).unwrap();
        assert_eq!(zero_cap.page_cache_capacity(), 0);

        let nonzero_cap = PersistentFactStorage::new(MemoryBackend::new(), 64).unwrap();
        assert_eq!(nonzero_cap.page_cache_capacity(), 64);
    }

    #[test]
    fn test_persistent_fact_storage_new() {
        let backend = MemoryBackend::new();
        let storage = PersistentFactStorage::new(backend, 256).unwrap();

        // Should be able to create new storage
        assert_eq!(storage.storage().fact_count(), 0);
    }

    #[test]
    fn test_persistent_fact_storage_save_load() {
        // Create separate scopes to test persistence
        let alice = Uuid::new_v4();

        // First session: create and save facts
        {
            let backend = MemoryBackend::new();
            let mut storage = PersistentFactStorage::new(backend, 256).unwrap();

            storage
                .storage()
                .transact(
                    vec![
                        (
                            alice,
                            ":person/name".to_string(),
                            Value::String("Alice".to_string()),
                        ),
                        (alice, ":person/age".to_string(), Value::Integer(30)),
                    ],
                    None,
                )
                .unwrap();

            storage.mark_dirty();
            storage.save().unwrap();

            // Verify facts are persisted
            assert_eq!(storage.storage().fact_count(), 2);
        }

        // Note: In a real scenario, we'd reopen the same file.
        // MemoryBackend doesn't persist across instances, so this test
        // mainly validates the save/load mechanism.
    }

    #[test]
    fn test_persistent_fact_storage_auto_save() {
        let backend = MemoryBackend::new();

        let alice = Uuid::new_v4();

        // Create storage in a scope so it drops
        {
            let mut storage = PersistentFactStorage::new(backend, 256).unwrap();
            storage
                .storage()
                .transact(
                    vec![(
                        alice,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            storage.mark_dirty();
            // Drop happens here, should auto-save
        }

        // Load into new storage - backend is consumed, need to create a new test
        // This test verifies the pattern, actual persistence is tested above
    }

    #[test]
    fn test_load_preserves_original_tx_id() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 256).unwrap();

        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        let original_tx_id = pfs.storage().get_all_facts().unwrap()[0].tx_id;

        pfs.mark_dirty();
        pfs.save().unwrap();

        // Reload from the same backend
        let backend = pfs.into_backend().unwrap();
        let pfs2 = PersistentFactStorage::new(backend, 256).unwrap();
        let loaded_tx_id = pfs2.storage().get_all_facts().unwrap()[0].tx_id;

        assert_eq!(
            original_tx_id, loaded_tx_id,
            "tx_id must survive save/load round-trip"
        );
    }

    #[test]
    fn test_save_writes_v4_header() {
        use crate::storage::FORMAT_VERSION;

        let backend = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(backend, 256).unwrap();
        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(
                vec![(
                    alice,
                    ":name".to_string(),
                    crate::graph::types::Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();

        // Read back the header and verify version and last_checkpointed_tx_count
        let backend = pfs.into_backend().unwrap();
        let header_page = backend.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_page).unwrap();
        assert_eq!(header.version, FORMAT_VERSION);
        assert_eq!(header.last_checkpointed_tx_count, 1); // one transact call
    }

    #[test]
    fn test_last_checkpointed_tx_count_getter() {
        let backend = MemoryBackend::new();
        let pfs = PersistentFactStorage::new(backend, 256).unwrap();
        // Fresh database: no checkpoint yet
        assert_eq!(pfs.last_checkpointed_tx_count(), 0);
    }

    #[test]
    fn test_indexes_survive_save_load_roundtrip() {
        use crate::graph::types::Value;
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        use uuid::Uuid;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // Save phase
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            pfs.storage()
                .transact(
                    vec![
                        (
                            alice,
                            ":name".to_string(),
                            Value::String("Alice".to_string()),
                        ),
                        (alice, ":friend".to_string(), Value::Ref(bob)),
                    ],
                    None,
                )
                .unwrap();
            pfs.dirty = true;
            pfs.save().unwrap();
        }

        // Load phase — indexes must be accessible via on-disk B+tree
        {
            let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            // v6: indexes live on disk via CommittedIndexReader, not in pending RAM
            let alice_facts = pfs.storage().get_facts_by_entity(&alice).unwrap();
            assert_eq!(
                alice_facts.len(),
                2,
                "EAVT must resolve 2 entries after reload"
            );
            // Check that Ref-valued fact is accessible
            let ref_facts: Vec<_> = alice_facts
                .iter()
                .filter(|f| matches!(&f.value, crate::graph::types::Value::Ref(_)))
                .collect();
            assert_eq!(
                ref_facts.len(),
                1,
                "Ref fact must be accessible after reload"
            );
        }
    }

    #[test]
    fn test_sync_check_detects_mismatch_and_rebuilds() {
        use crate::graph::types::Value;
        use crate::storage::StorageBackend;
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        use uuid::Uuid;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();

        // Write a database with 1 fact
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            pfs.storage()
                .transact(
                    vec![(
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            pfs.dirty = true;
            pfs.save().unwrap();
        }

        // Corrupt the index_checksum (bytes 64..68 of page 0), then recompute header_checksum
        {
            let mut backend = FileBackend::open(&path).unwrap();
            let mut page = backend.read_page(0).unwrap();
            page[64] ^= 0xFF;
            let new_header_checksum = compute_header_checksum_from_bytes(&page);
            page[80] = (new_header_checksum & 0xFF) as u8;
            page[81] = ((new_header_checksum >> 8) & 0xFF) as u8;
            page[82] = ((new_header_checksum >> 16) & 0xFF) as u8;
            page[83] = ((new_header_checksum >> 24) & 0xFF) as u8;
            backend.write_page(0, &page).unwrap();
            backend.sync().unwrap();
        }

        // Re-open — new() should detect mismatch, rebuild, and succeed
        {
            let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            // v6: after rebuild, indexes are on disk; verify fact accessibility
            let alice_facts = pfs.storage().get_facts_by_entity(&alice).unwrap();
            assert_eq!(
                alice_facts.len(),
                1,
                "After rebuild, fact must be accessible via index"
            );
        }
    }

    /// Regression test for #370: the rebuild-on-load path must derive `FactRef`s from the
    /// on-disk fact-page layout, not by re-packing all facts as a single batch. Each
    /// `save()` starts a fresh page, so a file written by several checkpoints has partially
    /// filled pages that a contiguous re-pack would not reproduce.
    #[test]
    fn test_rebuild_after_multiple_checkpoints_keeps_entity_lookups() {
        use crate::graph::types::Value;
        use crate::storage::StorageBackend;
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        use uuid::Uuid;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let entities: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

        // One checkpoint per entity: five fact batches, each on its own partial page.
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            for (i, e) in entities.iter().enumerate() {
                pfs.storage()
                    .transact(
                        vec![(*e, ":name".to_string(), Value::String(format!("n{i}")))],
                        None,
                    )
                    .unwrap();
                pfs.dirty = true;
                pfs.save().unwrap();
            }
        }

        // Corrupt index_checksum (and re-seal the header) to force the rebuild path.
        {
            let mut backend = FileBackend::open(&path).unwrap();
            let mut page = backend.read_page(0).unwrap();
            page[64] ^= 0xFF;
            let cs = compute_header_checksum_from_bytes(&page);
            page[80..84].copy_from_slice(&cs.to_le_bytes());
            backend.write_page(0, &page).unwrap();
            backend.sync().unwrap();
        }

        let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
        for e in &entities {
            let facts = pfs.storage().get_facts_by_entity(e).unwrap();
            assert_eq!(
                facts.len(),
                1,
                "entity lookup must survive index rebuild after multiple checkpoints"
            );
            assert_eq!(facts[0].entity, *e, "lookup returned a different entity");
        }
    }

    #[test]
    fn test_compute_index_checksum_stable() {
        use crate::graph::types::{Fact, VALID_TIME_FOREVER, Value};
        use uuid::Uuid;

        let e = Uuid::new_v4();
        let facts = vec![
            Fact::with_valid_time(
                e,
                ":a".to_string(),
                Value::Integer(1),
                100,
                2,
                0,
                VALID_TIME_FOREVER,
            ),
            Fact::with_valid_time(
                e,
                ":b".to_string(),
                Value::Integer(2),
                200,
                1,
                0,
                VALID_TIME_FOREVER,
            ),
        ];
        let c1 = compute_index_checksum(&facts);
        // Reversed order — same checksum (deterministic sort applied inside)
        let facts_reversed = vec![facts[1].clone(), facts[0].clone()];
        let c2 = compute_index_checksum(&facts_reversed);
        assert_eq!(c1, c2, "Checksum must be order-independent");
    }

    #[test]
    fn test_save_writes_packed_pages() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            let mut tuples = Vec::new();
            for i in 0u64..50 {
                tuples.push((alice, format!(":attr{}", i), Value::Integer(i as i64)));
            }
            tuples.push((alice, ":friend".to_string(), Value::Ref(bob)));
            pfs.storage().transact(tuples, None).unwrap();
            pfs.mark_dirty();
            pfs.save().unwrap();
        }

        // Verify: header says v6, fact_page_format = PACKED
        {
            let backend = FileBackend::open(&path).unwrap();
            let header_bytes = backend.read_page(0).unwrap();
            let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
            assert_eq!(header.version, 7);
            assert_eq!(
                header.fact_page_format,
                crate::storage::FACT_PAGE_FORMAT_PACKED
            );
            // 51 facts @ ~25/page = ~3 pages (far fewer than 51)
            let fact_page_count = header.eavt_root_page.saturating_sub(1);
            assert!(
                fact_page_count <= 5,
                "got {} fact pages (expected <=5)",
                fact_page_count
            );
        }
    }

    #[test]
    fn test_save_v5_checksum_stored() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();

        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            pfs.storage()
                .transact(
                    vec![(
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            pfs.mark_dirty();
            pfs.save().unwrap();
        }

        {
            let backend = FileBackend::open(&path).unwrap();
            let header_bytes = backend.read_page(0).unwrap();
            let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
            // Checksum should be non-zero for a non-empty DB
            assert_ne!(header.index_checksum, 0, "checksum must be set");
        }
    }

    #[test]
    fn test_v5_load_fast_path_indexes_loaded() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();

        // Save in v5 format
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            pfs.storage()
                .transact(
                    vec![(
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            pfs.mark_dirty();
            pfs.save().unwrap();
        }

        // Reload — CommittedFactReader should be wired, fact accessible
        {
            let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            assert_eq!(pfs.storage().fact_count(), 1);
            // Query by entity should work via index
            let facts = pfs.storage().get_facts_by_entity(&alice).unwrap();
            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].entity, alice);
        }
    }

    // ── v6 on-disk B+tree tests ─────────────────────────────────────────────

    #[test]
    fn test_save_writes_v6_header() {
        let backend = MemoryBackend::new();
        let mut storage = PersistentFactStorage::new(backend, 256).unwrap();
        storage
            .storage()
            .transact(
                vec![(
                    Uuid::new_v4(),
                    ":name".to_string(),
                    Value::String("x".to_string()),
                )],
                None,
            )
            .unwrap();
        storage.mark_dirty();
        storage.save().unwrap();

        let backend = storage.into_backend().unwrap();
        let header_page = backend.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_page).unwrap();
        assert_eq!(header.version, 7, "save() must write v7 header");
        assert_eq!(header.to_bytes().len(), 84, "v7 header must be 84 bytes");
        assert!(header.fact_page_count > 0, "fact_page_count must be set");
        assert!(
            header.eavt_root_page > 0,
            "eavt_root must be set after save"
        );
    }

    #[test]
    fn test_load_v6_wires_committed_index_reader() {
        let alice = Uuid::new_v4();
        let backend = {
            let backend = MemoryBackend::new();
            let mut s = PersistentFactStorage::new(backend, 256).unwrap();
            s.storage()
                .transact(
                    vec![(
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            s.mark_dirty();
            s.save().unwrap();
            s.into_backend().unwrap()
        };

        let s2 = PersistentFactStorage::new(backend, 256).unwrap();
        let facts = s2.storage().get_facts_by_entity(&alice).unwrap();
        assert_eq!(
            facts.len(),
            1,
            "committed fact must be visible after reopen"
        );
    }

    #[test]
    fn test_save_twice_merges_committed_and_pending() {
        let backend = MemoryBackend::new();
        let mut storage = PersistentFactStorage::new(backend, 256).unwrap();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();

        // First checkpoint (e1 committed)
        storage
            .storage()
            .transact(
                vec![(e1, ":name".to_string(), Value::String("Alice".to_string()))],
                None,
            )
            .unwrap();
        storage.mark_dirty();
        storage.save().unwrap();

        // Second checkpoint (e2 pending → committed)
        storage
            .storage()
            .transact(
                vec![(e2, ":name".to_string(), Value::String("Bob".to_string()))],
                None,
            )
            .unwrap();
        storage.mark_dirty();
        storage.save().unwrap();

        let backend = storage.into_backend().unwrap();
        let s2 = PersistentFactStorage::new(backend, 256).unwrap();
        let e1_facts = s2.storage().get_facts_by_entity(&e1).unwrap();
        let e2_facts = s2.storage().get_facts_by_entity(&e2).unwrap();
        assert_eq!(
            e1_facts.len(),
            1,
            "e1 from first checkpoint must survive second checkpoint"
        );
        assert_eq!(
            e2_facts.len(),
            1,
            "e2 from second checkpoint must be visible"
        );
    }

    /// Regression test for fuzz-discovered timeout (artifact:
    /// `timeout-23b2b7e0aa43d92c12c49f123a48d6f4ace6ce33`).
    ///
    /// A crafted v5 header with page_count=3_604_123_350 and vaet_root_page=61
    /// caused migrate_v5_to_v6 to use page_count as the B-tree start page.
    /// build_btree then wrote a leaf at offset ~14 TB in a sparse file, and
    /// compute_page_checksum looped over 3.6 billion zero-filled sparse pages
    /// (each read_exact returns zeros, no error), hanging the process.
    ///
    /// Since v3.0.0 a v5 header is rejected before any page is touched.
    #[test]
    fn test_v5_migration_large_page_count_does_not_hang() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Write the exact fuzz-artifact header bytes (75 bytes + zero padding to 4096)
        let mut page = vec![0u8; PAGE_SIZE];
        page[0..4].copy_from_slice(b"MGRF");
        page[4..8].copy_from_slice(&5u32.to_le_bytes()); // version = 5
        page[8..16].copy_from_slice(&3_604_123_350u64.to_le_bytes()); // page_count (huge)
        page[56..64].copy_from_slice(&61u64.to_le_bytes()); // vaet_root_page = 61
        page[68] = 0x20; // fact_page_format
        std::fs::write(&path, &page).unwrap();

        // `FileBackend::open` itself reads and validates the header, so the
        // pre-v7 rejection surfaces there rather than in
        // `PersistentFactStorage::new`.
        let err = match FileBackend::open(&path) {
            Ok(_) => panic!("v5 header must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("no longer supported"),
            "must fail with STG-028"
        );
    }

    #[test]
    fn test_header_checksum_computation() {
        use crate::storage::FileHeader;

        let mut header = FileHeader::new();
        header.page_count = 10;
        header.node_count = 5;

        let checksum = compute_header_checksum(&header);
        assert_ne!(checksum, 0, "checksum must be non-zero");

        let mut header2 = FileHeader::new();
        header2.page_count = 10;
        header2.node_count = 5;
        assert_eq!(compute_header_checksum(&header2), checksum);

        let mut header3 = FileHeader::new();
        header3.page_count = 11;
        assert_ne!(compute_header_checksum(&header3), checksum);
    }

    #[test]
    fn test_header_checksum_corruption_detection() {
        use crate::storage::{FORMAT_VERSION, FileHeader};

        let mut header = FileHeader::new();
        header.version = FORMAT_VERSION;
        let valid_checksum = compute_header_checksum(&header);
        header.header_checksum = valid_checksum;

        header.page_count = 999;

        let computed = compute_header_checksum(&header);
        assert_ne!(computed, header.header_checksum);
    }

    #[test]
    fn test_save_with_valid_header_read() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        use uuid::Uuid;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();

        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            pfs.storage()
                .transact(
                    vec![(
                        alice,
                        ":name".to_string(),
                        Value::String("Alice".to_string()),
                    )],
                    None,
                )
                .unwrap();
            pfs.dirty = true;
            pfs.save().unwrap();
        }

        {
            let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            let facts = pfs.storage().get_facts_by_entity(&alice).unwrap();
            assert_eq!(facts.len(), 1, "should load facts from existing file");
        }
    }

    #[test]
    fn test_save_fails_on_corrupted_header() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().expect("valid path").to_string();
        drop(tmp);

        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            file.write_all(&vec![0u8; PAGE_SIZE]).unwrap();
            file.write_all(&vec![0u8; PAGE_SIZE]).unwrap();
        }

        let result = FileBackend::open(&path);
        assert!(
            result.is_err(),
            "should fail on corrupted header in existing file"
        );
    }

    #[test]
    fn test_is_new_returns_correct_value() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().expect("valid path").to_string();
        drop(tmp);

        let backend = FileBackend::open(&path).unwrap();
        assert!(backend.is_new(), "newly created file should be new");
        drop(backend);

        let backend = FileBackend::open(&path).unwrap();
        assert!(!backend.is_new(), "reopened file should not be new");
        drop(backend);
    }

    // ══ #359: STG-0xx regression tests ═══════════════════════════════════

    /// A poisoned backend mutex (a previous operation panicked while
    /// holding the lock) must surface as the coded STG-016, not a generic
    /// error.
    #[test]
    fn save_with_poisoned_backend_mutex_returns_stg_016() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 16).unwrap();
        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(
                vec![(
                    alice,
                    ":name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();
        pfs.mark_dirty();

        // Poison the backend mutex by panicking while holding it.
        let backend = pfs.backend.clone();
        let handle = std::thread::spawn(move || {
            let _guard = backend.lock().unwrap();
            panic!("deliberate poison for STG-016 regression test");
        });
        let _ = handle.join();

        let err = pfs
            .save()
            .expect_err("save on a poisoned backend mutex must fail");
        let coded: crate::error::MinigrafError = err.into();
        assert_eq!(coded.code(), "STG-016");
        assert_eq!(coded.category(), crate::error::ErrorCategory::Storage);
    }

    /// A backend that reports an existing (non-new) file but then fails to
    /// re-read the header at `save()` time (e.g. a transient disk I/O
    /// error) must surface as the coded STG-010, not a generic error.
    /// `read_page(0)` is allowed to succeed exactly once, so `load()` (run
    /// from `PersistentFactStorage::new()`, since `is_new()` is false)
    /// completes normally; the failure is injected only for the *second*
    /// `read_page(0)` call, which `save()` makes.
    struct FlakyHeaderBackend {
        inner: MemoryBackend,
        read_page_0_calls: std::sync::atomic::AtomicU32,
    }

    impl StorageBackend for FlakyHeaderBackend {
        fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
            self.inner.write_page(page_id, data)
        }
        fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
            if page_id == 0 {
                let calls = self
                    .read_page_0_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if calls >= 1 {
                    anyhow::bail!("simulated disk read failure");
                }
            }
            self.inner.read_page(page_id)
        }
        fn sync(&mut self) -> Result<()> {
            self.inner.sync()
        }
        fn page_count(&self) -> Result<u64> {
            self.inner.page_count()
        }
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }
        fn backend_name(&self) -> &'static str {
            "flaky-header-test"
        }
        fn is_new(&self) -> bool {
            false
        }
    }

    #[test]
    fn save_with_unreadable_existing_header_returns_stg_010() {
        let mut inner = MemoryBackend::new();
        // A valid, empty v7 header so `load()` (triggered by `is_new() ==
        // false`) succeeds cleanly on the first `read_page(0)` call.
        let mut header_page = FileHeader::new().to_bytes();
        header_page.resize(PAGE_SIZE, 0);
        inner.write_page(0, &header_page).unwrap();

        let backend = FlakyHeaderBackend {
            inner,
            read_page_0_calls: std::sync::atomic::AtomicU32::new(0),
        };
        let mut pfs = PersistentFactStorage::new(backend, 16).unwrap();
        pfs.mark_dirty();

        let err = pfs
            .save()
            .expect_err("save must fail when the header becomes unreadable");
        let coded: crate::error::MinigrafError = err.into();
        assert_eq!(coded.code(), "STG-010");
        assert_eq!(coded.category(), crate::error::ErrorCategory::Storage);
    }

    // ══ #214 fault-injection unit tests ══════════════════════════════════

    mod fault_injection_tests {
        use super::*;
        use crate::storage::backend::MemoryBackend;
        use crate::storage::backend::fault_inject::{FaultConfig, FaultInjectingBackend};

        fn make_pfs_with_config() -> (
            PersistentFactStorage<FaultInjectingBackend<MemoryBackend>>,
            std::sync::Arc<std::sync::Mutex<FaultConfig>>,
        ) {
            let (backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
            let pfs = PersistentFactStorage::new(backend, 16).unwrap();
            (pfs, config)
        }

        fn stage_fact(pfs: &mut PersistentFactStorage<FaultInjectingBackend<MemoryBackend>>) {
            let entity = Uuid::new_v4();
            pfs.storage()
                .transact(
                    vec![(entity, ":test/attr".to_string(), Value::Boolean(true))],
                    None,
                )
                .unwrap();
            pfs.mark_dirty();
        }

        #[test]
        fn save_returns_error_when_write_fails() {
            let (mut pfs, config) = make_pfs_with_config();
            stage_fact(&mut pfs);
            config.lock().unwrap().fail_write_after = Some(0);
            let result = pfs.save();
            assert!(
                result.is_err(),
                "save must return Err when write_page fails"
            );
        }

        #[test]
        fn save_returns_error_when_sync_fails() {
            let (mut pfs, config) = make_pfs_with_config();
            stage_fact(&mut pfs);
            config.lock().unwrap().fail_sync_after = Some(0);
            let result = pfs.save();
            assert!(result.is_err(), "save must return Err when sync fails");
        }

        #[test]
        fn save_error_message_is_non_empty() {
            let (mut pfs, config) = make_pfs_with_config();
            stage_fact(&mut pfs);
            config.lock().unwrap().fail_sync_after = Some(0);
            let err = pfs.save().unwrap_err();
            assert!(
                !err.to_string().is_empty(),
                "error message must not be empty"
            );
        }
    }
}
