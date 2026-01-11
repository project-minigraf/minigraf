# Minigraf

A tiny, portable GQL (Graph Query Language) engine written in Rust. **W.I.P.**

GQL stands for Graph Query Language, which has been standardized in [ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html).

## Purpose

This project was started to (in order):
- Learn Rust,
- Learn how to write a parser,
- Learn GQL,
- Possibly create a borderline useful tool in the process.

## Current Status - PoC

This is a proof-of-concept implementation featuring:
- ✅ Basic property graph model (nodes, edges with properties)
- ✅ In-memory storage
- ✅ Interactive REPL console
- ✅ Simple GQL-like query language
- ✅ Test coverage

## Build and Run

```bash
# Build the project
cargo build

# Build release version
cargo build --release

# Run the interactive console
cargo run

# Run tests
cargo test
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

## Future Features

Minigraf will support multiple backends to store its data, including:
- In-memory ✅ (PoC done)
- IndexedDB (browser only) ⏳
- SQLite ⏳
- One or more embedded KV stores (such as LevelDB or RocksDB) ⏳

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
