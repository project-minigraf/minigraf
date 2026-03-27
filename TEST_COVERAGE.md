# Minigraf Test Coverage Report

**Last Updated**: Phase 7.4 COMPLETE - `filter_facts_for_query` snapshot fix ✅

## Test Summary

**Total Tests**: 568 ✅
- ✅ 390 unit tests (lib)
- ✅ 10 bi-temporal tests (integration)
- ✅ 10 complex query tests (integration)
- ✅ 9 recursive rules tests (integration)
- ✅ 7 concurrency tests (integration)
- ✅ 12 WAL / crash recovery tests (integration)
- ✅ 6 index tests (integration, Phase 6.1)
- ✅ 7 performance / packed page tests (integration, Phase 6.2)
- ✅ 7 retraction tests (integration, Phase 6.4a)
- ✅ 4 edge case tests (integration, Phase 6.4a)
- ✅ 8 B+tree v6 tests (integration, Phase 6.5)
- ✅ 10 negation (`not`) tests (integration, Phase 7.1a)
- ✅ 14 not-join tests (integration, Phase 7.1b)
- ✅ 24 aggregation tests (integration, Phase 7.2a)
- ✅ 28 predicate expression tests (integration, Phase 7.2b)
- ✅ 16 disjunction tests (integration, Phase 7.3)
- ✅ 6 doc tests

**Status**: ✅ **All 568 tests passing**

## Phase 7.4 Completion Status: ✅ COMPLETE

**Phase 7.4 Features** (current, complete):
- ✅ `filter_facts_for_query` returns `Arc<[Fact]>` — eliminates O(N) four-BTreeMap index rebuild on every non-rules query call
- ✅ `execute_query` path constructs zero `FactStorage` objects; `execute_query_with_rules` still converts for `StratifiedEvaluator`
- ✅ `PatternMatcher::from_slice(Arc<[Fact]>)` constructor added
- ✅ `apply_or_clauses` and `evaluate_not_join` signatures updated to accept `Arc<[Fact]>`
- ✅ Evaluator loop: `accumulated_facts` computed once per iteration (was 4 separate `get_asserted_facts()` calls)
- ✅ ~62–65% speedup on non-rules queries at 10K facts (`query/point_entity/10k`: 22 ms → 8.6 ms; `aggregation/count_scale/10k`: 28 ms → 9.7 ms)
- ✅ 4 new unit tests in `matcher.rs`, 2 new unit tests in `executor.rs` (6 total)
- ✅ Version bumped to v0.13.1

## Phase 7.3 Completion Status: ✅ COMPLETE

**Phase 7.3 Features** (current, complete):
- ✅ `WhereClause::Or(Vec<Vec<WhereClause>>)` and `WhereClause::OrJoin { join_vars, branches }` variants in `types.rs`
- ✅ `(or branch1 branch2 ...)` and `(or-join [?v...] branch1 branch2 ...)` in `:where` clauses and rule bodies
- ✅ `(and ...)` grouping clause to collect multiple clauses into a single branch
- ✅ `match_patterns_seeded` on `PatternMatcher`; `evaluate_branch` and `apply_or_clauses` helpers in `executor.rs`
- ✅ `DependencyGraph::from_rules` refactored with recursive `collect_clause_deps`; `Or`/`OrJoin` branches contribute positive dependency edges
- ✅ Rules with `or`/`or-join` in bodies routed to `mixed_rules` path in `StratifiedEvaluator`
- ✅ `tests/disjunction_test.rs`: 16 integration tests (Phase 7.3)
- ✅ Version bumped to v0.13.0

**Core Features Implemented** (Phase 6.2):
- ✅ Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page (~25× space reduction)
- ✅ LRU page cache (`cache.rs`): approximate-LRU, read-lock on hits, `Arc<Vec<u8>>` entries
- ✅ `CommittedFactReader` trait + `CommittedFactLoaderImpl`: on-demand fact resolution
- ✅ Pending `FactRef` (`page_id = 0`): resolves to in-memory pending facts vec
- ✅ `FileHeader` v5: `fact_page_format` byte (0x02 = packed); auto v4→v5 migration on open
- ✅ `OpenOptions::page_cache_size(usize)` builder method (default 256)
- ✅ EAVT/AEVT range scans in `get_facts_by_entity` / `get_facts_by_attribute`

