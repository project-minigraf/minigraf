//! P12 / Refs #776 / G12: ten native storage cases, routed by the existing
//! open (^open/), checkpoint (^checkpoint) and query (^query/) nightly jobs.
//! Reserve 5–15 incremental job-minutes, pending phase-B measurement.
//! All sources are warm OS-cache fixtures; copies, fsync, opening (except open
//! cases), validation, handle destruction and directory removal are untimed.

use criterion::{
    BatchSize, BenchmarkGroup, BenchmarkId, Criterion, SamplingMode, measurement::WallTime,
};
use minigraf::{Minigraf, OpenOptions, QueryResult, Value};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCAN: &str = "(query [:find ?e ?v :where [?e :storage/value ?v]])";

fn configure(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
}

fn open(path: &Path, read_only: bool) -> Minigraf {
    OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .read_only(read_only)
        .path(path)
        .open()
        .unwrap()
}

fn copy(source: &Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("copy.graph");
    std::fs::copy(source, &path).unwrap();
    std::fs::File::open(&path).unwrap().sync_all().unwrap();
    (dir, path)
}

fn copy_with_wal(source: &Path) -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = copy(source);
    let source_wal = source.with_extension("graph.wal");
    if source_wal.exists() {
        let wal = path.with_extension("graph.wal");
        std::fs::copy(source_wal, &wal).unwrap();
        std::fs::File::open(wal).unwrap().sync_all().unwrap();
    }
    (dir, path)
}

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected query rows");
    };
    results
}

fn verify(db: &Minigraf, facts: u64) {
    let report = db.verify().unwrap();
    assert!(report.is_ok(), "file must verify cleanly");
    assert_eq!(report.facts, facts);
    assert!(report.pages > 0);
}

fn check_golden(db: &Minigraf) {
    assert_eq!(db.current_tx_count(), 7);
    assert!(
        rows(db, "(query [:find ?a :where [:alice :person/age ?a]])")
            == vec![vec![Value::Integer(31)]],
        "current golden fact survives"
    );
    assert!(
        rows(
            db,
            "(query [:find ?a :as-of 1 :where [:alice :person/age ?a]])"
        ) == vec![vec![Value::Integer(30)]],
        "historical golden fact survives"
    );
    assert_eq!(
        rows(db, "(query [:find ?v :where [?e :filler/n ?v]])").len(),
        300
    );
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/golden/v7_multi_checkpoint.json")).unwrap();
    // Compare every value in the frozen manifest, including history, refs,
    // long strings and valid-time windows. No result order is promised.
    for query in manifest["queries"].as_array().unwrap() {
        let mut actual: Vec<Vec<String>> = rows(db, query["query"].as_str().unwrap())
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| match v {
                        Value::String(s) => serde_json::to_string(s).unwrap(),
                        Value::Integer(i) => i.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Boolean(b) => b.to_string(),
                        Value::Keyword(k) if k.starts_with(':') => k.clone(),
                        Value::Keyword(k) => format!(":{k}"),
                        Value::Null => "nil".to_string(),
                        _ => panic!("unexpected golden output type"),
                    })
                    .collect()
            })
            .collect();
        let mut expected: Vec<Vec<String>> = query["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect()
            })
            .collect();
        actual.sort();
        expected.sort();
        assert!(actual == expected, "golden query must match its manifest");
    }
}

/// Decode only the frozen v8 meta fields needed to select a damage target.
/// The library's storage codecs are private. Offsets follow spec §4.1, like
/// the golden/integrity tests; no checksum is recomputed on the damaged page.
fn newest_meta(bytes: &[u8]) -> &[u8] {
    bytes
        .chunks_exact(4096)
        .take(2)
        .filter(|page| &page[..4] == b"MGRF" && &page[8..12] == b"META")
        .max_by_key(|page| u64::from_le_bytes(page[16..24].try_into().unwrap()))
        .unwrap()
}

fn scalar_file(n: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.graph");
    super::helpers::populate_file(n, path.to_str().unwrap());
    (dir, path)
}

