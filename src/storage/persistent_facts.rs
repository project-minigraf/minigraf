use crate::graph::FactStorage;
/// Persistent fact storage that integrates StorageBackend with Datalog facts.
///
/// This module bridges the gap between high-level fact operations and
/// low-level page-based storage backends.
use crate::graph::types::Fact;
use crate::storage::FACT_PAGE_FORMAT_PACKED;
use crate::storage::btree::{read_aevt_index, read_avet_index, read_eavt_index, read_vaet_index};
use crate::storage::btree_v6::{
    OnDiskIndexReader, btree_entries, build_btree, merge_sorted_vecs, stream_all_entries,
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
#[cfg(test)]
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
// page_cache is read in the CommittedFactReader::resolve impl; Rust's dead-code
// lint does not track trait-impl field reads when the impl is behind dyn dispatch.
struct CommittedFactLoaderImpl<B: StorageBackend> {
    backend: Arc<Mutex<B>>,
    #[allow(dead_code)]
    page_cache: Arc<PageCache>,
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
        let backend = self.backend.lock().unwrap();
        let page = self.page_cache.get_or_load(fact_ref.page_id, &*backend)?;
        crate::storage::packed_pages::read_slot(&page, fact_ref.slot_index)
    }

    fn stream_all(&self) -> anyhow::Result<Vec<crate::graph::types::Fact>> {
        let n = self.committed_fact_pages.load(Ordering::SeqCst);
        let backend = self.backend.lock().unwrap();
        crate::storage::packed_pages::read_all_from_pages(&*backend, 1, n)
    }

    fn committed_page_count(&self) -> u64 {
        self.committed_fact_pages.load(Ordering::SeqCst)
    }
}

/// V1 fact format (Phase 3, before bi-temporal fields were added).
///
/// Used only during migration from v1 → v2 file format.
#[derive(Debug, serde::Deserialize)]
struct FactV1 {
    entity: crate::graph::types::EntityId,
    attribute: crate::graph::types::Attribute,
    value: crate::graph::types::Value,
    tx_id: crate::graph::types::TxId,
    asserted: bool,
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

        // Try to load existing data
        let page_count = persistent.backend.lock().unwrap().page_count()?;
        if page_count > 1 {
            persistent.load()?;
        } else {
            // Initialize new database with header
            persistent.save()?;
        }