**Phase 6.1 Features** (also complete):
- ✅ EAVT, AEVT, AVET, VAET covering indexes with bi-temporal keys
- ✅ `FactRef { page_id, slot_index }`: forward-compatible disk location pointer
- ✅ Canonical value encoding (`encode_value`) for sort-order-preserving comparisons
- ✅ B+tree page serialisation for index persistence (`btree.rs`)
- ✅ `FileHeader` v4: `eavt/aevt/avet/vaet_root_page` + `index_checksum` (CRC32)
- ✅ Auto-rebuild on checksum mismatch
- ✅ Query optimizer: `IndexHint`, `select_index()`, selectivity-based `plan()`

**Phase 5 Features** (also complete):
- ✅ Fact-level sidecar WAL (`<db>.wal`) with CRC32-protected binary entries
- ✅ WAL-before-apply ordering: WAL fsynced before facts touch in-memory state
- ✅ `FileHeader` v3 with `last_checkpointed_tx_count` (replay deduplication)
- ✅ `WriteTransaction` API (`begin_write`, `commit`, `rollback`)
- ✅ Crash recovery: WAL replay on open, corrupt entries discarded at first bad CRC32
- ✅ Checkpoint: WAL flushed to `.graph` file, then WAL cleared
- ✅ Thread-safe: concurrent readers + exclusive writer (Mutex + RwLock)
- ✅ File format v2→v3 migration on first checkpoint

**Phase 4 Features** (also complete):
- ✅ EAV data model with `tx_count`, `valid_from`, `valid_to` fields
- ✅ `VALID_TIME_FOREVER = i64::MAX` sentinel
- ✅ `FactStorage` temporal query methods (`get_facts_as_of`, `get_facts_valid_at`)
- ✅ Parser: EDN maps, `:as-of`, `:valid-at`, per-fact valid time overrides
- ✅ Executor: 3-step temporal filter (tx-time → asserted → valid-time)
- ✅ File format v1→v2 migration
- ✅ UTC-only timestamp parsing (chrono, avoids GHSA-wcg3-cvx6-7396)

**Phase 7.2b Features** (also complete):
- ✅ `BinOp` (14 variants), `UnaryOp` (5 variants), `Expr` AST, `WhereClause::Expr { expr, binding }` in `types.rs`
- ✅ Filter predicates: `[(< ?age 30)]`, `[(string? ?v)]`, `[(starts-with? ?tag "work")]`, `[(matches? ?email "...")]`
- ✅ Arithmetic bindings: `[(+ ?price ?tax) ?total]`, nested `[(+ (* ?a 2) ?b) ?result]`, type-predicate binding `[(integer? ?v) ?is-int]`
- ✅ `parse_expr` / `parse_expr_clause` with parse-time regex validation; `check_expr_safety` recurses into `not`/`not-join` bodies
- ✅ Dispatch at all 4 clause sites; `outer_vars_from_clause` updated for binding variable scope
- ✅ `eval_expr` / `eval_binop` / `is_truthy` / `apply_expr_clauses` in `executor.rs`; `apply_expr_clauses_in_evaluator` in `evaluator.rs`
- ✅ `tests/predicate_expr_test.rs`: 28 integration tests (Phase 7.2b)
- ✅ Version bumped to v0.12.0

**Phase 7.2a Features** (also complete):
- ✅ `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max` aggregate functions in `:find` clause
- ✅ `:with` grouping clause — variables that participate in grouping but are excluded from output rows
- ✅ `AggFunc` enum, `FindSpec` enum; `DatalogQuery.find` migrated from `Vec<String>` to `Vec<FindSpec>`
- ✅ `apply_aggregation` post-processing in `executor.rs`; parse-time validation
- ✅ `tests/aggregation_test.rs`: 24 integration tests (Phase 7.2a)
- ✅ Version bumped to v0.11.0

**Phase 7.1 Features** (also complete):
- ✅ `src/query/datalog/stratification.rs`: `DependencyGraph`, `stratify()` — negative dependency edges + Bellman-Ford cycle detection; negative cycles rejected at rule registration time
- ✅ `WhereClause::Not(Vec<WhereClause>)` and `WhereClause::NotJoin { join_vars, clauses }` variants; all match arms updated
- ✅ `(not clause…)` — stratified negation; all body variables must be pre-bound by outer clauses
- ✅ `(not-join [?v…] clause…)` — existentially-quantified negation; only `join_vars` are shared from outer scope; remaining body variables are fresh
- ✅ Safety validation at parse time (unbound join vars → parse error; nesting constraint enforced)
- ✅ `StratifiedEvaluator`: stratifies rules, evaluates strata in order; `not`/`not-join` filters applied per binding in mixed-rule strata
- ✅ `evaluate_not_join` free function: handles both `Pattern` and `RuleInvocation` body clauses
- ✅ `tests/negation_test.rs`: 10 integration tests (Phase 7.1a)
- ✅ `tests/not_join_test.rs`: 14 integration tests (Phase 7.1b)
- ✅ Version bumped to v0.10.0

