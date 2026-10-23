//! Every plan returns the same rows the same number of times (#596, design
//! `docs/superpowers/specs/2026-10-15-plan-independent-answers-design.md`).
//!
//! Each query runs through `Minigraf::execute` and through a drained
//! `Minigraf::query` cursor, with clause reordering on and off
//! (`minigraf::__reorder`), and with the engine's `not` bodies chosen,
//! forced hashed and forced correlated (`minigraf::__subplans`, #546); all
//! twelve answers must be the same bag. With
//! the `test-oracle` feature, every query the streaming engine runs is also
//! run by the executor and must agree, so each run compares the two as well.
//! Per-fact pseudo-attributes read the source the parser names (#594).
#![cfg(all(feature = "test-oracle", not(target_arch = "wasm32")))]

mod answers;

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
    use minigraf::Subplans;
    let on = {
        let _g = minigraf::__reorder(true);
        match db.execute(q).expect("query") {
            QueryResult::QueryResults { results, .. } => {
                answers::check("plan_independence_test", q, &results);
                bag(&results)
            }
            _ => panic!("expected QueryResults"),
        }
    };
    for mode in [Subplans::Choose, Subplans::Hash, Subplans::Correlated] {
        let _m = minigraf::__subplans(mode);
        for reorder in [true, false] {
            let _g = minigraf::__reorder(reorder);
            for (how, rows) in [("execute", executed(db, q)), ("cursor", pulled(db, q))] {
                if rows != on {
                    let order = if reorder {
                        "reordering"
                    } else {
                        "source order"
                    };
                    return Err(format!(
                        "{how}, {order}, bodies {mode:?}: {} rows, reordering execute: {} rows, for {q}",
                        rows.len(),
                        on.len()
                    ));
                }
            }
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
                      [:alice :bio "{long_a}"] [:bob :motto "{long_a}"] [:carol :motto "{long_b}"]
                      [:z1 :tiny 0.00000000000000000001] [:z2 :tiny2 0.00000000000000000002]])"#,
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
    // Aggregates and windows (#547): exact sums, peers, the total value order.
    r#"[:find ?a (count ?e) :where [?e :age ?a]]"#,
    r#"[:find (sum ?s) (count-distinct ?s) :where [?p :score ?s]]"#,
    r#"[:find (sum ?v) (min ?v) (max ?v) (count-distinct ?v) :where [?e ?attr ?v] [(float? ?v)]]"#,
    r#"[:find ?n (row-number :over (:order-by ?a)) :where [?e :age ?a] [?e :name ?n]]"#,
    r#"[:find ?n (sum ?a :over (:order-by ?a)) (rank :over (:order-by ?a :desc)) :where [?e :name ?n] [?e :age ?a]]"#,
    r#"[:find ?e (rank :over (:order-by ?b)) (row-number :over (:order-by ?b)) :where [?e :friend ?b]]"#,
    r#"[:find ?a (count ?e) (row-number :over (:order-by ?a :desc)) :where [?e :age ?a]]"#,
    r#"[:find ?a (count ?b) :where (knows ?a ?b)]"#,
    // A group of `0.0` and `-0.0` returns `-0.0`, the least by value order.
    r#"[:find ?v (count ?e) :where [?e ?attr ?v] [(float? ?v)]]"#,
    r#"[:find ?v (count ?e) :with ?attr :where [?e ?attr ?v] [(float? ?v)]]"#,
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
    // #546: imported floats by IEEE `==` (0.0 and -0.0 equal; 1e-20 and
    // 2e-20 not), keywords and refs of one entity, long strings, bodies that
    // read an imported variable they do not bind, no variable at all, or
    // the valid time.
    r#"[:find ?e :where [?e :x ?v] (not-join [?v] [?f :y ?v])]"#,
    r#"[:find ?e :where [?e :x ?v] (not-join [?v] [?f :y ?v] [(>= ?v 0.0)])]"#,
    r#"[:find ?e :where [?e :x ?v] (not [:bob :y ?v])]"#,
    r#"[:find ?e :where [?e :x ?v] (not [:bob :y ?v] [(< ?v 1.0)])]"#,
    r#"[:find ?e :where [?e :tiny ?v] (not-join [?v] [?f :tiny2 ?v])]"#,
    r#"[:find ?e :where [?e :tiny ?v] (not-join [?v] [?f :tiny2 ?v] [(> ?v 0.0)])]"#,
    r#"[:find ?a :where [?a :friend ?b] (not [?b :age 40])]"#,
    r#"[:find ?a :where [?a :friend ?b] (not-join [?b] [?b :age ?x] [(> ?x 35)])]"#,
    r#"[:find ?b :where [?b :name ?n] (not-join [?b] [?a :friend ?b])]"#,
    r#"[:find ?b :where [?b :name ?n] (not-join [?b] [?a :friend ?b] [?a :age 30])]"#,
    r#"[:find ?e :where [?e :bio ?s] (not-join [?s] [?f :motto ?s])]"#,
    r#"[:find ?n :where [?e :name ?n] [?e :age ?a] (not-join [?a] [?f :age ?b] [(< ?b ?a)])]"#,
    r#"[:find ?n :where [?e :name ?n] (not [:alice :age 30])]"#,
    r#"[:find ?n :where [?e :name ?n] (not-join [?e] [?x :age 25])]"#,
    r#"[:find ?n :valid-at "2030-01-01T00:00:00Z" :where [?e :name ?n] (not-join [?e] [?e :db/valid-at ?t] [(> ?t 0)])]"#,
    r#"[:find ?n :valid-at "2030-01-01T00:00:00Z" :where [?e :name ?n] (not-join [?e] [?e :db/valid-at ?t])]"#,
    // #546: `or` / `or-join` by join equality (a keyword meets a ref to the
    // entity it names; floats by IEEE `==`), branches naming different outer
    // variables or no join variable, a branch with `not`, an `or` with
    // nothing before it, a cross product, and what follows an `or`.
    r#"[:find ?a ?n :where [?a :friend ?b] (or [?b :name ?n] [?b :motto ?n])]"#,
    r#"[:find ?a :where [?a :friend ?b] (or-join [?b] [?b :age 40] [?b :motto _])]"#,
    r#"[:find ?a :where [?a :friend ?b] (or-join [?b] [?x :friend ?b] [?b :age 25])]"#,
    r#"[:find ?e :where [?e :x ?v] (or-join [?v] [?f :y ?v] [?f :tiny ?v])]"#,
    r#"[:find ?e :where [?e :tiny ?v] (or-join [?v] [?f :tiny2 ?v] [?f :x ?v])]"#,
    r#"[:find ?e :where [?e :bio ?s] (or-join [?s] [?f :motto ?s] [?f :name ?s])]"#,
    r#"[:find ?a ?b ?x :where [?a :friend ?b] (or [?a :age ?x] [?b :age ?x])]"#,
    r#"[:find ?n :where [?e :name ?n] (or-join [?e] [?x :score 41] [?e :age 99])]"#,
    r#"[:find ?n ?a :where [?e :name ?n] (or (and [?e :age ?a] (not [?e :friend :bob])) [?e :score ?a])]"#,
    r#"[:find ?n :where [?e :name ?n] [?e :age ?a] (or [(> ?a 35)] [?e :friend :bob])]"#,
    r#"[:find ?e :where (or [?e :age 40] [?e :score 41])]"#,
    r#"[:find ?e ?p :where [?e :age 40] (or [?p :score 41] [?p :score 31])]"#,
    r#"[:find ?n ?a :where [?e :name ?n] (or [?e :age ?a] [?e :score ?a]) [(> ?a 30)]]"#,
    r#"[:find ?n :where [?e :name ?n] (or [?e :age 40] [?e :age 30]) (or-join [?e] [?e :friend :carol] [?e :x _])]"#,
    r#"[:find ?n ?t :valid-at "2030-01-01T00:00:00Z" :where [?e :name ?n] (or [?e :db/valid-at ?t] [?e :age ?t])]"#,
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

