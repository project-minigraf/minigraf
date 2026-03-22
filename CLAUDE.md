# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable **bi-temporal graph database with Datalog queries** written in Rust. It's designed to be the embedded graph memory layer for AI agents, mobile apps, and the browser — built on the SQLite philosophy: embedded, single-file, reliable, with time travel capabilities.

**Current Status: Phase 6.2 COMPLETE ✅ → Phase 6.4 Next** - Packed Pages + LRU Cache (note: Phase 6.3 query optimization was completed as part of Phase 6.1):
- ✅ Phase 1: Property graph PoC (in-memory)
- ✅ Phase 2: Persistent storage (`.graph` file format, embedded API)
- ✅ Phase 3: Datalog core (EAV model, recursive rules) - COMPLETE!
- ✅ **Phase 4: Bi-temporal support (transaction time + valid time) - COMPLETE!**
- ✅ **Phase 5: ACID + WAL (crash safety, explicit transactions) - COMPLETE!**
- ✅ **Phase 6.1: Covering indexes (EAVT, AEVT, AVET, VAET) + query optimizer - COMPLETE!**
- ✅ **Phase 6.2: Packed pages + LRU page cache - COMPLETE!**
- 🎯 Phase 6.4: Benchmarks + edge case tests + crates.io publish - **NEXT** (Phase 6.3 query optimization done in 6.1)
- 🎯 Phase 7: Datalog Completeness (negation, aggregation, disjunction; ≥90% branch coverage target)
- 🎯 v1.0.0: 9-12 months

## Core Philosophy - CRITICAL

**Before implementing ANY feature or change, you MUST assess it against the design philosophy in PHILOSOPHY.md.**

Minigraf follows the "SQLite for bi-temporal graph databases" philosophy:
- **Zero-configuration** - No setup, no config files, just works
- **Embedded-first** - Library not server, in-process execution
- **Single-file database** - One portable `.graph` file
- **Self-contained** - Minimal dependencies, small binary (<1MB goal)
- **Cross-platform** - Native, WASM, mobile, embedded
- **Reliability over features** - Do less, do it perfectly
- **Bi-temporal first-class** - Time travel is a core feature, not addon
- **Datalog queries** - Simpler, more powerful for graphs than SQL/GQL
- **Stability** - Backwards compatibility, stable file format
- **Long-term support** - Decades-long commitment

### Why Datalog?

Datalog is the right choice for Minigraf:
1. **Simpler** - ~50 pages of core concepts
2. **Proven** - 40+ years, Datomic/XTDB production use
3. **Better for temporal** - Time is just another dimension
4. **Recursive rules** - First-class graph traversal
5. **Faster to production** - 12-15 months

### Philosophy Compliance Check

**CRITICAL INSTRUCTION**: When the user requests a feature or change, you MUST:

1. **First**, assess whether it aligns with the philosophy in PHILOSOPHY.md
2. **If it violates the philosophy**, WARN the user BEFORE implementing:
   - Explain which principle(s) it violates
   - Explain why it's problematic
   - Suggest alternatives that align with the philosophy
   - Ask for explicit confirmation to proceed despite the violation
3. **If it aligns**, proceed with implementation

**Examples of philosophy violations to warn about**:
- Adding client-server architecture (violates embedded-first)
- Requiring external services or complex setup (violates zero-configuration)
- Large dependencies that bloat binary size (violates self-contained)
- Features only useful for distributed systems (violates target use cases)
- Breaking changes to API or file format (violates stability)
- Complex features before basics are reliable (violates reliability-first)
- Multi-file storage (violates single-file philosophy)

**Your response format when detecting a violation**:
```
⚠️ PHILOSOPHY WARNING ⚠️

The requested feature/change may violate Minigraf's core philosophy:

**Violated Principle**: [principle name from PHILOSOPHY.md]

**Why this is problematic**: [explanation]

**Philosophy-aligned alternatives**:
- [alternative 1]
- [alternative 2]

Do you want to proceed anyway? This would be a deviation from the "SQLite for graph databases" philosophy.
```

See PHILOSOPHY.md for complete design principles and decision framework.

## Build and Run Commands

