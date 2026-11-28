#![cfg(test)]
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{
    IncomingRefBudget, IncomingRefLimits, IncomingRefs, Minigraf, OpenOptions, TxOptions,
};
use std::collections::BTreeSet;
use uuid::Uuid;
mod common;

fn main_file_state(path: &std::path::Path) -> impl PartialEq {
    // Windows locks are mandatory: a second handle cannot read the open database.
    // Compare its length there, and exact bytes elsewhere. The WAL is not locked.
    #[cfg(windows)]
    {
        std::fs::metadata(path).unwrap().len()
    }
    #[cfg(not(windows))]
    {
        std::fs::read(path).unwrap()
    }
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn statement(asserted: bool, source: u128, attr: &str, target: u128) -> String {
    format!(
        "({} [[#uuid \"{}\" {attr} #uuid \"{}\"]])",
        if asserted { "transact" } else { "retract" },
        id(source),
        id(target)
    )
}
fn complete(out: IncomingRefs) -> (BTreeSet<(Uuid, String)>, usize) {
    match out {
        IncomingRefs::Complete {
            links,
            records_examined,
        } => (
            links.into_iter().map(|l| (l.source, l.attribute)).collect(),
            records_examined,
        ),
        _ => panic!("expected complete incoming references"),
    }
}
fn lookup(db: &Minigraf, target: u128) -> (BTreeSet<(Uuid, String)>, usize) {
    complete(
        db.begin_write()
            .unwrap()
            .incoming_refs(id(target), IncomingRefLimits::new(usize::MAX, usize::MAX))
            .unwrap(),
    )
}
fn exhausted(out: IncomingRefs, expected: IncomingRefBudget, cap: usize) {
    match out {
        IncomingRefs::Exhausted {
            budget,
            records_examined,
        } => {
            assert_eq!(budget, expected);
            assert!(records_examined <= cap);
        }
        _ => panic!("expected exhaustion without links"),
    }
}
fn database(file: bool, path: &std::path::Path) -> Minigraf {
    if file {
        Minigraf::open_with_options(
            path,
            OpenOptions::default().wal_checkpoint_threshold(usize::MAX),
        )
        .unwrap()
    } else {
        Minigraf::in_memory().unwrap()
    }
}

#[test]
fn incoming_refs_exact_types_and_attributes() {
    for file in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = database(file, &dir.path().join("types.graph"));
        let target = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":target");
        db.execute(&format!("(transact [[#uuid \"{}\" :a #uuid \"{target}\"] [#uuid \"{}\" :b #uuid \"{target}\"] [#uuid \"{target}\" :self #uuid \"{target}\"] [:alias :a :target] [:string :a \"{target}\"] [:set :a #{{#uuid \"{target}\"}}] [:integer :a 1]])", id(1), id(2))).unwrap();
        for checkpoint in [false, true] {
            if checkpoint && file {
                db.checkpoint().unwrap();
            }
            let (links, _) = lookup(&db, target.as_u128());
            assert!(
                links
                    == BTreeSet::from([
                        (id(1), ":a".into()),
                        (id(2), ":b".into()),
                        (target, ":self".into())
                    ])
            );
        }
    }
}

#[test]
fn incoming_refs_outcomes_are_distinct() {
    let db = Minigraf::in_memory().unwrap();
    assert!(lookup(&db, 9).0.is_empty());
    db.execute(&statement(true, 1, ":a", 9)).unwrap();
    let mut tx = db.begin_write().unwrap();
    exhausted(
        tx.incoming_refs(id(9), IncomingRefLimits::new(0, 10))
            .unwrap(),
        IncomingRefBudget::Links,
        10,
    );
    exhausted(
        tx.incoming_refs(id(9), IncomingRefLimits::new(10, 0))
            .unwrap(),
        IncomingRefBudget::Records,
        0,
    );
    assert_eq!(
        complete(
            tx.incoming_refs(id(9), IncomingRefLimits::new(1, 1))
                .unwrap()
        )
        .0
        .len(),
        1
    );
}

#[test]
fn incoming_refs_unique_pairs_and_any_valid_window() {
    for file in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = database(file, &dir.path().join("windows.graph"));
        for (source, from, to) in [
            (1, "2000", "2001"),
            (2, "2998", "2999"),
            (1, "2002", "2003"),
        ] {
            db.execute(&format!("(transact [[#uuid \"{}\" :a #uuid \"{}\" {{:valid-from \"{from}-01-01T00:00:00Z\" :valid-to \"{to}-01-01T00:00:00Z\"}}]])", id(source), id(9))).unwrap();
        }
        db.execute(&statement(true, 1, ":b", 9)).unwrap();
        db.execute(&statement(true, 1, ":b", 9)).unwrap();
        db.execute(&statement(false, 2, ":a", 9)).unwrap();
        if file {
            db.checkpoint().unwrap();
        }
        assert!(lookup(&db, 9).0 == BTreeSet::from([(id(1), ":a".into()), (id(1), ":b".into())]));
    }
}

