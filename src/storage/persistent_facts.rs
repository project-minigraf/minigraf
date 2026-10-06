//! Persistent fact storage: bridges [`FactStorage`] and a page-based
//! [`StorageBackend`] in format v8.
//!
//! Layout (spec §4): pages 0 and 1 are alternating meta pages, the only commit
//! point. Every other page carries the common header (type, CRC, page id,
//! generation) and is verified on every read. A checkpoint writes only pages the
//! active meta does not reference, syncs, then writes the other meta slot and
//! syncs again, so a crash at any point leaves the previous checkpoint intact.
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::FactStorage;
use crate::graph::types::Fact;
use crate::storage::btree::{build_btree, cow_insert};
use crate::storage::cache::PageCache;
use crate::storage::dict::{DictReader, Encoded, Encoder};
use crate::storage::meta::{MetaPage, SlotState, slot_page};
use crate::storage::page::PageAllocator;
use crate::storage::reader::OnDiskReader;
use crate::storage::{LegacyHeaderV7, PAGE_SIZE, StorageBackend, freelist, page};
use anyhow::Result;
use std::sync::{Arc, Mutex};

/// Persistent fact storage with page-based persistence.
///
/// Committed facts live on disk and are read on demand through the on-disk
/// indexes and the page cache; facts written since the last checkpoint live in
/// memory (and in the WAL) until `save()` commits them.
pub struct PersistentFactStorage<B: StorageBackend + 'static> {
    backend: Arc<Mutex<B>>,
    page_cache: Arc<PageCache>,
    storage: FactStorage,
    dirty: bool,
    /// The active (last committed) meta page.
    meta: MetaPage,
}

/// What open found in the meta slots.
enum Opened {
    Meta(MetaPage),
    LegacyV7(LegacyHeaderV7),
    Fresh,
}

