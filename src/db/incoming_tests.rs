use super::*;
use crate::storage::fault::{self, Fault};
use crate::{IncomingRefBudget, IncomingRefLimits, IncomingRefs};

fn target() -> uuid::Uuid {
    uuid::Uuid::from_u128(9)
}
fn assert_one(out: IncomingRefs) {
    assert!(
        matches!(out, IncomingRefs::Complete { links, .. } if links.len() == 1 && links[0].attribute == ":a" && links[0].source == uuid::Uuid::from_u128(1))
    );
}

#[test]
fn incoming_refs_on_failed_handle() {
    for checkpoint in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("failed.graph")).unwrap();
        db.execute(&format!(
            "(transact [[#uuid \"{}\" :a #uuid \"{}\"]])",
            uuid::Uuid::from_u128(1),
            target()
        ))
        .unwrap();
        fault::arm(Fault::Torn(9), 0);
        let failed = if checkpoint {
            db.checkpoint().map(|_| QueryResult::Ok)
        } else {
            db.execute("(transact [[:b :n 2]])")
        };
        fault::disarm();
        assert!(failed.is_err());
        assert_eq!(
            db.execute("(transact [[:b :n 3]])").unwrap_err().code(),
            if checkpoint { "STG-046" } else { "WAL-007" }
        );
        let mut tx = db.begin_write().unwrap();
        assert_one(
            tx.incoming_refs(target(), IncomingRefLimits::new(10, 10))
                .unwrap(),
        );
        assert!(matches!(
            tx.incoming_refs(target(), IncomingRefLimits::new(10, 0))
                .unwrap(),
            IncomingRefs::Exhausted {
                budget: IncomingRefBudget::Records,
                records_examined: 0
            }
        ));
        tx.execute("(query [:find ?e :where [?e :a ?v]])").unwrap();
        tx.rollback();
        let mut tx = db.begin_write_owned().unwrap();
        assert_one(
            tx.incoming_refs(target(), IncomingRefLimits::new(10, 10))
                .unwrap(),
        );
        tx.rollback();
    }
}

#[test]
fn incoming_refs_legacy_ties_and_group_budget() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("legacy.graph")).unwrap();
        let mut facts = Vec::new();
        for source in 1..=2 {
            for window in 0..5 {
                let mut f = Fact::new(
                    uuid::Uuid::from_u128(source),
                    ":a".into(),
                    Value::Ref(target()),
                    100,
                );
                f.tx_count = 1;
                f.valid_from = window;
                f.valid_to = window + 1;
                f.asserted = source == 1 || window != 4;
                facts.push(f);
            }
        }
        db.inner.fact_storage.load_facts(facts).unwrap();
        if durable {
            db.checkpoint().unwrap();
        }
        let mut tx = db.begin_write().unwrap();
        for cap in 0..10 {
            crate::incoming::enumeration();
            let out = tx
                .incoming_refs(target(), IncomingRefLimits::new(10, cap))
                .unwrap();
            assert!(
                matches!(out, IncomingRefs::Exhausted { budget: IncomingRefBudget::Records, records_examined } if records_examined == cap)
            );
            assert_eq!(crate::incoming::enumeration().0, cap);
        }
        assert_one(
            tx.incoming_refs(target(), IncomingRefLimits::new(1, 10))
                .unwrap(),
        );
    }
}

