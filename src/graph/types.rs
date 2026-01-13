use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub type NodeId = String;
pub type EdgeId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PropertyValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

impl PropertyValue {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            PropertyValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            PropertyValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            PropertyValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

pub type Property = HashMap<String, PropertyValue>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub labels: Vec<String>,
    pub properties: Property,
}

impl Node {
    pub fn new(labels: Vec<String>, properties: Property) -> Self {
        Node {
            id: Uuid::new_v4().to_string(),
            labels,
            properties,
        }
    }

    pub fn with_id(id: NodeId, labels: Vec<String>, properties: Property) -> Self {
        Node {
            id,
            labels,
            properties,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub label: String,
    pub properties: Property,
}

impl Edge {
    pub fn new(source: NodeId, target: NodeId, label: String, properties: Property) -> Self {
        Edge {
            id: Uuid::new_v4().to_string(),
            source,
            target,
            label,
            properties,
        }
    }

    pub fn with_id(
        id: EdgeId,
        source: NodeId,
        target: NodeId,
        label: String,
        properties: Property,
    ) -> Self {
        Edge {
            id,
            source,
            target,
            label,
            properties,
        }
    }
}

// ============================================================================
// Datalog EAV Model (Phase 3+)
// ============================================================================

/// Transaction ID type for tracking when facts were asserted/retracted
pub type TxId = u64;

/// Entity ID type - using UUID for consistency with existing Node/Edge IDs
pub type EntityId = Uuid;

/// Attribute name - namespace-qualified keywords like ":person/name" or ":friend"
pub type Attribute = String;

/// Value types for Datalog facts
/// Extends PropertyValue with Ref (entity reference) and Keyword support
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Value {
    /// String value
    String(String),
    /// 64-bit integer
    Integer(i64),
    /// 64-bit floating point
    Float(f64),
    /// Boolean value
    Boolean(bool),
    /// Reference to another entity (for relationships)
    Ref(EntityId),
    /// Keyword (e.g., ":status/active", ":person")
    Keyword(String),
    /// Null/None value
    Null,
}

impl Value {
    /// Extract string value if this is a String variant
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Extract integer value if this is an Integer variant
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract float value if this is a Float variant
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Extract boolean value if this is a Boolean variant
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract entity reference if this is a Ref variant
    pub fn as_ref(&self) -> Option<EntityId> {
        match self {
            Value::Ref(id) => Some(*id),
            _ => None,
        }
    }

    /// Extract keyword if this is a Keyword variant
    pub fn as_keyword(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// Check if this value is Null
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// A Datalog fact: (Entity, Attribute, Value) triple with transaction metadata
///
/// This is the core data structure for Phase 3+. Facts are immutable and versioned
/// by transaction ID. Facts are never deleted, only retracted (asserted=false).
///
/// # Examples
/// ```
/// use minigraf::{Fact, Value};
/// use uuid::Uuid;
///
/// // Fact: Alice's name is "Alice"
/// let alice_id = Uuid::new_v4();
/// let fact = Fact::new(
///     alice_id,
///     ":person/name".to_string(),
///     Value::String("Alice".to_string()),
///     1, // transaction ID
/// );
///
/// // Fact: Alice is friends with Bob (reference)
/// let bob_id = Uuid::new_v4();
/// let friendship = Fact::new(
///     alice_id,
///     ":friend".to_string(),
///     Value::Ref(bob_id),
///     2,
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    /// The entity this fact is about
    pub entity: EntityId,
    /// The attribute/property name (namespace-qualified, e.g., ":person/name")
    pub attribute: Attribute,
    /// The value of this attribute
    pub value: Value,
    /// Transaction ID that asserted or retracted this fact
    pub tx_id: TxId,
    /// True if this fact is asserted, false if retracted
    /// Retractions are used instead of deletions to maintain history
    pub asserted: bool,
}

impl Fact {
    /// Create a new asserted fact
    pub fn new(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            asserted: true,
        }
    }

    /// Create a retraction of a fact
    pub fn retract(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            asserted: false,
        }
    }

    /// Create a fact with explicit asserted flag
    pub fn with_asserted(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        asserted: bool,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            asserted,
        }
    }

    /// Check if this is an assertion (not a retraction)
    pub fn is_asserted(&self) -> bool {
        self.asserted
    }

    /// Check if this is a retraction
    pub fn is_retracted(&self) -> bool {
        !self.asserted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_creation() {
        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));
        props.insert("age".to_string(), PropertyValue::Integer(30));

