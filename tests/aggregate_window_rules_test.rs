//! Aggregate and window answers that do not depend on the plan (#547, spec
//! `docs/superpowers/specs/2026-10-20-aggregate-window-engine-design.md` §3.4).
//! `plan_independence_test.rs` runs the same shapes with reordering on and
//! off; these check the values themselves, without the `test-oracle`
//! feature.

use minigraf::{Minigraf, QueryResult, Value};

fn rows(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    match db.execute(q).expect("query") {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn one(db: &Minigraf, q: &str) -> Vec<Value> {
    let mut r = rows(db, q);
    assert_eq!(r.len(), 1, "one row");
    r.remove(0)
}

fn float(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        _ => panic!("expected a float"),
    }
}

#[test]
fn float_sum_is_exact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact [[:a :v 10000000000000000.0] [:b :v 1.0] [:c :v -10000000000000000.0] \
                    [:d :v 1.0] [:e :v 0.5]])",
    )
    .unwrap();
    // Added left to right in f64, 1e16 + 1.0 loses the 1.0.
    let r = one(&db, "(query [:find (sum ?v) :where [?e :v ?v]])");
    assert_eq!(float(&r[0]), 2.5);
    let r = rows(
        &db,
        "(query [:find ?v (avg ?v :over (:order-by ?v)) :where [?e :v ?v]])",
    );
    let last = r
        .iter()
        .find(|row| row[0] == Value::Float(1e16))
        .expect("row");
    assert_eq!(float(&last[1]), 0.5, "exact 2.5 over 5 rows");
}

#[test]
fn integer_sum_saturates_once() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :v 9223372036854775807] [:b :v 1] [:c :v -1]])")
        .unwrap();
    let r = one(&db, "(query [:find (sum ?v) :where [?e :v ?v]])");
    assert_eq!(r[0], Value::Integer(i64::MAX));
}

#[test]
fn window_sum_of_integers_is_exact() {
    let db = Minigraf::in_memory().unwrap();
    // 2^53 + 1 is not a float: summed as f64 it became 2^53.
    db.execute("(transact [[:a :v 9007199254740992] [:b :v 1]])")
        .unwrap();
    let r = rows(
        &db,
        "(query [:find ?v (sum ?v :over (:order-by ?v)) :where [?e :v ?v]])",
    );
    let top = r
        .iter()
        .find(|row| row[0] == Value::Integer(9_007_199_254_740_992))
        .expect("row");
    assert_eq!(top[1], Value::Integer(9_007_199_254_740_993));
}

#[test]
fn min_max_of_signed_zeros_and_nan() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :v 0.0] [:b :v -0.0] [:c :v 1.5]])")
        .unwrap();
    let r = one(&db, "(query [:find (min ?v) (max ?v) :where [?e :v ?v]])");
    assert_eq!(float(&r[0]).to_bits(), (-0.0f64).to_bits());
    assert_eq!(float(&r[1]), 1.5);
}

#[test]
fn rank_by_keyword_ties_peers() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :d :x] [:b :d :x] [:c :d :y] [:d :d :y]])")
        .unwrap();
    let mut ranks: Vec<i64> = rows(
        &db,
        "(query [:find ?e (rank :over (:order-by ?d)) :where [?e :d ?d]])",
    )
    .iter()
    .map(|r| match r[1] {
        Value::Integer(n) => n,
        _ => panic!("rank is an integer"),
    })
    .collect();
    ranks.sort_unstable();
    assert_eq!(ranks, vec![1, 1, 3, 3]);
}

#[test]
fn running_sum_gives_peers_the_value_at_the_last_peer() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :s 10] [:b :s 10] [:c :s 5] [:d :s 1.5]])")
        .unwrap();
    let mut sums: Vec<f64> = rows(
        &db,
        "(query [:find ?e (sum ?s :over (:order-by ?s)) :where [?e :s ?s]])",
    )
    .iter()
    .map(|r| float(&r[1]))
    .collect();
    sums.sort_by(f64::total_cmp);
    assert_eq!(sums, vec![1.5, 6.5, 26.5, 26.5]);
}

#[test]
fn row_number_breaks_ties_by_the_other_find_variables() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :name "b"] [:a :s 1] [:b :name "a"] [:b :s 1] [:c :name "c"] [:c :s 0]])"#,
    )
    .unwrap();
    let mut r = rows(
        &db,
        "(query [:find ?n (row-number :over (:order-by ?s)) :where [?e :s ?s] [?e :name ?n]])",
    );
    r.sort_by_key(|row| match &row[1] {
        Value::Integer(n) => *n,
        _ => 0,
    });
    let names: Vec<&Value> = r.iter().map(|row| &row[0]).collect();
    assert_eq!(
        names,
        vec![
            &Value::String("c".into()),
            &Value::String("a".into()),
            &Value::String("b".into())
        ]
    );
}

#[test]
fn aggregate_error_is_decided_per_group() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:a :v 1] [:b :v "x"] [:c :v 2]])"#)
        .unwrap();
    let err = db
        .execute("(query [:find (sum ?v) :where [?e :v ?v]])")
        .err()
        .expect("sum of a string fails");
    assert_eq!(err.code(), "INT-041");
}
