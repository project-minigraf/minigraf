//! `LogWriter` builds a new database from explicit fact records (#431). These
//! tests copy a source's fact log through it and check that the result answers
//! every query, `:as-of` and fact-log read as the source does, survives a
//! reopen with intact indexes (#370), and that holes, rejections, and the
//! `.partial` build file behave as documented.
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{FactFilter, FactRecord, LogWriter, Minigraf, OpenOptions, QueryResult, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const NAMES: [&str; 4] = ["e1", "e2", "e3", "e4"];
const LONG: &str = "a long string value that is stored in a value page, not in the key: \
                    0123456789abcdefghijklmnopqrstuvwxyz";

fn uuid(i: usize) -> Uuid {
    Uuid::from_u128(0x2000 + i as u128)
}

fn e(i: usize) -> String {
    format!("#uuid \"{}\"", uuid(i))
}

fn name_of(u: &Uuid) -> &'static str {
    (0..NAMES.len())
        .find(|&i| uuid(i) == *u)
        .map_or("?", |i| NAMES[i])
}

fn show(v: &Value) -> String {
    match v {
        Value::Ref(u) => format!("ref {}", name_of(u)),
        other => format!("{other:?}"),
    }
}

/// A record without UUIDs, for comparisons.
fn row(r: &FactRecord) -> String {
    format!(
        "{} {} {} {} {} {} {} {}",
        r.tx_count,
        r.tx_id,
        name_of(&r.entity),
        r.attribute,
        show(&r.value),
        r.valid_from,
        r.valid_to,
        r.asserted
    )
}

fn log_rows(db: &Minigraf) -> Vec<String> {
    let mut rows: Vec<String> = db
        .fact_log(&FactFilter::new())
        .unwrap()
        .map(|r| row(&r.unwrap()))
        .collect();
    rows.sort();
    rows
}

fn records(db: &Minigraf) -> Vec<FactRecord> {
    db.fact_log(&FactFilter::new())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Query rows without UUIDs, sorted.
fn rows(db: &Minigraf, query: &str) -> Vec<String> {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => {
            let mut out: Vec<String> = results
                .iter()
                .map(|r| r.iter().map(show).collect::<Vec<_>>().join(" "))
                .collect();
            out.sort();
            out
        }
        _ => panic!("expected query results"),
    }
}

fn all_at(as_of: Option<u64>, valid: &str) -> String {
    let as_of = as_of.map(|n| format!(":as-of {n}")).unwrap_or_default();
    format!("(query [:find ?e ?a ?v {as_of} :valid-at {valid} :where [?e ?a ?v]])")
}

/// A source with assertions, retractions, explicit windows, long strings,
/// refs, keywords and a multi-fact transaction.
fn source(path: &Path) -> Minigraf {
    let db = Minigraf::open(path).unwrap();
    let writes = [
        format!(
            r#"(transact [[{} :person/name "Alice"] [{} :person/age 30]
                          [{} :person/name "Bob"] [{} :person/friend {}]])"#,
            e(0),
            e(0),
            e(1),
            e(1),
            e(0)
        ),
        format!(
            "(transact [[{} :person/age 31] [{} :person/role :role/admin]])",
            e(0),
            e(1)
        ),
        format!("(retract [[{} :person/age 30]])", e(0)),
        format!("(transact [[{} :doc/body \"{LONG}\"]])", e(2)),
        format!(
            r#"(transact {{:valid-from "2020-01-01" :valid-to "2022-01-01"}} [[{} :person/salary 100]])"#,
            e(1)
        ),
        format!("(retract [[{} :person/role :role/admin]])", e(1)),
        format!(
            "(transact [[{} :person/score 2.5] [{} :flag/on true]])",
            e(3),
            e(3)
        ),
    ];
    for (i, w) in writes.iter().enumerate() {
        db.execute(w).unwrap();
        if i == 3 {
            db.checkpoint().unwrap();
        }
    }
    db
}

