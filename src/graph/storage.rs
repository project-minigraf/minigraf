use crate::graph::types::{
    Attribute, EntityId, Fact, TransactOptions, TxId, VALID_TIME_FOREVER, Value, tx_id_now,
};
use crate::query::datalog::types::AsOf;
use crate::storage::index::{FactRef, Indexes, encode_value};
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

// ============================================================================
// Datalog Fact Storage (Phase 3+)
// ============================================================================

/// Private container that co-locates the fact list and all four indexes under
/// a single `RwLock`. This ensures facts and indexes are always updated together
/// without needing a second lock.
struct FactData {
    facts: Vec<Fact>,
    pending_indexes: Indexes,
    /// Resolves committed (on-disk) FactRefs to Fact objects.
    /// None for in-memory databases or before load() is called.
    committed: Option<Arc<dyn crate::storage::CommittedFactReader>>,
    /// Provides bounded range scans over the four committed (on-disk) covering indexes.
    /// Set by `set_committed_index_reader()` after open/migration/checkpoint.
    committed_index_reader: Option<Arc<dyn crate::storage::CommittedIndexReader>>,
}

/// Resolve a FactRef to a Fact using the committed reader (for on-disk facts)
/// or the pending facts vector (for in-memory facts with page_id=0).
fn resolve_fact_ref(d: &FactData, fr: FactRef) -> Result<Fact> {
    if fr.page_id == 0 {
        // Pending fact: page_id=0, slot_index is index into d.facts
        d.facts
            .get(fr.slot_index as usize)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("pending fact index {} out of bounds", fr.slot_index))
    } else {
        // Committed fact: resolve via CommittedFactReader
        match &d.committed {
            Some(loader) => loader.resolve(fr),
            None => anyhow::bail!(
                "no CommittedFactReader but got committed FactRef (page_id={})",
                fr.page_id
            ),
        }
    }
}

/// In-memory storage for Datalog facts with transaction support
///
/// FactStorage maintains an append-only log of facts. Facts are never deleted,
/// only retracted (with asserted=false). This enables:
/// - Full history tracking
/// - Time travel queries (Phase 4)
/// - Audit trails
///
/// # Storage Model (Phase 3-6)
///
/// This is a simple in-memory store using `Vec<Fact>` plus four covering
/// indexes (EAVT, AEVT, AVET, VAET). For persistence, see `PersistentFactStorage`
/// which wraps this with a "load all, save all" strategy.
///
/// # Examples
/// ```
/// use minigraf::{FactStorage, Value};
/// use uuid::Uuid;
///
/// let storage = FactStorage::new();
///
/// // Add facts (automatic timestamping)
/// let alice = Uuid::new_v4();
/// storage.transact(vec![
///     (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
///     (alice, ":person/age".to_string(), Value::Integer(30)),
/// ], None).unwrap();
///
/// // Query facts
/// let facts = storage.get_facts_by_entity(&alice).unwrap();
/// assert_eq!(facts.len(), 2);
/// ```
#[derive(Clone)]
pub struct FactStorage {
    /// Append-only log of all facts (assertions and retractions) plus indexes.
    data: Arc<RwLock<FactData>>,
    /// Monotonically incrementing batch counter — increments once per transact/retract call.
    tx_counter: Arc<AtomicU64>,
}

impl Default for FactStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl FactStorage {
    /// Create a new empty fact storage
    pub fn new() -> Self {
        FactStorage {
            data: Arc::new(RwLock::new(FactData {
                facts: Vec::new(),
                pending_indexes: Indexes::new(),
                committed: None,
                committed_index_reader: None,
            })),
            tx_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Transact a batch of facts with automatic timestamping
    ///
    /// All facts in a single transaction get the same timestamp (TxId) and the same
    /// `tx_count`. The `tx_count` increments once per call (not per fact), so all
    /// facts in a batch share the same counter value.
    ///
    /// # Arguments
    /// * `fact_tuples` - Vec of (EntityId, Attribute, Value) tuples to assert
    /// * `opts` - Optional TransactOptions to override valid_from / valid_to
    ///
    /// # Returns
    /// The TxId (timestamp) assigned to these facts
    pub fn transact(
        &self,
        fact_tuples: Vec<(EntityId, Attribute, Value)>,
        opts: Option<TransactOptions>,
    ) -> Result<TxId> {
        let tx_id = tx_id_now();
        let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let opts = opts.unwrap_or_default();

        let facts: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| {
                let valid_from = opts.valid_from.unwrap_or(tx_id as i64);
                let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
                Fact::with_valid_time(
                    entity, attribute, value, tx_id, tx_count, valid_from, valid_to,
                )
            })
            .collect();

        let mut d = self.data.write().unwrap();
        let mut slot = d.facts.len() as u16;
        for fact in &facts {
            d.pending_indexes.insert(
                fact,
                FactRef {
                    page_id: 0,
                    slot_index: slot,
                },
            );
            slot += 1;
        }
        d.facts.extend(facts);

        Ok(tx_id)
    }

    /// Retract a batch of facts with automatic timestamping
    ///
    /// Retractions are new facts with asserted=false. The original facts remain
    /// in the log for history tracking.
    ///
    /// # Arguments
    /// * `fact_tuples` - Vec of (EntityId, Attribute, Value) tuples to retract
    ///
    /// # Returns
    /// The TxId (timestamp) assigned to these retractions
    pub fn retract(&self, fact_tuples: Vec<(EntityId, Attribute, Value)>) -> Result<TxId> {
        let tx_id = tx_id_now();
        let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;

        let retractions: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| {
                let mut f = Fact::retract(entity, attribute, value, tx_id);
                f.tx_count = tx_count;
                f
            })
            .collect();

        let mut d = self.data.write().unwrap();
        let mut slot = d.facts.len() as u16;
        for fact in &retractions {
            d.pending_indexes.insert(
                fact,
                FactRef {
                    page_id: 0,
                    slot_index: slot,
                },
            );
            slot += 1;
        }
        d.facts.extend(retractions);

        Ok(tx_id)
    }

