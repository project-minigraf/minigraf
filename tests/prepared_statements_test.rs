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

// ─── Bind slots through execute() (#407) ──────────────────────────────────────

#[test]
fn execute_rejects_bind_slots_with_api_error() {
    let db = Minigraf::in_memory().unwrap();
    let err = db
        .execute(
            "(query [:find ?status :as-of $tx :valid-at $date \
             :where [$entity :employment/status ?status]])",
        )
        .unwrap_err();
    assert_eq!(err.code(), "API-010");
    let msg = err.to_string();
    assert!(msg.contains("$date, $entity, $tx"), "slots listed sorted");
    assert!(msg.contains("prepare"), "error should point at prepare()");
}

#[test]
fn execute_rejects_bind_slot_in_every_position() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/age 30]])"#)
        .unwrap();
    for q in [
        "(query [:find ?e :as-of $tx :where [?e :person/age _]])",
        "(query [:find ?e :valid-at $d :where [?e :person/age _]])",
        "(query [:find ?a :where [$e :person/age ?a]])",
        "(query [:find ?e :where [?e :person/age $age]])",
        "(query [:find ?v :where [?e $attr ?v]])",
        "(query [:find ?e :where [?e :person/age ?a] [(> ?a $min)]])",
        "(query [:find ?e :where [?e :person/age _] (not [?e :person/name $n])])",
        "(query [:find ?e :where (or [?e :person/age $a] [?e :person/name $n])])",
    ] {
        let err = db.execute(q).unwrap_err();
        assert_eq!(
            err.code(),
            "API-010",
            "query with a bind slot must be API-010"
        );
    }
}

#[test]
fn write_transaction_execute_rejects_bind_slots() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:alice :person/age 30]])"#)
        .unwrap();
    let err = tx
        .execute("(query [:find ?e :where [?e :person/age $age]])")
        .unwrap_err();
    assert_eq!(err.code(), "API-010");
    tx.rollback();
}

// ─── Literal time travel in a prepared query (#603) ──────────────────────────

/// Rows of `q` through `execute`, a prepared `execute` and a prepared cursor,
/// none with bind values; all three must agree.
fn three_ways(db: &Minigraf, q: &str) -> usize {
    let direct = match db.execute(q).unwrap() {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    };
    let prepared = db.prepare(q).unwrap();
    let executed = match prepared.execute(&[]).unwrap() {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    };
    let mut cursor = prepared.query(&[]).unwrap();
    let mut pulled = 0;
    while let Some(batch) = cursor.next_batch(100).unwrap() {
        pulled += batch.rows().len();
    }
    assert_eq!(executed, direct, "prepared execute agrees with execute");
    assert_eq!(pulled, direct, "prepared cursor agrees with execute");
    direct
}

#[test]
fn prepared_literal_as_of_and_valid_at_are_kept() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact {:valid-from "2020-01-01" :valid-to "2021-01-01"} [[:a :n 1]])"#)
        .unwrap();
    db.execute("(transact [[:a :n 2]])").unwrap();
    assert_eq!(
        three_ways(
            &db,
            "(query [:find ?v :as-of 1 :valid-at :any-valid-time :where [:a :n ?v]])"
        ),
        1
    );
    assert_eq!(
        three_ways(
            &db,
            r#"(query [:find ?v :as-of "2000-01-01T00:00:00Z" :any-valid-time :where [:a :n ?v]])"#
        ),
        0
    );
    assert_eq!(
        three_ways(
            &db,
            r#"(query [:find ?v :valid-at "2020-06-01T00:00:00Z" :where [:a :n ?v]])"#
        ),
        1
    );
    assert_eq!(
        three_ways(&db, "(query [:find ?v :any-valid-time :where [:a :n ?v]])"),
        2
    );
    assert_eq!(three_ways(&db, "(query [:find ?v :where [:a :n ?v]])"), 1);
    // A per-fact pseudo-attribute needs :any-valid-time; a prepared query
    // keeps it (INT-026 before the fix).
    assert_eq!(
        three_ways(
            &db,
            "(query [:find ?v ?t :any-valid-time :where [:a :n ?v] [:a :db/tx-count ?t]])"
        ),
        2
    );
}

// ─── Bind mistakes are API errors (#181) ─────────────────────────────────────

/// The error of `execute` and of `query` with `binds`, which must be the same code.
fn bind_error(
    pq: &minigraf::PreparedQuery,
    binds: &[(&str, BindValue)],
) -> minigraf::MinigrafError {
    let exec = pq
        .execute(binds)
        .expect_err("execute must reject the bindings");
    let cursor = pq.query(binds).expect_err("query must reject the bindings");
    assert_eq!(exec.code(), cursor.code(), "execute and query disagree");
    exec
}

