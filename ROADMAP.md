# Minigraf Roadmap

> The path from property graph PoC to production-ready bi-temporal Datalog database

**Philosophy**: "SQLite for bi-temporal graph databases" - Be boring, be reliable, be embeddable.

---

## Phase 1: Property Graph PoC ✅ COMPLETE

**Goal**: Prove the concept works

**Status**: ✅ Completed (December 2025)

**What Was Built**:
- ✅ Property graph model (nodes, edges, properties)
- ✅ In-memory storage engine
- ✅ Query parser
- ✅ Query executor
- ✅ Interactive REPL console
- ✅ Basic test coverage

**Deliverable**: Working in-memory graph database with REPL

---

## Phase 2: Embeddability ✅ COMPLETE

**Goal**: Make it truly embeddable with persistence

**Status**: ✅ Completed (January 2026)

**What Was Built**:
- ✅ Storage backend abstraction (FileBackend, MemoryBackend)
- ✅ Single-file `.graph` format (4KB pages)
- ✅ Cross-platform file format (endian-safe)
- ✅ Persistent graph storage with serialization
- ✅ Embedded database API (`Minigraf::open()`)
- ✅ Auto-save on drop
- ✅ Thread-safe concurrent access
- ✅ Comprehensive test suite (54 tests)
- ✅ Edge case and concurrency tests

**Deliverable**: Embeddable persistent graph database

**Philosophy Alignment**: ✅ Single-file, self-contained, embedded-first

**Learnings**: Storage layer is solid. Datalog chosen for better temporal support.

---

## Phase 3: Datalog Core ✅ COMPLETE

**Goal**: Implement core Datalog query engine

**Status**: ✅ Completed (January 2026)

**Priority**: 🔴 Critical - Foundation for everything else

### 3.1 EAV Data Model ✅

**Features**:
- ✅ Migrate from property graph to Entity-Attribute-Value model
- ✅ Fact representation: `(Entity, Attribute, Value)`
- ✅ Entities as UUIDs (keep existing ID system)
- ✅ Attributes as keywords (`:person/name`, `:friend`)
- ✅ Values: primitives or entity references
- ✅ Update storage format to support EAV tuples

**Technical Approach**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,  // Will become typed Keyword later
    value: Value,
    tx_id: TxId,        // Transaction that asserted this fact
    asserted: bool,     // true = assert, false = retract
}

enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Ref(Uuid),  // Reference to another entity
    Keyword(String),
    Null,
}
```

**Migration Path**:
- Keep existing node/edge types for backward compat
- Add EAV layer on top
- Gradually migrate tests
- Eventually deprecate property graph types

### 3.2 Datalog Parser ✅

**Features**:
- ✅ Parse basic Datalog queries (EDN syntax)
- ✅ Query structure: `[:find ?vars :where [clauses]]`
- ✅ Pattern matching: `[?e :attr ?v]`
- ✅ Variable binding
- ✅ Constants and entity references

**Example Queries**:
```datalog
;; Find all entities with a name
[:find ?e
 :where [?e :person/name _]]

;; Find friends of Alice
[:find ?friend
 :where
   [?alice :person/name "Alice"]
   [?alice :friend ?friend]]

;; Find names of Alice's friends
[:find ?name
 :where
   [?alice :person/name "Alice"]
   [?alice :friend ?friend]
   [?friend :person/name ?name]]
```

**Parser Components**:
- EDN reader (simple S-expression parser)
- Query validation
- Variable extraction
- Pattern recognition

### 3.3 Query Executor (Basic) ✅

**Features**:
- ✅ Pattern matching against fact database
- ✅ Variable unification
- ✅ Join multiple patterns
- ✅ Return results as tuples

**Implementation**:
- Naive evaluation initially (optimize in Phase 6)
- Iterate through facts, unify variables
- Cartesian product + filter (like nested loops)
- Return binding sets

### 3.4 Recursive Rules ✅

**Features**:
- ✅ Define rules: `[(rule-name ?args) [body]]`
- ✅ Stratified recursion (safe subset)
- ✅ Rule evaluation via semi-naive evaluation
- ✅ Transitive closure queries

**Example Rules**:
```datalog
;; Define reachability
[(reachable ?from ?to)
 [?from :connected ?to]]