// ── #620: an `or` branch whose expression reads an outer variable ────────────

fn issue_620_data(db: &Minigraf) {
    db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
}

fn ints(rows: &[Vec<String>]) -> Vec<String> {
    let mut v: Vec<String> = rows.iter().map(|r| r.join(",")).collect();
    v.sort();
    v
}

#[test]
fn or_branch_expression_reads_outer_variable() {
    let db = Minigraf::in_memory().unwrap();
    issue_620_data(&db);
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or [(> ?v 1)] [(< ?v 0)])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "only ?v = 2 is over 1");
    let r = same(
        &db,
        "(query [:find ?w :where [?e :n ?v] (or (and [(= ?v 1)] [?e :n ?w]) (and [(= ?v 2)] [?e :n ?w]))])",
    );
    assert_eq!(ints(&r), vec!["i:1", "i:2"], "each row matches one branch");
    // One branch binds ?v itself, the other only reads it.
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or (and [?e :n ?v] [(> ?v 1)]) [(< ?v 0)])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "mixed branches");
}

#[test]
fn or_branch_expression_binds_from_outer_variable() {
    let db = Minigraf::in_memory().unwrap();
    issue_620_data(&db);
    let r = same(
        &db,
        "(query [:find ?v ?w :where [?e :n ?v] (or [(+ ?v 10) ?w] [(- ?v 10) ?w])])",
    );
    assert_eq!(
        ints(&r),
        vec!["i:1,i:-9", "i:1,i:11", "i:2,i:-8", "i:2,i:12"],
        "both branches bind ?w from ?v"
    );
}