#[test]
fn incoming_refs_unrelated_growth() {
    use std::fmt::Write;
    for layer in 0..4 {
        let mut baseline = None;
        for unrelated in [0, 100_000] {
            let dir = tempfile::tempdir().unwrap();
            let db = if layer < 2 {
                Minigraf::in_memory().unwrap()
            } else {
                Minigraf::open_with_options(
                    dir.path().join("growth.graph"),
                    OpenOptions::default().wal_checkpoint_threshold(usize::MAX),
                )
                .unwrap()
            };
            let mut target_command = String::from("(transact [");
            for source in 1..=3 {
                write!(
                    target_command,
                    "[#uuid \"{}\" :a #uuid \"{}\"]",
                    uuid::Uuid::from_u128(source),
                    target()
                )
                .unwrap();
            }
            target_command.push_str("])");
            let mut command = String::from("(transact [");
            for i in 0..unrelated {
                let value = match i % 3 {
                    0 => i.to_string(),
                    1 => ":other".into(),
                    _ => format!("#uuid \"{}\"", uuid::Uuid::from_u128(10_000_000)),
                };
                write!(command, "[:e{i} :unrelated {value}]").unwrap();
            }
            command.push_str("])");
            if layer != 1 && layer != 3 {
                db.execute(&target_command).unwrap();
                if unrelated != 0 {
                    db.execute(&command).unwrap();
                }
                if layer == 2 {
                    db.checkpoint().unwrap();
                }
            }
            let mut tx = db
                .begin_write_with(TxOptions::new().chunk_facts(if layer == 3 {
                    2
                } else {
                    1_000_000
                }))
                .unwrap();
            if layer == 1 || layer == 3 {
                tx.execute(&target_command).unwrap();
                if layer == 3 {
                    tx.state.chunk_facts = 65_536;
                }
                if unrelated != 0 {
                    tx.execute(&command).unwrap();
                }
            }
            // The first call must not build an index or copy unrelated staged facts.
            for _ in 0..2 {
                crate::incoming::enumeration();
                let out = tx
                    .incoming_refs(target(), IncomingRefLimits::new(3, 3))
                    .unwrap();
                let IncomingRefs::Complete {
                    links,
                    records_examined,
                } = out
                else {
                    panic!("expected complete");
                };
                assert_eq!(links.len(), 3);
                let (enumerated, ends) = crate::incoming::enumeration();
                assert_eq!(enumerated, 3);
                assert_eq!(records_examined, *baseline.get_or_insert(records_examined));
                assert_eq!(ends, usize::from(layer >= 2 && unrelated != 0));
            }
            for cap in 0..3 {
                crate::incoming::enumeration();
                assert!(
                    matches!(tx.incoming_refs(target(), IncomingRefLimits::new(3, cap)).unwrap(), IncomingRefs::Exhausted { budget: IncomingRefBudget::Records, records_examined } if records_examined == cap)
                );
                assert_eq!(crate::incoming::enumeration().0, cap);
            }
        }
    }
}

#[test]
fn incoming_refs_history_enumeration_is_bounded() {
    for layer in 0..4 {
        for one_pair in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let db = if layer < 2 {
                Minigraf::in_memory().unwrap()
            } else {
                Minigraf::open(dir.path().join("bounded.graph")).unwrap()
            };
            let facts: Vec<_> = (0..1000)
                .map(|i| {
                    let mut f = Fact::new(
                        uuid::Uuid::from_u128(if one_pair { 1 } else { i + 1 }),
                        ":a".into(),
                        Value::Ref(target()),
                        100,
                    );
                    f.tx_count = u64::try_from(i + 1).unwrap();
                    f.asserted = one_pair && i == 999;
                    f
                })
                .collect();
            if layer != 1 {
                db.inner.fact_storage.load_facts(facts.clone()).unwrap();
                db.inner.fact_storage.restore_tx_counter().unwrap();
            }
            if layer == 2 {
                db.checkpoint().unwrap();
            }
            let mut tx = db
                .begin_write_with(TxOptions::new().chunk_facts(1))
                .unwrap();
            if layer == 1 {
                for f in facts {
                    tx.state.staged.stage(vec![f]);
                }
            }
            if layer == 3 {
                tx.execute("(transact [[:x :n 1] [:y :n 2]])").unwrap();
            }
            crate::incoming::enumeration();
            assert!(matches!(
                tx.incoming_refs(target(), IncomingRefLimits::new(10, 20))
                    .unwrap(),
                IncomingRefs::Exhausted {
                    budget: IncomingRefBudget::Records,
                    records_examined: 20
                }
            ));
            assert_eq!(crate::incoming::enumeration().0, 20);
        }
    }
}

#[test]
fn incoming_refs_non_ref_staging_keeps_batch_capacity() {
    let mut staged = Staged::new(0);
    let facts = (0..1000)
        .map(|i| Fact::new(uuid::Uuid::from_u128(i), ":n".into(), Value::Integer(1), 0))
        .collect();
    staged.stage(facts);
    assert_eq!(staged.pending_facts.capacity(), 1000);
}

fn ref_statement(asserted: bool, source: u128, attr: &str) -> String {
    format!(
        "({} [[#uuid \"{}\" {attr} #uuid \"{}\"]])",
        if asserted { "transact" } else { "retract" },
        uuid::Uuid::from_u128(source),
        target()
    )
}

