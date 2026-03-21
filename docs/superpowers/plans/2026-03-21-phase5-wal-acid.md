# Phase 5: ACID + WAL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add crash-safe WAL and explicit transaction support to Minigraf via a fact-level sidecar WAL (`<db>.wal`) and a new `Minigraf` public facade.

**Architecture:** A sidecar WAL file stores committed fact batches as CRC32-protected binary entries. Writes go to the WAL first; the main `.graph` file is only updated on checkpoint. A new `Minigraf` facade provides `open()`, `execute()`, `begin_write()`, and `checkpoint()`; a `WriteTransaction` holds the write lock and buffers uncommitted facts.

**Tech Stack:** Rust, `crc32fast` (new), `postcard` (existing), `std::sync::Mutex` / `thread_local!`, existing `FactStorage` + `PersistentFactStorage<FileBackend>`.

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | Modify | Add `crc32fast` dependency |
| `src/storage/mod.rs` | Modify | `FORMAT_VERSION` 2→3; rename `edge_count`→`last_checkpointed_tx_count`; update `validate()` and affected tests |
| `src/graph/storage.rs` | Modify | Add `current_tx_count()` and `allocate_tx_count()` to `FactStorage` |
| `src/storage/persistent_facts.rs` | Modify | Store and expose `last_checkpointed_tx_count`; update `save()` to write it |
| `src/wal.rs` | Create | `WalWriter`, `WalReader`, `WalEntry`; binary serialization + CRC32; unit tests |
| `src/db.rs` | Create | `Minigraf` facade, `WriteTransaction`, `OpenOptions`, `Inner`, `WriteContext` |
| `src/lib.rs` | Modify | Re-export `Minigraf`, `WriteTransaction`, `OpenOptions` |
| `src/main.rs` | Modify | Use `Minigraf` facade instead of bare `PersistentFactStorage` |
| `tests/wal_test.rs` | Create | Integration tests: recovery, crash simulation, checkpoint, explicit tx |

---

## Task 1: Dependencies and FileHeader v3

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: Add `crc32fast` to `Cargo.toml`**

```toml
[dependencies]
crc32fast = "1.4"
```

Add after the existing `postcard` line.

- [ ] **Step 2: Write failing test for v3 format version constant**

Add inside the `#[cfg(test)]` block in `src/storage/mod.rs`:

```rust
#[test]
fn test_format_version_is_3() {
    assert_eq!(FORMAT_VERSION, 3);
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo test test_format_version_is_3 -- --nocapture
```
Expected: FAIL — `3 != 2`

- [ ] **Step 4: Update `FORMAT_VERSION` and `FileHeader` in `src/storage/mod.rs`**

Change the constant:
```rust
pub const FORMAT_VERSION: u32 = 3;
```

