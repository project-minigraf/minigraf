/// Persistent graph storage that integrates StorageBackend with graph operations.
///
/// This module bridges the gap between high-level graph operations (nodes, edges)
/// and low-level page-based storage backends.

use crate::graph::types::{Edge, EdgeId, Node, NodeId};
use crate::storage::{FileHeader, StorageBackend, PAGE_SIZE};
use anyhow::Result;
use std::collections::HashMap;

/// Persistent graph storage with serialization support.
///
/// Architecture:
/// - Page 0: File header (metadata)
/// - Page 1: Index (NodeId/EdgeId -> page mapping)
/// - Page 2+: Serialized nodes and edges
///
/// Current implementation uses a simple "load all, save all" approach:
/// - On open: Deserialize all nodes and edges into memory
/// - On save: Serialize all nodes and edges back to disk
///
/// This works well for small-to-medium graphs (<1M nodes) and provides
/// a complete embedded database experience.
pub struct PersistentGraphStorage<B: StorageBackend> {
    backend: B,
    nodes: HashMap<NodeId, Node>,
    edges: HashMap<EdgeId, Edge>,
    dirty: bool,
}

impl<B: StorageBackend> PersistentGraphStorage<B> {
    /// Create a new persistent storage with the given backend.
    ///
    /// If the backend already contains data, loads it.
    /// Otherwise, initializes a new empty graph.
    pub fn new(backend: B) -> Result<Self> {
        let mut storage = PersistentGraphStorage {
            backend,
            nodes: HashMap::new(),
            edges: HashMap::new(),
            dirty: false,
        };

        // Try to load existing data
        if storage.backend.page_count()? > 1 {
            storage.load()?;
        }

        Ok(storage)
    }

    /// Load all nodes and edges from the backend into memory.
    fn load(&mut self) -> Result<()> {
        // Read index from page 1
        let index_page = self.backend.read_page(1)?;
        let index: StorageIndex = bincode::deserialize(&index_page)
            .unwrap_or_else(|_| StorageIndex::default());

        // Load nodes
        for (node_id, page_id) in &index.node_pages {
            let page = self.backend.read_page(*page_id)?;
            if let Ok(node) = bincode::deserialize::<Node>(&page) {
                self.nodes.insert(node_id.clone(), node);
            }
        }

        // Load edges
        for (edge_id, page_id) in &index.edge_pages {
            let page = self.backend.read_page(*page_id)?;
            if let Ok(edge) = bincode::deserialize::<Edge>(&page) {
                self.edges.insert(edge_id.clone(), edge);
            }
        }

        self.dirty = false;
        Ok(())
    }

