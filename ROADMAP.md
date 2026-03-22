# Minigraf Roadmap

> The path from property graph PoC to production-ready bi-temporal Datalog database

**Philosophy**: Embedded graph memory for agents, mobile, and the browser — built on the SQLite approach: be boring, be reliable, be embeddable.

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

### 6.4 Benchmarks + Edge Case Tests + crates.io Publish 🎯 NEXT

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

**Edge Case Tests** (identified gaps from external code review):
- 🎯 Facts larger than ~4KB — verify the oversized-fact error path is exercised and the page layout is not corrupted
- 🎯 Checkpoint-during-crash — simulate a crash mid-checkpoint (partial page writes to `.graph` while WAL is being cleared) and verify recovery correctness
- 🎯 Error handling coverage — raise error-path coverage from ~82% toward the same bar as happy-path coverage; prioritise storage and WAL error paths

**Community Infrastructure** (do before publish):
- 🎯 Enable GitHub Discussions — minimum viable channel for questions, feedback, and contributor coordination before external users arrive via crates.io

**crates.io Publish Gate**:

v0.8.0 is the first public release. All items on the pre-publishing checklist in CLAUDE.md must pass before tagging:
- `cargo test` green on Linux, macOS, Windows
- `cargo clippy -- -D warnings` clean
- `cargo doc --no-deps` builds without warnings
- No `unwrap()`/`expect()` in library code paths
- `lib.rs` exports narrowed to public API only
- `clap` moved to `[[bin]]` deps only
- `Cargo.toml` metadata complete (`repository`, `keywords`, `categories`, `readme`, `documentation`)
- All doc examples compile and run

Publishing to crates.io at v0.8.0 is a **hard gate** — the project is invisible to the ecosystem until this happens. docs.rs auto-populates on publish, making the API reference available for free.

**Deliverable**: Validated performance numbers, strengthened crash-safety test coverage, and Minigraf published to crates.io as v0.8.0

**Timeline**: ~2-3 weeks

---

## Phase 7: Datalog Completeness 🎯 FUTURE

**Goal**: Negation, aggregation, and disjunction — the three features required for production-grade query workloads

**Status**: 🎯 Planned

**Priority**: 🔴 Critical — without these, realistic production queries cannot be expressed in Datalog

**Rationale**: The highlighted use cases (agentic memory, audit, mobile, browser) all require at minimum negation and aggregation. Expanding to mobile and WASM platforms before the query engine can express production-grade queries means shipping an incomplete product to more places. All three features are additive — existing queries continue to work unchanged. Semantics are well-established (Datomic and XTDB are production references for all three).

### 7.1 Stratified Negation (`not` / `not-join`)

**Goal**: Express "find entities where attribute X is absent" and similar absence queries.

**Why it's load-bearing**:
- Agentic memory: "what has the agent not verified?", "beliefs with no supporting evidence"
- Audit: "contracts without a sign-off event", "records missing a required field"
- Developer tooling: "modules with no dependents", "entities never retracted"
- Without negation these queries require pulling results into application memory and filtering — defeating the query engine

**Semantics**: Stratified negation (Datalog^¬) — the standard safe subset. The rule dependency graph is analysed at query time; programs where negation creates a recursive cycle are rejected with a clear error (unstable semantics). Non-recursive negation is always safe.

**Syntax** (Datomic-inspired):
```datalog
;; not — exclude bindings where sub-clause matches
(query [:find ?e
        :where [?e :person/name _]
               (not [?e :person/age _])])

;; not-join — exclude with shared variables from outer scope
(query [:find ?e
        :where [?e :task/status :pending]
               (not-join [?e]
                 [?e :task/blocked-by _])])
```

**Implementation**:
- Parser: add `not` and `not-join` clause types to `WhereClause` enum
- Stratification analysis: build rule dependency graph, detect negation cycles, return error if unstable
- Executor: evaluate negative sub-query against current binding set, subtract matching bindings
- All changes additive; existing queries unaffected

**Estimated complexity**: 2-4 weeks

### 7.2 Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`)

**Goal**: Express counting, summing, and extremes directly in queries rather than post-processing in application code.

