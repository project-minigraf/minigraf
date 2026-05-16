# Design: SIMD Benchmarking (Issue #229)

**Issue**: #229 (SIMD vectorization evaluation)  
**Date**: 2026-05-16  
**Branch**: single worktree + PR  
**Prerequisite**: #208 (B+Tree selective lookup) — closed

---

## Summary

Evaluate whether explicit SIMD vectorization at the three highest-impact hot paths yields meaningful performance gains over the current scalar implementation, given Minigraf's embedded/small-dataset profile. The deliverable is benchmarks + a minimal SIMD prototype (bench-only) + a written crossover analysis and recommendation.

**What ships:**
- `benches/simd_helpers.rs` — three SIMD functions using `wide` (dev-dependency)
- `benches/minigraf_bench.rs` — three new Criterion groups at 5 dataset sizes
- `docs/simd-analysis.md` — crossover findings and final recommendation (written after running benchmarks locally)

**What does NOT ship:**
- No changes to any `src/` file
- No production SIMD code
- No feature flags

---

## Targets

Three hot paths identified after #208 (selective lookup reduces residual filter workload):

| Target | Location | Pattern |
|---|---|---|
| Valid-time range filter | `src/graph/storage.rs` `get_facts_valid_at` | `valid_from <= ts && ts < valid_to` per fact |
| Tx-time as-of filter | `src/graph/storage.rs` `get_facts_as_of` | `tx_count <= n` per fact |
| Aggregation sum | `src/query/datalog/functions.rs` `apply_builtin_aggregate` | horizontal `i64`/`f64` reduction |

---

## SIMD Helpers — `benches/simd_helpers.rs`

All three functions use `wide` (added to `[dev-dependencies]` only — no binary size impact).

> **Note for implementer**: Verify the exact `wide 0.7` API before coding — specifically `cmp_le`, `cmp_gt`, mask extraction via `Into<[T; 4]>`, and whether `u64x4` is available at that version. The pseudocode below shows intent; adapt to the actual API as needed.

### `valid_time_filter_simd(valid_from: &[i64], valid_to: &[i64], ts: i64) -> Vec<usize>`

Processes 4 facts at a time using `i64x4`. Returns indices of facts where `valid_from[i] <= ts && ts < valid_to[i]`. Scalar tail handles remainder when `len % 4 != 0`.

```rust
pub fn valid_time_filter_simd(valid_from: &[i64], valid_to: &[i64], ts: i64) -> Vec<usize> {
    use wide::i64x4;
    let ts_v = i64x4::splat(ts);
    let mut result = Vec::new();
    let chunks = valid_from.len() / 4;
    for i in 0..chunks {
        let base = i * 4;
        let vf = i64x4::new([valid_from[base], valid_from[base+1], valid_from[base+2], valid_from[base+3]]);
        let vt = i64x4::new([valid_to[base], valid_to[base+1], valid_to[base+2], valid_to[base+3]]);
        // vf <= ts && ts < vt  ↔  vf <= ts && vt > ts
        let mask_lo = vf.cmp_le(ts_v);
        let mask_hi = vt.cmp_gt(ts_v);
        let mask = mask_lo & mask_hi;
        let bits: [i64; 4] = mask.into();
        for (j, &b) in bits.iter().enumerate() {
            if b != 0 { result.push(base + j); }
        }
    }
    // scalar tail
    for i in (chunks * 4)..valid_from.len() {
        if valid_from[i] <= ts && ts < valid_to[i] { result.push(i); }
    }
    result
}
```

### `as_of_filter_simd(tx_counts: &[u64], threshold: u64) -> Vec<usize>`

Processes 4 facts at a time using `u64x4`. Returns indices where `tx_counts[i] <= threshold`.

```rust
pub fn as_of_filter_simd(tx_counts: &[u64], threshold: u64) -> Vec<usize> {
    use wide::u64x4;
    let thr_v = u64x4::splat(threshold);
    let mut result = Vec::new();
    let chunks = tx_counts.len() / 4;
    for i in 0..chunks {
        let base = i * 4;
        let tc = u64x4::new([tx_counts[base], tx_counts[base+1], tx_counts[base+2], tx_counts[base+3]]);
        let mask = tc.cmp_le(thr_v);
        let bits: [u64; 4] = mask.into();
        for (j, &b) in bits.iter().enumerate() {
            if b != 0 { result.push(base + j); }
        }
    }
    for i in (chunks * 4)..tx_counts.len() {
        if tx_counts[i] <= threshold { result.push(i); }
    }
    result
}
```

### `sum_simd_i64(values: &[i64]) -> i64`

