//! Every plan returns the same rows the same number of times (#596, design
//! `docs/superpowers/specs/2026-10-15-plan-independent-answers-design.md`).
//!
//! Each query runs through `Minigraf::execute` and through a drained
//! `Minigraf::query` cursor, with clause reordering on and off
//! (`minigraf::__reorder`), and all four answers must be the same bag. With
//! the `test-oracle` feature, every query the streaming engine runs is also
//! run by the executor and must agree, so each run compares the two as well.
//! Per-fact pseudo-attributes read the source the parser names (#594).
#![cfg(all(feature = "test-oracle", not(target_arch = "wasm32")))]

use minigraf::{Minigraf, QueryResult, Value};

/// A value as text, without `Debug` (CodeQL `rust/cleartext-logging`).
fn key(v: &Value) -> String {
    match v {
        Value::String(s) => format!("s:{s}"),
        Value::Integer(i) => format!("i:{i}"),
        Value::Float(f) => format!("f:{:016x}", f.to_bits()),
        Value::Boolean(b) => format!("b:{b}"),
        Value::Ref(u) => format!("r:{}", u.as_hyphenated()),
        Value::Keyword(k) => format!("k:{k}"),
        Value::Null => "nil".to_string(),
    }
}

fn bag(rows: &[Vec<Value>]) -> Vec<Vec<String>> {
    let mut b: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(key).collect()).collect();
    b.sort();
    b
}

fn executed(db: &Minigraf, q: &str) -> Vec<Vec<String>> {
    match db.execute(q).expect("query") {
        QueryResult::QueryResults { results, .. } => bag(&results),
        _ => panic!("expected QueryResults"),
    }
}

fn pulled(db: &Minigraf, q: &str) -> Vec<Vec<String>> {
    let mut cursor = db.query(q).expect("query");
    let mut rows = Vec::new();
    while let Some(batch) = cursor.next_batch(7).expect("batch") {
        rows.extend(batch.rows().iter().cloned());
    }
    bag(&rows)
}

/// The answer to `q`, or how the plans disagree.
fn check(db: &Minigraf, q: &str) -> Result<Vec<Vec<String>>, String> {
    let on = {
        let _g = minigraf::__reorder(true);
        executed(db, q)
    };
    let runs = [
        ("cursor, reordering", {
            let _g = minigraf::__reorder(true);
            pulled(db, q)
        }),
        ("execute, source order", {
            let _g = minigraf::__reorder(false);
            executed(db, q)
        }),
        ("cursor, source order", {
            let _g = minigraf::__reorder(false);
            pulled(db, q)
        }),
    ];
    for (name, rows) in runs {
        if rows != on {
            return Err(format!(
                "{name}: {} rows, reordering execute: {} rows, for {q}",
                rows.len(),
                on.len()
            ));
        }
    }
    Ok(on)
}

/// The answer to `q`, after checking that every plan gives it.
fn same(db: &Minigraf, q: &str) -> Vec<Vec<String>> {
    check(db, q).unwrap_or_else(|e| panic!("{e}"))
}

fn bob() -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b":bob")
}

fn people(db: &Minigraf) {
    let long_a = "a".repeat(80);
    let long_b = "b".repeat(80);
    db.execute(&format!(
        r#"(transact [[:alice :name "Alice"] [:alice :age 30] [:alice :friend :bob]
                      [:bob :name "Bob"] [:bob :age 40] [:bob :friend :carol]
                      [:carol :name "Carol"] [:carol :age 40] [:carol :friend #uuid "{}"]
                      [:dave :name "Dave"] [:dave :age 25]
                      [:p1 :score 41] [:p2 :score 31] [:p3 :score 41.0]
                      [:alice :x 0.0] [:bob :y -0.0] [:carol :x 1.5] [:dave :y 1.5]
                      [:alice :bio "{long_a}"] [:bob :motto "{long_a}"] [:carol :motto "{long_b}"]])"#,
        bob().as_hyphenated()
    ))
    .unwrap();
    db.execute(r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2021-01-01T00:00:00Z"} [[:erin :name "Erin"] [:erin :age 50]])"#)
        .unwrap();
    db.execute("(retract [[:dave :age 25]])").unwrap();
    db.execute("(rule [(knows ?a ?b) [?a :friend ?b]])")
        .unwrap();
    db.execute("(rule [(knows ?a ?c) [?a :friend ?b] (knows ?b ?c)])")
        .unwrap();
    db.execute("(rule [(lonely ?e) [?e :name _] (not [?e :friend _])])")
        .unwrap();
    db.execute("(rule [(stamped ?e ?t) [?e :name _] [?e :db/tx-count ?t]])")
        .unwrap();
}

