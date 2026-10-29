# #693 — Prove format-v8 key decoders (approved design)

**Release/branch target.** The user wants this correctness proof before v3.0.0, superseding #693's earlier scheduling note. Target the current `v3` branch and complete verification before the release cut. The issue's reason for waiting was to prove decoders after format v8 had frozen, avoiding rework if `keys.rs` changed later, and to keep this refactor behind the then-current v3 release queue. A frozen format is a useful sequencing condition, not a technical prerequisite for a valid proof. Keep the patch focused so it does not delay unrelated v3 release work. If proof or release gates do not fit before the cut, move #693 back to v3.next; do not hold v3.0.0. No `main` backport: v2 lacks format-v8 `keys.rs`, and this is proof coverage rather than a v2 data-integrity/security fix.

## Purpose and philosophy

Pure, nonallocating inner decoders will make Kani round trips feasible without modeling the `anyhow` error path. This directly improves storage reliability (PHILOSOPHY.md §6). It adds no dependency or user feature and changes no encoding, file format, public API, or INT-049 error. It is a production refactor, so parity is a release condition before v3.0.0.

## Scope and contracts

Pure slice decoders for unsigned integer, signed integer, descending transaction, valid-to, and the eight-byte float payload. The scalar value helper covers Null, Boolean, Integer, Float, Keyword and Ref. String and Set, including their long forms, continue through existing `Reader` code. Packed pages and WAL are separate work. `skip_value` remains on its present path, using `Reader::int`/`uint` as today.

Each pure decoder returns `Result<(T, consumed), DecodeFailure>`; `DecodeFailure` has a small error-kind enum and a consumed count. The enum carries the reported bad byte. No `anyhow`, formatting, backtrace, allocation, or `Reader` call occurs in a pure decoder. The `Reader` adapter advances by `consumed` on success or failure, then constructs the exact current INT-049 message. `Reader::position()` after failure is therefore part of parity testing.

| Decoder | Failure | Bytes consumed before error |
|---|---|---:|
| `uint`, `int` | empty | 0 |
| `uint`, `int` | bad type or truncated magnitude | 1 |
| `int` | out of range after full magnitude | encoded length |
| `tx_desc` | empty, bad complemented type, truncated payload | 0 |
| `valid_to` | empty | 0 |
| `valid_to` | finite value error | same as `int` |
| float payload decoder alone | fewer than 8 bytes | 0 |
| scalar value | empty | 0 |
| scalar Float value | fewer than 8 payload bytes | 1 tag byte |
| scalar value | unknown tag, or missing Boolean payload | 1 |
| scalar value | Integer/Keyword/Ref failure | 1 tag byte plus inner decoder's consumed count |

`tx_desc` reports the **complemented** type byte, matching today's `!peek()` message. Its pure decoder complements bytes into a fixed `u64` rather than allocating a temporary `Vec`. Error kinds map to `key ends early`, `bad unsigned type byte 0x..`, `bad integer type byte 0x..`, `bad tx type byte 0x..`, `integer out of range`, `iid out of range`, or `bad value tag 0x..` as applicable.

The standalone float payload decoder takes eight bytes without a tag and is called by the scalar helper. The scalar helper takes the complete slice **including** its tag. It returns `NotScalar` with zero consumption for String or Set; `Reader::value` then takes its existing path starting at the same tag. Unknown tags produce `BadValueTag` with one byte consumed. A Keyword `iid out of range` error consumes the tag plus the entire successfully decoded unsigned integer. Successful scalar decoding consumes its tag and payload only. Boolean accepts any nonzero byte as true, as today.

The decoder accepts noncanonical but decodable encodings: leading-zero magnitudes, padded negative magnitudes, finite-form `i64::MAX`, arbitrary float bit patterns, and any nonzero Boolean payload. No proof claims decoder injectivity or rejection of noncanonical bytes. `put_valid_to(i64::MAX)` emits `0xFF`; `Reader::valid_to` also accepts the signed-integer form. The key float encoder maps every NaN to `0xFFF8_0000_0000_0000`; decoding those key bits yields canonical f64 bits `0x7FF8_0000_0000_0000` (related prior finding #688). Signed zero retains its bit pattern. For scalar value round trips, compare re-encoded bytes or bits as appropriate, never `NaN == NaN`.

## Verification

Kani harnesses prove `decode(encode(x) ++ suffix) == (expected x, encode(x).len())` for symbolic `i64`, `u64`, and float bit patterns (with NaN canonicalization); use a symbolic one-byte suffix to show the decoder stops at the component boundary. Add separate harnesses for each concrete scalar value variant; avoid symbolic `KeyValue` with String/Set and give loops explicit unwind bounds. The float harness must exercise the new slice decoder, beyond #627's existing `float_bits` inverse proof.

Malformed-input harnesses use symbolic slices of length 0–9 (0–10 for scalar values), assert no panic, `consumed <= len`, and the table above for every returned failure. Those bounded harnesses do **not** establish behavior for arbitrary-length input, or the `Reader`/`anyhow` error path. Existing fuzz and property tests retain that coverage.

Parity tests compare the new `Reader` methods with the old ones retained as a `#[cfg(test)]` oracle: decoded value/encoded bytes, INT-049 code, exact message, and cursor position. Cover all byte strings of length 0–2 plus deterministic random 3–10 byte strings and explicit boundary inputs (max values, overflows, NaNs, signed zero, FOREVER, noncanonical forms, suffixes). Keep the oracle limited to this test module and remove it if a better independent reference gives the same coverage. For values with NaNs compare re-encoded bytes or `to_bits`, not equality.

## CI acceptance

Prototype new Kani harnesses on an idle bench machine with Kani 0.68.0. Measure each harness's peak RSS and time, then the entire set with the existing 12 harnesses at runner-equivalent concurrency (`-j 4`) and `--harness-timeout 10m`. `kani.yml` automatically discovers every `#[kani::proof]`; therefore only commit new harnesses if each finishes well under 10 minutes and the full set fits the 16 GB runner and 30-minute job with clear margin. If it does not, do not land the production refactor alone before v3.0.0; revise the design with the measured bottleneck. Update the workflow comments to state the measured set. Run heavy tests and builds on the bench machine and clean temporary artifacts after each run.

The format-v8 binary-size gate is 1230 KiB (`.github/workflows/binary-size.yml`). Measure the release binary before and after on the bench machine with the same build profile as CI; a size failure requires simplifying the refactor, not raising the cap. Compare before/after key-decode throughput and representative scan and point-lookup Criterion cases on the idle bench machine, using the release/bench profile and repeatable inputs. Investigate a material slowdown before landing. Re-run parity tests and Kani after any later pre-release change to `keys.rs`; the proofs apply to the code revision they checked, not all future edits. The existing `kani.yml` path filter includes `src/storage/keys.rs` and `src/storage/keys/**`, so decoder-only PRs trigger the check.

## Delivery

Document proof scope and limits in `docs/TEST_COVERAGE.md`, `CLAUDE.md` (including its stale Kani size guidance), the Kani module header, the `kani.yml` header, and `CHANGELOG.md` under the v3.0.0 unreleased section. Add a follow-up note to #627's spec that its decoder exclusion is addressed by #693; preserve the original measurements as history. Cross-check the other project docs for contradictions. Move #693 to the v3.0.0 milestone when implementation begins. Commit a specification and separate implementation plan before coding. Request Claude spec and plan reviews before coding, and final code review before opening the PR.
