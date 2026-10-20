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

            for node in &nodes[1..10] {
                assert!(targets.contains(node));
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

/// #528, #553: `max_derived_facts` limits what the rules derive, not the size
/// of the database. A rule query must answer on a database with more live
/// facts than the limit.
#[test]
fn test_max_derived_facts_counts_derived_facts_not_base_facts() {
    let db = minigraf::OpenOptions::new()
        .max_derived_facts(10)
        .open_memory()
        .unwrap();
    // A chain a -> b -> c -> d, plus 50 unrelated facts: 53 live facts in all.
    exec(
        &db,
        "(transact [[:a :manager :b] [:b :manager :c] [:c :manager :d]])",
    );
    let filler: String = (0..50).map(|i| format!("[:p{i} :age {i}] ")).collect();
    exec(&db, &format!("(transact [{filler}])"));
    exec(&db, "(rule [(boss ?x ?y) [?x :manager ?y]])");
    exec(&db, "(rule [(boss ?x ?y) [?x :manager ?m] (boss ?m ?y)])");

    // Bound first argument (magic-sets path): b, c, d.
    match exec(&db, "(query [:find ?y :where (boss :a ?y)])") {
        QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 3),
        _ => panic!("Expected QueryResults"),
    }
    // All free: 6 pairs.
    match exec(&db, "(query [:find ?x ?y :where (boss ?x ?y)])") {
        QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 6),
        _ => panic!("Expected QueryResults"),
    }
    // The limit still stops a rule that derives more than it allows.
    let err = db
        .execute("(query [:find ?x ?y :where (boss ?x ?y) :max-derived-facts 5])")
        .unwrap_err();
    assert_eq!(err.code(), "INT-020");
}

/// 2030-01-01T00:00:00Z in Unix ms.
const VALID_AT_2030: i64 = 1_893_456_000_000;

fn result_rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

/// #617: a `:db/valid-at` pattern in a rule body binds the query's valid time,
/// not `nil` (which failed the head with INT-022).
#[test]
fn test_rule_body_valid_at_binds_query_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, "(transact [[:a :n 1]])");
    exec(&db, "(rule [(vt ?e ?t) [?e :n _] [?e :db/valid-at ?t]])");

    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?t :valid-at "2030-01-01" :where (vt ?e ?t)])"#,
    ));
    assert_eq!(rows, vec![vec![Value::Integer(VALID_AT_2030)]]);

    // Bound first argument: the magic-sets path.
    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?t :valid-at "2030-01-01" :where (vt :a ?t)])"#,
    ));
    assert_eq!(rows, vec![vec![Value::Integer(VALID_AT_2030)]]);

    // No :valid-at: the current time, as outside a rule.
    let rows = result_rows(exec(&db, "(query [:find ?t :where (vt ?e ?t)])"));
    assert_eq!(rows.len(), 1, "one row");
    assert!(
        matches!(rows[0][0], Value::Integer(t) if t > VALID_AT_2030 / 2),
        "current time bound"
    );
}

/// #617: the same in a recursive rule, evaluated semi-naively.
#[test]
fn test_recursive_rule_body_valid_at_binds_query_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, "(transact [[:a :next :b] [:b :next :c]])");
    exec(
        &db,
        "(rule [(reach ?x ?y) [?x :next ?y] [?x :db/valid-at ?t] [(> ?t 0)]])",
    );
    exec(&db, "(rule [(reach ?x ?y) [?x :next ?z] (reach ?z ?y)])");

    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?y :valid-at "2030-01-01" :where (reach :a ?y)])"#,
    ));
    assert_eq!(rows.len(), 2, ":a reaches :b and :c");
}

/// #617: a `not` body inside a rule binds the query's valid time, so it
/// excludes the entity whose `:t` equals it.
#[test]
fn test_rule_not_body_valid_at_binds_query_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    exec(
        &db,
        "(transact [[:a :t 1893456000000] [:a :n 1] [:b :t 5] [:b :n 2]])",
    );
    exec(
        &db,
        "(rule [(other ?e ?v) [?e :t ?t] [?e :n ?v] (not [?e :db/valid-at ?t])])",
    );

    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?v :valid-at "2030-01-01" :where (other ?e ?v)])"#,
    ));
    assert_eq!(rows, vec![vec![Value::Integer(2)]], "not body excludes :a");
}

/// #617: the same for a `not-join` body inside a rule.
#[test]
fn test_rule_not_join_body_valid_at_binds_query_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, "(transact [[:a :n 1] [:b :n 2]])");
    exec(
        &db,
        "(rule [(fresh ?e) [?e :n _] (not-join [?e] [?e :db/valid-at ?t] [(> ?t 0)])])",
    );

    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?e :valid-at "2030-01-01" :where (fresh ?e)])"#,
    ));
    assert_eq!(rows.len(), 0, "not-join body excludes both");
}

