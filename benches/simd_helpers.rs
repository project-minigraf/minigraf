//! SIMD kernel benchmarks for issue #229.
//!
//! All three functions operate on pre-extracted numeric slices.
//! `Fact` internals are `pub(crate)` and unavailable from bench code;
//! benchmarks use synthetic data to isolate the hot loop.
//!
//! `wide` API notes (verified against wide 0.7.33 source):
//!   - `i64x4::new([a, b, c, d])` — construct from array
//!   - `i64x4::splat(v)` — broadcast scalar to all lanes
//!   - `a.cmp_gt(b)` — i64x4 mask: all-bits-1 (= -1) where a > b, 0 elsewhere
//!   - `a.cmp_lt(b)` — i64x4 mask: all-bits-1 where a < b, 0 elsewhere
//!   - `!mask` — bitwise NOT (flips -1↔0)
//!   - `a.move_mask()` — i32 bitmask of sign bits; non-zero lane = matched
//!   - `a.to_array()` — extract [i64; 4] lanes
//!   - `u64x4` (not u64x2) is the 4-wide unsigned-64 type; no move_mask, use to_array

#![allow(clippy::cast_possible_truncation)] // synthetic bench data: N ≤ 1M fits in i64
#![allow(clippy::cast_sign_loss)] // tx_count cast: monotonic counter, always positive

use wide::{CmpGt, i64x4, u64x4};

/// Count facts where `valid_from[i] <= ts && ts < valid_to[i]`.
///
/// Processes 4 facts per SIMD step using `i64x4`. Scalar tail handles remainder.
/// Both slices must be the same length.
pub fn valid_time_filter_simd(valid_from: &[i64], valid_to: &[i64], ts: i64) -> usize {
    let ts_v = i64x4::splat(ts);
    let mut count = 0usize;

    for (vf_chunk, vt_chunk) in valid_from.chunks_exact(4).zip(valid_to.chunks_exact(4)) {
        let [vf0, vf1, vf2, vf3] = *vf_chunk else {
            unreachable!()
        };
        let [vt0, vt1, vt2, vt3] = *vt_chunk else {
            unreachable!()
        };

        let vf = i64x4::new([vf0, vf1, vf2, vf3]);
        let vt = i64x4::new([vt0, vt1, vt2, vt3]);

        // vf <= ts  ≡  NOT (vf > ts)
        let lo = !vf.cmp_gt(ts_v);
        // ts < vt   ≡  vt > ts
        let hi = vt.cmp_gt(ts_v);
        let mask = lo & hi;

        // move_mask sets bit i if lane i has its sign bit set (i.e. value is -1)
        count += mask.move_mask().count_ones() as usize;
    }

    let rem = (valid_from.len() / 4) * 4;
    for (&vf, &vt) in valid_from
        .get(rem..)
        .unwrap_or(&[])
        .iter()
        .zip(valid_to.get(rem..).unwrap_or(&[]).iter())
    {
        if vf <= ts && ts < vt {
            count += 1;
        }
    }

    count
}

/// Count facts where `tx_counts[i] <= threshold`.
///
/// Processes 4 facts per SIMD step using `u64x4`.
/// Scalar tail handles remainder.
pub fn as_of_filter_simd(tx_counts: &[u64], threshold: u64) -> usize {
    let thr_v = u64x4::splat(threshold);
    let mut count = 0usize;

    for chunk in tx_counts.chunks_exact(4) {
        let [a, b, c, d] = *chunk else { unreachable!() };
        let v = u64x4::new([a, b, c, d]);

        // tc <= threshold  ≡  NOT (tc > threshold)
        // cmp_gt gives u64::MAX where true, 0 where false
        // !mask gives u64::MAX where tc <= threshold, 0 elsewhere
        let mask = !v.cmp_gt(thr_v);
        let arr: [u64; 4] = mask.to_array();
        count += arr.iter().filter(|&&x| x != 0).count();
    }

    let rem = (tx_counts.len() / 4) * 4;
    for &tc in tx_counts.get(rem..).unwrap_or(&[]) {
        if tc <= threshold {
            count += 1;
        }
    }

    count
}

