#![cfg(test)]
//! Nightly acceptance for the #841 shape-sweep candidates qualified by #863:
//! each test pins one candidate's answer and the path it guards (operator
//! names from the cursor's `Debug` view, `minigraf::__path_events`) and,
//! where the behavior is memory, its charged query memory
//! (`Cursor::__memory_peak`). Time is measured separately, by the Criterion
//! cases in `benches/minigraf_bench/nightly_candidates.rs`; a path regression
//! fails here, a slower runner cannot.
//!
//! Ignored by default; `.github/workflows/smoke.yml` runs them nightly:
//!
//! ```text
//! cargo test --profile bench --features test-hooks --test nightly_candidates_acceptance_test -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `shape_port_matches_the_database` is not ignored: it checks the shared
//! fixture port against the database at small sizes on every test run.
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

#[path = "../benches/helpers/shape.rs"]
mod shape;

use minigraf::{Cursor, Minigraf, OpenOptions, Value};
use shape::{Digest, Shape, drain, entity};

fn fixture(shape: Shape, pending: bool) -> (tempfile::TempDir, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let db = shape.build(&dir.path().join("candidate.graph"), pending);
    (dir, db)
}

/// Pull exactly `rows` rows into a digest, leaving the stream open: a larger
/// request pulls past the end, and the cursor then drops its stream.
fn pull(cursor: &mut Cursor, rows: usize, ordered: bool) -> Digest {
    let mut digest = Digest::default();
    let mut n = 0;
    while n < rows {
        let batch = cursor
            .next_batch((rows - n).min(1000))
            .unwrap()
            .expect("more rows");
        for row in batch.rows() {
            digest.add(row, ordered);
        }
        n += batch.rows().len();
    }
    digest
}

fn ops(cursor: &Cursor) -> String {
    // Operator counters only: checked before any row is formatted.
    format!("{cursor:?}")
}

#[test]
fn shape_port_matches_the_database() {
    for (shape, pending) in [
        (Shape::WindowSkewSum { n: 1000 }, false),
        (Shape::SetMembership { n: 100, k: 8 }, false),
        (
            Shape::Sort {
                n: 1000,
                desc: true,
                offset: 500,
                limit: Some(10),
            },
            false,
        ),
        (
            Shape::Sort {
                n: 1000,
                desc: false,
                offset: 500,
                limit: Some(10),
            },
            true,
        ),
        (Shape::Ancestors { edges: 110, k: 1 }, true),
        (Shape::Ancestors { edges: 110, k: 4 }, false),
    ] {
        let (_dir, db) = fixture(shape, pending);
        assert_eq!(
            drain(&db, &shape.query(), shape.ordered()),
            shape.oracle(),
            "port answer"
        );
    }
}

/// N1: 50,000 rows (100k facts), 90% in one partition, running sum,
/// committed. Partition-at-a-time windows (#641) retain one partition: the
/// largest, not less, and not the whole input.
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n1_skew_running_sum_streams_by_partition() {
    let shape = Shape::WindowSkewSum { n: 50_000 };
    let (_dir, db) = fixture(shape, false);
    let mut cursor = db.query(&shape.query()).unwrap();
    let plan = ops(&cursor);
    assert!(plan.contains("WindowPartitioned"), "partitioned window");
    let digest = pull(&mut cursor, shape.rows(), false);
    let partitioned = cursor.__memory_peak(Some("Window")).expect("open");
    assert_eq!(digest, shape.oracle(), "answer");
    drop(cursor);

    let full = {
        let _guard = minigraf::__input_order(false);
        let mut cursor = db.query(&shape.query()).unwrap();
        let plan = ops(&cursor);
        assert!(!plan.contains("WindowPartitioned"), "full-input control");
        assert_eq!(pull(&mut cursor, shape.rows(), false), shape.oracle());
        cursor.__memory_peak(Some("Window")).expect("open")
    };
    println!("PEAK_CHECK n1 partitioned_bytes={partitioned} full_bytes={full}");
    // Charged bytes are deterministic: 0.907 of the full input on 2026-11-06
    // (45,000 of 50,000 rows plus fixed costs). Retaining the whole input is
    // 1.0; retaining less than the largest partition is below 0.9.
    assert!(partitioned * 100 <= full * 95, "not the whole input");
    assert!(
        partitioned * 100 >= full * 85,
        "the largest partition (90% of rows) is retained"
    );
}

