# Minigraf Design Philosophy

> "Minigraf is not trying to replace Neo4j. It's trying to replace `serde_json` for graph data."

Minigraf aims to be **the embedded graph memory layer for AI agents, mobile apps, and the browser** — built on the SQLite philosophy: small, fast, reliable, zero-configuration, single-file.

## Why Datalog?

Minigraf uses Datalog as its query language. Here's why it's the right choice:

### 1. Better Philosophy Alignment

**Datalog is simpler** → Aligns with "do less, do it perfectly":
- Datalog spec: ~50 pages of core concepts
- Smaller surface area = fewer bugs, faster to production

**Datalog is proven** → 40+ years of production use (Datomic since 2012, XTDB, LogicBlox)
**Datalog is reliable** → Well-understood semantics, extensive research

### 2. Natural Fit for Temporal Databases

**Bi-temporal support was always the plan.** Datalog makes it natural:
- Facts are tuples: `(Entity, Attribute, Value, ValidFrom, ValidTo, TxTime)`
- Time is just another dimension in relations
- Temporal queries use simple predicates: `[(<= ?valid-from ?query-time)]`
- No special temporal syntax needed - it's just data

Bi-temporal is 3-4 months of proven patterns (Datomic/XTDB model).

### 3. Graph Traversal is MORE Powerful

**Recursive rules are first-class in Datalog:**
```datalog
[(reachable ?from ?to)
 [?from :connected ?to]]

[(reachable ?from ?to)
 [?from :connected ?intermediate]
 (reachable ?intermediate ?to)]
```

Transitive closure is native, not bolted on.

### 4. Faster Path to a Reliable Core

**Datalog**: proven implementation patterns (semi-naive evaluation, stratified negation) mean less of the engine is novel.

A smaller, well-understood query engine leaves more effort for the storage engine, which is where the data-integrity risk is.

### 5. Unique Market Position

**Datalog space**: Gap exists for single-file embedded bi-temporal DB

Minigraf = embedded graph memory for agents/mobile/browser + SQLite's simplicity + Datomic's temporal model (no one else offers this combination)

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
- Target: ~1.2MB budget for core engine (raised from the original <1MB goal for
  #277's structured error codes — 130 documented codes across 6 categories
  cannot fit in 1MB even with opt-level="z", LTO, codegen-units=1, and
  strip=symbols all already applied; see CI's `binary-size.yml` for the
  enforced limit)
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

**Philosophy**: Your graph database files should keep working, and every upgrade path should be written down before it is needed.

