# Phase 5: ACID + WAL Design Spec

**Date**: 2026-03-21
**Status**: Approved
**Phase**: 5 of Minigraf roadmap (follows Phase 4: Bi-temporal Support)

---

## Goal

Add crash safety and explicit transaction support to Minigraf. Every `transact` call must be atomic and durable. Users may also group multiple `transact` calls into a single explicit transaction with commit/rollback semantics.

---

## Constraints

- Single-file philosophy must be preserved for the database itself.
- No changes to `StorageBackend` trait — WASM backend (Phase 7) is unaffected.
- No changes to `FactStorage` or the Datalog engine.
- Minimal new dependencies.
- Existing `execute()` API continues to work unchanged.

---

## Approach: Fact-level Sidecar WAL

A sidecar WAL file (`<db>.wal`, e.g. `mydb.graph.wal`) stores committed transaction entries as sequences of serialized facts. The main `.graph` file is only rewritten during checkpointing. On open, any sidecar WAL is replayed on top of the main file to recover committed-but-not-checkpointed transactions.

This approach is chosen over page-level WAL because:
- Minigraf's save path is already a full rewrite of all facts (not page-diffs).
- Facts are the natural unit of truth in the EAV model.
- Crash recovery is trivially correct: replay = retransact.
- The `StorageBackend` trait remains unchanged, keeping WASM clean.

The sidecar file is deleted after a successful checkpoint. During normal operation, only one `.wal` file exists alongside the `.graph` file.

---

## WAL File Format

**File**: `<db_path>.wal`

```
WAL header (32 bytes):
  magic:    [u8; 4]  = b"MWAL"
  version:  u32      = 1
  reserved: [u8; 24]

WAL entries (variable length, appended sequentially):
  checksum:  u32        CRC32 of the remainder of this entry
  tx_count:  u64        monotonic counter value assigned to this transaction
  num_facts: u64        number of facts in this entry
  facts:     [Fact]     postcard-serialized, length-prefixed per fact
```

Each entry is independently verifiable. On recovery, entries are replayed in order; the first entry with an invalid checksum terminates replay (partial write discarded). No earlier entries are affected.

---

## Write Path

### Implicit transaction (existing `execute()` / `transact()`)

1. Acquire write lock.
2. Apply facts to in-memory `FactStorage` (existing path).
3. Serialize facts into a WAL entry; append to sidecar; `fsync`.
4. Mark dirty; release write lock.
5. If WAL entry count ≥ checkpoint threshold → trigger checkpoint.

### Explicit transaction

```rust
let mut tx = db.begin_write()?;    // acquires write lock
tx.execute("(transact [...])")?;   // buffers in-memory, not yet in WAL
tx.execute("(transact [...])")?;   // more buffering
tx.commit()?;                      // single WAL entry for entire batch, fsync
// Drop without commit = implicit rollback; buffer discarded, no WAL write
```

The write lock is held for the lifetime of `WriteTransaction`. This enforces serializable isolation — one writer at a time; readers always see committed in-memory state.

---

## Crash Recovery

On `Minigraf::open()`:

1. Load main `.graph` file into `FactStorage` (existing path).
2. Check for `<db>.wal` sidecar.
3. If found: read and validate WAL header; replay entries sequentially via `load_fact()`.
4. Stop replay at first entry with an invalid checksum (partial write).
5. Restore `tx_counter` from the maximum `tx_count` seen across all loaded facts.
6. Leave WAL open for future writes.

The main file is never partially updated during normal operation — all writes go to the WAL first. The main file is only written during checkpointing, which is atomic at the OS level (write + fsync + delete WAL).

---

## Checkpointing

### Automatic

Triggered after every commit when WAL entry count ≥ configurable threshold (default: 1000).

### Manual

```rust
db.checkpoint()?;
```

### Procedure

1. Acquire write lock.
2. Call existing `save()` — rewrites main `.graph` file from in-memory `FactStorage`.
3. `fsync` the main file.
4. Delete the `.wal` sidecar.
5. Release write lock.

---

## Public API

New file: `src/db.rs`

