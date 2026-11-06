#![cfg(not(target_arch = "wasm32"))]
//! `OpenOptions::allow_overrides` (#869, spec #862 §1–§2): the embedder
//! decides which query keywords may override its limits; a keyword the list
//! leaves out is refused with `API-025` before the query runs.

use minigraf::{Minigraf, OpenOptions, Overridable, QueryResult};

const RULE: &str = "(rule [(reach ?x ?y) [?x :edge ?y]])";
const DERIVED: &str = "(query [:find ?x ?y :where (reach ?x ?y) :max-derived-facts 100])";
const RESULTS: &str = "(query [:find ?x ?y :where (reach ?x ?y) :max-results 100])";
const PLAIN: &str = "(query [:find ?x ?y :where (reach ?x ?y)])";

fn setup(opts: OpenOptions) -> Minigraf {
    let db = opts.open_memory().expect("open");
    db.execute("(transact [[:a :edge :b] [:b :edge :c]])")
        .expect("transact");
    db.execute(RULE).expect("rule");
    db
}

fn rows(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    }
}

fn refused(err: minigraf::MinigrafError, keyword: &str) {
    assert_eq!(err.code(), "API-025", "expected API-025");
    assert!(
        err.to_string().contains(keyword),
        "message names the keyword"
    );
}

#[test]
fn defaults_allow_both_keywords() {
    let db = setup(OpenOptions::new());
    assert_eq!(rows(db.execute(DERIVED).expect("derived")), 2);
    assert_eq!(rows(db.execute(RESULTS).expect("results")), 2);
}

#[test]
fn empty_list_refuses_every_entry_point() {
    let db = setup(OpenOptions::new().allow_overrides([]));
    for (q, kw) in [(DERIVED, ":max-derived-facts"), (RESULTS, ":max-results")] {
        refused(db.execute(q).err().expect("execute refused"), kw);
        refused(db.query(q).err().expect("query refused"), kw);
        refused(db.prepare(q).err().expect("prepare refused"), kw);
        let mut tx = db.begin_write().expect("begin");
        refused(tx.execute(q).err().expect("tx refused"), kw);
        tx.rollback();
    }
    assert_eq!(rows(db.execute(PLAIN).expect("plain")), 2);
    assert_eq!(db.query(PLAIN).expect("cursor").count(), 2);
    assert_eq!(
        rows(
            db.prepare(PLAIN)
                .expect("prepare")
                .execute(&[])
                .expect("run")
        ),
        2
    );
}

#[test]
fn list_replaces_defaults() {
    let db = setup(OpenOptions::new().allow_overrides([Overridable::MaxResults]));
    refused(
        db.execute(DERIVED).err().expect("refused"),
        ":max-derived-facts",
    );
    assert_eq!(rows(db.execute(RESULTS).expect("allowed")), 2);
}

#[test]
fn refused_tx_execute_leaves_transaction_usable() {
    let db = setup(OpenOptions::new().allow_overrides([]));
    let mut tx = db.begin_write().expect("begin");
    refused(
        tx.execute(DERIVED).err().expect("refused"),
        ":max-derived-facts",
    );
    tx.execute("(transact [[:c :edge :d]])").expect("transact");
    tx.commit().expect("commit");
    assert_eq!(rows(db.execute(PLAIN).expect("plain")), 3);
}

#[test]
fn defaults_read_back_and_compose() {
    assert_eq!(
        OpenOptions::default().allow_overrides,
        Overridable::DEFAULTS.to_vec()
    );
    assert_eq!(
        Overridable::DEFAULTS,
        &[
            Overridable::MaxDerivedFacts,
            Overridable::MaxResults,
            Overridable::MaxIterations
        ]
    );
    assert_eq!(Overridable::MaxDerivedFacts.keyword(), ":max-derived-facts");
    assert_eq!(Overridable::MaxResults.keyword(), ":max-results");

    let list: Vec<Overridable> = Overridable::DEFAULTS
        .iter()
        .copied()
        .chain([Overridable::MaxResults, Overridable::MaxResults])
        .collect();
    let db = setup(OpenOptions::new().allow_overrides(list));
    assert_eq!(rows(db.execute(DERIVED).expect("derived")), 2);
    assert_eq!(rows(db.execute(RESULTS).expect("results")), 2);
}

#[test]
fn file_database_option_not_persisted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("o.graph");
    {
        let db = OpenOptions::new()
            .allow_overrides([])
            .path(&path)
            .open()
            .expect("open");
        db.execute("(transact [[:a :edge :b]])").expect("transact");
        db.execute(RULE).expect("rule");
        refused(
            db.execute(DERIVED).err().expect("refused"),
            ":max-derived-facts",
        );
    }
    let db = Minigraf::open(&path).expect("reopen");
    db.execute(RULE).expect("rule");
    assert_eq!(rows(db.execute(DERIVED).expect("allowed")), 1);
}