/// N2: 10k entities with 256-element skewed sets (long value pages),
/// committed; six membership predicates. Answer only: no event identifies
/// the long-set path (#863 attribution limit).
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n2_set_membership_k256() {
    let shape = Shape::SetMembership { n: 10_000, k: 256 };
    let (_dir, db) = fixture(shape, false);
    assert_eq!(drain(&db, &shape.query(), false), shape.oracle(), "answer");
}

/// N3: 100k facts, descending `:offset 50000 :limit 1000`, committed. The
/// Sort heap holds `offset + limit` rows (#306), not every row; ascending,
/// the committed index order omits the Sort (#641).
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n3_sort_heap_holds_offset_plus_limit() {
    let n = 100_000;
    let limited = Shape::Sort {
        n,
        desc: true,
        offset: 50_000,
        limit: Some(1000),
    };
    let (_dir, db) = fixture(limited, false);
    let mut cursor = db.query(&limited.query()).unwrap();
    assert!(ops(&cursor).contains("Sort"), "descending sorts");
    assert_eq!(pull(&mut cursor, 1000, true), limited.oracle(), "answer");
    let heap = cursor.__memory_peak(None).expect("open");
    drop(cursor);

    let all = Shape::Sort {
        n,
        desc: true,
        offset: 0,
        limit: None,
    };
    let mut cursor = db.query(&all.query()).unwrap();
    assert_eq!(pull(&mut cursor, n, true), all.oracle(), "control answer");
    let full = cursor.__memory_peak(None).expect("open");
    drop(cursor);
    println!("PEAK_CHECK n3 heap_bytes={heap} full_bytes={full}");
    // 0.51 of the full Sort on 2026-11-06 (deterministic charges); a heap of
    // every row is 1.0.
    assert!(heap * 10 <= full * 6, "heap of 51,000 of 100,000 rows");

    let ascending = Shape::Sort {
        n,
        desc: false,
        offset: 50_000,
        limit: Some(1000),
    };
    let cursor = db.query(&ascending.query()).unwrap();
    assert!(!ops(&cursor).contains("Sort"), "ascending omits the Sort");
    drop(cursor);
    assert_eq!(drain(&db, &ascending.query(), true), ascending.oracle());
}

/// N4: a 1,110-edge tree, one leaf's ancestors, pending (empty committed
/// dictionary): the small in-memory ancestor alternative (#723).
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n4_small_tree_takes_ancestor_alternative() {
    let shape = Shape::Ancestors { edges: 1110, k: 1 };
    let (_dir, db) = fixture(shape, true);
    minigraf::__path_events();
    assert_eq!(drain(&db, &shape.query(), false), shape.oracle(), "answer");
    let events = minigraf::__path_events();
    assert!(
        events
            .get("ancestor_alternative_selected")
            .is_some_and(|n| *n >= 1),
        "alternative selected"
    );
}

/// N5: the same tree, 16 demanded leaves, committed: bound-value AVET probes
/// (#721), and not the in-memory alternative.
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n5_committed_tree_uses_bound_value_probes() {
    let shape = Shape::Ancestors { edges: 1110, k: 16 };
    let (_dir, db) = fixture(shape, false);
    minigraf::__path_events();
    assert_eq!(drain(&db, &shape.query(), false), shape.oracle(), "answer");
    let events = minigraf::__path_events();
    assert!(
        events
            .get("bound_value_avet_range")
            .is_some_and(|n| *n >= 1),
        "bound-value AVET probes"
    );
    assert!(
        !events.contains_key("ancestor_alternative_selected"),
        "committed generation keeps the probes"
    );
}

