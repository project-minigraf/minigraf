//! P05 / Refs #776: a ten-case set-storage shortlist, routed by the existing
//! insert_file, checkpoint and query jobs. Keep membership/collect-set in P06.
//!
//! Four 3-byte positive integer magnitudes encode to 20 element bytes
//! (inline); 32 encode to 160 bytes (long, above the 64-byte boundary). Unique
//! sets have disjoint elements; repeated long sets exercise value-page deduplication.
//! Fix 10k facts rather than multiplying large cardinalities by 100k rows.
//! Ingest measures the first write to a fresh database, including lazy WAL
//! creation, header/parent-directory syncs and the transaction append/sync.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use minigraf::{Minigraf, OpenOptions, QueryResult, Value, ValueSet};
use std::fmt::Write;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::TempDir;

const FACTS: usize = 10_000;
const SCAN: &str = "(query [:find ?v :where [?e :bench/set-storage ?v]])";

#[derive(Clone, Copy)]
enum Shape {
    Scalar,
    InlineUnique,
    LongRepeated,
    LongUnique,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::InlineUnique => "inline_unique",
            Self::LongRepeated => "long_repeated",
            Self::LongUnique => "long_unique",
        }
    }

    fn cardinality(self) -> usize {
        match self {
            Self::Scalar => 0,
            Self::InlineUnique => 4,
            Self::LongRepeated | Self::LongUnique => 32,
        }
    }

    fn literal(self, row: usize) -> String {
        // Every magnitude remains in 0x010000..=0xffffff: stable encoded
        // width, irrespective of row. Descending input also exercises the
        // canonical element ordering used by writes and exact-value probes.
        let row = if matches!(self, Self::LongRepeated) {
            0
        } else {
            row
        };
        let base = 1_000_000 + row * 32;
        if matches!(self, Self::Scalar) {
            return base.to_string();
        }
        let mut literal = String::from("#{");
        for element in (0..self.cardinality()).rev() {
            write!(literal, "{} ", base + element).unwrap();
        }
        literal.push('}');
        literal
    }

    fn command(self) -> String {
        let mut command = String::from("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [");
        for row in 0..FACTS {
            write!(
                command,
                "[:set-storage/e{row} :bench/set-storage {}]",
                self.literal(row)
            )
            .unwrap();
        }
        command.push_str("])");
        command
    }

    fn exact_query(self) -> String {
        format!(
            "(query [:find ?e :where [?e :bench/set-storage {}]])",
            self.literal(FACTS / 2)
        )
    }
}

// Declaration order matters: close the handle before removing its directory,
// including any WAL. iter_batched_ref keeps both drops outside measurement.
struct Fixture {
    db: Minigraf,
    dir: TempDir,
}

impl Fixture {
    fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = OpenOptions::new()
            .wal_checkpoint_threshold(usize::MAX)
            .path(dir.path().join("sets.graph"))
            .open()
            .unwrap();
        Self { db, dir }
    }

    fn pending(command: &str) -> Self {
        let fixture = Self::empty();
        fixture.db.execute(command).unwrap();
        fixture
    }

    // Run once per selected benchmark, outside Criterion's timed routine.
    // Check the pending results, persisted file, reopen and committed results.
    fn validated(shape: Shape, command: &str) -> Self {
        let fixture = Self::pending(command);
        validate_rows(&fixture.db, shape);
        fixture.db.checkpoint().unwrap();
        let report = fixture.db.verify().unwrap();
        assert!(
            report.is_ok(),
            "set fixture must pass integrity verification"
        );
        assert_eq!(report.facts, u64::try_from(FACTS).unwrap());
        let Self { db, dir } = fixture;
        drop(db);
        let path = dir.path().join("sets.graph");
        assert!(!dir.path().join("sets.graph.wal").exists());
        let db = OpenOptions::new()
            .read_only(true)
            .path(&path)
            .open()
            .unwrap();
        validate_rows(&db, shape);
        eprintln!(
            "set_storage fixture {}: {} facts, {} bytes, integrity OK, reopened read-only",
            shape.name(),
            report.facts,
            std::fs::metadata(path).unwrap().len()
        );
        Self { db, dir }
    }
}

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        unreachable!("expected query rows");
    };
    results
}

