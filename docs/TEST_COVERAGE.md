# Minigraf Test Coverage

## v2.0.1 Coverage Summary

**Verified**: 2026-09-26 with `cargo test`

**Result**: 1,202 passing tests and 8 ignored tests (1,210 total)

The v2.0.1 suite covers the public database API, the Datalog engine, storage and recovery paths, and the v2.0.0 locking and structured-error-code changes, plus v2.0.1's regression test for rebuilding indexes on open after several checkpoints (#370). Since v2.0.1, `tests/point_query_history_test.rs` and new storage unit tests check that attribute-narrowed bound-entity lookups return exactly what a full scan returns, and that `net_asserted_facts` matches its previous implementation on randomized histories (#323). Attribute-scan tests check that the AEVT range covers exactly the queried attribute, including non-ASCII names such as `:丿` and prefix siblings such as `:ab` for `:a` (#381). Incremental B+tree rebuild tests check that checkpoints which copy untouched index leaves produce exactly the entries and file checksum a full rebuild would, including over repeated checkpoints, that single-fact checkpoints do not fragment the index, and that a cyclic leaf chain or damaged slot directory is rejected rather than copied (#315). Directory-sync unit tests check that creating the `.graph` file or the WAL, and deleting the WAL, fsync the parent directory after the file operation, and that reopening an existing file does not (#389). Disjunction tests check that `or-join` returns an empty result, not INT-031, when no rows reach it: on an empty database, after a pattern that matches nothing, after a `not` that removes every row, and in a rule body (#405).

### Covered Areas

- **Core database behavior**: in-memory and file-backed operation, transactions, checkpoints, retractions, bi-temporal queries, recursive rules, prepared statements, aggregation, expressions, disjunction, window functions, UDFs, and magic-sets evaluation.
- **Storage correctness**: packed pages, B+tree indexes and range scans, cache behavior, file-header validation, migration, WAL replay, checksums, corruption handling, and fault injection.
- **Reliability and concurrency**: concurrent reads and writes, rollback behavior, same-process handle exclusion, cross-process locking, PID namespaces, NFS and container lock behavior, and SIGKILL recovery.
- **Public diagnostics**: structured parser, query, storage, WAL, API, and internal error codes; the error-code registry is checked against [`ERROR_REFERENCE.md`](ERROR_REFERENCE.md).
- **Compatibility and quality**: cross-platform format compatibility, property-based tests, long-haul smoke coverage, XTDB/Datomic semantic compatibility, grammar conformance, and rustdoc examples.

### Ignored Tests

The eight ignored tests are intentionally excluded from a normal local run:

- Six rustdoc examples that reference internal types and cannot compile as standalone examples.
- One high-contention concurrency stress test intended for scheduled runs.
- One long-haul smoke test intended for scheduled runs.

### Reproducing

```bash
# Standard suite
cargo test --quiet

# Include the scheduled/ignored tests when the host supports them
cargo test -- --include-ignored

# Generate a local branch-coverage report (requires cargo-llvm-cov)
cargo llvm-cov --branch --html
```

Coverage percentages are intentionally not fixed in this document: they depend on the current toolchain and instrumentation. The CI coverage gates and the command above are the source of truth for the current measurement.