/// #617: an `or` branch inside a rule binds the query's valid time, not the
/// current time.
#[test]
fn test_rule_or_branch_valid_at_binds_query_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, "(transact [[:a :n 1] [:b :n 2]])");
    exec(
        &db,
        "(rule [(vt ?e ?t) [?e :n _] (or (and [?e :n 1] [?e :db/valid-at ?t]) (and [?e :n 2] [?e :db/valid-at ?t]))])",
    );

    let rows = result_rows(exec(
        &db,
        r#"(query [:find ?t :valid-at "2030-01-01" :where (vt ?e ?t)])"#,
    ));
    assert_eq!(
        rows,
        vec![vec![Value::Integer(VALID_AT_2030)]; 2],
        "one row per entity"
    );
}

/// #661: a per-fact pseudo-attribute in a rule body reads the stored fact's
/// metadata, as it does in a query body. Positive rules used to read the
/// evaluator's working copy: valid-from was the time the query ran,
/// valid-to forever, and tx-count / tx-id the copy's.
#[test]
fn test_pseudo_attributes_in_rule_bodies_read_the_stored_fact() {
    let db = Minigraf::in_memory().unwrap();
    exec(
        &db,
        r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2021-01-01T00:00:00Z"} [[:a :edge :b]])"#,
    );
    exec(&db, "(transact [[:b :edge :c]])");
    exec(&db, "(transact [[:c :edge :d]])");
    let direct = |attr: &str| {
        let mut rows = result_rows(exec(
            &db,
            &format!(
                "(query [:find ?x ?m :valid-at :any-valid-time :where [?x :edge ?y] [?x {attr} ?m]])"
            ),
        ));
        rows.sort();
        rows
    };
    for (i, attr) in [
        ":db/valid-from",
        ":db/valid-to",
        ":db/tx-count",
        ":db/tx-id",
    ]
    .into_iter()
    .enumerate()
    {
        // A positive rule, and a recursive one whose later iterations read
        // the base facts again.
        exec(
            &db,
            &format!("(rule [(m{i} ?x ?m) [?x :edge ?y] [?x {attr} ?m]])"),
        );
        exec(
            &db,
            &format!("(rule [(r{i} ?x ?m) [?x :edge ?y] [?x {attr} ?m]])"),
        );
        exec(
            &db,
            &format!("(rule [(r{i} ?x ?m) [?x :edge ?y] [?x {attr} ?m] (r{i} ?y ?n)])"),
        );
        let expected = direct(attr);
        assert_eq!(expected.len(), 3, "three :edge facts");
        for rule in [format!("m{i}"), format!("r{i}")] {
            let mut rows = result_rows(exec(
                &db,
                &format!("(query [:find ?x ?m :valid-at :any-valid-time :where ({rule} ?x ?m)])"),
            ));
            rows.sort();
            assert!(rows == expected, "a rule body reads the stored metadata");
        }
    }
}

/// The error code of a command that must fail.
fn err_code(db: &Minigraf, input: &str) -> String {
    match db.execute(input) {
        Ok(_) => panic!("expected an error for {input}"),
        Err(e) => e.code().to_string(),
    }
}

/// #548: a rule head whose entity is not a ref or a keyword fails the query
/// with QRY-013 on every path: a positive rule (was INT-022), the magic-sets
/// rewrite, and rules with `not` or `or` (which dropped the row).
#[test]
fn test_rule_head_entity_not_an_entity_is_qry013() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, r#"(transact [[:a :name "alice"] [:b :alias 7]])"#);
    exec(&db, "(rule [(named ?n ?e) [?e :name ?n]])");
    exec(
        &db,
        "(rule [(visible ?n ?e) [?e :name ?n] (not [?e :hidden true])])",
    );
    exec(
        &db,
        "(rule [(called ?n ?e) (or [?e :name ?n] [?e :alias ?n])])",
    );
    exec(
        &db,
        "(rule [(unnamed ?n ?e) [?e :name ?n] (not-join [?e] [?e :hidden true])])",
    );
    for query in [
        "(query [:find ?e :where (named ?n ?e)])",
        r#"(query [:find ?e :where (named "alice" ?e)])"#,
        "(query [:find ?e :where (visible ?n ?e)])",
        "(query [:find ?e :where (called ?n ?e)])",
        "(query [:find ?e :where (unnamed ?n ?e)])",
        "(query [:find ?e :where [?e :name _] (not-join [?e] (visible ?n ?e))])",
    ] {
        assert_eq!(err_code(&db, query), "QRY-013", "head entity is a value");
    }
}

