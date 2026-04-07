//! Integration tests for Phase 7.7b — User-Defined Functions (UDFs).
//!
//! Testing conventions: no `{:?}` of Result/Value/Fact in assert messages.

use minigraf::db::Minigraf;
use minigraf::Value;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn db() -> Minigraf {
    Minigraf::in_memory().expect("in_memory failed")
}

fn seed(db: &Minigraf, cmds: &[&str]) {
    for cmd in cmds {
        db.execute(cmd).expect("seed failed");
    }
}

// ─── Test 1: custom aggregate (geometric mean) ───────────────────────────────

#[test]
fn custom_aggregate_geometric_mean() {
    let db = db();
    // geometric mean = exp(mean of ln(values))
    db.register_aggregate(
        "geomean",
        || (0.0_f64, 0usize), // (sum_of_ln, count)
        |acc: &mut (f64, usize), v: &Value| {
            if let Value::Float(f) = v {
                if *f > 0.0 {
                    acc.0 += f.ln();
                    acc.1 += 1;
                }
            } else if let Value::Integer(i) = v {
                if *i > 0 {
                    acc.0 += (*i as f64).ln();
                    acc.1 += 1;
                }
            }
        },
        |acc: &(f64, usize), _n: usize| {
            if acc.1 == 0 {
                Value::Null
            } else {
                Value::Float((acc.0 / acc.1 as f64).exp())
            }
        },
    )
    .expect("register geomean");

    seed(
        &db,
        &[r#"(transact [[:a :item/score 2] [:b :item/score 8]])"#],
    );

    let result = db
        .execute(r#"(query [:find (geomean ?score) :where [?e :item/score ?score]])"#)
        .expect("query failed");

    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert_eq!(results.len(), 1);
        // geomean(2, 8) = sqrt(16) = 4.0
        if let Value::Float(f) = &results[0][0] {
            assert!((*f - 4.0).abs() < 1e-9, "expected ~4.0");
        } else {
            panic!("expected Float result");
        }
    } else {
        panic!("expected QueryResults");
    }
}

// ─── Test 2: custom aggregate — empty result returns Null ────────────────────

#[test]
fn custom_aggregate_empty_result() {
    let db = db();
    db.register_aggregate(
        "geomean2",
        || (0.0_f64, 0usize),
        |acc: &mut (f64, usize), v: &Value| {
            if let Value::Float(f) = v {
                if *f > 0.0 {
                    acc.0 += f.ln();
                    acc.1 += 1;
                }
            }
        },
        |acc: &(f64, usize), _n: usize| {
            if acc.1 == 0 {
                Value::Null
            } else {
                Value::Float((acc.0 / acc.1 as f64).exp())
            }
        },
    )
    .expect("register geomean2");

    // No facts — empty result
    let result = db
        .execute(r#"(query [:find (geomean2 ?score) :where [?e :item/score ?score]])"#)
        .expect("query failed");

    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        // Empty input → no groups → no output rows (consistent with built-in behaviour
        // for non-count aggregates when no facts match)
        assert!(results.is_empty() || results[0][0] == Value::Null);
    } else {
        panic!("expected QueryResults");
    }
}

// ─── Test 3: custom predicate filter ────────────────────────────────────────

#[test]
fn custom_predicate_filter() {
    let db = db();
    db.register_predicate(
        "email?",
        |v: &Value| matches!(v, Value::String(s) if s.contains('@')),
    )
    .expect("register email?");

    seed(
        &db,
        &[r#"(transact [
        [:alice :person/email "alice@example.com"]
        [:bob   :person/email "notanemail"]
    ])"#],
    );

    let result = db
        .execute(r#"(query [:find ?e :where [?e :person/email ?addr] [(email? ?addr)]])"#)
        .expect("query failed");

    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert_eq!(results.len(), 1, "only alice has a valid email");
        // Entity IDs are stored as Value::Ref(deterministic UUID derived from the keyword).
        // We verify the result is a Ref (entity ID) rather than checking the exact UUID.
        assert!(
            matches!(results[0][0], Value::Ref(_)),
            "entity result must be a Ref"
        );
    } else {
        panic!("expected QueryResults");
    }
}

// ─── Test 4: UDF as window function ─────────────────────────────────────────

#[test]
fn udf_as_window_function() {
    let db = db();
    db.register_aggregate(
        "winsum",
        || 0i64,
        |acc: &mut i64, v: &Value| {
            if let Value::Integer(i) = v {
                *acc += i;
            }
        },
        |acc: &i64, _n: usize| Value::Integer(*acc),
    )
    .expect("register winsum");

    seed(
        &db,
        &[r#"(transact [
        [:a :item/score 1]
        [:b :item/score 2]
        [:c :item/score 3]
    ])"#],
    );

    let result = db
        .execute(
            r#"(query [:find ?e (winsum ?score :over (:order-by ?score))
                  :where [?e :item/score ?score]])"#,
        )
        .expect("query failed");

    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert_eq!(results.len(), 3, "three rows");
        // After ordering by score: rows are 1,2,3; cumulative sums are 1,3,6
        let mut sums: Vec<i64> = results
            .iter()
            .map(|r| if let Value::Integer(n) = r[1] { n } else { -1 })
            .collect();
        sums.sort();
        assert_eq!(sums, vec![1, 3, 6]);
    } else {
        panic!("expected QueryResults");
    }
}