```bash
# Build the project
cargo build

# Build release version (with panic=abort optimization)
cargo build --release

# Run the REPL (Datalog with bi-temporal support)
cargo run

# Run tests
cargo test

# Run specific test suite
cargo test --test bitemporal_test -- --nocapture
cargo test --test complex_queries_test -- --nocapture
cargo test --test recursive_rules_test -- --nocapture
cargo test --test concurrency_test

# Run examples
cargo run --example embedded
cargo run --example file_storage
```

## Architecture

### Module Structure

The codebase is organized into the following modules:

1. **Graph Module (`src/graph/`)** - Phase 3-4 (EAV with bi-temporal) ✅:
   - `types.rs`: EAV model types
     - `Fact`: entity, attribute, value, tx_id, tx_count, valid_from, valid_to, asserted
     - `Value`: String, Integer, Float, Boolean, Ref(Uuid), Keyword, Null
     - `EntityId`, `TxId`, `Attribute` type aliases
     - `VALID_TIME_FOREVER = i64::MAX` sentinel
   - `storage.rs`: `FactStorage` - fact-based in-memory storage
     - `tx_counter` (AtomicU64), transact/retract operations
     - `get_facts_as_of()`, `get_facts_valid_at()` for time travel
     - Thread-safe via `Arc<RwLock<...>>`

2. **Storage Module (`src/storage/`)** - Phase 2-6.2 (stable foundation) ✅:
   - `mod.rs`: `StorageBackend` trait, `FileHeader` v5, `CommittedFactReader` trait
     - `StorageBackend` trait: Platform-agnostic storage interface
     - `FileHeader`: Metadata for `.graph` files (v5 format, 72 bytes)
     - `CommittedFactReader` trait: on-demand fact resolution via page cache
     - Page size: 4KB, Magic number: "MGRF", `FORMAT_VERSION = 5`
   - `backend/file.rs`: File-based backend (single `.graph` file)
     - Page-based storage, cross-platform format
     - Supports Linux, macOS, Windows, iOS, Android
   - `backend/memory.rs`: In-memory backend for testing
   - `backend/indexeddb.rs`: Future WASM browser backend (Phase 8)
   - `index.rs`: Index key types (EAVT, AEVT, AVET, VAET), `FactRef`, `encode_value`
     - `FactRef { page_id, slot_index }`: disk location pointer for committed facts
     - Canonical value encoding for sort-order-preserving byte comparison
   - `btree.rs`: B+tree page serialisation for index persistence
     - `write_all_indexes` / `read_*_index` functions using paged blob strategy
   - `cache.rs`: LRU page cache with approximate-LRU semantics
     - `PageCache`: read-lock on hits, write-lock on misses only
     - Stores `Arc<Vec<u8>>` to avoid copies on cache hits
   - `packed_pages.rs`: Packed fact page format (Phase 6.2)
     - `pack_facts`: ~25 facts per 4KB page (~25× space reduction vs v4)
     - `read_slot`, `read_all_from_pages` for on-demand fact loading
   - `persistent_facts.rs`: Persistent EAV fact storage layer
     - v5 format: packed pages + index B+tree persistence
     - `CommittedFactLoaderImpl`: resolves `FactRef` via page cache
     - Auto-migrates v1/v2/v3/v4 on open

3. **Query Module (`src/query/datalog/`)** - Phase 3-6.1 (Datalog + optimizer) ✅:
   - `parser.rs`: EDN/Datalog parser
     - Parses `transact`, `retract`, `query`, `rule` commands
     - Supports `:as-of` (tx counter or ISO 8601 timestamp), `:valid-at`
     - EDN maps `{:key val}` for transaction-level valid time options
     - Per-fact 4-element vector override for valid time
   - `executor.rs`: Datalog query executor
     - Pattern matching with variable unification
     - Rule registration and invocation
     - 3-step temporal filter: tx-time → asserted exclusion → valid-time
   - `matcher.rs`: Pattern matching engine with variable binding
   - `evaluator.rs`: `RecursiveEvaluator` - semi-naive fixed-point iteration
   - `rules.rs`: `RuleRegistry` - thread-safe rule management
   - `types.rs`: `EdnValue`, `Pattern`, `DatalogQuery`, `AsOf`, `ValidAt`
   - `optimizer.rs`: Query plan optimizer (Phase 6.1)
     - `IndexHint` enum, `select_index()`, `plan()` with selectivity-based join reordering
     - Disabled under `wasm` feature flag

