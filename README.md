# Minigraf

A tiny, portable GQL (Graph Query Language) engine written in Rust. **W.I.P.**

GQL stands for Graph Query Language, which has been standardized in [ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html).

## Purpose

This project was started to (in order):
- Learn Rust,
- Learn how to write a parser,
- Learn GQL,
- Possibly create a borderline useful tool in the process.

## Current Status - Phase 2

Minigraf is now a **fully embeddable graph database** with persistent storage:

- ✅ **Property graph model** - Nodes and edges with typed properties
- ✅ **Persistent storage** - Single `.graph` file format (Phase 2 NEW!)
- ✅ **Embedded database API** - Use like SQLite (Phase 2 NEW!)
- ✅ **Cross-platform** - Works on Linux, macOS, Windows, iOS, Android
- ✅ **Auto-persistence** - Changes auto-save when database is dropped
- ✅ **Interactive REPL** - Query console for exploration
- ✅ **Query language** - GQL-inspired syntax
- ✅ **Test coverage** - 35+ tests, all passing

## Quick Start

### As an Embedded Database

```rust
use minigraf::Minigraf;

// Open or create a database
let mut db = Minigraf::open("myapp.graph")?;

// Execute queries
db.execute("CREATE NODE (:Person) {name: \"Alice\", age: 30}")?;
db.execute("MATCH (:Person)")?;

// Auto-saves on drop
drop(db);

// Later - data is still there!
let db = Minigraf::open("myapp.graph")?;
```

### As an Interactive Console

```bash
# Build and run the REPL
cargo run

# Or build release version
cargo build --release
./target/release/minigraf

# Run tests
cargo test

# Run examples
cargo run --example embedded
```

## Query Language

The PoC implements a simple GQL-like query language:

### Create a Node

```gql
CREATE NODE (:Person) {name: "Alice", age: 30}
CREATE NODE (:Person:Employee) {name: "Bob"}
CREATE NODE (:Company)
```

### Create an Edge

```gql
CREATE EDGE (node-id-1)-[KNOWS]->(node-id-2) {since: 2020}
CREATE EDGE (alice-id)-[WORKS_AT]->(company-id)
```

### Match/Query Nodes

```gql
MATCH (:Person)
MATCH (:Person) WHERE name = "Alice"
MATCH (:Employee)
```

### Match/Query Edges

```gql
MATCH -[:KNOWS]->
MATCH -[:WORKS_AT]->
```

### Show All Data

```gql
SHOW NODES
SHOW EDGES
```

### Help and Exit

```gql
HELP    # Show available commands
EXIT    # Exit the console (or use QUIT)
```

## Example Session

```
$ cargo run
Minigraf v0.1.0 - Graph Query Language Engine
Using in-memory storage

Minigraf v0.1.0 - Interactive Graph Query Console
Type HELP for available commands, EXIT to quit.

minigraf> CREATE NODE (:Person) {name: "Alice", age: 30}
Node created: <uuid> (labels: Person, properties: {age: 30, name: "Alice"})

minigraf> CREATE NODE (:Person) {name: "Bob", age: 25}
Node created: <uuid> (labels: Person, properties: {age: 25, name: "Bob"})

minigraf> CREATE EDGE (<alice-uuid>)-[KNOWS]->(<bob-uuid>) {since: 2020}
Edge created: <edge-uuid> (<alice-uuid> -[KNOWS]-> <bob-uuid>, properties: {since: 2020})

minigraf> MATCH (:Person)
Found 2 node(s):
  - <alice-uuid> (labels: Person, properties: {age: 30, name: "Alice"})
  - <bob-uuid> (labels: Person, properties: {age: 25, name: "Bob"})

minigraf> MATCH (:Person) WHERE name = "Alice"
Found 1 node(s):
  - <alice-uuid> (labels: Person, properties: {age: 30, name: "Alice"})

minigraf> SHOW EDGES
Found 1 edge(s):
  - <edge-uuid> (<alice-uuid> -[KNOWS]-> <bob-uuid>, properties: {since: 2020})

minigraf> EXIT
Goodbye!
```

## Architecture

### Module Structure

- **`src/graph/types.rs`**: Core property graph data structures (Node, Edge, Property, PropertyValue)
- **`src/graph/storage.rs`**: In-memory storage layer with thread-safe operations
- **`src/storage/`**: Storage backend abstraction (NEW in Phase 2)
  - **`mod.rs`**: StorageBackend trait and file format definitions
  - **`backend/file.rs`**: File-based backend for native platforms (.graph files)
  - **`backend/memory.rs`**: In-memory backend for testing/embedded
  - **`backend/indexeddb.rs`**: Browser backend (future, WASM only)
- **`src/query/parser.rs`**: Query language parser
- **`src/query/executor.rs`**: Query execution engine
- **`src/repl.rs`**: Interactive REPL console
- **`src/lib.rs`**: Library exports
- **`src/main.rs`**: Binary entry point

### Property Graph Model

**Nodes**:
- Unique UUID identifier
- Multiple labels (e.g., `Person`, `Employee`)
- Properties as key-value pairs

**Edges**:
- Unique UUID identifier
- Source and target node IDs
- Single label/type (e.g., `KNOWS`, `WORKS_AT`)
- Properties as key-value pairs

**Property Values**:
- String
- Integer (i64)
- Float (f64)
- Boolean
- Null

## Scope

Minigraf will be designed to run in multiple environments, including:
- As a standalone binary ✅ (PoC done)
- As a library ✅ (PoC done)
- As a WebAssembly module (for browsers) ⏳ (future)

## Unscope

Minigraf will **NOT** be designed to be (for now):
- Distributed,
- Fault-tolerant,
- ACID-compliant.

