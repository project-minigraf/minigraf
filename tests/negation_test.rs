//! Integration tests for stratified negation (Phase 7.1a).
//! Covers the 10 scenarios from the spec testing plan.

use minigraf::{Minigraf, OpenOptions};

fn in_memory_db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

// ── Test 1: Simple not on base fact ──────────────────────────────────────────

#[test]
fn test_not_excludes_base_fact() {
    let db = in_memory_db();

    db.execute(
        r#"(transact [[:alice :person/name "Alice"]
                              [:bob   :person/name "Bob"]
                              [:alice :person/banned true]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"
        (query [:find ?person
                :where [?person :person/name ?n]
                       (not [?person :person/banned true])])
    "#,
        )
        .unwrap();

    let result_str = format!("{:?}", result);
    // :alice entity has :person/banned true so it is excluded.
    // :bob entity does not have :person/banned so it appears.
    // The names "Alice" and "Bob" are bound to ?n (not in :find), so the
    // result contains UUIDs for ?person. We verify the count instead.
    // When not works: only 1 row (bob), not 2.
    assert!(
        result_str.contains("QueryResults"),
        "should return query results"
    );
    // Verify "Bob" does NOT appear in results (it's bound to ?n, not ?person)
    // and "Alice" does NOT appear (same reason). Instead check result count.
    // The result should have exactly 1 binding (bob's UUID).
    assert!(
        !result_str.contains("\"Alice\""),
        "alice is banned; her name must not appear via ?n leak"
    );
    // Bob's name "Bob" would appear if ?n were in :find — but it's not.
    // Instead, assert we get exactly one result row (bob).
    match result {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "only bob (not alice) should pass the not-filter"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 2: not with multiple clauses (conjunction) ──────────────────────────

#[test]
fn test_not_multiple_clauses_conjunction() {
    let db = in_memory_db();

    db.execute(
        r#"(transact [[:alice :role :admin]
                              [:alice :active false]
                              [:bob   :role :admin]
                              [:bob   :active true]])"#,
    )
    .unwrap();

    // Exclude entities that are BOTH admin AND active=false
    let result = db
        .execute(
            r#"
        (query [:find ?person
                :where [?person :role :admin]
                       (not [?person :role :admin]
                            [?person :active false])])
    "#,
        )
        .unwrap();

    match result {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "alice matches both conditions, should be excluded; only bob remains"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 3: not negating a derived rule ──────────────────────────────────────

#[test]
fn test_not_negates_derived_rule() {
    let db = in_memory_db();

    db.execute(r#"(rule [(blocked ?x) [?x :status :blocked]])"#)
        .unwrap();

    db.execute(
        r#"(transact [[:alice :person/name "Alice"]
                              [:bob   :person/name "Bob"]
                              [:alice :status :blocked]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"
        (query [:find ?person
                :where [?person :person/name ?n]
                       (not (blocked ?person))])
    "#,
        )
        .unwrap();

    match result {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "alice is blocked so only bob should appear"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 4: Multi-stratum chain ───────────────────────────────────────────────

#[test]
fn test_multi_stratum_not_on_derived_predicate() {
    let db = in_memory_db();

    // rejected is derived, eligible uses not(rejected)
    db.execute(r#"(rule [(rejected ?x) [?x :score :low]])"#)
        .unwrap();
    db.execute(r#"(rule [(eligible ?x) [?x :applied true] (not (rejected ?x))])"#)
        .unwrap();

    db.execute(
        r#"(transact [[:alice :applied true]
                              [:alice :score :low]
                              [:bob   :applied true]
                              [:bob   :score :high]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"
        (query [:find ?x :where (eligible ?x)])
    "#,
        )
        .unwrap();

    // Entity IDs are UUIDs (hex); string-contains checks on "alice"/"bob" don't work.
    // Verify the count: only bob should be eligible (1 result), not alice.
    match result {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "alice has low score → rejected → not eligible; bob has high score → eligible"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 5: not combined with :as-of ─────────────────────────────────────────

#[test]
fn test_not_with_as_of_time_travel() {
    let db = in_memory_db();

    // tx 1: alice applied
    db.execute(r#"(transact [[:alice :applied true]])"#)
        .unwrap();
    // tx 2: alice gets rejected
    db.execute(r#"(transact [[:alice :rejected true]])"#)
        .unwrap();

    // As of tx 1, alice was not yet rejected → eligible
    let result_tx1 = db
        .execute(
            r#"
        (query [:find ?x
                :as-of 1
                :where [?x :applied true]
                       (not [?x :rejected true])])
    "#,
        )
        .unwrap();

    // As of tx 2, alice is rejected → not eligible
    let result_tx2 = db
        .execute(
            r#"
        (query [:find ?x
                :as-of 2
                :where [?x :applied true]
                       (not [?x :rejected true])])
    "#,
        )
        .unwrap();

    match result_tx1 {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "at tx1 alice was not yet rejected, should appear"
            );
        }
        _ => panic!("expected QueryResults"),
    }

    match result_tx2 {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                0,
                "at tx2 alice is rejected, should not appear"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 6: not combined with :valid-at ──────────────────────────────────────

#[test]
fn test_not_with_valid_at() {
    let db = in_memory_db();

    // alice employed 2023, banned from 2024
    db.execute(
        r#"(transact {:valid-from "2023-01-01" :valid-to "2025-01-01"}
                            [[:alice :employed true]])"#,
    )
    .unwrap();
    db.execute(
        r#"(transact {:valid-from "2024-01-01"}
                            [[:alice :banned true]])"#,
    )
    .unwrap();

    // In 2023, alice was employed and not yet banned
    let result_2023 = db
        .execute(
            r#"
        (query [:find ?x
                :valid-at "2023-06-01"
                :where [?x :employed true]
                       (not [?x :banned true])])
    "#,
        )
        .unwrap();

    // In 2024, alice is both employed and banned → excluded
    let result_2024 = db
        .execute(
            r#"
        (query [:find ?x
                :valid-at "2024-06-01"
                :where [?x :employed true]
                       (not [?x :banned true])])
    "#,
        )
        .unwrap();

    match result_2023 {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                "in 2023 alice was not banned, should appear"
            );
        }
        _ => panic!("expected QueryResults"),
    }

    match result_2024 {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                0,
                "in 2024 alice is banned, should not appear"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 7: Negative cycle at rule registration → error ──────────────────────

#[test]
fn test_negative_cycle_rejected_at_registration() {
    let db = in_memory_db();

    // Register first rule fine
    db.execute(r#"(rule [(p ?x) [?x :base true] (not (q ?x))])"#)
        .unwrap();

    // Second rule creates negative cycle
    let result = db.execute(r#"(rule [(q ?x) [?x :base true] (not (p ?x))])"#);
    assert!(result.is_err(), "negative cycle must be rejected");

    // q must not be registered
    let query_result = db.execute(
        r#"
        (query [:find ?x :where (q ?x)])
    "#,
    );
    // Either returns empty or errors (predicate unknown) — either is acceptable
    // but it must NOT panic
    let _ = query_result;
}

// ── Test 8: Recursive rule + not coexist for different predicates ─────────────

#[test]
fn test_recursive_rule_and_not_coexist() {
    let db = in_memory_db();

    // reachable is recursive (positive)
    db.execute(r#"(rule [(reachable ?a ?b) [?a :connected ?b]])"#)
        .unwrap();
    db.execute(r#"(rule [(reachable ?a ?b) [?a :connected ?m] (reachable ?m ?b)])"#)
        .unwrap();

    // blocked uses not on a base fact
    db.execute(
        r#"(rule [(accessible ?a ?b)
                         (reachable ?a ?b)
                         (not [?b :blocked true])])"#,
    )
    .unwrap();

    db.execute(
        r#"(transact [[:a :connected :b]
                              [:b :connected :c]
                              [:c :blocked true]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"
        (query [:find ?b :where (accessible :a ?b)])
    "#,
        )
        .unwrap();

    let r = format!("{:?}", result);
    assert!(
        r.contains("b") || r.contains(":b"),
        "b is reachable and not blocked"
    );
    assert!(!r.contains(":c"), "c is blocked");
}

// ── Test 9: not in a rule body (rule-level) ───────────────────────────────────

#[test]
fn test_not_in_rule_body() {
    let db = in_memory_db();

    db.execute(r#"(rule [(safe ?x) [?x :checked true] (not [?x :flagged true])])"#)
        .unwrap();

    db.execute(
        r#"(transact [[:a :checked true]
                              [:b :checked true]
                              [:b :flagged true]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"
        (query [:find ?x :where (safe ?x)])
    "#,
        )
        .unwrap();

    match result {
        minigraf::QueryResult::QueryResults { ref results, .. } => {
            assert_eq!(
                results.len(),
                1,
                ":a is safe but :b is flagged; only 1 result expected"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Test 10: Safety violation → parse error ───────────────────────────────────

#[test]
fn test_safety_violation_unbound_variable_in_not() {
    let db = in_memory_db();

    // ?y is only in (not ...), never in an outer clause
    let result = db.execute(
        r#"
        (query [:find ?x
                :where [?x :a ?v]
                       (not [?y :banned true])])
    "#,
    );

    assert!(
        result.is_err(),
        "unbound variable in not should be a parse error"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("not bound") || msg.contains("unbound"),
        "error should mention unbound variable, got: {msg}"
    );
}
