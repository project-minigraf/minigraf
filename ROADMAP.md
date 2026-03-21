# Minigraf Roadmap

> The path from property graph PoC to production-ready bi-temporal Datalog database

**Philosophy**: "SQLite for bi-temporal graph databases" - Be boring, be reliable, be embeddable.

**Strategic Pivot** (January 2026): After completing Phase 2, we pivoted from GQL (property graph) to Datalog (EAV) for better temporal semantics, simpler implementation, and faster time-to-production.

---

## Phase 1: Property Graph PoC ✅ COMPLETE

**Goal**: Prove the concept works

**Status**: ✅ Completed (December 2025)

**What Was Built**:
- ✅ Property graph model (nodes, edges, properties)
- ✅ In-memory storage engine
- ✅ GQL-inspired query parser
- ✅ Query executor
- ✅ Interactive REPL console
- ✅ Basic test coverage

**Deliverable**: Working in-memory graph database with REPL

**Learnings**: Property graphs work well, but GQL spec is too large. Need simpler query language.

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

**Archive**: GQL implementation preserved at `archive/gql-phase-2` branch

**Learnings**: Storage layer is solid. Pivot to Datalog for better temporal support.

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

## Phase 6: Performance & Indexes 🎯 FUTURE

**Goal**: Make queries fast

**Status**: 🎯 Planned

**Priority**: 🟡 High

### 6.1 Indexes

**Features**:
- 🎯 EAVT, AEVT, AVET, VAET covering indexes
- 🎯 B-tree or sorted page indexes
- 🎯 Index maintenance (auto-update on changes)
- 🎯 Temporal index (ValidFrom, ValidTo ranges)

**Performance Targets**:
- Indexed fact lookup: <1ms
- Pattern match with index: <10ms for 100K facts
- Transitive closure: <100ms for typical graphs

### 6.2 Query Optimization

**Features**:
- 🎯 Query plan generation
- 🎯 Cost-based optimization
- 🎯 Index selection
- 🎯 Join order optimization
- 🎯 Rule evaluation optimization

**Optimizer**:
- Choose best index for each pattern
- Reorder patterns by selectivity
- Push filters down
- Eliminate redundant patterns

### 6.3 Caching

**Features**:
- 🎯 Query result caching
- 🎯 Rule evaluation caching
- 🎯 Page cache (LRU)

### 6.4 Benchmarks

**Benchmark Suite**:
- Insert throughput (facts/sec)
- Query latency (indexed, unindexed)
- Memory usage (various dataset sizes)
- File size growth
- Transaction throughput
- Time travel query performance

**Target Scales**:
- Small: 10K facts (personal knowledge base)
- Medium: 100K facts (small business app)
- Large: 1M facts (production single-machine)

**Deliverable**: Fast, indexed query engine

**Timeline**: 2-3 months

---

## Phase 7: Cross-Platform Expansion 🎯 FUTURE

**Goal**: WASM, mobile, language bindings

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

### 7.1 WebAssembly Support

**Features**:
- 🎯 IndexedDB backend for browsers
- 🎯 WASM compilation target
- 🎯 JavaScript API
- 🎯 Import/export between native and WASM

### 7.2 Mobile Optimization

**Features**:
- 🎯 iOS bindings (Swift FFI)
- 🎯 Android bindings (JNI/Kotlin)
- 🎯 Mobile-specific optimizations

### 7.3 Language Bindings

**Features**:
- 🎯 Python bindings
- 🎯 JavaScript/TypeScript bindings
- 🎯 C FFI for other languages

**Deliverable**: Run anywhere - desktop, mobile, web, embedded

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
- GQL-inspired queries
- REPL console

### v0.2.0 - ✅ Phase 2 Complete (Embeddable)
- Persistent storage
- Embedded database API
- Cross-platform file format
- Auto-save

**GQL Archive**: `archive/gql-phase-2` branch, `gql-phase-2-complete` tag

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
- ✅ 213 tests passing

### v0.6.0 - 🎯 Phase 6 (Performance)
- Indexes (EAVT, AEVT, AVET, VAET)
- Query optimization
- Performance benchmarks

### v0.7.0 - 🎯 Phase 7 (Cross-platform)
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
- 🎯 Phase 6: 2-3 months (Performance) - **NEXT**
- 🎯 Phase 7: 3-4 months (Cross-platform)
- 🎯 Phase 8: Ongoing (Ecosystem)
- 🎯 **v1.0.0: 11-14 months** from now (Phase 3 completed faster than expected!)

**Note**: This is a hobby project. Timeline is flexible but realistic.

**Comparison**:
- **GQL path**: 24-30 months to v1.0 (catching up to GraphLite)
- **Datalog path**: 12-15 months to v1.0 (proven patterns from Datomic/XTDB)

---

## Current Focus

**Right Now**: ✅ Phase 5 Complete! Planning Phase 6 - Performance & Indexes

**Phase 5 Achievements**:
1. ✅ Fact-level sidecar WAL with CRC32-protected entries
2. ✅ `FileHeader` v3 with `last_checkpointed_tx_count` field
3. ✅ `WriteTransaction` API (`begin_write`, `commit`, `rollback`)
4. ✅ Crash recovery: WAL replay on open, corrupt entries discarded
5. ✅ Checkpoint: WAL flushed to `.graph`, then WAL cleared
6. ✅ Thread-safe: concurrent readers + exclusive writer
7. ✅ 212 tests passing (40 new tests)

**Immediate Next Steps (Phase 6)**:
1. Design index format (EAVT, AEVT, AVET, VAET covering indexes)
2. Implement B-tree or sorted page indexes
3. Query optimizer (cost-based, index selection)
4. Benchmark suite (insert throughput, query latency)

**Key Decisions Made**:
- ✅ Pivot to Datalog (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Recursive rules with semi-naive evaluation
- ✅ UTC-only timestamps (avoids chrono GHSA-wcg3-cvx6-7396)
- ✅ Target 10-13 months to v1.0 (ahead of schedule!)

See [GitHub Issues](https://github.com/adityamukho/minigraf/issues) for specific tasks.

---

## Why This Roadmap is Better

**Compared to GQL roadmap:**

1. **Faster**: 12-15 months vs. 24-30 months
2. **Simpler**: Proven Datalog patterns vs. novel GQL implementation
3. **More unique**: Single-file bi-temporal Datalog (no competitor)
4. **Better fit**: Temporal semantics natural in Datalog
5. **More reliable**: 40+ years of Datalog research vs. new GQL spec

**Trade-offs accepted:**
- ❌ Lose familiarity of SQL-like syntax
- ❌ Smaller developer audience (fewer know Datalog)
- ✅ Gain simplicity and power of logic programming
- ✅ Gain natural temporal queries
- ✅ Gain recursive rules without pain

**Market positioning:**
- **GQL space**: GraphLite won (full spec, mature, ACID)
- **Datalog space**: Gap for single-file embedded bi-temporal DB

We're not competing with GraphLite anymore. We're creating a new category.

---

Last Updated: Phase 5 Complete - ACID + WAL (March 2026)
