//! `collect-set` (#316 spec §7): `[(collect-set ?e :a) ?s]` binds the set of
//! the values of `[?e :a ?v]` under the query's view, `#{}` when there are
//! none.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{BindValue, Minigraf, OpenOptions, QueryResult, Value, ValueSet};

fn rows(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    match db.execute(q).unwrap() {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    }
}

fn set(values: Vec<Value>) -> Value {
    Value::Set(ValueSet::new(values).unwrap())
}

fn kw(k: &str) -> Value {
    Value::Keyword(k.to_string())
}

/// The set `q` binds for its one row.
fn one_set(db: &Minigraf, q: &str) -> Value {
    let r = rows(db, q);
    assert_eq!(r.len(), 1, "one row");
    r[0][0].clone()
}

fn code(db: &Minigraf, q: &str) -> String {
    db.execute(q).err().expect("an error").code().to_string()
}

fn scopes() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:alice :user/name "alice"] [:alice :user/scope :scope/read]
                      [:alice :user/scope :scope/write] [:bob :user/name "bob"]])"#,
    )
    .unwrap();
    db
}

#[test]
fn values_of_the_attribute() {
    let db = scopes();
    let s = one_set(
        &db,
        r#"(query [:find ?s :where [?e :user/name "alice"] [(collect-set ?e :user/scope) ?s]])"#,
    );
    assert!(
        s == set(vec![kw(":scope/read"), kw(":scope/write")]),
        "alice's scopes"
    );
}

#[test]
fn missing_attribute_gives_empty_set_and_keeps_the_row() {
    let db = scopes();
    let s = one_set(
        &db,
        r#"(query [:find ?s :where [?e :user/name "bob"] [(collect-set ?e :user/scope) ?s]])"#,
    );
    assert!(s == set(vec![]), "bob has no scope");
    let r = rows(
        &db,
        "(query [:find ?n ?s :where [?e :user/name ?n] [(collect-set ?e :user/scope) ?s]])",
    );
    assert_eq!(r.len(), 2, "both people are kept");
}

#[test]
fn one_element_per_value() {
    let db = Minigraf::in_memory().unwrap();
    // The same triple in two valid windows, and values equal as set
    // elements: `-0.0` and `0.0` are one, `1` and `1.0` two.
    db.execute(r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2021-01-01T00:00:00Z"} [[:a :v 1]])"#)
        .unwrap();
    db.execute(r#"(transact {:valid-from "2022-01-01T00:00:00Z" :valid-to "2023-01-01T00:00:00Z"} [[:a :v 1]])"#)
        .unwrap();
    db.execute("(transact [[:a :name \"a\"] [:b :name \"b\"] [:b :v 0.0] [:b :v 1.0]])")
        .unwrap();
    db.execute("(transact [[:b :v -0.0]])").unwrap();
    let a = one_set(
        &db,
        r#"(query [:find ?s :any-valid-time :where [?e :name "a"] [(collect-set ?e :v) ?s]])"#,
    );
    assert!(
        a == set(vec![Value::Integer(1)]),
        "one element for two windows"
    );
    let b = one_set(
        &db,
        r#"(query [:find ?s :where [?e :name "b"] [(collect-set ?e :v) ?s]])"#,
    );
    match b {
        Value::Set(s) => assert_eq!(s.len(), 2, "0.0 and -0.0 one element, 1.0 another"),
        _ => panic!("a set"),
    }
}

#[test]
fn retractions_apply() {
    let db = scopes();
    db.execute("(retract [[:alice :user/scope :scope/write]])")
        .unwrap();
    let s = one_set(
        &db,
        r#"(query [:find ?s :where [?e :user/name "alice"] [(collect-set ?e :user/scope) ?s]])"#,
    );
    assert!(s == set(vec![kw(":scope/read")]), "write was retracted");
}

#[test]
fn as_of_and_valid_at() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact {:valid-from "2019-01-01T00:00:00Z"}
                     [[:alice :user/name "alice"] [:alice :user/scope :scope/read]
                      [:alice :user/scope :scope/write]])"#,
    )
    .unwrap();
    db.execute("(transact [[:alice :user/scope :scope/admin]])")
        .unwrap();
    db.execute(r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2021-01-01T00:00:00Z"} [[:alice :user/scope :scope/old]])"#)
        .unwrap();
    let q = |opts: &str| {
        one_set(
            &db,
            &format!(
                r#"(query [:find ?s {opts} :where [?e :user/name "alice"] [(collect-set ?e :user/scope) ?s]])"#
            ),
        )
    };
    assert!(
        q(":as-of 1") == set(vec![kw(":scope/read"), kw(":scope/write")]),
        "as of the first transaction"
    );
    assert!(
        q("")
            == set(vec![
                kw(":scope/read"),
                kw(":scope/write"),
                kw(":scope/admin")
            ]),
        "now: the old scope is no longer valid"
    );
    assert!(
        q(r#":valid-at "2020-06-01T00:00:00Z""#)
            == set(vec![
                kw(":scope/read"),
                kw(":scope/write"),
                kw(":scope/old")
            ]),
        "the old scope was valid in 2020, the admin scope not yet"
    );
}

#[test]
fn pending_committed_and_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("c.graph");
    let db = Minigraf::open(&file).unwrap();
    db.execute(r#"(transact [[:alice :user/name "alice"] [:alice :user/scope :scope/read]])"#)
        .unwrap();
    let q =
        r#"(query [:find ?s :where [?e :user/name "alice"] [(collect-set ?e :user/scope) ?s]])"#;
    assert!(one_set(&db, q) == set(vec![kw(":scope/read")]), "pending");
    db.checkpoint().unwrap();
    assert!(one_set(&db, q) == set(vec![kw(":scope/read")]), "committed");
    db.execute("(transact [[:alice :user/scope :scope/write]])")
        .unwrap();
    db.execute("(retract [[:alice :user/scope :scope/read]])")
        .unwrap();
    assert!(
        one_set(&db, q) == set(vec![kw(":scope/write")]),
        "committed fact retracted while pending, pending fact added"
    );
    db.checkpoint().unwrap();
    drop(db);
    let db = Minigraf::open(&file).unwrap();
    assert!(one_set(&db, q) == set(vec![kw(":scope/write")]), "reopened");
}

#[test]
fn refs_and_long_strings_are_collected() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("r.graph")).unwrap();
    let long = "x".repeat(100);
    db.execute(&format!(
        r#"(transact [[:alice :name "alice"] [:bob :name "bob"]
                      [:alice :bio "{long}"] [:alice :bio "short"]])"#
    ))
    .unwrap();
    let bob = match &rows(&db, r#"(query [:find ?b :where [?b :name "bob"]])"#)[0][0] {
        Value::Ref(u) => *u,
        _ => panic!("an entity"),
    };
    db.execute(&format!(
        r#"(transact [[:alice :knows #uuid "{}"]])"#,
        bob.as_hyphenated()
    ))
    .unwrap();
    db.checkpoint().unwrap();
    let q = |attr: &str| {
        one_set(
            &db,
            &format!(
                r#"(query [:find ?s :where [?e :name "alice"] [(collect-set ?e {attr}) ?s]])"#
            ),
        )
    };
    assert!(q(":knows") == set(vec![Value::Ref(bob)]), "a ref");
    assert!(
        q(":bio")
            == set(vec![
                Value::String(long.clone()),
                Value::String("short".into())
            ]),
        "a long string from a value page"
    );
}

