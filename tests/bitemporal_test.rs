use minigraf::graph::FactStorage;
use minigraf::{parse_datalog_command, DatalogExecutor, QueryResult};
use minigraf::graph::types::Value;
use uuid::Uuid;

/// Helper: parse and execute a Datalog command string, panicking on error
fn exec(executor: &DatalogExecutor, input: &str) -> QueryResult {
    let cmd = parse_datalog_command(input)
        .unwrap_or_else(|e| panic!("parse error for {:?}: {}", input, e));
    executor
        .execute(cmd)
        .unwrap_or_else(|e| panic!("execution error for {:?}: {}", input, e))
}

/// Helper: extract rows from a QueryResult::QueryResults
fn result_rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        other => panic!("expected QueryResults, got {:?}", other),
    }
}

// ============================================================================
// Test 1: Transaction time travel via counter (:as-of N)
// ============================================================================

#[test]
fn test_tx_time_travel_via_counter() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // tx_count=1: assert Alice's name
    exec(
        &executor,
        r#"(transact [[:alice :person/name "Alice"]])"#,
    );

    // tx_count=2: assert Alice's age
    exec(
        &executor,
        r#"(transact [[:alice :person/age "30"]])"#,
    );

    // :as-of 1 → only the name fact was asserted at tx_count=1
    // Use :valid-at :any-valid-time so the forever-valid fact passes the valid-time filter
    let result = exec(
        &executor,
        r#"(query [:find ?attr :as-of 1 :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "as-of tx 1 should see only the name fact");

    // Verify it is the name attribute
    match &rows[0][0] {
        Value::Keyword(k) => assert_eq!(k, ":person/name"),
        other => panic!("expected keyword :person/name, got {:?}", other),
    }
}

// ============================================================================
// Test 2: Transaction time travel via counter — two transacts, as-of latest
// ============================================================================

#[test]
fn test_tx_time_travel_as_of_all() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // tx_count=1
    exec(
        &executor,
        r#"(transact [[:alice :person/name "Alice"]])"#,
    );
    // tx_count=2
    exec(
        &executor,
        r#"(transact [[:alice :person/age "30"]])"#,
    );

    // :as-of 2 (or higher) → both facts visible
    let result = exec(
        &executor,
        r#"(query [:find ?attr :as-of 2 :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 2, "as-of tx 2 should see both facts");
}

// ============================================================================
// Test 3: Valid time inside range
// ============================================================================

#[test]
fn test_valid_at_inside_range() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // Alice was employed at Acme from 2023-01-01 to 2023-06-30
    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :active]])"#,
    );

    // Query on 2023-03-01 (inside range) → should match
    let result = exec(
        &executor,
        r#"(query [:find ?s :valid-at "2023-03-01" :where [:alice :employment/status ?s]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "2023-03-01 is inside the valid range, should return 1 result");
    match &rows[0][0] {
        Value::Keyword(k) => assert_eq!(k, ":active"),
        other => panic!("expected :active, got {:?}", other),
    }
}

// ============================================================================
// Test 4: Valid time outside range
// ============================================================================

#[test]
fn test_valid_at_outside_range() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :active]])"#,
    );

    // Query on 2024-01-01 (outside range) → no match
    let result = exec(
        &executor,
        r#"(query [:find ?s :valid-at "2024-01-01" :where [:alice :employment/status ?s]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 0, "2024-01-01 is outside the valid range, should return 0 results");
}

// ============================================================================
// Test 5: Default query (no :valid-at) returns only currently valid facts
// ============================================================================

#[test]
fn test_no_valid_at_returns_only_current() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // Expired fact — valid only in 2020
    exec(
        &executor,
        r#"(transact {:valid-from "2020-01-01" :valid-to "2020-12-31"} [[:alice :employment/org :old-company]])"#,
    );

    // Forever fact (default valid time: now to far future)
    exec(
        &executor,
        r#"(transact [[:alice :person/name "Alice"]])"#,
    );

    // Default query (no :valid-at) → only the forever-valid name fact
    let result = exec(
        &executor,
        r#"(query [:find ?attr :where [:alice ?attr ?v]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "default query should return only currently valid facts");
    match &rows[0][0] {
        Value::Keyword(k) => assert_eq!(k, ":person/name"),
        other => panic!("expected :person/name, got {:?}", other),
    }
}

// ============================================================================
// Test 6: :valid-at :any-valid-time returns all facts regardless of validity
// ============================================================================

#[test]
fn test_valid_at_any_valid_time_returns_all() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // Expired fact
    exec(
        &executor,
        r#"(transact {:valid-from "2020-01-01" :valid-to "2020-12-31"} [[:alice :employment/org :old-company]])"#,
    );

    // Forever valid fact
    exec(
        &executor,
        r#"(transact [[:alice :person/name "Alice"]])"#,
    );

    // :any-valid-time → both facts returned
    let result = exec(
        &executor,
        r#"(query [:find ?attr :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 2, ":any-valid-time should return both expired and current facts");
}

