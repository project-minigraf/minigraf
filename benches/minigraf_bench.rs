// TODO (Phase 7.4): Add profiling integration before optimizing the query path.
//
// Integrate `pprof` as a Criterion profiler (via the `pprof` crate's Criterion
// feature or `criterion-pprof`) so that `cargo bench` can emit flamegraphs.
// Run the query/negation/aggregation/disjunction groups at 10K and 100K facts
// to confirm that `filter_facts_for_query` (index rebuild) and
// `net_asserted_facts` are the dominant costs before making structural changes.
//
// Example setup (Cargo.toml):
//   [dev-dependencies]
//   pprof = { version = "...", features = ["flamegraph", "criterion"] }
//
//   [[bench]]
//   name = "minigraf_bench"
//   harness = false
//
// Then wrap criterion_group! with a PProfProfiler and run:
//   cargo bench -- --profile-time 10

mod helpers;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use minigraf::OpenOptions;

// ── Task 3: insert/ ───────────────────────────────────────────────────────────

fn bench_insert(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("100k", 100_000)];

    // single_fact: insert one fact into a pre-populated in-memory DB.
    // DB created once per scale; b.iter() accumulates facts across iterations
    // (realistic steady-state: insert into a DB of approximately scale N).
    {
        let mut group = c.benchmark_group("insert/single_fact");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute("(transact [[:ebench :val 0]])").unwrap());
            });
        }
        group.finish();
    }

    // batch_100: insert 100 facts in a single transact
    {
        let mut group = c.benchmark_group("insert/batch_100");
        let batch_cmd: String = {
            let mut s = String::from("(transact [");
            for i in 0..100 {
                s.push_str(&format!("[:eb{} :val {}]", i, i));
            }
            s.push(']');
            s.push(')');
            s
        };
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            let cmd = batch_cmd.clone();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute(&cmd).unwrap());
            });
        }
        group.finish();
    }

    // explicit_tx: single fact via begin_write()/commit()
    {
        let mut group = c.benchmark_group("insert/explicit_tx");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    let mut tx = db.begin_write().unwrap();
                    tx.execute("(transact [[:ebench :val 0]])").unwrap();
                    tx.commit().unwrap();
                });
            });
        }
        group.finish();
    }
}

// ── Task 4: insert_file/ ──────────────────────────────────────────────────────

fn bench_insert_file(c: &mut Criterion) {
    use tempfile::NamedTempFile;
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("100k", 100_000)];

    // single_fact: one execute() per iter against growing file-backed DB
    {
        let mut group = c.benchmark_group("insert_file/single_fact");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| db.execute("(transact [[:ebench :val 0]])").unwrap());
                drop(tmp); // explicit: keep file alive for entire bench duration
            });
        }
        group.finish();
    }

    // batch_100: 100 facts per execute()
    {
        let mut group = c.benchmark_group("insert_file/batch_100");
        let batch_cmd: String = {
            let mut s = String::from("(transact [");
            for i in 0..100 {
                s.push_str(&format!("[:eb{} :val {}]", i, i));
            }
            s.push(']');
            s.push(')');
            s
        };
        for &(label, n) in SCALES {
            let cmd = batch_cmd.clone();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| db.execute(&cmd).unwrap());
                drop(tmp);
            });
        }
        group.finish();
    }

    // explicit_tx: begin_write()/commit() per iter
    {
        let mut group = c.benchmark_group("insert_file/explicit_tx");
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let tmp = NamedTempFile::new().unwrap();
                let path = tmp.path().to_str().unwrap().to_string();
                helpers::populate_file(n, &path);
                let db = helpers::open_file_no_checkpoint(&path);
                b.iter(|| {
                    let mut tx = db.begin_write().unwrap();
                    tx.execute("(transact [[:ebench :val 0]])").unwrap();
                    tx.commit().unwrap();
                });
                drop(tmp);
            });
        }
        group.finish();
    }
}

// ── Task 5: query/ ────────────────────────────────────────────────────────────