#[test]
fn set_valued_attribute_is_qry_015() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :name \"alice\"] [:alice :user/scopes #{:scope/read}]])")
        .unwrap();
    assert_eq!(
        code(
            &db,
            "(query [:find ?s :where [?e :name _] [(collect-set ?e :user/scopes) ?s]])"
        ),
        "QRY-015"
    );
    // The pattern binds the stored set.
    let r = rows(&db, "(query [:find ?s :where [?e :user/scopes ?s]])");
    assert_eq!(r.len(), 1, "one stored set");
}

#[test]
fn unbound_entity_is_int_018() {
    let db = scopes();
    assert_eq!(
        code(
            &db,
            "(query [:find ?s :where [(collect-set ?e :user/scope) ?s]])"
        ),
        "INT-018"
    );
    assert_eq!(
        code(
            &db,
            "(query [:find ?s :where [(collect-set ?e :user/scope) ?s] [?e :user/name _]])"
        ),
        "INT-018",
        "bound by a later clause"
    );
}

#[test]
fn keyword_entities_and_non_entities() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:alice :friend :bob] [:alice :friend :nobody] [:alice :name "alice"]
                      [:bob :age 40] [:bob :age 41]])"#,
    )
    .unwrap();
    // `:friend` holds keywords: each names an entity, or none.
    let r = rows(
        &db,
        "(query [:find ?f ?s :where [?a :friend ?f] [(collect-set ?f :age) ?s]])",
    );
    assert_eq!(r.len(), 2, "both friends kept");
    for row in &r {
        let expected = match &row[0] {
            Value::Keyword(k) if k == ":bob" => set(vec![Value::Integer(40), Value::Integer(41)]),
            Value::Keyword(k) if k == ":nobody" => set(vec![]),
            _ => panic!("a friend keyword"),
        };
        assert!(row[1] == expected, "ages of the friend");
    }
    // A string or an integer is no entity.
    for var in ["?n", "?x"] {
        let q = format!(
            "(query [:find ?s :where [?e :name ?n] [(+ 1 2) ?x] [(collect-set {var} :age) ?s]])"
        );
        assert!(one_set(&db, &q) == set(vec![]), "{var} is no entity");
    }
}

