# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable **bi-temporal graph database with Datalog queries** written in Rust. It's designed to be the "SQLite of graph databases" - embedded, single-file, reliable, with time travel capabilities.

**Current Status: Phase 3 COMPLETE ✅ → Phase 4 Starting** - Datalog with Recursive Rules:
- ✅ Phase 1: Property graph PoC (in-memory)
- ✅ Phase 2: Persistent storage (`.graph` file format, embedded API)
- ✅ **Phase 3: Datalog core (EAV model, recursive rules) - COMPLETE!**
- 🎯 Phase 4: Bi-temporal support (transaction time + valid time) - **NEXT**
- 🎯 Phase 5: ACID + WAL (crash safety, transactions)
- 🎯 Phase 6: Performance (indexes, query optimization)
- 🎯 v1.0.0: 12-15 months

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

# Run the REPL (currently GQL, will become Datalog in Phase 3)
cargo run

# Run tests
cargo test

# Run specific test suite
cargo test --test integration_test -- --nocapture
cargo test --test concurrency_test
cargo test --test edge_cases_test

# Run examples
cargo run --example embedded
cargo run --example file_storage
```

## Architecture

### Module Structure

The codebase is organized into the following modules:

1. **Graph Module (`src/graph/`)** - Phase 1-2 (will evolve in Phase 3):
   - `types.rs`: Core graph types (will become EAV model)
     - Current: `Node`, `Edge`, `PropertyValue` (property graph)
     - Future: `Fact`, `Entity`, `Attribute`, `Value` (EAV model)
   - `storage.rs`: Thread-safe in-memory storage (Phase 1)
     - Uses `Arc<RwLock<HashMap>>` for nodes/edges
     - Will migrate to fact-based storage in Phase 3

2. **Storage Module (`src/storage/`)** - Phase 2 (stable foundation) ✅:
   - `mod.rs`: StorageBackend trait and file format
     - `StorageBackend` trait: Platform-agnostic storage interface
     - `FileHeader`: Metadata for `.graph` files
     - Page size: 4KB, Magic number: "MGRF"
   - `backend/file.rs`: File-based backend (single `.graph` file)
     - Page-based storage, cross-platform format
     - Supports Linux, macOS, Windows, iOS, Android
   - `backend/memory.rs`: In-memory backend for testing
   - `backend/indexeddb.rs`: Future WASM browser backend (Phase 7)
   - `persistent.rs`: Persistent graph storage layer
     - Serialization/deserialization of property graph
     - Will evolve to support EAV facts in Phase 3

3. **Query Module (`src/query/`)** - Phase 1-2 (will be rewritten in Phase 3):
   - `parser.rs`: Query parser
     - Current: GQL-inspired syntax (Phase 1-2, archived)
     - Future: Datalog EDN syntax (Phase 3)
   - `executor.rs`: Query execution engine
     - Current: Property graph executor
     - Future: Datalog pattern matcher with recursive rules

4. **REPL Module (`src/repl.rs`)** - Phase 1-2 (will be updated in Phase 3):
   - Interactive console
   - Prompt-based interface (`minigraf>`)
   - Handles EOF gracefully (src/repl.rs:27-30)
   - Will support Datalog syntax in Phase 3

5. **Minigraf Module (`src/minigraf.rs`)** - Phase 2 (stable) ✅:
   - Public embedded database API
   - `Minigraf::open()` - Opens or creates database
   - `Minigraf::execute()` - Executes queries
   - `Minigraf::save()` - Explicit save
   - Auto-save on drop

6. **Library (`src/lib.rs`)**: Public API
   - Exports core types and functions
   - Stable foundation for Phase 3 evolution

7. **Binary (`src/main.rs`)**: Standalone executable
   - Launches interactive REPL
   - Uses in-memory storage currently

### Current Data Model (Phase 1-2)

**Property Graph** (will be replaced with EAV in Phase 3):
```rust
struct Node {
    id: Uuid,
    labels: Vec<String>,
    properties: HashMap<String, PropertyValue>,
}

struct Edge {
    id: Uuid,
    source: Uuid,
    target: Uuid,
    label: String,
    properties: HashMap<String, PropertyValue>,
}
```

### Future Data Model (Phase 3+)

**Entity-Attribute-Value (Datalog Triple Store)**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,  // e.g., ":person/name", ":friend"
    value: Value,
    tx_id: TxId,        // Transaction that asserted this
    asserted: bool,     // true = assert, false = retract
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
```

**Bi-temporal Extension (Phase 4)**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,
    value: Value,
    valid_from: DateTime,   // When fact became valid in real world
    valid_to: DateTime,     // When fact stopped being valid
    tx_id: TxId,            // Transaction ID
    tx_time: SystemTime,    // When fact was recorded
    asserted: bool,
}
```

### Storage Implementation

**Layered Architecture**:

**High-level** (Phase 1-2, will evolve):
- `GraphStorage`: In-memory property graph operations
- `PersistentGraphStorage`: Serialization layer
- Will become fact-based storage in Phase 3

**Low-level** (Phase 2, stable foundation) ✅:
- `StorageBackend` trait: Platform-agnostic interface
- `FileBackend`: Single `.graph` file (4KB pages)
- `MemoryBackend`: In-memory for testing
- Future: `IndexedDbBackend` for WASM

**File Format**:
```
Page 0: Header
  - Magic: "MGRF"
  - Version: u32
  - Page count: u64
  - Node count: u64 (Phase 1-2)
  - Edge count: u64 (Phase 1-2)
  - Fact count: u64 (Phase 3+)
  - Tx counter: u64 (Phase 4+)

