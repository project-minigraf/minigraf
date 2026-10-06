use crate::error::{ErrorCode, err_coded};
use crate::graph::types::{
    Attribute, EntityId, Fact, TransactOptions, TxId, VALID_TIME_FOREVER, Value, tx_id_now,
};
use crate::query::datalog::types::AsOf;
use crate::storage::index::{Indexes, encode_value};
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
    /// Reads committed (checkpointed) facts through the on-disk covering indexes.
    /// None for in-memory databases. Set after every open, migration and checkpoint.
    committed: Option<Arc<dyn crate::storage::CommittedReader>>,
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
        let tx_count = self
            .tx_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let opts = opts.unwrap_or_default();

        let facts: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| {
                let valid_from = opts
                    .valid_from
                    .unwrap_or_else(|| i64::try_from(tx_id).unwrap_or(i64::MAX));
                let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
                Fact::with_valid_time(
                    entity, attribute, value, tx_id, tx_count, valid_from, valid_to,
                )
            })
            .collect();

        let mut d = self
            .data
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;
        let base = d.facts.len();
        for (i, fact) in facts.iter().enumerate() {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(fact, base + i);
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
    /// `(tx_id, tx_count)` — the Unix-ms timestamp and the monotonic counter
    /// assigned to all facts in this batch.
    pub(crate) fn transact_batch(
        &self,
        fact_tuples: Vec<(EntityId, Attribute, Value, Option<TransactOptions>)>,
        default_opts: Option<TransactOptions>,
    ) -> Result<(TxId, u64)> {
        let tx_id = tx_id_now();
        let tx_count = self
            .tx_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let default_opts = default_opts.unwrap_or_default();

        let facts: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value, per_fact_opts)| {
                let opts = per_fact_opts.unwrap_or_else(|| default_opts.clone());
                let valid_from = opts
                    .valid_from
                    .unwrap_or_else(|| i64::try_from(tx_id).unwrap_or(i64::MAX));
                let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
                Fact::with_valid_time(
                    entity, attribute, value, tx_id, tx_count, valid_from, valid_to,
                )
            })
            .collect();

        let mut d = self
            .data
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;
        let base = d.facts.len();
        for (i, fact) in facts.iter().enumerate() {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(fact, base + i);
        }
        d.facts.extend(facts);

        Ok((tx_id, tx_count))
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
    /// `(tx_id, tx_count)` — the Unix-ms timestamp and the monotonic counter
    /// assigned to these retractions.
    pub(crate) fn retract(
        &self,
        fact_tuples: Vec<(EntityId, Attribute, Value)>,
    ) -> Result<(TxId, u64)> {
        let tx_id = tx_id_now();
        let tx_count = self
            .tx_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);

        let retractions: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| {
                let mut f = Fact::retract(entity, attribute, value, tx_id);
                f.tx_count = tx_count;
                f
            })
            .collect();

        let mut d = self
            .data
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;
        let base = d.facts.len();
        for (i, fact) in retractions.iter().enumerate() {
            d.pending_keys.insert(pending_key(fact));
            d.pending_indexes.insert(fact, base + i);
        }
        d.facts.extend(retractions);

        Ok((tx_id, tx_count))
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
        let mut d = self
            .data
            .write()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;

        // O(1) duplicate check via the pending_keys HashSet.
        // Previously this was an O(n) linear scan over d.facts, causing O(n²)
        // total complexity when loading n facts (e.g. 1M-fact benchmarks).
        let key = pending_key(&fact);
        if !d.pending_keys.insert(key) {
            return Ok(false); // Already exists, not loaded
        }

        let pos = d.facts.len();
        d.pending_indexes.insert(&fact, pos);
        d.facts.push(fact);
        Ok(true)
    }

    /// Raise tx_counter to max(tx_count) across all loaded facts.
    ///
    /// Must be called after all `load_fact()` calls complete so that the next
    /// `transact()` call picks up from the right sequence number. The counter
    /// never moves backwards: it may already hold the file's checkpointed count,
    /// and when WAL replay loads no facts that count must survive (#447).
    pub(crate) fn restore_tx_counter(&self) -> Result<()> {
        let d = self
            .data
            .read()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;
        let max = d.facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.tx_counter.fetch_max(max, Ordering::SeqCst);
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
        self.tx_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1)
    }

    /// Get all facts (including retractions)
    ///
    /// Returns the complete append-only log. For current state, filter by
    /// asserted=true and take the most recent fact for each (E, A) pair.
    /// Includes both committed (on-disk) facts and pending (in-memory) facts.
    pub(crate) fn get_all_facts(&self) -> Result<Vec<Fact>> {
        let d = self
            .data
            .read()
            .map_err(|_| err_coded!(ErrorCode::Int050, "data"))?;
        let mut all = Vec::new();
        // Committed facts first (on disk, in EAVT order)
        if let Some(reader) = &d.committed {
            all.extend(reader.all_facts()?);
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

    /// Return the pending (uncommitted) facts held in memory.
    pub(crate) fn get_pending_facts(&self) -> Vec<Fact> {
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        d.facts.clone()
    }

    /// Clear pending facts and pending indexes after a successful checkpoint.
    pub(crate) fn post_checkpoint_clear(&self) {
        let mut d = self.data.write().unwrap_or_else(|e| e.into_inner());
        d.facts.clear();
        d.pending_keys.clear();
        d.pending_indexes = Indexes::new();
    }

    /// Set the tx_counter to `max` (used on load to restore from persisted state).
    pub(crate) fn restore_tx_counter_from(&self, max: u64) {
        self.tx_counter.store(max, Ordering::SeqCst);
    }

    /// Set the committed reader. Called by `PersistentFactStorage` after each
    /// open, migration and checkpoint.
    pub(crate) fn set_committed_reader(&self, reader: Arc<dyn crate::storage::CommittedReader>) {
        let mut d = self.data.write().unwrap_or_else(|e| e.into_inner());
        d.committed = Some(reader);
    }

    /// Returns (eavt_len, aevt_len) for the pending indexes (tests).
    #[cfg(test)]
    pub(crate) fn pending_index_counts(&self) -> (usize, usize) {
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        (d.pending_indexes.eavt.len(), d.pending_indexes.aevt.len())
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
            AsOf::Timestamp(t) => f.tx_id <= u64::try_from(*t).unwrap_or(0),
            AsOf::Slot(_) => false,
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
///
/// # Implementation note
///
/// Hot path for every non-`:as-of` query (#323). Each value is encoded once;
/// the two group maps borrow `(entity, attribute, value_bytes)` from the input
/// instead of cloning them. They keep std's randomly keyed hasher on purpose:
/// values are often untrusted text (agent memory), and a fixed-seed fast hash
/// would let crafted colliding values make every query quadratic. `by_window` stores the
/// index and `tx_count` of the winning assertion per validity window; survivors
/// are moved out of `facts` at the end, preserving input order.
///
/// Idempotent under duplicated input records (a duplicate assertion ties with
/// the original and loses; a duplicate retraction leaves the max unchanged).
/// `selective_fact_fetch` relies on this instead of deduplicating.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use std::collections::HashMap;

    type EavKey<'a> = (&'a EntityId, &'a str, &'a [u8]);
    type WindowKey<'a> = (&'a EntityId, &'a str, &'a [u8], i64, i64);

    let encoded: Vec<Vec<u8>> = facts.iter().map(|f| encode_value(&f.value)).collect();
    let mut keep = vec![false; facts.len()];

    {
        let mut max_retract_tx: HashMap<EavKey<'_>, u64> = HashMap::new();
        let mut by_window: HashMap<WindowKey<'_>, (usize, u64)> = HashMap::new();

        for (idx, (fact, value_bytes)) in facts.iter().zip(encoded.iter()).enumerate() {
            let entity = &fact.entity;
            let attribute = fact.attribute.as_str();
            let value = value_bytes.as_slice();
            if fact.asserted {
                by_window
                    .entry((entity, attribute, value, fact.valid_from, fact.valid_to))
                    .and_modify(|winner| {
                        if fact.tx_count > winner.1 {
                            *winner = (idx, fact.tx_count);
                        }
                    })
                    .or_insert((idx, fact.tx_count));
            } else {
                max_retract_tx
                    .entry((entity, attribute, value))
                    .and_modify(|max_tx| *max_tx = (*max_tx).max(fact.tx_count))
                    .or_insert(fact.tx_count);
            }
        }

        for ((entity, attribute, value, _, _), (idx, tx_count)) in &by_window {
            let retract_tx = max_retract_tx
                .get(&(*entity, *attribute, *value))
                .copied()
                .unwrap_or(0);
            if *tx_count > retract_tx
                && let Some(slot) = keep.get_mut(*idx)
            {
                *slot = true;
            }
        }
    }

    facts
        .into_iter()
        .zip(keep)
        .filter_map(|(fact, kept)| kept.then_some(fact))
        .collect()
}