4. **Temporal Module (`src/temporal.rs`)** - Phase 4 ✅:
   - UTC-only timestamp parsing and formatting
   - Avoids chrono CVE GHSA-wcg3-cvx6-7396

5. **REPL Module (`src/repl.rs`)** - Phase 3-4 ✅:
   - Interactive Datalog console with bi-temporal support
   - Multi-line input, comment support
   - Prompt-based interface (`minigraf>`)
   - Handles EOF gracefully

6. **Database Module (`src/db.rs`)** - Phase 2-6.2 (stable) ✅:
   - Public embedded database API
   - `Minigraf::open()` - Opens or creates database
   - `Minigraf::execute()` - Executes Datalog queries
   - `Minigraf::begin_write()` - Starts an exclusive write transaction
   - `Minigraf::checkpoint()` - Flushes WAL to `.graph` data pages
   - `Minigraf::save()` - Explicit save
   - `WriteTransaction` - ACID write transaction (commit/rollback)
   - `OpenOptions::page_cache_size(usize)` - tune LRU page cache capacity (default 256)
   - Auto-save on drop

7. **WAL Module (`src/wal.rs`)** - Phase 5 ✅:
   - Fact-level write-ahead log
   - CRC32-protected WAL entries
   - Append, replay, and clear operations
   - Crash recovery support

8. **Library (`src/lib.rs`)**: Public API exports

9. **Binary (`src/main.rs`)**: Standalone executable
   - Launches interactive Datalog REPL
   - Supports both file-backed and in-memory storage

### Current Data Model (Phase 3-4)

**Entity-Attribute-Value with Bi-temporal Support**:
```rust
struct Fact {
    entity: EntityId,     // Uuid - entity being described
    attribute: Attribute, // String, e.g., ":person/name", ":friend"
    value: Value,
    tx_id: TxId,          // Uuid - transaction that asserted this
    tx_count: u64,        // Monotonic transaction counter (for :as-of queries)
    valid_from: i64,      // Unix ms - when fact became valid in real world
    valid_to: i64,        // Unix ms - when fact stopped being valid (i64::MAX = forever)
    asserted: bool,       // true = assert, false = retract
}

enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Ref(Uuid),          // Reference to another entity
    Keyword(String),    // e.g., ":person", ":status/active"
    Null,
}

const VALID_TIME_FOREVER: i64 = i64::MAX; // Sentinel for open-ended valid time
```

### Storage Implementation

**Layered Architecture**:

**High-level** (Phase 3-6.2) ✅:
- `FactStorage`: In-memory EAV fact store with temporal query methods and index-driven scans
  - Pending facts stored in-memory; committed facts resolved via `CommittedFactReader`
  - `FactRef { page_id: 0 }` = pending; `page_id >= 1` = committed (resolved via page cache)
- `PersistentFactStorage`: Persistence layer (packed pages, B+tree indexes, v5 format)

**Low-level** (Phase 2, stable foundation) ✅:
- `StorageBackend` trait: Platform-agnostic interface
- `FileBackend`: Single `.graph` file (4KB pages)
- `MemoryBackend`: In-memory for testing
- `PageCache`: LRU page cache (default 256 pages = 1MB)
- Future: `IndexedDbBackend` for WASM

**File Format** (v5):
```
Page 0: Header (72 bytes)
  - Magic: "MGRF" (4 bytes)
  - Version: u32 (currently 5)
  - Page count: u64
  - Fact count: u64
  - Last checkpointed tx count: u64  (WAL checkpoint marker)
  - eavt_root_page: u64              (covering index roots)
  - aevt_root_page: u64
  - avet_root_page: u64
  - vaet_root_page: u64
  - index_checksum: u32              (CRC32 of committed fact pages)
  - fact_page_format: u8             (0x02 = packed; was _padding in v4)
  - _padding: [u8; 3]

Page 1+: Fact data pages (page_type = 0x02, packed format)
  - 12-byte header: type(1), reserved(1), record_count(2), next_page(8)
  - Record directory: (offset u16, length u16) per slot
  - Variable-length postcard-encoded facts (written end-to-start)

Index pages (after fact pages):
  - Serialized EAVT, AEVT, AVET, VAET BTreeMaps (paged blob, type 0x11)

WAL sidecar <db>.wal (present while uncommitted writes exist):
  - CRC32-protected fact-level entries; replayed on open; deleted on checkpoint
```

