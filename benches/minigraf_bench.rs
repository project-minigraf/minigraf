mod helpers;

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert/single_fact");
    group.bench_function("stub", |b| b.iter(|| 1 + 1));
    group.finish();
}

criterion_group!(benches, bench_insert);
criterion_main!(benches);
