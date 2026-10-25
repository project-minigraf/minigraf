//! Integration tests for Phase 7.7b — User-Defined Functions (UDFs).
//!
//! Testing conventions: no `{:?}` of Result/Value/Fact in assert messages.

use minigraf::db::Minigraf;
use minigraf::{MinigrafError, Value};
use std::any::Any;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn db() -> Minigraf {
    Minigraf::in_memory().expect("in_memory failed")
}

fn seed(db: &Minigraf, cmds: &[&str]) {
    for cmd in cmds {
        db.execute(cmd).expect("seed failed");
    }
}

/// How a test registers its functions: each test runs once per way, so the
/// `try_register_*` methods with closures that never fail are checked to
/// give the same rows as `register_*` (#180).
#[derive(Clone, Copy)]
enum Reg {
    Plain,
    Try,
}

const REGS: [Reg; 2] = [Reg::Plain, Reg::Try];

impl Reg {
    fn aggregate<Acc: Any + Send + 'static>(
        self,
        db: &Minigraf,
        name: &str,
        init: impl Fn() -> Acc + Send + Sync + 'static,
        step: impl Fn(&mut Acc, &Value) + Send + Sync + 'static,
        finalise: impl Fn(&Acc, usize) -> Value + Send + Sync + 'static,
    ) -> Result<(), MinigrafError> {
        match self {
            Reg::Plain => db.register_aggregate(name, init, step, finalise),
            Reg::Try => db.try_register_aggregate(
                name,
                move || Ok(init()),
                move |acc, v| {
                    step(acc, v);
                    Ok(())
                },
                move |acc, n| Ok(finalise(acc, n)),
            ),
        }
    }

    fn predicate(
        self,
        db: &Minigraf,
        name: &str,
        f: impl Fn(&Value) -> bool + Send + Sync + 'static,
    ) -> Result<(), MinigrafError> {
        match self {
            Reg::Plain => db.register_predicate(name, f),
            Reg::Try => db.try_register_predicate(name, move |v| Ok(f(v))),
        }
    }
}

// ─── Test 1: custom aggregate (geometric mean) ───────────────────────────────