impl<B: StorageBackend + 'static> PersistentFactStorage<B> {
    /// Open storage on `backend` with no WAL beside it (in-memory and browser
    /// databases). See [`PersistentFactStorage::open`].
    pub fn new(backend: B, page_cache_capacity: usize) -> Result<Self> {
        Self::open(backend, page_cache_capacity, None)
    }

    /// Open storage on `backend`, creating an empty database if it has no pages.
    ///
    /// `wal_base_generation` is the base generation recorded in the WAL beside
    /// the file, or `None` if there is no WAL. It decides meta selection when
    /// only one meta slot is valid (spec §4.1.1).
    ///
    /// `page_cache_capacity` controls the LRU page cache size (in pages).
    /// A value of 256 means at most 256 x 4KB = 1MB of cached pages.
    pub fn open(
        backend: B,
        page_cache_capacity: usize,
        wal_base_generation: Option<u64>,
    ) -> Result<Self> {
        let mut pfs = PersistentFactStorage {
            backend: Arc::new(Mutex::new(backend)),
            page_cache: Arc::new(PageCache::new(page_cache_capacity)),
            storage: FactStorage::new(),
            dirty: false,
            meta: MetaPage::empty(1),
        };
        let meta = match pfs.select_meta(wal_base_generation)? {
            Opened::Fresh => pfs.init_empty()?,
            Opened::Meta(m) => {
                m.check_features()?;
                m.check_layout()?;
                m
            }
            Opened::LegacyV7(h) => pfs.migrate_v7(&h)?,
        };
        pfs.activate(meta);
        pfs.storage
            .restore_tx_counter_from(meta.last_checkpointed_tx_count);
        Ok(pfs)
    }

    /// The LRU page cache capacity this storage was constructed with (for testing).
    #[allow(dead_code)]
    pub(crate) fn page_cache_capacity(&self) -> usize {
        self.page_cache.capacity()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, B>> {
        self.backend
            .lock()
            .map_err(|_| err_coded!(ErrorCode::Stg016))
    }

    /// Choose the meta page to open at (spec §4.1.1). Never modifies the file,
    /// except to restore page 0 from the migration backup meta (§9 step 5).
    fn select_meta(&self, wal_base: Option<u64>) -> Result<Opened> {
        let mut backend = self.lock()?;
        let page_count = backend.page_count()?;
        if page_count == 0 {
            return Ok(Opened::Fresh);
        }
        let page0 = backend.read_page(0)?;
        let slot_a = MetaPage::decode(&page0);
        let slot_b = if page_count > 1 {
            MetaPage::decode(&backend.read_page(1)?)
        } else {
            SlotState::Empty
        };
        match (slot_a, slot_b) {
            (SlotState::Valid(a), SlotState::Valid(b)) => {
                Ok(Opened::Meta(if a.generation >= b.generation {
                    a
                } else {
                    b
                }))
            }
            (SlotState::Valid(m), _) | (_, SlotState::Valid(m)) => {
                // Refuse an unreadable file before probing its pages.
                m.check_features()?;
                check_single_valid(&*backend, m, wal_base)?;
                Ok(Opened::Meta(m))
            }
            _ => {
                if let Some(h) = LegacyHeaderV7::detect(&page0)? {
                    return Ok(Opened::LegacyV7(h));
                }
                // A torn page-0 write while committing a migration: the backup
                // meta in the last page carries generation 1 (spec §9 step 5).
                let last = backend.read_page(page_count.saturating_sub(1))?;
                if let SlotState::Valid(m) = MetaPage::decode(&last)
                    && m.generation == 1
                    && m.page_count == page_count
                {
                    m.check_features()?;
                    backend.write_page(0, &m.encode())?;
                    backend.sync()?;
                    return Ok(Opened::Meta(m));
                }
                if page_count <= 2 && is_torn_initial_meta(&*backend, &page0)? {
                    return Ok(Opened::Fresh);
                }
                if page0.get(0..4) != Some(&crate::storage::MAGIC_NUMBER[..]) {
                    // Not a Minigraf file at all (or page 0 overwritten).
                    bail_coded!(ErrorCode::Stg002)
                }
                bail_coded!(ErrorCode::Stg032)
            }
        }
    }

    /// Write an empty generation-1 meta to page 0 and a blank page 1, and sync.
    fn init_empty(&self) -> Result<MetaPage> {
        let meta = MetaPage::empty(1);
        let mut backend = self.lock()?;
        backend.write_page(0, &meta.encode())?;
        backend.write_page(1, &vec![0u8; PAGE_SIZE])?;
        backend.sync()?;
        Ok(meta)
    }

    /// Make `meta` the active state: cache bound and committed reader.
    fn activate(&mut self, meta: MetaPage) {
        self.meta = meta;
        self.page_cache.set_generation_bound(meta.generation);
        let reader: Arc<dyn crate::storage::CommittedReader> = Arc::new(OnDiskReader::new(
            self.backend.clone(),
            self.page_cache.clone(),
            &meta,
        ));
        self.storage.set_committed_reader(reader);
    }

    /// Migrate a format v7 file to v8 (spec §9).
    ///
    /// Everything is appended past the v7 `page_count`; the v7 pages from 2 up
    /// become the new free list. A backup of the generation-1 meta is the last
    /// page, so a torn page-0 commit write can be repaired on the next open. A
    /// crash before the page-0 write leaves the v7 header intact, and the next
    /// open migrates again.
    fn migrate_v7(&mut self, header: &LegacyHeaderV7) -> Result<MetaPage> {
        let mut backend = self.lock()?;
        let num_fact_pages = header.fact_page_count_or_derived();
        let mut facts = crate::storage::packed_pages::read_all_v7(&*backend, 1, num_fact_pages)?;

        // Empty transactions allocate a tx_count without producing facts, so the
        // highest fact tx_count can undercount; never rewind past the header's
        // counter (#371, #287).
        let max_fact_tx = facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        let max_tx = max_fact_tx.max(header.last_checkpointed_tx_count);

        // Ids are assigned in transaction order (spec §9 step 2).
        facts.sort_by_key(|f| f.tx_count);
        let old_page_count = header.page_count.max(2);
        let mut alloc = PageAllocator::new(Vec::new(), old_page_count, 1);
        let encoded = encode(
            &facts,
            &MetaPage::empty(1),
            &mut alloc,
            &mut *backend,
            &self.page_cache,
        )?;
        let fact_count =
            u64::try_from(encoded.index[0].len()).map_err(|_| err_coded!(ErrorCode::Stg024))?;
        let (next_eid, next_iid) = (encoded.next_eid, encoded.next_iid);
        let [eavt_root, aevt_root, avet_root, vaet_root, dict_root] = {
            let mut roots = [0u64; 5];
            let Encoded { index, dict, .. } = encoded;
            for (root, entries) in roots.iter_mut().zip(index.into_iter().chain([dict])) {
                *root = build_btree(entries, &mut *backend, &self.page_cache, &mut alloc)?;
            }
            roots
        };

        // Free list: old pages 2.. (page 1 is meta slot B) plus the backup page,
        // which is appended right after the chain's own pages.
        let mut free: Vec<u64> = (2..old_page_count).collect();
        let chain_len = u64::try_from(
            free.len()
                .saturating_add(1)
                .div_ceil(freelist::IDS_PER_PAGE),
        )
        .map_err(|_| err_coded!(ErrorCode::Int048, "free-list length"))?;
        let backup_id = alloc
            .next_append()
            .checked_add(chain_len)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "page id overflow"))?;
        free.push(backup_id);
        let (freelist_head, _) =
            freelist::write_chain(&free, &mut alloc, &mut *backend, &self.page_cache)?;
        if alloc.alloc_append()? != backup_id {
            bail_coded!(ErrorCode::Int049, "migration backup page id mismatch");
        }

        let meta = MetaPage {
            generation: 1,
            page_count: alloc.next_append(),
            fact_count,
            last_checkpointed_tx_count: max_tx,
            eavt_root,
            aevt_root,
            avet_root,
            vaet_root,
            dict_root,
            next_eid,
            next_iid,
            freelist_head,
            freelist_count: u64::try_from(free.len())
                .map_err(|_| err_coded!(ErrorCode::Int048, "free-list length"))?,
            ..MetaPage::default()
        };
        let encoded = meta.encode();
        backend.write_page(backup_id, &encoded)?;
        backend.sync()?;
        backend.write_page(0, &encoded)?;
        backend.sync()?;
        Ok(meta)
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

    /// Commit pending facts as the next generation, copy-on-write (spec §8.3).
    ///
    /// New long values are appended to fresh value pages. Each of the five trees
    /// gets a copy-on-write batch insert: only touched leaves and their paths to
    /// the root are rewritten, at pages the active meta does not reference (its
    /// free list, read lazily, then appended). The replaced pages and the free-list
    /// pages read are pushed onto the free list in front of its unread tail. After
    /// a sync, the new meta goes to the other slot and is synced: the only commit
    /// point. A failure at any step leaves the active meta and all it references
    /// untouched. The pages written depend on the change, not on the graph size.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let pending_facts = self.storage.get_pending_facts();
        let m = self.meta;
        let next_gen = m.next_generation()?;

        let mut backend = self.lock()?;
        let mut alloc =
            PageAllocator::from_chain(m.freelist_head, m.freelist_count, m.page_count, next_gen);
        let encoded = encode(
            &pending_facts,
            &m,
            &mut alloc,
            &mut *backend,
            &self.page_cache,
        )?;
        let added =
            u64::try_from(encoded.index[0].len()).map_err(|_| err_coded!(ErrorCode::Stg024))?;
        let (next_eid, next_iid) = (encoded.next_eid, encoded.next_iid);
        let Encoded { index, dict, .. } = encoded;
        let mut freed: Vec<u64> = Vec::new();
        let mut roots = m.tree_roots();
        for (root, entries) in roots.iter_mut().zip(index.into_iter().chain([dict])) {
            *root = cow_insert(
                *root,
                entries,
                &mut *backend,
                &self.page_cache,
                &mut alloc,
                &mut freed,
            )?;
        }
        let [eavt_root, aevt_root, avet_root, vaet_root, dict_root] = roots;
        let (freelist_head, freelist_count) =
            alloc.finish_free_list(freed, &mut *backend, &self.page_cache)?;

        backend.sync()?;

        let new_meta = MetaPage {
            generation: next_gen,
            page_count: alloc.next_append(),
            fact_count: m
                .fact_count
                .checked_add(added)
                .ok_or_else(|| err_coded!(ErrorCode::Int048, "fact_count overflow"))?,
            last_checkpointed_tx_count: self.storage.current_tx_count(),
            eavt_root,
            aevt_root,
            avet_root,
            vaet_root,
            dict_root,
            next_eid,
            next_iid,
            freelist_head,
            freelist_count,
            ..m
        };
        backend.write_page(slot_page(next_gen), &new_meta.encode())?;
        backend.sync()?;
        drop(backend);

        self.dirty = false;
        self.activate(new_meta);
        // Clear pending — all data now on disk
        self.storage.post_checkpoint_clear();
        Ok(())
    }

    /// Get a reference to the underlying fact storage
    pub fn storage(&self) -> &FactStorage {
        &self.storage
    }

    /// The `last_checkpointed_tx_count` recorded in the active meta page.
    ///
    /// Used by WAL replay to skip entries already present in the main file.
    pub fn last_checkpointed_tx_count(&self) -> u64 {
        self.meta.last_checkpointed_tx_count
    }

    /// The active meta page's generation. A WAL created now records it as its
    /// base generation.
    pub fn generation(&self) -> u64 {
        self.meta.generation
    }

    /// The active meta page (for tests).
    #[cfg(test)]
    pub(crate) fn meta(&self) -> MetaPage {
        self.meta
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

/// Exactly one meta slot is valid, at generation `g`. Fail with STG-033 if a
/// later commit `g + 1` completed and its meta was lost (spec §4.1.1 table).
fn check_single_valid(
    backend: &dyn StorageBackend,
    m: MetaPage,
    wal_base: Option<u64>,
) -> Result<()> {
    let g = m.generation;
    match wal_base {
        // The WAL has existed since generation `b <= g` and is deleted only after
        // a durable commit, so it holds every fact that is not in `g`.
        Some(b) if b <= g => Ok(()),
        Some(b) => bail_coded!(ErrorCode::Stg033, g, format!("WAL base generation {b}")),
        None => {
            // Without a WAL, look where `g + 1` could have written: `M_g`'s free
            // list and the pages past `M_g.page_count`.
            let next = m.next_generation()?;
            let probe = PageCache::new(0);
            probe.set_generation_bound(g);
            let (free, _) = if m.freelist_head != 0 {
                freelist::read_chain(m.freelist_head, backend, &probe, m.page_count)?
            } else {
                (Vec::new(), Vec::new())
            };
            let end = backend.page_count()?;
            for id in free.into_iter().chain(m.page_count..end) {
                let Ok(p) = backend.read_page(id) else {
                    continue;
                };
                if page::verify(&p, id, u64::MAX).is_ok() && page::page_generation(&p)? == next {
                    bail_coded!(
                        ErrorCode::Stg033,
                        g,
                        format!("page {id} was written by generation {next}")
                    );
                }
            }
            Ok(())
        }
    }
}

/// True if `page0` is a torn write of the empty generation-1 meta onto a new
/// file: a prefix of its bytes, then zeros, with page 1 (if present) all zero.
/// Such a file never committed anything, so it is safe to initialise again.
fn is_torn_initial_meta(backend: &dyn StorageBackend, page0: &[u8]) -> Result<bool> {
    let expected = MetaPage::empty(1).encode();
    let written = page0
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |i| i.saturating_add(1));
    if page0.get(..written) != expected.get(..written) {
        return Ok(false);
    }
    if backend.page_count()? > 1 && backend.read_page(1)?.iter().any(|&b| b != 0) {
        return Ok(false);
    }
    Ok(true)
}

