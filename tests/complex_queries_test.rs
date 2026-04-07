use minigraf::{Minigraf, QueryResult, Value};
use uuid::Uuid;

/// Helper: execute a Datalog command via `Minigraf::execute`, panicking on error
fn exec(db: &Minigraf, input: &str) -> QueryResult {
    db.execute(input)
        .unwrap_or_else(|e| panic!("execution error for {:?}: {}", input, e))
}

/// Test 3-pattern join: find entities with name, age, and city
#[test]
fn test_three_pattern_join() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice :person/name "Alice"]
                       [:alice :person/age 30]
                       [:alice :person/city "NYC"]])"#,
    );

    let result = exec(
        &db,
        r#"(query [:find ?name ?age ?city
                   :where [?e :person/name ?name]
                          [?e :person/age ?age]
                          [?e :person/city ?city]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name", "?age", "?city"]);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
            assert_eq!(results[0][1], Value::Integer(30));
            assert_eq!(results[0][2], Value::String("NYC".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test 4-pattern join with multiple entities
#[test]
fn test_four_pattern_join() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice :person/name "Alice"]
                       [:alice :person/age 30]
                       [:bob   :person/name "Bob"]
                       [:bob   :person/age 25]
                       [:alice :friend :bob]])"#,
    );

    // Find pairs where person1 friends person2
    let result = exec(
        &db,
        r#"(query [:find ?name1 ?age1 ?name2 ?age2
                   :where [?p1 :person/name ?name1]
                          [?p1 :person/age ?age1]
                          [?p1 :friend ?p2]
                          [?p2 :person/name ?name2]
                          [?p2 :person/age ?age2]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars.len(), 4);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
            assert_eq!(results[0][1], Value::Integer(30));
            assert_eq!(results[0][2], Value::String("Bob".to_string()));
            assert_eq!(results[0][3], Value::Integer(25));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test self-join: find friends of friends
#[test]
fn test_self_join_friends_of_friends() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice   :person/name "Alice"]
                       [:bob     :person/name "Bob"]
                       [:charlie :person/name "Charlie"]
                       [:alice   :friend :bob]
                       [:bob     :friend :charlie]])"#,
    );

    // Find friends of Alice's friends (Bob's friends)
    let result = exec(
        &db,
        r#"(query [:find ?name
                   :where [:alice :friend ?friend]
                          [?friend :friend ?fof]
                          [?fof :person/name ?name]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name"]);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Charlie".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test entity reference join: find people working at specific company
#[test]
fn test_entity_reference_join() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice   :person/name "Alice"]
                       [:bob     :person/name "Bob"]
                       [:techcorp :company/name "TechCorp"]
                       [:alice   :works-at :techcorp]
                       [:bob     :works-at :techcorp]])"#,
    );

    // Find people working at TechCorp
    let result = exec(
        &db,
        r#"(query [:find ?person-name
                   :where [?person :works-at ?company]
                          [?company :company/name "TechCorp"]
                          [?person :person/name ?person-name]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?person-name"]);
            assert_eq!(results.len(), 2);

            let names: Vec<String> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected String"),
                })
                .collect();

            assert!(names.contains(&"Alice".to_string()));
            assert!(names.contains(&"Bob".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test query with no results
#[test]
fn test_query_no_results() {
    let db = Minigraf::in_memory().unwrap();

    exec(&db, r#"(transact [[:alice :person/name "Alice"]])"#);

    // Query for non-existent attribute
    let result = exec(
        &db,
        r#"(query [:find ?email :where [?e :person/email ?email]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?email"]);
            assert_eq!(results.len(), 0);
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test query with partial matches (some entities match, some don't)
#[test]
fn test_query_partial_matches() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice :person/name "Alice"]
                       [:alice :person/age 30]
                       [:bob   :person/name "Bob"]])"#,
    );

    // Query for name AND age - should only return Alice
    let result = exec(
        &db,
        r#"(query [:find ?name ?age
                   :where [?e :person/name ?name]
                          [?e :person/age ?age]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars.len(), 2);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test query with same variable used multiple times
#[test]
fn test_query_variable_reuse() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice :person/name "Alice"]
                       [:alice :person/nickname "Alice"]])"#,
    );

    // Find people whose name equals their nickname
    let result = exec(
        &db,
        r#"(query [:find ?name
                   :where [?e :person/name ?name]
                          [?e :person/nickname ?name]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name"]);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test query with multiple entities, complex filtering
#[test]
fn test_complex_multi_entity_query() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice    :person/name "Alice"]
                       [:bob      :person/name "Bob"]
                       [:charlie  :person/name "Charlie"]
                       [:project1 :project/name "Project X"]
                       [:project2 :project/name "Project Y"]
                       [:alice    :works-on :project1]
                       [:bob      :works-on :project1]
                       [:charlie  :works-on :project2]
                       [:alice    :manages :project1]])"#,
    );

    // Find projects that Alice manages, along with other people working on them
    let result = exec(
        &db,
        r#"(query [:find ?project-name ?coworker-name
                   :where [:alice :manages ?project]
                          [?project :project/name ?project-name]
                          [?coworker :works-on ?project]
                          [?coworker :person/name ?coworker-name]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars.len(), 2);
            // Should find Alice and Bob working on Project X
            assert_eq!(results.len(), 2);

            for row in &results {
                assert_eq!(row[0], Value::String("Project X".to_string()));
                match &row[1] {
                    Value::String(name) => {
                        assert!(name == "Alice" || name == "Bob");
                    }
                    _ => panic!("Expected String"),
                }
            }
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test query returning multiple variable bindings per entity
#[test]
fn test_multiple_values_same_attribute() {
    let db = Minigraf::in_memory().unwrap();

    exec(
        &db,
        r#"(transact [[:alice :person/name "Alice"]
                       [:alice :hobby "Reading"]])"#,
    );
    exec(&db, r#"(transact [[:alice :hobby "Hiking"]])"#);
    exec(&db, r#"(transact [[:alice :hobby "Coding"]])"#);

    // Find all hobbies (should get 3 separate results)
    let result = exec(
        &db,
        r#"(query [:find ?hobby
                   :where [?e :person/name "Alice"]
                          [?e :hobby ?hobby]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?hobby"]);
            assert_eq!(results.len(), 3);

            let hobbies: Vec<String> = results
                .iter()
                .map(|row| match &row[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected String"),
                })
                .collect();

            assert!(hobbies.contains(&"Reading".to_string()));
            assert!(hobbies.contains(&"Hiking".to_string()));
            assert!(hobbies.contains(&"Coding".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test empty database query
#[test]
fn test_query_empty_database() {
    let db = Minigraf::in_memory().unwrap();

    let result = exec(
        &db,
        r#"(query [:find ?name :where [?e :person/name ?name]])"#,
    );
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name"]);
            assert_eq!(results.len(), 0);
        }
        _ => panic!("Expected QueryResults"),
    }
}

/// Test UUID entity references (verify #uuid literals work in queries)
#[test]
fn test_uuid_entity_reference() {
    let db = Minigraf::in_memory().unwrap();

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Use UUID entities via the execute API
    let cmd = format!(
        r#"(transact [[#uuid "{}" :person/name "Alice"]
                       [#uuid "{}" :person/name "Bob"]
                       [#uuid "{}" :friend #uuid "{}"]])"#,
        alice, bob, alice, bob
    );
    exec(&db, &cmd);

    // Query friends of Alice
    let query = format!(
        r#"(query [:find ?name
                   :where [#uuid "{}" :friend ?friend]
                          [?friend :person/name ?name]])"#,
        alice
    );
    let result = exec(&db, &query);
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name"]);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Bob".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}
