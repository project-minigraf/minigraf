//! `TxOptions` and `Minigraf::begin_write_with` (#871): each option applies
//! to one transaction only, and what survives a process kill or a power loss
//! when WAL sync modes are mixed per transaction on one WAL file.
//!
//! A power loss is [`Minigraf::power_loss`]: the WAL is cut back to its
//! length at its last successful sync, then the handle ends as a kill would.

use crate::db::{Minigraf, OpenOptions, SyncMode, TxOptions};
use crate::fact_log::FactFilter;
use std::path::{Path, PathBuf};

/// One fact version, comparable across handles.
type Rec = (String, String, String, u64, u64, i64, i64, bool);

fn records(db: &Minigraf) -> Vec<Rec> {
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

fn wal_of(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".wal");
    PathBuf::from(p)
}

/// A file database in `mode` that never checkpoints by itself.
fn open(path: &Path, mode: SyncMode) -> Minigraf {
    OpenOptions::new()
        .synchronous(mode)
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()
        .unwrap()
}

/// Commit `stmt` in one transaction begun with `opts`.
fn commit(db: &Minigraf, opts: TxOptions, stmt: &str) {
    let mut tx = db.begin_write_with(opts).unwrap();
    tx.execute(stmt).unwrap();
    tx.commit().unwrap();
}

fn tx(mode: Option<SyncMode>) -> TxOptions {
    match mode {
        Some(m) => TxOptions::new().synchronous(m),
        None => TxOptions::new(),
    }
}

const MODES: [SyncMode; 2] = [SyncMode::Full, SyncMode::Normal];

// ─── API ─────────────────────────────────────────────────────────────────────

#[test]
fn tx_options_builders_set_their_fields() {
    let opts = TxOptions::new();
    assert_eq!(opts, TxOptions::default());
    assert_eq!(opts.synchronous, None);
    assert_eq!(opts.chunk_facts, None);
    let opts = opts.synchronous(SyncMode::Normal).chunk_facts(7);
    assert_eq!(opts.synchronous, Some(SyncMode::Normal));
    assert_eq!(opts.chunk_facts, Some(7));
    assert_eq!(opts.clone(), opts);
    assert!(format!("{opts:?}").contains("Normal"));
    let root: crate::TxOptions = opts;
    assert_eq!(root.chunk_facts, Some(7));
}

#[test]
fn begin_write_with_fails_as_begin_write_does() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    {
        let _tx = db.begin_write().unwrap();
        let err = db
            .begin_write_with(TxOptions::new().synchronous(SyncMode::Normal))
            .err()
            .expect("a second transaction on this thread");
        assert_eq!(err.code(), "INT-001");
    }
    // The failed call left the handle usable.
    commit(
        &db,
        TxOptions::new().synchronous(SyncMode::Normal),
        "(transact [[:a :p/n 1]])",
    );
    drop(db);

    let ro = OpenOptions::new()
        .read_only(true)
        .path(&path)
        .open()
        .unwrap();
    let err = ro
        .begin_write_with(TxOptions::new().chunk_facts(2))
        .err()
        .expect("a read-only handle");
    assert_eq!(err.code(), "API-014");
    let err = ro.begin_write().err().expect("a read-only handle");
    assert_eq!(err.code(), "API-014");
    assert_eq!(records(&ro).len(), 1, "the read-only handle still reads");
}

// ─── synchronous ─────────────────────────────────────────────────────────────

/// AC 4: a small commit syncs the WAL exactly when its effective mode is
/// `Full`, in all four combinations, whether the commit creates the WAL or
/// appends to one.
#[test]
fn commit_syncs_by_the_transaction_mode() {
    for db_mode in MODES {
        for tx_mode in [None, Some(SyncMode::Full), Some(SyncMode::Normal)] {
            let effective = tx_mode.unwrap_or(db_mode);
            for wal_exists in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("t.graph");
                let db = open(&path, db_mode);
                if wal_exists {
                    db.execute("(transact [[:a :p/n 0]])").unwrap();
                    assert!(wal_of(&path).exists());
                } else {
                    assert_eq!(db.wal_synced(), None, "no WAL yet");
                }
                commit(&db, tx(tx_mode), "(transact [[:a :p/n 1]])");
                assert_eq!(
                    db.wal_synced(),
                    Some(effective == SyncMode::Full),
                    "the commit synced the WAL by its effective mode"
                );
            }
        }
    }
}