/// Queries whose answers must not depend on the plan.
const MATRIX: &[&str] = &[
    // Patterns, joins, cross products.
    r#"[:find ?n ?a :where [?e :name ?n] [?e :age ?a]]"#,
    r#"[:find ?n ?a :where [?e :age ?a] [?e :name ?n]]"#,
    r#"[:find ?n :where [?e :age ?a] [?e :name "Alice"] [?e :name ?n]]"#,
    r#"[:find ?n ?fn :where [?e :friend ?f] [?f :name ?fn] [?e :name ?n]]"#,
    r#"[:find ?n ?m :where [?e :name ?n] [?f :score ?m]]"#,
    r#"[:find ?e ?f :where [?e :age ?a] [?f :age ?a]]"#,
    // Keyword or ref (#545 decision 2), both text orders.
    r#"[:find ?b ?n :where [?a :friend ?b] [?b :name ?n]]"#,
    r#"[:find ?b ?n :where [?b :name ?n] [?a :friend ?b]]"#,
    // Floats: 0.0 and -0.0 join; an integer never equals a float.
    r#"[:find ?e ?f :where [?e :x ?v] [?f :y ?v]]"#,
    r#"[:find ?e ?f :where [?e :score ?v] [?f :score ?v]]"#,
    // Long strings.
    r#"[:find ?e ?f :where [?e :bio ?s] [?f :motto ?s]]"#,
    // Filter and binding expressions.
    r#"[:find ?n :where [?e :age ?a] [(> ?a 30)] [?e :name ?n]]"#,
    r#"[:find ?n ?b :where [?e :age ?a] [(+ ?a 1) ?b] [?e :name ?n]]"#,
    // #596: an expression binding a variable a pattern binds unifies.
    r#"[:find ?a ?x :where [?e :age ?a] [:p1 :score ?x] [(+ ?a 1) ?x]]"#,
    r#"[:find ?e ?s :where [?e :age ?a] [(+ ?a 1) ?s] [?p :score ?s]]"#,
    r#"[:find ?e :where [?e :age ?v] [(+ ?v 1) ?v]]"#,
    r#"[:find ?e ?x :where [?e :age ?a] [(+ ?a 1) ?x] [(- ?a -1) ?x]]"#,
    r#"[:find ?e ?x :where [?e :age ?a] [(+ ?a 1) ?x] [(* ?a 2) ?x]]"#,
    r#"[:find ?e ?x :where [?e :age ?a] [(+ ?a 1.0) ?x] [?p :score ?x]]"#,
    // Rules: recursive, and with negation.
    r#"[:find ?x ?y :where (knows ?x ?y)]"#,
    r#"[:find ?n ?y :where (knows ?x ?y) [?x :name ?n]]"#,
    r#"[:find ?n :where (lonely ?e) [?e :name ?n]]"#,
    r#"[:find ?n ?b :where (knows ?x ?y) [?x :age ?a] [(+ ?a 1) ?b] [?x :name ?n]]"#,
    // or / or-join, including #250's pure-filter and covered-key shapes.
    r#"[:find ?n :where [?e :name ?n] (or [?e :age 40] [?e :friend :bob])]"#,
    r#"[:find ?n :where [?e :name ?n] (or [?e :name "Alice"] [?e :name "Bob"])]"#,
    r#"[:find ?n ?v :where [?e :name ?n] (or [?e :age ?v] [?e :x ?v])]"#,
    r#"[:find ?n :where [?e :name ?n] (or-join [?e] [?e :friend ?f] [?e :x ?s])]"#,
    r#"[:find ?n :where [?e :name ?n] (or-join [?e] [?e :age 40] [?e :friend _])]"#,
    // not / not-join, placed and deferred.
    r#"[:find ?n :where [?e :name ?n] (not [?e :friend _])]"#,
    r#"[:find ?n :where [?e :name ?n] (not-join [?e] [?e :friend ?f] [?f :age 40])]"#,
    r#"[:find ?n :where [?e :name ?n] (or [?e :age ?a] [?e :score ?a]) (not [(> ?a 35)])]"#,
    r#"[:find ?n :where [?e :name ?n] [?e :age ?a] (not-join [?a] [?f :age ?a] [?f :friend :carol])]"#,
    // Aggregates.
    r#"[:find ?a (count ?e) :where [?e :age ?a] [?e :name _]]"#,
    // Per-fact pseudo-attributes (#594): #594's example in both text orders,
    // across joins, in `not`, `not-join`, `or`, `or-join` and rule bodies.
    r#"[:find ?n ?t :any-valid-time :where [?e :name ?n] [?e :age ?a] [?e :db/tx-count ?t]]"#,
    r#"[:find ?n ?t :any-valid-time :where [?e :age ?a] [?e :name ?n] [?e :db/tx-count ?t]]"#,
    r#"[:find ?n ?t :any-valid-time :where [?e :name "Alice"] [?e :age ?a] [?e :name ?n] [?e :db/tx-id ?t]]"#,
    r#"[:find ?e ?t ?f :any-valid-time :where [?e :friend ?b] [?b :name ?n] [?e :db/tx-count ?t] [?e :db/valid-from ?f]]"#,
    r#"[:find ?e ?a ?t :any-valid-time :where [?e ?a _] [?e :db/tx-count ?t]]"#,
    r#"[:find ?e ?t :any-valid-time :where [?e :age ?a] [?p :score ?t] [?e :db/tx-count ?t]]"#,
    r#"[:find ?n :any-valid-time :where [?e :name ?n] (not [?e :db/tx-count 1])]"#,
    r#"[:find ?n :any-valid-time :where [?e :name ?n] (not-join [?e] [?e :db/valid-to ?u] [(< ?u 0)])]"#,
    r#"[:find ?n :any-valid-time :where [?e :name ?n] (or [?e :db/tx-count 1] [?e :age 40])]"#,
    r#"[:find ?n :any-valid-time :where [?e :name ?n] (or-join [?e] [?e :db/tx-count 1] [?e :friend _])]"#,
    r#"[:find ?e ?t :any-valid-time :where (stamped ?e ?t) [?e :age ?a]]"#,
    // `:db/valid-at` first in the text: it never gives a variable's value.
    // (A fixed `:valid-at`: by default `?t` is each query's "now".)
    r#"[:find ?b ?t :valid-at "2030-01-01T00:00:00Z" :where [?b :db/valid-at ?t] [?a :friend ?b] [?b :name ?n]]"#,
    r#"[:find ?b :where [?b :db/valid-at ?t] [?a :friend ?b]]"#,
    // Time travel.
    r#"[:find ?n :as-of 1 :where [?e :name ?n] [?e :age ?a]]"#,
    r#"[:find ?n ?a :valid-at "2020-06-01T00:00:00Z" :where [?e :name ?n] [?e :age ?a]]"#,
    r#"[:find ?n ?a :any-valid-time :where [?e :name ?n] [?e :age ?a]]"#,
];

