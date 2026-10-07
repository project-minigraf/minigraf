//! `Minigraf::query` and `PreparedQuery::query` return a `Cursor` that delivers
//! the answer in batches (#432). These tests pin the public contract the
//! streaming engine must keep: batches partition the `execute()` answer, the
//! answer is fixed when the cursor opens, and non-queries are rejected.
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{BindValue, Cursor, Minigraf, OpenOptions, QueryResult, Value};

fn seeded(n: i64) -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..n)
        .map(|i| format!("[:p{i} :person/age {i}] [:p{i} :person/name \"n{i}\"]"))
        .collect::<Vec<_>>()
        .join(" ");
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db
}

fn execute_rows(db: &Minigraf, q: &str) -> (Vec<String>, Vec<Vec<Value>>) {
    match db.execute(q).unwrap() {
        QueryResult::QueryResults { vars, results } => (vars, results),
        _ => panic!("expected query results"),
    }
}

/// Rows in a stable order, so answers can be compared as multisets.
fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by_key(|r| format!("{r:?}"));
    rows
}

fn drain(cursor: &mut Cursor, max_rows: usize) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    while let Some(batch) = cursor.next_batch(max_rows).unwrap() {
        assert!(!batch.is_empty(), "a batch is never empty");
        assert!(batch.len() <= max_rows.max(1), "batch exceeds max_rows");
        assert_eq!(batch.len(), batch.rows().len());
        rows.extend(batch.into_rows());
    }
    rows
}

const AGES: &str = "(query [:find ?e ?age :where [?e :person/age ?age]])";

#[test]
fn batches_partition_the_execute_answer() {
    let db = seeded(25);
    let (vars, expected) = execute_rows(&db, AGES);
    for max_rows in [1, 7, 25, 1000] {
        let mut cursor = db.query(AGES).unwrap();
        assert_eq!(cursor.vars(), vars.as_slice());
        let rows = drain(&mut cursor, max_rows);
        assert_eq!(rows.len(), 25, "every row delivered once");
        assert_eq!(sorted(rows), sorted(expected.clone()));
        // The end stays the end.
        assert!(cursor.next_batch(max_rows).unwrap().is_none());
    }
}

#[test]
fn batch_sizes_are_full_until_the_last() {
    let db = seeded(25);
    let mut cursor = db.query(AGES).unwrap();
    let mut sizes = Vec::new();
    while let Some(batch) = cursor.next_batch(10).unwrap() {
        sizes.push(batch.len());
    }
    assert_eq!(sizes, vec![10, 10, 5]);
}

#[test]
fn zero_max_rows_still_makes_progress() {
    let db = seeded(3);
    let mut cursor = db.query(AGES).unwrap();
    let rows = drain(&mut cursor, 0);
    assert_eq!(rows.len(), 3);
}

#[test]
fn iterator_yields_the_execute_rows() {
    let db = seeded(12);
    let (_, expected) = execute_rows(&db, AGES);
    let rows: Vec<Vec<Value>> = db.query(AGES).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(sorted(rows), sorted(expected));
}

#[test]
fn batch_into_iterator_yields_rows() {
    let db = seeded(4);
    let batch = db.query(AGES).unwrap().next_batch(10).unwrap().unwrap();
    let n = batch.into_iter().filter(|row| row.len() == 2).count();
    assert_eq!(n, 4);
}

#[test]
fn empty_answer_has_vars_and_no_batch() {
    let db = seeded(3);
    let mut cursor = db
        .query("(query [:find ?e :where [?e :person/missing ?x]])")
        .unwrap();
    assert_eq!(cursor.vars(), ["?e".to_string()].as_slice());
    assert!(cursor.next_batch(10).unwrap().is_none());
    assert!(cursor.next().is_none());
}

#[test]
fn aggregates_rules_and_as_of_match_execute() {
    let db = seeded(10);
    db.execute("(transact [[:p1 :person/friend :p2] [:p2 :person/friend :p3]])")
        .unwrap();
    db.execute("(rule [(reach ?a ?b) [?a :person/friend ?b]])")
        .unwrap();
    db.execute("(rule [(reach ?a ?b) [?a :person/friend ?m] (reach ?m ?b)])")
        .unwrap();
    let queries = [
        "(query [:find (count ?e) (max ?age) :where [?e :person/age ?age]])",
        "(query [:find ?a ?b :where (reach ?a ?b)])",
        "(query [:find ?e :as-of 1 :where [?e :person/friend ?f]])",
        "(query [:find ?e :any-valid-time :where [?e :person/age ?age]])",
    ];
    for q in queries {
        let (vars, expected) = execute_rows(&db, q);
        let mut cursor = db.query(q).unwrap();
        assert_eq!(cursor.vars(), vars.as_slice());
        assert_eq!(sorted(drain(&mut cursor, 2)), sorted(expected));
    }
}

