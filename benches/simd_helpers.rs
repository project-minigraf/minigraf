//! SIMD kernel benchmarks for issue #229.
//!
//! All three functions operate on pre-extracted numeric slices.
//! `Fact` internals are `pub(crate)` and unavailable from bench code;
//! benchmarks use synthetic data to isolate the hot loop.
//!
//! `wide` API notes (verify against docs.rs/wide/0.7):
//!   - `i64x4::new([a, b, c, d])` — construct from array
//!   - `i64x4::splat(v)` — broadcast scalar to all lanes
//!   - `a.cmp_le(b)` — returns i64x4 with all-bits-1 where ≤, 0 elsewhere
//!   - `a.cmp_gt(b)` — returns i64x4 with all-bits-1 where >, 0 elsewhere
//!   - `let arr: [i64; 4] = simd_val.into()` — extract lanes
//!   - `u64x2` is the widest unsigned-64 type in wide 0.7 (no u64x4)

#![allow(clippy::cast_possible_truncation)] // synthetic bench data: N ≤ 1M fits in i64
#![allow(clippy::cast_sign_loss)]           // tx_count cast: monotonic counter, always positive

use wide::{i64x4, u64x2};

/// Count facts where `valid_from[i] <= ts && ts < valid_to[i]`.
///
/// Processes 4 facts per SIMD step using `i64x4`. Scalar tail handles remainder.
/// Both slices must be the same length.
pub fn valid_time_filter_simd(valid_from: &[i64], valid_to: &[i64], ts: i64) -> usize {
    todo!("implement in Task 4")
}

/// Count facts where `tx_counts[i] <= threshold`.
///
/// Processes 2 facts per SIMD step using `u64x2` (widest unsigned-64 in wide 0.7).
/// Scalar tail handles remainder.
pub fn as_of_filter_simd(tx_counts: &[u64], threshold: u64) -> usize {
    todo!("implement in Task 4")
}

/// Sum all values using `i64x4` horizontal reduction.
///
/// Accumulates 4 lanes in parallel; reduces to scalar at the end.
/// Wrapping arithmetic matches Rust's default release-mode overflow behaviour.
pub fn sum_simd_i64(values: &[i64]) -> i64 {
    todo!("implement in Task 4")
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
        // 9 tx_counts (non-multiple of 2 to exercise tail)
        let tx_counts: Vec<u64> = (1..=9).collect();
        let threshold = 5_u64;

        let scalar = tx_counts.iter().filter(|&&tc| tc <= threshold).count();

        assert_eq!(as_of_filter_simd(&tx_counts, threshold), scalar);
    }

    #[test]
    fn test_as_of_filter_matches_scalar_exact_chunk() {
        // 10 tx_counts (exactly five 2-wide chunks)
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
