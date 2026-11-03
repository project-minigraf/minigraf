//! P02 / G02 of #776: preparation, cursor and typed-slot costs.
//!
//! Eight new 10k cases complement the four existing prepared execute cases.
//! Each execution path has an unprepared control with identical output. Setup,
//! preparation (except prepare_value_lookup) and result checks stay untimed.

use criterion::Criterion;
use minigraf::{BindValue, Cursor, QueryResult, Value};
use std::hint::black_box;
use std::time::Duration;
use uuid::Uuid;

fn rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query rows"),
    }
}

fn drain(mut cursor: Cursor) -> usize {
    let mut count = 0;
    while let Some(batch) = cursor.next_batch(256).unwrap() {
        count += batch.len();
        black_box(batch);
    }
    count
}

pub fn bench_prepared_extras(c: &mut Criterion) {
    let mut group = c.benchmark_group("prepared");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    // Keep fixtures inside the selected closures: other nightly suites must
    // neither populate this DB nor execute the validation queries.
    group.bench_function("prepare_value_lookup/10k", |b| {
        let db = super::helpers::populate_in_memory(10_000);
        let template = "(query [:find ?e :where [?e :val $val]])";
        let prepared = db.prepare(template).unwrap();
        assert_eq!(
            rows(
                prepared
                    .execute(&[("val", BindValue::Val(Value::Integer(0)))])
                    .unwrap()
            )
            .len(),
            1
        );
        b.iter(|| black_box(db.prepare(black_box(template)).unwrap()));
    });

    group.bench_function("unprepared_value_lookup/10k", |b| {
        let db = super::helpers::populate_in_memory(10_000);
        let query = "(query [:find ?e :where [?e :val 0]])";
        assert_eq!(rows(db.execute(query).unwrap()).len(), 1);
        // Pairs with existing prepared/value_lookup/10k, including output
        // materialization. Query string construction is not timed.
        b.iter(|| black_box(db.execute(black_box(query)).unwrap()));
    });

    for prepared in [true, false] {
        let name = if prepared {
            "cursor_drain/10k"
        } else {
            "unprepared_cursor_drain/10k"
        };
        group.bench_function(name, |b| {
            let db = super::helpers::populate_in_memory(10_000);
            let template = "(query [:find ?e :where [?e :val ?v] [(< ?v $threshold)]])";
            let literal = "(query [:find ?e :where [?e :val ?v] [(< ?v 5000)]])";
            let pq = db.prepare(template).unwrap();
            let binds = [("threshold", BindValue::Val(Value::Integer(5_000)))];
            assert_eq!(rows(pq.execute(&binds).unwrap()).len(), 5_000);
            assert_eq!(drain(pq.query(&binds).unwrap()), 5_000);
            assert_eq!(drain(db.query(literal).unwrap()), 5_000);
            if prepared {
                b.iter(|| black_box(drain(pq.query(black_box(&binds)).unwrap())));
            } else {
                b.iter(|| black_box(drain(db.query(black_box(literal)).unwrap())));
            }
        });
    }

    for prepared in [true, false] {
        let name = if prepared {
            "count_slots/10k"
        } else {
            "unprepared_count_slots/10k"
        };
        group.bench_function(name, |b| {
            let db = super::helpers::populate_in_memory(10_000);
            let template =
                "(query [:find ?v :where [?e :val ?v] :order-by [?v] :limit $n :offset $o])";
            let literal =
                "(query [:find ?v :where [?e :val ?v] :order-by [?v] :limit 10 :offset 100])";
            let pq = db.prepare(template).unwrap();
            let binds = [
                ("n", BindValue::Val(Value::Integer(10))),
                ("o", BindValue::Val(Value::Integer(100))),
            ];
            let expected: Vec<_> = (100..110).map(|v| vec![Value::Integer(v)]).collect();
            assert!(
                rows(pq.execute(&binds).unwrap()) == expected,
                "prepared page"
            );
            assert!(
                rows(db.execute(literal).unwrap()) == expected,
                "unprepared page"
            );
            if prepared {
                b.iter(|| black_box(pq.execute(black_box(&binds)).unwrap()));
            } else {
                b.iter(|| black_box(db.execute(black_box(literal)).unwrap()));
            }
        });
    }

    for prepared in [true, false] {
        let name = if prepared {
            "time_entity_slots/10k"
        } else {
            "unprepared_time_entity_slots/10k"
        };
        group.bench_function(name, |b| {
            let db = super::helpers::populate_in_memory(10_000);
            // Fixed UUID and valid-time window. A later transaction makes
            // :as-of select history instead of using an all-visible bound.
            let entity = Uuid::from_u128(776);
            db.execute(&format!(
                "(transact {{:valid-from \"2001-09-09T01:46:40Z\" :valid-to \"2065-01-24T05:20:00Z\"}} [[#uuid \"{entity}\" :history 1]])"
            )).unwrap();
            let tx = db.current_tx_count();
            db.execute(&format!(
                "(transact {{:valid-from \"2001-09-09T01:46:40Z\" :valid-to \"2065-01-24T05:20:00Z\"}} [[#uuid \"{entity}\" :history 2]])"
            )).unwrap();
            let template = "(query [:find ?v :as-of $tx :valid-at $date :where [$entity :history ?v]])";
            let literal = format!(
                "(query [:find ?v :as-of {tx} :valid-at \"2033-05-18T03:33:20Z\" :where [#uuid \"{entity}\" :history ?v]])"
            );
            let pq = db.prepare(template).unwrap();
            let binds = [
                ("tx", BindValue::TxCount(tx)),
                ("date", BindValue::Timestamp(2_000_000_000_000)),
                ("entity", BindValue::Entity(entity)),
            ];
            let expected = vec![vec![Value::Integer(1)]];
            assert!(rows(pq.execute(&binds).unwrap()) == expected, "prepared history");
            assert!(rows(db.execute(&literal).unwrap()) == expected, "unprepared history");
            if prepared {
                b.iter(|| black_box(pq.execute(black_box(&binds)).unwrap()));
            } else {
                b.iter(|| black_box(db.execute(black_box(&literal)).unwrap()));
            }
        });
    }
    group.finish();
}
