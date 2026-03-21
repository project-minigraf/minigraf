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

**WAL is file-backend-only.** `WalWriter` is instantiated only when the database is backed by `FileBackend`. `in_memory()` databases write directly to `FactStorage` with no WAL. Future `IndexedDbBackend` (Phase 7) also has no WAL — it uses IndexedDB's native transaction API instead. This is an explicit invariant, not an implementation detail.

---

## WAL File Format

**File**: `<db_path>.wal`

```
WAL header (32 bytes):
  magic:    [u8; 4]  = b"MWAL"
  version:  u32      = 1
  reserved: [u8; 24]

WAL entries (variable length, appended sequentially):
  checksum:  u32     CRC32 of everything after this field in this entry
                     (covers tx_count + num_facts + all length-prefixed fact bytes)
  tx_count:  u64     monotonic counter value assigned to this transaction
  num_facts: u64     number of facts in this entry
  facts:     sequence of num_facts length-prefixed postcard-serialized facts,
             each encoded as: fact_len: u32 (LE) | fact_bytes: [u8; fact_len]
```

Each entry is independently verifiable. On recovery, entries are replayed in order; the first entry with an invalid checksum terminates replay (partial write discarded cleanly). No earlier entries are affected. A partial write always produces a bad checksum because the checksum covers the complete entry payload.

---

## Main File Header (v3)

`FORMAT_VERSION` is bumped from 2 → 3. The `FileHeader` struct's previously-unused `edge_count` field is repurposed as `last_checkpointed_tx_count`:

```
Page 0 header (64 bytes):
  magic:                      [u8; 4]  = b"MGRF"
  version:                    u32      = 3
  page_count:                 u64
  fact_count:                 u64      (was node_count — repurposed in v2, formalized in v3)
  last_checkpointed_tx_count: u64      (was edge_count — repurposed in v3; 0 = never checkpointed)
  reserved:                   [u8; 32]
```

**v2 → v3 upgrade**: A v2 file opened by Phase 5 code is read normally (`last_checkpointed_tx_count` is read as 0 from the `edge_count` field, which was always written as 0). The file is upgraded to v3 on the **first checkpoint** (`save()` rewrites the header with `version = 3` and the current `last_checkpointed_tx_count`). Opening and querying a v2 file without writing does not change its version.

**Backwards-incompatibility**: A v3 file opened by Phase 4 (or earlier) binaries will be rejected with "Unsupported format version: 3". This is intentional. v3 files are not readable by older binaries.

---

## Write Path

### Implicit transaction (existing `execute()` / `transact()`)

1. Acquire write lock.
2. Apply facts to in-memory `FactStorage` (existing path).
3. Serialize facts into a WAL entry (checksum + tx_count + length-prefixed facts); append to sidecar; `fsync`.
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

**Queries inside `WriteTransaction::execute()`** see committed state plus all facts buffered in the current transaction (read-your-own-writes). The in-transaction `FactStorage` snapshot is extended with the buffered facts for query purposes only; buffered facts are not committed to the main `FactStorage` until `commit()`.

**Failed `commit()`**: If writing or fsyncing the WAL entry fails, all buffered facts are rolled back from `FactStorage` (as if `rollback()` had been called), the write lock is released, and the error is returned. After a failed `commit()`, the database is in the same state as before `begin_write()` was called.

---

## Crash Recovery

On `Minigraf::open()`:

1. Load main `.graph` file into `FactStorage` (existing path). Note `last_checkpointed_tx_count` from the header.
2. Check for `<db>.wal` sidecar.
3. If found: read and validate WAL header.
4. Replay WAL entries sequentially. **Skip any entry whose `tx_count` ≤ `last_checkpointed_tx_count`** — these facts are already present in the main file. This prevents duplicate facts when a crash occurs after `save()` but before the WAL is deleted.
5. Stop replay at the first entry with an invalid checksum (partial write discarded).
6. After WAL replay completes, restore `tx_counter` from the maximum `tx_count` seen across all loaded facts (main file + replayed WAL entries combined). WAL entries always have higher `tx_count` values than the checkpointed main file, so a single call to `restore_tx_counter()` after replay is sufficient.
7. Leave WAL open for future writes.

**Invariant**: `last_checkpointed_tx_count` in the main file is always the `tx_count` of the last transaction included in that file. WAL replay unconditionally skips entries at or below this value, making replay idempotent with respect to checkpoint races.

---

## Checkpointing

### Automatic

Triggered after every commit when the in-memory WAL entry counter ≥ configurable threshold (default: 1000). The counter is initialized to the number of entries replayed during `open()`, so WAL files that survive a crash and are partially replayed immediately contribute toward the next checkpoint threshold.

### Manual

```rust
db.checkpoint()?;
```

### Procedure

1. Acquire write lock.
2. Force `dirty = true` on `PersistentFactStorage` (bypasses the `if !self.dirty { return Ok(()) }` early-return guard in `save()`), then call `save()` — rewrites main `.graph` file from in-memory `FactStorage`, writing header with `version = 3`, current `page_count`, `fact_count`, and `last_checkpointed_tx_count` = current `tx_counter` value.
3. `fsync` the main file.
4. Delete the `.wal` sidecar.
5. Reset the in-memory WAL entry counter to 0.
6. Release write lock.

The `dirty` force-set in step 2 ensures that `db.checkpoint()` always writes the main file and deletes the WAL, even if no writes have occurred since the last save. This makes manual checkpoint always meaningful.

