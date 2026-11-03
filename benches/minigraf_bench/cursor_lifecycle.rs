//! G09 / #776 P08: incremental delivery and snapshot lifetime costs.
//! Selected by the existing nightly query job's `^query/` filter.

use criterion::{BatchSize, BenchmarkId, Criterion};
use minigraf::{Cursor, Minigraf, OpenOptions, QueryResult, Value};
use std::hint::black_box;
use std::time::Duration;

const QUERY: &str = "(query [:find ?v :where [?e :val ?v]])";
const BATCH_ROWS: usize = 256;

// Field order keeps the database alive until after any cursor in a surrounding
// (cursor, fixture) tuple, and the directory alive until after the database.
struct FileFixture {
    db: Minigraf,
    _dir: tempfile::TempDir,
}

fn fixture(n: usize) -> FileFixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.graph");
    super::helpers::populate_file(n, path.to_str().unwrap());
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(&path)
        .open()
        .unwrap();
    FileFixture { db, _dir: dir }
}

// An old reader pins the original committed generation. Two writes/checkpoints
// replace index pages and exercise quarantine across more than one generation.
// A fresh file per iteration bounds history and prevents accumulation in DICT.
fn held_snapshot(n: usize) -> (Cursor, FileFixture) {
    let file = fixture(n);
    let cursor = file.db.query(QUERY).unwrap();
    file.db
        .execute("(transact [[:after-open-1 :val -1]])")
        .unwrap();
    file.db.checkpoint().unwrap();
    file.db
        .execute("(transact [[:after-open-2 :val -2]])")
        .unwrap();
    file.db.checkpoint().unwrap();
    (cursor, file)
}

// Check the entire answer outside timing: every integer appears exactly once,
// and callers never receive more than the requested batch size.
fn validate_rows<'a>(rows: impl IntoIterator<Item = &'a Vec<Value>>, seen: &mut [bool]) {
    for row in rows {
        assert_eq!(row.len(), 1);
        let Value::Integer(value) = row[0] else {
            panic!("expected an integer value");
        };
        let index = usize::try_from(value).expect("snapshot excludes later negative values");
        assert!(index < seen.len(), "value outside the fixture");
        assert!(!seen[index], "duplicate value");
        seen[index] = true;
    }
}

fn validate_cursor(cursor: &mut Cursor, n: usize) {
    let mut seen = vec![false; n];
    while let Some(batch) = cursor.next_batch(BATCH_ROWS).unwrap() {
        assert!(!batch.is_empty());
        assert!(batch.len() <= BATCH_ROWS);
        validate_rows(batch.rows(), &mut seen);
    }
    assert!(
        seen.into_iter().all(|present| present),
        "missing fixture rows"
    );
}

fn drain(cursor: &mut Cursor) -> usize {
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(black_box(BATCH_ROWS)).unwrap() {
        rows += batch.len();
        black_box(&batch);
    }
    black_box(rows)
}