#[test]
fn cursor_answer_is_fixed_at_open() {
    let db = seeded(5);
    let mut cursor = db.query(AGES).unwrap();
    let first = cursor.next_batch(2).unwrap().unwrap();
    assert_eq!(first.len(), 2);

    db.execute("(transact [[:late :person/age 99]])").unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:later :person/age 100]])").unwrap();
    tx.commit().unwrap();
    db.execute("(retract [[:p0 :person/age 0]])").unwrap();

    let rest = drain(&mut cursor, 2);
    assert_eq!(
        first.len() + rest.len(),
        5,
        "pre-transaction answer to the end"
    );
    let ages: Vec<Value> = first
        .into_rows()
        .into_iter()
        .chain(rest)
        .map(|r| r[1].clone())
        .collect();
    assert!(!ages.contains(&Value::Integer(99)), "later write not seen");
    assert!(
        !ages.contains(&Value::Integer(100)),
        "later commit not seen"
    );
    assert!(ages.contains(&Value::Integer(0)), "later retract not seen");
}

#[test]
fn file_backed_cursor_answer_is_fixed_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.graph");
    let db = Minigraf::open(&path).unwrap();
    db.execute("(transact [[:a :n 1] [:b :n 2] [:c :n 3]])")
        .unwrap();
    db.checkpoint().unwrap();
    db.execute("(transact [[:d :n 4]])").unwrap();

    let mut cursor = db.query("(query [:find ?v :where [?e :n ?v]])").unwrap();
    db.execute("(transact [[:e :n 5]])").unwrap();
    db.checkpoint().unwrap();
    let rows = drain(&mut cursor, 1);
    assert_eq!(
        rows.len(),
        4,
        "committed and WAL facts at open, nothing later"
    );
}

#[test]
fn non_queries_are_api_012() {
    let db = seeded(1);
    for (cmd, kind) in [
        ("(transact [[:x :person/age 1]])", "transact"),
        ("(retract [[:p0 :person/age 0]])", "retract"),
        ("(rule [(adult ?e) [?e :person/age ?a]])", "rule"),
    ] {
        let err = db.query(cmd).unwrap_err();
        assert_eq!(err.code(), "API-012");
        assert!(err.to_string().contains(kind), "message names the command");
    }
    // Nothing was written or registered.
    let (_, rows) = execute_rows(&db, AGES);
    assert_eq!(rows.len(), 1);
}

#[test]
fn bind_slots_are_api_010() {
    let db = seeded(1);
    let err = db
        .query("(query [:find ?e :where [?e :person/age $age]])")
        .unwrap_err();
    assert_eq!(err.code(), "API-010");
}

#[test]
fn parse_errors_keep_their_code() {
    let db = seeded(1);
    let err = db.query("(query [:find ?e :where").unwrap_err();
    assert!(err.code().starts_with("PRS-"), "parse error code");
}

#[test]
fn query_on_a_write_transaction_thread_is_int_001() {
    let db = seeded(1);
    let tx = db.begin_write().unwrap();
    let err = db.query(AGES).unwrap_err();
    assert_eq!(err.code(), "INT-001");
    tx.rollback();
}

#[test]
fn rule_limits_apply_while_the_engine_materialises() {
    let db = OpenOptions::new().max_results(3).open_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c] [:c :next :d] [:d :next :e]])")
        .unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)])")
        .unwrap();
    let q = "(query [:find ?x ?y :where (reach ?x ?y)])";
    assert_eq!(db.execute(q).unwrap_err().code(), "INT-020");
    assert_eq!(db.query(q).unwrap_err().code(), "INT-020");
}

#[test]
fn prepared_query_cursor_matches_prepared_execute() {
    let db = seeded(10);
    let pq = db
        .prepare("(query [:find ?name :where [?e :person/age $age] [?e :person/name ?name]])")
        .unwrap();
    for age in [0, 4, 9, 42] {
        let binds = [("age", BindValue::Val(Value::Integer(age)))];
        let expected = match pq.execute(&binds).unwrap() {
            QueryResult::QueryResults { results, .. } => results,
            _ => panic!("expected query results"),
        };
        let mut cursor = pq.query(&binds).unwrap();
        assert_eq!(cursor.vars(), ["?name".to_string()].as_slice());
        assert_eq!(drain(&mut cursor, 1), expected);
    }
    let err = pq.query(&[]).unwrap_err();
    assert_eq!(err.code(), "INT-033", "missing bind value");
}

#[test]
fn cursor_is_send_and_outlives_the_database_handle() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<Cursor>();

    let db = seeded(6);
    let cursor = db.query(AGES).unwrap();
    drop(db);
    let n = std::thread::spawn(move || cursor.map(Result::unwrap).count())
        .join()
        .unwrap();
    assert_eq!(n, 6);
}

#[test]
fn close_after_first_batch() {
    let db = seeded(10);
    let mut cursor = db.query(AGES).unwrap();
    assert!(cursor.next_batch(1).unwrap().is_some());
    cursor.close();
    // The database is unaffected.
    assert_eq!(execute_rows(&db, AGES).1.len(), 10);
}