#[test]
fn custom_aggregate_geometric_mean() {
    for reg in REGS {
        let db = db();
        // geometric mean = exp(mean of ln(values))
        reg.aggregate(
            &db,
            "geomean",
            || (0.0_f64, 0usize), // (sum_of_ln, count)
            |acc: &mut (f64, usize), v: &Value| {
                if let Value::Float(f) = v {
                    if *f > 0.0 {
                        acc.0 += f.ln();
                        acc.1 += 1;
                    }
                } else if let Value::Integer(i) = v
                    && *i > 0
                {
                    acc.0 += (*i as f64).ln();
                    acc.1 += 1;
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
}

// ─── Test 2: custom aggregate — empty result returns Null ────────────────────

#[test]
fn custom_aggregate_empty_result() {
    for reg in REGS {
        let db = db();
        reg.aggregate(
            &db,
            "geomean2",
            || (0.0_f64, 0usize),
            |acc: &mut (f64, usize), v: &Value| {
                if let Value::Float(f) = v
                    && *f > 0.0
                {
                    acc.0 += f.ln();
                    acc.1 += 1;
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
}

// ─── Test 3: custom predicate filter ────────────────────────────────────────

#[test]
fn custom_predicate_filter() {
    for reg in REGS {
        let db = db();
        reg.predicate(
            &db,
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
}

// ─── Test 4: UDF as window function ─────────────────────────────────────────

#[test]
fn udf_as_window_function() {
    for reg in REGS {
        let db = db();
        reg.aggregate(
            &db,
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
}

// ─── Test 5: name collision — shadowing a built-in aggregate ─────────────────

#[test]
fn name_collision_builtin_aggregate() {
    for reg in REGS {
        let db = db();
        let result = reg.aggregate(
            &db,
            "sum",
            || 0i64,
            |_acc: &mut i64, _v: &Value| {},
            |acc: &i64, _n: usize| Value::Integer(*acc),
        );
        assert!(result.is_err(), "shadowing built-in 'sum' must return Err");
    }
}

// ─── Test 6: name collision — UDF-on-UDF ─────────────────────────────────────

#[test]
fn name_collision_udf_on_udf() {
    for reg in REGS {
        let db = db();
        reg.aggregate(
            &db,
            "myfn",
            || 0i64,
            |_acc: &mut i64, _v: &Value| {},
            |acc: &i64, _n: usize| Value::Integer(*acc),
        )
        .expect("first registration");

        let result = reg.aggregate(
            &db,
            "myfn",
            || 0i64,
            |_acc: &mut i64, _v: &Value| {},
            |acc: &i64, _n: usize| Value::Integer(*acc),
        );
        assert!(result.is_err(), "duplicate UDF name must return Err");
    }
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
#[cfg_attr(target_os = "wasi", ignore = "spawns threads; WASI has no threads")]
fn thread_safety() {
    for reg in REGS {
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
        reg.aggregate(
            &db,
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
}

// ─── Test 14: UDF predicate in rule body (issue #83) ─────────────────────────────--

#[test]
fn udf_predicate_works_in_rule_body() {
    for reg in REGS {
        let db = db();
        reg.predicate(
            &db,
            "large?",
            |v| matches!(v, Value::Integer(n) if *n > 100),
        )
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
}

// ─── Failing user functions → QRY-014 (#180) ─────────────────────────────────

use minigraf::{BindValue, UdfError};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `n` entities `:n 0` … `:n n-1`, in one transaction.
fn seed_n(db: &Minigraf, n: usize) {
    let facts: String = (0..n).map(|i| format!("[:e{i} :n {i}]")).collect();
    db.execute(&format!("(transact [{facts}])")).expect("seed");
}

/// Registers `boom?`, a predicate that is true until its `fail_at`-th call,
/// which fails. Returns its call counter.
fn failing_predicate(db: &Minigraf, fail_at: usize) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    db.try_register_predicate("boom?", move |_| {
        if c.fetch_add(1, Ordering::SeqCst) + 1 == fail_at {
            Err(UdfError::new("bad value"))
        } else {
            Ok(true)
        }
    })
    .expect("register boom?");
    calls
}

fn assert_qry014(err: &MinigrafError, name: &str, message: &str) {
    assert_eq!(err.code(), "QRY-014");
    assert_eq!(
        err.to_string(),
        format!("[QRY-014] user function `{name}` failed: {message}")
    );
}

#[test]
fn failing_predicate_fails_the_query_in_every_position() {
    for (what, rule, query) in [
        (
            ":where",
            None,
            "(query [:find ?e :where [?e :n ?v] [(boom? ?v)]])",
        ),
        (
            "rule body",
            Some("(rule [(ok ?e) [?e :n ?v] [(boom? ?v)]])"),
            "(query [:find ?e :where (ok ?e)])",
        ),
        (
            "not body",
            None,
            "(query [:find ?e :where [?e :n ?v] (not [(boom? ?v)])])",
        ),
        (
            "or branch",
            None,
            "(query [:find ?e :where [?e :n ?v] (or [(boom? ?v)] [(nil? ?v)])])",
        ),
    ] {
        let db = db();
        seed_n(&db, 10);
        if let Some(rule) = rule {
            db.execute(rule).expect("rule");
        }
        let calls = failing_predicate(&db, 5);
        let err = db.execute(query).expect_err(what);
        assert_qry014(&err, "boom?", "bad value");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            5,
            "{what}: no call after the failing one"
        );
    }
}

#[test]
fn failing_predicate_through_a_cursor_returns_earlier_rows_then_the_error() {
    let db = db();
    seed_n(&db, 1500);
    let calls = failing_predicate(&db, 1500);
    let mut cursor = db
        .query("(query [:find ?e :where [?e :n ?v] [(boom? ?v)]])")
        .expect("opens");
    let mut rows = 0;
    let err = loop {
        match cursor.next_batch(100) {
            Ok(Some(batch)) => rows += batch.len(),
            Ok(None) => panic!("the cursor ended without the error"),
            Err(e) => break e,
        }
    };
    assert!(
        rows > 0 && rows < 1500,
        "rows before the failure come first"
    );
    assert_qry014(&err, "boom?", "bad value");
    assert!(cursor.next_batch(100).expect("after the error").is_none());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1500,
        "no call after the failing one"
    );
}

#[test]
fn failing_predicate_fails_a_prepared_query() {
    let db = db();
    seed_n(&db, 10);
    let calls = failing_predicate(&db, 3);
    let pq = db
        .prepare("(query [:find ?e :where [?e :n ?v] [(>= ?v $min)] [(boom? ?v)]])")
        .expect("prepare");
    let err = pq
        .execute(&[("min", BindValue::Val(Value::Integer(0)))])
        .expect_err("fails");
    assert_qry014(&err, "boom?", "bad value");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Init,
    Step,
    Finalise,
}

/// Calls of a failing aggregate: every call, and the number of calls made
/// when it failed.
struct AggCalls {
    total: Arc<AtomicUsize>,
    at_failure: Arc<AtomicUsize>,
}

/// Registers `boom`, a sum whose `fail_at`-th call of `phase` fails.
fn failing_aggregate(db: &Minigraf, phase: Phase, fail_at: usize) -> AggCalls {
    let total = Arc::new(AtomicUsize::new(0));
    let at_failure = Arc::new(AtomicUsize::new(0));
    let counts: Arc<[AtomicUsize; 3]> = Arc::new(Default::default());
    let call = {
        let (total, at_failure) = (Arc::clone(&total), Arc::clone(&at_failure));
        move |p: Phase| -> Result<(), UdfError> {
            let n = total.fetch_add(1, Ordering::SeqCst) + 1;
            if p == phase && counts[p as usize].fetch_add(1, Ordering::SeqCst) + 1 == fail_at {
                at_failure.store(n, Ordering::SeqCst);
                return Err(UdfError::new(format!("{p:?} failed")));
            }
            Ok(())
        }
    };
    let (c1, c2, c3) = (call.clone(), call.clone(), call);
    db.try_register_aggregate(
        "boom",
        move || c1(Phase::Init).map(|()| 0i64),
        move |acc: &mut i64, v: &Value| {
            c2(Phase::Step)?;
            if let Value::Integer(i) = v {
                *acc += i;
            }
            Ok(())
        },
        move |acc: &i64, _n| c3(Phase::Finalise).map(|()| Value::Integer(*acc)),
    )
    .expect("register boom");
    AggCalls { total, at_failure }
}

#[test]
fn failing_aggregate_fails_the_query_in_every_phase() {
    let aggregate = "(query [:find ?g (boom ?v) :where [?e :n ?v] [?e :g ?g]])";
    let window = "(query [:find ?e (boom ?v :over (:partition-by ?g :order-by ?v)) :where [?e :n ?v] [?e :g ?g]])";
    for query in [aggregate, window] {
        for (phase, fail_at) in [(Phase::Init, 2), (Phase::Step, 4), (Phase::Finalise, 2)] {
            let db = db();
            let facts: String = (0..10)
                .map(|i| format!("[:e{i} :n {i}] [:e{i} :g {}]", i % 3))
                .collect();
            db.execute(&format!("(transact [{facts}])")).expect("seed");
            let calls = failing_aggregate(&db, phase, fail_at);
            let err = db.execute(query).expect_err("fails");
            assert_qry014(&err, "boom", &format!("{phase:?} failed"));
            let at = calls.at_failure.load(Ordering::SeqCst);
            assert!(at > 0, "the failing call was made");
            assert_eq!(
                calls.total.load(Ordering::SeqCst),
                at,
                "no call after the failing one"
            );
        }
    }
}