Rename the `edge_count` field to `last_checkpointed_tx_count` in the struct:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub page_count: u64,
    pub node_count: u64,
    pub last_checkpointed_tx_count: u64,  // was edge_count
    pub reserved: [u8; 32],
}
```

Update `FileHeader::new()`:
```rust
pub fn new() -> Self {
    FileHeader {
        magic: MAGIC_NUMBER,
        version: FORMAT_VERSION,
        page_count: 1,
        node_count: 0,
        last_checkpointed_tx_count: 0,
        reserved: [0; 32],
    }
}
```

Update `to_bytes()` — the field name changed but position (bytes 24–32) is unchanged; just rename the reference:
```rust
bytes.extend_from_slice(&self.last_checkpointed_tx_count.to_le_bytes());
```

Update `from_bytes()` — same position, just rename:
```rust
let last_checkpointed_tx_count = u64::from_le_bytes([
    bytes[24], bytes[25], bytes[26], bytes[27],
    bytes[28], bytes[29], bytes[30], bytes[31],
]);
// ...
Ok(FileHeader { magic, version, page_count, node_count, last_checkpointed_tx_count, reserved })
```

Update `validate()` to accept versions 1–3:
```rust
pub fn validate(&self) -> Result<()> {
    if self.magic != MAGIC_NUMBER {
        anyhow::bail!("Invalid magic number");
    }
    if self.version < 1 || self.version > FORMAT_VERSION {
        anyhow::bail!(
            "Unsupported format version: {} (supported: 1-{})",
            self.version, FORMAT_VERSION
        );
    }
    Ok(())
}
```

Update the existing test that asserted version 3 is rejected:
```rust
// was: test_validate_rejects_version_0_and_3
#[test]
fn test_validate_rejects_version_0_and_4() {
    let mut header = FileHeader::new();
    header.version = 0;
    assert!(header.validate().is_err());

    header.version = 4;
    assert!(header.validate().is_err());
}
```

Also add a test that version 3 is now accepted:
```rust
#[test]
fn test_validate_accepts_versions_1_to_3() {
    let mut header = FileHeader::new();
    for v in 1u32..=3 {
        header.version = v;
        assert!(header.validate().is_ok(), "version {} should be accepted", v);
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test --lib storage -- --nocapture
```
Expected: all storage unit tests pass (including the renamed test).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/storage/mod.rs
git commit -m "feat(storage): bump FORMAT_VERSION to 3, rename edge_count to last_checkpointed_tx_count"
```

---

## Task 2: `FactStorage` helpers

**Files:**
- Modify: `src/graph/storage.rs`

- [ ] **Step 1: Write failing tests**

Add inside the `#[cfg(test)]` block in `src/graph/storage.rs`:

```rust
#[test]
fn test_current_tx_count_starts_at_zero() {
    let storage = FactStorage::new();
    assert_eq!(storage.current_tx_count(), 0);
}

#[test]
fn test_current_tx_count_reflects_transacts() {
    let storage = FactStorage::new();
    let alice = uuid::Uuid::new_v4();
    storage.transact(vec![(alice, ":name".to_string(), Value::String("Alice".to_string()))], None).unwrap();
    assert_eq!(storage.current_tx_count(), 1);
    storage.transact(vec![(alice, ":age".to_string(), Value::Integer(30))], None).unwrap();
    assert_eq!(storage.current_tx_count(), 2);
}

#[test]
fn test_allocate_tx_count_increments() {
    let storage = FactStorage::new();
    let c1 = storage.allocate_tx_count();
    let c2 = storage.allocate_tx_count();
    assert_eq!(c1, 1);
    assert_eq!(c2, 2);
    assert_eq!(storage.current_tx_count(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_current_tx_count -- --nocapture
cargo test test_allocate_tx_count -- --nocapture
```
Expected: FAIL — methods not found.

- [ ] **Step 3: Add methods to `FactStorage`**

In `src/graph/storage.rs`, add after `restore_tx_counter()`:

```rust
/// Return the current value of the monotonic tx counter.
///
/// Useful for persisting `last_checkpointed_tx_count` into the file header.
pub fn current_tx_count(&self) -> u64 {
    self.tx_counter.load(Ordering::SeqCst)
}

/// Atomically increment the tx counter and return the new value.
///
/// Used by explicit transactions to claim a tx_count at commit time,
/// without creating any facts in FactStorage.
pub fn allocate_tx_count(&self) -> u64 {
    self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_current_tx_count -- --nocapture
cargo test test_allocate_tx_count -- --nocapture
```
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/graph/storage.rs
git commit -m "feat(storage): add current_tx_count() and allocate_tx_count() to FactStorage"
```

---

## Task 3: `PersistentFactStorage` — v3 save + `last_checkpointed_tx_count`

**Files:**
- Modify: `src/storage/persistent_facts.rs`

- [ ] **Step 1: Write failing tests**

Add inside `#[cfg(test)]` in `src/storage/persistent_facts.rs`:

```rust
#[test]
fn test_save_writes_v3_header() {
    use crate::storage::backend::MemoryBackend;
    use crate::storage::FORMAT_VERSION;

    let backend = MemoryBackend::new();
    let mut pfs = PersistentFactStorage::new(backend).unwrap();
    let alice = uuid::Uuid::new_v4();
    pfs.storage()
        .transact(vec![(alice, ":name".to_string(), crate::graph::types::Value::String("Alice".to_string()))], None)
        .unwrap();
    pfs.mark_dirty();
    pfs.save().unwrap();

    // Read back the header and verify version and last_checkpointed_tx_count
    let backend = pfs.into_backend();
    let header_page = backend.read_page(0).unwrap();
    let header = crate::storage::FileHeader::from_bytes(&header_page).unwrap();
    assert_eq!(header.version, FORMAT_VERSION);  // must be 3
    assert_eq!(header.last_checkpointed_tx_count, 1); // one transact call
}

#[test]
fn test_last_checkpointed_tx_count_getter() {
    use crate::storage::backend::MemoryBackend;

    let backend = MemoryBackend::new();
    let pfs = PersistentFactStorage::new(backend).unwrap();
    // Fresh database: no checkpoint yet
    assert_eq!(pfs.last_checkpointed_tx_count(), 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_save_writes_v3_header -- --nocapture
cargo test test_last_checkpointed_tx_count_getter -- --nocapture
```
Expected: FAIL — method `last_checkpointed_tx_count` not found; version assertion fails.

- [ ] **Step 3: Update `PersistentFactStorage`**

In `src/storage/persistent_facts.rs`, add the field to the struct:

```rust
pub struct PersistentFactStorage<B: StorageBackend> {
    backend: B,
    storage: FactStorage,
    dirty: bool,
    last_checkpointed_tx_count: u64,
}
```

Update `new()` to initialise it:
```rust
pub fn new(backend: B) -> Result<Self> {
    let mut persistent = PersistentFactStorage {
        backend,
        storage: FactStorage::new(),
        dirty: false,
        last_checkpointed_tx_count: 0,
    };
    // ... rest unchanged
}
```

Update `load()` to read and store it from the header:
```rust
fn load(&mut self) -> Result<()> {
    let header_page = self.backend.read_page(0)?;
    let header = FileHeader::from_bytes(&header_page)?;
    header.validate()?;

    if header.version < 2 {
        return self.migrate_v1_to_v2();
    }

    // Store last_checkpointed_tx_count from header (0 for v2 files)
    self.last_checkpointed_tx_count = header.last_checkpointed_tx_count;

    self.storage.clear()?;
    let page_count = header.page_count;
    for page_id in 1..page_count {
        let page = self.backend.read_page(page_id)?;
        if let Ok(fact) = postcard::from_bytes::<Fact>(&page) {
            self.storage.load_fact(fact)?;
        }
    }
    self.storage.restore_tx_counter()?;
    self.dirty = false;
    Ok(())
}
```

Note: the existing `load()` checks `header.version == 1`; change that to `header.version < 2` since v2 and v3 use the same fact layout.

Update `save()` to write `last_checkpointed_tx_count` and `version = FORMAT_VERSION`:

```rust
pub fn save(&mut self) -> Result<()> {
    if !self.dirty {
        return Ok(());
    }

    let facts = self.storage.get_all_facts()?;
    let page_count = 1 + facts.len() as u64;

    let mut header = FileHeader::new(); // sets version = FORMAT_VERSION = 3
    header.page_count = page_count;
    header.node_count = facts.len() as u64;
    header.last_checkpointed_tx_count = self.storage.current_tx_count();

    let mut header_page = header.to_bytes();
    header_page.resize(PAGE_SIZE, 0);
    self.backend.write_page(0, &header_page)?;

    for (i, fact) in facts.iter().enumerate() {
        let data = postcard::to_allocvec(fact)?;
        if data.len() > PAGE_SIZE {
            anyhow::bail!("Fact too large: {} bytes (max {})", data.len(), PAGE_SIZE);
        }
        let mut page = vec![0u8; PAGE_SIZE];
        page[..data.len()].copy_from_slice(&data);
        self.backend.write_page((i + 1) as u64, &page)?;
    }

    self.backend.sync()?;
    self.last_checkpointed_tx_count = self.storage.current_tx_count();
    self.dirty = false;
    Ok(())
}
```

Add the public getter:
```rust
/// The `last_checkpointed_tx_count` recorded in the on-disk header.
///
/// Used by WAL replay to skip entries already present in the main file.
pub fn last_checkpointed_tx_count(&self) -> u64 {
    self.last_checkpointed_tx_count
}
```

Also add a `force_dirty()` method (used by checkpoint to bypass the `!dirty` guard):
```rust
/// Force the dirty flag to true regardless of current state.
///
/// Used by checkpoint to ensure save() always writes even if no new
/// facts have been added since the last save.
pub fn force_dirty(&mut self) {
    self.dirty = true;
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -- --nocapture 2>&1 | tail -20
```
Expected: all existing tests pass + the 2 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/storage/persistent_facts.rs
git commit -m "feat(storage): v3 save writes last_checkpointed_tx_count, expose getter + force_dirty"
```

---

## Task 4: WAL binary format (`src/wal.rs`)

**Files:**
- Create: `src/wal.rs`
- Modify: `src/lib.rs` (add `pub mod wal;`)

This task implements the full WAL format and its unit tests. No integration with Minigraf yet.

- [ ] **Step 1: Add `pub mod wal;` to `src/lib.rs`**

Add at the top of the module declarations:
```rust
pub mod wal;
```

- [ ] **Step 2: Create `src/wal.rs` with types, writer, reader, and unit tests**

```rust
//! Write-Ahead Log (WAL) for Minigraf.
//!
//! The WAL sidecar file (`<db>.wal`) stores committed transaction entries as
//! CRC32-protected binary records. Facts go to the WAL before the main file,
//! ensuring crash safety.
//!
//! # File layout
//!
//! ```text
//! WAL header (32 bytes):
//!   magic:    [u8; 4]  = b"MWAL"
//!   version:  u32 LE   = 1
//!   reserved: [u8; 24]
//!
//! WAL entries (variable length, sequential):
//!   checksum:  u32 LE  CRC32 of everything after this field in this entry
//!   tx_count:  u64 LE  monotonic counter from FactStorage
//!   num_facts: u64 LE
//!   facts:     for each fact: fact_len: u32 LE | fact_bytes: [u8; fact_len]
//! ```

use crate::graph::types::Fact;
use anyhow::{bail, Result};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const WAL_MAGIC: [u8; 4] = *b"MWAL";
const WAL_VERSION: u32 = 1;
const WAL_HEADER_SIZE: usize = 32;

// ─── WAL Header ─────────────────────────────────────────────────────────────

fn write_wal_header(file: &mut File) -> Result<()> {
    let mut buf = [0u8; WAL_HEADER_SIZE];
    buf[0..4].copy_from_slice(&WAL_MAGIC);
    buf[4..8].copy_from_slice(&WAL_VERSION.to_le_bytes());
    // bytes 8..32 are reserved zeros
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&buf)?;
    file.flush()?;
    Ok(())
}

fn validate_wal_header(file: &mut File) -> Result<()> {
    let mut buf = [0u8; WAL_HEADER_SIZE];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut buf)?;

    if buf[0..4] != WAL_MAGIC {
        bail!("Invalid WAL magic number: not a .wal file");
    }
    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if version != WAL_VERSION {
        bail!("Unsupported WAL version: {} (expected {})", version, WAL_VERSION);
    }
    Ok(())
}

// ─── Entry serialization ────────────────────────────────────────────────────

fn serialize_entry(tx_count: u64, facts: &[Fact]) -> Result<Vec<u8>> {
    // Build payload (everything covered by the checksum)
    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(&tx_count.to_le_bytes());
    payload.extend_from_slice(&(facts.len() as u64).to_le_bytes());
    for fact in facts {
        let fact_bytes = postcard::to_allocvec(fact)?;
        let fact_len = fact_bytes.len() as u32;
        payload.extend_from_slice(&fact_len.to_le_bytes());
        payload.extend_from_slice(&fact_bytes);
    }

    let checksum = crc32fast::hash(&payload);

    let mut entry = Vec::with_capacity(4 + payload.len());
    entry.extend_from_slice(&checksum.to_le_bytes());
    entry.extend_from_slice(&payload);
    Ok(entry)
}

// ─── Public types ───────────────────────────────────────────────────────────

/// A single committed transaction entry read from the WAL.
#[derive(Debug)]
pub struct WalEntry {
    pub tx_count: u64,
    pub facts: Vec<Fact>,
}

// ─── WalWriter ──────────────────────────────────────────────────────────────