[(reachable ?from ?to)
 [?from :connected ?intermediate]
 (reachable ?intermediate ?to)]

;; Use in query
[:find ?person
 :where
   (reachable :alice ?person)]
```

**Technical Approach**:
- Fixed-point iteration
- Track delta (new facts) each round
- Stop when no new facts produced
- Detect cycles, prevent infinite loops

### 3.5 REPL for Datalog ✅

**Features**:
- ✅ Interactive Datalog console
- ✅ Transact facts
- ✅ Query with Datalog
- ✅ Pretty-print results

**Commands**:
```clojure
;; Transact facts
(transact [[:alice :person/name "Alice"]
           [:alice :person/age 30]])

;; Query
(query [:find ?name :where [?e :person/name ?name]])

;; Define rule
(rule [(friends ?a ?b) [?a :friend ?b] [?b :friend ?a]])

;; Help
(help)

;; Exit
(exit)
```

### 3.6 Tests ✅

**Test Coverage**:
- ✅ EAV data model CRUD (8 tests)
- ✅ Datalog parser (all query forms) (15 tests)
- ✅ Pattern matching (6 tests)
- ✅ Variable unification (included in matcher tests)
- ✅ Multi-pattern joins (10 integration tests)
- ✅ Recursive rule evaluation (15 tests)
- ✅ Transitive closure queries (9 integration tests)
- ✅ Concurrency (7 integration tests)

**Total: 123 tests passing** ✅

**Deliverable**: ✅ Working Datalog query engine with recursion (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (January 2026)

---

## Phase 4: Bi-temporal Support ✅ COMPLETE

**Goal**: Add transaction time and valid time

**Status**: ✅ Completed (March 2026)

**Priority**: 🔴 Critical - Core differentiator

### 4.1 Transaction Time ✅

**Features**:
- ✅ Every fact records when it was added (`tx_id` wall-clock millis, `tx_count` monotonic counter)
- ✅ Facts are never deleted, only retracted (`asserted=false`)
- ✅ Query as of past transaction counter: `[:as-of 50]`
- ✅ Query as of past wall-clock time: `[:as-of "2024-01-15T10:00:00Z"]`

**Data Model**:
```rust
struct Fact {
    entity: EntityId,
    attribute: Attribute,
    value: Value,
    tx_id: TxId,       // wall-clock millis since epoch (u64)
    tx_count: u64,     // monotonically incrementing counter (1, 2, 3…)
    valid_from: i64,   // millis since epoch (i64)
    valid_to: i64,     // millis since epoch; i64::MAX = "forever"
    asserted: bool,
}
```

### 4.2 Valid Time ✅

**Features**:
- ✅ Facts carry validity period (`valid_from`, `valid_to`)
- ✅ `VALID_TIME_FOREVER = i64::MAX` sentinel for open-ended facts
- ✅ Query valid at specific time: `[:valid-at "2023-06-01"]`
- ✅ Default (no `:valid-at`): currently valid facts only
- ✅ `:any-valid-time` disables the valid time filter entirely
- ✅ Per-transaction and per-fact valid time overrides

### 4.3 Bi-temporal Queries ✅

```datalog
;; Full bi-temporal query
[:find ?status
 :valid-at "2023-06-01"
 :as-of "2024-01-15T10:00:00Z"
 :where [:alice :employment/status ?status]]

;; Transact with valid time
(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"}
          [[:alice :employment/status :active]])
