//! #776 P13 / G17: expression-local keyword/ref equality cache controls.
//!
//! Nightly ^expr/ selects six 10k-row cases. Every query scans the same pending
//! keyword facts and projects the same entity column. Dynamic operands read
//! the keyword from data, bypassing the literal cache; they are not an exact
//! cache-off simulation (binding lookup and cloning also differ). Nested and
//! prepared literals cover cache construction beyond a plain expression.

use criterion::Criterion;
use minigraf::{BindValue, Minigraf, QueryResult, Value};
use std::hint::black_box;
use std::time::Duration;
use uuid::Uuid;

const ROWS: usize = 10_000;
const PREPARED: &str = "(query [:find ?e :where [?e :keyword ?k] [(= ?e $target)]])";

fn fixture() -> Minigraf {
    let db = Minigraf::in_memory().unwrap();
    for start in (0..ROWS).step_by(100) {
        let mut command = String::from("(transact [");
        for i in start..start + 100 {
            command.push_str(&format!("[:e{i} :keyword :e5]"));
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    db
}

fn entity(i: usize) -> Value {
    Value::Ref(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(":e{i}").as_bytes(),
    ))
}

fn validate(result: QueryResult, target: usize, unequal: bool) {
    let QueryResult::QueryResults { mut results, .. } = result else {
        panic!("expected keyword query results");
    };
    let mut expected: Vec<_> = if unequal {
        (0..ROWS)
            .filter(|&i| i != target)
            .map(|i| vec![entity(i)])
            .collect()
    } else {
        vec![vec![entity(target)]]
    };
    results.sort();
    expected.sort();
    assert!(
        results == expected,
        "unexpected keyword/ref expression rows"
    );
}

pub fn bench_keyword_equality(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr/keyword_equality");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    for (mode, expr, unequal) in [
        ("literal_eq", "(= ?e :e5)", false),
        ("dynamic_eq", "(= ?e ?k)", false),
        ("literal_neq", "(!= ?e :e5)", true),
        ("dynamic_neq", "(!= ?e ?k)", true),
        ("nested_literal_eq", "(= (= ?e :e5) true)", false),
    ] {
        group.bench_function(format!("{mode}/10k"), |b| {
            // Lazy construction: other nightly filters do no fixture work.
            let db = fixture();
            let query = format!("(query [:find ?e :where [?e :keyword ?k] [{expr}]])");
            validate(db.execute(&query).unwrap(), 5, unequal);
            b.iter(|| black_box(db.execute(black_box(&query)).unwrap()));
        });
    }

    group.bench_function("prepared_keyword_eq/10k", |b| {
        let db = fixture();
        let prepared = db.prepare(PREPARED).unwrap();
        // Each execution builds its cache after slot substitution. Alternating
        // targets verifies that a previous execution's cache cannot leak.
        for target in [5, 6, 5] {
            let binds = [(
                "target",
                BindValue::Val(Value::Keyword(format!(":e{target}"))),
            )];
            validate(prepared.execute(&binds).unwrap(), target, false);
        }
        let binds = [("target", BindValue::Val(Value::Keyword(":e5".into())))];
        b.iter(|| black_box(prepared.execute(black_box(&binds)).unwrap()));
    });
    group.finish();
}
