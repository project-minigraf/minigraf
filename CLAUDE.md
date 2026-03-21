# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable **bi-temporal graph database with Datalog queries** written in Rust. It's designed to be the "SQLite of graph databases" - embedded, single-file, reliable, with time travel capabilities.

**Current Status: Phase 4 COMPLETE ✅ → Phase 5 Starting** - Bi-temporal Support:
- ✅ Phase 1: Property graph PoC (in-memory)
- ✅ Phase 2: Persistent storage (`.graph` file format, embedded API)
- ✅ Phase 3: Datalog core (EAV model, recursive rules) - COMPLETE!
- ✅ **Phase 4: Bi-temporal support (transaction time + valid time) - COMPLETE!**
- 🎯 Phase 5: ACID + WAL (crash safety, transactions) - **NEXT**
- 🎯 Phase 6: Performance (indexes, query optimization)
- 🎯 v1.0.0: 10-13 months

**Important Strategic Pivot** (January 2026): After completing Phase 2 with a GQL-inspired implementation, we pivoted to Datalog for:
1. Simpler implementation (proven patterns vs. novel GQL spec)
2. Better temporal semantics (bi-temporal is natural in Datalog)
3. Faster time-to-production (12-15 months vs. 24-30 months)
4. Unique market positioning (single-file bi-temporal Datalog doesn't exist)

**GQL Archive**: Previous GQL implementation preserved at `archive/gql-phase-2` branch and `gql-phase-2-complete` tag.

## Core Philosophy - CRITICAL

**Before implementing ANY feature or change, you MUST assess it against the design philosophy in PHILOSOPHY.md.**

Minigraf follows the "SQLite for bi-temporal graph databases" philosophy:
- **Zero-configuration** - No setup, no config files, just works
- **Embedded-first** - Library not server, in-process execution
- **Single-file database** - One portable `.graph` file
- **Self-contained** - Minimal dependencies, small binary (<1MB goal)
- **Cross-platform** - Native, WASM, mobile, embedded
- **Reliability over features** - Do less, do it perfectly
- **Bi-temporal first-class** - Time travel is a core feature, not addon
- **Datalog queries** - Simpler, more powerful for graphs than SQL/GQL
- **Stability** - Backwards compatibility, stable file format
- **Long-term support** - Decades-long commitment

### Why Datalog?

**Critical Context**: We chose Datalog over GQL because:
1. **Simpler** - 50 pages vs. 600-page spec
2. **Proven** - 40+ years, Datomic/XTDB production use
3. **Better for temporal** - Time is just another dimension
4. **Recursive rules** - First-class graph traversal
5. **Faster to production** - 12-15 months vs. 24-30 months

### Philosophy Compliance Check

**CRITICAL INSTRUCTION**: When the user requests a feature or change, you MUST:

1. **First**, assess whether it aligns with the philosophy in PHILOSOPHY.md
2. **If it violates the philosophy**, WARN the user BEFORE implementing:
   - Explain which principle(s) it violates
   - Explain why it's problematic
   - Suggest alternatives that align with the philosophy
   - Ask for explicit confirmation to proceed despite the violation
3. **If it aligns**, proceed with implementation

**Examples of philosophy violations to warn about**:
- Adding client-server architecture (violates embedded-first)
- Requiring external services or complex setup (violates zero-configuration)
- Large dependencies that bloat binary size (violates self-contained)
- Features only useful for distributed systems (violates target use cases)
- Breaking changes to API or file format (violates stability)
- Complex features before basics are reliable (violates reliability-first)
- Multi-file storage (violates single-file philosophy)

**Your response format when detecting a violation**:
```
⚠️ PHILOSOPHY WARNING ⚠️

The requested feature/change may violate Minigraf's core philosophy:

**Violated Principle**: [principle name from PHILOSOPHY.md]

**Why this is problematic**: [explanation]

**Philosophy-aligned alternatives**:
- [alternative 1]
- [alternative 2]

Do you want to proceed anyway? This would be a deviation from the "SQLite for graph databases" philosophy.
```

See PHILOSOPHY.md for complete design principles and decision framework.

## Build and Run Commands

```bash
# Build the project
cargo build

# Build release version (with panic=abort optimization)
cargo build --release

# Run the REPL (Datalog with bi-temporal support)
cargo run

# Run tests
cargo test

# Run specific test suite
cargo test --test bitemporal_test -- --nocapture
cargo test --test complex_queries_test -- --nocapture
cargo test --test recursive_rules_test -- --nocapture
cargo test --test concurrency_test

# Run examples
cargo run --example embedded
cargo run --example file_storage
```

## Architecture

### Module Structure

The codebase is organized into the following modules:

1. **Graph Module (`src/graph/`)** - Phase 3-4 (EAV with bi-temporal) ✅:
   - `types.rs`: EAV model types
     - `Fact`: entity, attribute, value, tx_id, tx_count, valid_from, valid_to, asserted
     - `Value`: String, Integer, Float, Boolean, Ref(Uuid), Keyword, Null
     - `EntityId`, `TxId`, `Attribute` type aliases
     - `VALID_TIME_FOREVER = i64::MAX` sentinel
   - `storage.rs`: `FactStorage` - fact-based in-memory storage
     - `tx_counter` (AtomicU64), transact/retract operations
     - `get_facts_as_of()`, `get_facts_valid_at()` for time travel
     - Thread-safe via `Arc<RwLock<...>>`

2. **Storage Module (`src/storage/`)** - Phase 2-4 (stable foundation) ✅:
   - `mod.rs`: `StorageBackend` trait and file format
     - `StorageBackend` trait: Platform-agnostic storage interface
     - `FileHeader`: Metadata for `.graph` files (v2 format)
     - Page size: 4KB, Magic number: "MGRF"
   - `backend/file.rs`: File-based backend (single `.graph` file)
     - Page-based storage, cross-platform format
     - Supports Linux, macOS, Windows, iOS, Android
   - `backend/memory.rs`: In-memory backend for testing
   - `backend/indexeddb.rs`: Future WASM browser backend (Phase 7)
   - `persistent_facts.rs`: Persistent EAV fact storage layer
     - postcard serialization of facts with temporal fields
     - `migrate_v1_to_v2()` for file format migration

3. **Query Module (`src/query/datalog/`)** - Phase 3-4 (Datalog + bi-temporal) ✅:
   - `parser.rs`: EDN/Datalog parser
     - Parses `transact`, `retract`, `query`, `rule` commands
     - Supports `:as-of` (tx counter or ISO 8601 timestamp), `:valid-at`
     - EDN maps `{:key val}` for transaction-level valid time options
     - Per-fact 4-element vector override for valid time
   - `executor.rs`: Datalog query executor
     - Pattern matching with variable unification
     - Rule registration and invocation
     - 3-step temporal filter: tx-time → asserted exclusion → valid-time
   - `matcher.rs`: Pattern matching engine with variable binding
   - `evaluator.rs`: `RecursiveEvaluator` - semi-naive fixed-point iteration
   - `rules.rs`: `RuleRegistry` - thread-safe rule management
   - `types.rs`: `EdnValue`, `Pattern`, `DatalogQuery`, `AsOf`, `ValidAt`

4. **Temporal Module (`src/temporal.rs`)** - Phase 4 ✅:
   - UTC-only timestamp parsing and formatting
   - Avoids chrono CVE GHSA-wcg3-cvx6-7396

5. **REPL Module (`src/repl.rs`)** - Phase 3-4 ✅:
   - Interactive Datalog console with bi-temporal support
   - Multi-line input, comment support
   - Prompt-based interface (`minigraf>`)
   - Handles EOF gracefully

6. **Minigraf Module (`src/minigraf.rs`)** - Phase 2+ (stable) ✅:
   - Public embedded database API
   - `Minigraf::open()` - Opens or creates database
   - `Minigraf::execute()` - Executes Datalog queries
   - `Minigraf::save()` - Explicit save
   - Auto-save on drop

7. **Library (`src/lib.rs`)**: Public API exports

8. **Binary (`src/main.rs`)**: Standalone executable
   - Launches interactive Datalog REPL
   - Supports both file-backed and in-memory storage

### Current Data Model (Phase 3-4)

**Entity-Attribute-Value with Bi-temporal Support**:
```rust
struct Fact {
    entity: EntityId,     // Uuid - entity being described
    attribute: Attribute, // String, e.g., ":person/name", ":friend"
    value: Value,
    tx_id: TxId,          // Uuid - transaction that asserted this
    tx_count: u64,        // Monotonic transaction counter (for :as-of queries)
    valid_from: i64,      // Unix ms - when fact became valid in real world
    valid_to: i64,        // Unix ms - when fact stopped being valid (i64::MAX = forever)
    asserted: bool,       // true = assert, false = retract
}

enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Ref(Uuid),          // Reference to another entity
    Keyword(String),    // e.g., ":person", ":status/active"
    Null,
}

const VALID_TIME_FOREVER: i64 = i64::MAX; // Sentinel for open-ended valid time
```

### Storage Implementation

**Layered Architecture**:

**High-level** (Phase 3-4) ✅:
- `FactStorage`: In-memory EAV fact store with temporal query methods
- `PersistentFactStorage`: Serialization layer (postcard, v2 file format)

**Low-level** (Phase 2, stable foundation) ✅:
- `StorageBackend` trait: Platform-agnostic interface
- `FileBackend`: Single `.graph` file (4KB pages)
- `MemoryBackend`: In-memory for testing
- Future: `IndexedDbBackend` for WASM

**File Format** (v2):
```
Page 0: Header
  - Magic: "MGRF"
  - Version: u32 (currently 2)
  - Page count: u64
  - Fact count: u64
  - Tx counter: u64

Page 1+: Data
  - EAV facts with full bi-temporal fields (postcard serialization)
```

**Serialization Format**:
- Using **postcard** (v1.0+) for fact serialization
- Replaced bincode (unmaintained as of 2024/2025)
- postcard: Lightweight, embedded-focused, better size than bincode
- Future consideration: Evaluate **rkyv** in Phase 5/6 for zero-copy
  deserialization when implementing WAL or memory-mapped access

### Query Language

**Current: Datalog with bi-temporal support (Phase 3-4)** ✅:
```datalog
;; Transact facts
(transact [[:alice :person/name "Alice"]
           [:alice :person/age 30]
           [:alice :friend :bob]])

;; Transact with transaction-level valid time
(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"}
          [[:alice :employment/status :employed]])

;; Simple query
(query [:find ?name
        :where [?e :person/name ?name]])

;; Recursive rule
(rule [(reachable ?from ?to)
       [?from :connected ?to]])

(rule [(reachable ?from ?to)
       [?from :connected ?intermediate]
       (reachable ?intermediate ?to)])

;; Time travel: as-of tx counter
(query [:find ?status
        :as-of 50
        :where [?e :employment/status ?status]])

;; Time travel: as-of ISO 8601 timestamp
(query [:find ?status
        :as-of "2024-01-15T10:00:00Z"
        :where [?e :employment/status ?status]])

;; Valid-time query
(query [:find ?status
        :valid-at "2023-06-01"
        :where [:alice :employment/status ?status]])
```

**Archived: GQL-inspired syntax (Phase 1-2)** - see `archive/gql-phase-2` branch:
```gql
CREATE NODE (:Person) {name: "Alice", age: 30}
CREATE EDGE (id1)-[KNOWS]->(id2) {since: 2020}
MATCH (:Person) WHERE name = "Alice"
```

### Error Handling

- Parse errors: Descriptive messages, REPL continues
- Execution errors: Validated before execution
- EOF handling: REPL exits gracefully (for piped input)
- Storage errors: Result<T, Error> with context

## Test Coverage

**Current Tests (Phase 4)**: 172 tests passing ✅
- **Unit tests** (133 tests):
  - `src/graph/types.rs`: Fact types, Value types, EAV model, temporal fields
  - `src/graph/storage.rs`: FactStorage, CRUD, history, tx_count, temporal methods
  - `src/temporal.rs`: UTC timestamp parsing and formatting
  - `src/query/datalog/parser.rs`: EDN/Datalog syntax, rules, `:as-of`, `:valid-at`, EDN maps
  - `src/query/datalog/types.rs`: Pattern, WhereClause, DatalogQuery, AsOf, ValidAt
  - `src/query/datalog/matcher.rs`: Pattern matching, variable unification
  - `src/query/datalog/executor.rs`: Query execution, rule registration, temporal filtering
  - `src/query/datalog/rules.rs`: RuleRegistry, rule management
  - `src/query/datalog/evaluator.rs`: Semi-naive evaluation, transitive closure
  - `src/storage/`: Backend operations, persistence (postcard)

- **Integration tests** (36 tests):
  - `tests/bitemporal_test.rs` (10 tests): Bi-temporal queries, time travel, valid time
  - `tests/complex_queries_test.rs` (10 tests): Multi-pattern joins, self-joins, edge cases
  - `tests/recursive_rules_test.rs` (9 tests): Transitive closure, cycles, long chains, family trees
  - `tests/concurrency_test.rs` (7 tests): Thread safety, concurrent rule registration/queries

- **Doc tests** (3 tests): Inline documentation examples

**Comprehensive Coverage**:
- ✅ Datalog parser (EDN syntax)
- ✅ Pattern matching and unification
- ✅ Recursive rule evaluation (semi-naive)
- ✅ Transitive closure - 9 tests
- ✅ Concurrency - 7 tests
- ✅ Complex queries (3+ patterns, self-joins) - 10 tests
- ✅ **Bi-temporal queries** (`:as-of`, `:valid-at`) - 10 integration + 39 unit tests
- ✅ **File format migration** (v1→v2)

**Demo Scripts**:
- `demo_recursive.txt`: Comprehensive recursive rules examples (transitive closure, cycles, family trees)

Run tests with: `cargo test`
See `TEST_COVERAGE.md` for detailed coverage report.

**Future Tests (Phase 5+)**:
- WAL and crash recovery - Phase 5
- Index performance - Phase 6

## Development Notes

### Phase 2 (Complete) - Foundation ✅

- **Storage backend abstraction** - Solid foundation for Phase 3+
- **Single `.graph` file** - Philosophy-aligned persistent storage
- **Embedded API** - `Minigraf::open()` works like SQLite
- **UUID-based IDs** - Continues in EAV model
- **Thread-safe** - Concurrent read/write via RwLock
- **Auto-save** - On drop, works reliably
- **Cross-platform** - Endian-safe file format

### Phase 3 (Complete) - Datalog Core ✅

**Implemented Features**:
- ✅ EAV data model with Facts (entity, attribute, value, tx_id, asserted)
- ✅ Datalog parser (EDN syntax, lists, vectors, UUIDs, keywords)
- ✅ Pattern matching engine with variable unification
- ✅ Query executor (transact, retract, query, rule commands)
- ✅ **Recursive rules with semi-naive evaluation** (fixed-point iteration)
- ✅ **Transitive closure queries** (multi-hop reachability)
- ✅ **Cycle handling** (graphs with cycles converge correctly)
- ✅ RuleRegistry (thread-safe rule management)
- ✅ RecursiveEvaluator (delta-based fixed-point iteration)
- ✅ REPL with multi-line support and comments
- ✅ Persistent storage with postcard serialization

**Test Coverage**: 123 tests (94 unit + 26 integration + 3 doc)

**Demo**: `demo_recursive.txt` - Working examples of recursive rules

### Phase 4 (Complete) - Bi-temporal Support ✅

**Implemented Features**:
- ✅ Extended `Fact` struct: `tx_count`, `valid_from`, `valid_to`
- ✅ `VALID_TIME_FOREVER = i64::MAX` sentinel
- ✅ `FactStorage`: `tx_counter` (AtomicU64), `load_fact()`, `get_facts_as_of()`, `get_facts_valid_at()`
- ✅ `TransactOptions { valid_from, valid_to }` for batch-level valid time
- ✅ Parser: EDN maps (`{:key val}`), `:as-of` (counter + ISO 8601), `:valid-at` (timestamp + `:any-valid-time`)
- ✅ Parser: `(transact {...} [...])` with transaction-level valid time; per-fact 4-element vector override
- ✅ Executor: 3-step temporal filter (tx-time → asserted exclusion → valid-time)
- ✅ File format v1→v2 migration in `migrate_v1_to_v2()`
- ✅ Fixed latent Phase 3 bug: `tx_id` preserved on load via `load_fact()`
- ✅ UTC-only timestamp parsing (`src/temporal.rs`, chrono, avoids GHSA-wcg3-cvx6-7396)

**Test Coverage**: 172 tests (133 unit + 36 integration + 3 doc)

### Phase 5 (Next) - ACID + WAL 🎯

**Planned Features**:
- Write-ahead logging (embedded in `.graph` file)
- Transaction API (BEGIN, COMMIT, ROLLBACK)
- Crash recovery

### Philosophy-Aligned Development

When implementing features, always ask:
1. Does this keep the single-file philosophy?
2. Does this maintain zero-configuration?
3. Does this add unnecessary complexity?
4. Is this needed for embedded use cases?
5. Does this compromise reliability?

## Future Work (Roadmap)

**Phase 3** ✅ **COMPLETE** - Datalog Core
- ✅ EAV data model with Facts
- ✅ Datalog parser (EDN syntax)
- ✅ Pattern matching and unification
- ✅ Recursive rules (semi-naive evaluation)
- ✅ Transitive closure and cycle handling
- ✅ Updated REPL with multi-line and comments
- ✅ 123 comprehensive tests

**Phase 4** ✅ **COMPLETE** - Bi-temporal Support
- ✅ Transaction time (tx_id, tx_count)
- ✅ Valid time (valid_from, valid_to)
- ✅ Time travel queries (:as-of, :valid-at)
- ✅ File format v2 with migration
- ✅ 172 comprehensive tests

**Phase 5** (2-3 months): ACID + WAL - **NEXT**
- Write-ahead logging (embedded in .graph file)
- Transaction API (BEGIN, COMMIT, ROLLBACK)
- Crash recovery
- ACID compliance

**Phase 6** (2-3 months): Performance
- Indexes (EAVT, AEVT, AVET, VAET)
- Query optimization
- Benchmarking
- **Evaluate rkyv**: Consider switching from postcard to rkyv for zero-copy
  deserialization. rkyv offers 2x better performance for reads/writes but adds
  complexity. Worth evaluating when implementing memory-mapped database access
  or when WAL performance becomes critical.

**Phase 7** (3-4 months): Cross-platform
- WASM (IndexedDB backend)
- Mobile bindings (iOS, Android)
- Language bindings (Python, JavaScript)

**v1.0.0** (12-15 months): Production Ready
- Stable API
- Stable file format
- Comprehensive docs
- Backwards compatibility promise

## Comparison to Similar Projects

**XTDB** (formerly Crux):
- ✅ Bi-temporal Datalog database (inspiration)
- ✅ Production-ready
- ❌ Clojure, multi-file storage (directories)
- Minigraf: Single file, Rust, simpler scope

**Cozo**:
- ✅ Embedded Datalog, Rust
- ✅ Graph algorithms, vector search
- ❌ Multi-file storage (RocksDB/Sled)
- Minigraf: Single file, bi-temporal first-class

**Datomic**:
- ✅ Temporal Datalog database (major inspiration)
- ✅ Production-proven since 2012
- ❌ Client-server, Clojure, proprietary
- Minigraf: Embedded, single file, open source

**GraphLite**:
- ✅ Full GQL spec compliance
- ✅ Embedded, ACID, mature
- ❌ Multi-file storage (Sled), no bi-temporal
- Minigraf: Datalog (not GQL), single file, bi-temporal

**Positioning**: Minigraf = SQLite + Datomic + single file

## Contributing Guidelines

This is a hobby project with a decades-long vision. When contributing:

1. **Read PHILOSOPHY.md first** - Understand the core principles
2. **Check ROADMAP.md** - See where we are and where we're going
3. **Align with philosophy** - Simplicity, reliability, embedded-first
4. **Write tests** - All features must be tested
5. **Keep it simple** - Prefer boring, proven solutions
6. **Think long-term** - We're building for decades, not months

**Say NO to**:
- Features that break single-file philosophy
- Client-server architecture
- Complex configuration
- Features only for distributed systems
- Breaking changes without overwhelming justification

**Say YES to**:
- Crash safety and data integrity
- Query performance improvements
- Better error messages
- Documentation improvements
- Cross-platform support

## Key Files to Understand

**For Phase 5 work (ACID + WAL)**:
1. `PHILOSOPHY.md` - Why single-file, reliability-first
2. `ROADMAP.md` - Detailed Phase 5 plan
3. `src/storage/mod.rs` - StorageBackend trait (stable foundation)
4. `src/storage/backend/file.rs` - File format implementation (extend for WAL)
5. `src/storage/persistent_facts.rs` - Persistence layer (postcard serialization)
6. `src/minigraf.rs` - Public API

**For understanding the Datalog engine (Phase 3-4, stable)**:
1. `src/graph/types.rs` - EAV model: `Fact`, `Value`, bi-temporal fields
2. `src/graph/storage.rs` - `FactStorage` with temporal query methods
3. `src/query/datalog/parser.rs` - EDN/Datalog parser
4. `src/query/datalog/executor.rs` - Query executor with temporal filtering
5. `src/query/datalog/evaluator.rs` - Semi-naive recursive rule evaluation
6. `src/temporal.rs` - UTC timestamp parsing

## Important Reminders

1. **We pivoted to Datalog** - Don't implement GQL features
2. **Bi-temporal is first-class** - Not an afterthought
3. **Single file is sacred** - Never break this
4. **Simplicity over features** - Do less, do it perfectly
5. **Test everything** - No untested code
6. **Think SQLite** - Would SQLite do this?
7. **Long-term vision** - Building for decades

---

*When in doubt, refer to PHILOSOPHY.md and ROADMAP.md. The goal is not to be the most feature-complete graph database. The goal is to be the one that's always there when you need it, works reliably, and never gets in your way.*

*Be boring. Be reliable. Be Minigraf.*
