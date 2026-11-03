//! #776 P07 / G08: representative streaming-operator controls.
//!
//! All fixtures and answer checks are lazy (inside selected callbacks), retained
//! across Criterion samples, and outside `iter`. No test-hook build is needed:
//! join counters are checked through the open cursor's Debug view, and
//! the natural planner choices are covered by `planner_chooses_the_join`,
//! `value_probe_reads_only_the_probed_keys`, `entity_probe_reuses_repeated_entities`
//! and `planner_chooses_the_body_form` in the engine's counter/plan tests.
//! Run once, untimed: cargo bench --bench minigraf_bench -- --test 'operator_paths/'

use criterion::{BenchmarkId, Criterion};
use minigraf::{FactRecord, LogWriter, Minigraf, OpenOptions, Value};
use std::hint::black_box;
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;

const N: usize = 10_000;

struct Fixture {
    // Drop the database before removing its directory (including on Windows).
    db: Minigraf,
    _dir: Option<TempDir>,
}

impl Fixture {
    fn new(committed: bool, populate: impl FnOnce(&Minigraf)) -> Self {
        let dir = committed.then(|| tempfile::tempdir().unwrap());
        let db = match &dir {
            Some(dir) => OpenOptions::new()
                .wal_checkpoint_threshold(usize::MAX)
                .path(dir.path().join("operators.graph"))
                .open()
                .unwrap(),
            None => Minigraf::in_memory().unwrap(),
        };
        populate(&db);
        if committed {
            db.checkpoint().unwrap();
        }
        Self { db, _dir: dir }
    }
}