fn copy(src: &Minigraf, out: &Path, keep: impl Fn(&FactRecord) -> bool) {
    let mut w = LogWriter::create(out, OpenOptions::new()).unwrap();
    for rec in records(src) {
        if keep(&rec) {
            w.append(&rec).unwrap();
        }
    }
    w.advance_tx_count(src.current_tx_count()).unwrap();
    w.finish().unwrap();
}

fn same_answers(a: &Minigraf, b: &Minigraf) {
    assert_eq!(a.current_tx_count(), b.current_tx_count(), "tx count");
    assert_eq!(log_rows(a), log_rows(b), "fact log");
    let mut queries = vec![
        all_at(None, ":any-valid-time"),
        all_at(None, "\"2021-06-01\""),
        "(query [:find ?e ?a ?v :where [?e ?a ?v]])".to_string(),
        format!("(query [:find ?a ?v :where [{} ?a ?v]])", e(0)),
        "(query [:find ?e :where [?e :person/name \"Bob\"]])".to_string(),
        "(query [:find ?e ?v :where [?e :person/friend ?v]])".to_string(),
        format!("(query [:find ?e :where [?e :person/friend {}]])", e(0)),
    ];
    for n in 0..=a.current_tx_count() {
        queries.push(all_at(Some(n), ":any-valid-time"));
    }
    for q in &queries {
        assert_eq!(rows(a, q), rows(b, q), "{q}");
    }
}

