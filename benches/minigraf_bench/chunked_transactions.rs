//! #776 P09: default chunk-boundary writes, routed by nightly `^insert_file/`.
//! Fixtures, command construction, validation and database teardown are untimed.

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, Throughput};
use minigraf::{Minigraf, OpenOptions, QueryResult};
use std::fmt::Write;
use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

const CHUNK: usize = 65_536;
const SCALES: [usize; 2] = [CHUNK - 1, CHUNK + 1];
const BEFORE: &str = "(transact [[:chunk/control :chunk/marker 1]])";
const AFTER: &str = "(transact [[:chunk/control :chunk/marker 2]])";

// Field order keeps the directory alive until the handle has closed.
struct Fixture {
    db: Minigraf,
    _dir: tempfile::TempDir,
}

fn fixture(template: Option<&Path>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chunk.graph");
    if let Some(template) = template {
        std::fs::copy(template, &path).unwrap();
        std::fs::File::open(&path).unwrap().sync_all().unwrap();
    }
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(path.to_str().unwrap())
        .open()
        .unwrap();
    Fixture { db, _dir: dir }
}

fn statement(kind: &str, start: usize, end: usize) -> String {
    let mut command = format!("({kind} [");
    for i in start..end {
        write!(command, "[:chunk/e{i} :chunk/value {i}]").unwrap();
    }
    command.push_str("])");
    command
}

fn count(db: &Minigraf, query: &str) -> usize {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected query results");
    };
    results.len()
}

fn validate(db: &Minigraf, values: usize, tx_count: u64, large: bool, dir: &Path) {
    assert_eq!(db.current_tx_count(), tx_count);
    assert_eq!(
        count(db, "(query [:find ?e ?v :where [?e :chunk/value ?v]])"),
        values
    );
    assert_eq!(
        dir.join("chunk.graph.wal").exists(),
        !large,
        "small writes retain their WAL; large writes commit as checkpoints"
    );
}

pub fn bench_chunked_transactions(c: &mut Criterion) {
    assert_eq!(OpenOptions::default().transaction_chunk_facts, CHUNK);
    let mut group = c.benchmark_group("insert_file/chunked");
    // Flat sampling avoids Criterion's increasing iteration counts on these
    // expensive file writes. These are targets, not a measured runtime bound.
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    for n in SCALES {
        group.throughput(Throughput::Elements(u64::try_from(n).unwrap()));
        group.bench_with_input(BenchmarkId::new("execute", n), &n, |b, &n| {
            let command = statement("transact", 0, n);
            let check = fixture(None);
            check.db.execute(&command).unwrap();
            validate(&check.db, n, 1, n > CHUNK, check._dir.path());
            drop(check);

            b.iter_batched_ref(
                || fixture(None),
                |f| black_box(f.db.execute(black_box(&command)).unwrap()),
                BatchSize::PerIteration,
            );
        });

        group.bench_with_input(BenchmarkId::new("retract", n), &n, |b, &n| {
            let command = statement("retract", 0, n);
            // Build this committed source once per selected case; every
            // iteration copies and opens it outside the measured routine.
            let seed = fixture(None);
            seed.db.execute(&statement("transact", 0, n)).unwrap();
            seed.db.checkpoint().unwrap();
            let template = seed._dir.path().join("chunk.graph");
            drop(seed.db);
            let check = fixture(Some(&template));
            assert_eq!(
                count(&check.db, "(query [:find ?e :where [?e :chunk/value ?v]])"),
                n
            );
            check.db.execute(&command).unwrap();
            validate(&check.db, 0, 2, n > CHUNK, check._dir.path());
            // Retractions preserve the original assertions for time travel.
            assert_eq!(
                count(
                    &check.db,
                    "(query [:find ?e :as-of 1 :where [?e :chunk/value ?v]])"
                ),
                n
            );
            drop(check);

            b.iter_batched_ref(
                || fixture(Some(&template)),
                |f| black_box(f.db.execute(black_box(&command)).unwrap()),
                BatchSize::PerIteration,
            );
        });

        group.bench_with_input(BenchmarkId::new("write_tx", n), &n, |b, &n| {
            // Both statements fit a chunk individually. Only their combined
            // staged count crosses the boundary. The second also replaces
            // e0's window from the first (last statement wins).
            let first = statement("transact", 0, n / 2);
            let mut second = statement("transact", n / 2, n - 1);
            second.truncate(second.len() - 2);
            second.push_str("[:chunk/e0 :chunk/value 0 {:valid-from \"2000-01-01T00:00:00Z\"}]])");
            let run = |db: &Minigraf| {
                let mut tx = db.begin_write().unwrap();
                black_box(tx.execute(black_box(&first)).unwrap());
                black_box(tx.execute(black_box(&second)).unwrap());
                black_box(tx.commit().unwrap());
            };
            let check = fixture(None);
            run(&check.db);
            validate(&check.db, n - 1, 1, n > CHUNK, check._dir.path());
            assert_eq!(count(&check.db, "(query [:find ?v :valid-at \"2001-01-01T00:00:00Z\" :where [:chunk/e0 :chunk/value ?v]])"), 1);
            drop(check);

            b.iter_batched_ref(|| fixture(None), |f| run(&f.db), BatchSize::PerIteration);
        });
    }

    // A late, cross-chunk API-011 after the first chunk reached working
    // trees; this measures statement-base rollback, not a parser rejection.
    group.throughput(Throughput::Elements(u64::try_from(CHUNK + 1).unwrap()));
    group.bench_function(BenchmarkId::new("statement_rollback", CHUNK + 1), |b| {
        let mut rejected = String::from(
            "(transact [[:chunk/bad :chunk/value -1 {:valid-from \"2000-01-01T00:00:00Z\"}]",
        );
        for i in 0..CHUNK - 1 {
            write!(rejected, "[:chunk/e{i} :chunk/value {i}]").unwrap();
        }
        rejected.push_str("[:chunk/bad :chunk/value -1 {:valid-from \"2020-01-01T00:00:00Z\"}]])");
        let run = |db: &Minigraf| {
            let mut tx = db.begin_write().unwrap();
            black_box(tx.execute(black_box(BEFORE)).unwrap());
            let error = tx.execute(black_box(&rejected)).unwrap_err();
            assert_eq!(error.code(), "API-011");
            black_box(error);
            black_box(tx.execute(black_box(AFTER)).unwrap());
            black_box(tx.commit().unwrap());
        };
        let check = fixture(None);
        run(&check.db);
        validate(&check.db, 0, 1, true, check._dir.path());
        assert_eq!(
            count(
                &check.db,
                "(query [:find ?e ?v :any-valid-time :where [?e :chunk/value ?v]])"
            ),
            0
        );
        assert_eq!(
            count(
                &check.db,
                "(query [:find ?v :where [:chunk/control :chunk/marker ?v]])"
            ),
            2
        );
        drop(check);

        b.iter_batched_ref(|| fixture(None), |f| run(&f.db), BatchSize::PerIteration);
    });
    group.finish();
}