/// The pending fact at `pos`, as indexed by `FactData::pending_indexes`.
fn pending_fact(d: &FactData, pos: usize) -> Result<Fact> {
    d.facts
        .get(pos)
        .cloned()
        .ok_or_else(|| err_coded!(ErrorCode::Int045, pos))
}

/// Production helpers on FactStorage: index-driven entity/attribute lookups used by the query executor.
impl FactStorage {
    /// Facts for a query's net-assert step: the pending facts of `scan` with
    /// `tx_count <= as_of` (all when `None`), plus the committed facts of `scan`
    /// that survive net-assert at `as_of` (#379).
    ///
    /// `net_asserted_facts` over the result equals `net_asserted_facts` over
    /// every record of `scan` up to `as_of` (see
    /// [`CommittedReader::live_facts`](crate::storage::CommittedReader::live_facts)),
    /// but committed history is not translated into facts.
    pub(crate) fn get_live_facts(
        &self,
        scan: crate::storage::Scan<'_>,
        as_of: Option<u64>,
    ) -> Result<Vec<Fact>> {
        use crate::storage::Scan;
        use crate::storage::index::{AevtKey, EavtKey};
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        let mut facts = Vec::new();
        let mut push = |pos: usize| -> Result<()> {
            let f = pending_fact(&d, pos)?;
            if as_of.is_none_or(|n| f.tx_count <= n) {
                facts.push(f);
            }
            Ok(())
        };
        match scan {
            Scan::All => {
                for pos in 0..d.facts.len() {
                    push(pos)?;
                }
            }
            Scan::Entity(entity) => {
                for (key, &pos) in d
                    .pending_indexes
                    .eavt
                    .range(EavtKey::entity_start(*entity)..)
                {
                    if key.entity != *entity {
                        break;
                    }
                    push(pos)?;
                }
            }
            Scan::EntityAttribute(entity, attribute) => {
                let start = EavtKey::entity_attribute_start(*entity, attribute);
                for (key, &pos) in d.pending_indexes.eavt.range(start..) {
                    if key.entity != *entity || key.attribute != attribute {
                        break;
                    }
                    push(pos)?;
                }
            }
            Scan::Attribute(attribute) => {
                for (key, &pos) in d
                    .pending_indexes
                    .aevt
                    .range(AevtKey::attribute_start(attribute)..)
                {
                    if key.attribute != attribute {
                        break;
                    }
                    push(pos)?;
                }
            }
        }
        if let Some(reader) = &d.committed {
            facts.extend(reader.live_facts(scan, as_of)?);
        }
        Ok(facts)
    }

