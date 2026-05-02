use crate::graph::types::{
    Attribute, EntityId, Fact, TransactOptions, TxId, VALID_TIME_FOREVER, Value, tx_id_now,
};
use crate::query::datalog::types::AsOf;
use crate::storage::index::{FactRef, Indexes, encode_value};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Compact key for O(1) duplicate detection in `FactData::pending_keys`.
///
/// Mirrors the equality predicate used by `load_fact`:
/// (entity, attribute, encoded_value, valid_from, valid_to, tx_count, asserted).
/// `encode_value` is used for the value field because `Value` contains `f64`
/// and therefore cannot implement `Hash` directly.
type PendingKey = (EntityId, String, Vec<u8>, i64, i64, u64, bool);

fn pending_key(f: &Fact) -> PendingKey {
    (
        f.entity,
        f.attribute.clone(),
        encode_value(&f.value),
        f.valid_from,
        f.valid_to,
        f.tx_count,
        f.asserted,
    )
}

// ============================================================================
// Datalog Fact Storage (Phase 3+)
// ============================================================================

/// Private container that co-locates the fact list and all four indexes under
/// a single `RwLock`. This ensures facts and indexes are always updated together
/// without needing a second lock.
struct FactData {
    facts: Vec<Fact>,
    /// O(1) duplicate-detection set for `load_fact`.
    ///
    /// Maintained in sync with `facts` by every method that appends to `facts`.
    /// Replaces the O(n) linear scan that made `load_fact` O(n²) for large
    /// fact sets (e.g. 1M-fact benchmark setup).
    pending_keys: HashSet<PendingKey>,
    pending_indexes: Indexes,
    /// Resolves committed (on-disk) FactRefs to Fact objects.
    /// None for in-memory databases or before load() is called.
    committed: Option<Arc<dyn crate::storage::CommittedFactReader>>,
    /// Provides bounded range scans over the four committed (on-disk) covering indexes.
    /// Set by `set_committed_index_reader()` after open/migration/checkpoint.
    committed_index_reader: Option<Arc<dyn crate::storage::CommittedIndexReader>>,
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
/// ```ignore
/// use crate::graph::storage::FactStorage;
/// use crate::graph::types::Value;
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
pub(crate) struct FactStorage {
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
    pub(crate) fn new() -> Self {
        FactStorage {
            data: Arc::new(RwLock::new(FactData {
                facts: Vec::new(),
                pending_keys: HashSet::new(),
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
    pub(crate) fn transact(
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
        for (slot, fact) in (d.facts.len() as u16..).zip(facts.iter()) {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(
                fact,
                FactRef {
                    page_id: 0,
                    slot_index: slot,
                },
            );
        }
        d.facts.extend(facts);

        Ok(tx_id)
    }

    /// Transact a batch of facts where each fact may carry its own valid-time opts.
    ///
    /// All facts share **one** `tx_count` (incremented once for the whole batch),
    /// matching the semantics of a single user-level `(transact [...])` command.
    /// Per-fact opts override `default_opts` for that individual fact only.
    ///
    /// # Arguments
    /// * `fact_tuples` - Vec of `(entity, attribute, value, per_fact_opts)`
    /// * `default_opts` - Transaction-level valid-time opts applied when a fact
    ///   has no per-fact override
    ///
    /// # Returns
    /// The TxId (timestamp) assigned to all facts in this batch
    pub(crate) fn transact_batch(
        &self,
        fact_tuples: Vec<(EntityId, Attribute, Value, Option<TransactOptions>)>,
        default_opts: Option<TransactOptions>,
    ) -> Result<TxId> {
        let tx_id = tx_id_now();
        let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let default_opts = default_opts.unwrap_or_default();

        let facts: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value, per_fact_opts)| {
                let opts = per_fact_opts.unwrap_or_else(|| default_opts.clone());
                let valid_from = opts.valid_from.unwrap_or(tx_id as i64);
                let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
                Fact::with_valid_time(
                    entity, attribute, value, tx_id, tx_count, valid_from, valid_to,
                )
            })
            .collect();

        let mut d = self.data.write().unwrap();
        for (slot, fact) in (d.facts.len() as u16..).zip(facts.iter()) {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(
                fact,
                FactRef {
                    page_id: 0,
                    slot_index: slot,
                },
            );
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
    pub(crate) fn retract(&self, fact_tuples: Vec<(EntityId, Attribute, Value)>) -> Result<TxId> {
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
        for (slot, fact) in (d.facts.len() as u16..).zip(retractions.iter()) {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(
                fact,
                FactRef {
                    page_id: 0,
                    slot_index: slot,
                },
            );
        }
        d.facts.extend(retractions);

        Ok(tx_id)
    }

    /// Insert a fact with its original tx_id and tx_count preserved.
    ///
    /// Used by the load and migration paths only — bypasses tx_counter entirely.
    /// After loading all facts, call `restore_tx_counter()` to re-synchronise the
    /// counter so subsequent `transact()` calls get correct tx_count values.
    ///
    /// Checks for duplicate facts before loading (based on entity, attribute, value,
    /// valid_from, valid_to, tx_count, and asserted).
    pub(crate) fn load_fact(&self, fact: Fact) -> Result<bool> {
        let mut d = self.data.write().unwrap();

        // O(1) duplicate check via the pending_keys HashSet.
        // Previously this was an O(n) linear scan over d.facts, causing O(n²)
        // total complexity when loading n facts (e.g. 1M-fact benchmarks).
        let key = pending_key(&fact);
        if !d.pending_keys.insert(key) {
            return Ok(false); // Already exists, not loaded
        }

        let slot = d.facts.len() as u16;
        d.pending_indexes.insert(
            &fact,
            FactRef {
                page_id: 0,
                slot_index: slot,
            },
        );
        d.facts.push(fact);
        Ok(true)
    }

    /// Set tx_counter to max(tx_count) across all loaded facts.
    ///
    /// Must be called after all `load_fact()` calls complete so that the next
    /// `transact()` call picks up from the right sequence number.
    pub(crate) fn restore_tx_counter(&self) -> Result<()> {
        let d = self.data.read().unwrap();
        let max = d.facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.tx_counter.store(max, Ordering::SeqCst);
        Ok(())
    }

    /// Return the current value of the monotonic tx counter.
    ///
    /// Useful for persisting `last_checkpointed_tx_count` into the file header.
    pub(crate) fn current_tx_count(&self) -> u64 {
        self.tx_counter.load(Ordering::SeqCst)
    }

    /// Atomically increment the tx counter and return the new value.
    ///
    /// Used by explicit transactions to claim a tx_count at commit time,
    /// without creating any facts in FactStorage.
    pub(crate) fn allocate_tx_count(&self) -> u64 {
        self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Get all facts (including retractions)
    ///
    /// Returns the complete append-only log. For current state, filter by
    /// asserted=true and take the most recent fact for each (E, A) pair.
    /// Includes both committed (on-disk) facts and pending (in-memory) facts.
    pub(crate) fn get_all_facts(&self) -> Result<Vec<Fact>> {
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

    /// Return all facts visible as of the given transaction point.
    ///
    /// * `AsOf::Counter(n)` — include facts whose `tx_count <= n`
    /// * `AsOf::Timestamp(t)` — include facts whose `tx_id <= t as u64`
    pub(crate) fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        Ok(filter_facts_as_of(all, as_of))
    }

    /// Get all asserted facts (filters out retractions)
    ///
    /// Returns only facts where asserted=true. This gives you the currently
    /// valid facts, but includes all historical versions.
    pub(crate) fn get_asserted_facts(&self) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        Ok(all.into_iter().filter(|f| f.is_asserted()).collect())
    }

    /// Clear all facts (for testing)
    pub(crate) fn clear(&self) -> Result<()> {
        let mut d = self.data.write().unwrap();
        d.facts.clear();
        d.pending_keys.clear();
        d.pending_indexes = Indexes::new();
        d.committed = None;
        d.committed_index_reader = None;
        self.tx_counter.store(0, Ordering::SeqCst);
        Ok(())
    }

    /// Replace the pending in-memory indexes with a freshly rebuilt set.
    ///
    /// Used by `PersistentFactStorage` after detecting an index checksum
    /// mismatch (e.g. after crash recovery).
    #[allow(dead_code)]
    pub(crate) fn replace_pending_indexes(&self, indexes: Indexes) {
        let mut d = self.data.write().unwrap();
        d.pending_indexes = indexes;
    }

    /// Return the pending (uncommitted) facts held in memory.
    pub(crate) fn get_pending_facts(&self) -> Vec<Fact> {
        let d = self.data.read().unwrap();
        d.facts.clone()
    }

    /// Clear pending facts and pending indexes after a successful checkpoint.
    pub(crate) fn post_checkpoint_clear(&self) {
        let mut d = self.data.write().unwrap();
        d.facts.clear();
        d.pending_keys.clear();
        d.pending_indexes = Indexes::new();
    }

    /// Set the tx_counter to `max` (used on load to restore from persisted state).
    pub(crate) fn restore_tx_counter_from(&self, max: u64) {
        self.tx_counter.store(max, Ordering::SeqCst);
    }

    /// Return a snapshot (clone) of the current pending in-memory indexes.
    ///
    /// Used by `PersistentFactStorage::save()` to write index B+tree pages.
    /// Clones the BTreeMaps — acceptable since `save()` is not on the hot path.
    pub(crate) fn pending_indexes_snapshot(&self) -> Indexes {
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
    pub(crate) fn set_committed_reader(
        &self,
        reader: Arc<dyn crate::storage::CommittedFactReader>,
    ) {
        let mut d = self.data.write().unwrap();
        d.committed = Some(reader);
    }

    /// Set the committed index reader. Called by PersistentFactStorage after
    /// each open/migration/checkpoint so queries can range-scan the on-disk B+tree.
    pub(crate) fn set_committed_index_reader(
        &self,
        reader: Arc<dyn crate::storage::CommittedIndexReader>,
    ) {
        let mut d = self.data.write().unwrap();
        d.committed_index_reader = Some(reader);
    }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len) for the pending indexes.
    /// Used in tests to verify pending index state.
    #[allow(dead_code)]
    pub(crate) fn pending_index_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.data.read().unwrap();
        (
            d.pending_indexes.eavt.len(),
            d.pending_indexes.aevt.len(),
            d.pending_indexes.avet.len(),
            d.pending_indexes.vaet.len(),
        )
    }
}

/// Apply transaction-time snapshot semantics to a batch of facts.
///
/// Shared by `FactStorage::get_facts_as_of()` and transactional overlay reads.
pub(crate) fn filter_facts_as_of(facts: Vec<Fact>, as_of: &AsOf) -> Vec<Fact> {
    facts
        .into_iter()
        .filter(|f| match as_of {
            AsOf::Counter(n) => f.tx_count <= *n,
            AsOf::Timestamp(t) => f.tx_id <= *t as u64,
            AsOf::Slot(_) => {
                panic!("internal: unsubstituted :as-of bind slot reached get_facts_as_of");
            }
        })
        .collect()
}

/// Compute the net-asserted view of a fact set.
///
/// For each unique `(entity, attribute, value)` triple:
/// 1. Find the retraction with the highest `tx_count` (if any).
/// 2. Keep all assertions whose `tx_count` is greater than that retraction.
/// 3. Deduplicate surviving assertions by `(valid_from, valid_to)`, keeping the
///    one with the highest `tx_count` for each validity window.
///
/// This allows the same EAV triple to be asserted at multiple non-overlapping
/// valid-time intervals (e.g., salary=$100k valid 2020–2022 AND 2024–2026).
/// A retraction still cancels all prior assertions of that triple, but
/// re-assertions after the retraction are preserved.
///
/// Uses [`encode_value`] for the value key to handle floating-point edge cases
/// (NaN canonicalisation, ±0.0 disambiguation) consistently with the rest of
/// the storage layer.
///
/// This is the single source of truth for retraction semantics, shared by
/// `get_current_value` and `filter_facts_for_query`.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use std::collections::HashMap;

    // Group all facts by (entity, attribute, canonical value bytes).
    let mut groups: HashMap<(EntityId, Attribute, Vec<u8>), Vec<Fact>> = HashMap::new();
    for fact in facts {
        let key = (
            fact.entity,
            fact.attribute.clone(),
            encode_value(&fact.value),
        );
        groups.entry(key).or_default().push(fact);
    }

    let mut result = Vec::new();
    for (_key, group) in groups {
        // Find the highest tx_count among retractions in this group.
        let max_retract_tx = group
            .iter()
            .filter(|f| !f.asserted)
            .map(|f| f.tx_count)
            .max()
            .unwrap_or(0);

        // Keep assertions whose tx_count > max_retract_tx.
        // (If no retraction exists, max_retract_tx is 0 and all assertions pass.)
        // Deduplicate by (valid_from, valid_to): keep the highest-tx_count assertion
        // for each validity window so that re-asserting the same EAV at the same
        // window acts as an update rather than a duplicate.
        let mut by_window: HashMap<(i64, i64), Fact> = HashMap::new();
        for fact in group {
            if !fact.asserted || fact.tx_count <= max_retract_tx {
                continue;
            }
            let window = (fact.valid_from, fact.valid_to);
            match by_window.get(&window) {
                None => {
                    by_window.insert(window, fact);
                }
                Some(existing) if fact.tx_count > existing.tx_count => {
                    by_window.insert(window, fact);
                }
                _ => {}
            }
        }
        result.extend(by_window.into_values());
    }
    result
}

/// Resolve a FactRef to a Fact using the committed reader (for on-disk facts)
/// or the pending facts vector (for in-memory facts with page_id=0).
/// Only used from test-only FactStorage methods and the tests module.
#[cfg(test)]
fn resolve_fact_ref(d: &FactData, fr: FactRef) -> Result<Fact> {
    if fr.page_id == 0 {
        d.facts
            .get(fr.slot_index as usize)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("pending fact index {} out of bounds", fr.slot_index))
    } else {
        match &d.committed {
            Some(loader) => loader.resolve(fr),
            None => anyhow::bail!(
                "no CommittedFactReader but got committed FactRef (page_id={})",
                fr.page_id
            ),
        }
    }
}

/// Increment the last byte of a string for prefix upper-bound construction.
/// Returns `None` if all bytes are 0xFF (true unbounded scan needed).
/// Only used from test-only FactStorage methods.
#[cfg(test)]
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

/// Test-only helpers on FactStorage: index-driven entity/attribute queries.
///
/// These methods are only used from tests; they are not part of the production
/// query path (the executor uses `get_asserted_facts` + temporal filtering).
#[cfg(test)]
impl FactStorage {
    /// Return all asserted facts valid at the given timestamp.
    ///
    /// A fact is valid at `ts` when `valid_from <= ts < valid_to` and it is asserted.
    pub(crate) fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        let filtered = all
            .into_iter()
            .filter(|f| f.is_asserted() && f.valid_from <= ts && ts < f.valid_to)
            .collect();
        Ok(filtered)
    }

    /// Get all facts for a specific entity (index-driven, test use only).
    pub(crate) fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
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

    /// Get all facts for a specific attribute (index-driven, test use only).
    pub(crate) fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
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

    /// Get all facts for a specific entity and attribute (test use only).
    pub(crate) fn get_facts_by_entity_attribute(
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

    /// Get the current value for an entity-attribute pair (test use only).
    pub(crate) fn get_current_value(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Option<Value>> {
        let relevant_facts = self.get_facts_by_entity_attribute(entity_id, attribute)?;
        let mut net = net_asserted_facts(relevant_facts);
        net.sort_by(|a, b| b.tx_count.cmp(&a.tx_count));
        Ok(net.first().map(|f| f.value.clone()))
    }

    /// Get the count of all facts in storage (committed + pending). Test use only.
    pub(crate) fn fact_count(&self) -> usize {
        let d = self.data.read().unwrap();
        let committed_count = d
            .committed
            .as_ref()
            .and_then(|l| l.stream_all().ok())
            .map(|v| v.len())
            .unwrap_or(0);
        committed_count + d.facts.len()
    }

    /// Get the count of currently asserted facts. Test use only.
    pub(crate) fn asserted_fact_count(&self) -> usize {
        self.get_asserted_facts().map(|v| v.len()).unwrap_or(0)
    }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len). Test use only.
    pub(crate) fn index_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.data.read().unwrap();
        (
            d.pending_indexes.eavt.len(),
            d.pending_indexes.aevt.len(),
            d.pending_indexes.avet.len(),
            d.pending_indexes.vaet.len(),
        )
    }
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

    #[test]
    fn test_load_fact_prevents_duplicates() {
        use crate::graph::types::Value;

        let storage = FactStorage::new();

        let entity = uuid::Uuid::new_v4();
        let attr = ":test/attr".to_string();
        let value = Value::Integer(42);

        let fact1 = Fact::new(entity, attr.clone(), value.clone(), 1);
        let fact1_key = (entity, attr.clone(), value.clone());

        let fact2 = Fact::new(uuid::Uuid::new_v4(), attr.clone(), value.clone(), 1);

        // Different entities - should load both
        assert!(storage.load_fact(fact1).unwrap());
        assert!(storage.load_fact(fact2).unwrap());

        let count = storage.fact_count();
        assert_eq!(count, 2);

        // Try loading the exact same fact again - should be rejected as duplicate
        let fact1_dup = Fact::new(fact1_key.0, fact1_key.1, fact1_key.2, 1);
        assert!(!storage.load_fact(fact1_dup).unwrap());

        // Count should remain the same
        assert_eq!(storage.fact_count(), 2);
    }

    #[test]
    fn test_load_fact_duplicate_detection_includes_asserted() {
        let storage = FactStorage::new();
        let entity = uuid::Uuid::new_v4();
        let attr = ":test/attr".to_string();
        let value = Value::Integer(42);

        // Load an asserted fact
        let mut fact1 = Fact::new(entity, attr.clone(), value.clone(), 1);
        fact1.asserted = true;
        assert!(storage.load_fact(fact1).unwrap());

        // Load a retraction for the same entity/attr/value/tx_count but different asserted
        let mut fact2 = Fact::new(entity, attr.clone(), value.clone(), 1);
        fact2.asserted = false;
        // Should NOT be deduplicated - different asserted values should both survive
        assert!(storage.load_fact(fact2).unwrap());

        // Both facts should be present
        assert_eq!(storage.fact_count(), 2);
    }
}