    /// Save all nodes and edges from memory to the backend.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(()); // No changes to save
        }

        let mut index = StorageIndex::default();
        let mut next_page = 2u64; // Pages 0 and 1 are reserved

        // Serialize nodes
        for (node_id, node) in &self.nodes {
            let data = bincode::serialize(node)?;
            if data.len() > PAGE_SIZE {
                anyhow::bail!(
                    "Node {} too large: {} bytes (max {})",
                    node_id,
                    data.len(),
                    PAGE_SIZE
                );
            }

            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);

            self.backend.write_page(next_page, &page)?;
            index.node_pages.insert(node_id.clone(), next_page);
            next_page += 1;
        }

        // Serialize edges
        for (edge_id, edge) in &self.edges {
            let data = bincode::serialize(edge)?;
            if data.len() > PAGE_SIZE {
                anyhow::bail!(
                    "Edge {} too large: {} bytes (max {})",
                    edge_id,
                    data.len(),
                    PAGE_SIZE
                );
            }

            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);

            self.backend.write_page(next_page, &page)?;
            index.edge_pages.insert(edge_id.clone(), next_page);
            next_page += 1;
        }

        // Write index to page 1
        let index_data = bincode::serialize(&index)?;
        if index_data.len() > PAGE_SIZE {
            anyhow::bail!(
                "Index too large: {} bytes (max {}). Consider splitting into multiple pages.",
                index_data.len(),
                PAGE_SIZE
            );
        }

        let mut index_page = vec![0u8; PAGE_SIZE];
        index_page[..index_data.len()].copy_from_slice(&index_data);
        self.backend.write_page(1, &index_page)?;

        // Update header
        let header = FileHeader {
            magic: crate::storage::MAGIC_NUMBER,
            version: crate::storage::FORMAT_VERSION,
            page_count: next_page,
            node_count: self.nodes.len() as u64,
            edge_count: self.edges.len() as u64,
            reserved: [0; 32],
        };

        let header_data = header.to_bytes();
        let mut header_page = vec![0u8; PAGE_SIZE];
        header_page[..header_data.len()].copy_from_slice(&header_data);
        self.backend.write_page(0, &header_page)?;

        // Sync to disk
        self.backend.sync()?;
        self.dirty = false;

        Ok(())
    }

    /// Add a node to the graph.
    pub fn add_node(&mut self, node: Node) -> Result<NodeId> {
        let id = node.id.clone();
        self.nodes.insert(id.clone(), node);
        self.dirty = true;
        Ok(id)
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get all nodes.
    pub fn get_all_nodes(&self) -> Vec<&Node> {
        self.nodes.values().collect()
    }

    /// Add an edge to the graph.
    pub fn add_edge(&mut self, edge: Edge) -> Result<EdgeId> {
        let id = edge.id.clone();
        self.edges.insert(id.clone(), edge);
        self.dirty = true;
        Ok(id)
    }

    /// Get an edge by ID.
    pub fn get_edge(&self, id: &EdgeId) -> Option<&Edge> {
        self.edges.get(id)
    }

    /// Get all edges.
    pub fn get_all_edges(&self) -> Vec<&Edge> {
        self.edges.values().collect()
    }

    /// Get nodes by label.
    pub fn get_nodes_by_label(&self, label: &str) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|node| node.labels.contains(&label.to_string()))
            .collect()
    }

    /// Get edges by label.
    pub fn get_edges_by_label(&self, label: &str) -> Vec<&Edge> {
        self.edges
            .values()
            .filter(|edge| edge.label == label)
            .collect()
    }

    /// Check if storage has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Get statistics about the graph.
    pub fn stats(&self) -> StorageStats {
        StorageStats {
            node_count: self.nodes.len(),
            edge_count: self.edges.len(),
            dirty: self.dirty,
        }
    }

    /// Close the storage and underlying backend.
    pub fn close(mut self) -> Result<()> {
        if self.dirty {
            self.save()?;
        }
        self.backend.close()
    }
}

impl<B: StorageBackend> Drop for PersistentGraphStorage<B> {
    /// Automatically save on drop if there are unsaved changes.
    fn drop(&mut self) {
        if self.dirty {
            if let Err(e) = self.save() {
                eprintln!("Warning: Failed to save graph on drop: {}", e);
            }
        }
    }
}

/// Index structure mapping IDs to page numbers.
///
/// Stored in page 1 of the .graph file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct StorageIndex {
    node_pages: HashMap<NodeId, u64>,
    edge_pages: HashMap<EdgeId, u64>,
}

/// Statistics about the storage.
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub dirty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Property, PropertyValue};
    use crate::storage::backend::MemoryBackend;

    #[test]
    fn test_persistent_storage_add_node() {
        let backend = MemoryBackend::new();
        let mut storage = PersistentGraphStorage::new(backend).unwrap();

        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));

        let node = Node::new(vec!["Person".to_string()], props);
        let id = storage.add_node(node.clone()).unwrap();

        assert_eq!(storage.get_node(&id).unwrap().id, node.id);
        assert!(storage.is_dirty());
    }

    #[test]
    fn test_persistent_storage_save_load() {
        let backend = MemoryBackend::new();

        // Create and save data
        {
            let mut storage = PersistentGraphStorage::new(backend.clone()).unwrap();

            let mut props = HashMap::new();
            props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));
            let node = Node::new(vec!["Person".to_string()], props);

            storage.add_node(node).unwrap();
            storage.save().unwrap();
            assert!(!storage.is_dirty());
        }

        // Load data in new storage instance
        {
            let storage = PersistentGraphStorage::new(backend).unwrap();
            assert_eq!(storage.get_all_nodes().len(), 1);
            let node = storage.get_all_nodes()[0];
            assert_eq!(node.labels, vec!["Person"]);
        }
    }

    #[test]
    fn test_auto_save_on_drop() {
        let backend = MemoryBackend::new();

        // Create and let it auto-save on drop
        {
            let mut storage = PersistentGraphStorage::new(backend.clone()).unwrap();
            let node = Node::new(vec!["Person".to_string()], HashMap::new());
            storage.add_node(node).unwrap();
            // Drop happens here, should auto-save
        }

        // Load and verify
        {
            let storage = PersistentGraphStorage::new(backend).unwrap();
            assert_eq!(storage.get_all_nodes().len(), 1);
        }
    }
}
