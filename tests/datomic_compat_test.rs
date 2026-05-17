//! Datomic-inspired compatibility tests (#221).
//!
//! License notice: Datomic is a commercial product. These tests are
//! INDEPENDENTLY WRITTEN semantic ports — they test concepts from Datomic's
//! query model (EAV, pull, as-of, history) but share no code or literal
//! test data with any Datomic test suite. Written from scratch based on
//! Datomic's public documentation.
//!
//! Run: cargo test --test datomic_compat_test

#![cfg(not(target_arch = "wasm32"))]

use minigraf::{BindValue, Minigraf, QueryResult, Value};

fn count_results(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

// ── Datomic concept: EAV triple model ────────────────────────────────────────

/// Datomic concept: entity attributes are independent facts (datoms).
/// Datomic doc reference: "Datomic Data Model" — datoms.
#[test]
fn datomic_entity_attributes_are_independent_facts() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:user42 :user/name "Jane"]
        [:user42 :user/email "jane@example.com"]
        [:user42 :user/role :admin]
    ])"#,
    )
    .unwrap();

    // Each attribute is an independent queryable fact.
    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?n :where [?e :user/name ?n]])"#)
                .unwrap()
        ),
        1,
        "name fact must be independently queryable"
    );
    assert_eq!(
        count_results(
            db.execute(r#"(query [:find ?em :where [?e :user/email ?em]])"#)
                .unwrap()
        ),
        1,
        "email fact must be independently queryable"
    );
}

/// Datomic concept: multiple entities sharing the same attribute — each entity independently
/// carries its own :tag fact; querying [:find ?e ?t] returns all (entity, value) pairs.
/// Datomic doc reference: "Schema" — EAV model; each datom is an independent fact.
#[test]
fn datomic_multiple_entities_same_attribute() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:article1 :tag "rust"]
        [:article2 :tag "database"]
        [:article3 :tag "embedded"]
    ])"#,
    )
    .unwrap();

    let tags = count_results(
        db.execute(r#"(query [:find ?e ?t :where [?e :tag ?t]])"#)
            .unwrap(),
    );
    assert_eq!(tags, 3, "all 3 tag facts must be independently queryable");
}

// ── Datomic concept: transaction metadata ────────────────────────────────────

/// Datomic concept: transaction time (tx-id) is queryable via :as-of.
/// Datomic doc reference: "Time" — transaction entity.
/// Minigraf equivalent: :as-of by tx_count.
#[test]
fn datomic_transaction_time_as_of() {
    let db = Minigraf::in_memory().unwrap();

    // tx 1
    db.execute(r#"(transact [[:inv :qty 10]])"#).unwrap();
    // tx 2 + 3
    db.execute(r#"(retract [[:inv :qty 10]])"#).unwrap();
    db.execute(r#"(transact [[:inv :qty 20]])"#).unwrap();

    // As-of tx 1 must return qty = 10.
    match db
        .execute(r#"(query [:find ?q :as-of 1 :valid-at :any-valid-time :where [?e :qty ?q]])"#)
        .unwrap()
    {
        QueryResult::QueryResults { results, .. } => {
            let qty: Vec<i64> = results
                .into_iter()
                .flatten()
                .filter_map(|v| match v {
                    Value::Integer(n) => Some(n),
                    _ => None,
                })
                .collect();
            assert!(qty.contains(&10), "as-of tx 1: qty must be 10");
        }
        _ => panic!("expected QueryResults"),
    }
}

// ── Datomic concept: retraction ──────────────────────────────────────────────

/// Datomic concept: retract-entity removes all facts about an entity.
/// Datomic doc reference: "Transactions" — :db/retractEntity.
/// Minigraf equivalent: retract each attribute individually.
#[test]
fn datomic_retract_all_entity_facts() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:ghost :name "Ghost"]
        [:ghost :age 100]
        [:ghost :role "phantom"]
    ])"#,
    )
    .unwrap();

    db.execute(
        r#"(retract [
        [:ghost :name "Ghost"]
        [:ghost :age 100]
        [:ghost :role "phantom"]
    ])"#,
    )
    .unwrap();

    let n = count_results(
        db.execute(r#"(query [:find ?e :where [?e :name ?n]])"#)
            .unwrap(),
    );
    assert_eq!(n, 0, "fully-retracted entity must not appear in queries");
}

// ── Datomic concept: Datalog query patterns ───────────────────────────────────