#[test]
fn incoming_refs_statement_order_rejected_statement_and_promotion() {
    for file in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = database(file, &dir.path().join("order.graph"));
        db.execute(&statement(true, 1, ":a", 9)).unwrap();
        if file {
            db.checkpoint().unwrap();
        }
        db.execute(&statement(true, 2, ":a", 9)).unwrap();
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(4))
            .unwrap();
        let limits = IncomingRefLimits::new(10, 100);
        for source in [1, 2, 3] {
            tx.execute(&statement(false, source, ":a", 9)).unwrap();
        }
        assert!(
            complete(tx.incoming_refs(id(9), limits).unwrap())
                .0
                .is_empty()
        );
        tx.execute(&statement(true, 3, ":a", 9)).unwrap();
        let before = complete(tx.incoming_refs(id(9), limits).unwrap()).0;
        tx.execute("(transact [[:unrelated :n 1]])").unwrap();
        assert!(complete(tx.incoming_refs(id(9), limits).unwrap()).0 == before);
        let bad = format!(
            "(transact [[#uuid \"{}\" :bad #uuid \"{}\" {{:valid-from \"2000-01-01T00:00:00Z\"}}] [#uuid \"{}\" :bad #uuid \"{}\" {{:valid-from \"2001-01-01T00:00:00Z\"}}]])",
            id(4),
            id(9),
            id(4),
            id(9)
        );
        assert_eq!(tx.execute(&bad).unwrap_err().code(), "API-011");
        assert!(complete(tx.incoming_refs(id(9), limits).unwrap()).0 == before);
        tx.execute(&statement(true, 5, ":new-attribute", 9))
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(lookup(&db, 9).0.len(), 2);
    }
}

#[test]
fn incoming_refs_unknown_pending_only_dangling_and_checkpoint_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.graph");
    let db = database(true, &path);
    let before = main_file_state(&path);
    {
        let mut tx = db.begin_write().unwrap();
        assert!(
            complete(
                tx.incoming_refs(id(999), IncomingRefLimits::new(0, 0))
                    .unwrap()
            )
            .0
            .is_empty()
        );
        tx.commit().unwrap();
    }
    assert!(main_file_state(&path) == before);
    db.execute(&statement(true, 1, ":a", 9)).unwrap();
    assert_eq!(lookup(&db, 9).0.len(), 1);
    db.checkpoint().unwrap();
    let expected = lookup(&db, 9).0;
    drop(db);
    let db = Minigraf::open(&path).unwrap();
    assert!(lookup(&db, 9).0 == expected);
    db.verify().unwrap();
}

