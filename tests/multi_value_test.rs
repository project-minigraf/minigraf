//! Same-transaction multi-valued facts (#371, #287): every query path must
//! return every value, before checkpoint, after WAL replay, and after
//! checkpoint + reopen.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult, Value};
use std::collections::BTreeSet;

mod common;
use common::{CrashTx, run_crashing_child};

fn rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn kw(v: &Value) -> String {
    match v {
        Value::Keyword(k) => k.clone(),
        _ => panic!("expected keyword value"),
    }
}

/// Values of `attr` on the entity `ent` through EAVT (entity bound), AEVT
/// (attribute bound, joined to a marker attribute) and a full scan.
fn three_paths(db: &Minigraf, ent: &str, attr: &str, marker: &str) -> [BTreeSet<String>; 3] {
    let eavt = rows(
        db.execute(&format!("(query [:find ?v :where [{ent} {attr} ?v]])"))
            .unwrap(),
    )
    .iter()
    .map(|r| kw(&r[0]))
    .collect();
    let aevt = rows(
        db.execute(&format!(
            r#"(query [:find ?v :where [?e {attr} ?v] [?e :note "{marker}"]])"#
        ))
        .unwrap(),
    )
    .iter()
    .map(|r| kw(&r[0]))
    .collect();
    let scan = rows(
        db.execute(&format!(
            r#"(query [:find ?a ?v :where [?e ?a ?v] [?e :note "{marker}"]])"#
        ))
        .unwrap(),
    )
    .iter()
    .filter(|r| matches!(&r[0], Value::Keyword(a) if a == attr))
    .map(|r| kw(&r[1]))
    .collect();
    [eavt, aevt, scan]
}

fn setup_statements(i: usize) -> Vec<String> {
    let mut facts = vec![
        format!("[:t/x{i} :kind :k/a]"),
        format!("[:t/x{i} :kind :k/b]"),
        format!(r#"[:t/x{i} :note "two{i}"]"#),
    ];
    for f in 0..30 {
        facts.push(format!("[:t/f{i}-{f} :kind :k/c]"));
        facts.push(format!(r#"[:t/f{i}-{f} :note "f"]"#));
    }
    vec![format!("(transact [{}])", facts.join(" "))]
}

fn both() -> BTreeSet<String> {
    [":k/a", ":k/b"].iter().map(|s| s.to_string()).collect()
}

#[test]
fn multi_value_visible_on_every_path_before_checkpoint() {
    let db = Minigraf::in_memory().unwrap();
    for s in setup_statements(0) {
        db.execute(&s).unwrap();
    }
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x0", ":kind", "two0"))
    {
        assert_eq!(
            got,
            both(),
            "{name} path must return both values before checkpoint"
        );
    }
}

#[test]
fn multi_value_visible_on_every_path_after_checkpoint_and_reopen() {
    // Several graph shapes: the issue saw EAVT and AEVT disagree depending on content.
    for i in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        {
            let db = Minigraf::open(&path).unwrap();
            for s in setup_statements(i) {
                db.execute(&s).unwrap();
            }
            db.checkpoint().unwrap();
        }
        let db = Minigraf::open(&path).unwrap();
        let x = format!(":t/x{i}");
        let marker = format!("two{i}");
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, &x, ":kind", &marker))
        {
            assert_eq!(
                got,
                both(),
                "{name} path must return both values after reopen"
            );
        }
    }
}

#[test]
fn multi_value_visible_on_every_path_after_wal_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.graph");
    let stmts = setup_statements(0);
    let refs: Vec<&str> = stmts.iter().map(String::as_str).collect();
    run_crashing_child(&path, 1_000_000, &refs, CrashTx::Implicit);
    let db = Minigraf::open(&path).unwrap();
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x0", ":kind", "two0"))
    {
        assert_eq!(
            got,
            both(),
            "{name} path must return both values after WAL replay"
        );
    }
}

#[test]
fn batched_retract_of_both_values_hides_both_on_every_path() {
    for i in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        {
            let db = Minigraf::open(&path).unwrap();
            for s in setup_statements(i) {
                db.execute(&s).unwrap();
            }
            db.checkpoint().unwrap();
        }
        {
            let db = Minigraf::open(&path).unwrap();
            db.execute(&format!(
                "(retract [[:t/x{i} :kind :k/a] [:t/x{i} :kind :k/b]])"
            ))
            .unwrap();
            db.checkpoint().unwrap();
        }
        let db = Minigraf::open(&path).unwrap();
        let x = format!(":t/x{i}");
        let marker = format!("two{i}");
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, &x, ":kind", &marker))
        {
            assert!(
                got.is_empty(),
                "{name} path must hide both retracted values"
            );
        }
    }
}