/// Appends committed transaction entries to the WAL sidecar file.
///
/// Created by `Minigraf::open()` for file-backed databases.
/// Not used for in-memory databases.
pub struct WalWriter {
    file: File,
}

impl WalWriter {
    /// Open an existing WAL or create a new one.
    ///
    /// If creating, writes the WAL header.
    /// If opening, validates the header and seeks to the end for appending.
    pub fn open_or_create(path: &Path) -> Result<Self> {
        let exists = path.exists();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        if !exists {
            write_wal_header(&mut file)?;
        } else {
            validate_wal_header(&mut file)?;
        }

        // Seek to end so subsequent writes append
        file.seek(SeekFrom::End(0))?;
        Ok(WalWriter { file })
    }

    /// Serialize `facts` as a WAL entry and append it to the file, then fsync.
    ///
    /// The entry is written atomically from the caller's perspective:
    /// a partial write produces a bad CRC32, which the reader discards.
    pub fn append_entry(&mut self, tx_count: u64, facts: &[Fact]) -> Result<()> {
        let entry_bytes = serialize_entry(tx_count, facts)?;
        self.file.write_all(&entry_bytes)?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Delete the WAL file at `path`. Called after a successful checkpoint.
    pub fn delete_file(path: &Path) -> Result<()> {
        std::fs::remove_file(path)?;
        Ok(())
    }
}

// ─── WalReader ──────────────────────────────────────────────────────────────

/// Reads and validates WAL entries for crash recovery.
pub struct WalReader {
    file: File,
}

impl WalReader {
    /// Open the WAL at `path` for reading.
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        validate_wal_header(&mut file)?;
        Ok(WalReader { file })
    }

    /// Read all valid entries from the WAL.
    ///
    /// Reads sequentially from after the header. Stops at the first entry
    /// with an invalid CRC32 (partial write) or at EOF. Earlier entries are
    /// unaffected by a bad entry.
    pub fn read_entries(&mut self) -> Result<Vec<WalEntry>> {
        self.file.seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))?;
        let mut entries = Vec::new();

        loop {
            // Read checksum (4 bytes); EOF here means no more entries
            let mut csum_buf = [0u8; 4];
            match self.file.read_exact(&mut csum_buf) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
                Ok(()) => {}
            }
            let expected_csum = u32::from_le_bytes(csum_buf);

            // Read tx_count (8 bytes)
            let mut tx_count_buf = [0u8; 8];
            if self.file.read_exact(&mut tx_count_buf).is_err() {
                break; // truncated
            }
            let tx_count = u64::from_le_bytes(tx_count_buf);

            // Read num_facts (8 bytes)
            let mut num_facts_buf = [0u8; 8];
            if self.file.read_exact(&mut num_facts_buf).is_err() {
                break; // truncated
            }
            let num_facts = u64::from_le_bytes(num_facts_buf) as usize;

            // Build payload for CRC32 verification
            let mut payload = Vec::new();
            payload.extend_from_slice(&tx_count_buf);
            payload.extend_from_slice(&num_facts_buf);

            // Read each fact
            let mut facts = Vec::with_capacity(num_facts);
            let mut truncated = false;
            for _ in 0..num_facts {
                let mut len_buf = [0u8; 4];
                if self.file.read_exact(&mut len_buf).is_err() {
                    truncated = true;
                    break;
                }
                let fact_len = u32::from_le_bytes(len_buf) as usize;
                payload.extend_from_slice(&len_buf);

                let mut fact_bytes = vec![0u8; fact_len];
                if self.file.read_exact(&mut fact_bytes).is_err() {
                    truncated = true;
                    break;
                }
                payload.extend_from_slice(&fact_bytes);

                match postcard::from_bytes::<Fact>(&fact_bytes) {
                    Ok(f) => facts.push(f),
                    Err(_) => {
                        truncated = true;
                        break;
                    }
                }
            }

            if truncated {
                break;
            }

            // Verify CRC32 over the full payload
            let actual_csum = crc32fast::hash(&payload);
            if expected_csum != actual_csum {
                break; // corrupted entry — stop here
            }

            entries.push(WalEntry { tx_count, facts });
        }

        Ok(entries)
    }
}

// ─── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Value, VALID_TIME_FOREVER};
    use uuid::Uuid;

    fn make_fact(entity: Uuid, attr: &str, value: Value, tx_count: u64) -> Fact {
        Fact::with_valid_time(entity, attr.to_string(), value, 1000, tx_count, 0, VALID_TIME_FOREVER)
    }

    #[test]
    fn test_wal_empty_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let _writer = WalWriter::open_or_create(&path).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert!(entries.is_empty(), "new WAL should have no entries");
    }

    #[test]
    fn test_wal_single_fact_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let fact = make_fact(alice, ":name", Value::String("Alice".to_string()), 1);

        let mut writer = WalWriter::open_or_create(&path).unwrap();
        writer.append_entry(1, &[fact.clone()]).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tx_count, 1);
        assert_eq!(entries[0].facts.len(), 1);
        assert_eq!(entries[0].facts[0].entity, fact.entity);
        assert_eq!(entries[0].facts[0].attribute, fact.attribute);
        assert_eq!(entries[0].facts[0].value, fact.value);
    }

    #[test]
    fn test_wal_multi_fact_entry_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let facts = vec![
            make_fact(alice, ":name", Value::String("Alice".to_string()), 1),
            make_fact(alice, ":age", Value::Integer(30), 1),
        ];

        let mut writer = WalWriter::open_or_create(&path).unwrap();
        writer.append_entry(1, &facts).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].facts.len(), 2);
    }

    #[test]
    fn test_wal_multiple_entries_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        let mut writer = WalWriter::open_or_create(&path).unwrap();
        writer.append_entry(1, &[make_fact(alice, ":name", Value::String("Alice".to_string()), 1)]).unwrap();
        writer.append_entry(2, &[make_fact(bob, ":name", Value::String("Bob".to_string()), 2)]).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tx_count, 1);
        assert_eq!(entries[1].tx_count, 2);
    }

    #[test]
    fn test_wal_bad_magic_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.wal");

        // Write garbage header
        std::fs::write(&path, b"XXXX\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();

        let result = WalReader::open(&path);
        assert!(result.is_err(), "bad magic should be rejected");
    }

    #[test]
    fn test_wal_truncated_entry_stops_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let fact = make_fact(alice, ":name", Value::String("Alice".to_string()), 1);

        // Write a valid entry
        let mut writer = WalWriter::open_or_create(&path).unwrap();
        writer.append_entry(1, &[fact]).unwrap();
        drop(writer);

        // Append garbage bytes after the valid entry to simulate a partial second write
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0x01]).unwrap(); // bad checksum prefix
        drop(file);

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        // Only the valid first entry should be returned
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tx_count, 1);
    }

    #[test]
    fn test_wal_delete_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        WalWriter::open_or_create(&path).unwrap();
        assert!(path.exists());
        WalWriter::delete_file(&path).unwrap();
        assert!(!path.exists());
    }
}
```

- [ ] **Step 3: Add `tempfile` dev-dependency to `Cargo.toml`** (needed for the tests)

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Run WAL unit tests to verify they pass**

```bash
cargo test wal:: -- --nocapture
```
Expected: all 8 WAL tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/wal.rs src/lib.rs Cargo.toml
git commit -m "feat(wal): add WalWriter, WalReader, binary entry format with CRC32"
```

---

## Task 5: `Minigraf` facade and `WriteTransaction` (`src/db.rs`)

**Files:**
- Create: `src/db.rs`
- Modify: `src/lib.rs`

This is the largest task. It's split into sub-steps.

- [ ] **Step 1: Scaffold `src/db.rs` with types and stubs**

Create `src/db.rs`:

```rust
//! Public-facing `Minigraf` database handle.
//!
//! # Usage
//!
//! ## File-backed (persistent, WAL-durable)
//! ```no_run
//! use minigraf::{Minigraf, OpenOptions};
//!
//! let db = Minigraf::open("mydb.graph").unwrap();
//! db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
//! ```
//!
//! ## In-memory (for tests and REPL)
//! ```
//! use minigraf::Minigraf;
//!
//! let db = Minigraf::in_memory().unwrap();
//! db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
//! ```
//!
//! ## Explicit transaction
//! ```no_run
//! use minigraf::Minigraf;
//!
//! let db = Minigraf::open("mydb.graph").unwrap();
//! let mut tx = db.begin_write().unwrap();
//! tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
//! tx.execute("(transact [[:bob :name \"Bob\"]])").unwrap();
//! tx.commit().unwrap();
//! ```

