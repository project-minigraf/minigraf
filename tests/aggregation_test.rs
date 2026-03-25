//! Integration tests for Phase 7.2a aggregation.
//! Covers count, sum, min, max, :with, rules, negation, and temporal queries.

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

fn vars(r: &QueryResult) -> &Vec<String> {
    match r {
        QueryResult::QueryResults { vars, .. } => vars,
        _ => panic!("expected QueryResults"),
    }
}

// ── count ─────────────────────────────────────────────────────────────────────

#[test]
fn count_all() {
    let db = db();
    db.execute(r#"(transact [[:a :t "x"] [:b :t "y"] [:c :t "z"]])"#).unwrap();
    let r = db.execute(r#"(query [:find (count ?e) :where [?e :t ?v]])"#).unwrap();
    assert_eq!(vars(&r), &["(count ?e)"]);
    assert_eq!(results(&r).len(), 1);
    assert_eq!(results(&r)[0][0], Value::Integer(3));
}

#[test]
fn count_with_grouping() {
    let db = db();
    db.execute(r#"(transact [[:a :dept "eng"] [:b :dept "eng"] [:c :dept "hr"]])"#).unwrap();
    let r = db
        .execute(r#"(query [:find ?dept (count ?e) :where [?e :dept ?dept]])"#)
        .unwrap();
    let mut rows = results(&r).clone();
    rows.sort_by_key(|r| match &r[0] { Value::String(s) => s.clone(), _ => String::new() });
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec![Value::String("eng".to_string()), Value::Integer(2)]);
    assert_eq!(rows[1], vec![Value::String("hr".to_string()), Value::Integer(1)]);
}

#[test]
fn count_distinct_deduplicates() {
    let db = db();
    // Three entities share two distinct :tag values
    db.execute(r#"(transact [[:a :tag "x"] [:b :tag "x"] [:c :tag "y"]])"#).unwrap();
    let r_count = db.execute(r#"(query [:find (count ?v) :where [?e :tag ?v]])"#).unwrap();
    let r_distinct = db.execute(r#"(query [:find (count-distinct ?v) :where [?e :tag ?v]])"#).unwrap();
    assert_eq!(results(&r_count)[0][0], Value::Integer(3));
    assert_eq!(results(&r_distinct)[0][0], Value::Integer(2));
}

#[test]
fn count_empty_result_no_grouping_vars() {
    let db = db();
    // No facts — count with no grouping vars → [[0]]
    let r = db.execute(r#"(query [:find (count ?e) :where [?e :nonexistent ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 1, "count on empty should return one row");
    assert_eq!(results(&r)[0][0], Value::Integer(0));
}

#[test]
fn count_empty_with_grouping_var() {
    let db = db();
    let r = db
        .execute(r#"(query [:find ?dept (count ?e) :where [?e :dept ?dept]])"#)
        .unwrap();
    assert_eq!(results(&r).len(), 0, "grouped count on empty should return no rows");
}

#[test]
fn count_distinct_empty_result() {
    let db = db();
    let r = db.execute(r#"(query [:find (count-distinct ?v) :where [?e :x ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 1);
    assert_eq!(results(&r)[0][0], Value::Integer(0));
}

// ── sum ───────────────────────────────────────────────────────────────────────

#[test]
fn sum_integers() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score 20] [:c :score 30]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(60));
}

#[test]
fn sum_mixed_widens_to_float() {
    let db = db();
    db.execute(r#"(transact [[:a :v 10] [:b :v 0.5]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :v ?v]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Float(10.5));
}

#[test]
fn sum_distinct_deduplicates() {
    let db = db();
    db.execute(r#"(transact [[:a :v 5] [:b :v 5] [:c :v 10]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum-distinct ?v) :where [?e :v ?v]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(15));
}

#[test]
fn sum_empty_result() {
    let db = db();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :nothing ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 0, "sum on empty should return no rows");
}

#[test]
fn sum_skips_nulls() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score 20]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(30));
}

#[test]
fn sum_type_error() {
    let db = db();
    db.execute(r#"(transact [[:a :v "not-a-number"]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "sum of string should fail");
}

// ── min / max ─────────────────────────────────────────────────────────────────

#[test]
fn min_max_integers() {
    let db = db();
    db.execute(r#"(transact [[:a :n 30] [:b :n 10] [:c :n 20]])"#).unwrap();
    let r_min = db.execute(r#"(query [:find (min ?n) :where [?e :n ?n]])"#).unwrap();
    let r_max = db.execute(r#"(query [:find (max ?n) :where [?e :n ?n]])"#).unwrap();
    assert_eq!(results(&r_min)[0][0], Value::Integer(10));
    assert_eq!(results(&r_max)[0][0], Value::Integer(30));
}

#[test]
fn min_max_strings() {
    let db = db();
    db.execute(r#"(transact [[:a :s "banana"] [:b :s "apple"] [:c :s "cherry"]])"#).unwrap();
    let r_min = db.execute(r#"(query [:find (min ?s) :where [?e :s ?s]])"#).unwrap();
    let r_max = db.execute(r#"(query [:find (max ?s) :where [?e :s ?s]])"#).unwrap();
    assert_eq!(results(&r_min)[0][0], Value::String("apple".to_string()));
    assert_eq!(results(&r_max)[0][0], Value::String("cherry".to_string()));
}

#[test]
fn min_type_error_boolean() {
    let db = db();
    db.execute(r#"(transact [[:a :v true]])"#).unwrap();
    let r = db.execute(r#"(query [:find (min ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "min of boolean should fail");
}

#[test]
fn min_mixed_int_float_error() {
    let db = db();
    db.execute(r#"(transact [[:a :v 1] [:b :v 2.0]])"#).unwrap();
    let r = db.execute(r#"(query [:find (min ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "min of mixed Integer/Float should fail");
}

// ── :with ─────────────────────────────────────────────────────────────────────

#[test]
fn with_adds_to_grouping_key() {
    let db = db();
    // Two entities in same dept, same salary.
    // Without :with, both land in group "eng" → sum = 100.
    // With :with ?e, each entity gets its own group → two rows of 50 each.
    db.execute(
        r#"(transact [[:e1 :dept "eng"] [:e1 :salary 50]
                      [:e2 :dept "eng"] [:e2 :salary 50]])"#,
    )
    .unwrap();

    // Without :with: one group, sum = 100
    let r_no_with = db
        .execute(r#"(query [:find ?dept (sum ?salary) :where [?e :dept ?dept] [?e :salary ?salary]])"#)
        .unwrap();
    assert_eq!(results(&r_no_with).len(), 1);
    assert_eq!(results(&r_no_with)[0][1], Value::Integer(100));

    // With :with ?e: two groups (one per entity), each sum = 50
    let r_with = db
        .execute(r#"(query [:find ?dept (sum ?salary) :with ?e
                            :where [?e :dept ?dept] [?e :salary ?salary]])"#)
        .unwrap();
    assert_eq!(results(&r_with).len(), 2);
    // :with var must NOT appear in output
    assert_eq!(vars(&r_with), &["?dept", "(sum ?salary)"]);
}

// ── parse errors ──────────────────────────────────────────────────────────────

#[test]
fn parse_error_with_without_aggregate() {
    let db = db();
    let r = db.execute(r#"(query [:find ?e :with ?x :where [?e :a ?x]])"#);
    assert!(r.is_err(), ":with without aggregate should fail");
}

#[test]
fn parse_error_unknown_aggregate_function() {
    let db = db();
    let r = db.execute(r#"(query [:find (average ?e) :where [?e :a ?v]])"#);
    assert!(r.is_err(), "unknown aggregate should fail");
}

#[test]
fn parse_error_aggregate_var_unbound() {
    let db = db();
    let r = db.execute(r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#);
    assert!(r.is_err(), "unbound aggregate var should fail");
}

// ── integration: rules, negation, temporal ───────────────────────────────────

#[test]
fn aggregate_after_nonrecursive_rule() {
    let db = db();
    db.execute(r#"(transact [[:a :member :g1] [:b :member :g1] [:c :member :g2]])"#).unwrap();
    db.execute(r#"(rule [(in-group ?e ?g) [?e :member ?g]])"#).unwrap();
    let r = db
        .execute(r#"(query [:find ?g (count ?e) :where (in-group ?e ?g)])"#)
        .unwrap();
    let mut rows = results(&r).clone();
    rows.sort_by_key(|r| match &r[1] { Value::Integer(n) => *n, _ => 0 });
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], Value::Integer(1)); // g2 has 1 member
    assert_eq!(rows[1][1], Value::Integer(2)); // g1 has 2 members
}

#[test]
fn aggregate_after_recursive_rule() {
    let db = db();
    // Chain: a → b → c → d  (3 nodes reachable from a)
    db.execute(r#"(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d]])"#).unwrap();
    db.execute(r#"(rule [(reach ?x ?y) [?x :edge ?y]])"#).unwrap();
    db.execute(r#"(rule [(reach ?x ?y) [?x :edge ?z] (reach ?z ?y)])"#).unwrap();
    let r = db
        .execute(r#"(query [:find (count ?y) :where (reach :a ?y)])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(3));
}

#[test]
fn aggregate_with_negation() {
    let db = db();
    db.execute(
        r#"(transact [[:a :score 10] [:b :score 20] [:b :excluded true]])"#,
    )
    .unwrap();
    let r = db
        .execute(r#"(query [:find (sum ?s) :where [?e :score ?s] (not [?e :excluded true])])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(10));
}

#[test]
fn aggregate_with_as_of() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score 20]])"#).unwrap(); // tx 1
    db.execute(r#"(transact [[:c :score 30]])"#).unwrap(); // tx 2
    // As of tx 1, only :a and :b exist
    let r = db
        .execute(r#"(query [:find (count ?e) :as-of 1 :valid-at :any-valid-time :where [?e :score ?s]])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(2));
}
