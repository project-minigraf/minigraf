# Minigraf Test Coverage

## Coverage Summary

**Verified**: 2026-09-25 with `cargo test --quiet`

**Result**: 1,147 passing tests and 8 ignored tests (1,155 total)

This supersedes the v2.0.0-era count (1,145 passing, 8 ignored) with the unreleased
file-format-v8 work (#287, #371): the index-key value-collision fix, the v7→v8
migration, and the drop of formats v1–v6. It otherwise still covers the public
database API, the Datalog engine, storage and recovery paths, and v2.0.0's locking
and structured-error-code changes.

### Covered Areas

- **Core database behavior**: in-memory and file-backed operation, transactions, checkpoints, retractions, bi-temporal queries, recursive rules, prepared statements, aggregation, expressions, disjunction, window functions, UDFs, and magic-sets evaluation.
- **Multi-value and index-key correctness** (`tests/multi_value_test.rs`, 8 tests): same-transaction multi-valued facts surviving query-time dedup, retracting several values of one attribute in one call, and format v8 index keys (value bytes + assert/retract flag) (#287, #371).
- **Storage correctness**: packed pages, B+tree indexes and range scans, cache behavior, file-header validation, migration (including v7→v8 index rebuild and v1–v6 rejection via `STG-028`), WAL replay, checksums, corruption handling, and fault injection.
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