    /// History read (every record); tests compare it with live reads and models.
    #[cfg(test)]
    /// Get all facts for a specific entity (index-driven).
    pub(crate) fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
        use crate::storage::index::EavtKey;
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        let mut facts = Vec::new();
        for (key, &pos) in d
            .pending_indexes
            .eavt
            .range(EavtKey::entity_start(*entity_id)..)
        {
            if key.entity != *entity_id {
                break;
            }
            facts.push(pending_fact(&d, pos)?);
        }
        if let Some(reader) = &d.committed {
            facts.extend(reader.facts_for_entity(entity_id)?);
        }
        Ok(facts)
    }

    /// History read (every record); tests compare it with live reads and models.
    #[cfg(test)]
    /// Get every stored record for one `(entity, attribute)` pair (index-driven, #323).
    ///
    /// Reads only that pair's EAVT range, so other attributes of the same entity
    /// (including prefix siblings such as `:ab` for `:a`) and their version
    /// history are never read.
    pub(crate) fn get_facts_by_entity_attribute_indexed(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Vec<Fact>> {
        use crate::storage::index::EavtKey;
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        let start = EavtKey::entity_attribute_start(*entity_id, attribute);
        let mut facts = Vec::new();
        for (key, &pos) in d.pending_indexes.eavt.range(start..) {
            if key.entity != *entity_id || key.attribute != *attribute {
                break;
            }
            facts.push(pending_fact(&d, pos)?);
        }
        if let Some(reader) = &d.committed {
            facts.extend(reader.facts_for_entity_attribute(entity_id, attribute)?);
        }
        Ok(facts)
    }

    /// History read (every record); tests compare it with live reads and models.
    #[cfg(test)]
    /// Get all facts for a specific attribute (index-driven).
    pub(crate) fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
        use crate::storage::index::AevtKey;
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        let mut facts = Vec::new();
        for (key, &pos) in d
            .pending_indexes
            .aevt
            .range(AevtKey::attribute_start(attribute)..)
        {
            if key.attribute != *attribute {
                break;
            }
            facts.push(pending_fact(&d, pos)?);
        }
        if let Some(reader) = &d.committed {
            facts.extend(reader.facts_for_attribute(attribute)?);
        }
        Ok(facts)
    }
}