/// Datomic concept: find tuples (multi-find-variable query).
/// Datomic doc reference: "Queries" — :find with multiple variables.
#[test]
fn datomic_multi_variable_find() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:p1 :person/name "Alice"] [:p1 :person/age 30]
        [:p2 :person/name "Bob"]   [:p2 :person/age 25]
    ])"#,
    )
    .unwrap();

    match db
        .execute(r#"(query [:find ?n ?a :where [?e :person/name ?n] [?e :person/age ?a]])"#)
        .unwrap()
    {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 2, "should find 2 name+age pairs");
        }
        _ => panic!("expected QueryResults"),
    }
}

/// Datomic concept: ground values in queries (constant binding).
/// Datomic doc reference: "Queries" — binding constants.
#[test]
fn datomic_ground_value_binding() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:a :score 10] [:b :score 20] [:c :score 10] [:d :score 30]
    ])"#,
    )
    .unwrap();

    let tens = count_results(
        db.execute(r#"(query [:find ?e :where [?e :score 10]])"#)
            .unwrap(),
    );
    assert_eq!(tens, 2, "entities with score=10: a and c");
}

/// Datomic concept: :in clause (parameterized query / prepared statements).
/// Datomic doc reference: "Queries" — :in bindings.
/// Minigraf equivalent: prepared queries with $slot bindings.
#[test]
fn datomic_parameterized_query_prepared() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:x :val 42] [:y :val 7] [:z :val 42]
    ])"#,
    )
    .unwrap();

    let prep = db
        .prepare("(query [:find ?e :where [?e :val $target]])")
        .unwrap();

    let results_42 = count_results(
        prep.execute(&[("target", BindValue::Val(Value::Integer(42)))])
            .unwrap(),
    );
    let results_7 = count_results(
        prep.execute(&[("target", BindValue::Val(Value::Integer(7)))])
            .unwrap(),
    );

    assert_eq!(results_42, 2, "val=42: x and z");
    assert_eq!(results_7, 1, "val=7: y only");
}

/// Datomic concept: rules are named reusable query fragments.
/// Datomic doc reference: "Rules".
#[test]
fn datomic_named_rule_reuse() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:a :likes :b] [:b :likes :c] [:c :likes :a]
    ])"#,
    )
    .unwrap();

    db.execute(r#"(rule [(likes-transitively ?x ?y) [?x :likes ?y]])"#)
        .unwrap();
    db.execute(r#"(rule [(likes-transitively ?x ?z) [?x :likes ?y] (likes-transitively ?y ?z)])"#)
        .unwrap();

    let all_pairs = count_results(
        db.execute(r#"(query [:find ?x ?y :where (likes-transitively ?x ?y)])"#)
            .unwrap(),
    );
    // In a 3-node cycle, every entity transitively likes every other.
    assert!(
        all_pairs >= 3,
        "transitive closure must find at least 3 pairs"
    );
}

// ── Datomic concept: predicates and filters ───────────────────────────────────

/// Datomic concept: predicate expressions in :where clauses.
/// Datomic doc reference: "Queries" — expression clauses.
#[test]
fn datomic_predicate_expression_filter() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [
        [:a :age 25] [:b :age 35] [:c :age 15] [:d :age 40]
    ])"#,
    )
    .unwrap();

    let adults = count_results(
        db.execute(r#"(query [:find ?e :where [?e :age ?a] [(>= ?a 18)]])"#)
            .unwrap(),
    );
    assert_eq!(adults, 3, "entities with age >= 18: a, b, d");
}

// ═════════════════════════════════════════════════════════════════════════════
// SKIPPED CASES
// ═════════════════════════════════════════════════════════════════════════════
//
// The following Datomic concepts are intentionally out of scope or divergent:
//
// 1. Pull API — Datomic has a pull syntax for shaped reads. Minigraf uses
//    pattern-matching :find/:where queries. No pull syntax planned.
//
// 2. :db/ident — Datomic uses schema-defined attribute identities. Minigraf
//    uses string keywords directly without a separate schema registry.
//
// 3. :db/unique — Datomic enforces uniqueness constraints at schema level.
//    Minigraf does not enforce attribute uniqueness (multiple values allowed).
//
// 4. Transaction functions — Datomic supports arbitrary Clojure functions
//    in transactions. Out of scope for Minigraf.
//
// 5. Excision / hard delete — Datomic's `d/excise`. Not implemented in Minigraf.
//
// 6. Peer vs. Client API — Datomic has two access modes. Minigraf is always
//    embedded; no distinction applies.