fn history_file() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = scalar_file(100_000);
    let db = open(&path, false);
    let mut updates = String::from("(transact [");
    for i in 0..100 {
        updates.push_str(&format!("[:e{i} :val {i}]"));
    }
    updates.push_str("])");
    db.execute(&updates).unwrap();
    let mut retracts = String::from("(retract [");
    for i in 100..150 {
        retracts.push_str(&format!("[:e{i} :val {i}]"));
    }
    retracts.push_str("])");
    db.execute(&retracts).unwrap();
    db.checkpoint().unwrap();
    verify(&db, 100_150);
    assert_eq!(
        rows(&db, "(query [:find ?e ?v :where [?e :val ?v]])").len(),
        99_950
    );
    drop(db);
    (dir, path)
}

/// 10k facts, four distinct values of the same E/A asserted in each statement.
/// Inline strings are 32 bytes; long strings are 256 bytes. The repeated case
/// shares four long payloads across all 2,500 entities; unique has 10k payloads.
fn multivalue_file(mode: &str) -> (tempfile::TempDir, PathBuf, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("values.graph");
    let db = open(&path, false);
    let len = if mode == "inline_unique" { 32 } else { 256 };
    for start in (0..2_500).step_by(100) {
        let mut command = String::from("(transact [");
        for e in start..start + 100 {
            for v in 0..4 {
                let id = if mode == "long_repeated" {
                    v
                } else {
                    e * 4 + v
                };
                let payload = format!("{id:08}{}", "x".repeat(len - 8));
                command.push_str(&format!("[:mv{e} :storage/value \"{payload}\"]"));
            }
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    assert_eq!(rows(&db, SCAN).len(), 10_000);
    assert_eq!(
        rows(&db, "(query [:find ?v :where [:mv0 :storage/value ?v]])").len(),
        4
    );
    (dir, path, db)
}

pub fn bench_storage_v3(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("open/storage_v3");
        configure(&mut group);
        for read_only in [false, true] {
            let case = if read_only {
                "read_only_v7"
            } else {
                "migrate_v7"
            };
            group.bench_function(BenchmarkId::new(case, "multi_checkpoint"), |b| {
                // Copy the frozen file; never migrate the corpus in place.
                let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/golden/v7_multi_checkpoint.graph");
                let (_check_dir, check_path) = copy(&source);
                let db = open(&check_path, read_only);
                check_golden(&db);
                if !read_only {
                    let bytes = std::fs::read(&check_path).unwrap();
                    assert_eq!(&newest_meta(&bytes)[4..8], &8u32.to_le_bytes());
                    let report = db.verify().unwrap();
                    assert!(report.is_ok());
                    assert!(report.facts >= 300);
                }
                drop(db);
                b.iter_batched_ref(
                    || copy(&source),
                    |(_, path)| black_box(open(black_box(path), read_only)),
                    BatchSize::PerIteration,
                );
                if read_only {
                    assert!(std::fs::read(&check_path).unwrap() == std::fs::read(&source).unwrap());
                }
            });
        }

        for with_wal in [false, true] {
            let case = if with_wal {
                "read_only_wal"
            } else {
                "read_only_v8"
            };
            let label = if with_wal { "10k_plus_1k" } else { "10k" };
            group.bench_function(BenchmarkId::new(case, label), |b| {
                let (_source_dir, source) = scalar_file(10_000);
                let writer = open(&source, false);
                if with_wal {
                    let mut command = String::from("(transact [");
                    for i in 10_000..11_000 {
                        command.push_str(&format!("[:e{i} :val {i}]"));
                    }
                    command.push_str("])");
                    writer.execute(&command).unwrap();
                }
                let tx_count = writer.current_tx_count();
                // Capture a quiescent graph/WAL pair and close the writer.
                // Read-only opens use their own locked copy.
                let (_dir, path) = copy_with_wal(&source);
                drop(writer);
                let db = open(&path, true);
                assert_eq!(
                    rows(&db, "(query [:find ?e :where [?e :val ?v]])").len(),
                    if with_wal { 11_000 } else { 10_000 }
                );
                assert_eq!(db.current_tx_count(), tx_count);
                drop(db);
                let before = std::fs::read(&path).unwrap();
                let wal_path = path.with_extension("graph.wal");
                let wal_before = std::fs::read(&wal_path).ok();
                b.iter_batched(
                    || (),
                    |()| black_box(open(black_box(&path), true)),
                    BatchSize::PerIteration,
                );
                assert!(std::fs::read(&path).unwrap() == before);
                assert!(std::fs::read(&wal_path).ok() == wal_before);
            });
        }
        group.finish();
    }

    {
        let mut group = c.benchmark_group("checkpoint/storage_v3");
        configure(&mut group);
        group.bench_function(BenchmarkId::new("verify", "100k_history"), |b| {
            let (_dir, path) = history_file();
            let db = open(&path, true);
            verify(&db, 100_150);
            b.iter(|| black_box(db.verify().unwrap()));
        });
        group.bench_function(
            BenchmarkId::new("rebuild_damaged_avet", "100k_history"),
            |b| {
                let (_dir, path) = history_file();
                let mut bytes = std::fs::read(&path).unwrap();
                let root = u64::from_le_bytes(newest_meta(&bytes)[64..72].try_into().unwrap());
                let offset = usize::try_from(root).unwrap() * 4096 + 2000;
                bytes[offset] ^= 0x5a;
                std::fs::write(&path, bytes).unwrap();
                std::fs::File::open(&path).unwrap().sync_all().unwrap();
                let setup = || {
                    let (dir, path) = copy(&path);
                    let db = open(&path, false);
                    (db, dir)
                };
                let (db, _check_dir) = setup();
                assert!(!db.verify().unwrap().is_ok(), "damage must be detected");
                db.rebuild_indexes().unwrap();
                verify(&db, 100_150);
                assert_eq!(
                    rows(&db, "(query [:find ?e ?v :where [?e :val ?v]])").len(),
                    99_950
                );
                drop(db);
                b.iter_batched_ref(
                    setup,
                    |(db, _)| black_box(&*db).rebuild_indexes().unwrap(),
                    BatchSize::PerIteration,
                );
            },
        );
        for mode in ["inline_unique", "long_repeated", "long_unique"] {
            group.bench_function(BenchmarkId::new(format!("multivalue_{mode}"), "10k"), |b| {
                let (_dir, path, writer) = multivalue_file(mode);
                // Freeze a pending graph/WAL pair before dropping its writer.
                let (_template_dir, template) = copy_with_wal(&path);
                drop(writer);
                let setup = || {
                    let (dir, path) = copy_with_wal(&template);
                    (open(&path, false), dir)
                };
                let (db, _check_dir) = setup();
                db.checkpoint().unwrap();
                verify(&db, 10_000);
                assert_eq!(rows(&db, SCAN).len(), 10_000);
                assert_eq!(
                    rows(&db, "(query [:find ?v :where [:mv0 :storage/value ?v]])").len(),
                    4
                );
                drop(db);
                b.iter_batched_ref(
                    setup,
                    |(db, _)| black_box(&*db).checkpoint().unwrap(),
                    BatchSize::PerIteration,
                );
            });
        }
        group.finish();
    }

    let mut group = c.benchmark_group("query/storage_v3");
    configure(&mut group);
    group.bench_function(BenchmarkId::new("multivalue_long_scan", "10k"), |b| {
        let (_dir, _path, db) = multivalue_file("long_unique");
        db.checkpoint().unwrap();
        verify(&db, 10_000);
        let result = rows(&db, SCAN);
        assert_eq!(result.len(), 10_000);
        assert!(
            result
                .iter()
                .all(|row| matches!(&row[1], Value::String(s) if s.len() == 256))
        );
        b.iter(|| black_box(rows(&db, black_box(SCAN))));
    });
    group.finish();
}
