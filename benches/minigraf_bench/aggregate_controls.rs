//! P14 / G16, Refs #776: ten aggregate controls on the nightly ^aggregation/ route.
//! Reserve 3–8 incremental job-minutes pending phase-B timing. Numeric edge-case
//! permutations stay in correctness tests; these fixtures exercise representative costs.

use criterion::{BenchmarkId, Criterion};
use minigraf::{Minigraf, OpenOptions, QueryResult, Value};
use std::fmt::Write;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Duration;

const HASH_ROWS: usize = 10_000;
const GROUP_ROWS: usize = 100_000;

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        unreachable!("expected aggregate rows");
    };
    results
}

// Analytic oracles, independent of the database's accumulator implementation.
// Exact sums must preserve small terms through cancellation; mixed distinct
// sums retain Integer(1) and Float(1.0) as separate numeric identities.
fn hash_expected(mode: &str) -> Vec<Vec<Value>> {
    vec![match mode {
        "hash_avg" => vec![Value::Float(2.5)],
        "hash_exact_float_sum" => vec![Value::Float(5_000.0)],
        "hash_mixed_sum_distinct" => vec![Value::Float(5_000.0), Value::Float(4.0)],
        "hash_mixed_extrema" => vec![Value::Integer(-2), Value::Keyword(":last".into())],
        _ => unreachable!("unknown hash case"),
    }]
}

fn group_expected(mode: &str) -> Vec<Vec<Value>> {
    (0..10)
        .map(|g| {
            let count = if g == 0 { 90_000 } else { 100 };
            match mode {
                "skew_count_committed" | "skew_count_pending" => {
                    vec![Value::Integer(g), Value::Integer(count)]
                }
                "skew_avg_committed" => vec![Value::Integer(g), Value::Float(g as f64)],
                "tuple_repeated_constant" => vec![
                    Value::Integer(2),
                    Value::Integer(g),
                    Value::Integer(g),
                    Value::Integer(count),
                ],
                "tuple_float_constant_fallback" => vec![
                    Value::Float(2.0),
                    Value::Integer(g),
                    Value::Integer(g),
                    Value::Integer(count),
                ],
                // :with ?e creates 100k singleton effective groups. The first
                // ten visible rows all belong to the large bucket, with count 1.
                "tuple_hidden_with_fallback" => vec![Value::Integer(0), Value::Integer(1)],
                _ => unreachable!("unknown group case"),
            }
        })
        .collect()
}

fn insert_values(db: &Minigraf, n: usize, attribute: &str, value: impl Fn(usize) -> String) {
    for start in (0..n).step_by(1_000) {
        let mut command = String::from("(transact [");
        for i in start..(start + 1_000).min(n) {
            write!(command, "[:agg-control/e{i} {attribute} {}]", value(i)).unwrap();
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
}

// Close the database before removing its directory, including its pending WAL.
struct GroupFixture {
    db: Minigraf,
    _dir: tempfile::TempDir,
}

fn group_fixture(pending: bool) -> GroupFixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aggregate.graph");
    let mut db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(&path)
        .open()
        .unwrap();
    insert_values(&db, GROUP_ROWS, ":agg-control/group", |i| {
        // One bucket owns 90% of the rows; the other 100 own 100 each.
        // Nonadjacent insertion of small buckets forces a real AVET ordering.
        if i < 90_000 { 0 } else { 1 + i % 100 }.to_string()
    });
    if !pending {
        db.checkpoint().unwrap();
        drop(db);
        db = OpenOptions::new()
            .read_only(true)
            .path(path)
            .open()
            .unwrap();
    }
    GroupFixture { db, _dir: dir }
}

pub fn bench_aggregate_controls(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation/v3_controls");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    // Pending in-memory input cannot receive an ordered-input certificate:
    // these are ordinary hash aggregates, with no :over window syntax.
    for (mode, literals, find) in [
        ("hash_avg", &["1", "2.0", "3", "4.0"][..], "(avg ?v)"),
        (
            "hash_exact_float_sum",
            &["10000000000000000.0", "1.0", "-10000000000000000.0", "1.0"][..],
            "(sum ?v)",
        ),
        (
            "hash_mixed_sum_distinct",
            &[
                "9223372036854775807",
                "1.0",
                "-9223372036854775807",
                "1",
                "true",
                "\"skip\"",
                "nil",
                "2",
            ][..],
            "(sum ?v) (sum-distinct ?v)",
        ),
        (
            "hash_mixed_extrema",
            &["nil", "-2", "1.5", "true", "\"z\"", ":last", "3", "false"][..],
            "(min ?v) (max ?v)",
        ),
    ] {
        let fixture = OnceLock::new();
        group.bench_function(BenchmarkId::new(mode, "10k"), |b| {
            let (db, query) = fixture.get_or_init(|| {
                let db = Minigraf::in_memory().unwrap();
                insert_values(&db, HASH_ROWS, ":agg-control/value", |i| {
                    literals[i % literals.len()].to_string()
                });
                let query = format!("(query [:find {find} :where [?e :agg-control/value ?v]])");
                assert!(
                    rows(&db, &query) == hash_expected(mode),
                    "hash aggregate matches oracle"
                );
                (db, query)
            });
            b.iter(|| black_box(rows(db, black_box(query))));
        });
    }

    // Shared lazily initialized fixtures survive Criterion's repeated sample
    // callbacks. Unselected routes and --list do no construction or validation.
    let committed = OnceLock::new();
    let pending = OnceLock::new();
    // These mirror the strategy checks in engine/aggregate_order_tests.rs:
    // generalized_constant_and_repeated_group_columns_keep_projection_and_sort_order
    // and generalized_unproved_tuple_components_stay_materialized.
    // Integer constants/repeated dimensions certify the complete tuple; a float
    // constant or hidden entity dimension keeps hash aggregation and Sort.
    for (mode, is_pending, find, with, clause) in [
        ("skew_count_committed", false, "?g (count ?e)", "", ""),
        ("skew_count_pending", true, "?g (count ?e)", "", ""),
        ("skew_avg_committed", false, "?g (avg ?g)", "", ""),
        (
            "tuple_repeated_constant",
            false,
            "?k ?g ?g (count ?e)",
            "",
            "[(+ 1 1) ?k]",
        ),
        (
            "tuple_float_constant_fallback",
            false,
            "?k ?g ?g (count ?e)",
            "",
            "[(+ 1.0 1.0) ?k]",
        ),
        (
            "tuple_hidden_with_fallback",
            false,
            "?g (count ?e)",
            ":with ?e",
            "",
        ),
    ] {
        let validated = OnceLock::new();
        group.bench_function(BenchmarkId::new(mode, "100k"), |b| {
            let fixture = if is_pending { &pending } else { &committed }
                .get_or_init(|| group_fixture(is_pending));
            let query = validated.get_or_init(|| {
                let order = if clause.is_empty() { "?g" } else { "?k ?g" };
                let query = format!(
                    "(query [:find {find} {with} :where [?e :agg-control/group ?g] {clause} :order-by [{order}] :limit 10])"
                );
                assert!(rows(&fixture.db, &query) == group_expected(mode), "group page matches oracle");
                query
            });
            // Includes parse/plan, proof reads, aggregation, ordering and complete
            // materialization of the limited page. No setup or oracle is timed.
            b.iter(|| black_box(rows(&fixture.db, black_box(query))));
        });
    }
    group.finish();
}
