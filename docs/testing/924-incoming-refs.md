# #924 core incoming references

Scope: deliverable 1 of the [approved spec](../superpowers/specs/2026-11-28-924-incoming-ref-lookup-design.md), based on `73388aec5` on `v3`. JVM bindings and the docs site are separate repositories. No format, WAL, grammar, Datalog or error-code changes.

## Design

`IncomingRefLimits::new(max_links, max_records)` is constructible by downstream code. Both write transaction forms expose `incoming_refs(&mut self, target, limits)`. `IncomingRef`, `IncomingRefLimits`, `IncomingRefs` and `IncomingRefBudget` are non-exhaustive crate-root exports. `IncomingRef::new` also permits external link construction.

- Pending: `RefPositions`, a standard randomly seeded `HashMap<EntityId, Vec<usize>>`, records scalar-ref positions during `PendingStore::push` and `extend`. WAL replay uses the same insertion path. Query-only stores built by `Snapshot::with_staged` omit this unused index. A checkpoint publishes a new empty pending store together with its committed reader.
- Small staged transactions: the same position structure is maintained when an accepted statement is staged. Lookup borrows the facts directly, including in-memory transactions of unbounded size. It never calls `Staged::read_view`. Rejected statements add nothing. Successful promotion releases the staged facts and index. Promotion filters the staged vector in place with the existing last-statement mask before the writer stamps one transaction counter. It reuses the vector and retained facts without cloning. A failed staged-chunk write retains those facts and rebuilds their target positions, accounting for filtering or writer reordering; lookup uses committed/pending storage plus the completed staged statements. Only the error path rebuilds the index. Rollback/drop discard the transaction.
- Durable and large working trees: a dedicated raw VAET walk seeks the target prefix, checks only that prefix before charging, then decodes one version at a time. It uses the selected reader's dictionary, including new working-tree entities and attributes. Neither `live_keys` nor a buffered version-group reader is used. Tree-page decoding is bounded navigation; a single nonmatching successor probe is counted separately. A failed large-statement chunk restores its statement base. If finalization fails while reading the claims tree, lookup retains the completed statements’ roots and dictionary; their held pages remain allocated. Existing query readers still select the working roots. Commit remains refused with STG-046.
- Visibility: a per-lookup map retains the newest `(layer, tx_count, asserted)` per `(source, attribute)`, with tied retractions winning. Staged beats pending, which beats committed. It examines all target history rather than seeking past old versions. Only after all participating ranges finish are live links established and the links cap checked. Consequently, records exhaustion can precede links exhaustion even when some inspected assertions might eventually survive. No partial list escapes. Temporary state follows inspected distinct pairs, never a requested cap.

The test-only enumeration counter counts actual lookup advances as well as general staged copying, pending-store construction/extension, range copies and pending scans. These sites compile only for tests or `test-hooks`, independently of public `records_examined`.

The lookup does not write, allocate dictionary IDs, promote, checkpoint or take a transaction counter. Write-failure flags are not checked on this read. Lock, read and decode errors propagate. Valid time is ignored; keywords, strings, sets and rule-derived facts are excluded.

## Acceptance mapping

Names below are executable tests; combined names cover several closely related criteria.

