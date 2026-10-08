//! One current valid-time window per `(e, a, v)` (#435).
//!
//! At transaction time N a triple has exactly one current window: the one of
//! its latest assertion at or before N that is newer than its last retraction.
//! A later assertion replaces an earlier window, which stays visible through
//! `:as-of`. Each scenario runs in memory, file-backed with every write
//! checkpointed (committed key walk), and file-backed after a reopen, through
//! an entity-bound pattern (EAVT) and an attribute scan (AEVT).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult};

/// The databases each scenario runs against.
enum Mode {
    Memory,
    Checkpointed,
    Reopened,
}

const MODES: [Mode; 3] = [Mode::Memory, Mode::Checkpointed, Mode::Reopened];

/// Run `writes` in a fresh database of `mode`, then hand it to `check`.
fn with_db(mode: &Mode, writes: &[&str], check: impl Fn(&Minigraf)) {
    match mode {
        Mode::Memory => {
            let db = Minigraf::in_memory().unwrap();
            for w in writes {
                db.execute(w).unwrap();
            }
            check(&db);
        }
        Mode::Checkpointed | Mode::Reopened => {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("w.graph");
            {
                let db = Minigraf::open(&path).unwrap();
                for w in writes {
                    db.execute(w).unwrap();
                    db.checkpoint().unwrap();
                }
                if matches!(mode, Mode::Checkpointed) {
                    check(&db);
                    return;
                }
            }
            let db = Minigraf::open(&path).unwrap();
            check(&db);
        }
    }
}

fn count(db: &Minigraf, query: &str) -> usize {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    }
}

/// Whether `[:d/c :description "r"]` is visible at `as_of` (`""` = now) and
/// `valid_at`, checked through EAVT and AEVT; both must agree.
fn visible(db: &Minigraf, as_of: &str, valid_at: &str) -> bool {
    let eavt = count(
        db,
        &format!(
            r#"(query [:find ?v {as_of} :valid-at "{valid_at}" :where [:d/c :description ?v]])"#
        ),
    );
    let aevt = count(
        db,
        &format!(
            r#"(query [:find ?e {as_of} :valid-at "{valid_at}" :where [?e :description "r"]])"#
        ),
    );
    assert_eq!(eavt, aevt, "EAVT and AEVT paths agree");
    assert!(eavt <= 1, "at most one window per triple");
    eavt == 1
}

/// The issue's table: a later bounded window replaces an earlier one.
#[test]
fn later_assertion_replaces_earlier_window() {
    let writes = [
        r#"(transact {:valid-from "2020-01-01" :valid-to "2022-01-01"} [[:d/c :description "r"]])"#,
        r#"(transact {:valid-from "2024-01-01"} [[:d/c :description "r"]])"#,
    ];
    for mode in &MODES {
        with_db(mode, &writes, |db| {
            assert!(visible(db, ":as-of 1", "2021-01-01"), "tx 1 window");
            assert!(!visible(db, ":as-of 2", "2021-01-01"), "replaced at tx 2");
            assert!(visible(db, ":as-of 2", "2025-01-01"), "tx 2 window");
            assert!(!visible(db, "", "2021-01-01"), "replaced now");
            assert!(visible(db, "", "2025-01-01"), "current window");
            assert!(!visible(db, ":as-of 1", "2025-01-01"), "not yet asserted");
        });
    }
}

/// Closing an open window is a `transact` with `:valid-to`.
#[test]
fn transact_with_valid_to_closes_open_window() {
    let writes = [
        r#"(transact {:valid-from "2020-01-01"} [[:d/c :description "r"]])"#,
        r#"(transact {:valid-from "2020-01-01" :valid-to "2023-01-01"} [[:d/c :description "r"]])"#,
    ];
    for mode in &MODES {
        with_db(mode, &writes, |db| {
            assert!(visible(db, ":as-of 1", "2025-01-01"), "open before closing");
            assert!(!visible(db, "", "2025-01-01"), "closed now");
            assert!(visible(db, "", "2021-01-01"), "inside the closed window");
        });
    }
}

/// Reopening a closed window, and extending one.
#[test]
fn transact_reopens_and_extends_window() {
    let writes = [
        r#"(transact {:valid-from "2020-01-01" :valid-to "2021-01-01"} [[:d/c :description "r"]])"#,
        r#"(transact {:valid-from "2020-01-01" :valid-to "2022-01-01"} [[:d/c :description "r"]])"#,
        r#"(transact {:valid-from "2020-01-01"} [[:d/c :description "r"]])"#,
    ];
    for mode in &MODES {
        with_db(mode, &writes, |db| {
            assert!(!visible(db, ":as-of 1", "2021-06-01"), "before extending");
            assert!(visible(db, ":as-of 2", "2021-06-01"), "extended");
            assert!(!visible(db, ":as-of 2", "2030-01-01"), "still bounded");
            assert!(visible(db, "", "2030-01-01"), "reopened");
        });
    }
}

/// A retraction withdraws the triple; a later assertion brings back only
/// its own window.
#[test]
fn retraction_withdraws_then_reassertion_has_own_window() {
    let writes = [
        r#"(transact {:valid-from "2020-01-01"} [[:d/c :description "r"]])"#,
        r#"(retract [[:d/c :description "r"]])"#,
        r#"(transact {:valid-from "2024-01-01" :valid-to "2025-01-01"} [[:d/c :description "r"]])"#,
    ];
    for mode in &MODES {
        with_db(mode, &writes, |db| {
            assert!(!visible(db, ":as-of 2", "2021-01-01"), "retracted");
            assert!(!visible(db, "", "2021-01-01"), "old window gone");
            assert!(visible(db, "", "2024-06-01"), "new window");
        });
    }
}

