//! A value a `not` / `not-join` body takes from the outer row compares as a
//! join compares it (#546, design
//! `docs/superpowers/specs/2026-10-16-negation-disjunction-engine-design.md`
//! §3.1, §3.6 item 1): floats by IEEE `==`, a keyword equal to a ref to the
//! entity it names. Before, a path that substituted the outer value as a
//! literal matched floats within `f64::EPSILON`, so the answer depended on
//! whether the body held an expression.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};

fn executed(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    match db.execute(&format!("(query {q})")).expect("query") {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn pulled(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    let mut cursor = db.query(&format!("(query {q})")).expect("query");
    let mut rows = Vec::new();
    while let Some(batch) = cursor.next_batch(3).expect("batch") {
        rows.extend(batch.rows().iter().cloned());
    }
    rows
}

/// The entity a keyword names, as `edn_to_entity_id` derives it.
fn named(k: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, k.as_bytes())
}

/// The entities `q` returns, as the keywords that name them, through
/// `execute` and a cursor (which must agree), sorted.
fn entities(db: &Minigraf, q: &str, names: &[&str]) -> Vec<String> {
    let name = |v: &Value| -> String {
        names
            .iter()
            .find(|k| matches!(v, Value::Ref(u) if *u == named(k)))
            .map_or_else(|| "?".to_string(), |k| k.to_string())
    };
    let mut a: Vec<String> = executed(db, q).iter().map(|r| name(&r[0])).collect();
    let mut b: Vec<String> = pulled(db, q).iter().map(|r| name(&r[0])).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "execute and cursor agree");
    a
}

/// Two floats `1e-20` apart: within `f64::EPSILON`, but not equal.
const TINY: &str = "(transact [[:e1 :x 0.00000000000000000001] [:g :y 0.00000000000000000002]])";

/// Each `not` / `not-join` shape over [`TINY`]; every one keeps `:e1`.
const TINY_QUERIES: &[&str] = &[
    // §3.6 item 1: the exclusion-set path and the per-row path.
    "[:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f])]",
    "[:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f] [(> ?f 0.0)])]",
    "[:find ?e :where [?e :x ?f] (not [:g :y ?f])]",
    "[:find ?e :where [?e :x ?f] (not [:g :y ?f] [(> ?f 0.0)])]",
    // A query that invokes a rule takes the rule path for its negation.
    "[:find ?e :where [?e :x ?f] (any ?e) (not [:g :y ?f])]",
    "[:find ?e :where [?e :x ?f] (any ?e) (not-join [?f] [?g :y ?f])]",
    // A rule body's negation.
    "[:find ?e :where (kept ?e)]",
];

fn tiny(db: &Minigraf) {
    db.execute(TINY).unwrap();
    db.execute("(rule [(any ?e) [?e :x _]])").unwrap();
    db.execute("(rule [(kept ?e) [?e :x ?f] (not [:g :y ?f])])")
        .unwrap();
}

#[test]
fn imported_float_compares_by_ieee_equality() {
    let db = Minigraf::in_memory().unwrap();
    tiny(&db);
    for q in TINY_QUERIES {
        assert_eq!(entities(&db, q, &[":e1"]), vec![":e1"], "{q}");
    }
}

#[test]
fn imported_float_compares_by_ieee_equality_after_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neg.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        tiny(&db);
        db.checkpoint().unwrap();
    }
    let db = OpenOptions::new().path(&path).open().unwrap();
    db.execute("(rule [(any ?e) [?e :x _]])").unwrap();
    db.execute("(rule [(kept ?e) [?e :x ?f] (not [:g :y ?f])])")
        .unwrap();
    for q in TINY_QUERIES {
        assert_eq!(entities(&db, q, &[":e1"]), vec![":e1"], "{q}");
    }
}

#[test]
fn imported_zero_equals_negative_zero() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 0.0] [:b :x 1.0] [:g :y -0.0]])")
        .unwrap();
    db.execute("(rule [(any ?e) [?e :x _]])").unwrap();
    for q in [
        "[:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f])]",
        "[:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f] [(>= ?f 0.0)])]",
        "[:find ?e :where [?e :x ?f] (not [:g :y ?f] [(>= ?f 0.0)])]",
        "[:find ?e :where [?e :x ?f] (any ?e) (not [:g :y ?f])]",
    ] {
        assert_eq!(entities(&db, q, &[":a", ":b"]), vec![":b"], "{q}");
    }
}

#[test]
fn imported_keyword_equals_ref_to_its_entity() {
    let db = Minigraf::in_memory().unwrap();
    // `:alice` and `:carol` hold a keyword naming an entity; `:bob` is banned.
    db.execute("(transact [[:alice :friend :bob] [:carol :friend :dave] [:bob :banned true]])")
        .unwrap();
    for q in [
        "[:find ?e :where [?e :friend ?f] (not [?f :banned true])]",
        "[:find ?e :where [?e :friend ?f] (not [?f :banned true] [(< 0 1)])]",
        "[:find ?e :where [?e :friend ?f] (not-join [?f] [?f :banned true])]",
        "[:find ?e :where [?e :friend ?f] (not-join [?f] [?f :banned ?b] [(= ?b true)])]",
    ] {
        assert_eq!(
            entities(&db, q, &[":alice", ":carol"]),
            vec![":carol"],
            "{q}"
        );
    }
}

#[test]
fn body_float_constant_keeps_epsilon_match() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 1.0] [:b :x 2.0] [:a :y 1.0]])")
        .unwrap();
    // A literal in the body still matches within `f64::EPSILON`.
    for q in [
        "[:find ?e :where [?e :x _] (not [?e :y 1.0000000000000001])]",
        "[:find ?e :where [?e :x ?v] (not [?e :y 1.0000000000000001] [(> ?v 0.0)])]",
    ] {
        assert_eq!(entities(&db, q, &[":a", ":b"]), vec![":b"], "{q}");
    }
}