```rust
pub struct Minigraf { ... }
pub struct WriteTransaction<'a> { ... }

pub struct OpenOptions {
    /// Number of WAL entries before automatic checkpoint. Default: 1000.
    pub wal_checkpoint_threshold: usize,
}

impl Default for OpenOptions {
    fn default() -> Self { OpenOptions { wal_checkpoint_threshold: 1000 } }
}

impl Minigraf {
    /// Open or create a file-backed database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;

    /// Open with custom options.
    pub fn open_with_options(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self>;

    /// Create an in-memory database (for tests and REPL).
    pub fn in_memory() -> Result<Self>;

    /// Execute a Datalog command. Writes are WAL-durable.
    pub fn execute(&self, input: &str) -> Result<QueryResult>;

    /// Begin an explicit write transaction.
    pub fn begin_write(&self) -> Result<WriteTransaction<'_>>;

    /// Manually trigger a checkpoint.
    pub fn checkpoint(&self) -> Result<()>;
}

impl WriteTransaction<'_> {
    /// Execute a Datalog command within this transaction.
    /// Writes are buffered; not durable until commit().
    pub fn execute(&mut self, input: &str) -> Result<QueryResult>;

    /// Commit the transaction. Writes a single WAL entry and fsyncs.
    pub fn commit(self) -> Result<()>;

    /// Explicitly roll back. Also happens on drop.
    pub fn rollback(self);
}
```

`Minigraf::open()` is a one-liner equivalent to `open_with_options(path, OpenOptions::default())`. Existing REPL and test code that calls `execute()` requires no changes.

---

## Module Structure

```
src/
├── db.rs                     NEW  Minigraf facade + WriteTransaction
├── wal.rs                    NEW  WalWriter, WalReader, WalEntry, WAL header
├── storage/
│   └── persistent_facts.rs   MODIFIED  open() replays WAL; commits write WAL entry;
│                                        auto-checkpoint after threshold
└── lib.rs                    MODIFIED  re-export Minigraf, WriteTransaction, OpenOptions
```

No other files change. `StorageBackend`, `FactStorage`, `FileHeader`, and the Datalog engine are untouched.

---

## Dependencies

- **`crc32fast`** (new): CRC32 checksums for WAL entries. Tiny (~1KB compiled), no_std-compatible (Phase 7 friendly).
- No other new dependencies.

---

## File Format Version

`FORMAT_VERSION` bumped 2 → 3.

Migration: v2 files open fine — no WAL sidecar means clean state, nothing to replay. The existing `migrate_v1_to_v2()` path is unaffected.

---

## Testing Plan

### Unit tests (`src/wal.rs`)
- Write a WAL entry, read it back: checksum matches, facts match.
- Truncated entry (partial write): read stops before corrupted entry, earlier entries intact.
- Empty WAL: read returns zero entries without error.
- WAL header validation: wrong magic rejected.

### Unit tests (`src/db.rs`)
- `in_memory()` database: implicit transact works, no WAL file created.
- `begin_write()` / `commit()`: committed facts visible after commit.
- `begin_write()` / `rollback()`: no facts visible after rollback.
- Drop without commit: equivalent to rollback.
- `checkpoint()`: WAL sidecar deleted, main file updated.
- `open_with_options()`: custom threshold respected.

### Integration tests (`tests/wal_test.rs`) — new file
- **WAL recovery**: write facts, skip checkpoint, reopen → facts present.
- **Partial write recovery**: truncate WAL mid-entry, reopen → earlier entries recovered, partial entry discarded.
- **Checkpoint correctness**: after checkpoint, WAL deleted; reopen loads from main file.
- **Auto-checkpoint**: write entries up to threshold, verify checkpoint triggered.
- **Explicit tx rollback durability**: rolled-back facts absent after reopen.
- **Concurrent reads**: readers see committed state while writer holds lock.
- **Explicit tx across multiple transacts**: all-or-nothing on commit and rollback.

---

## WASM Compatibility (Phase 7)

The WAL is entirely managed within `PersistentFactStorage` when backed by `FileBackend`. The `IndexedDbBackend` for WASM will implement its own transaction semantics using IndexedDB's native transaction API. `StorageBackend` requires no WAL-related methods. Phase 7 is unaffected.

---

## Out of Scope for Phase 5

- Indexes (Phase 6)
- Multi-reader/writer isolation beyond serializable single-writer (not needed for embedded use)
- WAL compaction beyond simple checkpoint (not needed at target scale)
- Distributed transactions (explicitly out of scope forever)
