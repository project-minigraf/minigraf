//! `OpenOptions::read_only` (#429): a handle that shows the same data as a
//! read-write open, writes nothing to the `.graph` file or its WAL, refuses
//! writes with API-014, and shares the file with other read-only handles.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{FactFilter, Minigraf, OpenOptions, QueryResult};
use std::path::{Path, PathBuf};

fn wal_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".wal");
    PathBuf::from(p)
}

fn ro() -> OpenOptions {
    OpenOptions::new().read_only(true)
}

/// The error from an open that must fail (`Minigraf` is not `Debug`).
fn open_err(
    result: Result<Minigraf, minigraf::MinigrafError>,
    what: &str,
) -> minigraf::MinigrafError {
    match result {
        Ok(_) => panic!("{what}: open must fail"),
        Err(e) => e,
    }
}

fn open_ro(path: &Path) -> Minigraf {
    Minigraf::open_with_options(path, ro()).expect("read-only open")
}

const NAMES: &str = "(query [:find ?n :where [?e :person/name ?n]])";

fn names(db: &Minigraf) -> Vec<String> {
    match db.execute(NAMES).unwrap() {
        QueryResult::QueryResults { results, .. } => {
            let mut v: Vec<String> = results
                .iter()
                .map(|r| match &r[0] {
                    minigraf::Value::String(s) => s.clone(),
                    _ => panic!("expected a string"),
                })
                .collect();
            v.sort();
            v
        }
        _ => panic!("expected query results"),
    }
}

/// A file whose facts are all checkpointed, with no WAL: alice and carol
/// current, bob asserted then retracted. Returns its tx count.
fn checkpointed(path: &Path) -> u64 {
    let db = Minigraf::open(path).unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();
    db.execute(r#"(transact [[:bob :person/name "Bob"]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    db.execute(r#"(transact [[:carol :person/name "Carol"]])"#)
        .unwrap();
    db.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
        .unwrap();
    // A clean close checkpoints and removes the WAL.
    db.current_tx_count()
}

/// The same facts, but the last two transactions stay in the WAL.
fn with_pending_wal(path: &Path) -> u64 {
    {
        let db = Minigraf::open(path).unwrap();
        db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();
        db.execute(r#"(transact [[:bob :person/name "Bob"]])"#)
            .unwrap();
    }
    let db = Minigraf::open_with_options(
        path,
        OpenOptions::new().wal_checkpoint_threshold(usize::MAX),
    )
    .unwrap();
    db.execute(r#"(transact [[:carol :person/name "Carol"]])"#)
        .unwrap();
    db.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
        .unwrap();
    db.current_tx_count()
}

struct Snapshot {
    graph: Vec<u8>,
    wal: Option<Vec<u8>>,
    graph_mtime: std::time::SystemTime,
}

fn snapshot(path: &Path) -> Snapshot {
    Snapshot {
        graph: std::fs::read(path).unwrap(),
        wal: std::fs::read(wal_path(path)).ok(),
        graph_mtime: std::fs::metadata(path).unwrap().modified().unwrap(),
    }
}

fn assert_unchanged(path: &Path, before: &Snapshot) {
    let after = snapshot(path);
    assert!(after.graph == before.graph, ".graph bytes changed");
    assert!(after.wal == before.wal, "WAL changed, appeared or vanished");
    assert_eq!(
        after.graph_mtime, before.graph_mtime,
        ".graph mtime changed"
    );
}

#[test]
fn shows_the_same_data_as_a_read_write_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same.graph");
    let tx = with_pending_wal(&path);
    assert!(wal_path(&path).exists(), "fixture needs a pending WAL");

    let before = snapshot(&path);
    let (ro_names, ro_tx, ro_log) = {
        let db = open_ro(&path);
        let log: Vec<_> = db
            .fact_log(&FactFilter::new())
            .unwrap()
            .map(|r| {
                let r = r.unwrap();
                (r.tx_count, r.attribute, r.asserted)
            })
            .collect();
        (names(&db), db.current_tx_count(), log)
    };
    assert_unchanged(&path, &before);

    assert_eq!(ro_names, vec!["Alice".to_string(), "Carol".to_string()]);
    assert_eq!(ro_tx, tx);
    assert_eq!(ro_log.len(), 4, "3 assertions + 1 retraction");

    let db = Minigraf::open(&path).unwrap();
    assert_eq!(names(&db), ro_names);
    assert_eq!(db.current_tx_count(), ro_tx);
    let rw_log: Vec<_> = db
        .fact_log(&FactFilter::new())
        .unwrap()
        .map(|r| {
            let r = r.unwrap();
            (r.tx_count, r.attribute, r.asserted)
        })
        .collect();
    assert_eq!(rw_log, ro_log);
}

#[test]
fn reads_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("untouched.graph");
    with_pending_wal(&path);
    let before = snapshot(&path);
    {
        let db = open_ro(&path);
        names(&db);
        let mut cursor = db.query(NAMES).unwrap();
        while cursor.next_batch(1).unwrap().is_some() {}
        let log = db.fact_log(&FactFilter::new()).unwrap();
        assert_eq!(log.count(), 4);
        let report = db.verify().unwrap();
        assert!(report.is_ok(), "verify finds no problems");
        let p = db.prepare("(query [:find ?n :where [?e :person/name ?n]])");
        p.unwrap();
    }
    assert_unchanged(&path, &before);
}

#[test]
fn no_wal_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nowal.graph");
    checkpointed(&path);
    assert!(!wal_path(&path).exists());
    let before = snapshot(&path);
    {
        let db = open_ro(&path);
        assert_eq!(names(&db), vec!["Alice".to_string(), "Carol".to_string()]);
        assert!(
            db.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
                .is_err()
        );
    }
    assert!(!wal_path(&path).exists(), "no WAL created");
    assert_unchanged(&path, &before);
}

