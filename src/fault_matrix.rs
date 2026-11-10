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
//!
//! The mixed-mode operations ([`Mix`], #871) commit `X` through
//! `begin_write_with` with a WAL sync mode other than the database's, or
//! after entries committed without a sync. A lost sync may then lose those
//! entries, so their reopen check is the prefix guarantee instead.

use crate::db::{Minigraf, OpenOptions, SyncMode, TxOptions};
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
    /// A large transaction (#550): chunks written into working trees and
    /// committed as a checkpoint. `one_slot`: on a file never checkpointed,
    /// with one valid meta slot; else on the file `build` makes. `wal`: with
    /// entries pending in the WAL beforehand, else with no WAL. `tx`: as a
    /// `WriteTransaction` of several statements (statement bases).
    Large { one_slot: bool, wal: bool, tx: bool },
}

impl Op {
    /// Whether the operation writes its own transaction (`X`).
    fn writes_x(self) -> bool {
        matches!(
            self,
            Op::Transact | Op::TransactNewWal | Op::AutoCheckpoint | Op::Large { .. }
        )
    }

    /// The transaction the operation writes, and how many records it leaves.
    fn x(self) -> (&'static str, usize) {
        match self {
            Op::Large { tx: true, .. } => (X_LARGE, X_TX_FACTS),
            Op::Large { .. } => (X_LARGE, X_LARGE_FACTS),
            _ => (X, X_FACTS),
        }
    }
}

const X: &str = r#"(transact [[:x1 :p/x 1] [:x1 :p/x 2] [:x2 :p/xlong "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"]])"#;
const X_FACTS: usize = 3;

/// A large transaction at a chunk size of [`LARGE_CHUNK`]: several values of
/// one attribute, long values shared by two facts, and a repeated fact, which
/// is stored once.
const X_LARGE: &str = r#"(transact [[:x1 :p/x 1] [:x1 :p/x 2] [:x1 :p/x 3]
    [:x2 :p/xlong "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"]
    [:x3 :p/xlong "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"]
    [:x4 :p/x 4] [:x5 :p/x 5] [:x1 :p/x 1] [:x6 :p/xref :x1] [:x7 :p/x 7] [:x8 :p/x 8]])"#;
const X_LARGE_FACTS: usize = 10;
const LARGE_CHUNK: usize = 3;

/// A `WriteTransaction` around [`X_LARGE`]: a staged statement first, then,
/// after it, a statement that fails `API-011` after writing a chunk and one
/// that repeats and changes earlier statements' triples. It leaves
/// [`X_TX_FACTS`] records.
const X_TX_BEFORE: &str = "(transact [[:x1 :p/x 1] [:x9 :p/x 9]])";
const X_TX_FAILS: &str = r#"(transact [[:x11 :p/x 1 {:valid-from "2020-01-01T00:00:00Z"}]
    [:x12 :p/x 1] [:x13 :p/x 1] [:x11 :p/x 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#;
const X_TX_AFTER: &str =
    r#"(transact [[:x1 :p/x 2 {:valid-from "2020-01-01T00:00:00Z"}] [:x10 :p/x 10] [:x9 :p/x 9]])"#;
const X_TX_FACTS: usize = 12;

/// Run the `WriteTransaction` of `Op::Large { tx: true, .. }`.
fn large_write_tx(db: &Minigraf) -> Result<(), crate::MinigrafError> {
    let mut tx = db.begin_write()?;
    tx.execute(X_TX_BEFORE)?;
    tx.execute(X_LARGE)?;
    match tx.execute(X_TX_FAILS) {
        Err(e) if e.code() == "API-011" => {}
        Err(e) => return Err(e),
        Ok(_) => panic!("a statement with two windows of a triple was accepted"),
    }
    tx.execute(X_TX_AFTER)?;
    tx.commit()
}

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
    fill(&db, op);
    db
}

