use crate::graph::types::{Edge, EdgeId, Node, NodeId};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct GraphStorage {
    nodes: Arc<RwLock<HashMap<NodeId, Node>>>,
    edges: Arc<RwLock<HashMap<EdgeId, Edge>>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Property, PropertyValue};
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
}