```

### 4.4 Storage Format ✅

- **File format version** bumped 1→2
- Automatic v1→v2 migration on open (assigns `tx_count`, sets temporal defaults)
- Fixed latent Phase 3 bug: `tx_id` now preserved on load via `load_fact()`

### 4.5 Tests ✅

- ✅ 10 new integration tests (`tests/bitemporal_test.rs`)
- ✅ 39 new unit tests (types, storage, parser, executor)
- Transaction time travel (counter + timestamp)
- Valid time filtering (inside/outside range, boundary, default)
- Combined bi-temporal queries
- File format migration

**Deliverable**: ✅ Full bi-temporal Datalog database (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (March 2026)

---

## Phase 5: ACID + WAL ✅ COMPLETE

**Goal**: Add crash safety and transactions

**Status**: ✅ Completed (March 2026)

**Priority**: 🟡 High

### 5.1 Write-Ahead Logging (WAL) ✅

**Features**:
- ✅ Fact-level sidecar WAL (embedded in `.graph` file)
- ✅ CRC32-protected WAL entries
- ✅ Crash recovery (WAL replay on open)
- ✅ Checkpoint mechanism (WAL → .graph, then WAL deleted)
- ✅ `FileHeader` v3 (`last_checkpointed_tx_count` field)

**Why Embedded WAL**:
- ✅ Maintains single-file philosophy
- ✅ Easy to backup/share (one file)
- ✅ Simpler user experience

### 5.2 Transactions ✅

**Features**:
- ✅ `WriteTransaction` API: `begin_write`, `commit`, `rollback`
- ✅ Thread-safe: concurrent readers + exclusive writer
- ✅ ACID compliance:
  - Atomicity: All-or-nothing transactions
  - Consistency: Enforced via WAL
  - Isolation: Exclusive write lock
  - Durability: WAL ensures persistence

**API**:
```rust
let mut tx = db.begin_write()?;
tx.execute("(transact [[:alice :person/name \"Alice\"]])")?;
tx.commit()?;  // or tx.rollback()?
```

### 5.3 Crash Recovery ✅

**Features**:
- ✅ Detect uncommitted WAL entries on open
- ✅ Replay committed WAL entries to reconstruct state
- ✅ CRC32 checksum validation
- ✅ Incomplete entries discarded (not replayed)

### 5.4 Tests ✅

**Test Coverage**:
- ✅ WAL write and read
- ✅ Transaction commit/rollback
- ✅ Crash recovery (WAL replay on open)
- ✅ Recovery from partial/corrupt writes
- ✅ Checkpoint correctness
- ✅ Concurrent readers + exclusive writer

**Total: 212 tests passing** ✅

**Deliverable**: ✅ ACID-compliant database with crash safety (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (March 2026)

---

## Phase 6: Performance & Indexes 🚧 IN PROGRESS

**Goal**: Make queries fast

**Status**: 🚧 Phases 6.1 and 6.2 complete; 6.3 next

**Priority**: 🟡 High

### 6.1 Covering Indexes ✅ COMPLETE

**What Was Built**:
- ✅ EAVT, AEVT, AVET, VAET covering indexes (Datomic-style, bi-temporal keys)
- ✅ `FactRef { page_id, slot_index }` — forward-compatible disk location pointer
- ✅ Canonical value encoding with sort-order-preserving byte comparison
- ✅ B+tree page serialisation for index persistence (`btree.rs`)
- ✅ `FileHeader` v4: `eavt/aevt/avet/vaet_root_page` + `index_checksum` (CRC32)
- ✅ Auto-rebuild on checksum mismatch
- ✅ File format v1/v2/v3→v4 migration on first save

### 6.2 Packed Pages + LRU Page Cache ✅ COMPLETE

**What Was Built**:
- ✅ Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page (~25× space reduction)
- ✅ LRU page cache with approximate-LRU semantics (read-lock on hits)
- ✅ `CommittedFactReader` trait: index-driven on-demand fact resolution
- ✅ Eliminated "load all facts at startup" — only pending facts in memory
- ✅ EAVT/AEVT range scans for `get_facts_by_entity` / `get_facts_by_attribute`
- ✅ `FileHeader` v5 (`fact_page_format` byte); auto v4→v5 migration on open
- ✅ `OpenOptions::page_cache_size(usize)` — tune cache capacity (default 256 pages = 1MB)

### 6.3 Query Optimization

**Features**:
- ✅ Selectivity-based join reordering (Phase 6.1, `optimizer.rs`)
- ✅ Index selection per pattern (`IndexHint` enum)
- 🎯 Cost-based optimization improvements
- 🎯 Rule evaluation optimization

### 6.4 Benchmarks 🎯 NEXT

**Benchmark Suite**:
- 🎯 Criterion benchmarks: insert throughput, query latency (indexed/unindexed)
- 🎯 Memory profiling (various dataset sizes)
- 🎯 File size growth tracking
- 🎯 Transaction throughput
- 🎯 Time travel query performance

**Target Scales**:
- Small: 10K facts (personal knowledge base)
- Medium: 100K facts (small business app)
- Large: 1M facts (production single-machine)

**Deliverable**: Fast, indexed query engine with validated performance numbers

**Timeline**: 6.3 (benchmarks) ~1-2 weeks

---

## Phase 7: Cross-Platform Expansion 🎯 FUTURE

**Goal**: WASM, mobile, language bindings

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

### 7.1 WebAssembly Support

**Goal**: Run Minigraf as a WASM module in any modern browser, with IndexedDB as the storage backend.

**Features**:
- 🎯 `IndexedDbBackend`: stub already exists in `src/storage/backend/indexeddb.rs`; implement using `web-sys` + `wasm-bindgen`
- 🎯 WASM compilation target (`wasm32-unknown-unknown`)
- 🎯 The `.graph` file stored as a single blob in IndexedDB — consistent with single-file philosophy
- 🎯 JavaScript / TypeScript API (auto-generated via `wasm-bindgen`)
- 🎯 Import/export between native and WASM (same `.graph` binary format)
- 🎯 Disable `optimizer.rs` under `wasm` feature flag (already gated)

**WASM-specific constraints**:
- No filesystem access in `wasm32-unknown-unknown` — all storage goes through IndexedDB
- No threads in standard WASM — lock-free or single-threaded execution paths required
- Binary size budget: target <1MB gzipped; audit dependencies under `wasm` feature

**Deliverable**: `minigraf` crate compiles to WASM; browser apps can open/query/transact against a `.graph` stored in IndexedDB

### 7.2 Mobile Bindings

**Goal**: Ship Minigraf as a drop-in native library for Android (Kotlin/Java) and iOS (Swift), with pre-built artifacts so mobile developers don't need to touch Rust.

**Architecture: SDK approach, not engine-only**

Exposing a raw Rust crate and expecting mobile developers to write their own JNI/FFI layer creates a prohibitively high barrier to entry. The standard pattern (used by Mozilla Application Services, Matrix.org SDK, etc.) is to ship pre-generated language bindings as part of the release artifacts.

**Crate structure** (workspace):
```
minigraf/             ← core Rust library (current crate, no mobile-specific code)
minigraf-ffi/         ← separate crate: UniFFI bridge, no core logic
  src/lib.rs          ← #[uniffi::export] wrappers around minigraf public API
  minigraf.udl        ← UniFFI interface definition (or use proc-macro approach)