/// The commits of [`build`], on `db`.
fn fill(db: &Minigraf, op: Op) {
    if let Op::Large {
        one_slot: true,
        wal,
        ..
    } = op
    {
        if wal {
            db.execute(&bulk("c", 20)).unwrap();
            db.execute(r#"(transact [[:e1 :p/tag "c"] [:e3 :p/ref :e1]])"#)
                .unwrap();
        }
        return;
    }
    db.execute(&bulk("a", 150)).unwrap();
    db.execute(r#"(transact [[:e1 :p/tag "a"] [:e1 :p/tag "b"] [:e2 :p/n 1]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    db.execute(&bulk("b", 150)).unwrap();
    db.execute(r#"(retract [[:e1 :p/tag "a"] [:a7 :p/n 7]])"#)
        .unwrap();
    db.checkpoint().unwrap();
    if !matches!(op, Op::TransactNewWal | Op::Large { wal: false, .. }) {
        db.execute(&bulk("c", 20)).unwrap();
        db.execute(r#"(transact [[:e1 :p/tag "c"] [:e3 :p/ref :e1]])"#)
            .unwrap();
    }
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

/// Whether `recs` is exactly one whole `X` transaction of `op`.
fn is_whole_x(recs: &[Rec], op: Op) -> bool {
    recs.len() == op.x().1
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
        more.is_empty() || (x_failed && is_whole_x(&more, op)),
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
            Op::Large { tx, .. } => {
                let _chunk =
                    crate::query::datalog::parser::statement::set_tx_chunk_facts(Some(LARGE_CHUNK));
                let result = if tx {
                    large_write_tx(&db)
                } else {
                    db.execute(X_LARGE).map(|_| ())
                };
                (result, Some(db))
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
                        applied.is_empty() || is_whole_x(&applied, op),
                        "the handle applied part of a transaction"
                    );
                    if result.is_ok() {
                        assert!(
                            is_whole_x(&applied, op),
                            "a committed transaction is missing"
                        );
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

#[test]
fn faults_during_a_large_transaction() {
    run_all(Op::Large {
        one_slot: false,
        wal: true,
        tx: false,
    });
}

#[test]
fn faults_during_a_large_transaction_that_creates_the_wal() {
    run_all(Op::Large {
        one_slot: false,
        wal: false,
        tx: false,
    });
}

/// One valid meta slot: without the WAL guard, a fault after the first page
/// write would leave a file refused with STG-033 (#550 spec §4.5).
#[test]
fn faults_during_a_large_transaction_on_a_new_file() {
    run_all(Op::Large {
        one_slot: true,
        wal: false,
        tx: false,
    });
}

#[test]
fn faults_during_a_large_transaction_on_a_new_file_with_a_wal() {
    run_all(Op::Large {
        one_slot: true,
        wal: true,
        tx: false,
    });
}

#[test]
fn faults_during_a_large_write_transaction() {
    run_all(Op::Large {
        one_slot: false,
        wal: true,
        tx: true,
    });
}

#[test]
fn faults_during_a_large_write_transaction_that_creates_the_wal() {
    run_all(Op::Large {
        one_slot: false,
        wal: false,
        tx: true,
    });
}

#[test]
fn faults_during_a_large_write_transaction_on_a_new_file() {
    run_all(Op::Large {
        one_slot: true,
        wal: false,
        tx: true,
    });
}

#[test]
fn faults_during_a_large_write_transaction_on_a_new_file_with_a_wal() {
    run_all(Op::Large {
        one_slot: true,
        wal: true,
        tx: true,
    });
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
    db.kill();
    // Count the checkpoint's writes on a copy (Windows cannot copy a file
    // that is open), then fill the disk at half of them.
    let writes = checkpoint_calls(&path, Fault::Eio);
    let db = open(&path, Op::Checkpoint);
    let live = snapshot(&db);
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

// ─── Mixed WAL sync modes (#871) ─────────────────────────────────────────────

/// A mixed-mode operation: `X` committed through `begin_write_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mix {
    /// (i) On a `Full` database whose WAL ends in an entry committed with
    /// `Normal`, `X` commits with `Full`.
    FullAfterNormal,
    /// (ii) On a `Full` database whose WAL holds synced entries, `X` commits
    /// with `Normal`.
    NormalOnFull,
    /// (iii) On a `Normal` database whose WAL holds unsynced entries, `X`
    /// commits with `Full`.
    FullOnNormal,
}

impl Mix {
    fn db_mode(self) -> SyncMode {
        match self {
            Mix::FullAfterNormal | Mix::NormalOnFull => SyncMode::Full,
            Mix::FullOnNormal => SyncMode::Normal,
        }
    }

    fn x_mode(self) -> SyncMode {
        match self {
            Mix::FullAfterNormal | Mix::FullOnNormal => SyncMode::Full,
            Mix::NormalOnFull => SyncMode::Normal,
        }
    }
}

/// The states a file may come back in: the records after each commit, in
/// commit order, and the first one every reopen must reach.
struct History {
    states: Vec<Vec<Rec>>,
    /// The last state at a successful WAL sync or a checkpoint: what even a
    /// lost sync keeps.
    durable: usize,
}

impl History {
    fn push(&mut self, db: &Minigraf) {
        self.states.push(snapshot(db));
        // No WAL: the last commit was a checkpoint.
        if db.wal_synced().is_none_or(|synced| synced) {
            self.durable = self.states.len() - 1;
        }
    }
}

fn commit_with(db: &Minigraf, mode: SyncMode, stmt: &str) -> Result<(), crate::MinigrafError> {
    let mut tx = db.begin_write_with(TxOptions::new().synchronous(mode))?;
    tx.execute(stmt)?;
    tx.commit()
}

/// Run `mix` with `f` at every call index. Returns how many indexes hit.
fn run_mixed(mix: Mix, f: Fault) -> u64 {
    let op = Op::Transact;
    let mut hits = 0;
    for k in 0u64.. {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.graph");
        let open_db = || {
            OpenOptions::new()
                .synchronous(mix.db_mode())
                .wal_checkpoint_threshold(1000)
                .path(&path)
                .open()
                .expect("open")
        };
        let db = open_db();
        // Two checkpoints, then two entries in the database mode.
        fill(&db, Op::TransactNewWal);
        let mut history = History {
            states: Vec::new(),
            durable: 0,
        };
        history.push(&db);
        db.execute(&bulk("c", 20)).unwrap();
        history.push(&db);
        db.execute(r#"(transact [[:e1 :p/tag "c"] [:e3 :p/ref :e1]])"#)
            .unwrap();
        history.push(&db);
        if mix == Mix::FullAfterNormal {
            commit_with(&db, SyncMode::Normal, r#"(transact [[:n1 :p/n "normal"]])"#).unwrap();
            history.push(&db);
        }
        assert_eq!(
            db.wal_synced(),
            Some(mix == Mix::NormalOnFull),
            "the WAL before X"
        );
        let before = snapshot(&db);

        fault::arm(f, k);
        let result = commit_with(&db, mix.x_mode(), X);
        let tripped = fault::disarm();

        let live = snapshot(&db);
        let applied = extra(&before, &live).expect("the handle lost a fact");
        assert!(
            applied.is_empty() || is_whole_x(&applied, op),
            "the handle applied part of a transaction"
        );
        if result.is_ok() {
            assert!(
                is_whole_x(&applied, op),
                "a committed transaction is missing"
            );
            history.push(&db);
        } else {
            assert!(applied.is_empty(), "a failed commit was applied");
        }

        // A later write succeeds, or is refused as failed.
        let live = match db.execute(r#"(transact [[:y :p/y "later"]])"#) {
            Ok(_) => {
                history.push(&db);
                snapshot(&db)
            }
            Err(e) => {
                let c = code(e);
                assert!(c == "WAL-007" || c == "STG-046", "later write: {c}");
                assert!(result.is_err(), "a write refused after a success");
                live
            }
        };
        db.kill();

        // Reopen: a prefix of the commit order, from the durable state on.
        let db = open_db();
        let reopened = snapshot(&db);
        let at = history.states.iter().rposition(|s| *s == reopened);
        // A failed X's entry may still have reached the file whole: then
        // every transaction before it is there too.
        let failed_x_on_disk =
            result.is_err() && extra(&before, &reopened).is_some_and(|more| is_whole_x(&more, op));
        assert!(
            at.is_some_and(|at| at >= history.durable) || failed_x_on_disk,
            "the reopened file is not a prefix of the commit order from its last sync"
        );
        if f != Fault::SyncLost {
            // Only a lost sync may lose an entry committed without one.
            assert!(
                extra(&live, &reopened).is_some(),
                "a fact the handle held was lost without a lost sync"
            );
        }
        assert!(
            db.verify().unwrap().problems.is_empty(),
            "verify after reopen"
        );

        // One more commit after recovery must survive a kill.
        db.execute(r#"(transact [[:z :p/z "after"]])"#).unwrap();
        let with_z = snapshot(&db);
        db.kill();
        let db = open_db();
        assert!(snapshot(&db) == with_z, "a commit after recovery was lost");
        assert!(db.verify().unwrap().problems.is_empty(), "verify after Z");
        db.kill();

        if tripped.is_none() {
            assert!(result.is_ok(), "failed without a fault");
            break;
        }
        hits += 1;
    }
    hits
}

fn run_all_mixed(mix: Mix) {
    for f in FAULTS {
        let hits = run_mixed(mix, f);
        // A `Normal` commit to an existing WAL makes no sync for a sync
        // fault to hit.
        let syncs = mix.x_mode() == SyncMode::Full;
        let sync_fault = matches!(f, Fault::SyncEio | Fault::SyncLost);
        assert!(hits > 0 || (sync_fault && !syncs), "the fault never fired");
    }
}

#[test]
fn faults_during_a_full_commit_after_a_normal_one() {
    run_all_mixed(Mix::FullAfterNormal);
}

#[test]
fn faults_during_a_normal_commit_on_a_full_database() {
    run_all_mixed(Mix::NormalOnFull);
}

#[test]
fn faults_during_a_full_commit_on_a_normal_database() {
    run_all_mixed(Mix::FullOnNormal);
}