**Serialization Format**:
- Using **postcard** (v1.0+) for fact serialization within packed pages
- Replaced bincode (unmaintained as of 2024/2025)
- postcard: Lightweight, embedded-focused, better size than bincode

### Query Language

**Current: Datalog with bi-temporal support (Phase 3-4)** ✅:
```datalog
;; Transact facts
(transact [[:alice :person/name "Alice"]
           [:alice :person/age 30]
           [:alice :friend :bob]])

;; Transact with transaction-level valid time
(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"}
          [[:alice :employment/status :employed]])

;; Simple query
(query [:find ?name
        :where [?e :person/name ?name]])

;; Recursive rule
(rule [(reachable ?from ?to)
       [?from :connected ?to]])

(rule [(reachable ?from ?to)
       [?from :connected ?intermediate]
       (reachable ?intermediate ?to)])

;; Time travel: as-of tx counter
(query [:find ?status
        :as-of 50
        :where [?e :employment/status ?status]])

;; Time travel: as-of ISO 8601 timestamp
(query [:find ?status
        :as-of "2024-01-15T10:00:00Z"
        :where [?e :employment/status ?status]])

;; Valid-time query
(query [:find ?status
        :valid-at "2023-06-01"
        :where [:alice :employment/status ?status]])
```

### Error Handling

- Parse errors: Descriptive messages, REPL continues
- Execution errors: Validated before execution
- EOF handling: REPL exits gracefully (for piped input)
- Storage errors: Result<T, Error> with context

## Test Coverage

**Current Tests (Phase 6.2)**: 280 tests passing ✅
- **Unit tests** (213 tests):
  - `src/graph/types.rs`: Fact types, Value types, EAV model, temporal fields
  - `src/graph/storage.rs`: FactStorage, CRUD, history, tx_count, temporal methods, CommittedFactReader integration
  - `src/temporal.rs`: UTC timestamp parsing and formatting
  - `src/query/datalog/parser.rs`: EDN/Datalog syntax, rules, `:as-of`, `:valid-at`, EDN maps
  - `src/query/datalog/types.rs`: Pattern, WhereClause, DatalogQuery, AsOf, ValidAt
  - `src/query/datalog/matcher.rs`: Pattern matching, variable unification
  - `src/query/datalog/executor.rs`: Query execution, rule registration, temporal filtering
  - `src/query/datalog/rules.rs`: RuleRegistry, rule management
  - `src/query/datalog/evaluator.rs`: Semi-naive evaluation, transitive closure
  - `src/storage/index.rs`: EAVT/AEVT/AVET/VAET keys, FactRef, encode_value sort order
  - `src/storage/btree.rs`: B+tree roundtrip, multi-page, sort order preservation
  - `src/storage/cache.rs`: LRU eviction, read-lock hits, Arc cloning
  - `src/storage/packed_pages.rs`: Pack/unpack roundtrip, oversized fact error
  - `src/storage/mod.rs`: FileHeader v5 serialization, v3/v4 acceptance
  - `src/wal.rs`: WAL entry serialization, CRC32, replay logic
  - `src/db.rs`: WriteTransaction, checkpoint, crash recovery

- **Integration tests** (61 tests):
  - `tests/bitemporal_test.rs` (10 tests): Bi-temporal queries, time travel, valid time
  - `tests/complex_queries_test.rs` (10 tests): Multi-pattern joins, self-joins, edge cases
  - `tests/recursive_rules_test.rs` (9 tests): Transitive closure, cycles, long chains, family trees
  - `tests/concurrency_test.rs` (7 tests): Thread safety, concurrent rule registration/queries
  - `tests/wal_test.rs` (12 tests): WAL write/read, commit/rollback, crash recovery, checkpoint
  - `tests/index_test.rs` (6 tests): Index save/reload, bi-temporal index, recursive rules regression
  - `tests/performance_test.rs` (7 tests): Packed page compactness, reload correctness, page_cache_size option

