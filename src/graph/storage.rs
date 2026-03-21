use crate::graph::types::{Attribute, EntityId, Fact, TxId, Value, TransactOptions, tx_id_now, VALID_TIME_FOREVER};
use crate::query::datalog::types::AsOf;
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

// ============================================================================
// Datalog Fact Storage (Phase 3+)
// ============================================================================

/// In-memory storage for Datalog facts with transaction support
///
/// FactStorage maintains an append-only log of facts. Facts are never deleted,
/// only retracted (with asserted=false). This enables:
/// - Full history tracking
/// - Time travel queries (Phase 4)
/// - Audit trails
///
/// # Storage Model (Phase 3-5)
///
/// This is a simple in-memory store using `Vec<Fact>`. All facts are kept in
/// memory for fast access. For persistence, see `PersistentFactStorage` which
/// wraps this with a "load all, save all" strategy.
///
/// **This is intentionally simple for Phase 3-5.** Phase 6 will add:
/// - Index-based access (EAVT, AEVT, AVET, VAET)
/// - On-demand loading from disk
/// - Bounded memory usage
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
    /// Append-only log of all facts (assertions and retractions)
    facts: Arc<RwLock<Vec<Fact>>>,
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
            facts: Arc::new(RwLock::new(Vec::new())),
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
                Fact::with_valid_time(entity, attribute, value, tx_id, tx_count, valid_from, valid_to)
            })
            .collect();

        let mut storage = self.facts.write().unwrap();
        storage.extend(facts);

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

        let mut storage = self.facts.write().unwrap();
        storage.extend(retractions);

        Ok(tx_id)
    }

    /// Insert a fact with its original tx_id and tx_count preserved.
    ///
    /// Used by the load and migration paths only — bypasses tx_counter entirely.
    /// After loading all facts, call `restore_tx_counter()` to re-synchronise the
    /// counter so subsequent `transact()` calls get correct tx_count values.
    pub fn load_fact(&self, fact: Fact) -> Result<()> {
        let mut storage = self.facts.write().unwrap();
        storage.push(fact);
        Ok(())
    }

    /// Set tx_counter to max(tx_count) across all loaded facts.
    ///
    /// Must be called after all `load_fact()` calls complete so that the next
    /// `transact()` call picks up from the right sequence number.
    pub fn restore_tx_counter(&self) -> Result<()> {
        let storage = self.facts.read().unwrap();
        let max = storage.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.tx_counter.store(max, Ordering::SeqCst);
        Ok(())
    }

    /// Return all facts visible as of the given transaction point.
    ///
    /// * `AsOf::Counter(n)` — include facts whose `tx_count <= n`
    /// * `AsOf::Timestamp(t)` — include facts whose `tx_id <= t as u64`
    pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        let filtered = storage
            .iter()
            .filter(|f| match as_of {
                AsOf::Counter(n) => f.tx_count <= *n,
                AsOf::Timestamp(t) => f.tx_id <= *t as u64,
            })
            .cloned()
            .collect();
        Ok(filtered)
    }

    /// Return all asserted facts valid at the given timestamp.
    ///
    /// A fact is valid at `ts` when `valid_from <= ts < valid_to` and it is asserted.
    pub fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        let filtered = storage
            .iter()
            .filter(|f| f.is_asserted() && f.valid_from <= ts && ts < f.valid_to)
            .cloned()
            .collect();
        Ok(filtered)
    }

    /// Get all facts (including retractions)
    ///
    /// Returns the complete append-only log. For current state, filter by
    /// asserted=true and take the most recent fact for each (E, A) pair.
    pub fn get_all_facts(&self) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        Ok(storage.clone())
    }

    /// Get all asserted facts (filters out retractions)
    ///
    /// Returns only facts where asserted=true. This gives you the currently
    /// valid facts, but includes all historical versions.
    pub fn get_asserted_facts(&self) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        Ok(storage
            .iter()
            .filter(|f| f.is_asserted())
            .cloned()
            .collect())
    }

    /// Get all facts for a specific entity
    ///
    /// # Arguments
    /// * `entity_id` - The entity to query
    ///
    /// # Returns
    /// All facts (assertions and retractions) about this entity
    pub fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        Ok(storage
            .iter()
            .filter(|f| &f.entity == entity_id)
            .cloned()
            .collect())
    }

    /// Get all facts for a specific attribute
    ///
    /// # Arguments
    /// * `attribute` - The attribute to query (e.g., ":person/name")
    ///
    /// # Returns
    /// All facts with this attribute
    pub fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
        let storage = self.facts.read().unwrap();
        Ok(storage
            .iter()
            .filter(|f| &f.attribute == attribute)
            .cloned()
            .collect())
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
        let storage = self.facts.read().unwrap();
        Ok(storage
            .iter()
            .filter(|f| &f.entity == entity_id && &f.attribute == attribute)
            .cloned()
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
        let storage = self.facts.read().unwrap();

        // Find the most recent fact for this (entity, attribute) pair
        let mut relevant_facts: Vec<&Fact> = storage
            .iter()
            .filter(|f| &f.entity == entity_id && &f.attribute == attribute)
            .collect();

        // Sort by transaction ID (timestamp) descending
        relevant_facts.sort_by(|a, b| b.tx_id.cmp(&a.tx_id));

        // Return the value if the most recent fact is an assertion
        Ok(relevant_facts
            .first()
            .and_then(|f| if f.is_asserted() { Some(f.value.clone()) } else { None }))
    }

    /// Get the count of all facts in storage
    pub fn fact_count(&self) -> usize {
        let storage = self.facts.read().unwrap();
        storage.len()
    }

    /// Get the count of currently asserted facts
    pub fn asserted_fact_count(&self) -> usize {
        let storage = self.facts.read().unwrap();
        storage.iter().filter(|f| f.is_asserted()).count()
    }

    /// Clear all facts (for testing)
    pub fn clear(&self) -> Result<()> {
        let mut storage = self.facts.write().unwrap();
        storage.clear();
        self.tx_counter.store(0, Ordering::SeqCst);
        Ok(())
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
            .transact(vec![
                (
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                ),
                (alice, ":person/age".to_string(), Value::Integer(30)),
            ], None)
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
            .transact(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            )], None)
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
            .transact(vec![
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
            ], None)
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
            .transact(vec![
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
            ], None)
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
            .transact(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            )], None)
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        // Update value
        storage
            .transact(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice Smith".to_string()),
            )], None)
            .unwrap();

        // Current value should be the most recent
        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, Some(Value::String("Alice Smith".to_string())));

        std::thread::sleep(std::time::Duration::from_millis(2));

        // Retract the value
        storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice Smith".to_string()),
            )])
            .unwrap();

        // Current value should now be None (retracted)
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
            .transact(vec![
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
            ], None)
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
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(30))], None)
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx2 = storage
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(31))], None)
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx3 = storage
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(32))], None)
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
            .transact(vec![
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
            ], None)
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

        storage.transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
        ], None).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        storage.transact(vec![
            (alice, ":person/age".to_string(), Value::Integer(30)),
        ], None).unwrap();

        let facts = storage.get_all_facts().unwrap();
        let name_fact = facts.iter().find(|f| f.attribute == ":person/name").unwrap();
        let age_fact = facts.iter().find(|f| f.attribute == ":person/age").unwrap();

        assert_eq!(name_fact.tx_count, 1);
        assert_eq!(age_fact.tx_count, 2);
    }

    #[test]
    fn test_batch_facts_share_tx_count() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        storage.transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (alice, ":person/age".to_string(), Value::Integer(30)),
        ], None).unwrap();

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
            12345_u64,  // original tx_id
            7,          // original tx_count
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
        storage.transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
        ], None).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        // tx_count = 2
        storage.transact(vec![
            (alice, ":person/age".to_string(), Value::Integer(30)),
        ], None).unwrap();

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

        storage.transact(vec![
            (alice, ":employment/status".to_string(), Value::Keyword(":active".to_string())),
        ], Some(opts)).unwrap();

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
            entity, ":a".to_string(), Value::Integer(1),
            1000, 5, 1000_i64, VALID_TIME_FOREVER,
        );
        storage.load_fact(fact).unwrap();
        storage.restore_tx_counter().unwrap();

        // Next transact should get tx_count = 6
        storage.transact(vec![
            (entity, ":b".to_string(), Value::Integer(2)),
        ], None).unwrap();

        let facts = storage.get_all_facts().unwrap();
        let b_fact = facts.iter().find(|f| f.attribute == ":b").unwrap();
        assert_eq!(b_fact.tx_count, 6);
    }
}
