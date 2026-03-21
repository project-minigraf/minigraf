use minigraf::graph::types::Value;
use minigraf::graph::FactStorage;
use minigraf::query::datalog::{DatalogExecutor, QueryResult};
use minigraf::query::datalog::parser::parse_datalog_command;
use uuid::Uuid;

/// Test simple transitive closure: A -> B -> C
#[test]
fn test_simple_transitive_closure() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Create chain: A -> B -> C
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    storage
        .transact(vec![
            (a, ":connected".to_string(), Value::Ref(b)),
            (b, ":connected".to_string(), Value::Ref(c)),
        ], None)
        .unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query: what can A reach?
    let query_str = format!(
        r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
        a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?to"]);
            assert_eq!(results.len(), 2);

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(targets.contains(&b));
            assert!(targets.contains(&c));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test transitive closure with cycle
#[test]
fn test_transitive_closure_with_cycle() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Create cycle: A -> B -> C -> A
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    storage
        .transact(vec![
            (a, ":connected".to_string(), Value::Ref(b)),
            (b, ":connected".to_string(), Value::Ref(c)),
            (c, ":connected".to_string(), Value::Ref(a)),
        ], None)
        .unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query: what can A reach? (should be B, C, and A itself)
    let query_str = format!(
        r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
        a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?to"]);
            assert_eq!(results.len(), 3); // B, C, A (cycle back to self)

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(targets.contains(&a)); // Can reach itself via cycle
            assert!(targets.contains(&b));
            assert!(targets.contains(&c));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test long chain (10 nodes)
#[test]
fn test_long_chain_transitive_closure() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Create chain: n0 -> n1 -> n2 -> ... -> n9
    let nodes: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();

    let mut facts = Vec::new();
    for i in 0..9 {
        facts.push((nodes[i], ":next".to_string(), Value::Ref(nodes[i + 1])));
    }
    storage.transact(facts, None).unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reachable ?x ?y) [?x :next ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reachable ?x ?y) [?x :next ?z] (reachable ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query: what can n0 reach? (should be all 9 others)
    let query_str = format!(
        r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
        nodes[0]
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?to"]);
            assert_eq!(results.len(), 9); // All nodes except n0

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            for i in 1..10 {
                assert!(targets.contains(&nodes[i]));
            }
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test ancestor relationship (family tree)
#[test]
fn test_ancestor_relationship() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Family tree: Alice -> Bob, Alice -> Charlie
    //              Bob -> Diana, Bob -> Eve
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let charlie = Uuid::new_v4();
    let diana = Uuid::new_v4();
    let eve = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":parent".to_string(), Value::Ref(bob)),
            (alice, ":parent".to_string(), Value::Ref(charlie)),
            (bob, ":parent".to_string(), Value::Ref(diana)),
            (bob, ":parent".to_string(), Value::Ref(eve)),
        ], None)
        .unwrap();

    // Register ancestor rules
    executor
        .execute(parse_datalog_command(r#"(rule [(ancestor ?a ?d) [?a :parent ?d]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(ancestor ?a ?d) [?a :parent ?p] (ancestor ?p ?d)])"#,
        ).unwrap())
        .unwrap();

    // Query: who are Alice's descendants?
    let query_str = format!(
        r#"(query [:find ?descendant :where (ancestor #uuid "{}" ?descendant)])"#,
        alice
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?descendant"]);
            assert_eq!(results.len(), 4); // Bob, Charlie (children), Diana, Eve (grandchildren)

            let descendants: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(descendants.contains(&bob));
            assert!(descendants.contains(&charlie));
            assert!(descendants.contains(&diana));
            assert!(descendants.contains(&eve));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test multiple recursive predicates in same database
#[test]
fn test_multiple_recursive_predicates() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    storage
        .transact(vec![
            (a, ":friend".to_string(), Value::Ref(b)),
            (b, ":friend".to_string(), Value::Ref(c)),
            (a, ":coworker".to_string(), Value::Ref(c)),
        ], None)
        .unwrap();

    // Register friend-reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(friend-reach ?x ?y) [?x :friend ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(friend-reach ?x ?y) [?x :friend ?z] (friend-reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Register coworker-reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(coworker-reach ?x ?y) [?x :coworker ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(coworker-reach ?x ?y) [?x :coworker ?z] (coworker-reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query: who can A reach via friends?
    let query1_str = format!(
        r#"(query [:find ?to :where (friend-reach #uuid "{}" ?to)])"#,
        a
    );
    let query1 = parse_datalog_command(&query1_str).unwrap();

    let result1 = executor.execute(query1).unwrap();
    match result1 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 2); // B and C via B

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(targets.contains(&b));
            assert!(targets.contains(&c));
        }
        _ => panic!("Expected QueryResults"),
    }

    // Query: who can A reach via coworkers?
    let query2_str = format!(
        r#"(query [:find ?to :where (coworker-reach #uuid "{}" ?to)])"#,
        a
    );
    let query2 = parse_datalog_command(&query2_str).unwrap();

    let result2 = executor.execute(query2).unwrap();
    match result2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1); // Only C directly

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(targets.contains(&c));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test recursive rule with constants in query
#[test]
fn test_recursive_rule_with_constants() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    storage
        .transact(vec![
            (a, ":connected".to_string(), Value::Ref(b)),
            (b, ":connected".to_string(), Value::Ref(c)),
        ], None)
        .unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query with both constants: can A reach C?
    let query_str = format!(
        r#"(query [:find ?x :where (reach #uuid "{}" #uuid "{}") [#uuid "{}" :connected ?x]])"#,
        a, c, a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { results, .. } => {
            // If A can reach C, and A connects to something, we should get results
            assert!(!results.is_empty());
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test diamond pattern in graph
#[test]
fn test_diamond_pattern_reachability() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Diamond: A -> B -> D
    //          A -> C -> D
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();

    storage
        .transact(vec![
            (a, ":connected".to_string(), Value::Ref(b)),
            (a, ":connected".to_string(), Value::Ref(c)),
            (b, ":connected".to_string(), Value::Ref(d)),
            (c, ":connected".to_string(), Value::Ref(d)),
        ], None)
        .unwrap();

    // Register reachable rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query: what can A reach?
    let query_str = format!(
        r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#,
        a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 3); // B, C, D

            let targets: Vec<Uuid> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::Ref(uuid) => *uuid,
                    _ => panic!("Expected Ref"),
                })
                .collect();

            assert!(targets.contains(&b));
            assert!(targets.contains(&c));
            assert!(targets.contains(&d));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test rule with no base facts (should return empty)
#[test]
fn test_recursive_rule_no_base_facts() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Register rules but no facts
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query with no matching facts
    let a = Uuid::new_v4();
    let query_str = format!(
        r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#,
        a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 0);
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test convergence with max iterations
#[test]
fn test_convergence_simple_graph() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Simple 2-node chain
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    storage
        .transact(vec![(a, ":connected".to_string(), Value::Ref(b))], None)
        .unwrap();

    // Register rules
    executor
        .execute(parse_datalog_command(r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#).unwrap())
        .unwrap();

    executor
        .execute(parse_datalog_command(
            r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
        ).unwrap())
        .unwrap();

    // Query should converge quickly
    let query_str = format!(
        r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#,
        a
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1); // Just B
        }
        _ => panic!("Expected QueryResults"),
    }
}