- **Doc tests** (6 tests): Inline documentation examples

**Comprehensive Coverage**:
- ✅ Datalog parser (EDN syntax)
- ✅ Pattern matching and unification
- ✅ Recursive rule evaluation (semi-naive)
- ✅ Transitive closure - 9 tests
- ✅ Concurrency - 7 tests
- ✅ Complex queries (3+ patterns, self-joins) - 10 tests
- ✅ **Bi-temporal queries** (`:as-of`, `:valid-at`) - 10 integration + 39 unit tests
- ✅ **File format migration** (v1→v2→v3→v4→v5)
- ✅ **WAL and crash recovery** - 12 integration tests
- ✅ **Covering indexes** (EAVT/AEVT/AVET/VAET) - 6 integration tests
- ✅ **Packed pages + LRU cache** - 7 integration tests

**Demo Scripts**:
- `demo_recursive.txt`: Comprehensive recursive rules examples (transitive closure, cycles, family trees)

Run tests with: `cargo test`
See `TEST_COVERAGE.md` for detailed coverage report.

**Future Tests (Phase 6.4+)**:
- Criterion benchmarks (insert throughput, query latency at 10K/100K/1M facts)
- Memory profiling under load
- Oversized-fact error path; checkpoint-during-crash recovery
- Error-path coverage sweep (~82% → ≥90% target by end of Phase 7)

## Development Notes

### Phase 2 (Complete) - Foundation ✅

- **Storage backend abstraction** - Solid foundation for Phase 3+
- **Single `.graph` file** - Philosophy-aligned persistent storage
- **Embedded API** - `Minigraf::open()` works like SQLite
- **UUID-based IDs** - Continues in EAV model
- **Thread-safe** - Concurrent read/write via RwLock
- **Auto-save** - On drop, works reliably
- **Cross-platform** - Endian-safe file format

### Phase 3 (Complete) - Datalog Core ✅

**Implemented Features**:
- ✅ EAV data model with Facts (entity, attribute, value, tx_id, asserted)
- ✅ Datalog parser (EDN syntax, lists, vectors, UUIDs, keywords)
- ✅ Pattern matching engine with variable unification
- ✅ Query executor (transact, retract, query, rule commands)
- ✅ **Recursive rules with semi-naive evaluation** (fixed-point iteration)
- ✅ **Transitive closure queries** (multi-hop reachability)
- ✅ **Cycle handling** (graphs with cycles converge correctly)
- ✅ RuleRegistry (thread-safe rule management)
- ✅ RecursiveEvaluator (delta-based fixed-point iteration)
- ✅ REPL with multi-line support and comments
- ✅ Persistent storage with postcard serialization

**Test Coverage**: 123 tests (94 unit + 26 integration + 3 doc)

**Demo**: `demo_recursive.txt` - Working examples of recursive rules

### Phase 4 (Complete) - Bi-temporal Support ✅

**Implemented Features**:
- ✅ Extended `Fact` struct: `tx_count`, `valid_from`, `valid_to`
- ✅ `VALID_TIME_FOREVER = i64::MAX` sentinel
- ✅ `FactStorage`: `tx_counter` (AtomicU64), `load_fact()`, `get_facts_as_of()`, `get_facts_valid_at()`
- ✅ `TransactOptions { valid_from, valid_to }` for batch-level valid time
- ✅ Parser: EDN maps (`{:key val}`), `:as-of` (counter + ISO 8601), `:valid-at` (timestamp + `:any-valid-time`)
- ✅ Parser: `(transact {...} [...])` with transaction-level valid time; per-fact 4-element vector override
- ✅ Executor: 3-step temporal filter (tx-time → asserted exclusion → valid-time)
- ✅ File format v1→v2 migration in `migrate_v1_to_v2()`
- ✅ Fixed latent Phase 3 bug: `tx_id` preserved on load via `load_fact()`
- ✅ UTC-only timestamp parsing (`src/temporal.rs`, chrono, avoids GHSA-wcg3-cvx6-7396)