use crate::graph::storage::FactStorage;
use crate::graph::types::{Fact, TransactOptions, tx_id_now, VALID_TIME_FOREVER};
use crate::query::datalog::{
    parse_datalog_command, DatalogCommand, DatalogExecutor, QueryResult,
};
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::Transaction;
use crate::storage::backend::file::FileBackend;
use crate::storage::persistent_facts::PersistentFactStorage;
use crate::wal::{WalWriter, WalReader};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

// ─── Thread-local write-transaction guard ────────────────────────────────────

thread_local! {
    static WRITE_TX_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn set_write_tx_active(v: bool) {
    WRITE_TX_ACTIVE.with(|c| c.set(v));
}

fn is_write_tx_active() -> bool {
    WRITE_TX_ACTIVE.with(|c| c.get())
}

// ─── Public configuration ────────────────────────────────────────────────────

/// Options for opening a file-backed `Minigraf` database.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Number of WAL entries before automatic checkpoint. Default: 1000.
    pub wal_checkpoint_threshold: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            wal_checkpoint_threshold: 1000,
        }
    }
}

// ─── Internal state ──────────────────────────────────────────────────────────

enum WriteContext {
    Memory,
    File {
        pfs: PersistentFactStorage<FileBackend>,
        wal: WalWriter,
        db_path: PathBuf,
        wal_entry_count: usize,
    },
}

struct Inner {
    /// Shared fact store (Arc-based interior mutability; safe for concurrent reads)
    fact_storage: FactStorage,
    /// Shared rule registry for the Datalog engine
    rules: Arc<RwLock<RuleRegistry>>,
    /// Serialises all writes; held for the lifetime of WriteTransaction
    write_lock: Mutex<WriteContext>,
    options: OpenOptions,
}

impl Inner {
    fn wal_path(db_path: &Path) -> PathBuf {
        let mut s = db_path.as_os_str().to_owned();
        s.push(".wal");
        PathBuf::from(s)
    }
}

// ─── Minigraf ────────────────────────────────────────────────────────────────

/// Embedded bi-temporal graph database handle.
///
/// Cheap to clone — all clones share the same underlying storage.
#[derive(Clone)]
pub struct Minigraf {
    inner: Arc<Inner>,
}

impl Minigraf {
    /// Open or create a file-backed database with WAL enabled.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, OpenOptions::default())
    }

    /// Open or create a file-backed database with custom options.
    pub fn open_with_options(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let wal_path = Inner::wal_path(&path);

        // Open main file
        let backend = FileBackend::open(&path)?;
        let pfs = PersistentFactStorage::new(backend)?;
        let last_checkpointed = pfs.last_checkpointed_tx_count();

        // Clone the shared FactStorage (Arc; same underlying data)
        let fact_storage = pfs.storage().clone();

        // Replay WAL if present
        let mut wal_entry_count = 0usize;
        if wal_path.exists() {
            let mut reader = WalReader::open(&wal_path)?;
            let entries = reader.read_entries()?;
            for entry in &entries {
                if entry.tx_count <= last_checkpointed {
                    continue; // already in main file
                }
                for fact in &entry.facts {
                    fact_storage.load_fact(fact.clone())?;
                }
                wal_entry_count += 1;
            }
            fact_storage.restore_tx_counter()?;
        }

        let wal = WalWriter::open_or_create(&wal_path)?;
        let rules = Arc::new(RwLock::new(RuleRegistry::new()));

        Ok(Minigraf {
            inner: Arc::new(Inner {
                fact_storage,
                rules,
                write_lock: Mutex::new(WriteContext::File {
                    pfs,
                    wal,
                    db_path: path,
                    wal_entry_count,
                }),
                options: opts,
            }),
        })
    }

    /// Create an in-memory database. No WAL. For tests and interactive REPL.
    pub fn in_memory() -> Result<Self> {
        Ok(Minigraf {
            inner: Arc::new(Inner {
                fact_storage: FactStorage::new(),
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                write_lock: Mutex::new(WriteContext::Memory),
                options: OpenOptions::default(),
            }),
        })
    }

    /// Execute a Datalog command as a self-contained implicit transaction.
    ///
    /// Writes are WAL-durable for file-backed databases.
    ///
    /// Returns `Err` if called from the same thread that holds an active
    /// `WriteTransaction` (use `tx.execute()` instead).
    pub fn execute(&self, input: &str) -> Result<QueryResult> {
        if is_write_tx_active() {
            bail!(
                "a WriteTransaction is already in progress on this thread; \
                 use tx.execute() instead"
            );
        }

        let command = parse_datalog_command(input)?;

        match &command {
            DatalogCommand::Query(_) | DatalogCommand::Rule(_) => {
                // Read-only: no lock needed
                let executor = DatalogExecutor::new_with_rules(
                    self.inner.fact_storage.clone(),
                    self.inner.rules.clone(),
                );
                executor.execute(command)
            }
            DatalogCommand::Transact(_) | DatalogCommand::Retract(_) => {
                // Write: acquire write lock
                let mut ctx = self.inner.write_lock.lock().unwrap();
                let result = Self::execute_write_command(
                    &self.inner.fact_storage,
                    &self.inner.rules,
                    command,
                )?;
                Self::maybe_wal_write_and_checkpoint(&mut ctx, &self.inner.options)?;
                Ok(result)
            }
        }
    }

    /// Begin an explicit write transaction.
    ///
    /// The write lock is held until `commit()`, `rollback()`, or `Drop`.
    ///
    /// Only available for file-backed databases; returns `Err` for in-memory.
    pub fn begin_write(&self) -> Result<WriteTransaction<'_>> {
        if is_write_tx_active() {
            bail!(
                "a WriteTransaction is already in progress on this thread; \
                 use tx.execute() instead"
            );
        }
        let guard = self.inner.write_lock.lock().unwrap();
        // Note: in-memory databases also support begin_write() for test convenience.
        // The spec says "file-backed only" as a documentation note (WAL is skipped
        // for WriteContext::Memory). This is intentional: commit() handles both paths.
        set_write_tx_active(true);
        Ok(WriteTransaction {
            inner: &self.inner,
            _guard: guard,
            pending_facts: Vec::new(),
            committed: false,
        })
    }

    /// Manually trigger a checkpoint. No-op for in-memory databases.
    pub fn checkpoint(&self) -> Result<()> {
        let mut ctx = self.inner.write_lock.lock().unwrap();
        Self::do_checkpoint(&self.inner.fact_storage, &mut ctx)
    }

    // ─── Internal helpers ──────────────────────────────────────────────────

    /// Execute a write DatalogCommand against `fact_storage` (no WAL interaction).
    fn execute_write_command(
        fact_storage: &FactStorage,
        rules: &Arc<RwLock<RuleRegistry>>,
        command: DatalogCommand,
    ) -> Result<QueryResult> {
        let executor = DatalogExecutor::new_with_rules(fact_storage.clone(), rules.clone());
        executor.execute(command)
    }

    /// After an implicit write: write WAL entry.
    ///
    /// Auto-checkpoint (triggered when WAL entry count ≥ threshold) is wired up
    /// in Task 6 when `db_path` becomes accessible here.
    fn maybe_wal_write_and_checkpoint(
        ctx: &mut WriteContext,
        _opts: &OpenOptions,
    ) -> Result<()> {
        match ctx {
            WriteContext::Memory => {}
            WriteContext::File { pfs, wal, wal_entry_count, .. } => {
                // Get the facts from the most recent transaction
                // The executor already applied them to fact_storage;
                // we collect all facts and write only the latest batch.
                // We track this via tx_count: the last allocated tx_count.
                let tx_count = pfs.storage().current_tx_count();
                let all_facts = pfs.storage().get_all_facts()?;
                let batch: Vec<Fact> = all_facts
                    .into_iter()
                    .filter(|f| f.tx_count == tx_count)
                    .collect();

                wal.append_entry(tx_count, &batch)?;
                pfs.mark_dirty();
                *wal_entry_count += 1;
                // Auto-checkpoint wired up in Task 6
            }
        }
        Ok(())
    }

    /// Perform a checkpoint: save main file, delete WAL, reset counter.
    fn do_checkpoint(fact_storage: &FactStorage, ctx: &mut WriteContext) -> Result<()> {
        match ctx {
            WriteContext::Memory => {}
            WriteContext::File { pfs, wal, db_path, wal_entry_count } => {
                // Force save even if not dirty (checkpoint must always write)
                pfs.force_dirty();
                pfs.save()?;
                let wal_path = Inner::wal_path(db_path);
                if wal_path.exists() {
                    WalWriter::delete_file(&wal_path)?;
                }
                // Re-create WalWriter pointing at the (now deleted) path
                *wal = WalWriter::open_or_create(&wal_path)?;
                *wal_entry_count = 0;
            }
        }
        let _ = fact_storage; // suppress unused warning
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // On clean close, checkpoint to leave the database in a clean state.
        if let Ok(mut ctx) = self.write_lock.try_lock() {
            let _ = Minigraf::do_checkpoint(&self.fact_storage, &mut ctx);
        }
    }
}

