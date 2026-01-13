use crate::graph::types::{Attribute, EntityId, Fact, TxId, Value, tx_id_now};
use anyhow::Result;
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
/// ]).unwrap();
///
/// // Query facts
/// let facts = storage.get_facts_by_entity(&alice).unwrap();
/// assert_eq!(facts.len(), 2);
/// ```
#[derive(Clone)]
pub struct FactStorage {
    /// Append-only log of all facts (assertions and retractions)
    facts: Arc<RwLock<Vec<Fact>>>,
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
        }
    }

    /// Transact a batch of facts with automatic timestamping
    ///
    /// All facts in a single transaction get the same timestamp (TxId).
    /// This groups related facts together for atomicity.
    ///
    /// # Arguments
    /// * `fact_tuples` - Vec of (EntityId, Attribute, Value) tuples to assert
    ///
    /// # Returns
    /// The TxId (timestamp) assigned to these facts
    pub fn transact(&self, fact_tuples: Vec<(EntityId, Attribute, Value)>) -> Result<TxId> {
        let tx_id = tx_id_now();

        let facts: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| Fact::new(entity, attribute, value, tx_id))
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

        let retractions: Vec<Fact> = fact_tuples
            .into_iter()
            .map(|(entity, attribute, value)| Fact::retract(entity, attribute, value, tx_id))
            .collect();

        let mut storage = self.facts.write().unwrap();
        storage.extend(retractions);

        Ok(tx_id)
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
            ])
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
            )])
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
            ])
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
            ])
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
            )])
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        // Update value
        storage
            .transact(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice Smith".to_string()),
            )])
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
            ])
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
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(30))])
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx2 = storage
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(31))])
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let tx3 = storage
            .transact(vec![(alice, ":person/age".to_string(), Value::Integer(32))])
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
            ])
            .unwrap();

        // All facts should have same tx_id (atomic batch)
        let facts = storage.get_facts_by_entity(&alice).unwrap();
        assert_eq!(facts.len(), 3);
        assert!(facts.iter().all(|f| f.tx_id == tx_id));
    }
}
