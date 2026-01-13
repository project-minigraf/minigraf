# Minigraf Roadmap

> The path from property graph PoC to production-ready bi-temporal Datalog database

**Philosophy**: "SQLite for bi-temporal graph databases" - Be boring, be reliable, be embeddable.

**Strategic Pivot** (January 2026): After completing Phase 2, we pivoted from GQL to Datalog for better temporal semantics, simpler implementation, and faster time-to-production.

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

## Phase 3: Datalog Core 🎯 NEXT

**Goal**: Implement core Datalog query engine

**Status**: 🎯 In Progress (Expected: 3-4 months)

**Priority**: 🔴 Critical - Foundation for everything else

### 3.1 EAV Data Model

**Features**:
- 🎯 Migrate from property graph to Entity-Attribute-Value model
- 🎯 Fact representation: `(Entity, Attribute, Value)`
- 🎯 Entities as UUIDs (keep existing ID system)
- 🎯 Attributes as keywords (`:person/name`, `:friend`)
- 🎯 Values: primitives or entity references
- 🎯 Update storage format to support EAV tuples

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

### 3.2 Datalog Parser

**Features**:
- 🎯 Parse basic Datalog queries (EDN syntax)
- 🎯 Query structure: `[:find ?vars :where [clauses]]`
- 🎯 Pattern matching: `[?e :attr ?v]`
- 🎯 Variable binding
- 🎯 Constants and entity references

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

### 3.3 Query Executor (Basic)

**Features**:
- 🎯 Pattern matching against fact database
- 🎯 Variable unification
- 🎯 Join multiple patterns
- 🎯 Return results as tuples

**Implementation**:
- Naive evaluation initially (optimize in Phase 6)
- Iterate through facts, unify variables
- Cartesian product + filter (like nested loops)
- Return binding sets

### 3.4 Recursive Rules

**Features**:
- 🎯 Define rules: `[(rule-name ?args) [body]]`
- 🎯 Stratified recursion (safe subset)
- 🎯 Rule evaluation via semi-naive evaluation
- 🎯 Transitive closure queries

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

### 3.5 REPL for Datalog

**Features**:
- 🎯 Interactive Datalog console
- 🎯 Transact facts
- 🎯 Query with Datalog
- 🎯 Pretty-print results

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

### 3.6 Tests

**Test Coverage**:
- EAV data model CRUD
- Datalog parser (all query forms)
- Pattern matching
- Variable unification
- Multi-pattern joins
- Recursive rule evaluation
- Transitive closure queries

**Deliverable**: Working Datalog query engine with recursion

**Timeline**: 3-4 months

---

## Phase 4: Bi-temporal Support 🎯 FUTURE

**Goal**: Add transaction time and valid time

**Status**: 🎯 Planned (After Phase 3)

**Priority**: 🔴 Critical - Core differentiator

### 4.1 Transaction Time

**Features**:
- 🎯 Every fact records when it was added (`tx_id`, `tx_time`)
- 🎯 Facts are never deleted, only retracted
- 🎯 Query as of past transaction: `[:as-of tx-100]`
- 🎯 History of an entity/attribute

**Data Model**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,
    value: Value,
    tx_id: TxId,           // NEW: Transaction ID
    tx_time: SystemTime,   // NEW: Wall-clock time
    asserted: bool,        // true = assert, false = retract
}
```

**Queries**:
```datalog
;; Current state (default)
[:find ?name :where [?e :person/name ?name]]

;; State as of transaction 100
[:find ?name
 :as-of 100
 :where [?e :person/name ?name]]

;; History of Alice's name
[:find ?name ?tx
 :where
   [?alice :person/name "Alice"]
   [?alice :person/name ?name ?tx]]
```

### 4.2 Valid Time

**Features**:
- 🎯 Facts can have validity period (when true in real world)
- 🎯 `valid_from` and `valid_to` timestamps
- 🎯 Query valid at specific time: `[:valid-at "2023-06-01"]`
- 🎯 Separate from transaction time

**Data Model**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,
    value: Value,
    valid_from: DateTime,  // NEW: When valid starts
    valid_to: DateTime,    // NEW: When valid ends (MAX for open)
    tx_id: TxId,
    tx_time: SystemTime,
    asserted: bool,
}
```

**Queries**:
```datalog
;; Valid on specific date
[:find ?status
 :valid-at "2023-06-01"
 :where
   [:alice :employment/status ?status]]

;; Valid during range
[:find ?status
 :valid-from "2023-01-01"
 :valid-to "2023-12-31"
 :where
   [:alice :employment/status ?status]]
```

### 4.3 Bi-temporal Queries

**Features**:
- 🎯 Combine both time dimensions
- 🎯 "What did we believe on date X about facts valid on date Y?"
- 🎯 Audit trails and compliance queries