**Why it's load-bearing**:
- Audit / compliance: "how many transactions in this time window?", "total value asserted per entity"
- Agentic memory: "how many beliefs does the agent hold about entity X?"
- Analytics: "most-referenced entity", "earliest valid-from per attribute"
- Without aggregation, every app that needs a count writes its own post-processing loop

**Syntax** (Datomic-inspired):
```datalog
;; count
(query [:find (count ?e)
        :where [?e :person/name _]])

;; sum with grouping (:with specifies grouping variables)
(query [:find ?dept (sum ?salary)
        :with ?e
        :where [?e :employee/dept ?dept]
               [?e :employee/salary ?salary]])

;; min / max
(query [:find (min ?ts)
        :where [?e :event/timestamp ?ts]])

;; distinct collect
(query [:find (distinct ?tag)
        :where [?e :note/tag ?tag]])
```

**Implementation**:
- Parser: add aggregate expression variants to the `:find` clause; add `:with` clause
- Types: `FindSpec::Aggregate { func, var }`, `AggregateFunc` enum
- Executor: post-process binding sets — group by non-aggregate find variables, apply aggregate functions; no changes to core evaluation engine
- All changes additive

**Estimated complexity**: 2-3 weeks

### 7.3 Disjunction (`or` / `or-join`)

**Goal**: Express "match condition A or condition B" without running two queries and unioning in application code.

**Why it's useful** (lower urgency than 7.1 and 7.2 — can be worked around, but becomes painful in complex rules):
- "Find notes tagged :work or :urgent"
- "Find entities where :status is :active or :pending"
- Recursive rules with branching reachability conditions

**Syntax** (Datomic-inspired):
```datalog
;; or — all branches must bind the same variables
(query [:find ?e
        :where (or [?e :task/status :active]
                   [?e :task/status :pending])])

;; or-join — branches may bind different variables, explicit join vars declared
(query [:find ?e
        :where (or-join [?e]
                 [?e :employee/dept :engineering]
                 (and [?e :employee/role :contractor]
                      [?e :employee/dept :product]))])
```

**Implementation**:
- Parser: add `or` and `or-join` clause types; add `and` grouping clause
- Executor: evaluate each branch independently against current binding set, union results, deduplicate
- `or-join`: validate that declared join variables appear in all branches
- All changes additive

**Estimated complexity**: 2-3 weeks

### 7.4 Tests + Error Coverage

- Unit tests for each new clause type (parser, types, matcher)
- Integration tests covering realistic production query patterns:
  - Absence queries with `not` / `not-join`
  - Aggregation with grouping, bi-temporal filters, and recursive rules
  - Disjunction in flat queries and rules
- Stratification rejection tests: programs with negation cycles must produce clear errors, not incorrect results
- Regression suite: all existing tests continue to pass

**Error handling coverage sweep**: Phase 7 adds significant new code paths (stratification analysis, aggregate post-processing, branch evaluation). Bring error-path coverage for new code to parity with happy-path coverage from the start, rather than letting it lag. Target: ≥90% overall branch coverage by end of Phase 7.

**Deliverable**: A Datalog engine that can express any query a production workload is likely to require — negation, aggregation, disjunction, and recursion, composable with bi-temporal filters; ≥90% branch coverage

**Timeline**: 6-8 weeks

---

## Phase 8: Cross-Platform Expansion 🎯 FUTURE

**Goal**: WASM, mobile, language bindings

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

**Rationale**: Cross-platform expansion delivers a complete query engine (Phase 7) to more environments — not an incomplete one. Phase 7 ships first.

### 8.1 WebAssembly Support

**Goal**: Run Minigraf as a WASM module in two distinct environments: browser (JavaScript/TypeScript) and server-side WASM runtimes (WASI).

There are two separate compilation targets with different requirements:

#### 8.1a Browser (`wasm32-unknown-unknown` + `wasm-bindgen`)

