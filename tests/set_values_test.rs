//! Set values (#316): transact, query, retract and persist `#{...}`, through
//! the pending store, a checkpoint, a reopen, WAL replay, the fact log and
//! the log writer.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{FactFilter, LogWriter, Minigraf, OpenOptions, QueryResult, Value, ValueSet};

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

/// A string element long enough that its set goes to a value page.
fn long_text() -> String {
    "x".repeat(100)
}

const FACTS: &str = r#"(transact [[:alice :user/scopes #{:scope/write :scope/read :scope/read}]
                               [:bob :user/scopes #{}]
                               [:carol :user/tags #{1 1.5 "a" true nil -0.0 0.0}]
                               [:dave :user/notes #{"LONG" :k/a 7}]
                               [:erin :user/friends #{:alice :bob}]])"#;

fn load(db: &Minigraf) {
    db.execute(&FACTS.replace("LONG", &long_text())).unwrap();
}

/// Every check, on whatever state `db` is in.
fn check(db: &Minigraf, phase: &str) {
    let alice = rows(db, "(query [:find ?s :where [:alice :user/scopes ?s]])");
    assert_eq!(alice.len(), 1, "{phase}: one alice set");
    assert!(
        alice[0][0] == set(vec![kw(":scope/read"), kw(":scope/write")]),
        "{phase}: alice's set, duplicates removed"
    );

    let bob = rows(db, "(query [:find ?s :where [:bob :user/scopes ?s]])");
    assert!(bob[0][0] == set(vec![]), "{phase}: empty set");

    let carol = rows(db, "(query [:find ?s :where [:carol :user/tags ?s]])");
    let Value::Set(tags) = &carol[0][0] else {
        panic!("{phase}: carol's tags are a set");
    };
    assert_eq!(tags.len(), 6, "{phase}: 0.0 and -0.0 are one element");
    let zero = tags
        .iter()
        .find_map(|v| match v {
            Value::Float(f) if *f == 0.0 => Some(*f),
            _ => None,
        })
        .expect("a zero");
    assert!(zero.is_sign_negative(), "{phase}: -0.0 is kept");

    let dave = rows(db, "(query [:find ?s :where [:dave :user/notes ?s]])");
    assert!(
        dave[0][0]
            == set(vec![
                Value::String(long_text()),
                kw(":k/a"),
                Value::Integer(7)
            ]),
        "{phase}: long set with a long string element"
    );

    let erin = rows(db, "(query [:find ?s :where [:erin :user/friends ?s]])");
    assert!(
        erin[0][0] == set(vec![kw(":alice"), kw(":bob")]),
        "{phase}: keyword elements stay keywords"
    );

    // A set constant in a pattern matches the equal set, in any order.
    let by_value = rows(
        db,
        "(query [:find ?e :where [?e :user/scopes #{:scope/write :scope/read}]])",
    );
    assert_eq!(by_value.len(), 1, "{phase}: alice by her set");
    let empty = rows(db, "(query [:find ?e :where [?e :user/scopes #{}]])");
    assert_eq!(empty.len(), 1, "{phase}: bob by the empty set");
    let long = rows(
        db,
        &format!(
            r#"(query [:find ?e :where [?e :user/notes #{{7 :k/a "{}"}}]])"#,
            long_text()
        ),
    );
    assert_eq!(long.len(), 1, "{phase}: dave by his long set");
    let none = rows(
        db,
        "(query [:find ?e :where [?e :user/scopes #{:scope/read}]])",
    );
    assert!(none.is_empty(), "{phase}: a subset is not equal");
    let unknown = rows(
        db,
        "(query [:find ?e :where [?e :user/scopes #{:scope/never-stored}]])",
    );
    assert!(unknown.is_empty(), "{phase}: an element no fact holds");

    // A join on a set column, and `=` between sets.
    let same = rows(
        db,
        "(query [:find ?a ?b :where [?a :user/scopes ?s] [?b :user/scopes ?s]])",
    );
    assert_eq!(same.len(), 2, "{phase}: each set joins only itself");
    let eq = rows(
        db,
        "(query [:find ?e :where [?e :user/scopes ?s] [(= ?s #{:scope/read :scope/write})]])",
    );
    assert_eq!(eq.len(), 1, "{phase}: = between sets");

    // Grouping and counting by a set.
    let n = rows(
        db,
        "(query [:find (count-distinct ?s) :where [?e :user/scopes ?s]])",
    );
    assert!(n[0][0] == Value::Integer(2), "{phase}: two distinct sets");
}

#[test]
fn sets_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    load(&db);
    check(&db, "memory");
}

#[test]
fn sets_survive_checkpoint_reopen_and_wal_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sets.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        load(&db);
        check(&db, "pending");
        db.checkpoint().unwrap();
        check(&db, "committed");
        // The same sets again, after their ids and value page exist.
        db.execute(r#"(transact [[:frank :user/scopes #{:scope/read :scope/write}]])"#)
            .unwrap();
        let both = rows(
            &db,
            "(query [:find ?e :where [?e :user/scopes #{:scope/write :scope/read}]])",
        );
        assert_eq!(both.len(), 2, "committed and pending set join");
        db.execute(r#"(retract [[:frank :user/scopes #{:scope/read :scope/write}]])"#)
            .unwrap();
    }
    {
        let db = Minigraf::open(&path).unwrap();
        check(&db, "after WAL replay");
        db.checkpoint().unwrap();
    }
    let db = Minigraf::open(&path).unwrap();
    check(&db, "reopened");
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
}

#[test]
fn retracting_a_set_value_is_exact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :tags #{1 2}] [:a :tags #{3}]])")
        .unwrap();
    db.execute("(retract [[:a :tags #{2 1}]])").unwrap();
    let left = rows(&db, "(query [:find ?s :where [:a :tags ?s]])");
    assert_eq!(left.len(), 1, "one set left");
    assert!(left[0][0] == set(vec![Value::Integer(3)]), "#{{3}} is left");
    let before = rows(&db, "(query [:find ?s :as-of 1 :where [:a :tags ?s]])");
    assert_eq!(before.len(), 2, "both before the retraction");
}

#[test]
fn fact_log_and_log_writer_copy_sets() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("src.graph");
    let out = dir.path().join("out.graph");
    let src = Minigraf::open(&src_path).unwrap();
    load(&src);
    src.checkpoint().unwrap();
    let records: Vec<_> = src
        .fact_log(&FactFilter::new())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        records.iter().all(|r| matches!(r.value, Value::Set(_))),
        "every record holds a set"
    );
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    for rec in &records {
        w.append(rec).unwrap();
    }
    w.finish().unwrap();
    let copy = Minigraf::open(&out).unwrap();
    check(&copy, "log writer copy");
    assert!(copy.verify().unwrap().is_ok(), "copy verifies");
}

