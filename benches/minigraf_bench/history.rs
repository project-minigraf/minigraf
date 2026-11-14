use criterion::{BenchmarkId, Criterion};
use std::hint::black_box;

// ── point_query_chain_depth (Issue #323) ─────────────────────────────────────
//
// One entity whose :hash is retracted/reasserted `depth` times (exactly one live
// value), plus a never-churned :other on the same entity; checkpointed file DB.

pub(super) fn bench_point_query_chain_depth(c: &mut Criterion) {
    const DEPTHS: &[usize] = &[1, 500, 2000];
    const QUERIES: &[(&str, &str)] = &[
        (
            "churned_attr",
            "(query [:find ?v :where [:e/hot :hash ?v]])",
        ),
        (
            "sibling_attr",
            "(query [:find ?v :where [:e/hot :other ?v]])",
        ),
        ("attr_scan", "(query [:find ?v :where [?e :hash ?v]])"),
        // tx 2002 asserted h0: an :as-of point query on committed history (#379).
        (
            "churned_attr_as_of",
            "(query [:find ?v :as-of 2002 :valid-at :any-valid-time :where [:e/hot :hash ?v]])",
        ),
    ];
    for &(name, q) in QUERIES {
        let mut group = c.benchmark_group(format!("point_query_chain_depth/{name}"));
        group.sample_size(20);
        for &depth in DEPTHS {
            let fixture = std::sync::OnceLock::new();
            group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
                // Skip unselected IDs and reuse one fixture across Criterion's
                // warmup and sample callbacks. No fixture work is timed.
                let (db, _dir) = fixture.get_or_init(|| {
                    let dir = tempfile::tempdir().unwrap();
                    let db = minigraf::Minigraf::open(dir.path().join("b.graph")).unwrap();
                    for i in 0..2000 {
                        db.execute(&format!("(transact [[:f/{i} :x {i}]])"))
                            .unwrap();
                    }
                    db.execute("(transact [[:e/hot :other \"o\"]])").unwrap();
                    db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h0\"]])")
                        .unwrap();
                    for i in 1..depth {
                        db.execute(&format!("(retract [[:e/hot :hash \"h{}\"]])", i - 1))
                            .unwrap();
                        db.execute(&format!(
                            "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [[:e/hot :hash \"h{i}\"]])"
                        ))
                        .unwrap();
                    }
                    db.checkpoint().unwrap();
                    let minigraf::QueryResult::QueryResults { results, .. } = db.execute(q).unwrap()
                    else {
                        panic!("expected history query rows");
                    };
                    assert_eq!(results.len(), 1, "history query must return one row");
                    let expected = match name {
                        "sibling_attr" => "o".to_owned(),
                        "churned_attr_as_of" => "h0".to_owned(),
                        _ => format!("h{}", depth - 1),
                    };
                    assert!(
                        matches!(results.first().and_then(|row| row.first()), Some(minigraf::Value::String(value)) if value == &expected),
                        "history query must read the intended version"
                    );
                    // Close the handle before removing its temporary directory.
                    (db, dir)
                });
                b.iter(|| black_box(db.execute(q).unwrap()));
            });
        }
        group.finish();
    }
}