```

**Why UniFFI**:
- Developed by Mozilla, production-proven (Firefox for Android, iOS)
- Generates Kotlin, Swift, and Python bindings from a single interface definition
- Handles complex types (strings, enums, `Option`, `Result`) without manual C-style boilerplate
- Alternative: Diplomat (stricter, good for multi-language SDKs); Flutter Rust Bridge (if Flutter is a target)

**Cross-compilation targets**:
- Android: `aarch64-linux-android` (modern 64-bit), `armv7-linux-androideabi` (legacy 32-bit), `x86_64-linux-android` (emulator)
- iOS: `aarch64-apple-ios` (physical devices), `aarch64-apple-ios-sim` (simulators on Apple Silicon)

**Build outputs**:
- Android: `libminigraf.so` per ABI → bundled into a `.aar` (Android Archive) for easy Gradle import
- iOS: Static `.a` per target → lipo'd and wrapped into an `.xcframework` (Apple's standard multi-arch bundle)
- Both generated by a GitHub Actions workflow on every release tag

**Release artifacts** (GitHub Releases):
```
minigraf-android-v0.9.0.aar       ← drop into libs/ in Android project
MinigrafKit-v0.9.0.xcframework    ← add to Xcode project
MinigrafKit-v0.9.0.zip            ← Swift Package Manager checksum source
```

**Swift Package Manager support**:
- `Package.swift` pointing to the `.xcframework` release artifact
- Allows `swift package add https://github.com/adityamukho/minigraf` in Xcode

**Maven / Gradle support**:
- Publish `.aar` to GitHub Packages or Maven Central
- `implementation("io.github.adityamukho:minigraf-android:0.9.0")`

