//! Integration tests for retraction semantics in Datalog queries.
//!
//! These tests verify that `filter_facts_for_query` computes the net-asserted
//! view per (entity, attribute, value) triple, correctly hiding retracted facts.

use minigraf::Minigraf;

// ── Test 1: Basic retraction ──────────────────────────────────────────────────

#[test]
fn test_retraction_hides_fact_from_query() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap();
    db.execute("(retract [[:alice :age 30]])").unwrap();
    let result = db
        .execute("(query [:find ?v :where [:alice :age ?v]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("[]") || !format!("{:?}", result).contains("30"),
        "retracted fact must not appear in query results"
    );
}

// ── Test 2: as-of before and after retraction ─────────────────────────────────

#[test]
fn test_retraction_as_of_before_shows_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap(); // tx_count = 1
    db.execute("(transact [[:alice :age 31]])").unwrap(); // tx_count = 2
    db.execute("(retract [[:alice :age 30]])").unwrap(); // tx_count = 3

    // as-of 2: retraction not yet in window — fact 30 must appear
    let result = db
        .execute("(query [:find ?v :as-of 2 :where [:alice :age ?v]])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(
        s.contains("30"),
        "fact 30 must appear when as-of precedes retraction"
    );
}

#[test]
fn test_retraction_as_of_after_hides_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap(); // tx_count = 1
    db.execute("(transact [[:alice :age 31]])").unwrap(); // tx_count = 2
    db.execute("(retract [[:alice :age 30]])").unwrap(); // tx_count = 3

    // as-of 3: retraction is in window — fact 30 must not appear
    let result = db
        .execute("(query [:find ?v :as-of 3 :where [:alice :age ?v]])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(
        !s.contains("30"),
        "fact 30 must not appear when as-of includes retraction"
    );
    assert!(
        s.contains("31"),
        "fact 31 must still appear (it was not retracted)"
    );
}

// ── Test 3: Assert → retract → re-assert ─────────────────────────────────────

#[test]
fn test_retraction_then_reassert() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :status :active]])").unwrap();
    db.execute("(retract [[:alice :status :active]])").unwrap();
    db.execute("(transact [[:alice :status :active]])").unwrap();

    let result = db
        .execute("(query [:find ?s :where [:alice :status ?s]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("active"),
        "re-asserted fact must appear after retract+reassert"
    );
}

// ── Test 4: Retraction + :any-valid-time ─────────────────────────────────────

#[test]
fn test_retraction_with_any_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact {:valid-from "2023-01-01"} [[:alice :role :engineer]])"#)
        .unwrap();
    db.execute("(retract [[:alice :role :engineer]])").unwrap();

    let result = db
        .execute("(query [:find ?r :any-valid-time :where [:alice :role ?r]])")
        .unwrap();
    assert!(
        !format!("{:?}", result).contains("engineer"),
        "retracted fact must not appear even with :any-valid-time"
    );
}

// ── Test 5: Retraction + recursive rule — as-of before retraction ─────────────

#[test]
fn test_retraction_rule_as_of_before_sees_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c]])")
        .unwrap(); // tx_count = 1
    db.execute("(retract [[:a :next :b]])").unwrap(); // tx_count = 2
    db.execute("(rule [(reach ?x ?y) [?x :next ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)])")
        .unwrap();

    let result = db
        .execute("(query [:find ?to :as-of 1 :where (reach :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("b"), "b must be reachable from a at as-of 1");
    assert!(
        s.contains("c"),
        "c must be reachable from a at as-of 1 (via b)"
    );
}

// ── Test 6: Retraction + recursive rule — as-of after retraction ──────────────

#[test]
fn test_retraction_rule_as_of_after_breaks_chain() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c]])")
        .unwrap(); // tx_count = 1
    db.execute("(retract [[:a :next :b]])").unwrap(); // tx_count = 2
    db.execute("(rule [(reach ?x ?y) [?x :next ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)])")
        .unwrap();

    // as-of 2: retraction is in window — :a→:b link is broken
    let result = db
        .execute("(query [:find ?to :as-of 2 :where (reach :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(
        !s.contains("\"b\"") && !s.contains(":b"),
        "b must not be reachable from a after retraction of :a→:b"
    );
    assert!(
        !s.contains("\"c\"") && !s.contains(":c"),
        "c must not be reachable from a: chain is broken at :a→:b"
    );
}
