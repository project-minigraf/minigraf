# Minigraf Roadmap

> The path from PoC to production-ready embedded graph database

**Philosophy**: "SQLite for graph databases" - Be boring, be reliable, be embeddable.

---

## Phase 1: Proof of Concept ✅ COMPLETE

**Goal**: Prove the concept works

**Status**: ✅ Completed

**What Was Built**:
- ✅ Property graph model (nodes, edges, properties)
- ✅ In-memory storage engine
- ✅ GQL-inspired query parser
- ✅ Query executor
- ✅ Interactive REPL console
- ✅ Basic test coverage

**Deliverable**: Working in-memory graph database with REPL

---

## Phase 2: Embeddability ✅ COMPLETE

**Goal**: Make it truly embeddable with persistence

**Status**: ✅ Completed (Current)

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

---

## Phase 3: Reliability & Performance 🔄 NEXT

**Goal**: Add crash safety, transactions, and query optimization

**Status**: ⏳ Planned

### 3.1 Write-Ahead Logging (WAL) & Crash Safety

**Priority**: 🔴 Critical

**Features**:
- ⏳ Embedded WAL (in same `.graph` file)
- ⏳ Transaction support (BEGIN, COMMIT, ROLLBACK)
- ⏳ Crash recovery (replay WAL on open)
- ⏳ Checkpoint mechanism (merge WAL to main pages)
- ⏳ ACID compliance:
  - Atomicity: All-or-nothing transactions
  - Consistency: Enforce constraints
  - Isolation: Transaction isolation levels
  - Durability: WAL ensures persistence

**Technical Approach**:
```
File Structure:
Page 0: Header (includes wal_offset)
Page 1: Index
Page 2+: Data
WAL: Append-only log at end of file
```

**Why Embedded WAL**:
- ✅ Maintains single-file philosophy
- ✅ Easy to backup/share (one file)
- ✅ Simpler user experience
- ⚠️ Slightly more complex than separate file
- ✅ Acceptable trade-off for our use cases

**Tests Needed**:
- Crash during transaction (unclean shutdown)
- Multiple transactions before checkpoint
- Recovery from partial writes
- Transaction rollback

### 3.2 Indexes for Query Performance

**Priority**: 🟡 High

**Features**:
- ⏳ B-tree indexes for properties
- ⏳ Label indexes for fast MATCH
- ⏳ Composite indexes (multi-property)
- ⏳ Index maintenance (auto-update on changes)
- ⏳ Query planner (choose index vs. scan)

**Performance Targets**:
- Indexed property lookup: <1ms
- MATCH with filter: <10ms for 100K nodes
- Bulk insert: 10K nodes/sec

### 3.3 Query Optimization

**Priority**: 🟡 High

**Features**:
- ⏳ Query plan generation
- ⏳ Cost-based optimization
- ⏳ Index selection
- ⏳ JOIN optimization (for multi-hop paths)

### 3.4 Incremental Persistence

**Priority**: 🟢 Medium

**Features**:
- ⏳ Only save changed pages (not full "load all, save all")
- ⏳ Page-level dirty tracking
- ⏳ Efficient diff detection

**Why After WAL**:
- WAL provides atomic updates
- Safer to do incremental with WAL

**Deliverable**: ACID-compliant database with fast queries

**Timeline**: 2-3 months of focused development

---

## Phase 4: Advanced Graph Features

**Goal**: Multi-hop queries, advanced patterns

**Status**: ⏳ Future

### 4.1 Complex Path Patterns

**Features**:
- ⏳ Variable-length paths: `(a)-[:REL*1..5]->(b)`
- ⏳ Shortest path queries
- ⏳ Multi-hop traversals
- ⏳ Path finding algorithms

### 4.2 Advanced Query Language

**Features**:
- ⏳ RETURN clause (projections, expressions)
- ⏳ WITH clause (intermediate results)
- ⏳ ORDER BY, LIMIT, SKIP
- ⏳ DISTINCT
- ⏳ Aggregations (COUNT, SUM, AVG, MIN, MAX)
- ⏳ GROUP BY, HAVING

### 4.3 Data Manipulation

**Features**:
- ⏳ UPDATE nodes/edges
- ⏳ DELETE nodes/edges (with cascade options)
- ⏳ MERGE (upsert semantics)
- ⏳ SET/REMOVE properties

**Deliverable**: Feature-rich query language

---

## Phase 5: Cross-Platform Expansion

**Goal**: WASM, mobile, multiple backends

**Status**: ⏳ Future

### 5.1 WebAssembly Support

**Features**:
- ⏳ IndexedDB backend for browsers
- ⏳ WASM compilation target
- ⏳ Browser-compatible API
- ⏳ Import/export between native and WASM

### 5.2 Mobile Optimization

**Features**:
- ⏳ iOS bindings (Swift FFI)
- ⏳ Android bindings (JNI/Kotlin)
- ⏳ Mobile-specific optimizations (battery, storage)
- ⏳ Background sync support

### 5.3 Backend Flexibility

**Features**:
- ⏳ Pluggable backends (users can implement `StorageBackend`)
- ⏳ Optional SQLite backend (feature flag)
- ⏳ Optional RocksDB backend (for advanced users, feature flag)

**Deliverable**: Run anywhere - desktop, mobile, web, embedded

---

## Phase 6: Ecosystem & Tooling

**Goal**: Developer experience and ecosystem

**Status**: ⏳ Future

### 6.1 Developer Tools

**Features**:
- ⏳ Visual query builder
- ⏳ Database inspector/debugger
- ⏳ Schema designer
- ⏳ Query profiler
- ⏳ Migration tools