/// AC 5: the mode of a transaction does not stay with the WAL writer, also
/// when the transaction's own commit created the WAL file.
#[test]
fn transaction_mode_applies_to_that_transaction_only() {
    for creates_wal in [true, false] {
        for db_mode in MODES {
            let other = match db_mode {
                SyncMode::Full => SyncMode::Normal,
                SyncMode::Normal => SyncMode::Full,
            };
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.graph");
            let db = open(&path, db_mode);
            if !creates_wal {
                commit(
                    &db,
                    TxOptions::new().synchronous(SyncMode::Full),
                    "(transact [[:a :p/n 0]])",
                );
            }
            commit(
                &db,
                TxOptions::new().synchronous(other),
                "(transact [[:a :p/n 1]])",
            );
            assert_eq!(db.wal_synced(), Some(other == SyncMode::Full));

            // An implicit transaction follows the database mode again.
            db.execute("(transact [[:a :p/n 2]])").unwrap();
            assert_eq!(db.wal_synced(), Some(db_mode == SyncMode::Full));

            // So does `begin_write()`.
            commit(
                &db,
                TxOptions::new().synchronous(other),
                "(transact [[:a :p/n 3]])",
            );
            let mut t = db.begin_write().unwrap();
            t.execute("(transact [[:a :p/n 4]])").unwrap();
            t.commit().unwrap();
            assert_eq!(db.wal_synced(), Some(db_mode == SyncMode::Full));

            // A rolled-back or dropped transaction leaves nothing behind.
            commit(
                &db,
                TxOptions::new().synchronous(other),
                "(transact [[:a :p/n 5]])",
            );
            let mut t = db
                .begin_write_with(TxOptions::new().synchronous(other))
                .unwrap();
            t.execute("(transact [[:a :p/n 6]])").unwrap();
            t.rollback();
            drop(
                db.begin_write_with(TxOptions::new().synchronous(other))
                    .unwrap(),
            );
            db.execute("(transact [[:a :p/n 7]])").unwrap();
            assert_eq!(db.wal_synced(), Some(db_mode == SyncMode::Full));
        }
    }
}

/// AC 5: a `Normal` transaction on a `Full` database that became large
/// (creating the WAL guard) and was rolled back; the next implicit
/// transaction syncs.
#[test]
fn rolled_back_large_normal_transaction_leaves_no_mode_behind() {
    for drop_it in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = open(&path, SyncMode::Full);
        let mut t = db
            .begin_write_with(
                TxOptions::new()
                    .synchronous(SyncMode::Normal)
                    .chunk_facts(2),
            )
            .unwrap();
        t.execute("(transact [[:a :p/n 1] [:b :p/n 2] [:c :p/n 3]])")
            .unwrap();
        assert!(wal_of(&path).exists(), "the WAL guard exists");
        if drop_it {
            drop(t);
        } else {
            t.rollback();
        }
        assert_eq!(db.wal_synced(), Some(true), "the guard header is synced");
        db.execute("(transact [[:d :p/n 4]])").unwrap();
        assert_eq!(db.wal_synced(), Some(true), "the implicit commit synced");
        db.power_loss();
        let db = open(&path, SyncMode::Full);
        assert_eq!(records(&db).len(), 1, "only the implicit commit");
    }
}

