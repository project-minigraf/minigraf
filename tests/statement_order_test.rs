//! Several writes to one `(e, a, v)` in one `WriteTransaction` (#477).
//!
//! The last statement that writes a triple decides it, in any mix of
//! `transact` and `retract`: reads inside the transaction and the committed
//! result agree. Each scenario runs in memory, file-backed with the commit
//! checkpointed, and file-backed after a reopen.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult, Value};

enum Mode {
    Memory,
    Checkpointed,
    Reopened,
}

const MODES: [Mode; 3] = [Mode::Memory, Mode::Checkpointed, Mode::Reopened];

/// `:tag` facts of `:d/c` as `value [valid-to)` rows, sorted, with the
/// current view first and the `:any-valid-time` view second.
type Rows = (Vec<String>, Vec<String>);

const NOW: &str = r#"(query [:find ?v :where [:d/c :tag ?v]])"#;
const ANY: &str =
    r#"(query [:find ?v ?vt :any-valid-time :where [?e :tag ?v] [?e :db/valid-to ?vt]])"#;

fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Integer(n) => n.to_string(),
        _ => "other".to_string(),
    }
}

fn to_rows(result: QueryResult) -> Vec<String> {
    let QueryResult::QueryResults { results, .. } = result else {
        panic!("expected query results");
    };
    let mut rows: Vec<String> = results
        .iter()
        .map(|r| r.iter().map(render).collect::<Vec<_>>().join(" "))
        .collect();
    rows.sort();
    rows
}

fn read(db: &Minigraf) -> Rows {
    (
        to_rows(db.execute(NOW).unwrap()),
        to_rows(db.execute(ANY).unwrap()),
    )
}

/// Run `setup` as separate transactions, then `stmts` in one
/// `WriteTransaction`. Returns the rows read inside the transaction just
/// before commit and the rows read after it (and after a checkpoint or
/// reopen, by `mode`).
fn run(mode: &Mode, setup: &[&str], stmts: &[&str]) -> (Rows, Rows) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("o.graph");
    let db = match mode {
        Mode::Memory => Minigraf::in_memory().unwrap(),
        _ => Minigraf::open(&path).unwrap(),
    };
    for w in setup {
        db.execute(w).unwrap();
    }
    let mut tx = db.begin_write().unwrap();
    for s in stmts {
        tx.execute(s).unwrap();
    }
    let inside = (
        to_rows(tx.execute(NOW).unwrap()),
        to_rows(tx.execute(ANY).unwrap()),
    );
    tx.commit().unwrap();
    let after = match mode {
        Mode::Memory => read(&db),
        Mode::Checkpointed => {
            db.checkpoint().unwrap();
            read(&db)
        }
        Mode::Reopened => {
            drop(db);
            read(&Minigraf::open(&path).unwrap())
        }
    };
    (inside, after)
}

fn rows(now: &[&str], any: &[&str]) -> Rows {
    (
        now.iter().map(|s| s.to_string()).collect(),
        any.iter().map(|s| s.to_string()).collect(),
    )
}

const FOREVER: &str = "9223372036854775807";
/// 2030-01-01 in Unix ms.
const T2030: &str = "1893456000000";

const ASSERT: &str = r#"(transact [[:d/c :tag "a"]])"#;
const ASSERT_2030: &str =
    r#"(transact {:valid-from "2020-01-01" :valid-to "2030-01-01"} [[:d/c :tag "a"]])"#;
const RETRACT: &str = r#"(retract [[:d/c :tag "a"]])"#;

