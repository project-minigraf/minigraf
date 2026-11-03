//! #776 P13 / G14: successful fallible callbacks, registry churn and re-entry.
//!
//! Nightly ^udf/ selects six cases. The matched 10k pairs differ only in
//! registration API; both APIs ultimately use the same fallible dispatch.
//! Registration is untimed except in registry_snapshot_churn. That case holds
//! an old cursor, registers one function into a fixed 32-UDF registry, then
//! opens a new cursor. Fresh inputs and all teardown stay outside measurement.

use criterion::{BatchSize, Criterion};
use minigraf::{Cursor, Minigraf, QueryResult, Value};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

const PREDICATE: &str = "(query [:find ?v :where [?e :val ?v] [(selected? ?v)]])";
const AGGREGATE: &str = "(query [:find (bench_sum ?v) :where [?e :val ?v]])";
const OLD_QUERY: &str = "(query [:find ?v :where [?e :val ?v] [(p0? ?v)]])";
const NEW_QUERY: &str = "(query [:find ?v :where [?e :val ?v] [(late? ?v)]])";

fn selected(value: &Value) -> bool {
    matches!(value, Value::Integer(i) if *i >= 500)
}

fn add(acc: &mut i64, value: &Value) {
    if let Value::Integer(i) = value {
        *acc += i;
    }
}

fn rows(result: QueryResult) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = result else {
        panic!("expected UDF query results");
    };
    results
}

fn validate_selected(result: QueryResult, n: i64) {
    let mut actual = rows(result);
    actual.sort();
    let expected: Vec<_> = (500..n).map(|i| vec![Value::Integer(i)]).collect();
    assert!(actual == expected, "unexpected UDF predicate rows");
}

fn drain(mut cursor: Cursor) -> usize {
    let mut count = 0;
    while let Some(batch) = cursor.next_batch(32).unwrap() {
        count += batch.len();
    }
    count
}

fn registry_fixture() -> (Arc<Minigraf>, Cursor) {
    let db = super::helpers::populate_in_memory(64);
    for i in 0..32 {
        db.try_register_predicate(&format!("p{i}?"), |_| Ok(true))
            .unwrap();
    }
    let held = db.query(OLD_QUERY).unwrap();
    (db, held)
}

pub fn bench_udf_v3(c: &mut Criterion) {
    let mut group = c.benchmark_group("udf/v3");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    for fallible in [false, true] {
        let mode = if fallible { "fallible" } else { "infallible" };
        group.bench_function(format!("predicate_{mode}/10k"), |b| {
            let db = super::helpers::populate_in_memory(10_000);
            if fallible {
                db.try_register_predicate("selected?", |v| Ok(selected(v)))
                    .unwrap();
            } else {
                db.register_predicate("selected?", selected).unwrap();
            }
            validate_selected(db.execute(PREDICATE).unwrap(), 10_000);
            b.iter(|| black_box(db.execute(black_box(PREDICATE)).unwrap()));
        });

        group.bench_function(format!("aggregate_{mode}/10k"), |b| {
            let db = super::helpers::populate_in_memory(10_000);
            if fallible {
                db.try_register_aggregate(
                    "bench_sum",
                    || Ok(0i64),
                    |acc, v| {
                        add(acc, v);
                        Ok(())
                    },
                    |acc, _| Ok(Value::Integer(*acc)),
                )
                .unwrap();
            } else {
                db.register_aggregate("bench_sum", || 0i64, add, |acc, _| Value::Integer(*acc))
                    .unwrap();
            }
            assert!(
                rows(db.execute(AGGREGATE).unwrap()) == vec![vec![Value::Integer(49_995_000)]],
                "unexpected UDF aggregate sum"
            );
            b.iter(|| black_box(db.execute(black_box(AGGREGATE)).unwrap()));
        });
    }

    group.bench_function("registry_snapshot_churn/32", |b| {
        let (db, held) = registry_fixture();
        db.try_register_predicate("late?", |_| Ok(true)).unwrap();
        assert_eq!(drain(held), 64, "old registry snapshot remains usable");
        assert_eq!(
            drain(db.query(NEW_QUERY).unwrap()),
            64,
            "new registry snapshot"
        );
        b.iter_batched_ref(
            registry_fixture,
            |(db, _held)| {
                // One insertion per fresh registry: no iteration-dependent
                // growth. _held keeps its old immutable registry alive.
                db.try_register_predicate(black_box("late?"), |_| Ok(true))
                    .unwrap();
                black_box(db.query(black_box(NEW_QUERY)).unwrap())
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("callback_reentry/1k", |b| {
        let db = super::helpers::populate_in_memory(1_000);
        // A weak capture prevents a registry -> callback -> database cycle.
        let inner = Arc::downgrade(&db);
        db.try_register_predicate("selected?", move |v| {
            let inner = inner.upgrade().expect("callback database is alive");
            // A real read on the same database for every callback, without
            // recursively invoking selected?. Query opening/draining/dropping
            // and the weak upgrade are all part of the re-entry cost.
            black_box(
                inner
                    .execute("(query [:find ?v :where [:e0 :val ?v]])")
                    .unwrap(),
            );
            Ok(selected(v))
        })
        .unwrap();
        assert!(
            rows(
                db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                    .unwrap()
            ) == vec![vec![Value::Integer(0)]],
            "unexpected re-entry point-read fixture"
        );
        validate_selected(db.execute(PREDICATE).unwrap(), 1_000);
        b.iter(|| black_box(db.execute(black_box(PREDICATE)).unwrap()));
    });
    group.finish();
}