#[test]
fn writes_are_api_014_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("refuse.graph");
    let tx = with_pending_wal(&path);
    let before = snapshot(&path);
    {
        let db = open_ro(&path);
        let cases: [(&str, Result<(), minigraf::MinigrafError>); 5] = [
            (
                "transact",
                db.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
                    .map(|_| ()),
            ),
            (
                "retract",
                db.execute(r#"(retract [[:alice :person/name "Alice"]])"#)
                    .map(|_| ()),
            ),
            ("begin_write", db.begin_write().map(|_| ())),
            ("checkpoint", db.checkpoint()),
            ("rebuild_indexes", db.rebuild_indexes()),
        ];
        for (op, result) in cases {
            let err = result.expect_err(op);
            assert_eq!(err.code(), "API-014", "{op}");
            assert!(err.to_string().contains(op), "{op} named in the message");
        }
        assert_eq!(db.current_tx_count(), tx, "no tx count allocated");
        assert_eq!(names(&db), vec!["Alice".to_string(), "Carol".to_string()]);
    }
    assert_unchanged(&path, &before);
}

#[test]
fn rules_and_functions_work() {
    const RULES: [&str; 2] = [
        "(rule [(reach ?x ?y) [?x :link ?y]])",
        "(rule [(reach ?x ?y) [?x :link ?z] (reach ?z ?y)])",
    ];
    const REACH: &str = "(query [:find ?x ?y :where (reach ?x ?y)])";
    let reach = |db: &Minigraf| -> usize {
        for r in RULES {
            db.execute(r).unwrap();
        }
        match db.execute(REACH).unwrap() {
            QueryResult::QueryResults { results, .. } => results.len(),
            _ => panic!("expected query results"),
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rules.graph");
    let expected = {
        let db = Minigraf::open(&path).unwrap();
        db.execute("(transact [[:a :link :b] [:b :link :c]])")
            .unwrap();
        db.checkpoint().unwrap();
        reach(&db)
    };
    assert!(expected > 0);
    let before = snapshot(&path);
    {
        let db = open_ro(&path);
        assert_eq!(reach(&db), expected);
        db.register_predicate("always?", |_| true).unwrap();
    }
    assert_unchanged(&path, &before);
}

#[test]
fn missing_file_is_stg_042_and_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.graph");
    let err = open_err(Minigraf::open_with_options(&path, ro()), "missing file");
    assert_eq!(err.code(), "STG-042");
    assert!(!path.exists(), "no file created");
    assert!(!wal_path(&path).exists(), "no WAL created");
}

#[test]
fn empty_file_is_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.graph");
    std::fs::write(&path, b"").unwrap();
    {
        let db = open_ro(&path);
        assert!(names(&db).is_empty());
        assert_eq!(db.current_tx_count(), 0);
        assert_eq!(db.fact_log(&FactFilter::new()).unwrap().count(), 0);
    }
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        0,
        "not initialised"
    );
}