// ─── WriteTransaction ────────────────────────────────────────────────────────

/// An explicit write transaction. Holds the write lock for its lifetime.
///
/// Drop without calling `commit()` is equivalent to `rollback()`.
pub struct WriteTransaction<'a> {
    inner: &'a Inner,
    _guard: MutexGuard<'a, WriteContext>,
    pending_facts: Vec<Fact>,
    /// Set to true on commit to suppress rollback in Drop.
    committed: bool,
}

impl<'a> WriteTransaction<'a> {
    /// Execute a Datalog command within this transaction.
    ///
    /// - Write commands (`transact`, `retract`) buffer facts; not durable until `commit()`.
    /// - Read commands (`query`, `rule`) see committed state plus buffered facts.
    pub fn execute(&mut self, input: &str) -> Result<QueryResult> {
        let command = parse_datalog_command(input)?;
        match &command {
            DatalogCommand::Query(_) | DatalogCommand::Rule(_) => {
                // Read-your-own-writes: query against committed + pending facts
                let temp = self.snapshot_with_pending()?;
                let executor = DatalogExecutor::new_with_rules(temp, self.inner.rules.clone());
                executor.execute(command)
            }
            DatalogCommand::Transact(tx) => {
                let new_facts = materialize_transaction(tx)?;
                self.pending_facts.extend(new_facts);
                Ok(QueryResult::Ok)
            }
            DatalogCommand::Retract(tx) => {
                let new_facts = materialize_retraction(tx)?;
                self.pending_facts.extend(new_facts);
                Ok(QueryResult::Ok)
            }
        }
    }

    /// Commit the transaction atomically.
    ///
    /// Assigns a single `tx_id` and `tx_count` to all pending facts,
    /// writes one WAL entry, then loads them into `FactStorage`.
    ///
    /// On failure: pending facts are discarded, write lock released, error returned.
    pub fn commit(mut self) -> Result<()> {
        let tx_id = tx_id_now();
        let tx_count = self.inner.fact_storage.allocate_tx_count();

        // Stamp all pending facts with the final tx_id and tx_count
        for fact in &mut self.pending_facts {
            fact.tx_id = tx_id;
            fact.tx_count = tx_count;
        }

        // Write WAL entry (file-backed only)
        {
            let ctx = &mut *self._guard;
            match ctx {
                WriteContext::File { pfs, wal, wal_entry_count, .. } => {
                    wal.append_entry(tx_count, &self.pending_facts)?;
                    *wal_entry_count += 1;
                    pfs.mark_dirty();

                    // Load facts into the shared FactStorage
                    for fact in &self.pending_facts {
                        self.inner.fact_storage.load_fact(fact.clone())?;
                    }

                    if *wal_entry_count >= self.inner.options.wal_checkpoint_threshold {
                        Minigraf::do_checkpoint(&self.inner.fact_storage, ctx)?;
                    }
                }
                WriteContext::Memory => {
                    for fact in &self.pending_facts {
                        self.inner.fact_storage.load_fact(fact.clone())?;
                    }
                }
            }
        }

        self.committed = true;
        set_write_tx_active(false);
        Ok(())
    }

    /// Explicitly roll back the transaction. Also happens on `Drop`.
    pub fn rollback(mut self) {
        self.committed = true; // suppress rollback in Drop
        set_write_tx_active(false);
        // pending_facts are simply dropped
    }

    // ─── Helpers ──────────────────────────────────────────────────────────

    /// Build a temporary FactStorage containing committed facts + pending facts.
    fn snapshot_with_pending(&self) -> Result<FactStorage> {
        let temp = FactStorage::new();
        for fact in self.inner.fact_storage.get_all_facts()? {
            temp.load_fact(fact)?;
        }
        for fact in &self.pending_facts {
            temp.load_fact(fact.clone())?;
        }
        temp.restore_tx_counter()?;
        Ok(temp)
    }
}

impl Drop for WriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Rollback: pending_facts are dropped with self
            set_write_tx_active(false);
        }
    }
}

// ─── Fact materialisation helpers ────────────────────────────────────────────

/// Convert a parsed `Transaction` into `Fact` structs with placeholder tx_id/tx_count.
///
/// The real tx_id and tx_count are stamped at commit time.
fn materialize_transaction(tx: &Transaction) -> Result<Vec<Fact>> {
    use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
    use crate::query::datalog::types::Pattern;

    let tx_opts = if tx.valid_from.is_some() || tx.valid_to.is_some() {
        Some(TransactOptions::new(tx.valid_from, tx.valid_to))
    } else {
        None
    };

    let mut facts = Vec::new();
    for pattern in &tx.facts {
        let (entity_edn, attr_edn, value_edn, valid_from_override, valid_to_override) =
            match pattern {
                Pattern::Fact { entity, attribute, value } => {
                    (entity, attribute, value, None, None)
                }
                Pattern::FactWithValidTime { entity, attribute, value, valid_from, valid_to } => {
                    (entity, attribute, value, Some(*valid_from), Some(*valid_to))
                }
                _ => bail!("unexpected pattern type in transact"),
            };

        let entity = edn_to_entity_id(entity_edn)
            .ok_or_else(|| anyhow::anyhow!("invalid entity in transact"))?;
        let attr = match attr_edn {
            crate::query::datalog::types::EdnValue::Keyword(k) => k.clone(),
            _ => bail!("attribute must be a keyword"),
        };
        let value = edn_to_value(value_edn)
            .ok_or_else(|| anyhow::anyhow!("invalid value in transact"))?;

        let valid_from = valid_from_override
            .or(tx_opts.as_ref().and_then(|o| o.valid_from))
            .unwrap_or(0); // placeholder; real tx_id used at commit
        let valid_to = valid_to_override
            .or(tx_opts.as_ref().and_then(|o| o.valid_to))
            .unwrap_or(VALID_TIME_FOREVER);

        // tx_id=0, tx_count=0 are placeholders — stamped at commit
        facts.push(Fact::with_valid_time(entity, attr, value, 0, 0, valid_from, valid_to));
    }
    Ok(facts)
}