## Storage Backends

Minigraf uses a **platform-abstraction layer** inspired by SQLite's VFS architecture. Different storage backends enable cross-platform support:

### Available Backends

- **In-memory** ✅ - Fast, non-persistent storage (Phase 1 PoC)
  - Perfect for testing and embedded systems
  - Currently used by the REPL

- **File-based (`.graph` files)** ✅ - Single-file persistent storage (Phase 2)
  - Page-based storage with 4KB pages
  - Cross-platform file format (endian-safe)
  - Supports native platforms: Linux, macOS, Windows, iOS, Android
  - See `examples/file_storage.rs` for usage

### Future Backends

- **IndexedDB** ⏳ - Browser storage for WASM (Phase 4)
  - Same API, different implementation
  - Enables Minigraf in web applications

- **Optional: Custom implementations** ⏳
  - Users can implement `StorageBackend` trait for custom needs
  - Example: SQLite, RocksDB (via feature flags)

### `.graph` File Format

The file-based backend uses a simple, stable page-based format:

```
+----------------+
| Page 0: Header | <- Magic "MGRF", version, page count, node/edge counts
+----------------+
| Page 1: Data   | <- Future: Nodes, edges, indexes
+----------------+
| Page 2: Data   |
+----------------+
| ...            |
+----------------+
```

- **Page size**: 4KB (like SQLite)
- **Endian-safe**: Works across all platforms
- **Single file**: Easy to backup, share, version control
- **Stable format**: Once v1.0 ships, backwards compatible forever

## GQL Spec Compliance

**Current Status: ~2-5% of ISO/IEC 39075:2024**

This is a learning project implementing a GQL-inspired query language. It is **not fully compliant** with the GQL standard and does not aim for complete compliance in the near term.

### ✅ Implemented (PoC Level)

**Basic Graph Model:**
- ✅ Nodes with multiple labels
- ✅ Directed edges with single label
- ✅ Properties on nodes and edges
- ✅ Property types: String, Integer, Float, Boolean, Null

**Basic Query Operations:**
- ✅ CREATE NODE with labels and properties
- ✅ CREATE EDGE between existing nodes
- ✅ Simple MATCH by label: `MATCH (:Label)`
- ✅ Single property equality filter: `WHERE prop = value`
- ✅ SHOW NODES / SHOW EDGES (non-standard convenience commands)

### ❌ Not Yet Implemented (Majority of GQL Spec)

**Graph Pattern Matching:**
- ❌ Complex path patterns: `(a)-[:REL]->(b)-[:REL2]->(c)`
- ❌ Variable-length paths: `(a)-[:REL*1..5]->(b)`
- ❌ Shortest path queries
- ❌ Optional patterns (OPTIONAL MATCH)
- ❌ Pattern alternatives/disjunction
- ❌ Quantified path patterns

**Query Clauses:**
- ❌ RETURN clause (projections, expressions)
- ❌ WITH clause (intermediate results)
- ❌ ORDER BY, LIMIT, SKIP
- ❌ DISTINCT
- ❌ OPTIONAL MATCH
- ❌ UNION, INTERSECT, EXCEPT

**Data Manipulation:**
- ❌ INSERT (vs. CREATE)
- ❌ SET (update properties/labels)
- ❌ REMOVE (remove properties/labels)
- ❌ DELETE (delete nodes/edges)
- ❌ MERGE (upsert semantics)

**Expressions & Operators:**
- ❌ Arithmetic operations (+, -, *, /, %)
- ❌ Comparison operators (<, >, <=, >=, <>)
- ❌ Logical operators (AND, OR, NOT) in WHERE
- ❌ String operations (CONTAINS, STARTS WITH, ENDS WITH)
- ❌ List operations
- ❌ Map/record operations
- ❌ CASE expressions
- ❌ NULL handling (IS NULL, COALESCE)

**Aggregations & Grouping:**
- ❌ Aggregation functions (COUNT, SUM, AVG, MIN, MAX)
- ❌ GROUP BY
- ❌ HAVING

**Advanced Data Types:**
- ❌ Lists/Arrays
- ❌ Maps/Records
- ❌ Path type
- ❌ Temporal types (Date, Time, DateTime, Duration)
- ❌ Spatial types (Point, Geography)

**Advanced Features:**
- ❌ Multiple named graphs
- ❌ Graph constructors
- ❌ Schema definitions and validation
- ❌ Constraints and indexes
- ❌ Functions (string, math, temporal, etc.)
- ❌ Subqueries
- ❌ Transactions

**Conformance:**
- ❌ No formal conformance testing
- ❌ Not validated against official GQL test suite
- ❌ Syntax may differ from official spec

### Roadmap to Compliance

This project prioritizes learning over spec compliance. Future milestones:

1. **Phase 1 (Current)**: Basic PoC - simple CRUD and queries ✅
2. **Phase 2**: Complex patterns - multi-hop paths, variable-length
3. **Phase 3**: RETURN clause, projections, ORDER BY/LIMIT
4. **Phase 4**: UPDATE/DELETE operations
5. **Phase 5**: Aggregations and GROUP BY
6. **Phase 6**: Advanced expressions and operators
7. **Phase 7**: Schema, constraints, indexes
8. **Phase 8+**: Advanced types, multiple graphs, full spec compliance

For official GQL resources, see: [ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html)

## Testing

The project includes comprehensive test coverage:

```bash
cargo test
```

Tests cover:
- Property graph data structures
- Storage operations (CRUD for nodes and edges)
- Query parser
- Query executor
- Edge traversal operations

## License

This project is open source and available under the MIT License.