/// AC 7: a large transaction begun with `Normal` commits as a checkpoint
/// and survives a power loss whole.
#[test]
fn large_normal_transaction_survives_a_power_loss() {
    for db_mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = open(&path, db_mode);
        db.execute("(transact [[:base :p/n 0]])").unwrap();
        db.checkpoint().unwrap();
        let mut t = db
            .begin_write_with(
                TxOptions::new()
                    .synchronous(SyncMode::Normal)
                    .chunk_facts(2),
            )
            .unwrap();
        t.execute("(transact [[:a :p/n 1]])").unwrap();
        t.execute("(transact [[:b :p/n 2] [:c :p/n 3] [:d :p/n 4]])")
            .unwrap();
        t.execute("(transact [[:e :p/n 5]])").unwrap();
        t.commit().unwrap();
        assert!(!wal_of(&path).exists(), "a large commit is a checkpoint");
        let live = records(&db);
        assert_eq!(live.len(), 6);
        db.power_loss();

        let db = open(&path, db_mode);
        assert!(records(&db) == live, "the whole large transaction survives");
        assert!(db.verify().unwrap().problems.is_empty(), "verify");
    }
}

/// AC 8: the options are accepted on an in-memory database, and change
/// nothing there.
#[test]
fn options_on_an_in_memory_database() {
    let stmt = "(transact [[:a :p/n 1] [:b :p/n 2] [:c :p/n 3]])";
    let plain = Minigraf::in_memory().unwrap();
    commit(&plain, TxOptions::new(), stmt);
    for opts in [
        TxOptions::new().synchronous(SyncMode::Normal),
        TxOptions::new().synchronous(SyncMode::Full),
        TxOptions::new().chunk_facts(1),
        TxOptions::new().chunk_facts(0),
        TxOptions::new()
            .chunk_facts(usize::MAX)
            .synchronous(SyncMode::Normal),
    ] {
        let db = Minigraf::in_memory().unwrap();
        commit(&db, opts, stmt);
        assert_eq!(db.wal_synced(), None, "no WAL");
        assert!(
            without_tx(&records(&db)) == without_tx(&records(&plain)),
            "the same facts as without options"
        );
    }
}

/// AC 8: a `Normal` commit that brings the WAL to the threshold still
/// checkpoints.
#[test]
fn normal_commit_still_auto_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(2)
        .path(&path)
        .open()
        .unwrap();
    let normal = TxOptions::new().synchronous(SyncMode::Normal);
    commit(&db, normal.clone(), "(transact [[:a :p/n 1]])");
    assert!(wal_of(&path).exists(), "one entry pending");
    commit(&db, normal, "(transact [[:a :p/n 2]])");
    assert!(!wal_of(&path).exists(), "the threshold checkpointed");
    let live = records(&db);
    db.power_loss();
    let db = Minigraf::open(&path).unwrap();
    assert!(records(&db) == live, "the checkpoint is durable");
}

// ─── Mixed modes, crash after each (AC 10) ───────────────────────────────────

/// How a handle ends.
#[derive(Clone, Copy, PartialEq)]
enum End {
    Kill,
    PowerLoss,
}

/// On a database in `db_mode` holding a checkpointed fact and one synced
/// WAL entry, commit one transaction per mode in `modes` through
/// `begin_write_with`, end the handle with `end`, and reopen. Returns the
/// records after each commit (index 0: before the first) and the reopened
/// handle.
fn mixed(
    db_mode: SyncMode,
    modes: &[SyncMode],
    end: End,
    path: &Path,
) -> (Vec<Vec<Rec>>, Minigraf) {
    let db = open(path, db_mode);
    db.execute("(transact [[:base :p/n 0]])").unwrap();
    db.checkpoint().unwrap();
    commit(
        &db,
        TxOptions::new().synchronous(SyncMode::Full),
        "(transact [[:synced :p/n 0]])",
    );
    let mut after = vec![records(&db)];
    for (i, mode) in modes.iter().enumerate() {
        commit(
            &db,
            TxOptions::new().synchronous(*mode),
            &format!("(transact [[:t{i} :p/n {i}] [:t{i} :p/name \"tx {i}\"]])"),
        );
        after.push(records(&db));
    }
    match end {
        End::Kill => db.kill(),
        End::PowerLoss => db.power_loss(),
    }
    (after, open(path, db_mode))
}

