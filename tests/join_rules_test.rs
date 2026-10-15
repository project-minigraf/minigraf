//! Join rules that do not depend on the query plan (#545, #473 spec §5.4):
//! bound floats are equal by IEEE `==`, and a variable bound to both a
//! keyword and a ref returns the kind from the first pattern, in `:where`
//! text order, that binds it.

use minigraf::{Minigraf, QueryResult, Value};

fn rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn query(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    rows(db.execute(&format!("(query {q})")).expect("query"))
}

/// The entity a keyword names, as `edn_to_entity_id` derives it.
fn named(k: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, k.as_bytes())
}

fn is_keyword(v: &Value, k: &str) -> bool {
    matches!(v, Value::Keyword(x) if x == k)
}

fn is_ref_to(v: &Value, k: &str) -> bool {
    matches!(v, Value::Ref(u) if *u == named(k))
}

#[test]
fn float_join_pairs_zero_with_negative_zero() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 0.0] [:b :y -0.0] [:c :x 1.5] [:d :y 1.5]])")
        .unwrap();
    // `?v` is the join key: entity unbound, value bound.
    let r = query(&db, "[:find ?e ?f :where [?e :x ?v] [?f :y ?v]]");
    assert_eq!(r.len(), 2, "0.0 joins -0.0, 1.5 joins 1.5");
    // `?e` is the join key and `?v` a shared variable compared after it.
    db.execute("(transact [[:a :y -0.0]])").unwrap();
    let r = query(&db, "[:find ?e :where [?e :x ?v] [?e :y ?v]]");
    assert_eq!(r.len(), 1, "a's 0.0 and -0.0 agree");
}

#[test]
fn float_constant_keeps_epsilon_match() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 1.0]])").unwrap();
    let r = query(&db, "[:find ?e :where [?e :x 1.0000000000000001]]");
    assert_eq!(r.len(), 1, "a float constant matches within epsilon");
}

fn friends_db() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    // `:friend` holds a keyword that names `:bob`.
    db.execute(r#"(transact [[:alice :friend :bob] [:bob :name "Bob"]])"#)
        .unwrap();
    db
}

#[test]
fn keyword_or_ref_follows_where_order_not_plan() {
    let db = friends_db();
    // The planner runs `[?b :name "Bob"]` first (two constants), which
    // binds `?b` to a ref; the first pattern in the text binds a keyword.
    let r = query(&db, r#"[:find ?b :where [?a :friend ?b] [?b :name "Bob"]]"#);
    assert_eq!(r.len(), 1);
    assert!(
        is_keyword(&r[0][0], ":bob"),
        "keyword from the first pattern"
    );

    let r = query(&db, r#"[:find ?b :where [?b :name "Bob"] [?a :friend ?b]]"#);
    assert_eq!(r.len(), 1);
    assert!(is_ref_to(&r[0][0], ":bob"), "ref from the first pattern");
}

#[test]
fn keyword_or_ref_same_order_both_ways() {
    let db = friends_db();
    // Same selectivity: plan order is text order, as before.
    let r = query(&db, "[:find ?b :where [?a :friend ?b] [?b :name ?n]]");
    assert_eq!(r.len(), 1);
    assert!(is_keyword(&r[0][0], ":bob"));
    let r = query(&db, "[:find ?b :where [?b :name ?n] [?a :friend ?b]]");
    assert_eq!(r.len(), 1);
    assert!(is_ref_to(&r[0][0], ":bob"));
}

#[test]
fn keyword_or_ref_through_the_attribute_position() {
    let db = Minigraf::in_memory().unwrap();
    // `:ptr` refers to the entity that the keyword `:color` names, and
    // `:color` is also an attribute.
    db.execute(&format!(
        r#"(transact [[:e1 :ptr #uuid "{}"] [:e2 :color "red"]])"#,
        named(":color")
    ))
    .unwrap();
    // The planner runs `[?x :ptr ?a]` first (one constant), binding `?a` to
    // a ref; the first pattern in the text binds the attribute keyword.
    let r = query(&db, "[:find ?a :where [?e ?a ?v] [?x :ptr ?a]]");
    assert_eq!(r.len(), 1);
    assert!(is_keyword(&r[0][0], ":color"), "attribute keyword first");
    let r = query(&db, "[:find ?a :where [?x :ptr ?a] [?e ?a ?v]]");
    assert_eq!(r.len(), 1);
    assert!(is_ref_to(&r[0][0], ":color"), "ref first");
}

#[test]
fn expression_binding_unifies_and_the_first_binder_gives_the_value() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:f :m 0.0] [:e :n 0.0] [:g :k 0.0]])")
        .unwrap();
    // Plan: `[:f :m ?v]` binds `?v`, `[:e :n ?w]`, the expression's -0.0
    // equals it (#596), then `[?g :k ?v]` joins on it. That pattern comes
    // first in the text, so its 0.0 is returned.
    let r = query(
        &db,
        "[:find ?v :where [?g :k ?v] [:f :m ?v] [:e :n ?w] [(* ?w -1.0) ?v]]",
    );
    assert_eq!(r.len(), 1);
    assert!(
        matches!(r[0][0], Value::Float(f) if f == 0.0 && f.is_sign_positive()),
        "the first pattern's 0.0"
    );
    // Natively `[:f :m ?v]` runs first; the expression, earlier in the
    // text, gives its -0.0.
    let r = query(
        &db,
        "[:find ?v :where [?x :n ?w] [(* ?w -1.0) ?v] [:f :m ?v]]",
    );
    assert_eq!(r.len(), 1);
    assert!(
        matches!(r[0][0], Value::Float(f) if f == 0.0 && f.is_sign_negative()),
        "the expression's -0.0"
    );
}

#[test]
fn keyword_or_ref_in_a_rule_query() {
    let db = friends_db();
    db.execute("(rule [(knows ?a ?b) [?a :friend ?b]])")
        .unwrap();
    let r = query(&db, r#"[:find ?b :where (knows ?a ?b) [?b :name "Bob"]]"#);
    assert_eq!(r.len(), 1);
    assert!(is_keyword(&r[0][0], ":bob"), "keyword from the rule call");
}

#[test]
fn keyword_or_ref_after_a_rule_call() {
    let db = friends_db();
    db.execute("(rule [(knows ?a ?b) [?a :friend ?b]])")
        .unwrap();
    let r = query(&db, r#"[:find ?b :where [?b :name "Bob"] (knows ?a ?b)]"#);
    assert_eq!(r.len(), 1);
    assert!(is_ref_to(&r[0][0], ":bob"), "ref from the first pattern");
}

#[test]
fn keyword_or_ref_with_a_repeated_pattern() {
    let db = friends_db();
    let r = query(
        &db,
        r#"[:find ?b :where [?a :friend ?b] [?b :name "Bob"] [?a :friend ?b]]"#,
    );
    assert_eq!(r.len(), 1);
    assert!(
        is_keyword(&r[0][0], ":bob"),
        "keyword from the first pattern"
    );
}

// Whatever runs first, the first real pattern gives `?b` its value (#606).
#[test]
fn pseudo_attribute_pattern_never_replaces_a_value() {
    let db = friends_db();
    // `[?b :db/valid-at ?t]` is first in the text and the planner runs it
    // last, matching every fact of `:bob`; it keeps `?b` as bound.
    let r = query(
        &db,
        r#"[:find ?b ?t :where [?b :db/valid-at ?t] [?a :friend ?b] [?b :name "Bob"]]"#,
    );
    assert_eq!(r.len(), 1);
    assert!(
        is_keyword(&r[0][0], ":bob"),
        "keyword from the first real pattern"
    );
}