struct Dir {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Dir {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        Dir { _tmp: tmp, root }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

fn partial(p: &Path) -> PathBuf {
    p.with_file_name(format!(
        "{}.partial",
        p.file_name().unwrap().to_str().unwrap()
    ))
}

fn wal(p: &Path) -> PathBuf {
    p.with_file_name(format!("{}.wal", p.file_name().unwrap().to_str().unwrap()))
}

#[test]
fn copy_answers_like_the_source_and_survives_reopen() {
    let dir = Dir::new();
    let src = source(&dir.path("src.graph"));
    let out = dir.path("out.graph");
    copy(&src, &out, |_| true);

    assert!(out.exists());
    assert!(!partial(&out).exists(), "no .partial left");
    assert!(!wal(&out).exists(), "no WAL");

    let db = Minigraf::open(&out).unwrap();
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
    same_answers(&src, &db);
    drop(db);

    // Read-only, then read-write again: indexed lookups after reopen (#370).
    let ro = Minigraf::open_with_options(&out, OpenOptions::new().read_only(true)).unwrap();
    same_answers(&src, &ro);
    drop(ro);
    let db = Minigraf::open(&out).unwrap();
    same_answers(&src, &db);
}

#[test]
fn copy_equals_a_database_built_by_transact() {
    // Records with the same timestamps in a source and its copy give equal
    // fact logs; the copy also takes new writes after the source's counter.
    let dir = Dir::new();
    let src = source(&dir.path("src.graph"));
    let out = dir.path("out.graph");
    copy(&src, &out, |_| true);
    let db = Minigraf::open(&out).unwrap();
    src.execute(&format!("(transact [[{} :person/name \"Cy\"]])", e(2)))
        .unwrap();
    db.execute(&format!("(transact [[{} :person/name \"Cy\"]])", e(2)))
        .unwrap();
    assert_eq!(db.current_tx_count(), src.current_tx_count());
    let q = "(query [:find ?e ?v :where [?e :person/name ?v]])";
    assert_eq!(rows(&db, q), rows(&src, q));
    db.checkpoint().unwrap();
    assert!(
        db.verify().unwrap().is_ok(),
        "verify is clean after new writes"
    );
}

#[test]
fn many_batches_give_the_same_file_contents() {
    // Two batch commits: the first when transaction 8 opens with 70,000 facts
    // pending (over the 65,536 batch), the second in `finish`.
    let dir = Dir::new();
    let out = dir.path("big.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    let per_tx = 10_000u64;
    for tx in 1..=9u64 {
        for i in 0..per_tx {
            let rec = FactRecord {
                entity: Uuid::from_u128(u128::from(i % 1000) + 1),
                attribute: format!(":n/a{}", i % 7),
                value: Value::Integer((tx * per_tx + i) as i64),
                tx_count: tx * 2,
                tx_id: 1_700_000_000_000 + tx,
                valid_from: 1_700_000_000_000 + tx as i64,
                valid_to: i64::MAX,
                asserted: true,
            };
            w.append(&rec).unwrap();
        }
    }
    assert_eq!(w.tx_count(), 18);
    w.finish().unwrap();

    let db = Minigraf::open(&out).unwrap();
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
    assert_eq!(db.current_tx_count(), 18);
    let recs = records(&db);
    assert_eq!(recs.len(), 90_000);
    assert!(recs.windows(2).all(|p| p[0].tx_count <= p[1].tx_count));
    let count = |q: &str| match db.execute(q).unwrap() {
        QueryResult::QueryResults { results, .. } => match results[0][0] {
            Value::Integer(n) => n,
            _ => panic!("count"),
        },
        _ => panic!("results"),
    };
    let a3_per_tx = (0..per_tx).filter(|i| i % 7 == 3).count() as i64;
    assert_eq!(
        count("(query [:find (count ?v) :where [?e :n/a3 ?v]])"),
        9 * a3_per_tx
    );
    assert_eq!(
        count("(query [:find (count ?v) :as-of 7 :where [?e :n/a3 ?v]])"),
        // Transactions 2, 4 and 6.
        3 * a3_per_tx
    );
}

#[test]
fn a_purged_transaction_is_a_hole_that_shows_the_state_before_it() {
    let dir = Dir::new();
    let src = source(&dir.path("src.graph"));
    let out = dir.path("out.graph");
    // Transaction 2 asserted age 31 and the admin role.
    copy(&src, &out, |r| r.tx_count != 2);
    let db = Minigraf::open(&out).unwrap();
    assert_eq!(db.current_tx_count(), src.current_tx_count());
    assert_eq!(
        rows(&db, &all_at(Some(2), ":any-valid-time")),
        rows(&src, &all_at(Some(1), ":any-valid-time"))
    );
    assert!(
        log_rows(&db).iter().all(|r| !r.starts_with("2 ")),
        "nothing from transaction 2"
    );
    // Later transactions are unchanged.
    assert_eq!(
        rows(
            &db,
            &format!("(query [:find ?v :as-of 4 :where [{} :doc/body ?v]])", e(2))
        ),
        vec![format!("{:?}", Value::String(LONG.to_string()))]
    );
}

#[test]
fn trailing_holes_keep_the_counter() {
    let dir = Dir::new();
    let src = source(&dir.path("src.graph"));
    let last = src.current_tx_count();
    let out = dir.path("out.graph");
    copy(&src, &out, |r| r.tx_count < last - 1);
    let db = Minigraf::open(&out).unwrap();
    assert_eq!(db.current_tx_count(), last);
    db.execute(&format!("(transact [[{} :person/name \"Dee\"]])", e(3)))
        .unwrap();
    assert_eq!(db.current_tx_count(), last + 1);
}

#[test]
fn an_empty_log_is_an_empty_database_with_the_counter() {
    let dir = Dir::new();
    let out = dir.path("empty.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    w.advance_tx_count(5).unwrap();
    w.finish().unwrap();
    let db = Minigraf::open(&out).unwrap();
    assert_eq!(db.current_tx_count(), 5);
    assert!(records(&db).is_empty());
    assert!(db.verify().unwrap().is_ok(), "verify is clean");

    let out = dir.path("nothing.graph");
    LogWriter::create(&out, OpenOptions::new())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(Minigraf::open(&out).unwrap().current_tx_count(), 0);
}

fn rec(tx_count: u64, tx_id: u64, attr: &str, value: i64) -> FactRecord {
    FactRecord {
        entity: uuid(0),
        attribute: attr.to_string(),
        value: Value::Integer(value),
        tx_count,
        tx_id,
        valid_from: tx_id as i64,
        valid_to: i64::MAX,
        asserted: true,
    }
}

#[test]
fn rejected_records_change_nothing() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    let code = |r: Result<(), minigraf::MinigrafError>| r.unwrap_err().code().to_string();

    assert_eq!(code(w.append(&rec(0, 100, ":a/x", 0))), "API-015", "tx 0");
    w.append(&rec(3, 300, ":a/x", 1)).unwrap();
    assert_eq!(
        code(w.append(&rec(2, 200, ":a/x", 2))),
        "API-015",
        "lower tx"
    );
    assert_eq!(
        code(w.append(&rec(3, 301, ":a/x", 3))),
        "API-016",
        "two tx_ids"
    );

    let mut other_window = rec(3, 300, ":a/x", 1);
    other_window.valid_to = 400;
    assert_eq!(code(w.append(&other_window)), "API-011", "two windows");
    let mut retraction = other_window.clone();
    retraction.asserted = false;
    assert_eq!(
        code(w.append(&retraction)),
        "API-020",
        "retracting what the transaction asserts"
    );
    let mut inverted = rec(3, 300, ":a/z", 1);
    inverted.valid_to = 300;
    assert_eq!(code(w.append(&inverted)), "API-019", "empty window");

    let mut big = rec(3, 300, ":a/x", 0);
    big.value = Value::String("x".repeat(1 << 20));
    assert_eq!(code(w.append(&big)), "WAL-003", "oversize value");
    let mut long_attr = rec(3, 300, ":a/x", 0);
    long_attr.attribute = format!(":a/{}", "x".repeat(1 << 16));
    assert_eq!(code(w.append(&long_attr)), "WAL-003", "oversize ident");

    // Exact repeats are written once.
    w.append(&rec(3, 300, ":a/x", 1)).unwrap();
    w.append(&rec(3, 300, ":a/y", 9)).unwrap();

    // Closing a transaction by advancing makes it unjoinable.
    assert_eq!(code(w.advance_tx_count(2)), "API-015", "advance backwards");
    w.advance_tx_count(3).unwrap();
    assert_eq!(
        code(w.append(&rec(3, 300, ":a/x", 5))),
        "API-015",
        "closed tx"
    );
    w.append(&rec(4, 250, ":a/x", 6)).unwrap(); // tx_id may go backwards
    assert_eq!(w.tx_count(), 4);
    w.finish().unwrap();

    let db = Minigraf::open(&out).unwrap();
    let mut got: Vec<String> = records(&db).iter().map(row).collect();
    got.sort();
    let want = vec![
        "3 300 e1 :a/x Integer(1) 300 9223372036854775807 true".to_string(),
        "3 300 e1 :a/y Integer(9) 300 9223372036854775807 true".to_string(),
        "4 250 e1 :a/x Integer(6) 250 9223372036854775807 true".to_string(),
    ];
    assert_eq!(got, want);
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
}

/// Every file the writer builds keeps the rules of a normal write (#477):
/// an inverted window, two windows of one fact in one transaction, and an
/// assertion and a retraction of one fact in one transaction (either order)
/// are rejected. Duplicate retractions, identical repeats, and the same fact
/// in the next transaction are accepted.
#[test]
fn records_that_break_write_rules_are_rejected() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    let code = |r: Result<(), minigraf::MinigrafError>| r.unwrap_err().code().to_string();
    let retract = |mut r: FactRecord| {
        r.asserted = false;
        r
    };

    let mut inverted = rec(1, 100, ":a/w", 4);
    inverted.valid_to = 50;
    assert_eq!(
        code(w.append(&inverted)),
        "API-019",
        "ends before it starts"
    );
    assert_eq!(w.tx_count(), 0, "a rejected record opens no transaction");

    w.append(&retract(rec(1, 100, ":a/y", 2))).unwrap();
    w.append(&retract(rec(1, 100, ":a/y", 2))).unwrap();
    assert_eq!(
        code(w.append(&rec(1, 100, ":a/y", 2))),
        "API-020",
        "retract then assert"
    );
    w.append(&rec(1, 100, ":a/x", 1)).unwrap();
    w.append(&rec(1, 100, ":a/x", 1)).unwrap();
    assert_eq!(
        code(w.append(&retract(rec(1, 100, ":a/x", 1)))),
        "API-020",
        "assert then retract"
    );
    let mut second = rec(1, 100, ":a/x", 1);
    second.valid_from = 50;
    assert_eq!(code(w.append(&second)), "API-011", "second window");
    // The next transaction may write the fact again, any way.
    w.append(&retract(rec(2, 200, ":a/x", 1))).unwrap();
    w.append(&rec(2, 200, ":a/y", 2)).unwrap();
    w.finish().unwrap();

    let db = Minigraf::open(&out).unwrap();
    let mut got: Vec<String> = records(&db).iter().map(row).collect();
    got.sort();
    let want = vec![
        "1 100 e1 :a/x Integer(1) 100 9223372036854775807 true".to_string(),
        "1 100 e1 :a/y Integer(2) 100 9223372036854775807 false".to_string(),
        "2 200 e1 :a/x Integer(1) 200 9223372036854775807 false".to_string(),
        "2 200 e1 :a/y Integer(2) 200 9223372036854775807 true".to_string(),
    ];
    assert_eq!(got, want);
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
}

#[test]
fn create_refuses_existing_targets_and_read_only() {
    let dir = Dir::new();
    let existing = dir.path("db.graph");
    std::fs::write(&existing, b"").unwrap();
    let err = LogWriter::create(&existing, OpenOptions::new())
        .err()
        .unwrap();
    assert_eq!(err.code(), "STG-043");
    assert_eq!(std::fs::metadata(&existing).unwrap().len(), 0);

    let with_wal = dir.path("w.graph");
    std::fs::write(wal(&with_wal), b"stale").unwrap();
    let err = LogWriter::create(&with_wal, OpenOptions::new())
        .err()
        .unwrap();
    assert_eq!(err.code(), "STG-043");
    assert!(!with_wal.exists());

    let err = LogWriter::create(dir.path("r.graph"), OpenOptions::new().read_only(true))
        .err()
        .unwrap();
    assert_eq!(err.code(), "API-014");
    assert!(!partial(&dir.path("r.graph")).exists());
}

#[test]
fn a_second_writer_for_the_same_path_is_refused() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    let mut first = LogWriter::create(&out, OpenOptions::new()).unwrap();
    let err = LogWriter::create(&out, OpenOptions::new()).err().unwrap();
    assert_eq!(err.code(), "STG-025");
    first.append(&rec(1, 100, ":a/x", 1)).unwrap();
    first.finish().unwrap();
    assert_eq!(records(&Minigraf::open(&out).unwrap()).len(), 1);
}

#[test]
fn a_stale_partial_is_reused() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    std::fs::write(partial(&out), vec![0xAB; 3 * 4096]).unwrap();
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    w.append(&rec(1, 100, ":a/x", 1)).unwrap();
    w.finish().unwrap();
    let db = Minigraf::open(&out).unwrap();
    assert_eq!(records(&db).len(), 1);
    assert!(db.verify().unwrap().is_ok(), "verify is clean");
}

#[test]
fn dropping_an_unfinished_writer_leaves_nothing() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    w.append(&rec(1, 100, ":a/x", 1)).unwrap();
    assert!(partial(&out).exists());
    drop(w);
    assert!(!partial(&out).exists());
    assert!(!out.exists());
    assert!(!wal(&out).exists());
}

#[test]
fn finish_refuses_a_target_created_during_the_build() {
    let dir = Dir::new();
    let out = dir.path("out.graph");
    let mut w = LogWriter::create(&out, OpenOptions::new()).unwrap();
    w.append(&rec(1, 100, ":a/x", 1)).unwrap();
    std::fs::write(&out, b"someone else's").unwrap();
    let err = w.finish().err().unwrap();
    assert_eq!(err.code(), "STG-043");
    assert_eq!(std::fs::read(&out).unwrap(), b"someone else's");
    assert!(!partial(&out).exists());
}

#[test]
fn log_writer_is_send() {
    fn is_send<T: Send>() {}
    is_send::<LogWriter>();
}
