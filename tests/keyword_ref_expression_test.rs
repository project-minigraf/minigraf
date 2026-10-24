//! An expression's `=` and `!=` equate a keyword with a ref to the entity it
//! names, as pattern constants and joins do (#744). Before, they compared
//! the two values as they are, so `[(= ?g :bob)]` kept no row where the
//! pattern `[?x :ref/friend :bob]` matched one.

use minigraf::{BindValue, Minigraf, QueryResult, Value};

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
/// `:alice` holds a ref to `:bob`'s entity, `:carol` the keyword `:bob`.
fn ref_and_keyword_db() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    db.execute(&format!(
        r#"(transact [[:alice :ref/friend #uuid "{}"] [:carol :kw/friend :bob] [:dan :kw/friend :eve]])"#,
        named(":bob")
    ))
    .unwrap();
    db
}

/// An expression's `=` and `!=` equate a keyword with a ref to the entity
/// it names, as patterns and joins do (#744).
fn assert_expression_equality_matches_joins(db: &Minigraf) {
    for (q, want) in [
        // The pattern constant and the join the expressions must agree with.
        ("[:find ?x :where [?x :ref/friend :bob]]", 1),
        (
            "[:find ?x :where [?y :ref/friend ?g] [?x :kw/friend ?g]]",
            1,
        ),
        // A ref against a keyword constant, either side.
        ("[:find ?x :where [?x :ref/friend ?g] [(= ?g :bob)]]", 1),
        ("[:find ?x :where [?x :ref/friend ?g] [(= :bob ?g)]]", 1),
        ("[:find ?x :where [?x :ref/friend ?g] [(!= ?g :bob)]]", 0),
        ("[:find ?x :where [?x :ref/friend ?g] [(= ?g :eve)]]", 0),
        ("[:find ?x :where [?x :ref/friend ?g] [(!= ?g :eve)]]", 1),
        // A ref against a keyword value.
        (
            "[:find ?x :where [?y :ref/friend ?g] [?x :kw/friend ?k] [(= ?g ?k)]]",
            1,
        ),
        (
            "[:find ?x :where [?y :ref/friend ?g] [?x :kw/friend ?k] [(!= ?k ?g)]]",
            1,
        ),
        // A keyword still equals the same keyword only.
        ("[:find ?x :where [?x :kw/friend ?k] [(= ?k :bob)]]", 1),
        ("[:find ?x :where [?x :kw/friend ?k] [(!= ?k :bob)]]", 1),
    ] {
        assert_eq!(query(db, q).len(), want, "{q}");
    }
    // A binding expression's value is the comparison.
    let r = query(db, "[:find ?b :where [?x :ref/friend ?g] [(= ?g :bob) ?b]]");
    assert!(matches!(r[..], [ref row] if row[0] == Value::Boolean(true)));
    // A keyword value against a bound ref.
    let p = db
        .prepare("(query [:find ?x :where [?x :kw/friend ?k] [(= ?k $r)]])")
        .unwrap();
    let r = rows(
        p.execute(&[("r", BindValue::Val(Value::Ref(named(":bob"))))])
            .unwrap(),
    );
    assert_eq!(r.len(), 1, "bound ref");
}

#[test]
fn expression_equality_equates_keyword_and_ref() {
    assert_expression_equality_matches_joins(&ref_and_keyword_db());
}

#[test]
fn expression_equality_in_rule_bodies_and_negation() {
    let db = ref_and_keyword_db();
    db.execute("(rule [(bobs-friend ?x) [?x :ref/friend ?g] [(= ?g :bob)]])")
        .unwrap();
    assert_eq!(query(&db, "[:find ?x :where (bobs-friend ?x)]").len(), 1);
    let r = query(
        &db,
        "[:find ?x :where [?x :ref/friend ?g] (not [(= ?g :bob)])]",
    );
    assert!(r.is_empty(), "not of an equal pair");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn expression_equality_equates_keyword_and_ref_after_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kw-ref.graph");
    let db = Minigraf::open(&path).unwrap();
    db.execute(&format!(
        r#"(transact [[:alice :ref/friend #uuid "{}"] [:carol :kw/friend :bob] [:dan :kw/friend :eve]])"#,
        named(":bob")
    ))
    .unwrap();
    db.checkpoint().unwrap();
    assert_expression_equality_matches_joins(&db);
    drop(db);
    assert_expression_equality_matches_joins(&Minigraf::open(&path).unwrap());
}

#[test]
fn expression_equality_in_or_branches_and_on_uuid_keywords() {
    let db = ref_and_keyword_db();
    let r = query(
        &db,
        "[:find ?x :where [?x :ref/friend ?g] (or [(= ?g :bob)] [(= ?g :eve)])]",
    );
    assert_eq!(r.len(), 1, "or branch");
    // A keyword whose name is a UUID names the entity with that UUID.
    let r = query(
        &db,
        &format!(
            "[:find ?x :where [?x :ref/friend ?g] [(= ?g :{})]]",
            named(":bob")
        ),
    );
    assert_eq!(r.len(), 1, "UUID keyword");
}