**Features**:
- 🎯 `IndexedDbBackend`: stub already exists in `src/storage/backend/indexeddb.rs`; implement using `web-sys` + `wasm-bindgen`
- 🎯 Annotate public API with `#[wasm_bindgen]` to expose it to JavaScript
- 🎯 Build with `wasm-pack` — it handles compilation, JS glue code, and TypeScript `.d.ts` generation automatically
- 🎯 TypeScript definition file auto-generated by `wasm-pack` — gives IDE auto-complete and type safety to JS/TS consumers (including AI coding assistants)
- 🎯 Publish to npm via `wasm-pack publish` — makes Minigraf discoverable to web-based AI frameworks (LangChain.js, etc.)
- 🎯 Export API to reconstruct a portable `.graph` blob from IndexedDB (same binary format as native)
- 🎯 Disable `optimizer.rs` under `wasm` feature flag (already gated)

**IndexedDB storage design: page-granular, not single-blob**

A naive implementation would store the entire `.graph` file as a single IndexedDB blob. This is simple but has unacceptable write amplification at any non-trivial scale:

| Scale | File size | Single-blob save cost |
|-------|-----------|----------------------|
| 10K facts | ~1.6MB | Write 1.6MB on every checkpoint |
| 100K facts | ~16MB | Write 16MB on every checkpoint |
| 1M facts | ~160MB | Write 160MB on every checkpoint |

The correct approach maps directly onto Minigraf's existing `StorageBackend` trait:

```
IndexedDB object store: { key: page_id (u64), value: page_bytes (4KB Uint8Array) }
```

- `read_page(id)` → IndexedDB `get(page_id)` — async, one record
- `write_page(id, bytes)` → IndexedDB `put(page_id, bytes)` — only dirty pages written
- The LRU page cache already sits in front of `StorageBackend` — hot pages never hit IndexedDB at all
- On checkpoint, only pages modified since the last checkpoint are written back

This is not a compromise of the single-file philosophy — logically, it is still one database. Physically, IndexedDB is a key-value store, not a filesystem; storing pages as records is the correct abstraction. The `.graph` binary format exists for portability between environments, not as a storage constraint within a browser.

**Export / import for portability**:
- Export: read all pages from IndexedDB in `page_id` order, concatenate, offer as a `.graph` file download
- Import: read a `.graph` file, split into pages, write each page to IndexedDB

This is a deliberate operation (a button or API call), not transparent — which is the right model for a browser environment.

**Browser-specific constraints**:
- No filesystem access in `wasm32-unknown-unknown` — all storage goes through IndexedDB
- No threads in standard browser WASM — lock-free or single-threaded execution paths required
- Binary size budget: target <1MB gzipped; audit dependencies under `wasm` feature

**Build toolchain**:
```bash
# Install wasm-pack
cargo install wasm-pack

# Build for browser (generates pkg/ with .js, .d.ts, .wasm)
wasm-pack build --target web

# Publish to npm
wasm-pack publish
```

**Deliverable**: npm package `@minigraf/core` — browser apps can `import { Minigraf } from '@minigraf/core'` and get full TypeScript types; page-granular IndexedDB backend keeps write amplification proportional to actual changes

#### 8.1b Server-side WASM (`wasm32-wasip1` / WASI)

**Goal**: Run Minigraf inside server-side WASM runtimes (Wasmtime, Wasmer, Cloudflare Workers with WASI, Fastly Compute).

**Why this matters**: Agent frameworks running in sandboxed server-side WASM environments (more secure than Docker containers) can embed Minigraf without any JavaScript bridge. Standard Rust code — no `wasm-bindgen`, no `#[wasm_bindgen]` annotations needed.

**Features**:
- 🎯 Verify `FileBackend` works correctly under WASI's capability-based filesystem
- 🎯 Compile to `wasm32-wasip1` target (formerly `wasm32-wasi`)
- 🎯 No storage backend changes needed — WASI exposes a filesystem API, so `FileBackend` works as-is with capability grants
- 🎯 Validate with Wasmtime and Wasmer runtimes in CI

**Build**:
```bash
cargo build --target wasm32-wasip1 --release
```

**Constraint**: WASI filesystem access requires the host runtime to grant explicit capability permissions (`--dir` in Wasmtime). Document this for users.