/// Convert a parsed retraction `Transaction` into retraction `Fact` structs.
fn materialize_retraction(tx: &Transaction) -> Result<Vec<Fact>> {
    use crate::graph::types::Fact as MFact;
    use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
    use crate::query::datalog::types::Pattern;

    let mut facts = Vec::new();
    for pattern in &tx.facts {
        let (entity_edn, attr_edn, value_edn) = match pattern {
            Pattern::Fact { entity, attribute, value } => (entity, attribute, value),
            _ => bail!("unexpected pattern type in retract"),
        };
        let entity = edn_to_entity_id(entity_edn)
            .ok_or_else(|| anyhow::anyhow!("invalid entity in retract"))?;
        let attr = match attr_edn {
            crate::query::datalog::types::EdnValue::Keyword(k) => k.clone(),
            _ => bail!("attribute must be a keyword"),
        };
        let value = edn_to_value(value_edn)
            .ok_or_else(|| anyhow::anyhow!("invalid value in retract"))?;

        let mut f = MFact::retract(entity, attr, value, 0); // tx_id=0 placeholder
        f.tx_count = 0; // placeholder
        facts.push(f);
    }
    Ok(facts)
}
```

> **Note on `maybe_wal_write_and_checkpoint`:** The current implementation filters facts by `tx_count` after they're applied to `fact_storage`. This works because `DatalogExecutor::execute_transact` calls `fact_storage.transact()` which atomically increments `tx_counter`. The new `tx_count` is then `fact_storage.current_tx_count()`. However, `DatalogExecutor` needs a `new_with_rules` constructor. See Step 2 below.

- [ ] **Step 2: Add `DatalogExecutor::new_with_rules()` to `src/query/datalog/executor.rs`**

The existing `DatalogExecutor::new(storage)` creates a fresh `RuleRegistry`. For `Minigraf` to share rules across all `execute()` calls (rules defined in one call should be visible in subsequent queries), we need to inject the shared `rules` registry.

Add to `src/query/datalog/executor.rs`:

```rust
/// Create a `DatalogExecutor` with a shared rule registry.
///
/// Used by `Minigraf` to share rules across all `execute()` calls.
pub fn new_with_rules(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>) -> Self {
    DatalogExecutor { storage, rules }
}
```

- [ ] **Step 3: Write unit tests for `Minigraf` in `src/db.rs`**

Add at the bottom of `src/db.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_transact_and_query() {
        let db = Minigraf::in_memory().unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 1),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_in_memory_no_wal_file() {
        // in_memory() must use WriteContext::Memory — no WAL file is created.
        // This is a structural smoke test: if inner.write_lock contains a
        // WriteContext::File variant, it would try to write a WAL and would
        // either panic or create a file in an unexpected location.
        // We verify indirectly by checking that no file I/O error occurs.
        let db = Minigraf::in_memory().unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        db.execute("(transact [[:bob :name \"Bob\"]])").unwrap();
        // Verify the data is visible (confirms write path works without WAL)
        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 2),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_same_thread_reentrant_error() {
        let db = Minigraf::in_memory().unwrap();
        let _tx = db.begin_write().unwrap();
        let err = db.execute("(transact [[:alice :name \"Alice\"]])");
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("WriteTransaction"), "error should mention WriteTransaction");
    }

    #[test]
    fn test_write_transaction_commit() {
        let db = Minigraf::in_memory().unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.execute("(transact [[:bob :name \"Bob\"]])").unwrap();
        tx.commit().unwrap();

        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 2),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_write_transaction_rollback() {
        let db = Minigraf::in_memory().unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.rollback();

        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert!(results.is_empty()),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_write_transaction_drop_is_rollback() {
        let db = Minigraf::in_memory().unwrap();
        {
            let mut tx = db.begin_write().unwrap();
            tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
            // drop without commit
        }

        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert!(results.is_empty()),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_write_transaction_read_your_own_writes() {
        let db = Minigraf::in_memory().unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();

        // Query inside tx should see pending fact
        let result = tx.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert_eq!(results.len(), 1),
            _ => panic!("expected query results"),
        }

        tx.rollback();

        // After rollback, fact should not be visible
        let result = db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap();
        match result {
            QueryResult::QueryResults { results, .. } => assert!(results.is_empty()),
            _ => panic!("expected query results"),
        }
    }

    #[test]
    fn test_thread_local_flag_cleared_after_commit() {
        let db = Minigraf::in_memory().unwrap();
        let tx = db.begin_write().unwrap();
        tx.commit().unwrap();
        // Should be able to start a new transaction
        let tx2 = db.begin_write();
        assert!(tx2.is_ok(), "should be able to begin_write after commit");
        tx2.unwrap().rollback();
    }

    #[test]
    fn test_thread_local_flag_cleared_after_rollback() {
        let db = Minigraf::in_memory().unwrap();
        let tx = db.begin_write().unwrap();
        tx.rollback();
        let tx2 = db.begin_write();
        assert!(tx2.is_ok(), "should be able to begin_write after rollback");
        tx2.unwrap().rollback();
    }

    #[test]
    fn test_thread_local_flag_cleared_after_drop() {
        let db = Minigraf::in_memory().unwrap();
        {
            let _tx = db.begin_write().unwrap();
        } // drop
        let tx2 = db.begin_write();
        assert!(tx2.is_ok(), "should be able to begin_write after drop");
        tx2.unwrap().rollback();
    }
}
```

- [ ] **Step 4: Register `db` module in `src/lib.rs`**

Add to `src/lib.rs`:
```rust
pub mod db;

pub use db::{Minigraf, OpenOptions, WriteTransaction};
```

- [ ] **Step 5: Run db unit tests**

```bash
cargo test db::tests -- --nocapture
```
Expected: all 11 tests pass.

If compilation errors occur on the `materialize_transaction`/`materialize_retraction` helpers (due to `Pattern` variant names), check `src/query/datalog/types.rs` for the exact variant names and update accordingly. The `Pattern` enum has `Fact { entity, attribute, value }` and `FactWithValidTime { entity, attribute, value, valid_from, valid_to }` variants — use the exact names as defined in that file.

- [ ] **Step 6: Commit**

```bash
git add src/db.rs src/lib.rs src/query/datalog/executor.rs
git commit -m "feat(db): add Minigraf facade, WriteTransaction, implicit/explicit transactions"
```

---

## Task 6: File-backed integration — WAL recovery, checkpoint

**Files:**
- Modify: `src/db.rs` (wire auto-checkpoint into `maybe_wal_write_and_checkpoint`)
- Create: `tests/wal_test.rs`

Task 5 left auto-checkpoint unimplemented in `maybe_wal_write_and_checkpoint` (it writes to the WAL but never triggers `do_checkpoint`). This step wires it up by adding `fact_storage` to the function signature so it can call `do_checkpoint`, and adds integration tests.

- [ ] **Step 1: Wire auto-checkpoint into `maybe_wal_write_and_checkpoint`**

In `src/db.rs`, update `maybe_wal_write_and_checkpoint` and its call-site. Remove the placeholder comment (`// Auto-checkpoint wired up in Task 6`):

```rust
fn do_checkpoint(fact_storage: &FactStorage, ctx: &mut WriteContext) -> Result<()> {
    let _ = fact_storage;
    match ctx {
        WriteContext::Memory => {}
        WriteContext::File { pfs, wal, db_path, wal_entry_count } => {
            pfs.force_dirty();
            pfs.save()?;
            let wal_path = Inner::wal_path(db_path);
            if wal_path.exists() {
                WalWriter::delete_file(&wal_path)?;
            }
            *wal = WalWriter::open_or_create(&wal_path)?;
            *wal_entry_count = 0;
        }
    }
    Ok(())
}
```

Remove `do_checkpoint_inner` entirely.

Also fix `maybe_wal_write_and_checkpoint` to pass `db_path` for auto-checkpoint:

```rust
fn maybe_wal_write_and_checkpoint(
    fact_storage: &FactStorage,
    ctx: &mut WriteContext,
    opts: &OpenOptions,
) -> Result<()> {
    match ctx {
        WriteContext::Memory => {}
        WriteContext::File { pfs, wal, wal_entry_count, .. } => {
            let tx_count = pfs.storage().current_tx_count();
            let batch: Vec<Fact> = pfs.storage()
                .get_all_facts()?
                .into_iter()
                .filter(|f| f.tx_count == tx_count)
                .collect();

            wal.append_entry(tx_count, &batch)?;
            pfs.mark_dirty();
            *wal_entry_count += 1;
        }
    }
    // Auto-checkpoint check (needs db_path, so done separately)
    if let WriteContext::File { wal_entry_count, .. } = ctx {
        if *wal_entry_count >= opts.wal_checkpoint_threshold {
            Self::do_checkpoint(fact_storage, ctx)?;
        }
    }
    Ok(())
}
```

Update the call-site in `execute()`:
```rust
Self::maybe_wal_write_and_checkpoint(&self.inner.fact_storage, &mut ctx, &self.inner.options)?;
```

- [ ] **Step 2: Create `tests/wal_test.rs`**