**Features** (implementation order):
- 🎯 `minigraf-ffi` crate with UniFFI proc-macro bindings
- 🎯 GitHub Actions cross-compilation matrix (Android ABIs + iOS targets)
- 🎯 `.xcframework` and `.aar` assembly in CI
- 🎯 Swift Package manifest
- 🎯 Android Gradle integration example
- 🎯 Memory/resource management: ensure `Minigraf` handle lifecycle is safe across FFI boundary

### 7.3 Language Bindings

**Goal**: Python and C FFI as the highest-priority non-mobile targets (covers scripting, agent frameworks, and "any other language via C").

**Features**:
- 🎯 Python bindings via UniFFI (same `.udl` / proc-macro as mobile — no extra code)
- 🎯 C header (`minigraf.h`) via `cbindgen` for any language with a C FFI
- 🎯 Node.js / TypeScript bindings via `neon` or `napi-rs`
- 🎯 Published to PyPI (`minigraf`), npm (`@minigraf/core`)

**Note**: Python and C bindings share the UniFFI / cbindgen work done for mobile — the incremental cost is small once Phase 7.2 is complete.

**Deliverable**: Run anywhere - desktop, mobile, web, embedded; official packages on crates.io, PyPI, npm, Maven, Swift Package Index

**Timeline**: 3-4 months

---

## Phase 8: Ecosystem & Tooling 🎯 FUTURE

**Goal**: Developer experience and ecosystem

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

### 8.1 Developer Tools

**Features**:
- 🎯 Database inspector/debugger
- 🎯 Query profiler
- 🎯 Time travel visualizer
- 🎯 Migration tools

### 8.2 Documentation

**Features**:
- 🎯 Complete API reference
- 🎯 Datalog language specification
- 🎯 Cookbook (common patterns)
- 🎯 Performance tuning guide
- 🎯 Real-world examples

### 8.3 Ecosystem Libraries

**Features**:
- 🎯 Graph algorithms (as separate crate)
- 🎯 Schema validation (optional)
- 🎯 Import/export tools
- 🎯 Backup utilities

**Deliverable**: Production-ready ecosystem

**Timeline**: Ongoing

---

## Release Strategy

### v0.1.0 - ✅ Phase 1 Complete (PoC)
- In-memory property graph
- REPL console

### v0.2.0 - ✅ Phase 2 Complete (Embeddable)
- Persistent storage
- Embedded database API
- Cross-platform file format
- Auto-save

### v0.3.0 - ✅ Phase 3 Complete (Datalog Core)
- ✅ EAV data model
- ✅ Datalog queries
- ✅ Recursive rules
- ✅ Pattern matching
- ✅ Semi-naive evaluation
- ✅ 123 tests passing

### v0.4.0 - ✅ Phase 4 Complete (Bi-temporal)
- ✅ Transaction time (`tx_id`, `tx_count`)
- ✅ Valid time (`valid_from`, `valid_to`)
- ✅ Time travel queries (`:as-of`, `:valid-at`)
- ✅ File format v2 with v1 migration
- ✅ 172 tests passing

### v0.5.0 - ✅ Phase 5 Complete (ACID + WAL)
- ✅ Write-ahead logging (fact-level sidecar WAL, CRC32-protected)
- ✅ `WriteTransaction` API (begin_write, commit, rollback)
- ✅ Crash recovery (WAL replay on open)
- ✅ FileHeader v3 (`last_checkpointed_tx_count`)
- ✅ Thread-safe: concurrent readers + exclusive writer
- ✅ 212 tests passing

### v0.6.0 - ✅ Phase 6.1 Complete (Covering Indexes + Query Optimizer)
- ✅ EAVT, AEVT, AVET, VAET covering indexes with bi-temporal keys
- ✅ B+tree index persistence (FileHeader v4)
- ✅ Selectivity-based query plan optimizer (`optimizer.rs`)
- ✅ CRC32 index sync check; auto-rebuild on mismatch
- ✅ File format v1/v2/v3→v4 migration

### v0.7.0 - ✅ Phase 6.2 Complete (Packed Pages + LRU Cache)
- ✅ Packed fact pages (~25 facts/page, ~25× disk space reduction)
- ✅ LRU page cache (configurable, default 256 pages = 1MB)
- ✅ `CommittedFactReader` trait: on-demand fact loading (no startup load-all)
- ✅ FileHeader v5 (`fact_page_format` byte); auto v4→v5 migration
- ✅ 280 tests passing