    /// Insert a fact with its original tx_id and tx_count preserved.
    ///
    /// Used by the load and migration paths only — bypasses tx_counter entirely.
    /// After loading all facts, call `restore_tx_counter()` to re-synchronise the
    /// counter so subsequent `transact()` calls get correct tx_count values.
    pub fn load_fact(&self, fact: Fact) -> Result<()> {
        let mut d = self.data.write().unwrap();
        let slot = d.facts.len() as u16;
        d.pending_indexes.insert(
            &fact,
            FactRef {
                page_id: 0,
                slot_index: slot,
            },
        );
        d.facts.push(fact);
        Ok(())
    }

    /// Set tx_counter to max(tx_count) across all loaded facts.
    ///
    /// Must be called after all `load_fact()` calls complete so that the next
    /// `transact()` call picks up from the right sequence number.
    pub fn restore_tx_counter(&self) -> Result<()> {
        let d = self.data.read().unwrap();
        let max = d.facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.tx_counter.store(max, Ordering::SeqCst);
        Ok(())
    }

    /// Return the current value of the monotonic tx counter.
    ///
    /// Useful for persisting `last_checkpointed_tx_count` into the file header.
    pub fn current_tx_count(&self) -> u64 {
        self.tx_counter.load(Ordering::SeqCst)
    }

    /// Atomically increment the tx counter and return the new value.
    ///
    /// Used by explicit transactions to claim a tx_count at commit time,
    /// without creating any facts in FactStorage.
    pub fn allocate_tx_count(&self) -> u64 {
        self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Return all facts visible as of the given transaction point.
    ///
    /// * `AsOf::Counter(n)` — include facts whose `tx_count <= n`
    /// * `AsOf::Timestamp(t)` — include facts whose `tx_id <= t as u64`
    pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        let filtered = all
            .into_iter()
            .filter(|f| match as_of {
                AsOf::Counter(n) => f.tx_count <= *n,
                AsOf::Timestamp(t) => f.tx_id <= *t as u64,
            })
            .collect();
        Ok(filtered)
    }