        let node = Node::new(vec!["Person".to_string()], props);

        assert_eq!(node.labels, vec!["Person"]);
        assert_eq!(
            node.properties.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(
            node.properties.get("age"),
            Some(&PropertyValue::Integer(30))
        );
    }

    #[test]
    fn test_edge_creation() {
        let source_id = "node1".to_string();
        let target_id = "node2".to_string();

        let mut props = HashMap::new();
        props.insert("since".to_string(), PropertyValue::Integer(2020));

        let edge = Edge::new(
            source_id.clone(),
            target_id.clone(),
            "KNOWS".to_string(),
            props,
        );

        assert_eq!(edge.source, source_id);
        assert_eq!(edge.target, target_id);
        assert_eq!(edge.label, "KNOWS");
        assert_eq!(
            edge.properties.get("since"),
            Some(&PropertyValue::Integer(2020))
        );
    }

    #[test]
    fn test_property_value_accessors() {
        let string_val = PropertyValue::String("test".to_string());
        assert_eq!(string_val.as_string(), Some("test"));
        assert_eq!(string_val.as_integer(), None);

        let int_val = PropertyValue::Integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        assert_eq!(int_val.as_string(), None);

        let float_val = PropertyValue::Float(42.5);
        assert_eq!(float_val.as_float(), Some(42.5));

        let bool_val = PropertyValue::Boolean(true);
        assert_eq!(bool_val.as_boolean(), Some(true));
    }

    // ========================================================================
    // EAV Model Tests (Phase 3+)
    // ========================================================================

    #[test]
    fn test_value_creation_and_accessors() {
        // String value
        let string_val = Value::String("Alice".to_string());
        assert_eq!(string_val.as_string(), Some("Alice"));
        assert_eq!(string_val.as_integer(), None);
        assert!(!string_val.is_null());

        // Integer value
        let int_val = Value::Integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        assert_eq!(int_val.as_string(), None);

        // Float value
        let float_val = Value::Float(3.14);
        assert_eq!(float_val.as_float(), Some(3.14));

        // Boolean value
        let bool_val = Value::Boolean(true);
        assert_eq!(bool_val.as_boolean(), Some(true));

        // Reference value
        let ref_id = Uuid::new_v4();
        let ref_val = Value::Ref(ref_id);
        assert_eq!(ref_val.as_ref(), Some(ref_id));
        assert_eq!(ref_val.as_string(), None);

        // Keyword value
        let keyword_val = Value::Keyword(":person".to_string());
        assert_eq!(keyword_val.as_keyword(), Some(":person"));

        // Null value
        let null_val = Value::Null;
        assert!(null_val.is_null());
        assert_eq!(null_val.as_string(), None);
    }

    #[test]
    fn test_fact_creation() {
        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.value, Value::String("Alice".to_string()));
        assert_eq!(fact.tx_id, 1);
        assert!(fact.is_asserted());
        assert!(!fact.is_retracted());
    }

    #[test]
    fn test_fact_retraction() {
        let entity = Uuid::new_v4();
        let fact = Fact::retract(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.tx_id, 2);
        assert!(!fact.is_asserted());
        assert!(fact.is_retracted());
    }

    #[test]
    fn test_fact_with_ref_value() {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // Fact: Alice is friends with Bob
        let friendship = Fact::new(alice, ":friend".to_string(), Value::Ref(bob), 1);

        assert_eq!(friendship.entity, alice);
        assert_eq!(friendship.attribute, ":friend");
        assert_eq!(friendship.value.as_ref(), Some(bob));
        assert!(friendship.is_asserted());
    }

    #[test]
    fn test_fact_equality() {
        let entity = Uuid::new_v4();

        let fact1 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        let fact2 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact1, fact2);

        // Different transaction ID = different fact
        let fact3 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_ne!(fact1, fact3);
    }

    #[test]
    fn test_value_types() {
        let values = vec![
            Value::String("test".to_string()),
            Value::Integer(42),
            Value::Float(3.14),
            Value::Boolean(true),
            Value::Ref(Uuid::new_v4()),
            Value::Keyword(":status/active".to_string()),
            Value::Null,
        ];

        // All values should serialize/deserialize correctly
        for value in values {
            let serialized = serde_json::to_string(&value).unwrap();
            let deserialized: Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(value, deserialized);
        }
    }
}
