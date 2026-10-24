// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]
//! A database call on a thread that holds a `WriteTransaction` returns
//! `INT-001` instead of waiting on the transaction's own write lock (#734).

use minigraf::{Minigraf, PreparedQuery};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Runs `call` on a fresh thread holding a `WriteTransaction` on a file
/// database, and returns the error code `call` gave, or `None` if it
/// succeeded. A call that deadlocks fails the test after 30 s instead of
/// hanging it. The transaction must still commit after the call.
fn code_inside_tx(
    call: fn(&Minigraf, &PreparedQuery) -> Option<&'static str>,
) -> Option<&'static str> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("g.graph")).unwrap();
        db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();
        let prepared = db
            .prepare("(query [:find ?n :where [?e :person/name ?n]])")
            .unwrap();
        let mut wtx = db.begin_write().unwrap();
        wtx.execute(r#"(transact [[:bob :person/name "Bob"]])"#)
            .unwrap();
        let code = call(&db, &prepared);
        wtx.commit()
            .expect("the transaction commits after the refusal");
        tx.send(code).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(30))
        .expect("the call returned instead of deadlocking")
}

#[test]
fn checkpoint_inside_write_tx_is_int001() {
    let code = code_inside_tx(|db, _| db.checkpoint().err().map(|e| e.code()));
    assert_eq!(code, Some("INT-001"));
}

#[test]
fn verify_inside_write_tx_is_int001() {
    let code = code_inside_tx(|db, _| db.verify().err().map(|e| e.code()));
    assert_eq!(code, Some("INT-001"));
}

#[test]
fn rebuild_indexes_inside_write_tx_is_int001() {
    let code = code_inside_tx(|db, _| db.rebuild_indexes().err().map(|e| e.code()));
    assert_eq!(code, Some("INT-001"));
}

#[test]
fn prepare_inside_write_tx_is_int001() {
    let code = code_inside_tx(|db, _| {
        db.prepare("(query [:find ?n :where [?e :person/name ?n]])")
            .err()
            .map(|e| e.code())
    });
    assert_eq!(code, Some("INT-001"));
}

#[test]
fn prepared_execute_inside_write_tx_is_int001() {
    let code = code_inside_tx(|_, p| p.execute(&[]).err().map(|e| e.code()));
    assert_eq!(code, Some("INT-001"));
}

#[test]
fn prepared_query_inside_write_tx_is_int001() {
    let code = code_inside_tx(|_, p| p.query(&[]).err().map(|e| e.code()));
    assert_eq!(code, Some("INT-001"));
}

/// Outside a transaction the same calls still run.
#[test]
fn guarded_calls_run_outside_a_write_tx() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("g.graph")).unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    db.verify().unwrap();
    db.rebuild_indexes().unwrap();
    let prepared = db
        .prepare("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    prepared.execute(&[]).unwrap();
    assert_eq!(prepared.query(&[]).unwrap().count(), 1, "one row");
}