### v0.8.0 - 🎯 Phase 6.3 (Benchmarks)
- Criterion benchmark suite
- Validated performance at 10K / 100K / 1M facts

### v0.9.0 - 🎯 Phase 7 (Cross-platform)
- WASM support
- Mobile bindings
- Language bindings

### v1.0.0 - 🎯 Production Ready (12-15 months)
- Stable API
- Stable file format
- Comprehensive tests
- Full documentation
- Performance validated
- Backwards compatibility promise

**Stability Promise**: After v1.0.0, we commit to:
- Backwards-compatible file format (decades)
- Stable public API (semantic versioning)
- Migration tools for any format changes
- Long-term support

---

## Decision Framework

When evaluating features, ask:

1. **Does it align with philosophy?** (embedded, reliable, simple, bi-temporal)
2. **Is it needed for target use cases?** (audit, event sourcing, knowledge graphs)
3. **Does it compromise reliability?** (stability over features)
4. **Can it be a separate crate?** (keep core small)

**Say NO to**:
- Distributed consensus
- Multi-datacenter replication
- Built-in ML/AI
- Features only useful at massive scale
- Complex configuration
- Breaking the single-file philosophy

**Say YES to**:
- Crash safety
- Data integrity
- Temporal queries
- Query performance
- Developer experience
- Cross-platform support

---

## Timeline (Rough Estimates)

- ✅ Phase 1: Complete (December 2025)
- ✅ Phase 2: Complete (January 2026)
- ✅ Phase 3: Complete (January 2026) - Datalog core with recursive rules
- ✅ Phase 4: Complete (March 2026) - Bi-temporal support
- ✅ Phase 5: Complete (March 2026) - ACID + WAL
- ✅ Phase 6.1: Complete (March 2026) - Covering Indexes + Query Optimizer
- ✅ Phase 6.2: Complete (March 2026) - Packed Pages + LRU Cache
- 🎯 Phase 6.3: 1-2 weeks (Benchmarks) - **NEXT**
- 🎯 Phase 7: 3-4 months (Cross-platform)
- 🎯 Phase 8: Ongoing (Ecosystem)
- 🎯 **v1.0.0: 9-12 months** (ahead of schedule)

**Note**: This is a hobby project. Timeline is flexible but realistic.

---

## Current Focus

**Right Now**: ✅ Phase 6.2 Complete! Planning Phase 6.3 - Benchmarks

**Phase 6.2 Achievements**:
1. ✅ Packed fact pages (~25 facts/4KB page, ~25× space reduction vs v4)
2. ✅ LRU page cache with approximate-LRU semantics (`cache.rs`)
3. ✅ `CommittedFactReader` trait: on-demand fact loading (no startup load-all)
4. ✅ EAVT/AEVT range scans in `get_facts_by_entity` / `get_facts_by_attribute`
5. ✅ `FileHeader` v5 (`fact_page_format` byte); auto v4→v5 migration
6. ✅ `OpenOptions::page_cache_size(usize)` builder method
7. ✅ 280 tests passing (68 new tests since Phase 5)

**Immediate Next Steps (Phase 6.3)**:
1. Add Criterion as a dev-dependency
2. Write benchmarks: insert throughput, point-lookup, range scan, time travel
3. Profile memory usage at 10K / 100K / 1M facts
4. Document performance characteristics in README

**Key Decisions Made**:
- ✅ Datalog query language (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Recursive rules with semi-naive evaluation
- ✅ UTC-only timestamps (avoids chrono GHSA-wcg3-cvx6-7396)
- ✅ Packed pages over one-per-page (philosophy: small binary, efficient storage)
- ✅ Approximate LRU (read-lock on hits — avoids write-lock contention)
- ✅ Target 9-12 months to v1.0 (ahead of schedule!)

See [GitHub Issues](https://github.com/adityamukho/minigraf/issues) for specific tasks.

---

Last Updated: Phase 6.2 Complete - Packed Pages + LRU Cache (March 2026)
