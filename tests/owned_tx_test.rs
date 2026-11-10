// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]
//! `OwnedWriteTransaction` (#824): a write transaction that owns its
//! database handle, so a language binding can hold it in an object that is
//! used, committed or dropped on any thread. It must leave the database as
//! the borrowed `WriteTransaction` does, follow its begin thread for
//! `INT-001`, and make other threads' writers wait.

use minigraf::{
    FactFilter, Minigraf, OpenOptions, OwnedWriteTransaction, QueryResult, TxOptions,
    WriteTransaction,
};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a call that should wait is given to return anyway, and how long
/// a call that should not wait may take before the test calls it stuck.
const WAITS: Duration = Duration::from_millis(300);
const STUCK: Duration = Duration::from_secs(30);

/// A compile-time check: the owned transaction can be held in a `'static`
/// object and moved between threads (#824 A1).
#[test]
fn owned_tx_is_send_and_static() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<OwnedWriteTransaction>();
}

// ─── Equivalence with the borrowed transaction (A2) ──────────────────────────

/// One fact version, comparable across databases: a default valid-from
/// (the commit time) is shown as `None`, because two databases commit at
/// different times.
type Rec = (String, String, String, u64, Option<i64>, i64, bool);

fn records(db: &Minigraf) -> Vec<Rec> {
    let mut v: Vec<Rec> = db
        .fact_log(&FactFilter::new())
        .unwrap()
        .map(|r| {
            let r = r.unwrap();
            let from = (r.valid_from != r.tx_id.cast_signed()).then_some(r.valid_from);
            (
                r.entity.to_string(),
                r.attribute,
                format!("{:?}", r.value),
                r.tx_count,
                from,
                r.valid_to,
                r.asserted,
            )
        })
        .collect();
    v.sort();
    v
}

/// The rows of a query over every current fact, sorted.
fn rows(db: &Minigraf) -> Vec<String> {
    let QueryResult::QueryResults { results, .. } = db
        .execute("(query [:find ?e ?a ?v :where [?e ?a ?v]])")
        .unwrap()
    else {
        panic!("not a query result");
    };
    let mut v: Vec<String> = results.iter().map(|r| format!("{r:?}")).collect();
    v.sort();
    v
}

/// What a statement inside a transaction returned: its rows or error code.
fn outcome(result: Result<QueryResult, minigraf::MinigrafError>) -> String {
    match result {
        Ok(QueryResult::QueryResults { results, .. }) => {
            let mut v: Vec<String> = results.iter().map(|r| format!("{r:?}")).collect();
            v.sort();
            v.join(";")
        }
        Ok(QueryResult::Ok) => "ok".to_string(),
        Ok(_) => "other".to_string(),
        Err(e) => e.code().to_string(),
    }
}

const SETUP: &str = r#"(transact [[:alice :person/name "Alice"] [:alice :person/age 30]
    [:bob :person/name "Bob"] [:carol :person/name "Carol"]])"#;

/// Statement sequences: writes, retractions, reads of staged state, rules,
/// rejections (API-011, parse errors, API-002 via `parse`), several windows
/// of a triple across statements (latest statement wins, #477), and enough
/// facts to pass a chunk of 2.
const SEQUENCES: &[&[&str]] = &[
    &[],
    &["(query [:find ?n :where [?e :person/name ?n]])"],
    &[
        r#"(transact [[:dave :person/name "Dave"] [:dave :person/age 40]])"#,
        "(query [:find ?n :where [?e :person/name ?n]])",
        r#"(retract [[:bob :person/name "Bob"]])"#,
        "(query [:find ?n :where [?e :person/name ?n]])",
        "(transact [[:alice :person/age 31]])",
    ],
    &[
        r#"(transact [[:erin :person/name "Erin" {:valid-from "2020-01-01T00:00:00Z"}]])"#,
        r#"(transact [[:erin :person/name "Erin" {:valid-from "2021-01-01T00:00:00Z" :valid-to "2030-01-01T00:00:00Z"}]])"#,
        r#"(transact [[:x :p/v 1 {:valid-from "2020-01-01T00:00:00Z"}] [:x :p/v 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#,
        "(transact [[:x :p/v",
        "(query [:find ?e :where [?e :person/name \"Erin\"]])",
    ],
    &[
        "(rule [(named ?e ?n) [?e :person/name ?n]])",
        r#"(transact [[:frank :person/name "Frank"] [:frank :person/friend :alice] [:gina :person/name "Gina"]])"#,
        "(query [:find ?n :where (named ?e ?n)])",
        r#"(retract [[:carol :person/name "Carol"] [:alice :person/age 30]])"#,
        r#"(transact [[:hal :person/name "Hal"] [:hal :person/age 50] [:ida :person/name "Ida"]])"#,
        "(query [:find ?n ?a :where [?e :person/name ?n] [?e :person/age ?a]])",
    ],
];