| AC | Tests / evidence |
|---|---|
| 1 | `incoming_refs_external_types_and_large_read_only`, API rustdoc, `owned_incoming_refs_cross_thread_and_guards` |
| 2 | `incoming_refs_outcomes_are_distinct` |
| 3 | `incoming_refs_exact_types_and_attributes` (memory, pending and durable) |
| 4–5 | `incoming_refs_unique_pairs_and_any_valid_window`, `incoming_refs_legacy_ties_and_group_budget` |
| 6 | `incoming_refs_statement_order_rejected_statement_and_promotion` (memory and file), `incoming_refs_model`, `incoming_refs_promotion_filters_before_stamping` (allocation reuse, both statement orders and tied retractions after stamping), the three failed-statement tests in AC8a |
| 7 | `incoming_refs_unknown_target_with_populated_dictionary` (committed dictionary/VAET plus pending facts, small and promoted transactions, zero/large limits, unchanged main-file bytes (length on Windows), WAL bytes and counters, empty commit, reopen integrity and exact known links), `incoming_refs_unknown_pending_only_dangling_and_checkpoint_reopen`, `incoming_refs_external_types_and_large_read_only`, statement/promotion test |
| 8 | `incoming_refs_read_only_retryable_exhaustion_rollback_and_drop`, `incoming_refs_external_types_and_large_read_only` |
| 8a | `incoming_refs_on_failed_handle` (fault-injected STG-046 and WAL-007; borrowed and owned); `incoming_refs_failed_promotion_keeps_completed_statements` (earlier assertion and retraction), `incoming_refs_failed_large_statement_keeps_completed_statements` (every chunk-write fault ordinal), `incoming_refs_failed_end_statement_keeps_completed_statements` (claims-tree read failure) |
| 9 | `incoming_refs_wal_recovery_and_recovered_then_promoted` (actual crashing child, implicit and explicit commits), `incoming_refs_promotion_exact_set` (before/after promotion, new source/attribute and retraction after promotion, exact set before commit); checkpoint/reopen, statement/promotion, rollback/drop and model tests |
| 10 | `incoming_refs_model`: 32 proptest cases by default, `PROPTEST_CASES` overrides; up to 79 operations, independent exact-typed set model; asserts/retracts, aliases, commit/rollback, checkpoint/reopen and low promotion thresholds |
| 11 | `incoming_refs_exact_link_cap_overflow_zero_and_huge_limits` |
| 12 | `incoming_refs_budget_shared_across_layers`, `incoming_refs_legacy_ties_and_group_budget` |
| 13 | `incoming_refs_dead_pairs_exhaust_and_one_pair_long_history`, `incoming_refs_history_enumeration_is_bounded` |
| 14 | `incoming_refs_unrelated_growth`: 100,000 mixed unrelated facts, all four layers, first and repeated calls; independent enumeration and separate successor probes; history and legacy budget tests independently check low-budget counts; `incoming_refs_setup_work_is_counted`, `incoming_refs_pending_extend_work_is_counted`, `incoming_refs_query_snapshot_skips_target_index` |
| 15 | outcome, shared-layer budget and external large read-only tests; reduction establishes links only after all ranges finish |
| 16 | `owned_incoming_refs_cross_thread_and_guards`, existing `owned_tx_guards_begin_thread`, `owned_tx_user_function_off_the_begin_thread`, `owned_tx_other_thread_query_reads_committed` |
| 19 | `incoming_refs_read_errors_remain_errors`, `incoming_refs_default_reader_returns_int019_without_visiting` (MockReader inherits the default, INT-019, visitor never called); full native suite, golden/model/WAL/crash/owned suites, fmt and native/browser/WASI Clippy |
| 20 | Distribution binary measurements below |
| 21, Rust half | Criterion median comparison below: no regression above 5% on any of 65 IDs in either round; `incoming_refs_position_memory` capacity accounting |
| 22, in-repo half | API rustdoc, CHANGELOG, CLAUDE, TEST_COVERAGE, ROADMAP and README |

## Ref write-cost benchmarks

The comparison measures Criterion medians for base `73388aec5` versus the branch. Method: Linux x86_64, Rust 1.99.1, bench profile; base and branch ran side by side with CPU/memory pinning, swapped between the two rounds (r1, r2); the base used the same `incoming_refs_write` bench copied onto it; controls selected with the filter `^(insert|insert_file|checkpoint)/|^open/wal_replay/`.

`benches/incoming_refs_write.rs` uses only APIs present on the base. To compare, copy that file and its `[[bench]]` Cargo entry onto the base in a disposable directory. No production source from this branch is needed. Statements contain `#uuid` values, which the parser stores as scalar `Value::Ref`.

New IDs, each at `1000` and `10000` facts:

- `incoming_ref_writes/pending/{scale}`: in-memory insertion, including the pending position index.
- `incoming_ref_writes/staged/{scale}`: staging in an owned in-memory transaction.
- `incoming_ref_writes/wal_recovery/{scale}`: open and replay a retained Ref WAL.
- `incoming_ref_writes/checkpoint/{scale}`: publish a checkpoint over pending refs.
- `incoming_ref_writes/promotion/{scale}`: a second Ref statement crosses the chunk threshold; includes pending refs and earlier staged refs in promotion.

The harness uses 30 samples with flat sampling, one second of warmup and a three-second measurement target per ID (Criterion may extend this for slow operations). Command construction, fixture creation, and handle teardown are outside the timed operation. Run the same profile/toolchain on base and branch; report Criterion's **median** for each ID and percent change. A regression above 5% requires user acceptance before readiness.

