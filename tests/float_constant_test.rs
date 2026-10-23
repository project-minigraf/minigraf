//! A float constant in a pattern matches equal floats only, by IEEE `==`,
//! as joins and predicates compare them (#706). Before, it matched every
//! float within `f64::EPSILON`, so `0.0` matched `1e-17`.

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

/// `1e-17`, `0.0` and `-0.0`: the first is within `f64::EPSILON` of the
/// others but equal to neither.
const SMALL_FLOATS: &str =
    "(transact [[:tiny :m/x 0.00000000000000001] [:zero :m/x 0.0] [:neg :m/x -0.0]])";

/// The keywords naming the entities in `r`'s first column, sorted.
fn names_of(r: &[Vec<Value>]) -> Vec<&'static str> {
    let mut n: Vec<&str> = r
        .iter()
        .map(|row| {
            [":tiny", ":zero", ":neg"]
                .into_iter()
                .find(|k| matches!(&row[0], Value::Ref(u) if *u == named(k)))
                .unwrap_or("?")
        })
        .collect();
    n.sort();
    n
}

/// A float constant written in a pattern, bound to a `$slot`, written in a
/// rule or `not` body, or compared by a predicate matches equal floats only.
fn assert_float_constants_exact(db: &Minigraf) {
    db.execute("(rule [(zeroish ?e) [?e :m/x 0.0]])").unwrap();
    db.execute("(rule [(xval ?e ?v) [?e :m/x ?v]])").unwrap();
    for q in [
        "[:find ?e :where [?e :m/x 0.0]]",
        "[:find ?e :where [?e :m/x -0.0]]",
        "[:find ?e :where [?e :m/x ?v] [(= ?v 0.0)]]",
        "[:find ?e :where (zeroish ?e)]",
    ] {
        assert_eq!(names_of(&query(db, q)), [":neg", ":zero"], "{q}");
    }
    // A rule call's literal joins by `Value`'s hash on v2.x, which tells
    // `-0.0` from `0.0` (v3.0.0 pairs them, #545); only `1e-17` is checked.
    let r = query(db, "[:find ?e :where (xval ?e 0.0)]");
    assert_eq!(names_of(&r), [":zero"], "a rule call's float argument");
    let r = query(db, "[:find ?e :where (xval ?e 0.00000000000000001)]");
    assert_eq!(names_of(&r), [":tiny"], "a rule call's float argument");
    for q in [
        "[:find ?e :where [?e :m/x 0.00000000000000001]]",
        "[:find ?e :where [?e :m/x _] (not [?e :m/x 0.0])]",
        "[:find ?e :where [?e :m/x ?v] (not [?e :m/x 0.0] [(< ?v 1.0)])]",
    ] {
        assert_eq!(names_of(&query(db, q)), [":tiny"], "{q}");
    }
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
