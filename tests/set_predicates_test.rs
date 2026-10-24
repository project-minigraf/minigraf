//! Set-membership predicates (#316): `in?`, `not-in?`, `any-in?`, `all-in?`,
//! `none-in?` as filters and as Boolean bindings, over literal, stored and
//! bind-slot operands.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{BindValue, Minigraf, QueryResult, Value, ValueSet};
use uuid::Uuid;

fn rows(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    match db.execute(q).unwrap() {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    }
}

fn set(values: Vec<Value>) -> Value {
    Value::Set(ValueSet::new(values).unwrap())
}

/// A database with one entity, for ground expressions.
fn one() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:one :t/k 1]])").unwrap();
    db
}

/// `expr` as a filter: whether the one row is kept.
fn keeps(db: &Minigraf, expr: &str) -> bool {
    let q = format!("(query [:find ?x :where [?x :t/k _] [{expr}]])");
    match rows(db, &q).len() {
        0 => false,
        1 => true,
        _ => panic!("one entity, at most one row"),
    }
}

/// `expr` as a binding: the bound Boolean, or `None` when no row is bound.
fn binds(db: &Minigraf, expr: &str) -> Option<bool> {
    let q = format!("(query [:find ?b :where [?x :t/k _] [{expr} ?b]])");
    let r = rows(db, &q);
    match r.len() {
        0 => None,
        1 => match &r[0][0] {
            Value::Boolean(b) => Some(*b),
            _ => panic!("a set predicate binds a Boolean"),
        },
        _ => panic!("one entity, at most one row"),
    }
}

/// Filter and binding forms agree, and both equal `expected`.
fn check(db: &Minigraf, expr: &str, expected: bool) {
    assert_eq!(keeps(db, expr), expected, "filter form of {expr}");
    assert_eq!(binds(db, expr), Some(expected), "binding form of {expr}");
}

/// A type mismatch: the filter drops the row, the binding binds nothing.
fn mismatch(db: &Minigraf, expr: &str) {
    assert!(!keeps(db, expr), "filter form of {expr} drops the row");
    assert_eq!(
        binds(db, expr),
        None,
        "binding form of {expr} binds nothing"
    );
}

/// Refs come from a stored attribute: `#uuid` is not an expression argument.
#[test]
fn in_and_not_in_per_scalar_type() {
    let db = one();
    let cases = [
        ("\"a\"", "#{\"a\" \"b\"}", "#{\"b\" \"c\"}"),
        ("1", "#{1 2}", "#{2 3}"),
        ("1.5", "#{1.5 2.5}", "#{2.5}"),
        ("true", "#{true}", "#{false}"),
        ("nil", "#{nil 1}", "#{1}"),
        (":k/a", "#{:k/a :k/b}", "#{:k/b}"),
    ];
    for (x, with, without) in cases {
        check(&db, &format!("(in? {x} {with})"), true);
        check(&db, &format!("(in? {x} {without})"), false);
        check(&db, &format!("(not-in? {x} {with})"), false);
        check(&db, &format!("(not-in? {x} {without})"), true);
    }
}

#[test]
fn membership_is_set_equality() {
    let db = one();
    // -0.0 and 0.0 are one element.
    check(&db, "(in? 0.0 #{-0.0})", true);
    check(&db, "(in? -0.0 #{0.0})", true);
    // An integer is not a float, a string is not a keyword.
    check(&db, "(in? 1 #{1.0})", false);
    check(&db, "(in? \"k/a\" #{:k/a})", false);
    check(&db, "(any-in? #{0.0} #{-0.0 5})", true);
    check(&db, "(all-in? #{-0.0} #{0.0})", true);
}