```sh
CARGO_BUILD_JOBS=4 cargo bench --offline --bench incoming_refs_write -- --list
CARGO_BUILD_JOBS=4 cargo bench --offline --bench incoming_refs_write
# Existing integer controls (all existing scales):
CARGO_BUILD_JOBS=4 cargo bench --offline --bench minigraf_bench -- '^insert/'
CARGO_BUILD_JOBS=4 cargo bench --offline --bench minigraf_bench -- '^insert_file/'
CARGO_BUILD_JOBS=4 cargo bench --offline --bench minigraf_bench -- '^open/wal_replay/'
CARGO_BUILD_JOBS=4 cargo bench --offline --bench minigraf_bench -- '^checkpoint/'
```

Integer control IDs include `insert/{single_fact,batch_100,explicit_tx}/{1k,10k,100k,1m}`, their `insert_file` equivalents, `open/wal_replay/{1k,10k,100k}`, `checkpoint/{1k,10k,100k,1m}`, `checkpoint/after_1_fact/{10k,100k,1m}`, `checkpoint/after_100k_facts/{10k,100k}`, and `insert_file/chunked/{execute,retract,write_tx}/{65535,65537}`. Use `--list` on the existing harness for its authoritative inventory and any additional controls selected by these filters.

No median regression exceeds 5% on any of the 65 IDs in either round. The largest increase is **+4.01%** (`insert/explicit_tx/10k`, r1). This in-memory explicit-transaction insert control increases at all four scales in both rounds: +2.9% to +4.0% at 1k, 10k and 1m, and +0.63% / +1.01% at 100k. The largest ref-workload increase is **+2.36%** (`incoming_ref_writes/staged/1000`, r1). The largest decrease is **-7.45%** (`open/wal_replay/10k`, r2).

