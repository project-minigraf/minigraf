#![cfg(test)]
//! XTDB compatibility tests (#221).
//!
//! License: XTDB is Apache 2.0. These tests are semantic ports of query
//! concepts from the XTDB documentation and test suite. Each test is
//! annotated with its XTDB concept source.
//!
//! Skipped cases are listed at the bottom of this file.
//!
//! Run: cargo test --test xtdb_compat_test

#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult, Value};

fn count_results(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

fn query_strings(db: &Minigraf, q: &str) -> Vec<String> {
    match db.execute(q).unwrap() {
        QueryResult::QueryResults { results, .. } => results
            .into_iter()
            .flatten()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

// ── Basic EAV queries ─────────────────────────────────────────────────────────

/// XTDB concept: find all entities with a specific attribute value.
/// Source: XTDB "Basic Queries" documentation.
#[test]
fn xtdb_basic_find_by_attribute_value() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:pablo    :profession "painter"]
        [:salvador :profession "painter"]
        [:kafka    :profession "writer"]
    ])"#,
    )
    .unwrap();

    let painters = count_results(
        db.execute(r#"(query [:find ?e :where [?e :profession "painter"]])"#)
            .unwrap(),
    );
    assert_eq!(painters, 2, "should find 2 painters");
}

/// XTDB concept: multi-attribute join (entities satisfying multiple conditions).
/// Source: XTDB "Joins" documentation.
#[test]
fn xtdb_multi_attribute_join() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:e1 :role "admin"] [:e1 :active true]
        [:e2 :role "user"]  [:e2 :active true]
        [:e3 :role "admin"] [:e3 :active false]
    ])"#,
    )
    .unwrap();

    let active_admins = count_results(
        db.execute(r#"(query [:find ?e :where [?e :role "admin"] [?e :active true]])"#)
            .unwrap(),
    );
    assert_eq!(active_admins, 1, "only e1 is an active admin");
}

/// XTDB concept: find entities that share the same attribute value. The
/// shared value here is a bare keyword (`:dept-eng`), not an entity
/// reference (`Value::Ref(Uuid)`) — the join is on value equality, not on
/// traversing a reference to another entity.
/// Source: XTDB "Joins" — matching on shared attribute values.
#[test]
fn xtdb_entity_reference_join() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:alice :dept :dept-eng]
        [:bob   :dept :dept-eng]
        [:carol :dept :dept-hr]
    ])"#,
    )
    .unwrap();

    let eng_employees = count_results(
        db.execute(r#"(query [:find ?emp :where [?emp :dept :dept-eng]])"#)
            .unwrap(),
    );
    assert_eq!(eng_employees, 2, "alice and bob are in engineering");
}

// ── Retraction ───────────────────────────────────────────────────────────────

