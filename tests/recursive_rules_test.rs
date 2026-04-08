use minigraf::{Minigraf, QueryResult, Value};
use uuid::Uuid;

/// Helper: execute a Datalog command via `Minigraf::execute`, panicking on error
fn exec(db: &Minigraf, input: &str) -> QueryResult {
    db.execute(input)
        .unwrap_or_else(|e| panic!("execution error for {:?}: {}", input, e))
}

/// Test simple transitive closure: A -> B -> C
#[test]
fn test_simple_transitive_closure() {
    let db = Minigraf::in_memory().unwrap();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]])"#,
            a, b, b, c
        ),
    );

    // Register reachable rules
    exec(&db, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
    );

    // Query: what can A reach?
    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
            a
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]])"#,
            a, b, b, c, c, a
        ),
    );

    // Register reachable rules
    exec(&db, r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#,
    );

    // Query: what can A reach? (should be B, C, and A itself via cycle)
    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
            a
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let nodes: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();

    let mut transact = String::from("(transact [");
    for i in 0..9 {
        transact.push_str(&format!(
            r#"[#uuid "{}" :next #uuid "{}"]"#,
            nodes[i],
            nodes[i + 1]
        ));
    }
    transact.push_str("])");
    exec(&db, &transact);

    // Register reachable rules
    exec(&db, r#"(rule [(reachable ?x ?y) [?x :next ?y]])"#);
    exec(
        &db,
        r#"(rule [(reachable ?x ?y) [?x :next ?z] (reachable ?z ?y)])"#,
    );

    // Query: what can n0 reach? (should be all 9 others)
    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?to :where (reachable #uuid "{}" ?to)])"#,
            nodes[0]
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let charlie = Uuid::new_v4();
    let diana = Uuid::new_v4();
    let eve = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :parent #uuid "{}"]
                           [#uuid "{}" :parent #uuid "{}"]
                           [#uuid "{}" :parent #uuid "{}"]
                           [#uuid "{}" :parent #uuid "{}"]])"#,
            alice, bob, alice, charlie, bob, diana, bob, eve
        ),
    );

    // Register ancestor rules
    exec(&db, r#"(rule [(ancestor ?a ?d) [?a :parent ?d]])"#);
    exec(
        &db,
        r#"(rule [(ancestor ?a ?d) [?a :parent ?p] (ancestor ?p ?d)])"#,
    );

    // Query: who are Alice's descendants?
    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?descendant :where (ancestor #uuid "{}" ?descendant)])"#,
            alice
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :friend #uuid "{}"]
                           [#uuid "{}" :friend #uuid "{}"]
                           [#uuid "{}" :coworker #uuid "{}"]])"#,
            a, b, b, c, a, c
        ),
    );

    // Register friend-reachable rules
    exec(&db, r#"(rule [(friend-reach ?x ?y) [?x :friend ?y]])"#);
    exec(
        &db,
        r#"(rule [(friend-reach ?x ?y) [?x :friend ?z] (friend-reach ?z ?y)])"#,
    );

    // Register coworker-reachable rules
    exec(&db, r#"(rule [(coworker-reach ?x ?y) [?x :coworker ?y]])"#);
    exec(
        &db,
        r#"(rule [(coworker-reach ?x ?y) [?x :coworker ?z] (coworker-reach ?z ?y)])"#,
    );

    // Query: who can A reach via friends?
    let result1 = exec(
        &db,
        &format!(
            r#"(query [:find ?to :where (friend-reach #uuid "{}" ?to)])"#,
            a
        ),
    );
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
    let result2 = exec(
        &db,
        &format!(
            r#"(query [:find ?to :where (coworker-reach #uuid "{}" ?to)])"#,
            a
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]])"#,
            a, b, b, c
        ),
    );

    // Register reachable rules
    exec(&db, r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
    );

    // Query with both constants: can A reach C?
    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?x :where (reach #uuid "{}" #uuid "{}") [#uuid "{}" :connected ?x]])"#,
            a, c, a
        ),
    );
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
    let db = Minigraf::in_memory().unwrap();

    // Diamond: A -> B -> D
    //          A -> C -> D
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]
                           [#uuid "{}" :connected #uuid "{}"]])"#,
            a, b, a, c, b, d, c, d
        ),
    );

    // Register reachable rules
    exec(&db, r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
    );

    // Query: what can A reach?
    let result = exec(
        &db,
        &format!(r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#, a),
    );
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
    let db = Minigraf::in_memory().unwrap();

    // Register rules but no facts
    exec(&db, r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
    );

    // Query with no matching facts
    let a = Uuid::new_v4();
    let result = exec(
        &db,
        &format!(r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#, a),
    );
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
    let db = Minigraf::in_memory().unwrap();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    exec(
        &db,
        &format!(r#"(transact [[#uuid "{}" :connected #uuid "{}"]])"#, a, b),
    );

    // Register rules
    exec(&db, r#"(rule [(reach ?x ?y) [?x :connected ?y]])"#);
    exec(
        &db,
        r#"(rule [(reach ?x ?y) [?x :connected ?z] (reach ?z ?y)])"#,
    );

    // Query should converge quickly
    let result = exec(
        &db,
        &format!(r#"(query [:find ?to :where (reach #uuid "{}" ?to)])"#, a),
    );
    match result {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1); // Just B
        }
        _ => panic!("Expected QueryResults"),
    }
}