/// Two windows of one triple in one `transact` are API-011, and nothing is
/// written or counted.
#[test]
fn same_transaction_two_windows_rejected() {
    for mode in &MODES {
        with_db(mode, &[r#"(transact [[:d/x :n 1]])"#], |db| {
            let err = db
                    .execute(
                        r#"(transact [[:d/c :description "r" {:valid-from "2020-01-01" :valid-to "2021-01-01"}]
                                      [:d/c :description "r" {:valid-from "2024-01-01"}]])"#,
                    )
                    .expect_err("two windows in one transaction");
            assert_eq!(err.code(), "API-011");
            assert!(!visible(db, "", "2020-06-01"), "nothing written");
            assert!(!visible(db, "", "2024-06-01"), "nothing written");

            // The next transaction takes tx 2: the rejected one took none.
            db.execute(r#"(transact [[:d/c :description "r"]])"#)
                .unwrap();
            assert_eq!(
                count(
                    db,
                    r#"(query [:find ?v :as-of 2 :valid-at :any-valid-time :where [:d/c :description ?v]])"#
                ),
                1,
                "next transaction is tx 2"
            );
        });
    }
}

/// Identical repeats and different values in one transaction are fine.
#[test]
fn same_transaction_repeat_or_other_value_accepted() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:d/c :description "r"] [:d/c :description "r"]])"#)
        .unwrap();
    db.execute(
        r#"(transact [[:d/c :tag "a" {:valid-to "2030-01-01"}] [:d/c :tag "b" {:valid-from "2020-01-01"}]])"#,
    )
    .unwrap();
}

/// Two windows of one triple across `execute` calls of one
/// `WriteTransaction`: the later statement's window is the one committed
/// (#477).
#[test]
fn write_transaction_two_windows_across_statements_last_wins() {
    for mode in &MODES {
        with_db(mode, &[], |db| {
            let mut tx = db.begin_write().unwrap();
            tx.execute(
                r#"(transact {:valid-from "2020-01-01" :valid-to "2021-01-01"} [[:d/c :description "r"]])"#,
            )
            .unwrap();
            tx.execute(r#"(transact {:valid-from "2024-01-01"} [[:d/c :description "r"]])"#)
                .unwrap();
            tx.commit().unwrap();
            assert!(!visible(db, "", "2020-06-01"), "earlier statement dropped");
            assert!(visible(db, "", "2024-06-01"), "later statement committed");
            assert!(!visible(db, ":as-of 1", "2020-06-01"), "never in history");
        });
    }
}

/// Default valid-from in a `WriteTransaction` resolves to the commit time
/// for every fact, so repeating a fact without bounds is one window.
#[test]
fn write_transaction_repeat_without_bounds_accepted() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:d/c :description "r"]])"#)
        .unwrap();
    tx.execute(r#"(transact [[:d/c :description "r"]])"#)
        .unwrap();
    tx.commit().unwrap();
}

// ── Empty or inverted windows (#436) ─────────────────────────────────────────

/// Transacts whose effective window ends at or before it starts.
const INVERTED: [&str; 5] = [
    r#"(transact {:valid-from "2026-08-21" :valid-to "2026-08-20"} [[:d/c :description "r"]])"#,
    r#"(transact {:valid-from "2026-08-21" :valid-to "2026-08-21"} [[:d/c :description "r"]])"#,
    // valid-from defaults to the transaction time, after valid-to.
    r#"(transact {:valid-to "2020-01-01"} [[:d/c :description "r"]])"#,
    // A per-fact bound inverts the transaction's window.
    r#"(transact {:valid-from "2020-01-01" :valid-to "2030-01-01"} [[:d/x :n 1] [:d/c :description "r" {:valid-to "2019-01-01"}]])"#,
    r#"(transact [[:d/x :n 1] [:d/c :description "r" {:valid-from "2031-01-01" :valid-to "2030-01-01"}]])"#,
];

/// Each is API-019 from `execute`, writes nothing and takes no `tx_count`.
#[test]
fn inverted_window_rejected_by_execute() {
    for mode in &MODES {
        with_db(mode, &[r#"(transact [[:d/x :n 0]])"#], |db| {
            for (i, w) in INVERTED.iter().enumerate() {
                let err = db.execute(w).expect_err("inverted window");
                assert_eq!(err.code(), "API-019", "case {i}");
            }
            assert_eq!(db.current_tx_count(), 1, "no tx_count taken");
            assert_eq!(
                count(
                    db,
                    r#"(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])"#
                ),
                1,
                "nothing written"
            );
        });
    }
}

/// A window that ends after it starts, also in the past, is accepted.
#[test]
fn past_window_that_ends_after_it_starts_accepted() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact {:valid-from "2019-01-01" :valid-to "2020-01-01"} [[:d/c :description "r"]])"#,
    )
    .unwrap();
    assert!(visible(&db, "", "2019-06-01"), "inside the window");
    assert!(!visible(&db, "", "2020-01-01"), "end is exclusive");
}

/// `tx.execute` rejects an inverted statement and stages nothing; the
/// transaction still commits its other statements.
#[test]
fn inverted_window_rejected_by_tx_execute() {
    let db = Minigraf::in_memory().unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:d/x :n 1]])"#).unwrap();
    for (i, w) in INVERTED.iter().enumerate() {
        let err = tx.execute(w).expect_err("inverted window");
        assert_eq!(err.code(), "API-019", "case {i}");
    }
    tx.commit().unwrap();
    assert_eq!(db.current_tx_count(), 1, "one transaction");
    assert_eq!(
        count(
            &db,
            r#"(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])"#
        ),
        1,
        "only the accepted statement"
    );
}
