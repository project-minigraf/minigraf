// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]
//! A query reads its functions from a snapshot of the registry taken when it
//! opens, and holds no lock while a user function runs, so a function may
//! call back into its own database (#180, spec §5).
//!
//! Testing conventions: no `{:?}` of Result/Value/Fact in assert messages.

use minigraf::{Minigraf, QueryResult, Value};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Runs `f` on a fresh thread and returns its result. A call that deadlocks
/// fails the test after 30 s instead of hanging it; the blocked thread is
/// left until the test process exits.
fn within_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(v) => v,
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("deadlocked"),
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("the call panicked"),
    }
}

fn rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

// ─── Registry snapshot (plan step 9) ─────────────────────────────────────────

#[test]
fn a_predicate_that_registers_a_function_returns() {
    let n = within_timeout(|| {
        let db = Minigraf::in_memory().unwrap();
        db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
        let inner = db.clone();
        db.register_predicate("reg?", move |v| {
            if let Value::Integer(i) = v {
                inner
                    .register_predicate(&format!("p{i}?"), |_| true)
                    .expect("registers");
            }
            true
        })
        .unwrap();
        let n = rows(
            db.execute("(query [:find ?e :where [?e :n ?v] [(reg? ?v)]])")
                .unwrap(),
        )
        .len();
        // A later query sees what the callback registered.
        db.execute("(query [:find ?e :where [?e :n ?v] [(p1? ?v)]])")
            .expect("p1? is registered");
        n
    });
    assert_eq!(n, 2);
}

#[test]
fn an_aggregate_that_registers_a_function_returns() {
    let sum = within_timeout(|| {
        let db = Minigraf::in_memory().unwrap();
        db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
        let inner = db.clone();
        db.register_aggregate(
            "regsum",
            || 0i64,
            move |acc: &mut i64, v: &Value| {
                if let Value::Integer(i) = v {
                    *acc += i;
                    inner
                        .register_predicate(&format!("q{i}?"), |_| true)
                        .expect("registers");
                }
            },
            |acc: &i64, _| Value::Integer(*acc),
        )
        .unwrap();
        rows(
            db.execute("(query [:find (regsum ?v) :where [?e :n ?v]])")
                .unwrap(),
        )
    });
    assert_eq!(sum, vec![vec![Value::Integer(3)]]);
}

#[test]
fn a_rule_predicate_that_registers_a_function_returns() {
    let n = within_timeout(|| {
        let db = Minigraf::in_memory().unwrap();
        db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
        db.execute("(rule [(big ?e) [?e :n ?v] [(reg? ?v)]])")
            .unwrap();
        let inner = db.clone();
        db.register_predicate("reg?", move |v| {
            if let Value::Integer(i) = v {
                inner
                    .register_predicate(&format!("r{i}?"), |_| true)
                    .expect("registers");
            }
            true
        })
        .unwrap();
        rows(db.execute("(query [:find ?e :where (big ?e)])").unwrap()).len()
    });
    assert_eq!(n, 2);
}

#[test]
fn a_cursor_keeps_the_functions_it_opened_with() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..3000).map(|i| format!("[:e{i} :n {i}]")).collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.register_predicate("any?", |_| true).unwrap();

    // Unknown at open: QRY-007, now and after a later registration.
    let q = "(query [:find ?e :where [?e :n ?v] [(late? ?v)]])";
    assert_eq!(db.query(q).err().map(|e| e.code()), Some("QRY-007"));

    let mut cursor = db
        .query("(query [:find ?e :where [?e :n ?v] [(any? ?v)]])")
        .unwrap();
    let mut n = cursor.next_batch(10).unwrap().map_or(0, |b| b.len());
    // Registering between pulls neither blocks nor changes the open cursor.
    db.register_predicate("late?", |_| true).unwrap();
    while let Some(batch) = cursor.next_batch(1000).unwrap() {
        n += batch.len();
    }
    assert_eq!(n, 3000);
    assert_eq!(
        rows(db.execute(q).unwrap()).len(),
        3000,
        "a later query sees it"
    );
}