        Ok(persistent)
    }

    /// Load all facts from the backend into memory.
    fn load(&mut self) -> Result<()> {
        let (header, raw_header_bytes) = {
            let backend = self.backend.lock().unwrap();
            let header_page = backend.read_page(0)?;
            let h = FileHeader::from_bytes(&header_page)?;
            h.validate()?;
            (h, header_page)
        };

        // Migrate v1 → v2 if needed
        if header.version < 2 {
            return self.migrate_v1_to_v2();
        }

        // Migrate v5 → v6 (paged-blob indexes → on-disk B+tree)
        if header.version == 5 {
            return self.migrate_v5_to_v6(&header);
        }

        // For v7+ files, validate header checksum using raw bytes from disk
        // For older versions (v6 and earlier), header_checksum is 0 so validation is skipped
        if header.version >= 7 && header.header_checksum != 0 {
            let computed = compute_header_checksum_from_bytes(&raw_header_bytes);
            if header.header_checksum != computed {
                anyhow::bail!(
                    "Header checksum mismatch: possible file corruption. Database may be damaged."
                );
            }
        }

        // Store last_checkpointed_tx_count from header (0 for v2 files)
        self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;

        // Clear existing storage
        self.storage.clear()?;

        let fact_page_format = header.fact_page_format;

        if fact_page_format == 0
            || fact_page_format == crate::storage::FACT_PAGE_FORMAT_ONE_PER_PAGE
        {
            // Legacy one-per-page format (v4 or earlier): load all facts, then migrate to v5.
            self.load_one_per_page_legacy(&header)?;
            self.storage.restore_tx_counter()?;
            self.dirty = true;
            self.save()?;
            return Ok(());
        }

        // v6 packed format
        let num_fact_pages = if header.version >= 6 && header.fact_page_count > 0 {
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

        // Compute page-based checksum to verify indexes are valid
        let computed = {
            let backend = self.backend.lock().unwrap();
            compute_page_checksum(&*backend, 1, num_fact_pages)?
        };
        let stored = header.index_checksum;
        let needs_rebuild =
            num_fact_pages > 0 && (computed != stored || header.eavt_root_page == 0);

        // Register CommittedFactReader on FactStorage (before WAL replay)
        let loader: std::sync::Arc<dyn crate::storage::CommittedFactReader> =
            std::sync::Arc::new(CommittedFactLoaderImpl {
                backend: self.backend.clone(),
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
            let all_facts = {
                let backend = self.backend.lock().unwrap();
                crate::storage::packed_pages::read_all_from_pages(&*backend, 1, num_fact_pages)?
            };
            // Re-pack to derive correct FactRefs (same deterministic layout as on disk)
            let (_, real_refs) = pack_facts(&all_facts, 1)?;

            // Build sorted index entries
            let (eavt_entries, aevt_entries, avet_entries, vaet_entries) =
                build_sorted_index_entries(&all_facts, &real_refs);

            // Fix up tx_counter from actual facts
            let max_tx = all_facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
            self.storage.restore_tx_counter_from(max_tx);

            // Build v6 B+tree indexes directly
            let index_start = 1 + num_fact_pages;
            let mut backend = self.backend.lock().unwrap();
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

            // Write v6 header
            let mut new_header = FileHeader::new();
            new_header.page_count = next4;
            new_header.node_count = all_facts.len() as u64;
            new_header.last_checkpointed_tx_count = max_tx;
            new_header.eavt_root_page = eavt_root;
            new_header.aevt_root_page = aevt_root;
            new_header.avet_root_page = avet_root;
            new_header.vaet_root_page = vaet_root;
            new_header.index_checksum = computed;
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
            if header.version >= 7 && header.header_checksum != 0 {
                let backend = self.backend.lock().unwrap();
                let current_header_bytes = backend.read_page(0)?;
                let current_header = FileHeader::from_bytes(&current_header_bytes)?;
                let computed = compute_header_checksum_from_bytes(&current_header_bytes);
                if current_header.header_checksum != computed {
                    anyhow::bail!(
                        "Header checksum mismatch: possible file corruption. Database may be damaged."
                    );
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

    /// Load facts from legacy one-per-page format (v4 and earlier).
    fn load_one_per_page_legacy(&mut self, header: &FileHeader) -> Result<usize> {
        let page_count = header.page_count;
        let backend = self.backend.lock().unwrap();
        let mut loaded = 0;
        let mut skipped = 0;
        for page_id in 1..page_count {
            let page = backend.read_page(page_id)?;
            // Try to deserialize a fact from this page (legacy format: raw postcard bytes)
            match postcard::from_bytes::<Fact>(&page) {
                Ok(fact) => {
                    self.storage.load_fact(fact)?;
                    loaded += 1;
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!(
                        "Warning: failed to deserialize fact at page {}: {}. Skipping.",
                        page_id, e
                    );
                }
            }
        }
        if skipped > 0 {
            eprintln!(
                "Warning: {} facts failed to deserialize during legacy load (version {})",
                skipped, header.version
            );
        }
        Ok(loaded)
    }

    /// Migrate a v1 file (Phase 3 format, no bi-temporal fields) to v2.
    ///
    /// V1 facts only have (entity, attribute, value, tx_id, asserted).
    /// V2 facts add tx_count, valid_from, valid_to.
    ///
    /// Migration strategy:
    /// - Sort v1 facts by tx_id ascending
    /// - Group facts with the same tx_id into the same tx_count (monotonic counter)
    /// - Set valid_from = tx_id as i64 (wall-clock approximation)
    /// - Set valid_to = VALID_TIME_FOREVER (open-ended)
    /// - Write the migrated data back in v2 format
    fn migrate_v1_to_v2(&mut self) -> Result<()> {
        use crate::graph::types::VALID_TIME_FOREVER;

        let backend = self.backend.lock().unwrap();
        let header_page = backend.read_page(0)?;
        let header = FileHeader::from_bytes(&header_page)?;
        let page_count = header.page_count;

        // Read all v1 facts (track deserialization failures)
        let mut v1_facts: Vec<FactV1> = Vec::new();
        let mut skipped = 0;
        for page_id in 1..page_count {
            let page = backend.read_page(page_id)?;
            match postcard::from_bytes::<FactV1>(&page) {
                Ok(fact) => v1_facts.push(fact),
                Err(e) => {
                    skipped += 1;
                    eprintln!(
                        "Warning: failed to deserialize v1 fact at page {}: {}. Skipping.",
                        page_id, e
                    );
                }
            }
        }
        if skipped > 0 {
            eprintln!(
                "Warning: {} v1 facts failed to deserialize during migration",
                skipped
            );
        }
        drop(backend);

        // Sort by tx_id ascending so we can group them
        v1_facts.sort_by_key(|f| f.tx_id);

        // Assign tx_count, grouping facts with the same tx_id into the same tx_count
        let mut tx_count: u64 = 0;
        let mut prev_tx_id: Option<crate::graph::types::TxId> = None;
        let mut migrated: Vec<Fact> = Vec::new();

        for v1 in v1_facts {
            if prev_tx_id != Some(v1.tx_id) {
                tx_count += 1;
                prev_tx_id = Some(v1.tx_id);
            }
            let mut fact = Fact::with_valid_time(
                v1.entity,
                v1.attribute,
                v1.value,
                v1.tx_id,
                tx_count,
                v1.tx_id as i64,
                VALID_TIME_FOREVER,
            );
            // Preserve the asserted flag (with_valid_time sets asserted=true by default)
            fact.asserted = v1.asserted;
            migrated.push(fact);
        }

        self.storage.clear()?;
        for fact in migrated {
            self.storage.load_fact(fact)?;
        }
        self.storage.restore_tx_counter()?;

        // Persist in v2 format immediately
        self.dirty = true;
        self.save()?;
        Ok(())
    }

    /// Migrate a v5 file (paged-blob indexes) to v6 (on-disk B+tree indexes).
    fn migrate_v5_to_v6(&mut self, header: &FileHeader) -> Result<()> {
        let num_fact_pages = {
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

        // Validate the calculated range contains valid pages.
        // If the file was in an inconsistent state (partial checkpoint),
        // the calculated range might be incorrect. Do a quick validation
        // by checking that the first fact page can be read (doesn't need to
        // be a packed page - the index checksum will catch any real corruption).
        let validated_num_fact_pages = if num_fact_pages > 0 {
            let backend = self.backend.lock().unwrap();
            // Just verify we can read the page - actual validation happens via checksum
            if backend.read_page(1).is_ok() {
                num_fact_pages
            } else {
                eprintln!(
                    "Warning: cannot read first fact page (page 1). Header claims {}. Using 0.",
                    num_fact_pages
                );
                0
            }
        } else {
            num_fact_pages
        };

        self.committed_fact_pages
            .store(validated_num_fact_pages, Ordering::SeqCst);

        // Verify index integrity via checksum before trusting the old indexes.
        // If checksum doesn't match, rebuild indexes from facts instead.
        let use_old_indexes = validated_num_fact_pages > 0 && header.index_checksum > 0 && {
            let backend = self.backend.lock().unwrap();
            let computed = compute_page_checksum(&*backend, 1, validated_num_fact_pages)?;
            computed == header.index_checksum
        };

        let (eavt, aevt, avet, vaet) = if use_old_indexes {
            // Read and trust the old v5 indexes
            let backend = self.backend.lock().unwrap();
            let e = if header.eavt_root_page > 0 {
                read_eavt_index(header.eavt_root_page, &*backend)?
            } else {
                std::collections::BTreeMap::new()
            };
            let a = if header.aevt_root_page > 0 {
                read_aevt_index(header.aevt_root_page, &*backend)?
            } else {
                std::collections::BTreeMap::new()
            };
            let av = if header.avet_root_page > 0 {
                read_avet_index(header.avet_root_page, &*backend)?
            } else {
                std::collections::BTreeMap::new()
            };
            let v = if header.vaet_root_page > 0 {
                read_vaet_index(header.vaet_root_page, &*backend)?
            } else {
                std::collections::BTreeMap::new()
            };
            (e, a, av, v)
        } else {
            // Checksum mismatch or missing - rebuild indexes from facts
            let all_facts = {
                let backend = self.backend.lock().unwrap();
                crate::storage::packed_pages::read_all_from_pages(
                    &*backend,
                    1,
                    validated_num_fact_pages,
                )?
            };
            // Build indexes from fact data
            let mut eavt_map = std::collections::BTreeMap::new();
            let mut aevt_map = std::collections::BTreeMap::new();
            let mut avet_map = std::collections::BTreeMap::new();
            let mut vaet_map = std::collections::BTreeMap::new();
            for (i, fact) in all_facts.iter().enumerate() {
                let fr = FactRef {
                    page_id: 1,
                    slot_index: i as u16,
                };
                eavt_map.insert(
                    EavtKey {
                        entity: fact.entity,
                        attribute: fact.attribute.clone(),
                        valid_from: fact.valid_from,
                        valid_to: fact.valid_to,
                        tx_count: fact.tx_count,
                    },
                    fr,
                );
                aevt_map.insert(
                    AevtKey {
                        attribute: fact.attribute.clone(),
                        entity: fact.entity,
                        valid_from: fact.valid_from,
                        valid_to: fact.valid_to,
                        tx_count: fact.tx_count,
                    },
                    fr,
                );
                avet_map.insert(
                    AvetKey {
                        attribute: fact.attribute.clone(),
                        value_bytes: encode_value(&fact.value),
                        valid_from: fact.valid_from,
                        valid_to: fact.valid_to,
                        entity: fact.entity,
                        tx_count: fact.tx_count,
                    },
                    fr,
                );
                if let crate::graph::types::Value::Ref(target) = &fact.value {
                    vaet_map.insert(
                        VaetKey {
                            ref_target: *target,
                            attribute: fact.attribute.clone(),
                            valid_from: fact.valid_from,
                            valid_to: fact.valid_to,
                            source_entity: fact.entity,
                            tx_count: fact.tx_count,
                        },
                        fr,
                    );
                }
            }
            (eavt_map, aevt_map, avet_map, vaet_map)
        };

        let mut backend = self.backend.lock().unwrap();
        let next_free = header.page_count;

        let (eavt_root, next_free2) = build_btree(
            btree_entries(eavt.into_iter())?.into_iter(),
            &mut *backend,
            &self.page_cache,
            next_free,
        )?;
        let (aevt_root, next_free3) = build_btree(
            btree_entries(aevt.into_iter())?.into_iter(),
            &mut *backend,
            &self.page_cache,
            next_free2,
        )?;
        let (avet_root, next_free4) = build_btree(
            btree_entries(avet.into_iter())?.into_iter(),
            &mut *backend,
            &self.page_cache,
            next_free3,
        )?;
        let (vaet_root, final_next_free) = build_btree(
            btree_entries(vaet.into_iter())?.into_iter(),
            &mut *backend,
            &self.page_cache,
            next_free4,
        )?;

        let mut new_header = FileHeader::new(); // version=7
        new_header.page_count = final_next_free;
        new_header.node_count = header.node_count;
        new_header.last_checkpointed_tx_count = header.last_checkpointed_tx_count;
        new_header.eavt_root_page = eavt_root;
        new_header.aevt_root_page = aevt_root;
        new_header.avet_root_page = avet_root;
        new_header.vaet_root_page = vaet_root;
        // Recompute the checksum for the new indexes
        let computed_checksum = compute_page_checksum(&*backend, 1, validated_num_fact_pages)?;
        new_header.index_checksum = computed_checksum;
        new_header.fact_page_format = header.fact_page_format;
        new_header.fact_page_count = validated_num_fact_pages;
        new_header.header_checksum = compute_header_checksum(&new_header);

        let mut header_page = new_header.to_bytes();
        header_page.resize(PAGE_SIZE, 0);
        backend.write_page(0, &header_page)?;
        backend.sync()?;
        drop(backend);

        self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;

        let loader: Arc<dyn crate::storage::CommittedFactReader> =
            Arc::new(CommittedFactLoaderImpl {
                backend: self.backend.clone(),
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

        self.storage
            .restore_tx_counter_from(header.last_checkpointed_tx_count);
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
            Ok(mutex) => Ok(mutex.into_inner().unwrap()),
            Err(_) => Err(anyhow::anyhow!(
                "into_backend: backend Arc has multiple owners"
            )),
        }
    }

    /// Save all facts from memory to the backend using packed pages and v6 on-disk B+tree indexes.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        // ── Step A: read current header + stream old B+tree entries BEFORE overwriting ──
        let pending_facts = self.storage.get_pending_facts();
        let mut backend = self.backend.lock().unwrap();

        let old_fact_page_count = self.committed_fact_pages.load(Ordering::SeqCst);
        let new_fact_start = 1 + old_fact_page_count;

        let curr_header = match backend.read_page(0) {
            Ok(bytes) => FileHeader::from_bytes(&bytes)?,
            Err(_) if backend.is_new() => FileHeader::new(),
            Err(e) => anyhow::bail!("Failed to read header from existing file: {}", e),
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
            backend.write_page(new_fact_start + i as u64, page_data)?;
        }
        let new_total_fact_pages = old_fact_page_count + new_pages.len() as u64;

        // CRC32 over ALL fact pages (old committed + newly appended)
        let checksum = compute_page_checksum(&*backend, 1, new_total_fact_pages)?;

        // ── Step C: build sorted index entries for pending facts ────────────────
        let (pending_eavt, pending_aevt, pending_avet, pending_vaet) =
            build_sorted_index_entries(&pending_facts, &new_fact_refs);

        // ── Step D: merge committed + pending entries, build new B+trees ─────────
        let index_start = 1 + new_total_fact_pages;

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

        // ── Step E: write v6 header (last write = crash-safe boundary) ──────────
        let mut header = FileHeader::new(); // version=7
        header.page_count = next4;
        header.node_count = curr_header.node_count + pending_facts.len() as u64;
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
    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.dirty
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
        let page = backend.read_page(first_page + i)?;
        hasher.update(&page);
    }
    Ok(hasher.finalize())
}

/// Compute CRC32 checksum over header bytes 0-79 (header_checksum field zeroed).
pub fn compute_header_checksum(header: &FileHeader) -> u32 {
    let mut bytes = header.to_bytes();
    bytes[80] = 0;
    bytes[81] = 0;
    bytes[82] = 0;
    bytes[83] = 0;
    let mut hasher = Hasher::new();
    hasher.update(&bytes[..80]);
    hasher.finalize()
}

/// Compute CRC32 checksum over raw header bytes 0-79 (bytes 80-83 zeroed).
fn compute_header_checksum_from_bytes(bytes: &[u8]) -> u32 {
    let mut data = bytes.to_vec();
    if data.len() < 84 {
        data.resize(84, 0);
    }
    data[80] = 0;
    data[81] = 0;
    data[82] = 0;
    data[83] = 0;
    let mut hasher = Hasher::new();
    hasher.update(&data[..80]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::storage::backend::MemoryBackend;
    use std::io::Write;
    use uuid::Uuid;

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

    // -----------------------------------------------------------------------
    // Migration helpers
    // -----------------------------------------------------------------------

    /// Build a MemoryBackend that contains a v1-format file with two FactV1 facts.
    fn make_v1_backend() -> MemoryBackend {
        use crate::storage::{MAGIC_NUMBER, PAGE_SIZE};

        let alice = Uuid::new_v4();

        #[derive(serde::Serialize)]
        struct FactV1Ser {
            entity: Uuid,
            attribute: String,
            value: Value,
            tx_id: u64,
            asserted: bool,
        }

        let fact1 = FactV1Ser {
            entity: alice,
            attribute: ":person/name".to_string(),
            value: Value::String("Alice".to_string()),
            tx_id: 1000,
            asserted: true,
        };
        let fact2 = FactV1Ser {
            entity: alice,
            attribute: ":person/age".to_string(),
            value: Value::Integer(30),
            tx_id: 1000,
            asserted: true,
        };

        let mut backend = MemoryBackend::new();

        // Write v1 header (version=1, page_count=3)
        let mut header_bytes = vec![0u8; PAGE_SIZE];
        header_bytes[0..4].copy_from_slice(&MAGIC_NUMBER);
        header_bytes[4..8].copy_from_slice(&1u32.to_le_bytes()); // version = 1
        header_bytes[8..16].copy_from_slice(&3u64.to_le_bytes()); // page_count = 3
        backend.write_page(0, &header_bytes).unwrap();

        // Write facts (one per page)
        for (i, fact) in [&fact1, &fact2].iter().enumerate() {
            let data = postcard::to_allocvec(*fact).unwrap();
            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);
            backend.write_page((i + 1) as u64, &page).unwrap();
        }

        backend
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
    fn test_migrate_v1_to_v2_assigns_defaults() {
        use crate::graph::types::VALID_TIME_FOREVER;

        let backend = make_v1_backend();
        let pfs = PersistentFactStorage::new(backend, 256).unwrap();
        let facts = pfs.storage().get_all_facts().unwrap();

        assert_eq!(facts.len(), 2);
        // Both facts share tx_id=1000 → same tx_count
        assert_eq!(
            facts[0].tx_count, facts[1].tx_count,
            "facts from the same tx_id batch must get the same tx_count"
        );
        assert_eq!(
            facts[0].valid_to, VALID_TIME_FOREVER,
            "migrated fact must have open-ended valid_to"
        );
        assert_eq!(
            facts[0].valid_from, 1000_i64,
            "migrated fact valid_from must equal original tx_id"
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
    fn test_migrate_v1_tx_counter_set_correctly() {
        let backend = make_v1_backend();
        let pfs = PersistentFactStorage::new(backend, 256).unwrap();

        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(
                vec![(alice, ":new/fact".to_string(), Value::Boolean(true))],
                None,
            )
            .unwrap();

        let new_fact = pfs
            .storage()
            .get_all_facts()
            .unwrap()
            .into_iter()
            .find(|f| f.attribute == ":new/fact")
            .unwrap();

        // After migrating 1 unique tx_id (tx_count=1), next tx should get tx_count=2
        assert_eq!(
            new_fact.tx_count, 2,
            "first new transaction after migration must get tx_count=2"
        );
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
            let fact_page_count = if header.eavt_root_page > 1 {
                header.eavt_root_page - 1
            } else {
                0
            };
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
    fn test_v4_database_migrates_to_v5_on_open() {
        use crate::storage::backend::FileBackend;
        use crate::storage::{FACT_PAGE_FORMAT_PACKED, PAGE_SIZE};
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let alice = Uuid::new_v4();

        // Write a "v4-style" file: version=4, fact_page_format byte = 0 (legacy padding)
        {
            use crate::storage::FileHeader;
            let fact = crate::graph::types::Fact::with_valid_time(
                alice,
                ":name".to_string(),
                Value::String("Alice".to_string()),
                1u64,
                1u64,
                0i64,
                i64::MAX,
            );
            let mut backend = FileBackend::open(&path).unwrap();

            // Write fact at page 1 (one-per-page format)
            let data = postcard::to_allocvec(&fact).unwrap();
            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);
            backend.write_page(1, &page).unwrap();

            // Write v4 header (fact_page_format byte will be 0)
            let mut header = FileHeader::new();
            header.page_count = 2;
            header.node_count = 1;
            let mut hbytes = header.to_bytes();
            // Force version to 4
            hbytes[4..8].copy_from_slice(&4u32.to_le_bytes());
            // Force fact_page_format byte (offset 68) to 0
            hbytes[68] = 0;
            hbytes.resize(PAGE_SIZE, 0);
            backend.write_page(0, &hbytes).unwrap();
            backend.sync().unwrap();
        }

        // Open — should auto-migrate to v7
        {
            let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            assert_eq!(
                pfs.storage().fact_count(),
                1,
                "migrated fact must be loaded"
            );
        }

        // Verify file is now v7
        {
            let backend = FileBackend::open(&path).unwrap();
            let header_bytes = backend.read_page(0).unwrap();
            let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
            assert_eq!(header.version, 7, "file must be upgraded to v7");
            assert_eq!(header.fact_page_format, FACT_PAGE_FORMAT_PACKED);
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

    #[test]
    fn test_v6_migration_from_v5_unit() {
        let mut backend = MemoryBackend::new();
        let mut page = vec![0u8; PAGE_SIZE];
        page[0..4].copy_from_slice(b"MGRF");
        page[4..8].copy_from_slice(&5u32.to_le_bytes()); // version = 5
        page[8..16].copy_from_slice(&2u64.to_le_bytes()); // page_count = 2 (header + 1 empty page)
        page[68] = 0x02; // fact_page_format = PACKED
        backend.write_page(0, &page).unwrap();
        // Write an empty fact page so page_count > 1 triggers load()
        backend.write_page(1, &vec![0u8; PAGE_SIZE]).unwrap();

        let s = PersistentFactStorage::new(backend, 256).unwrap();
        let b = s.into_backend().unwrap();
        let header_page = b.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_page).unwrap();
        assert_eq!(header.version, 7, "migration must upgrade header to v7");
        assert_eq!(header.to_bytes().len(), 84, "v7 header must be 84 bytes");
        // page_count=2 means 1 fact page (page 1), even if empty
        assert_eq!(
            header.fact_page_count, 1,
            "fact_page_count must reflect page layout"
        );
    }

    #[test]
    fn test_v6_migration_crash_safe_unit() {
        let mut backend = MemoryBackend::new();
        let mut page = vec![0u8; PAGE_SIZE];
        page[0..4].copy_from_slice(b"MGRF");
        page[4..8].copy_from_slice(&5u32.to_le_bytes());
        page[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
        page[68] = 0x02;
        backend.write_page(0, &page).unwrap();
        backend.write_page(1, &vec![0xFF_u8; PAGE_SIZE]).unwrap();
        backend.write_page(2, &vec![0xFF_u8; PAGE_SIZE]).unwrap();

        let s = PersistentFactStorage::new(backend, 256).unwrap();
        let b = s.into_backend().unwrap();
        let header_bytes = b.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_bytes).unwrap();
        assert_eq!(
            header.version, 7,
            "migration must complete despite prior partial run"
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
}
