//! Final projected-tuple distinct (#776 P03 / G03), selected by nightly ^query/.
//!
//! Eight cases: 10k/100k pending rows with 1,000 unique values/tuples (90%/99%
//! duplicates). The scalar bag is the no-distinct control; pagination composes
//! distinct with descending Sort, offset and limit. Tuple keys contain floats
//! and mixed Integer/Float sets, including both signs of zero. Fixtures and
//! result checks are outside b.iter; the measured operation is execute + drop.

use criterion::{BenchmarkId, Criterion};
use minigraf::{Minigraf, QueryResult, Value, ValueSet};
use std::hint::black_box;
use std::time::Duration;

const UNIQUE: usize = 1_000;
const SCALAR_BODY: &str = ":find ?v :where [?e :v ?v]";

pub fn bench_distinct(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/distinct");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    for (label, n) in [("10k", 10_000), ("100k", 100_000)] {
        for mode in ["bag", "scalar", "sorted_page", "float_set_tuple"] {
            group.bench_with_input(BenchmarkId::new(mode, label), &n, |b, &n| {
                let tuple = mode == "float_set_tuple";
                let db = fixture(n, tuple);
                let query = match mode {
                    "bag" => format!("(query [{SCALAR_BODY}])"),
                    "scalar" => format!("(query [{SCALAR_BODY} :distinct])"),
                    "sorted_page" => format!(
                        "(query [{SCALAR_BODY} :distinct :order-by [?v :desc] :offset 100 :limit 10])"
                    ),
                    _ => "(query [:find ?f ?s :where [?e :f ?f] [?e :s ?s] :distinct])"
                        .to_owned(),
                };
                validate(&db, &query, mode, n);
                b.iter(|| black_box(db.execute(black_box(&query)).unwrap()));
            });
        }
    }
    group.finish();
}

fn fixture(n: usize, tuple: bool) -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    for start in (0..n).step_by(100) {
        let mut command = String::from("(transact [");
        for i in start..(start + 100).min(n) {
            let key = i % UNIQUE;
            if tuple {
                // Each float has two different sets, each set has 500 floats:
                // deduplicating either column alone would give a wrong count.
                let zero = if (i / UNIQUE).is_multiple_of(2) {
                    "0.0"
                } else {
                    "-0.0"
                };
                let float = if key / 2 == 0 {
                    zero.to_owned()
                } else {
                    format!("{}.0", key / 2)
                };
                command.push_str(&format!(
                    "[:e{i} :f {float}] [:e{i} :s #{{{zero} {}}}]",
                    key % 2
                ));
            } else {
                command.push_str(&format!("[:e{i} :v {key}]"));
            }
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    db
}

fn validate(db: &Minigraf, query: &str, mode: &str, n: usize) {
    let QueryResult::QueryResults { mut results, .. } = db.execute(query).unwrap() else {
        panic!("expected query results");
    };
    let mut expected: Vec<Vec<Value>> = match mode {
        "bag" => (0..n)
            .map(|i| vec![Value::Integer(i64::try_from(i % UNIQUE).unwrap())])
            .collect(),
        "sorted_page" => (890..900).rev().map(|i| vec![Value::Integer(i)]).collect(),
        "float_set_tuple" => (0..UNIQUE)
            .map(|key| {
                let float = if key / 2 == 0 { -0.0 } else { (key / 2) as f64 };
                vec![
                    Value::Float(float),
                    Value::Set(
                        ValueSet::new([
                            Value::Float(-0.0),
                            Value::Integer(i64::try_from(key % 2).unwrap()),
                        ])
                        .unwrap(),
                    ),
                ]
            })
            .collect(),
        _ => (0..UNIQUE)
            .map(|i| vec![Value::Integer(i64::try_from(i).unwrap())])
            .collect(),
    };
    if mode != "sorted_page" {
        results.sort();
        expected.sort();
    }
    assert!(results == expected, "unexpected distinct benchmark output");
    if mode == "float_set_tuple" {
        // Value equality equates signed zeros; check the canonical bits too.
        for row in &results {
            for value in row {
                match value {
                    Value::Float(f) => check_zero(*f),
                    Value::Set(set) => {
                        for element in set.iter() {
                            if let Value::Float(f) = element {
                                check_zero(*f);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn check_zero(f: f64) {
    if f == 0.0 {
        assert_eq!(f.to_bits(), (-0.0_f64).to_bits(), "canonical zero");
    }
}