#[derive(Clone, Copy, Debug)]
enum End {
    Commit,
    Rollback,
    Drop,
}

/// Run `stmts` in a borrowed transaction, return each statement's outcome.
fn run_borrowed(db: &Minigraf, opts: TxOptions, stmts: &[&str], end: End) -> Vec<String> {
    let mut tx: WriteTransaction<'_> = db.begin_write_with(opts).unwrap();
    let out = stmts.iter().map(|s| outcome(tx.execute(s))).collect();
    match end {
        End::Commit => tx.commit().unwrap(),
        End::Rollback => tx.rollback(),
        End::Drop => drop(tx),
    }
    out
}

fn run_owned(db: &Minigraf, opts: TxOptions, stmts: &[&str], end: End) -> Vec<String> {
    let mut tx = db.begin_write_owned_with(opts).unwrap();
    let out = stmts.iter().map(|s| outcome(tx.execute(s))).collect();
    match end {
        End::Commit => tx.commit().unwrap(),
        End::Rollback => tx.rollback(),
        End::Drop => drop(tx),
    }
    out
}

fn open_file(path: &Path) -> Minigraf {
    OpenOptions::new().path(path).open().unwrap()
}

#[test]
fn owned_tx_matches_borrowed_tx() {
    for (i, stmts) in SEQUENCES.iter().enumerate() {
        for end in [End::Commit, End::Rollback, End::Drop] {
            // `None`: the database's chunk; `Some(2)`: large on a file.
            for chunk in [None, Some(2)] {
                let opts = || match chunk {
                    Some(n) => TxOptions::new().chunk_facts(n),
                    None => TxOptions::new(),
                };
                let case = format!("sequence {i}, {end:?}, chunk {chunk:?}");

                // In memory.
                let a = Minigraf::in_memory().unwrap();
                let b = Minigraf::in_memory().unwrap();
                a.execute(SETUP).unwrap();
                b.execute(SETUP).unwrap();
                let out_a = run_borrowed(&a, opts(), stmts, end);
                let out_b = run_owned(&b, opts(), stmts, end);
                assert!(
                    out_a == out_b,
                    "statement outcomes differ in memory: {case}"
                );
                assert!(
                    records(&a) == records(&b),
                    "records differ in memory: {case}"
                );
                assert!(rows(&a) == rows(&b), "rows differ in memory: {case}");
                assert_eq!(a.current_tx_count(), b.current_tx_count(), "{case}");

                // On a file, before and after reopening.
                let dir = tempfile::tempdir().unwrap();
                let (pa, pb) = (dir.path().join("a.graph"), dir.path().join("b.graph"));
                let (out_a, out_b, rec_a, rec_b) = {
                    let a = open_file(&pa);
                    let b = open_file(&pb);
                    a.execute(SETUP).unwrap();
                    b.execute(SETUP).unwrap();
                    let out_a = run_borrowed(&a, opts(), stmts, end);
                    let out_b = run_owned(&b, opts(), stmts, end);
                    assert!(rows(&a) == rows(&b), "rows differ on a file: {case}");
                    (out_a, out_b, records(&a), records(&b))
                };
                assert!(
                    out_a == out_b,
                    "statement outcomes differ on a file: {case}"
                );
                assert!(rec_a == rec_b, "records differ on a file: {case}");
                let (a, b) = (open_file(&pa), open_file(&pb));
                assert!(
                    records(&a) == records(&b),
                    "records differ after reopen: {case}"
                );
                assert!(rows(&a) == rows(&b), "rows differ after reopen: {case}");
            }
        }
    }
}