**Phase 6.5 Features** (also complete):
- ✅ `src/storage/btree_v6.rs`: proper on-disk B+tree with `build_btree` bulk-load and `range_scan` leaf-chain traversal
- ✅ `OnDiskIndexReader` + `CommittedIndexReader` trait: page-cache-backed index lookup; no full in-memory BTreeMap
- ✅ `MutexStorageBackend<B>`: backend mutex held per page read; cache-warm pages require no lock
- ✅ FileHeader v6 (80 bytes): adds `fact_page_count u64` field at bytes 72–80; auto v5→v6 migration
- ✅ `tests/btree_v6_test.rs`: 8 integration tests (B+tree insert/scan, multi-page, concurrent correctness, v5→v6 migration)
- ✅ `test_concurrent_range_scans_correctness` unit test: 8 barrier-synchronised threads, all return identical results
- ✅ Version bumped to v0.9.0; BENCHMARKS.md updated with v6 results

**Phase 6.4b Features** (also complete):
- ✅ Criterion benchmark suite at 1K–1M facts; results documented in `BENCHMARKS.md`
- ✅ heaptrack memory profiles: 10K=14.4MB / 100K=136MB / 1M=1.33GB peak heap (v5 baseline)
- ✅ Byte-layout unit tests pin all FileHeader v5 field offsets (`src/storage/mod.rs`)
- ✅ Byte-layout unit tests pin packed page header + record directory offsets (`src/storage/packed_pages.rs`)
- ✅ Dead `clap` dependency removed; `Cargo.toml` metadata complete; version bumped to v0.8.0
- ✅ README trimmed (794 → 166 lines); detail offloaded to GitHub wiki

**Phase 6.4a Features** (also complete):
- ✅ Retraction semantics fix: `net_asserted_facts()` computes net view per EAV triple in `filter_facts_for_query`
- ✅ `check_fact_sizes()` early validation in `db.rs`: rejects oversized facts before WAL write
- ✅ `MAX_FACT_BYTES` constant (`packed_pages.rs`): 4 080 bytes — maximum serialised size per fact
- ✅ 7 new retraction integration tests (`tests/retraction_test.rs`)
- ✅ 4 new edge case integration tests (`tests/edge_cases_test.rs`)

**Phase 3 Features** (also complete):
- ✅ Datalog parser (EDN syntax)
- ✅ Pattern matching with variable unification
- ✅ Query executor (transact, retract, query)
- ✅ Recursive rules with semi-naive evaluation
- ✅ Transitive closure queries
- ✅ Persistent storage (postcard serialization)
- ✅ REPL with multi-line and comment support

---

## Test Coverage by Module

### 1. Graph Types (`src/graph/types.rs`) - ✅ Excellent (8 tests)

- ✅ Fact creation, equality, retraction, entity references
- ✅ Transaction ID generation and ordering
- ✅ `VALID_TIME_FOREVER` sentinel, `with_valid_time()`, `TransactOptions`
- ✅ All `Value` types (String, Integer, Float, Boolean, Ref, Keyword, Null)

**Coverage**: ~95%

### 2. Fact Storage (`src/graph/storage.rs`) - ✅ Excellent (18+ tests)

**Core Operations**:
- ✅ Transact, retract, batch transact
- ✅ Get facts by entity/attribute, history tracking

**Phase 4 (Bi-temporal)**:
- ✅ `tx_count` increments, `get_facts_as_of()`, `get_facts_valid_at()`
- ✅ `load_fact()` preserves original `tx_id`/`tx_count`

**Phase 5 (WAL helpers)**:
- ✅ `get_all_facts()`, `restore_tx_counter()`, `allocate_tx_count()`