| Workload ID | Base r1 | Branch r1 | Change r1 | Base r2 | Branch r2 | Change r2 |
|---|---:|---:|---:|---:|---:|---:|
| `incoming_ref_writes/checkpoint/1000` | 3.294 ms | 3.283 ms | -0.32% | 3.318 ms | 3.375 ms | +1.74% |
| `incoming_ref_writes/checkpoint/10000` | 35.54 ms | 35.6 ms | +0.18% | 35.4 ms | 36.17 ms | +2.16% |
| `incoming_ref_writes/pending/1000` | 3.72 ms | 3.661 ms | -1.60% | 3.716 ms | 3.671 ms | -1.20% |
| `incoming_ref_writes/pending/10000` | 40.99 ms | 41.47 ms | +1.16% | 40.82 ms | 41.21 ms | +0.94% |
| `incoming_ref_writes/promotion/1000` | 6.981 ms | 6.893 ms | -1.26% | 6.993 ms | 6.905 ms | -1.26% |
| `incoming_ref_writes/promotion/10000` | 67.66 ms | 66.77 ms | -1.32% | 68.15 ms | 66.94 ms | -1.77% |
| `incoming_ref_writes/staged/1000` | 2.484 ms | 2.542 ms | +2.36% | 2.474 ms | 2.532 ms | +2.32% |
| `incoming_ref_writes/staged/10000` | 26.14 ms | 26.59 ms | +1.71% | 26.19 ms | 26.62 ms | +1.65% |
| `incoming_ref_writes/wal_recovery/1000` | 4.059 ms | 4.117 ms | +1.41% | 4.088 ms | 4.18 ms | +2.25% |
| `incoming_ref_writes/wal_recovery/10000` | 45.56 ms | 46.49 ms | +2.05% | 45.9 ms | 46.83 ms | +2.03% |
| `checkpoint/100k` | 377.8 ms | 378.2 ms | +0.10% | 380.3 ms | 380.7 ms | +0.09% |
| `checkpoint/10k` | 28.53 ms | 28.62 ms | +0.30% | 28.66 ms | 28.7 ms | +0.12% |
| `checkpoint/1k` | 2.843 ms | 2.765 ms | -2.74% | 2.774 ms | 2.77 ms | -0.15% |
| `checkpoint/1m` | 5.126 s | 5.079 s | -0.92% | 5.071 s | 5.103 s | +0.63% |
| `checkpoint/after_100k_facts/100k` | 1.461 s | 1.459 s | -0.14% | 1.47 s | 1.481 s | +0.78% |
| `checkpoint/after_100k_facts/10k` | 1.41 s | 1.403 s | -0.44% | 1.421 s | 1.41 s | -0.78% |
| `checkpoint/after_1_fact/100k` | 504 µs | 504.1 µs | +0.02% | 505.3 µs | 512.8 µs | +1.48% |
| `checkpoint/after_1_fact/10k` | 472.3 µs | 464.1 µs | -1.73% | 466.4 µs | 467.8 µs | +0.30% |
| `checkpoint/after_1_fact/1m` | 531.5 µs | 526 µs | -1.04% | 532.7 µs | 534.2 µs | +0.27% |
| `checkpoint/set_storage/inline_unique/10k` | 38.88 ms | 38.82 ms | -0.15% | 39.1 ms | 38.79 ms | -0.80% |
| `checkpoint/set_storage/long_repeated/10k` | 93.61 ms | 93.17 ms | -0.47% | 93.03 ms | 93.9 ms | +0.94% |
| `checkpoint/set_storage/long_unique/10k` | 112.8 ms | 112.1 ms | -0.63% | 112.2 ms | 112.3 ms | +0.02% |
| `checkpoint/storage_v3/multivalue_inline_unique/10k` | 27.1 ms | 26.84 ms | -0.97% | 27.29 ms | 26.91 ms | -1.39% |
| `checkpoint/storage_v3/multivalue_long_repeated/10k` | 38.34 ms | 37.88 ms | -1.20% | 38.24 ms | 38.24 ms | -0.01% |
| `checkpoint/storage_v3/multivalue_long_unique/10k` | 58.82 ms | 58.12 ms | -1.19% | 58.45 ms | 58.45 ms | +0.01% |
| `checkpoint/storage_v3/rebuild_damaged_avet/100k_history` | 510.3 ms | 509.3 ms | -0.20% | 512.2 ms | 513.9 ms | +0.33% |
| `checkpoint/storage_v3/verify/100k_history` | 254.8 ms | 251.6 ms | -1.23% | 254.5 ms | 253.7 ms | -0.34% |
| `insert/batch_100/100k` | 455.1 µs | 463.4 µs | +1.81% | 455.5 µs | 462.5 µs | +1.53% |
| `insert/batch_100/10k` | 412.8 µs | 409.7 µs | -0.77% | 413.6 µs | 417.5 µs | +0.94% |
| `insert/batch_100/1k` | 395.4 µs | 397.1 µs | +0.45% | 402.7 µs | 400.7 µs | -0.49% |
| `insert/batch_100/1m` | 410.7 µs | 409.6 µs | -0.27% | 414.8 µs | 413.8 µs | -0.24% |
| `insert/explicit_tx/100k` | 5.619 µs | 5.654 µs | +0.63% | 5.603 µs | 5.66 µs | +1.01% |
| `insert/explicit_tx/10k` | 4.922 µs | 5.119 µs | +4.01% | 4.973 µs | 5.166 µs | +3.88% |
| `insert/explicit_tx/1k` | 4.914 µs | 5.058 µs | +2.93% | 4.942 µs | 5.114 µs | +3.47% |
| `insert/explicit_tx/1m` | 4.861 µs | 5.044 µs | +3.77% | 4.905 µs | 5.1 µs | +3.98% |
| `insert/single_fact/100k` | 5.019 µs | 4.983 µs | -0.71% | 4.941 µs | 4.995 µs | +1.10% |
| `insert/single_fact/10k` | 4.473 µs | 4.52 µs | +1.05% | 4.516 µs | 4.552 µs | +0.79% |
| `insert/single_fact/1k` | 4.374 µs | 4.409 µs | +0.81% | 4.37 µs | 4.353 µs | -0.37% |
| `insert/single_fact/1m` | 4.243 µs | 4.35 µs | +2.52% | 4.34 µs | 4.427 µs | +2.02% |
| `insert_file/batch_100/100k` | 430.2 µs | 428.2 µs | -0.45% | 429.8 µs | 426.9 µs | -0.69% |
| `insert_file/batch_100/10k` | 423.5 µs | 424.5 µs | +0.24% | 427.3 µs | 424.7 µs | -0.60% |
| `insert_file/batch_100/1k` | 423.3 µs | 424.8 µs | +0.35% | 424.6 µs | 428.7 µs | +0.97% |
| `insert_file/batch_100/1m` | 442.5 µs | 427.4 µs | -3.42% | 427.5 µs | 426.7 µs | -0.18% |
| `insert_file/chunked/execute/65535` | 305.4 ms | 307.1 ms | +0.55% | 306.2 ms | 305.5 ms | -0.24% |
| `insert_file/chunked/execute/65537` | 343.4 ms | 339.9 ms | -1.01% | 336.3 ms | 341.9 ms | +1.66% |
| `insert_file/chunked/retract/65535` | 268.4 ms | 271.3 ms | +1.09% | 267.1 ms | 270.8 ms | +1.37% |
| `insert_file/chunked/retract/65537` | 542.3 ms | 538.5 ms | -0.70% | 535.7 ms | 544 ms | +1.54% |
| `insert_file/chunked/statement_rollback/65537` | 350.4 ms | 353.5 ms | +0.90% | 348.9 ms | 356.9 ms | +2.30% |
| `insert_file/chunked/write_tx/65535` | 346.6 ms | 345.9 ms | -0.20% | 339.1 ms | 348.5 ms | +2.75% |
| `insert_file/chunked/write_tx/65537` | 469 ms | 473.6 ms | +0.97% | 473.6 ms | 464 ms | -2.04% |
| `insert_file/explicit_tx/100k` | 7.34 µs | 7.27 µs | -0.96% | 7.081 µs | 7.167 µs | +1.21% |
| `insert_file/explicit_tx/10k` | 7.347 µs | 7.238 µs | -1.48% | 7.112 µs | 7.156 µs | +0.62% |
| `insert_file/explicit_tx/1k` | 7.385 µs | 7.232 µs | -2.07% | 7.114 µs | 7.146 µs | +0.45% |
| `insert_file/explicit_tx/1m` | 7.22 µs | 7.26 µs | +0.56% | 7.075 µs | 7.189 µs | +1.62% |
| `insert_file/set_storage/inline_unique/10k` | 68.58 ms | 69.13 ms | +0.81% | 68.78 ms | 69.44 ms | +0.96% |
| `insert_file/set_storage/long_repeated/10k` | 251.6 ms | 254.5 ms | +1.15% | 251.5 ms | 252.9 ms | +0.57% |
| `insert_file/set_storage/long_unique/10k` | 252.8 ms | 254.1 ms | +0.53% | 252.7 ms | 253.9 ms | +0.48% |
| `insert_file/set_storage/scalar/10k` | 39.51 ms | 39.98 ms | +1.20% | 39.33 ms | 40.43 ms | +2.80% |
| `insert_file/single_fact/100k` | 6.301 µs | 6.364 µs | +1.00% | 6.219 µs | 6.341 µs | +1.96% |
| `insert_file/single_fact/10k` | 6.367 µs | 6.505 µs | +2.17% | 6.376 µs | 6.462 µs | +1.35% |
| `insert_file/single_fact/1k` | 6.417 µs | 6.459 µs | +0.66% | 6.403 µs | 6.5 µs | +1.51% |
| `insert_file/single_fact/1m` | 6.293 µs | 6.384 µs | +1.46% | 6.272 µs | 6.367 µs | +1.52% |
| `open/wal_replay/100k` | 300.9 ms | 301.9 ms | +0.33% | 300 ms | 302.6 ms | +0.89% |
| `open/wal_replay/10k` | 33.41 µs | 32.85 µs | -1.68% | 34.44 µs | 31.88 µs | -7.45% |
| `open/wal_replay/1k` | 28.89 µs | 27.87 µs | -3.51% | 29.19 µs | 28.44 µs | -2.58% |

