//! #776 P15 / G07: finite memory-limit enforcement versus unlimited queries.
//!
//! Accounting is always enabled in the engine: `usize::MAX` disables the
//! limit, not charges. These pairs measure enforcement on/off, not the cost
//! of accounting versus an unmetered engine. The separate smoke acceptance
//! jobs enforce state/RSS bounds; Criterion reports latency only.

use criterion::{BenchmarkId, Criterion};
use minigraf::{Minigraf, OpenOptions, Value};
use std::hint::black_box;
use std::time::Duration;

fn drain(db: &Minigraf, query: &str) -> (usize, i64) {
    let mut cursor = db.query(query).unwrap();
    let mut rows = 0;
    let mut total = 0;
    while let Some(batch) = cursor.next_batch(256).unwrap() {
        rows += batch.len();
        for row in batch.rows() {
            let Some(Value::Integer(n)) = row.last() else {
                panic!("expected integer count");
            };
            total += n;
        }
        black_box(batch);
    }
    (rows, total)
}

pub fn bench_memory_meter(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_meter");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    for (shape, query, expected_rows) in [
        ("count", "(query [:find (count ?v) :where [?e :val ?v]])", 1),
        (
            "hash_groups",
            // An expression-bound key cannot use the committed input-order
            // proof. All 10k groups stay in Aggregate until input is drained.
            "(query [:find ?g (count ?e) :where [?e :val ?v] [(+ ?v 0) ?g]])",
            10_000,
        ),
    ] {
        for (mode, limit) in [("unlimited", usize::MAX), ("limited", 8 << 20)] {
            group.bench_function(BenchmarkId::new(shape, format!("{mode}/10k")), |b| {
                // Setup is lazy and untimed, including file copy, open, cache
                // warm-up, validation and a low-limit rejection control.
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("meter.graph");
                super::helpers::populate_file(10_000, path.to_str().unwrap());
                let db = OpenOptions::new()
                    .query_memory_limit(limit)
                    .read_only(true)
                    .path(&path)
                    .open()
                    .unwrap();
                assert_eq!(drain(&db, query), (expected_rows, 10_000));
                if shape == "hash_groups" {
                    let constrained = OpenOptions::new()
                        .query_memory_limit(4 << 10)
                        .read_only(true)
                        .path(&path)
                        .open()
                        .unwrap();
                    let err = constrained.execute(query).expect_err("limit enforced");
                    assert_eq!(err.code(), "QRY-012");
                    assert!(
                        err.to_string().contains("Aggregate"),
                        "hash aggregate state"
                    );
                }
                b.iter(|| black_box(drain(&db, black_box(query))));
            });
        }
    }
    group.finish();
}
