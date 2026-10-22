# Kani proofs of the storage encodings — Design

**Issue:** #627
**Milestone:** v3.0.0, branch `v3`, with a backport to `main` (v2.x). Test-only: no change to the library, the `.graph` format or the WAL.
**Status:** prototyped and measured 2026-10-22 against `v3` at `9ecc02f5` (`keys.rs`, `index.rs` and `graph/types.rs` are unchanged since `d24fb585`, where the prototype ran); not merged

## 1. Decisions

1. **Kani, encoders only.** The harnesses prove the byte encoders that order index keys. Decoders, strings, packed pages and the WAL are left out, because CBMC does not finish on them at any useful bound (§5). The fuzz and property tests stay their defence.
2. **Both branches.** A failed proof on `main` would be a silent encoding bug in shipped v7 files, so `main` gets the harnesses that apply to it (§3.2). A bug a harness finds is filed as its own issue and goes through the usual known-issues process (#421); #627 tracks only the harnesses.
3. **One workflow, every relevant PR and nightly** (§4). The whole set runs in about 30 seconds of proof time, so it is cheap enough for PRs that touch the encoders.
4. **No production change.** Harnesses live in `#[cfg(kani)]` modules. The only non-test change is declaring the `kani` cfg to rustc (§3.3).

## 2. Terms

- **Harness**: a `#[kani::proof]` function. Kani turns it and everything it calls into a CBMC program; CBMC proves its assertions for every value of the inputs drawn with `kani::any()`, within the loop bound `#[kani::unwind(n)]`.
- **Proved**: CBMC reports `VERIFICATION:- SUCCESSFUL`. Kani adds an unwinding assertion to every loop, so a success also shows the bound was never reached: the proof covers every input, not only those up to a bound.
- **Order-preserving**: for all inputs `a`, `b`, `a.cmp(&b) == enc(a).cmp(&enc(b))`, where `enc` is the encoder and the right side compares bytes. This also proves the encoder injective: `enc(a) == enc(b)` only if `a == b`.

## 3. Harnesses

### 3.1 `v3`: `src/storage/keys/verification.rs`

Declared at the end of `keys.rs` as `#[cfg(kani)] mod verification;`. Uses `super::*`, so it reaches the private `float_bits`, `float_from_bits` and `INT_ZERO`.

| Harness | Property | Proof time¹ |
|---|---|---|
| `put_int_preserves_order` | `put_int` is order-preserving over all `i64` | 14 s |
| `put_uint_preserves_order` | `put_uint` is order-preserving over all `u64` | 6 s |
| `put_tx_desc_reverses_order` | `b.cmp(&a) == enc(a).cmp(&enc(b))` over all `u64`: newer transactions sort first | 13 s |
| `put_valid_to_preserves_order` | `put_valid_to` is order-preserving over all `i64`, including `VALID_TIME_FOREVER` (`i64::MAX`, the single byte 0xFF) | 26 s |
| `float_bits_preserves_order` | for non-NaN `a`, `b`: `a.total_cmp(&b) == float_bits(a).cmp(&float_bits(b))`, so `-0.0` sorts before `0.0` | 0.1 s |
| `float_bits_nan_sorts_last` | every NaN maps to `float_bits(f64::NAN)`, which is greater than `float_bits` of every non-NaN | 0.05 s |
| `float_bits_round_trips` | for non-NaN `a`: `float_from_bits(float_bits(a))` has the bits of `a` | 0.06 s |

`put_value` is covered through its parts: the type tags are constants in increasing order, and integers, floats, keywords (`put_uint`) and refs (`put_uint`) use the encoders above. Strings are not covered (§5).

### 3.2 Both branches: `src/storage/index/verification.rs`

`encode_value` is the same function on `main` and `v3`. On `v3` it keys the in-memory pending indexes and compares values by identity (net-assert grouping in `db.rs`, the log writer); on `main` it is also the `value_bytes` of the on-disk v7 indexes. Declared at the end of `index.rs` before the test module.

| Harness | Property | Proof time¹ |
|---|---|---|
| `encode_value_null_and_boolean` | `Null` is `[0x00]`; `Boolean` has type byte 0x01 and keeps `false < true` | 0.8 s |
| `encode_value_integer_preserves_order` | type byte 0x02; order-preserving over all `i64` | 1.0 s |
| `encode_value_float_preserves_order` | type byte 0x03; for non-NaN floats, order-preserving with `total_cmp` | 1.4 s |
| `encode_value_nan_is_canonical` | every NaN encodes to the same bytes | 0.8 s |
| `encode_value_ref_preserves_order` | type byte 0x06; order-preserving over all UUIDs | 1.3 s |

Distinct type bytes make values of different variants order by variant and never share an encoding, so per-variant harnesses cover the cross-variant properties. A direct harness over two arbitrary `Value`s timed out at 10 minutes: building a symbolic `Value` brings in the `String` variant and its drop glue.

¹ Bench machine, Kani 0.68.0, CBMC 6.11.0, CaDiCaL; all 12 with `-j 8`: 33 s wall, 1.1 GB peak RSS.

### 3.3 Build

- `Cargo.toml` adds `[workspace.lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }`. Without it every normal build warns `unexpected cfg condition name: kani`.
- Harnesses use `assert!`/`assert_eq!` with no message, or with plain literals (Testing Conventions in `CLAUDE.md`). They are not compiled by `cargo build`, `cargo test` or clippy.
- Running locally: `cargo install --locked kani-verifier && cargo kani setup`, then `cargo kani --lib -j`. Kani pins its own nightly toolchain.

## 4. CI: `.github/workflows/kani.yml`

- **Triggers:** `pull_request` with `paths:` `src/storage/keys.rs`, `src/storage/keys/**`, `src/storage/index.rs`, `src/storage/index/**`, `src/graph/types.rs`, `Cargo.toml`, `.github/workflows/kani.yml`; two `schedule` crons, one per branch, choosing the branch from the cron string as `grammar-verifier.yml` does; `workflow_dispatch` with a `ref` input.
- **Job:** checkout, `model-checking/kani-github-action@v1.1` with `kani-version: 0.68.0`, `args: --lib -j -Z unstable-options --harness-timeout 10m`. Kani upgrades are deliberate PRs that re-run the set. `timeout-minutes: 30`.
- **Failure on a schedule** opens or comments on an issue titled `Nightly Kani failure (<branch>)`, as `grammar-verifier.yml` does. A PR failure fails the check.
- The file lands on both branches as separate PRs (shared CI rule). Scheduled runs use the copy on `main`.

## 5. What was measured and left out

All runs on the bench machine, Kani 0.68.0, 10 to 60 minute limits.

| Target | Result |
|---|---|
| `Reader::int`, `uint`, `tx_desc`, `valid_to`, `value` round trips | timed out at 15–60 min, 13–27 GB, even for a one-byte magnitude |
| Error path alone (`Reader::new(&[]).int()`) | timed out at 10 min: building an `anyhow::Error` |
| Same, with `registry_entry`, `format_template`, `Backtrace::capture` or `CodedError::new` stubbed | timed out; stubbing does not remove `anyhow` from the path |
| Short strings (≤ 3 bytes) order and round trip, no decoder | timed out at 15 min |
| `encode_value` over two arbitrary `Value`s | timed out at 10 min (§3.2) |

For the 3-byte string order harness, CBMC's own statistics: symbolic execution 39 s, 538,545 steps, 6,054 checks after simplification, a formula of 24.4 M variables and 105 M clauses, 52–70 s per CaDiCaL call, repeated per refinement round. The formula is built quickly. Its size, from modelling `std`'s `Vec` growth, slicing and `memcmp` exactly, is the obstacle. Another SAT or SMT solver might shorten each call, but would not shrink the formula.

Making the decoders provable would need an error-free inner decoder (`Option` instead of `anyhow::Result`), a production change the issue rules out. Packed pages and WAL framing are decoders of the same kind and are left out for the same reason. A follow-up issue can revisit them, or a deductive tool, if that changes.

## 6. Finding: `encode_value` does not sort NaN last

`encode_value` writes NaN as `0x7FF8_0000_0000_0000` without the sign transform it applies to every other float, so NaN sorts between two negative subnormals, before `-0.0` and every positive float, not after `+∞` as `Value::cmp` orders it. A harness `encode_value(non-NaN) < encode_value(NaN)` fails in 1.5 s with a counterexample. `keys.rs` on `v3` is correct: `float_bits` maps NaN to `0xFFF8…` (§3.1, `float_bits_nan_sorts_last`).

Every lookup by `encode_value` bytes is an exact match (`start..=end` with the same `value_bytes`), and every other use compares for equality or sorts only to compare two sets in tests, so no result depends on where NaN sorts. On `main`, changing the encoding would reorder AVET keys in existing v7 files. So it is filed as its own low-severity issue, not data-integrity: state the NaN position in the doc comment on both branches, and leave the encoding as it is. No harness asserts the wrong order.

## 7. Delivery

1. **This spec**, PR to `v3`.
2. **`v3` PR:** §3.1, §3.2, §3.3, `kani.yml`, and docs: `docs/TEST_COVERAGE.md` (a Kani section with the harness list), `CLAUDE.md` (module list notes the `verification` modules; how to run), `CHANGELOG.md` `[Unreleased]` (test-only entry). Refs #627.
3. **`main` backport PR:** §3.2, §3.3 and `kani.yml`, with the same docs. `main` has no `keys.rs`, so the §3.1 harnesses are left out, and the PR says so. Refs #627.
4. #627 closes when the `v3` PR merges (v3 fixes close on merge); the backport refs it.

## 8. Differences from the issue

- The issue's example `a.cmp(&b) == encode_value(&a).cmp(&encode_value(&b))` over all `Value`s is false by design: `Value::cmp` orders variants String < Integer < Float < Boolean < Ref < Keyword < Null, while the encoding orders them by type byte, and `Value::cmp` treats `-0.0 == 0.0` while the encoding keeps them apart. The harnesses state the encoding's own order (§3.2).
- Index key ordering (issue target 2) on `v3` is the byte encoding in `keys.rs`, not a derived `Ord`; on `main` the keys use a derived `Ord`, which needs no proof.
- Packed pages and WAL framing (targets 3 and 4) are left out (§5).
- Harnesses run on PRs that touch the encoders as well as nightly, since they are fast (§4).
