//! Cross-feature integration tests for Phase 7.5.
//! Each test models a realistic embedder workload combining 2–3 Datalog features.

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

fn result_count(r: &QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 1: not + :as-of ─────────────────────────────────────────────────────
// "Who has a name but no department assignment, as of each transaction?"

#[test]
fn not_absent_from_dept_as_of() {
    let db = db();
    // tx 1: alice (with dept) and bob (without dept)
    db.execute(
        r#"(transact [[:alice :person/name "Alice"] [:alice :person/dept "eng"]
                      [:bob   :person/name "Bob"]])"#,
    )
    .unwrap();
    // tx 2: charlie joins with dept
    db.execute(r#"(transact [[:charlie :person/name "Charlie"] [:charlie :person/dept "hr"]])"#)
        .unwrap();

    // As of tx 1: only bob lacks a dept
    let r1 = db
        .execute(
            r#"(query [:find ?e
                       :as-of 1 :valid-at :any-valid-time
                       :where [?e :person/name ?_n]
                              (not [?e :person/dept ?_d])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r1), 1, "as-of tx 1: only bob lacks a dept");

    // As of tx 2: charlie now exists with dept; still only bob lacks one
    let r2 = db
        .execute(
            r#"(query [:find ?e
                       :as-of 2 :valid-at :any-valid-time
                       :where [?e :person/name ?_n]
                              (not [?e :person/dept ?_d])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r2), 1, "as-of tx 2: still only bob lacks a dept");
}

// ── Test 2: not-join + count aggregation ─────────────────────────────────────
// "How many users have no completed orders?"

#[test]
fn users_without_completed_orders_not_join_count() {
    let db = db();
    db.execute(
        r#"(transact [[:alice   :user/name "Alice"]
                      [:bob     :user/name "Bob"]
                      [:charlie :user/name "Charlie"]
                      [:o1 :order/owner :alice] [:o1 :order/status :completed]
                      [:o2 :order/owner :bob]   [:o2 :order/status :pending]])"#,
    )
    .unwrap();

    // Users without any completed order: bob (has pending) and charlie (no orders)
    let r = db
        .execute(
            r#"(query [:find (count ?u)
                       :where [?u :user/name ?_n]
                              (not-join [?u]
                                [?o :order/owner ?u]
                                [?o :order/status :completed])])"#,
        )
        .unwrap();
    assert_eq!(
        results(&r)[0][0],
        Value::Integer(2),
        "bob and charlie have no completed orders"
    );
}

// ── Test 3: count aggregation + not ──────────────────────────────────────────
// "Headcount per department, excluding contractors."

#[test]
fn headcount_by_dept_excluding_contractors() {
    let db = db();
    db.execute(
        r#"(transact [[:alice :emp/dept "eng"] [:bob   :emp/dept "eng"] [:carol :emp/dept "eng"]
                      [:dave  :emp/dept "hr"]  [:eve   :emp/dept "hr"]
                      [:carol :emp/contractor true]])"#,
    )
    .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?dept (count ?e)
                       :where [?e :emp/dept ?dept]
                              (not [?e :emp/contractor true])])"#,
        )
        .unwrap();

    let mut rows = results(&r).clone();
    rows.sort_by_key(|row| match &row[0] {
        Value::String(s) => s.clone(),
        _ => String::new(),
    });
    assert_eq!(rows.len(), 2, "two departments");
    // eng: alice + bob (carol is contractor, excluded) = 2
    assert_eq!(rows[0][0], Value::String("eng".into()));
    assert_eq!(rows[0][1], Value::Integer(2));
    // hr: dave + eve = 2
    assert_eq!(rows[1][0], Value::String("hr".into()));
    assert_eq!(rows[1][1], Value::Integer(2));
}

// ── Test 4: count aggregation + :valid-at bi-temporal ────────────────────────
// "Count active staff per role at two different points in time."

#[test]
fn active_staff_by_role_valid_at() {
    let db = db();
    // alice and carol: valid indefinitely from 2023-01-01
    db.execute(
        r#"(transact {:valid-from "2023-01-01"}
                     [[:alice :staff/role "eng"] [:carol :staff/role "hr"]])"#,
    )
    .unwrap();
    // bob: only valid in 2023 (expires at 2024-01-01)
    db.execute(
        r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"}
                     [[:bob :staff/role "eng"]])"#,
    )
    .unwrap();

    // At 2023-06-01: alice (eng), bob (eng), carol (hr) → eng=2, hr=1
    let r_2023 = db
        .execute(
            r#"(query [:find ?role (count ?e)
                       :valid-at "2023-06-01"
                       :where [?e :staff/role ?role]])"#,
        )
        .unwrap();
    let mut rows_2023 = results(&r_2023).clone();
    rows_2023.sort_by_key(|row| match &row[0] {
        Value::String(s) => s.clone(),
        _ => String::new(),
    });
    assert_eq!(rows_2023.len(), 2, "two roles in 2023");
    assert_eq!(rows_2023[0][0], Value::String("eng".into()));
    assert_eq!(rows_2023[0][1], Value::Integer(2));
    assert_eq!(rows_2023[1][0], Value::String("hr".into()));
    assert_eq!(rows_2023[1][1], Value::Integer(1));

    // At 2024-06-01: bob has expired → eng=1, hr=1
    let r_2024 = db
        .execute(
            r#"(query [:find ?role (count ?e)
                       :valid-at "2024-06-01"
                       :where [?e :staff/role ?role]])"#,
        )
        .unwrap();
    let mut rows_2024 = results(&r_2024).clone();
    rows_2024.sort_by_key(|row| match &row[0] {
        Value::String(s) => s.clone(),
        _ => String::new(),
    });
    assert_eq!(rows_2024.len(), 2, "two roles in 2024");
    assert_eq!(rows_2024[0][0], Value::String("eng".into()));
    assert_eq!(rows_2024[0][1], Value::Integer(1));
    assert_eq!(rows_2024[1][0], Value::String("hr".into()));
    assert_eq!(rows_2024[1][1], Value::Integer(1));
}