/// XTDB concept: retraction removes a specific fact, not the whole entity.
/// Source: XTDB "Transactions" — retract.
#[test]
fn xtdb_retraction_removes_specific_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :name "Alice"] [:alice :role "admin"]])"#)
        .unwrap();

    // Retract only the :role fact.
    db.execute(r#"(retract [[:alice :role "admin"]])"#).unwrap();

    let roles = count_results(
        db.execute(r#"(query [:find ?r :where [?e :role ?r]])"#)
            .unwrap(),
    );
    assert_eq!(roles, 0, "role should be retracted");

    let names = count_results(
        db.execute(r#"(query [:find ?n :where [?e :name ?n]])"#)
            .unwrap(),
    );
    assert_eq!(names, 1, "name should survive retraction of role");
}

/// XTDB concept: retracted fact is not visible after retraction.
#[test]
fn xtdb_retracted_fact_not_visible() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:item :status "active"]])"#)
        .unwrap();
    db.execute(r#"(retract [[:item :status "active"]])"#)
        .unwrap();

    let n = count_results(
        db.execute(r#"(query [:find ?s :where [?item :status ?s]])"#)
            .unwrap(),
    );
    assert_eq!(n, 0, "retracted fact must not be visible");
}

// ── Temporal queries ──────────────────────────────────────────────────────────

/// XTDB concept: as-of query returns state at a past transaction count.
/// Source: XTDB "Bitemporality" — transaction-time queries.
#[test]
fn xtdb_as_of_returns_past_state() {
    let db = Minigraf::in_memory().unwrap();

    // tx 1: alice has role "user"
    db.execute(r#"(transact [[:alice :role "user"]])"#).unwrap();

    // tx 2 + 3: alice's role changes to "admin"
    db.execute(r#"(retract [[:alice :role "user"]])"#).unwrap();
    db.execute(r#"(transact [[:alice :role "admin"]])"#)
        .unwrap();

    // Current state: admin.
    let current = query_strings(&db, r#"(query [:find ?r :where [?e :role ?r]])"#);
    assert!(
        current.contains(&"admin".to_string()),
        "current role should be admin"
    );

    // As-of tx 1: user.
    let past = query_strings(
        &db,
        r#"(query [:find ?r :as-of 1 :valid-at :any-valid-time :where [?e :role ?r]])"#,
    );
    assert!(
        past.contains(&"user".to_string()),
        "past role at tx 1 should be user"
    );
}

/// XTDB concept: valid-time query returns facts valid at a specific time.
/// Source: XTDB "Bitemporality" — valid-time queries.
#[test]
fn xtdb_valid_at_query() {
    let db = Minigraf::in_memory().unwrap();

    // Assert a fact with valid-time range.
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2023-12-31"} [[:contract :status "active"]])"#)
        .unwrap();

    // Query at valid-time within the range — should find the fact.
    let n = count_results(
        db.execute(r#"(query [:find ?s :valid-at "2023-06-01" :where [?e :status ?s]])"#)
            .unwrap(),
    );
    assert_eq!(n, 1, "fact should be visible within valid-time range");

    // Query at valid-time outside the range — should not find the fact.
    let n_after = count_results(
        db.execute(r#"(query [:find ?s :valid-at "2024-01-01" :where [?e :status ?s]])"#)
            .unwrap(),
    );
    assert_eq!(
        n_after, 0,
        "fact should not be visible after valid-to boundary"
    );
}

// ── Negation ─────────────────────────────────────────────────────────────────

/// XTDB concept: not clause excludes entities matching a pattern.
/// Source: XTDB "Queries" — not clauses.
#[test]
fn xtdb_not_excludes_matching_entities() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:alice :person true] [:alice :banned true]
        [:bob   :person true]
        [:carol :person true] [:carol :banned true]
    ])"#,
    )
    .unwrap();

    let unbanned = count_results(
        db.execute(r#"(query [:find ?e :where [?e :person true] (not [?e :banned true])])"#)
            .unwrap(),
    );
    assert_eq!(unbanned, 1, "only bob should appear (not banned)");
}

// ── Aggregation ──────────────────────────────────────────────────────────────

/// XTDB concept: count aggregate returns number of matching tuples.
/// Source: XTDB "Aggregates" documentation.
#[test]
fn xtdb_count_aggregate() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:a :tag "rust"] [:b :tag "rust"] [:c :tag "go"] [:d :tag "rust"]
    ])"#,
    )
    .unwrap();

    match db
        .execute(r#"(query [:find (count ?e) :where [?e :tag "rust"]])"#)
        .unwrap()
    {
        QueryResult::QueryResults { results, .. } => {
            assert!(!results.is_empty(), "count aggregate must return a result");
            match results[0][0] {
                Value::Integer(n) => assert_eq!(n, 3, "count of :tag rust should be 3"),
                _ => panic!("expected integer count value"),
            }
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Recursive rules ───────────────────────────────────────────────────────────

/// XTDB concept: recursive rules traverse transitive relationships.
/// Source: XTDB "Rules" — transitive closure.
#[test]
fn xtdb_recursive_ancestor_rule() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:alice :parent :bob]
        [:bob   :parent :carol]
        [:carol :parent :dave]
    ])"#,
    )
    .unwrap();

    db.execute(r#"(rule [(ancestor ?x ?y) [?x :parent ?y]])"#)
        .unwrap();
    db.execute(r#"(rule [(ancestor ?x ?z) [?x :parent ?y] (ancestor ?y ?z)])"#)
        .unwrap();

    let ancestors_of_alice = count_results(
        db.execute(r#"(query [:find ?anc :where (ancestor :alice ?anc)])"#)
            .unwrap(),
    );
    assert_eq!(
        ancestors_of_alice, 3,
        "alice has 3 ancestors: bob, carol, dave"
    );
}

// ── Predicate-style keyword names ─────────────────────────────────────────────

/// XTDB concept: attributes are EDN keywords, and names ending in '?' are the
/// idiomatic Clojure spelling for a predicate (`:artist/dead?`).
///
/// Regression test — '?' formerly terminated the keyword token, so
/// `:artist/dead?` lexed as `:artist/dead` followed by a stray symbol, and a
/// three-element fact was read as a four-element one.
/// Source: XTDB "Data Model" — documents are EDN; attributes are keywords.
#[test]
fn xtdb_attribute_keyword_may_end_in_question_mark() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:pablo    :artist/dead? true]
        [:salvador :artist/dead? true]
        [:banksy   :artist/dead? false]
    ])"#,
    )
    .unwrap();

    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?e ?v :where [?e :artist/dead? ?v]])"#)
                .unwrap()
        ),
        3,
        "every predicate-named fact must be queryable"
    );

    // Binding by value proves the fact was stored as (e, a, v) rather than
    // being misread as a fact with a spurious fourth element.
    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?e :where [?e :artist/dead? false]])"#)
                .unwrap()
        ),
        1,
        "exactly one artist is living"
    );
}