pub fn bench_cursor_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/cursor_lifecycle");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    // Fixture construction is lazy: --list and unrelated filters do no file IO.
    for operation in ["open", "first_batch", "drain", "iterator"] {
        let scales: &[(&str, usize)] = if operation == "drain" {
            &[("10k", 10_000), ("100k", 100_000)]
        } else {
            &[("10k", 10_000)]
        };
        for &(label, n) in scales {
            group.bench_function(BenchmarkId::new(operation, label), |b| {
                let file = fixture(n);
                validate_cursor(&mut file.db.query(QUERY).unwrap(), n);
                // Check the first pull and iterator adapter outside timing too.
                let mut first = file.db.query(QUERY).unwrap();
                assert_eq!(
                    first.next_batch(BATCH_ROWS).unwrap().unwrap().len(),
                    BATCH_ROWS
                );
                first.close();
                let mut seen = vec![false; n];
                for row in file.db.query(QUERY).unwrap() {
                    validate_rows([&row.unwrap()], &mut seen);
                }
                assert!(seen.into_iter().all(|present| present));

                match operation {
                    "open" => b.iter_batched(
                        || (),
                        |()| black_box(file.db.query(black_box(QUERY)).unwrap()),
                        BatchSize::PerIteration,
                    ),
                    "first_batch" => b.iter_batched_ref(
                        || file.db.query(QUERY).unwrap(),
                        |cursor| black_box(cursor.next_batch(black_box(BATCH_ROWS)).unwrap()),
                        BatchSize::PerIteration,
                    ),
                    "drain" => b.iter_batched_ref(
                        || file.db.query(QUERY).unwrap(),
                        drain,
                        BatchSize::PerIteration,
                    ),
                    "iterator" => b.iter_batched_ref(
                        || file.db.query(QUERY).unwrap(),
                        |cursor| {
                            let mut rows = 0;
                            for row in cursor {
                                black_box(row.unwrap());
                                rows += 1;
                            }
                            black_box(rows)
                        },
                        BatchSize::PerIteration,
                    ),
                    _ => unreachable!(),
                }
                // Cursor teardown is outside timing.
            });
        }
    }

    group.bench_function("held_checkpoint_drain/10k", |b| {
        let (mut old, file) = held_snapshot(10_000);
        validate_cursor(&mut old, 10_000);
        // The two checkpointed rows are visible to a new snapshot, not the old.
        assert_eq!(drain(&mut file.db.query(QUERY).unwrap()), 10_002);
        drop((old, file));
        b.iter_batched_ref(
            || held_snapshot(10_000),
            |(cursor, _file)| drain(cursor),
            BatchSize::PerIteration,
        );
    });

    group.bench_function("held_checkpoint_close/10k", |b| {
        let (mut old, file) = held_snapshot(10_000);
        validate_cursor(&mut old, 10_000);
        old.close();
        assert_eq!(drain(&mut file.db.query(QUERY).unwrap()), 10_002);
        drop(file);
        b.iter_batched_ref(
            || {
                let (mut cursor, file) = held_snapshot(10_000);
                assert_eq!(
                    cursor.next_batch(BATCH_ROWS).unwrap().unwrap().len(),
                    BATCH_ROWS
                );
                (Some(cursor), file)
            },
            // This is the last pin on the old generation: close includes
            // pruning its quarantined pages (Pins::release), with the database
            // and file teardown still outside timing.
            |(cursor, _file)| black_box(cursor.take().unwrap()).close(),
            BatchSize::PerIteration,
        );
    });

    group.bench_function("staged_read/10k_plus_200", |b| {
        let file = fixture(10_000);
        let statement = |start, end| {
            let facts: String = (start..end).map(|i| format!("[:e{i} :val {i}]")).collect();
            format!("(transact [{facts}])")
        };
        // 10k checkpointed, 100 WAL-committed/pending, 100 uncommitted/staged.
        // Stay below the chunk boundary: large transaction reads belong to P09.
        file.db.execute(&statement(10_000, 10_100)).unwrap();
        validate_cursor(&mut file.db.query(QUERY).unwrap(), 10_100);
        let mut tx = file.db.begin_write().unwrap();
        tx.execute(&statement(10_100, 10_200)).unwrap();
        let QueryResult::QueryResults { results, .. } = tx.execute(QUERY).unwrap() else {
            panic!("expected staged query results");
        };
        let mut seen = vec![false; 10_200];
        validate_rows(&results, &mut seen);
        assert!(seen.into_iter().all(|present| present));
        drop(results);
        // Reuse the same transaction: measured reads never grow its staged data.
        // Result destruction and transaction rollback are outside timing.
        b.iter_batched(
            || (),
            |()| black_box(tx.execute(black_box(QUERY)).unwrap()),
            BatchSize::PerIteration,
        );
        drop(tx);
        validate_cursor(&mut file.db.query(QUERY).unwrap(), 10_100);
    });
    group.finish();
}
