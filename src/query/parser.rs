use crate::graph::types::{Property, PropertyValue};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    CreateNode {
        labels: Vec<String>,
        properties: Property,
    },
    CreateEdge {
        source_id: String,
        target_id: String,
        label: String,
        properties: Property,
    },
    MatchNodes {
        label: Option<String>,
        property_filter: Option<(String, PropertyValue)>,
    },
    MatchEdges {
        label: Option<String>,
    },
    ShowNodes,
    ShowEdges,
    Help,
    Exit,
}

pub fn parse_query(input: &str) -> Result<Query> {
    let input = input.trim();

    if input.is_empty() {
        return Err(anyhow!("Empty query"));
    }

    let input_upper = input.to_uppercase();

    if input_upper == "EXIT" || input_upper == "QUIT" {
        return Ok(Query::Exit);
    }

    if input_upper == "HELP" {
        return Ok(Query::Help);
    }

    if input_upper.starts_with("SHOW NODES") {
        return Ok(Query::ShowNodes);
    }

    if input_upper.starts_with("SHOW EDGES") {
        return Ok(Query::ShowEdges);
    }

    if input_upper.starts_with("CREATE NODE") {
        return parse_create_node(input);
    }

    if input_upper.starts_with("CREATE EDGE") {
        return parse_create_edge(input);
    }

    if input_upper.starts_with("MATCH") {
        return parse_match(input);
    }

    Err(anyhow!("Unknown query type. Type HELP for available commands."))
}

fn parse_create_node(input: &str) -> Result<Query> {
    let input = input.trim();
    let rest = input.strip_prefix("CREATE NODE").or_else(|| input.strip_prefix("create node"))
        .ok_or_else(|| anyhow!("Invalid CREATE NODE syntax"))?
        .trim();

    let (labels, properties) = if rest.contains('{') {
        let parts: Vec<&str> = rest.splitn(2, '{').collect();
        let label_part = parts[0].trim();
        let prop_part = parts.get(1)
            .ok_or_else(|| anyhow!("Unclosed property block"))?
            .trim_end_matches('}')
            .trim();

        let labels = parse_labels(label_part)?;
        let properties = parse_properties(prop_part)?;
        (labels, properties)
    } else {
        let labels = parse_labels(rest)?;
        (labels, HashMap::new())
    };

    Ok(Query::CreateNode { labels, properties })
}

fn parse_create_edge(input: &str) -> Result<Query> {
    let input = input.trim();
    let rest = input.strip_prefix("CREATE EDGE").or_else(|| input.strip_prefix("create edge"))
        .ok_or_else(|| anyhow!("Invalid CREATE EDGE syntax"))?
        .trim();

    let (edge_pattern, properties) = if rest.contains('{') {
        let parts: Vec<&str> = rest.splitn(2, '{').collect();
        let pattern = parts[0].trim();
        let prop_part = parts.get(1)
            .ok_or_else(|| anyhow!("Unclosed property block"))?
            .trim_end_matches('}')
            .trim();

        let properties = parse_properties(prop_part)?;
        (pattern, properties)
    } else {
        (rest, HashMap::new())
    };

    if !edge_pattern.contains("->") {
        return Err(anyhow!("Edge pattern must contain '->' (e.g., (source_id)-[LABEL]->(target_id))"));
    }

    let parts: Vec<&str> = edge_pattern.split("->").collect();
    if parts.len() != 2 {
        return Err(anyhow!("Invalid edge pattern"));
    }

    let source_part = parts[0].trim();
    let target_part = parts[1].trim();

    let source_id = source_part
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(")-[")
        .next()
        .ok_or_else(|| anyhow!("Invalid source node ID"))?
        .trim()
        .to_string();

    let label = source_part
        .split(")-[")
        .nth(1)
        .ok_or_else(|| anyhow!("Invalid edge label"))?
        .trim_end_matches(']')
        .trim()
        .to_string();

    let target_id = target_part
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim()
        .to_string();

    Ok(Query::CreateEdge {
        source_id,
        target_id,
        label,
        properties,
    })
}

fn parse_match(input: &str) -> Result<Query> {
    let input = input.trim();
    let rest = input.strip_prefix("MATCH").or_else(|| input.strip_prefix("match"))
        .ok_or_else(|| anyhow!("Invalid MATCH syntax"))?
        .trim();

    if rest.starts_with('(') && rest.contains(')') {
        let node_pattern = rest
            .trim_start_matches('(')
            .split(')')
            .next()
            .ok_or_else(|| anyhow!("Invalid node pattern"))?
            .trim();

        let remaining = rest.split(')').skip(1).collect::<String>();

        let (label, property_filter) = if node_pattern.contains(':') {
            let parts: Vec<&str> = node_pattern.splitn(2, ':').collect();
            let label = Some(parts[1].trim().to_string());

            if remaining.trim().to_uppercase().starts_with("WHERE") {
                let where_clause = remaining.trim().strip_prefix("WHERE")
                    .or_else(|| remaining.trim().strip_prefix("where"))
                    .ok_or_else(|| anyhow!("Invalid WHERE clause"))?
                    .trim();

                let filter = parse_where_clause(where_clause)?;
                (label, Some(filter))
            } else {
                (label, None)
            }
        } else {
            (None, None)
        };

        Ok(Query::MatchNodes {
            label,
            property_filter,
        })
    } else if rest.starts_with('-') || rest.starts_with('[') {
        let label = if rest.contains('[') && rest.contains(']') {
            let label_str = rest
                .split('[')
                .nth(1)
                .ok_or_else(|| anyhow!("Invalid edge pattern"))?
                .split(']')
                .next()
                .ok_or_else(|| anyhow!("Invalid edge pattern"))?
                .trim();

            if label_str.is_empty() {
                None
            } else {
                Some(label_str.to_string())
            }
        } else {
            None
        };

        Ok(Query::MatchEdges { label })
    } else {
        Err(anyhow!("Invalid MATCH pattern. Use (:Label) for nodes or -[:LABEL]-> for edges"))
    }
}