#[test]
fn incoming_refs_unknown_target_with_populated_dictionary() {
    for large in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unknown.graph");
        let wal_path = path.with_extension("graph.wal");
        let db = database(true, &path);
        for (source, attr, target) in [(1, ":a", 9), (2, ":b", 9), (3, ":a", 10)] {
            db.execute(&statement(true, source, attr, target)).unwrap();
        }
        db.execute("(transact [[:property :n 42]])").unwrap();
        db.checkpoint().unwrap();
        db.execute(&statement(true, 4, ":c", 9)).unwrap();
        db.execute(&statement(true, 5, ":d", 10)).unwrap();
        let count = db.current_tx_count();
        let before = main_file_state(&path);
        let wal_before = std::fs::read(&wal_path).unwrap();
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(1))
            .unwrap();
        if large {
            // Two facts cross the threshold and populate the working trees.
            tx.execute("(transact [[:filler-a :n 1] [:filler-b :n 2]])")
                .unwrap();
            // Changed pages need not change file length, so this proof needs bytes.
            #[cfg(not(windows))]
            assert!(main_file_state(&path) != before);
        }
        // Promotion may write pages; only lookup itself must leave them intact.
        let state = main_file_state(&path);
        let wal = std::fs::read(&wal_path).unwrap();
        if !large {
            assert!(state == before);
            assert!(wal == wal_before);
        }
        for limits in [
            IncomingRefLimits::new(0, 0),
            IncomingRefLimits::new(usize::MAX, usize::MAX),
        ] {
            let (links, examined) = complete(tx.incoming_refs(id(999), limits).unwrap());
            assert!(links.is_empty());
            assert_eq!(examined, 0);
            assert!(main_file_state(&path) == state);
            assert!(std::fs::read(&wal_path).unwrap() == wal);
            assert_eq!(db.current_tx_count(), count);
        }
        if large {
            tx.rollback();
        } else {
            tx.commit().unwrap();
            assert!(main_file_state(&path) == state);
            assert!(std::fs::read(&wal_path).unwrap() == wal);
        }
        assert_eq!(db.current_tx_count(), count);
        let state = main_file_state(&path);
        let wal = std::fs::read(&wal_path).unwrap();
        db.begin_write().unwrap().commit().unwrap();
        assert!(main_file_state(&path) == state);
        assert!(std::fs::read(&wal_path).unwrap() == wal);
        assert_eq!(db.current_tx_count(), count);
        db.checkpoint().unwrap();
        drop(db);

        let db = database(true, &path);
        let report = db.verify().unwrap();
        assert!(report.is_ok());
        assert_eq!(report.facts, 6);
        assert_eq!(db.current_tx_count(), count);
        assert!(
            lookup(&db, 9).0
                == BTreeSet::from([
                    (id(1), ":a".into()),
                    (id(2), ":b".into()),
                    (id(4), ":c".into())
                ])
        );
        assert!(lookup(&db, 10).0 == BTreeSet::from([(id(3), ":a".into()), (id(5), ":d".into())]));
        let (links, examined) = lookup(&db, 999);
        assert!(links.is_empty());
        assert_eq!(examined, 0);
        // The public fact log exposes neither an entity nor a ref for the unknown ID.
        let records: Vec<_> = db
            .fact_log(&minigraf::FactFilter::new())
            .unwrap()
            .map(|record| record.unwrap())
            .collect();
        assert_eq!(records.len(), 6);
        assert!(records.iter().all(|record| {
            record.entity != id(999) && record.value != minigraf::Value::Ref(id(999))
        }));
    }
}

#[test]
fn incoming_refs_read_only_retryable_exhaustion_rollback_and_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.graph");
    let db = database(true, &path);
    db.execute(&statement(true, 1, ":a", 9)).unwrap();
    let state = main_file_state(&path);
    let wal = std::fs::read(path.with_extension("graph.wal")).unwrap();
    for rollback in [false, true] {
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(1))
            .unwrap();
        tx.execute(&statement(false, 1, ":a", 9)).unwrap();
        exhausted(
            tx.incoming_refs(id(9), IncomingRefLimits::new(0, 0))
                .unwrap(),
            IncomingRefBudget::Records,
            0,
        );
        assert!(
            complete(
                tx.incoming_refs(id(9), IncomingRefLimits::new(10, 10))
                    .unwrap()
            )
            .0
            .is_empty()
        );
        assert!(main_file_state(&path) == state);
        assert!(std::fs::read(path.with_extension("graph.wal")).unwrap() == wal);
        if rollback {
            tx.rollback();
        }
    }
    assert_eq!(lookup(&db, 9).0.len(), 1);
    let mut tx = db.begin_write().unwrap();
    tx.incoming_refs(id(9), IncomingRefLimits::new(0, 10))
        .unwrap();
    tx.execute(&statement(true, 2, ":b", 9)).unwrap();
    tx.commit().unwrap();
    let counts: BTreeSet<_> = db
        .fact_log(&minigraf::FactFilter::new())
        .unwrap()
        .map(|r| r.unwrap().tx_count)
        .collect();
    assert_eq!(counts, BTreeSet::from([1, 2]));
}

#[test]
fn incoming_refs_exact_link_cap_overflow_zero_and_huge_limits() {
    let db = Minigraf::in_memory().unwrap();
    for i in 1..=3 {
        db.execute(&statement(true, i, ":a", 9)).unwrap();
    }
    let mut tx = db.begin_write().unwrap();
    for cap in [3, usize::MAX] {
        assert_eq!(
            complete(
                tx.incoming_refs(id(9), IncomingRefLimits::new(cap, usize::MAX))
                    .unwrap()
            )
            .0
            .len(),
            3
        );
    }
    for cap in [0, 2] {
        exhausted(
            tx.incoming_refs(id(9), IncomingRefLimits::new(cap, usize::MAX))
                .unwrap(),
            IncomingRefBudget::Links,
            usize::MAX,
        );
    }
    assert!(
        complete(
            tx.incoming_refs(id(999), IncomingRefLimits::new(0, 0))
                .unwrap()
        )
        .0
        .is_empty()
    );
    exhausted(
        tx.incoming_refs(id(9), IncomingRefLimits::new(0, 0))
            .unwrap(),
        IncomingRefBudget::Records,
        0,
    );
}