/// The sequences above that pass a chunk of 2 do become large: a query of
/// the working trees sees the staged facts, and the commit is a checkpoint
/// (no WAL left).
#[test]
fn owned_tx_large_commit_is_a_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.graph");
    let db = open_file(&path);
    db.execute(SETUP).unwrap();
    let mut tx = db
        .begin_write_owned_with(TxOptions::new().chunk_facts(2))
        .unwrap();
    tx.execute(r#"(transact [[:a :p/n "A"] [:b :p/n "B"] [:c :p/n "C"] [:d :p/n "D"]])"#)
        .unwrap();
    let QueryResult::QueryResults { results, .. } = tx
        .execute("(query [:find ?n :where [?e :p/n ?n]])")
        .unwrap()
    else {
        panic!("not a query result");
    };
    assert_eq!(results.len(), 4, "the staged facts are read");
    tx.commit().unwrap();
    let mut wal = path.as_os_str().to_owned();
    wal.push(".wal");
    assert!(
        !Path::new(&wal).exists(),
        "a large commit is a checkpoint and deletes the WAL"
    );
    drop(db);
    let db = open_file(&path);
    assert_eq!(rows(&db).len(), 8, "every fact after reopening");
}

// ─── Threads (A3, A4) ────────────────────────────────────────────────────────

/// Every guarded call, by name, with its error code or `None` on success.
fn guarded_calls(db: &Minigraf) -> Vec<(&'static str, Option<&'static str>)> {
    let code = |r: Result<(), minigraf::MinigrafError>| r.err().map(|e| e.code());
    let prepared = || db.prepare("(query [:find ?n :where [?e :person/name ?n]])");
    vec![
        (
            "execute transact",
            code(
                db.execute(r#"(transact [[:zed :person/name "Zed"]])"#)
                    .map(|_| ()),
            ),
        ),
        (
            "execute query",
            code(
                db.execute("(query [:find ?e :where [?e :person/name _]])")
                    .map(|_| ()),
            ),
        ),
        (
            "query",
            code(
                db.query("(query [:find ?e :where [?e :person/name _]])")
                    .map(|_| ()),
            ),
        ),
        ("prepare", code(prepared().map(|_| ()))),
        (
            "fact_log",
            code(db.fact_log(&FactFilter::new()).map(|_| ())),
        ),
        ("checkpoint", code(db.checkpoint())),
        ("verify", code(db.verify().map(|_| ()))),
        ("rebuild_indexes", code(db.rebuild_indexes())),
        ("begin_write", code(db.begin_write().map(|_| ()))),
        (
            "begin_write_owned",
            code(db.begin_write_owned().map(|_| ())),
        ),
    ]
}

/// Prepared `execute`/`query` of a query prepared before the transaction.
fn prepared_calls(pq: &minigraf::PreparedQuery) -> Vec<(&'static str, Option<&'static str>)> {
    vec![
        ("prepared execute", pq.execute(&[]).err().map(|e| e.code())),
        ("prepared query", pq.query(&[]).err().map(|e| e.code())),
    ]
}

fn assert_all(calls: &[(&'static str, Option<&'static str>)], want: Option<&str>, when: &str) {
    for (name, code) in calls {
        assert_eq!(*code, want, "{name} {when}");
    }
}

fn file_db(dir: &tempfile::TempDir) -> Minigraf {
    let db = open_file(&dir.path().join("g.graph"));
    db.execute(SETUP).unwrap();
    db
}

#[test]
fn owned_tx_guards_begin_thread() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    let pq = db
        .prepare("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    assert_all(&guarded_calls(&db), Some("INT-001"), "on the begin thread");
    assert_all(&prepared_calls(&pq), Some("INT-001"), "on the begin thread");

    // Moved to another thread, it still guards the begin thread.
    let (back, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        tx.execute(r#"(transact [[:erin :person/name "Erin"]])"#)
            .unwrap();
        rx.recv().unwrap();
        tx.commit().unwrap();
    });
    assert_all(
        &guarded_calls(&db),
        Some("INT-001"),
        "on the begin thread while another thread holds the transaction",
    );
    back.send(()).unwrap();
    handle.join().unwrap();

    // Finished on the other thread: every call works here again.
    assert_all(&prepared_calls(&pq), None, "after the commit");
    assert_all(&guarded_calls(&db), None, "after the commit");
    assert_eq!(rows(&db).len(), 7, "both staged facts and Zed committed");
}

#[test]
fn owned_tx_commit_on_other_thread() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    let db2 = db.clone();
    thread::spawn(move || {
        tx.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
            .unwrap();
        tx.commit().unwrap();
        // The committing thread did not begin it: nothing is guarded here.
        assert_all(&guarded_calls(&db2), None, "on the committing thread");
    })
    .join()
    .unwrap();
    assert_all(
        &guarded_calls(&db),
        None,
        "on the begin thread after commit",
    );
    let names = db
        .execute("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    let QueryResult::QueryResults { results, .. } = names else {
        panic!("not a query result");
    };
    // Alice, Carol, Dave and Zed (from the first guarded_calls).
    assert_eq!(results.len(), 4, "Dave committed, Bob retracted");
}