/// Encode `facts` for a checkpoint on top of `m`'s dictionary: assign ids,
/// write new long values to fresh value pages, and build every tree's entries.
fn encode(
    facts: &[Fact],
    m: &MetaPage,
    alloc: &mut PageAllocator,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
) -> Result<Encoded> {
    let mut encoder = Encoder::new(m.next_eid, m.next_iid);
    {
        let mut dict = DictReader::new(m.dict_root, &*backend, cache);
        for f in facts {
            encoder.stage(f, &mut dict)?;
        }
    }
    encoder.finish(alloc, backend, cache)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::storage::backend::FaultInjectingBackend;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::btree::collect_leaf_pages;
    use crate::storage::keys::Index;
    use crate::storage::reader::all_entries_as_facts;
    use std::collections::BTreeSet;
    use uuid::Uuid;

    fn entity(i: u128) -> Uuid {
        Uuid::from_u128(i + 1)
    }

    fn code(e: anyhow::Error) -> &'static str {
        crate::error::MinigrafError::from(e).code()
    }

    /// Page ids the meta references: tree nodes, value pages (through DICT's
    /// long-value entries), free-list chain pages.
    fn reachable(meta: &MetaPage, backend: &dyn StorageBackend) -> BTreeSet<u64> {
        let cache = PageCache::new(0);
        let mut nodes = Vec::new();
        for root in meta.tree_roots() {
            if root != 0 {
                collect_leaf_pages(root, backend, &cache, Some(&mut nodes)).unwrap();
            }
        }
        let mut set: BTreeSet<u64> = nodes.into_iter().collect();
        if meta.dict_root != 0 {
            let prefix = [crate::storage::keys::DICT_LONG_VALUE];
            for (k, _) in
                crate::storage::btree::prefix_scan(meta.dict_root, &prefix, backend, &cache)
                    .unwrap()
            {
                set.insert(crate::storage::keys::dict_long_value_ref(&k).unwrap().page);
            }
        }
        if meta.freelist_head != 0 {
            let (_, chain) =
                freelist::read_chain(meta.freelist_head, backend, &cache, meta.page_count).unwrap();
            set.extend(chain);
        }
        set
    }

    /// Spec §11 invariant: reachable pages and free ids are disjoint and together
    /// cover `2..page_count` exactly; every reachable page verifies.
    fn assert_space_accounted(meta: &MetaPage, backend: &dyn StorageBackend) {
        let reach = reachable(meta, backend);
        let free: Vec<u64> = if meta.freelist_head == 0 {
            Vec::new()
        } else {
            freelist::read_chain(
                meta.freelist_head,
                backend,
                &PageCache::new(0),
                meta.page_count,
            )
            .unwrap()
            .0
        };
        assert_eq!(free.len() as u64, meta.freelist_count, "freelist_count");
        let free_set: BTreeSet<u64> = free.iter().copied().collect();
        assert_eq!(free_set.len(), free.len(), "duplicate free id");
        assert!(
            reach.is_disjoint(&free_set),
            "a free page is still referenced"
        );
        let all: BTreeSet<u64> = (2..meta.page_count).collect();
        let union: BTreeSet<u64> = reach.union(&free_set).copied().collect();
        assert!(union == all, "pages leaked or out of range");
        for &id in &reach {
            page::verify(&backend.read_page(id).unwrap(), id, meta.generation)
                .expect("reachable page verifies");
        }
    }

    /// The four indexes hold the same facts (VAET: the ref facts), each in its
    /// own key order, and EAVT holds `fact_count` entries.
    fn assert_indexes_exact(meta: &MetaPage, backend: &dyn StorageBackend) {
        let cache = PageCache::new(0);
        let facts = |index, root| {
            let mut v: Vec<(Uuid, String, Vec<u8>, u64, i64, i64, bool)> =
                all_entries_as_facts(index, root, meta.dict_root, backend, &cache)
                    .unwrap()
                    .into_iter()
                    .map(|f| {
                        (
                            f.entity,
                            f.attribute,
                            crate::storage::index::encode_value(&f.value),
                            f.tx_count,
                            f.valid_from,
                            f.valid_to,
                            f.asserted,
                        )
                    })
                    .collect();
            v.sort();
            v
        };
        let e = facts(Index::Eavt, meta.eavt_root);
        assert_eq!(e.len() as u64, meta.fact_count, "fact_count");
        assert!(
            facts(Index::Aevt, meta.aevt_root) == e,
            "AEVT differs from EAVT"
        );
        assert!(
            facts(Index::Avet, meta.avet_root) == e,
            "AVET differs from EAVT"
        );
        let refs: Vec<_> = e
            .iter()
            .filter(|f| f.2.first() == Some(&0x06))
            .cloned()
            .collect();
        assert!(
            facts(Index::Vaet, meta.vaet_root) == refs,
            "VAET differs from EAVT refs"
        );
    }

    /// One transact of `n` facts: new and reused entities, strings and refs.
    fn transact_mixed<B: StorageBackend + 'static>(
        pfs: &mut PersistentFactStorage<B>,
        next_entity: &mut u128,
        n: usize,
        seed: u64,
    ) {
        let mut batch = Vec::new();
        for i in 0..n {
            let k = seed.wrapping_mul(31).wrapping_add(i as u64);
            let e = if *next_entity == 0 || k % 3 == 0 {
                *next_entity += 1;
                entity(*next_entity)
            } else {
                entity(u128::from(k) % *next_entity + 1)
            };
            let value = if k % 4 == 0 {
                Value::Ref(entity(u128::from(k / 4) % *next_entity + 1))
            } else {
                Value::String(format!("v{k}"))
            };
            batch.push((e, format!(":a{}", k % 5), value));
        }
        pfs.storage().transact(batch, None).unwrap();
        pfs.mark_dirty();
    }

    fn put_batch<B: StorageBackend + 'static>(
        pfs: &mut PersistentFactStorage<B>,
        ids: std::ops::Range<u128>,
    ) {
        let batch = ids
            .map(|i| (entity(i), ":test/n".to_string(), Value::Integer(i as i64)))
            .collect();
        pfs.storage().transact(batch, None).unwrap();
        pfs.mark_dirty();
    }

    fn count_n<B: StorageBackend + 'static>(pfs: &PersistentFactStorage<B>) -> usize {
        pfs.storage()
            .get_facts_by_attribute(&":test/n".to_string())
            .unwrap()
            .len()
    }

    /// Records the id of every page written.
    #[derive(Clone)]
    struct RecordingBackend {
        inner: MemoryBackend,
        written: Arc<Mutex<Vec<u64>>>,
    }

    impl StorageBackend for RecordingBackend {
        fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
            self.written.lock().unwrap().push(page_id);
            self.inner.write_page(page_id, data)
        }
        fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
            self.inner.read_page(page_id)
        }
        fn sync(&mut self) -> Result<()> {
            Ok(())
        }
        fn page_count(&self) -> Result<u64> {
            self.inner.page_count()
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "recording"
        }
    }

    // ── save ────────────────────────────────────────────────────────────────

    /// Many saves: indexes stay exact, space is fully accounted for, and no
    /// save writes a page the previous meta references (spec §8.1).
    #[test]
    fn saves_never_write_committed_pages_and_account_for_all_space() {
        let rec = RecordingBackend {
            inner: MemoryBackend::new(),
            written: Arc::new(Mutex::new(Vec::new())),
        };
        let log = rec.written.clone();
        let mem = rec.inner.clone();
        let mut pfs = PersistentFactStorage::new(rec, 64).unwrap();
        let mut next_entity = 0;
        for round in 0..25u64 {
            let before = pfs.meta();
            let committed = reachable(&before, &mem);
            log.lock().unwrap().clear();
            let n = 1 + ((round * 37) % 60) as usize;
            transact_mixed(&mut pfs, &mut next_entity, n, round);
            pfs.save().unwrap();
            let after = pfs.meta();
            assert_eq!(after.generation, before.generation + 1);
            for &id in log.lock().unwrap().iter() {
                assert!(!committed.contains(&id), "save overwrote a committed page");
                assert_ne!(id, slot_page(before.generation), "active meta overwritten");
            }
            assert_space_accounted(&after, &mem);
            assert_indexes_exact(&after, &mem);
        }
        // Freed tree pages are reused: the file does not grow by a full copy
        // of the trees per save.
        let m = pfs.meta();
        assert!(m.freelist_count > 0, "old tree pages are on the free list");
    }

    #[test]
    fn generations_alternate_between_the_two_meta_pages() {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 16).unwrap();
        assert_eq!(pfs.generation(), 1);
        for g in 2..6u64 {
            put_batch(&mut pfs, u128::from(g) * 10..u128::from(g) * 10 + 3);
            pfs.save().unwrap();
            let page = mem.read_page(slot_page(g)).unwrap();
            match MetaPage::decode(&page) {
                SlotState::Valid(m) => assert_eq!(m.generation, g),
                _ => panic!("slot for the new generation must be valid"),
            }
            let other = mem.read_page(slot_page(g - 1)).unwrap();
            match MetaPage::decode(&other) {
                SlotState::Valid(m) => assert_eq!(m.generation, g - 1),
                _ => panic!("the previous generation stays valid"),
            }
        }
    }

    #[test]
    fn new_file_gets_an_empty_generation_1_meta() {
        let mem = MemoryBackend::new();
        let pfs = PersistentFactStorage::new(mem.clone(), 16).unwrap();
        assert_eq!(pfs.storage().fact_count(), 0);
        assert_eq!(pfs.last_checkpointed_tx_count(), 0);
        drop(pfs);
        assert_eq!(mem.page_count().unwrap(), 2);
        match MetaPage::decode(&mem.read_page(0).unwrap()) {
            SlotState::Valid(m) => assert!(m == MetaPage::empty(1), "empty gen-1 meta"),
            _ => panic!("page 0 must hold the initial meta"),
        }
        assert!(matches!(
            MetaPage::decode(&mem.read_page(1).unwrap()),
            SlotState::Empty
        ));
        // Reopening an untouched new file is fine (g == 1, slot B empty, no WAL).
        let pfs = PersistentFactStorage::new(mem, 16).unwrap();
        assert_eq!(pfs.generation(), 1);
    }

    #[test]
    fn torn_initial_meta_is_initialised_again() {
        for n in [1usize, 16, 40, 2000] {
            let mut mem = MemoryBackend::new();
            let mut page = vec![0u8; PAGE_SIZE];
            page[..n].copy_from_slice(&MetaPage::empty(1).encode()[..n]);
            mem.write_page(0, &page).unwrap();
            let pfs = PersistentFactStorage::new(mem, 16).expect("re-init");
            assert_eq!(pfs.generation(), 1);
        }
        // Foreign bytes are never overwritten.
        let mut mem = MemoryBackend::new();
        mem.write_page(0, &vec![0xAA; PAGE_SIZE]).unwrap();
        assert_eq!(
            code(PersistentFactStorage::new(mem.clone(), 16).err().unwrap()),
            "STG-002"
        );
        assert!(mem.read_page(0).unwrap().iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn file_backed_reopen_sees_every_save() {
        use crate::storage::backend::FileBackend;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        let mut next_entity = 0;
        let last = {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 16).unwrap();
            for round in 0..5 {
                transact_mixed(&mut pfs, &mut next_entity, 40, round);
                pfs.save().unwrap();
            }
            pfs.meta()
        };
        let pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 16).unwrap();
        assert!(pfs.meta() == last, "reopen picks the last committed meta");
        assert_eq!(pfs.storage().fact_count(), 200);
        let backend = pfs.lock().unwrap();
        assert_space_accounted(&last, &*backend);
    }

    #[test]
    fn test_page_cache_capacity_reflects_constructed_value() {
        let zero_cap = PersistentFactStorage::new(MemoryBackend::new(), 0).unwrap();
        assert_eq!(zero_cap.page_cache_capacity(), 0);
        let nonzero_cap = PersistentFactStorage::new(MemoryBackend::new(), 64).unwrap();
        assert_eq!(nonzero_cap.page_cache_capacity(), 64);
    }

    #[test]
    fn same_tx_multi_value_visible_through_entity_and_attribute_lookups_before_save() {
        let pfs = PersistentFactStorage::new(MemoryBackend::new(), 256).unwrap();
        let e = Uuid::from_u128(42);
        pfs.storage()
            .transact(
                vec![
                    (e, ":kind".to_string(), Value::Keyword(":k/a".to_string())),
                    (e, ":kind".to_string(), Value::Keyword(":k/b".to_string())),
                ],
                None,
            )
            .unwrap();
        assert_eq!(pfs.storage().get_facts_by_entity(&e).unwrap().len(), 2);
        assert_eq!(
            pfs.storage()
                .get_facts_by_attribute(&":kind".to_string())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn save_and_reopen_preserve_facts_tx_id_and_tx_count() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 256).unwrap();
        let alice = entity(1);
        let bob = entity(2);
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
        pfs.storage().transact(vec![], None).unwrap(); // empty tx still counts
        let tx_id = pfs.storage().get_all_facts().unwrap()[0].tx_id;
        pfs.mark_dirty();
        pfs.save().unwrap();
        assert_eq!(pfs.last_checkpointed_tx_count(), 2);

        let pfs2 = PersistentFactStorage::new(pfs.into_backend().unwrap(), 256).unwrap();
        assert_eq!(pfs2.last_checkpointed_tx_count(), 2);
        assert_eq!(pfs2.storage().get_all_facts().unwrap()[0].tx_id, tx_id);
        let facts = pfs2.storage().get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 2, "EAVT resolves both facts after reopen");
        assert!(facts.iter().any(|f| matches!(f.value, Value::Ref(_))));
    }

    #[test]
    fn stream_all_returns_committed_facts_in_insertion_order() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 16).unwrap();
        for b in 0..4u128 {
            put_batch(&mut pfs, b * 50..(b + 1) * 50);
            pfs.save().unwrap();
        }
        let all = pfs.storage().get_all_facts().unwrap();
        let ns: Vec<i64> = all
            .iter()
            .map(|f| match f.value {
                Value::Integer(n) => n,
                _ => -1,
            })
            .collect();
        assert_eq!(ns, (0..200).collect::<Vec<i64>>());
    }

    #[test]
    fn save_with_poisoned_backend_mutex_returns_stg_016() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 16).unwrap();
        put_batch(&mut pfs, 0..1);
        let backend = pfs.backend.clone();
        let handle = std::thread::spawn(move || {
            let _guard = backend.lock().unwrap();
            panic!("deliberate poison for STG-016 regression test");
        });
        let _ = handle.join();
        let err = pfs.save().expect_err("poisoned mutex must fail");
        assert_eq!(code(err), "STG-016");
        pfs.dirty = false;
    }

    // ── meta selection (spec §4.1.1) ────────────────────────────────────────

    /// A store with generations 1..=3 committed (gen 3 in page 0, gen 2 in page 1).
    fn three_generations() -> MemoryBackend {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 16).unwrap();
        put_batch(&mut pfs, 0..20);
        pfs.save().unwrap();
        put_batch(&mut pfs, 20..40);
        pfs.save().unwrap();
        assert_eq!(pfs.generation(), 3);
        mem
    }

    fn damage(mem: &mut MemoryBackend, page_id: u64) {
        let mut p = mem.read_page(page_id).unwrap();
        p[30] ^= 0xFF;
        mem.write_page(page_id, &p).unwrap();
    }

    fn open_mem(
        mem: &MemoryBackend,
        wal: Option<u64>,
    ) -> Result<PersistentFactStorage<MemoryBackend>> {
        PersistentFactStorage::open(mem.clone(), 16, wal)
    }

    fn snapshot(mem: &MemoryBackend) -> Vec<Vec<u8>> {
        (0..mem.page_count().unwrap())
            .map(|i| mem.read_page(i).unwrap_or_default())
            .collect()
    }

    #[test]
    fn both_valid_opens_the_higher_generation() {
        let mem = three_generations();
        let pfs = open_mem(&mem, None).unwrap();
        assert_eq!(pfs.generation(), 3);
        assert_eq!(count_n(&pfs), 40);
    }

    #[test]
    fn torn_newest_with_wal_base_g_opens_at_g() {
        let mut mem = three_generations();
        damage(&mut mem, slot_page(3));
        let pfs = open_mem(&mem, Some(2)).unwrap();
        assert_eq!(pfs.generation(), 2);
        assert_eq!(count_n(&pfs), 20, "gen 2 holds the first batch only");
        // A WAL older than g holds every later fact too.
        assert_eq!(open_mem(&mem, Some(1)).unwrap().generation(), 2);
    }

    #[test]
    fn wal_newer_than_the_valid_meta_is_stg_033() {
        let mut mem = three_generations();
        damage(&mut mem, slot_page(3));
        let before = snapshot(&mem);
        assert_eq!(code(open_mem(&mem, Some(3)).err().unwrap()), "STG-033");
        assert!(
            snapshot(&mem) == before,
            "a failed open must not modify the file"
        );
    }

    #[test]
    fn no_wal_with_next_generation_pages_is_stg_033() {
        // Pages of gen 3 exist past gen 2's page_count.
        let mut mem = three_generations();
        damage(&mut mem, slot_page(3));
        assert_eq!(code(open_mem(&mem, None).err().unwrap()), "STG-033");

        // A gen-4 page on gen 3's free list, and nothing past its page_count:
        // keep only gen 3's extent of the file and drop gen 4's meta.
        let mem = three_generations();
        let m3 = match MetaPage::decode(&mem.read_page(slot_page(3)).unwrap()) {
            SlotState::Valid(m) => m,
            _ => panic!("gen 3 valid"),
        };
        let cache = PageCache::new(0);
        let (free, _) =
            freelist::read_chain(m3.freelist_head, &mem, &cache, m3.page_count).unwrap();
        let target = *free.first().expect("gen 3 has free pages");
        let mut trimmed = MemoryBackend::new();
        for id in 0..m3.page_count {
            trimmed.write_page(id, &mem.read_page(id).unwrap()).unwrap();
        }
        // Slot of gen 4 (page 1) is damaged: it held gen 2, now rotted.
        damage(&mut trimmed, slot_page(4));
        // No evidence yet: opens at gen 3.
        assert_eq!(open_mem(&trimmed, None).unwrap().generation(), 3);
        let alloc = PageAllocator::new(Vec::new(), 0, 4);
        alloc
            .write(
                &mut trimmed,
                &cache,
                target,
                page::new_page(page::PAGE_TYPE_LEAF, 0),
            )
            .unwrap();
        assert_eq!(code(open_mem(&trimmed, None).err().unwrap()), "STG-033");
    }

    #[test]
    fn no_wal_and_no_evidence_opens_at_g() {
        // The older slot rots; the newest is intact and is the only valid one.
        let mut mem = three_generations();
        damage(&mut mem, slot_page(2));
        let pfs = open_mem(&mem, None).unwrap();
        assert_eq!(pfs.generation(), 3);
        assert_eq!(count_n(&pfs), 40);
    }

    #[test]
    fn unknown_feature_bit_is_refused_without_modifying_the_file() {
        let mut mem = three_generations();
        let mut m = match MetaPage::decode(&mem.read_page(slot_page(3)).unwrap()) {
            SlotState::Valid(m) => m,
            _ => panic!("gen 3 valid"),
        };
        m.required_features = 1 << 9;
        mem.write_page(slot_page(3), &m.encode()).unwrap();
        let before = snapshot(&mem);
        assert_eq!(code(open_mem(&mem, None).err().unwrap()), "STG-034");
        assert!(snapshot(&mem) == before, "file unchanged");
    }

    #[test]
    fn pre_release_v8_single_header_is_stg_032() {
        let mut mem = MemoryBackend::new();
        let mut h = LegacyHeaderV7::new();
        h.version = 8;
        h.page_count = 3;
        let mut page = h.to_bytes();
        page.resize(PAGE_SIZE, 0);
        mem.write_page(0, &page).unwrap();
        mem.write_page(1, &vec![0u8; PAGE_SIZE]).unwrap();
        mem.write_page(2, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(code(open_mem(&mem, None).err().unwrap()), "STG-032");
    }

    /// A file from a v3.0.0 development build before covering keys has valid
    /// meta pages but trees without a dictionary: refused, never misread.
    #[test]
    fn pre_release_v8_meta_without_dictionary_is_stg_032() {
        let mut mem = three_generations();
        let mut m = match MetaPage::decode(&mem.read_page(slot_page(3)).unwrap()) {
            SlotState::Valid(m) => m,
            _ => panic!("gen 3 valid"),
        };
        m.dict_root = 0;
        m.next_eid = 0;
        mem.write_page(slot_page(3), &m.encode()).unwrap();
        let before = snapshot(&mem);
        assert_eq!(code(open_mem(&mem, None).err().unwrap()), "STG-032");
        assert!(snapshot(&mem) == before, "file unchanged");
    }

    // ── v7 migration (spec §9) ──────────────────────────────────────────────

    /// A v7 file: header in page 0 and v7 (0x02, 12-byte header) fact pages from 1.
    fn v7_file(facts: &[Fact], last_tx: u64) -> MemoryBackend {
        let mut mem = MemoryBackend::new();
        let pages = crate::storage::packed_pages::pack_facts_v7(facts);
        for (i, p) in pages.iter().enumerate() {
            mem.write_page(i as u64 + 1, p).unwrap();
        }
        let mut h = LegacyHeaderV7::new();
        h.page_count = pages.len() as u64 + 1;
        h.fact_page_count = pages.len() as u64;
        h.node_count = facts.len() as u64;
        h.last_checkpointed_tx_count = last_tx;
        h.header_checksum = h.checksum();
        let mut page = h.to_bytes();
        page.resize(PAGE_SIZE, 0);
        mem.write_page(0, &page).unwrap();
        mem
    }

    fn sample_facts(n: u128) -> Vec<Fact> {
        let pfs = PersistentFactStorage::new(MemoryBackend::new(), 0).unwrap();
        for i in 0..n {
            pfs.storage()
                .transact(
                    vec![(entity(i), ":test/n".to_string(), Value::Integer(i as i64))],
                    None,
                )
                .unwrap();
        }
        pfs.storage().get_all_facts().unwrap()
    }

    #[test]
    fn v7_file_migrates_to_generation_1_with_backup_meta() {
        let facts = sample_facts(300);
        // Two trailing empty transactions: the counter must not rewind (#371).
        let mem = v7_file(&facts, 302);
        let old_pages = mem.page_count().unwrap();
        let pfs = open_mem(&mem, None).unwrap();
        let m = pfs.meta();
        assert_eq!(m.generation, 1);
        assert_eq!(count_n(&pfs), 300);
        assert_eq!(pfs.last_checkpointed_tx_count(), 302);
        assert!(pfs.storage().current_tx_count() >= 302);
        assert_space_accounted(&m, &mem);
        assert_indexes_exact(&m, &mem);
        // Old pages 2.. and the backup page are free; the backup is the last page
        // and holds the same meta.
        let backup_id = m.page_count - 1;
        assert!(MetaPage::decode(&mem.read_page(backup_id).unwrap()).is_valid_generation(1));
        let (free, _) =
            freelist::read_chain(m.freelist_head, &mem, &PageCache::new(0), m.page_count).unwrap();
        assert!(free.contains(&backup_id));
        assert!((2..old_pages).all(|id| free.contains(&id)));
        drop(pfs);
        // Reopening does not migrate again.
        assert_eq!(
            open_mem(&mem, None).unwrap().meta().page_count,
            m.page_count
        );
    }

    #[test]
    fn crash_at_every_point_of_migration_keeps_every_fact() {
        let facts = sample_facts(120);
        let mut points = 0;
        for torn in [None, Some(0usize), Some(20), Some(2048)] {
            for k in 0u64.. {
                let mem = v7_file(&facts, 120);
                let (backend, config) = FaultInjectingBackend::with_config(mem.clone());
                {
                    let mut cfg = config.lock().unwrap();
                    cfg.fail_write_after = Some(k);
                    cfg.torn_write_bytes = torn;
                }
                let done = PersistentFactStorage::open(backend, 16, None).is_ok();
                let pfs = open_mem(&mem, None).expect("reopen after an interrupted migration");
                assert_eq!(count_n(&pfs), 120, "every v7 fact survives");
                assert_eq!(pfs.generation(), 1);
                assert_space_accounted(&pfs.meta(), &mem);
                if done {
                    break;
                }
                points += 1;
            }
        }
        assert!(points > 20, "expected many crash points");
    }

    // ── crash atomicity (spec §11) ──────────────────────────────────────────

    /// Kill `save()` at every page write (cleanly or with a torn page) and every
    /// sync, on a file already written by several checkpoints. Reopen with the
    /// WAL's base generation: the old generation (old facts, WAL replays the
    /// rest) or the new one (all facts), never anything else, and the space
    /// invariant holds either way.
    #[test]
    fn crash_at_every_point_in_save_loses_no_checkpointed_fact() {
        const PER_BATCH: u128 = 150;
        const OLD_BATCHES: u128 = 3;
        let build = || {
            let mem = MemoryBackend::new();
            let mut pfs = PersistentFactStorage::new(mem.clone(), 16).unwrap();
            for b in 0..OLD_BATCHES {
                put_batch(&mut pfs, b * PER_BATCH..(b + 1) * PER_BATCH);
                pfs.save().unwrap();
            }
            let g = pfs.generation();
            (mem, g)
        };
        let old_total = (OLD_BATCHES * PER_BATCH) as usize;
        let new_total = old_total + PER_BATCH as usize;

        let mut points = 0;
        let mut kinds: Vec<(Option<u64>, Option<u64>, Option<usize>)> = Vec::new();
        for torn in [None, Some(0usize), Some(512), Some(4095)] {
            kinds.push((Some(0), None, torn));
        }
        kinds.push((None, Some(0), None));
        for (write_kind, sync_kind, torn) in kinds {
            for k in 0u64.. {
                let (mem, g) = build();
                let (backend, config) = FaultInjectingBackend::with_config(mem.clone());
                let mut pfs = PersistentFactStorage::new(backend, 16).unwrap();
                put_batch(
                    &mut pfs,
                    OLD_BATCHES * PER_BATCH..(OLD_BATCHES + 1) * PER_BATCH,
                );
                {
                    let mut cfg = config.lock().unwrap();
                    cfg.fail_write_after = write_kind.map(|_| k);
                    cfg.fail_sync_after = sync_kind.map(|_| k);
                    cfg.torn_write_bytes = torn;
                }
                let saved = pfs.save().is_ok();
                pfs.dirty = false; // simulated kill: no auto-save on drop
                drop(pfs);

                // The WAL holding the in-flight batch was created at base g.
                let pfs = open_mem(&mem, Some(g)).expect("reopen after crash");
                let count = count_n(&pfs);
                if pfs.generation() == g + 1 {
                    assert_eq!(count, new_total, "new generation sees all facts");
                } else {
                    assert_eq!(pfs.generation(), g, "otherwise the old generation");
                    assert_eq!(count, old_total, "old generation sees the old facts");
                    assert!(!saved, "a reported success must be durable");
                }
                for i in 0..count as u128 {
                    let facts = pfs.storage().get_facts_by_entity(&entity(i)).unwrap();
                    assert_eq!(facts.len(), 1, "entity lookup lost a checkpointed fact");
                }
                assert_space_accounted(&pfs.meta(), &mem);
                if saved {
                    assert_eq!(count, new_total, "completed save keeps all facts");
                    break;
                }
                points += 1;
            }
        }
        assert!(points > 50, "expected many crash points");
    }

    // ── covering reads (spec §7, §11) ───────────────────────────────────────

    /// A fact's identity for set comparison (values via their canonical bytes).
    type FactKey = (Uuid, String, Vec<u8>, u64, u64, i64, i64, bool);

    fn fact_key(f: &Fact) -> FactKey {
        (
            f.entity,
            f.attribute.clone(),
            crate::storage::index::encode_value(&f.value),
            f.tx_id,
            f.tx_count,
            f.valid_from,
            f.valid_to,
            f.asserted,
        )
    }

    fn as_set(facts: Vec<Fact>) -> Vec<FactKey> {
        let mut v: Vec<FactKey> = facts.iter().map(fact_key).collect();
        v.sort();
        v
    }

    /// Mixed facts: short and long strings (some repeated), keywords, refs,
    /// numbers, multi-valued attributes and retractions.
    fn mixed_batch(round: u64, n: u64) -> (Vec<(Uuid, String, Value)>, Vec<(Uuid, String, Value)>) {
        let mut asserts = Vec::new();
        let mut retracts = Vec::new();
        for i in 0..n {
            let k = round * 1000 + i;
            let e = entity(u128::from(k % 37));
            let a = format!(":attr/{}", k % 7);
            let v = match k % 8 {
                0 => Value::String(format!("short {k}")),
                1 => Value::String(format!("{}{}", "long value ".repeat(10), k % 5)),
                2 => Value::Keyword(format!(":kw/{}", k % 4)),
                3 => Value::Ref(entity(u128::from(k % 41))),
                4 => Value::Integer(i64::try_from(k).unwrap() - 500),
                5 => Value::Float(k as f64 / 7.0),
                6 => Value::Boolean(k % 2 == 0),
                _ => Value::String("x".repeat(65 + (k % 3) as usize)),
            };
            if k % 11 == 0 {
                retracts.push((e, a, v));
            } else {
                asserts.push((e, a, v));
            }
        }
        (asserts, retracts)
    }

    fn assert_reads_match(pfs: &PersistentFactStorage<MemoryBackend>, model: &FactStorage) {
        let s = pfs.storage();
        assert!(
            as_set(s.get_all_facts().unwrap()) == as_set(model.get_all_facts().unwrap()),
            "all facts"
        );
        for i in 0..42u128 {
            let e = entity(i);
            assert!(
                as_set(s.get_facts_by_entity(&e).unwrap())
                    == as_set(model.get_facts_by_entity(&e).unwrap()),
                "by entity"
            );
            for a in 0..7 {
                let a = format!(":attr/{a}");
                assert!(
                    as_set(s.get_facts_by_entity_attribute_indexed(&e, &a).unwrap())
                        == as_set(model.get_facts_by_entity_attribute_indexed(&e, &a).unwrap()),
                    "by entity and attribute"
                );
            }
        }
        for a in 0..8 {
            let a = format!(":attr/{a}");
            assert!(
                as_set(s.get_facts_by_attribute(&a).unwrap())
                    == as_set(model.get_facts_by_attribute(&a).unwrap()),
                "by attribute"
            );
        }
    }

    /// Committed reads return exactly what was written, across checkpoints
    /// (new ids, reused ids, dedup) and a reopen.
    #[test]
    fn committed_reads_match_the_model_across_checkpoints() {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 32).unwrap();
        let model = FactStorage::new();
        for round in 0..8u64 {
            let (asserts, retracts) = mixed_batch(round, 120);
            pfs.storage().transact(asserts, None).unwrap();
            if !retracts.is_empty() {
                pfs.storage().retract(retracts).unwrap();
            }
            for f in pfs.storage().get_pending_facts() {
                model.load_fact(f).unwrap();
            }
            pfs.mark_dirty();
            pfs.save().unwrap();
            assert_reads_match(&pfs, &model);
            assert_space_accounted(&pfs.meta(), &mem);
            assert_indexes_exact(&pfs.meta(), &mem);
        }
        drop(pfs);
        let pfs = PersistentFactStorage::new(mem, 32).unwrap();
        assert_reads_match(&pfs, &model);
    }

    /// Records the id of every page read.
    #[derive(Clone)]
    struct ReadLog {
        inner: MemoryBackend,
        read: Arc<Mutex<Vec<u64>>>,
    }

    impl StorageBackend for ReadLog {
        fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
            self.inner.write_page(page_id, data)
        }
        fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
            self.read.lock().unwrap().push(page_id);
            self.inner.read_page(page_id)
        }
        fn sync(&mut self) -> Result<()> {
            Ok(())
        }
        fn page_count(&self) -> Result<u64> {
            self.inner.page_count()
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "read-log"
        }
    }

    /// Reading facts without long values touches only index and DICT pages.
    #[test]
    fn reads_without_long_values_touch_no_value_page() {
        let log = ReadLog {
            inner: MemoryBackend::new(),
            read: Arc::new(Mutex::new(Vec::new())),
        };
        let reads = log.read.clone();
        let mem = log.inner.clone();
        let mut pfs = PersistentFactStorage::new(log, 0).unwrap();
        let long = "L".repeat(500);
        pfs.storage()
            .transact(
                vec![
                    (entity(1), ":name".into(), Value::String("Ann".into())),
                    (entity(1), ":tag".into(), Value::Keyword(":t/x".into())),
                    (entity(2), ":bio".into(), Value::String(long.clone())),
                ],
                None,
            )
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        let page_type = |id: u64| mem.read_page(id).unwrap()[0];

        reads.lock().unwrap().clear();
        assert_eq!(
            pfs.storage().get_facts_by_entity(&entity(1)).unwrap().len(),
            2
        );
        let touched = std::mem::take(&mut *reads.lock().unwrap());
        assert!(!touched.is_empty());
        assert!(
            touched
                .iter()
                .all(|&id| page_type(id) != page::PAGE_TYPE_VALUE),
            "no value page read for short values"
        );

        let bio = pfs.storage().get_facts_by_entity(&entity(2)).unwrap();
        assert!(matches!(&bio[0].value, Value::String(s) if *s == long));
        let touched = std::mem::take(&mut *reads.lock().unwrap());
        assert!(
            touched
                .iter()
                .any(|&id| page_type(id) == page::PAGE_TYPE_VALUE),
            "a long value is read from its value page"
        );
    }

    fn value_page_count(mem: &MemoryBackend) -> usize {
        (2..mem.page_count().unwrap())
            .filter(|&id| mem.read_page(id).unwrap()[0] == page::PAGE_TYPE_VALUE)
            .count()
    }

    /// Re-asserting or retracting a committed long value writes no value page.
    #[test]
    fn long_values_are_stored_once_across_checkpoints() {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 16).unwrap();
        let long = Value::String("y".repeat(300));
        pfs.storage()
            .transact(vec![(entity(1), ":doc".into(), long.clone())], None)
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        assert_eq!(value_page_count(&mem), 1);
        pfs.storage()
            .retract(vec![(entity(1), ":doc".into(), long.clone())])
            .unwrap();
        pfs.storage()
            .transact(vec![(entity(2), ":doc".into(), long)], None)
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        assert_eq!(value_page_count(&mem), 1, "dedup: no new value page");
        assert_eq!(pfs.storage().get_all_facts().unwrap().len(), 3);
        assert_space_accounted(&pfs.meta(), &mem);
    }

    /// A v7 file with every value type, long strings, refs and retractions
    /// migrates to exactly the same facts.
    #[test]
    fn v7_file_with_mixed_values_migrates_exactly() {
        let model = FactStorage::new();
        for round in 0..3 {
            let (asserts, retracts) = mixed_batch(round, 90);
            model.transact(asserts, None).unwrap();
            model.retract(retracts).unwrap();
        }
        let facts = model.get_all_facts().unwrap();
        let mem = v7_file(&facts, model.current_tx_count());
        let pfs = open_mem(&mem, None).unwrap();
        assert!(
            as_set(pfs.storage().get_all_facts().unwrap()) == as_set(facts),
            "migrated facts equal the v7 facts"
        );
        assert_indexes_exact(&pfs.meta(), &mem);
        assert_space_accounted(&pfs.meta(), &mem);
    }

    // ── checkpoint cost (spec §11, #434) ────────────────────────────────────

    /// Depth of the tree at `root` (a lone leaf is depth 1).
    fn depth(root: u64, backend: &dyn StorageBackend) -> u64 {
        let cache = PageCache::new(0);
        let mut d = 1;
        let mut id = root;
        loop {
            let p = cache.get_or_load(id, backend).unwrap();
            if p[0] != page::PAGE_TYPE_INTERNAL {
                return d;
            }
            id = crate::storage::node::Internal::new(&p[..])
                .unwrap()
                .child(0)
                .unwrap();
            d += 1;
        }
    }

    /// Entity `i` of the cost tests: spaced so that new UUIDs can fall between.
    fn spaced(i: u64) -> Uuid {
        Uuid::from_u128(u128::from(i) << 20)
    }

    /// Pages written by a checkpoint that adds `k` facts to a graph of `n`, and
    /// the summed depth of the five trees. New facts get new entities; with
    /// `sequential` their UUIDs and values follow the existing ones, otherwise
    /// they are random.
    fn checkpoint_writes(n: u64, k: u64, sequential: bool) -> (usize, u64) {
        let rec = RecordingBackend {
            inner: MemoryBackend::new(),
            written: Arc::new(Mutex::new(Vec::new())),
        };
        let log = rec.written.clone();
        let mem = rec.inner.clone();
        let mut pfs = PersistentFactStorage::new(rec, 256).unwrap();
        for chunk in (0..n).collect::<Vec<u64>>().chunks(5000) {
            let batch = chunk
                .iter()
                .map(|&i| (spaced(i), ":n".to_string(), Value::Integer(i as i64 * 1000)))
                .collect();
            pfs.storage().transact(batch, None).unwrap();
        }
        pfs.mark_dirty();
        pfs.save().unwrap();
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let batch = (0..k)
            .map(|i| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                if sequential {
                    (
                        spaced(n + i),
                        ":n".to_string(),
                        Value::Integer((n + i) as i64 * 1000),
                    )
                } else {
                    // Between existing UUIDs and values, so every tree but the
                    // ones keyed by new ids takes random-position inserts.
                    let r = rng % n;
                    (
                        Uuid::from_u128(u128::from(r) << 20 | 1),
                        ":n".to_string(),
                        Value::Integer(r as i64 * 1000 + 1),
                    )
                }
            })
            .collect();
        pfs.storage().transact(batch, None).unwrap();
        pfs.mark_dirty();
        log.lock().unwrap().clear();
        pfs.save().unwrap();
        let written = log.lock().unwrap().len();
        let m = pfs.meta();
        let depths = m
            .tree_roots()
            .iter()
            .filter(|&&r| r != 0)
            .map(|&r| depth(r, &mem))
            .sum();
        assert_space_accounted(&m, &mem);
        (written, depths)
    }

    /// #434 acceptance: a checkpoint's cost depends on the change, not on the
    /// graph. With keys at the right edge, 10x more facts costs at most the
    /// extra tree depth; with random keys, at most one path per new entry per
    /// tree.
    #[test]
    fn checkpoint_cost_does_not_grow_with_the_graph() {
        for k in [1u64, 100] {
            let (small, d_small) = checkpoint_writes(10_000, k, true);
            let (large, d_large) = checkpoint_writes(100_000, k, true);
            assert!(
                large <= small + (d_large - d_small) as usize + 2,
                "k={k}: {small} pages at 10k, {large} at 100k"
            );
            let (random, d) = checkpoint_writes(100_000, k, false);
            // Five trees, one path each per new entry, plus a value-free
            // allowance for free-list pages and the meta.
            assert!(
                random <= (k * d) as usize + 8,
                "k={k}: {random} pages for random keys (depth sum {d})"
            );
        }
        let (one, _) = checkpoint_writes(100_000, 1, true);
        assert!(one <= 20, "one fact at 100k writes {one} pages");
    }

    // ── corruption surfacing (spec §11) ─────────────────────────────────────

    /// A file with several checkpoints, long values and a free list.
    fn corruptible() -> MemoryBackend {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 0).unwrap();
        for round in 0..4u64 {
            let (asserts, retracts) = mixed_batch(round, 200);
            pfs.storage().transact(asserts, None).unwrap();
            pfs.storage().retract(retracts).unwrap();
            pfs.mark_dirty();
            pfs.save().unwrap();
        }
        assert!(pfs.meta().freelist_head != 0, "fixture has a free list");
        mem
    }

    /// An independent copy of `mem` (`clone` shares the pages).
    fn deep_copy(mem: &MemoryBackend) -> MemoryBackend {
        let mut copy = MemoryBackend::new();
        for id in 0..mem.page_count().unwrap() {
            if let Ok(p) = mem.read_page(id) {
                copy.write_page(id, &p).unwrap();
            }
        }
        copy
    }

    /// Flip a body byte of `id`, leaving its CRC stale.
    fn flip(mem: &mut MemoryBackend, id: u64) {
        let mut p = mem.read_page(id).unwrap();
        p[2000] ^= 0x5A;
        mem.write_page(id, &p).unwrap();
    }

    fn leaves_of(root: u64, mem: &MemoryBackend) -> Vec<u64> {
        let cache = PageCache::new(0);
        let mut nodes = Vec::new();
        collect_leaf_pages(root, mem, &cache, Some(&mut nodes)).unwrap();
        nodes
            .into_iter()
            .filter(|&id| mem.read_page(id).unwrap()[0] == page::PAGE_TYPE_LEAF)
            .collect()
    }

    /// Every committed read that reaches a damaged page fails with STG-029; it
    /// never returns the page's data or silently fewer facts.
    #[test]
    fn damaged_pages_surface_as_stg_029_on_read() {
        let mem = corruptible();
        let m = open_mem(&mem, None).unwrap().meta();
        let value_page = (2..m.page_count)
            .find(|&id| mem.read_page(id).unwrap()[0] == page::PAGE_TYPE_VALUE)
            .expect("fixture has a value page");
        let eavt_leaf = *leaves_of(m.eavt_root, &mem).last().unwrap();
        let dict_leaf = leaves_of(m.dict_root, &mem)[0];
        let internal = m.eavt_root;
        assert_eq!(
            mem.read_page(internal).unwrap()[0],
            page::PAGE_TYPE_INTERNAL
        );
        for (what, id) in [
            ("EAVT leaf", eavt_leaf),
            ("EAVT internal node", internal),
            ("DICT leaf", dict_leaf),
            ("value page", value_page),
        ] {
            let mut damaged = deep_copy(&mem);
            flip(&mut damaged, id);
            let pfs = PersistentFactStorage::open(damaged, 0, None).expect("open reads only metas");
            let err = pfs
                .storage()
                .get_all_facts()
                .err()
                .unwrap_or_else(|| panic!("{what}: read must fail"));
            assert_eq!(code(err), "STG-029", "{what}");
        }
        // An AEVT leaf: attribute scans that reach it fail; other reads still work.
        let aevt_leaf = leaves_of(m.aevt_root, &mem)[0];
        let mut damaged = deep_copy(&mem);
        flip(&mut damaged, aevt_leaf);
        let pfs = PersistentFactStorage::open(damaged, 0, None).unwrap();
        let failures = (0..8)
            .filter(|a| {
                pfs.storage()
                    .get_facts_by_attribute(&format!(":attr/{a}"))
                    .is_err()
            })
            .count();
        assert!(
            failures > 0,
            "the attribute stored in the damaged leaf fails"
        );
        assert!(pfs.storage().get_all_facts().is_ok(), "EAVT is intact");
    }

    /// A damaged free-list page fails the next checkpoint that pops from it. The
    /// file is not changed in any way that matters: it still opens at its last
    /// generation and every fact reads back.
    #[test]
    fn damaged_free_list_page_fails_the_checkpoint_not_the_file() {
        let mem = corruptible();
        let before = open_mem(&mem, None).unwrap();
        let m = before.meta();
        let all = as_set(before.storage().get_all_facts().unwrap());
        drop(before);
        let mut damaged = deep_copy(&mem);
        flip(&mut damaged, m.freelist_head);
        let mut pfs = PersistentFactStorage::open(damaged.clone(), 0, None).unwrap();
        assert!(
            as_set(pfs.storage().get_all_facts().unwrap()) == all,
            "reads do not touch the free list"
        );
        let (asserts, _) = mixed_batch(9, 50);
        pfs.storage().transact(asserts, None).unwrap();
        pfs.mark_dirty();
        let err = pfs.save().unwrap_err();
        assert_eq!(code(err), "STG-029");
        pfs.dirty = false;
        drop(pfs);
        let reopened = PersistentFactStorage::open(damaged, 0, None).unwrap();
        assert_eq!(
            reopened.generation(),
            m.generation,
            "the last checkpoint stands"
        );
        assert!(as_set(reopened.storage().get_all_facts().unwrap()) == all);
    }

    // ── size (spec §10.1, §11, #433) ────────────────────────────────────────

    /// File bytes per fact for `entities` entities in #433's acceptance shape:
    /// 10 attributes each, 20 % of attributes multi-valued, 10 % retracted and
    /// re-asserted, written over 10 checkpoints.
    fn bytes_per_fact(entities: u64) -> (u64, u64) {
        let mem = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(mem.clone(), 256).unwrap();
        let mut facts = 0u64;
        let per_round = entities / 10;
        for round in 0..10 {
            let mut asserts = Vec::new();
            for e in round * per_round..(round + 1) * per_round {
                let ent = entity(u128::from(e));
                for a in 0..10u64 {
                    let attr = format!(":person/a{a}");
                    let v = match a {
                        0 => Value::String(format!("Name {e}")),
                        1 => Value::Keyword(format!(":status/s{}", e % 4)),
                        2 => Value::Ref(entity(u128::from((e * 7919) % entities))),
                        3 => Value::Float(e as f64 / 3.0),
                        _ => Value::Integer((e * 10 + a) as i64),
                    };
                    asserts.push((ent, attr.clone(), v));
                    if (e + a) % 5 == 0 {
                        // Multi-valued: a second value.
                        asserts.push((ent, attr, Value::Integer(-((e * 100 + a) as i64) - 1)));
                    }
                }
            }
            facts += asserts.len() as u64;
            let churn: Vec<_> = asserts.iter().step_by(10).cloned().collect();
            pfs.storage().transact(asserts, None).unwrap();
            pfs.storage().retract(churn.clone()).unwrap();
            pfs.storage().transact(churn.clone(), None).unwrap();
            facts += 2 * churn.len() as u64;
            pfs.mark_dirty();
            pfs.save().unwrap();
        }
        assert_eq!(pfs.meta().fact_count, facts);
        let bytes = mem.page_count().unwrap() * PAGE_SIZE as u64;
        (bytes / facts, facts)
    }

    /// Spec §11: at most 200 bytes per fact, history included (G6: 300 B at 1B).
    #[test]
    fn size_per_fact_in_433_shape() {
        let (per_fact, facts) = bytes_per_fact(10_000);
        assert!(facts > 100_000);
        assert!(per_fact <= 200, "{per_fact} bytes per fact");
    }

    /// The same at 1M facts. Slow in a debug build: run by the scheduled job.
    #[test]
    #[ignore]
    fn size_per_fact_in_433_shape_1m() {
        let (per_fact, facts) = bytes_per_fact(80_000);
        assert!(facts > 1_000_000);
        assert!(per_fact <= 200, "{per_fact} bytes per fact");
    }
}
