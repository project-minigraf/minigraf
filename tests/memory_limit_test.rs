//! `OpenOptions::query_memory_limit` and `QRY-012` through the public API
//! (#549): the default, the builder, and every way a query runs.

use minigraf::{BindValue, Minigraf, OpenOptions, QueryResult, Value};

/// An in-memory database with `limit`, holding 50 people.
fn db(limit: usize) -> Minigraf {
    let db = OpenOptions::new()
        .query_memory_limit(limit)
        .open_memory()
        .unwrap();
    let facts: String = (0..50)
        .map(|i| format!("[:p{i} :age {i}] [:p{i} :friend :p{}]", (i + 1) % 50))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db
}

fn rows(result: QueryResult) -> usize {
    match result {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    }
}

const QUERY: &str = "(query [:find ?e ?a :where [?e :age ?a]])";

#[test]
fn off_by_default() {
    assert_eq!(OpenOptions::default().query_memory_limit, usize::MAX);
    assert_eq!(
        OpenOptions::new().query_memory_limit(64).query_memory_limit,
        64
    );
    let db = db(usize::MAX);
    assert_eq!(rows(db.execute(QUERY).unwrap()), 50);
}

#[test]
fn execute_and_query_fail_with_qry_012() {
    let db = db(0);
    let err = db.execute(QUERY).expect_err("execute");
    assert_eq!(err.code(), "QRY-012");
    assert!(
        err.to_string().contains("query memory limit exceeded"),
        "message"
    );
    let mut cursor = db.query(QUERY).expect("a cursor opens");
    let err = cursor.next_batch(10).expect_err("the first pull");
    assert_eq!(err.code(), "QRY-012");
    assert!(cursor.next_batch(10).unwrap().is_none(), "then done");
}

#[test]
fn prepared_queries_are_limited() {
    let db = db(0);
    let prepared = db
        .prepare("(query [:find ?e :where [?e :age $age]])")
        .unwrap();
    let err = prepared
        .execute(&[("age", BindValue::Val(Value::Integer(7)))])
        .expect_err("prepared");
    assert_eq!(err.code(), "QRY-012");

    let db = self::db(usize::MAX);
    let prepared = db
        .prepare("(query [:find ?e :where [?e :age $age]])")
        .unwrap();
    let result = prepared
        .execute(&[("age", BindValue::Val(Value::Integer(7)))])
        .unwrap();
    assert_eq!(rows(result), 1);
}

#[test]
fn rule_queries_are_limited() {
    let rules = [
        "(rule [(reach ?x ?y) [?x :friend ?y]])",
        "(rule [(reach ?x ?z) (reach ?x ?y) [?y :friend ?z]])",
    ];
    let query = "(query [:find ?x ?y :where (reach ?x ?y)])";
    let db = db(10_000);
    for r in rules {
        db.execute(r).unwrap();
    }
    let err = db.execute(query).expect_err("2,500 derived facts");
    assert_eq!(err.code(), "QRY-012");
    let err = db
        .query(query)
        .expect_err("rules run when the cursor opens");
    assert_eq!(err.code(), "QRY-012");

    let db = self::db(usize::MAX);
    for r in rules {
        db.execute(r).unwrap();
    }
    assert_eq!(rows(db.execute(query).unwrap()), 2500);
}

#[test]
fn the_database_answers_after_qry_012() {
    let db = db(4_000);
    let err = db
        .execute("(query [:find ?a (count ?e) :where [?e :age ?a]])")
        .expect_err("50 groups");
    assert_eq!(err.code(), "QRY-012");
    let result = db
        .execute("(query [:find ?a :where [:p3 :age ?a]])")
        .unwrap();
    assert_eq!(rows(result), 1, "a small query still runs");
}

/// A query inside a write transaction that has staged writes runs on the
/// executor, which does not count yet (#549 spec §2); before any write it
/// runs on the engine and is counted.
#[test]
fn write_transactions_with_staged_writes_are_not_counted_yet() {
    let db = db(0);
    let mut tx = db.begin_write().unwrap();
    assert_eq!(
        tx.execute(QUERY).expect_err("no write yet").code(),
        "QRY-012"
    );
    tx.execute("(transact [[:p50 :age 50]])").unwrap();
    assert_eq!(rows(tx.execute(QUERY).unwrap()), 51);
    tx.rollback();
}

/// A prepared rule query runs with the database's `max_derived_facts`, as
/// every other query does.
#[test]
fn prepared_rule_queries_use_the_database_limits() {
    let db = OpenOptions::new()
        .max_derived_facts(5)
        .open_memory()
        .unwrap();
    db.execute("(transact [[:a :friend :b] [:b :friend :c] [:c :friend :d] [:d :friend :e]])")
        .unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :friend ?y]])")
        .unwrap();
    db.execute("(rule [(reach ?x ?z) (reach ?x ?y) [?y :friend ?z]])")
        .unwrap();
    let prepared = db
        .prepare("(query [:find ?x ?y :where (reach ?x ?y) [?y :friend $z]])")
        .unwrap();
    let err = prepared
        .execute(&[("z", BindValue::Val(Value::Keyword(":e".to_string())))])
        .expect_err("10 derived facts");
    assert_eq!(err.code(), "INT-020");
}
