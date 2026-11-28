#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]
//! #924 write-cost controls. Uses only public APIs present on the v3 base.
//! Copy this file and its Cargo.toml bench entry to the base for comparison.

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use minigraf::{Minigraf, OpenOptions, TxOptions};
use std::fmt::Write;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

fn command(start: usize, end: usize) -> String {
    let mut text = String::from("(transact [");
    for i in start..end {
        let target = uuid::Uuid::from_u128(1 + u128::try_from(i % 16).unwrap());
        write!(text, "[:source{i} :ref #uuid \"{target}\"]").unwrap();
    }
    text.push_str("])");
    text
}

fn file() -> (Minigraf, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("refs.graph");
    let db = Minigraf::open_with_options(
        &path,
        OpenOptions::default().wal_checkpoint_threshold(usize::MAX),
    )
    .unwrap();
    (db, dir, path)
}

fn bench_ref_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("incoming_ref_writes");
    group.sample_size(30);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for n in [1_000, 10_000] {
        let all = command(0, n);
        group.bench_with_input(BenchmarkId::new("pending", n), &n, |b, _| {
            b.iter_batched(
                || Minigraf::in_memory().unwrap(),
                |db| {
                    black_box(db.execute(black_box(&all)).unwrap());
                    db
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_with_input(BenchmarkId::new("staged", n), &n, |b, _| {
            b.iter_batched(
                || Minigraf::in_memory().unwrap().begin_write_owned().unwrap(),
                |mut tx| {
                    black_box(tx.execute(black_box(&all)).unwrap());
                    tx
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_with_input(BenchmarkId::new("wal_recovery", n), &n, |b, _| {
            b.iter_batched_ref(
                || {
                    let (db, dir, path) = file();
                    db.execute(&all).unwrap();
                    drop(db); // usize::MAX retains the WAL on the base and branch.
                    assert!(path.with_extension("graph.wal").exists());
                    (None::<Minigraf>, dir, path)
                },
                |(opened, _, path)| {
                    *opened = Some(black_box(
                        Minigraf::open_with_options(
                            path,
                            OpenOptions::default().wal_checkpoint_threshold(usize::MAX),
                        )
                        .unwrap(),
                    ));
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_with_input(BenchmarkId::new("checkpoint", n), &n, |b, _| {
            b.iter_batched_ref(
                || {
                    let f = file();
                    f.0.execute(&all).unwrap();
                    f
                },
                |(db, _, _)| {
                    db.checkpoint().unwrap();
                    black_box(())
                },
                BatchSize::PerIteration,
            );
        });
        let first = command(0, n / 2);
        let second = command(n / 2, n);
        group.bench_with_input(BenchmarkId::new("promotion", n), &n, |b, _| {
            b.iter_batched_ref(
                || {
                    let (db, dir, _) = file();
                    // Include a pending ref range in the promotion's first chunk.
                    db.execute(&command(n, n + 16)).unwrap();
                    let mut tx = db
                        .begin_write_owned_with(TxOptions::new().chunk_facts(n - 1))
                        .unwrap();
                    tx.execute(&first).unwrap();
                    (tx, dir)
                },
                |(tx, _)| black_box(tx.execute(black_box(&second)).unwrap()),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_ref_writes);
criterion_main!(benches);