#[test]
fn in_rule_bodies_not_bodies_and_or_branches() {
    let db = scopes();
    db.execute(
        "(rule [(writer ?e) [?e :user/name _] [(collect-set ?e :user/scope) ?s] [(in? :scope/write ?s)]])",
    )
    .unwrap();
    let r = rows(&db, "(query [:find ?e :where (writer ?e)])");
    assert_eq!(r.len(), 1, "alice writes");
    let r = rows(
        &db,
        "(query [:find ?n :where [?e :user/name ?n]
                 (not-join [?e] [(collect-set ?e :user/scope) ?s] [(any-in? ?s #{:scope/write})])])",
    );
    assert!(
        r.len() == 1 && r[0][0] == Value::String("bob".into()),
        "bob does not write"
    );
    // In `not`, the output variable is bound outside: an already-bound one.
    db.execute("(transact [[:none :empty #{}]])").unwrap();
    let r = rows(
        &db,
        "(query [:find ?n :where [?e :user/name ?n] [:none :empty ?s]
                 (not [(collect-set ?e :user/scope) ?s])])",
    );
    assert!(
        r.len() == 1 && r[0][0] == Value::String("alice".into()),
        "alice has scopes"
    );
    let r = rows(
        &db,
        "(query [:find ?n :where [?e :user/name ?n]
                 (or-join [?e] (and [(collect-set ?e :user/scope) ?s] [(= ?s #{})]) [?e :user/name \"alice\"])])",
    );
    assert_eq!(r.len(), 2, "bob by the first branch, alice by the second");
}

#[test]
fn output_variable_already_bound() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact [[:a :tags #{1 2}] [:a :tag 1] [:a :tag 2]
                    [:b :tags #{1}] [:b :tag 1] [:b :tag 3]])",
    )
    .unwrap();
    for q in [
        "(query [:find ?e :where [?e :tags ?s] [(collect-set ?e :tag) ?s]])",
        "(query [:find ?e :where [?e :tag _] [(collect-set ?e :tag) ?s] [?e :tags ?s]])",
    ] {
        let r = rows(&db, q);
        let mut entities: Vec<_> = r.iter().map(|row| row[0].clone()).collect();
        entities.dedup();
        assert_eq!(entities.len(), 1, "only :a's stored set equals its tags");
    }
}

#[test]
fn memory_limit_is_qry_012() {
    let db = OpenOptions::new()
        .query_memory_limit(2048)
        .open_memory()
        .unwrap();
    let facts: String = (0..200).map(|i| format!("[:a :v {i}]")).collect();
    db.execute(&format!("(transact [[:a :name \"a\"] {facts}])"))
        .unwrap();
    let err = db
        .execute("(query [:find ?s :where [?e :name _] [(collect-set ?e :v) ?s]])")
        .err()
        .expect("over the limit");
    assert_eq!(err.code(), "QRY-012");
    assert!(
        err.to_string().contains("CollectSet"),
        "the set is the holder"
    );
    // The same values streamed by a pattern hold nothing.
    assert_eq!(rows(&db, "(query [:find ?v :where [?e :v ?v]])").len(), 200);
}

#[test]
fn predicates_over_the_result_and_prepared_slots() {
    let db = scopes();
    let r = rows(
        &db,
        "(query [:find ?n ?w :where [?e :user/name ?n] [(collect-set ?e :user/scope) ?s]
                 [(all-in? ?s #{:scope/read :scope/write}) ?w]])",
    );
    assert_eq!(r.len(), 2, "every set is within, the empty one too");
    assert!(
        r.iter().all(|row| row[1] == Value::Boolean(true)),
        "all-in? is true for both"
    );
    let p = db
        .prepare(
            "(query [:find ?n :where [?e :user/name ?n] [(collect-set ?e :user/scope) ?s]
                     [(any-in? ?s $wanted)]])",
        )
        .unwrap();
    for (wanted, n) in [(vec![kw(":scope/write")], 1), (vec![kw(":scope/none")], 0)] {
        let w = BindValue::Val(set(wanted));
        match p.execute(&[("wanted", w)]).unwrap() {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), n, "prepared any-in?")
            }
            _ => panic!("expected query results"),
        }
    }
}

#[test]
fn parse_errors() {
    let db = scopes();
    let q = |clause: &str| format!("(query [:find ?e :where [?e :user/name _] {clause}])");
    for (clause, expected) in [
        ("[(collect-set ?e :user/scope)]", "PRS-122"),
        ("[(collect-set :alice :user/scope) ?s]", "PRS-123"),
        ("[(collect-set ?e \"user/scope\") ?s]", "PRS-123"),
        ("[(collect-set ?e ?a) ?s]", "PRS-123"),
        ("[(collect-set $e :user/scope) ?s]", "PRS-123"),
        ("[(collect-set ?e $a) ?s]", "PRS-123"),
        ("[(collect-set ?e) ?s]", "PRS-065"),
        ("[(collect-set ?e :a :b) ?s]", "PRS-065"),
        ("[(collect-set ?e :user/scope) ?s ?t]", "PRS-068"),
        ("[(collect-set ?e :user/scope) :s]", "PRS-069"),
        ("[(in? :x (collect-set ?e :user/scope)) ?b]", "PRS-067"),
        ("[(nil? (collect-set ?e))]", "PRS-067"),
    ] {
        assert_eq!(code(&db, &q(clause)), expected, "{clause}");
    }
}

#[test]
fn name_cannot_be_registered() {
    let db = Minigraf::in_memory().unwrap();
    assert!(
        db.register_predicate("collect-set", |_| true).is_err(),
        "collect-set is built in"
    );
}
