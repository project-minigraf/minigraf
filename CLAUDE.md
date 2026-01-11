# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable GQL engine written in Rust. GQL stands for Graph Query Language (ISO/IEC 39075:2024). It's a work-in-progress learning project designed to run as a standalone binary, library, or WebAssembly module. The project currently implements a REPL-based GQL query parser.

## Build and Run Commands

```bash
# Build the project
cargo build

# Build release version (with panic=abort optimization)
cargo build --release

# Run the REPL (uses schema.gql by default)
cargo run

# Run with custom schema file
cargo run -- path/to/schema.gql

# Run tests
cargo test

# Run with debug logging enabled
LOG_LEVEL=Debug cargo run
```

## Architecture

### Module Structure

The codebase is organized into two main components:

1. **Library (`src/lib.rs`)**: Re-exports GQL parser functions (`parse_query`, `parse_schema`) from the `graphql-parser` crate (used as a temporary placeholder), making them available to both the binary and external consumers.

2. **Binary (`src/main.rs`)**: Implements a REPL that:
   - Loads a GQL schema file on startup (default: `schema.gql`)
   - Validates the schema using `parse_schema`
   - Enters a loop accepting user queries
   - Parses each query using `parse_query` and provides debug output

3. **Server Module (`src/server/`)**: Contains internal utilities:
   - `logger.rs`: Implements logging with configurable log levels (Error, Warn, Info, Debug, Trace) controlled by the `LOG_LEVEL` environment variable
   - `error_codes.rs`: Defines typed error codes (InvalidSchemaFile=1, InvalidSchema=2, BadQuery=3) used for process exit codes and error reporting

### Error Handling Pattern

The project uses a consistent error handling pattern:
- Errors are classified with typed `ErrorCode` enums (src/server/error_codes.rs:3-7)
- The `logger::error_log` function formats errors with code, message, and underlying error details
- Fatal errors (schema loading/parsing) exit with the error code as the process exit code
- Non-fatal errors (bad queries in REPL) are logged but execution continues

### Logging System

The logger uses `lazy_static` to parse the `LOG_LEVEL` environment variable once at startup (src/server/logger.rs:32-37). Debug logging is conditional and only prints when `LOG_LEVEL` is Debug or higher.

## Development Notes

- The project currently depends on `graphql-parser` for parsing functionality, though this will likely be replaced as GQL is not GraphQL
- Create a `schema.gql` file (see `schema.gql.example` for reference format) before running
- The REPL currently only parses and validates queries; query execution is not yet implemented
- Error codes are defined as enums with explicit integer values for stable process exit codes