/// Every mix of two statements on one triple, after a committed assertion
/// and on an empty database: the second statement decides, before and after
/// commit.
#[test]
fn last_statement_decides_triple() {
    let live = rows(&["a"], &[&format!("a {FOREVER}")]);
    let live_2030 = rows(&["a"], &[&format!("a {T2030}")]);
    let gone = rows(&[], &[]);
    let cases: [(&[&str], [&str; 2], &Rows); 10] = [
        (&[ASSERT], [RETRACT, ASSERT], &live),
        (&[], [RETRACT, ASSERT], &live),
        (&[ASSERT], [ASSERT, RETRACT], &gone),
        (&[], [ASSERT, RETRACT], &gone),
        (&[ASSERT], [RETRACT, RETRACT], &gone),
        (&[ASSERT], [ASSERT, ASSERT_2030], &live_2030),
        (&[], [ASSERT_2030, ASSERT], &live),
        (&[ASSERT], [RETRACT, ASSERT_2030], &live_2030),
        (&[ASSERT_2030], [ASSERT, ASSERT], &live),
        (&[ASSERT], [ASSERT_2030, RETRACT], &gone),
    ];
    for (i, (setup, stmts, want)) in cases.iter().enumerate() {
        for mode in &MODES {
            let (inside, after) = run(mode, setup, stmts);
            assert_eq!(&inside, *want, "case {i}: inside the transaction");
            assert_eq!(&after, *want, "case {i}: committed");
        }
    }
}

/// Only the triple's earlier statements are dropped: other values in the
/// same statements stay, and the commit is still one transaction.
#[test]
fn other_triples_of_overridden_statement_survive() {
    for mode in &MODES {
        let (inside, after) = run(
            mode,
            &[],
            &[
                r#"(transact [[:d/c :tag "a"] [:d/c :tag "b"]])"#,
                r#"(retract [[:d/c :tag "a"] [:d/c :tag "c"]])"#,
                r#"(transact [[:d/c :tag "c"]])"#,
            ],
        );
        let want = rows(
            &["b", "c"],
            &[&format!("b {FOREVER}"), &format!("c {FOREVER}")],
        );
        assert_eq!(inside, want, "inside the transaction");
        assert_eq!(after, want, "committed");
    }
}

/// The committed transaction holds only the surviving records: one
/// `tx_count`, and an overridden assertion is not in history.
#[test]
fn commit_writes_one_transaction_without_overridden_records() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(ASSERT_2030).unwrap();
    tx.execute(RETRACT).unwrap();
    tx.execute(ASSERT).unwrap();
    tx.commit().unwrap();
    assert_eq!(db.current_tx_count(), 1, "one transaction");
    let history = to_rows(
        db.execute(
            r#"(query [:find ?v ?vt ?tc :any-valid-time :where [?e :tag ?v] [?e :db/valid-to ?vt] [?e :db/tx-count ?tc]])"#,
        )
        .unwrap(),
    );
    assert_eq!(
        history,
        vec![format!("a {FOREVER} 1")],
        "only the last statement's record"
    );
}

/// Two windows of one triple in one statement are API-011 from
/// `tx.execute`; nothing of that statement is staged and the transaction
/// still commits its other statements.
#[test]
fn two_windows_in_one_statement_rejected_by_tx_execute() {
    for mode in &MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.graph");
        let db = match mode {
            Mode::Memory => Minigraf::in_memory().unwrap(),
            _ => Minigraf::open(&path).unwrap(),
        };
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:d/c :tag "b"]])"#).unwrap();
        let err = tx
            .execute(
                r#"(transact [[:d/c :tag "a" {:valid-to "2030-01-01"}] [:d/c :tag "a" {:valid-from "2020-01-01"}] [:d/c :tag "z"]])"#,
            )
            .expect_err("two windows in one statement");
        assert_eq!(err.code(), "API-011");
        tx.commit().unwrap();
        let db = match mode {
            Mode::Reopened => {
                drop(db);
                Minigraf::open(&path).unwrap()
            }
            _ => db,
        };
        assert_eq!(
            read(&db).0,
            vec!["b".to_string()],
            "only the accepted statement"
        );
        assert_eq!(db.current_tx_count(), 1, "one transaction");
    }
}