#[test]
fn read_only_handles_share_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.graph");
    with_pending_wal(&path);
    let a = open_ro(&path);
    let b = open_ro(&path);
    assert_eq!(names(&a), names(&b));
    drop(a);
    let c = open_ro(&path);
    assert_eq!(names(&b), names(&c));
}

#[test]
fn readers_and_a_writer_exclude_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exclusive.graph");
    with_pending_wal(&path);

    let r1 = open_ro(&path);
    let r2 = open_ro(&path);
    let err = open_err(Minigraf::open(&path), "writer while readers hold the file");
    assert_eq!(err.code(), "STG-025");
    drop(r1);
    let err = open_err(
        Minigraf::open(&path),
        "writer while one reader holds the file",
    );
    assert_eq!(err.code(), "STG-025");
    drop(r2);

    let w = Minigraf::open(&path).unwrap();
    let err = open_err(
        Minigraf::open_with_options(&path, ro()),
        "reader while a writer holds it",
    );
    assert_eq!(err.code(), "STG-025");
    drop(w);
    open_ro(&path);
}

#[test]
fn page_cache_size_applies_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.graph");
    checkpointed(&path);
    let pages = std::fs::metadata(&path).unwrap().len() / 4096;
    for size in [1, pages as usize] {
        let db = Minigraf::open_with_options(&path, ro().page_cache_size(size)).unwrap();
        assert_eq!(names(&db), vec!["Alice".to_string(), "Carol".to_string()]);
    }
}

#[cfg(unix)]
#[test]
fn opens_a_file_without_write_permission() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("perm.graph");
    checkpointed(&path);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    // Root ignores file modes, so the read-write open below would succeed.
    let as_root = std::fs::OpenOptions::new().write(true).open(&path).is_ok();
    if !as_root {
        assert!(
            Minigraf::open(&path).is_err(),
            "read-write open needs write access"
        );
    }
    let db = open_ro(&path);
    assert_eq!(names(&db), vec!["Alice".to_string(), "Carol".to_string()]);
}

#[test]
fn in_memory_read_only_refuses_writes() {
    let db = OpenOptions::new().read_only(true).open_memory().unwrap();
    let err = db
        .execute(r#"(transact [[:a :person/name "A"]])"#)
        .expect_err("transact");
    assert_eq!(err.code(), "API-014");
    assert_eq!(
        db.begin_write()
            .map(|_| ())
            .expect_err("begin_write")
            .code(),
        "API-014"
    );
    assert_eq!(db.checkpoint().expect_err("checkpoint").code(), "API-014");
    assert!(names(&db).is_empty());
}

#[test]
fn builder_path_open_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("builder.graph");
    checkpointed(&path);
    let db = ro().path(&path).open().unwrap();
    assert_eq!(
        db.execute(r#"(transact [[:x :person/name "X"]])"#)
            .expect_err("transact")
            .code(),
        "API-014"
    );
}