### 6.2 Language Bindings

**Features**:
- ⏳ Python bindings
- ⏳ JavaScript/TypeScript bindings
- ⏳ C FFI for other languages

### 6.3 Documentation & Examples

**Features**:
- ⏳ Complete API reference
- ⏳ Query language specification
- ⏳ Cookbook (common patterns)
- ⏳ Performance tuning guide
- ⏳ Real-world examples

**Deliverable**: Production-ready ecosystem

---

## Phase 7: Enterprise Features (Optional)

**Goal**: Features for larger deployments

**Status**: ⏳ Future (Maybe)

**Features** (if needed):
- ⏳ Backup/restore utilities
- ⏳ Replication (read replicas)
- ⏳ Encryption at rest
- ⏳ Audit logging
- ⏳ Schema validation/enforcement

**Note**: These might be better as separate crates, not in core.

---

## GQL Spec Compliance Roadmap

**Note**: Separate from architecture phases above. These are **feature milestones** for GQL spec coverage, not project phases.

**Current**: ~2-5% of ISO/IEC 39075:2024

1. ✅ **Milestone 1**: Basic PoC - simple CRUD and queries
2. ⏳ **Milestone 2**: Complex patterns - multi-hop paths, variable-length
3. ⏳ **Milestone 3**: RETURN clause, projections, ORDER BY/LIMIT
4. ⏳ **Milestone 4**: UPDATE/DELETE operations
5. ⏳ **Milestone 5**: Aggregations and GROUP BY
6. ⏳ **Milestone 6**: Advanced expressions and operators
7. ⏳ **Milestone 7**: Schema, constraints, indexes
8. ⏳ **Milestone 8+**: Advanced types, multiple graphs, full spec compliance

**Important**: We prioritize practical usefulness over spec compliance. A 20% compliant implementation that's reliable is better than 100% compliance that's buggy.

---

## Performance Benchmarking

**Start After**: Phase 3.2 (indexes implemented)

**Benchmark Suite**:
- Insert throughput (nodes/sec, edges/sec)
- Query latency (indexed, unindexed)
- Memory usage (various dataset sizes)
- File size growth patterns
- Cache hit rates
- Transaction throughput

**Target Scales**:
- Small: 1K nodes, 5K edges (use case: personal notes)
- Medium: 100K nodes, 500K edges (use case: small business)
- Large: 1M nodes, 5M edges (use case: enterprise single-machine)

**Not Targeting**:
- Billions of nodes (use Neo4j, TigerGraph)
- Distributed systems (not our use case)
- Real-time analytics at scale (not our use case)

---

## Release Strategy

### v0.1.0 - ✅ Phase 1 Complete (PoC)
- In-memory graph database
- Basic query language
- REPL console

### v0.2.0 - ✅ Phase 2 Complete (Embeddable)
- Persistent storage
- Embedded database API
- Cross-platform file format
- Auto-save

### v0.3.0 - ⏳ Phase 3.1 (WAL & ACID)
- Write-ahead logging
- Transactions (BEGIN/COMMIT/ROLLBACK)
- Crash recovery
- ACID compliance

### v0.4.0 - ⏳ Phase 3.2 (Performance)
- B-tree indexes
- Query optimization
- Performance benchmarks

### v0.5.0 - ⏳ Phase 4 (Advanced Queries)
- Multi-hop paths
- RETURN clause
- Aggregations

### v1.0.0 - ⏳ Production Ready
- Stable API
- Stable file format
- Comprehensive tests
- Full documentation
- Performance validated
- Backwards compatibility promise

**Stability Promise**: After v1.0.0, we commit to:
- Backwards-compatible file format
- Stable public API (semantic versioning)
- Migration tools for any format changes
- Long-term support (decades)

---

## Decision Framework

When evaluating features, ask:

1. **Does it align with philosophy?** (embedded, reliable, simple)
2. **Is it needed for target use cases?** (not enterprise distributed)
3. **Does it compromise reliability?** (stability over features)
4. **Can it be a separate crate?** (keep core small)

**Say NO to**:
- Distributed consensus
- Multi-datacenter replication
- Built-in ML/AI
- Features only useful at massive scale
- Complex configuration

**Say YES to**:
- Crash safety
- Data integrity
- Query performance
- Developer experience
- Cross-platform support

---

## How to Contribute

See each phase for specific tasks. Good starting points:

**Easy**:
- Documentation improvements
- Example applications
- Test coverage
- Error message improvements

**Medium**:
- Additional query types
- Performance optimizations
- Platform-specific backends

**Hard**:
- WAL implementation
- B-tree indexes
- Query optimizer
- WASM backend

---

## Timeline (Rough Estimates)

- ✅ Phase 1: Complete
- ✅ Phase 2: Complete
- ⏳ Phase 3: 3-6 months
- ⏳ Phase 4: 4-6 months
- ⏳ Phase 5: 6-12 months
- ⏳ Phase 6: Ongoing
- ⏳ v1.0.0: 12-18 months from now

**Note**: This is a hobby project. Timeline is flexible.

---

## Current Focus

**Right Now**: Phase 3.1 - Write-Ahead Logging

**Next Steps**:
1. Design embedded WAL structure
2. Implement transaction API
3. Add crash recovery
4. Test unclean shutdowns
5. Benchmark transaction throughput

See [GitHub Issues](https://github.com/adityamukho/minigraf/issues) for specific tasks.

---

Last Updated: Phase 2 Complete (January 2026)