fn incoming_set(tx: &mut WriteTransaction<'_>) -> std::collections::BTreeSet<(uuid::Uuid, String)> {
    match tx
        .incoming_refs(target(), IncomingRefLimits::new(100, 1000))
        .unwrap()
    {
        IncomingRefs::Complete { links, .. } => {
            links.into_iter().map(|l| (l.source, l.attribute)).collect()
        }
        _ => panic!("expected complete links"),
    }
}

#[test]
fn incoming_refs_failed_promotion_keeps_completed_statements() {
    for asserted in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("promotion-failure.graph")).unwrap();
        if !asserted {
            db.execute(&ref_statement(true, 1, ":a")).unwrap();
            db.checkpoint().unwrap();
        }
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(2))
            .unwrap();
        tx.execute(&ref_statement(asserted, 1, ":a")).unwrap();
        tx.execute(&ref_statement(true, 7, ":z")).unwrap();
        let expected = incoming_set(&mut tx);
        // WAL header succeeds, then the first staged-chunk page write fails.
        fault::arm(Fault::Eio, 1);
        let failure = tx.execute(&ref_statement(true, 2, ":rejected"));
        let site = fault::disarm();
        assert!(failure.is_err());
        assert_eq!(site, Some(fault::Site::PageWrite));
        assert!(
            incoming_set(&mut tx) == expected,
            "completed staged statements survive failed promotion"
        );
        assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
    }
}

#[test]
fn incoming_refs_failed_large_statement_keeps_completed_statements() {
    let run = |at| {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("statement-failure.graph")).unwrap();
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(2))
            .unwrap();
        tx.execute(&ref_statement(true, 1, ":a")).unwrap();
        tx.execute(&ref_statement(true, 7, ":z")).unwrap();
        tx.execute("(transact [[:promote :n 1]])").unwrap();
        let expected = incoming_set(&mut tx);
        let mut command = ref_statement(false, 1, ":a");
        command.truncate(command.len() - 2);
        for source in 10..16 {
            command.push_str(&format!(
                "[#uuid \"{}\" :rejected #uuid \"{}\"]",
                uuid::Uuid::from_u128(source),
                target()
            ));
        }
        command.push_str("])");
        fault::arm(Fault::Eio, at);
        let result = tx.execute(&command);
        let writes = fault::count();
        fault::disarm();
        if at != u64::MAX {
            assert!(result.is_err());
            assert!(
                incoming_set(&mut tx) == expected,
                "a failed chunk contributes no links or retractions"
            );
            assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
        } else {
            result.unwrap();
        }
        writes
    };
    let writes = run(u64::MAX);
    assert!(writes > 1);
    for at in 0..writes {
        run(at);
    }
}

#[test]
fn incoming_refs_setup_work_is_counted() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(&ref_statement(true, 1, ":a")).unwrap();
    db.execute("(transact [[:other :n 1]])").unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(&ref_statement(true, 2, ":b")).unwrap();
    tx.execute("(transact [[:staged :n 2]])").unwrap();
    crate::incoming::enumeration();
    let staged = tx.state.staged.read_view();
    assert!(
        crate::incoming::enumeration().0 >= staged.len(),
        "staged cloning must be counted"
    );
    crate::incoming::enumeration();
    let snapshot = db
        .inner
        .fact_storage
        .snapshot()
        .unwrap()
        .with_staged(staged);
    assert!(
        crate::incoming::enumeration().0 >= 2,
        "staged reindexing must be counted"
    );
    crate::incoming::enumeration();
    let all = snapshot.pending_range(0, usize::MAX).unwrap();
    assert!(
        crate::incoming::enumeration().0 >= all.len(),
        "range copies must be counted"
    );
    crate::incoming::enumeration();
    let all = snapshot
        .pending_records(crate::storage::Scan::All, &crate::storage::TxBound::All)
        .unwrap();
    assert!(
        crate::incoming::enumeration().0 >= all.len(),
        "pending scans must be counted"
    );
    crate::incoming::enumeration();
    let all = db.inner.fact_storage.get_pending_facts();
    assert!(
        crate::incoming::enumeration().0 >= all.len(),
        "whole-store copies must be counted"
    );
}

