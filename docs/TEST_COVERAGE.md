# Minigraf Test Coverage

## Coverage Summary

| | Default features | With `--features test-hooks` |
|---|---|---|
| **Active** | 2,038 | 2,073 |
| **Ignored** | 19 | 20 |
| **Total** | 2,057 | 2,093 |

Counts above are the current Cargo test inventory (active versus ignored),
including #777, #789, #781 and #794. Verification results below identify which suites ran.

**#794 verification (2026-10-30)**: one added `storage::keys` unit test checks that a set's element encoding equals its elements' encodings, ordered and deduplicated by a `BTreeSet`, over 2,000 random sets of up to about 60 elements with duplicates, signed zeros, NaNs and strings up to 300 bytes, in shuffled, sorted and reversed order. It passes with both the old stable sort and the new unstable sort. The full default suite passes (2,038 active, 19 ignored). CI-equivalent native, browser and WASI clippy pass on Rust 1.99.1, and Rust 1.89 checks all features. The release binary is 1,260,568 bytes (3,024 smaller), below the unchanged 1,263,616-byte cap.

**#781 verification (2026-10-29)**: 24 active tests and two ignored measurement
probes are added. On implementation `90cb88924`, the complete default suite
passes (2,037 active, 19 ignored), including all 202 engine tests. After the two
native-only metadata/dictionary-cache test refinements in `a8afaee12`, all 202
engine tests pass again. The final `test-hooks` run passes every non-fault case
(2,056 active, 20 ignored); the 16 unchanged fault-matrix cases passed in the
complete default run. This includes the expanded input-order plan matrix.
No frozen answers changed. Native, browser and WASI strict lint, formatting,
WASI tests and the WASI release build pass. All 14 portable ordering/proof tests
pass again on the final source under WASI. Firefox passes 63 tests, including
the new committed MemoryBackend grouping fixture.
The release binary is **1,263,592 bytes**, below the unchanged 1,263,616-byte cap.
The initial docs-site build passes all 13 versions and 368 pages with link
validation; delivery rechecks the reviewed site content at `7664c5d`.

The existing ignored `grouped_count_1m_groups` nightly test first reproduced its
obsolete expectation that sum must exceed 32 MiB. Its corrected test passes
under the actual `bench` profile: count and sum both return 1M groups under
that limit, while floating sum-distinct retains the Aggregate QRY-012 control.
The final required `cargo clippy --all-features -- -D warnings` passes. An
exploratory integration-target lint command is outside that CI gate and reports
test-only unwrap/indexing/cast violations also present on the unchanged base;
this is not a claim of clean `--all-targets` lint.

The new regressions cover complete effective grouping tuples, hidden `:with`
keys and empty count rows, argument provenance and raw historical kind proofs,
pending/staged writes, metadata, slot replacement and join orientations,
unsupported finalizers and callbacks, scalar/numeric results and dropped groups,
partial pulls, rewind/close, pinning, and holder accounting. The final dictionary
control exercises nonzero IdSpace usage while avg skips reference arguments
within one large group. An explicit test
preserves the approved resource-error policy: common output construction may
report QRY-012 before an unread later source error; aggregate-specific fallible
finalizers remain ineligible. Passing batches retain the existing uncharged
convention. The ignored probes separately measure Aggregate and IdSpace peaks
for fixed groups and growing distinct sets over prebuilt fixtures.

#777 adds 12 default-feature tests (three accumulator tests, one engine memory test and eight integration tests) and one additional `test-hooks` plan matrix. The focused 13-test run passes. Strict native/browser/WASI clippy passes, and the rebased release binary is 1,263,144 bytes against the unchanged 1,263,616-byte cap.

#774 adds one default-feature unit test (also present with `test-hooks`). On
2026-10-29, after rebasing onto #723, its full default suite passed 1,989 tests
with 17 ignored; the targeted `test-hooks` join-equality and plan-independence
suites passed 31 tests. The
full `test-hooks` total above includes the added test; that full suite was not
rerun for #774. Formatting and strict all-feature clippy passed.

**#785 patch-coverage verification (2026-10-29)**: the bench machine passed the complete CI branch-coverage run (1,973 passing, 13 ignored; this command excludes doctests) and every unchanged module floor. Overall line coverage is 95.54%. The new boundary cases cover five lines previously reported as missing or partial. CI-equivalent all-features clippy and formatting pass.

**#785 integration verification (2026-10-29)**: after merging the #693 decoder work from `v3`, the bench machine passed 76 engine tests (2 ignored), 24 key-decoder tests, the 16-test plan-independence matrix, and strict all-features clippy. Formatting passed. The release binary is 1,261,656 bytes, below the approved 1,263,616-byte cap. The totals above include both branches' four added tests and the additional #723 costing-boundary test; the earlier full-suite and proof results below remain tied to their original revisions.

**#723 verification (2026-10-29)**: full default and `test-hooks` suites passed on the bench machine before the final head-binding guard. The final revision passes the full `test-hooks` suite excluding the unchanged fault matrix (2,000 passing, 18 ignored); those 16 fault tests passed in both earlier full runs. The final non-fault default unit suite and plan-independence matrix also pass. Strict all-features clippy and formatting pass; the docs site builds 13 versions. No frozen answers were changed.

**#693 and earlier verification**: 2026-10-29 on the designated bench machine: the #693 full default suite passed all 1,983 active tests, all 29 Kani harnesses passed, and the strengthened key differential test passed again. On 2026-10-28, full default/test-hooks tests and the full branch-coverage suite passed, including golden files and migration/fault injection after sharing stable-sort code. Disabling internal reference-result cache eviction makes the targeted window regression fail. All unchanged coverage floors pass; measured overall coverage is 95.44% lines and 86.50% branches. Strict native/browser/WASI lint, WASI tests (1,182 passing, seven ignored), the WASI release build and Firefox tests (61 passing) also passed on that earlier revision; the new committed-reader window fixture executes on both portable runners. The #672 default and `test-hooks` suites passed separately on 2026-10-28 before rebasing onto #641.

#641 slice 1 adds 30 tests with default features and one more with `test-hooks`. Slice 2 adds 22 default-feature tests (21 active and one ignored committed-fixture memory diagnostic). Slice 3 adds 26 default-feature tests (25 active and one ignored committed-fixture memory diagnostic).

#183 adds bounded row-frame parser, evaluator, and integration tests; one additional plan-independence case runs with `test-hooks`, including duplicate tie keys and NaN. A sliding-sum test checks recovery after integer saturation. Grammar corpus cases cover frame spellings and rejection. A meter test checks that a zero-width min frame reserves one deque slot. An ignored bench-machine diagnostic reproduces the Criterion datasets and prints the engine meter's peak for every measured case.

#493 (`:distinct`) adds 18 tests with default features and one more with `test-hooks`.

#693 adds four default-feature key decoder tests; the same tests are available with `test-hooks` and WASI.

#306 PR 2 (`:order-by`, `:offset`, `Sort`) added 18 tests and 7 more under `test-hooks`; the ignored acceptance test now also checks top-k. #306 PR 1 (`:limit`) added 13 tests, 5 more under `test-hooks`, and one ignored acceptance test.

#551 PR 2 deleted the executor's query path, with the 73 unit tests of its internals and the oracle. Two engine tests replace shapes no other test covered, and a storage unit test covers the reads only the executor made.

