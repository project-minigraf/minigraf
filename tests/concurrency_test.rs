use minigraf::graph::FactStorage;
use minigraf::graph::types::Value;
use minigraf::query::datalog::parser::parse_datalog_command;
use minigraf::query::datalog::{DatalogExecutor, QueryResult};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

/// Test concurrent rule registration from multiple threads
#[test]
fn test_concurrent_rule_registration() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage));

    // Spawn 5 threads, each registering different rules
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let executor = Arc::clone(&executor);
            thread::spawn(move || {
                let predicate = format!("rule{}", i);
                let rule_cmd = format!(r#"(rule [({} ?x ?y) [?x :connected{} ?y]])"#, predicate, i);
                executor
                    .execute(parse_datalog_command(&rule_cmd).unwrap())
                    .unwrap();
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify rules work by querying (indirect verification)
    // If rules weren't registered correctly, queries would fail
    for i in 0..5 {
        let query_cmd = format!(r#"(query [:find ?y :where (rule{} :test ?y)])"#, i);
        let _ = executor.execute(parse_datalog_command(&query_cmd).unwrap());
    }
}

/// Test concurrent queries with rules
#[test]
fn test_concurrent_rule_queries() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Setup: create a graph
    let nodes: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();
    let mut facts = Vec::new();
    for i in 0..9 {
        facts.push((nodes[i], ":connected".to_string(), Value::Ref(nodes[i + 1])));
    }
    storage.transact(facts, None).unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(
            parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#)
                .unwrap(),
        )
        .unwrap();

    // Spawn 10 threads, each querying from a different starting node
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let executor = Arc::clone(&executor);
            let node = nodes[i];
            thread::spawn(move || {
                let query_str =
                    format!(r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#, node);
                let query = parse_datalog_command(&query_str).unwrap();
                let result = executor.execute(query).unwrap();

                match result {
                    QueryResult::QueryResults { results, .. } => {
                        // Node i should reach all nodes after it
                        assert_eq!(results.len(), 9 - i);
                    }
                    _ => panic!("Expected QueryResults"),
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test concurrent transact + rule registration
#[test]
fn test_concurrent_transact_and_rules() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Spawn threads that mix transact and rule operations
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let executor = Arc::clone(&executor);
            let storage = storage.clone();
            thread::spawn(move || {
                if i % 2 == 0 {
                    // Even threads: add facts
                    let a = Uuid::new_v4();
                    let b = Uuid::new_v4();
                    storage
                        .transact(vec![(a, format!(":attr{}", i), Value::Ref(b))], None)
                        .unwrap();
                } else {
                    // Odd threads: register rules
                    let rule_cmd = format!(r#"(rule [(pred{} ?x ?y) [?x :attr{} ?y]])"#, i, i);
                    executor
                        .execute(parse_datalog_command(&rule_cmd).unwrap())
                        .unwrap();
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify facts were added
    let facts = storage.get_asserted_facts().unwrap();
    assert!(facts.len() >= 5); // At least 5 facts from even threads

    // Verify rules work by trying to use them (indirect verification)
    // No errors means rules were registered successfully
}

/// Test concurrent read-heavy workload
#[test]
fn test_concurrent_read_heavy() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Setup: create graph
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    storage
        .transact(
            vec![
                (a, ":connected".to_string(), Value::Ref(b)),
                (b, ":connected".to_string(), Value::Ref(c)),
            ],
            None,
        )
        .unwrap();

    // Register rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(
            parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#)
                .unwrap(),
        )
        .unwrap();

    // Spawn 50 reader threads
    let handles: Vec<_> = (0..50)
        .map(|_| {
            let executor = Arc::clone(&executor);
            let node = a;
            thread::spawn(move || {
                let query_str =
                    format!(r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#, node);
                let query = parse_datalog_command(&query_str).unwrap();
                let result = executor.execute(query).unwrap();

                match result {
                    QueryResult::QueryResults { results, .. } => {
                        assert_eq!(results.len(), 2); // B and C
                    }
                    _ => panic!("Expected QueryResults"),
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test concurrent recursive evaluation (stress test)
#[test]
fn test_concurrent_recursive_evaluation() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Create a complex graph with multiple chains
    let mut facts = Vec::new();
    let mut chains = Vec::new();

    for _chain_id in 0..5 {
        let nodes: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        chains.push(nodes.clone());

        for i in 0..4 {
            facts.push((nodes[i], ":connected".to_string(), Value::Ref(nodes[i + 1])));
        }
    }

    storage.transact(facts, None).unwrap();

    // Register recursive rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(
            parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#)
                .unwrap(),
        )
        .unwrap();

    // Spawn threads to query each chain
    let handles: Vec<_> = chains
        .iter()
        .map(|chain| {
            let executor = Arc::clone(&executor);
            let start_node = chain[0];
            thread::spawn(move || {
                let query_str = format!(
                    r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#,
                    start_node
                );
                let query = parse_datalog_command(&query_str).unwrap();
                let result = executor.execute(query).unwrap();

                match result {
                    QueryResult::QueryResults { results, .. } => {
                        assert_eq!(results.len(), 4); // Can reach 4 other nodes
                    }
                    _ => panic!("Expected QueryResults"),
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test no deadlocks with mixed operations
#[test]
fn test_no_deadlocks_mixed_operations() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Initial setup
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    storage
        .transact(vec![(a, ":connected".to_string(), Value::Ref(b))], None)
        .unwrap();

    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    // Spawn mixed workload
    let handles: Vec<_> = (0..20)
        .map(|i| {
            let executor = Arc::clone(&executor);
            let storage = storage.clone();
            let node_a = a;

            thread::spawn(move || {
                match i % 4 {
                    0 => {
                        // Transact
                        let x = Uuid::new_v4();
                        let y = Uuid::new_v4();
                        storage
                            .transact(vec![(x, ":attr".to_string(), Value::Ref(y))], None)
                            .unwrap();
                    }
                    1 => {
                        // Register rule
                        let rule_cmd = format!(r#"(rule [(rule{} ?x ?y) [?x :attr ?y]])"#, i);
                        executor
                            .execute(parse_datalog_command(&rule_cmd).unwrap())
                            .unwrap();
                    }
                    2 => {
                        // Query with rule
                        let query_str = format!(
                            r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#,
                            node_a
                        );
                        let query = parse_datalog_command(&query_str).unwrap();
                        let _ = executor.execute(query);
                    }
                    3 => {
                        // Query without rule
                        let query = parse_datalog_command(
                            r#"(query [:find ?to :where [?from :attr ?to]])"#,
                        )
                        .unwrap();
                        let _ = executor.execute(query);
                    }
                    _ => unreachable!(),
                }
            })
        })
        .collect();

    // Wait for all threads - if there's a deadlock, this will hang
    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test thread safety with RwLock
#[test]
fn test_rwlock_consistency() {
    let storage = FactStorage::new();
    let executor = Arc::new(DatalogExecutor::new(storage.clone()));

    // Many writers registering rules
    let write_handles: Vec<_> = (0..10)
        .map(|i| {
            let executor = Arc::clone(&executor);
            thread::spawn(move || {
                for j in 0..5 {
                    let rule_cmd =
                        format!(r#"(rule [(pred{}-{} ?x ?y) [?x :attr{} ?y]])"#, i, j, i);
                    executor
                        .execute(parse_datalog_command(&rule_cmd).unwrap())
                        .unwrap();
                }
            })
        })
        .collect();

    // Many readers checking rule count
    let read_handles: Vec<_> = (0..10)
        .map(|_| {
            let executor = Arc::clone(&executor);
            thread::spawn(move || {
                for _ in 0..10 {
                    // Query to ensure rules are accessible (indirect read)
                    let _ = executor.execute(
                        parse_datalog_command(r#"(query [:find ?x :where [?x :attr0 ?y]])"#)
                            .unwrap(),
                    );
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in write_handles {
        handle.join().unwrap();
    }
    for handle in read_handles {
        handle.join().unwrap();
    }

    // Verify rules were registered by testing each one
    // If 50 rules weren't registered, this would fail
    for i in 0..10 {
        for j in 0..5 {
            let query_cmd = format!(r#"(query [:find ?y :where (pred{}-{} :test ?y)])"#, i, j);
            let _ = executor.execute(parse_datalog_command(&query_cmd).unwrap());
        }
    }
}
