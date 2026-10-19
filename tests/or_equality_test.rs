//! An `or` / `or-join` branch row meets the outer row by join equality (#546,
//! design `docs/superpowers/specs/2026-10-16-negation-disjunction-engine-design.md`
//! §3.5, §3.6 item 3, §5.3): a keyword equal to a ref to the entity it names,
//! floats by IEEE `==`. Before, the path that evaluates branches unseeded and
//! hash-joins their rows compared exact `Value`s, so a keyword never met a ref,
//! while the seeded path (a branch with `not`) matched them: the answer
//! depended on whether a branch held a `not`.
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

/// Each row of `q` as text (`:k` for a ref to the entity `:k` names, the
/// value otherwise), through `execute` and a cursor (which must agree), sorted.
fn rows(db: &Minigraf, q: &str, names: &[&str]) -> Vec<String> {
    let cell = |v: &Value| -> String {
        match v {
            Value::Ref(u) => names
                .iter()
                .find(|k| *u == named(k))
                .map_or_else(|| "?".to_string(), |k| k.to_string()),
            Value::Keyword(k) => format!("kw{k}"),
            Value::String(s) => s.clone(),
            Value::Integer(i) => i.to_string(),
            _ => "other".to_string(),
        }
    };
    let text = |rs: Vec<Vec<Value>>| -> Vec<String> {
        let mut t: Vec<String> = rs
            .iter()
            .map(|r| r.iter().map(cell).collect::<Vec<_>>().join(" "))
            .collect();
        t.sort();
        t
    };
    let a = text(executed(db, q));
    assert_eq!(a, text(pulled(db, q)), "execute and cursor agree");
    a
}

const NAMES: &[&str] = &[":alice", ":bob", ":carol", ":dave"];

/// `:alice`'s friend is the keyword `:bob`, `:carol`'s the keyword `:dave`.
/// `:bob` has a name, `:dave` an alias.
const PEOPLE: &str = "(transact [[:alice :friend :bob] [:bob :name \"B\"] \
     [:dave :alias \"D\"] [:carol :friend :dave] [:x :blocked true]])";

fn people(db: &Minigraf) {
    db.execute(PEOPLE).unwrap();
}

/// Each query with the rows it returns: the outer `?f` is a keyword, the
/// branch's `?f` an entity (a ref). `?f` keeps the outer row's cell.
const CASES: &[(&str, &[&str])] = &[
    // The unseeded path.
    (
        "[:find ?e ?n :where [?e :friend ?f] (or [?f :name ?n] [?f :alias ?n])]",
        &[":alice B", ":carol D"],
    ),
    (
        "[:find ?e ?f :where [?e :friend ?f] (or [?f :name _] [?f :alias _])]",
        &[":alice kw:bob", ":carol kw:dave"],
    ),
    (
        "[:find ?e :where [?e :friend ?f] (or-join [?f] [?f :name _] [?f :alias _])]",
        &[":alice", ":carol"],
    ),
    // The seeded path: a branch holds a `not`.
    (
        "[:find ?e ?n :where [?e :friend ?f] \
         (or (and [?f :name ?n] (not [:x :blocked false])) [?f :alias ?n])]",
        &[":alice B", ":carol D"],
    ),
    // The per-row `or-join` path: a branch's expression reads `?f`.
    (
        "[:find ?e :where [?e :friend ?f] \
         (or-join [?f] (and [?f :name ?n] [(!= ?f \"none\")]) [?f :alias _])]",
        &[":alice", ":carol"],
    ),
];

fn check(db: &Minigraf) {
    for (q, want) in CASES {
        assert_eq!(rows(db, q, NAMES), want.to_vec(), "{q}");
    }
}

#[test]
fn branch_ref_meets_outer_keyword() {
    let db = Minigraf::in_memory().unwrap();
    people(&db);
    check(&db);
}

#[test]
fn branch_ref_meets_outer_keyword_after_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("or.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        people(&db);
        db.checkpoint().unwrap();
    }
    let db = OpenOptions::new().path(&path).open().unwrap();
    check(&db);
}

#[test]
fn branch_float_meets_outer_float_by_ieee_equality() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :x 0.0] [:b :x 0.00000000000000000001] [:g :y -0.0]])")
        .unwrap();
    for q in [
        "[:find ?e :where [?e :x ?f] (or [?g :y ?f] [?g :z ?f])]",
        "[:find ?e :where [?e :x ?f] (or-join [?f] [?g :y ?f] [?g :z ?f])]",
    ] {
        assert_eq!(rows(&db, q, &[":a", ":b"]), vec![":a"], "{q}");
    }
}