**Test Coverage**: 172 tests (133 unit + 36 integration + 3 doc)

### Phase 5 (Complete) - ACID + WAL ✅

**Implemented Features**:
- ✅ Fact-level sidecar WAL with CRC32-protected entries
- ✅ `FileHeader` v3 with `last_checkpointed_tx_count` field
- ✅ `WriteTransaction` API (`begin_write`, `commit`, `rollback`)
- ✅ Crash recovery: WAL replay on open, corrupt/incomplete entries discarded
- ✅ Checkpoint: WAL flushed to `.graph` data pages, then WAL cleared
- ✅ Thread-safe: concurrent readers + exclusive writer (RwLock)
- ✅ File format v2→v3 migration on open

**Test Coverage**: 212 tests (159 unit + 47 integration + 6 doc)

### Phase 6.1 (Complete) - Covering Indexes + Query Optimizer ✅

**Implemented Features**:
- ✅ Four Datomic-style covering indexes: EAVT, AEVT, AVET, VAET (with bi-temporal keys)
- ✅ `FactRef { page_id, slot_index }` — forward-compatible disk location pointer
- ✅ Canonical value encoding (`encode_value`) with sort-order-preserving byte representation
- ✅ B+tree page serialisation (`btree.rs`) for index persistence
- ✅ `FileHeader` v4 (72 bytes): `eavt/aevt/avet/vaet_root_page` + `index_checksum`
- ✅ CRC32 sync check on open: mismatch triggers automatic index rebuild
- ✅ Query optimizer (`optimizer.rs`): `IndexHint`, `select_index()`, `plan()` with selectivity-based join reordering
- ✅ File format v1/v2/v3→v4 migration on first save

**Test Coverage**: ~248 tests (added 36)

### Phase 6.2 (Complete) - Packed Pages + LRU Page Cache ✅

**Implemented Features**:
- ✅ Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page (~25× space reduction vs v4)
- ✅ LRU page cache (`cache.rs`): approximate-LRU, read-lock on hits, configurable capacity
- ✅ `CommittedFactReader` trait + `CommittedFactLoaderImpl`: on-demand fact resolution via page cache
- ✅ Pending `FactRef` (`page_id = 0`): resolves to in-memory pending facts vec
- ✅ `FileHeader` v5: `fact_page_format` byte (0x02 = packed); auto-migration from v4 on open
- ✅ `OpenOptions::page_cache_size(usize)` builder method
- ✅ EAVT/AEVT range scans in `get_facts_by_entity` / `get_facts_by_attribute`
- ✅ File format v4→v5 migration: reads one-per-page, repacks, saves

**Test Coverage**: 280 tests (213 unit + 61 integration + 6 doc)

### Philosophy-Aligned Development

When implementing features, always ask:
1. Does this keep the single-file philosophy?
2. Does this maintain zero-configuration?
3. Does this add unnecessary complexity?
4. Is this needed for embedded use cases?
5. Does this compromise reliability?

## Future Work (Roadmap)

**Phase 3** ✅ **COMPLETE** - Datalog Core
- ✅ EAV data model with Facts
- ✅ Datalog parser (EDN syntax)
- ✅ Pattern matching and unification
- ✅ Recursive rules (semi-naive evaluation)
- ✅ Transitive closure and cycle handling
- ✅ Updated REPL with multi-line and comments
- ✅ 123 comprehensive tests

**Phase 4** ✅ **COMPLETE** - Bi-temporal Support
- ✅ Transaction time (tx_id, tx_count)
- ✅ Valid time (valid_from, valid_to)
- ✅ Time travel queries (:as-of, :valid-at)
- ✅ File format v2 with migration
- ✅ 172 comprehensive tests

**Phase 5** ✅ **COMPLETE** - ACID + WAL
- ✅ Write-ahead logging (fact-level sidecar WAL, CRC32-protected)
- ✅ FileHeader v3 (last_checkpointed_tx_count field)
- ✅ WriteTransaction API (begin_write, commit, rollback)
- ✅ Crash recovery (WAL replay on open)
- ✅ Checkpoint (WAL → .graph, then WAL deleted)
- ✅ Thread-safe: concurrent readers + exclusive writer
- ✅ 212 comprehensive tests