#[test]
fn nan_is_one_element() {
    let db = one();
    let p = db
        .prepare("(query [:find ?x :where [?x :t/k _] [(in? $x $s)]])")
        .unwrap();
    let run = |x: f64, s: Vec<Value>| match p
        .execute(&[
            ("x", BindValue::Val(Value::Float(x))),
            ("s", BindValue::Val(set(s))),
        ])
        .unwrap()
    {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    };
    assert_eq!(
        run(f64::NAN, vec![Value::Float(f64::NAN)]),
        1,
        "NaN in #{{NaN}}"
    );
    assert_eq!(
        run(f64::NAN, vec![Value::Float(1.0)]),
        0,
        "NaN not in #{{1.0}}"
    );
    assert_eq!(
        run(1.0, vec![Value::Float(f64::NAN)]),
        0,
        "1.0 not in #{{NaN}}"
    );
}

#[test]
fn keyword_is_not_a_ref_to_its_entity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :person/name \"Alice\"] [:erin :user/friends #{:alice}]])")
        .unwrap();
    // ?a is a ref to the entity :alice names; the set holds the keyword.
    let q = |op: &str| {
        format!(
            "(query [:find ?a :where [?a :person/name \"Alice\"] [:erin :user/friends ?fs] [({op} ?a ?fs)]])"
        )
    };
    assert_eq!(rows(&db, &q("in?")).len(), 0, "ref not in a keyword set");
    assert_eq!(
        rows(&db, &q("not-in?")).len(),
        1,
        "ref not-in a keyword set"
    );
    // A ref element matches the ref.
    let alice_u = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":alice");
    let other = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    let refq = |op: &str, s: &str| {
        format!("(query [:find ?a :where [?a :person/name \"Alice\"] [({op} ?a {s})]])")
    };
    let with = format!("#{{#uuid \"{alice_u}\" #uuid \"{other}\"}}");
    let without = format!("#{{#uuid \"{other}\"}}");
    assert_eq!(rows(&db, &refq("in?", &with)).len(), 1, "ref in a ref set");
    assert_eq!(
        rows(&db, &refq("in?", &without)).len(),
        0,
        "ref not in another ref set"
    );
    assert_eq!(
        rows(&db, &refq("not-in?", &without)).len(),
        1,
        "ref not-in another ref set"
    );
    let alice = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":alice");
    let p = db
        .prepare("(query [:find ?fs :where [:erin :user/friends ?fs] [(in? $a ?fs)]])")
        .unwrap();
    match p.execute(&[("a", BindValue::Entity(alice))]).unwrap() {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 0, "entity bind is a ref, not the keyword")
        }
        _ => panic!("expected query results"),
    }
}

#[test]
fn set_against_set() {
    let db = one();
    check(&db, "(any-in? #{1 2} #{2 3})", true);
    check(&db, "(any-in? #{1 2} #{3 4})", false);
    check(&db, "(all-in? #{1 2} #{1 2 3})", true);
    check(&db, "(all-in? #{1 2 3} #{1 2})", false);
    check(&db, "(all-in? #{1 4} #{1 2 3})", false);
    check(&db, "(all-in? #{1 2} #{2 1})", true);
    check(&db, "(none-in? #{1 2} #{3 4})", true);
    check(&db, "(none-in? #{1 2} #{2 3})", false);
    // The larger set first.
    check(&db, "(any-in? #{1 2 3 4} #{4})", true);
    check(&db, "(none-in? #{1 2 3 4} #{5})", true);
}

#[test]
fn every_empty_set_case() {
    let db = one();
    check(&db, "(in? 1 #{})", false);
    check(&db, "(not-in? 1 #{})", true);
    check(&db, "(all-in? #{} #{1})", true);
    check(&db, "(all-in? #{} #{})", true);
    check(&db, "(none-in? #{} #{1})", true);
    check(&db, "(none-in? #{} #{})", true);
    check(&db, "(any-in? #{} #{1})", false);
    check(&db, "(any-in? #{} #{})", false);
    check(&db, "(any-in? #{1} #{})", false);
    check(&db, "(all-in? #{1} #{})", false);
    check(&db, "(none-in? #{1} #{})", true);
}