    /// Return all asserted facts valid at the given timestamp.
    ///
    /// A fact is valid at `ts` when `valid_from <= ts < valid_to` and it is asserted.
    pub fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        let filtered = all
            .into_iter()
            .filter(|f| f.is_asserted() && f.valid_from <= ts && ts < f.valid_to)
            .collect();
        Ok(filtered)
    }

    /// Get all facts (including retractions)
    ///
    /// Returns the complete append-only log. For current state, filter by
    /// asserted=true and take the most recent fact for each (E, A) pair.
    /// Includes both committed (on-disk) facts and pending (in-memory) facts.
    pub fn get_all_facts(&self) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        let mut all = Vec::new();
        // Committed facts first (on disk, via CommittedFactReader)
        if let Some(loader) = &d.committed {
            all.extend(loader.stream_all()?);
        }
        // Then pending facts (post-checkpoint, in memory)
        all.extend(d.facts.iter().cloned());
        Ok(all)
    }

    /// Get all asserted facts (filters out retractions)
    ///
    /// Returns only facts where asserted=true. This gives you the currently
    /// valid facts, but includes all historical versions.
    pub fn get_asserted_facts(&self) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        Ok(all.into_iter().filter(|f| f.is_asserted()).collect())
    }

    /// Get all facts for a specific entity
    ///
    /// Uses the EAVT index to find facts by entity, resolving FactRefs
    /// via the CommittedFactReader (for on-disk facts) or the pending
    /// facts vector (for in-memory facts).
    ///
    /// # Arguments
    /// * `entity_id` - The entity to query
    ///
    /// # Returns
    /// All facts (assertions and retractions) about this entity
    pub fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
        use crate::storage::index::EavtKey;
        let d = self.data.read().unwrap();

        let start = EavtKey {
            entity: *entity_id,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let next_entity = uuid::Uuid::from_u128(entity_id.as_u128().wrapping_add(1));
        let end = EavtKey {
            entity: next_entity,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };

        // Fallback: no indexes built yet
        if d.pending_indexes.eavt.is_empty() && d.committed_index_reader.is_none() {
            if d.committed.is_none() {
                return Ok(d
                    .facts
                    .iter()
                    .filter(|f| &f.entity == entity_id)
                    .cloned()
                    .collect());
            }
            let mut result: Vec<Fact> = d
                .facts
                .iter()
                .filter(|f| &f.entity == entity_id)
                .cloned()
                .collect();
            if let Some(loader) = &d.committed {
                for fact in loader.stream_all()? {
                    if &fact.entity == entity_id {
                        result.push(fact);
                    }
                }
            }
            return Ok(result);
        }

        let mut facts = Vec::new();

        // Pending: in-memory BTreeMap bounded range.
        // The `end` key uses wrapping_add(1) on entity_id, which wraps to nil at u128::MAX —
        // an astronomically rare edge case where the range becomes empty. The entity check
        // below is a safety net for that edge case; it's redundant for all normal UUIDs.
        for (key, &fr) in d.pending_indexes.eavt.range(start.clone()..end.clone()) {
            if key.entity != *entity_id {
                break;
            }
            facts.push(resolve_fact_ref(&d, fr)?);
        }

        // Committed: on-disk B+tree range scan
        if let Some(reader) = &d.committed_index_reader {
            let committed_refs = reader.range_scan_eavt(&start, Some(&end))?;
            for fr in committed_refs {
                facts.push(resolve_fact_ref(&d, fr)?);
            }
        }

        Ok(facts)
    }

    /// Get all facts for a specific attribute
    ///
    /// Uses the AEVT index when populated (index-driven path), otherwise falls
    /// back to a full scan via `get_all_facts()` for backwards compatibility.
    ///
    /// # Arguments
    /// * `attribute` - The attribute to query (e.g., ":person/name")
    ///
    /// # Returns
    /// All facts with this attribute
    pub fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
        use crate::storage::index::AevtKey;
        let d = self.data.read().unwrap();

        // Fallback: no index
        if d.pending_indexes.aevt.is_empty() && d.committed_index_reader.is_none() {
            drop(d);
            return Ok(self
                .get_all_facts()?
                .into_iter()
                .filter(|f| &f.attribute == attribute)
                .collect());
        }

        let start = AevtKey {
            attribute: attribute.clone(),
            entity: uuid::Uuid::nil(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let end_opt: Option<AevtKey> = next_string_prefix(attribute).map(|next_attr| AevtKey {
            attribute: next_attr,
            entity: uuid::Uuid::nil(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        });

        let mut facts = Vec::new();

        // Pending
        let pending_range: Vec<FactRef> = match &end_opt {
            Some(end) => d
                .pending_indexes
                .aevt
                .range(start.clone()..end.clone())
                .filter(|(k, _)| k.attribute == *attribute)
                .map(|(_, &r)| r)
                .collect(),
            None => d
                .pending_indexes
                .aevt
                .range(start.clone()..)
                .take_while(|(k, _)| k.attribute == *attribute)
                .map(|(_, &r)| r)
                .collect(),
        };
        for fr in pending_range {
            facts.push(resolve_fact_ref(&d, fr)?);
        }

        // Committed
        if let Some(reader) = &d.committed_index_reader {
            let committed_refs = reader.range_scan_aevt(&start, end_opt.as_ref())?;
            for fr in committed_refs {
                let fact = resolve_fact_ref(&d, fr)?;
                if &fact.attribute == attribute {
                    facts.push(fact);
                }
            }
        }

        Ok(facts)
    }

    /// Get all facts for a specific entity and attribute
    ///
    /// # Arguments
    /// * `entity_id` - The entity to query
    /// * `attribute` - The attribute to query
    ///
    /// # Returns
    /// All facts (including history) for this entity-attribute pair
    pub fn get_facts_by_entity_attribute(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        Ok(all
            .into_iter()
            .filter(|f| &f.entity == entity_id && &f.attribute == attribute)
            .collect())
    }

    /// Get the current value for an entity-attribute pair
    ///
    /// Returns the most recent asserted (non-retracted) value, or None if
    /// the attribute was retracted or never existed.
    ///
    /// # Arguments
    /// * `entity_id` - The entity to query
    /// * `attribute` - The attribute to query
    ///
    /// # Returns
    /// The current value, or None if retracted or not found
    pub fn get_current_value(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Option<Value>> {
        let relevant_facts = self.get_facts_by_entity_attribute(entity_id, attribute)?;
        // net_asserted_facts groups by (entity, attribute, value) triple and keeps
        // only triples whose most-recent record (by tx_count) is an assertion.
        // Sort the survivors by tx_count descending to return the latest value.
        let mut net = net_asserted_facts(relevant_facts);
        net.sort_by(|a, b| b.tx_count.cmp(&a.tx_count));
        Ok(net.first().map(|f| f.value.clone()))
    }

    /// Get the count of all facts in storage (committed + pending).
    pub fn fact_count(&self) -> usize {
        let d = self.data.read().unwrap();
        let committed_count = d
            .committed
            .as_ref()
            .and_then(|l| l.stream_all().ok())
            .map(|v| v.len())
            .unwrap_or(0);
        committed_count + d.facts.len()
    }

    /// Get the count of currently asserted facts
    pub fn asserted_fact_count(&self) -> usize {
        self.get_asserted_facts().map(|v| v.len()).unwrap_or(0)
    }

    /// Clear all facts (for testing)
    pub fn clear(&self) -> Result<()> {
        let mut d = self.data.write().unwrap();
        d.facts.clear();
        d.pending_indexes = Indexes::new();
        d.committed = None;
        d.committed_index_reader = None;
        self.tx_counter.store(0, Ordering::SeqCst);
        Ok(())
    }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len) for testing.
    pub fn index_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.data.read().unwrap();
        (
            d.pending_indexes.eavt.len(),
            d.pending_indexes.aevt.len(),
            d.pending_indexes.avet.len(),
            d.pending_indexes.vaet.len(),
        )
    }

    /// Replace the pending in-memory indexes with a freshly rebuilt set.
    ///
    /// Used by `PersistentFactStorage` after detecting an index checksum
    /// mismatch (e.g. after crash recovery).
    pub fn replace_pending_indexes(&self, indexes: Indexes) {
        let mut d = self.data.write().unwrap();
        d.pending_indexes = indexes;
    }

    /// Return the pending (uncommitted) facts held in memory.
    pub fn get_pending_facts(&self) -> Vec<Fact> {
        let d = self.data.read().unwrap();
        d.facts.clone()
    }

    /// Clear pending facts and pending indexes after a successful checkpoint.
    pub fn post_checkpoint_clear(&self) {
        let mut d = self.data.write().unwrap();
        d.facts.clear();
        d.pending_indexes = Indexes::new();
    }

    /// Set the tx_counter to `max` (used on load to restore from persisted state).
    pub fn restore_tx_counter_from(&self, max: u64) {
        self.tx_counter.store(max, Ordering::SeqCst);
    }

    /// Return a snapshot (clone) of the current pending in-memory indexes.
    ///
    /// Used by `PersistentFactStorage::save()` to write index B+tree pages.
    /// Clones the BTreeMaps — acceptable since `save()` is not on the hot path.
    pub fn pending_indexes_snapshot(&self) -> Indexes {
        let d = self.data.read().unwrap();
        Indexes {
            eavt: d.pending_indexes.eavt.clone(),
            aevt: d.pending_indexes.aevt.clone(),
            avet: d.pending_indexes.avet.clone(),
            vaet: d.pending_indexes.vaet.clone(),
        }
    }

    /// Set the committed fact reader. Called by PersistentFactStorage::load() after
    /// opening a v5 file so index-driven reads can resolve FactRefs via page cache.
    pub fn set_committed_reader(&self, reader: Arc<dyn crate::storage::CommittedFactReader>) {
        let mut d = self.data.write().unwrap();
        d.committed = Some(reader);
    }

    /// Set the committed index reader. Called by PersistentFactStorage after
    /// each open/migration/checkpoint so queries can range-scan the on-disk B+tree.
    pub fn set_committed_index_reader(
        &self,
        reader: Arc<dyn crate::storage::CommittedIndexReader>,
    ) {
        let mut d = self.data.write().unwrap();
        d.committed_index_reader = Some(reader);
    }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len) for the pending indexes.
    /// Used in tests to verify pending index state.
    pub fn pending_index_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.data.read().unwrap();
        (
            d.pending_indexes.eavt.len(),
            d.pending_indexes.aevt.len(),
            d.pending_indexes.avet.len(),
            d.pending_indexes.vaet.len(),
        )
    }
}

