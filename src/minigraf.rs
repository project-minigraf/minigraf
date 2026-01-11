/// High-level API for embedded graph database operations.
///
/// This module provides the main entry point for using Minigraf as an
/// embedded graph database, following the "SQLite for graphs" philosophy.

use crate::graph::types::{Edge, Node};
use crate::query::executor::QueryResult;
use crate::query::parser::parse_query;
use crate::storage::backend::FileBackend;
use crate::storage::persistent::{PersistentGraphStorage, StorageStats};
use anyhow::Result;

/// High-level embedded graph database.
///
/// This is the main API for using Minigraf as an embedded database.
/// It provides a simple interface for opening/creating databases,
/// executing queries, and automatic persistence.
///
/// # Example
///
/// ```
/// use minigraf::Minigraf;
///
/// # fn example() -> anyhow::Result<()> {
/// // Open or create a database
/// let mut db = Minigraf::open("myapp.graph")?;
///
/// // Execute queries
/// db.execute("CREATE NODE (:Person) {name: \"Alice\", age: 30}")?;
/// db.execute("MATCH (:Person)")?;
///
/// // Automatically persists on drop
/// drop(db);
///
/// // Later - reload the database
/// let db = Minigraf::open("myapp.graph")?;
/// // Alice is still there!
/// # Ok(())
/// # }
/// ```
pub struct Minigraf {
    storage: PersistentGraphStorage<FileBackend>,
}

impl Minigraf {
    /// Open or create a graph database at the specified path.
    ///
    /// If the file doesn't exist, creates a new empty database.
    /// If it exists, loads the existing graph data.
    ///
    /// The file will have a `.graph` extension.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use minigraf::Minigraf;
    ///
    /// let db = Minigraf::open("data/myapp.graph")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let backend = FileBackend::open(path)?;
        let storage = PersistentGraphStorage::new(backend)?;