#[test]
fn wrong_operand_shapes_are_type_mismatches() {
    let db = one();
    // A set as in?'s element.
    mismatch(&db, "(in? #{1} #{1 2})");
    mismatch(&db, "(not-in? #{1} #{1 2})");
    // A scalar where a set is needed: not-in? is not true then.
    mismatch(&db, "(in? 1 1)");
    mismatch(&db, "(not-in? 1 2)");
    mismatch(&db, "(not-in? 1 \"abc\")");
    for op in ["any-in?", "all-in?", "none-in?"] {
        mismatch(&db, &format!("({op} 1 #{{1}})"));
        mismatch(&db, &format!("({op} #{{1}} 1)"));
        mismatch(&db, &format!("({op} nil nil)"));
    }
}

#[test]
fn stored_operands_pending_and_committed() {
    let path = tempfile::tempdir().unwrap();
    let file = path.path().join("p.graph");
    let db = Minigraf::open(&file).unwrap();
    db.execute(
        "(transact [[:alice :user/role :role/admin] [:alice :user/scopes #{:scope/read :scope/write}]
                    [:bob :user/role :role/guest] [:bob :user/scopes #{}]
                    [:carol :user/role :role/owner] [:carol :user/scopes #{:scope/read}]])",
    )
    .unwrap();
    let check_all = |db: &Minigraf, phase: &str| {
        let staff = rows(
            db,
            "(query [:find ?e :where [?e :user/role ?r] [(in? ?r #{:role/admin :role/owner})]])",
        );
        assert_eq!(staff.len(), 2, "{phase}: admin and owner");
        let readers = rows(
            db,
            "(query [:find ?e :where [?e :user/scopes ?s] [(any-in? ?s #{:scope/read})]])",
        );
        assert_eq!(readers.len(), 2, "{phase}: alice and carol read");
        let within = rows(
            db,
            "(query [:find ?e :where [?e :user/scopes ?s] [(all-in? ?s #{:scope/read})]])",
        );
        assert_eq!(within.len(), 2, "{phase}: bob's empty set and carol's");
        let none = rows(
            db,
            "(query [:find ?e ?n :where [?e :user/scopes ?s] [(none-in? ?s #{:scope/write}) ?n]])",
        );
        assert_eq!(none.len(), 3, "{phase}: a Boolean per entity");
        let trues = none.iter().filter(|r| r[1] == Value::Boolean(true)).count();
        assert_eq!(trues, 2, "{phase}: bob and carol write nothing");
        // A role is a keyword, not a set: all-in? over it binds nothing.
        let bad = rows(
            db,
            "(query [:find ?e :where [?e :user/role ?r] [(all-in? ?r #{:role/admin}) ?b]])",
        );
        assert_eq!(bad.len(), 0, "{phase}: scalar where a set is needed");
    };
    check_all(&db, "pending");
    db.checkpoint().unwrap();
    check_all(&db, "committed");
    drop(db);
    let db = Minigraf::open(&file).unwrap();
    check_all(&db, "reopened");
}

#[test]
fn equality_between_sets() {
    let db = one();
    check(&db, "(= #{1 2} #{2 1})", true);
    check(&db, "(= #{1 2} #{1})", false);
    check(&db, "(!= #{1 2} #{1})", true);
    check(&db, "(= #{1} 1)", false);
}

#[test]
fn contains_is_still_string_only() {
    let db = one();
    check(&db, "(contains? \"abc\" \"b\")", true);
    mismatch(&db, "(contains? #{\"b\"} \"b\")");
    mismatch(&db, "(contains? \"b\" #{\"b\"})");
}

#[test]
fn wrong_arity_is_prs_065() {
    let db = one();
    for op in ["in?", "not-in?", "any-in?", "all-in?", "none-in?"] {
        for args in ["1", "1 #{1} #{2}"] {
            let q = format!("(query [:find ?x :where [?x :t/k _] [({op} {args})]])");
            let err = db.execute(&q).err().expect("arity error");
            assert_eq!(err.code(), "PRS-065", "{op} with {args}");
        }
    }
}

#[test]
fn names_cannot_be_registered() {
    let db = one();
    for op in ["in?", "not-in?", "any-in?", "all-in?", "none-in?"] {
        assert!(
            db.register_predicate(op, |_| true).is_err(),
            "{op} is built in"
        );
    }
}