/// Increment the last byte of a string for prefix upper-bound construction.
/// Returns `None` if all bytes are 0xFF (true unbounded scan needed).
fn next_string_prefix(s: &str) -> Option<String> {
    let mut bytes = s.as_bytes().to_vec();
    for i in (0..bytes.len()).rev() {
        if bytes[i] < 0xFF {
            bytes[i] += 1;
            bytes.truncate(i + 1);
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

/// Compute the net-asserted view of a fact set.
///
/// For each unique `(entity, attribute, value)` triple, keeps the fact only if
/// the record with the highest `tx_count` for that triple has `asserted=true`.
/// Facts whose most recent record is a retraction are excluded entirely.
///
/// Uses [`encode_value`] for the value key to handle floating-point edge cases
/// (NaN canonicalisation, ±0.0 disambiguation) consistently with the rest of
/// the storage layer.
///
/// This is the single source of truth for retraction semantics, shared by
/// `get_current_value` and `filter_facts_for_query`.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use std::collections::HashMap;
    // key: (entity, attribute, canonical value bytes) → fact with highest tx_count
    let mut latest: HashMap<(EntityId, Attribute, Vec<u8>), Fact> = HashMap::new();
    for fact in facts {
        let key = (
            fact.entity,
            fact.attribute.clone(),
            encode_value(&fact.value),
        );
        match latest.get(&key) {
            None => {
                latest.insert(key, fact);
            }
            Some(existing) if fact.tx_count > existing.tx_count => {
                latest.insert(key, fact);
            }
            _ => {}
        }
    }
    latest.into_values().filter(|f| f.asserted).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fact_storage_transact() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Transact facts
        let tx_id = storage
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

        // Verify facts were stored
        assert_eq!(storage.fact_count(), 2);
        assert_eq!(storage.asserted_fact_count(), 2);

        // Verify all facts have same tx_id
        let facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().all(|f| f.tx_id == tx_id));
        assert!(facts.iter().all(|f| f.is_asserted()));
    }

    #[test]
    fn test_fact_storage_retract() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Assert a fact
        let _tx1 = storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        // Retract the fact
        let tx2 = storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            )])
            .unwrap();

        // Both facts exist in storage (assertion + retraction)
        assert_eq!(storage.fact_count(), 2);
        // But only 1 is asserted
        assert_eq!(storage.asserted_fact_count(), 1);

        let facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 2);

        // Find the retraction
        let retraction = facts.iter().find(|f| f.tx_id == tx2).unwrap();
        assert!(retraction.is_retracted());
    }

    #[test]
    fn test_fact_storage_get_by_entity() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        storage
            .transact(
                vec![
                    (
                        alice,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (
                        bob,
                        ":person/name".to_string(),
                        Value::String("Bob".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        let alice_facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(alice_facts.len(), 1);
        assert_eq!(alice_facts[0].value, Value::String("Alice".to_string()));

        let bob_facts = storage.get_facts_by_entity(&bob).unwrap();
        assert_eq!(bob_facts.len(), 1);
        assert_eq!(bob_facts[0].value, Value::String("Bob".to_string()));
    }

    #[test]
    fn test_fact_storage_get_by_attribute() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        storage
            .transact(
                vec![
                    (
                        alice,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice, ":person/age".to_string(), Value::Integer(30)),
                    (
                        bob,
                        ":person/name".to_string(),
                        Value::String("Bob".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        // Get all :person/name facts
        let name_facts = storage
            .get_facts_by_attribute(&":person/name".to_string())
            .unwrap();
        assert_eq!(name_facts.len(), 2);

        // Get all :person/age facts
        let age_facts = storage
            .get_facts_by_attribute(&":person/age".to_string())
            .unwrap();
        assert_eq!(age_facts.len(), 1);
    }

    #[test]
    fn test_fact_storage_get_current_value() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Set initial value
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        // Update value
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice Smith".to_string()),
                )],
                None,
            )
            .unwrap();

        // Current value should be the most recent
        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, Some(Value::String("Alice Smith".to_string())));

        // Retract "Alice Smith" specifically
        storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice Smith".to_string()),
            )])
            .unwrap();

        // "Alice Smith" was retracted, but "Alice" is still asserted (value-level
        // retraction semantics: each distinct value is tracked independently).
        // get_current_value returns the highest-tx_count surviving asserted fact.
        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, Some(Value::String("Alice".to_string())));

        // Now retract "Alice" as well — the attribute should have no asserted value.
        storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            )])
            .unwrap();

        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, None);
    }

    #[test]
    fn test_fact_storage_entity_references() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // Alice is friends with Bob (using Ref)
        storage
            .transact(
                vec![
                    (
                        alice,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice, ":friend".to_string(), Value::Ref(bob)),
                    (
                        bob,
                        ":person/name".to_string(),
                        Value::String("Bob".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        // Get friendship
        let friendship_facts = storage
            .get_facts_by_entity_attribute(&alice, &":friend".to_string())
            .unwrap();
        assert_eq!(friendship_facts.len(), 1);
        assert_eq!(friendship_facts[0].value.as_ref(), Some(bob));
    }

    #[test]
    fn test_fact_storage_history_tracking() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Create multiple versions over time
        let tx1 = storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(30))],
                None,
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx2 = storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(31))],
                None,
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx3 = storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(32))],
                None,
            )
            .unwrap();

        // All versions are in history
        let history = storage
            .get_facts_by_entity_attribute(&alice, &":person/age".to_string())
            .unwrap();
        assert_eq!(history.len(), 3);

        // TxIds should be increasing (chronological)
        assert!(tx1 < tx2);
        assert!(tx2 < tx3);

        // Current value should be most recent
        let current = storage
            .get_current_value(&alice, &":person/age".to_string())
            .unwrap();
        assert_eq!(current, Some(Value::Integer(32)));
    }

    #[test]
    fn test_fact_storage_batch_transact() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Transact multiple facts at once
        let tx_id = storage
            .transact(
                vec![
                    (
                        alice,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice, ":person/age".to_string(), Value::Integer(30)),
                    (
                        alice,
                        ":person/email".to_string(),
                        Value::String("alice@example.com".to_string()),
                    ),
                ],
                None,
            )
            .unwrap();

        // All facts should have same tx_id (atomic batch)
        let facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 3);
        assert!(facts.iter().all(|f| f.tx_id == tx_id));
    }

    // =========================================================================
    // Phase 4: tx_counter, load_fact, temporal query tests
    // =========================================================================

    #[test]
    fn test_tx_count_increments_per_call() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(30))],
                None,
            )
            .unwrap();

        let facts = storage.get_all_facts().unwrap();
        let name_fact = facts
            .iter()
            .find(|f| f.attribute == ":person/name")
            .unwrap();
        let age_fact = facts.iter().find(|f| f.attribute == ":person/age").unwrap();

        assert_eq!(name_fact.tx_count, 1);
        assert_eq!(age_fact.tx_count, 2);
    }

    #[test]
    fn test_batch_facts_share_tx_count() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        storage
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

        let facts = storage.get_all_facts().unwrap();
        assert!(facts.iter().all(|f| f.tx_count == 1));
    }

    #[test]
    fn test_load_fact_preserves_tx_id_and_tx_count() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let entity = Uuid::new_v4();

        let original_fact = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            12345_u64, // original tx_id
            7,         // original tx_count
            12345_i64,
            VALID_TIME_FOREVER,
        );

        storage.load_fact(original_fact.clone()).unwrap();

        let facts = storage.get_all_facts().unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].tx_id, 12345);
        assert_eq!(facts[0].tx_count, 7);
    }

    #[test]
    fn test_get_facts_as_of_counter() {
        use crate::query::datalog::types::AsOf;
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // tx_count = 1
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        // tx_count = 2
        storage
            .transact(
                vec![(alice, ":person/age".to_string(), Value::Integer(30))],
                None,
            )
            .unwrap();

        // as-of tx 1: only name fact visible
        let snapshot = storage.get_facts_as_of(&AsOf::Counter(1)).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].attribute, ":person/name");
    }

    #[test]
    fn test_get_facts_valid_at() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        let opts = TransactOptions::new(
            Some(1672531200000_i64), // 2023-01-01
            Some(1685577600000_i64), // 2023-06-01
        );

        storage
            .transact(
                vec![(
                    alice,
                    ":employment/status".to_string(),
                    Value::Keyword(":active".to_string()),
                )],
                Some(opts),
            )
            .unwrap();

        // Valid on 2023-03-01 (inside range)
        let inside = storage.get_facts_valid_at(1677628800000_i64).unwrap();
        assert_eq!(inside.len(), 1);

        // Valid on 2024-01-01 (outside range)
        let outside = storage.get_facts_valid_at(1704067200000_i64).unwrap();
        assert_eq!(outside.len(), 0);
    }

    #[test]
    fn test_tx_counter_restored_after_load_fact() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let entity = Uuid::new_v4();

        // Load a fact with tx_count = 5 (simulating migration/load)
        let fact = Fact::with_valid_time(
            entity,
            ":a".to_string(),
            Value::Integer(1),
            1000,
            5,
            1000_i64,
            VALID_TIME_FOREVER,
        );
        storage.load_fact(fact).unwrap();
        storage.restore_tx_counter().unwrap();

        // Next transact should get tx_count = 6
        storage
            .transact(vec![(entity, ":b".to_string(), Value::Integer(2))], None)
            .unwrap();

        let facts = storage.get_all_facts().unwrap();
        let b_fact = facts.iter().find(|f| f.attribute == ":b").unwrap();
        assert_eq!(b_fact.tx_count, 6);
    }

    // =========================================================================
    // Phase 5: current_tx_count, allocate_tx_count helpers
    // =========================================================================

    #[test]
    fn test_current_tx_count_starts_at_zero() {
        let storage = FactStorage::new();
        assert_eq!(storage.current_tx_count(), 0);
    }

    #[test]
    fn test_current_tx_count_reflects_transacts() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice,
                    ":name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();
        assert_eq!(storage.current_tx_count(), 1);
        storage
            .transact(vec![(alice, ":age".to_string(), Value::Integer(30))], None)
            .unwrap();
        assert_eq!(storage.current_tx_count(), 2);
    }

    #[test]
    fn test_allocate_tx_count_increments() {
        let storage = FactStorage::new();
        let c1 = storage.allocate_tx_count();
        let c2 = storage.allocate_tx_count();
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
        assert_eq!(storage.current_tx_count(), 2);
    }

    // =========================================================================
    // Phase 6.1: index population tests
    // =========================================================================

    #[test]
    fn test_indexes_populated_on_transact() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        storage
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
        let (eavt, aevt, avet, vaet) = storage.index_counts();
        assert_eq!(eavt, 2);
        assert_eq!(aevt, 2);
        assert_eq!(avet, 2);
        assert_eq!(vaet, 1, "Only Ref values go into VAET");
    }

    #[test]
    fn test_slot_index_is_zero_in_6_1() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let e = Uuid::new_v4();
        storage
            .transact(vec![(e, ":x".to_string(), Value::Integer(1))], None)
            .unwrap();
        let (eavt, _, _, _) = storage.index_counts();
        assert_eq!(eavt, 1);
    }

    #[test]
    fn test_load_fact_populates_indexes() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let e = Uuid::new_v4();
        let fact = crate::graph::types::Fact::with_valid_time(
            e,
            ":name".to_string(),
            Value::String("Test".to_string()),
            0,
            1,
            0,
            crate::graph::types::VALID_TIME_FOREVER,
        );
        storage.load_fact(fact).unwrap();
        storage.restore_tx_counter().unwrap();
        let (eavt, _, _, _) = storage.index_counts();
        assert_eq!(eavt, 1);
    }

    // =========================================================================
    // Phase 6.2: CommittedFactReader integration tests
    // =========================================================================

    #[test]
    fn test_committed_reader_resolves_facts() {
        use crate::storage::CommittedFactReader;
        use crate::storage::index::{FactRef, Indexes};
        use std::sync::Arc;
        use uuid::Uuid;

        /// Mock loader: resolves FactRefs by slot_index into a fixed Vec<Fact>.
        struct MockLoader {
            facts: Vec<Fact>,
        }
        impl CommittedFactReader for MockLoader {
            fn resolve(&self, fr: FactRef) -> anyhow::Result<Fact> {
                self.facts
                    .get(fr.slot_index as usize)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("MockLoader: no fact at slot {}", fr.slot_index))
            }
            fn stream_all(&self) -> anyhow::Result<Vec<Fact>> {
                Ok(self.facts.clone())
            }
            fn committed_page_count(&self) -> u64 {
                1
            }
        }

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let committed_fact = Fact::with_valid_time(
            alice,
            ":name".to_string(),
            Value::String("Alice".to_string()),
            0,
            1,
            0,
            VALID_TIME_FOREVER,
        );
        let loader = Arc::new(MockLoader {
            facts: vec![committed_fact.clone()],
        });

        // Insert a committed FactRef into the indexes (page_id > 0 → committed path).
        let mut indexes = Indexes::new();
        indexes.insert(
            &committed_fact,
            FactRef {
                page_id: 1,
                slot_index: 0,
            },
        );
        storage.replace_pending_indexes(indexes);
        storage.set_committed_reader(loader);

        // get_facts_by_entity must resolve via CommittedFactReader (EAVT range scan).
        let entity_facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(
            entity_facts.len(),
            1,
            "EAVT range scan should resolve committed fact"
        );
        assert_eq!(entity_facts[0].entity, alice);
        assert_eq!(entity_facts[0].attribute, ":name");

        // get_facts_by_attribute must resolve via CommittedFactReader (AEVT range scan).
        let attr_facts = storage
            .get_facts_by_attribute(&":name".to_string())
            .unwrap();
        assert_eq!(
            attr_facts.len(),
            1,
            "AEVT range scan should resolve committed fact"
        );
        assert_eq!(attr_facts[0].value, Value::String("Alice".to_string()));

        // get_all_facts must include committed facts via stream_all().
        let all = storage.get_all_facts().unwrap();
        assert_eq!(all.len(), 1, "get_all_facts must include committed facts");
        assert_eq!(all[0].entity, alice);

        // get_facts_as_of should see committed facts.
        let as_of = storage
            .get_facts_as_of(&crate::query::datalog::types::AsOf::Counter(10))
            .unwrap();
        assert_eq!(
            as_of.len(),
            1,
            "get_facts_as_of should include committed facts"
        );

        // get_facts_valid_at should see committed facts valid at time 0.
        let valid_at = storage.get_facts_valid_at(0).unwrap();
        assert_eq!(
            valid_at.len(),
            1,
            "get_facts_valid_at should include committed facts"
        );
    }

    #[test]
    fn test_committed_reader_combined_with_pending() {
        use crate::storage::CommittedFactReader;
        use crate::storage::index::{FactRef, Indexes};
        use std::sync::Arc;
        use uuid::Uuid;

        struct MockLoader {
            facts: Vec<Fact>,
        }
        impl CommittedFactReader for MockLoader {
            fn resolve(&self, fr: FactRef) -> anyhow::Result<Fact> {
                self.facts
                    .get(fr.slot_index as usize)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("slot {} not found", fr.slot_index))
            }
            fn stream_all(&self) -> anyhow::Result<Vec<Fact>> {
                Ok(self.facts.clone())
            }
            fn committed_page_count(&self) -> u64 {
                1
            }
        }

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // One committed fact (Alice, on disk)
        let alice_fact = Fact::with_valid_time(
            alice,
            ":name".to_string(),
            Value::String("Alice".to_string()),
            1000,
            1,
            1000,
            VALID_TIME_FOREVER,
        );
        let loader = Arc::new(MockLoader {
            facts: vec![alice_fact.clone()],
        });
        let mut indexes = Indexes::new();
        indexes.insert(
            &alice_fact,
            FactRef {
                page_id: 1,
                slot_index: 0,
            },
        );
        storage.replace_pending_indexes(indexes);
        storage.set_committed_reader(loader);

        // Restore tx_counter so pending transact gets tx_count = 2
        storage.restore_tx_counter_from(1);

        // One pending fact (Bob, in memory)
        storage
            .transact(
                vec![(bob, ":name".to_string(), Value::String("Bob".to_string()))],
                None,
            )
            .unwrap();

        // get_all_facts should see both
        let all = storage.get_all_facts().unwrap();
        assert_eq!(
            all.len(),
            2,
            "Both committed and pending facts must be visible"
        );

        // get_facts_by_attribute uses AEVT — must also see both
        let name_facts = storage
            .get_facts_by_attribute(&":name".to_string())
            .unwrap();
        assert_eq!(
            name_facts.len(),
            2,
            "AEVT scan must return both committed and pending facts"
        );
    }

    #[test]
    fn test_post_checkpoint_clear_clears_indexes() {
        use uuid::Uuid;
        let storage = FactStorage::new();
        let e = Uuid::new_v4();
        storage
            .transact(
                vec![(e, ":name".to_string(), Value::String("Alice".to_string()))],
                None,
            )
            .unwrap();

        assert_eq!(
            storage.pending_index_counts().0,
            1,
            "one pending EAVT entry"
        );
        storage.post_checkpoint_clear();
        assert_eq!(
            storage.pending_index_counts().0,
            0,
            "pending indexes cleared"
        );
        assert_eq!(
            storage.get_pending_facts().len(),
            0,
            "pending facts cleared"
        );
    }

    #[test]
    fn test_set_committed_index_reader_accepted() {
        use crate::storage::CommittedIndexReader;
        use crate::storage::index::{AevtKey, AvetKey, EavtKey, FactRef, VaetKey};
        use std::sync::Arc;

        struct NoopReader;
        impl CommittedIndexReader for NoopReader {
            fn range_scan_eavt(
                &self,
                _: &EavtKey,
                _: Option<&EavtKey>,
            ) -> anyhow::Result<Vec<FactRef>> {
                Ok(vec![])
            }
            fn range_scan_aevt(
                &self,
                _: &AevtKey,
                _: Option<&AevtKey>,
            ) -> anyhow::Result<Vec<FactRef>> {
                Ok(vec![])
            }
            fn range_scan_avet(
                &self,
                _: &AvetKey,
                _: Option<&AvetKey>,
            ) -> anyhow::Result<Vec<FactRef>> {
                Ok(vec![])
            }
            fn range_scan_vaet(
                &self,
                _: &VaetKey,
                _: Option<&VaetKey>,
            ) -> anyhow::Result<Vec<FactRef>> {
                Ok(vec![])
            }
        }

        let storage = FactStorage::new();
        // Verify set_committed_index_reader wires the reader (no panic, usable storage)
        storage.set_committed_index_reader(Arc::new(NoopReader));
        // After setting, get_facts_by_entity should use the index path without panicking
        let result = storage.get_facts_by_entity(&uuid::Uuid::nil());
        assert!(
            result.is_ok(),
            "storage should be usable after setting committed index reader"
        );
        assert_eq!(result.unwrap().len(), 0);
    }
}
