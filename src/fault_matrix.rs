//! Fault-injection matrix through the whole `Minigraf` write path (#390).
//!
//! Each case builds a file database (checkpointed facts, multi-valued and
//! retracted facts, long values, entries pending in the WAL), arms one fault
//! at the `k`-th write or sync of one operation, runs the operation, and
//! then:
//!
//! - checks that a later write either succeeds or is refused with WAL-007 or
//!   STG-046, and that after a failed checkpoint another one is STG-046
//!   (never retried);
//! - kills the handle (no close-time checkpoint) and reopens with working
//!   I/O: the facts are exactly what the handle held, plus, only when the
//!   transaction's own write failed, that whole transaction or none of it.
//!   `verify` finds nothing;
//! - commits once more, kills, reopens, and finds that commit too.
//!
//! `k` runs from 0 until the operation finishes without reaching the fault.

use crate::db::{Minigraf, OpenOptions};
use crate::fact_log::FactFilter;
use crate::storage::fault::{self, Fault, Site};
use std::path::Path;

/// One fact version, comparable across handles.
type Rec = (String, String, String, u64, u64, i64, i64, bool);

fn snapshot(db: &Minigraf) -> Vec<Rec> {
    let mut v: Vec<Rec> = db
        .fact_log(&FactFilter::new())
        .unwrap()
        .map(|r| {
            let r = r.unwrap();
            (
                r.entity.to_string(),
                r.attribute,
                format!("{:?}", r.value),
                r.tx_count,
                r.tx_id,
                r.valid_from,
                r.valid_to,
                r.asserted,
            )
        })
        .collect();
    v.sort();
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// A transact appended to an existing WAL.
    Transact,
    /// A transact that creates the WAL (header, directory sync).
    TransactNewWal,
    /// A transact whose WAL entry reaches the checkpoint threshold.
    AutoCheckpoint,
    /// `checkpoint()` with entries pending.
    Checkpoint,
    /// Dropping the handle: the close-time checkpoint.
    Close,
    /// `rebuild_indexes()` with entries pending.
    Rebuild,
}

impl Op {
    /// Whether the operation writes its own transaction (`X`).
    fn writes_x(self) -> bool {
        matches!(self, Op::Transact | Op::TransactNewWal | Op::AutoCheckpoint)
    }
}

const X: &str = r#"(transact [[:x1 :p/x 1] [:x1 :p/x 2] [:x2 :p/xlong "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"]])"#;
const X_FACTS: usize = 3;

fn open(path: &Path, op: Op) -> Minigraf {
    let threshold = if op == Op::AutoCheckpoint { 3 } else { 1000 };
    OpenOptions::new()
        .wal_checkpoint_threshold(threshold)
        .path(path)
        .open()
        .expect("open")
}

/// `n` facts on entities `<prefix>0..n`, some with long values.
fn bulk(prefix: &str, n: usize) -> String {
    let mut s = String::from("(transact [");
    for i in 0..n {
        s.push_str(&format!(
            "[:{prefix}{i} :p/n {i}] [:{prefix}{i} :p/name \"name {prefix} {i}\"]"
        ));
        if i % 25 == 0 {
            s.push_str(&format!(
                "[:{prefix}{i} :p/long \"{}\"]",
                "l".repeat(80 + i % 7)
            ));
        }
    }
    s.push_str("])");
    s
}