/// N6: 10k facts, one cursor held across 256 of 544 checkpoints, each
/// retracting and asserting 100 entities' values (history kept). While the
/// cursor holds its generation, freed pages are quarantined (#543) and the
/// file grows faster; after release they are reused. The held cursor still
/// reads its own state, and the file verifies.
#[test]
#[ignore = "nightly acceptance (#863)"]
fn n6_held_cursor_quarantine_and_reuse() {
    const N: usize = 10_000;
    const QUERY: &str = "(query [:find ?e ?v :where [?e :pin/value ?v]])";
    let (baseline, held, after) = (32, 256, 256);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("churn.graph");
    {
        let mut writer = minigraf::LogWriter::create(&path, OpenOptions::new()).unwrap();
        for i in 0..N {
            writer
                .append(&minigraf::FactRecord {
                    entity: entity(i),
                    attribute: ":pin/value".into(),
                    value: Value::Integer(i64::try_from(i).expect("fixture value fits in i64")),
                    tx_count: (i / 1000 + 1) as u64,
                    tx_id: 1_700_000_000_000 + (i / 1000) as u64,
                    valid_from: 0,
                    valid_to: i64::MAX,
                    asserted: true,
                })
                .unwrap();
        }
        writer.finish().unwrap();
    }
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(&path)
        .open()
        .unwrap();
    let mut values: Vec<usize> = (0..N).collect();
    let mut cursor = None;
    let mut expected = Vec::new();
    let mut sizes = Vec::new();
    for round in 0..baseline + held + after {
        if round == baseline {
            cursor = Some(db.query(QUERY).unwrap());
            expected = values.clone();
        }
        if round == baseline + held {
            let mut cursor: Cursor = cursor.take().expect("held cursor");
            let mut seen = vec![false; N];
            while let Some(batch) = cursor.next_batch(1000).unwrap() {
                for row in batch.rows() {
                    let [Value::Ref(e), Value::Integer(v)] = row.as_slice() else {
                        panic!("entity and value expected");
                    };
                    let i = (e.as_u128() - entity(0).as_u128()) as usize;
                    assert_eq!(
                        usize::try_from(*v).expect("fixture value fits in usize"),
                        expected[i],
                        "held cursor reads its state"
                    );
                    assert!(!seen[i], "one row per entity");
                    seen[i] = true;
                }
            }
            assert!(seen.iter().all(|s| *s), "every entity");
        }
        let mut retract = String::from("(retract [");
        let mut transact = String::from("(transact [");
        for j in 0..100 {
            let i = (round * 100 + j) % N;
            retract.push_str(&format!(
                "[#uuid \"{}\" :pin/value {}]",
                entity(i),
                values[i]
            ));
            values[i] += N;
            transact.push_str(&format!(
                "[#uuid \"{}\" :pin/value {}]",
                entity(i),
                values[i]
            ));
        }
        retract.push_str("])");
        transact.push_str("])");
        db.execute(&retract).unwrap();
        db.execute(&transact).unwrap();
        db.checkpoint().unwrap();
        sizes.push(std::fs::metadata(&path).unwrap().len());
    }
    // Growth over the last 128 held rounds, and the last 128 after release.
    let growth = |end: usize| sizes[end - 1] - sizes[end - 129];
    let held_growth = growth(baseline + held);
    let after_growth = growth(baseline + held + after);
    println!("CHURN_CHECK n6 held_growth_bytes={held_growth} after_growth_bytes={after_growth}");
    assert!(held_growth > 0, "quarantine grows the file");
    // 10,280,960 and 0 bytes on 2026-11-06: once released, the freed pages
    // absorb the churn and the file stops growing.
    assert!(after_growth * 2 <= held_growth, "released pages are reused");
    assert!(db.verify().unwrap().is_ok(), "integrity");
}
