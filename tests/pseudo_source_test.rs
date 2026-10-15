//! Pseudo-attribute sources (#594, design
//! `docs/superpowers/specs/2026-10-15-plan-independent-answers-design.md`
//! §3.1–3.3): a per-fact pseudo-attribute (`:db/valid-from`, `:db/valid-to`,
//! `:db/tx-count`, `:db/tx-id`) reads the fact of the nearest pattern before
//! it in the text with the same entity term and a stored attribute, in its own
//! clause list and then outwards before each containing `not` / `or`
//! (`not-join` / `or-join` only for a listed entity variable), never past a
//! rule body; with none the query is PRS-081.

use minigraf::{Minigraf, QueryResult, Value};

fn rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn query(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    let mut r = rows(db.execute(&format!("(query {q})")).expect("query"));
    r.sort_by_key(|row| {
        row.iter()
            .map(|v| match v {
                Value::Integer(i) => format!("{i:020}"),
                Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
    });
    r
}

fn code(db: &Minigraf, q: &str) -> &'static str {
    db.execute(&format!("(query {q})"))
        .expect_err("query must fail")
        .code()
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        _ => panic!("expected an integer"),
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn alice() -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b":alice")
}

/// tx 1: Alice's name (valid from 2020); tx 2: her age (valid from 2021);
/// tx 3: Bob's name; tx 4: Alice's friend; tx 5: a count of 3.
fn db() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact {:valid-from "2020-01-01"} [[:alice :person/name "Alice"]])"#)
        .unwrap();
    db.execute(r#"(transact {:valid-from "2021-01-01"} [[:alice :person/age 30]])"#)
        .unwrap();
    db.execute(r#"(transact [[:bob :person/name "Bob"]])"#)
        .unwrap();
    db.execute(r#"(transact [[:alice :person/friend :bob]])"#)
        .unwrap();
    db.execute(r#"(transact [[:k :count 3]])"#).unwrap();
    db
}

const ANY: &str = ":any-valid-time";

#[test]
fn nearest_earlier_pattern_in_the_text_is_the_source() {
    let db = db();
    // #594's example: the age pattern is nearest, whatever runs last.
    let r = query(
        &db,
        &format!(
            "[:find ?n ?t {ANY} :where [?e :person/name ?n] [?e :person/age ?a] [?e :db/tx-count ?t]]"
        ),
    );
    assert_eq!(
        r,
        vec![vec![s("Alice"), Value::Integer(2)]],
        "age fact, tx 2"
    );
    let r = query(
        &db,
        &format!(
            "[:find ?n ?t {ANY} :where [?e :person/age ?a] [?e :person/name ?n] [?e :db/tx-count ?t]]"
        ),
    );
    assert_eq!(
        r,
        vec![vec![s("Alice"), Value::Integer(1)]],
        "name fact, tx 1"
    );
}

#[test]
fn two_pseudo_attributes_share_a_source() {
    let db = db();
    let r = query(
        &db,
        &format!(
            "[:find ?t ?f {ANY} :where [?e :person/age _] [?e :db/tx-count ?t] [?e :db/valid-from ?f]]"
        ),
    );
    assert_eq!(r.len(), 1);
    assert_eq!(int(&r[0][0]), 2);
    assert_eq!(int(&r[0][1]), 1_609_459_200_000, "2021-01-01");
}

#[test]
fn every_fact_of_an_entity_through_a_variable_attribute() {
    let db = db();
    // One row per fact of :alice: name (1), age (2), friend (4).
    let r = query(
        &db,
        &format!("[:find ?a ?t {ANY} :where [:alice ?a _] [:alice :db/tx-count ?t]]"),
    );
    let ts: Vec<i64> = r.iter().map(|row| int(&row[1])).collect();
    assert_eq!(ts, vec![1, 2, 4]);
}

#[test]
fn constant_and_bound_values_check_the_metadata() {
    let db = db();
    let r = query(
        &db,
        &format!("[:find ?n {ANY} :where [?e :person/name ?n] [?e :db/tx-count 3]]"),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
    // `?t` is bound by `[?k :count ?t]` first: the pseudo-attribute checks.
    let r = query(
        &db,
        &format!(
            "[:find ?n {ANY} :where [?e :person/name ?n] [?k :count ?t] [?e :db/tx-count ?t]]"
        ),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
}

#[test]
fn constants_name_the_same_entity_by_id() {
    let db = db();
    let r = query(
        &db,
        &format!(
            r#"[:find ?t {ANY} :where [:alice :person/age _] [#uuid "{}" :db/tx-count ?t]]"#,
            alice()
        ),
    );
    assert_eq!(r, vec![vec![Value::Integer(2)]]);
    // A variable and a constant are never the same term.
    assert_eq!(
        code(
            &db,
            &format!("[:find ?t {ANY} :where [?e :person/age _] [:alice :db/tx-count ?t]]")
        ),
        "PRS-081"
    );
}

#[test]
fn no_source_is_prs_081() {
    let db = db();
    for q in [
        // The pattern is after the pseudo-attribute.
        "[:find ?t :any-valid-time :where [?e :db/tx-count ?t] [?e :person/name ?n]]",
        // Alone.
        "[:find ?t :any-valid-time :where [:alice :db/valid-to ?t]]",
        // `_` and `?_` entities are never the same as anything.
        "[:find ?t :any-valid-time :where [_ :person/name ?n] [_ :db/tx-id ?t]]",
        "[:find ?t :any-valid-time :where [?_x :person/name ?n] [?_x :db/tx-id ?t]]",
        // Another entity's pattern.
        "[:find ?t :any-valid-time :where [?e :person/friend ?f] [?f :db/tx-count ?t]]",
        // A rule call is not a source.
        "[:find ?t :any-valid-time :where (knows ?e ?f) [?e :db/tx-count ?t]]",
    ] {
        assert_eq!(code(&db, q), "PRS-081", "{q}");
    }
}

#[test]
fn a_not_body_reads_the_outer_fact() {
    let db = db();
    // Alice's name fact is tx 1, Bob's tx 3: the body reads the outer
    // `[?e :person/name ?n]` fact, not any fact of the entity.
    let r = query(
        &db,
        &format!("[:find ?n {ANY} :where [?e :person/name ?n] (not [?e :db/tx-count 1])]"),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
    let r = query(
        &db,
        &format!("[:find ?n {ANY} :where [?e :person/name ?n] (not [?e :db/tx-count 2])]"),
    );
    assert_eq!(r.len(), 2, "no name fact is tx 2");
    // A source inside the body.
    let r = query(
        &db,
        &format!(
            "[:find ?n {ANY} :where [?e :person/name ?n] (not [?e :person/age _] [?e :db/tx-count 2])]"
        ),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
}

#[test]
fn not_join_crosses_only_for_a_listed_variable() {
    let db = db();
    let r = query(
        &db,
        &format!(
            "[:find ?n {ANY} :where [?e :person/name ?n] (not-join [?e] [?e :db/tx-count 1])]"
        ),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
    // `?e` is not listed: inside it is fresh, with no source.
    assert_eq!(
        code(
            &db,
            &format!(
                "[:find ?n {ANY} :where [?e :person/name ?n] (not-join [?n] [?e :db/tx-count 1])]"
            )
        ),
        "PRS-081"
    );
}

#[test]
fn or_branches_read_the_outer_fact_never_a_sibling() {
    let db = db();
    let r = query(
        &db,
        &format!(
            "[:find ?n {ANY} :where [?e :person/name ?n] (or [?e :db/tx-count 1] [?e :db/tx-count 3])]"
        ),
    );
    assert_eq!(r.len(), 2);
    // The second branch reads the name fact (tx 1), not the first branch's
    // age pattern (tx 2), so nothing matches.
    let r = query(
        &db,
        &format!(
            "[:find ?n {ANY} :where [?e :person/name ?n] (or [?e :person/age 99] [?e :db/tx-count 2])]"
        ),
    );
    assert!(r.is_empty());
    let r = query(
        &db,
        &format!("[:find ?n {ANY} :where [?e :person/name ?n] (or-join [?e] [?e :db/tx-count 3])]"),
    );
    assert_eq!(r, vec![vec![s("Bob")]]);
    assert_eq!(
        code(
            &db,
            &format!(
                "[:find ?n {ANY} :where [?e :person/name ?n] (or-join [?n] [?e :db/tx-count 3])]"
            )
        ),
        "PRS-081"
    );
}

#[test]
fn rule_bodies_are_closed() {
    let db = db();
    let err = db
        .execute("(rule [(stamp ?e ?t) [?e :db/tx-count ?t]])")
        .expect_err("no source in the body");
    assert_eq!(err.code(), "PRS-081");
    db.execute("(rule [(stamp ?e ?t) [?e :person/name _] [?e :db/tx-count ?t]])")
        .unwrap();
    // The caller's age pattern is not the source: the body's name fact is.
    let r = query(
        &db,
        &format!("[:find ?t {ANY} :where [?e :person/age _] (stamp ?e ?t)]"),
    );
    assert_eq!(r, vec![vec![Value::Integer(1)]]);
}

#[test]
fn prepared_slots_compare_by_name() {
    let db = db();
    // Resolved on the template, before any value is bound: the same slot is
    // the same term, whatever it is bound to later.
    db.prepare("(query [:find ?t :any-valid-time :where [$p :person/age _] [$p :db/tx-count ?t]])")
        .expect("same slot");
    let err = db
        .prepare(
            "(query [:find ?t :any-valid-time :where [$p :person/age _] [$q :db/tx-count ?t]])",
        )
        .expect_err("different slots");
    assert_eq!(err.code(), "PRS-081");
}

#[test]
fn valid_at_is_unchanged() {
    let db = db();
    // `:db/valid-at` needs no source: the query's valid time, once per
    // visible fact of the entity (name, age, friend).
    let r = query(&db, "[:find ?v :where [:alice :db/valid-at ?v]]");
    assert_eq!(r.len(), 3, "one row per fact");
}