fn insert(db: &Minigraf, n: usize, facts: impl Fn(usize) -> String) {
    for start in (0..n).step_by(500) {
        let mut command = String::from("(transact [");
        for i in start..(start + 500).min(n) {
            command.push_str(&facts(i));
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
}

fn drain(db: &Minigraf, query: &str) -> usize {
    let mut cursor = db.query(black_box(query)).unwrap();
    let mut count = 0;
    while let Some(batch) = cursor.next_batch(1000).unwrap() {
        count += batch.rows().len();
        black_box(batch);
    }
    black_box(count)
}

fn case(
    c: &mut Criterion,
    group_name: &str,
    name: &str,
    query: &str,
    setup: impl Fn() -> Fixture,
    check: impl Fn(&Minigraf),
) {
    let mut group = c.benchmark_group(group_name);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    let mut fixture = None;
    group.bench_function(BenchmarkId::new(name, "10k"), |b| {
        let fixture = fixture.get_or_insert_with(|| {
            let fixture = setup();
            check(&fixture.db);
            fixture
        });
        b.iter(|| drain(&fixture.db, query));
    });
    group.finish();
}

pub fn bench_operator_paths(c: &mut Criterion) {
    // Same 20k facts and 10k output rows in all six controls. Binding the
    // entity selects Probe; binding a value with a constant attribute selects
    // ProbeValue (AVET/pending probes). A variable inner attribute selects Hash,
    // then an expression keeps :op/right. The hash control deliberately pays
    // for that broader scan. Pseudo metadata belongs to the right-hand source.
    for (mode, committed) in [("pending", false), ("committed", true)] {
        for (path, query) in [
            (
                "entity",
                "(query [:find ?e ?v ?tx :any-valid-time :where [?e :op/left ?v] [?e :op/right ?v] [?e :db/tx-count ?tx]])",
            ),
            (
                "value",
                "(query [:find ?e ?v ?tx :any-valid-time :where [?e :op/left ?v] [?f :op/right ?v] [?f :db/tx-count ?tx]])",
            ),
            (
                "hash",
                "(query [:find ?e ?v ?tx :any-valid-time :where [?e :op/left ?v] [?f ?a ?v] [?f :db/tx-count ?tx] [(= ?a :op/right)]])",
            ),
        ] {
            case(
                c,
                "query/operator_paths",
                &format!("{path}_{mode}"),
                query,
                || {
                    Fixture::new(committed, |db| {
                        insert(db, N, |i| {
                            format!("[:e{i} :op/left {i}] [:e{i} :op/right {i}]")
                        })
                    })
                },
                |db| {
                    // Cursor's existing Debug view includes Stream operator
                    // counters. Check before pulling: no answer values are
                    // formatted, and no debug output is logged. A renamed
                    // operator intentionally requires updating this check.
                    let cursor = db.query(query).unwrap();
                    let counters = format!("{cursor:?}");
                    let expected = if path == "hash" {
                        "HashJoin"
                    } else {
                        "IndexNestedLoopJoin"
                    };
                    assert!(counters.contains(expected), "expected join operator");
                    if path != "hash" {
                        assert!(
                            !counters.contains("HashJoin"),
                            "probe control has no hash join"
                        );
                    }
                    drop(cursor);
                    let mut seen = vec![false; N];
                    let mut cursor = db.query(query).unwrap();
                    while let Some(batch) = cursor.next_batch(1000).unwrap() {
                        for row in batch.rows() {
                            let [Value::Ref(entity), Value::Integer(v), Value::Integer(tx)] =
                                row.as_slice()
                            else {
                                unreachable!("expected entity, value and source transaction");
                            };
                            let i = usize::try_from(*v).unwrap();
                            assert!(i < N && !seen[i], "one row per input value");
                            assert!(
                                *entity
                                    == Uuid::new_v5(
                                        &Uuid::NAMESPACE_OID,
                                        format!(":e{i}").as_bytes()
                                    ),
                                "correct entity"
                            );
                            assert_eq!(
                                *tx,
                                i64::try_from(i / 500 + 1).unwrap(),
                                "right source transaction"
                            );
                            seen[i] = true;
                        }
                    }
                    assert!(seen.into_iter().all(|v| v), "every value joined");
                },
            );
        }
    }

    // One query contains both natural body strategies: an entity-correlated
    // `not` probe and a closed, hashed `not-join`. No forced-form timing is
    // claimed. The union case has nested expressions using an outer variable.
    for (name, query, expected) in [
        (
            "anti_committed",
            "(query [:find ?v :where [?e :op/left ?v] (not [?e :op/blocked true]) (not-join [?v] [?x :op/excluded ?v])])",
            N / 2,
        ),
        (
            "union_committed",
            "(query [:find ?v :where [?e :op/left ?v] (or (and [?e :op/right ?w] [(< ?v 5000)] [(= ?v ?w)]) (and [?e :op/right ?w] [(>= ?v 5000)] [(= ?v ?w)]))])",
            N,
        ),
    ] {
        case(
            c,
            "query/operator_paths",
            name,
            query,
            || {
                Fixture::new(true, |db| {
                    insert(db, N, |i| {
                        let extra = match i % 4 {
                            0 => format!("[:e{i} :op/blocked true]"),
                            1 => format!("[:excluded{i} :op/excluded {i}]"),
                            _ => String::new(),
                        };
                        format!("[:e{i} :op/left {i}] [:e{i} :op/right {i}] {extra}")
                    })
                })
            },
            |db| {
                assert_eq!(drain(db, query), expected);
            },
        );
    }

    // 100 partitions of 100 rows, plus an incompatible second window. This
    // exercises full window buffering/sorting, rather than the existing
    // singleton-partition proof. Validate both ranks before measuring.
    let query = "(query [:find ?p (row-number :over (:partition-by ?p :order-by ?v)) (row-number :over (:partition-by ?v :order-by ?p)) :where [?e :op/part ?p] [?e :op/value ?v]])";
    case(
        c,
        "query/operator_paths",
        "windows_committed",
        query,
        || {
            Fixture::new(true, |db| {
                insert(db, N, |i| {
                    format!("[:e{i} :op/part {}] [:e{i} :op/value {i}]", i / 100)
                })
            })
        },
        |db| {
            let mut cursor = db.query(query).unwrap();
            let mut ranks = vec![0usize; 100];
            let mut count = 0;
            while let Some(batch) = cursor.next_batch(1000).unwrap() {
                for row in batch.rows() {
                    let [
                        Value::Integer(p),
                        Value::Integer(rank),
                        Value::Integer(other),
                    ] = row.as_slice()
                    else {
                        unreachable!("expected integer window results");
                    };
                    assert!((1..=100).contains(rank));
                    assert_eq!(*other, 1);
                    ranks[usize::try_from(*p).unwrap()] += usize::try_from(*rank).unwrap();
                    count += 1;
                }
            }
            assert_eq!(count, N);
            assert!(ranks.into_iter().all(|sum| sum == 5050));
        },
    );

    for irregular in [false, true] {
        let query = "(query [:find ?v :as-of \"2020-01-01T00:00:05Z\" :where [?e :op/value ?v]])";
        case(
            c,
            "time_travel/operator_paths",
            if irregular {
                "timestamp_irregular"
            } else {
                "timestamp_monotone"
            },
            query,
            || timestamp_fixture(irregular),
            |db| {
                let mut cursor = db.query(query).unwrap();
                let mut seen = vec![false; N];
                while let Some(batch) = cursor.next_batch(1000).unwrap() {
                    for row in batch.rows() {
                        let [Value::Integer(v)] = row.as_slice() else {
                            unreachable!("expected integer");
                        };
                        let i = usize::try_from(*v).unwrap();
                        assert!(i < N && !seen[i]);
                        seen[i] = true;
                    }
                }
                for (i, found) in seen.into_iter().enumerate() {
                    let time_index = if irregular { i ^ 1 } else { i };
                    assert_eq!(found, time_index <= 5000, "timestamp cutoff membership");
                }
            },
        );
    }

    // Small committed ancestor chain amid 10k unrelated facts. Magic sets and
    // semi-naive evaluation run each iteration; rule registration is setup.
    let query = "(query [:find ?a :where (op-ancestor :node10 ?a)])";
    case(
        c,
        "recursion/operator_paths",
        "ancestors_committed",
        query,
        || {
            Fixture::new(true, |db| {
                insert(db, N, |i| format!("[:filler{i} :op/value {i}]"));
                insert(db, 10, |i| format!("[:node{} :op/up :node{i}]", i + 1));
                db.execute("(rule [(op-ancestor ?x ?a) [?x :op/up ?a]])")
                    .unwrap();
                db.execute("(rule [(op-ancestor ?x ?a) [?x :op/up ?p] (op-ancestor ?p ?a)])")
                    .unwrap();
            })
        },
        |db| {
            assert_eq!(drain(db, query), 10);
        },
    );
}

fn timestamp_fixture(irregular: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("timestamps.graph");
    let mut writer = LogWriter::create(&path, OpenOptions::new()).unwrap();
    // A transaction per record; adjacent timestamps swap in the irregular
    // control, so the cutoff includes tx 5002 but excludes tx 5001. It cannot
    // be represented by a transaction-count prefix. Valid time is independent.
    for i in 0..N {
        let time_index = if irregular { i ^ 1 } else { i };
        writer
            .append(&FactRecord {
                entity: Uuid::from_u128(u128::try_from(i + 1).unwrap()),
                attribute: ":op/value".to_string(),
                value: Value::Integer(i64::try_from(i).unwrap()),
                tx_count: u64::try_from(i + 1).unwrap(),
                tx_id: 1_577_836_800_000 + u64::try_from(time_index).unwrap(),
                valid_from: 0,
                valid_to: i64::MAX,
                asserted: true,
            })
            .unwrap();
    }
    writer.finish().unwrap();
    let db = OpenOptions::new()
        .read_only(true)
        .path(path)
        .open()
        .unwrap();
    Fixture {
        db,
        _dir: Some(dir),
    }
}