Page 1+: Data
  - Current: Serialized nodes/edges
  - Future: EAV facts with temporal dimensions
```

**Serialization Format** (Phase 3+):
- Using **postcard** (v1.0+) for fact serialization
- Replaced bincode (unmaintained as of 2024/2025)
- postcard: Lightweight, embedded-focused, better size than bincode
- Future consideration: Evaluate **rkyv** in Phase 5/6 for zero-copy
  deserialization when implementing WAL or memory-mapped access

### Query Language

**Current (Phase 1-2, GQL-inspired)** - Archived:
```gql
CREATE NODE (:Person) {name: "Alice", age: 30}
CREATE EDGE (id1)-[KNOWS]->(id2) {since: 2020}
MATCH (:Person) WHERE name = "Alice"
SHOW NODES
```

**Future (Phase 3+, Datalog)** - Target syntax:
```datalog
;; Transact facts
(transact [[:alice :person/name "Alice"]
           [:alice :person/age 30]
           [:alice :friend :bob]])

;; Simple query
(query [:find ?name
        :where [?e :person/name ?name]])

;; Recursive rule
(rule [(reachable ?from ?to)
       [?from :connected ?to]])

(rule [(reachable ?from ?to)
       [?from :connected ?intermediate]
       (reachable ?intermediate ?to)])

;; Bi-temporal query (Phase 4)
(query [:find ?status
        :valid-at "2023-06-01"
        :as-of tx-100
        :where [:alice :employment/status ?status]])
```

### Error Handling

- Parse errors: Descriptive messages, REPL continues
- Execution errors: Validated before execution
- EOF handling: REPL exits gracefully (for piped input)
- Storage errors: Result<T, Error> with context

## Test Coverage

**Current Tests (Phase 3)**: 123 tests passing ✅
- **Unit tests** (94 tests):
  - `src/graph/types.rs`: Fact types, Value types, EAV model
  - `src/graph/storage.rs`: FactStorage, CRUD, history tracking
  - `src/query/datalog/parser.rs`: EDN/Datalog syntax parsing, rules
  - `src/query/datalog/types.rs`: Pattern, WhereClause, DatalogQuery
  - `src/query/datalog/matcher.rs`: Pattern matching, variable unification
  - `src/query/datalog/executor.rs`: Query execution, rule registration
  - `src/query/datalog/rules.rs`: RuleRegistry, rule management
  - `src/query/datalog/evaluator.rs`: Semi-naive evaluation, transitive closure
  - `src/storage/`: Backend operations, persistence (postcard)

- **Integration tests** (26 tests):
  - `tests/complex_queries_test.rs` (10 tests): Multi-pattern joins, self-joins, edge cases
  - `tests/recursive_rules_test.rs` (9 tests): Transitive closure, cycles, long chains, family trees
  - `tests/concurrency_test.rs` (7 tests): Thread safety, concurrent rule registration/queries

- **Doc tests** (3 tests): Inline documentation examples

**Comprehensive Coverage**:
- ✅ Datalog parser (EDN syntax) - 15 tests
- ✅ Pattern matching and unification - 16 tests
- ✅ **Recursive rule evaluation** - 15 tests (NEW!)
- ✅ **Transitive closure** - 9 tests (NEW!)
- ✅ **Concurrency** - 7 tests (NEW!)
- ✅ Complex queries (3+ patterns, self-joins) - 10 tests (NEW!)

**Demo Scripts**:
- `demo_recursive.txt`: Comprehensive recursive rules examples (transitive closure, cycles, family trees)

Run tests with: `cargo test`
See `TEST_COVERAGE.md` for detailed coverage report.

**Future Tests (Phase 4+)**:
- Bi-temporal queries (:as-of, :valid-at) - Phase 4
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

### Phase 4 (Next) - Bi-temporal Support 🎯

**Planned Features**:
- Transaction time queries (`:as-of tx-id`)
- Valid time dimensions (`valid_from`, `valid_to`)
- Time travel queries (`:valid-at timestamp`)
- History tracking and audit trails
- Bi-temporal joins

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

**Phase 4** (3-4 months): Bi-temporal Support - **NEXT**
- Transaction time (tx_id, tx_time)
- Valid time (valid_from, valid_to)
- Time travel queries (:as-of, :valid-at)
- History queries

**Phase 5** (2-3 months): ACID + WAL
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

**For Phase 3 work (Datalog implementation)**:
1. `PHILOSOPHY.md` - Why Datalog, why bi-temporal
2. `ROADMAP.md` - Detailed Phase 3 plan
3. `src/storage/mod.rs` - Storage abstraction (stable foundation)
4. `src/graph/types.rs` - Current types (will evolve to EAV)
5. `src/query/parser.rs` - Current parser (will be rewritten)
6. `src/query/executor.rs` - Current executor (will be rewritten)

**For understanding storage (stable)**:
1. `src/storage/backend/file.rs` - File format implementation
2. `src/storage/persistent.rs` - Persistence layer
3. `src/minigraf.rs` - Public API

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
