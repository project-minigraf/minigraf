//! #776 P10: historical log reads and offline import, without Datalog costs.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use minigraf::{
    FactFilter, FactLog, FactOrder, FactRecord, LogWriter, Minigraf, OpenOptions, Value, ValueSet,
};
use std::hint::black_box;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;

const BATCH: usize = 1_024;
const PENDING: usize = 100;

// Four versions per triple: two asserted windows, a retraction, then a new
// assertion. Fixed UUIDs/timestamps, 100 records per transaction, tx holes,
// and inline/long values exercise record preservation rather than live rows.
fn records(n: usize) -> Vec<FactRecord> {
    let entities = n / 4;
    (0..n)
        .map(|i| {
            let entity = i % entities;
            let phase = i / entities;
            let tx_count = u64::try_from(i / 100 + 1).unwrap() * 3;
            let value = match entity % 4 {
                0 => Value::Integer(i64::try_from(entity).unwrap()),
                1 => Value::String(format!("{entity:08}{}", "x".repeat(96))),
                2 => Value::Set(ValueSet::new([Value::Integer(1), Value::Integer(2)]).unwrap()),
                _ => Value::Set(ValueSet::new((0..32).map(Value::Integer)).unwrap()),
            };
            FactRecord {
                entity: Uuid::from_u128(u128::try_from(entity).unwrap() + 1),
                attribute: if entity.is_multiple_of(2) {
                    ":history/a"
                } else {
                    ":history/b"
                }
                .into(),
                value,
                tx_count,
                tx_id: 1_600_000_000_000 + tx_count * 1_000,
                valid_from: i64::try_from(phase).unwrap() * 10_000,
                valid_to: i64::try_from(phase + 1).unwrap() * 10_000,
                asserted: phase != 2,
            }
        })
        .collect()
}

fn append(writer: &mut LogWriter, records: &[FactRecord]) {
    for record in records {
        writer.append(black_box(record)).unwrap();
    }
}

fn write(path: &Path, records: &[FactRecord], last_tx: u64) {
    let mut writer = LogWriter::create(path, OpenOptions::new()).unwrap();
    append(&mut writer, records);
    writer.advance_tx_count(last_tx).unwrap();
    assert_eq!(writer.tx_count(), last_tx);
    writer.finish().unwrap();
}

fn read(mut log: FactLog) -> Vec<FactRecord> {
    let mut records = Vec::new();
    while let Some(batch) = log.next_batch(BATCH).unwrap() {
        assert!(batch.len() <= BATCH);
        records.extend(batch);
    }
    records
}

fn canonical(records: &mut [FactRecord]) {
    records.sort_by_key(|r| (r.tx_count, r.entity));
}

// Full field equality is intentional: counts alone miss lost retractions,
// altered windows/timestamps, or missing long/set values after an import.
fn verify(db: &Minigraf, expected: &[FactRecord], last_tx: u64) {
    assert_eq!(db.current_tx_count(), last_tx);
    let mut actual = read(db.fact_log(&FactFilter::new()).unwrap());
    assert_eq!(actual.len(), expected.len());
    assert!(actual.windows(2).all(|w| w[0].tx_count <= w[1].tx_count));
    canonical(&mut actual);
    let mut expected = expected.to_vec();
    canonical(&mut expected);
    assert!(
        actual == expected,
        "import must preserve every record field"
    );
}

fn drain(mut log: FactLog) -> usize {
    let mut count = 0;
    while let Some(batch) = log.next_batch(BATCH).unwrap() {
        count += batch.len();
        black_box(batch);
    }
    count
}

struct Mixed {
    db: Minigraf,
    // Drop the database before deleting its files (including on Windows).
    _dir: TempDir,
}

fn mixed(records: &[FactRecord], last_tx: u64) -> Mixed {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.graph");
    write(&path, records, last_tx);
    let db = Minigraf::open_with_options(
        &path,
        OpenOptions::new().wal_checkpoint_threshold(usize::MAX),
    )
    .unwrap();
    verify(&db, records, last_tx);
    let mut command =
        String::from("(transact {:valid-from \"2020-01-01\" :valid-to \"2021-01-01\"} [");
    for i in 0..PENDING {
        command.push_str(&format!(
            "[#uuid \"{}\" :pending/value {i}]",
            Uuid::from_u128(u128::try_from(i).unwrap() + 1)
        ));
    }
    command.push_str("])");
    db.execute(&command).unwrap();
    assert_eq!(db.current_tx_count(), last_tx + 1);
    let pending = read(
        db.fact_log(&FactFilter::new().attributes([":pending/value"]))
            .unwrap(),
    );
    assert_eq!(pending.len(), PENDING);
    assert!(
        pending
            .iter()
            .all(|r| r.tx_count == last_tx + 1 && r.asserted)
    );
    Mixed { db, _dir: dir }
}