fn run_matrix(db: &Minigraf) {
    let failures: Vec<String> = MATRIX
        .iter()
        .filter_map(|q| check(db, &format!("(query {q})")).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn matrix_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    people(&db);
    run_matrix(&db);
}

#[test]
fn matrix_file_backed_after_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        people(&db);
        db.checkpoint().unwrap();
        // Pending facts over committed ones.
        db.execute(
            r#"(transact [[:frank :name "Frank"] [:frank :age 40] [:frank :friend :alice]])"#,
        )
        .unwrap();
        run_matrix(&db);
    }
    let db = Minigraf::open(&path).unwrap();
    db.execute("(rule [(knows ?a ?b) [?a :friend ?b]])")
        .unwrap();
    db.execute("(rule [(knows ?a ?c) [?a :friend ?b] (knows ?b ?c)])")
        .unwrap();
    db.execute("(rule [(lonely ?e) [?e :name _] (not [?e :friend _])])")
        .unwrap();
    db.execute("(rule [(stamped ?e ?t) [?e :name _] [?e :db/tx-count ?t]])")
        .unwrap();
    run_matrix(&db);
}

// ── #596: expression bindings unify ──────────────────────────────────────────

#[test]
fn issue_596_example() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :age 30] [:b :age 40] [:p1 :score 41]])")
        .unwrap();
    let r = same(
        &db,
        "(query [:find ?a ?x :where [?e :age ?a] [:p1 :score ?x] [(+ ?a 1) ?x]])",
    );
    assert_eq!(
        r,
        vec![vec!["i:40".to_string(), "i:41".to_string()]],
        "only the row where ?a + 1 is the score"
    );
}

#[test]
fn expression_output_equal_to_its_input_never_matches() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :age 30]])").unwrap();
    let r = same(&db, "(query [:find ?e :where [?e :age ?v] [(+ ?v 1) ?v]])");
    assert!(r.is_empty(), "?v + 1 is never ?v");
}

#[test]
fn two_expressions_on_one_variable_must_agree() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :age 0] [:b :age 3]])").unwrap();
    // 2a = a + a for every a; a + 1 = 2a only for a = 1 (none here).
    let r = same(
        &db,
        "(query [:find ?a ?x :where [?e :age ?a] [(* ?a 2) ?x] [(+ ?a ?a) ?x]])",
    );
    assert_eq!(r.len(), 2, "both agree for every age");
    let r = same(
        &db,
        "(query [:find ?a ?x :where [?e :age ?a] [(* ?a 2) ?x] [(+ ?a 1) ?x]])",
    );
    assert!(r.is_empty(), "they never agree here");
}

#[test]
fn unify_int_never_equals_float() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :age 40] [:p :score 41] [:q :score 41.0]])")
        .unwrap();
    let r = same(
        &db,
        "(query [:find ?p :where [?e :age ?a] [(+ ?a 1) ?x] [?p :score ?x]])",
    );
    assert_eq!(r.len(), 1, "41 matches 41, not 41.0");
    let r = same(
        &db,
        "(query [:find ?p :where [?e :age ?a] [(+ ?a 1.0) ?x] [?p :score ?x]])",
    );
    assert_eq!(r.len(), 1, "41.0 matches 41.0, not 41");
}

#[test]
fn unify_negative_zero_equals_zero() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 0.0] [:b :y -0.0]])").unwrap();
    let r = same(
        &db,
        "(query [:find ?e ?f :where [?e :x ?v] [(* ?v 1.0) ?w] [?f :y ?w]])",
    );
    assert_eq!(r.len(), 1, "0.0 unifies with -0.0");
}
