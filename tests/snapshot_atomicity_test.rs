//! Reproducers: a query must see each transaction whole, and each fact once,
//! while other threads commit and checkpoint (#473 spec §2 and §4.1).
//!
//! Each test runs a writer thread against a reader thread for a fixed time and
//! fails on the first answer that breaks the invariant.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, OpenOptions, QueryResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_FOR: Duration = Duration::from_secs(3);

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<String>> {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => results
            .iter()
            .map(|r| {
                r.iter()
                    .map(|v| match v {
                        minigraf::Value::Integer(n) => n.to_string(),
                        minigraf::Value::Keyword(k) => k.clone(),
                        minigraf::Value::String(s) => s.clone(),
                        _ => "other".to_string(),
                    })
                    .collect()
            })
            .collect(),
        _ => panic!("expected query results"),
    }
}

/// Runs `write` in a loop on one thread and `check` in a loop on this one
/// until `RUN_FOR` passes. `check` returns an error message on a violation.
fn race(
    db: Arc<Minigraf>,
    mut write: impl FnMut(&Minigraf, u64) + Send + 'static,
    mut check: impl FnMut(&Minigraf) -> Option<String>,
) -> Option<String> {
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (db, stop) = (Arc::clone(&db), Arc::clone(&stop));
        thread::spawn(move || {
            let mut i = 0;
            while !stop.load(Ordering::Relaxed) {
                write(&db, i);
                i += 1;
            }
        })
    };
    let start = Instant::now();
    let mut found = None;
    while start.elapsed() < RUN_FOR {
        if let Some(msg) = check(&db) {
            found = Some(msg);
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    found
}

const N: u64 = 500;

fn batch(i: u64) -> String {
    let facts: String = (0..N)
        .map(|j| format!("[:b{i}x{j} :batch/id {i}]"))
        .collect();
    format!("(transact [{facts}])")
}

/// Every write commits `N` facts in one transaction; a query must never see
/// a count that is not a multiple of `N` (#554).
fn commit_is_seen_whole(db: Minigraf, write: impl FnMut(&Minigraf, u64) + Send + 'static) {
    let found = race(Arc::new(db), write, |db| {
        let n = rows(db, "(query [:find ?e :where [?e :batch/id ?b]])").len() as u64;
        (n % N != 0).then(|| format!("saw {n} facts, not a multiple of {N}"))
    });
    if let Some(msg) = found {
        panic!("partial transaction visible: {msg}");
    }
}

fn write_transaction(db: &Minigraf, i: u64) {
    let mut tx = db.begin_write().unwrap();
    tx.execute(&batch(i)).unwrap();
    tx.commit().unwrap();
}

fn execute_transact(db: &Minigraf, i: u64) {
    db.execute(&batch(i)).unwrap();
}

fn file_db(dir: &tempfile::TempDir) -> Minigraf {
    let opts = OpenOptions::default().wal_checkpoint_threshold(usize::MAX);
    Minigraf::open_with_options(dir.path().join("w.graph"), opts).unwrap()
}

#[test]
fn write_transaction_commit_is_seen_whole_in_memory() {
    commit_is_seen_whole(Minigraf::in_memory().unwrap(), write_transaction);
}

#[test]
fn write_transaction_commit_is_seen_whole_on_file() {
    let dir = tempfile::tempdir().unwrap();
    commit_is_seen_whole(file_db(&dir), write_transaction);
}

#[test]
fn execute_transact_is_seen_whole_in_memory() {
    commit_is_seen_whole(Minigraf::in_memory().unwrap(), execute_transact);
}

#[test]
fn execute_transact_is_seen_whole_on_file() {
    let dir = tempfile::tempdir().unwrap();
    commit_is_seen_whole(file_db(&dir), execute_transact);
}

/// A checkpoint sets the new committed reader and clears the pending store
/// under two separate locks, so a query in between can read the checkpointed
/// facts twice. Every fact here has its own entity, so a repeated row is a
/// fact read twice.
#[test]
#[ignore = "reproducer: fails on v3 today; fixed by #473 sub-issue #543"]
fn checkpoint_never_shows_a_fact_twice() {
    let dir = tempfile::tempdir().unwrap();
    let opts = OpenOptions::default().wal_checkpoint_threshold(usize::MAX);
    let db = Minigraf::open_with_options(dir.path().join("c.graph"), opts).unwrap();
    let found = race(
        Arc::new(db),
        |db, i| {
            db.execute(&format!("(transact [[:c{i} :ck/n {i}]])"))
                .unwrap();
            db.checkpoint().unwrap();
        },
        |db| {
            for q in [
                "(query [:find ?e ?n :where [?e :ck/n ?n]])",
                "(query [:find ?e ?n :valid-at :any-valid-time :where [?e :ck/n ?n]])",
            ] {
                let mut r = rows(db, q);
                let n = r.len();
                r.sort();
                r.dedup();
                if r.len() != n {
                    return Some(format!("{n} rows, {} distinct: {q}", r.len()));
                }
            }
            None
        },
    );
    if let Some(msg) = found {
        panic!("fact seen twice during a checkpoint: {msg}");
    }
}

/// A query fetched each attribute with its own `get_live_facts` call, each
/// under its own read lock, so a transaction that committed between the two
/// calls was seen by one pattern and not the other (#555). Every transaction
/// writes `:pair/x` and `:pair/y` for one entity together, atomically (#554).
/// Attributes are fetched in sorted order, so only the query whose positive
/// pattern is `:pair/y` can catch a commit between the fetches; both are run.
fn patterns_see_one_state(db: Minigraf) {
    let found = race(
        Arc::new(db),
        |db, i| {
            db.execute(&format!(
                "(transact [[:p{i} :pair/x {i}] [:p{i} :pair/y {i}]])"
            ))
            .unwrap();
        },
        |db| {
            for q in [
                "(query [:find ?e :where [?e :pair/x ?v] (not [?e :pair/y ?v])])",
                "(query [:find ?e :where [?e :pair/y ?v] (not [?e :pair/x ?v])])",
            ] {
                let n = rows(db, q).len();
                if n != 0 {
                    return Some(format!("{n} entities with one attribute of the pair: {q}"));
                }
            }
            None
        },
    );
    if let Some(msg) = found {
        panic!("one query saw two database states: {msg}");
    }
}

#[test]
fn patterns_of_one_query_see_one_state_in_memory() {
    patterns_see_one_state(Minigraf::in_memory().unwrap());
}

#[test]
fn patterns_of_one_query_see_one_state_on_file() {
    let dir = tempfile::tempdir().unwrap();
    patterns_see_one_state(Minigraf::open(dir.path().join("p.graph")).unwrap());
}

/// A checkpoint wrote its new fact pages over the index pages queries were
/// reading, and published its new readers in three locked steps, so a query
/// during a checkpoint could miss committed facts or see half a transaction
/// (#558, v2.x only: v3's copy-on-write checkpoints never overwrite pages
/// a published reader uses). Every write here is followed by a checkpoint.
#[test]
fn checkpoint_never_hides_committed_facts() {
    let dir = tempfile::tempdir().unwrap();
    let committed = Arc::new(AtomicU64::new(0));
    let found = {
        let written = Arc::clone(&committed);
        race(
            Arc::new(file_db(&dir)),
            move |db, i| {
                db.execute(&format!(
                    "(transact [[:c{i} :pair/x {i}] [:c{i} :pair/y {i}]])"
                ))
                .unwrap();
                written.store(i + 1, Ordering::SeqCst);
                db.checkpoint().unwrap();
            },
            |db| {
                let before = committed.load(Ordering::SeqCst) as usize;
                let half = rows(
                    db,
                    "(query [:find ?e :where [?e :pair/y ?v] (not [?e :pair/x ?v])])",
                )
                .len();
                let seen = rows(db, "(query [:find ?e :where [?e :pair/x ?v]])").len();
                if half != 0 {
                    Some(format!("{half} entities with y and no x"))
                } else if seen < before {
                    Some(format!("{seen} entities seen, {before} committed before"))
                } else {
                    None
                }
            },
        )
    };
    if let Some(msg) = found {
        panic!("a query during a checkpoint saw the wrong state: {msg}");
    }
}