/// After recovery, one more commit survives a kill and a reopen.
fn commit_after_recovery(db: Minigraf, path: &Path, db_mode: SyncMode) {
    assert!(
        db.verify().unwrap().problems.is_empty(),
        "verify after reopen"
    );
    db.execute(r#"(transact [[:z :p/z "after"]])"#).unwrap();
    let with_z = records(&db);
    db.kill();
    let db = open(path, db_mode);
    assert!(records(&db) == with_z, "a commit after recovery was lost");
    assert!(db.verify().unwrap().problems.is_empty(), "verify after Z");
}

#[test]
fn normal_then_kill_keeps_it() {
    for db_mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let (after, db) = mixed(db_mode, &[SyncMode::Normal], End::Kill, &path);
        assert!(records(&db) == after[1], "A survives a process kill");
        commit_after_recovery(db, &path, db_mode);
    }
}

#[test]
fn normal_then_power_loss_keeps_a_prefix() {
    for db_mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let (after, db) = mixed(db_mode, &[SyncMode::Normal], End::PowerLoss, &path);
        let got = records(&db);
        assert!(after.contains(&got), "a prefix of the commit order");
        assert!(got == after[0], "the unsynced A is lost");
        commit_after_recovery(db, &path, db_mode);
    }
}

#[test]
fn normal_then_full_then_power_loss_keeps_both() {
    for db_mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let (after, db) = mixed(
            db_mode,
            &[SyncMode::Normal, SyncMode::Full],
            End::PowerLoss,
            &path,
        );
        assert!(records(&db) == after[2], "B's sync made A durable too");
        commit_after_recovery(db, &path, db_mode);
    }
}

#[test]
fn full_then_normal_then_power_loss_keeps_the_full_one() {
    for db_mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let (after, db) = mixed(
            db_mode,
            &[SyncMode::Full, SyncMode::Normal],
            End::PowerLoss,
            &path,
        );
        let got = records(&db);
        // B may be whole or absent; the model loses it.
        assert!(
            got == after[1] || got == after[2],
            "A, and B whole or absent"
        );
        assert!(got == after[1], "the unsynced B is lost in the model");
        commit_after_recovery(db, &path, db_mode);
    }
}

// ─── chunk_facts ─────────────────────────────────────────────────────────────

/// Whether the transaction committed as a large one: its commit is a
/// checkpoint, which deletes the WAL (a small commit leaves an entry in it).
fn committed_large(path: &Path) -> bool {
    !wal_of(path).exists()
}

/// Records without their transaction stamps: what a commit wrote,
/// whatever its `tx_id`/`tx_count` (a default `valid_from` is the `tx_id`).
fn without_tx(recs: &[Rec]) -> Vec<(String, String, String, bool, i64, bool)> {
    let mut v: Vec<_> = recs
        .iter()
        .map(|r| {
            (
                r.0.clone(),
                r.1.clone(),
                r.2.clone(),
                r.5 == r.4.cast_signed(),
                r.6,
                r.7,
            )
        })
        .collect();
    v.sort();
    v
}

const THREE: &str = "(transact [[:a :p/n 1] [:b :p/n 2] [:c :p/n 3]])";

#[test]
fn chunk_facts_replaces_the_database_chunk_size() {
    // Default database chunk, transaction chunk 2: three facts are large.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    commit(&db, TxOptions::new().chunk_facts(2), THREE);
    assert!(committed_large(&path), "three facts pass a chunk of 2");
    assert_eq!(records(&db).len(), 3);

    // Staged statements that pass it together make it large too.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    let mut t = db
        .begin_write_with(TxOptions::new().chunk_facts(2))
        .unwrap();
    t.execute("(transact [[:a :p/n 1] [:b :p/n 2]])").unwrap();
    t.execute("(transact [[:c :p/n 3]])").unwrap();
    t.commit().unwrap();
    assert!(committed_large(&path), "staged facts pass a chunk of 2");

    // Database chunk 2, transaction chunk 1000: three facts are small.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = OpenOptions::new()
        .transaction_chunk_facts(2)
        .wal_checkpoint_threshold(usize::MAX)
        .path(&path)
        .open()
        .unwrap();
    commit(&db, TxOptions::new().chunk_facts(1000), THREE);
    assert!(!committed_large(&path), "a WAL entry");
    assert_eq!(db.wal_synced(), Some(true));
    assert_eq!(records(&db).len(), 3);
}