/// Sum all values using `i64x4` horizontal reduction.
///
/// Accumulates 4 lanes in parallel; reduces to scalar at the end.
/// Wrapping arithmetic matches Rust's default release-mode overflow behaviour.
pub fn sum_simd_i64(values: &[i64]) -> i64 {
    let mut acc = i64x4::splat(0_i64);

    for chunk in values.chunks_exact(4) {
        let [a, b, c, d] = *chunk else { unreachable!() };
        acc = acc + i64x4::new([a, b, c, d]);
    }

    let lanes: [i64; 4] = acc.to_array();
    let mut sum: i64 = lanes.iter().fold(0_i64, |acc, &x| acc.wrapping_add(x));

    let rem = (values.len() / 4) * 4;
    for &v in values.get(rem..).unwrap_or(&[]) {
        sum = sum.wrapping_add(v);
    }

    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── valid_time_filter_simd ────────────────────────────────────────────────

    #[test]
    fn test_valid_time_filter_matches_scalar_partial() {
        // 6 facts (non-multiple of 4 to exercise tail path)
        // valid window: [i, i + 3]; ts = 2; facts 0,1,2 pass, facts 3,4,5 fail
        let valid_from: Vec<i64> = (0_i64..6).collect();
        let valid_to: Vec<i64> = valid_from.iter().map(|&vf| vf + 3).collect();
        let ts = 2_i64;

        let scalar = valid_from
            .iter()
            .zip(valid_to.iter())
            .filter(|&(&vf, &vt)| vf <= ts && ts < vt)
            .count();

        assert_eq!(valid_time_filter_simd(&valid_from, &valid_to, ts), scalar);
    }

    #[test]
    fn test_valid_time_filter_matches_scalar_exact_chunk() {
        // 8 facts (exactly two SIMD chunks)
        let valid_from: Vec<i64> = (0_i64..8).collect();
        let valid_to: Vec<i64> = valid_from.iter().map(|&vf| vf + 4).collect();
        let ts = 3_i64;

        let scalar = valid_from
            .iter()
            .zip(valid_to.iter())
            .filter(|&(&vf, &vt)| vf <= ts && ts < vt)
            .count();

        assert_eq!(valid_time_filter_simd(&valid_from, &valid_to, ts), scalar);
    }

    #[test]
    fn test_valid_time_filter_empty() {
        assert_eq!(valid_time_filter_simd(&[], &[], 0), 0);
    }

    // ── as_of_filter_simd ─────────────────────────────────────────────────────

    #[test]
    fn test_as_of_filter_matches_scalar_partial() {
        // 9 tx_counts (non-multiple of 4 to exercise tail)
        let tx_counts: Vec<u64> = (1..=9).collect();
        let threshold = 5_u64;

        let scalar = tx_counts.iter().filter(|&&tc| tc <= threshold).count();

        assert_eq!(as_of_filter_simd(&tx_counts, threshold), scalar);
    }

    #[test]
    fn test_as_of_filter_matches_scalar_exact_chunk() {
        // 10 tx_counts (exercises two 4-wide chunks + 2 scalar tail)
        let tx_counts: Vec<u64> = (1..=10).collect();
        let threshold = 7_u64;

        let scalar = tx_counts.iter().filter(|&&tc| tc <= threshold).count();

        assert_eq!(as_of_filter_simd(&tx_counts, threshold), scalar);
    }

    #[test]
    fn test_as_of_filter_empty() {
        assert_eq!(as_of_filter_simd(&[], 100), 0);
    }

    // ── sum_simd_i64 ──────────────────────────────────────────────────────────

    #[test]
    fn test_sum_simd_matches_scalar_partial() {
        // 9 values (non-multiple of 4 to exercise tail)
        let values: Vec<i64> = (1_i64..=9).collect();
        let scalar: i64 = values.iter().sum();

        assert_eq!(sum_simd_i64(&values), scalar);
    }

    #[test]
    fn test_sum_simd_matches_scalar_exact_chunk() {
        // 8 values (exactly two 4-wide chunks)
        let values: Vec<i64> = (1_i64..=8).collect();
        let scalar: i64 = values.iter().sum();

        assert_eq!(sum_simd_i64(&values), scalar);
    }

    #[test]
    fn test_sum_simd_empty() {
        assert_eq!(sum_simd_i64(&[]), 0);
    }

    #[test]
    fn test_sum_simd_negatives() {
        let values = vec![-3_i64, -2, -1, 0, 1, 2, 3];
        let scalar: i64 = values.iter().sum();

        assert_eq!(sum_simd_i64(&values), scalar);
    }
}