#[test]
fn incoming_refs_failed_end_statement_keeps_completed_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open_with_options(
        dir.path().join("end-failure.graph"),
        OpenOptions::default().page_cache_size(0),
    )
    .unwrap();
    let mut tx = db
        .begin_write_with(TxOptions::new().chunk_facts(1))
        .unwrap();
    tx.execute(&ref_statement(true, 1, ":a")).unwrap();
    tx.execute(&ref_statement(true, 7, ":z")).unwrap();
    let expected = incoming_set(&mut tx);
    // Execute a full chunk, then fail the claims-tree read at statement end.
    // These are the same three phases TxState::write_statement calls.
    let (WriteContext::File { pfs, .. }, Some(large)) =
        (&mut tx.guard.ctx, tx.state.large.as_mut())
    else {
        panic!("expected large transaction");
    };
    pfs.begin_statement(large);
    let mut facts = vec![
        Fact::retract(
            uuid::Uuid::from_u128(1),
            ":a".into(),
            Value::Ref(target()),
            large.tx_id(),
        ),
        Fact::new(
            uuid::Uuid::from_u128(2),
            ":rejected".into(),
            Value::Ref(target()),
            large.tx_id(),
        ),
    ];
    assert!(pfs.write_large_chunk(large, &mut facts).is_ok());
    fault::arm(Fault::ReadEio, 0);
    let ended = pfs.end_statement(large, true);
    assert_eq!(fault::disarm(), Some(fault::Site::PageRead));
    assert!(ended.is_err());
    assert!(
        incoming_set(&mut tx) == expected,
        "failed finalization must not expose its chunk"
    );
    assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
}

#[test]
fn incoming_refs_promotion_filters_before_stamping() {
    fn strings<'a>(facts: impl Iterator<Item = &'a Fact>) -> Vec<*const u8> {
        let mut pointers = Vec::new();
        for f in facts {
            pointers.push(f.attribute.as_ptr());
            if let Value::String(s) = &f.value {
                pointers.push(s.as_ptr());
            }
        }
        pointers.sort_unstable();
        pointers
    }

    for fail in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("promotion-stamping.graph")).unwrap();
        db.execute(&ref_statement(true, 2, ":removed")).unwrap();
        db.checkpoint().unwrap();
        let mut tx = db
            .begin_write_with(TxOptions::new().chunk_facts(9))
            .unwrap();
        for asserted in [true, false, true] {
            tx.execute(&ref_statement(asserted, 1, ":kept")).unwrap();
        }
        for asserted in [false, true, false] {
            tx.execute(&ref_statement(asserted, 2, ":removed")).unwrap();
        }
        // Legacy tied records in the latest statement retain both versions;
        // stamping them must keep the retraction's precedence.
        tx.state.staged.stage(vec![
            Fact::new(
                uuid::Uuid::from_u128(3),
                ":tied".into(),
                Value::Ref(target()),
                0,
            ),
            Fact::retract(
                uuid::Uuid::from_u128(3),
                ":tied".into(),
                Value::Ref(target()),
                0,
            ),
        ]);
        tx.execute("(transact [[:text :payload \"retained string allocation\"]])")
            .unwrap();
        let expected =
            std::collections::BTreeSet::from([(uuid::Uuid::from_u128(1), ":kept".to_string())]);
        assert!(incoming_set(&mut tx) == expected);
        let facts = &tx.state.staged.pending_facts;
        let allocation = facts.as_ptr();
        let capacity = facts.capacity();
        let retained_strings = strings(
            facts
                .iter()
                .zip(last_statement_mask(facts))
                .filter_map(|(f, keep)| keep.then_some(f)),
        );
        if fail {
            fault::arm(Fault::Eio, 1);
        }
        let result = tx.execute("(transact [[:promote :n 1]])");
        if fail {
            assert_eq!(fault::disarm(), Some(fault::Site::PageWrite));
            assert!(result.is_err());
            let facts = &tx.state.staged.pending_facts;
            assert_eq!(facts.len(), 5, "only the latest statements survive");
            assert!(facts.as_ptr() == allocation, "reuse the staged vector");
            assert_eq!(facts.capacity(), capacity);
            assert!(
                strings(facts.iter()) == retained_strings,
                "do not clone strings"
            );
            assert!(facts.iter().all(|f| f.tx_count == 2));
            assert!(incoming_set(&mut tx) == expected);
            assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
        } else {
            result.unwrap();
            assert!(tx.state.staged.pending_facts.is_empty());
            assert!(incoming_set(&mut tx) == expected);
            tx.commit().unwrap();
            let mut tx = db.begin_write().unwrap();
            assert!(incoming_set(&mut tx) == expected);
        }
    }
}
