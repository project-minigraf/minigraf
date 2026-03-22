# Minigraf

[![Build Status](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml)
[![Clippy Status](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/adityamukho/minigraf#license)
[![Rust Edition](https://img.shields.io/badge/rust-2024-orange.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)
[![Phase](https://img.shields.io/badge/phase-6.2%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)

> **Embedded graph memory for AI agents, mobile apps, and the browser** — the SQLite of bi-temporal graph databases

A tiny, self-contained graph database with **Datalog queries** and **bi-temporal time travel**. Think SQLite, but for connected data with full history.

## Vision

Minigraf is a **single-file embedded graph database** that lets you:
- ✅ **Query relationships with Datalog** - Recursive rules, natural graph traversal
- ✅ **Time travel through history** - Bi-temporal queries (transaction time + valid time)
- ✅ **Embed anywhere** - Native, WASM, mobile, IoT - one `.graph` file
- ✅ **Zero configuration** - Just `Minigraf::open("data.graph")` and you're done

**Status**: Early development. Phase 6.2 complete (Packed Pages + LRU Cache). Now starting Phase 6.3 (Benchmarks).

## Why Datalog?

**Datalog is fundamentally better for graphs than SQL-like languages:**

1. **Recursive by design** - Multi-hop traversals are natural, not an afterthought
2. **Simpler to implement** - Smaller spec = more reliable, faster to production
3. **Perfect for temporal** - Time is just another dimension in relations
4. **Proven at scale** - 40+ years of research, production use (Datomic, XTDB)
5. **Graph-native** - Facts (Entity-Attribute-Value) are literally edges

## Current Status - Phase 6.2 Complete

Minigraf has a **crash-safe bi-temporal Datalog query engine with covering indexes, packed storage, and LRU page cache**:

- ✅ **EAV data model** - Entity-Attribute-Value facts with transaction IDs
- ✅ **Datalog queries** - Pattern matching with variable unification
- ✅ **Recursive rules** - Semi-naive evaluation, transitive closure
- ✅ **Bi-temporal support** - Transaction time (`tx_id`, `tx_count`) + valid time (`valid_from`, `valid_to`)
- ✅ **Time travel queries** - `:as-of` (transaction counter or timestamp) + `:valid-at` (point-in-time)
- ✅ **Transact with valid time** - Per-transaction and per-fact valid time overrides
- ✅ **Write-ahead log** - Fact-level sidecar WAL with CRC32 protection
- ✅ **Crash recovery** - WAL replay on open; partial writes safely discarded
- ✅ **Explicit transactions** - `begin_write()` / `commit()` / `rollback()` API
- ✅ **Checkpoint** - WAL flushed to `.graph` file on demand or automatically
- ✅ **Covering indexes** - EAVT, AEVT, AVET, VAET with bi-temporal keys; B+tree persistence
- ✅ **Query optimizer** - Selectivity-based join reordering, index selection
- ✅ **Packed pages** - ~25 facts per 4KB page (~25× space reduction vs Phase 5)
- ✅ **LRU page cache** - Configurable bounded memory (`page_cache_size`, default 256 pages = 1MB)
- ✅ **On-demand fact loading** - No load-all at startup; committed facts resolved via page cache
- ✅ **File format v5** - Automatic migration from v1/v2/v3/v4
- ✅ **Single `.graph` file** - Page-based storage (4KB pages), WAL as sidecar
- ✅ **Embedded database API** - Use like SQLite (`Minigraf::open()`)
- ✅ **Cross-platform** - Works on Linux, macOS, Windows, iOS, Android
- ✅ **280 tests passing** - Comprehensive test coverage
- 🎯 **Next: Benchmarks** - Criterion suite, performance at scale (Phase 6.3)

## Quick Start

### Embedded Datalog Database (Working!)

```rust
use minigraf::{Minigraf, OpenOptions};

// Open or create a file-backed database
let db = OpenOptions::new().path("myapp.graph").open()?;

// Add facts via the Datalog REPL protocol
db.execute(r#"(transact [[:alice :person/name "Alice"]
                         [:alice :person/age 30]
                         [:alice :friend :bob]
                         [:bob :person/name "Bob"]])"#)?;

// Query with Datalog
let results = db.execute(r#"
    (query [:find ?friend-name
            :where [:alice :friend ?friend]
                   [?friend :person/name ?friend-name]])
"#)?;

// Explicit transaction — all-or-nothing
let mut tx = db.begin_write()?;
tx.execute(r#"(transact [[:alice :person/age 31]])"#)?;
tx.commit()?;  // or tx.rollback()

// Time travel — query as of past transaction counter
db.execute("(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])")?;
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

## Datalog Query Language

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

- **`src/graph/types.rs`**: Core EAV data structures (`Fact`, `Value`, bi-temporal fields)
- **`src/graph/storage.rs`**: In-memory fact store with temporal query methods and index-driven range scans
- **`src/storage/`**: Storage backend abstraction
  - **`mod.rs`**: `StorageBackend` trait, `FileHeader` v5, `CommittedFactReader` trait
  - **`backend/file.rs`**: Single-file persistent backend
  - **`backend/memory.rs`**: In-memory backend for testing
  - **`backend/indexeddb.rs`**: Future WASM backend
  - **`index.rs`**: EAVT/AEVT/AVET/VAET key types, `FactRef`, `encode_value`
  - **`btree.rs`**: B+tree page serialisation for index persistence
  - **`cache.rs`**: LRU page cache (`PageCache`, approximate-LRU)
  - **`packed_pages.rs`**: Packed fact page format (~25 facts/page)
  - **`persistent_facts.rs`**: v5 save/load, `CommittedFactLoaderImpl`
- **`src/wal.rs`**: Write-ahead log (`WalWriter`, `WalReader`, CRC32 entries)
- **`src/db.rs`**: Public API — `Minigraf`, `OpenOptions`, `WriteTransaction`
- **`src/query/datalog/parser.rs`**: EDN/Datalog parser
- **`src/query/datalog/executor.rs`**: Query executor with temporal filtering
- **`src/query/datalog/optimizer.rs`**: Selectivity-based query plan optimizer
- **`src/repl.rs`**: Interactive REPL console
- **`src/lib.rs`**: Public API exports
- **`src/main.rs`**: Binary entry point (`--file <path>` or in-memory)

### Data Model

- Facts: `(Entity, Attribute, Value, ValidFrom, ValidTo, TxTime)`
- Entities are just UUIDs
- Attributes are keywords (`:person/name`, `:friend`)
- Values can be primitives or entity references
- Time dimensions for bi-temporal support

### Storage Format

The `.graph` file uses a page-based format (like SQLite), with an optional WAL sidecar:

```
.graph file:
  Page 0: Header (72 bytes)
    - Magic "MGRF", version 5, page_count, fact_count
    - last_checkpointed_tx_count (WAL marker)
    - eavt/aevt/avet/vaet_root_page (covering index roots)
    - index_checksum (CRC32 of committed fact pages)
    - fact_page_format (0x02 = packed)

  Page 1+: Packed fact data pages (type 0x02)
    - 12-byte header: type, reserved, record_count, next_page
    - Record directory: (offset, length) per slot
    - Variable-length postcard-encoded facts

  Index pages (after fact data):
    - Serialised EAVT, AEVT, AVET, VAET BTreeMaps (type 0x11)

.wal sidecar (present when there are uncheckpointed writes):
  Header: Magic "MWAL", version
  Entries: checksum u32 | tx_count u64 | num_facts u64 | [len u32 | bytes]*
```

- **Page size**: 4KB (like SQLite)
- **Endian-safe**: Works across all platforms
- **~25 facts per page**: Packed format, ~25× smaller than Phase 5
- **Single `.graph` file**: WAL sidecar is deleted on clean close
- **CRC32-protected WAL**: Partial writes safely discarded on recovery
- **Stable format**: Automatic v1/v2/v3/v4→v5 migration; backwards compatible

## Roadmap

**Phase 1**: ✅ Property graph PoC (Complete)
**Phase 2**: ✅ Persistent storage (Complete)
**Phase 3**: ✅ Datalog core (Complete)
**Phase 4**: ✅ Bi-temporal support (Complete)
- Transaction time + valid time, time travel queries, file format v2

**Phase 5**: ✅ ACID + WAL (Complete)
- Write-ahead logging, explicit transactions, crash recovery, file format v3

**Phase 6.1**: ✅ Covering Indexes + Query Optimizer (Complete)
- EAVT/AEVT/AVET/VAET indexes, B+tree persistence, selectivity-based optimizer, file format v4

**Phase 6.2**: ✅ Packed Pages + LRU Cache (Complete)
- ~25 facts/page, LRU page cache, on-demand fact loading, file format v5

**Phase 6.3**: 🎯 Benchmarks (Next)
- Criterion suite, performance at 10K/100K/1M facts

**Phase 7**: 🎯 Datalog Completeness
- Stratified negation (`not` / `not-join`)
- Aggregation (`count`, `sum`, `min`, `max`, `:with`)
- Disjunction (`or` / `or-join`)

**Phase 8**: 🎯 Cross-platform
- WASM (browser via `wasm-pack` + npm; server-side via WASI)
- Mobile bindings (iOS `.xcframework`, Android `.aar` via UniFFI)
- Language bindings (Python, C, Node.js)

**v1.0.0**: Phase 8 complete

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

**Embedded graph memory for agents, mobile, and the browser — SQLite's simplicity + Datomic's temporal model**

### For AI Agents

Minigraf is a natural fit for agents that need **verifiable reasoning** — the ability to reconstruct exactly what the agent knew at the moment it made a decision, even after its beliefs have been updated or corrected.

Because every fact carries both a *transaction time* (when it was recorded) and a *valid time* (when it was true in the world), an agent's entire decision-making lineage is preserved and queryable:

```datalog
;; Agent records a belief
(transact [[:agent :belief/sky-color "blue"]])

;; Belief is later corrected
(retract [[:agent :belief/sky-color "blue"]])
(transact [[:agent :belief/sky-color "red"]])

;; Reconstruct what the agent believed at tx 1 — before the correction
(query [:find ?color :as-of 1 :where [:agent :belief/sky-color ?color]])
;; => "blue"
```

**Agentic use cases:**

- **Agent memory with provenance** - Store what an agent believes, retract and correct without losing history, replay past states to audit decisions
- **Verifiable reasoning** - Post-hoc root cause analysis: rewind to the exact knowledge state at the moment of a mistake
- **Task planning graphs** - Model a DAG of sub-tasks as a graph; update dependencies over time; query historical task states
- **Code dependency agents** - Embed call graphs or module dependency graphs; traverse them with recursive Datalog rules
- **Multi-agent coordination** - Each agent carries its own `.graph` file as a private, embedded memory store — no shared server required

**Why embedded (no server) is a feature for agents, not a limitation:**

An agent's memory is private to that agent instance. Embedding Minigraf directly in the agent's process means no network latency, no external service to manage, offline-capable operation, and a portable `.graph` file that travels with the agent. The single-file model also makes agent memory trivially snapshotable, versioned, or rolled back.

**Scope: per-agent-instance memory, not a shared fleet brain:**

Minigraf is designed for *one agent instance, one `.graph` file*. This is a deliberate constraint, not an oversight.

In a distributed fleet where multiple agent nodes need to share and synchronise a single memory store, Minigraf is the wrong tool — use a distributed database for that layer. But for the common case of an agent that handles a session, a task, or a user interaction on a single machine, the single-file model is an advantage: the agent's memory is private, fast, portable, and trivially snapshotable.

**Practical patterns for distributed deployments:**

- **Sticky sessions**: Route a given user or task ID consistently to the same node. The agent's local `.graph` stays coherent for the lifetime of that session.
- **Worker-local reasoning (L1 cache pattern)**: Use Minigraf for high-speed, private reasoning during a task. At task completion, flush the audited results to a central store (relational DB, distributed graph DB). Minigraf handles the "internal monologue"; the central store handles global state.
- **Swarm / multi-agent**: Each agent carries its own `.graph` brain. Agents coordinate by passing small, serialised sub-graphs to each other rather than sharing a database. Individual memories, explicit sync.

**A note on bitemporality and clock drift:**

Minigraf's transaction time is based on `tx_count` — a monotonic counter *per database instance*, not wall-clock time. Clock drift between machines does not affect the correctness of the bitemporal ordering within a single `.graph` file. The concern only arises if you attempt to merge two independently-operated instances, which Minigraf does not support.

**Pairing with vector stores (GraphRAG pattern):**

Minigraf has no vector search — and doesn't need it. In a complete agentic memory stack, the two layers are complementary:

| Layer | Tool | Job |
|-------|------|-----|
| Fuzzy retrieval | Vector store (Chroma, Pinecone, etc.) | "Find things similar to this prompt" |
| Relational backbone | Minigraf | "Follow this relationship, audit this fact, rewind to this moment" |

The recommended pattern: the vector store holds embeddings alongside an entity UUID; that UUID is the entry point into Minigraf where the bitemporal history and relationships live. Vectors find the starting node; Minigraf navigates and audits from there.

```
Vector store:  embedding → entity_uuid
                                │
                                ▼
Minigraf:      entity_uuid ── :approved-by ──▶ approver
                    │
                    └── :approved-at "2025-01-14T14:00:00Z"
                    └── tx history (who recorded this, when)
```

This keeps Minigraf lean (no vector index bloat) while giving agents both fuzzy discovery and deterministic, auditable relationship traversal.

### For Mobile Apps

Minigraf is a natural fit for mobile applications that need to store and query relational or graph-structured data locally, without a network call.

**Why embedded (no server) is the right model for mobile:**

Mobile apps operate in environments where connectivity is intermittent and latency is unacceptable. Embedding Minigraf directly in the app process means queries are local, the `.graph` file travels with the app's data directory, and there is no server to provision or authenticate against. It's the same reason SQLite dominates mobile relational storage.

**Why bitemporality matters especially on mobile:**

Mobile data is inherently eventually consistent. A user records a fact offline, syncs later, and then discovers the fact was wrong — they need to correct it retroactively. A uni-temporal database forces you to delete and re-insert, losing the original record. Minigraf's bi-temporal model lets you retract the incorrect fact and assert the corrected one while preserving the full history of what the app believed and when:

```datalog
;; User logs a health measurement offline
(transact {:valid-from "2025-06-01"}
          [[:user :health/weight-kg 82.5]])

;; Later corrects a mis-entered value — old record is preserved in history
(retract [[:user :health/weight-kg 82.5]])
(transact {:valid-from "2025-06-01"}
          [[:user :health/weight-kg 80.5]])

;; Reconstruct what the app recorded before the correction
(query [:find ?weight :as-of 1 :where [:user :health/weight-kg ?weight]])
;; => 82.5  (the original, uncorrected entry is still there)

;; Query what was actually true on 2025-06-01
(query [:find ?weight
        :valid-at "2025-06-01"
        :where [:user :health/weight-kg ?weight]])
;; => 80.5  (the corrected value)
```

**Mobile use cases:**

- **Health and fitness tracking** — Weight, nutrition, exercise logs with retroactive corrections. Bi-temporal means the app can distinguish "what I recorded" from "what was actually true" — useful for syncing corrections from a doctor or wearable after the fact.
- **Personal knowledge management** — Notes, tags, and links stored as a graph. Obsidian-like apps on mobile where offline-first is a requirement, not an option.
- **Game state** — RPG character graphs, quest dependency DAGs, world state with rollback. Bitemporality gives you cheap save states: record what the world looked like at each checkpoint, query it back without storing copies.
- **Local AI context** — On-device LLMs need structured facts to reason over. Minigraf acts as the relational backbone: store entities and relationships that the model can query instead of re-deriving from unstructured text.
- **Offline-first productivity apps** — Task managers, CRMs, project trackers where the device is the source of truth and sync is a background process. Each device carries its own `.graph`; sync at the application layer when connectivity is available.

**How integration works (Phase 7):**

Rust compiles to native machine code for each mobile architecture — the same binary performance as C or C++. Phase 7 will ship a `minigraf-ffi` crate using [UniFFI](https://github.com/mozilla/uniffi-rs) (Mozilla's binding generator) to auto-generate Kotlin and Swift wrappers. Release artifacts will be pre-built: a `.xcframework` for iOS (add to Xcode or import via Swift Package Manager) and a `.aar` for Android (drop into `libs/` or import via Gradle). Mobile developers will not need to touch Rust. See `ROADMAP.md` Phase 7.2 for the full integration plan.

**Practical note on sync:**

Minigraf does not provide built-in sync — this is intentional. Sync strategies are application-specific and often require domain knowledge about conflict resolution. The recommended pattern is: use Minigraf for high-speed local reasoning; at sync points, export the facts you want to share and merge them into a central store using your application's conflict resolution logic. The bitemporal timestamps make conflict detection straightforward: compare `tx_count` values to determine which device recorded a fact first.

### For WASM / Browser

Minigraf's single-file, zero-configuration design maps cleanly onto the browser environment. The planned Phase 7 WASM backend persists data in IndexedDB using page-granular records (one IndexedDB entry per 4KB page), giving browser applications a persistent, queryable graph database with no server required. The LRU page cache sits in front of IndexedDB, so hot pages are served from memory and only dirty pages are written back on checkpoint — write cost scales with what changed, not the total database size.

**Why a graph database in the browser:**

Most browser-side storage APIs (localStorage, IndexedDB raw) are key-value stores. They work for simple caching but are awkward for relational or graph-structured data — you end up maintaining foreign keys and join logic in JavaScript. Minigraf brings structured graph queries to the client, reducing the need for round-trips to a backend API.

**Why bitemporality matters for local-first web apps:**

Browser apps built on a local-first architecture face the same eventual-consistency problem as mobile: users make changes offline, and those changes may need to be corrected or reconciled later. The bitemporal model gives you a principled way to record corrections without destroying history — the same correction pattern shown in the mobile section applies here.

**WASM use cases:**

- **Offline PWAs** — A Progressive Web App that keeps its relational state in a `.graph` file in IndexedDB survives page reloads, browser restarts, and network outages. The app queries Minigraf directly from WASM; no fetch calls needed for local data.

- **Privacy-sensitive applications** — Medical records, financial data, personal journals. Process sensitive data entirely client-side: the data never leaves the browser, nothing is sent to a server, and the `.graph` file lives under the user's control. GDPR compliance becomes simpler when there is no server storing personal data.

- **In-browser analytics and exploration** — Load a dataset into Minigraf in the browser and run Datalog queries against it. Think Datasette or Observable notebooks, but for graph-structured data, with no backend required. Useful for data journalism, research tools, and interactive documentation.

- **Developer tooling** — Dependency graphs, call graphs, module relationship maps. A web-based IDE or code analysis tool can load a project's dependency graph into Minigraf and run recursive Datalog rules to find cycles, transitive dependencies, or impact analysis — all client-side, all queryable without a server round-trip:

```datalog
;; Find all transitive dependencies of a module
(rule [(depends-on ?a ?b)
       [?a :module/depends-directly ?b]])

(rule [(depends-on ?a ?b)
       [?a :module/depends-directly ?intermediate]
       (depends-on ?intermediate ?b)])

(query [:find ?dep
        :where (depends-on :my-module ?dep)])
```

- **Collaborative local-first tools** — Each browser tab or user session carries its own `.graph` file as the local replica. The application layer handles sync and conflict resolution; Minigraf handles fast, structured local queries. This is the same pattern as the mobile sync note above — Minigraf as the local reasoning layer, not the sync layer.

**How integration works (Phase 7):**

There are two distinct WASM targets, each with different requirements:

- **Browser (`wasm32-unknown-unknown`)**: Compiled and packaged with `wasm-pack`, which generates a `.wasm` binary, JavaScript glue code, and TypeScript `.d.ts` definitions automatically. Public API is annotated with `#[wasm_bindgen]`. Storage uses IndexedDB with **page-granular records** (`page_id → 4KB bytes`) — not a single blob — so only dirty pages are written back on checkpoint, keeping write amplification proportional to actual changes regardless of database size. Published to npm as `@minigraf/core` — consumable like any npm package, with full TypeScript auto-complete.

- **Server-side WASM (`wasm32-wasip1` / WASI)**: Standard `cargo build` to a WASI target — no `wasm-bindgen` or JavaScript bindings needed. Runs inside sandboxed runtimes like Wasmtime, Wasmer, or Cloudflare Workers (WASI mode). `FileBackend` works as-is because WASI exposes a capability-based filesystem API. More secure than Docker for agent sandboxing.

The `wasm` feature flag already gates `optimizer.rs` (which uses `std`-only code) to keep the browser binary lean. See `ROADMAP.md` Phase 7.1 for the full plan.

**Portability: export and import as a `.graph` file:**

Although the browser backend stores data as page-granular IndexedDB records internally, the `.graph` binary format is preserved for portability. Export (download a `.graph` file) reconstructs the file by reading pages in order; import (upload a `.graph` file) writes each page to IndexedDB. A database created on desktop can be loaded in the browser, and vice versa. This is a deliberate operation, not transparent — the right model for a browser environment where "files" are an explicit user action.

### Target Use Cases

1. **AI agents** - Verifiable reasoning, agent memory with provenance, task planning (see above)
2. **Mobile apps** - Offline-first, health/fitness tracking, game state, local AI context (see above)
3. **WASM / browser** - Offline PWAs, privacy-sensitive client-side apps, in-browser analytics (see above)
4. **Audit-heavy applications** - Finance, healthcare, legal (bi-temporal = compliance)
5. **Event sourcing** - Full history, time travel debugging
6. **Personal knowledge bases** - Obsidian, Logseq, Roam-like apps
7. **Local-first applications** - Offline-capable, user-owned data
8. **Development/testing** - Local graph DB like SQLite

### Philosophy: The SQLite Approach

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
- **Distributed** - No clustering, no sharding, no replication. For agents, this is intentional: each agent instance owns its own private `.graph` file. No shared server, no network calls, no contention.
- **Client-server** - No network protocol in core
- **Enterprise-focused** - No RBAC, no HA, no multi-datacenter
- **Billion-node scale** - Optimized for <1M nodes (like SQLite)

If you need a distributed graph database, use Neo4j, TigerGraph, or similar.

## Testing

Comprehensive test coverage:

```bash
cargo test
```

Current tests (280 total):
- ✅ **213 unit tests** - Core Datalog, EAV model, parser, matcher, executor, bi-temporal, WAL, indexes, cache, packed pages
- ✅ **61 integration tests** - Complex queries, recursive rules, concurrency, bi-temporal, WAL crash recovery, indexes, packed pages
- ✅ **6 doc tests** - Inline documentation examples

**Phase 3-4 Coverage** (Complete):
- ✅ Datalog parser (EDN syntax) and recursive rule evaluation
- ✅ Pattern matching, variable unification, transitive closure
- ✅ Bi-temporal queries (`:as-of`, `:valid-at`, `:any-valid-time`)
- ✅ Valid time filtering, file format v1→v2 migration, concurrency

**Phase 5 Coverage** (Complete):
- ✅ WAL write/read, CRC32, partial-entry discard
- ✅ Crash recovery via WAL replay (`mem::forget` simulation)
- ✅ Explicit transaction commit + rollback
- ✅ Checkpoint: WAL flushed to `.graph`, WAL deleted
- ✅ File format v2→v3 upgrade on checkpoint

**Phase 6.1 Coverage** (Complete):
- ✅ EAVT/AEVT/AVET/VAET index save/reload roundtrip
- ✅ B+tree multi-page spanning, sort order preservation
- ✅ Bi-temporal index keys, recursive rules regression

**Phase 6.2 Coverage** (Complete):
- ✅ Packed page compactness (1K facts fit in ≤50 pages)
- ✅ Bitemporal and as-of queries survive packed reload
- ✅ Explicit transaction survives packed reload
- ✅ `page_cache_size` option accepted

**Future tests** (Phase 6.3+):
- ⏳ Criterion benchmarks (insert throughput, query latency at scale)

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

**Minigraf aims to be the simplest, most portable option: embedded graph memory for agents, mobile, and the browser — built on SQLite's simplicity and Datomic's temporal model.**

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