#[test]
fn a_large_transaction_writes_sets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.graph");
    let db =
        Minigraf::open_with_options(&path, OpenOptions::new().transaction_chunk_facts(2)).unwrap();
    load(&db);
    check(&db, "large transaction");
    drop(db);
    let db = Minigraf::open(&path).unwrap();
    check(&db, "large transaction reopened");
}

#[test]
fn an_oversized_set_is_wal_003() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("big.graph")).unwrap();
    // 4,068 bytes is the limit: 80 strings of 50 bytes count 53 each.
    let elements: Vec<String> = (0..80).map(|i| format!("\"{i:050}\"")).collect();
    let err = db
        .execute(&format!(
            "(transact [[:a :big #{{{}}}]])",
            elements.join(" ")
        ))
        .unwrap_err();
    assert_eq!(err.code(), "WAL-003");
    // 76 of them fit: 76 * 53 = 4,028.
    let elements: Vec<String> = (0..76).map(|i| format!("\"{i:050}\"")).collect();
    db.execute(&format!(
        "(transact [[:a :big #{{{}}}]])",
        elements.join(" ")
    ))
    .unwrap();
    db.checkpoint().unwrap();
    let back = rows(&db, "(query [:find ?s :where [:a :big ?s]])");
    let Value::Set(s) = &back[0][0] else {
        panic!("a set");
    };
    assert_eq!(s.len(), 76, "every element back");
}

#[test]
fn set_literal_errors() {
    let db = Minigraf::in_memory().unwrap();
    for (input, code) in [
        ("(transact [[:a :b #{#{1}}]])", "PRS-119"),
        ("(query [:find ?e :where [?e :b #{?x}]])", "PRS-120"),
        ("(transact [[:a :b #{1 [2]}]])", "PRS-120"),
        ("(transact [[:a :b #{1", "PRS-121"),
        ("(transact [[:a :b # {1}]])", "PRS-073"),
    ] {
        assert_eq!(db.execute(input).unwrap_err().code(), code, "{input}");
    }
}

#[test]
fn a_set_is_not_an_entity_or_attribute() {
    let db = Minigraf::in_memory().unwrap();
    assert!(db.execute("(transact [[#{1} :b 1]])").is_err(), "entity");
    assert!(db.execute("(transact [[:a #{1} 1]])").is_err(), "attribute");
}