Not counted above: the browser tests and the WASI run (see [Browser and WASI](#browser-and-wasi)).

Also outside those counts: three native unit tests in the separate #760 `minigraf_shuttle` build. The [bounded suite and findings](testing/deterministic-schedules.md) record 90,000 passing schedule executions and three deliberately injected faults, each replayed 10/10 times. The suite checks real storage synchronization with immutable committed-reader fixtures; it does not simulate file durability. `deterministic-schedules.yml` runs relevant v3 PRs on three native OSes plus Linux MSRV, with larger nightly/manual searches and replay artifacts. Default-feature test counts are unchanged.

**Scope**: the unreleased file-format-v8 work (#287, #371): the index-key value-collision fix, the v7→v8 migration, and the drop of formats v1–v6. Otherwise the public database API, the Datalog engine, storage and recovery paths, v2.0.0's locking and structured-error-code changes, and v2.0.1's regression test for rebuilding indexes on open after several checkpoints (#370). Tests merged from the v2.x line are marked *(v2.x)*.

**#789 verification (2026-10-29)**: the change adds 12 default-feature tests
and one additional `test-hooks` test. Before the runtime edits, all seven new
API tests failed on INT-041/043 and the distinct-membership test failed on
charging a skipped type. The implementation is reconciled with merged #777
at v3 base `0fc97bd65`: 64 aggregate unit/engine tests (one ignored) and all
15 avg/value-handling API tests pass. The full pre-rebase unit suite passed
1,167 tests, including all 16 unchanged storage fault-matrix cases. That run
and local continuation exposed six obsolete type-error assertions across
`aggregation_test.rs` and `error_handling_test.rs`; they now assert the
successful values required by #789.
On the final base, all 26 aggregation tests, eight error-handling tests and
34 focused `test-hooks` tests
pass, as does the dedicated Sum/Avg retained-heap test. Strict native/browser/WASI
clippy, formatting and WASI test compilation pass (no WASI execution).
The release binary is 1,261,736 bytes against the unchanged 1,263,616-byte cap.
The docs site builds all 13 versions and 368 pages with link validation.
The final default run (`cargo test --no-fail-fast -- --skip fault_matrix::`)
passes 1,997 tests with zero failures and 17 ignored; the 16 unchanged fault
cases passed separately as described above. This covers all 2,013 active
default tests across the two runs. The default run used the local fallback
(Rust 1.94.0) after a bench crash-test child-progress timeout under competing
I/O; the exact recorded seed passes locally with the unchanged 30-second
limit. Platform/focused checks used bench Rust 1.99.1. Full `test-hooks`
execution was not repeated; its 2,066-test inventory was verified.

## Covered Areas

- **Certified ordered aggregates** (#781; `src/query/engine/aggregate_order_tests.rs`, `order/tests.rs`, `order/arguments.rs`, `tests/input_order_test.rs`): sum/avg, scalar extrema, Integer/nil distinct sums, complete effective tuples, hidden `:with` keys, output remapping and raw-history argument domains. The original #641 count-only eligibility coverage is extended with explicit scalar/numeric answers, retained Sort, dropped groups, source-error/resource-error competition, pending/staged invalidation, optional-proof rollback, portable committed readers, and separately bounded Aggregate/IdSpace controls. Unproved tuples/arguments and registered finalizers retain full-input behavior.

- **Aggregate value handling** (#789; `tests/aggregate_value_handling_test.rs`, seven default tests and one with `test-hooks`): all value variants, numeric-only sum/distinct sum, mixed-type extrema, existing nil/nonnumeric groups versus empty queries, grouped and ungrouped queries, hidden `:with` groups and join multiplicity, prepared binds/cursors, recursive rules, transaction-time views and staged rollback, checkpoint/reopen, and default/bounded partition windows over identical populations. Three added accumulator tests check adversarial permutations, signed-zero/NaN bits, infinities, integer boundaries, the explicit total order and distinct membership prediction. Two added engine tests check retained numeric-member accounting, duplicates, QRY-012 boundaries, upstream failures and release on rewind/close. Rewritten error-precedence tests use real UDF finalizer failures under Limit/Offset; ordered-aggregate eligibility and frozen answers are unchanged.

- **Ordinary average** (#777; `tests/ordinary_avg_test.rs`, `aggregate/tests.rs`, `engine/tests.rs`): grouped and ungrouped averages, every nonnumeric value kind skipped, nil versus no matching rows, Float results, exact unsaturated totals, signed zero/subnormal underflow, infinities/NaNs, input permutations, prepared execution/cursors, batches, recursive rules, transaction reads, checkpoint/reopen and alternative plans. Default and bounded windows preserve skipped rows. Sum/avg exact-sum heap state is charged once before allocation, transferred on finish, and released on errors, rewind and close. Neighboring sums follow #789 numeric filtering; empty-min group dropping is preserved.

- **Expression keyword literals** (#774): a test-only conversion counter verifies that increasing input from 8 to 2,048 rows does not increase literal resolution work, across `=` / `!=`, both operand orders, nested expressions, bindings, negation, disjunction and rule bodies. Existing join-equality tests also cover changing prepared keyword slots across executions and unchanged keyword output, in memory, after checkpoint and after reopen. The envelope harness checks all eight new expression/control shapes on regular and irregular fixtures.

- **In-memory ancestor join ordering** (#723; four tests): a 1,999-edge tree returns three ancestors within a 128 KiB query budget, also after staged edge replacement; cursor close releases its metered state. Reordered and text-order plans return identical bags across keyword/ref edges, cycles, multi-valued edges, retractions, transaction/valid-time filters, empty and unbound queries, in memory and with checkpointed facts. A direct comparison against the original delta plan protects keyword/ref output representation in rule heads. The bounded cost count excludes writes after a captured snapshot, includes staged records, respects the cap, and excludes prefix-neighbor attributes.

- **Proven input order** (#641, first slice; `src/query/engine/order_tests.rs`, `order/tests.rs`, `tests/input_order_test.rs`): full result keys and projection slots, constants and repeated slots, integer/Boolean/float edge cases including NaNs and signed zeros, replacements, join fallback orientation and distinct canonicalization. Snapshot tests cover unrelated versus in-range pending/staged records, captured prefixes, temporal filters, prepared queries, checkpoints and large-transaction working readers including rejected statements and rollback. Counters establish early stopping, bounded raw proof reads and no reads without demand; optional proof-read errors leave the original plan. Late callback and `collect-set` errors retain their evaluation. A forced-fallback differential matrix checks identical ordered lists across plan switches. A committed `MemoryBackend` fixture runs on native, WASI and the browser. Storage tests cover every key kind/tag and raw seeks over retracted history.

- **One-group count aggregation** (#641, second slice; `src/query/engine/aggregate_order_tests.rs`, `order/tests.rs`, `engine/tests.rs`): count/count-distinct over one certified scalar group, cross-batch boundaries, unique-key Sort omission and retained descending/aggregate-first/mixed-numeric Sort, Boolean/float representatives and mixed numeric group order. Arbitrary argument kinds, nil, `:distinct`, offset/limit and probe joins match forced fallback. Tests cover conservative eligibility (extended by #781 for effective tuples and additional built-ins), later registered-finalizer/callback/collect-set errors and dropped-group precedence, pending/staged/prepared/working-reader changes and rejected statements, atomic fallback on every optional proof-read failure, bounded group state with one active group, internal DICT-cache eviction within one group spanning many probe batches, a growing distinct-group control, low budgets, allocator refusal, close/drop/rewind, and defensive `INT-032` for injected decreasing input before and after an answer batch. A committed `MemoryBackend` fixture also runs under WASI and in the browser.

- **Partition windows** (#641, third slice; `window_order_tests.rs`, `order/tests.rs`, `order_tests.rs`, `engine/tests.rs`): compatible multiple windows with independent orders, peers, frames and neighbors; scalar/mixed-number, entity, Ref and Keyword adjacency with comparator-specific fallbacks; boundaries on and across batch edges; empty/selective ranges; callback/CollectSet/aggregate-query exclusions; bounded early-limit reads and output coalescing; pending/staged/working-reader/prepared/cursor pinning; fixed versus growing partitions; separately tracked Window/IdSpace peaks; internal input and reference-result cache eviction, including a failing retention mutation; lookahead ownership, low budgets, allocator refusal, close/drop/rewind and defensive `INT-032` before and after returned batches. The committed `MemoryBackend` fixture runs on native, WASI and browser targets.

- **Bounded row frames** (`tests/window_functions_test.rs`, `src/query/datalog/aggregate/tests.rs`, #183): `:rows N preceding` parsing and errors, physical rows with tied order keys, partition reset, descending order, width zero and maximum, all five built-in aggregates against a replay oracle with numeric edge cases, UDFs through prepared queries and cursors, grouping with `:with`, distinct output, memory-limit refusal for long min/max strings, and final `:order-by` matching. A `test-hooks` case checks answer bags with clause reordering on and off.

- **Optional distinct output** (`tests/distinct_test.rs`, #493): bag defaults, whole tuples, section syntax, cross-batch and committed/pending values, chunked write transactions, prepared execute/cursors, joins/rules/disjunction, aggregates and hidden `:with` groups, window peers and partitioned row numbers, temporal filters, typed equality and canonical float/set output, memory limits (including retained set payloads) and cursor cleanup. A meter unit test verifies each cell/key/value estimate includes set elements and their strings. With `test-hooks`, checks both reorder settings and hashed/correlated subplans. An engine unit test checks counters and state release.

- **Core database behavior**: in-memory and file-backed operation, transactions, checkpoints, retractions, bi-temporal queries, recursive rules, prepared statements, aggregation, expressions, disjunction, window functions, UDFs, and magic-sets evaluation.
- **Storage correctness**: packed pages, B+tree indexes and range scans, cache behavior, file-header validation, migration (including v7→v8 index rebuild and v1–v6 rejection via `STG-028`), WAL replay, checksums, corruption handling, and fault injection.
- **Reliability and concurrency**: concurrent reads and writes, rollback behavior, same-process handle exclusion, cross-process locking, PID namespaces, NFS and container lock behavior, and SIGKILL recovery.
- **Public diagnostics**: structured parser, query, storage, WAL, API, and internal error codes; the error-code registry is checked against [`ERROR_REFERENCE.md`](ERROR_REFERENCE.md).
- **Compatibility and quality**: golden-file corpus of released file formats, cross-platform format compatibility, property-based tests, long-haul smoke coverage, a weekly soak at 10M facts with a 5-hour churn (#392), XTDB/Datomic semantic compatibility, grammar conformance, and rustdoc examples.

The #723 ancestor tests also cover empty deltas, the 128-row demand cap, the eight-records-per-demand-row cutoff, stored demand facts, repeated head variables and internally generated constant heads. These protect the fallback decisions as well as the optimized path.

## Whole-System Checks

These tests compare the database against an independent answer rather than a single fixed case.

- **Frozen answers** (#473, #544, #551)
  - The streaming engine is the only query path (#551). Until then, every query it ran in tests was also run by the executor and had to agree.
  - The answers of `query::engine::tests`, `tests/recursive_engine_test.rs` and `tests/plan_independence_test.rs` were recorded then and are checked against `tests/answers/`: one line per test, query and answer, holding the row count and a digest of the rows as a bag.
  - The model-based, property and golden-corpus suites check against `tests/reference_model/` and the stored corpus.
- **Model-based test** (`tests/model_based_test.rs`, #385)
  - Runs random sequences of transacts, retracts, write transactions, checkpoints, reopens and simulated crashes against a file.
  - After every step, compares with a reference model: the transaction counter, `verify`, full rows with windows and tx counts, and six query shapes under `:as-of N` and `:valid-at T`.
  - With `test-hooks`, runs at a chunk size of 2, so write-transaction steps cross chunks and statement bases (#550), and reruns every generated query with reordering off.
- **Property tests** (`tests/property_test.rs`, #386)
  - Write random histories (batched transacts, retracts, write transactions, checkpoints) in memory and to a file.
  - Compare random queries with a reference evaluator, on the open file and after a reopen. Queries use one to three joined patterns, `not` or `not-join`, a comparison predicate, `:as-of`, `:valid-at` or `:any-valid-time`, and `or` / `or-join` over bound variables (#546).
  - A query error fails the test.
  - With `test-hooks`, reruns every query with reordering off and with negation bodies and `or` branches forced hashed.
- **Crash-kill test** (`tests/crash_kill_test.rs`, #384)
  - Kills a child process running a seeded workload (multi-valued batches, batched retracts, five checkpoint modes) at a random point.
  - The reopened file must hold exactly the transactions the child logged as committed, or those plus the one in flight.
  - Checked through full-scan, AEVT, AVET, EAVT and `:as-of` queries and `verify`; again after a second reopen; and after a checkpoint plus reopen.
  - With `test-hooks`, a `Large` variant makes most transactions large, runs every third one as a `WriteTransaction` of three statements, and rolls back a large statement after writing pages (#550).
- **Golden-file corpus** (`tests/golden_corpus_test.rs`, #391 *(v2.x)*)
  - `tests/golden/` holds v7 files written by minigraf 2.0.3: one checkpoint, several checkpoints, indexes rebuilt on open, a crash-left WAL, an already-checkpointed WAL, same-transaction multi-values.
  - For a copy of each file, checks its manifest's CRC32, `tx_count` and query results after open, after a checkpoint and reopen, and after one more write. A second test checks that every golden file has a manifest.
  - On `v3`: each v7 file has a v8 meta page after its first open and gives the same results after reopening the migrated file. The #371 queries, skipped on v2.x, run and pass.
  - Seven v8 golden files: one checkpoint, seven copy-on-write checkpoints, a crash-left v2 WAL, an already-checkpointed v2 WAL, same-transaction multi-values, a v7 file migrated in place, and long values at the size limits.
  - Every golden file is first opened read-only: its `tx_count` and queries are checked, and neither the file nor its WAL changes (#429).
  - The `stale_wal` fixtures must have no WAL after a checkpoint (#457).
- **Plan independence** (`tests/plan_independence_test.rs`, 16 tests, `test-hooks` only; #596)
  - A matrix of queries gives the same bag of rows with clause reordering on and off (`minigraf::__reorder`), and with the engine's negation bodies and `or` branches chosen, forced hashed and forced correlated (`minigraf::__subplans`, #546).
  - Run in memory and on file after a checkpoint and a reopen, through `execute` and a cursor.
  - Shapes: joins in both text orders, cross products, keyword and ref, floats, long strings, filter and binding expressions, recursive rules and rules with `not`, `or`/`or-join` including the #250 short-circuit shapes, placed and deferred `not`/`not-join`, aggregates, `:as-of`, `:valid-at`, `:any-valid-time`, #546's negation and disjunction shapes, and eleven pseudo-attribute shapes (#594).
  - #596's example and the unification cases: an expression whose output is its input, two expressions on one variable, integer vs float, `-0.0` vs `0.0`.
  - `or` / `or-join` branches whose expressions read or bind from a variable only the outer row binds, nested and in rule bodies (#620).
  - Expressions, chains of them and a `not` after them, reading a variable only an `or` binds, in queries, with rules and in rule bodies (#625). `tests/disjunction_test.rs` repeats them without `test-hooks`.
  - Aggregate, window and mixed queries (#547).
  - Unit tests: `branch_outer_reads` (#620); the planner deferring an expression only an `or` can bind for, after the expressions placed before it and ahead of the deferred `not`s (#625); `apply_expr` (equal, different, integer vs float, NaN, `-0.0`/`0.0` with and without replacing, keyword vs ref both ways, a failing expression); `Binders::expression`; the switch (source order kept, per thread, restored on drop); and the engine planner's `Bind` modes and build side with the switch off. The engine's join matrix gains six unification shapes.
- **Fault injection of saves**
  - Crashes `save()` at every page write (cleanly and with torn pages, including the meta write) and every sync, and crashes a v7 migration at every write. Each reopen finds exactly the old or the new checkpoint, and every reachable page plus the free list covers the file exactly.
  - Twenty-five consecutive saves never write a page the previous meta references.
- **Fault matrix** (`src/fault_matrix.rs`, #390)
  - Drives a test-only fault plan through the real file backend, WAL and directory fsync.
  - Faults: an EIO; a short or torn write (1, 40 and 4,095 bytes); a full disk that keeps failing every later write; a failed fsync; and a failed fsync that also loses the writes since the last good one ("fsyncgate").
  - Each fault is injected at every write or sync call of: a transact, a transact that creates the WAL, an automatic checkpoint, `checkpoint()`, the close-time checkpoint and `rebuild_indexes()`. The database is committed over two checkpoints, with multi-valued, retracted and long values and entries pending in the WAL.
  - After each fault:
    - the handle holds the old facts or those plus the whole transaction, never part of one;
    - after a failed checkpoint, another is `STG-046`;
    - a later write succeeds or is `WAL-007`/`STG-046`;
    - a kill and reopen gives exactly the handle's facts, plus the failed transaction whole or not at all, and `verify` finds nothing;
    - a commit after recovery survives another kill.
  - Two more tests:
    - fill the disk halfway through a checkpoint: `STG-046`, reads unchanged, reopen intact;
    - fail the fsync after a checkpoint's meta write, then retry the checkpoint, crashing at each of its writes: the retry is refused and every reopen is intact (without the refusal, a reopen hits `STG-029`).
  - Unit tests check the fault plan's counting, and that a `rebuild_indexes` refused with `STG-041` leaves the handle usable.
  - Large transactions and large write transactions are covered too (see [Large transactions](#large-transactions-550)).
- **Grammar verifier** (`src/query/datalog/parser/verifier.rs`, 4 tests; #643)
  - Checks the parser against `tests/grammar/grammar.pest` on about 1.07M lexical strings and 555K structural inputs at the default bound (7.6M at `MINIGRAF_GRAMMAR_BOUND=4`).
  - Known drift by cause in `KNOWN_DRIFT`: none left (#650, #651 fixed). Semantic codes with witnesses in `SEMANTIC`.
### Kani proofs of key encoders and scalar decoders (#627, #693)

These are not `#[test]`s; run them with `cargo kani` or `.github/workflows/kani.yml`.

- 12 encoder harnesses prove, for every input, that the byte encoders ordering index keys keep their order.
- `src/storage/keys/verification.rs` (7): `put_int`, `put_uint` and `put_valid_to` (including `VALID_TIME_FOREVER`) preserve order; `put_tx_desc` reverses it; `float_bits` preserves IEEE total order on non-NaN floats (`-0.0` before `0.0`), maps every NaN to one value after all others, and round-trips through `float_from_bits`.
- `src/storage/index/verification.rs` (5): `encode_value` writes `Null` as `[0x00]`, each of `Boolean`, `Integer`, `Float` and `Ref` with its own type byte and in its own order, and every NaN to the same bytes.
- 17 decoder harnesses (#693): `decode_uint`, `decode_int`, `decode_tx_desc`, `decode_valid_to` and float payload round-trip with one arbitrary suffix byte; six separate tagged scalar value round trips (Null, Boolean, Integer, Float, Keyword, Ref); and six malformed-input harnesses that prove no panic and bounded consumption for symbolic slices up to 9 bytes (10 with the value tag). NaNs use the canonical key encoding; signed zero keeps its bits.
- All 29 harnesses passed on the bench machine with Kani 0.68.0 at `-j 4` in 1:59 warm; aggregate CBMC peak RSS was 7.6 GiB. The workflow caps concurrency at four for its 16 GiB runner and gives each harness 10 minutes.
- A test-only copy of the old `Reader` in `src/storage/keys/legacy_verification.rs` compares results, INT-049 code and message, and cursor position on every 0–2 byte input, deterministic 3–10 byte inputs and explicit boundary values, including String and Set escapes. Three other focused unit tests check suffix boundaries, tagged values and malformed cases. Before/after scalar and six-component key decoding were measured on the bench machine and recorded in the #693 PR.
- Left out: `Reader`'s `anyhow` mapping, strings, sets, packed pages, WAL and arbitrary-length malformed inputs from Kani. CBMC could not finish on `Reader`'s error path or variable-length strings (a 3-byte string order proof was a 105M-clause formula); differential, fuzz and property tests cover those paths (specs `docs/superpowers/specs/2026-10-22-kani-storage-encoding-design.md` §5 and `docs/superpowers/specs/2026-10-29-693-decoder-proofs.md`).

## Storage and File Format

### Format v8 pages and keys

- **Multi-value and index-key correctness** (`tests/multi_value_test.rs`, 10 tests; #287, #371): same-transaction multi-valued facts surviving query-time dedup, retracting several values of one attribute in one call, valid-time stints (facts differing only in valid time), maximum-value-size boundaries, and format v8 index keys (value bytes + assert/retract flag).
- **Set values** (`tests/set_values_test.rs`, 8 tests, 10 with `test-hooks`; #316): `#{...}` transacted, queried by exact value, joined on and grouped by, in memory, pending, committed, after WAL replay and reopen, in a large transaction, through the fact log and the log writer; exact retraction; the stored-size limit (`WAL-003`); literal errors (`PRS-119`–`121`); with `test-hooks`, the same answers with reordering on and off, and `#{0.0}` equal to `#{-0.0}` through a constant and a join, committed or pending. Unit tests cover `ValueSet` (dedup, `-0.0` kept, one NaN, `API-021`, `Ord`/`Hash`/`Eq` property test, serde), v8 set keys (inline and long, the 64-byte boundary, decoding, `set_bytes_bound`), long-set dedup and a set record under a string's hash, the pending encoding, `value_order`, parser errors and REPL/JSON output; the model-based test generates sets.
- **Set predicates** (`tests/set_predicates_test.rs`, 12 tests; #316): `in?`/`not-in?` per scalar type (string, integer, float, boolean, nil, keyword, ref), `any-in?`/`all-in?`/`none-in?`, filter and binding forms agreeing, every empty-set case, `-0.0`/`0.0` and NaN (through bind slots) as one element, a keyword not a ref to its entity, each wrong operand shape (filter drops, binding binds nothing; `not-in?` with a non-set too), stored operands pending, committed and reopened, `=`/`!=` between sets, `contains?` still string-only, arity `PRS-065`, the names refused by `register_predicate`. `tests/prepared_statements_test.rs` binds sets to an operand slot and a value slot across executes; an `executor.rs` unit test covers `eval_binop`'s arms.
- **`collect-set`** (`tests/collect_set_test.rs`, 16 tests; #316): the values of an attribute; a missing attribute gives `#{}` and keeps the row; one element per value (two valid windows of one triple, `0.0`/`-0.0`); retractions; `:as-of` and `:valid-at`; pending, committed, mixed (a committed fact retracted while pending) and reopened; refs and long strings from a value page; a set-valued attribute `QRY-015`; an unbound or later-bound entity `INT-018`; keyword entities (one naming none) and non-entity cells; in a rule body, `not-join` and `not` bodies and an `or-join` branch; an already-bound output variable; `QRY-012` from the `CollectSet` holder under a small `query_memory_limit`; set predicates and a prepared `any-in?` slot over the result; parse errors (`PRS-065`, `PRS-067` nested, `PRS-068`, `PRS-069`, `PRS-122`, `PRS-123` incl. bind slots); the name refused by `register_predicate`. A `parser.rs` unit test checks the AST; `tests/plan_independence_test.rs` adds seven `collect-set` queries to its matrix.
- **B+tree cursor** (v8 storage format PR 1, #374): every scan walks the tree, not the leaf `next_leaf` chain.
  - Full scans and start positions match the input.
  - `seek` from the current position lands where a fresh descent from the root would: randomized, plus within a leaf, to a sibling, across subtrees, past the end, into the gap after a leaf, and backwards as a no-op.
  - A seek to the sibling leaf reads exactly one page.
  - Reads are unchanged with every `next_leaf` zeroed.
  - A child pointer into a non-B+tree page is `STG-013`; a cycle in child pointers is an error, not a hang.
- **Page format** (PR 2, #374, #388)
  - The 24-byte page header: a flipped bit, a wrong page id and a future generation each give their own STG code; legacy page types fail before the CRC; a failed page is never cached.
  - Meta-page encoding, the free-list chain, and WAL v2's base generation.
  - Every row of the meta-selection table:
    - both slots valid;
    - a torn newest slot with the WAL's base generation (opens at the older one);
    - a WAL newer than the valid meta, and pages of the next generation on the free list or past `page_count` with no WAL: all `STG-033`, with the file unmodified;
    - an unknown feature bit: `STG-034`;
    - a pre-release v8 single header: `STG-032`.
- **Covering keys** (PR 3, #433)
  - Byte order equals logical order for every key component and value type on random tuples (negative, zero and large integers, NaN and ±0.0, strings with 0x00, 64- and 65-byte strings, FOREVER, newest-transaction-first), and every component round-trips.
  - Prefix-compressed leaves and shortest-separator internal nodes round-trip, agree with a linear scan, keep `max(left) < s ≤ min(right)`, and reject an overflow.
  - The byte-key B+tree's builds, incremental rebuilds, cursor and `seek` match their expected sets.
  - Dictionary ids are assigned once, never reused, shared between a keyword attribute and value, and deterministic.
  - A re-asserted or retracted long value writes no new value page, and a forced hash collision still finds the right value.
  - One transaction with two timestamps is `STG-037`; a missing dictionary entry is `STG-036`.
  - Committed reads by entity, entity and attribute, attribute, and in full equal an in-memory model across eight checkpoints and a reopen. Short-valued reads touch no value page.
  - A v7 file with every value type, long strings, refs and retractions migrates to exactly the same facts. A pre-covering-key v8 file is `STG-032`.
  - String values over 4,068 bytes and attributes or keywords over 1,024 bytes are `WAL-003`.
- **Copy-on-write checkpoints** (PR 4, #434)
  - Lazy free-list pops and pushes conserve every page across page boundaries, partial head pages and chain exhaustion; never hand out a page freed in the same generation; and share unread chain pages unchanged.
  - A copy-on-write insert equals a sorted merge for random batches: keys below the first and above the last, many into one leaf, root growth to depth 3, and 1 KB separators that split internal nodes.
  - The insert leaves the old tree readable and unchanged, frees exactly the replaced nodes, and writes only new pages: exactly one leaf and two internal nodes for one entry in a depth-3 tree. Leaves do not fragment under single inserts.
  - A checkpoint after 1 or 100 right-edge facts writes at most the tree-depth difference more pages at 100,000 facts than at 10,000, and at most one path per new entry per tree for random keys.
  - Four reader threads querying throughout 40 checkpoints never fail and never see the fact count go backwards.
- **Copy-on-write delete** (`src/storage/btree.rs`, 3 tests; #550)
  - `cow_update` against a `BTreeSet` model over random insert and delete batches (samples, contiguous runs that empty leaves and subtrees, a deleted key inserted again) on trees of 0 to 60,000 keys.
  - Each step is walked as `verify` walks a tree; every page of the old tree is unchanged; `freed` is exactly the old nodes dropped.
  - Deleting everything, front half first, gives root 0, and root 0 takes inserts again.
  - A missing, unsorted or repeated delete, or a delete on root 0, is `INT-049`.
- **Size**: at most 200 bytes per fact (142 measured) on 144k facts in #433's shape. The 1.15M-fact variant is ignored locally and runs in the nightly smoke workflow.
- **Dictionary cache**: cached ident and timestamp lookups return the same facts, and the cache stays bounded.

### Damage, verification and repair

- **Hardening** (PR 5)
  - Damage an index leaf, an internal node, a dictionary leaf, a value page and a free-list page in turn. Every committed read or checkpoint that reaches the damaged page fails with `STG-029`, never with wrong or fewer rows.
  - A query through `Minigraf` on a file with damaged leaves fails with `STG-029`, and the file still opens at its last checkpoint.
  - A B+tree node that points back up fails a scan with `INT-049` instead of looping (#597).
- **Integrity verification** (#373)
  - Intact files verify with no problems and an exact fact and page count.
  - `verify` reports each kind of damage with its code:
    - an EAVT, AEVT, AVET or VAET leaf missing an entry: `STG-038`;
    - keys out of order, or a page reached from two trees: `STG-039`;
    - a rotted index leaf: `STG-029`;
    - a free page still in use, or a leaked page: `STG-035`;
    - a missing transaction entry: `STG-036`;
    - an entity id whose UUID maps to another id: `STG-040`.
- **Index rebuild** (#373)
  - After each index or free-list damage, `rebuild_indexes` leaves a file that verifies clean, reads back every fact and accounts for every page.
  - The source is an index that agrees with another one, or the only intact one. Three intact indexes that all disagree, or a damaged dictionary, give `STG-041` with the file unchanged.
  - A rebuild commits pending facts, including new ids and long values.
  - A crash at every write (clean and torn) and every sync of a rebuild reopens either at the old generation with only its old damage, or at the new one with none, never losing a fact.
  - Through `Minigraf` (`tests/integrity_test.rs`): a rebuild commits WAL entries and deletes the WAL. Flipping a byte in each leaf of a file in turn is always reported, then either repaired (index leaves) or refused with `STG-041` and the file unchanged (dictionary leaves).
- **Page count past the end of the file** (#497)
  - A CRC-valid meta whose `page_count` is past the end (one page past, 2^50, `u64::MAX`; with the other slot valid or damaged; with or without a WAL), and a file cut short by one page, three pages or down to three pages, are `STG-044` at open, with the file unchanged and not grown. A `page_count` exactly at the end opens.
  - A checkpoint whose pages did not all reach the backend fails with `INT-049` before writing its meta, and the file reopens at the previous generation.
  - `verify` at a meta counting pages past the end reports `STG-044` without scanning to that count.
  - Through `Minigraf` (`tests/integrity_test.rs`): a file truncated by one or three pages, and a file whose meta was rewritten with `page_count` 2^50 and a valid CRC, are `STG-044` read-write and read-only, with no WAL created and the file length unchanged.
  - The `file_header` fuzz input from #497 is a corpus seed.
- **Short files** (#506)
  - `tests/integrity_test.rs`: a file of 1 to 4,095 bytes is `STG-045` read-write and read-only, unchanged and with no WAL, when it is a database cut below one page or foreign bytes. It opens as a new database that takes writes when it is a prefix of the first meta page or all zeros.
  - A file-backend unit test checks that `partial_page` returns the bytes past the last whole page.
  - The `corrupt_file` fuzzer no longer skips files under one page: they must fail to open or open empty, and the #506 input is a corpus seed.
  - *(v2.x)* `tests/edge_cases_test.rs` checks the same refusal and acceptance through `Minigraf::open` on a database checkpointed twice.
- **Damaged v7 fact pages** (#496)
  - A page in the counted fact range whose type byte is anything but the fact-page type (zero, `0x01`, the v2.x B+tree types, `0xFF`, a zeroed page; first and last page) is `STG-014` rather than skipped. A range derived from the first index root skips only v2.x B+tree pages.
  - Opening such a v7 file read-write or read-only fails with `STG-014` and writes nothing.
  - `tests/migration_matrix_test.rs`: the golden `v7_basic.graph` with page 1's type byte changed, or page 1 overwritten with noise, fails both opens with the file unchanged and no WAL created.
  - Both inputs are `fact_page` corpus seeds.

### Reads

- **Live reads** (#379)
  - `triple_len` ends exactly where `tx↓` begins, for every value type and index.
  - On random histories (several valid-time windows, retractions, re-assertions, several values per attribute, long values, 12 transactions per checkpoint), the committed live read for every scan kind and `:as-of` point is exactly the net-asserted set, stays equivalent with pending facts on top, and after reopen.
  - A point read on a triple asserted and retracted 2,000 times reads under a third of the pages a history read does, with or without `:as-of`.
  - `tests/live_reads_test.rs` runs the same history in a checkpointed file database and in memory: point, attribute, full-scan, join, valid-time, `:as-of` and rule queries return the same rows, before and after reopen.
- *(v2.x)* **Point queries** (`tests/point_query_history_test.rs` and storage unit tests, #323): attribute-narrowed bound-entity lookups return exactly what a full scan returns, and `net_asserted_facts` matches its previous implementation on randomized histories.
- *(v2.x)* **Attribute scans** (#381): the AEVT range covers exactly the queried attribute, including non-ASCII names such as `:丿` and prefix siblings such as `:ab` for `:a`.
- *(v2.x)* **Incremental B+tree rebuild** (#315): checkpoints that copy untouched index leaves produce exactly the entries and file checksum of a full rebuild, also over repeated checkpoints; single-fact checkpoints do not fragment the index; a damaged slot directory is rejected rather than copied.
- *(v2.x)* **More than 65,535 uncheckpointed facts** (storage and `edge_cases_test.rs` tests, #445): each resolves to its own fact through entity lookups and attribute scans, in memory and file-backed.
- *(v2.x)* **Directory sync** (unit tests, #389): creating the `.graph` file or the WAL, and deleting the WAL, fsync the parent directory after the file operation; reopening an existing file does not.

### Snapshots and generation pins

- **Snapshot atomicity** (`tests/snapshot_atomicity_test.rs`, #554, #555, #558): queries see each concurrent transaction whole and read one state for all their patterns, including while a checkpoint runs.
- **Generation pins** (#543)
  - A pinned generation's pages are never written by later saves or index rebuilds, and are reused once the pin is dropped (by page id; `storage::persistent_facts`, `storage::pins`, `storage::page`).
  - Fact logs stay correct while checkpoints, rebuilds and concurrent writers run, and see whole transactions only (`tests/fact_log_test.rs`).
  - Query and prepared-query cursors keep their answer and their pages across writes, checkpoints and index rebuilds, and the file stops growing once they close (`tests/cursor_test.rs`).
  - A snapshot leaves out facts appended after it on every scan (`graph::storage`).
  - A crash at every write point of a save with a pinned generation loses no fact and no page, and the reopened database reuses the quarantined ids (`storage::persistent_facts`).
  - The browser withholds free pages a pinned generation may read, and releases them once out of force (`storage::pins::split_released`, and a `#[wasm_bindgen_test]` with an open `BrowserDb` cursor).

### Coverage-gate tests (#396)

- Replay stops at a WAL entry whose first fact is longer than 10 MB or that claims more than 1M facts.
- A WAL writer cannot be opened in a missing directory; a failed cut of a torn WAL tail fails the open and leaves the file as it was.
- A short v7 page and a slot past the record count are `INT-024`.
- A migration backup meta of another generation or file length is not used (`STG-032`, file unchanged); a torn first meta with data in page 1 is `STG-032`, not a new file.
- `save` with nothing pending writes nothing; `into_backend` saves pending facts; `rebuild_indexes` with nothing checkpointed commits the pending facts.
- Unsorted or repeated B+tree input, and a stored key inserted with another value, are `INT-049`. A tree pointer to a free-list page is `STG-013` from `get`, a leaf walk and a copy-on-write insert.
- The read-only `MutexStorageBackend` refuses writes, syncs and close.
- A page-cache test holds two threads in the backend read so both miss the same page, and checks that both get the one cached copy. Before it, that path ran only when the scheduler happened to race the threads, and the gate flapped.

## WAL and Recovery

- **Torn WAL tail** (#513)
  - Opening a WAL writer cuts off a partial last entry (cut in the checksum, the header or the payload), or an entry with a bad checksum and everything after it, then appends where replay reaches. An intact WAL is left byte for byte.
  - An append that fails part-way makes every later append `WAL-007`. A rejected oversize fact (`WAL-003`, nothing written) does not.
  - Through `Minigraf`: the failed transaction is not applied, later `transact` and `commit` are `WAL-007`, and a `checkpoint()` makes writes work again.
  - In `tests/wal_test.rs`: a commit made after reopening a crash-left WAL with each kind of torn tail survives a second crash.
- **WAL entries of any size** (`src/wal.rs`, 3 tests, one replacing the 1,000,000-fact cap test; #686)
  - An entry of 1,000,001 facts and the entries after it replay, and reopening the writer cuts none of them off.
  - A fact count the bytes after it cannot hold (one more than a quarter of them, or `u64::MAX`) ends replay at that entry, and reopening the writer cuts the WAL off there.
  - A count the bytes could hold, whose facts are cut short at a fact's length or inside its bytes, also ends replay there.
- **Already-checkpointed WAL**
  - Reopening next to a fully checkpointed WAL does not reuse transaction numbers (#447).
  - A checkpoint with no facts to flush still removes a WAL whose entries are all already in the file (#457).
- **Close without checkpoint** (#476): closing a handle opened with `wal_checkpoint_threshold(usize::MAX)`, in both sync modes, leaves the `.graph` file byte for byte as it was and keeps the WAL, whose entries replay on the next open. A unit test closes an in-memory handle under the same setting.

## Transactions

### Large transactions (#550)

- **Statement reader** (`src/query/datalog/parser/statement.rs`, 8 tests; `tests/read_only_open_test.rs`, 1 test)
  - Every `transact`/`retract` built from up to three of 34 statement tokens (balanced or not) in 10 templates gives the same command or error through the reader as through the whole-input parser, and the same facts or error through the write path at chunk sizes 1, 2, 3 and the default. The sequences raise 24 distinct codes and reach pass 2.
  - So does a corpus of statements with errors in several phases: a later lexer error over an earlier structure error, a later structure error over an earlier fact error, phase 3 in today's check order, too-deep nesting inside a fact, and an extra element.
  - Pass 2 resumes just after the last kept fact.
  - Materialisation and window errors come with the facts, after `API-014` on a read-only database.
  - The parser's `keep` flag checks structure and holds nothing. Lexer offsets are byte offsets.
- **Large `transact` / `retract`** (`src/storage/persistent_facts/large_tx.rs`, 7 tests; `src/fault_matrix.rs`, 4 tests; `src/storage/page.rs`, 2 tests; `src/storage/cache.rs`, `src/storage/btree.rs` and `statement.rs`, 4 tests)
  - A statement of more than one chunk, run by `Minigraf::execute` on a file database at chunk sizes 1 to 4, stores the same records as the one-chunk path: repeats across chunks stored once, several values and windows of one attribute, shared long values, refs, keywords, transaction-level windows, retractions, and pending WAL entries committed with it.
  - It returns its `tx_id`, answers `:as-of`, passes `verify`, deletes the WAL and survives reopen.
  - Another window of a triple an earlier chunk wrote (`API-011`), and a value too long for a checkpoint in a later chunk (`WAL-003`), give the one-chunk path's code and message, take no `tx_count`, write no meta and leave the WAL. The next checkpoint writes a meta before deleting it, as the first checkpoint after an open with a WAL does.
  - A crash before the meta write on a new file reopens through the WAL guard, and is `STG-033` without it.
  - Cursors and fact logs opened before and between chunks keep their snapshot through later checkpoints.
  - The fault matrix injects every write and sync fault at every call of a large transaction, on a file with one and with two valid meta slots, with and without a WAL beforehand.
  - Allocator tracking and release, cache eviction, and cursor peeking have unit tests.
  - `tests/large_transaction_acceptance_test.rs` (ignored, `test-hooks` only, nightly in `.github/workflows/smoke.yml`) holds at most 65,536 materialised facts (the default chunk) for a 1M-fact `transact`, and prints peak RSS.
- **Statement bases and large write transactions** (`src/storage/persistent_facts/large_tx.rs`, 9 tests; `src/fault_matrix.rs`, 4 tests; `src/storage/page.rs` and `src/storage/btree.rs`, 2 tests; `tests/memory_limit_test.rs`, 1 test changed)
  - A `WriteTransaction` whose statements repeat, change, retract and re-assert earlier statements' facts within and across chunks, at chunk sizes 1 to 4, gives every statement the small path's outcome and stores the same records. This includes two statements that fail `API-011`, one only through a repeat of an earlier statement's fact (the claims case). Checked over one transaction and over two with committed and pending facts between them; passes `verify` and survives reopen.
  - Queries after each statement, inside the transaction at chunk sizes none and 1 to 4, return what a commit of the statements so far holds, including a staged retraction of a pending assertion and the reverse.
  - A failed statement that replaced pages of the committed generation leaves them in use (`verify` after later checkpoints).
  - A rolled-back large transaction leaves the file, generation and records as they were, and the next checkpoint writes a meta. One whose only statement failed commits nothing and takes no `tx_count`.
  - A staged window that has ended when the transaction becomes large rejects that statement with `API-019`, writes nothing, and leaves the transaction small.
  - A failed handle is refused (`STG-046`) when a transaction would become large; a write failure inside one fails its commit with `STG-046` and leaves the file as it was.
  - The fault matrix injects every fault at every call of a large `WriteTransaction` (staged statement, large statement, a failing one, then one changing earlier triples), on one and two meta slots, with and without a WAL.
  - `OpenOptions::transaction_chunk_facts` sets the chunk size (0 is 1, above 1,000,000 is 1,000,000).
  - Base id tracking and `tree_pages` have unit tests.
  - Queries inside a write transaction run on the engine and count toward `query_memory_limit`, with staged writes too.

### Valid-time windows and statement order

- **One window per fact** (#435)
  - `net_asserted_facts` keeps each triple's newest transaction only, against a per-triple model on random histories: a later window replaces an earlier one, closing an open window; a retraction in the newest transaction hides it; records with two windows in one transaction keep both, once each.
  - The on-disk key walk agrees at every `:as-of`, before and after reopen.
  - `API-011` rejects two windows of one fact in one transaction without taking a `tx_count`.
  - `tests/valid_time_window_test.rs`: replacing, closing, extending, reopening and retract-then-reassert through entity-bound and attribute queries, in memory, on a checkpointed file and after reopen; `API-011` from `transact` and from `WriteTransaction::execute`; and a later statement's window committed when two statements of a `WriteTransaction` give one fact different windows.
- **Window validation** (#436)
  - `check_valid_windows` rejects an empty window, an inverted one, and a `:valid-to` at or before the transaction time when `:valid-from` is left out, but not retractions.
  - `execute` rejects each such `transact` (both bounds, equal bounds, `:valid-to` only, a per-fact bound inverting the transaction's window) with `API-019` without writing or taking a `tx_count`, in memory, checkpointed and after reopen.
  - `tx.execute()` rejects them, and the transaction still commits its other statements.
  - `commit()` rejects a default `valid_from` that has passed the `:valid-to` by commit time.
  - `FactStorage::transact_batch` takes no `tx_count` on rejection.
- **Statement order** (#477, `tests/statement_order_test.rs`)
  - Every mix of two statements on one fact inside a `WriteTransaction` (retract then assert, assert then retract, two retracts, two windows; after a committed assertion and on an empty database): reads inside the transaction and the committed rows agree, in memory, checkpointed and after reopen.
  - Other facts of an overridden statement survive.
  - The commit is one transaction without the overridden records.
  - Two windows in one statement are `API-011` from `tx.execute()`, while the rest commits.
  - A unit test checks the commit collapse keeps each fact's newest statement, in order.

## Public API

- **Cursors** (#432, `tests/cursor_test.rs`)
  - `query()` batches partition the `execute()` answer for several batch sizes: never empty, never over `max_rows`; `max_rows == 0` still progresses; `None` stays `None`.
  - The iterator, and aggregates, rules, `:as-of` and `:any-valid-time`, match `execute()`.
  - A cursor opened before a `transact`, a `WriteTransaction` commit, a retract or a checkpoint returns the pre-write answer to the end, in memory and file-backed.
  - `transact`/`retract`/`rule` are `API-012`; bind slots are `API-010`; a `WriteTransaction` thread gets `INT-001`; rule limits give `INT-020` as from `execute()`.
  - `tests/write_tx_guard_test.rs` (#734): `checkpoint`, `verify`, `rebuild_indexes`, `prepare` and `PreparedQuery::execute`/`query` on a thread holding a `WriteTransaction` return `INT-001` (each under a 30 s timeout, so a deadlock fails instead of hanging), and the transaction then commits; outside a transaction they run.
  - `PreparedQuery::query` matches `PreparedQuery::execute`.
  - A cursor is `Send` and outlives its database handle.
- **User functions** (#180, `tests/udf_test.rs`, `tests/udf_reentry_test.rs`)
  - Every UDF test runs once through `register_*` and once through `try_register_*` with closures that never fail, and gets the same rows.
  - A `try_` predicate failing on its fifth call fails the query with `QRY-014` (name and message in the text) in `:where`, a rule body, a `not` body and an `or` branch, and is not called again; through a prepared query too. Through a cursor, the rows of the first batch come first, then the error, then `None`.
  - A `try_` aggregate failing in `init`, `step` or `finalise`, as a `:find` aggregate and as a partitioned `:over` window: `QRY-014`, and no call after the failing one.
  - A predicate, an aggregate and a rule's predicate that register a function from inside the call return (they deadlocked before; each under a 30 s timeout), and a later query sees the new function. Registering between a cursor's pulls neither blocks it nor changes its rows; an unknown predicate is `QRY-007` at open.
  - Re-entry, one test per row of spec §5's table, on a file database with committed and pending facts, as a plain read and inside a `WriteTransaction`: `execute` of a query, `query` drained, `prepare` + `execute`, `PreparedQuery::execute` and `query`, `fact_log`, `transact`, `retract`, `rule`, `begin_write` + commit, `checkpoint`, `verify`, `rebuild_indexes`, both kinds of registration both ways, and a nested query whose UDF queries again. In a plain read each runs, the outer query's rows are the database's before the call, a later query sees the write, and `verify` is clean; in a transaction each returns `INT-001` except registration.
  - Unit tests: `UdfError`'s message and `QRY-014`'s code, category and text.
- **Fact log** (#430, `tests/fact_log_test.rs`)
  - `fact_log()` returns exactly the records written (assertions and retractions, with `tx_count` and `tx_id`), in non-decreasing `tx_count` in Tx order. Checked in memory, with everything pending, half checkpointed, fully checkpointed and after reopen, in both orders, with windows of 1 to 1,000 records and batch sizes of 1 to 1,000.
  - Every layout gives identical records, including valid-time windows and long string values.
  - A transaction larger than the window is split with no loss or duplicate.
  - Attribute, attribute-prefix, entity and tx-range filters, and their combinations, match a model; unknown names match nothing.
  - Writes after open are not in the log.
  - `checkpoint()` and `rebuild_indexes()` are `API-013` while a log is open (a checkpoint with nothing to write is allowed). Automatic checkpoints from `execute()` and `WriteTransaction::commit` wait for the log. Closing, dropping or finishing the log releases the database.
  - A `WriteTransaction` thread gets `INT-001`.
  - A log is `Send` and outlives its database handle, and the deferred writes replay on the next open.
- **Read-only open** (#429, `tests/read_only_open_test.rs`)
  - A read-only handle on a file with checkpointed facts and a pending WAL returns the same query results, `tx_count` and fact log as a read-write open.
  - Queries, a cursor, a fact log, `verify`, `prepare`, rules and a UDF leave the `.graph` and WAL bytes and the `.graph` mtime unchanged, and no WAL is created.
  - `transact`, `retract`, `begin_write`, `checkpoint` and `rebuild_indexes` are `API-014` without taking a `tx_count`, also on a read-only in-memory database and through the builder path.
  - A missing file is `STG-042` and nothing is created; an empty file is an empty database and stays 0 bytes.
  - Read-only handles share a file, while a read-write one is refused with `STG-025`, both ways. A file without write permission opens read-only. Small and whole-file page caches work.
  - File-backend unit tests: a shared-mode backend refuses page writes and syncs with `INT-056`; the in-process registry counts readers and admits readers or one writer (also without kernel locks); a reader in another process shares the file with a reader here and refuses a writer here with `STG-026`.
  - Storage unit tests: a read-only open of a v7 file at every point of an interrupted migration (clean and torn writes) sees every fact and writes nothing; an empty backend stays empty.
- **Log writer** (#431, `tests/log_writer_test.rs`)
  - Copies a source's fact log (assertions, retractions, an explicit valid-time window, long strings, refs, keywords, checkpointed and pending facts) through `LogWriter`.
  - The new file has no `.partial` or WAL, verifies clean, and gives the source's `tx_count`, fact log and query results (current, `:valid-at`, `:any-valid-time`, every `:as-of`, and entity-, attribute- and value-bound lookups), read-write, read-only and after reopen (#370). It takes new writes at the source's next `tx_count`.
  - 90,000 facts over two batch commits verify clean and count correctly at `:as-of`.
  - A purged middle transaction is a hole whose `:as-of` equals the one before; trailing holes keep the counter through `advance_tx_count`. An empty log is an empty database with the counter.
  - Rejections, each leaving the writer usable and the output exact, with repeats written once:
    - `tx_count` 0, a lower or closed transaction, and a backwards advance: `API-015`;
    - a second `tx_id`: `API-016`;
    - a second window of a fact in one transaction: `API-011`;
    - an assertion and a retraction of one fact in one transaction, in either order: `API-020`;
    - an empty or inverted window: `API-019`;
    - oversize values or idents: `WAL-003`.
  - Duplicate retractions, identical repeats and the same fact in the next transaction are accepted.
  - An existing target or WAL is `STG-043`, a second writer `STG-025`, and `read_only` `API-014`.
  - A stale `.partial` is reused; dropping an unfinished writer leaves nothing; `finish` refuses a target created during the build and leaves it untouched. `LogWriter` is `Send`.
  - Unit tests: a batch commits only when a transaction opens past the batch size, never splitting one; `FileBackend::truncate` empties a file.
- **Online backup** (#398): runs the online backup from `docs/DURABILITY.md`. It copies a read-write handle with an uncheckpointed transaction through `fact_log` and `LogWriter`, while another thread writes past the automatic checkpoint threshold. The copy verifies clean and holds exactly the records the log returned and the counter read after the log opened; every concurrent write reaches the source's file once the log is dropped.
- **Prepared time travel** (#603, `tests/prepared_statements_test.rs`): literal `:as-of` (counter and timestamp), `:valid-at`, `:any-valid-time` and a per-fact pseudo-attribute query through `execute`, a prepared `execute` and a prepared cursor must agree. A `prepared.rs` unit test checks that `substitute` keeps non-slot `:as-of` and `:valid-at`.
- **Bind mistakes and entity bind values** (#181, `tests/prepared_statements_test.rs`): a missing bind value is `API-022` in every slot position (entity, value, expression, rule argument, `:as-of`, `:valid-at`, inside `not`, `not-join`, `or`, `or-join`), an attribute slot is `API-023` from `prepare` at the top level and in each clause kind, and a bind value of the wrong kind is `API-024` for every slot kind, naming the slot and position, through both `execute` and `query`. `Entity` and `Val(Ref)` give the same rows in entity, value, rule-argument and expression slots.
- *(v2.x)* **Bind tokens outside prepared queries** (#407): `execute()` and `WriteTransaction::execute()` reject `$slot` bind tokens in every position with `API-010`.
- **Binding constructors**: a unit test in `error.rs` checks that `MinigrafError::invalid_argument` and `MinigrafError::closed` give `API-017` and `API-018` with their text.
- *(v2.x)* **REPL** (#415): the `run()` test runs in a child process with null stdin, so it no longer hangs when the test runner's stdin is an open non-TTY stream.

## Query Engine

- **Batched projected UUIDs** (#530; 11 tests): DICT scalar agreement across dense multi-chunk and sparse/small batches, empty/invalid input, missing/malformed requested entries and unrelated malformed values, callback failure, wrong pages and cycles; a zero-cache reader guard compares results and reads at 63/64 ids and spans of `2c`/`2c+1`; committed/pending/repeated projection and duplicate columns; partial speculative fallback, memory refusal and scalar error order; unused entity/count queries inspected before cache release; a sorted-buffer model checks ascending, descending, repeated and shuffled ids across capacity boundaries.

- **Engine paths** (`query::engine::tests`, #473, #544)
  - Reaches every index choice; net-assert of pending records over committed ones; `:as-of`; valid time; long strings and reads of several batches, in memory, across checkpoints and after reopen.
  - Expression filters and bindings, including rebinding and failing expressions; pseudo-attribute columns; keyword/UUID/float constants against keywords, refs and UUID-named keywords.
  - Every query runs on the engine, and `INT-026` and `QRY-007` come first.
  - Resumed `live_keys` reads equal one live read over random histories (`storage::persistent_facts`).
  - DICT lookups are cached for one pull only, and provisional ids keep their values across pulls (`query::engine::ids`).
- **Lazy cursors** (`cursor::tests`, #544 PR 3)
  - One pull reads one internal batch of the scan (operator counters); rows left in a batch need no pull; a closed stream reads nothing more; a filter's counters chain.
  - A cursor's answer is unchanged by writes and by checkpoints that free the leaves it has yet to read (file-backed), or by later writes (in memory).
  - A read error after some rows are gathered comes on the next call, then the cursor ends (`query::engine::tests`).
- **Joins** (#545 PR 2)
  - Two- and three-pattern joins on the entity (`IndexNestedLoopJoin`), value and attribute positions (`HashJoin`), and cross products, at every temporal setting.
  - Keywords and refs of one entity in both text orders, UUID-named keywords, attributes that name entities, floats `0.0`/`-0.0`, long strings committed and pending, and expressions between joins.
  - Joins whose answers span many batches: one probe with more than a batch of matches, cross products, tables built on either side.
  - The planner's join choice, and a keyword's entity id (`query::engine::ids`).
- **Pseudo-attributes across joins** (#545 PR 3)
  - They read the metadata of the last pattern before them that matched their entity, chosen per row: a pattern on another entity variable that equals theirs on some rows only, and constant entities.
  - Pseudo-attribute patterns planned before any pattern with a stored attribute are joined with every fact of their entity, including cross products.
  - `:db/valid-at` at every temporal setting with variables, `nil` and integer constants.
- **The engine runs every query** (#551; `src/query/engine/tests.rs`, 6 tests and 2 changed; `src/query/datalog/executor.rs`, 2 tests; `src/storage/persistent_facts.rs`, 2 tests and 1 extended; `src/graph/storage.rs`, 1 test; `tests/plan_independence_test.rs`, 1 test; `tests/disjunction_test.rs`, 1 changed; `examples/perf_envelope.rs`, 1 test)
  - `:as-of` timestamps on `LogWriter` files whose `tx_id`s rise, and whose `tx_id`s fall and rise: before, between, at and after each transaction, with committed, pending and staged facts, plain and rule queries.
  - `Snapshot::tx_bound` keeps the versions the executor's `tx_id <= t` filter kept, and is a counter bound exactly when `tx_id` rises with `tx_count`.
  - The DICT's `tx_order`, `last_tx_at` (gaps, before the first and after the last transaction) and `tx_runs` against a scan.
  - `live_keys` under range bounds equals net-assert over the kept records on random histories.
  - `PendingStore::first_irregular` under `push`, `extend` and `of`.
  - Nested `or`/`or-join` in branches (one reading imports the branch cannot hash) under every subplan form; expressions and `not`s with no pattern first; `not` bodies with rule invocations; magic-set head constants and constants no cell equals.
  - Every planner error's code, and the error order.
  - The two answer changes of the spec (§4), and an unknown predicate in a `not` body, were checked against both the engine and the executor; their tests keep the engine's answer.
  - The envelope harness's `-irregular` fixture keeps its source's records, with `tx_id`s that fall and rise (`cargo test` runs the example's tests).
- **`:limit`, `:offset` and `:order-by`** (#306)
  - `tests/limit_order_test.rs` (5 tests, `test-hooks`): 14 shapes (patterns, joins, `not`, `or`, grouped and ungrouped aggregates and the zero row, a window, rule queries, `:as-of`, `:any-valid-time`, an expression with no pattern), in memory and over committed and pending facts, with limits 0, 1, 2, the answer size less one, the answer size and five more: through `execute`, a cursor pulled one row at a time, a prepared `:limit $n` and its cursor, with reordering on and off, each returns `min(n, |answer|)` rows that are a sub-bag of the unlimited answer. A write transaction's own staged facts; slot errors (`API-010`, `PRS-115`, `API-024`).
  - `src/query/engine/limit.rs` (4 unit tests): at most `limit` rows across batch boundaries, `limit 0` pulls nothing, one batch read for 10 rows and two for `BATCH_ROWS + 1`, offsets across batches and past the end, `rewind`.
  - `src/cursor.rs` (2 tests): through a cursor, in memory and on file, `:limit 10` leaves the scan at one internal batch and `:limit 0` at none.
  - Parser (3 unit tests), prepared slots (3 unit tests), `prs_115_…` in `tests/error_codes_prs_test.rs`, the grammar corpus (3 valid, 2 syntax, 2 semantic files) and two verifier `SEMANTIC` witnesses (`INT-012`, `PRS-115`).
  - `tests/limit_order_test.rs` (7 more tests, `test-hooks`): 14 ordered shapes (each `:limit` shape with an `:order-by` over plain, aggregate and window elements, ascending and descending, with ties), in memory and over committed and pending facts, with no option, limits 0, 1, 3 and the answer size, each with offsets 1, the answer size less one and two past it, and `:offset 2` alone: through `execute` and a cursor pulled one row at a time, and the limited ones through a prepared `:offset $o :limit $n` and its cursor, with reordering on and off, each returns the test's own full sort of the unlimited answer, sliced. Pages of three cover the ordered answer; every value class in one column ascending and descending (an integer before an equal float, `-0.0` before `0.0`); a write transaction's staged facts; `:offset` slot errors; a full sort over `query_memory_limit` is `QRY-012`, top-k under the same limit is not.
  - `src/query/engine/sort.rs` (7 unit tests): a full sort and top-k (k = 1 to past the input) equal a reference sort, ascending and descending, ties to the next column; `keep 0` pulls nothing; top-k's peak meter bytes are the same for 1,000 and 100,001 rows; `QRY-012` with and without `keep`; `rewind`. `src/query/engine/tests.rs` (1 test): top-k over 20,000 committed entities runs under a 512 KiB limit a full sort exceeds (the DICT lookups are dropped per batch).
  - `src/query/datalog/aggregate/tests.rs` (2 tests, `row_order`), parser (4 unit tests: entries, directions, section orders, every `PRS-116`–`118`, `INT-012`/`013` and `PRS-115` case of `:offset`/`:order-by`), prepared `:offset` slots (1 unit test), `prs_116_…`–`prs_118_…` in `tests/error_codes_prs_test.rs`, the grammar corpus (4 valid, 3 syntax, 4 semantic fixtures added by #306) and three verifier `SEMANTIC` witnesses (`PRS-116`–`118`); the verifier now has 40 tokens and 17 templates.
- **Query memory limit** (`tests/memory_limit_test.rs`, 7 tests; #549)
  - `OpenOptions::query_memory_limit` is off by default and set by its builder.
  - `QRY-012` through `execute`, a cursor's first pull, a prepared query and a rule query (from `execute`, and from `query` when the rules run at open). The database answers after a query fails.
  - A write transaction's query is counted until it stages a write, then runs uncounted.
  - A prepared rule query uses the database's `max_derived_facts`.
  - Engine unit tests (`src/query/engine/tests.rs`, 7 more):
    - `HashJoin`, `AntiJoin`, `Union`, `Aggregate` and `Window` charge while they hold state, and the meter is back at 0 after a drain and after an early drop;
    - `QRY-012` names the holder over the limit (each operator, `IdSpace`, `Recursive`, `Distinct` and `Relations`), and its allocator form through the `try_reserve` test hook;
    - a cursor returns rows, then `QRY-012`, then nothing; `rewind` and `close` release;
    - the planner builds a keyless join on the side with the smaller product estimate (#575's shape, a selective outer side, a tie, reordering off).
  - Five engine unit tests (#765) cover the exact limit while partition indices overlap input and prior window results, allocator refusal and charge release on errors, empty input, and unchanged row-number, rank, lag and lead over tied rows and interleaved float partitions.
  - `meter.rs` (4 unit tests) checks charge, release, drop, peak and the allocator form.
  - `Aggregate` charges its sorted map of groups until every row is built, and lets its hash table and group rows go first, with `sum-distinct` members charged once (`aggregate_finish_charges_its_sorted_groups`, `aggregate_finish_keeps_sum_distinct_members_charged_once`).
- **Join rules** (`tests/join_rules_test.rs`, 15 tests; #545, #744)
  - A join on a float value pairs `0.0` with `-0.0`, through the hash key and as a shared variable. A unit test checks that `-0.0` and `0.0` hash alike.
  - A float constant in a pattern, bound to a `$slot`, in a rule body or in an `=` predicate matches equal floats only (`1e-17` does not match `0.0`; `0.0` matches `-0.0`; #706), in memory, on file before and after a checkpoint and after reopen.
  - A variable bound to a keyword by one pattern and to the ref of the same entity by another returns the kind from the first pattern in `:where` text order: when the planner runs the patterns in another order, through the attribute position, before and after a rule call, and with a repeated pattern.
  - An expression binding unifies with a pattern binding the same variable; the first binder in the text, pattern or expression, gives the value (#596).
  - An expression's `=` and `!=` equate a keyword with a ref to the entity it names, as the pattern constant and the join do: either operand order, a keyword constant or value, a bound `Val(Ref)`, a binding expression, a rule body, a `not` clause, an `or` branch and a keyword whose name is a UUID, in memory, after a checkpoint and after reopen; a keyword still equals the same keyword only, and set elements still compare as they are (#744).
  - A pseudo-attribute pattern never gives a variable its value, even when it runs first (#606; the plan-independence matrix checks it with reordering off).
- **Pseudo-attribute sources** (`tests/pseudo_source_test.rs`, 12 tests; #594)
  - A per-fact pseudo-attribute reads the nearest earlier pattern in the text with the same entity term (#594's example in both text orders). Two pseudo-attributes share a source. `[:alice ?a _]` gives one row per fact. Constant and bound value terms check.
  - `:alice` and its UUID are one term, while `?e` and `:alice` are not.
  - `PRS-081` for a pattern after it, alone, `_`/`?_x` entities, another entity's pattern, and a rule call.
  - A `not` body reads the outer fact (and a source inside the body); `not-join` crosses for a listed variable only; `or` branches read the outer fact and never a sibling; also `or-join`, closed rule bodies, `$slot`s by name at prepare time, and `:db/valid-at` once per fact.
  - Parser unit tests check the hidden variables (`?#<n>.<attr>`), the join variable a crossed `not-join` gains, and the `PRS-081` text. An optimizer test places a pseudo-attribute pattern after its source and the pattern binding its value. Matcher tests read the hidden variable, a substituted integer, and nothing without a source. The engine tests run the sourced shapes and check `PRS-081` for the old fallback shapes.
- **Negation equality** (`tests/negation_equality_test.rs`, 5 tests; #546)
  - A value a `not` / `not-join` body takes from the outer row compares by join equality on every path (exclusion set, per-row body with an expression, a query that invokes a rule, a rule body): `1e-20` and `2e-20` differ; `0.0` equals `-0.0`; a keyword imported into an entity position equals the ref; a float literal in the body matches equal floats only (#706). Through `execute` and a cursor, in memory and after a checkpoint.
  - Engine unit tests (`src/query/engine/tests.rs`, 6 more) run every negation shape at six temporal settings, with each body form forced and reordering on and off, in memory and on file before and after checkpoints, against their frozen answers. Shapes: hashed and correlated bodies; keys on entity, value and attribute positions; floats, keywords and refs, long strings; bodies reading an imported variable; variable-free bodies; per-fact pseudo-attributes and `:db/valid-at` in bodies; deferred and repeated negations.
  - They also check the form the planner picks, routing, multiplicity, failing expressions and unknown predicates in bodies, and that a hashed body is read once while a correlated one probes once per outer row.
  - Optimizer unit tests cover `plan_seeded` and the `SUBPLANS` switch.
- **Disjunction equality** (`tests/or_equality_test.rs`, 5 tests; #546)
  - An `or` / `or-join` branch meets the outer row by join equality on every path: a keyword in the outer row meets a ref in a branch; `0.0` and `-0.0` meet; floats `1e-20` apart do not.
  - Branches naming different outer variables each meet the outer row on their own.
  - An `or-join` branch that does not bind its join variable keeps every outer row when it has a row.
  - Through `execute` and a cursor, in memory and after a checkpoint.
  - Engine unit tests run every disjunction shape with each branch form forced and reordering on and off, against their frozen answers. They cover routing (rule invocations, nested `or`, registry predicates, branch-local metadata), the branch form the planner picks, `Distinct` (equal rows collapse; a keyword and a ref, `-0.0` and `0.0`), and that a hashed branch is read once and a correlated one runs only for rows earlier branches did not match.
- **`or` with no shared variable** (#610, `tests/disjunction_test.rs`): joins as a cross product, before or after the earlier clauses, through the seeded path (a branch with `not`) and in a rule body; keeps the earlier clauses' variables and fact metadata (a pseudo-attribute); gives no rows when no branch matches.
- *(v2.x)* **Empty `or-join`** (#405): returns an empty result, not `INT-031`, when no rows reach it: on an empty database, after a pattern that matches nothing, after a `not` that removes every row, and in a rule body.
- **Aggregates and windows** (`tests/aggregate_window_rules_test.rs`, 8 tests; #547)
  - Exact float `sum` and window `avg`; integer `sum` saturating once; exact window `sum` of integers above 2^53; `min`/`max` of `-0.0` and `0.0`.
  - `rank` by keywords (peers 1 1 3 3); a running `sum` giving peers the value at the last peer; `row-number` breaking ties by the other `:find` variables.
  - Nonnumeric arguments are skipped by ordinary sum (#789), rather than raising INT-041.
  - `src/query/datalog/aggregate/tests.rs` (27 unit tests) checks every accumulator over every order of small inputs, and that `value_order` is a total order.
  - `tests/window_functions_test.rs` checks `PRS-082` and a window over the groups of an aggregate query.
  - `tests/window_functions_test.rs` (#182) also checks `lag`/`lead` on whole and partitioned results, descending order and ties, singleton and mixed-type partitions, `nil` edges, long strings, keywords, refs and sets, grouped rows, distinct output, pagination, prepared queries, cursors and preserved user aggregates of those names. Parser tests cover malformed shapes and `tests/plan_independence_test.rs` checks both directions under reordered plans.
  - `Aggregate` on the engine (#547 PR 2, `src/query/engine/tests.rs`, 3 tests): every built-in aggregate grouped by keywords, refs, booleans, floats and long strings, over `nil` values, keywords and refs, with `:with` and `or`, at six temporal settings in memory, after a checkpoint and after reopen, against their frozen answers. Also: an ungrouped aggregate reads every row and returns one batch of one row; the zero row and empty answers; groups in value order over several batches; nonnumeric sum arguments skipped in a later group; a group dropped by `min` of no value.
  - `tests/aggregation_test.rs` checks that a group of `0.0` and `-0.0` returns `-0.0` in either insertion order.
- **Query validation** (#406, #414)
  - A plain `:find` variable, or a window function's argument, `:partition-by` or `:order-by` variable, that no `:where` clause binds (including `?_` wildcards) is `PRS-080` from both `execute()` and `prepare()`, naming the variable.
  - Variables bound by a pattern, an attribute position, a rule call, an expression binding, an `or-join`, every branch of an `or`, a pseudo-attribute or a window spec are accepted. Bind slots are not mistaken for `:find` variables.
  - An unknown aggregate is `QRY-010`, and an unknown window function `QRY-011`: on an empty database, when no row matches, when rows match, through a rule query and through a prepared query. A registered UDF in either position returns an empty result on an empty database.

## Rules

- **Rule queries on the engine** (`tests/recursive_engine_test.rs`, 11 tests; #548)
  - Seven engine unit tests for rule-query entity-probe reuse (#672) check one inner read for repeated entities, full answer bags through LRU eviction, other shared value bindings, keyword/ref normalization, a recursive inner relation across rounds, streaming above the 128-row cache bound, and successful streaming when the query memory limit refuses an optional cache entry. The nightly `reach_5m` acceptance test still checks 4,882,500 derived facts, RSS, and the default derived-fact limit.
  - Linear and non-linear transitive closure, mutual recursion, and strata whose `not` and `or` rules recurse (#633's cases).
  - Naive rules: a same-stratum read in an `or` branch, through a `:p` pattern, through `[?y ?a ?z]`.
  - Derived facts visible to every pattern and to the magic-set predicate's attribute.
  - Identity of derived facts: keyword vs ref, `-0.0` vs `0.0`, long strings, a head equal to a stored fact.
  - `nil` metadata and `:db/valid-at` over derived facts; one-argument heads; `QRY-013` and `INT-022` heads; both `INT-020` limits.
  - Rule queries with aggregates, expressions, `not-join`, `or` and windows.
  - Each in memory, on a file with pending facts and after a checkpoint, against their frozen answers.
  - Engine unit tests check routing (`:as-of` timestamps, a rule body with no pattern, a three-argument call and a write transaction) and derived rows read across batches before and after a checkpoint. `relation.rs` unit tests check row identity and the `Old` / `Delta` / `Full` ranges.
  - Every delta run of the magic-set rewrite of a recursive rule joins each clause on a variable bound before it, with no keyless join (`rule_delta_runs_join_on_a_bound_variable`, #694). An optimizer unit test checks `plan_after`'s connected order.
  - A join on a bound value probes its AVET ranges and pending facts (#721): the delta run of the ancestors shape probes `[?x :manager ?y]` by value, in memory and on a half-checkpointed file (`rule_delta_probes_a_bound_value`); a value join reads only the probed keys, through a pending retraction (`value_probe_reads_only_the_probed_keys`); keyword and ref from either side, an entity cell no keyword was recorded for (the hash-join fallback), `0.0`/`-0.0`, long strings, `:as-of` and `:valid-at`, in memory, checkpointed and pending (`value_probe_matches_as_joins_do`); a fallback in the middle of a batch joins the rows before it, the rest of the batch and later batches once each (`value_probe_falls_back_mid_batch`).
  - A query's boxed `Stream` stays within the 1,032 bytes glibc serves from its per-thread cache (`stream_fits_the_allocator_thread_cache`, 64-bit glibc): #721's value-probe state, kept inline, had grown it to 1,056 bytes and a point query by about 5% (#721 follow-up).
- **`:db/valid-at` in rule bodies** (`tests/recursive_rules_test.rs`, #617): a `:db/valid-at` pattern in a rule definition, in its `not` and `not-join` bodies and in its `or` branches binds the query's valid time, on the semi-naive, stratified and magic-sets paths.
- **Rules calling `not` / `or` rules** (`tests/not_join_test.rs`, #628)
  - A rule calling a rule with `not-join`, `not` or `or` returns that rule's rows, with a free and a literal argument and through a forwarding rule.
  - A rule with its own `not-join` calling one gets the same rows whether or not the query asks for the callee first.
  - Stratification unit tests: such a caller, a rule forwarding to it and a caller of a recursive one sit above the callee; recursion through a rule with a `not` still stratifies.
- **Recursion through `not` / `or` rules** (`tests/not_join_test.rs`, #633)
  - Recursion through a rule with `not`, `not-join` or `or`, directly or through another rule, returns every row along a chain, and the `not` still removes the rows it should.
  - Evaluator unit tests: such recursion stops at its fixpoint; `max_iterations` and `max_results` end one that does not with `INT-020`; `max_results` counts the facts every pass derives, not only the mixed rule's.
- **Rule limits on large databases** (#528): a rule query on a database with more live facts than `max_results` returns its answer, with a bound and with a free first argument; a query deriving more facts than its `:max-results` is still `INT-020`.
- **Magic-sets demand**
  - `tests/magic_sets_test.rs` (#608): rule queries with a literal argument where some call site cannot carry the demand — a rule-body call with an argument nothing binds yet, the same rule called with free arguments, a call from a rule called with free arguments, calls inside `or` and `not`, a fully literal call, and a call bound only by a variable. `magic_sets.rs` unit tests check the adornment each shape solves to.
  - *(v2.x)* A recursive rule with a literal start that recurses through another rule returns the same results as without the literal (#297).
- *(v2.x)* **Deterministic stratification errors** (#410): a unit test builds the same two-rule negative cycle 64 times and checks that the `INT-054` message names the same predicate pair every time.

## Parser

- **Parser error codes** (`tests/error_codes_prs_test.rs`, 26 new tests; #643): each of `PRS-083` to `PRS-107` (the parse errors that were `INT-019`, and the retract arity that was `INT-000`) from its `docs/ERROR_REFERENCE.md` example, in the Parser category, with its argument in the message and without the old `datalog parser: ` prefix; three of them through `prepare`.
- **Fact arity** (`tests/error_codes_prs_test.rs`, 2 tests; #649): a fact of five or six elements, with or without a map 4th, fails with `PRS-108`, and a transaction holding one stores none of its facts.
- **Parser tightening** (`tests/error_codes_prs_test.rs`, 10 tests; #651)
  - `PRS-109` to `PRS-114` from their inputs: an unterminated string, a `:` with no name, `(query [])`, an empty `:find`/`:where`/`:with` in each position, an unknown query keyword with and without a value after it, a constant rule head argument.
  - `PRS-061` for a value after each query option.
  - `PRS-097` for a clause list headed by an operator or a variable, also inside `not`, `or` and a rule body.
  - `PRS-105` for a rule named by a variable or an operator.
  - `PRS-025`/`PRS-029`/`PRS-063` for an aggregate, window or expression named by a variable.
  - The forms next to them still run: an option followed by a keyword, `(- ?n 1)`, a zero-argument rule.
  - Thirteen new `tests/grammar/invalid/syntax/` fixtures.
- **Grammar page** (`tests/grammar_doc.rs`, 10 tests; #643): `docs/GRAMMAR.md` matches the EBNF rendered from `tests/grammar/grammar.pest`. Each rendering rule is checked on a small grammar: sequence, choice and grouping, names and builtins, `SOI`/`EOI` dropped, lookaheads as comments, bounded repetition, quoting, token marks, section titles, line breaking, the whitespace note and the comment note it joins.

## Browser and WASI

None of these are counted in the summary. The wasm32 targets run in CI (`.github/workflows/wasm.yml`).

- **In a browser** (`wasm-pack test --headless`, Firefox and Chrome): 61 `wasm_bindgen_test` tests. The final #641 snapshot passed in headless Firefox on the bench; Chrome remains a CI run.
  - Three committed-memory-backend input-order fixtures: scalar Sort omission, streaming counts with Boolean/float representatives and mixed numeric group order, and partition windows preserving float edges and numeric group classes (#641).
  - Twenty-five for `BrowserDb`, including IndexedDB persistence, the cursor, and the flush returning every dirty page and failing on one it cannot read (#470).
  - Eleven for `BrowserBufferBackend` (which used `#[test]` and never ran before).
  - Two checking that IndexedDB page keys accept only whole numbers from 0 to 2^53.
  - Fifteen for `BrowserDb` user functions (#180), below.
- **`BrowserDb.registerPredicate` / `registerAggregate`** (#180): a predicate filters rows; every value kind reaches JS as its `{type, value}` object (an integer as a `bigint`, a ref as its UUID, a set's elements in canonical order); an aggregate in `:find` and with `:over`; a plain `finish` result read as a bind value; a throw (an `Error`, a string), a predicate returning a non-boolean, a failing `init`/`step`/`finish` and a `finish` result that is a bad ref or not a value each fail with `QRY-014` (`execute`, cursor and prepared query; aggregates also as windows); `TypeError` for a non-function and `INT-040` for a taken or built-in name; from inside a UDF, a nested `query` running a UDF, a registration (not seen by the outer query, seen by the next), and `execute`/`checkpoint`/`importGraph` that resolve after the outer query with both answers right; a value missing from the thread's table fails the call (`called off the JavaScript thread`), and a dropped function frees its entry.
- **`BrowserDb.prepare`** (#181): one prepared query run with three bindings; every plain JS value (boolean, whole and fractional `number`, `bigint`, string, `null`) and every `{type, value}` form; a prepared cursor gives the rows of `execute`; `TypeError` for a non-bind value or non-object bindings, `API-017` for a bad UUID, a negative tx count or a value on `null`/`anyValidTime`, `API-022`, `API-023`, `API-024` (both `execute` and `query`) and `API-005`; a later write is seen by the next run.
- **`BrowserDb` cursor**: `query()` batches of 1, 7 and 1,000 give the rows of `execute()`; a cursor keeps the answer from when it opened and returns the end after `close()`; a `transact` is `API-012`.
- **Page reuse** (#411, #440; five `BrowserDb` tests)
  - Opening a v7 store written straight into IndexedDB stores a v8 meta page and only the live pages, with nothing written on the next open.
  - After each of 60 transacts, memory, and at the end IndexedDB, hold exactly the pages not on the free list, while the file grows by under one page per transact.
  - A commit applied in memory but never flushed reopens at the previous one and keeps working.
  - An export after churn is as long as the meta says, and imports back with its free pages dropped.
  - `importGraph()` deletes the replaced database's IndexedDB keys.
  - Two buffer tests check that releasing a page drops its bytes and any unflushed write, and that export writes a released page as zeros.
  - A native storage test checks that a file whose free pages are all zeros (as the browser exports them), both just migrated from v7 and after 20 saves, opens, verifies clean and keeps committing. The free-list test checks that a checkpoint reports exactly the ids it newly lists.
- **On `wasm32-wasip1` under Wasmtime**: the unit and integration tests that do not use a file run, 1,182 passing and seven ignored in the 2026-10-28 bench run with Wasmtime v49.0.2 (`wasm.yml`). All three committed input-order fixtures run here too, and the WASI REPL builds in release mode.

## Ignored Tests

The seventeen ignored tests (eighteen with `test-hooks`) are intentionally excluded from a normal local run:

- Four rustdoc examples that reference internal types and cannot compile as standalone examples.
- One high-contention concurrency stress test intended for scheduled runs.
- One long-haul smoke test intended for scheduled runs.
- One soak test at 10M facts (`soak`), run weekly by `.github/workflows/soak.yml` (#392). `soak_short` runs the same driver at 20K facts in the normal suite. The memory bound is measured in a child process (`soak_memory_child`, #542).
- One size test at 1.15M facts (`size_per_fact_in_433_shape_1m`), run by the nightly smoke workflow.
- One recursive-rule acceptance test (`reach_5m`, #548), run by the nightly smoke workflow: a layered graph whose `reach` rule derives 4,882,500 facts in 62 rounds must return them all with peak RSS under 256 bytes per derived fact, and the default `max_derived_facts` must stop the same rule with `INT-020`.
- Three query-memory acceptance tests (`tests/memory_limit_acceptance_test.rs`, #549, #575, #551), run one at a time by the nightly smoke workflow:
  - 1M ordered count and sum groups return 1M rows under a 32 MiB `query_memory_limit`; the floating `sum-distinct` fallback control stops with `QRY-012` naming `Aggregate`, after which the database answers;
  - 13 patterns sharing no variable over the fuzz harness's 3 facts stream 3^13 rows with peak RSS under 64 MiB, and with a 64-byte limit stop with `QRY-012` naming `HashJoin`;
  - `count` over every one of 1M facts counts them under a 4 KiB `query_memory_limit` (`count_holds_no_rows`, the #473 constant-memory `count` item).
- One `:limit` and top-k acceptance test (`tests/limit_acceptance_test.rs`, #306, the #473 `:limit` item), run by the nightly smoke workflow over one 10M-fact fixture written with `LogWriter`: `:limit 10` returns 10 rows in under 250 ms under a 64 KiB `query_memory_limit`, adding under 32 MiB of peak RSS; `:order-by [?v :desc] :limit 10` returns the ten largest values under a 1 MiB limit, adding under 32 MiB.
- One large-transaction acceptance test (`tests/large_transaction_acceptance_test.rs`, #550; `test-hooks` only), run by the nightly smoke workflow (see [Large transactions](#large-transactions-550)).
- One measurement test (`meter_peak_against_rss`, `src/query/engine/tests.rs`, #549): prints the meter's peak against the peak RSS for the same two shapes, opening the engine directly.
- One committed-group measurement test (`committed_group_memory_probe`, `src/query/engine/aggregate_order_tests.rs`, #641): opens prebuilt fixed-size and growing-group fixtures, drains count/count-distinct with and without a limit, and prints meter peaks, active-group high-water and input counts.
- Two generalized group measurement tests (`generalized_committed_memory_probe` and `certified_growing_distinct_memory_probe`, `src/query/engine/aggregate_order_tests.rs`, #781): drain prebuilt fixed-group and growing-distinct fixtures, reporting Aggregate and IdSpace peaks separately, input counts and exact results. See the [100K–10M evidence](benchmarks/781-ordered-aggregate.md).
- One row-frame measurement test (`window_rows_benchmark_meter_peaks`, `src/query/engine/tests.rs`, #183): prints metered peaks for the window benchmark fixtures.

## Reproducing

```bash
# Standard suite
cargo test --quiet

# Include the scheduled/ignored tests when the host supports them
cargo test -- --include-ignored

# Kani proofs of key encoders and scalar decoders (#627, #693); Kani pins its own nightly
cargo install --locked kani-verifier && cargo kani setup
cargo kani --lib -j 4 --output-format terse -Z unstable-options --harness-timeout 10m

# Generate a local branch-coverage report (requires cargo-llvm-cov and a nightly toolchain)
cargo +nightly llvm-cov --branch --html

# Check the per-file gates the way CI does
cargo +nightly llvm-cov --branch --no-report
cargo +nightly llvm-cov report --branch --json --summary-only --output-path coverage.json
python3 scripts/coverage_gates.py coverage-thresholds.toml coverage.json
```

### Coverage gates

- Coverage percentages are intentionally not fixed in this document: they depend on the toolchain and instrumentation.
- `coverage-thresholds.toml` holds the whole-crate and per-file line and branch floors, and lists the paths left uncovered on purpose (#396).
- The floors are measured with the nightly and cargo-llvm-cov version pinned in `.github/workflows/coverage-gates.yml`, the only coverage workflow; it also uploads the report to Codecov.
- Delete `target/llvm-cov-target` after changing the toolchain or cargo-llvm-cov version: stale binaries from an older build are merged into the report and skew it.