// ─── Test 5: name collision — shadowing a built-in aggregate ─────────────────

#[test]
fn name_collision_builtin_aggregate() {
    let db = db();
    let result = db.register_aggregate(
        "sum",
        || 0i64,
        |_acc: &mut i64, _v: &Value| {},
        |acc: &i64, _n: usize| Value::Integer(*acc),
    );
    assert!(result.is_err(), "shadowing built-in 'sum' must return Err");
}

// ─── Test 6: name collision — UDF-on-UDF ─────────────────────────────────────

#[test]
fn name_collision_udf_on_udf() {
    let db = db();
    db.register_aggregate(
        "myfn",
        || 0i64,
        |_acc: &mut i64, _v: &Value| {},
        |acc: &i64, _n: usize| Value::Integer(*acc),
    )
    .expect("first registration");

    let result = db.register_aggregate(
        "myfn",
        || 0i64,
        |_acc: &mut i64, _v: &Value| {},
        |acc: &i64, _n: usize| Value::Integer(*acc),
    );
    assert!(result.is_err(), "duplicate UDF name must return Err");
}

// ─── Test 7: unknown aggregate function → runtime error ──────────────────────

#[test]
fn unknown_function_runtime_error() {
    let db = db();
    seed(&db, &[r#"(transact [[:a :x 1]])"#]);
    let result = db.execute(r#"(query [:find (nosuchfn ?x) :where [?e :x ?x]])"#);
    assert!(
        result.is_err(),
        "unknown aggregate should return Err, not panic"
    );
}

// ─── Test 8: unknown predicate → runtime error ───────────────────────────────

#[test]
fn unknown_predicate_runtime_error() {
    let db = db();
    seed(&db, &[r#"(transact [[:a :x "hello"]])"#]);
    let result = db.execute(r#"(query [:find ?e :where [?e :x ?v] [(nosuchpred? ?v)]])"#);
    assert!(
        result.is_err(),
        "unknown predicate should return Err, not panic"
    );
}

// ─── Test 9: thread safety ───────────────────────────────────────────────────

#[test]
fn thread_safety() {
    use std::sync::Arc;

    let db = Arc::new(db());

    // Seed some facts.
    db.execute(r#"(transact [[:a :x 1] [:b :x 2]])"#)
        .expect("seed");

    // Spawn reader threads.
    let mut handles = Vec::new();
    for _ in 0..4 {
        let db2 = Arc::clone(&db);
        handles.push(std::thread::spawn(move || {
            for _ in 0..10 {
                db2.execute(r#"(query [:find ?e :where [?e :x _]])"#)
                    .expect("concurrent read");
            }
        }));
    }

    // Register a UDF from the main thread while readers run.
    db.register_aggregate(
        "threadfn",
        || 0i64,
        |acc: &mut i64, v: &Value| {
            if let Value::Integer(i) = v {
                *acc += i;
            }
        },
        |acc: &i64, _n: usize| Value::Integer(*acc),
    )
    .expect("register threadfn");

    for h in handles {
        h.join().expect("reader thread panicked");
    }

    // Verify the UDF works after concurrent registration.
    let result = db
        .execute(r#"(query [:find (threadfn ?x) :where [?e :x ?x]])"#)
        .expect("post-registration query");
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert_eq!(results.len(), 1);
        assert_eq!(results[0][0], Value::Integer(3)); // 1 + 2
    } else {
        panic!("expected QueryResults");
    }
}

// ─── Test 14: UDF predicate in rule body (issue #83) ─────────────────────────────--

#[test]
fn udf_predicate_works_in_rule_body() {
    let db = db();
    db.register_predicate("large?", |v| matches!(v, Value::Integer(n) if *n > 100))
        .unwrap();
    db.execute(r#"(transact [[:e1 :score 200] [:e2 :score 50]])"#)
        .unwrap();
    db.execute(r#"(rule [(high-scorer ?e) [?e :score ?v] [(large? ?v)]])"#)
        .unwrap();
    let result = db
        .execute(r#"(query [:find ?e :where (high-scorer ?e)])"#)
        .unwrap();
    if let minigraf::QueryResult::QueryResults { results, .. } = result {
        assert_eq!(
            results.len(),
            1,
            "only the entity with score > 100 should match"
        );
    } else {
        panic!("expected QueryResults");
    }
}
