# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable GQL (Graph Query Language) engine written in Rust. It's a work-in-progress learning project designed to run as a standalone binary, library, or WebAssembly module. The project currently implements a working PoC with:
- Property graph data model (nodes, edges, properties)
- In-memory storage engine
- Custom GQL query parser
- Query executor
- Interactive REPL console

## Build and Run Commands

```bash
# Build the project
cargo build

# Build release version (with panic=abort optimization)
cargo build --release

# Run the REPL
cargo run

# Run tests
cargo test

# Run integration tests with output
cargo test --test integration_test -- --nocapture
```

## Architecture

### Module Structure

The codebase is organized into the following modules:

1. **Graph Module (`src/graph/`)**:
   - `types.rs`: Core property graph types
     - `Node`: Vertices with UUID IDs, multiple labels, and typed properties
     - `Edge`: Directed edges with UUID IDs, source/target nodes, label, and properties
     - `PropertyValue`: Enum supporting String, Integer, Float, Boolean, and Null types
   - `storage.rs`: Thread-safe in-memory storage using `Arc<RwLock<HashMap>>`
     - CRUD operations for nodes and edges
     - Query helpers: filter by labels, properties, get edges from/to nodes

2. **Query Module (`src/query/`)**:
   - `parser.rs`: Text-based query parser supporting:
     - `CREATE NODE (:Label1:Label2) {prop: value}` - Create nodes
     - `CREATE EDGE (id1)-[LABEL]->(id2) {prop: value}` - Create edges
     - `MATCH (:Label) [WHERE prop = value]` - Find nodes
     - `MATCH -[:LABEL]->` - Find edges
     - `SHOW NODES` / `SHOW EDGES` - List all entities
     - `HELP` / `EXIT` - Console commands
   - `executor.rs`: Query execution engine
     - Executes parsed queries against storage
     - Validates edge creation (source/target existence)
     - Returns formatted results

3. **REPL Module (`src/repl.rs`)**: Interactive console
   - Prompt-based interface (`minigraf>`)
   - Reads user input, parses queries, executes, and displays results
   - Handles EOF gracefully (src/repl.rs:27-30) for piped input
   - Error handling for parse/execution errors

4. **Library (`src/lib.rs`)**: Public API
   - Exports all core types: `Node`, `Edge`, `PropertyValue`, `GraphStorage`
   - Exports query functions: `parse_query`, `QueryExecutor`, `QueryResult`
   - Exports REPL: `Repl`

5. **Binary (`src/main.rs`)**: Standalone executable
   - Creates in-memory storage
   - Launches interactive REPL

### Storage Implementation

The current implementation uses **in-memory storage** with:
- `Arc<RwLock<HashMap<NodeId, Node>>>` for nodes
- `Arc<RwLock<HashMap<EdgeId, Edge>>>` for edges
- Thread-safe concurrent access via read/write locks
- UUIDs for automatic ID generation

Note: RocksDB was initially considered but switched to in-memory storage for the PoC due to compilation issues.

### Query Language Syntax

The query language is inspired by GQL (ISO/IEC 39075:2024) but simplified for the PoC:

```
# Create nodes with labels and properties
CREATE NODE (:Person) {name: "Alice", age: 30}
CREATE NODE (:Person:Employee) {name: "Bob", age: 25}

# Create edges between existing nodes (use node IDs)
CREATE EDGE (node-id-1)-[KNOWS]->(node-id-2) {since: 2020}

# Match nodes by label
MATCH (:Person)

# Match nodes with property filter
MATCH (:Person) WHERE name = "Alice"

# Match edges by label
MATCH -[:KNOWS]->

# Show all nodes or edges
SHOW NODES
SHOW EDGES
```

### Error Handling

- Parse errors: Reported with descriptive messages, REPL continues
- Execution errors: Validated before execution (e.g., edge endpoints must exist)
- EOF handling: REPL exits gracefully when stdin closes (for piped input/scripts)

## Test Coverage

The project has comprehensive test coverage:

- **Unit tests** (19 tests):
  - `src/graph/types.rs`: Node/Edge creation, PropertyValue accessors
  - `src/graph/storage.rs`: CRUD operations, queries, edge relationships
  - `src/query/parser.rs`: All query syntax variations
  - `src/query/executor.rs`: Query execution, validation

- **Integration tests** (`tests/integration_test.rs`):
  - Complete workflow: create nodes, create edges, query, filter

Run tests with: `cargo test`

## Development Notes

- **No schema file required**: The engine uses in-memory storage and doesn't require a schema on startup
- **UUID-based IDs**: All nodes and edges get automatic UUID identifiers
- **Thread-safe**: Storage can be safely shared across threads
- **Property types**: Supports String, Integer, Float, Boolean, and Null values
- **EOF handling**: REPL properly exits when stdin closes (fixed in src/repl.rs:27-30)

## Future Work

- Persistent storage (RocksDB integration)
- Indexes for fast property lookups
- Complex graph traversals (multi-hop paths)
- Aggregations and analytics
- Library embedding support
- WebAssembly compilation
- Transaction support
- Schema validation