#[test]
fn nested_or_expression_reads_variable_of_enclosing_branch() {
    let db = Minigraf::in_memory().unwrap();
    issue_620_data(&db);
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or (and [?e :n ?v] (or [(> ?v 1)] [(< ?v 0)])) [?e :n 5])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "the inner or sees ?v");
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or (or [(> ?v 1)] [(< ?v 0)]) [?e :n 5])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "the inner or sees the outer ?v");
}

#[test]
fn or_join_branch_expression_reads_join_variable() {
    let db = Minigraf::in_memory().unwrap();
    issue_620_data(&db);
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or-join [?v] [(> ?v 1)] [(< ?v 0)])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "only ?v = 2 is over 1");
    let r = same(
        &db,
        "(query [:find ?v :where [?e :n ?v] (or-join [?v] (and [?x :n ?y] [(> ?v ?y)]) [(< ?v 0)])])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "2 is over some ?y, 1 is not");
}

#[test]
fn or_branch_expression_reads_outer_variable_in_rule_body() {
    let db = Minigraf::in_memory().unwrap();
    issue_620_data(&db);
    db.execute("(rule [(big ?e) [?e :n ?v] (or [(> ?v 1)] [(< ?v 0)])])")
        .unwrap();
    db.execute("(rule [(big2 ?e) [?e :n ?v] (or-join [?v] [(> ?v 1)] [(< ?v 0)])])")
        .unwrap();
    db.execute(
        "(rule [(pick ?e ?w) [?e :n ?v] (or (and [(= ?v 1)] [?e :n ?w]) (and [(= ?v 2)] [?e :n ?w]))])",
    )
    .unwrap();
    let r = same(&db, "(query [:find ?v :where (big ?e) [?e :n ?v]])");
    assert_eq!(ints(&r), vec!["i:2"], "or in a rule body");
    let r = same(&db, "(query [:find ?v :where (big2 ?e) [?e :n ?v]])");
    assert_eq!(ints(&r), vec!["i:2"], "or-join in a rule body");
    let r = same(&db, "(query [:find ?w :where (pick ?e ?w)])");
    assert_eq!(ints(&r), vec!["i:1", "i:2"], "or-and in a rule body");
}

// ── #551: nested `or`s in branches run on the engine ────────────────────────