// ─── Re-entry from inside a user function (plan step 10, spec §5 table) ──────

use minigraf::{FactFilter, MinigrafError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What a callback does on the database, once, from inside the outer query.
type Action = Box<dyn Fn(&Minigraf) -> Result<(), MinigrafError> + Send + Sync>;

/// The outer query: every `:n` fact, through the callback predicate `cb?`.
const OUTER: &str = "(query [:find ?e ?v :where [?e :n ?v] [(cb? ?v)]])";
const PLAIN: &str = "(query [:find ?e ?v :where [?e :n ?v]])";

/// What the outer query gave, what the action returned (its error code, or
/// `None`), and the number of `:n` facts after the outer query.
struct Outcome {
    rows: Vec<Vec<Value>>,
    baseline: Vec<Vec<Value>>,
    code: Option<&'static str>,
    after: usize,
}

fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort();
    rows
}

/// Runs [`OUTER`] on a file database with committed facts (`:a`, `:b`) and a
/// pending one (`:c`), as a plain read or inside a `WriteTransaction`; the
/// callback runs `make`'s action on its first call.
fn reenter(make: fn(&Minigraf) -> Action, in_tx: bool) -> Outcome {
    within_timeout(move || {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("g.graph")).unwrap();
        db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
        db.checkpoint().unwrap();
        db.execute("(transact [[:c :n 3]])").unwrap();
        let baseline = sorted(rows(db.execute(PLAIN).unwrap()));

        let action = make(&db);
        let inner = db.clone();
        let started = Arc::new(AtomicBool::new(false));
        let code = Arc::new(Mutex::new(None));
        let c = Arc::clone(&code);
        db.register_predicate("cb?", move |_| {
            if !started.swap(true, Ordering::SeqCst) {
                let r = action(&inner).err().map(|e| e.code());
                *c.lock().unwrap() = Some(r);
            }
            true
        })
        .unwrap();

        let result = if in_tx {
            let mut wtx = db.begin_write().unwrap();
            let r = wtx.execute(OUTER).unwrap();
            wtx.commit().unwrap();
            r
        } else {
            db.execute(OUTER).unwrap()
        };
        let code = code.lock().unwrap().take().expect("the callback ran");
        let after = rows(db.execute(PLAIN).unwrap()).len();
        db.verify().expect("the file is sound after the callback");
        Outcome {
            rows: sorted(rows(result)),
            baseline,
            code,
            after,
        }
    })
}

/// One row of spec §5's table: the plain-read column (the action runs, and
/// the outer query's rows are those of the database before it) and the
/// write-transaction column (`tx_code`). `after` is the number of `:n`
/// facts after the action ran in a plain read.
fn check_row(make: fn(&Minigraf) -> Action, tx_code: Option<&str>, after: usize) {
    let plain = reenter(make, false);
    assert_eq!(plain.code, None, "runs from inside a plain read");
    assert!(
        plain.rows == plain.baseline,
        "the outer query does not see the action"
    );
    assert_eq!(plain.after, after, "a later query sees the action");

    let tx = reenter(make, true);
    assert_eq!(tx.code, tx_code, "inside a write transaction");
    assert!(
        tx.rows == tx.baseline,
        "the outer query's rows inside a write transaction"
    );
    let tx_after = if tx_code.is_none() { after } else { 3 };
    assert_eq!(tx.after, tx_after);
}

const INT001: Option<&str> = Some("INT-001");

#[test]
fn reentry_execute_query() {
    check_row(
        |_| Box::new(|db| db.execute("(query [:find ?v :where [_ :n ?v]])").map(drop)),
        INT001,
        3,
    );
}

#[test]
fn reentry_query_drained() {
    check_row(
        |_| {
            Box::new(|db| {
                db.query("(query [:find ?v :where [_ :n ?v]])")?
                    .collect::<Result<Vec<_>, _>>()
                    .map(drop)
            })
        },
        INT001,
        3,
    );
}