// ============================================================================
// Test 7: Bi-temporal combined query (:as-of N :valid-at "date")
// ============================================================================

#[test]
fn test_bitemporal_combined_query() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // tx_count=1: Alice was active from 2023-01 to 2023-06
    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :active]])"#,
    );

    // tx_count=2: Correction — Alice was actually inactive in that period
    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :inactive]])"#,
    );

    // As-of tx 1, valid on 2023-03-01 → should see only the original :active fact
    let result = exec(
        &executor,
        r#"(query [:find ?s :as-of 1 :valid-at "2023-03-01" :where [:alice :employment/status ?s]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "as-of tx 1 should see only the original :active fact");
    match &rows[0][0] {
        Value::Keyword(k) => assert_eq!(k, ":active", "expected :active at tx_count=1"),
        other => panic!("expected keyword, got {:?}", other),
    }
}

// ============================================================================
// Test 8: Valid time — exact boundary (valid_to is exclusive)
// ============================================================================

#[test]
fn test_valid_at_boundary_exclusive() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // Fact valid from 2023-01-01 (inclusive) to 2023-06-30 (exclusive)
    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :active]])"#,
    );

    // Query exactly at valid_to boundary (should be exclusive)
    let result = exec(
        &executor,
        r#"(query [:find ?s :valid-at "2023-06-30" :where [:alice :employment/status ?s]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 0, "valid_to is exclusive: querying at exactly valid_to should return no results");

    // Query one day before valid_to boundary → should match
    let result2 = exec(
        &executor,
        r#"(query [:find ?s :valid-at "2023-06-29" :where [:alice :employment/status ?s]])"#,
    );
    let rows2 = result_rows(result2);
    assert_eq!(rows2.len(), 1, "one day before valid_to should still be in range");
}

// ============================================================================
// Test 9: Migration note — PersistentFactStorage
// ============================================================================
// Note: PersistentFactStorage v1→v2 migration is tested comprehensively at
// the unit-test level in src/storage/persistent_facts.rs (Task 5).
// The migration logic automatically upgrades on load, which is covered by
// tests in that module. We omit a higher-level integration test here because
// PersistentFactStorage requires a file path and writing binary fixtures,
// which is better suited to the unit-test boundary.

// ============================================================================
// Test 10: Multi-entity bi-temporal query
// ============================================================================

#[test]
fn test_bitemporal_multi_entity() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Both employed in 2023 Q1
    storage
        .transact(
            vec![
                (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
                (bob, ":person/name".to_string(), Value::String("Bob".to_string())),
            ],
            None,
        )
        .unwrap();

    // Alice: employed at Acme in 2023 H1
    exec(
        &executor,
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice-kw :employment/org :acme]])"#,
    );

    // Bob: employed at Beta in 2023 H2
    exec(
        &executor,
        r#"(transact {:valid-from "2023-07-01" :valid-to "2023-12-31"} [[:bob-kw :employment/org :beta]])"#,
    );

    // Query at 2023-03-01: only :alice-kw is employed
    let result = exec(
        &executor,
        r#"(query [:find ?who :valid-at "2023-03-01" :where [?who :employment/org ?org]])"#,
    );
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1, "only alice-kw should be employed at 2023-03-01");

    // Query at 2023-09-01: only :bob-kw is employed
    let result2 = exec(
        &executor,
        r#"(query [:find ?who :valid-at "2023-09-01" :where [?who :employment/org ?org]])"#,
    );
    let rows2 = result_rows(result2);
    assert_eq!(rows2.len(), 1, "only bob-kw should be employed at 2023-09-01");
}

// ============================================================================
// Test 11: :as-of counter limits visibility to recorded transactions
// ============================================================================

#[test]
fn test_as_of_counter_time_travel() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    // tx_count=1: name
    exec(
        &executor,
        r#"(transact [[:alice :person/name "Alice"]])"#,
    );

    // tx_count=2: age
    exec(
        &executor,
        r#"(transact [[:alice :person/age "30"]])"#,
    );

    // tx_count=3: city
    exec(
        &executor,
        r#"(transact [[:alice :person/city "NYC"]])"#,
    );

    // :as-of 1 → only name
    let result1 = exec(
        &executor,
        r#"(query [:find ?attr :as-of 1 :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    assert_eq!(result_rows(result1).len(), 1, "as-of 1: only name");

    // :as-of 2 → name + age
    let result2 = exec(
        &executor,
        r#"(query [:find ?attr :as-of 2 :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    assert_eq!(result_rows(result2).len(), 2, "as-of 2: name + age");

    // :as-of 3 → name + age + city
    let result3 = exec(
        &executor,
        r#"(query [:find ?attr :as-of 3 :valid-at :any-valid-time :where [:alice ?attr ?v]])"#,
    );
    assert_eq!(result_rows(result3).len(), 3, "as-of 3: name + age + city");
}
