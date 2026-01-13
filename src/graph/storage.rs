use crate::graph::types::{Edge, EdgeId, Node, NodeId};
use crate::graph::types::{Fact, Value, EntityId, Attribute, TxId, tx_id_now};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct GraphStorage {
    nodes: Arc<RwLock<HashMap<NodeId, Node>>>,
    edges: Arc<RwLock<HashMap<EdgeId, Edge>>>,
}

impl Default for GraphStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphStorage {
    pub fn new() -> Self {
        GraphStorage {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            edges: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create_node(&self, node: &Node) -> Result<()> {
        let mut nodes = self.nodes.write().unwrap();
        nodes.insert(node.id.clone(), node.clone());
        Ok(())
    }

    pub fn get_node(&self, id: &NodeId) -> Result<Option<Node>> {
        let nodes = self.nodes.read().unwrap();
        Ok(nodes.get(id).cloned())
    }

    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let nodes = self.nodes.read().unwrap();
        Ok(nodes.values().cloned().collect())
    }

    pub fn delete_node(&self, id: &NodeId) -> Result<()> {
        let mut nodes = self.nodes.write().unwrap();
        nodes.remove(id);
        Ok(())
    }

    pub fn create_edge(&self, edge: &Edge) -> Result<()> {
        let mut edges = self.edges.write().unwrap();
        edges.insert(edge.id.clone(), edge.clone());
        Ok(())
    }

    pub fn get_edge(&self, id: &EdgeId) -> Result<Option<Edge>> {
        let edges = self.edges.read().unwrap();
        Ok(edges.get(id).cloned())
    }

    pub fn get_all_edges(&self) -> Result<Vec<Edge>> {
        let edges = self.edges.read().unwrap();
        Ok(edges.values().cloned().collect())
    }

    pub fn get_edges_from_node(&self, node_id: &NodeId) -> Result<Vec<Edge>> {
        let edges = self.edges.read().unwrap();
        Ok(edges
            .values()
            .filter(|edge| &edge.source == node_id)
            .cloned()
            .collect())
    }

    pub fn get_edges_to_node(&self, node_id: &NodeId) -> Result<Vec<Edge>> {
        let edges = self.edges.read().unwrap();
        Ok(edges
            .values()
            .filter(|edge| &edge.target == node_id)
            .cloned()
            .collect())
    }

    pub fn delete_edge(&self, id: &EdgeId) -> Result<()> {
        let mut edges = self.edges.write().unwrap();
        edges.remove(id);
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let mut nodes = self.nodes.write().unwrap();
        let mut edges = self.edges.write().unwrap();
        nodes.clear();
        edges.clear();
        Ok(())
    }
}

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
    use crate::graph::types::PropertyValue;
    use std::collections::HashMap;

    #[test]
    fn test_node_crud() {
        let storage = GraphStorage::new();

        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));

        let node = Node::new(vec!["Person".to_string()], props);
        let node_id = node.id.clone();

        storage.create_node(&node).unwrap();

        let retrieved = storage.get_node(&node_id).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, node_id);

        storage.delete_node(&node_id).unwrap();

        let deleted = storage.get_node(&node_id).unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn test_edge_crud() {
        let storage = GraphStorage::new();

        let node1 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node2 = Node::new(vec!["Person".to_string()], HashMap::new());

        storage.create_node(&node1).unwrap();
        storage.create_node(&node2).unwrap();

        let mut props = HashMap::new();
        props.insert("since".to_string(), PropertyValue::Integer(2020));

        let edge = Edge::new(
            node1.id.clone(),
            node2.id.clone(),
            "KNOWS".to_string(),
            props,
        );
        let edge_id = edge.id.clone();

        storage.create_edge(&edge).unwrap();

        let retrieved = storage.get_edge(&edge_id).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, edge_id);

        storage.delete_edge(&edge_id).unwrap();

        let deleted = storage.get_edge(&edge_id).unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn test_get_edges_from_node() {
        let storage = GraphStorage::new();

        let node1 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node2 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node3 = Node::new(vec!["Person".to_string()], HashMap::new());

        storage.create_node(&node1).unwrap();
        storage.create_node(&node2).unwrap();
        storage.create_node(&node3).unwrap();

        let edge1 = Edge::new(
            node1.id.clone(),
            node2.id.clone(),
            "KNOWS".to_string(),
            HashMap::new(),
        );

        let edge2 = Edge::new(
            node1.id.clone(),
            node3.id.clone(),
            "KNOWS".to_string(),
            HashMap::new(),
        );

        storage.create_edge(&edge1).unwrap();
        storage.create_edge(&edge2).unwrap();

        let edges_from_node1 = storage.get_edges_from_node(&node1.id).unwrap();
        assert_eq!(edges_from_node1.len(), 2);

        let edges_from_node2 = storage.get_edges_from_node(&node2.id).unwrap();
        assert_eq!(edges_from_node2.len(), 0);
    }

    #[test]
    fn test_get_all_nodes() {
        let storage = GraphStorage::new();

        let node1 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node2 = Node::new(vec!["Person".to_string()], HashMap::new());

        storage.create_node(&node1).unwrap();
        storage.create_node(&node2).unwrap();

        let all_nodes = storage.get_all_nodes().unwrap();
        assert_eq!(all_nodes.len(), 2);
    }

    // ========================================================================
    // FactStorage Tests (Phase 3+)
    // ========================================================================

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
