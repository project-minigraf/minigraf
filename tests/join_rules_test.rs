//! Join rules that do not depend on the query plan (#545, #473 spec §5.4):
//! bound floats are equal by IEEE `==`, and a variable bound to both a
//! keyword and a ref returns the kind from the first pattern, in `:where`
//! text order, that binds it.

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

/// `1e-17`, `0.0` and `-0.0`: the first is within `f64::EPSILON` of the
/// others but equal to neither (#706).
const SMALL_FLOATS: &str =
    "(transact [[:tiny :m/x 0.00000000000000001] [:zero :m/x 0.0] [:neg :m/x -0.0]])";

/// The keywords naming the entities in `r`'s first column, sorted.
fn names_of(r: &[Vec<Value>]) -> Vec<&'static str> {
    let mut n: Vec<&str> = r
        .iter()
        .map(|row| {
            [":tiny", ":zero", ":neg"]
                .into_iter()
                .find(|k| is_ref_to(&row[0], k))
                .unwrap_or("?")
        })
        .collect();
    n.sort();
    n
}

/// A float constant written in a pattern, bound to a `$slot`, written in a
/// rule body, or compared by a predicate matches equal floats only (#706).
fn assert_float_constants_exact(db: &Minigraf) {
    db.execute("(rule [(zeroish ?e) [?e :m/x 0.0]])").unwrap();
    db.execute("(rule [(xval ?e ?v) [?e :m/x ?v]])").unwrap();
    for q in [
        "[:find ?e :where [?e :m/x 0.0]]",
        "[:find ?e :where [?e :m/x -0.0]]",
        "[:find ?e :where [?e :m/x ?v] [(= ?v 0.0)]]",
        "[:find ?e :where (zeroish ?e)]",
        "[:find ?e :where (xval ?e 0.0)]",
    ] {
        assert_eq!(names_of(&query(db, q)), [":neg", ":zero"], "{q}");
    }
    let r = query(db, "[:find ?e :where (xval ?e 0.00000000000000001)]");
    assert_eq!(names_of(&r), [":tiny"], "a rule call's float argument");
    let r = query(db, "[:find ?e :where [?e :m/x 0.00000000000000001]]");
    assert_eq!(names_of(&r), [":tiny"], "1e-17 matches itself only");
    let r = query(db, "[:find ?e :where [?e :m/x 0.00000000000000002]]");
    assert!(r.is_empty(), "2e-17 matches nothing");

    let p = db
        .prepare("(query [:find ?e :where [?e :m/x $v]])")
        .unwrap();
    for (v, want) in [(0.0, &[":neg", ":zero"][..]), (1e-17, &[":tiny"][..])] {
        let r = rows(
            p.execute(&[("v", BindValue::Val(Value::Float(v)))])
                .unwrap(),
        );
        assert_eq!(names_of(&r), want, "bound {v}");
    }
}

#[test]
fn float_constant_matches_equal_floats_only() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(SMALL_FLOATS).unwrap();
    assert_float_constants_exact(&db);
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn float_constant_matches_equal_floats_only_after_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("floats.graph");
    let db = Minigraf::open(&path).unwrap();
    db.execute(SMALL_FLOATS).unwrap();
    assert_float_constants_exact(&db);
    db.checkpoint().unwrap();
    assert_float_constants_exact(&db);
    drop(db);
    assert_float_constants_exact(&Minigraf::open(&path).unwrap());
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
        ("[:find ?x :where [?x :ref/friend ?g] [(!= :bob ?g)]]", 0),
        ("[:find ?x :where [?x :ref/friend ?g] [(= ?g :eve)]]", 0),
        ("[:find ?x :where [?x :ref/friend ?g] [(!= ?g :eve)]]", 1),
        ("[:find ?x :where [?x :ref/friend ?g] [(!= :eve ?g)]]", 1),
        (
            "[:find ?x :where [?x :ref/friend ?g] [(= (= :bob ?g) true)]]",
            1,
        ),
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

    // #774: each execution resolves its substituted keyword; an earlier
    // execution's literal must not leak into a later one. Cover both operand
    // orders and keep the keyword spelling in the returned value.
    for (op, reverse) in [("=", false), ("=", true), ("!=", false), ("!=", true)] {
        let args = if reverse { "$k ?g" } else { "?g $k" };
        let p = db
            .prepare(&format!(
                "(query [:find ?x :where [?x :ref/friend ?g] [({op} {args})]])"
            ))
            .unwrap();
        for k in [":bob", ":eve", ":bob"] {
            let r = rows(
                p.execute(&[("k", BindValue::Val(Value::Keyword(k.into())))])
                    .unwrap(),
            );
            assert_eq!(r.len(), usize::from((k == ":bob") == (op == "=")));
        }
    }
    let r = query(db, "[:find ?k :where [?x :kw/friend ?k] [(= ?k :bob)]]");
    assert!(matches!(r.as_slice(), [row] if is_keyword(&row[0], ":bob")));
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
    let u = named(":bob");
    let r = query(
        &db,
        &format!("[:find ?x :where [?x :ref/friend ?g] [(= ?g :{u})]]"),
    );
    assert_eq!(r.len(), 1, "UUID keyword");
    // Set elements still compare as they are: a keyword element never
    // equals a ref element (#316).
    db.execute(&format!(
        r#"(transact [[:s1 :tags #{{:bob}}] [:s2 :tags #{{#uuid "{u}"}}]])"#
    ))
    .unwrap();
    let r = query(
        &db,
        "[:find ?a :where [:s1 :tags ?a] [:s2 :tags ?b] [(= ?a ?b)]]",
    );
    assert!(r.is_empty(), "sets compare structurally");
}