/// Test-only helpers on FactStorage: for use in tests, not the production query path.
#[cfg(test)]
impl FactStorage {
    /// Get all facts for a specific entity and attribute.
    ///
    /// Note: uses a full scan via `get_all_facts()` rather than an index-driven range scan.
    /// For index-driven lookups, use `get_facts_by_entity` and filter by attribute in the caller.
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
            .and_then(|r| r.all_facts().ok())
            .map(|v| v.len())
            .unwrap_or(0);
        committed_count + d.facts.len()
    }

    /// Get the count of currently asserted facts. Test use only.
    pub(crate) fn asserted_fact_count(&self) -> usize {
        self.get_asserted_facts().map(|v| v.len()).unwrap_or(0)
    }

    /// Returns (eavt_len, aevt_len). Test use only.
    pub(crate) fn index_counts(&self) -> (usize, usize) {
        self.pending_index_counts()
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
        let (tx2, _) = storage
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
        let (eavt, aevt) = storage.index_counts();
        assert_eq!(eavt, 2);
        assert_eq!(aevt, 2);
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
        let (eavt, _) = storage.index_counts();
        assert_eq!(eavt, 1);
    }

    // =========================================================================
    // Committed reader integration
    // =========================================================================

    /// A committed reader over a fixed fact list, answering by filtering.
    struct MockReader(Vec<Fact>);

    impl crate::storage::CommittedReader for MockReader {
        fn all_facts(&self) -> anyhow::Result<Vec<Fact>> {
            Ok(self.0.clone())
        }
        fn facts_for_entity(&self, e: &EntityId) -> anyhow::Result<Vec<Fact>> {
            Ok(self.0.iter().filter(|f| f.entity == *e).cloned().collect())
        }
        fn facts_for_entity_attribute(&self, e: &EntityId, a: &str) -> anyhow::Result<Vec<Fact>> {
            Ok(self
                .0
                .iter()
                .filter(|f| f.entity == *e && f.attribute == a)
                .cloned()
                .collect())
        }
        fn facts_for_attribute(&self, a: &str) -> anyhow::Result<Vec<Fact>> {
            Ok(self
                .0
                .iter()
                .filter(|f| f.attribute == a)
                .cloned()
                .collect())
        }
    }

    #[test]
    fn committed_and_pending_facts_combine_in_every_lookup() {
        let (e, other) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let committed = vec![
            Fact::with_valid_time(
                e,
                ":a".into(),
                Value::Integer(1),
                10,
                1,
                0,
                crate::graph::types::VALID_TIME_FOREVER,
            ),
            Fact::with_valid_time(
                e,
                ":b".into(),
                Value::Integer(2),
                10,
                1,
                0,
                crate::graph::types::VALID_TIME_FOREVER,
            ),
            Fact::with_valid_time(
                other,
                ":a".into(),
                Value::Integer(3),
                10,
                1,
                0,
                crate::graph::types::VALID_TIME_FOREVER,
            ),
        ];
        let storage = FactStorage::new();
        storage.set_committed_reader(std::sync::Arc::new(MockReader(committed)));
        storage
            .transact(vec![(e, ":a".to_string(), Value::Integer(4))], None)
            .unwrap();
        assert_eq!(storage.get_all_facts().unwrap().len(), 4);
        assert_eq!(storage.get_facts_by_entity(&e).unwrap().len(), 3);
        let ea = storage
            .get_facts_by_entity_attribute_indexed(&e, &":a".to_string())
            .unwrap();
        assert_eq!(ea.len(), 2, "one committed and one pending :a for e");
        assert!(ea.iter().all(|f| f.entity == e && f.attribute == ":a"));
        assert_eq!(
            storage
                .get_facts_by_attribute(&":a".to_string())
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            storage
                .get_facts_by_entity(&uuid::Uuid::from_u128(9))
                .unwrap()
                .len(),
            0
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

    // -------------------------------------------------------------------------
    // Unit tests for net_asserted_facts directly
    // -------------------------------------------------------------------------

    /// Helper: build an asserted fact with explicit tx_count and valid window.
    fn make_assert(
        entity: uuid::Uuid,
        attr: &str,
        value: Value,
        tx_count: u64,
        valid_from: i64,
        valid_to: i64,
    ) -> Fact {
        Fact {
            entity,
            attribute: attr.to_string(),
            value,
            tx_id: tx_count as u64,
            tx_count,
            valid_from,
            valid_to,
            asserted: true,
        }
    }

    /// Helper: build a retraction with explicit tx_count and default valid window.
    fn make_retract(entity: uuid::Uuid, attr: &str, value: Value, tx_count: u64) -> Fact {
        Fact {
            entity,
            attribute: attr.to_string(),
            value,
            tx_id: tx_count as u64,
            tx_count,
            valid_from: 0,
            valid_to: VALID_TIME_FOREVER,
            asserted: false,
        }
    }

    /// Multiple retractions of the same EAV: `max_retract_tx` must be the
    /// global maximum, not just the first retraction seen.
    ///
    /// Timeline:
    ///   tx=1  assert W1        (should be wiped by retraction at tx=5)
    ///   tx=3  retract          (max_retract so far = 3)
    ///   tx=4  assert W1        (should survive — tx 4 > 3, BUT a later retraction at tx=5 wipes it)
    ///   tx=5  retract          (max_retract = 5, wipes tx=4 assertion)
    ///   tx=6  assert W1        (should survive — tx 6 > 5)
    #[test]
    fn test_net_asserted_multiple_retractions_max_wins() {
        let entity = uuid::Uuid::new_v4();
        let attr = ":salary";
        let value = Value::Integer(100_000);
        let w = (1_000_i64, VALID_TIME_FOREVER);

        let facts = vec![
            make_assert(entity, attr, value.clone(), 1, w.0, w.1),
            make_retract(entity, attr, value.clone(), 3),
            make_assert(entity, attr, value.clone(), 4, w.0, w.1),
            make_retract(entity, attr, value.clone(), 5),
            make_assert(entity, attr, value.clone(), 6, w.0, w.1),
        ];

        let result = net_asserted_facts(facts);
        assert_eq!(
            result.len(),
            1,
            "only the post-retraction assertion should survive"
        );
        assert_eq!(result[0].tx_count, 6);
    }

    /// A single retraction wipes all valid-time windows of the same EAV triple,
    /// not just the window that was explicitly retracted.
    ///
    /// Timeline:
    ///   tx=1  assert W1 (2020–2022)
    ///   tx=2  assert W2 (2024–2026)
    ///   tx=3  retract          → both windows must disappear
    #[test]
    fn test_net_asserted_retraction_wipes_all_windows() {
        let entity = uuid::Uuid::new_v4();
        let attr = ":salary";
        let value = Value::Integer(100_000);

        let facts = vec![
            make_assert(
                entity,
                attr,
                value.clone(),
                1,
                1_577_836_800_000,
                1_640_995_200_000,
            ),
            make_assert(
                entity,
                attr,
                value.clone(),
                2,
                1_704_067_200_000,
                VALID_TIME_FOREVER,
            ),
            make_retract(entity, attr, value.clone(), 3),
        ];

        let result = net_asserted_facts(facts);
        assert_eq!(
            result.len(),
            0,
            "retraction should wipe all windows for the EAV triple"
        );
    }

    /// Pre-#323 implementation, kept verbatim as the oracle for the
    /// randomized equivalence test.
    fn net_asserted_facts_reference(facts: Vec<Fact>) -> Vec<Fact> {
        use std::collections::HashMap;

        type EavKey = (EntityId, Attribute, Vec<u8>);
        type WindowKey = (EntityId, Attribute, Vec<u8>, i64, i64);

        let mut max_retract_tx: HashMap<EavKey, u64> = HashMap::new();
        let mut by_window: HashMap<WindowKey, Fact> = HashMap::new();

        for fact in facts {
            let eav_key = (
                fact.entity,
                fact.attribute.clone(),
                encode_value(&fact.value),
            );

            if fact.asserted {
                let window_key = (
                    eav_key.0,
                    eav_key.1,
                    eav_key.2,
                    fact.valid_from,
                    fact.valid_to,
                );
                match by_window.get(&window_key) {
                    None => {
                        by_window.insert(window_key, fact);
                    }
                    Some(existing) if fact.tx_count > existing.tx_count => {
                        by_window.insert(window_key, fact);
                    }
                    _ => {}
                }
            } else {
                let tx_count = fact.tx_count;
                max_retract_tx
                    .entry(eav_key)
                    .and_modify(|max_tx| *max_tx = (*max_tx).max(tx_count))
                    .or_insert(tx_count);
            }
        }

        by_window
            .into_iter()
            .filter_map(|((entity, attribute, value, _, _), fact)| {
                let retract_tx = max_retract_tx
                    .get(&(entity, attribute, value))
                    .copied()
                    .unwrap_or(0);
                (fact.tx_count > retract_tx).then_some(fact)
            })
            .collect()
    }

    /// Order-independent comparison key for a fact set.
    fn sorted_keys(facts: &[Fact]) -> Vec<String> {
        let mut v: Vec<String> = facts
            .iter()
            .map(|f| {
                format!(
                    "{}|{}|{:?}|{}|{}|{}|{}",
                    f.entity,
                    f.attribute,
                    f.value,
                    f.tx_count,
                    f.valid_from,
                    f.valid_to,
                    f.asserted
                )
            })
            .collect();
        v.sort();
        v
    }

    /// Deterministic xorshift so the test needs no RNG dependency.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn net_asserted_matches_reference_on_random_histories() {
        let entities = [uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2)];
        let attrs = [":a", ":ab", ":b"];
        let windows = [
            (0_i64, VALID_TIME_FOREVER),
            (1_000, 2_000),
            (1_500, VALID_TIME_FOREVER),
        ];
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _case in 0..500 {
            let n = rng.below(40) + 1;
            let mut facts = Vec::new();
            let mut tx = 0_u64;
            for _ in 0..n {
                // ~25% of records share the previous tx_count (same-transaction batches).
                if rng.below(4) != 0 {
                    tx += 1;
                }
                let e = entities[rng.below(2) as usize];
                let a = attrs[rng.below(3) as usize];
                let v = Value::Integer(i64::try_from(rng.below(3)).unwrap());
                if rng.below(3) == 0 {
                    facts.push(make_retract(e, a, v, tx));
                } else {
                    let (vf, vt) = windows[rng.below(3) as usize];
                    facts.push(make_assert(e, a, v, tx, vf, vt));
                }
            }
            let expected = sorted_keys(&net_asserted_facts_reference(facts.clone()));
            let actual = sorted_keys(&net_asserted_facts(facts));
            assert_eq!(
                actual, expected,
                "new net_asserted_facts diverged from reference"
            );
        }
    }

    /// The executor's selective fetch no longer dedups (#323); it relies on
    /// net_asserted_facts collapsing duplicated input records.
    #[test]
    fn net_asserted_idempotent_under_duplicates() {
        let e = uuid::Uuid::from_u128(7);
        let facts = vec![
            make_assert(
                e,
                ":hash",
                Value::String("h0".into()),
                1,
                0,
                VALID_TIME_FOREVER,
            ),
            make_retract(e, ":hash", Value::String("h0".into()), 2),
            make_assert(
                e,
                ":hash",
                Value::String("h1".into()),
                3,
                0,
                VALID_TIME_FOREVER,
            ),
            make_assert(
                e,
                ":other",
                Value::String("o".into()),
                3,
                0,
                VALID_TIME_FOREVER,
            ),
        ];
        let mut doubled = facts.clone();
        doubled.extend(facts.clone());
        let once = sorted_keys(&net_asserted_facts(facts));
        let twice = sorted_keys(&net_asserted_facts(doubled));
        assert_eq!(once.len(), 2, "h1 and o are live");
        assert_eq!(twice, once, "duplicated records must not change the result");
    }

    #[test]
    fn net_asserted_preserves_input_order() {
        let e = uuid::Uuid::from_u128(9);
        let facts = vec![
            make_assert(e, ":z", Value::Integer(1), 1, 0, VALID_TIME_FOREVER),
            make_assert(e, ":a", Value::Integer(2), 2, 0, VALID_TIME_FOREVER),
            make_assert(e, ":m", Value::Integer(3), 3, 0, VALID_TIME_FOREVER),
        ];
        let out = net_asserted_facts(facts);
        let attrs: Vec<&str> = out.iter().map(|f| f.attribute.as_str()).collect();
        assert_eq!(attrs, vec![":z", ":a", ":m"]);
    }

    #[test]
    fn entity_attribute_indexed_pending_only() {
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        let other = uuid::Uuid::from_u128(2);
        storage
            .transact(
                vec![
                    (e, ":hash".to_string(), Value::String("h0".into())),
                    (e, ":other".to_string(), Value::String("o".into())),
                    (other, ":hash".to_string(), Value::String("x".into())),
                ],
                None,
            )
            .unwrap();
        storage
            .retract(vec![(e, ":hash".to_string(), Value::String("h0".into()))])
            .unwrap();
        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":hash".to_string())
            .unwrap();
        assert_eq!(facts.len(), 2, "assert + retract of :hash for e only");
        assert!(
            facts
                .iter()
                .all(|f| f.entity == e && f.attribute == ":hash")
        );
    }

    #[test]
    fn entity_attribute_indexed_excludes_prefix_sibling() {
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        storage
            .transact(
                vec![
                    (e, ":a".to_string(), Value::Integer(1)),
                    (e, ":ab".to_string(), Value::Integer(2)),
                    (e, ":a/b".to_string(), Value::Integer(3)),
                ],
                None,
            )
            .unwrap();
        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":a".to_string())
            .unwrap();
        assert_eq!(facts.len(), 1, "only :a, not :ab or :a/b");
        assert_eq!(facts[0].value, Value::Integer(1));
    }

    /// #323 review: the committed EAVT range must end at the exact successor of
    /// `(e, a)` for every attribute — including ones whose last UTF-8 byte is 0xBF
    /// (e.g. `:丿`), where incrementing the last byte is not valid UTF-8 and the scan
    /// used to run unbounded to the end of the index.
    #[test]
    fn attribute_scan_excludes_prefix_siblings_and_non_ascii_neighbours() {
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(7);
        for attr in [
            ":a",
            ":ab",
            ":a/b",
            ":\u{4e3f}",
            ":\u{4e40}",
            ":\u{bf}",
            ":\u{c0}",
        ] {
            storage
                .transact(
                    vec![(e, attr.to_string(), Value::String(attr.to_string()))],
                    None,
                )
                .unwrap();
        }
        for attr in [":a", ":\u{4e3f}", ":\u{bf}"] {
            let facts = storage.get_facts_by_attribute(&attr.to_string()).unwrap();
            assert_eq!(facts.len(), 1, "exactly one fact for the attribute");
            assert!(
                facts.iter().all(|f| f.attribute == attr),
                "no sibling facts"
            );
        }
    }

    /// #447: restoring the counter after loading facts never lowers it.
    #[test]
    fn restore_tx_counter_never_moves_backwards() {
        let storage = FactStorage::new();
        storage.restore_tx_counter_from(5);
        storage.restore_tx_counter().unwrap();
        assert_eq!(storage.current_tx_count(), 5, "no loaded facts: keep 5");
        let mut f = Fact::new(
            uuid::Uuid::from_u128(1),
            ":a".to_string(),
            Value::Integer(1),
            1,
        );
        f.tx_count = 9;
        storage.load_fact(f).unwrap();
        storage.restore_tx_counter().unwrap();
        assert_eq!(storage.current_tx_count(), 9, "a higher loaded count wins");
    }

    /// #445: more than 65,535 pending facts must each resolve to their own fact
    /// through the EAVT (entity) and AEVT (attribute) paths, for every write path.
    #[test]
    fn pending_facts_past_u16_resolve_to_themselves() {
        const N: u32 = 70_000;
        let entity = |i: u32| uuid::Uuid::from_u128(u128::from(i) + 1);

        let storage = FactStorage::new();
        storage
            .transact_batch(
                (0..N)
                    .map(|i| {
                        (
                            entity(i),
                            ":n".to_string(),
                            Value::Integer(i64::from(i)),
                            None,
                        )
                    })
                    .collect(),
                None,
            )
            .unwrap();
        // One fact each through transact, retract and load_fact, all past slot 65,535.
        storage
            .transact(
                vec![(entity(N), ":n".to_string(), Value::Integer(-1))],
                None,
            )
            .unwrap();
        storage
            .retract(vec![(entity(N), ":n".to_string(), Value::Integer(-1))])
            .unwrap();
        let mut loaded = Fact::new(entity(N + 1), ":n".to_string(), Value::Integer(-2), 1);
        loaded.tx_count = 99;
        assert!(storage.load_fact(loaded).unwrap());

        for i in [0, 65_534, 65_535, 65_536, N - 1] {
            let facts = storage.get_facts_by_entity(&entity(i)).unwrap();
            assert_eq!(facts.len(), 1, "one fact per entity");
            assert_eq!(
                facts[0].value,
                Value::Integer(i64::from(i)),
                "entity's own value"
            );
            let facts = storage
                .get_facts_by_entity_attribute_indexed(&entity(i), &":n".to_string())
                .unwrap();
            assert_eq!(facts.len(), 1, "one fact per (entity, attribute)");
            assert_eq!(
                facts[0].value,
                Value::Integer(i64::from(i)),
                "entity's own value"
            );
        }
        let retract_pair = storage.get_facts_by_entity(&entity(N)).unwrap();
        assert_eq!(retract_pair.len(), 2, "assert and retract");
        assert_eq!(
            retract_pair.iter().filter(|f| !f.asserted).count(),
            1,
            "one retraction"
        );
        let loaded = storage.get_facts_by_entity(&entity(N + 1)).unwrap();
        assert_eq!(loaded.len(), 1, "loaded fact");
        assert_eq!(loaded[0].value, Value::Integer(-2), "loaded fact's value");

        let by_attr = storage.get_facts_by_attribute(&":n".to_string()).unwrap();
        assert_eq!(by_attr.len(), N as usize + 3, "every fact once");
        let distinct: HashSet<PendingKey> = by_attr.iter().map(pending_key).collect();
        assert_eq!(distinct.len(), N as usize + 3, "no fact returned twice");
    }
}