fn bench_query(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[
        ("1k", 1_000),
        ("10k", 10_000),
        ("100k", 100_000),
        ("1m", 1_000_000),
    ];

    // point_entity: EAVT range scan on a known entity
    {
        let mut group = c.benchmark_group("query/point_entity");
        group.sample_size(10); // 1m scale takes ~2s/iter
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // point_attribute: AEVT scan — all entities with :val attribute
    {
        let mut group = c.benchmark_group("query/point_attribute");
        group.sample_size(10); // 1m scale returns all N results — slow
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute("(query [:find ?e :where [?e :val _]])").unwrap());
            });
        }
        group.finish();
    }

    // join_3pattern: 3-clause join across two :next hops
    // Uses populate_for_join which inserts both :val and :next facts.
    // Query: e0 -> e1 -> e2, return e2's :val
    {
        let mut group = c.benchmark_group("query/join_3pattern");
        group.sample_size(10); // 1m scale may be slow
        for &(label, n) in SCALES {
            let db = helpers::populate_for_join(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?v :where [:e0 :next ?m] [?m :next ?end] [?end :val ?v]])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Task 6: time_travel/ ──────────────────────────────────────────────────────

fn bench_time_travel(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[
        ("1k", 1_000),
        ("10k", 10_000),
        ("100k", 100_000),
        ("1m", 1_000_000),
    ];

    // as_of_counter: :as-of with a large tx counter (all facts pass)
    // tx count after N facts inserted in batches of 100 is N/100.
    // Using 999999 ensures all real tx counts are <= this.
    {
        let mut group = c.benchmark_group("time_travel/as_of_counter");
        group.sample_size(10); // 1m scale takes ~2s/iter; 10 samples suffices
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?v :as-of 999999 :where [:e0 :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // valid_at: :valid-at with a far-future timestamp (all facts valid)
    // Facts inserted without explicit valid-from default to tx time (~2026);
    // valid-to defaults to MAX (forever). "2099-01-01T00:00:00Z" is within that window.
    {
        let mut group = c.benchmark_group("time_travel/valid_at");
        group.sample_size(10); // 1m scale takes ~2s/iter; 10 samples suffices
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        r#"(query [:find ?v :valid-at "2099-01-01T00:00:00Z" :where [:e0 :val ?v]])"#,
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Task 7: recursion/ ────────────────────────────────────────────────────────

fn bench_recursion(c: &mut Criterion) {
    // chain: linear chain of depth N — worst case for iteration depth.
    // depth_100 already takes ~16s/iter (semi-naive is O(depth²) on chains).
    // depth_1k is excluded as it would take hours. sample_size(10) keeps total time manageable.
    {
        let mut group = c.benchmark_group("recursion/chain");
        group.sample_size(10);
        for &(label, depth) in &[("depth_10", 10usize), ("depth_100", 100)] {
            let db = helpers::chain_graph(depth);
            group.bench_with_input(BenchmarkId::from_parameter(label), &depth, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?to :where (reach :n0 ?to)])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // fanout: fan-out tree — tests delta size per semi-naive iteration.
    // w5_d5 (~3905 nodes) is excluded: produces ~7.6M transitive-closure tuples,
    // takes ~140s/iter and causes OOM. w10_d3 (~1110 nodes) is sufficient.
    {
        let mut group = c.benchmark_group("recursion/fanout");
        group.sample_size(10);
        // (width, depth): (10,3) ~1110 nodes — manageable transitive closure
        let (label, width, depth) = ("w10_d3", 10usize, 3usize);
        let db = helpers::fanout_graph(width, depth);
        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &(width, depth),
            |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?to :where (reach :n0 ?to)])")
                        .unwrap()
                });
            },
        );
        group.finish();
    }
}

// ── Task 8: open/ ─────────────────────────────────────────────────────────────

