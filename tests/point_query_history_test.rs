//! #323 — bound-entity point queries must return exactly what a full scan returns,
//! while only reading the queried attributes' history.
//!
//! Oracle: the same query with `:as-of 1000000000` goes through the full-scan
//! `get_facts_as_of` path (never the selective path) and, since every tx_count in
//! these tests is far below that bound, must produce identical rows.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult};

const ORACLE_AS_OF: &str = ":as-of 1000000000";

fn rows(r: QueryResult) -> Vec<String> {
    match r {
        QueryResult::QueryResults { results, .. } => {
            let mut v: Vec<String> = results.iter().map(|row| format!("{row:?}")).collect();
            v.sort();
            v
        }
        _ => panic!("expected QueryResults"),
    }
}

/// Run `(query [:find <find> <opts> :where <body>])` via the selective path and via
/// the full-scan oracle; assert equal; return the selective rows.
fn same_as_full_scan(db: &Minigraf, find: &str, opts: &str, body: &str) -> Vec<String> {
    let selective = rows(
        db.execute(&format!("(query [:find {find} {opts} :where {body}])"))
            .unwrap(),
    );
    let oracle = rows(
        db.execute(&format!(
            "(query [:find {find} {ORACLE_AS_OF} {opts} :where {body}])"
        ))
        .unwrap(),
    );
    assert_eq!(selective, oracle, "selective path diverged from full scan");
    selective
}

fn churn(db: &Minigraf, depth: usize) {
    db.execute("(transact [[:e/hot :other \"o\"]])").unwrap();
    db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h0\"]])")
        .unwrap();
    for i in 1..depth {
        db.execute(&format!("(retract [[:e/hot :hash \"h{}\"]])", i - 1))
            .unwrap();
        db.execute(&format!(
            "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [[:e/hot :hash \"h{i}\"]])"
        ))
        .unwrap();
    }
}

#[test]
fn churned_attribute_live_value_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 50);
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
    assert_eq!(r.len(), 1, "exactly one live value");
    assert!(r[0].contains("h49"));
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :other ?v]");
    assert_eq!(r.len(), 1);
}

#[test]
fn churned_attribute_live_value_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        churn(&db, 30);
        db.checkpoint().unwrap();
        // Pending on top of committed: more churn after the checkpoint,
        // including reasserting a previously retracted value.
        db.execute("(retract [[:e/hot :hash \"h29\"]])").unwrap();
        db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h3\"]])")
            .unwrap();
        let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
        assert_eq!(r.len(), 1);
        assert!(r[0].contains("\"h3\""));
    }
    let db = Minigraf::open(&path).unwrap();
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("\"h3\""));
}

#[test]
fn two_attributes_on_one_entity() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 10);
    let r = same_as_full_scan(&db, "?h ?o", "", "[:e/hot :hash ?h] [:e/hot :other ?o]");
    assert_eq!(r.len(), 1);
}

#[test]
fn five_attributes_on_one_entity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:e/x :a 1] [:e/x :b 2] [:e/x :c 3] [:e/x :d 4] [:e/x :f 5]])")
        .unwrap();
    let r = same_as_full_scan(
        &db,
        "?a ?b ?c ?d ?f",
        "",
        "[:e/x :a ?a] [:e/x :b ?b] [:e/x :c ?c] [:e/x :d ?d] [:e/x :f ?f]",
    );
    assert_eq!(r.len(), 1);
}

#[test]
fn bound_entity_variable_attribute_sees_all() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    let r = same_as_full_scan(&db, "?a ?v", "", "[:e/hot ?a ?v]");
    assert_eq!(r.len(), 2, ":hash h4 and :other o");
    // Mixed: one narrowed pattern + one variable-attribute pattern on the same entity.
    let r = same_as_full_scan(&db, "?a ?h", "", "[:e/hot :hash ?h] [:e/hot ?a _]");
    assert_eq!(r.len(), 2);
}

#[test]
fn bound_entity_pseudo_attribute_matches_full_scan() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    same_as_full_scan(
        &db,
        "?v ?vf",
        ":any-valid-time",
        "[:e/hot :hash ?v] [:e/hot :db/valid-from ?vf]",
    );
}

#[test]
fn not_and_or_on_narrowed_entity() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    db.execute("(transact [[:e/hot :flag true]])").unwrap();
    same_as_full_scan(
        &db,
        "?v",
        "",
        "[:e/hot :hash ?v] (not [:e/hot :flag false])",
    );
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v] (not [:e/hot :flag true])");
    assert!(
        r.is_empty(),
        "not must see :flag although only :hash is projected"
    );
    same_as_full_scan(&db, "?v", "", "(or [:e/hot :hash ?v] [:e/hot :other ?v])");
}

#[test]
fn prefix_sibling_attribute_not_leaked() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("p.graph")).unwrap();
    db.execute("(transact [[:e/x :hash \"a\"] [:e/x :hash2 \"b\"] [:e/x :hash/sub \"c\"]])")
        .unwrap();
    db.checkpoint().unwrap();
    db.execute("(transact [[:e/x :hashx \"d\"]])").unwrap();
    let r = same_as_full_scan(&db, "?v", "", "[:e/x :hash ?v]");
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("\"a\""));
}

#[test]
fn entity_and_attribute_patterns_mixed() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    db.execute("(transact [[:e/a :hash \"z\"]])").unwrap();
    same_as_full_scan(&db, "?e ?v ?o", "", "[?e :hash ?v] [:e/hot :other ?o]");
}

#[test]
fn valid_at_past_on_churned_attribute() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact {:valid-from \"2020-01-01T00:00:00Z\" :valid-to \"2021-01-01T00:00:00Z\"} [[:e/v :hash \"old\"]])",
    )
    .unwrap();
    db.execute("(transact {:valid-from \"2021-01-01T00:00:00Z\"} [[:e/v :hash \"new\"]])")
        .unwrap();
    db.execute("(transact [[:e/v :other 1]])").unwrap();
    let r = same_as_full_scan(
        &db,
        "?v",
        ":valid-at \"2020-06-01T00:00:00Z\"",
        "[:e/v :hash ?v]",
    );
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("old"));
}

/// One WriteTransaction writing the same (entity, attribute) in two valid-time
/// windows gives both facts the same tx_count. The selective path used to dedup on
/// (entity, attribute, tx_count, asserted) and silently drop one window's row; it
/// must match a full scan (#323 review).
#[test]
fn same_transaction_multi_window_matches_full_scan() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact {:valid-from "2020-01-01T00:00:00Z" :valid-to "2021-01-01T00:00:00Z"} [[:e/w :x 1]])"#)
        .unwrap();
    tx.execute(r#"(transact {:valid-from "2021-01-01T00:00:00Z"} [[:e/w :x 2]])"#)
        .unwrap();
    tx.commit().unwrap();
    let r = same_as_full_scan(&db, "?v", ":any-valid-time", "[:e/w :x ?v]");
    assert_eq!(r.len(), 2, "both windows' values must be returned");
}