fn validate_rows(db: &Minigraf, shape: Shape) {
    let values = rows(db, SCAN);
    assert_eq!(values.len(), FACTS);
    assert!(values.iter().all(|row| match row.first() {
        Some(Value::Integer(_)) => matches!(shape, Shape::Scalar),
        Some(Value::Set(set)) => set.len() == shape.cardinality(),
        _ => false,
    }));
    let expected = if matches!(shape, Shape::LongRepeated) {
        FACTS
    } else {
        1
    };
    assert_eq!(rows(db, &shape.exact_query()).len(), expected);
    let probe = rows(
        db,
        &format!(
            "(query [:find ?v :where [:set-storage/e{} :bench/set-storage ?v]])",
            FACTS / 2
        ),
    );
    let base = if matches!(shape, Shape::LongRepeated) {
        1_000_000
    } else {
        1_000_000 + (FACTS / 2) * 32
    };
    let value = if matches!(shape, Shape::Scalar) {
        Value::Integer(i64::try_from(base).unwrap())
    } else {
        Value::Set(
            ValueSet::new(
                (0..shape.cardinality())
                    .map(|offset| Value::Integer(i64::try_from(base + offset).unwrap())),
            )
            .unwrap(),
        )
    };
    assert!(probe == vec![vec![value]], "fixture values must round trip");
}

pub fn bench_set_storage(c: &mut Criterion) {
    // Four first-write ingest cases include parsing, pending indexes and WAL
    // creation/header/parent-directory syncs plus the transaction append/sync;
    // three checkpoint cases isolate v8 inline/long writes and long dedup.
    // The scalar ingest control uses exactly the same fresh-file lifecycle.
    for (prefix, checkpoint, shapes) in [
        (
            "insert_file/set_storage",
            false,
            &[
                Shape::Scalar,
                Shape::InlineUnique,
                Shape::LongRepeated,
                Shape::LongUnique,
            ][..],
        ),
        (
            "checkpoint/set_storage",
            true,
            &[Shape::InlineUnique, Shape::LongRepeated, Shape::LongUnique][..],
        ),
    ] {
        let mut group = c.benchmark_group(prefix);
        group.sample_size(10);
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(3));
        group.throughput(Throughput::Elements(u64::try_from(FACTS).unwrap()));
        for &shape in shapes {
            let validated = OnceLock::new();
            group.bench_function(BenchmarkId::new(shape.name(), "10k"), |b| {
                // Everything expensive is lazy: unselected jobs create no files.
                // Criterion invokes this closure again for each sample.
                let (command, _validated_fixture) = validated.get_or_init(|| {
                    let command = shape.command();
                    let fixture = Fixture::validated(shape, &command);
                    (command, fixture)
                });
                b.iter_batched_ref(
                    || {
                        if checkpoint {
                            Fixture::pending(command)
                        } else {
                            Fixture::empty()
                        }
                    },
                    |fixture| {
                        if checkpoint {
                            black_box(fixture.db.checkpoint()).unwrap();
                        } else {
                            black_box(fixture.db.execute(black_box(command)).unwrap());
                        }
                    },
                    BatchSize::PerIteration,
                );
            });
        }
        group.finish();
    }

    let mut group = c.benchmark_group("query/set_storage");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (operation, shape, scan) in [
        ("exact_inline_unique", Shape::InlineUnique, false),
        ("exact_long_unique", Shape::LongUnique, false),
        ("scan_long_unique", Shape::LongUnique, true),
    ] {
        let validated = OnceLock::new();
        group.throughput(Throughput::Elements(if scan {
            u64::try_from(FACTS).unwrap()
        } else {
            1
        }));
        group.bench_function(BenchmarkId::new(operation, "10k"), |b| {
            let (fixture, query) = validated.get_or_init(|| {
                let fixture = Fixture::validated(shape, &shape.command());
                let query = if scan {
                    SCAN.to_string()
                } else {
                    shape.exact_query()
                };
                (fixture, query)
            });
            // Includes query parsing/planning, AVET exact-set lookup or AEVT
            // scan, and materialization of all results (including long sets).
            b.iter(|| black_box(fixture.db.execute(black_box(query)).unwrap()));
        });
    }
    group.finish();
}