**Phase 6.1-6.2 (Index + CommittedFactReader)**:
- ✅ `set_committed_reader()` wires CommittedFactReader
- ✅ `get_facts_by_entity()` uses EAVT range scan
- ✅ `get_facts_by_attribute()` uses AEVT range scan
- ✅ `FactRef { page_id: 0 }` resolved to pending facts; `page_id >= 1` via CommittedFactReader
- ✅ MockLoader in tests verifies committed path

**Coverage**: ~93%

### 3. WAL (`src/wal.rs`) - ✅ Excellent (8 unit tests)

- ✅ Empty WAL round-trip
- ✅ Single-fact and multi-fact entry round-trips
- ✅ Reopen-and-append
- ✅ Bad magic header rejected
- ✅ Truncated entry stops replay (partial write discard)
- ✅ `delete_file()` removes WAL

**Coverage**: ~97%

### 4. Database API (`src/db.rs`) - ✅ Excellent (12 unit tests)

- ✅ In-memory transact and query round-trip
- ✅ Explicit `WriteTransaction` commit and rollback
- ✅ `build_query_view()` read-your-own-writes within transaction
- ✅ Reentrant `begin_write()` returns error
- ✅ File-backed open, transact, reopen (persistence)
- ✅ WAL written before in-memory apply
- ✅ Auto-checkpoint threshold fires, `checkpoint()` manual trigger
- ✅ Concurrent `execute()` during active `WriteTransaction`

**Coverage**: ~93%

### 5. Covering Indexes (`src/storage/index.rs`) - ✅ Excellent (11 tests)

- ✅ `FactRef` field access
- ✅ `encode_value` sort order: integers, cross-type, floats, NaN canonicalization
- ✅ EAVT key ordering by entity
- ✅ AVET key ordering by value bytes
- ✅ VAET only populated for `Value::Ref`
- ✅ `Indexes::insert` populates all four indexes

**Coverage**: ~98%

### 6. B+tree Persistence (`src/storage/btree.rs`) - ✅ Good (4 tests)

- ✅ Empty EAVT roundtrip (exactly 1 page)
- ✅ Small EAVT roundtrip (10 entries)
- ✅ Large EAVT roundtrip (150 entries, multi-page)
- ✅ Sort order preserved after serialise/deserialise

**Coverage**: ~90%

### 7. LRU Page Cache (`src/storage/cache.rs`) - ✅ Good (6 tests)

- ✅ Cache miss loads from backend
- ✅ Cache hit returns same `Arc` without backend read
- ✅ LRU eviction evicts correct (oldest) page
- ✅ `put_dirty` / `flush` writes back to backend
- ✅ `invalidate` removes entry
- ✅ `cached_page_count` reports correctly

**Coverage**: ~92%

### 8. Packed Pages (`src/storage/packed_pages.rs`) - ✅ Good (8 tests)

- ✅ Single fact pack/unpack roundtrip
- ✅ Multiple facts pack/unpack roundtrip
- ✅ Correct `FactRef` slot assignments
- ✅ Oversized fact returns `Err` (not panic)
- ✅ `read_all_from_pages` with known page IDs
- ✅ Wrong page type returns `Err`
- ✅ **Byte-layout pin**: page header (bytes 0–11) field positions verified (Phase 6.4b)
- ✅ **Byte-layout pin**: record directory entries at byte 12+ verified (Phase 6.4b)

**Coverage**: ~93%

### 9. FileHeader (`src/storage/mod.rs`) - ✅ Excellent (10 tests)

- ✅ v6 serialisation: 80 bytes, correct field offsets
- ✅ v6 roundtrip with all index root pages, checksum, and `fact_page_count`
- ✅ v3/v4/v5 headers accepted with appropriate zero-filling
- ✅ v6 header with <80 bytes rejected
- ✅ Header validation (magic, version range 1-6)
- ✅ Version 0 and 7 rejected
- ✅ `FORMAT_VERSION == 6`
- ✅ **Byte-layout pin**: all 11 fields at exact offsets with LE encoding verified (Phase 6.4b / v6 update)

**Coverage**: ~98%

### 10. Datalog Parser (`src/query/datalog/parser.rs`) - ✅ Excellent (25 tests)

- ✅ All tokens, numbers, strings, booleans, UUIDs, nil
- ✅ Transact/Retract/Query/Rule commands
- ✅ `:as-of` (counter + ISO 8601 timestamp)
- ✅ `:valid-at` (timestamp + `:any-valid-time`)
- ✅ EDN map `{:key val}` with transaction-level valid time
- ✅ Per-fact valid time override (4-element fact vector)
- ✅ Reject negative `:as-of` counter and invalid timestamps

