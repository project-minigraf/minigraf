# Minigraf Design Philosophy

> "Minigraf is not trying to replace Neo4j. It's trying to replace `serde_json` for graph data."

Minigraf aims to be **the SQLite of bi-temporal graph databases** - a small, fast, reliable, embedded graph database with Datalog queries and time travel capabilities.

## Why Datalog?

**Strategic Decision** (January 2026): After completing Phase 2 with a GQL-inspired implementation, we pivoted to Datalog. Here's why:

### 1. Better Philosophy Alignment

**Datalog is simpler** → Aligns with "do less, do it perfectly":
- Datalog spec: ~50 pages of core concepts
- GQL spec: 600+ pages (ISO/IEC 39075:2024)
- Smaller surface area = fewer bugs, faster to production

**Datalog is proven** → 40+ years of production use (Datomic since 2012, XTDB, LogicBlox)
**Datalog is reliable** → Well-understood semantics, extensive research

### 2. Natural Fit for Temporal Databases

**Bi-temporal support was always the plan.** Datalog makes it natural:
- Facts are tuples: `(Entity, Attribute, Value, ValidFrom, ValidTo, TxTime)`
- Time is just another dimension in relations
- Temporal queries use simple predicates: `[(<= ?valid-from ?query-time)]`
- No special temporal syntax needed - it's just data

**With GQL**: Bi-temporal would be 12+ months of novel implementation (unclear semantics)
**With Datalog**: Bi-temporal is 3-4 months of proven patterns (Datomic/XTDB model)

### 3. Graph Traversal is MORE Powerful

**Recursive rules are first-class in Datalog:**
```datalog
[(reachable ?from ?to)
 [?from :connected ?to]]

[(reachable ?from ?to)
 [?from :connected ?intermediate]
 (reachable ?intermediate ?to)]
```

This is cleaner and more powerful than GQL's path patterns. Transitive closure is native, not bolted on.

### 4. Faster Path to Production

**GQL roadmap**: 24-30 months to production (catch up to GraphLite)
**Datalog roadmap**: 12-15 months to production (proven implementation patterns)

We can ship a useful, reliable database faster with Datalog.

### 5. Unique Market Position

**GQL space**: GraphLite already won (full spec, ACID, mature)
**Datalog space**: Gap exists for single-file embedded bi-temporal DB

Minigraf = SQLite + Datomic + single file (no one else offers this)

---

## Core Inspiration: SQLite

SQLite's success comes from a clear philosophy: be a library, not a server. Be small, not feature-complete. Be reliable, not cutting-edge. Minigraf adopts these same principles for graph databases.

## Guiding Principles

### 1. Zero-Configuration

**Philosophy**: It should just work, immediately, with no setup.

**Implementation**:
- No installation process beyond adding a dependency
- No server process to start or manage
- No configuration files to edit
- No connection strings or authentication for local use
- `Minigraf::open("data.graph")` and you're done

**Anti-pattern**: Requiring users to install external dependencies, start services, or edit config files.

### 2. Embedded-First Design

**Philosophy**: Minigraf is a library you link against, not a server you connect to.

**Implementation**:
- In-process execution - direct function calls, no network overhead
- Runs in the same address space as your application
- No client-server architecture for embedded use
- Network protocols are opt-in extensions, not core features

**Anti-pattern**: Designing for client-server first and retrofitting embedded mode.

**Target statement**: "The graph database you compile into your app, not connect to."

### 3. Single-File Database

**Philosophy**: All data in one portable file that's easy to manage.

**Implementation**:
- Single `.graph` file contains nodes, edges, properties, indexes, schema
- Easy to backup: copy one file
- Easy to share: email, USB drive, version control (for small DBs)
- Easy to delete: remove one file
- WASM: Store in browser's IndexedDB as single blob

**Anti-pattern**: Multiple files, directories, or complex file structures that are hard to manage.

### 4. Self-Contained

**Philosophy**: Minimal dependencies. Small binary size. No external requirements.

**Implementation**:
- Pure Rust implementation
- Minimal dependency tree (currently: serde, uuid, anyhow)
- No required system libraries (optional backends OK)
- Target: <1MB binary for core engine
- No runtime dependencies (no JVM, no Python, no Node.js)

