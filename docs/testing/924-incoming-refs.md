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
| 7 | `incoming_refs_unknown_pending_only_dangling_and_checkpoint_reopen`, `incoming_refs_external_types_and_large_read_only`, statement/promotion test |
| 8 | `incoming_refs_read_only_retryable_exhaustion_rollback_and_drop`, `incoming_refs_external_types_and_large_read_only` |
| 8a | `incoming_refs_on_failed_handle` (fault-injected STG-046 and WAL-007; borrowed and owned); `incoming_refs_failed_promotion_keeps_completed_statements` (earlier assertion and retraction), `incoming_refs_failed_large_statement_keeps_completed_statements` (every chunk-write fault ordinal), `incoming_refs_failed_end_statement_keeps_completed_statements` (claims-tree read failure) |
| 9 | `incoming_refs_wal_recovery_and_recovered_then_promoted` (actual crashing child, implicit and explicit commits), `incoming_refs_promotion_exact_set` (before/after promotion, new source/attribute and retraction after promotion, exact set before commit); checkpoint/reopen, statement/promotion, rollback/drop and model tests |
| 10 | `incoming_refs_model`: 32 proptest cases, up to 79 operations, independent exact-typed set model; asserts/retracts, aliases, commit/rollback, checkpoint/reopen and low promotion thresholds |
| 11 | `incoming_refs_exact_link_cap_overflow_zero_and_huge_limits` |
| 12 | `incoming_refs_budget_shared_across_layers`, `incoming_refs_legacy_ties_and_group_budget` |
| 13 | `incoming_refs_dead_pairs_exhaust_and_one_pair_long_history`, `incoming_refs_history_enumeration_is_bounded` |
| 14 | `incoming_refs_unrelated_growth`: 100,000 mixed unrelated facts, all four layers, first and repeated calls; independent enumeration and separate successor probes; history and legacy budget tests independently check low-budget counts; `incoming_refs_setup_work_is_counted`, `incoming_refs_pending_extend_work_is_counted`, `incoming_refs_query_snapshot_skips_target_index` |
| 15 | outcome, shared-layer budget and external large read-only tests; reduction establishes links only after all ranges finish |
| 16 | `owned_incoming_refs_cross_thread_and_guards`, existing `owned_tx_guards_begin_thread`, `owned_tx_user_function_off_the_begin_thread`, `owned_tx_other_thread_query_reads_committed` |
| 19 | `incoming_refs_read_errors_remain_errors`; full native suite, golden/model/WAL/crash/owned suites, fmt and native/browser/WASI Clippy |
| 20 | Distribution binary measurements below |
| 21, Rust half | Standalone `incoming_refs_write` benchmark below; `incoming_refs_position_memory` capacity accounting; bench-machine timing comparison pending |
| 22, in-repo half | API rustdoc, CHANGELOG, CLAUDE, TEST_COVERAGE, ROADMAP and README |

## Ref write-cost benchmarks

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

Base/branch timing medians and the >5% decision remain for the separate bench machine.

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

Total allocator/RSS measurements and write-time medians remain for the bench machine. No allocator instrumentation was added to the library.

## Verification (2026-11-28)

All Cargo commands ran offline with `CARGO_BUILD_JOBS=4`; the final test run also set `RUST_TEST_THREADS=4`. No release-mode tests or timing benchmarks were run. Both wasm targets were installed.

| Command | Result |
|---|---|
| `cargo build --offline` | Pass |
| `cargo test --offline` | **2,188 passed, 0 failed, 23 ignored** (2,211 total); 402.58 seconds |
| `cargo clippy --offline --all-targets -- -D warnings` | Pass |
| `cargo fmt --check` | Pass |
| `cargo clippy --offline --target wasm32-unknown-unknown --features browser -- -D warnings` | Pass |
| `cargo clippy --offline --target wasm32-wasip1 -- -D warnings` | Pass |
| Base-only source plus the new benchmark: `cargo check --offline --bench incoming_refs_write` | Pass |
| `cargo build --offline --profile dist` (Rust 1.94.0) | Build passes; size exceeds gate on both base and branch |
| `cargo +1.99.1 build --offline --profile dist` | Pass; size below gate |

The library suite is 1,268 passed / eight ignored. The incoming-reference integration file is 14 passed; owned transactions are 17 passed. Fourteen focused incoming-reference unit tests pass. The full run includes the existing golden-file, model, WAL, crash and transaction suites; ignored cases remain ignored. The new API rustdoc passes.

The complete `--features test-hooks` suite, wasm runtime tests, JVM bindings and docs-site builds were not run. Ordinary unit tests exercise the test-only enumeration sites.

## Distribution binary size

The workflow builds the native x86_64 Linux CLI with `cargo build --profile dist` (release's full LTO, opt-level z, one codegen unit, stripped symbols). The unchanged limit is **1,280,000 bytes (1,250 KiB)**. Base is `73388aec5`; base and branch use the same compiler and separate build directories.

| Installed compiler | Base bytes | Branch bytes | Growth | Gate |
|---|---:|---:|---:|---|
| Rust 1.94.0 (local default) | 1,284,448 | 1,288,240 | +3,792 | Both exceed |
| Rust 1.99.1 | 1,246,760 | 1,250,568 | +3,808 | Both pass |

The gate is satisfied with the installed newer toolchain. CI's exact Rust 1.100 toolchain is unavailable locally; that exact build remains unverified.