A crash between steps 3 and 4 leaves both a fully-updated main file and an intact WAL sidecar. On next open, WAL replay will skip all entries (their `tx_count` ≤ `last_checkpointed_tx_count`), then the empty WAL is effectively ignored. This is safe and correct.

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
    /// Open or create a file-backed database with WAL enabled.
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;

    /// Open with custom options.
    pub fn open_with_options(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self>;

    /// Create an in-memory database (no WAL). For tests and REPL.
    pub fn in_memory() -> Result<Self>;

    /// Execute a Datalog command. Writes are WAL-durable for file-backed databases.
    pub fn execute(&self, input: &str) -> Result<QueryResult>;

    /// Begin an explicit write transaction (file-backed databases only).
    /// Acquires the write lock; held until commit() or rollback()/drop.
    pub fn begin_write(&self) -> Result<WriteTransaction<'_>>;

    /// Manually trigger a checkpoint (file-backed databases only; no-op for in_memory).
    pub fn checkpoint(&self) -> Result<()>;
}

impl WriteTransaction<'_> {
    /// Execute a Datalog command within this transaction.
    /// Reads see committed state + buffered in-transaction facts (read-your-own-writes).
    /// Writes are buffered; not durable until commit().
    pub fn execute(&mut self, input: &str) -> Result<QueryResult>;

    /// Commit the transaction atomically.
    /// On failure: buffered facts are rolled back, write lock released, error returned.
    pub fn commit(self) -> Result<()>;

    /// Explicitly roll back. Also happens implicitly on drop.
    pub fn rollback(self);
}
```

`Minigraf::open()` is equivalent to `open_with_options(path, OpenOptions::default())`. Existing REPL and test code using `execute()` requires no changes.

---

## Module Structure

```
src/
├── db.rs                     NEW  Minigraf facade + WriteTransaction + OpenOptions
├── wal.rs                    NEW  WalWriter, WalReader, WalEntry, WAL header
│                                  Instantiated only for file-backed databases
├── storage/
│   └── persistent_facts.rs   MODIFIED  open() replays WAL (with tx_count dedup);
│                                        commits write WAL entry; auto-checkpoint;
│                                        save() writes v3 header with last_checkpointed_tx_count
└── lib.rs                    MODIFIED  re-export Minigraf, WriteTransaction, OpenOptions
```

**Changes to `src/storage/mod.rs`**:
- `FORMAT_VERSION` constant updated from `2` → `3`.
- `FileHeader::validate()` updated to accept versions 1–3 (currently accepts 1–2).
- Existing unit test `test_validate_rejects_version_0_and_3` updated to assert version 4 is rejected (version 3 is now valid).
- `FileHeader` struct's `edge_count` field renamed to `last_checkpointed_tx_count` (semantic rename only; wire layout unchanged).

All other files are untouched: `StorageBackend`, `FactStorage`, and the Datalog engine require no changes.

---

## Dependencies

- **`crc32fast`** (new): CRC32 checksums for WAL entries. Tiny (~1KB compiled), no_std-compatible (Phase 7 friendly).
- No other new dependencies.

---

## Testing Plan

### Unit tests (`src/wal.rs`)
- Write a WAL entry, read it back: checksum matches, facts match.
- Truncated entry (partial write): read stops before corrupted entry; earlier entries intact.
- Empty WAL: read returns zero entries without error.
- WAL header validation: wrong magic rejected.
- Multi-fact entry: all facts round-trip correctly with length-prefix framing.

### Unit tests (`src/db.rs`)
- `in_memory()` database: implicit transact works, no WAL file created.
- `begin_write()` / `commit()`: committed facts visible after commit.
- `begin_write()` / `rollback()`: no facts visible after rollback.
- Drop without commit: equivalent to rollback.
- `checkpoint()`: WAL sidecar deleted, main file updated with `last_checkpointed_tx_count`.
- `open_with_options()`: custom threshold respected.
- `WriteTransaction::execute()` with query: sees committed + buffered facts.
- Failed `commit()`: in-memory state unchanged after failure.

### Integration tests (`tests/wal_test.rs`) — new file
- **WAL recovery**: write facts, skip checkpoint, reopen → facts present.
- **Post-checkpoint crash simulation**: save main file + set `last_checkpointed_tx_count`, keep WAL, reopen → no duplicate facts.
- **Partial write recovery**: truncate WAL mid-entry, reopen → earlier entries recovered, partial entry discarded.
- **Checkpoint correctness**: after checkpoint, WAL deleted; main file `last_checkpointed_tx_count` matches final `tx_count`; reopen loads from main file.
- **Auto-checkpoint**: write entries up to threshold; verify checkpoint triggered and WAL reset.
- **Explicit tx rollback durability**: rolled-back facts absent after reopen.
- **Concurrent reads**: readers see committed state while writer holds lock.
- **Explicit tx across multiple transacts**: all-or-nothing on commit and rollback.
- **v2 → v3 upgrade**: open a v2 file, write facts, checkpoint → file is now v3 with correct `last_checkpointed_tx_count`.

---

## WASM Compatibility (Phase 7)

`WalWriter` is only instantiated for file-backed databases. `in_memory()` and the future `IndexedDbBackend` never create a WAL file. `IndexedDbBackend` implements ACID using IndexedDB's native transaction API. `StorageBackend` has no WAL-related methods. Phase 7 is unaffected.

---

## Out of Scope for Phase 5

- Indexes (Phase 6)
- Multi-reader/writer isolation beyond serializable single-writer
- WAL compaction beyond simple checkpoint
- Distributed transactions (explicitly out of scope forever)
- Concurrent open of the same `.graph` file by multiple processes (not a goal for embedded-first)
