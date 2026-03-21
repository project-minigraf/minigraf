# Minigraf

[![Build Status](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml)
[![Clippy Status](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/adityamukho/minigraf#license)
[![Rust Edition](https://img.shields.io/badge/rust-2024-orange.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)
[![Phase](https://img.shields.io/badge/phase-4%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)

> **The SQLite of bi-temporal graph databases** - Embedded Datalog engine written in Rust

A tiny, self-contained graph database with **Datalog queries** and **bi-temporal time travel**. Think SQLite, but for connected data with full history.

## Vision

Minigraf is a **single-file embedded graph database** that lets you:
- ✅ **Query relationships with Datalog** - Recursive rules, natural graph traversal
- ✅ **Time travel through history** - Bi-temporal queries (transaction time + valid time)
- ✅ **Embed anywhere** - Native, WASM, mobile, IoT - one `.graph` file
- ✅ **Zero configuration** - Just `Minigraf::open("data.graph")` and you're done

**Status**: Early development. Phase 4 complete (bi-temporal support). Now starting Phase 5 (ACID + WAL).

## Why Datalog?

**Datalog is fundamentally better for graphs than SQL-like languages:**

1. **Recursive by design** - Multi-hop traversals are natural, not an afterthought
2. **Simpler to implement** - Smaller spec = more reliable, faster to production
3. **Perfect for temporal** - Time is just another dimension in relations
4. **Proven at scale** - 40+ years of research, production use (Datomic, XTDB)
5. **Graph-native** - Facts (Entity-Attribute-Value) are literally edges

## Current Status - Phase 4 Complete

Minigraf has **full bi-temporal Datalog query engine**:

- ✅ **EAV data model** - Entity-Attribute-Value facts with transaction IDs
- ✅ **Datalog queries** - Pattern matching with variable unification
- ✅ **Recursive rules** - Semi-naive evaluation, transitive closure
- ✅ **Bi-temporal support** - Transaction time (`tx_id`, `tx_count`) + valid time (`valid_from`, `valid_to`)
- ✅ **Time travel queries** - `:as-of` (transaction counter or timestamp) + `:valid-at` (point-in-time)
- ✅ **Transact with valid time** - Per-transaction and per-fact valid time overrides
- ✅ **File format v2** - Automatic migration from v1 with correct temporal defaults
- ✅ **Single `.graph` file** - Page-based storage (4KB pages)
- ✅ **Embedded database API** - Use like SQLite (`Minigraf::open()`)
- ✅ **Cross-platform** - Works on Linux, macOS, Windows, iOS, Android
- ✅ **Auto-persistence** - Changes auto-save when database is dropped
- ✅ **172 tests passing** - Comprehensive test coverage
- 🎯 **Next: ACID + WAL** - Crash safety and transactions (Phase 5)

## Quick Start

### Embedded Datalog Database (Working!)

```rust
use minigraf::Minigraf;

// Open or create a database
let mut db = Minigraf::open("myapp.graph")?;

// Add facts (Entity-Attribute-Value triples)
db.transact(vec![
    [:alice, :person/name, "Alice"],
    [:alice, :person/age, 30],
    [:alice, :friend, :bob],
    [:bob, :person/name, "Bob"],
])?;

// Query with Datalog
let results = db.query("
    [:find ?friend-name
     :where
       [:alice :friend ?friend]
       [?friend :person/name ?friend-name]]
")?;

// Time travel - query as of past transaction
let past_db = db.as_of(tx_100);
let old_results = past_db.query(...)?;
```

### Interactive Console (Datalog REPL)

```bash
# Build and run the Datalog REPL
cargo run

# Run tests
cargo test

# Try the recursive rules demo
cargo run < demo_recursive.txt
```

## Datalog Query Language (Phase 3 - Working!)

### Basic Facts

```datalog
;; Add facts about entities
[:alice :person/name "Alice"]
[:alice :person/age 30]
[:alice :friend :bob]
[:bob :person/name "Bob"]
```

### Simple Queries

```datalog
;; Find all friends of Alice
[:find ?friend
 :where
   [:alice :friend ?friend]]

;; Find names of Alice's friends
[:find ?name
 :where
   [:alice :friend ?friend]
   [?friend :person/name ?name]]
```

### Recursive Rules (The Power of Datalog)

```datalog
;; Define transitive friendship
[(friends-network ?person ?reachable)
 [?person :friend ?reachable]]

[(friends-network ?person ?reachable)
 [?person :friend ?intermediate]
 (friends-network ?intermediate ?reachable)]

;; Find everyone in Alice's network
[:find ?person
 :where
   (friends-network :alice ?person)]
```

### Bi-temporal Queries (Phase 4 - Working!)

```datalog
;; Query valid at a specific date
[:find ?name
 :valid-at "2023-06-01"
 :where
   [:alice :person/name ?name]]

;; Query as of past transaction (counter or timestamp)
[:find ?friend
 :as-of 50
 :where
   [:alice :friend ?friend]]

;; Full bi-temporal query
[:find ?status
 :valid-at "2023-06-01"
 :as-of "2024-01-15T10:00:00Z"
 :where
   [:alice :employment/status ?status]]

;; Transact with explicit valid time
(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"}
          [[:alice :employment/status :active]])

;; Include all facts regardless of valid time
[:find ?name :valid-at :any-valid-time :where [?e :person/name ?name]]
```

## Architecture

### Module Structure

- **`src/graph/types.rs`**: Core graph data structures (EAV model)
- **`src/graph/storage.rs`**: In-memory storage layer
- **`src/storage/`**: Storage backend abstraction
  - **`mod.rs`**: StorageBackend trait, file format
  - **`backend/file.rs`**: Single-file persistent backend
  - **`backend/memory.rs`**: In-memory backend for testing
  - **`backend/indexeddb.rs`**: Future WASM backend
- **`src/query/datalog/parser.rs`**: Query parser
- **`src/query/datalog/executor.rs`**: Query executor
- **`src/repl.rs`**: Interactive REPL console
- **`src/lib.rs`**: Public API
- **`src/main.rs`**: Binary entry point

### Data Model

- Facts: `(Entity, Attribute, Value, ValidFrom, ValidTo, TxTime)`
- Entities are just UUIDs
- Attributes are keywords (`:person/name`, `:friend`)
- Values can be primitives or entity references
- Time dimensions for bi-temporal support

### Storage Format

The `.graph` file uses a page-based format (like SQLite):

```
+----------------+
| Page 0: Header | <- Magic "MGRF", version, page count
+----------------+
| Page 1: Facts  | <- (E, A, V, ValidFrom, ValidTo, TxTime)
+----------------+
| Page 2: Indexes| <- EAVT, AEVT, AVET, VAET indexes
+----------------+
| Page 3+: Data  |
+----------------+
```

- **Page size**: 4KB (like SQLite)
- **Endian-safe**: Works across all platforms
- **Single file**: Easy to backup, share, version control
- **Stable format**: Backwards compatible once v1.0 ships

## Roadmap

**Phase 1**: ✅ Property graph PoC (Complete)
**Phase 2**: ✅ Persistent storage (Complete)
**Phase 3**: ✅ Datalog core (Complete)
- ✅ Basic facts and queries
- ✅ Recursive rules
- ✅ Pattern matching

**Phase 4**: ✅ Bi-temporal support (Complete)
- Transaction time + valid time
- Time travel queries (`:as-of`, `:valid-at`)
- File format v2 with migration

**Phase 5**: 🎯 ACID + WAL
- Write-ahead logging
- Transactions
- Crash recovery

**Phase 6**: 🎯 Performance
- Indexes (EAVT, AEVT, AVET, VAET)
- Query optimization
- Benchmarking

**Phase 7**: 🎯 Cross-platform
- WASM (IndexedDB backend)
- Mobile bindings
- Language bindings

**v1.0.0**: Phase 7 complete

See [ROADMAP.md](ROADMAP.md) for detailed breakdown.

## Why Minigraf?

### Unique Positioning

No other database offers this combination:

| Feature | Minigraf | XTDB | Cozo | Neo4j | SQLite |
|---------|----------|------|------|-------|--------|
| **Query Language** | Datalog | Datalog | Datalog | Cypher | SQL |
| **Single File** | ✅ Yes | ❌ No | ❌ No | ❌ No | ✅ Yes |
| **Bi-temporal** | ✅ Yes | ✅ Yes | ⚠️ Time travel | ❌ No | ❌ No |
| **Embedded** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No | ✅ Yes |
| **Graph Native** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No |
| **Rust** | ✅ Yes | ❌ Clojure | ✅ Yes | ❌ Java | ❌ C |
| **WASM Ready** | 🎯 Goal | ❌ No | ⚠️ Limited | ❌ No | ✅ Yes |

**Minigraf = SQLite's simplicity + Datomic's temporal model**

### Target Use Cases

1. **Audit-heavy applications** - Finance, healthcare, legal (bi-temporal = compliance)
2. **Event sourcing** - Full history, time travel debugging
3. **Personal knowledge bases** - Obsidian, Logseq, Roam-like apps
4. **Local-first applications** - Offline-capable, user-owned data
5. **AI/RAG systems** - Knowledge graphs with provenance
6. **Mobile apps** - Embedded graph database on devices
7. **WASM applications** - Graph database in the browser
8. **Development/testing** - Local graph DB like SQLite

### Philosophy: The SQLite of Graph Databases

- **Zero-configuration** - No setup, just works
- **Embedded-first** - Library, not server
- **Single-file database** - Easy backup, share, version control
- **Self-contained** - <1MB binary, minimal dependencies (targetted)
- **Cross-platform** - Native, WASM, mobile, embedded
- **Reliability over features** - Do less, do it perfectly
- **Long-term support** - Decades-long commitment

See [PHILOSOPHY.md](PHILOSOPHY.md) for complete design principles.

## Scope

Minigraf is designed to run in multiple environments:
- ✅ As a standalone binary
- ✅ As an embedded library
- 🎯 As a WebAssembly module (future - Phase 7)

## Unscope

Minigraf will **NOT** be (by design):
- **Distributed** - No clustering, no sharding, no replication
- **Client-server** - No network protocol in core
- **Enterprise-focused** - No RBAC, no HA, no multi-datacenter
- **Billion-node scale** - Optimized for <1M nodes (like SQLite)

If you need these, use Neo4j, TigerGraph, or similar.

## Testing

Comprehensive test coverage:

```bash
cargo test
```

Current tests (172 total):
- ✅ **133 unit tests** - Core Datalog, EAV model, parser, matcher, executor, bi-temporal
- ✅ **36 integration tests** - Complex queries, recursive rules, concurrency, bi-temporal
- ✅ **3 doc tests** - Inline documentation examples

**Phase 3 Coverage** (Complete):
- ✅ Datalog parser (EDN syntax)
- ✅ Recursive rule evaluation (semi-naive)
- ✅ Transitive closure queries
- ✅ Pattern matching and unification
- ✅ Storage backend operations
- ✅ Concurrency and thread safety

**Phase 4 Coverage** (Complete):
- ✅ Bi-temporal queries (`:as-of` counter + timestamp, `:valid-at`)
- ✅ Valid time filtering (inside/outside range, boundary, default)
- ✅ Transaction with valid time (batch + per-fact override)
- ✅ File format v1→v2 migration
- ✅ tx_count increment and snapshot correctness

**Future tests** (Phase 5+):
- ⏳ Transaction isolation (ACID)
- ⏳ Crash recovery (WAL)

## Comparison to Similar Projects

### vs. XTDB (formerly Crux)
- ✅ **Minigraf**: Single `.graph` file, simpler scope
- ✅ **XTDB**: More mature, production-ready, but Clojure + multi-file storage

### vs. Cozo
- ✅ **Minigraf**: Single file, bi-temporal first-class
- ✅ **Cozo**: More features (vector search, time travel), but multi-file storage

### vs. GraphLite
- ✅ **Minigraf**: Datalog (recursive rules), bi-temporal
- ✅ **GraphLite**: Full GQL compliance, but multi-file (Sled directories)

### vs. Datomic
- ✅ **Minigraf**: Single file, embedded, Rust
- ✅ **Datomic**: Production-proven, but client-server, Clojure, proprietary

**Minigraf aims to be the simplest, most portable option: SQLite's simplicity + Datomic's temporal model.**

## Contributing

This is a hobby project with a long-term vision. Contributions welcome, but we prioritize:
1. Reliability over features
2. Simplicity over flexibility
3. Philosophy alignment

See [ROADMAP.md](ROADMAP.md) and [PHILOSOPHY.md](PHILOSOPHY.md) before proposing features.

## Learning Resources

### Datalog
- [Learn Datalog Today](http://www.learndatalogtoday.org/)
- [Datomic Query Tutorial](https://docs.datomic.com/query/query-tutorial.html)
- [XTDB Datalog Queries](https://xtdb.com/docs/)

### Temporal Databases
- [Temporal Database Wikipedia](https://en.wikipedia.org/wiki/Temporal_database)
- [XTDB Bitemporality](https://v1-docs.xtdb.com/concepts/bitemporality/)
- [Datomic Time Model](https://docs.datomic.com/time/time-model.html)

### SQLite's VFS
- [SQLite OS Interface](https://www.sqlite.org/vfs.html)
- [SQLite File Format](https://www.sqlite.org/fileformat.html)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