fn bench_open(c: &mut Criterion) {
    use tempfile::NamedTempFile;

    // checkpointed: open a fully-checkpointed .graph file (no WAL replay)
    {
        let mut group = c.benchmark_group("open/checkpointed");
        for &(label, n) in &[
            ("1k", 1_000usize),
            ("10k", 10_000),
            ("100k", 100_000),
            ("1m", 1_000_000),
        ] {
            // Create the pre-populated file ONCE (outside iter loop).
            let tmp = NamedTempFile::new().unwrap();
            let path = tmp.path().to_str().unwrap().to_string();
            helpers::populate_file(n, &path);
            // Checkpoint is already done by populate_file; WAL sidecar absent.

            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                let path = path.clone();
                b.iter(|| {
                    // Open and immediately drop — measures full open time.
                    let _db = OpenOptions::new()
                        .page_cache_size(256)
                        .path(&path)
                        .open()
                        .unwrap();
                });
            });
            drop(tmp);
        }
        group.finish();
    }

    // wal_replay: open with N WAL entries pending (crash-recovery path)
    {
        let mut group = c.benchmark_group("open/wal_replay");
        for &(label, n) in &[("1k", 1_000usize), ("10k", 10_000)] {
            let tmp = NamedTempFile::new().unwrap();
            let path = tmp.path().to_str().unwrap().to_string();
            // populate_file_no_checkpoint leaves all facts in the WAL (not checkpointed).
            helpers::populate_file_no_checkpoint(n, &path);

            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                let path = path.clone();
                b.iter(|| {
                    // Each open replays the WAL. WAL is NOT consumed (no checkpoint during bench).
                    let _db = OpenOptions::new()
                        .page_cache_size(256)
                        .path(&path)
                        .open()
                        .unwrap();
                });
            });
            drop(tmp);
        }
        group.finish();
    }
}

// ── Task 9: checkpoint/ ───────────────────────────────────────────────────────