#[test]
fn chunk_facts_is_clamped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    {
        let t = db
            .begin_write_with(TxOptions::new().chunk_facts(0))
            .unwrap();
        assert_eq!(t.chunk(), 1);
    }
    {
        let t = db
            .begin_write_with(TxOptions::new().chunk_facts(usize::MAX))
            .unwrap();
        assert_eq!(
            t.chunk(),
            crate::query::datalog::parser::statement::MAX_TX_CHUNK_FACTS
        );
    }
    // A chunk of 0 behaves as 1: two facts are large.
    commit(
        &db,
        TxOptions::new().chunk_facts(0),
        "(transact [[:a :p/n 1] [:b :p/n 2]])",
    );
    assert!(committed_large(&path), "two facts pass a chunk of 1");
}

#[test]
fn chunk_facts_applies_to_that_transaction_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    commit(&db, TxOptions::new().chunk_facts(2), THREE);
    assert!(committed_large(&path));
    {
        let t = db.begin_write().unwrap();
        assert_eq!(t.chunk(), 65_536, "the database chunk size again");
    }
    commit(
        &db,
        TxOptions::new(),
        "(transact [[:d :p/n 1] [:e :p/n 2] [:f :p/n 3]])",
    );
    assert!(!committed_large(&path), "the next transaction is small");
    db.execute("(transact [[:g :p/n 1] [:h :p/n 2] [:i :p/n 3]])")
        .unwrap();
    assert!(!committed_large(&path), "an implicit transaction is small");
    assert_eq!(records(&db).len(), 9);
}

/// The thread-local test hook keeps precedence over `TxOptions::chunk_facts`,
/// as it has over the database value.
#[test]
fn test_hook_beats_chunk_facts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    let db = open(&path, SyncMode::Full);
    let _chunk = crate::query::datalog::parser::statement::set_tx_chunk_facts(Some(2));
    {
        let t = db
            .begin_write_with(TxOptions::new().chunk_facts(1000))
            .unwrap();
        assert_eq!(t.chunk(), 2);
    }
    commit(&db, TxOptions::new().chunk_facts(1000), THREE);
    assert!(committed_large(&path), "the hook's chunk of 2 applies");
}

/// AC 15: committed data does not depend on the options.
#[test]
fn committed_data_does_not_depend_on_the_options() {
    let statements = [
        r#"(transact [[:a :p/n 1] [:a :p/n 2] [:b :p/name "bee"] [:c :p/ref :a]])"#,
        r#"(transact [[:d :p/n 4 {:valid-from "2020-01-01T00:00:00Z"}] [:a :p/n 1]])"#,
        "(retract [[:a :p/n 2]])",
        r#"(transact [[:e :p/long "lllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllll"]])"#,
    ];
    let run = |opts: TxOptions| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = open(&path, SyncMode::Full);
        db.execute("(transact [[:a :p/n 2] [:z :p/n 0]])").unwrap();
        db.checkpoint().unwrap();
        let mut t = db.begin_write_with(opts).unwrap();
        for s in statements {
            t.execute(s).unwrap();
        }
        t.commit().unwrap();
        let live = without_tx(&records(&db));
        db.kill();
        let db = open(&path, SyncMode::Full);
        assert!(
            without_tx(&records(&db)) == live,
            "the reopened file agrees"
        );
        live
    };
    let expected = run(TxOptions::new());
    for opts in [
        TxOptions::new().synchronous(SyncMode::Normal),
        TxOptions::new().synchronous(SyncMode::Full),
        TxOptions::new().chunk_facts(1),
        TxOptions::new().chunk_facts(3),
        TxOptions::new()
            .chunk_facts(2)
            .synchronous(SyncMode::Normal),
        TxOptions::new().chunk_facts(1_000_000),
    ] {
        assert!(run(opts) == expected, "the same committed data");
    }
}