#[test]
fn reentry_prepare_and_execute() {
    check_row(
        |_| {
            Box::new(|db| {
                db.prepare("(query [:find ?v :where [_ :n ?v]])")?
                    .execute(&[])
                    .map(drop)
            })
        },
        INT001,
        3,
    );
}

#[test]
fn reentry_prepared_execute() {
    check_row(
        |db| {
            let pq = db.prepare("(query [:find ?v :where [_ :n ?v]])").unwrap();
            Box::new(move |_| pq.execute(&[]).map(drop))
        },
        INT001,
        3,
    );
}

#[test]
fn reentry_prepared_query_drained() {
    check_row(
        |db| {
            let pq = db.prepare("(query [:find ?v :where [_ :n ?v]])").unwrap();
            Box::new(move |_| pq.query(&[])?.collect::<Result<Vec<_>, _>>().map(drop))
        },
        INT001,
        3,
    );
}

#[test]
fn reentry_fact_log() {
    check_row(
        |_| {
            Box::new(|db| {
                db.fact_log(&FactFilter::new())?
                    .collect::<Result<Vec<_>, _>>()
                    .map(drop)
            })
        },
        INT001,
        3,
    );
}

#[test]
fn reentry_transact() {
    check_row(
        |_| Box::new(|db| db.execute("(transact [[:z :n 99]])").map(drop)),
        INT001,
        4,
    );
}

#[test]
fn reentry_retract() {
    check_row(
        |_| Box::new(|db| db.execute("(retract [[:a :n 1]])").map(drop)),
        INT001,
        2,
    );
}

#[test]
fn reentry_rule() {
    check_row(
        |_| Box::new(|db| db.execute("(rule [(has-n ?e) [?e :n _]])").map(drop)),
        INT001,
        3,
    );
}

#[test]
fn reentry_begin_write_and_commit() {
    check_row(
        |_| {
            Box::new(|db| {
                let mut tx = db.begin_write()?;
                tx.execute("(transact [[:y :n 98]])")?;
                tx.commit()
            })
        },
        INT001,
        4,
    );
}

#[test]
fn reentry_checkpoint() {
    check_row(|_| Box::new(|db| db.checkpoint()), INT001, 3);
}

#[test]
fn reentry_verify() {
    check_row(|_| Box::new(|db| db.verify().map(drop)), INT001, 3);
}

#[test]
fn reentry_rebuild_indexes() {
    check_row(|_| Box::new(|db| db.rebuild_indexes()), INT001, 3);
}

#[test]
fn reentry_register_predicate() {
    check_row(
        |_| Box::new(|db| db.register_predicate("late?", |_| true)),
        None,
        3,
    );
    check_row(
        |_| Box::new(|db| db.try_register_predicate("late?", |_| Ok(true))),
        None,
        3,
    );
}

#[test]
fn reentry_register_aggregate() {
    check_row(
        |_| {
            Box::new(|db| {
                db.register_aggregate("late", || 0i64, |_, _| {}, |a, _| Value::Integer(*a))
            })
        },
        None,
        3,
    );
    check_row(
        |_| {
            Box::new(|db| {
                db.try_register_aggregate(
                    "late",
                    || Ok(0i64),
                    |_, _| Ok(()),
                    |a, _| Ok(Value::Integer(*a)),
                )
            })
        },
        None,
        3,
    );
}

#[test]
fn reentry_nested_query_calling_a_second_function() {
    check_row(
        |db| {
            let inner = db.clone();
            db.register_predicate("inner?", move |_| {
                // A third level: a query from inside the nested query's UDF.
                inner.execute("(query [:find ?v :where [_ :n ?v]])").is_ok()
            })
            .unwrap();
            Box::new(|db| {
                let n =
                    rows(db.execute("(query [:find ?v :where [_ :n ?v] [(inner? ?v)]])")?).len();
                assert_eq!(n, 3, "the nested query's own rows");
                Ok(())
            })
        },
        INT001,
        3,
    );
}