**Coverage**: ~98%

### 11. Datalog Types, Matcher, Executor, Rules, Evaluator - ✅ Good-Excellent

- Types: ~95% (7 tests)
- Matcher: ~85% (6 tests)
- Executor: ~94% (18 tests) — including temporal filter and optimizer integration
- Rule Registry: ~95% (6 tests)
- Recursive Evaluator: ~95% (10 tests)

### 12. Storage Backends (`src/storage/backend/`) - ✅ Good (8 tests)

- ✅ FileBackend create/write/read, persistence across close/reopen
- ✅ MemoryBackend write/read, error handling

**Coverage**: ~85%

### 13. Temporal (`src/temporal.rs`) - ✅ Good

- ✅ UTC timestamp parsing and formatting
- ✅ Chrono CVE GHSA-wcg3-cvx6-7396 avoidance verified

**Coverage**: ~90%

---

## Integration Tests

### Complex Queries (`tests/complex_queries_test.rs`) - ✅ 10 tests

- ✅ 3-pattern and 4-pattern joins, self-joins, entity reference joins
- ✅ No results, partial matches, variable reuse, multiple values, empty database

### Recursive Rules (`tests/recursive_rules_test.rs`) - ✅ 9 tests

- ✅ Transitive closure, cycles, long chains, diamond patterns
- ✅ Ancestor/descendant, family trees, multiple recursive predicates

### Concurrency (`tests/concurrency_test.rs`) - ✅ 7 tests

- ✅ Concurrent rule registration (5 threads), concurrent queries with rules (10 threads)
- ✅ Read-heavy workload (50 threads), recursive evaluation concurrency
- ✅ No deadlocks (20 threads mixed), RwLock consistency (10 writers + 10 readers)

### Bi-temporal (`tests/bitemporal_test.rs`) - ✅ 10 tests

- ✅ As-of counter and timestamp snapshots
- ✅ Valid-at inside/outside/boundary, default filter, any-valid-time
- ✅ Combined bi-temporal (both dimensions), multi-entity valid ranges

### WAL / Crash Recovery (`tests/wal_test.rs`) - ✅ 12 tests

- ✅ Basic persistence (file-backed transact and query)
- ✅ WAL replay after `mem::forget` crash simulation
- ✅ Stale WAL dedup via `last_checkpointed_tx_count`
- ✅ Corrupt/partial entry discard on recovery
- ✅ Manual checkpoint clears WAL and updates header
- ✅ Auto-checkpoint fires at threshold
- ✅ Explicit transaction crash safety and rollback
- ✅ Multi-transact rollback leaves no trace
- ✅ Concurrent reads while writer holds exclusive lock
- ✅ Implicit `execute()` WAL ordering verified
- ✅ v2→v3 format migration on checkpoint

### Covering Indexes (`tests/index_test.rs`) - ✅ 6 tests (Phase 6.1)

