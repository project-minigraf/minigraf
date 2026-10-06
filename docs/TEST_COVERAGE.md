# Minigraf Test Coverage

## Coverage Summary

**Verified**: 2026-10-05 with `cargo test --quiet`

**Result**: 1,225 passing tests and 8 ignored tests (1,233 total)

This covers the unreleased file-format-v8 work (#287, #371): the index-key value-collision fix, the v7→v8 migration, and the drop of formats v1–v6. It otherwise still covers the public database API, the Datalog engine, storage and recovery paths, v2.0.0's locking and structured-error-code changes, and v2.0.1's regression test for rebuilding indexes on open after several checkpoints (#370). Merged from the v2.x line: `tests/point_query_history_test.rs` and new storage unit tests check that attribute-narrowed bound-entity lookups return exactly what a full scan returns, and that `net_asserted_facts` matches its previous implementation on randomized histories (#323). Attribute-scan tests check that the AEVT range covers exactly the queried attribute, including non-ASCII names such as `:丿` and prefix siblings such as `:ab` for `:a` (#381). Incremental B+tree rebuild tests check that checkpoints which copy untouched index leaves produce exactly the entries and file checksum a full rebuild would, including over repeated checkpoints, that single-fact checkpoints do not fragment the index, and that a damaged slot directory is rejected rather than copied (#315). B+tree cursor tests check that every scan walks the tree, not the leaf `next_leaf` chain: full scans and start positions match the input, `seek` from the current position lands where a fresh descent from the root would (randomized, plus within a leaf, to a sibling, across subtrees, past the end, into the gap after a leaf, and backwards as a no-op), a seek to the sibling leaf reads exactly one page, reads are unchanged with every `next_leaf` zeroed, a child pointer into a non-B+tree page is STG-013, and a cycle in child pointers is an error, not a hang (v8 storage format PR 1, #374). Directory-sync unit tests check that creating the `.graph` file or the WAL, and deleting the WAL, fsync the parent directory after the file operation, and that reopening an existing file does not (#389). Disjunction tests check that `or-join` returns an empty result, not INT-031, when no rows reach it: on an empty database, after a pattern that matches nothing, after a `not` that removes every row, and in a rule body (#405). A stratification unit test builds the same two-rule negative cycle 64 times and checks that the INT-054 message names the same predicate pair every time (#410). Magic-sets tests check that a recursive rule with a literal start that recurses through another rule returns the same results as without the literal (#297). API tests check that `execute()` and `WriteTransaction::execute()` reject `$slot` bind tokens in every position with API-010 (#407). The REPL `run()` test now runs in a child process with null stdin, so it no longer hangs when the test runner's stdin is an open non-TTY stream (#415). v8 page-format tests (PR 2, #374, #388) check the 24-byte page header (a flipped bit, a wrong page id and a future generation each give their own STG code, legacy page types fail before the CRC, and a failed page is never cached), meta-page encoding, the free-list chain, WAL v2's base generation, and every row of the meta-selection table: both slots valid, a torn newest slot with the WAL's base generation (opens at the older one), a WAL newer than the valid meta, pages of the next generation on the free list or past `page_count` with no WAL (all STG-033, with the file unmodified), an unknown feature bit (STG-034), and a pre-release v8 single header (STG-032). Fault-injection tests crash `save()` at every page write (cleanly and with torn pages, including the meta write) and every sync, and crash a v7 migration at every write; each reopen finds exactly the old or the new checkpoint, and every reachable page plus the free list covers the file exactly. Twenty-five consecutive saves never write a page the previous meta references. A maximum-size fact's index keys fit in an internal node, and an oversized leaf entry or separator is rejected instead of overlapping the slot directory.

### Covered Areas

- **Core database behavior**: in-memory and file-backed operation, transactions, checkpoints, retractions, bi-temporal queries, recursive rules, prepared statements, aggregation, expressions, disjunction, window functions, UDFs, and magic-sets evaluation.
- **Multi-value and index-key correctness** (`tests/multi_value_test.rs`, 10 tests): same-transaction multi-valued facts surviving query-time dedup, retracting several values of one attribute in one call, valid-time stints (facts differing only in valid time), maximum-value-size boundaries, and format v8 index keys (value bytes + assert/retract flag) (#287, #371).
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