fn bench_checkpoint(c: &mut Criterion) {
    use criterion::BatchSize;
    use tempfile::NamedTempFile;

    let mut group = c.benchmark_group("checkpoint");
    for &(label, n) in &[("1k", 1_000usize), ("10k", 10_000)] {
        group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Setup: file DB with n WAL-committed facts, no checkpoint yet.
                    let tmp = NamedTempFile::new().unwrap();
                    let path = tmp.path().to_str().unwrap().to_string();
                    helpers::populate_file_no_checkpoint(n, &path);
                    // Re-open to get a fresh handle (populate drops its handle).
                    let db = helpers::open_file_no_checkpoint(&path);
                    (db, tmp) // keep tmp alive
                },
                |(db, _tmp)| {
                    // Routine: flush WAL to packed pages, delete WAL sidecar.
                    db.checkpoint().unwrap();
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

// ── Task 10: concurrent/ ─────────────────────────────────────────────────────

fn bench_concurrent(c: &mut Criterion) {
    use std::sync::{Arc as StdArc, Barrier};
    use std::time::Instant;

    // Fresh DB per scenario to prevent unbounded fact accumulation across benchmarks.
    // (Sharing one DB caused OOM as write scenarios accumulate millions of facts.)

    // readers: N threads all querying simultaneously
    {
        let mut group = c.benchmark_group("concurrent/readers");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = helpers::populate_in_memory(10_000);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                                        .unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait(); // release all threads simultaneously
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // readers_plus_writer: (N-1) readers + 1 writer
    {
        let mut group = c.benchmark_group("concurrent/readers_plus_writer");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let db = helpers::populate_in_memory(10_000);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let n_readers = n_threads - 1;
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        // readers
                        for _ in 0..n_readers {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                                        .unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        // 1 writer
                        {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // serialized_writers: N threads competing for the write Mutex.
    // NOTE: Measures lock-contention overhead, NOT write parallelism.
    // Writes are serialized by design. Throughput expected to stay flat or decrease slightly.
    {
        let mut group = c.benchmark_group("concurrent/serialized_writers");
        for &(label, n_threads) in &[("2", 2usize), ("4", 4), ("8", 8), ("16", 16)] {
            let db = helpers::populate_in_memory(10_000);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }
}

// ── Task 11: concurrent_file/ ────────────────────────────────────────────────

fn bench_concurrent_file(c: &mut Criterion) {
    use std::sync::{Arc as StdArc, Barrier};
    use std::time::Instant;
    use tempfile::NamedTempFile;

    // Fresh file-backed DB per scenario to prevent unbounded WAL growth and OOM.

    // readers (file-backed): concurrent page-cache reads under RwLock
    {
        let mut group = c.benchmark_group("concurrent_file/readers");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let tmp = Box::new(NamedTempFile::new().unwrap());
            let path = tmp.path().to_str().unwrap().to_string();
            helpers::populate_file(10_000, &path);
            let db = helpers::open_file_no_checkpoint(&path);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                                        .unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // readers_plus_writer (file-backed): readers + 1 WAL-writing thread
    {
        let mut group = c.benchmark_group("concurrent_file/readers_plus_writer");
        for &(label, n_threads) in &[("4", 4usize), ("8", 8), ("16", 16)] {
            let tmp = Box::new(NamedTempFile::new().unwrap());
            let path = tmp.path().to_str().unwrap().to_string();
            helpers::populate_file(10_000, &path);
            let db = helpers::open_file_no_checkpoint(&path);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let n_readers = n_threads - 1;
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_readers {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                                        .unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }

    // serialized_writers (file-backed): N WAL-writing threads queuing on Mutex
    {
        let mut group = c.benchmark_group("concurrent_file/serialized_writers");
        for &(label, n_threads) in &[("2", 2usize), ("4", 4), ("8", 8), ("16", 16)] {
            let tmp = Box::new(NamedTempFile::new().unwrap());
            let path = tmp.path().to_str().unwrap().to_string();
            helpers::populate_file(10_000, &path);
            let db = helpers::open_file_no_checkpoint(&path);
            let db = StdArc::clone(&db);
            group.bench_with_input(
                BenchmarkId::from_parameter(label),
                &n_threads,
                |b, &n_threads| {
                    b.iter_custom(|iters| {
                        let barrier = StdArc::new(Barrier::new(n_threads + 1));
                        let mut handles = Vec::new();
                        for _ in 0..n_threads {
                            let db = StdArc::clone(&db);
                            let barrier = StdArc::clone(&barrier);
                            handles.push(std::thread::spawn(move || {
                                barrier.wait();
                                let start = Instant::now();
                                for _ in 0..iters {
                                    db.execute("(transact [[:ebench :val 0]])").unwrap();
                                }
                                start.elapsed()
                            }));
                        }
                        barrier.wait();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .max()
                            .unwrap()
                    });
                },
            );
        }
        group.finish();
    }
}

// ── Negation: not / not-join ──────────────────────────────────────────────────

fn bench_negation(c: &mut Criterion) {
    // 100k excluded: returning O(N) results makes --test mode too slow even for one run.
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // not/scale: overhead of the `not` post-filter at different DB sizes.
    // 10% of entities are excluded (`:banned true`).
    // Query: all entities that have :val and are NOT banned.
    {
        let mut group = c.benchmark_group("negation/not_scale");
        for &(label, n) in SCALES {
            let excluded = n / 10;
            let db = helpers::populate_with_not_exclusion(n, excluded);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e :where [?e :val ?v] (not [?e :banned true])])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // not-join/scale: overhead of the `not-join` post-filter at different DB sizes.
    // 10% of entities have a dep on a "bad" dependency.
    // Query: all entities that have :val and have no :dep whose :status is :bad.
    {
        let mut group = c.benchmark_group("negation/not_join_scale");
        for &(label, n) in SCALES {
            let excluded = n / 10;
            let db = helpers::populate_with_not_join_exclusion(n, excluded);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e :where [?e :val ?v] \
                         (not-join [?e] [?e :dep ?d] [?d :status :bad])])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }

    // not/selectivity: fixed 10k DB, vary the excluded fraction.
    // Shows how the exclusion ratio affects query latency.
    {
        let mut group = c.benchmark_group("negation/not_selectivity");
        let n = 10_000;
        for &(label, pct) in &[
            ("excl_0pct", 0usize),
            ("excl_25pct", 25),
            ("excl_50pct", 50),
            ("excl_75pct", 75),
            ("excl_100pct", 100),
        ] {
            let excluded = n * pct / 100;
            let db = helpers::populate_with_not_exclusion(n, excluded);
            group.bench_with_input(BenchmarkId::from_parameter(label), &excluded, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e :where [?e :val ?v] (not [?e :banned true])])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // not/rule_body: `not` inside a rule body (StratifiedEvaluator overhead).
    // Rule: `(eligible ?x) :- [?x :val ?v] (not [?x :blocked true])`
    // 10% of entities are blocked.
    {
        let mut group = c.benchmark_group("negation/not_rule_body");
        for &(label, n) in &[("1k", 1_000usize), ("10k", 10_000)] {
            let excluded = n / 10;
            let db = helpers::populate_with_not_rule(n, excluded);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e :where (eligible ?e)])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Task 7: concurrent_btree_scan/ ────────────────────────────────────────
// Measures throughput of simultaneous EAVT range scans against committed B+tree.
// A near-linear drop in per-thread throughput as N grows signals backend mutex
// contention and would trigger the per-page locking revisit noted in the spec.

fn bench_concurrent_btree_scan(c: &mut Criterion) {
    use std::sync::{Arc as StdArc, Barrier};
    use std::time::Instant;
    use tempfile::NamedTempFile;

    let mut group = c.benchmark_group("concurrent_btree_scan");
    group.sample_size(10);

    for &(label, n_threads) in &[("2", 2usize), ("4", 4), ("8", 8)] {
        // Pre-populate and checkpoint file DB so all facts are in committed B+tree.
        let tmp = Box::new(NamedTempFile::new().unwrap());
        let path = tmp.path().to_str().unwrap().to_string();
        helpers::populate_file(10_000, &path);
        // Open a handle shared by all threads
        let db = helpers::open_file_no_checkpoint(&path);
        let db = StdArc::clone(&db);

        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &n_threads,
            |b, &n_threads| {
                b.iter_custom(|iters| {
                    let barrier = StdArc::new(Barrier::new(n_threads + 1));
                    let mut handles = Vec::new();
                    for _ in 0..n_threads {
                        let db = StdArc::clone(&db);
                        let barrier = StdArc::clone(&barrier);
                        handles.push(std::thread::spawn(move || {
                            barrier.wait();
                            let start = Instant::now();
                            for _ in 0..iters {
                                // EAVT range scan: entity :e0 with attribute :val
                                db.execute("(query [:find ?v :where [:e0 :val ?v]])")
                                    .unwrap();
                            }
                            start.elapsed()
                        }));
                    }
                    barrier.wait();
                    handles
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .max()
                        .unwrap()
                });
            },
        );
        drop(tmp);
    }
    group.finish();
}

// ── Disjunction: or / or-join ─────────────────────────────────────────────────

fn bench_disjunction(c: &mut Criterion) {
    // 100k excluded: O(N) results make --test mode too slow.
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // or/scale: overhead of the `or` expansion at different DB sizes.
    // 25% tagged-a (first quarter), 25% tagged-b (last quarter), 50% untagged.
    // Query returns the 50% that have either tag.
    // sample_size(10): 1k ~800ms/iter, 10k ~8s/iter — too slow for 100 samples.
    {
        let mut group = c.benchmark_group("disjunction/or_scale");
        group.sample_size(10);
        for &(label, n) in SCALES {
            let a_count = n / 4;
            let b_count = n / 4;
            let db = helpers::populate_with_or_tags(n, a_count, b_count);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e :where [?e :val ?v] \
                         (or [?e :tag-a true] [?e :tag-b true])])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }

    // or-join/scale: same data, using `or-join` with explicit join variable.
    // Semantically equivalent to or/scale but exercises the or-join projection path.
    {
        let mut group = c.benchmark_group("disjunction/or_join_scale");
        group.sample_size(10);
        for &(label, n) in SCALES {
            let a_count = n / 4;
            let b_count = n / 4;
            let db = helpers::populate_with_or_tags(n, a_count, b_count);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e :where [?e :val ?v] \
                         (or-join [?e] [?e :tag-a true] [?e :tag-b true])])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }

    // or/selectivity: fixed 10k DB, vary the fraction matching either branch.
    // Shows how match density affects or-expansion cost.
    {
        let mut group = c.benchmark_group("disjunction/or_selectivity");
        group.sample_size(10);
        let n = 10_000;
        for &(label, pct) in &[
            ("match_0pct", 0usize),
            ("match_25pct", 25),
            ("match_50pct", 50),
            ("match_75pct", 75),
            ("match_100pct", 100),
        ] {
            let a_count = n * pct / 100;
            let b_count = 0; // only tag-a varies; tag-b absent
            let db = helpers::populate_with_or_tags(n, a_count, b_count);
            group.bench_with_input(BenchmarkId::from_parameter(label), &pct, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e :where [?e :val ?v] \
                         (or [?e :tag-a true] [?e :tag-b true])])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }

    // or/rule_body: `or` inside a rule body (mixed-rules path overhead).
    // Rule: `(tagged ?x) :- (or [?x :tag-a true] [?x :tag-b true])`
    // Half have tag-a, half have tag-b → all entities match.
    {
        let mut group = c.benchmark_group("disjunction/or_rule_body");
        group.sample_size(10);
        for &(label, n) in SCALES {
            let db = helpers::populate_with_or_rule(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute("(query [:find ?e :where (tagged ?e)])").unwrap());
            });
        }
        group.finish();
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn bench_aggregation(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // count/scale: scalar `count` aggregate — measures aggregation post-processing overhead.
    // Single output row regardless of DB size; cost is dominated by binding collection.
    {
        let mut group = c.benchmark_group("aggregation/count_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find (count ?e) :where [?e :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // grouped_count/scale: `count` grouped by department — one output row per dept.
    // 10 departments → 10 output rows. Measures grouping + per-group aggregation.
    {
        let mut group = c.benchmark_group("aggregation/grouped_count_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_with_dept(n, 10);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?dept (count ?e) :where [?e :dept ?dept]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // sum/scale: `sum` aggregate over integer :val — measures numeric accumulation.
    {
        let mut group = c.benchmark_group("aggregation/sum_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find (sum ?v) :where [?e :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // with/grouped_sum: `:with` clause prevents row collapse before aggregation.
    // Each entity has a unique :val, so `:with ?e` keeps individual rows distinct.
    {
        let mut group = c.benchmark_group("aggregation/with_grouped_sum");
        for &(label, n) in SCALES {
            let db = helpers::populate_with_dept(n, 10);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?dept (sum ?v) :with ?e \
                         :where [?e :dept ?dept] [?e :val ?v]])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Expression clauses ────────────────────────────────────────────────────────

fn bench_expr(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // filter/scale: `[(< ?v N)]` comparison filter — measures expr post-filter pass overhead.
    // Keeps entities with :val < half of n; drops the other half.
    {
        let mut group = c.benchmark_group("expr/filter_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            let threshold = n / 2;
            let query = format!(
                "(query [:find ?e :where [?e :val ?v] [(< ?v {})]])",
                threshold
            );
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| db.execute(&query).unwrap());
            });
        }
        group.finish();
    }

    // binding/scale: `[(+ ?v 1) ?result]` arithmetic binding — measures expr eval + bind overhead.
    // Binds ?result = ?v + 1 for every row; all rows survive.
    {
        let mut group = c.benchmark_group("expr/binding_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?result :where [?e :val ?v] [(+ ?v 1) ?result]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // binding_into_agg: `[(* ?v 2) ?doubled]` feeding `(sum ?doubled)`.
    // Measures expr bind + aggregation pipeline together.
    {
        let mut group = c.benchmark_group("expr/binding_into_agg");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find (sum ?doubled) \
                         :where [?e :val ?v] [(* ?v 2) ?doubled]])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Window functions ────────────────────────────────────────────────────────────

fn bench_window(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // running_sum: sum over ordered rows — measures window accumulator path.
    {
        let mut group = c.benchmark_group("window/running_sum");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e (sum ?v :over (:order-by ?v)) :where [?e :val ?v]])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }

    // rank: rank function — measures sorting overhead for ranking.
    {
        let mut group = c.benchmark_group("window/rank");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e (rank :over (:order-by ?v)) :where [?e :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // row_number: row number function — similar overhead to rank.
    {
        let mut group = c.benchmark_group("window/row_number");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e (row-number :over (:order-by ?v)) :where [?e :val ?v]])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Temporal metadata queries ─────────────────────────────────────────────────

fn bench_temporal_metadata(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // tx_time: bind transaction timestamp — measures per-row projection overhead.
    {
        let mut group = c.benchmark_group("temporal_metadata/tx_time");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e ?t :any-valid-time :where [?e :val ?v] [?e :db/tx-count ?t]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // valid_from: bind valid-from timestamp.
    {
        let mut group = c.benchmark_group("temporal_metadata/valid_from");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e ?vf :any-valid-time :where [?e :val ?v] [?e :db/valid-from ?vf]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // valid_to: bind valid-to timestamp.
    {
        let mut group = c.benchmark_group("temporal_metadata/valid_to");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e ?vt :any-valid-time :where [?e :val ?v] [?e :db/valid-to ?vt]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── UDF dispatch overhead ─────────────────────────────────────────────────────

fn bench_udf(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // aggregate_sum_dispatch: UDF aggregate vs built-in sum — isolates closure dispatch.
    {
        let mut group = c.benchmark_group("udf/aggregate_sum_dispatch");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            db.register_aggregate(
                "udf_sum",
                || 0i64,
                |acc: &mut i64, v: &minigraf::Value| {
                    if let minigraf::Value::Integer(i) = v {
                        *acc += *i;
                    }
                },
                |acc: &i64, _n: usize| minigraf::Value::Integer(*acc),
            )
            .unwrap();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find (udf_sum ?v) :where [?e :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // predicate_filter_dispatch: UDF predicate vs built-in comparison.
    {
        let mut group = c.benchmark_group("udf/predicate_filter_dispatch");
        for &(label, n) in SCALES {
            let db = helpers::populate_in_memory(n);
            db.register_predicate("udf_gt", |v: &minigraf::Value| -> bool {
                if let minigraf::Value::Integer(i) = v {
                    *i > 500
                } else {
                    false
                }
            })
            .unwrap();
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find ?e :where [?e :val ?v] (udf_gt ?v)])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Aggregation: count-distinct ───────────────────────────────────────────────

fn bench_aggregation_extras(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // count_distinct_scale: count-distinct with 50% duplicate values.
    // Measures the distinct-dedup path overhead.
    {
        let mut group = c.benchmark_group("aggregation/count_distinct_scale");
        for &(label, n) in SCALES {
            let db = helpers::populate_with_duplicates(n, 50);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute("(query [:find (count-distinct ?v) :where [?e :val ?v]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }
}

// ── Query: regex filter ──────────────────────────────────────────────────────

fn bench_query_extras(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000)];

    // regex_filter: query with matches? predicate — measures regex evaluation overhead.
    // All entities have :val strings matching pattern "item-\d+".
    {
        let mut group = c.benchmark_group("query/regex_filter");
        for &(label, n) in SCALES {
            let db = helpers::populate_with_string_vals(n);
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, _| {
                b.iter(|| {
                    db.execute(
                        "(query [:find ?e :where [?e :val ?v] (matches? ?v \"item-\\\\d+\")])",
                    )
                    .unwrap()
                });
            });
        }
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_insert,
    bench_insert_file,
    bench_query,
    bench_time_travel,
    bench_recursion,
    bench_negation,
    bench_disjunction,
    bench_aggregation,
    bench_expr,
    bench_window,
    bench_temporal_metadata,
    bench_udf,
    bench_aggregation_extras,
    bench_query_extras,
    bench_open,
    bench_checkpoint,
    bench_concurrent,
    bench_concurrent_file,
    bench_concurrent_btree_scan,
);
criterion_main!(benches);