Horizontal reduction using `i64x4` with wrapping add (matching Rust's default integer overflow behaviour in release mode). Accumulates 4 lanes in parallel, reduces at the end.

```rust
pub fn sum_simd_i64(values: &[i64]) -> i64 {
    use wide::i64x4;
    let mut acc = i64x4::splat(0);
    let chunks = values.len() / 4;
    for i in 0..chunks {
        let base = i * 4;
        let v = i64x4::new([values[base], values[base+1], values[base+2], values[base+3]]);
        acc += v;
    }
    let lanes: [i64; 4] = acc.into();
    let mut sum: i64 = lanes.iter().sum();
    for i in (chunks * 4)..values.len() {
        sum = sum.wrapping_add(values[i]);
    }
    sum
}
```

---

## Benchmark Groups — `benches/minigraf_bench.rs`

Three new groups, each with 5 dataset sizes: 100, 1K, 10K, 100K, 1M facts.

### Timing scope (critical)

- **Scalar baseline**: calls the production function directly (e.g. `storage.get_facts_valid_at(ts)`). The production function's internal extraction is measured as part of its cost.
- **SIMD variant**: field extraction from `Vec<Fact>` into numeric slices **plus** the SIMD helper, all timed together inside the measurement loop. This reflects the full cost a production SIMD path would pay — if extraction overhead makes SIMD slower, that is the correct finding.

### `bench_simd_temporal`

`helpers::populate_temporal(n)` is a new benchmark helper (added in this PR) that inserts `n` facts with staggered `valid_from`/`valid_to` windows so roughly half pass a midpoint timestamp filter — realistic selectivity. `helpers::midpoint_timestamp` returns the median `valid_from` value.

```rust
fn bench_simd_temporal(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_temporal");
    for &n in &[100usize, 1_000, 10_000, 100_000, 1_000_000] {
        let storage = helpers::populate_temporal(n); // new helper: staggered valid-time windows
        let ts = helpers::midpoint_timestamp(&storage);

        group.bench_with_input(BenchmarkId::new("scalar", n), &n, |b, _| {
            b.iter(|| storage.get_facts_valid_at(black_box(ts)))
        });

        group.bench_with_input(BenchmarkId::new("simd", n), &n, |b, _| {
            b.iter(|| {
                let facts = storage.get_all_facts();
                let vf: Vec<i64> = facts.iter().map(|f| f.valid_from).collect();
                let vt: Vec<i64> = facts.iter().map(|f| f.valid_to).collect();
                simd_helpers::valid_time_filter_simd(&vf, &vt, black_box(ts))
            })
        });
    }
    group.finish();
}
```

### `bench_simd_as_of`

Mirrors `bench_simd_temporal` but:
- Scalar: `storage.get_facts_as_of(AsOf::Counter(black_box(threshold)))`
- SIMD: extract `tx_count` field into `Vec<u64>` + `as_of_filter_simd`
- `threshold` = median `tx_count` value (50th percentile — realistic as-of query)

### `bench_simd_aggregate`

- Dataset: N facts with `Value::Integer` attributes, pre-queried to `Vec<Value>`
- Scalar: production `apply_builtin_aggregate("sum", &values)`
- SIMD: extract `i64` values from `Vec<Value>` + `sum_simd_i64`
- Both timed including extraction (same rationale as above)

---

## Analysis Document — `docs/simd-analysis.md`

Written after running benchmarks locally on the development machine. Sections:

1. **Environment**: CPU, OS, Rust toolchain, `wide` version
2. **Results table**: ns/iter (scalar vs SIMD) at each dataset size, per target
3. **Crossover point**: first dataset size where SIMD is faster, or "none observed"
4. **Post-#208 residual analysis**: typical fact counts reaching these filters after selective lookup (from `btree_lookup` benchmark data); contextualises the crossover within real workloads
5. **Recommendation**: one of:
   - **Integrate**: SIMD wins at realistic dataset sizes → follow-up issue to promote helpers into `src/`
   - **Skip**: SIMD slower or negligible at all practical sizes → close as not worth it
   - **Revisit with SoA layout**: SIMD wins only at large N but extraction overhead dominates → document struct-of-arrays as prerequisite for SIMD to pay off

---

## Cargo.toml change

```toml
[dev-dependencies]
wide = "0.7"
# ... existing dev-dependencies
```

No change to `[dependencies]` — zero binary size impact.

---

## Files Changed

| File | Change |
|---|---|
| `Cargo.toml` | Add `wide` to `[dev-dependencies]` |
| `benches/simd_helpers.rs` | New: three SIMD functions |
| `benches/minigraf_bench.rs` | Add `bench_simd_temporal`, `bench_simd_as_of`, `bench_simd_aggregate` groups |
| `docs/simd-analysis.md` | New: crossover findings and recommendation (written last) |

---

## Invariants

- **No production code changes**: all SIMD code is in `benches/`
- **No feature flags**: `wide` is a dev-dependency; it never enters the release build
- **WASM portability unaffected**: `wide` has scalar fallbacks; since it's dev-only, WASM builds are unaffected
- **Existing benchmarks unchanged**: new groups are additive only
