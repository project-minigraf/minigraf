//! Integration tests for magic sets rewriting (#289).
//!
//! Asserts result *correctness* only — magic sets must never change query results.

use minigraf::{Minigraf, QueryResult, Value};
use uuid::Uuid;

fn open_db() -> Minigraf {
    Minigraf::in_memory().expect("open db")
}

fn exec(db: &Minigraf, cmd: &str) -> QueryResult {
    db.execute(cmd)
        .unwrap_or_else(|e| panic!("execution error: {}", e))
}

fn extract_refs(result: QueryResult) -> Vec<Uuid> {
    match result {
        QueryResult::QueryResults { results, .. } => results
            .into_iter()
            .map(|row| match &row[0] {
                Value::Ref(uuid) => *uuid,
                _ => panic!("Expected Ref in result row"),
            })
            .collect(),
        _ => panic!("Expected QueryResults"),
    }
}

/// Transitive closure with bound start: only reachable nodes returned.
#[test]
fn test_bound_start_transitive_closure() {
    let db = open_db();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :edge #uuid "{}"]
                           [#uuid "{}" :edge #uuid "{}"]
                           [#uuid "{}" :edge #uuid "{}"]])"#,
            a, b, b, c, c, d
        ),
    );
    exec(&db, r#"(rule [(reach ?x ?y) [?x :edge ?y]])"#);
    exec(&db, r#"(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])"#);

    let result = exec(
        &db,
        &format!(r#"(query [:find ?y :where (reach #uuid "{}" ?y)])"#, a),
    );
    let targets = extract_refs(result);

    assert!(targets.contains(&b), "should reach b");
    assert!(targets.contains(&c), "should reach c");
    assert!(targets.contains(&d), "should reach d");
    assert!(!targets.contains(&a), "should not reach a (no self-loop)");
}

/// All-free transitive closure: magic sets skipped, full result returned correctly.
#[test]
fn test_all_free_transitive_closure() {
    let db = open_db();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :edge #uuid "{}"]
                           [#uuid "{}" :edge #uuid "{}"]])"#,
            a, b, b, c
        ),
    );
    exec(&db, r#"(rule [(reach ?x ?y) [?x :edge ?y]])"#);
    exec(&db, r#"(rule [(reach ?x ?z) (reach ?x ?y) [?y :edge ?z]])"#);

    let result = exec(&db, r#"(query [:find ?x ?y :where (reach ?x ?y)])"#);
    match result {
        QueryResult::QueryResults { results, .. } => {
            // Should have: (a,b), (b,c), (a,c)
            assert_eq!(results.len(), 3, "expected 3 pairs in full closure");

            let pairs: Vec<(Uuid, Uuid)> = results
                .into_iter()
                .map(|row| {
                    let from = match &row[0] {
                        Value::Ref(u) => *u,
                        _ => panic!("Expected Ref for ?x"),
                    };
                    let to = match &row[1] {
                        Value::Ref(u) => *u,
                        _ => panic!("Expected Ref for ?y"),
                    };
                    (from, to)
                })
                .collect();

            assert!(pairs.contains(&(a, b)), "a->b expected");
            assert!(pairs.contains(&(b, c)), "b->c expected");
            assert!(pairs.contains(&(a, c)), "a->c expected (transitive)");
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Bound result is a subset of all-free result — no extra nodes returned.
#[test]
fn test_bound_result_subset_of_all_free() {
    let db = open_db();

    let x = Uuid::new_v4();
    let y = Uuid::new_v4();
    let z = Uuid::new_v4();
    let p = Uuid::new_v4();
    let q = Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :link #uuid "{}"]
                           [#uuid "{}" :link #uuid "{}"]
                           [#uuid "{}" :link #uuid "{}"]])"#,
            x, y, y, z, p, q
        ),
    );
    exec(&db, r#"(rule [(conn ?a ?b) [?a :link ?b]])"#);
    exec(&db, r#"(rule [(conn ?a ?c) (conn ?a ?b) [?b :link ?c]])"#);

    let bound_result = exec(
        &db,
        &format!(r#"(query [:find ?b :where (conn #uuid "{}" ?b)])"#, x),
    );
    let bound_targets = extract_refs(bound_result);

    assert!(bound_targets.contains(&y), "y is reachable from x");
    assert!(bound_targets.contains(&z), "z is reachable from x");
    assert!(!bound_targets.contains(&q), "q is unreachable from x");

    let all_free_result = exec(&db, r#"(query [:find ?a ?b :where (conn ?a ?b)])"#);
    match all_free_result {
        QueryResult::QueryResults { results, .. } => {
            let all_targets: Vec<Uuid> = results
                .into_iter()
                .map(|row| match &row[1] {
                    Value::Ref(u) => *u,
                    _ => panic!("Expected Ref"),
                })
                .collect();
            assert!(
                all_targets.contains(&q),
                "q is reachable from p in full closure"
            );
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Multi-hop: 4 levels of recursion with a bound start.
#[test]
fn test_multi_hop_recursion_with_bound_start() {
    let db = open_db();

    let nodes: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

    let mut transact = String::from("(transact [");
    for i in 0..4 {
        transact.push_str(&format!(
            r#"[#uuid "{}" :hop #uuid "{}"]"#,
            nodes[i],
            nodes[i + 1]
        ));
    }
    transact.push_str("])");
    exec(&db, &transact);

    exec(&db, r#"(rule [(path ?x ?y) [?x :hop ?y]])"#);
    exec(&db, r#"(rule [(path ?x ?z) (path ?x ?y) [?y :hop ?z]])"#);

    let result = exec(
        &db,
        &format!(
            r#"(query [:find ?y :where (path #uuid "{}" ?y)])"#,
            nodes[0]
        ),
    );
    let targets = extract_refs(result);

    assert!(targets.contains(&nodes[1]), "should reach node 1");
    assert!(targets.contains(&nodes[2]), "should reach node 2");
    assert!(targets.contains(&nodes[3]), "should reach node 3");
    assert!(targets.contains(&nodes[4]), "should reach node 4");
    assert!(
        !targets.contains(&nodes[0]),
        "should not reach node 0 (no self-loop)"
    );
}

/// Mutual recursion: even/odd distance from a seeded node.
#[test]
fn test_mutual_recursion_even_odd_distance() {
    let db = open_db();

    // Chain: n0 → n1 → n2 → n3 → n4; mark n0 as the even-distance seed
    let nodes: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

    let mut transact = format!(r#"(transact [[#uuid "{}" :is-start true]"#, nodes[0]);
    for i in 0..4 {
        transact.push_str(&format!(
            r#" [#uuid "{}" :next #uuid "{}"]"#,
            nodes[i],
            nodes[i + 1]
        ));
    }
    transact.push_str("])");
    exec(&db, &transact);

    exec(&db, r#"(rule [(even-d ?x) [?x :is-start true]])"#);
    exec(&db, r#"(rule [(even-d ?y) (odd-d ?x) [?x :next ?y]])"#);
    exec(&db, r#"(rule [(odd-d ?y) (even-d ?x) [?x :next ?y]])"#);

    let evens = extract_refs(exec(&db, r#"(query [:find ?x :where (even-d ?x)])"#));
    let odds = extract_refs(exec(&db, r#"(query [:find ?x :where (odd-d ?x)])"#));

    assert!(evens.contains(&nodes[0]), "n0 should be even-distance");
    assert!(evens.contains(&nodes[2]), "n2 should be even-distance");
    assert!(evens.contains(&nodes[4]), "n4 should be even-distance");
    assert!(!evens.contains(&nodes[1]), "n1 should not be even-distance");

    assert!(odds.contains(&nodes[1]), "n1 should be odd-distance");
    assert!(odds.contains(&nodes[3]), "n3 should be odd-distance");
    assert!(!odds.contains(&nodes[0]), "n0 should not be odd-distance");
    assert!(!odds.contains(&nodes[2]), "n2 should not be odd-distance");
    assert!(!odds.contains(&nodes[4]), "n4 should not be odd-distance");
}