/// #548: a keyword or ref head entity names an entity on every path.
#[test]
fn test_rule_head_keyword_and_ref_entities_derive() {
    let db = Minigraf::in_memory().unwrap();
    exec(
        &db,
        "(transact [[:a :likes :b] [:a :likes :c] [:c :hidden true]])",
    );
    exec(&db, "(rule [(liked ?y ?x) [?x :likes ?y]])");
    exec(
        &db,
        "(rule [(shown ?y ?x) [?x :likes ?y] (not [?y :hidden true])])",
    );
    let rows = result_rows(exec(&db, "(query [:find ?y :where (liked ?y ?x)])"));
    assert_eq!(rows.len(), 2, "two liked entities");
    let rows = result_rows(exec(&db, "(query [:find ?y :where (shown ?y ?x)])"));
    assert_eq!(rows.len(), 1, "one shown entity");
}

/// #548: a head variable the body does not bind fails with INT-022 on the
/// `not`/`or` path too, which dropped the row.
#[test]
fn test_rule_head_unbound_variable_is_int022_on_every_path() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, r#"(transact [[:a :name "alice"]])"#);
    exec(&db, "(rule [(loose ?x ?y) [?x :name _]])");
    exec(
        &db,
        "(rule [(looser ?x ?y) [?x :name _] (not [?x :hidden true])])",
    );
    for query in [
        "(query [:find ?x :where (loose ?x ?y)])",
        "(query [:find ?x :where (looser ?x ?y)])",
    ] {
        assert_eq!(err_code(&db, query), "INT-022", "unbound head variable");
    }
}

/// #548: a derived fact has no metadata. A per-fact pseudo-attribute whose
/// source matched one binds nil (the row is kept), and a constant never
/// matches it; a stored fact of the same attribute keeps its own. Facts
/// derived by positive rules and by rules with `not` alike.
#[test]
fn test_derived_facts_have_nil_metadata() {
    let db = Minigraf::in_memory().unwrap();
    exec(&db, "(transact [[:a :edge :b] [:b :edge :c]])");
    exec(&db, "(transact [[:z :reach :w]])");
    exec(&db, "(rule [(reach ?x ?y) [?x :edge ?y]])");
    exec(&db, "(rule [(reach ?x ?y) [?x :edge ?z] (reach ?z ?y)])");
    exec(&db, "(rule [(leaf ?x) [?y :edge ?x] (not [?x :edge _])])");

    for attr in [
        ":db/valid-from",
        ":db/valid-to",
        ":db/tx-count",
        ":db/tx-id",
    ] {
        let rows = result_rows(exec(
            &db,
            &format!(
                "(query [:find ?x ?y ?m :valid-at :any-valid-time :where (reach ?x ?y) [?x :reach ?y] [?x {attr} ?m]])"
            ),
        ));
        assert_eq!(rows.len(), 4, "three derived facts and a stored one");
        let nils = rows.iter().filter(|r| r[2] == Value::Null).count();
        assert_eq!(nils, 3, "derived facts bind nil");

        let rows = result_rows(exec(
            &db,
            &format!(
                "(query [:find ?x ?m :valid-at :any-valid-time :where (leaf ?x) [?x :leaf ?v] [?x {attr} ?m]])"
            ),
        ));
        assert_eq!(rows.len(), 1, "one leaf");
        assert_eq!(rows[0][1], Value::Null, "a derived leaf binds nil");
    }

    let rows = result_rows(exec(
        &db,
        "(query [:find ?x :valid-at :any-valid-time :where (reach ?x ?y) [?x :reach ?y] [?x :db/tx-count 2]])",
    ));
    assert_eq!(rows.len(), 1, "a constant matches the stored fact only");
}

/// #548: an `or` branch in a rule body knows the database's registered
/// predicates, not the built-ins only.
#[test]
fn test_rule_or_branch_uses_registered_predicates() {
    let db = Minigraf::in_memory().unwrap();
    db.register_predicate("big?", |v| matches!(v, Value::Integer(n) if *n > 100))
        .unwrap();
    exec(
        &db,
        "(transact [[:a :n 500] [:b :n 5] [:c :n 6] [:c :special true]])",
    );
    exec(
        &db,
        "(rule [(notable ?x) [?x :n ?v] (or [(big? ?v)] [?x :special true])])",
    );
    let rows = result_rows(exec(&db, "(query [:find ?x :where (notable ?x)])"));
    assert_eq!(rows.len(), 2, "a by the predicate, c by the branch pattern");
}