fn parse_labels(input: &str) -> Result<Vec<String>> {
    let input = input.trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();

    if input.is_empty() {
        return Ok(Vec::new());
    }

    Ok(input
        .split(':')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect())
}

fn parse_properties(input: &str) -> Result<Property> {
    let mut properties = HashMap::new();

    if input.is_empty() {
        return Ok(properties);
    }

    for pair in input.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }

        let parts: Vec<&str> = pair.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid property format: '{}'", pair));
        }

        let key = parts[0].trim().trim_matches('"').to_string();
        let value_str = parts[1].trim();

        let value = parse_property_value(value_str)?;
        properties.insert(key, value);
    }

    Ok(properties)
}

fn parse_property_value(input: &str) -> Result<PropertyValue> {
    let input = input.trim();

    if input.starts_with('"') && input.ends_with('"') {
        let s = input.trim_matches('"').to_string();
        return Ok(PropertyValue::String(s));
    }

    if input == "true" {
        return Ok(PropertyValue::Boolean(true));
    }

    if input == "false" {
        return Ok(PropertyValue::Boolean(false));
    }

    if input == "null" {
        return Ok(PropertyValue::Null);
    }

    if let Ok(i) = input.parse::<i64>() {
        return Ok(PropertyValue::Integer(i));
    }

    if let Ok(f) = input.parse::<f64>() {
        return Ok(PropertyValue::Float(f));
    }

    Err(anyhow!("Cannot parse property value: '{}'", input))
}

fn parse_where_clause(input: &str) -> Result<(String, PropertyValue)> {
    let input = input.trim();

    if input.contains('=') {
        let parts: Vec<&str> = input.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid WHERE clause"));
        }

        let key = parts[0].trim().to_string();
        let value = parse_property_value(parts[1].trim())?;

        Ok((key, value))
    } else {
        Err(anyhow!("WHERE clause must use '=' operator"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_create_node() {
        let query = parse_query("CREATE NODE (:Person) {name: \"Alice\", age: 30}").unwrap();
        match query {
            Query::CreateNode { labels, properties } => {
                assert_eq!(labels, vec!["Person"]);
                assert_eq!(properties.get("name"), Some(&PropertyValue::String("Alice".to_string())));
                assert_eq!(properties.get("age"), Some(&PropertyValue::Integer(30)));
            }
            _ => panic!("Expected CreateNode"),
        }
    }

    #[test]
    fn test_parse_create_node_no_props() {
        let query = parse_query("CREATE NODE (:Person)").unwrap();
        match query {
            Query::CreateNode { labels, properties } => {
                assert_eq!(labels, vec!["Person"]);
                assert!(properties.is_empty());
            }
            _ => panic!("Expected CreateNode"),
        }
    }

    #[test]
    fn test_parse_create_edge() {
        let query = parse_query("CREATE EDGE (node1)-[KNOWS]->(node2) {since: 2020}").unwrap();
        match query {
            Query::CreateEdge { source_id, target_id, label, properties } => {
                assert_eq!(source_id, "node1");
                assert_eq!(target_id, "node2");
                assert_eq!(label, "KNOWS");
                assert_eq!(properties.get("since"), Some(&PropertyValue::Integer(2020)));
            }
            _ => panic!("Expected CreateEdge"),
        }
    }

    #[test]
    fn test_parse_match_nodes() {
        let query = parse_query("MATCH (:Person)").unwrap();
        match query {
            Query::MatchNodes { label, property_filter } => {
                assert_eq!(label, Some("Person".to_string()));
                assert_eq!(property_filter, None);
            }
            _ => panic!("Expected MatchNodes"),
        }
    }

    #[test]
    fn test_parse_match_nodes_with_where() {
        let query = parse_query("MATCH (:Person) WHERE name = \"Alice\"").unwrap();
        match query {
            Query::MatchNodes { label, property_filter } => {
                assert_eq!(label, Some("Person".to_string()));
                assert_eq!(
                    property_filter,
                    Some(("name".to_string(), PropertyValue::String("Alice".to_string())))
                );
            }
            _ => panic!("Expected MatchNodes"),
        }
    }

    #[test]
    fn test_parse_show_commands() {
        assert_eq!(parse_query("SHOW NODES").unwrap(), Query::ShowNodes);
        assert_eq!(parse_query("SHOW EDGES").unwrap(), Query::ShowEdges);
    }

    #[test]
    fn test_parse_exit() {
        assert_eq!(parse_query("EXIT").unwrap(), Query::Exit);
        assert_eq!(parse_query("QUIT").unwrap(), Query::Exit);
    }

    #[test]
    fn test_parse_help() {
        assert_eq!(parse_query("HELP").unwrap(), Query::Help);
    }
}