/// A nested `or` / `or-join` in a branch, one reading outer variables the
/// branch's patterns do not bind (so the branch cannot be hashed), and a
/// rule whose branch nests an `or` around a rule invocation: every plan,
/// hashed or correlated, gives the executor's answer (#551 plan, step 5).
#[test]
fn nested_or_in_a_branch() {
    let db = Minigraf::in_memory().unwrap();
    people(&db);
    db.execute(
        "(rule [(deep ?x ?y) [?x :friend ?y] (or (or (knows ?y ?x) [?y :age 40]) [?x :age 30])])",
    )
    .unwrap();
    for (q, n) in [
        (
            "[:find ?n :where [?e :name ?n] (or-join [?e] (and [?e :age ?a] (or [(> ?a 35)] [?e :friend :bob])) [?e :x 1.5])]",
            3,
        ),
        (
            "[:find ?n :where [?e :name ?n] [?e :age ?a] (or-join [?e ?a] (and [?f :age ?b] (or [(= ?a ?b)] [?e :friend ?f])) [?e :x 1.5])]",
            3,
        ),
        (
            "[:find ?n :where [?e :name ?n] (or-join [?e] (and [?e :age ?a] (or-join [?a] [(> ?a 35)] [(< ?a 31)])) [?e :y _])]",
            4,
        ),
        (
            "[:find ?n :where [?e :name ?n] [?e :age ?a] (or-join [?e ?a] (and [?f :age ?b] (or [(< ?a ?b)] [(> ?a 39)])) [?e :x 1.5])]",
            3,
        ),
        ("[:find ?x ?y :where (deep ?x ?y)]", 3),
    ] {
        let r = same(&db, &format!("(query {q})"));
        assert_eq!(r.len(), n, "{q}");
    }
}

// ── #625: an expression reading a variable only an `or` binds ────────────────

fn issue_625_data(db: &Minigraf) {
    db.execute("(transact [[:a :n 1] [:b :n 2] [:c :ok true]])")
        .unwrap();
}

#[test]
fn expression_reads_variable_bound_only_by_or() {
    let db = Minigraf::in_memory().unwrap();
    issue_625_data(&db);
    let r = same(
        &db,
        "(query [:find ?e ?x :where [?e :n _] (or [?e :n ?x] [?e :ok ?x]) [(> ?x 1)]])",
    );
    assert_eq!(r.len(), 1, "only :b has ?x over 1");
    // Mixed: one variable from a pattern, the other from the or.
    let r = same(
        &db,
        "(query [:find ?x :where [?e :n ?v] (or [?e :n ?x] [?e :ok ?x]) [(= ?x ?v)]])",
    );
    assert_eq!(ints(&r), vec!["i:1", "i:2"], "?x equals ?v on both rows");
}

#[test]
fn expression_chain_on_variable_bound_only_by_or() {
    let db = Minigraf::in_memory().unwrap();
    issue_625_data(&db);
    let r = same(
        &db,
        "(query [:find ?y :where [?e :n _] (or [?e :n ?x] [?e :ok ?x]) [(+ ?x 10) ?y] [(> ?y 11)]])",
    );
    assert_eq!(ints(&r), vec!["i:12"], "?y from ?x, then filtered");
    // A not reading the expression's output runs after it.
    let r = same(
        &db,
        "(query [:find ?y :where [?e :n _] (or [?e :n ?x] [?e :ok ?x]) [(+ ?x 10) ?y] (not [(> ?y 11)])])",
    );
    assert_eq!(ints(&r), vec!["i:11"], "the not sees ?y");
}

#[test]
fn expression_reads_variable_bound_only_by_or_with_rules() {
    let db = Minigraf::in_memory().unwrap();
    issue_625_data(&db);
    db.execute("(rule [(num ?e) [?e :n _]])").unwrap();
    let r = same(
        &db,
        "(query [:find ?x :where (num ?e) (or [?e :n ?x] [?e :ok ?x]) [(> ?x 1)]])",
    );
    assert_eq!(ints(&r), vec!["i:2"], "query calling a rule");
    db.execute("(rule [(big ?e ?x) [?e :n _] (or [?e :n ?x] [?e :ok ?x]) [(> ?x 1)]])")
        .unwrap();
    let r = same(&db, "(query [:find ?x :where (big ?e ?x)])");
    assert_eq!(ints(&r), vec!["i:2"], "rule body");
}
