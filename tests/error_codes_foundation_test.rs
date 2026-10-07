// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{ErrorCategory, Minigraf};

#[test]
fn in_memory_open_returns_ok() {
    let db = Minigraf::in_memory();
    assert!(db.is_ok(), "in_memory() should succeed");
}

#[test]
fn open_nonexistent_directory_returns_coded_error() {
    let bad_path = "/nonexistent-dir-for-minigraf-test-277/db.graph";
    let result = Minigraf::open(bad_path);
    // `Result::expect_err`/`unwrap_err` require the `Ok` type (`Minigraf`) to
    // implement `Debug`, which it deliberately does not (its fields include
    // non-`Debug` function-pointer registries). Extract the error manually.
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("opening a file in a nonexistent directory should fail"),
    };
    assert_eq!(err.category(), ErrorCategory::Internal);
    assert_eq!(err.code(), "INT-000");
}

#[test]
fn execute_parse_error_returns_coded_error() {
    // Migrated by the PRS category PR (#358): parser.rs errors now carry
    // their real PRS-0xx code instead of falling back to INT-000. This
    // input never finds a closing `)`, so it's PRS-005 (Unclosed list).
    let db = Minigraf::in_memory().unwrap();
    let result = db.execute("(this is not valid datalog");
    let err = result.expect_err("malformed input should fail to parse");
    assert_eq!(err.category(), ErrorCategory::Parser);
    assert_eq!(err.code(), "PRS-005");
}

#[test]
fn execute_valid_transact_returns_ok() {
    let db = Minigraf::in_memory().unwrap();
    let result =
        db.execute(r#"(transact [[#uuid "550e8400-e29b-41d4-a716-446655440000" :name "alice"]])"#);
    assert!(result.is_ok(), "valid transact should succeed");
}

#[test]
fn begin_write_then_checkpoint_returns_ok() {
    let db = Minigraf::in_memory().unwrap();
    let tx = db.begin_write();
    assert!(tx.is_ok(), "begin_write on a fresh db should succeed");
    tx.unwrap().rollback();
    let checkpoint = db.checkpoint();
    assert!(
        checkpoint.is_ok(),
        "checkpoint on an in-memory db is a no-op success"
    );
}

#[test]
fn prepare_and_register_predicate_return_ok() {
    use minigraf::Value;

    let db = Minigraf::in_memory().unwrap();
    let prepared = db.prepare("(query [:find ?e :where [?e :name $name]])");
    assert!(prepared.is_ok(), "preparing a valid query should succeed");

    let registered = db.register_predicate(
        "even277?",
        |v: &Value| matches!(v, Value::Integer(i) if i % 2 == 0),
    );
    assert!(
        registered.is_ok(),
        "registering a new predicate name should succeed"
    );
}

#[test]
fn write_transaction_execute_and_commit_return_ok() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    let exec =
        tx.execute(r#"(transact [[#uuid "550e8400-e29b-41d4-a716-446655440001" :name "bob"]])"#);
    assert!(
        exec.is_ok(),
        "staging a valid transact in a tx should succeed"
    );
    let commit = tx.commit();
    assert!(
        commit.is_ok(),
        "committing a valid transaction should succeed"
    );
}

#[test]
fn prepared_query_execute_returns_ok() {
    use minigraf::{BindValue, Value};

    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:carol :person/name "Carol"]])"#)
        .unwrap();
    let prepared = db
        .prepare("(query [:find ?e :where [?e :person/name $name]])")
        .unwrap();
    let result = prepared.execute(&[("name", BindValue::Val(Value::String("Carol".to_string())))]);
    assert!(
        result.is_ok(),
        "executing a prepared query with a valid binding should succeed"
    );
}
