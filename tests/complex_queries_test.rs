use minigraf::graph::types::Value;
use minigraf::graph::FactStorage;
use minigraf::query::datalog::{DatalogExecutor, QueryResult};
use minigraf::query::datalog::parser::parse_datalog_command;
use uuid::Uuid;

/// Test 3-pattern join: find entities with name, age, and city
#[test]
fn test_three_pattern_join() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Create entity with 3 attributes
    let alice_id = Uuid::new_v4();
    storage
        .transact(vec![
            (
                alice_id,
                ":person/name".to_string(),
                Value::String("Alice".to_string()),
            ),
            (alice_id, ":person/age".to_string(), Value::Integer(30)),
            (
                alice_id,
                ":person/city".to_string(),
                Value::String("NYC".to_string()),
            ),
        ], None)
        .unwrap();

    let query = parse_datalog_command(
        r#"(query [:find ?name ?age ?city
                   :where [?e :person/name ?name]
                          [?e :person/age ?age]
                          [?e :person/city ?city]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (alice, ":person/age".to_string(), Value::Integer(30)),
            (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            (bob, ":person/age".to_string(), Value::Integer(25)),
            (alice, ":friend".to_string(), Value::Ref(bob)),
        ], None)
        .unwrap();

    // Find pairs where person1 is older than 28 and friends with person2
    let query = parse_datalog_command(
        r#"(query [:find ?name1 ?age1 ?name2 ?age2
                   :where [?p1 :person/name ?name1]
                          [?p1 :person/age ?age1]
                          [?p1 :friend ?p2]
                          [?p2 :person/name ?name2]
                          [?p2 :person/age ?age2]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let charlie = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            (charlie, ":person/name".to_string(), Value::String("Charlie".to_string())),
            (alice, ":friend".to_string(), Value::Ref(bob)),
            (bob, ":friend".to_string(), Value::Ref(charlie)),
        ], None)
        .unwrap();

    // Find friends of Alice's friends (Bob's friends)
    let query_str = format!(
        r#"(query [:find ?name
                   :where [#uuid "{}" :friend ?friend]
                          [?friend :friend ?fof]
                          [?fof :person/name ?name]])"#,
        alice
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let company = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            (company, ":company/name".to_string(), Value::String("TechCorp".to_string())),
            (alice, ":works-at".to_string(), Value::Ref(company)),
            (bob, ":works-at".to_string(), Value::Ref(company)),
        ], None)
        .unwrap();

    // Find people working at TechCorp
    let query = parse_datalog_command(
        r#"(query [:find ?person-name
                   :where [?person :works-at ?company]
                          [?company :company/name "TechCorp"]
                          [?person :person/name ?person-name]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    // Add some facts
    let alice = Uuid::new_v4();
    storage
        .transact(vec![(
            alice,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
        )], None)
        .unwrap();

    // Query for non-existent attribute
    let query = parse_datalog_command(
        r#"(query [:find ?email :where [?e :person/email ?email]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (alice, ":person/age".to_string(), Value::Integer(30)),
            (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            // Bob has no age
        ], None)
        .unwrap();

    // Query for name AND age - should only return Alice
    let query = parse_datalog_command(
        r#"(query [:find ?name ?age
                   :where [?e :person/name ?name]
                          [?e :person/age ?age]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (alice, ":person/nickname".to_string(), Value::String("Alice".to_string())),
        ], None)
        .unwrap();

    // Find people whose name equals their nickname
    let query = parse_datalog_command(
        r#"(query [:find ?name
                   :where [?e :person/name ?name]
                          [?e :person/nickname ?name]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let charlie = Uuid::new_v4();
    let project1 = Uuid::new_v4();
    let project2 = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            (charlie, ":person/name".to_string(), Value::String("Charlie".to_string())),
            (project1, ":project/name".to_string(), Value::String("Project X".to_string())),
            (project2, ":project/name".to_string(), Value::String("Project Y".to_string())),
            (alice, ":works-on".to_string(), Value::Ref(project1)),
            (bob, ":works-on".to_string(), Value::Ref(project1)),
            (charlie, ":works-on".to_string(), Value::Ref(project2)),
            (alice, ":manages".to_string(), Value::Ref(project1)),
        ], None)
        .unwrap();

    // Find projects that Alice manages, along with other people working on them
    let query_str = format!(
        r#"(query [:find ?project-name ?coworker-name
                   :where [#uuid "{}" :manages ?project]
                          [?project :project/name ?project-name]
                          [?coworker :works-on ?project]
                          [?coworker :person/name ?coworker-name]])"#,
        alice
    );
    let query = parse_datalog_command(&query_str).unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();

    storage
        .transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
            (alice, ":hobby".to_string(), Value::String("Reading".to_string())),
        ], None)
        .unwrap();

    storage
        .transact(vec![
            (alice, ":hobby".to_string(), Value::String("Hiking".to_string())),
        ], None)
        .unwrap();

    storage
        .transact(vec![
            (alice, ":hobby".to_string(), Value::String("Coding".to_string())),
        ], None)
        .unwrap();

    // Find all hobbies (should get 3 separate results)
    let query = parse_datalog_command(
        r#"(query [:find ?hobby
                   :where [?e :person/name "Alice"]
                          [?e :hobby ?hobby]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
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
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    let query = parse_datalog_command(
        r#"(query [:find ?name :where [?e :person/name ?name]])"#,
    )
    .unwrap();

    let result = executor.execute(query).unwrap();
    match result {
        QueryResult::QueryResults { vars, results } => {
            assert_eq!(vars, vec!["?name"]);
            assert_eq!(results.len(), 0);
        }
        _ => panic!("Expected QueryResults"),
    }
}
