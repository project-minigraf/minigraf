/// Comprehensive edge case and error handling tests.

use minigraf::{Minigraf, GraphStorage, Node, Edge, PropertyValue};
use std::collections::HashMap;
use std::fs;

#[test]
fn test_create_edge_with_invalid_source() {
    let path = "/tmp/test_invalid_source.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Try to create edge with non-existent source
    let result = db.execute("CREATE EDGE (nonexistent)-[KNOWS]->(alsononexistent)");
    assert!(result.is_err());

    fs::remove_file(path).unwrap();
}

#[test]
fn test_empty_database_queries() {
    let path = "/tmp/test_empty_db.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Query empty database
    let result = db.execute("MATCH (:Person)").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 0);
        }
        _ => panic!("Expected Nodes result"),
    }

    let result = db.execute("SHOW NODES").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 0);
        }
        _ => panic!("Expected Nodes result"),
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_match_nonexistent_label() {
    let path = "/tmp/test_nonexistent_label.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Create a node
    db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();

    // Query for non-existent label
    let result = db.execute("MATCH (:Company)").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 0);
        }
        _ => panic!("Expected Nodes result"),
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_where_nonexistent_property() {
    let path = "/tmp/test_nonexistent_property.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Create a node
    db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();

    // Query for non-existent property
    let result = db.execute("MATCH (:Person) WHERE age = 30").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 0);
        }
        _ => panic!("Expected Nodes result"),
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_unicode_properties() {
    let path = "/tmp/test_unicode.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Create node with Unicode properties
    db.execute("CREATE NODE (:Person) {name: \"Alice\", city: \"東京\"}").unwrap();

    let result = db.execute("SHOW NODES").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 1);
            let city = nodes[0].properties.get("city").unwrap();
            if let PropertyValue::String(s) = city {
                assert_eq!(s, "東京");
            }
        }
        _ => panic!("Expected Nodes result"),
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_reopen_after_save() {
    let path = "/tmp/test_reopen.graph";
    let _ = fs::remove_file(path);

    // Create and explicitly save
    {
        let mut db = Minigraf::open(path).unwrap();
        db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();
        db.save().unwrap();
    }

    // Reopen immediately
    {
        let mut db = Minigraf::open(path).unwrap();
        let result = db.execute("SHOW NODES").unwrap();
        match result {
            minigraf::query::executor::QueryResult::Nodes(nodes) => {
                assert_eq!(nodes.len(), 1);
            }
            _ => panic!("Expected Nodes result"),
        }
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_multiple_saves() {
    let path = "/tmp/test_multiple_saves.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Save multiple times
    db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();
    db.save().unwrap();

    db.execute("CREATE NODE (:Person) {name: \"Bob\"}").unwrap();
    db.save().unwrap();

    db.execute("CREATE NODE (:Person) {name: \"Charlie\"}").unwrap();
    db.save().unwrap();

    let stats = db.stats();
    assert_eq!(stats.node_count, 3);

    fs::remove_file(path).unwrap();
}

#[test]
fn test_large_property_values() {
    let path = "/tmp/test_large_props.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    // Create node with large string property (but within page size)
    let large_string = "A".repeat(1000);
    let query = format!("CREATE NODE (:Test) {{data: \"{}\"}}", large_string);
    db.execute(&query).unwrap();

    let result = db.execute("SHOW NODES").unwrap();
    match result {
        minigraf::query::executor::QueryResult::Nodes(nodes) => {
            assert_eq!(nodes.len(), 1);
            let data = nodes[0].properties.get("data").unwrap();
            if let PropertyValue::String(s) = data {
                assert_eq!(s.len(), 1000);
            }
        }
        _ => panic!("Expected Nodes result"),
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn test_dirty_flag_behavior() {
    let path = "/tmp/test_dirty_flag.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();
    assert!(!db.is_dirty()); // New database, not dirty

    db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();
    assert!(db.is_dirty()); // After modification, should be dirty

    db.save().unwrap();
    assert!(!db.is_dirty()); // After save, no longer dirty

    db.execute("CREATE NODE (:Person) {name: \"Bob\"}").unwrap();
    assert!(db.is_dirty()); // Dirty again

    fs::remove_file(path).unwrap();
}

#[test]
fn test_stats_accuracy() {
    let path = "/tmp/test_stats.graph";
    let _ = fs::remove_file(path);

    let mut db = Minigraf::open(path).unwrap();

    let stats = db.stats();
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);

    db.execute("CREATE NODE (:Person) {name: \"Alice\"}").unwrap();
    db.execute("CREATE NODE (:Person) {name: \"Bob\"}").unwrap();

    let stats = db.stats();
    assert_eq!(stats.node_count, 2);
    assert_eq!(stats.edge_count, 0);

    // Get node IDs for edge creation
    let nodes = db.nodes();
    if nodes.len() >= 2 {
        let query = format!("CREATE EDGE ({})-[KNOWS]->({})", nodes[0].id, nodes[1].id);
        db.execute(&query).unwrap();

        let stats = db.stats();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 1);
    }

    fs::remove_file(path).unwrap();
}