#[test]
fn incoming_refs_budget_shared_across_layers() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(true, &dir.path().join("layers.graph"));
    db.execute(&statement(true, 1, ":a", 9)).unwrap();
    db.checkpoint().unwrap();
    db.execute(&statement(false, 1, ":a", 9)).unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(&statement(true, 1, ":a", 9)).unwrap();
    for cap in 0..3 {
        exhausted(
            tx.incoming_refs(id(9), IncomingRefLimits::new(1, cap))
                .unwrap(),
            IncomingRefBudget::Records,
            cap,
        );
    }
    assert_eq!(
        complete(
            tx.incoming_refs(id(9), IncomingRefLimits::new(1, 3))
                .unwrap()
        )
        .1,
        3
    );
}

#[test]
fn incoming_refs_dead_pairs_exhaust_and_one_pair_long_history() {
    for one_pair in [false, true] {
        for layer in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let db = database(layer != 0, &dir.path().join("history.graph"));
            for i in 0..1000 {
                db.execute(&statement(
                    if one_pair { i == 999 } else { false },
                    if one_pair { 1 } else { i + 1 },
                    ":a",
                    2000,
                ))
                .unwrap();
            }
            if layer == 1 {
                db.checkpoint().unwrap();
            }
            let mut tx = db
                .begin_write_with(TxOptions::new().chunk_facts(1))
                .unwrap();
            if layer == 2 {
                tx.execute("(transact [[:x :n 1] [:y :n 2]])").unwrap();
            }
            exhausted(
                tx.incoming_refs(id(2000), IncomingRefLimits::new(10, 20))
                    .unwrap(),
                IncomingRefBudget::Records,
                20,
            );
            let (expected, b) = complete(
                tx.incoming_refs(id(2000), IncomingRefLimits::new(10, 10000))
                    .unwrap(),
            );
            assert_eq!(expected.len(), usize::from(one_pair));
            assert!(
                complete(
                    tx.incoming_refs(id(2000), IncomingRefLimits::new(10, b))
                        .unwrap()
                )
                .0 == expected
            );
            match tx
                .incoming_refs(id(2000), IncomingRefLimits::new(10, b - 1))
                .unwrap()
            {
                IncomingRefs::Complete { links, .. } => assert_eq!(links.len(), expected.len()),
                out => exhausted(out, IncomingRefBudget::Records, b - 1),
            }
        }
    }
}

#[test]
fn incoming_refs_wal_recovery_and_recovered_then_promoted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal.graph");
    common::run_crashing_child(
        &path,
        1000,
        &[&statement(true, 1, ":a", 9)],
        common::CrashTx::Explicit,
    );
    let other = dir.path().join("implicit.graph");
    common::run_crashing_child(
        &other,
        1000,
        &[&statement(true, 1, ":a", 9)],
        common::CrashTx::Implicit,
    );
    assert_eq!(lookup(&database(true, &other), 9).0.len(), 1);
    let db = database(true, &path);
    assert_eq!(lookup(&db, 9).0.len(), 1);
    let mut tx = db
        .begin_write_with(TxOptions::new().chunk_facts(1))
        .unwrap();
    tx.execute(&statement(false, 1, ":a", 9)).unwrap();
    assert!(
        complete(
            tx.incoming_refs(id(9), IncomingRefLimits::new(10, 10))
                .unwrap()
        )
        .0
        .is_empty()
    );
    tx.execute("(transact [[:x :n 1]])").unwrap();
    assert!(
        complete(
            tx.incoming_refs(id(9), IncomingRefLimits::new(10, 10))
                .unwrap()
        )
        .0
        .is_empty()
    );
    tx.rollback();
    assert_eq!(lookup(&db, 9).0.len(), 1);
}

use proptest::prelude::*;