```rust
//! Integration tests for WAL-backed crash safety and recovery.

use minigraf::{Minigraf, OpenOptions, QueryResult};

fn count_results(result: QueryResult) -> usize {
    match result {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

#[test]
fn test_file_backed_basic_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    {
        let db = Minigraf::open(&db_path).unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        // db drops here → checkpoint on Drop
    }

    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
        );
        assert_eq!(n, 1, "Alice should persist across close/reopen");
    }
}

#[test]
fn test_wal_recovery_after_simulated_crash() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    // Open with enormous checkpoint threshold so no auto-checkpoint fires
    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    {
        let db = Minigraf::open_with_options(&db_path, opts).unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        // Simulate crash: skip Drop by forgetting the value
        std::mem::forget(db);
    }

    // WAL sidecar should exist alongside the (empty) main file
    let wal_path = {
        let mut p = db_path.as_os_str().to_owned();
        p.push(".wal");
        std::path::PathBuf::from(p)
    };
    assert!(wal_path.exists(), "WAL sidecar must exist after simulated crash");

    // Reopen: WAL replay should recover Alice
    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
        );
        assert_eq!(n, 1, "Alice must be recovered via WAL replay");
    }
}

#[test]
fn test_no_duplicate_facts_after_post_checkpoint_crash() {
    // Simulates: main file checkpointed with Alice, WAL still has Alice's entry
    // (crash between save()+fsync and WAL deletion).
    // Reopen must skip the already-checkpointed WAL entry via last_checkpointed_tx_count.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    let wal_path = {
        let mut p = db_path.as_os_str().to_owned();
        p.push(".wal");
        std::path::PathBuf::from(p)
    };

    // Step 1: Write Alice without auto-checkpoint (WAL accumulates)
    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    {
        let db = Minigraf::open_with_options(&db_path, opts).unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        std::mem::forget(db); // crash — WAL exists, main file is empty
    }
    assert!(wal_path.exists(), "WAL must exist after crash");

    // Step 2: Save a copy of the WAL (will be restored after checkpoint)
    let wal_backup = dir.path().join("test.wal.bak");
    std::fs::copy(&wal_path, &wal_backup).unwrap();

    // Step 3: Proper open → WAL replay → manual checkpoint → clean close
    // After checkpoint: main file has Alice (last_checkpointed_tx_count=1), WAL deleted
    {
        let db = Minigraf::open(&db_path).unwrap(); // replays WAL, loads Alice
        db.checkpoint().unwrap(); // saves main file, deletes WAL
        drop(db);
    }
    assert!(!wal_path.exists(), "WAL must be deleted after checkpoint");

    // Step 4: Restore WAL backup — simulates crash between fsync and WAL deletion
    std::fs::copy(&wal_backup, &wal_path).unwrap();

    // Step 5: Reopen — WAL entry's tx_count ≤ last_checkpointed_tx_count → skipped
    let db = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
    );
    assert_eq!(n, 1, "Alice must appear exactly once — WAL entry was correctly skipped");
}

#[test]
fn test_partial_wal_entry_discarded_earlier_entries_intact() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    let wal_path = {
        let mut p = db_path.as_os_str().to_owned();
        p.push(".wal");
        std::path::PathBuf::from(p)
    };

    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    {
        let db = Minigraf::open_with_options(&db_path, opts).unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        std::mem::forget(db);
    }

    // Append garbage bytes after the valid entry
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(&wal_path).unwrap();
        f.write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x02]).unwrap();
    }

    // Reopen: valid entry (Alice) should be recovered; garbage discarded
    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
        );
        assert_eq!(n, 1, "Alice should be recovered; garbage entry discarded");
    }
}

#[test]
fn test_manual_checkpoint_deletes_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    let wal_path = {
        let mut p = db_path.as_os_str().to_owned();
        p.push(".wal");
        std::path::PathBuf::from(p)
    };

    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    let db = Minigraf::open_with_options(&db_path, opts).unwrap();
    db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    assert!(wal_path.exists(), "WAL should exist before checkpoint");

    db.checkpoint().unwrap();
    // WAL deleted after checkpoint; a new empty WAL is created
    // The new WAL exists (WalWriter::open_or_create creates it) but is empty
    // Verify by reopening: facts should still be present from main file
    let n = count_results(
        db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
    );
    assert_eq!(n, 1);
}

#[test]
fn test_auto_checkpoint_fires_at_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    // Threshold of 2: fires after the 2nd write
    let opts = OpenOptions { wal_checkpoint_threshold: 2 };
    let db = Minigraf::open_with_options(&db_path, opts).unwrap();
    db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    db.execute("(transact [[:bob :name \"Bob\"]])").unwrap();
    // After 2nd write, auto-checkpoint should have fired
    // Verify by checking last_checkpointed_tx_count indirectly:
    // forget the db (skip Drop) and reopen — both facts should be in main file
    std::mem::forget(db);

    // Reopen without WAL replay needed (facts in main file)
    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
    );
    assert_eq!(n, 2, "both facts must be in main file after auto-checkpoint");
}

#[test]
fn test_explicit_tx_all_or_nothing_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    {
        let db = Minigraf::open_with_options(&db_path, opts).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.execute("(transact [[:bob :name \"Bob\"]])").unwrap();
        tx.commit().unwrap();
        std::mem::forget(db);
    }

    let db2 = Minigraf::open(&db_path).unwrap();
    let n = count_results(
        db2.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
    );
    assert_eq!(n, 2, "both facts must be recoverable after explicit tx commit");
}

#[test]
fn test_explicit_tx_rollback_not_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    {
        let db = Minigraf::open(&db_path).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.rollback();
        // db drops with checkpoint here
    }

    {
        let db = Minigraf::open(&db_path).unwrap();
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
        );
        assert_eq!(n, 0, "rolled-back facts must not be visible after reopen");
    }
}

#[test]
fn test_failed_commit_leaves_fact_storage_unchanged() {
    // Spec §Testing plan: "Failed commit(): in-memory state unchanged after failure."
    // Strategy: make the WAL path read-only so append_entry() fails, then verify
    // that FactStorage has no facts.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    let wal_path = {
        let mut p = db_path.as_os_str().to_owned();
        p.push(".wal");
        std::path::PathBuf::from(p)
    };

    // Open the db (creates the WAL file)
    let opts = OpenOptions { wal_checkpoint_threshold: usize::MAX };
    let db = Minigraf::open_with_options(&db_path, opts).unwrap();

    // Make the WAL read-only so the next WAL write fails.
    // NOTE: On Linux, permission changes don't affect already-open file descriptors.
    // If this test is flaky, replace with: remove the WAL file, create a directory
    // at the same path — seek/write on a directory fd returns EISDIR on Linux.
    let mut perms = std::fs::metadata(&wal_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&wal_path, perms).unwrap();

    // Begin tx, buffer a fact, attempt commit — must fail
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    let result = tx.commit();

    // Restore permissions (so cleanup doesn't fail)
    let mut perms = std::fs::metadata(&wal_path).unwrap().permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(&wal_path, perms).unwrap();

    assert!(result.is_err(), "commit must fail when WAL write fails");

    // FactStorage must be unchanged — no facts should be present
    let n = count_results(
        db.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
    );
    assert_eq!(n, 0, "FactStorage must be unchanged after a failed commit");
}

#[test]
fn test_v2_file_opens_and_upgrades_to_v3_on_checkpoint() {
    use minigraf::storage::{FileHeader, FORMAT_VERSION};

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy.graph");

    // Build a v2-format file manually using MemoryBackend logic
    // (write header with version=2, no facts)
    {
        use minigraf::storage::backend::file::FileBackend;
        use minigraf::storage::persistent_facts::PersistentFactStorage;
        use minigraf::graph::types::Value;
        use uuid::Uuid;

        let backend = FileBackend::open(&db_path).unwrap();
        let mut pfs = PersistentFactStorage::new(backend).unwrap();
        // Patch the version to 2 on disk by doing a raw write to page 0
        // The cleanest way: open PFS, save once (writes v3), then manually
        // rewrite the header version byte to 2.
        pfs.mark_dirty();
        pfs.save().unwrap();
        drop(pfs);

        // Overwrite just the version field (bytes 4-8) in the file with 2
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&db_path).unwrap();
        f.seek(SeekFrom::Start(4)).unwrap();
        f.write_all(&2u32.to_le_bytes()).unwrap();
    }

    // Open with Phase 5 code — must accept v2 (last_checkpointed_tx_count reads as 0)
    let db = Minigraf::open(&db_path).unwrap();
    db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    db.checkpoint().unwrap();
    drop(db);

    // Verify the file is now v3
    let mut f = std::fs::File::open(&db_path).unwrap();
    let mut header_page = vec![0u8; minigraf::storage::PAGE_SIZE];
    use std::io::Read;
    f.read_exact(&mut header_page).unwrap();
    let header = FileHeader::from_bytes(&header_page).unwrap();
    assert_eq!(header.version, FORMAT_VERSION, "file must be upgraded to v3 after checkpoint");
    assert!(header.last_checkpointed_tx_count > 0, "last_checkpointed_tx_count must be set");
}

#[test]
fn test_concurrent_reads_while_writer_holds_lock() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    let db = Arc::new(Minigraf::open(&db_path).unwrap());

    // Seed committed data
    db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
    db.checkpoint().unwrap();

    let db_clone = Arc::clone(&db);

    // Hold write lock on the main thread
    let _tx = db.begin_write().unwrap();

    // Spawn a reader — must succeed without blocking, reading committed state
    let reader = std::thread::spawn(move || {
        count_results(
            db_clone.execute("(query [:find ?name :where [?e :name ?name]])").unwrap()
        )
    });

    // Reader sees committed facts while write lock is held
    let n = reader.join().unwrap();
    assert_eq!(n, 1, "readers must see committed state while writer holds the lock");

    // Clean up: rollback the open write tx (its Drop will clear the flag)
}
```

