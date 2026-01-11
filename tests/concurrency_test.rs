/// Thread safety and concurrency tests.
use minigraf::GraphStorage;
use std::sync::Arc;
use std::thread;

#[test]
fn test_concurrent_reads() {
    let storage = Arc::new(GraphStorage::new());

    // Pre-populate with some data
    let node1 = minigraf::Node::new(
        vec!["Person".to_string()],
        [("name".to_string(), minigraf::PropertyValue::String("Alice".to_string()))]
            .iter()
            .cloned()
            .collect(),
    );
    storage.create_node(&node1).unwrap();

    // Spawn multiple reader threads
    let mut handles = vec![];
    for i in 0..10 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            for _ in 0..100 {
                let nodes = storage_clone.get_all_nodes().unwrap();
                assert!(!nodes.is_empty(), "Thread {} found empty storage", i);
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_concurrent_writes() {
    let storage = Arc::new(GraphStorage::new());

    // Spawn multiple writer threads
    let mut handles = vec![];
    for i in 0..10 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            for j in 0..10 {
                let node = minigraf::Node::new(
                    vec!["Person".to_string()],
                    [(
                        "name".to_string(),
                        minigraf::PropertyValue::String(format!("Person-{}-{}", i, j)),
                    )]
                    .iter()
                    .cloned()
                    .collect(),
                );
                storage_clone.create_node(&node).unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all nodes were added
    let nodes = storage.get_all_nodes().unwrap();
    assert_eq!(nodes.len(), 100); // 10 threads * 10 nodes each
}

#[test]
fn test_concurrent_read_write() {
    let storage = Arc::new(GraphStorage::new());

    // Pre-populate
    for i in 0..10 {
        let node = minigraf::Node::new(
            vec!["Person".to_string()],
            [(
                "name".to_string(),
                minigraf::PropertyValue::String(format!("InitialPerson-{}", i)),
            )]
            .iter()
            .cloned()
            .collect(),
        );
        storage.create_node(&node).unwrap();
    }

    let mut handles = vec![];

    // Spawn reader threads
    for i in 0..5 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            for _ in 0..50 {
                let nodes = storage_clone.get_all_nodes().unwrap();
                assert!(nodes.len() >= 10, "Reader {} found too few nodes", i);
            }
        });
        handles.push(handle);
    }

    // Spawn writer threads
    for i in 0..5 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            for j in 0..10 {
                let node = minigraf::Node::new(
                    vec!["Person".to_string()],
                    [(
                        "name".to_string(),
                        minigraf::PropertyValue::String(format!("NewPerson-{}-{}", i, j)),
                    )]
                    .iter()
                    .cloned()
                    .collect(),
                );
                storage_clone.create_node(&node).unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify final count
    let nodes = storage.get_all_nodes().unwrap();
    assert_eq!(nodes.len(), 60); // 10 initial + (5 threads * 10 nodes)
}

#[test]
fn test_concurrent_edge_creation() {
    let storage = Arc::new(GraphStorage::new());

    // Create nodes first
    let node1 = minigraf::Node::new(vec!["Person".to_string()], Default::default());
    let node2 = minigraf::Node::new(vec!["Person".to_string()], Default::default());
    let id1 = node1.id.clone();
    let id2 = node2.id.clone();
    storage.create_node(&node1).unwrap();
    storage.create_node(&node2).unwrap();

    // Spawn threads to create edges
    let mut handles = vec![];
    for i in 0..10 {
        let storage_clone = Arc::clone(&storage);
        let id1_clone = id1.clone();
        let id2_clone = id2.clone();
        let handle = thread::spawn(move || {
            for j in 0..10 {
                let edge = minigraf::Edge::new(
                    id1_clone.clone(),
                    id2_clone.clone(),
                    format!("REL_{}", i),
                    [(
                        "index".to_string(),
                        minigraf::PropertyValue::Integer(j),
                    )]
                    .iter()
                    .cloned()
                    .collect(),
                );
                storage_clone.create_edge(&edge).unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all edges were added
    let edges = storage.get_all_edges().unwrap();
    assert_eq!(edges.len(), 100); // 10 threads * 10 edges each
}