**Deliverable**: Minigraf `.wasm` binary runs under Wasmtime/Wasmer with file-backed storage; suitable for use in Cloudflare Workers (WASI) and similar edge runtimes

### 8.2 Mobile Bindings

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

### 8.3 Language Bindings

**Goal**: Python and C FFI as the highest-priority non-mobile targets (covers scripting, agent frameworks, and "any other language via C").

**Features**:
- 🎯 Python bindings via UniFFI (same `.udl` / proc-macro as mobile — no extra code)
- 🎯 C header (`minigraf.h`) via `cbindgen` for any language with a C FFI
- 🎯 Node.js / TypeScript bindings via `neon` or `napi-rs`
- 🎯 Published to PyPI (`minigraf`), npm (`@minigraf/core`)

**Note**: Python and C bindings share the UniFFI / cbindgen work done for mobile — the incremental cost is small once Phase 8.2 is complete.

**Deliverable**: Run anywhere - desktop, mobile, web, embedded; official packages on crates.io, PyPI, npm, Maven, Swift Package Index

**Timeline**: 3-4 months

---

## Phase 9: Ecosystem & Tooling 🎯 FUTURE

**Goal**: Developer experience and ecosystem

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

### 9.1 Developer Tools

**Features**:
- 🎯 Database inspector/debugger
- 🎯 Query profiler
- 🎯 Time travel visualizer
- 🎯 Migration tools

### 9.2 Documentation

**Features**:
- 🎯 Complete API reference (auto-generated via docs.rs; supplement with narrative guides)
- 🎯 Datalog language specification
- 🎯 Cookbook: common patterns (graph traversal, audit queries, time travel idioms)
- 🎯 Performance tuning guide
- 🎯 Error message guide — every user-facing error has a documented cause and resolution

### 9.3 Integration Examples

**Goal**: Close the gap between "interesting concept" and "I can use this today" for the agent and mobile audiences.

**Features**:
- 🎯 GraphRAG pattern: runnable example wiring Minigraf to a vector store (entity UUID as the bridge between fuzzy retrieval and structured graph traversal)
- 🎯 LangChain / LangChain.js integration example — agent memory backed by Minigraf
- 🎯 LlamaIndex integration example — Minigraf as a knowledge graph store
- 🎯 Standalone `examples/` crate with annotated end-to-end scenarios (agentic memory, offline-first mobile, audit log)

**Note**: These are documentation and example artifacts, not library features. They are the difference between "technically impressive" and "I can adopt this." Prioritise before or alongside the Phase 8 platform launch so the new audiences arriving via npm/PyPI/Swift Package Index have something runnable to start from.

### 9.4 Ecosystem Libraries

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

### v0.8.0 - 🎯 Phase 6.4 (Benchmarks + Edge Cases + **crates.io publish**)
- Criterion benchmark suite; validated performance at 10K / 100K / 1M facts
- Oversized-fact and checkpoint-during-crash edge case tests
- Error-path coverage raised from ~82%
- GitHub Discussions enabled
- **First public release on crates.io** — API reference auto-published to docs.rs

### v0.9.0 - 🎯 Phase 7 (Datalog Completeness)
- Stratified negation (`not` / `not-join`)
- Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`)
- Disjunction (`or` / `or-join`)

### v0.10.0 - 🎯 Phase 8 (Cross-platform)
- WASM support (browser + WASI)
- Mobile bindings (iOS + Android)
- Language bindings (Python, C, Node.js)

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
- 🎯 Phase 6.4: 2-3 weeks (Benchmarks + edge case tests + **crates.io publish**) - **NEXT** (Phase 6.3 query optimization completed in Phase 6.1)
- 🎯 Phase 7: 6-8 weeks (Datalog Completeness — negation, aggregation, disjunction; ≥90% branch coverage)
- 🎯 Phase 8: 3-4 months (Cross-platform — WASM, mobile, language bindings)
- 🎯 Phase 9: Ongoing (Ecosystem — integration examples, cookbook, GraphRAG/LangChain examples)
- 🎯 **v1.0.0: 9-12 months**

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