#[test]
fn owned_tx_rollback_on_other_thread() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    thread::spawn(move || tx.rollback()).join().unwrap();
    let before = rows(&db);
    assert_eq!(before.len(), 4, "the staged fact is gone");
    assert_all(
        &guarded_calls(&db),
        None,
        "on the begin thread after rollback",
    );
}

#[test]
fn owned_tx_drop_on_other_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.graph");
    {
        let db = file_db(&dir);
        let mut tx = db
            .begin_write_owned_with(TxOptions::new().chunk_facts(2))
            .unwrap();
        // Large: its working pages are discarded by the drop.
        tx.execute(r#"(transact [[:a :p/n "A"] [:b :p/n "B"] [:c :p/n "C"]])"#)
            .unwrap();
        thread::spawn(move || drop(tx)).join().unwrap();
        assert_eq!(rows(&db).len(), 4, "nothing of the dropped transaction");
        assert_all(&guarded_calls(&db), None, "on the begin thread after drop");
        let db2 = db.clone();
        thread::spawn(move || {
            db2.execute(r#"(transact [[:y :p/n "Y"]])"#).unwrap();
        })
        .join()
        .unwrap();
    }
    let db = open_file(&path);
    let names: Vec<String> = rows(&db);
    // SETUP's 4 facts, Zed (guarded_calls) and Y.
    assert_eq!(names.len(), 6, "after reopening: no dropped fact");
}

/// The transaction keeps the database open after every other handle is
/// dropped, and the close-time checkpoint runs after it finishes.
#[test]
fn owned_tx_keeps_database_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.graph");
    let tx = {
        let db = file_db(&dir);
        let mut tx = db.begin_write_owned().unwrap();
        tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
            .unwrap();
        tx
    };
    tx.commit().unwrap();
    let db = open_file(&path);
    assert_eq!(
        rows(&db).len(),
        5,
        "the commit after the last handle dropped"
    );
}

/// Run `f` on a new thread; `Some` of its result if it returns within `t`.
fn within<T: Send + 'static>(
    t: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> (Option<T>, mpsc::Receiver<T>) {
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(f());
    });
    match recv.recv_timeout(t) {
        Ok(v) => (Some(v), recv),
        Err(_) => (None, recv),
    }
}

