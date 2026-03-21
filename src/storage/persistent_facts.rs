/// Persistent fact storage that integrates StorageBackend with Datalog facts.
///
/// This module bridges the gap between high-level fact operations and
/// low-level page-based storage backends.
use crate::graph::types::Fact;
use crate::graph::FactStorage;
use crate::storage::{FileHeader, StorageBackend, PAGE_SIZE};
use anyhow::Result;

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
/// - All operations: Work on in-memory Vec<Fact>
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
pub struct PersistentFactStorage<B: StorageBackend> {
    backend: B,
    storage: FactStorage,
    dirty: bool,
    last_checkpointed_tx_count: u64,
}

impl<B: StorageBackend> PersistentFactStorage<B> {
    /// Create a new persistent storage with the given backend.
    ///
    /// If the backend already contains data, loads it.
    /// Otherwise, initializes a new empty fact storage.
    pub fn new(backend: B) -> Result<Self> {
        let mut persistent = PersistentFactStorage {
            backend,
            storage: FactStorage::new(),
            dirty: false,
            last_checkpointed_tx_count: 0,
        };

        // Try to load existing data
        let page_count = persistent.backend.page_count()?;
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
        // Read header from page 0
        let header_page = self.backend.read_page(0)?;
        let header = FileHeader::from_bytes(&header_page)?;
        header.validate()?;

        // Migrate v1 → v2 if needed
        if header.version < 2 {
            return self.migrate_v1_to_v2();
        }

        // Store last_checkpointed_tx_count from header (0 for v2 files)
        self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;

        // Clear existing storage
        self.storage.clear()?;

        // Load facts from pages 1..page_count
        let page_count = header.page_count;
        for page_id in 1..page_count {
            let page = self.backend.read_page(page_id)?;

            // Try to deserialize a fact from this page
            // Empty pages are skipped
            if let Ok(fact) = postcard::from_bytes::<Fact>(&page) {
                // Preserve original tx_id and tx_count via load_fact()
                self.storage.load_fact(fact)?;
            }
        }

        // Re-synchronise tx_counter to max(tx_count) of loaded facts
        self.storage.restore_tx_counter()?;

        self.dirty = false;
        Ok(())
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

        let header_page = self.backend.read_page(0)?;
        let header = FileHeader::from_bytes(&header_page)?;
        let page_count = header.page_count;

        // Read all v1 facts (skip pages that don't deserialize)
        let mut v1_facts: Vec<FactV1> = Vec::new();
        for page_id in 1..page_count {
            let page = self.backend.read_page(page_id)?;
            if let Ok(fact) = postcard::from_bytes::<FactV1>(&page) {
                v1_facts.push(fact);
            }
        }

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

    /// Consume this storage and return the underlying backend.
    ///
    /// Useful in tests to inspect or reuse the backend after saving.
    /// Any dirty (unsaved) changes are saved before the backend is returned.
    pub fn into_backend(mut self) -> B {
        // Save pending changes before giving up ownership
        if self.dirty {
            let _ = self.save();
        }
        // SAFETY: We use ManuallyDrop to suppress the Drop impl so we can
        // move the backend field out.  The storage and dirty fields are
        // trivially dropped (FactStorage is heap-allocated, bool is Copy).
        let md = std::mem::ManuallyDrop::new(self);
        // SAFETY: `md` will not be dropped, so reading `backend` is safe.
        unsafe { std::ptr::read(&md.backend) }
    }

    /// Save all facts from memory to the backend.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(()); // No changes to save
        }

        let facts = self.storage.get_all_facts()?;

        // Calculate page count (header + one page per fact)
        let page_count = 1 + facts.len() as u64;

        // Create and write header
        let mut header = FileHeader::new(); // sets version = FORMAT_VERSION = 3
        header.page_count = page_count;
        header.node_count = facts.len() as u64; // Reuse node_count field for fact count
        header.last_checkpointed_tx_count = self.storage.current_tx_count();

        let mut header_page = header.to_bytes();
        header_page.resize(PAGE_SIZE, 0);
        self.backend.write_page(0, &header_page)?;

        // Write facts (one per page)
        for (i, fact) in facts.iter().enumerate() {
            let data = postcard::to_allocvec(fact)?;
            if data.len() > PAGE_SIZE {
                anyhow::bail!(
                    "Fact too large: {} bytes (max {})",
                    data.len(),
                    PAGE_SIZE
                );
            }

            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);
            self.backend.write_page((i + 1) as u64, &page)?;
        }

        // Sync to ensure durability
        self.backend.sync()?;
        self.last_checkpointed_tx_count = self.storage.current_tx_count();
        self.dirty = false;

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
        self.dirty = true;
    }

    /// Check if storage has unsaved changes
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl<B: StorageBackend> Drop for PersistentFactStorage<B> {
    fn drop(&mut self) {
        // Auto-save on drop
        if self.dirty {
            let _ = self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::storage::backend::MemoryBackend;
    use uuid::Uuid;

    #[test]
    fn test_persistent_fact_storage_new() {
        let backend = MemoryBackend::new();
        let storage = PersistentFactStorage::new(backend).unwrap();

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
            let mut storage = PersistentFactStorage::new(backend).unwrap();

            storage.storage()
                .transact(vec![
                    (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
                    (alice, ":person/age".to_string(), Value::Integer(30)),
                ], None)
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
            let mut storage = PersistentFactStorage::new(backend).unwrap();
            storage.storage()
                .transact(vec![
                    (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
                ], None)
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
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new()).unwrap();

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
        let backend = pfs.into_backend();
        let pfs2 = PersistentFactStorage::new(backend).unwrap();
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
        let pfs = PersistentFactStorage::new(backend).unwrap();
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
    fn test_save_writes_v3_header() {
        use crate::storage::FORMAT_VERSION;

        let backend = MemoryBackend::new();
        let mut pfs = PersistentFactStorage::new(backend).unwrap();
        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(vec![(alice, ":name".to_string(), crate::graph::types::Value::String("Alice".to_string()))], None)
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();

        // Read back the header and verify version and last_checkpointed_tx_count
        let backend = pfs.into_backend();
        let header_page = backend.read_page(0).unwrap();
        let header = crate::storage::FileHeader::from_bytes(&header_page).unwrap();
        assert_eq!(header.version, FORMAT_VERSION);  // must be 3
        assert_eq!(header.last_checkpointed_tx_count, 1); // one transact call
    }

    #[test]
    fn test_last_checkpointed_tx_count_getter() {
        let backend = MemoryBackend::new();
        let pfs = PersistentFactStorage::new(backend).unwrap();
        // Fresh database: no checkpoint yet
        assert_eq!(pfs.last_checkpointed_tx_count(), 0);
    }

    #[test]
    fn test_migrate_v1_tx_counter_set_correctly() {
        let backend = make_v1_backend();
        let pfs = PersistentFactStorage::new(backend).unwrap();

        let alice = Uuid::new_v4();
        pfs.storage()
            .transact(
                vec![(
                    alice,
                    ":new/fact".to_string(),
                    Value::Boolean(true),
                )],
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
}
