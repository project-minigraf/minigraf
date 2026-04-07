//! Integration tests for Phase 6.1 covering indexes.
//!
//! These tests verify that:
//! - Query correctness is maintained after transact and reload (indexes rebuilt from disk)
//! - Index population is correct through the in-process API
//! - Bi-temporal valid-at queries still work correctly
//! - Transaction-time as-of queries still work correctly
//! - Recursive rules are unaffected by the index layer
//! - Explicit write transactions work correctly with indexes

use minigraf::db::Minigraf;
use minigraf::{QueryResult, Value};

// ── helpers ───────────────────────────────────────────────────────────────────

fn open_temp_db() -> (Minigraf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    let db = Minigraf::open(&db_path).unwrap();
    (db, dir)
}

fn count_results(result: QueryResult) -> usize {
    match result {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

fn extract_rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        other => panic!("expected QueryResults, got {:?}", other),
    }
}

// ── 1. Save / reload — indexes rebuilt from disk ──────────────────────────────

/// Write facts in one session, drop (triggers checkpoint), reopen and query.
/// Verifies that indexes are correctly rebuilt on second open.
#[test]
fn test_query_correct_after_transact_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reload.graph");

    // First session: write facts, then drop (triggers auto-checkpoint)
    {
        let db = Minigraf::open(&db_path).unwrap();
        db.execute(
            r#"(transact [[:alice :person/name "Alice"]
                                  [:bob :person/name "Bob"]])"#,
        )
        .unwrap();
    }

    // Second session: reopen and query — indexes rebuilt from disk
    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
                .unwrap(),
        );
        assert_eq!(n, 2, "Both names should be found after reload");
    }
}

// ── 2. Index correctness via in-process API ───────────────────────────────────

/// Insert facts and immediately query through indexes in the same process.
#[test]
fn test_index_correctness_via_query() {
    let (db, _dir) = open_temp_db();
    db.execute(r#"(transact [[:a :x 1] [:a :y 2] [:a :link :b]])"#)
        .unwrap();

    let n = count_results(
        db.execute(r#"(query [:find ?v :where [?e :x ?v]])"#)
            .unwrap(),
    );
    assert_eq!(n, 1, "Exactly one entity has attribute :x");
}

// ── 3. Bi-temporal valid-at queries ──────────────────────────────────────────

/// Insert facts with different valid-time windows, then verify valid-at
/// filtering still returns the correct fact.
#[test]
fn test_bitemporal_valid_at_query_still_correct() {
    let (db, _dir) = open_temp_db();

    db.execute(
        r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"}
                     [[:alice :status :active]])"#,
    )
    .unwrap();
    db.execute(
        r#"(transact {:valid-from "2024-01-01"}
                     [[:alice :status :retired]])"#,
    )
    .unwrap();

    let rows = extract_rows(
        db.execute(r#"(query [:find ?s :valid-at "2023-06-01" :where [:alice :status ?s]])"#)
            .unwrap(),
    );
    assert_eq!(rows.len(), 1, "Should find exactly one status in mid-2023");
    assert_eq!(
        rows[0][0],
        Value::Keyword(":active".to_string()),
        "Status in 2023 should be :active"
    );
}

// ── 4. Transaction-time as-of queries ────────────────────────────────────────

/// Insert age facts in two separate transactions, then query :as-of 1 to
/// verify only the first transaction's fact is returned.
#[test]
fn test_as_of_query_still_correct() {
    let (db, _dir) = open_temp_db();
    db.execute(r#"(transact [[:alice :age 30]])"#).unwrap();
    db.execute(r#"(transact [[:alice :age 31]])"#).unwrap();

    // :as-of 1 should see age=30 only (first tx)
    let rows = extract_rows(
        db.execute(r#"(query [:find ?a :as-of 1 :where [:alice :age ?a]])"#)
            .unwrap(),
    );
    assert_eq!(rows.len(), 1, ":as-of 1 should return exactly one age");
    assert_eq!(rows[0][0], Value::Integer(30), "Age at tx 1 should be 30");
}

// ── 5. Recursive rules regression ────────────────────────────────────────────

/// Verify that recursive transitive-closure rules still produce correct results
/// after the Phase 6.1 index changes.
#[test]
fn test_recursive_rules_unchanged_after_6_1() {
    let (db, _dir) = open_temp_db();
    db.execute(r#"(transact [[:a :connected :b] [:b :connected :c] [:c :connected :d]])"#)
        .unwrap();
    db.execute(r#"(rule [(reachable ?from ?to) [?from :connected ?to]])"#)
        .unwrap();
    db.execute(r#"(rule [(reachable ?from ?to) [?from :connected ?mid] (reachable ?mid ?to)])"#)
        .unwrap();

    let n = count_results(
        db.execute(r#"(query [:find ?to :where (reachable :a ?to)])"#)
            .unwrap(),
    );
    assert_eq!(n, 3, ":a can reach :b, :c, :d (3 nodes)");
}

// ── 6. Explicit write transaction with indexes ────────────────────────────────

/// Use begin_write / commit and verify the committed fact is visible
/// immediately via a query on the same db handle.
#[test]
fn test_explicit_transaction_with_indexes() {
    let (db, _dir) = open_temp_db();

    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:alice :age 30]])"#).unwrap();
    tx.commit().unwrap();

    let n = count_results(
        db.execute(r#"(query [:find ?a :where [:alice :age ?a]])"#)
            .unwrap(),
    );
    assert_eq!(n, 1, "Committed fact should be visible after explicit tx");
}