**Queries**:
```datalog
;; Full bi-temporal query
[:find ?status
 :valid-at "2023-06-01"      ;; Valid time
 :as-of tx-100               ;; Transaction time
 :where
   [:alice :employment/status ?status]]

;; History of corrections
[:find ?value ?tx ?valid-at
 :where
   [:alice :account/balance ?value ?tx ?valid-at]]
```

### 4.4 Storage Format

**File Structure**:
```
Page 0: Header (includes tx counter)
Page 1+: Facts with (E, A, V, ValidFrom, ValidTo, TxTime, Asserted)
```

**Indexes for Temporal Queries**:
- EAVT: Entity, Attribute, Value, Transaction
- AEVT: Attribute, Entity, Value, Transaction
- AVET: Attribute, Value, Entity, Transaction
- VAET: Value, Attribute, Entity, Transaction
- ValidTime index: (ValidFrom, ValidTo) for range queries

### 4.5 Tests

**Test Coverage**:
- Transaction time recording
- As-of queries
- Valid time periods
- Valid-at queries
- Bi-temporal queries
- History queries
- Time travel edge cases

**Deliverable**: Full bi-temporal Datalog database

**Timeline**: 3-4 months

---

## Phase 5: ACID + WAL 🎯 FUTURE

**Goal**: Add crash safety and transactions

**Status**: 🎯 Planned

**Priority**: 🟡 High

### 5.1 Write-Ahead Logging (WAL)

**Features**:
- 🎯 Embedded WAL (in same `.graph` file)
- 🎯 Append-only transaction log
- 🎯 Crash recovery (replay WAL on open)
- 🎯 Checkpoint mechanism (merge WAL to main pages)

**Technical Approach**:
```
File Structure:
Page 0: Header (includes wal_offset, last_checkpoint)
Page 1: EAVT index
Page 2+: Fact pages
WAL: Append-only log at end of file
```

**Why Embedded WAL**:
- ✅ Maintains single-file philosophy
- ✅ Easy to backup/share (one file)
- ✅ Simpler user experience

### 5.2 Transactions

**Features**:
- 🎯 Transaction API: `BEGIN`, `COMMIT`, `ROLLBACK`
- 🎯 ACID compliance:
  - Atomicity: All-or-nothing transactions
  - Consistency: Enforce constraints
  - Isolation: Serializable isolation
  - Durability: WAL ensures persistence

**API**:
```rust
let mut tx = db.begin_transaction()?;
tx.transact(vec![
    [:alice, :person/name, "Alice"],
    [:alice, :friend, :bob],
])?;
tx.commit()?;  // or tx.rollback()?
```

### 5.3 Crash Recovery

**Features**:
- 🎯 Detect unclean shutdown (incomplete checkpoint)
- 🎯 Replay WAL to reconstruct state
- 🎯 Validate checksums
- 🎯 Verify transaction completeness

### 5.4 Tests

**Test Coverage**:
- WAL write and read
- Transaction commit/rollback
- Crash during transaction (kill -9)
- Recovery from partial writes
- Checkpoint correctness
- Multiple transactions before checkpoint

**Deliverable**: ACID-compliant database with crash safety

**Timeline**: 2-3 months

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

### v0.3.0 - 🎯 Phase 3 (Datalog Core)
- EAV data model
- Datalog queries
- Recursive rules
- Pattern matching

### v0.4.0 - 🎯 Phase 4 (Bi-temporal)
- Transaction time
- Valid time
- Time travel queries
- History queries

### v0.5.0 - 🎯 Phase 5 (ACID)
- Write-ahead logging
- Transactions
- Crash recovery
- ACID compliance

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
- 🎯 Phase 3: 3-4 months (Datalog core)
- 🎯 Phase 4: 3-4 months (Bi-temporal)
- 🎯 Phase 5: 2-3 months (ACID + WAL)
- 🎯 Phase 6: 2-3 months (Performance)
- 🎯 Phase 7: 3-4 months (Cross-platform)
- 🎯 Phase 8: Ongoing (Ecosystem)
- 🎯 **v1.0.0: 12-15 months** from now (vs. 24-30 months with GQL)

**Note**: This is a hobby project. Timeline is flexible but realistic.

**Comparison**:
- **GQL path**: 24-30 months to v1.0 (catching up to GraphLite)
- **Datalog path**: 12-15 months to v1.0 (proven patterns from Datomic/XTDB)

---

## Current Focus

**Right Now**: Planning Phase 3 - Datalog Core

**Immediate Next Steps**:
1. Design EAV data model
2. Implement Datalog parser (EDN syntax)
3. Build basic query executor
4. Add recursive rules
5. Update REPL for Datalog
6. Migrate tests

**Key Decisions Made**:
- ✅ Pivot to Datalog (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Target 12-15 months to v1.0

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

Last Updated: Phase 2 Complete, Datalog Pivot (January 2026)
