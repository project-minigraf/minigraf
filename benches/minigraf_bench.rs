mod helpers;

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert/single_fact");
    group.bench_with_input(BenchmarkId::from_parameter("smoke"), &10usize, |b, &n| {
        let db = helpers::populate_in_memory(n);
        b.iter(|| db.execute("(transact [[:esmoke :val 0]])").unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_insert);
criterion_main!(benches);