/// A database committed over two checkpoints (multi-level trees, a free
/// list, multi-valued and retracted facts, long values) and, except for
/// `TransactNewWal`, two entries pending in the WAL.
fn build(path: &Path, op: Op) -> Minigraf {
    let db = open(path, op);
    db.execute(&bulk("a", 150)).unwrap();
    db.execute(r#"(transact [[:e1 :p/tag "a"] [:e1 :p/tag "b"] [:e2 :p/n 1]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    db.execute(&bulk("b", 150)).unwrap();
    db.execute(r#"(retract [[:e1 :p/tag "a"] [:a7 :p/n 7]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    if op != Op::TransactNewWal {
        db.execute(&bulk("c", 20)).unwrap();
        db.execute(r#"(transact [[:e1 :p/tag "c"] [:e3 :p/ref :e1]])"#)
            .unwrap();
    }
    db
}

/// The records `after` has beyond `before`, if `before` is all in `after`.
fn extra(before: &[Rec], after: &[Rec]) -> Option<Vec<Rec>> {
    let mut rest = after.to_vec();
    for r in before {
        let i = rest.iter().position(|x| x == r)?;
        rest.remove(i);
    }
    Some(rest)
}

/// Whether `recs` is exactly one whole `X` transaction.
fn is_whole_x(recs: &[Rec]) -> bool {
    recs.len() == X_FACTS
        && recs.iter().all(|r| r.1.starts_with(":p/x") && r.7)
        && recs.iter().all(|r| r.3 == recs[0].3)
}

fn code(e: crate::MinigrafError) -> &'static str {
    e.code()
}

fn reopen_and_check(path: &Path, op: Op, live: &[Rec], x_failed: bool) {
    let db = open(path, op);
    let reopened = snapshot(&db);
    let more = extra(live, &reopened).expect("a fact the handle held was lost");
    assert!(
        more.is_empty() || (x_failed && is_whole_x(&more)),
        "reopened file holds facts the handle never committed, or part of one"
    );
    let report = db.verify().unwrap();
    assert!(report.problems.is_empty(), "verify after reopen");

    // One more commit after recovery must survive a kill.
    db.execute(r#"(transact [[:z :p/z "after"]])"#).unwrap();
    let with_z = snapshot(&db);
    db.kill();
    let db = open(path, op);
    assert!(snapshot(&db) == with_z, "a commit after recovery was lost");
    assert!(db.verify().unwrap().problems.is_empty(), "verify after Z");
    db.kill();
}

/// Run `op` with `fault` at every call index. Returns how many indexes hit.
fn run(op: Op, f: Fault) -> u64 {
    let mut hits = 0;
    for k in 0u64.. {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.graph");
        let db = build(&path, op);
        let before = snapshot(&db);

        fault::arm(f, k);
        let (result, db) = match op {
            Op::Transact | Op::TransactNewWal | Op::AutoCheckpoint => {
                (db.execute(X).map(|_| ()), Some(db))
            }
            Op::Checkpoint => (db.checkpoint(), Some(db)),
            Op::Rebuild => (db.rebuild_indexes(), Some(db)),
            Op::Close => {
                drop(db);
                (Ok(()), None)
            }
        };
        let tripped = fault::disarm();

        let live = match db {
            Some(db) => {
                let live = snapshot(&db);
                let applied = extra(&before, &live).expect("the handle lost a fact");
                if op.writes_x() {
                    assert!(
                        applied.is_empty() || is_whole_x(&applied),
                        "the handle applied part of a transaction"
                    );
                    if result.is_ok() {
                        assert!(is_whole_x(&applied), "a committed transaction is missing");
                    }
                } else {
                    assert!(applied.is_empty(), "a checkpoint changed the facts");
                }

                let checkpoint_failed =
                    result.is_err() && matches!(tripped, Some(Site::PageWrite | Site::PageSync));
                if checkpoint_failed {
                    let err = db.checkpoint().expect_err("a failed checkpoint is retried");
                    assert_eq!(code(err), "STG-046");
                }

                // A later write succeeds, or is refused as failed.
                let live = match db.execute(r#"(transact [[:y :p/y "later"]])"#) {
                    Ok(_) => snapshot(&db),
                    Err(e) => {
                        let c = code(e);
                        assert!(c == "WAL-007" || c == "STG-046", "later write: {c}");
                        assert!(result.is_err(), "a write refused after a success");

                        live
                    }
                };
                db.kill();
                live
            }
            // The close-time checkpoint's errors are not reported.
            None => before,
        };

        let x_failed = op.writes_x() && result.is_err();
        reopen_and_check(&path, op, &live, x_failed);

        if tripped.is_none() {
            assert!(result.is_ok(), "failed without a fault");
            break;
        }
        hits += 1;
    }
    hits
}

const FAULTS: [Fault; 7] = [
    Fault::Eio,
    Fault::Torn(1),
    Fault::Torn(40),
    Fault::Torn(4095),
    Fault::Enospc,
    Fault::SyncEio,
    Fault::SyncLost,
];

fn run_all(op: Op) {
    for f in FAULTS {
        let hits = run(op, f);
        assert!(hits > 0, "the fault never fired");
    }
}

#[test]
fn faults_during_transact() {
    run_all(Op::Transact);
}

#[test]
fn faults_during_transact_that_creates_the_wal() {
    run_all(Op::TransactNewWal);
}

#[test]
fn faults_during_auto_checkpoint() {
    run_all(Op::AutoCheckpoint);
}

#[test]
fn faults_during_checkpoint() {
    run_all(Op::Checkpoint);
}

#[test]
fn faults_during_close() {
    run_all(Op::Close);
}

#[test]
fn faults_during_rebuild_indexes() {
    run_all(Op::Rebuild);
}

/// The disk fills part-way through a checkpoint: every write from then on
/// fails. The checkpoint fails, the handle refuses writes with STG-046, and
/// after space is freed a reopen holds every committed fact.
#[test]
fn disk_full_halfway_through_a_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.graph");
    let db = build(&path, Op::Checkpoint);
    for i in 0..200 {
        db.execute(&format!("(transact [[:bulk{i} :p/n {i}]])"))
            .unwrap();
    }
    let live = snapshot(&db);

    // Count the checkpoint's page writes on a copy, then fill the disk at
    // half of them.
    let writes = {
        let copy_dir = tempfile::tempdir().unwrap();
        let copy = copy_dir.path().join("f.graph");
        std::fs::copy(&path, &copy).unwrap();
        let mut wal = copy.as_os_str().to_owned();
        wal.push(".wal");
        let mut src_wal = path.as_os_str().to_owned();
        src_wal.push(".wal");
        std::fs::copy(&src_wal, &wal).unwrap();
        let db2 = open(&copy, Op::Checkpoint);
        fault::arm(Fault::Eio, u64::MAX);
        db2.checkpoint().unwrap();
        let n = fault::count();
        fault::disarm();
        db2.kill();
        n
    };
    assert!(writes > 4, "the checkpoint writes several pages");

    fault::arm(Fault::Enospc, writes / 2);
    let err = db.checkpoint().expect_err("the disk is full");
    assert_eq!(fault::disarm(), Some(Site::PageWrite));
    let c = code(err);
    assert!(c.starts_with("INT") || c.starts_with("STG"), "{c}");
    let err = db
        .execute("(transact [[:late :p/n 1]])")
        .expect_err("writes are refused");
    assert_eq!(code(err), "STG-046");
    assert!(snapshot(&db) == live, "reads still see every fact");
    db.kill();

    reopen_and_check(&path, Op::Checkpoint, &live, false);
}

/// How many calls `fault` targets during a clean checkpoint of a copy of
/// the database at `path`.
fn checkpoint_calls(path: &Path, f: Fault) -> u64 {
    let copy_dir = tempfile::tempdir().unwrap();
    let copy = copy_dir.path().join("f.graph");
    std::fs::copy(path, &copy).unwrap();
    let mut wal = copy.as_os_str().to_owned();
    wal.push(".wal");
    let mut src_wal = path.as_os_str().to_owned();
    src_wal.push(".wal");
    if Path::new(&src_wal).exists() {
        std::fs::copy(&src_wal, &wal).unwrap();
    }
    let db = open(&copy, Op::Checkpoint);
    fault::arm(f, u64::MAX);
    db.checkpoint().unwrap();
    let n = fault::count();
    fault::disarm();
    db.kill();
    n
}

/// The fsyncgate case: the checkpoint's last fsync (after its meta page
/// write) fails, but the meta page reached the file. The handle must not
/// commit again: a retry would write the same generation into pages that
/// meta references, and a crash during it would leave the file pointing
/// at them. Here a later write and a retried checkpoint (crashing at each
/// of its writes) are both refused, and every reopen is intact.
#[test]
fn no_commit_after_a_failed_meta_fsync() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.graph");
    build(&base, Op::Checkpoint).kill();
    let syncs = checkpoint_calls(&base, Fault::SyncEio);
    let writes = checkpoint_calls(&base, Fault::Eio);
    let mut base_wal = base.as_os_str().to_owned();
    base_wal.push(".wal");

    for j in 0..writes {
        let case = tempfile::tempdir().unwrap();
        let path = case.path().join("f.graph");
        std::fs::copy(&base, &path).unwrap();
        let mut wal = path.as_os_str().to_owned();
        wal.push(".wal");
        std::fs::copy(&base_wal, &wal).unwrap();

        let db = open(&path, Op::Checkpoint);
        let live = snapshot(&db);
        // The last two syncs are the meta page's and the directory's,
        // after the WAL is removed.
        fault::arm(Fault::SyncEio, syncs - 2);
        db.checkpoint().expect_err("the last fsync fails");
        assert_eq!(fault::disarm(), Some(Site::PageSync));

        let live = match db.execute(r#"(transact [[:y :p/y "later"] [:a3 :p/n 33]])"#) {
            Ok(_) => snapshot(&db),
            Err(e) => {
                assert_eq!(code(e), "STG-046");
                live
            }
        };
        fault::arm(Fault::Torn(2048), j);
        if let Err(e) = db.checkpoint() {
            if fault::disarm().is_none() {
                assert_eq!(code(e), "STG-046");
            }
        } else {
            fault::disarm();
        }
        db.kill();
        reopen_and_check(&path, Op::Checkpoint, &live, false);
    }
}