This section is a policy, not a promise of a frozen format. Minigraf has changed its file format when a data-integrity fix needed it (v7 → v8 in v3.0.0 fixes #371), and it may need to again. What it guarantees is how those changes happen.

**What counts as a format change**: any change to the on-disk layout of the header, fact pages, B+tree index pages or the WAL sidecar that an older release could not read correctly. Every format change bumps the version number in the file header. A release never changes the layout without bumping it.

**Format readability**:
- Release line v(N+1) always reads files written in format v(N) and migrates them automatically.
- Read support for format v(N−1) and older is dropped only in a major release, and the CHANGELOG of the previous major line announces the drop before it ships. v3.0.0 (format v8) reads v7 and drops v1–v6; every v1.x and v2.x release already migrates v1–v6 to v7, so opening a file once with any v2.x release makes it readable by v3.0.0.
- A format change never ships in a minor or patch release.

**Migration guarantee**: migration runs on open or checkpoint and needs no separate tool or configuration. Back up the file before the first open with a new major release. A migration that loses or alters facts is a data-integrity bug and is handled as one (see the support policy below).

**API stability**: semantic versioning. No breaking API changes in minor or patch releases; the `Policy` CI workflow checks this on every PR to `main`. Deprecations are announced at least one minor release before removal in the next major.

**Anti-pattern**: silent format changes, format changes in minor releases, migrations that need a separate tool.

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
- A written support policy (below), so users know which release lines get fixes and for how long
- Conservative, deliberate feature additions
- Major releases only when a data-integrity fix or a real API problem needs one, not for new features
- Focus on stability over novelty

**Inspiration**: SQLite has been maintained for 20+ years and is committed to 2050. Minigraf aims for the same longevity, and the support policy is the part of that aim it can commit to today.

#### Support policy

| Release line | Gets | For how long |
|---|---|---|
| Latest minor of the current major (today: 2.0.x) | All fixes: bugs, data integrity, security | Until the next release |
| Previous major, after the next major's `.0` release | Data-integrity and security fixes only, as patch releases | 12 months after the next major's `.0` release |
| Older lines | Nothing | — |

When v3.0.0 ships, v2.x gets data-integrity and security fixes for 12 months after that date. A data-integrity fix that needs a format change cannot ship on the older line; in that case the older line gets a documented workaround and the issue stays listed as a known issue until the line leaves support. #371 is the current example.

Data-integrity bugs stay visible until they are fixed in a published release; see the known-issues process in [CONTRIBUTING.md](CONTRIBUTING.md#known-issues-and-data-integrity-bugs).

#### Support tiers

"Supported" depends on which binding and which platform. Tier 1 is what the project tests and releases together; Tier 2 is built and smoke-tested but released on a best-effort schedule.

| Tier | Bindings | What it means |
|---|---|---|
| **Tier 1** | Rust crate (`minigraf`), Python (`minigraf` on PyPI) | Full test suite in CI. Released at the same time as every core release. Data-integrity fixes land here first. |
| **Tier 2 (experimental)** | Node.js, browser WASM, WASI, Java/JVM, Android, Swift (iOS/macOS), C | Built and smoke-tested in its own repo. Released on a best-effort schedule, possibly after the core release. |

A binding moves to Tier 1 when real users need it: issues from outside users, or known dependents. No binding is removed by being Tier 2.

| Tier | Platform and filesystem |
|---|---|
| **Tier 1** | Linux on ext4 or xfs, macOS on APFS, Windows on NTFS, all on local disk |
| **Supported with caveats** | NFSv4 with working locks. Network filesystems add latency and failure modes that local disks do not have. |
| **Unsupported for multiple writers** | NFSv3 mounted with `nolock`, NFSv3 without `lockd`, and FUSE filesystems without working locks (see [STG-027](docs/ERROR_REFERENCE.md#stg-027-filesystem-does-not-support-file-locking)) |

Browser (IndexedDB) and WASI storage follow the tier of their binding.

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

These are goals, not a description of today. You'll know Minigraf has succeeded when:

1. **Ubiquity**: Developers say "just use Minigraf" for embedded graph storage
2. **Trust**: Known for never losing data, crash-safe, reliable. Not yet met: v2.x has a known data-integrity bug (#371), and the work to meet this goal is tracked in #383. See the [known issues](https://github.com/project-minigraf/minigraf/issues?q=is%3Aissue+is%3Aopen+label%3Aknown-issue).
3. **Simplicity**: New users are productive in under 5 minutes
4. **Size**: Core binary within its ~1.2MB budget, minimal dependencies
5. **Portability**: Runs everywhere from Raspberry Pi to browsers
6. **Stability**: API hasn't broken in years
7. **Documentation**: Comprehensive docs with examples
8. **Longevity**: Still maintained and improved 10+ years later

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

1. **Stable** - Format changes follow the policy in §7: versioned, only in major releases, and v(N+1) always reads v(N)
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

Minigraf evolves conservatively: preserve the embedded, single-file model; prioritize correctness and compatibility; and keep optional capabilities outside the core where practical.

Completed release history is maintained in [CHANGELOG.md](CHANGELOG.md). Planned ecosystem work, exploratory ideas, and the v3.0.0 file-format policy are maintained in [ROADMAP.md](ROADMAP.md).
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