## Added pending-store memory

`incoming_refs_position_memory` measures retained capacities at 10,000 scalar-ref facts on x86_64. The same representation is used for staged facts. The pre-existing facts, duplicate-check set and EAVT/AEVT indexes are unchanged; this table is the **additional** pending-store memory introduced by #924, not total database RSS.

| Distinct targets | Added capacity bytes | Bytes per Ref fact |
|---:|---:|---:|
| 1 | 131,300 | 13.1300 |
| 16 (benchmark distribution) | 132,448 | 13.2448 |
| 10,000 (one ref per target) | 751,808 | 75.1808 |

Derivation: `size_of::<RefPositions>()` (48 bytes) + hash-table buckets × (`size_of::<(EntityId, Vec<usize>)>()` (40) + one control byte) + 16 control-tail bytes + the sum of vector capacities × 8-byte positions. Bucket count is the next power of two of the measured map capacity. This accounts for capacity slack but excludes allocator metadata/rounding; it describes the installed standard-library implementation, not a portable allocation contract. Each singleton target reserves one position rather than Vec's default four. Logical position storage is eight bytes per Ref plus per-target overhead. Non-Ref facts add no entries or heap allocation; the empty map is one fixed 48-byte field per store/transaction. `incoming_refs_non_ref_staging_keeps_batch_capacity` also verifies that staging retains the original batch reservation.

