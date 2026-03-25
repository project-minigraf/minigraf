# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.0] - 2026-03-25

### Added
- Aggregation in `:find` clause: `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`
- `:with` grouping clause — variables that participate in grouping but are excluded from output rows
- `AggFunc` enum and `FindSpec` enum in `src/query/datalog/types.rs`; `DatalogQuery.find` migrated from `Vec<String>` to `Vec<FindSpec>`; `DatalogQuery.with_vars: Vec<String>` field added
- `apply_aggregation` post-processing step in `executor.rs` — runs after binding collection when any aggregate is present
- `extract_variables` helper in `executor.rs` — non-aggregate extraction path (replaces inline loops)
- `apply_agg_func` and `value_type_name` helpers in `executor.rs`
- `parse_aggregate` helper in `parser.rs`; `:find` arm extended to accept `EdnValue::List` (aggregate expressions); `:with` keyword arm added
- Parse-time validation: aggregate variables must be bound in `:where`; `:with` without any aggregate is rejected
- `tests/aggregation_test.rs` — 24 integration tests covering all aggregates, `:with`, rules, negation, temporal filters

### Semantics
- `count`/`count-distinct` with no grouping vars on zero bindings → `[[0]]` (SQL behavior)
- All other aggregates on zero bindings → empty result set
- All aggregates skip `Value::Null` silently (SQL behavior)
- Type mismatches (e.g. `sum` on `String`) fail fast with a runtime error
- `min`/`max` on mixed `Integer`/`Float` is a runtime error
- `:with ?v` adds `?v` to the grouping key without adding it to output columns

### Tests
- Added `tests/aggregation_test.rs` (24 integration tests)
- Total: 461 tests passing (327 unit + 128 integration + 6 doc)

## [0.10.0] - 2026-03-24

### Added
- `src/query/datalog/stratification.rs` — `DependencyGraph` and `stratify()`: analyse rule dependency graphs at registration time; programs with negative cycles are rejected with a clear error
- `WhereClause::Not(Vec<WhereClause>)` and `WhereClause::NotJoin { join_vars, clauses }` variants in `types.rs`; all exhaustive matches updated
- `(not clause…)` in `:where` and rule bodies — stratified negation where all body variables must be pre-bound by outer clauses
- `(not-join [?v…] clause…)` — existentially-quantified negation with explicit join-variable declaration; body variables not in `join_vars` are fresh/unbound
- Safety check at parse time: every `not` body variable must be bound by an outer clause; every `join_vars` variable in `not-join` must be bound by an outer clause
- Nesting constraint: `not-join` cannot appear inside `not` or another `not-join` — rejected at parse time
- `StratifiedEvaluator` in `evaluator.rs`: stratifies rules, runs positive rules first, then applies `not`/`not-join` filters per binding for mixed rules
- `evaluate_not_join` free function in `evaluator.rs`: builds partial binding from `join_vars`, converts `Pattern` and `RuleInvocation` body clauses to patterns, runs `PatternMatcher`; returns `true` if body is satisfiable (reject outer binding)
- `rule_invocation_to_pattern` extracted as `pub(super)` free function from `RecursiveEvaluator`
- Two not-post-filter sites in `executor.rs` now handle both `Not` and `NotJoin` via `evaluate_not_join`
- `tests/negation_test.rs` — 10 integration tests for `not` (Phase 7.1a): basic absence, multi-clause, rule body, time-travel, negative cycle rejection
- `tests/not_join_test.rs` — 14 integration tests for `not-join` (Phase 7.1b): basic exclusion, multiple join vars, multi-clause body, rule body, `:as-of`, `:valid-at`, negative cycle at registration, `not`+`not-join` coexistence, `RuleInvocation` in body end-to-end

### Changed
- `Rule.body` changed from `Vec<EdnValue>` to `Vec<WhereClause>` to support negation clauses alongside patterns
- `executor.rs` `execute_query_with_rules` now delegates to `StratifiedEvaluator` instead of `RecursiveEvaluator` directly
- `rules.rs` `register_rule` runs `stratify()` after each registration; returns `Err` on negative cycle (rules are not registered on error)

## [0.9.0] - 2026-03-23

### Added
- `src/storage/btree_v6.rs` — proper on-disk B+tree for all four covering indexes (EAVT, AEVT, AVET, VAET); each B+tree node is one 4KB page (internal + leaf), with `build_btree` for bulk-load and `range_scan` for leaf-chain traversal
- `OnDiskIndexReader` struct + `CommittedIndexReader` trait — page-cache-backed index lookup replacing the full in-memory BTreeMap; index memory usage is now O(cache_pages), not O(facts)
- `MutexStorageBackend<B>` adapter — holds backend mutex only for the duration of a single `read_page` call on a cache miss; cache-warm pages require no lock, enabling concurrent range scans to proceed in parallel
- `tests/btree_v6_test.rs` — 8 integration tests covering B+tree insert/range-scan, multi-page leaf chains, concurrent scan correctness with Barrier-synchronised threads, and v5→v6 migration roundtrip
- `test_concurrent_range_scans_correctness` unit test in `btree_v6.rs` — verifies all 8 concurrent threads return identical non-empty scan results
- `bench_concurrent_btree_scan` Criterion benchmark — measures wall-clock latency at 2/4/8 concurrent EAVT range scans; results updated in `BENCHMARKS.md`
- `FileHeader` v6 (80 bytes): adds `fact_page_count u64` field at bytes 72–80; automatic v5→v6 migration on first checkpoint

