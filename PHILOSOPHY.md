# Minigraf Design Philosophy

> "Minigraf is not trying to replace Neo4j. It's trying to replace `serde_json` for graph data."

Minigraf aims to be **the SQLite of graph databases** - a small, fast, reliable, embedded graph database engine that runs in-process with your application.

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

✅ **A local-first storage solution**
- Perfect for desktop applications
- Ideal for mobile apps
- Great for WASM in browsers
- Suitable for embedded devices

✅ **A single-file graph store**
- One `.graph` file, easy to manage
- Portable across platforms
- Simple backup and versioning

✅ **A reliable, ACID-compliant database**
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

❌ **Not chasing full GQL spec compliance**
- Implement what's useful, not what's in the spec
- Pragmatic subset of GQL
- Extensions for common patterns

## Target Use Cases

**Primary use cases** (optimize for these):

1. **Mobile applications** - Local graph storage on phones/tablets
2. **Desktop applications** - Apps that need relationship data (IDEs, note-taking, etc.)
3. **Web applications (WASM)** - Client-side graph storage in browsers
4. **Embedded devices** - IoT, edge computing with graph data
5. **Development/testing** - Local graph database for testing
6. **Small to medium production apps** - Where embedded DB is sufficient

**Secondary use cases** (should work, but not optimized for):

7. **Server applications** - Using Minigraf as an embedded component
8. **Data analysis** - Exploring graph datasets locally
9. **Education** - Learning graph databases and algorithms

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
- ❌ Full GQL standard compliance (implement useful subset)
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

**Phase 3**: ⏳ Establish reliability (NEXT)
- WAL, ACID transactions, crash recovery, indexes, query optimization

**Phase 4**: ⏳ Expand reach
- WASM support, mobile bindings, multiple backends

**Phase 5**: ⏳ Mature the ecosystem
- Full documentation, tooling, ecosystem libraries

**Phase 6+**: ⏳ Long-term maintenance
- Bug fixes, security patches, conservative improvements

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

- **SQLite**: Embedded database philosophy
- **Redis**: Simple, focused, well-documented
- **Rust**: Memory safety, zero-cost abstractions
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