**Anti-pattern**: Requiring external services, libraries, or runtimes to function.

### 5. Cross-Platform Portability

**Philosophy**: Run anywhere, from embedded devices to browsers to servers.

**Implementation**:
- Native: Linux, macOS, Windows, BSD, mobile
- WebAssembly: Run in any modern browser
- File format is endian-agnostic and cross-platform
- No platform-specific features in core (OS-specific optimizations OK)

**Target platforms**:
- Desktop: Windows, macOS, Linux
- Mobile: iOS, Android (via FFI/JNI)
- Web: WASM in browsers
- Embedded: Raspberry Pi, IoT devices
- Server: As a library in server applications

**Anti-pattern**: Platform-specific code in the core engine.

### 6. Reliability Over Features

**Philosophy**: It's better to do less and do it perfectly than to do more and do it poorly.

**Implementation**:
- ACID transactions (Atomicity, Consistency, Isolation, Durability)
- Write-ahead logging (WAL) for crash recovery
- Data integrity checks on every operation
- Rigorous testing (aim for 100% branch coverage)
- Conservative feature addition
- No data loss, ever

**Quality bar**:
- Every feature must be fully tested
- Every feature must handle edge cases
- Every feature must be crash-safe
- Prefer proven algorithms over novel ones

**Anti-pattern**: Adding features before existing ones are bulletproof.

### 7. Stability & Backwards Compatibility

**Philosophy**: Your graph database files should work forever.

**Implementation**:
- Stable file format once v1.0 ships
- Can read graphs created 20+ years ago
- API stability: semantic versioning, no breaking changes in minor versions
- Clear migration paths when absolutely necessary
- Deprecation warnings 12+ months before removal

**Commitment**: Once v1.0 ships, file format is stable for decades.

**Anti-pattern**: Breaking changes, format churn, forced migrations.

### 8. Performance Through Simplicity

**Philosophy**: Fast because simple, not simple because fast.

**Implementation**:
- Optimize the common case (small to medium graphs, <1M nodes)
- Page-based storage with locality of reference
- Indexes for frequently queried patterns
- Memory-mapped I/O where beneficial
- Avoid premature optimization

**Target performance**:
- Sub-millisecond queries for indexed lookups
- Thousands of transactions per second on commodity hardware
- Efficient memory usage (<100MB for medium graphs)

**Anti-pattern**: Complex optimization that sacrifices reliability or adds dependencies.

### 9. Well-Documented

**Philosophy**: Documentation is as important as code.

**Implementation**:
- Every public API has rustdoc comments with examples
- Query language reference manual (like SQL reference)
- Architecture documentation for contributors
- Performance tuning guide
- Common patterns and recipes
- Migration guides between versions

**Documentation types**:
- API reference (generated from code)
- User guide (getting started, tutorials)
- Query language specification
- Internals guide (for contributors)

**Anti-pattern**: "The code is the documentation."

### 10. Long-Term Support

**Philosophy**: This is a marathon, not a sprint.

**Implementation**:
- Commitment to decades of support
- Conservative, deliberate feature additions
- No rewrites or "version 2.0" churn
- Security patches for old versions
- Focus on stability over novelty

**Inspiration**: SQLite has been maintained for 20+ years and is committed to 2050.

**Anti-pattern**: Framework churn, major rewrites, abandoned versions.

## What Minigraf IS

✅ **An embedded graph database library**
- Link it into your application like SQLite
- Direct function calls, no network overhead
- Runs in-process with your app

✅ **A bi-temporal database**
- Track when facts were recorded (transaction time)
- Track when facts were valid in the real world (valid time)
- Time travel queries: see any point in history
- Audit trails and compliance built-in

✅ **A Datalog query engine**
- Recursive rules for graph traversal
- Logic programming paradigm
- Simpler than SQL, more powerful for graphs
- Proven semantics (40+ years of research)

✅ **A local-first storage solution**
- Perfect for desktop applications
- Ideal for mobile apps
- Great for WASM in browsers
- Suitable for embedded devices

✅ **A single-file graph store**
- One `.graph` file, easy to manage
- Portable across platforms
- Simple backup and versioning