### Changed
- `FORMAT_VERSION` bumped 5→6; v5 databases auto-migrated on first save
- `BENCHMARKS.md` updated with v6 open/memory improvements, concurrent B+tree scan results, heaptrack v6 numbers, and a "How to read these numbers" methodology section
- `README.md` and `BENCHMARKS.md`: performance table updated to reflect v6 open-time reduction (~2.4×) and peak-heap reduction (~21%)

### Fixed
- Concurrent B+tree range scans no longer serialise on cache-warm pages — `4→8 thread` scaling ratio improved from ~2.2× to ~1.9×

## [0.8.0] - 2026-03-22

### Added
- `BENCHMARKS.md` — full Criterion benchmark results at 1K/10K/100K/1M facts with machine spec, HTML report references, and heaptrack memory profiles
- `examples/memory_profile.rs` — heaptrack profiling binary; accepts fact count as positional arg
- `Cargo.toml` metadata: `repository`, `keywords`, `categories`, `readme`, `documentation` fields
- Memory profile table in `README.md` "Performance" section

### Changed
- `README.md` Performance section now links to `BENCHMARKS.md` for full benchmark details
- Phase badge and status text updated to reflect Phase 6.4b completion
- crates.io publish deferred to Phase 7.8 (API cleanup + publish prep; file format v6 now complete)

### Removed
- Dead `clap` dependency from `[dependencies]` — `clap` was listed but never imported in library or binary code

## [0.7.1] - 2026-03-22

### Fixed
- Retraction semantics in Datalog queries: `filter_facts_for_query` Step 2 now computes the *net view* per `(entity, attribute, value)` triple via `net_asserted_facts()`. Previously, retracted facts continued to appear in query results because the original assertion record remained in the append-only log. Now, for each EAV triple in the tx window, only the record with the highest `tx_count` is considered — if it is a retraction, the triple is excluded from results.
- Oversized facts are now rejected early in `db.rs` (`check_fact_sizes`) before any WAL write, using the `MAX_FACT_BYTES` constant (4 080 bytes) exported from `packed_pages.rs`. Previously, oversized facts could cause a panic deep in the page-packing path.

### Added
- `net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact>` helper in `src/graph/storage.rs`: groups facts by EAV triple, keeps the record with the highest `tx_count`, and discards the triple if that record is a retraction. Used by both `executor.rs` and `storage.rs`.
- `check_fact_sizes(facts: &[Fact])` in `src/db.rs`: validates all facts against `MAX_FACT_BYTES` and returns a descriptive `Err` before writing to the WAL.
- `MAX_FACT_BYTES: usize` constant in `src/storage/packed_pages.rs`: `PAGE_SIZE - PACKED_HEADER_SIZE - 4` = 4 080 bytes.
- `tests/retraction_test.rs` — 7 integration tests covering: assert/retract with no `:as-of`, as-of snapshot before/after retraction boundary, re-assert after retract, `:any-valid-time` with retraction, recursive rule retraction visibility at and before the retraction boundary.
- `tests/edge_cases_test.rs` — 4 integration tests covering: oversized-fact file-backed error path, `MAX_FACT_BYTES` exact boundary (accepted), `MAX_FACT_BYTES + 1` (rejected), in-memory database has no size limit.

## [0.7.0] - 2026-03-22

### Added
- Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page, ~25× disk space reduction vs v4
- LRU page cache (`src/storage/cache.rs`): configurable capacity (default 256 pages = 1MB)
- `OpenOptions::page_cache_size(usize)` — tune page cache capacity
- `CommittedFactReader` trait: index-driven fact resolution via page cache (no startup load-all)
- File format v5: `fact_page_format` header field; auto-migration from v4 on first open
- Page-based CRC32 checksum (v5): streams raw committed pages instead of all facts

### Changed
- `PersistentFactStorage::new()` takes `page_cache_capacity: usize` as second argument
- Committed facts no longer loaded into `Vec<Fact>` at startup; only pending facts held in memory
- `FactStorage::get_facts_by_entity`, `get_facts_by_attribute` use EAVT/AEVT index range scans

### Fixed
- v4 databases auto-migrated to v5 packed format on first open (no data loss)

## [0.6.0] - 2026-03-21