#[test]
fn missing_bind_value_is_api_022_in_every_position() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :n 1] [:a :m 2]])").unwrap();
    db.execute("(rule [(has-n ?e ?v) [?e :n ?v]])").unwrap();
    for q in [
        "(query [:find ?v :where [$e :n ?v]])",
        "(query [:find ?e :where [?e :n $v]])",
        "(query [:find ?v :where [?e :n ?v] [(> ?v $t)]])",
        "(query [:find ?e :where (has-n ?e $v)])",
        "(query [:find ?v :as-of $tx :where [?e :n ?v]])",
        "(query [:find ?v :valid-at $d :where [?e :n ?v]])",
        "(query [:find ?e :where [?e :n ?v] (not [?e :m $v])])",
        "(query [:find ?e :where [?e :n ?v] (not-join [?e] [?e :m $v])])",
        "(query [:find ?e :where (or [?e :n $v] [?e :m $v])])",
        "(query [:find ?e :where [?e :n ?w] (or-join [?e] [?e :n $v] [?e :m $v])])",
    ] {
        let pq = db.prepare(q).expect("prepare");
        let err = bind_error(&pq, &[]);
        assert_eq!(err.code(), "API-022", "missing bind value");
        assert_eq!(err.category(), minigraf::ErrorCategory::Api);
    }
}

#[test]
fn attribute_slot_is_api_023_in_every_clause() {
    let db = Minigraf::in_memory().unwrap();
    for q in [
        "(query [:find ?v :where [?e $a ?v]])",
        "(query [:find ?e :where [?e :n ?v] (not [?e $a ?v])])",
        "(query [:find ?e :where [?e :n ?v] (not-join [?e] [?e $a 1])])",
        "(query [:find ?e :where (or [?e $a 1] [?e :m 1])])",
        "(query [:find ?e :where [?e :n ?w] (or-join [?e] [?e :n 1] [?e $a 1])])",
    ] {
        let err = db.prepare(q).expect_err("attribute slot must be rejected");
        assert_eq!(err.code(), "API-023", "attribute slot");
        assert_eq!(err.category(), minigraf::ErrorCategory::Api);
    }
}

#[test]
fn bind_value_of_the_wrong_kind_is_api_024_for_every_slot_kind() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :n 1]])").unwrap();
    for (q, slot, bind, position) in [
        (
            "(query [:find ?v :where [$e :n ?v]])",
            "e",
            BindValue::TxCount(1),
            "entity",
        ),
        (
            "(query [:find ?e :where [?e :n $v]])",
            "v",
            BindValue::Timestamp(1),
            "value",
        ),
        (
            "(query [:find ?v :where [?e :n ?v] [(> ?v $t)]])",
            "t",
            BindValue::AnyValidTime,
            "expression",
        ),
        (
            "(query [:find ?v :as-of $tx :where [?e :n ?v]])",
            "tx",
            BindValue::Val(Value::Integer(1)),
            ":as-of",
        ),
        (
            "(query [:find ?v :valid-at $d :where [?e :n ?v]])",
            "d",
            BindValue::TxCount(1),
            ":valid-at",
        ),
    ] {
        let pq = db.prepare(q).expect("prepare");
        let err = bind_error(&pq, &[(slot, bind)]);
        assert_eq!(err.code(), "API-024", "wrong kind of bind value");
        assert_eq!(err.category(), minigraf::ErrorCategory::Api);
        let text = err.to_string();
        assert!(
            text.contains(&format!("'${slot}'")),
            "message names the slot"
        );
        assert!(
            text.contains(&format!("{position} position")),
            "message names the position"
        );
    }
}

// ─── Entity and Val(Ref) are interchangeable (#181) ──────────────────────────

fn rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { mut results, .. } => {
            results.sort();
            results
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn entity_and_ref_bind_values_fit_each_others_slots() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":alice");
    let bob = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":bob");
    // Friends are stored as refs, so the expression `=` (which compares
    // values, unlike a pattern) sees a `Ref` on both sides.
    db.execute(&format!(
        r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]
                      [:alice :person/friend #uuid "{bob}"] [:carol :person/friend #uuid "{alice}"]])"#
    ))
    .unwrap();
    db.execute("(rule [(friend-of ?x ?f) [?x :person/friend ?f]])")
        .unwrap();

    for (q, u) in [
        (
            "(query [:find ?name :where [$f :person/name ?name]])",
            alice,
        ),
        ("(query [:find ?x :where [?x :person/friend $f]])", bob),
        ("(query [:find ?x :where (friend-of ?x $f)])", bob),
        (
            "(query [:find ?x :where [?x :person/friend ?g] [(= ?g $f)]])",
            bob,
        ),
    ] {
        let pq = db.prepare(q).expect("prepare");
        let as_entity = rows(pq.execute(&[("f", BindValue::Entity(u))]).unwrap());
        let as_ref = rows(pq.execute(&[("f", BindValue::Val(Value::Ref(u)))]).unwrap());
        assert_eq!(as_entity.len(), 1, "one row");
        assert_eq!(as_entity, as_ref, "Entity and Val(Ref) give the same rows");
    }

    let pq = db
        .prepare("(query [:find ?name :where [$f :person/name ?name]])")
        .unwrap();
    let err = bind_error(&pq, &[("f", BindValue::Val(Value::Integer(1)))]);
    assert_eq!(err.code(), "API-024", "an integer is not an entity");
}