Reproduce the capacity report without running timing benchmarks:

```sh
CARGO_BUILD_JOBS=4 cargo test --offline --lib incoming_refs_position_memory -- --nocapture
```

Total allocator/RSS usage was not measured. No allocator instrumentation was added to the library.

## Verification (2026-11-28)

Full native runs on Linux x86_64 with Rust 1.99.1 include the new default-reader
test and precede the Windows-only test helper, which behaves identically on
non-Windows. Native all-target Clippy with and without `test-hooks` and fmt pass.
Coverage using the coverage-gates workflow's nightly-2026-10-01,
cargo-llvm-cov 0.9.1 and `--branch` passes every per-module gate:
`src/storage/mod.rs` has **85.82% line coverage** (84% floor) and
**93.33% branch coverage** (92% floor).

The build, WASI, Firefox, wasm Clippy, distribution-size and benchmark results
predate the later test-only changes; no production source has changed since.
Open main-file checks compare length on Windows because its mandatory lock
prevents reading through a second handle. Windows was not run.

| Command | Result |
|---|---|
| `cargo build` | Pass |
| `cargo test` | **2,190 passed, 0 failed, 23 ignored** (2,213 total) |
| `cargo test --features test-hooks` | **2,228 passed, 0 failed, 32 ignored** (2,260 total) |
| `PROPTEST_CASES=500 cargo test --test incoming_refs_test incoming_refs_model` | Pass |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `cargo clippy --all-targets --features test-hooks -- -D warnings` | Pass |
| `cargo fmt --check` | Pass |
| `cargo clippy --target wasm32-unknown-unknown --features browser -- -D warnings` | Pass |
| `cargo clippy --target wasm32-wasip1 -- -D warnings` | Pass |
| WASI tests on `wasm32-wasip1` under wasmtime 33.0.0 | **1,232 passed, 0 failed, 7 ignored** |
| `wasm-pack` headless Firefox browser tests | Pass: 95 library tests plus the integration targets |
| `cargo build --profile dist` | Pass: **1,250,600 bytes**, below the 1,280,000-byte gate; build path differs from the table's 1,250,568-byte measurement below |

The earlier base-only source check with the new benchmark, `cargo check --offline --bench incoming_refs_write`, also passed.

The library has 1,269 passed and eight ignored; incoming-reference integration
tests have 15 passed and owned transactions have 17 passed. The full runs
include the default-reader INT-019/no-visitor test, the existing golden-file,
model, WAL, crash and transaction suites, and the new API rustdoc; ignored cases
remain ignored.

Chrome and Safari browser runs and CI's Rust 1.100 size build were not run for this repository. Ordinary unit tests exercise the test-only enumeration sites.

## Distribution binary size

The workflow builds the native x86_64 Linux CLI with `cargo build --profile dist` (release's full LTO, opt-level z, one codegen unit, stripped symbols). The unchanged limit is **1,280,000 bytes (1,250 KiB)**. Base is `73388aec5`; base and branch use the same compiler and separate build directories.

| Installed compiler | Base bytes | Branch bytes | Growth | Gate |
|---|---:|---:|---:|---|
| Rust 1.94.0 (local default) | 1,284,448 | 1,288,240 | +3,792 | Both exceed |
| Rust 1.99.1 | 1,246,760 | 1,250,568 | +3,808 | Both pass |

The gate is satisfied with the installed newer toolchain. CI's exact Rust 1.100 toolchain is unavailable locally; that exact build remains unverified.