✅ **A reliable, ACID-compliant database** (Phase 5)
- Transactions with rollback support
- Crash recovery via WAL
- Data integrity guarantees

✅ **A learning-friendly implementation**
- Readable Rust code
- Well-documented internals
- Clear architecture

## What Minigraf IS NOT

❌ **Not a distributed database**
- No clustering, no sharding, no replication
- Single-node only (by design)
- If you need distributed, use Neo4j or similar

❌ **Not a graph analytics engine**
- No built-in PageRank, community detection, etc.
- You can build these on top, or use external tools
- Focus is on storage and queries, not analytics

❌ **Not a client-server system**
- No network protocol in core
- No authentication/authorization layer
- No multi-user access control (use OS permissions)

❌ **Not enterprise-focused**
- No role-based access control (RBAC)
- No audit logging
- No high-availability features
- (These can be built on top if needed)

❌ **Not trying to be Neo4j**
- Different use case (embedded vs. server)
- Different scale (millions vs. billions of nodes)
- Different philosophy (library vs. service)

❌ **Not chasing feature parity with XTDB/Datomic**
- Simpler scope: single-file only
- No distributed features
- No vector search (separate crate if needed)
- Focus on reliability over features

## Target Use Cases

**Primary use cases** (optimize for these):

1. **Audit-heavy applications** - Finance, healthcare, legal (bi-temporal = compliance)
2. **Event sourcing** - Full history, time travel debugging
3. **Personal knowledge bases** - Obsidian, Logseq, Roam-like apps with provenance
4. **Mobile applications** - Local graph storage on phones/tablets
5. **Desktop applications** - Apps that need relationship data (IDEs, note-taking, etc.)
6. **Web applications (WASM)** - Client-side graph storage in browsers
7. **AI/RAG systems** - Knowledge graphs with temporal provenance
8. **Embedded devices** - IoT, edge computing with graph data
9. **Development/testing** - Local graph database for testing
10. **Small to medium production apps** - Where embedded DB is sufficient

**Secondary use cases** (should work, but not optimized for):

11. **Server applications** - Using Minigraf as an embedded component
12. **Data analysis** - Exploring graph datasets locally
13. **Education** - Learning Datalog and temporal databases

**Non-use cases** (explicitly out of scope):

- Large-scale distributed systems
- Multi-datacenter replication
- Billion-node graphs
- Real-time analytics at scale

## Design Decision Framework

When evaluating a feature or design choice, ask:

### 1. Does it align with "SQLite for graphs"?
- Would SQLite do this?
- Does it keep things simple and embedded?

### 2. Does it compromise reliability?
- Can it cause data loss or corruption?
- Does it make the codebase harder to test?

### 3. Does it add complexity?
- How many lines of code?
- How many new dependencies?
- Does it complicate the API?

### 4. Does it serve the primary use cases?
- Is this needed for embedded/mobile/WASM?
- Or is it only useful for enterprise/distributed?

### 5. Can it be a separate crate instead?
- Could this be an optional feature flag?
- Could this be a separate library on top of Minigraf?

### Decision rubric:
- **YES**: Aligns with philosophy, improves reliability, serves primary use cases
- **MAYBE**: Useful but adds complexity, consider making optional
- **NO**: Violates philosophy, compromises reliability, or only serves non-use cases

## Success Metrics

You'll know Minigraf has succeeded when:

1. ✅ **Ubiquity**: Developers say "just use Minigraf" for embedded graph storage
2. ✅ **Trust**: Known for never losing data, crash-safe, reliable
3. ✅ **Simplicity**: New users are productive in under 5 minutes
4. ✅ **Size**: Core binary under 1MB, minimal dependencies
5. ✅ **Portability**: Runs everywhere from Raspberry Pi to browsers
6. ✅ **Stability**: API hasn't broken in years
7. ✅ **Documentation**: Comprehensive docs with examples
8. ✅ **Longevity**: Still maintained and improved 10+ years later

## Non-Goals

To maintain focus, these are explicitly NOT goals:

