//! #776 P06: six `lag`/`lead` cases under the nightly `^window/` filter.
//! Equal-size partitions share one fixture at each scale. Four-row peer groups
//! exercise tie resolution; the first find variable is the unique row id.

use criterion::{BenchmarkId, Criterion, Throughput};
use minigraf::{Minigraf, QueryResult, Value};
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Duration;

fn fixture(n: usize) -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    for start in (0..n).step_by(100) {
        let mut command = String::from("(transact [");
        for row in start..(start + 100).min(n) {
            command.push_str(&format!(
                "[:p06w{row} :p06/id {row}][:p06w{row} :p06/score {}][:p06w{row} :p06/part {}]",
                row / 4,
                row / (n / 100)
            ));
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    db
}

fn validate(db: &Minigraf, query: &str, n: usize, partitions: usize, funcs: &[&str]) {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected window rows");
    };
    assert_eq!(results.len(), n);
    let mut seen = vec![false; n];
    let partition_len = n / partitions;
    for row in results {
        let Value::Integer(id) = row[0] else {
            panic!("expected integer row id");
        };
        let id = usize::try_from(id).unwrap();
        assert!(!seen[id], "one result per input row");
        seen[id] = true;
        for (col, &func) in funcs.iter().enumerate() {
            let expected = match func {
                "lag" if id % partition_len == 0 => Value::Null,
                "lead" if id % partition_len == partition_len - 1 => Value::Null,
                "lag" => Value::Integer(i64::try_from(id - 1).unwrap()),
                "lead" => Value::Integer(i64::try_from(id + 1).unwrap()),
                _ => unreachable!(),
            };
            assert!(
                row[col + 1] == expected,
                "neighbor within partition, including ties"
            );
        }
    }
}

pub fn bench_window_neighbors(c: &mut Criterion) {
    let mut group = c.benchmark_group("window/neighbors");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (label, n) in [("10k", 10_000), ("100k", 100_000)] {
        let db = OnceLock::new();
        let cases: &[(&str, &[&str])] = if n == 10_000 {
            &[("lag", &["lag"]), ("lead", &["lead"])]
        } else {
            &[("lag_lead", &["lag", "lead"])]
        };
        for &(case, funcs) in cases {
            for (shape, partitions, partition_by) in
                [("one", 1, ""), ("hundred", 100, ":partition-by ?p ")]
            {
                let windows = funcs
                    .iter()
                    .map(|func| format!("({func} ?i :over ({partition_by}:order-by ?score))"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let query = format!(
                    "(query [:find ?i {windows} :where [?e :p06/id ?i] \
                     [?e :p06/score ?score] [?e :p06/part ?p]])"
                );
                group.throughput(Throughput::Elements(u64::try_from(n).unwrap()));
                group.bench_function(BenchmarkId::new(format!("{case}/{shape}"), label), |b| {
                    let db = db.get_or_init(|| fixture(n));
                    validate(db, &query, n, partitions, funcs);
                    b.iter(|| black_box(db.execute(black_box(&query)).unwrap()));
                });
            }
        }
    }
    group.finish();
}
