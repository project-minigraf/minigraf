use crate::graph::storage::GraphStorage;
use crate::graph::types::{Edge, Node};
use crate::query::parser::Query;
use anyhow::Result;

pub struct QueryExecutor<'a> {
    storage: &'a GraphStorage,
}

impl<'a> QueryExecutor<'a> {
    pub fn new(storage: &'a GraphStorage) -> Self {
        QueryExecutor { storage }
    }

    pub fn execute(&self, query: Query) -> Result<QueryResult> {
        match query {
            Query::CreateNode { labels, properties } => {
                let node = Node::new(labels, properties);
                self.storage.create_node(&node)?;
                Ok(QueryResult::NodeCreated(node))
            }
            Query::CreateEdge {
                source_id,
                target_id,
                label,
                properties,
            } => {
                if self.storage.get_node(&source_id)?.is_none() {
                    return Ok(QueryResult::Error(format!(
                        "Source node '{}' does not exist",
                        source_id
                    )));
                }

                if self.storage.get_node(&target_id)?.is_none() {
                    return Ok(QueryResult::Error(format!(
                        "Target node '{}' does not exist",
                        target_id
                    )));
                }

                let edge = Edge::new(source_id, target_id, label, properties);
                self.storage.create_edge(&edge)?;
                Ok(QueryResult::EdgeCreated(edge))
            }
            Query::MatchNodes {
                label,
                property_filter,
            } => {
                let all_nodes = self.storage.get_all_nodes()?;

                let filtered_nodes: Vec<Node> = all_nodes
                    .into_iter()
                    .filter(|node| {
                        let label_match = label
                            .as_ref()
                            .map(|l| node.labels.contains(l))
                            .unwrap_or(true);

                        let property_match = property_filter
                            .as_ref()
                            .map(|(key, value)| {
                                node.properties.get(key).map(|v| v == value).unwrap_or(false)
                            })
                            .unwrap_or(true);

                        label_match && property_match
                    })
                    .collect();

                Ok(QueryResult::Nodes(filtered_nodes))
            }
            Query::MatchEdges { label } => {
                let all_edges = self.storage.get_all_edges()?;

                let filtered_edges: Vec<Edge> = all_edges
                    .into_iter()
                    .filter(|edge| {
                        label
                            .as_ref()
                            .map(|l| &edge.label == l)
                            .unwrap_or(true)
                    })
                    .collect();

                Ok(QueryResult::Edges(filtered_edges))
            }
            Query::ShowNodes => {
                let nodes = self.storage.get_all_nodes()?;
                Ok(QueryResult::Nodes(nodes))
            }
            Query::ShowEdges => {
                let edges = self.storage.get_all_edges()?;
                Ok(QueryResult::Edges(edges))
            }
            Query::Help => Ok(QueryResult::Help),
            Query::Exit => Ok(QueryResult::Exit),
        }
    }
}

#[derive(Debug)]
pub enum QueryResult {
    NodeCreated(Node),
    EdgeCreated(Edge),
    Nodes(Vec<Node>),
    Edges(Vec<Edge>),
    Error(String),
    Help,
    Exit,
}

impl QueryResult {
    pub fn format(&self) -> String {
        match self {
            QueryResult::NodeCreated(node) => {
                format!(
                    "Node created: {} (labels: {}, properties: {})",
                    node.id,
                    node.labels.join(", "),
                    format_properties(&node.properties)
                )
            }
            QueryResult::EdgeCreated(edge) => {
                format!(
                    "Edge created: {} ({} -[{}]-> {}, properties: {})",
                    edge.id,
                    edge.source,
                    edge.label,
                    edge.target,
                    format_properties(&edge.properties)
                )
            }
            QueryResult::Nodes(nodes) => {
                if nodes.is_empty() {
                    "No nodes found.".to_string()
                } else {
                    let mut result = format!("Found {} node(s):\n", nodes.len());
                    for node in nodes {
                        result.push_str(&format!(
                            "  - {} (labels: {}, properties: {})\n",
                            node.id,
                            node.labels.join(", "),
                            format_properties(&node.properties)
                        ));
                    }
                    result
                }
            }
            QueryResult::Edges(edges) => {
                if edges.is_empty() {
                    "No edges found.".to_string()
                } else {
                    let mut result = format!("Found {} edge(s):\n", edges.len());
                    for edge in edges {
                        result.push_str(&format!(
                            "  - {} ({} -[{}]-> {}, properties: {})\n",
                            edge.id,
                            edge.source,
                            edge.label,
                            edge.target,
                            format_properties(&edge.properties)
                        ));
                    }
                    result
                }
            }
            QueryResult::Error(msg) => format!("Error: {}", msg),
            QueryResult::Help => {
                r#"
Minigraf Query Language Commands:

CREATE NODE (:Label1:Label2) {prop1: "value", prop2: 123}
  - Create a node with labels and properties
  - Properties are optional
  - Examples:
    CREATE NODE (:Person) {name: "Alice", age: 30}
    CREATE NODE (:Person:Employee)

CREATE EDGE (source_id)-[LABEL]->(target_id) {prop: "value"}
  - Create an edge between two existing nodes
  - Properties are optional
  - Example:
    CREATE EDGE (node-id-1)-[KNOWS]->(node-id-2) {since: 2020}

MATCH (:Label) [WHERE property = value]
  - Find nodes by label and optional property filter
  - Examples:
    MATCH (:Person)
    MATCH (:Person) WHERE name = "Alice"

MATCH -[:LABEL]->
  - Find edges by label
  - Example:
    MATCH -[:KNOWS]->

SHOW NODES
  - Show all nodes in the graph

SHOW EDGES
  - Show all edges in the graph

HELP
  - Show this help message

EXIT or QUIT
  - Exit the console
"#
                .to_string()
            }
            QueryResult::Exit => "Goodbye!".to_string(),
        }
    }
}