#[test]
fn owned_tx_other_thread_write_waits() {
    type Call = fn(&Minigraf) -> Result<(), minigraf::MinigrafError>;
    let calls: &[(&str, Call)] = &[
        ("execute transact", |db| {
            db.execute(r#"(transact [[:w :person/name "W"]])"#)
                .map(|_| ())
        }),
        ("execute rule", |db| {
            db.execute("(rule [(named2 ?e ?n) [?e :person/name ?n]])")
                .map(|_| ())
        }),
        ("checkpoint", Minigraf::checkpoint),
        ("verify", |db| db.verify().map(|_| ())),
        ("begin_write", |db| db.begin_write()?.commit()),
        ("begin_write_owned", |db| db.begin_write_owned()?.commit()),
    ];
    for (name, call) in calls {
        let dir = tempfile::tempdir().unwrap();
        let db = file_db(&dir);
        let mut tx = db.begin_write_owned().unwrap();
        tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
            .unwrap();
        let db2 = db.clone();
        let call = *call;
        let (early, rest) = within(WAITS, move || call(&db2).map_err(|e| e.code()));
        assert!(early.is_none(), "{name} from another thread did not wait");
        tx.commit().unwrap();
        let result = rest
            .recv_timeout(STUCK)
            .expect("the waiting call returns after the commit");
        assert_eq!(result, Ok(()), "{name} after the commit");
    }
}

#[test]
fn owned_tx_other_thread_query_reads_committed() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    let pq = db
        .prepare("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    tx.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
        .unwrap();
    let staged = outcome(tx.execute("(query [:find ?n :where [?e :person/name ?n]])"));
    let db2 = db.clone();
    let (got, _) = within(STUCK, move || {
        let names = outcome(db2.execute("(query [:find ?n :where [?e :person/name ?n]])"));
        let cursor_rows = db2
            .query("(query [:find ?n :where [?e :person/name ?n]])")
            .unwrap()
            .count();
        let prepared = outcome(pq.execute(&[]));
        let log = db2.fact_log(&FactFilter::new()).unwrap().count();
        (names, cursor_rows, prepared, log)
    });
    let (names, cursor_rows, prepared, log) = got.expect("queries from another thread do not wait");
    assert_eq!(
        names,
        r#"[String("Alice")];[String("Bob")];[String("Carol")]"#
    );
    assert_eq!(cursor_rows, 3, "a cursor reads the committed state");
    assert_eq!(
        prepared, names,
        "a prepared query reads the committed state"
    );
    assert_eq!(log, 4, "the fact log holds the committed records");
    assert_eq!(
        staged, r#"[String("Alice")];[String("Carol")];[String("Dave")]"#,
        "the transaction reads its staged state"
    );
    tx.commit().unwrap();
}

#[test]
fn owned_tx_leaves_other_database_usable() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    let other = Minigraf::in_memory().unwrap();
    let pq = other
        .prepare("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    let tx = db.begin_write_owned().unwrap();
    // On the begin thread and on another, the other database is not guarded
    // and does not wait.
    assert_all(&guarded_calls(&other), None, "on another database");
    assert_all(&prepared_calls(&pq), None, "on another database");
    let (got, _) = within(STUCK, move || {
        other
            .execute(r#"(transact [[:q :person/name "Q"]])"#)
            .map(|_| ())
            .map_err(|e| e.code())
    });
    assert_eq!(got, Some(Ok(())), "another database's write does not wait");
    tx.rollback();
}

// ─── Begin refusals (A5) ─────────────────────────────────────────────────────

#[test]
fn owned_begin_refused_inside_a_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir);
    {
        let tx = db.begin_write().unwrap();
        let err = db.begin_write_owned().expect_err("inside a borrowed one");
        assert_eq!(err.code(), "INT-001");
        tx.commit().unwrap();
    }
    {
        let tx = db.begin_write_owned().unwrap();
        let err = db.begin_write_owned().expect_err("inside an owned one");
        assert_eq!(err.code(), "INT-001");
        let err = db
            .begin_write_owned_with(TxOptions::new().chunk_facts(5))
            .expect_err("inside an owned one, with options");
        assert_eq!(err.code(), "INT-001");
        tx.commit().unwrap();
    }
    // The handle stays usable.
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(rows(&db).len(), 5);
}

#[test]
fn owned_begin_refused_on_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.graph");
    drop(file_db(&dir));
    let db = OpenOptions::new()
        .read_only(true)
        .path(&path)
        .open()
        .unwrap();
    let err = db.begin_write_owned().expect_err("read-only");
    assert_eq!(err.code(), "API-014");
    let err = db
        .begin_write_owned_with(TxOptions::new())
        .expect_err("read-only");
    assert_eq!(err.code(), "API-014");
    assert_eq!(rows(&db).len(), 4, "the handle still reads");
    let mem = OpenOptions::new().read_only(true).open_memory().unwrap();
    assert_eq!(
        mem.begin_write_owned().expect_err("read-only").code(),
        "API-014"
    );
}

// ─── Edge cases ──────────────────────────────────────────────────────────────

#[test]
fn owned_tx_empty_and_query_only() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(SETUP).unwrap();
    let before = db.current_tx_count();
    db.begin_write_owned().unwrap().commit().unwrap();
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    tx.commit().unwrap();
    let mut tx = db.begin_write_owned().unwrap();
    tx.execute("(query [:find ?n :where [?e :person/name ?n]])")
        .unwrap();
    tx.rollback();
    assert_eq!(db.current_tx_count(), before, "no transaction taken");
}

/// A statement that fails stages nothing and leaves the transaction open.
#[test]
fn owned_tx_failed_statement_stays_open() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write_owned().unwrap();
    assert_eq!(
        tx.execute("(transact [[:a").expect_err("parse").code(),
        "PRS-004"
    );
    let err = tx
        .execute(r#"(transact [[:x :p/v 1 {:valid-from "2020-01-01T00:00:00Z"}] [:x :p/v 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#)
        .expect_err("two windows of a triple");
    assert_eq!(err.code(), "API-011");
    tx.execute(r#"(transact [[:a :p/n "A"]])"#).unwrap();
    tx.commit().unwrap();
    assert_eq!(rows(&db).len(), 1, "only the good statement");
}
