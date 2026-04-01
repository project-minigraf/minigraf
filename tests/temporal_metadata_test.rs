//! Phase 7.6 integration tests — temporal metadata pseudo-attribute bindings.

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

fn results(r: &QueryResult) -> &Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

// ─── Time Interval Tests ─────────────────────────────────────────────────────

/// Time Interval — find facts alive at any point during interval [T1, T2].
/// Condition: valid_from <= T2 AND valid_to >= T1.
/// "2023-01-01" = 1672531200000 ms, "2024-01-01" = 1704067200000 ms.
#[test]
fn time_interval_any_point_during() {
    let db = db();
    // e1: valid 2022-01-01 to 2023-07-01 → overlaps [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2022-01-01" :valid-to "2023-07-01"} [[:e1 :item/label "A"]])"#).unwrap();
    // e2: valid 2023-07-01 onwards → overlaps [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2023-07-01"} [[:e2 :item/label "B"]])"#).unwrap();
    // e3: valid 2015-01-01 to 2020-01-01 → does NOT overlap [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2015-01-01" :valid-to "2020-01-01"} [[:e3 :item/label "C"]])"#).unwrap();

    // T1 = 2023-01-01 = 1672531200000, T2 = 2024-01-01 = 1704067200000
    let r = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :item/label _]
                       [?e :db/valid-from ?vf]
                       [?e :db/valid-to   ?vt]
                       [(<= ?vf 1704067200000)]
                       [(>= ?vt 1672531200000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2, "e1 and e2 overlap [2023, 2024]; e3 does not");
}

