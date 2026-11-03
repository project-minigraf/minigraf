//! G04 / Refs #776: ten sorting/pagination controls on the nightly ^query/ route.
//! Reserve 3–8 incremental job-minutes (unmeasured); the 10M envelope stays manual.

use criterion::{BenchmarkId, Criterion};
use minigraf::{Minigraf, OpenOptions, QueryResult, Value};
use std::hint::black_box;

/// Both states use the same file backend, insertion order and scalar values.
/// Pending cases keep every fact in the WAL; committed cases explicitly checkpoint.
/// Build only inside the selected Criterion closure, outside its timed loop.
fn fixture(n: usize, pending: bool, compound: bool) -> (tempfile::TempDir, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(dir.path().join("pagination.graph"))
        .open()
        .unwrap();
    for start in (0..n).step_by(1_000) {
        let mut command = String::from("(transact [");
        for j in start..(start + 1_000).min(n) {
            // 37 is coprime to both scales: every integer appears once, in a
            // fixed non-sorted order. Keyword entity IDs are deterministic too.
            let i = (j * 37) % n;
            command.push_str(&format!("[:page{i} :page/value {i}]"));
            if compound {
                // Many peers on BOTH explicit sort keys; value is the unique
                // implicit ascending suffix, independent of UUID/dictionary order.
                command.push_str(&format!(
                    "[:page{i} :page/group {}] [:page{i} :page/score {}]",
                    i % 4,
                    (i / 4) % 8
                ));
            }
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    if !pending {
        db.checkpoint().unwrap();
    }
    (dir, db)
}

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected query rows");
    };
    results
}

pub fn bench_pagination(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/pagination");
    group.sample_size(10);

    // Eight matched controls at 100k: no-Sort ascending committed scans versus
    // pending Sort, then descending heaps with k=10/1000 on both storage states.
    // See engine/order_tests.rs for the corresponding operator-counter checks:
    // ordered_scan_limit_offset_reads_only_a_prefix and
    // uuid_tie_suffix_and_descending_order_keep_sort. Pending source records
    // prevent the input-order certificate (engine/order.rs::candidate).
    let n = 100_000;
    for pending in [false, true] {
        let storage = if pending { "pending" } else { "committed" };
        for (mode, descending, offset, limit) in [
            ("asc_offset0_k10", false, 0, 10),
            ("asc_offset_half_k10", false, n / 2, 10),
            ("desc_k10", true, 0, 10),
            ("desc_k1000", true, 0, 1_000),
        ] {
            group.bench_function(BenchmarkId::new(format!("{storage}_{mode}"), "100k"), |b| {
                let (_dir, db) = fixture(n, pending, false);
                let query = format!(
                    "(query [:find ?v :where [?e :page/value ?v] :order-by [?v {}] :offset {offset} :limit {limit}])",
                    if descending { ":desc" } else { ":asc" }
                );
                let expected: Vec<_> = (offset..offset + limit)
                    .map(|i| vec![Value::Integer(if descending { n - 1 - i } else { i } as i64)])
                    .collect();
                // Validate the entire page (including offset/order), not just
                // its size. Keep fixture and oracle work out of the measurement.
                assert!(rows(&db, &query) == expected, "scalar page matches oracle");
                b.iter(|| black_box(rows(&db, black_box(&query))));
            });
        }
    }

    // Two retained-Sort compound controls. The offset cuts through a peer
    // group, so an incomplete comparator cannot return the expected page.
    for (label, n) in [("10k", 10_000), ("100k", 100_000)] {
        group.bench_function(BenchmarkId::new("committed_compound_ties", label), |b| {
            let (_dir, db) = fixture(n, false, true);
            let query = "(query [:find ?g ?s ?v :where [?e :page/group ?g] [?e :page/score ?s] [?e :page/value ?v] :order-by [?g :desc ?s] :offset 137 :limit 10])";
            let mut oracle: Vec<_> = (0..n).map(|i| (i % 4, (i / 4) % 8, i)).collect();
            oracle.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            let expected: Vec<_> = oracle[137..147]
                .iter()
                .map(|&(g, s, v)| vec![Value::Integer(g as i64), Value::Integer(s as i64), Value::Integer(v as i64)])
                .collect();
            assert!(rows(&db, query) == expected, "complete tie order matches oracle");
            b.iter(|| black_box(rows(&db, black_box(query))));
        });
    }
    group.finish();
}
