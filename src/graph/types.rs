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
}