/// Time Interval (strict) — facts alive for the *entire* interval [T1, T2].
/// Condition: valid_from <= T1 AND valid_to >= T2.
#[test]
fn time_interval_entire_interval() {
    let db = db();
    // e1: valid 2020-01-01 to 2025-01-01 → covers entire [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2020-01-01" :valid-to "2025-01-01"} [[:e1 :item/label "A"]])"#).unwrap();
    // e2: valid 2023-07-01 onwards → does NOT cover T1 = 2023-01-01
    db.execute(r#"(transact {:valid-from "2023-07-01"} [[:e2 :item/label "B"]])"#).unwrap();

    // T1 = 1672531200000, T2 = 1704067200000
    let r = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :item/label _]
                       [?e :db/valid-from ?vf]
                       [?e :db/valid-to   ?vt]
                       [(<= ?vf 1672531200000)]
                       [(>= ?vt 1704067200000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1, "only e1 covers the entire interval");
}

// ─── Time-Point Lookup ───────────────────────────────────────────────────────

/// Time-Point Lookup — find all valid_from timestamps when Alice's salary exceeded 50000.
#[test]
fn time_point_lookup_salary_threshold() {
    let db = db();
    // salary 100000, valid 2023-01-01 to 2024-01-01
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"} [[:alice :person/salary 100000]])"#).unwrap();
    // salary 30000, valid 2024-01-01 onwards
    db.execute(r#"(transact {:valid-from "2024-01-01"} [[:alice :person/salary 30000]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vf
                :any-valid-time
                :where [:alice :person/salary ?s]
                       [:alice :db/valid-from ?vf]
                       [(> ?s 50000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1, "only the 2023 salary entry exceeds 50000");
    assert_eq!(rows[0][0], Value::Integer(1672531200000), "valid-from = 2023-01-01");
}

// ─── Time-Interval Lookup ────────────────────────────────────────────────────

/// Time-Interval Lookup — enumerate all validity intervals for Alice's employment status.
#[test]
fn time_interval_lookup_employment_status() {
    let db = db();
    db.execute(r#"(transact {:valid-from "2022-01-01" :valid-to "2023-01-01"} [[:alice :employment/status :probation]])"#).unwrap();
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2025-01-01"} [[:alice :employment/status :permanent]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vf ?vt
                :any-valid-time
                :where [:alice :employment/status _]
                       [:alice :db/valid-from ?vf]
                       [:alice :db/valid-to   ?vt]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2, "two distinct employment intervals");
}

// ─── Tx-time Correlation ─────────────────────────────────────────────────────

/// Bind :db/tx-count and verify it matches :as-of counter semantics.
#[test]
fn tx_count_binding() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap(); // tx_count = 1
    db.execute(r#"(transact [[:bob :person/name "Bob"]])"#).unwrap();   // tx_count = 2

    // Query with :any-valid-time: bind tx_count for all name facts
    let r = db.execute(r#"
        (query [:find ?e ?tc
                :any-valid-time
                :where [?e :person/name _]
                       [?e :db/tx-count ?tc]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2);

    // The tx_counts must be 1 and 2 (in any order)
    let mut counts: Vec<i64> = rows.iter()
        .map(|r| match r[1] { Value::Integer(n) => n, _ => panic!("expected Integer") })
        .collect();
    counts.sort();
    assert_eq!(counts, vec![1, 2]);
}

/// Bind :db/tx-id across two entities written in the same transaction — same tx-id.
#[test]
fn tx_id_same_transaction_join() {
    let db = db();
    // Alice and Bob written in the same transaction → same tx_id
    db.execute(r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?e1 ?e2
                :any-valid-time
                :where [?e1 :person/name _]
                       [?e2 :person/name _]
                       [?e1 :db/tx-id ?tx]
                       [?e2 :db/tx-id ?tx]])
    "#).unwrap();
    let rows = results(&r);
    // Both share the same tx-id: (alice, alice), (alice, bob), (bob, alice), (bob, bob)
    assert_eq!(rows.len(), 4, "cross-join of 2 entities with same tx-id = 4 rows");
}

// ─── :db/valid-at Tests ──────────────────────────────────────────────────────

/// :db/valid-at binds the effective query timestamp when :valid-at is explicit.
#[test]
fn valid_at_explicit_timestamp() {
    let db = db();
    // Insert with valid-from before the query point so the fact is visible at "2023-01-01"
    db.execute(r#"(transact {:valid-from "2020-01-01"} [[:alice :person/name "Alice"]])"#).unwrap();

    // 2023-01-01 = 1672531200000
    let r = db.execute(r#"
        (query [:find ?vat
                :valid-at "2023-01-01"
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::Integer(1672531200000));
}

/// :db/valid-at binds the current time when no :valid-at is specified.
#[test]
fn valid_at_default_is_now() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vat
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    // The value should be a positive ms timestamp (approximately now)
    match rows[0][0] {
        Value::Integer(n) => assert!(n > 0, "valid-at default should be a positive timestamp"),
        _ => panic!("expected Integer for :db/valid-at default"),
    }
}

/// :db/valid-at binds Value::Null when :any-valid-time is used.
#[test]
fn valid_at_any_valid_time_is_null() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vat
                :any-valid-time
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::Null);
}

// ─── Parse-error Tests ───────────────────────────────────────────────────────

/// :db/* in entity position is a parse error.
#[test]
fn parse_error_pseudo_attr_in_entity_position() {
    let db = db();
    let result = db.execute(r#"
        (query [:find ?v
                :any-valid-time
                :where [:db/valid-from :person/name ?v]])
    "#);
    assert!(result.is_err(), "pseudo-attribute in entity position must be a parse error");
}

/// :db/* in value position is a parse error.
#[test]
fn parse_error_pseudo_attr_in_value_position() {
    let db = db();
    let result = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :person/name :db/valid-from]])
    "#);
    assert!(result.is_err(), "pseudo-attribute in value position must be a parse error");
}

// ─── Runtime Hard-error Tests ────────────────────────────────────────────────

/// :db/valid-from without :any-valid-time is a runtime error.
#[test]
fn runtime_error_valid_from_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vf
                :where [:alice :person/name _]
                       [:alice :db/valid-from ?vf]])
    "#);
    assert!(result.is_err(), ":db/valid-from requires :any-valid-time");
}

/// :db/valid-to without :any-valid-time is a runtime error.
#[test]
fn runtime_error_valid_to_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vt
                :where [:alice :person/name _]
                       [:alice :db/valid-to ?vt]])
    "#);
    assert!(result.is_err(), ":db/valid-to requires :any-valid-time");
}

/// :db/tx-count without :any-valid-time is a runtime error.
#[test]
fn runtime_error_tx_count_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?tc
                :where [:alice :person/name _]
                       [:alice :db/tx-count ?tc]])
    "#);
    assert!(result.is_err(), ":db/tx-count requires :any-valid-time");
}

/// :db/tx-id without :any-valid-time is a runtime error.
#[test]
fn runtime_error_tx_id_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?ti
                :where [:alice :person/name _]
                       [:alice :db/tx-id ?ti]])
    "#);
    assert!(result.is_err(), ":db/tx-id requires :any-valid-time");
}

/// :db/valid-at without :any-valid-time succeeds (no restriction on valid-at).
#[test]
fn valid_at_succeeds_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vat
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#);
    assert!(result.is_ok(), ":db/valid-at must not require :any-valid-time");
}
