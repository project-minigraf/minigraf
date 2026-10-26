# Minigraf Test Coverage

## v2.0.11 Coverage Summary

| | |
|---|---|
| **Verified** | 2026-10-26 with `cargo test` |
| **Passing** | 1,326 |
| **Ignored** | 9 (see [Ignored Tests](#ignored-tests)) |
| **Total** | 1,335, including the golden-file corpus (#391) |

Not counted above: the `BrowserDb` tests, which run only on wasm32 with `wasm-bindgen-test` in a browser (see [Browser](#browser)).

The suite covers the public database API, the Datalog engine, storage and recovery paths, and the v2.0.0 locking and structured-error-code changes. [Regression Tests by Area](#regression-tests-by-area) lists the tests added for each fixed issue; `CHANGELOG.md` says which release shipped each fix.

## Covered Areas

- **Core database behavior**: in-memory and file-backed operation, transactions, checkpoints, retractions, bi-temporal queries, recursive rules, prepared statements, aggregation, expressions, disjunction, window functions, UDFs, and magic-sets evaluation.
- **Storage correctness**: packed pages, B+tree indexes and range scans, cache behavior, file-header validation, migration, WAL replay, checksums, corruption handling, and fault injection.
- **Reliability and concurrency**: concurrent reads and writes, rollback behavior, same-process handle exclusion, cross-process locking, PID namespaces, NFS and container lock behavior, and SIGKILL recovery.
- **Public diagnostics**: structured parser, query, storage, WAL, API, and internal error codes; the error-code registry is checked against [`ERROR_REFERENCE.md`](ERROR_REFERENCE.md).
- **Compatibility and quality**: golden-file corpus of released file formats, cross-platform format compatibility, property-based tests, long-haul smoke coverage, a weekly soak at 10M facts with a 5-hour churn (#392), XTDB/Datomic semantic compatibility, grammar conformance, and rustdoc examples.

## Whole-System Checks

These tests compare the database against an independent answer rather than a single fixed case.

- **Property tests** (`tests/property_test.rs`, #386)
  - Write random histories (batched transacts, retracts, write transactions, checkpoints) in memory and to a file.
  - Compare random queries with a reference evaluator, on the open file and after a reopen. Queries use one to three joined patterns, `not` or `not-join`, a comparison predicate, `:as-of`, `:valid-at` or `:any-valid-time`.
  - A query error fails the test.
  - The histories leave out the v2.x known issues #371, #435 and #477.
- **Crash-kill test** (`tests/crash_kill_test.rs`, #384)
  - Kills a child process running a seeded workload at a random point, under five checkpoint modes.
  - The reopened file must hold exactly the transactions the child logged as committed, or those plus the one in flight.
  - Checked through full-scan, AEVT, AVET, EAVT and `:as-of` queries; again after a second reopen; and after a checkpoint plus reopen.
  - The workload leaves out the v2.x known issues #371 and #435.
- **Golden-file corpus** (`tests/golden_corpus_test.rs`, #391)
  - `tests/golden/` holds v7 files written by minigraf 2.0.3: one checkpoint, several checkpoints, indexes rebuilt on open, a crash-left WAL, an already-checkpointed WAL, same-transaction multi-values.
  - For a copy of each file, checks its manifest's CRC32, `tx_count` and query results after open, after a checkpoint and reopen, and after one more write.
  - A second test checks that every golden file has a manifest.
  - `v7_stale_wal` must have no WAL after a checkpoint (#457).
- **Grammar verifier** (`src/query/datalog/parser/verifier.rs`, 4 tests, backported from v3.0.0; #643)
  - Checks the parser against `tests/grammar/grammar.pest`: pest rejects ⇒ the parser rejects; pest accepts ⇒ the parser accepts or raises a `SEMANTIC` code with a witness (`INT-019` counts as semantic on v2.x).
  - About 1.07M lexical strings and 555K structural inputs at the default bound; 7.6M at `MINIGRAF_GRAMMAR_BOUND=4`, run nightly by `grammar-verifier.yml`.
  - Known drift is listed by cause in `KNOWN_DRIFT` (#651).
  - `tests/grammar_conformance.rs` classes a fact with too few elements and a retract of the wrong arity as syntax errors, since `grammar.pest`'s fact is exact.
- **Kani proofs of the value encoding** (`src/storage/index/verification.rs`, 5 harnesses; #627)
  - Not `#[test]`s: run by `cargo kani` and `.github/workflows/kani.yml`.
  - Prove, for every input, that `encode_value` (the `value_bytes` of the v7 indexes) writes `Null` as `[0x00]`, and each of `Boolean`, `Integer`, `Float` and `Ref` with its own type byte and in its own order. Non-NaN floats follow IEEE total order, so `-0.0` sorts before `0.0`; every NaN encodes to the same bytes.
  - Each harness fails when its encoder is broken on purpose.
  - Not asserted: where NaN sorts (#688).
  - Left out: strings, keywords, decoders, fact pages and the WAL. The model checker does not finish on them, so the fuzz and property tests cover them. v3.0.0 adds proofs of its key encoders.
- **Fault injection** (#374): crashes `save()` at each of its 47 page writes and syncs; every reopen must find exactly the old or the new checkpointed facts.

## Regression Tests by Area

### File open and storage

- **Short files** (`tests/edge_cases_test.rs`, #506): a file of 1 to 4,095 bytes
  - that is a database cut below one page, or foreign bytes, fails with `STG-045`, is left unchanged, and gets no WAL;
  - that is a prefix of the initial header, or all zeros, opens as a new database that takes a write and reads it back after reopen.
- **Index rebuild on open** (#370): indexes are rebuilt correctly on open after several checkpoints.
- **Incremental B+tree rebuild** (#315)
  - Checkpoints that copy untouched index leaves produce exactly the entries and file checksum of a full rebuild, also over repeated checkpoints.
  - Single-fact checkpoints do not fragment the index.
  - A cyclic leaf chain or a damaged slot directory is rejected rather than copied.
- **More than 65,535 uncheckpointed facts** (storage unit tests and two `edge_cases_test.rs` tests, #445)
  - Each fact resolves to itself through entity lookups and attribute scans, in memory and file-backed, before and after a checkpoint.
  - The widened `FactRef` encodes to the same bytes as before.
- **Point queries** (`tests/point_query_history_test.rs` and storage unit tests, #323)
  - Attribute-narrowed bound-entity lookups return exactly what a full scan returns.
  - `net_asserted_facts` matches its previous implementation on randomized histories.
- **Attribute scans** (#381): the AEVT range covers exactly the queried attribute, including non-ASCII names such as `:丿` and prefix siblings such as `:ab` for `:a`.
- **Directory sync** (unit tests, #389): creating the `.graph` file or the WAL, and deleting the WAL, fsync the parent directory after the file operation; reopening an existing file does not.

### WAL and recovery

- **Torn WAL tail** (#513)
  - Opening a WAL writer cuts off a partial last entry (cut in the checksum, the header or the payload), or an entry with a bad checksum and everything after it, then appends where replay reaches. An intact WAL is left byte for byte.
  - An append that fails part-way makes every later append `WAL-007`. A rejected oversize fact (`WAL-003`, nothing written) does not.
  - Through `Minigraf`: the failed transaction is not applied, later `transact` and `commit` fail with `WAL-007`, and a `checkpoint()` makes writes work again.
  - In `tests/wal_test.rs`: a commit made after reopening a crash-left WAL with each kind of torn tail survives a second crash.
- **WAL entries of any size** (`src/wal.rs`, 3 tests, one replacing the 1,000,000-fact cap test; #686)
  - An entry of 1,000,001 facts and the entries after it replay, and reopening the writer cuts none of them off.
  - A fact count the bytes after it cannot hold (one more than a quarter of them, or `u64::MAX`) ends replay at that entry, and reopening the writer cuts the WAL off there.
  - A count the bytes could hold, whose facts are cut short at a fact's length or inside its bytes, also ends replay there.
- **Already-checkpointed WAL**
  - Reopening next to a WAL whose entries are all checkpointed, then writing again, leaves `:as-of` history unchanged before and after another reopen; a unit test checks that restoring the transaction counter never lowers it (#447).
  - An explicit `checkpoint()` removes such a WAL, keeps the fact, and the next write does not reuse tx 1 (#457).
- **Close without checkpoint** (#476): dropping a handle opened with `wal_checkpoint_threshold(usize::MAX)` leaves the `.graph` file byte for byte unchanged under `Full` and `Normal` sync and keeps the WAL; a reopen replays it.

### Concurrency

- **Snapshot atomicity** (`tests/snapshot_atomicity_test.rs`, #554, #555, #558): queries see each concurrent transaction whole and read one state for all their patterns, also during a checkpoint.
- **Calls inside a write transaction** (`tests/write_tx_guard_test.rs`, 4 tests; #734): `checkpoint`, `prepare` and `PreparedQuery::execute` on a thread holding a `WriteTransaction` return `INT-001` (under a 30 s timeout, so a deadlock fails instead of hanging) and the transaction then commits; outside a transaction they run.

### Queries

- **Float constants** (`tests/float_constant_test.rs`, 2 tests; #706)
  - A float constant matches equal floats only: `1e-17` does not match `0.0`; `0.0` matches `-0.0`.
  - Checked in a pattern, bound to a `$slot`, in a rule body, as a rule call's argument, in a `not` body and in an `=` predicate; in memory, on file before and after a checkpoint, and after reopen.
  - `Value`'s hash gives `-0.0` and `0.0` the same hash (`src/graph/types.rs` unit test, #750), so a rule call's `0.0` returns `-0.0`'s row in every run.
- **Keyword or ref in `=` / `!=`** (`tests/keyword_ref_expression_test.rs`, 4 tests; #744)
  - An expression's `=` and `!=` equate a keyword with a ref to the entity it names, as the pattern constant and the join do: either operand order, a keyword constant or value, a bound `Val(Ref)`, a binding expression, a rule body, a `not` clause, an `or` branch and a keyword whose name is a UUID; a keyword still equals the same keyword only.
  - In memory, on file after a checkpoint, and after reopen.
- **`or` with no shared variable** (`tests/disjunction_test.rs`, #610)
  - Joins as a cross product, before or after the earlier clauses, through the seeded path (a branch with `not`) and in a rule body.
  - Keeps the earlier clauses' variables and fact metadata (a pseudo-attribute).
  - Gives no rows when no branch matches.
- **`or` branches and outer variables** (`tests/disjunction_test.rs`)
  - `or` / `or-join` branches whose expressions read or bind a variable only the outer row binds match, nested and in rule bodies; a unit test covers `branch_outer_reads` (#620).
  - Expressions reading a variable only an `or` binds, including a chain and a `not` after them, keep their rows in queries, with rules and in rule bodies; optimizer unit tests cover deferring them ahead of the deferred `not`s (#625).
  - A branch meets a keyword value as the entity it names, and the reverse, in a query and in a rule body (#637).
- **Empty `or-join`** (#405): returns an empty result, not `INT-031`, when no rows reach it: on an empty database, after a pattern that matches nothing, after a `not` that removes every row, and in a rule body.

### Rules

- **Rules calling `not` / `or` rules** (`tests/not_join_test.rs`, #628)
  - A rule calling a rule with `not-join`, `not` or `or` returns that rule's rows, with a free and a literal argument and through a forwarding rule.
  - A rule with its own `not-join` calling one gets the same rows whether or not the query asks for the callee first.
  - Stratification unit tests: such a caller, a rule forwarding to it and a caller of a recursive one sit above the callee; recursion through a rule with a `not` still stratifies.
- **Recursion through `not` / `or` rules** (`tests/not_join_test.rs`, #633)
  - Recursion through a rule with `not`, `not-join` or `or`, directly or through another rule, returns every row along a chain (the `or` binding the next step), and the `not` still removes the rows it should.
  - Evaluator unit tests: such recursion stops at its fixpoint; `max_iterations` and `max_results` end one that does not with `INT-020`; `max_results` counts the facts every pass derives, not only the mixed rule's.
- **`:db/valid-at` in rule bodies** (`tests/recursive_rules_test.rs`, #617): a `:db/valid-at` pattern in a rule definition, in its `not` and `not-join` bodies and in its `or` branches binds the query's valid time, on the semi-naive, stratified and magic-sets paths.
- **Pseudo-attributes in rule bodies** (`tests/recursive_rules_test.rs`, #661): `:db/valid-from`, `:db/valid-to`, `:db/tx-count` and `:db/tx-id` read the stored fact, not the rule evaluator's copy.
- **Rule limits on large databases** (#528): rule queries on a database with more live facts than `max_results` answer, and the limit still stops a rule that derives more facts than it allows.
- **Magic sets with a literal argument**
  - `tests/magic_sets_test.rs` (#608): rule queries with a literal argument where some call site cannot carry the demand — a rule-body call with an argument nothing binds yet, the same rule called with free arguments, a call from a rule called with free arguments, calls inside `or` and `not`, a fully literal call, and a call bound only by a variable. `magic_sets.rs` unit tests check the adornment each shape solves to.
  - A recursive rule with a literal start that recurses through another rule returns the same results as without the literal (#297).
- **Deterministic stratification errors** (#410): a unit test builds the same two-rule negative cycle 64 times and checks that the `INT-054` message names the same predicate pair every time.

### API and parser

- **Prepared queries** (`tests/prepared_statements_test.rs`)
  - A prepared query with a literal `:as-of`, `:valid-at` or `:any-valid-time` returns the same rows as the unprepared query (#603).
  - A prepared rule query fails with `INT-020` one below the database's `max_derived_facts` or `max_results` and succeeds at it, as `execute` does (#679).
- **Bind tokens outside prepared queries** (#407): `execute()` and `WriteTransaction::execute()` reject `$slot` tokens in every position with `API-010`.
- **Oversized facts** (`tests/error_codes_prs_test.rs`, #649): a fact of five or six elements, with or without a map 4th, fails with `PRS-108`, and a transaction holding one stores none of its facts.
- **REPL** (#415): the `run()` test runs in a child process with null stdin, so it no longer hangs when the test runner's stdin is an open non-TTY stream.

### Browser

The `BrowserDb` tests run only on wasm32 with `wasm-bindgen-test` in a browser and are not in the counts above.

- The IndexedDB flush returns every page a write dirtied.
- A dirty page that cannot be read is an error rather than left out.
- A parse, query and transact error reject with their codes (`PRS-011`, `QRY-007`, `PRS-046`), as on the native API (#673).

## Ignored Tests

The nine ignored tests are intentionally excluded from a normal local run:

- Six rustdoc examples that reference internal types and cannot compile as standalone examples.
- One high-contention concurrency stress test intended for scheduled runs.
- One long-haul smoke test intended for scheduled runs.
- One soak test at 10M facts (`soak`), run weekly by `.github/workflows/soak.yml` (#392); `soak_short` runs the same driver at 20K facts in the normal suite.

## Reproducing

```bash
# Standard suite
cargo test --quiet

# Include the scheduled/ignored tests when the host supports them
cargo test -- --include-ignored

# Kani proofs of the value encoding (#627): ~10 s and ~1 GB with -j; Kani pins its own nightly
cargo install --locked kani-verifier && cargo kani setup
cargo kani --lib -j --output-format terse -Z unstable-options --harness-timeout 10m

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