pub fn bench_fact_log(c: &mut Criterion) {
    let mut group = c.benchmark_group("fact_log");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (label, n) in [("10k", 10_000), ("100k", 100_000)] {
        // The scale counts historical records; each read also has 100 pending
        // records. All setup lives inside selected closures, even for --list.
        for order in [FactOrder::Storage, FactOrder::Tx] {
            let attribute_only = order == FactOrder::Storage && n == 10_000;
            let name = if attribute_only {
                "attribute_only_storage"
            } else if order == FactOrder::Storage {
                "storage_mixed"
            } else {
                "tx_mixed_default"
            };
            let expected = if attribute_only { n / 2 } else { n + PENDING };
            group.throughput(Throughput::Elements(u64::try_from(expected).unwrap()));
            let fixture = OnceLock::new();
            group.bench_with_input(BenchmarkId::new(name, label), &n, |b, &n| {
                // Criterion re-enters this callback for warmup and samples.
                // Build/validate once, and do no work for unselected IDs.
                let (fixture, filter) = fixture.get_or_init(|| {
                    let records = records(n);
                    let last_tx = records.last().unwrap().tx_count + 7;
                    let fixture = mixed(&records, last_tx);
                    let mut filter = FactFilter::new().order(order);
                    if attribute_only {
                        // No entities/prefix: this selects the AEVT path.
                        filter = filter.attributes([":history/a"]);
                    }
                    assert_eq!(drain(fixture.db.fact_log(&filter).unwrap()), expected);
                    (fixture, filter)
                });
                b.iter(|| black_box(drain(fixture.db.fact_log(black_box(filter)).unwrap())));
            });
        }
    }
    // One large window control and one selective case keep the shortlist at
    // ten IDs. 4096 requires multiple full key sweeps, unlike DEFAULT_WINDOW.
    for filtered in [false, true] {
        let name = if filtered {
            "filter_attribute_entity_tx"
        } else {
            "tx_mixed_window_4096"
        };
        group.throughput(Throughput::Elements(if filtered { 128 } else { 100_100 }));
        let fixture = OnceLock::new();
        group.bench_function(format!("{name}/100k"), |b| {
            let (fixture, filter) = fixture.get_or_init(|| {
                let records = records(100_000);
                let last_tx = records.last().unwrap().tx_count + 7;
                let fixture = mixed(&records, last_tx);
                let mut filter = FactFilter::new().window(4_096);
                let expected = if filtered {
                    // Positive prefix filter plus entity and tx bounds uses
                    // EAVT, unlike the attribute-only AEVT case above.
                    filter = filter
                        .attribute_prefix(":history/a")
                        .entities((1..=128).map(Uuid::from_u128))
                        .tx_range(3..=1_500);
                    records
                        .iter()
                        .filter(|r| {
                            r.entity.as_u128() <= 128
                                && r.attribute == ":history/a"
                                && r.tx_count <= 1_500
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let actual = read(fixture.db.fact_log(&filter).unwrap());
                assert!(actual.windows(2).all(|w| w[0].tx_count <= w[1].tx_count));
                if filtered {
                    assert_eq!(actual.len(), 128);
                    let mut actual = actual;
                    canonical(&mut actual);
                    let mut expected = expected;
                    canonical(&mut expected);
                    assert!(
                        actual == expected,
                        "filter keeps exactly the intended history"
                    );
                } else {
                    assert_eq!(actual.len(), 100_100);
                }
                (fixture, filter)
            });
            b.iter(|| black_box(drain(fixture.db.fact_log(black_box(filter)).unwrap())));
        });
    }
    group.finish();
}

pub fn bench_log_writer(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_writer");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (label, n) in [("10k", 10_000), ("100k", 100_000)] {
        for finish in [false, true] {
            let name = if finish { "finish" } else { "append" };
            group.throughput(Throughput::Elements(u64::try_from(n).unwrap()));
            let fixture = OnceLock::new();
            group.bench_with_input(BenchmarkId::new(name, label), &n, |b, &n| {
                let (records, last_tx) = fixture.get_or_init(|| {
                    let records = records(n);
                    let last_tx = records.last().unwrap().tx_count + 7;
                    // Verify a complete import once, outside timing. 100k
                    // crosses the writer's 65,536-record batch; 10k does not.
                    let check_dir = tempfile::tempdir().unwrap();
                    let check_path = check_dir.path().join("check.graph");
                    write(&check_path, &records, last_tx);
                    let check = Minigraf::open_with_options(
                        &check_path,
                        OpenOptions::new().read_only(true),
                    )
                    .unwrap();
                    verify(&check, &records, last_tx);
                    drop(check);
                    (records, last_tx)
                });
                b.iter_batched(
                    || {
                        let dir = tempfile::tempdir().unwrap();
                        let mut writer =
                            LogWriter::create(dir.path().join("out.graph"), OpenOptions::new())
                                .unwrap();
                        if finish {
                            append(&mut writer, records);
                            writer.advance_tx_count(*last_tx).unwrap();
                        }
                        (writer, dir)
                    },
                    |(mut writer, dir)| {
                        if finish {
                            writer.finish().unwrap();
                            // Directory/file cleanup happens after timing.
                            (None, dir)
                        } else {
                            append(&mut writer, records);
                            // Unfinished writer deletion also stays untimed.
                            (Some(writer), dir)
                        }
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}