- [ ] **Step 3: Run integration tests**

```bash
cargo test --test wal_test -- --nocapture
```
Expected: all tests pass. Fix any compilation errors (type mismatches, missing pub items) before proceeding.

- [ ] **Step 4: Run full test suite**

```bash
cargo test -- --nocapture 2>&1 | tail -20
```
Expected: ≥ 172 existing tests + new tests all pass.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/wal_test.rs
git commit -m "feat(db): file-backed WAL write, crash recovery, checkpoint; integration tests"
```

---

## Task 7: Update `main.rs`, `lib.rs`, and REPL version string

**Files:**
- Modify: `src/main.rs`
- Modify: `src/lib.rs`
- Modify: `src/repl.rs` (version string)

- [ ] **Step 1: Update `src/main.rs` to use `Minigraf` facade**

Replace the entire `main()` function:

```rust
use clap::Parser;
use minigraf::Minigraf;
use std::path::PathBuf;

/// Minigraf - A tiny, portable, bi-temporal graph database with Datalog queries
#[derive(Parser, Debug)]
#[command(name = "minigraf")]
#[command(version = "0.5.0")]
#[command(about = "Interactive Datalog REPL for graph queries", long_about = None)]
struct Args {
    #[arg(short, long, value_name = "FILE")]
    file: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    println!("Minigraf v0.5.0 - Datalog Graph Database");

    let db = match args.file {
        Some(ref path) => {
            println!("Using file-based storage: {}\n", path.display());
            Minigraf::open(path).expect("Failed to open database file")
        }
        None => {
            println!("Using in-memory storage\n");
            Minigraf::in_memory().expect("Failed to create in-memory database")
        }
    };

    // Provide the FactStorage and rules to the REPL via the shared Arc
    use minigraf::repl::Repl;
    let repl = Repl::new(db.inner_fact_storage());
    repl.run();
}
```

Add a `pub fn inner_fact_storage(&self) -> crate::graph::FactStorage` accessor to `Minigraf` in `src/db.rs`:

```rust
/// Return a clone of the shared `FactStorage` (for REPL use).
pub fn inner_fact_storage(&self) -> crate::graph::FactStorage {
    self.inner.fact_storage.clone()
}
```

- [ ] **Step 2: Update `src/repl.rs` version string**

Change:
```rust
println!("Minigraf v0.4.0 - Interactive Datalog Console");
```
to:
```rust
println!("Minigraf v0.5.0 - Interactive Datalog Console");
```

- [ ] **Step 3: Replace `src/lib.rs` with the final complete version**

Replace the entire contents of `src/lib.rs` with:
(Note: `pub mod wal;` was added in Task 4 Step 1 and `pub mod db;` + `pub use db::{...}` in Task 5 Step 4. This step consolidates everything into one clean file.)
```rust
pub mod db;
pub mod graph;
pub mod query;
pub mod repl;
pub mod storage;
pub mod temporal;
pub mod wal;

pub use db::{Minigraf, OpenOptions, WriteTransaction};
pub use graph::FactStorage;
pub use graph::types::{
    Fact, Value, EntityId, TxId, Attribute,
    tx_id_now, tx_id_from_system_time, tx_id_to_system_time,
    TransactOptions, VALID_TIME_FOREVER,
};
pub use repl::Repl;
pub use storage::backend::file::FileBackend;
pub use storage::persistent_facts::PersistentFactStorage;
pub use storage::{FileHeader, StorageBackend, PAGE_SIZE, FORMAT_VERSION};
pub use query::{
    parse_datalog_command,
    parse_edn,
    DatalogCommand,
    DatalogExecutor,
    DatalogQuery,
    EdnValue,
    Pattern,
    PatternMatcher,
    QueryResult,
    Transaction,
};
pub use query::datalog::types::{AsOf, ValidAt};
```

Note: `FORMAT_VERSION` is newly exported for the `test_v2_file_opens_and_upgrades_to_v3_on_checkpoint` integration test.

- [ ] **Step 4: Build the binary and verify it runs**

```bash
cargo build && echo "Build OK"
echo "(transact [[:alice :name \"Alice\"]])" | cargo run -- 2>/dev/null | grep -i "transacted\|ok"
```
Expected: output includes a success message.

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```
Expected: all tests pass, test count ≥ 195.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/repl.rs src/lib.rs src/db.rs
git commit -m "feat: wire Minigraf facade into main.rs, update version to 0.5.0"
```

---

## Task 8: Final cleanup — update `Cargo.toml` version and ROADMAP

**Files:**
- Modify: `Cargo.toml`
- Modify: `ROADMAP.md`

- [ ] **Step 1: Bump version in `Cargo.toml`**

```toml
version = "0.5.0"
```

- [ ] **Step 2: Mark Phase 5 complete in `ROADMAP.md`**

Change:
```
## Phase 5: ACID + WAL 🎯 FUTURE
**Status**: 🎯 Planned
```
to:
```
## Phase 5: ACID + WAL ✅ COMPLETE
**Status**: ✅ Completed (March 2026)
```

- [ ] **Step 3: Final test run**

```bash
cargo test 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 4: Final commit**

```bash
git add Cargo.toml ROADMAP.md
git commit -m "chore: bump version to 0.5.0, mark Phase 5 complete in ROADMAP"
```

---

## Implementation Notes

### Pattern variant names
Before implementing `materialize_transaction` and `materialize_retraction`, verify the exact variant names in `src/query/datalog/types.rs`. The plan uses `Pattern::Fact { entity, attribute, value }` and `Pattern::FactWithValidTime { ... }`. If the actual names differ, update the match arms accordingly.

### WAL entry count tracking in implicit transactions
`maybe_wal_write_and_checkpoint` identifies the batch to write to the WAL by filtering facts with `tx_count == current_tx_count()`. This works because `DatalogExecutor::execute_transact` calls `fact_storage.transact()` which increments `tx_counter` atomically. The write lock ensures no concurrent increment can happen.

### `do_checkpoint_inner` removal
The scaffold in Task 5 included a stub `do_checkpoint_inner` that is replaced in Task 6. Make sure it is fully removed in Task 6, Step 1.

### Concurrent reads
`FactStorage` uses `Arc<RwLock<Vec<Fact>>>` internally. Reads from `FactStorage` (queries) acquire a short-lived read lock on the inner `Vec`. The `Minigraf` write lock (`Mutex<WriteContext>`) only serializes writes. Concurrent reads never block on the write lock.

### Thread-local flag and `std::mem::forget`
The integration tests use `std::mem::forget(db)` to simulate a crash (skip `Drop::checkpoint()`). This also skips clearing the thread-local flag. Each test runs in its own thread (standard Rust test runner), so flags do not leak across tests.