### Added
- Four Datomic-style covering indexes (EAVT, AEVT, AVET, VAET) with bi-temporal keys (`valid_from`, `valid_to` in all key tuples)
- `FactRef { page_id: u64, slot_index: u16 }` — forward-compatible disk location pointer (slot_index=0 in 6.1)
- Canonical value encoding (`encode_value`) with sort-order-preserving byte representation
- B+tree page serialization for index persistence (`src/storage/btree.rs`)
- `FileHeader` v4 (72 bytes): adds `eavt_root_page`, `aevt_root_page`, `avet_root_page`, `vaet_root_page` (4×8=32 bytes), `index_checksum` (u32), replacing the `reserved` field
- CRC32 sync check on open: index mismatch triggers automatic rebuild
- `FactStorage::replace_indexes()` and `index_counts()` for index lifecycle management
- Query optimizer (`src/query/datalog/optimizer.rs`): `IndexHint` enum, `select_index()`, `plan()` with selectivity-based join reordering
- Join reordering skipped under `wasm` feature flag
- `Cargo.toml` `[features]` section with `default = []` and `wasm = []`
- 6 integration tests in `tests/index_test.rs` for save/reload, bi-temporal, recursive rules regression

### Changed
- `FactStorage` internal structure: `FactData { facts, indexes }` under single `Arc<RwLock<FactData>>` for consistent snapshots
- `PersistentFactStorage::save()` writes index B+tree pages and updates header checksum
- `PersistentFactStorage::load()` performs sync check and fast-path index load
- `executor::execute_query()` now calls `optimizer::plan()` before pattern matching
- File format version bumped 3→4; automatic v1/v2/v3→v4 migration on first save
- `FORMAT_VERSION` constant updated to 4

### Fixed
- NaN values in `Value::Float` now canonicalize to a single bit pattern in index encoding (deterministic sort order)

## [0.5.0] - 2026-03-21

### Added
- Write-ahead log (WAL): fact-level sidecar `<db>.wal` with CRC32-protected binary entries
- `WriteTransaction` API: `begin_write()` / `commit()` / `rollback()` for explicit ACID transactions
- Crash recovery: WAL entries replayed on open; corrupt/partial entries discarded at first bad CRC32
- Checkpoint: `checkpoint()` flushes WAL facts to `.graph` and deletes the WAL; auto-checkpoint on configurable threshold
- `FileHeader` v3: `last_checkpointed_tx_count` field (repurposes unused `edge_count` slot)
- `FactStorage` helpers: `get_all_facts()`, `restore_tx_counter()`, `allocate_tx_count()`
- `OpenOptions` builder: `OpenOptions::new().path("db.graph").open()` or `Minigraf::in_memory()`
- `--file <path>` CLI flag for the REPL binary
- 41 new tests covering WAL, crash recovery, transactions, and checkpoint

### Changed
- `src/minigraf.rs` replaced by `src/db.rs` — `Minigraf`, `OpenOptions`, `WriteTransaction` public API
- File format version bumped 2→3; automatic v1/v2→v3 migration on first checkpoint
- REPL version string now tracks `CARGO_PKG_VERSION` automatically

### Fixed
- WAL-before-apply ordering: facts are now applied to in-memory state only after the WAL entry is fsynced, ensuring crash safety for both implicit (`execute()`) and explicit (`WriteTransaction`) write paths

## [0.4.0] - 2026-03-21

### Added
- Bi-temporal support: every fact now carries transaction time (`tx_id`, `tx_count`)
  and valid time (`valid_from`, `valid_to`)
- `:as-of N` query modifier for transaction time travel (counter or ISO 8601 timestamp)
- `:valid-at "date"` query modifier for valid time point-in-time queries
- `:valid-at :any-valid-time` to disable valid time filtering
- `(transact {:valid-from ... :valid-to ...} [...])` syntax for specifying valid time
- Per-fact valid time override in transact (4-element fact vectors with metadata map)
- File format version 2 with automatic migration from version 1

### Changed
- **Breaking behaviour**: queries without `:valid-at` now return only currently valid
  facts (`valid_from <= now < valid_to`). Existing Phase 3 databases are unaffected
  because all migrated facts have `valid_to = MAX`.
- `FactStorage::transact()` now accepts an optional `TransactOptions` parameter

### Fixed
- `PersistentFactStorage::load()` previously discarded original `tx_id` when loading
  facts from disk, making time-travel queries on persisted databases incorrect

## [0.3.0] - 2026-03-10

### Added
- Datalog core implementation with recursive rules
- Entity-Attribute-Value (EAV) data model
- Pattern matching with variable unification
- Semi-naive evaluation for recursive rules
- Transitive closure support with cycle handling
- Rule registry for rule management
- Persistent storage with postcard serialization
- REPL with multi-line command support and comments
- 123 comprehensive tests (94 unit + 26 integration + 3 doc)

### Changed
- Replaced GQL-inspired syntax with Datalog EDN syntax
- Data model changed from property graph to EAV triples
- Query executor rewritten for Datalog pattern matching

## [0.2.0] - 2026-02-01

### Added
- Persistent storage backend with `.graph` file format
- StorageBackend trait for platform abstraction
- FileBackend implementation (4KB pages, cross-platform)
- MemoryBackend for testing
- PersistentGraphStorage layer for serialization
- Embedded API (`Minigraf::open()`, `Minigraf::execute()`)
- Auto-save on drop

### Changed
- Graph storage now supports persistence

## [0.1.0] - 2026-01-15

### Added
- Initial release
- In-memory property graph implementation
- Basic graph operations (nodes, edges, properties)
- Interactive REPL
- Thread-safe storage with `Arc<RwLock<>>`
