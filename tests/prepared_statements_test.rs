use minigraf::db::Minigraf;
use minigraf::{BindValue, QueryResult, Value};
use uuid::Uuid;

// ─── Happy-path tests ─────────────────────────────────────────────────────────

#[test]
fn prepare_and_execute_entity_slot() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :person/name "Alice"]
                      [#uuid "{bob}"  :person/name "Bob"]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    let r1 = prepared
        .execute(&[("entity", BindValue::Entity(alice))])
        .unwrap();
    match r1 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }

    let r2 = prepared
        .execute(&[("entity", BindValue::Entity(bob))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Bob".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_value_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?e :where [?e :person/name $name]])")
        .unwrap();

    let alice_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":alice");
    let bob_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":bob");

    let r = prepared
        .execute(&[("name", BindValue::Val(Value::String("Alice".to_string())))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::Ref(alice_id));
        }
        _ => panic!("expected QueryResults"),
    }

    let r2 = prepared
        .execute(&[("name", BindValue::Val(Value::String("Bob".to_string())))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::Ref(bob_id));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_as_of_counter() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice-v2"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :as-of $tx :where [?e :person/name ?name]])")
        .unwrap();

    // At tx 1 only "Alice" exists
    let r = prepared.execute(&[("tx", BindValue::TxCount(1))]).unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_as_of_timestamp() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    // Use a very large timestamp — all facts should be visible
    let prepared = db
        .prepare("(query [:find ?name :as-of $ts :where [?e :person/name ?name]])")
        .unwrap();

    let r = prepared
        .execute(&[("ts", BindValue::Timestamp(i64::MAX))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert!(!results.is_empty());
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_valid_at() {
    let db = Minigraf::in_memory().unwrap();
    // t1 = 2001-09-09T01:46:40Z, t2 = 2033-05-18T03:33:20Z, t3 (ISO) = 2065-01-24T05:20:00Z
    let t1: i64 = 1_000_000_000_000;
    let t2: i64 = 2_000_000_000_000;

    db.execute(
        "(transact {:valid-from \"2001-09-09T01:46:40Z\" :valid-to \"2065-01-24T05:20:00Z\"} \
                   [[:alice :employment/status :active]])",
    )
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?s :valid-at $date :where [?e :employment/status ?s]])")
        .unwrap();

    // Inside the valid window
    let r = prepared
        .execute(&[("date", BindValue::Timestamp(t2))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }

    // Before the valid window
    let r2 = prepared
        .execute(&[("date", BindValue::Timestamp(t1 - 1))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 0);
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_valid_at_any() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    // The slot $va is resolved to AnyValidTime at execute() time, bypassing valid-time filtering.
    let prepared = db
        .prepare("(query [:find ?name :valid-at $va :where [?e :person/name ?name]])")
        .unwrap();

    let r = prepared
        .execute(&[("va", BindValue::AnyValidTime)])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_expr_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:a :score 10] [:b :score 50] [:c :score 90]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:any-valid-time :find ?e :where [?e :score ?v] [(>= ?v $threshold)]])")
        .unwrap();

    let r = prepared
        .execute(&[("threshold", BindValue::Val(Value::Integer(50)))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 2); // :b (50) and :c (90)
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_combined() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :employment/status :active]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare(
            "(query [:find ?s \
                     :as-of $tx \
                     :valid-at $date \
                     :where [$entity :employment/status ?s] \
                            [(= ?s $expected-status)]])",
        )
        .unwrap();

    let r = prepared
        .execute(&[
            ("tx", BindValue::TxCount(100)),
            ("date", BindValue::AnyValidTime),
            ("entity", BindValue::Entity(alice)),
            (
                "expected-status",
                BindValue::Val(Value::Keyword(":active".to_string())),
            ),
        ])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn plan_reused_across_executions() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let carol = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :person/name "Alice"]
                      [#uuid "{bob}"   :person/name "Bob"]
                      [#uuid "{carol}" :person/name "Carol"]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    for (uuid, expected) in [(alice, "Alice"), (bob, "Bob"), (carol, "Carol")] {
        let r = prepared
            .execute(&[("entity", BindValue::Entity(uuid))])
            .unwrap();
        match r {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String(expected.to_string()));
            }
            _ => panic!("expected QueryResults"),
        }
    }
}

// ─── Error tests ──────────────────────────────────────────────────────────────

#[test]
fn prepare_rejects_attribute_slot() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare("(query [:find ?v :where [?e $attr ?v]])");
    assert!(result.is_err(), "expected error for attribute slot");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("attribute position"),
        "error should mention attribute position"
    );
}

#[test]
fn prepare_rejects_transact() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare(r#"(transact [[:alice :person/name "Alice"]])"#);
    assert!(result.is_err(), "expected error for transact");
    assert!(result.unwrap_err().to_string().contains("transact"));
}

#[test]
fn prepare_rejects_retract() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare(r#"(retract [[:alice :person/name "Alice"]])"#);
    assert!(result.is_err(), "expected error for retract");
    assert!(result.unwrap_err().to_string().contains("retract"));
}

#[test]
fn prepare_rejects_rule() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare("(rule [(reachable ?a ?b) [?a :edge ?b]])");
    assert!(result.is_err(), "expected error for rule");
    assert!(result.unwrap_err().to_string().contains("rule"));
}

#[test]
fn execute_missing_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    // Intentionally omit the "entity" binding
    let result = prepared.execute(&[]);
    assert!(result.is_err(), "expected error for missing slot");
    assert!(
        result.unwrap_err().to_string().contains("entity"),
        "error should mention the missing slot name"
    );
}

#[test]
fn execute_type_mismatch_as_of() {
    let db = Minigraf::in_memory().unwrap();
    let prepared = db
        .prepare("(query [:find ?v :as-of $tx :where [?e :score ?v]])")
        .unwrap();

    let result = prepared.execute(&[("tx", BindValue::Val(Value::Integer(42)))]);
    assert!(result.is_err(), "expected type mismatch error");
    assert!(
        result.unwrap_err().to_string().contains(":as-of position"),
        "error should mention :as-of position"
    );
}

#[test]
fn execute_type_mismatch_entity() {
    let db = Minigraf::in_memory().unwrap();
    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    let result = prepared.execute(&[(
        "entity",
        BindValue::Val(Value::String("not-a-uuid".to_string())),
    )]);
    assert!(result.is_err(), "expected type mismatch error");
    assert!(
        result.unwrap_err().to_string().contains("entity position"),
        "error should mention entity position"
    );
}

#[test]
fn execute_extra_bindings_ignored() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [:alice :person/name ?name]])")
        .unwrap();

    // Provide an extra binding that the query doesn't reference
    let result = prepared.execute(&[("unused-slot", BindValue::Val(Value::Integer(99)))]);
    assert!(result.is_ok(), "extra bindings should be silently ignored");
}
