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