- ✅ EAVT/AEVT/AVET/VAET save and reload roundtrip
- ✅ Bi-temporal queries still correct after index save/reload
- ✅ Recursive rules regression (indexes don't break rule evaluation)
- ✅ Index checksum mismatch triggers rebuild
- ✅ v3→v4 format migration on first save

### Packed Pages / Performance (`tests/performance_test.rs`) - ✅ 7 tests (Phase 6.2)

- ✅ 1K facts correct after packed save/reload
- ✅ Packed file size < one-per-page estimate (compactness check)
- ✅ Bitemporal query correct after packed reload
- ✅ As-of query correct after packed reload
- ✅ Recursive rules unchanged after Phase 6.2
- ✅ Explicit transaction survives packed reload
- ✅ `page_cache_size` option accepted without panic

### Retraction Semantics (`tests/retraction_test.rs`) - ✅ 7 tests (Phase 6.4a)

- ✅ Assert then retract; current-time query returns no results
- ✅ Assert at tx=1, retract at tx=3; `:as-of 2` shows fact, `:as-of 4` hides it
- ✅ Assert, retract, re-assert; current-time query returns fact
- ✅ Retraction + `:any-valid-time` combo
- ✅ Recursive rule: retracted edge not traversed (`:as-of` after retraction)
- ✅ Recursive rule: retracted edge is visible in historical snapshot before retraction
- ✅ Multiple retractions for different entities in same transaction

### Edge Cases (`tests/edge_cases_test.rs`) - ✅ 4 tests (Phase 6.4a)

- ✅ Oversized fact in file-backed database returns `Err`, not panic
- ✅ In-memory database accepts facts of any size (no size limit)
- ✅ Fact at exactly `MAX_FACT_BYTES` is accepted
- ✅ Fact at `MAX_FACT_BYTES + 1` is rejected with clear error message

### B+Tree v6 (`tests/btree_v6_test.rs`) - ✅ 8 tests (Phase 6.5)

- ✅ Single-page B+tree insert and range scan correctness
- ✅ Multi-page B+tree (leaf chain traversal across multiple pages)
- ✅ Range scan with exclusive upper bound
- ✅ Empty range scan returns empty result
- ✅ Concurrent range scans — 8 barrier-synchronised threads all return identical results
- ✅ v5 database opens and migrates to v6 on first checkpoint
- ✅ v6 database survives close/reopen with correct fact count
- ✅ Index lookup via `OnDiskIndexReader` returns correct `FactRef`s

### Negation — `not` (`tests/negation_test.rs`) - ✅ 10 tests (Phase 7.1a)

- ✅ Basic `not` — exclude entities where a pattern matches
- ✅ `not` with multi-clause body
- ✅ `not` in a rule body (stratification + derived negation)
- ✅ `not` with `:as-of` time travel
- ✅ `not` with `:valid-at`
- ✅ Negative cycle via `not` at rule registration → `Err`, rule not registered
- ✅ `not` where no entities match the body — all outer bindings survive
- ✅ Safety check: unbound variable in `not` body → parse error
- ✅ Nested `not` rejected at parse time
- ✅ `not` with `RuleInvocation` in body — derived rule facts correctly negated end-to-end

### Not-Join (`tests/not_join_test.rs`) - ✅ 14 tests (Phase 7.1b)

- ✅ Basic `not-join` — exclude entities where existentially-quantified dependency exists
- ✅ Multiple join variables in `not-join`
- ✅ Multi-clause body with a local variable linking inner patterns
- ✅ `not-join` in a rule body
- ✅ Multi-stage filtering chain (two independent `not-join` rules applied progressively)
- ✅ `not-join` vs `not` semantic difference (inner-only variable)
- ✅ `not-join` with `:as-of` time travel
- ✅ Unbound join variable → parse error naming the variable
- ✅ Nested `not-join` rejected at parse time
- ✅ `RuleInvocation` in `not-join` body — derived facts correctly negated end-to-end
- ✅ No-match survival — when no entity satisfies the body, all outer bindings survive
- ✅ `not-join` with `:valid-at`
- ✅ Negative cycle via `not-join` at rule registration → `Err`, rule not registered
- ✅ `not` and `not-join` coexist in the same query

---

## Coverage Metrics

**Overall Code Coverage**: ~94% (estimate)

**By Category**:
- ✅ Happy path: ~98%
- ✅ Core Datalog operations: ~95%
- ✅ Recursive rules: ~95%
- ✅ Bi-temporal queries: ~95%
- ✅ WAL and crash recovery: ~94%
- ✅ Transaction API: ~93%
- ✅ Covering indexes: ~94%
- ✅ Packed pages + LRU cache: ~91%
- ✅ Error handling: ~84% (raised from ~82% via edge case tests)
- ✅ Edge cases: ~90% (raised from ~87% via edge case + retraction tests)
- ✅ Concurrency: ~92%
- ✅ Performance benchmarks: Criterion suite run at 1K–1M facts; documented in `BENCHMARKS.md` (Phase 6.4b)

---

## What's Thoroughly Tested ✅

### Phase 3 Core Features
1. Datalog Core — Transact, retract, query
2. Pattern Matching — Variable unification, multi-pattern joins
3. Fact Storage — EAV model, history, retractions
4. EDN Parsing — All Datalog syntax variations
5. Storage Backends — File and memory persistence
6. Recursive Rules — Semi-naive evaluation, fixed-point iteration
7. Transitive Closure — Multi-hop reachability
8. Cycle Handling — Graphs with cycles converge correctly
9. Complex Queries — 3+ patterns, self-joins, entity references
10. Concurrency — Thread-safe rule registration and querying

### Phase 4 Bi-temporal Features
11. Transaction Time — `tx_count` increments, `get_facts_as_of()` snapshots
12. Valid Time — `valid_from`/`valid_to` filtering, boundary semantics
13. Time Travel Queries — `:as-of` counter and timestamp
14. Valid-at Queries — Point-in-time filter, `:any-valid-time`
15. Combined Bi-temporal — Both dimensions in one query
16. Transact with Valid Time — Batch-level and per-fact overrides
17. File Format Migration — v1→v2 with correct temporal defaults

### Phase 5 ACID + WAL Features
18. WAL Format — CRC32-protected entries, partial-write discard
19. Crash Recovery — WAL replay on open, dedup via `last_checkpointed_tx_count`
20. Explicit Transactions — `begin_write` / `commit` / `rollback`
21. WAL Ordering — WAL fsynced before in-memory apply (both implicit and explicit paths)
22. Checkpoint — WAL flushed to `.graph`, WAL deleted, header updated
23. Auto-checkpoint — Fires at configurable WAL entry threshold
24. Thread Safety — Concurrent readers + exclusive writer verified with Barrier

### Phase 6.1 Index Features
25. EAVT/AEVT/AVET/VAET — Four covering indexes with bi-temporal keys
26. FactRef — Disk location pointer, slot_index always 0 in 6.1
27. Value Encoding — Sort-order-preserving canonical encoding
28. B+tree Persistence — Multi-page blob strategy, sort order preserved
29. FileHeader v4 — Index root pages, CRC32 checksum
30. Index Rebuild — Triggered by checksum mismatch on open
31. Query Optimizer — Index hint selection, join reordering by selectivity

### Phase 6.2 Packed Page + Cache Features
32. Packed Pages — ~25 facts/page, header + directory + records layout
33. FactRef Semantics — `page_id=0` = pending, `page_id>=1` = committed via cache
34. CommittedFactReader — Trait + impl wired in PersistentFactStorage::load()
35. LRU Page Cache — Read-lock on hits, Arc cloning, eviction correctness
36. v4→v5 Migration — Reads one-per-page, repacks, saves with new format
37. EAVT/AEVT Range Scans — O(log n) entity and attribute lookups

### Phase 6.4a Retraction Semantics + Edge Cases
38. Retraction Net View — `net_asserted_facts()` groups by EAV triple, keeps highest `tx_count`
39. Current-Time Retraction — Retracted fact absent from query results with no `:as-of`
40. As-Of Retraction — Retraction visible/invisible at correct tx boundary
41. Re-Assert After Retract — Fact reappears when re-asserted
42. Retraction in Recursive Rules — Retracted edges not traversed in rule derivation
43. Oversized-Fact Early Validation — `check_fact_sizes()` rejects before WAL write
44. `MAX_FACT_BYTES` Boundary — Exact-size accepted, +1 rejected with clear error

### Phase 6.4b Byte-Layout Pins
45. FileHeader v5 Field Offsets — All 10 fields pinned at exact byte positions (big-endian detection coverage)
46. Packed Page Header Layout — page_type, reserved, record_count u16 LE, next_page u64 LE at bytes 0–11
47. Packed Page Record Directory — (offset u16 LE, length u16 LE) per slot, starting at byte 12

### Phase 6.5 On-Disk B+Tree Indexes
48. B+Tree Build + Range Scan — `build_btree` inserts and `range_scan` retrieves with correct ordering
49. Multi-Page Leaf Chain — range scan correctly follows `next_leaf` pointers across page boundaries
50. Concurrent Range Scans — 8 barrier-synchronised threads, all return identical non-empty results
51. v5→v6 Migration — database opened from v5 format migrates to v6 on first checkpoint
52. `OnDiskIndexReader` FactRef Lookup — committed facts resolved correctly via page cache
53. `MutexStorageBackend` — cache-warm pages acquire no backend lock; cache-cold pages lock briefly

### Phase 7.1 Stratified Negation
54. `not` — basic absence query excludes entities where pattern matches
55. `not` in rule body — stratified mixed-rule evaluation applies negation per binding
56. `not-join` — existentially-quantified exclusion with explicit join variables
57. `not-join` multi-clause body — inner variables link patterns without escaping to outer scope
58. `not-join` in rule body — negation inside derived rules
59. Negative cycle rejection — `not` / `not-join` creating a dependency cycle → `Err` at registration, rule not added
60. Safety validation — unbound variables in `not` body or `join_vars` → parse error with variable name
61. Nesting constraint — `not-join` inside `not` or `not-join` → parse error
62. `RuleInvocation` in `not-join` body — derived facts in accumulated store correctly negated
63. Time travel with negation — `not-join` respects `:as-of` and `:valid-at` temporal filters
64. `not` and `not-join` coexistence in the same query

---

## What's Not Tested Yet ⏳

### Phase 7.3+ (Remaining Datalog Completeness)
- ⏳ Disjunction (`or` / `or-join`) — Phase 7.3
- ⏳ Query optimizer improvements for new clause types (aggregation, expr, disjunction) — Phase 7.4
- ⏳ Prepared statements with temporal bind slots — Phase 7.6
- ⏳ Temporal metadata pseudo-attributes (`:db/valid-from`, `:db/valid-to`, `:db/tx-count`) — Phase 7.7

### Known Limitations (Acceptable for Phase 3-7.2b)
- ⏳ Crash during checkpoint write (safe by construction — WAL not deleted until save succeeds; explicit test deferred to Phase 7.5)
- ⏳ Disjunction — Phase 7.3
- ⏳ Known `not-join` limitation: when a rule B positively invokes rule A and both are stratum 0, single-pass mixed-rule evaluation means B may not see A's derived facts unless rules are declared in dependency order
- ⏳ `matches?` pattern compiled per-row (no caching); will be optimised in Phase 7.9b (`FunctionRegistry`)

---

## Test Execution

```bash
# Run all tests
cargo test

# Run tests quietly with summary
cargo test --quiet

# Run specific test suites
cargo test --lib                       # Unit tests (216)
cargo test --test bitemporal           # Bi-temporal (10)
cargo test --test complex_queries      # Complex queries (10)
cargo test --test recursive_rules      # Recursive rules (9)
cargo test --test concurrency          # Concurrency (7)
cargo test --test wal_test             # WAL / crash recovery (12)
cargo test --test index_test           # Covering indexes (6)
cargo test --test performance_test     # Packed pages (7)
cargo test --test retraction_test      # Retraction semantics (7)
cargo test --test edge_cases_test      # Edge cases (4)
cargo test --test btree_v6_test        # B+tree v6 (8)
cargo test --test negation_test        # stratified not (10)
cargo test --test not_join_test        # not-join (14)
cargo test --test aggregation_test     # aggregation (24)
cargo test --test predicate_expr_test  # arithmetic & predicate expr (28)

# Run with output
cargo test -- --nocapture
```

---

## Conclusion

**Phase 7.2b Status**: ✅ **COMPLETE**

**Test Quality**: ✅ **Excellent** — High confidence in all Phase 3-7.2b features

**Strengths**:
- WAL crash safety verified with real `mem::forget` simulation
- Both implicit and explicit transaction write paths verified
- Thread safety proven with Barrier-synchronized concurrent tests
- Index persistence and CRC32 sync check verified
- Packed page compactness verified against one-per-page estimate
- CommittedFactReader wiring verified with MockLoader in unit tests
- Retraction semantics verified across current-time, as-of, and recursive-rule queries
- Oversized-fact early rejection verified for file-backed databases
- Criterion benchmarks validated performance at 1K–1M facts
- Byte-layout tests pin FileHeader v5/v6 and packed page header field offsets
- On-disk B+tree correctness and concurrent scan safety verified (Phase 6.5)
- Stratified negation (`not` / `not-join`) verified: safety validation, stratification, negative cycle rejection, time-travel integration (Phase 7.1)
- Aggregation verified: all 6 aggregate functions, `:with` grouping, bi-temporal + aggregate, rule + aggregate (Phase 7.2a)
- Arithmetic & predicate expressions verified: all operators, silent-drop semantics, int/float promotion, regex validation, expr in not/rule body, bi-temporal + expr (Phase 7.2b)
- Disjunction (`or` / `or-join`) verified: flat queries, rule bodies, nested or/not/expr, or-join with private variables, dependency graph (Phase 7.3)
- 562 tests covering all Phase 3-7.3 features

**Confidence Level**: ✅ **Production-ready for Phase 7.3 scope**

**Readiness for Phase 7.4**: ✅ **Ready to proceed**

The disjunction + aggregation + arithmetic/predicate expression capable, stratified-negation-capable, on-disk B+tree indexed, packed, cached bi-temporal Datalog engine is **solid, well-tested, and benchmarked**.

---

**Next Steps**: Begin Phase 7.3 (Disjunction — `or` / `or-join`) 🚀