- ❌ Distributed consensus algorithms
- ❌ Multi-master replication
- ❌ Built-in authentication/authorization
- ❌ Competing with Neo4j/TigerGraph on their turf
- ❌ Real-time analytics (OLAP workloads)
- ❌ Graph visualization (provide data, let others visualize)
- ❌ Built-in ML/AI (provide APIs for external tools)

## Testing Philosophy

Inspired by SQLite's legendary testing rigor:

**Test coverage goals**:
- 100% branch coverage (aspirational)
- Property-based testing (quickcheck, proptest)
- Fuzz testing (cargo-fuzz)
- Fault injection (simulate disk errors, OOM)
- Memory safety (miri, valgrind)
- Cross-platform testing (CI on Linux, macOS, Windows)

**Test-to-code ratio**: Aim for 5:1 (5x more test code than library code)

**Release criteria**:
- All tests pass on all platforms
- No memory leaks detected
- No undefined behavior (miri clean)
- Performance benchmarks within 5% of baseline
- Documentation complete for new features

## File Format Principles

The `.graph` file format must be:

1. **Stable** - Once v1.0 ships, format is frozen for decades
2. **Self-describing** - Header with magic number and version
3. **Portable** - Endian-agnostic, cross-platform
4. **Efficient** - Page-based, locality of reference
5. **Extensible** - Can add features without breaking old readers
6. **Verifiable** - Checksums for integrity validation

## API Design Principles

1. **Simple common case**: `db.add_node()` should be one line
2. **Safe by default**: Require `unsafe` only where truly needed
3. **Transactions explicit**: Clear when you're in a transaction
4. **Ergonomic errors**: `Result<T, Error>` with helpful messages
5. **Builder patterns**: Complex operations use builders
6. **Zero-cost abstractions**: No runtime penalty for nice APIs

## Evolution Strategy

**Phase 1**: ✅ Prove the concept (COMPLETE)
- Basic graph model, simple queries, in-memory storage

**Phase 2**: ✅ Embeddability (COMPLETE)
- Single-file storage, persistent graph database, embedded API
- GQL-inspired PoC (archived at `archive/gql-phase-2`)

**Phase 3**: ✅ Datalog Core (COMPLETE)
- EAV data model, basic facts and queries, recursive rules, semi-naive evaluation

**Phase 4**: 🎯 Bi-temporal Support (3-4 months)
- Transaction time + valid time, time travel queries, history

**Phase 5**: 🎯 ACID + WAL (2-3 months)
- Write-ahead logging, transactions, crash recovery

**Phase 6**: 🎯 Performance (2-3 months)
- Indexes (EAVT, AEVT, AVET, VAET), query optimization

**Phase 7**: 🎯 Cross-platform (3-4 months)
- WASM support, mobile bindings, language bindings

**Phase 8+**: 🎯 Long-term maintenance
- Bug fixes, security patches, conservative improvements

**v1.0.0**: 12-15 months (vs. 24-30 months with GQL)

See ROADMAP.md for detailed feature breakdown.
## When to Say "No"

It's important to say "no" to preserve the project's focus:

**Say NO to**:
- Features that only serve enterprise/distributed use cases
- Complexity that compromises reliability
- Dependencies that increase binary size significantly

- Breaking changes without overwhelming justification
- Features that should be separate libraries
- Premature optimization

**It's OK to say**: "That's a great feature, but it's better suited for a library built on top of Minigraf."

## Inspirations

Beyond SQLite, we draw inspiration from:

- **Datomic**: Immutable facts, temporal queries, Datalog
- **XTDB**: Bi-temporal database, time travel
- **Cozo**: Embedded Datalog, graph algorithms
- **Redis**: Simple, focused, well-documented
- **Git**: Single-file stores (packfiles), content-addressed storage
- **DuckDB**: Modern analytics, SQLite-style
- **Local-first software**: Offline-capable, user-owned data

## Closing Thoughts

Minigraf is a decades-long project. We optimize for:
- **Reliability** over features
- **Simplicity** over flexibility
- **Longevity** over hype
- **Users** over competitors

The goal is not to be the most feature-complete graph database. The goal is to be the one that's always there when you need it, works reliably, and never gets in your way.

Be boring. Be reliable. Be Minigraf.

---

*This document is a living guide. When in doubt, refer back to these principles.*