// More cases: PROPTEST_CASES=500 cargo test --test incoming_refs_test incoming_refs_model
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn incoming_refs_model(ops in prop::collection::vec((0u8..10, 1u128..5, 0usize..2, 8u128..11), 1..80)) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.graph");
        let mut db = database(true, &path);
        let mut committed = BTreeSet::new();
        let mut model = committed.clone();
        let mut tx = db.begin_write_owned_with(TxOptions::new().chunk_facts(5)).unwrap();
        for (op, source, attr, target) in ops {
            let attr = [":a", ":b"][attr];
            match op {
                0..=3 => { tx.execute(&statement(true, source, attr, target)).unwrap(); model.insert((source, attr, target)); }
                4 => { tx.execute(&statement(false, source, attr, target)).unwrap(); model.remove(&(source, attr, target)); }
                5 => { tx.execute(&format!("(transact [[#uuid \"{}\" {attr} :{}]])", id(source), id(target))).unwrap(); }
                6 => { tx.commit().unwrap(); committed = model.clone(); tx = db.begin_write_owned_with(TxOptions::new().chunk_facts(5)).unwrap(); }
                7 => { tx.rollback(); model = committed.clone(); tx = db.begin_write_owned().unwrap(); }
                8 | 9 => {
                    tx.commit().unwrap(); committed = model.clone(); db.checkpoint().unwrap();
                    if op == 9 { drop(db); db = database(true, &path); }
                    tx = db.begin_write_owned_with(TxOptions::new().chunk_facts(1)).unwrap();
                }
                _ => unreachable!(),
            }
            for t in 8..11 {
                let got = complete(tx.incoming_refs(id(t), IncomingRefLimits::new(100, 10000)).unwrap()).0;
                let expected = model.iter().filter(|(_,_,target)| *target == t).map(|(s,a,_)| (id(*s), (*a).to_string())).collect();
                prop_assert!(got == expected);
            }
        }
    }
}

#[test]
fn incoming_refs_external_types_and_large_read_only() {
    let constructed = minigraf::IncomingRef::new(id(1), ":a".into());
    assert!(constructed.source == id(1));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large-read.graph");
    let db = database(true, &path);
    let mut tx = db
        .begin_write_with(TxOptions::new().chunk_facts(1))
        .unwrap();
    tx.execute(&statement(true, 1, ":a", 9)).unwrap();
    tx.execute(&statement(true, 2, ":b", 9)).unwrap();
    let before = main_file_state(&path);
    let wal = std::fs::read(path.with_extension("graph.wal")).unwrap();
    let count = db.current_tx_count();
    assert!(
        complete(
            tx.incoming_refs(id(999), IncomingRefLimits::new(0, 0))
                .unwrap()
        )
        .0
        .is_empty()
    );
    exhausted(
        tx.incoming_refs(id(9), IncomingRefLimits::new(0, 1))
            .unwrap(),
        IncomingRefBudget::Records,
        1,
    );
    assert_eq!(
        complete(
            tx.incoming_refs(id(9), IncomingRefLimits::new(2, 10))
                .unwrap()
        )
        .0
        .len(),
        2
    );
    assert!(main_file_state(&path) == before);
    assert!(std::fs::read(path.with_extension("graph.wal")).unwrap() == wal);
    assert_eq!(db.current_tx_count(), count);
    tx.commit().unwrap();
    db.checkpoint().unwrap();
    db.verify().unwrap();
    drop(db);
    assert_eq!(lookup(&Minigraf::open(&path).unwrap(), 9).0.len(), 2);
}

#[test]
fn incoming_refs_promotion_exact_set() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(true, &dir.path().join("promotion-set.graph"));
    db.execute(&statement(true, 1, ":committed", 9)).unwrap();
    db.checkpoint().unwrap();
    let mut tx = db
        .begin_write_with(TxOptions::new().chunk_facts(2))
        .unwrap();
    tx.execute(&statement(true, 2, ":staged", 9)).unwrap();
    tx.execute("(transact [[:filler :n 1]])").unwrap();
    let limits = IncomingRefLimits::new(10, 100);
    let before = complete(tx.incoming_refs(id(9), limits).unwrap()).0;
    assert!(before == BTreeSet::from([(id(1), ":committed".into()), (id(2), ":staged".into())]));
    tx.execute("(transact [[:promote :n 2]])").unwrap();
    assert!(complete(tx.incoming_refs(id(9), limits).unwrap()).0 == before);
    tx.execute(&statement(true, 3, ":created-after-promotion", 9))
        .unwrap();
    tx.execute(&statement(false, 1, ":committed", 9)).unwrap();
    let expected = BTreeSet::from([
        (id(2), ":staged".into()),
        (id(3), ":created-after-promotion".into()),
    ]);
    assert!(complete(tx.incoming_refs(id(9), limits).unwrap()).0 == expected);
    tx.commit().unwrap();
    assert!(lookup(&db, 9).0 == expected);
}
