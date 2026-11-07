//! #863: the #841 shape-sweep candidates qualified for nightly timing, under
//! the existing `^window/`, `^query/` and `^recursion/` routes.
//!
//! Fixtures, queries and oracles are `helpers::shape` (the #841 harness's).
//! Each fixture is built lazily inside its selected callback, outside `iter`,
//! and the answer digest is checked once before timing. As in #841, the timed
//! body opens the query, drains its cursor and digests every row. Path,
//! answer and memory are pinned separately, by
//! `tests/nightly_candidates_acceptance_test.rs` (nightly, `test-hooks`): a
//! path regression fails there, while time here can only raise an alert.
//! Run once, untimed: cargo bench --bench minigraf_bench -- --test 'nightly_candidates/'

use crate::helpers::shape::{Shape, drain};
use criterion::{BenchmarkId, Criterion};
use std::hint::black_box;
use tempfile::TempDir;

struct Case {
    group: &'static str,
    name: &'static str,
    size: &'static str,
    shape: Shape,
    pending: bool,
    /// An operator name the cursor's `Debug` view must (true) or must not
    /// (false) show: checked before timing, without formatting any row.
    plan: Option<(&'static str, bool)>,
}

const CASES: &[Case] = &[
    Case {
        group: "window/nightly_candidates",
        name: "skew_running_sum",
        size: "100k",
        shape: Shape::WindowSkewSum { n: 50_000 },
        pending: false,
        plan: Some(("WindowPartitioned", true)),
    },
    Case {
        group: "query/nightly_candidates",
        name: "set_membership_k256",
        size: "10k",
        shape: Shape::SetMembership { n: 10_000, k: 256 },
        pending: false,
        plan: None,
    },
    Case {
        group: "query/nightly_candidates",
        name: "sort_offset_desc",
        size: "100k",
        shape: Shape::Sort {
            n: 100_000,
            desc: true,
            offset: 50_000,
            limit: Some(1000),
        },
        pending: false,
        plan: Some(("Sort", true)),
    },
    Case {
        group: "query/nightly_candidates",
        name: "sort_offset_asc",
        size: "100k",
        shape: Shape::Sort {
            n: 100_000,
            desc: false,
            offset: 50_000,
            limit: Some(1000),
        },
        pending: false,
        plan: Some(("Sort", false)),
    },
    Case {
        group: "recursion/nightly_candidates",
        name: "tree_ancestors_k1_pending",
        size: "1110",
        shape: Shape::Ancestors { edges: 1110, k: 1 },
        pending: true,
        plan: None,
    },
    Case {
        group: "recursion/nightly_candidates",
        name: "tree_ancestors_k16_committed",
        size: "1110",
        shape: Shape::Ancestors { edges: 1110, k: 16 },
        pending: false,
        plan: None,
    },
];

pub fn bench_nightly_candidates(c: &mut Criterion) {
    for group_name in [
        "window/nightly_candidates",
        "query/nightly_candidates",
        "recursion/nightly_candidates",
    ] {
        let mut group = c.benchmark_group(group_name);
        group.sample_size(10);
        for case in CASES.iter().filter(|c| c.group == group_name) {
            group.bench_function(BenchmarkId::new(case.name, case.size), |b| {
                let dir = TempDir::new().unwrap();
                let db = case
                    .shape
                    .build(&dir.path().join("candidate.graph"), case.pending);
                let query = case.shape.query();
                if let Some((op, present)) = case.plan {
                    let cursor = db.query(&query).unwrap();
                    assert_eq!(
                        format!("{cursor:?}").contains(op),
                        present,
                        "candidate plan"
                    );
                }
                assert!(
                    drain(&db, &query, case.shape.ordered()) == case.shape.oracle(),
                    "candidate answer matches the #841 oracle"
                );
                b.iter(|| black_box(drain(&db, black_box(&query), case.shape.ordered())));
                drop(db);
            });
        }
        group.finish();
    }
}