/// XTDB concept: a keyword is a legal attribute *value*, so entities can be
/// joined on one — and it keeps its '?' through storage and back.
/// Source: XTDB "Data Model" — keyword values.
#[test]
fn xtdb_keyword_value_may_end_in_question_mark() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:job1 :job/state :ready?]
        [:job2 :job/state :blocked?]
        [:job3 :job/state :ready?]
    ])"#,
    )
    .unwrap();

    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?e :where [?e :job/state :ready?]])"#)
                .unwrap()
        ),
        2,
        "two jobs are ready"
    );

    match db
        .execute(r#"(query [:find ?s :where [:job2 :job/state ?s]])"#)
        .unwrap()
    {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected one binding");
            assert_eq!(
                results[0][0],
                Value::Keyword(":blocked?".to_string()),
                "value must round-trip with its '?'"
            );
        }
        _ => panic!("expected query results"),
    }
}

/// XTDB concept: retraction targets one fact, and a predicate-named attribute
/// is no different. Exercises the write path, where the original defect turned
/// a three-element fact into a rejected four-element one.
/// Source: XTDB "Transactions" — retract.
#[test]
fn xtdb_retract_predicate_named_attribute() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/admin? true] [:alice :name "Alice"]])"#)
        .unwrap();

    db.execute(r#"(retract [[:alice :person/admin? true]])"#)
        .unwrap();

    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?v :where [?e :person/admin? ?v]])"#)
                .unwrap()
        ),
        0,
        "the predicate-named fact should be retracted"
    );
    assert_eq!(
        query_strings(&db, r#"(query [:find ?n :where [?e :name ?n]])"#),
        vec!["Alice".to_string()],
        "the other fact should survive"
    );
}

/// XTDB concept: bitemporality applies to predicate-named attributes like any
/// other. Valid-time filtering is a separate path from a plain match, so a
/// keyword that survives parsing must survive it too.
/// Source: XTDB "Bitemporality" — valid-time queries.
#[test]
fn xtdb_valid_time_query_over_predicate_named_attribute() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact {:valid-from "2023-01-01" :valid-to "2023-12-31"} [[:contract :contract/active? true]])"#,
    )
    .unwrap();

    assert_eq!(
        count_results(
            db.execute(
                r#"(query [:find ?v :valid-at "2023-06-01" :where [?e :contract/active? ?v]])"#
            )
            .unwrap()
        ),
        1,
        "visible within the valid-time range"
    );
    assert_eq!(
        count_results(
            db.execute(
                r#"(query [:find ?v :valid-at "2024-01-01" :where [?e :contract/active? ?v]])"#
            )
            .unwrap()
        ),
        0,
        "not visible after the valid-to boundary"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// SKIPPED CASES
// ═════════════════════════════════════════════════════════════════════════════
//
// The following XTDB features are intentionally out of scope for Minigraf:
//
// 1. XTDB SQL compatibility — Minigraf uses Datalog only (not SQL/GQL).
//    XTDB v2 added SQL; we do not implement SQL.
//
// 2. XTDB distributed transaction log — Minigraf is embedded single-file;
//    no distributed transaction semantics apply.
//
// 3. XTDB Arrow/Parquet integration — Minigraf uses postcard serialization;
//    columnar formats are out of scope.
//
// 4. XTDB evict! (GDPR deletion) — Minigraf does not yet implement hard
//    deletion (tracked separately). These tests would fail.
//
// 5. XTDB multi-node consistency — Minigraf is single-process only.