fn format_properties(props: &std::collections::HashMap<String, crate::graph::types::PropertyValue>) -> String {
    if props.is_empty() {
        return "{}".to_string();
    }

    let mut items: Vec<String> = props
        .iter()
        .map(|(k, v)| format!("{}: {}", k, format_property_value(v)))
        .collect();
    items.sort();

    format!("{{{}}}", items.join(", "))
}

fn format_property_value(value: &crate::graph::types::PropertyValue) -> String {
    match value {
        crate::graph::types::PropertyValue::String(s) => format!("\"{}\"", s),
        crate::graph::types::PropertyValue::Integer(i) => i.to_string(),
        crate::graph::types::PropertyValue::Float(f) => f.to_string(),
        crate::graph::types::PropertyValue::Boolean(b) => b.to_string(),
        crate::graph::types::PropertyValue::Null => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Property, PropertyValue};
    use crate::query::parser::parse_query;
    use std::collections::HashMap;

    #[test]
    fn test_create_node() {
        let storage = GraphStorage::new();
        let executor = QueryExecutor::new(&storage);

        let query = parse_query("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();
        let result = executor.execute(query).unwrap();

        match result {
            QueryResult::NodeCreated(node) => {
                assert_eq!(node.labels, vec!["Person"]);
                assert_eq!(
                    node.properties.get("name"),
                    Some(&PropertyValue::String("Alice".to_string()))
                );
            }
            _ => panic!("Expected NodeCreated"),
        }
    }

    #[test]
    fn test_create_edge() {
        let storage = GraphStorage::new();
        let executor = QueryExecutor::new(&storage);

        let node1 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node2 = Node::new(vec!["Person".to_string()], HashMap::new());
        storage.create_node(&node1).unwrap();
        storage.create_node(&node2).unwrap();

        let query_str = format!(
            "CREATE EDGE ({})-[KNOWS]->({}) {{since: 2020}}",
            node1.id, node2.id
        );
        let query = parse_query(&query_str).unwrap();
        let result = executor.execute(query).unwrap();

        match result {
            QueryResult::EdgeCreated(edge) => {
                assert_eq!(edge.label, "KNOWS");
                assert_eq!(edge.source, node1.id);
                assert_eq!(edge.target, node2.id);
            }
            _ => panic!("Expected EdgeCreated"),
        }
    }

    #[test]
    fn test_match_nodes() {
        let storage = GraphStorage::new();
        let executor = QueryExecutor::new(&storage);

        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));
        let node = Node::new(vec!["Person".to_string()], props);
        storage.create_node(&node).unwrap();

        let query = parse_query("MATCH (:Person)").unwrap();
        let result = executor.execute(query).unwrap();

        match result {
            QueryResult::Nodes(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].id, node.id);
            }
            _ => panic!("Expected Nodes"),
        }
    }

    #[test]
    fn test_show_nodes() {
        let storage = GraphStorage::new();
        let executor = QueryExecutor::new(&storage);

        let node1 = Node::new(vec!["Person".to_string()], HashMap::new());
        let node2 = Node::new(vec!["Person".to_string()], HashMap::new());
        storage.create_node(&node1).unwrap();
        storage.create_node(&node2).unwrap();

        let query = parse_query("SHOW NODES").unwrap();
        let result = executor.execute(query).unwrap();

        match result {
            QueryResult::Nodes(nodes) => {
                assert_eq!(nodes.len(), 2);
            }
            _ => panic!("Expected Nodes"),
        }
    }
}