        Ok(Minigraf { storage })
    }

    /// Execute a query string and return the result.
    ///
    /// Supports all query types: CREATE, MATCH, SHOW, etc.
    ///
    /// Changes are marked as dirty but not immediately persisted.
    /// Call `save()` to persist, or rely on auto-save on drop.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use minigraf::Minigraf;
    /// # let mut db = Minigraf::open("test.graph")?;
    /// db.execute("CREATE NODE (:Person) {name: \"Bob\"}")?;
    /// db.execute("MATCH (:Person)")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn execute(&mut self, query: &str) -> Result<QueryResult> {
        let parsed = parse_query(query)?;

        // Create a temporary GraphStorage-compatible wrapper
        // For now, we'll execute directly against our storage
        match parsed {
            crate::query::Query::CreateNode { labels, properties } => {
                let node = Node::new(labels, properties);
                let _id = self.storage.add_node(node.clone())?;
                Ok(QueryResult::NodeCreated(node))
            }
            crate::query::Query::CreateEdge {
                source_id,
                target_id,
                label,
                properties,
            } => {
                // Validate that source and target exist
                if self.storage.get_node(&source_id).is_none() {
                    anyhow::bail!("Source node {} not found", source_id);
                }
                if self.storage.get_node(&target_id).is_none() {
                    anyhow::bail!("Target node {} not found", target_id);
                }

                let edge = Edge::new(source_id, target_id, label, properties);
                self.storage.add_edge(edge.clone())?;
                Ok(QueryResult::EdgeCreated(edge))
            }
            crate::query::Query::MatchNodes { label, property_filter } => {
                let nodes = if let Some(ref l) = label {
                    self.storage.get_nodes_by_label(l)
                } else {
                    self.storage.get_all_nodes()
                };

                let nodes: Vec<Node> = nodes.into_iter().cloned().collect();

                let filtered = if let Some((key, value)) = property_filter {
                    nodes
                        .into_iter()
                        .filter(|n| n.properties.get(&key) == Some(&value))
                        .collect()
                } else {
                    nodes
                };

                Ok(QueryResult::Nodes(filtered))
            }
            crate::query::Query::MatchEdges { label } => {
                let edges = if let Some(ref l) = label {
                    self.storage.get_edges_by_label(l)
                } else {
                    self.storage.get_all_edges()
                };

                Ok(QueryResult::Edges(edges.into_iter().cloned().collect()))
            }
            crate::query::Query::ShowNodes => {
                let nodes = self.storage.get_all_nodes();
                Ok(QueryResult::Nodes(nodes.into_iter().cloned().collect()))
            }
            crate::query::Query::ShowEdges => {
                let edges = self.storage.get_all_edges();
                Ok(QueryResult::Edges(edges.into_iter().cloned().collect()))
            }
            crate::query::Query::Help => Ok(QueryResult::Help),
            crate::query::Query::Exit => Ok(QueryResult::Exit),
        }
    }

    /// Explicitly save changes to disk.
    ///
    /// This is optional - changes are auto-saved when the database is dropped.
    /// Call this if you want to ensure data is persisted immediately.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use minigraf::Minigraf;
    /// # let mut db = Minigraf::open("test.graph")?;
    /// db.execute("CREATE NODE (:Person) {name: \"Alice\"}")?;
    /// db.save()?;  // Explicitly persist
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn save(&mut self) -> Result<()> {
        self.storage.save()
    }

    /// Check if there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.storage.is_dirty()
    }

    /// Get statistics about the database.
    ///
    /// Returns node count, edge count, and dirty status.
    pub fn stats(&self) -> StorageStats {
        self.storage.stats()
    }

    /// Get all nodes (for direct access).
    pub fn nodes(&self) -> Vec<&Node> {
        self.storage.get_all_nodes()
    }

    /// Get all edges (for direct access).
    pub fn edges(&self) -> Vec<&Edge> {
        self.storage.get_all_edges()
    }

    /// Explicitly close the database.
    ///
    /// This saves any unsaved changes and closes the backend.
    /// After calling this, the Minigraf instance should not be used.
    pub fn close(mut self) -> Result<()> {
        if self.storage.is_dirty() {
            self.storage.save()?;
        }
        self.storage.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_minigraf_create_and_query() {
        let path = "/tmp/test_minigraf_create.graph";
        let _ = fs::remove_file(path);

        let mut db = Minigraf::open(path).unwrap();

        // Create a node
        let result = db
            .execute("CREATE NODE (:Person) {name: \"Alice\", age: 30}")
            .unwrap();
        assert!(matches!(result, QueryResult::NodeCreated(_)));

        // Query it back
        let result = db.execute("MATCH (:Person)").unwrap();
        if let QueryResult::Nodes(nodes) = result {
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].labels, vec!["Person"]);
        } else {
            panic!("Expected Nodes result");
        }

        // Clean up
        drop(db);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_minigraf_persistence() {
        let path = "/tmp/test_minigraf_persistence.graph";
        let _ = fs::remove_file(path);

        // Create and save
        {
            let mut db = Minigraf::open(path).unwrap();
            db.execute("CREATE NODE (:Person) {name: \"Bob\"}").unwrap();
            assert_eq!(db.nodes().len(), 1, "Node should be created in memory");

            db.save().unwrap();
            assert!(!db.is_dirty(), "Database should not be dirty after save");

            db.close().unwrap();
        }

        // Verify file exists
        assert!(std::path::Path::new(path).exists(), "Database file should exist after close");

        // Reopen and verify
        {
            let mut db = Minigraf::open(path).unwrap();

            let result = db.execute("SHOW NODES").unwrap();

            if let QueryResult::Nodes(nodes) = result {
                assert_eq!(nodes.len(), 1, "Should find 1 node after reopening");
                assert_eq!(nodes[0].labels, vec!["Person"]);
            } else {
                panic!("Expected Nodes result");
            }
        }

        // Clean up
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_minigraf_auto_save() {
        let path = "/tmp/test_minigraf_auto_save.graph";
        let _ = fs::remove_file(path);

        // Create and let it auto-save on drop
        {
            let mut db = Minigraf::open(path).unwrap();
            db.execute("CREATE NODE (:Company) {name: \"Acme\"}").unwrap();
            // Drop happens here - should auto-save
        }

        // Reopen and verify
        {
            let db = Minigraf::open(path).unwrap();
            let stats = db.stats();
            assert_eq!(stats.node_count, 1);
        }

        // Clean up
        let _ = fs::remove_file(path);
    }
}