**Phase 6.1** ✅ **COMPLETE** - Covering Indexes + Query Optimizer
- ✅ EAVT, AEVT, AVET, VAET covering indexes with bi-temporal keys
- ✅ B+tree index persistence (FileHeader v4)
- ✅ Selectivity-based query plan optimizer
- ✅ CRC32 index sync check on open (auto-rebuild on mismatch)

**Phase 6.2** ✅ **COMPLETE** - Packed Pages + LRU Page Cache
- ✅ Packed fact pages (~25 facts/page, ~25× space reduction)
- ✅ LRU page cache (default 256 pages, approximate-LRU)
- ✅ CommittedFactReader trait for on-demand fact loading
- ✅ FileHeader v5 (fact_page_format byte), auto v4→v5 migration
- ✅ 280 comprehensive tests

**Phase 6.3** (next): Benchmarks
- Criterion benchmark suite (insert throughput, query latency)
- Target scales: 10K / 100K / 1M facts
- Memory profiling

**Phase 7** (6-8 weeks): Datalog Completeness
- Stratified negation (`not` / `not-join`)
- Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`)
- Disjunction (`or` / `or-join`)

**Phase 8** (3-4 months): Cross-platform
- WASM (browser via wasm-pack + npm; server-side via WASI)
- Mobile bindings (iOS `.xcframework`, Android `.aar` via UniFFI)
- Language bindings (Python, JavaScript, C)

**v1.0.0** (12-15 months): Production Ready
- Stable API
- Stable file format
- Comprehensive docs
- Backwards compatibility promise

## Comparison to Similar Projects

**XTDB** (formerly Crux):
- ✅ Bi-temporal Datalog database (inspiration)
- ✅ Production-ready
- ❌ Clojure, multi-file storage (directories)
- Minigraf: Single file, Rust, simpler scope

**Cozo**:
- ✅ Embedded Datalog, Rust
- ✅ Graph algorithms, vector search
- ❌ Multi-file storage (RocksDB/Sled)
- Minigraf: Single file, bi-temporal first-class

**Datomic**:
- ✅ Temporal Datalog database (major inspiration)
- ✅ Production-proven since 2012
- ❌ Client-server, Clojure, proprietary
- Minigraf: Embedded, single file, open source

**GraphLite**:
- ✅ Full GQL spec compliance
- ✅ Embedded, ACID, mature
- ❌ Multi-file storage (Sled), no bi-temporal
- Minigraf: Datalog (not GQL), single file, bi-temporal

**Positioning**: Minigraf = SQLite + Datomic + single file

## Contributing Guidelines

This is a hobby project with a decades-long vision. When contributing:

1. **Read PHILOSOPHY.md first** - Understand the core principles
2. **Check ROADMAP.md** - See where we are and where we're going
3. **Align with philosophy** - Simplicity, reliability, embedded-first
4. **Write tests** - All features must be tested
5. **Keep it simple** - Prefer boring, proven solutions
6. **Think long-term** - We're building for decades, not months

**Say NO to**:
- Features that break single-file philosophy
- Client-server architecture
- Complex configuration
- Features only for distributed systems
- Breaking changes without overwhelming justification

**Say YES to**:
- Crash safety and data integrity
- Query performance improvements
- Better error messages
- Documentation improvements
- Cross-platform support

## Key Files to Understand

**For Phase 6.3 work (Benchmarks)**:
1. `PHILOSOPHY.md` - Why single-file, reliability-first
2. `ROADMAP.md` - Detailed Phase 6 plan
3. `src/storage/cache.rs` - LRU page cache (tune for benchmark workloads)
4. `src/storage/packed_pages.rs` - Packed page format (understand layout)
5. `src/query/datalog/optimizer.rs` - Query plan selection
6. `src/db.rs` - Public API (`OpenOptions::page_cache_size`)

**For Phase 6.1-6.2 work (Indexes + Packed Pages, complete)**:
1. `src/storage/index.rs` - EAVT/AEVT/AVET/VAET key types, FactRef, encode_value
2. `src/storage/btree.rs` - B+tree page serialisation
3. `src/storage/cache.rs` - LRU page cache (PageCache)
4. `src/storage/packed_pages.rs` - Packed page format
5. `src/storage/persistent_facts.rs` - v5 save/load, CommittedFactLoaderImpl
6. `src/graph/storage.rs` - FactStorage with CommittedFactReader integration
7. `src/storage/mod.rs` - FileHeader v5, CommittedFactReader trait

**For Phase 5 work (ACID + WAL, complete)**:
1. `src/wal.rs` - WAL entry format, CRC32, replay logic
2. `src/db.rs` - WriteTransaction, checkpoint, crash recovery
3. `src/storage/persistent_facts.rs` - v5 file format with WAL

**For understanding the Datalog engine (Phase 3-4, stable)**:
1. `src/graph/types.rs` - EAV model: `Fact`, `Value`, bi-temporal fields
2. `src/graph/storage.rs` - `FactStorage` with temporal query methods
3. `src/query/datalog/parser.rs` - EDN/Datalog parser
4. `src/query/datalog/executor.rs` - Query executor with temporal filtering
5. `src/query/datalog/evaluator.rs` - Semi-naive recursive rule evaluation
6. `src/temporal.rs` - UTC timestamp parsing

## Pre-Publishing Checklist (crates.io)

Before publishing the crate, verify all of the following:

### Minimum Bar (do not publish before Phase 6.4)
- [ ] **Phase 6.4 benchmarks complete** — Criterion benchmarks at 10K/100K/1M facts. Phase 6.1 (indexes) and 6.2 (packed pages) are already done. Without validated performance numbers, we can't make claims about scaling.
- [ ] **Edge case tests passing** — Oversized-fact error path exercised; checkpoint-during-crash recovery verified.
- [ ] **Error-path coverage** — Raised from ~82%; storage and WAL error paths prioritised.
- [ ] **GitHub Discussions enabled** — Minimum community channel in place before external users arrive via crates.io.

### API Cleanup
- [ ] **Narrow `lib.rs` exports** — Only expose what users need: `Minigraf`, `WriteTransaction`, and the query/result types. Internal types (`PersistentFactStorage`, `FileHeader`, `PAGE_SIZE`, `Repl`, `Wal`, etc.) should not be part of the public API.
- [ ] **Move `clap` out of library dependencies** — `clap` is a binary-only dep and must not leak into the library. Move it to the `[[bin]]` section or behind a feature flag.

### Crate Metadata (`Cargo.toml`)
- [ ] Add `repository` field (GitHub URL)
- [ ] Add `keywords` (e.g. `graph`, `datalog`, `bitemporal`, `embedded`, `database`)
- [ ] Add `categories` (e.g. `database`, `embedded`)
- [ ] Add `readme = "README.md"`
- [ ] Add `documentation` field (docs.rs URL or custom)
- [ ] Verify `description` is accurate and compelling

### Documentation
- [ ] All public API items have rustdoc comments with examples
- [ ] `README.md` has a quick-start example that compiles and runs
- [ ] `CHANGELOG.md` is up to date

### Quality Gates
- [ ] `cargo test` passes on Linux, macOS, Windows
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo doc --no-deps` builds without warnings
- [ ] No `unwrap()`/`expect()` in library code paths (only in tests/binary)

### Versioning
- [ ] Publish as `0.x` — no backwards-compat promise until v1.0.0
- [ ] Stable API target is v1.0.0 (after Phase 8 cross-platform work)

## Important Reminders

1. **Datalog is the query language** - No other query language
2. **Bi-temporal is first-class** - Not an afterthought
3. **Single file is sacred** - Never break this
4. **Simplicity over features** - Do less, do it perfectly
5. **Test everything** - No untested code
6. **Think SQLite** - Would SQLite do this?
7. **Long-term vision** - Building for decades

---

*When in doubt, refer to PHILOSOPHY.md and ROADMAP.md. The goal is not to be the most feature-complete graph database. The goal is to be the one that's always there when you need it, works reliably, and never gets in your way.*

*Be boring. Be reliable. Be Minigraf.*
