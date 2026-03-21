# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-03-21

### Added
- Write-ahead log (WAL): fact-level sidecar `<db>.wal` with CRC32-protected binary entries
- `WriteTransaction` API: `begin_write()` / `commit()` / `rollback()` for explicit ACID transactions
- Crash recovery: WAL entries replayed on open; corrupt/partial entries discarded at first bad CRC32
- Checkpoint: `checkpoint()` flushes WAL facts to `.graph` and deletes the WAL; auto-checkpoint on configurable threshold
- `FileHeader` v3: `last_checkpointed_tx_count` field (repurposes unused `edge_count` slot)
- `FactStorage` helpers: `get_all_facts()`, `restore_tx_counter()`, `allocate_tx_count()`
- `OpenOptions` builder: `OpenOptions::new().path("db.graph").open()` or `Minigraf::in_memory()`
- `--file <path>` CLI flag for the REPL binary
- 41 new tests covering WAL, crash recovery, transactions, and checkpoint

### Changed
- `src/minigraf.rs` replaced by `src/db.rs` — `Minigraf`, `OpenOptions`, `WriteTransaction` public API
- File format version bumped 2→3; automatic v1/v2→v3 migration on first checkpoint
- REPL version string now tracks `CARGO_PKG_VERSION` automatically

### Fixed
- WAL-before-apply ordering: facts are now applied to in-memory state only after the WAL entry is fsynced, ensuring crash safety for both implicit (`execute()`) and explicit (`WriteTransaction`) write paths

## [0.4.0] - 2026-03-21

### Added
- Bi-temporal support: every fact now carries transaction time (`tx_id`, `tx_count`)
  and valid time (`valid_from`, `valid_to`)
- `:as-of N` query modifier for transaction time travel (counter or ISO 8601 timestamp)
- `:valid-at "date"` query modifier for valid time point-in-time queries
- `:valid-at :any-valid-time` to disable valid time filtering
- `(transact {:valid-from ... :valid-to ...} [...])` syntax for specifying valid time
- Per-fact valid time override in transact (4-element fact vectors with metadata map)
- File format version 2 with automatic migration from version 1

### Changed
- **Breaking behaviour**: queries without `:valid-at` now return only currently valid
  facts (`valid_from <= now < valid_to`). Existing Phase 3 databases are unaffected
  because all migrated facts have `valid_to = MAX`.
- `FactStorage::transact()` now accepts an optional `TransactOptions` parameter

### Fixed
- `PersistentFactStorage::load()` previously discarded original `tx_id` when loading
  facts from disk, making time-travel queries on persisted databases incorrect

## [0.3.0] - 2026-03-10

### Added
- Datalog core implementation with recursive rules
- Entity-Attribute-Value (EAV) data model
- Pattern matching with variable unification
- Semi-naive evaluation for recursive rules
- Transitive closure support with cycle handling
- Rule registry for rule management
- Persistent storage with postcard serialization
- REPL with multi-line command support and comments
- 123 comprehensive tests (94 unit + 26 integration + 3 doc)

### Changed
- Replaced GQL-inspired syntax with Datalog EDN syntax
- Data model changed from property graph to EAV triples
- Query executor rewritten for Datalog pattern matching

## [0.2.0] - 2026-02-01

### Added
- Persistent storage backend with `.graph` file format
- StorageBackend trait for platform abstraction
- FileBackend implementation (4KB pages, cross-platform)
- MemoryBackend for testing
- PersistentGraphStorage layer for serialization
- Embedded API (`Minigraf::open()`, `Minigraf::execute()`)
- Auto-save on drop

### Changed
- Graph storage now supports persistence

## [0.1.0] - 2026-01-15

### Added
- Initial release
- In-memory property graph implementation
- Basic graph operations (nodes, edges, properties)
- Interactive REPL
- Thread-safe storage with `Arc<RwLock<>>`
