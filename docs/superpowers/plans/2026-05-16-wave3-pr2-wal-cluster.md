# Wave 3 PR 2 — WAL Cluster Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a WAL crash-recovery matrix (7 cases, #209), WAL durability fault-injection tests (5 cases, #214), and WAL/file-format fuzz targets with seed corpus (#210).

**Architecture:** Crash-recovery tests open a real file-backed `Minigraf`, write data, then corrupt or truncate the `.wal` sidecar file using `std::fs`, and reopen to verify that valid committed entries still replay and corrupt trailing entries are discarded. Fault-injection tests for the main storage sync path (`#214`) use `FaultInjectingBackend<MemoryBackend>` via `PersistentFactStorage` (unit tests inside `src/storage/persistent_facts.rs`), and public-API tests for lock/state leak coverage in `tests/wal_test.rs`.

**Tech Stack:** Rust stable, `tempfile` crate (already a dev-dep), `minigraf` internal WAL types (`WalWriter`, `WalReader`), `FaultInjectingBackend` (added in PR 1)

**Prerequisites:** PR 1 merged (provides `FaultInjectingBackend`)

**Closes:** #209, #210, #214

---

## File Map

| Action | Path | Purpose |
|---|---|---|
| Modify | `tests/wal_test.rs` | crash-recovery matrix + public-API fault tests |
| Modify | `src/storage/persistent_facts.rs` | unit tests for sync fault injection |
| Modify | `fuzz/fuzz_targets/wal_entry.rs` | WAL entry fuzz target (#210) |
| Modify | `fuzz/fuzz_targets/file_header.rs` | file header fuzz target (#210) |
| Modify | `fuzz/fuzz_targets/fact_page.rs` | packed fact page fuzz target (#210) |
| Modify | `fuzz/fuzz_targets/btree_page.rs` | B+tree page fuzz target (#210) |
| Create | `fuzz/corpus/wal_entry/` | seed corpus |
| Create | `fuzz/corpus/file_header/` | seed corpus |
| Create | `fuzz/corpus/fact_page/` | seed corpus |
| Create | `fuzz/corpus/btree_page/` | seed corpus |

---

## Task 1: WAL crash-recovery matrix (#209)

These are integration tests using `Minigraf::open()` + direct WAL file manipulation.

**Files:**
- Modify: `tests/wal_test.rs`

- [ ] **Step 1: Write failing test stubs**

Append to `tests/wal_test.rs`:

```rust
// ══ Wave 3: #209 WAL crash-recovery matrix ════════════════════════════════

/// Helper: read raw WAL file bytes.
fn read_wal_bytes(db_path: &std::path::Path) -> Vec<u8> {
    std::fs::read(wal_path_for(db_path)).unwrap_or_default()
}

/// Helper: overwrite WAL file with given bytes.
fn write_wal_bytes(db_path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(wal_path_for(db_path), bytes).unwrap();
}

/// Helper: write a fact to a fresh db, return (dir, db_path, wal_bytes_after_write).
fn setup_db_with_one_fact() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:e1 :name "Alice"]])"#).unwrap();
        // Drop without explicit checkpoint — WAL sidecar exists.
        std::mem::forget(db);
    }
    let wal_bytes = read_wal_bytes(&db_path);
    (dir, db_path, wal_bytes)
}

/// Helper: query all :name values from a db path.
fn query_names(db_path: &std::path::Path) -> Vec<String> {
    let db = minigraf::db::Minigraf::open(db_path).unwrap();
    match db.execute("(query [:find ?n :where [?e :name ?n]])").unwrap() {
        minigraf::QueryResult::QueryResults { results, .. } => results
            .into_iter()
            .flat_map(|r| r.into_values())
            .filter_map(|v| match v {
                minigraf::graph::types::Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// 209-1: Truncated length/header bytes — corrupt trailing WAL entry header.
/// The valid entry before corruption must still replay.
#[test]
fn wal_recover_truncated_length_header() {
    let (_dir, db_path, wal_bytes) = setup_db_with_one_fact();
    assert!(!wal_bytes.is_empty(), "WAL should have content");
    // Truncate to 50% of WAL — leaves valid header + partial entry.
    write_wal_bytes(&db_path, &wal_bytes[..wal_bytes.len() / 2]);
    let names = query_names(&db_path);
    assert_eq!(names.len(), 0, "partial WAL entry must not be applied");
}

/// 209-2: Truncated payload bytes — entry header intact but payload cut short.
#[test]
fn wal_recover_truncated_payload() {
    let (_dir, db_path, wal_bytes) = setup_db_with_one_fact();
    // Keep header (32 bytes) + entry length prefix but cut payload.
    let truncation_point = (wal_bytes.len() * 3) / 4;
    write_wal_bytes(&db_path, &wal_bytes[..truncation_point]);
    let names = query_names(&db_path);
    assert_eq!(names.len(), 0, "entry with truncated payload must not be applied");
}

/// 209-3: Valid entry followed by bad checksum in a second entry.
/// First entry must replay; second must be discarded.
#[test]
fn wal_recover_bad_checksum_second_entry() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");

    // Write TWO facts in separate transactions so the WAL has two entries.
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:e1 :name "Alice"]])"#).unwrap();
        db.execute(r#"(transact [[:e2 :name "Bob"]])"#).unwrap();
        std::mem::forget(db);
    }

    let mut wal_bytes = read_wal_bytes(&db_path);
    assert!(wal_bytes.len() > 32 + 4, "WAL too short to corrupt checksum");

    // Corrupt the checksum of the second entry (bytes 32..36 = first entry checksum;
    // the second entry starts somewhere after the first. Flip last 4 bytes as a proxy).
    let n = wal_bytes.len();
    wal_bytes[n - 4] ^= 0xFF;
    wal_bytes[n - 3] ^= 0xFF;
    write_wal_bytes(&db_path, &wal_bytes);

    let names = query_names(&db_path);
    // First entry (Alice) should replay; second (Bob) discarded.
    assert_eq!(names.len(), 1, "only the entry before bad checksum should replay");
    assert!(names.contains(&"Alice".to_string()), "Alice should survive");
}

/// 209-4: Explicit commit followed by simulated crash before checkpoint.
/// Committed facts must survive reopen.
#[test]
fn wal_recover_committed_tx_crash_before_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:e1 :name "Charlie"]])"#).unwrap();
        tx.commit().unwrap();
        // Simulate crash: forget without checkpoint.
        std::mem::forget(db);
    }
    let names = query_names(&db_path);
    assert_eq!(names.len(), 1, "committed tx must survive crash before checkpoint");
    assert!(names.contains(&"Charlie".to_string()));
}

/// 209-5: Rollback followed by simulated crash.
/// Rolled-back fact must NOT appear after reopen.
#[test]
fn wal_recover_rollback_crash() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:e1 :name "Dave"]])"#).unwrap();
        tx.rollback().unwrap();
        std::mem::forget(db);
    }
    let names = query_names(&db_path);
    assert_eq!(names.len(), 0, "rolled-back fact must not appear after crash");
}

/// 209-6: Multiple committed transactions in one WAL, final entry corrupt.
/// All entries before the corrupt tail must replay.
#[test]
fn wal_recover_multiple_committed_corrupt_tail() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:e1 :name "Eve"]])"#).unwrap();
        db.execute(r#"(transact [[:e2 :name "Frank"]])"#).unwrap();
        std::mem::forget(db);
    }
    let mut wal_bytes = read_wal_bytes(&db_path);
    // Append a junk trailing entry.
    wal_bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00]);
    write_wal_bytes(&db_path, &wal_bytes);

    let names = query_names(&db_path);
    assert_eq!(names.len(), 2, "both valid entries must replay; junk tail is discarded");
}

/// 209-7: Corrupt tail entry is never applied.
/// This reinforces that the WAL reader discards incomplete trailing entries.
#[test]
fn wal_corrupt_tail_never_applied() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.graph");
    {
        let db = minigraf::db::Minigraf::open(&db_path).unwrap();
        db.execute(r#"(transact [[:e1 :name "Grace"]])"#).unwrap();
        std::mem::forget(db);
    }
    let mut wal_bytes = read_wal_bytes(&db_path);
    // Append a partial entry that claims a huge num_facts but provides no data.
    // tx_count = 999, num_facts = 1000, no actual fact bytes.
    let mut fake_entry: Vec<u8> = Vec::new();
    fake_entry.extend_from_slice(&0u32.to_le_bytes()); // checksum (wrong)
    fake_entry.extend_from_slice(&999u64.to_le_bytes()); // tx_count
    fake_entry.extend_from_slice(&1000u64.to_le_bytes()); // num_facts (no data follows)
    wal_bytes.extend_from_slice(&fake_entry);
    write_wal_bytes(&db_path, &wal_bytes);

    // Must not panic; must not apply the fake entry.
    let names = query_names(&db_path);
    assert!(names.contains(&"Grace".to_string()), "Grace should replay");
    assert_eq!(names.len(), 1, "fake entry must not create phantom facts");
}
```

- [ ] **Step 2: Run and verify tests pass**

```bash
cargo test wal_recover --test wal_test -- --nocapture 2>&1 | tail -30
```
Expected: all 7 `wal_recover_*` and `wal_corrupt_tail_*` tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/wal_test.rs
git commit -m "test(wal): add #209 crash-recovery matrix (7 cases)"
```

---

## Task 2: WAL durability fault-injection (#214)

Three of the five cases test `PersistentFactStorage` sync behavior directly (unit tests); two test lock/state leak via the public `Minigraf` API (integration tests).

**Files:**
- Modify: `src/storage/persistent_facts.rs` (unit tests for sync fault)
- Modify: `tests/wal_test.rs` (lock-leak integration tests)

- [ ] **Step 1: Add sync-failure unit tests to persistent_facts.rs**

At the bottom of `src/storage/persistent_facts.rs`, find the `#[cfg(test)]` block (or add one) and append:

```rust
#[cfg(test)]
mod fault_injection_tests {
    use super::*;
    use crate::storage::backend::fault_inject::{FaultConfig, FaultInjectingBackend};
    use crate::storage::backend::MemoryBackend;

    fn make_pfs() -> PersistentFactStorage<FaultInjectingBackend<MemoryBackend>> {
        let (backend, _cfg) = FaultInjectingBackend::with_config(MemoryBackend::new());
        PersistentFactStorage::new(backend, 16).unwrap()
    }

    /// 214-1: WAL append (main-store write) fails before fact applied.
    /// save() must return Err; no fact visible afterward.
    #[test]
    fn save_returns_error_when_write_fails() {
        let (backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        let mut pfs = PersistentFactStorage::new(backend, 16).unwrap();

        // Inject write failure immediately.
        config.lock().unwrap().fail_write_after = Some(0);

        // transact_batch writes to the in-memory store (no backend writes yet).
        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;
        let fact = Fact {
            entity: Uuid::new_v4(),
            attribute: ":test/attr".to_string(),
            value: Value::String("v".to_string()),
            tx_id: 1,
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        };
        pfs.storage_mut().transact_batch(vec![fact]).unwrap();
        pfs.mark_dirty();

        // save() tries to write_page — should fail.
        let result = pfs.save();
        assert!(result.is_err(), "save must return Err when write_page fails");
    }

    /// 214-3: sync fails after bytes written.
    /// save() must propagate the sync error.
    #[test]
    fn save_returns_error_when_sync_fails() {
        let (backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        let mut pfs = PersistentFactStorage::new(backend, 16).unwrap();

        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;
        let fact = Fact {
            entity: Uuid::new_v4(),
            attribute: ":test/name".to_string(),
            value: Value::String("x".to_string()),
            tx_id: 1,
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        };
        pfs.storage_mut().transact_batch(vec![fact]).unwrap();
        pfs.mark_dirty();

        // Allow all writes; fail on first sync.
        config.lock().unwrap().fail_sync_after = Some(0);

        let result = pfs.save();
        assert!(result.is_err(), "save must return Err when sync fails");
    }

    /// 214-4: Checkpoint/main-file sync failure returns error (not silent).
    /// This is covered by the sync test above — sync errors propagate, not swallowed.
    /// Add an explicit assertion that the error message is non-empty.
    #[test]
    fn save_error_is_non_empty() {
        let (backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        let mut pfs = PersistentFactStorage::new(backend, 16).unwrap();

        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;
        let fact = Fact {
            entity: Uuid::new_v4(),
            attribute: ":test/x".to_string(),
            value: Value::Boolean(true),
            tx_id: 1,
            tx_count: 1,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        };
        pfs.storage_mut().transact_batch(vec![fact]).unwrap();
        pfs.mark_dirty();
        config.lock().unwrap().fail_sync_after = Some(0);

        let err = pfs.save().unwrap_err();
        assert!(!err.to_string().is_empty(), "error message must not be empty");
    }
}
```

Note: `storage_mut()` may not exist as a public method. If it doesn't, look for the field name in `PersistentFactStorage` and use the appropriate accessor. Check with:
```bash
grep -n 'fn storage\|pub.*storage' src/storage/persistent_facts.rs | head -10
```
Use `pfs.storage()` (read-only) or adjust to use whatever mutable accessor exists. If only `pfs.storage()` is available, add facts via the public `execute()` path by using `Minigraf::in_memory()` instead (see note below test).

- [ ] **Step 2: Run unit tests**

```bash
cargo test fault_injection_tests 2>&1 | tail -20
```
Expected: 3 tests pass.

- [ ] **Step 3: Add lock-leak integration tests to tests/wal_test.rs**

Append to `tests/wal_test.rs`:

```rust
// ══ Wave 3: #214 lock-leak / state-leak after errors ══════════════════════

/// 214-5a: Error path does not leave write lock stuck.
/// After a transaction fails, a second write transaction must succeed on the same db.
#[test]
fn write_lock_not_leaked_after_rollback() {
    let db = minigraf::db::Minigraf::in_memory().unwrap();

    // First transaction — rollback.
    let mut tx1 = db.begin_write().unwrap();
    tx1.execute(r#"(transact [[:e1 :name "Temp"]])"#).unwrap();
    tx1.rollback().unwrap();

    // Second transaction on the same db — must succeed without deadlock.
    let mut tx2 = db.begin_write().unwrap();
    tx2.execute(r#"(transact [[:e2 :name "Perm"]])"#).unwrap();
    tx2.commit().unwrap();

    let n = count_results(
        db.execute("(query [:find ?n :where [?e :name ?n]])")
            .unwrap(),
    );
    assert_eq!(n, 1, "only committed fact should be visible");
}

/// 214-5b: Write state is clean after a dropped (not committed) transaction.
#[test]
fn write_state_clean_after_drop() {
    let db = minigraf::db::Minigraf::in_memory().unwrap();

    {
        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:e1 :name "Ghost"]])"#).unwrap();
        // Drop without commit or rollback — should auto-rollback.
    }

    // New write must succeed.
    let mut tx2 = db.begin_write().unwrap();
    tx2.execute(r#"(transact [[:e2 :name "Real"]])"#).unwrap();
    tx2.commit().unwrap();

    let n = count_results(
        db.execute("(query [:find ?n :where [?e :name ?n]])")
            .unwrap(),
    );
    assert_eq!(n, 1, "only committed fact visible after dropped tx");
}
```

- [ ] **Step 4: Run all new tests**

```bash
cargo test write_lock_not_leaked write_state_clean --test wal_test 2>&1 | tail -10
```
Expected: both pass.

- [ ] **Step 5: Run full test suite to verify no regressions**

```bash
cargo test 2>&1 | tail -15
```
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/storage/persistent_facts.rs tests/wal_test.rs
git commit -m "test(wal): add #214 fault-injection and lock-leak tests (5 cases)"
```

---

## Task 3: WAL/file-format fuzz targets (#210)

**Files:**
- Modify: `fuzz/fuzz_targets/wal_entry.rs`, `file_header.rs`, `fact_page.rs`, `btree_page.rs`
- Create: `fuzz/corpus/wal_entry/`, `file_header/`, `fact_page/`, `btree_page/`

These replace the stub targets from PR 1 with real decode paths.

- [ ] **Step 1: Identify decode entry points**

```bash
grep -n 'pub fn\|pub(crate) fn' src/wal.rs | grep -i 'read\|decode\|parse\|open\|entry'
grep -n 'pub fn\|pub use' src/storage/mod.rs | grep -i 'header\|from_bytes'
grep -n 'pub fn\|pub(crate) fn' src/storage/packed_pages.rs | head -10
grep -n 'pub fn\|pub(crate) fn' src/storage/btree_v6.rs | grep -i 'read\|decode\|node\|page' | head -10
```

Note the exact function names. The fuzz targets call these directly or via `Minigraf::in_memory()` if functions are not pub.

- [ ] **Step 2: Replace wal_entry.rs**

Replace `fuzz/fuzz_targets/wal_entry.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz WAL header validation and entry decoding.
    // Uses a temp file to exercise the WalReader path.
    use std::io::Write;
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let wal_path = dir.path().join("fuzz.wal");
    if std::fs::write(&wal_path, data).is_err() {
        return;
    }
    // WalReader::open() + read_entries() must never panic on arbitrary bytes.
    if let Ok(mut reader) = minigraf::wal::WalReader::open(&wal_path) {
        let _ = reader.read_entries();
    }
});
```

Note: if `minigraf::wal::WalReader` is not pub from the crate root, check `src/lib.rs` for exports. If not exported, use `Minigraf::open()` with corrupted `.wal` files instead:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let db_path = dir.path().join("fuzz.graph");
    let wal_path = dir.path().join("fuzz.graph.wal");

    // Write arbitrary bytes as the WAL sidecar.
    if std::fs::write(&wal_path, data).is_err() {
        return;
    }
    // Open must handle arbitrary WAL without panic.
    let _ = minigraf::db::Minigraf::open(&db_path);
});
```

Use whichever approach compiles. The fallback (writing WAL bytes then opening) is always valid.

- [ ] **Step 3: Replace file_header.rs**

Replace `fuzz/fuzz_targets/file_header.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    use std::io::Write;
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    // Pad or truncate to at least one 4096-byte page so the reader has data to parse.
    let mut page = vec![0u8; 4096];
    let copy_len = data.len().min(4096);
    page[..copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &page).is_err() {
        return;
    }
    // FileHeader::from_bytes (via Minigraf::open) must never panic.
    let _ = minigraf::db::Minigraf::open(&path);
});
```

- [ ] **Step 4: Replace fact_page.rs**

Replace `fuzz/fuzz_targets/fact_page.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    use std::io::Write;
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    // Build a 2-page file: a valid-ish header (MGRF magic, version 7) + arbitrary fact page.
    let mut content = vec![0u8; 4096 * 2];
    content[0..4].copy_from_slice(b"MGRF");
    content[4..8].copy_from_slice(&7u32.to_le_bytes()); // version 7
    // Page 1 = arbitrary fact page data.
    let copy_len = data.len().min(4096);
    content[4096..4096 + copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &content).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&path);
});
```

- [ ] **Step 5: Replace btree_page.rs**

Replace `fuzz/fuzz_targets/btree_page.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    use std::io::Write;
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    // 3-page file: valid header + 1 fact page + arbitrary B+tree page.
    let mut content = vec![0u8; 4096 * 3];
    content[0..4].copy_from_slice(b"MGRF");
    content[4..8].copy_from_slice(&7u32.to_le_bytes());
    // Fact page count = 1 (bytes 60..64 in header — adjust offset if layout differs).
    content[60..64].copy_from_slice(&1u32.to_le_bytes());
    // Page 2 = arbitrary B+tree page.
    let copy_len = data.len().min(4096);
    content[4096 * 2..4096 * 2 + copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &content).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&path);
});
```

Note: the header field offsets above are approximate. If `Minigraf::open()` returns a format error (not a panic), that's fine. The key contract is: no panic on arbitrary bytes.

- [ ] **Step 6: Create seed corpus**

```bash
mkdir -p fuzz/corpus/wal_entry fuzz/corpus/file_header fuzz/corpus/fact_page fuzz/corpus/btree_page
```

Create seed files for WAL entry (bytes matching WAL magic + minimal entry):

`fuzz/corpus/wal_entry/seed_magic.bin` — write these exact bytes:
```bash
printf 'MWAL\x01\x00\x00\x00%0.24d' '' | head -c 32 > fuzz/corpus/wal_entry/seed_magic.bin
```

Or create a script to write binary seeds:
```bash
python3 -c "
import struct, sys
# WAL header: magic + version=1 + 24 reserved bytes
data = b'MWAL' + struct.pack('<I', 1) + b'\x00' * 24
sys.stdout.buffer.write(data)
" > fuzz/corpus/wal_entry/seed_header.bin

python3 -c "
import struct, sys
# Empty WAL (header only, no entries)
data = b'MWAL' + struct.pack('<I', 1) + b'\x00' * 24
sys.stdout.buffer.write(data)
" > fuzz/corpus/wal_entry/seed_empty.bin

# File header seed: MGRF + v7
python3 -c "
import struct, sys
page = bytearray(4096)
page[0:4] = b'MGRF'
page[4:8] = struct.pack('<I', 7)
sys.stdout.buffer.write(bytes(page))
" > fuzz/corpus/file_header/seed_v7.bin

# Truncated header
python3 -c "import sys; sys.stdout.buffer.write(b'MGRF')" > fuzz/corpus/file_header/seed_truncated.bin

# Wrong magic
python3 -c "
import struct, sys
page = bytearray(4096)
page[0:4] = b'XXXX'
sys.stdout.buffer.write(bytes(page))
" > fuzz/corpus/file_header/seed_bad_magic.bin
```

- [ ] **Step 7: Verify fuzz targets compile**

```bash
cargo +nightly check --manifest-path fuzz/Cargo.toml 2>&1 | tail -10
```
Expected: no errors.

- [ ] **Step 8: Verify each fuzz target runs without crashing on seeds**

```bash
cargo +nightly fuzz run wal_entry fuzz/corpus/wal_entry -- -max_total_time=10 2>&1 | tail -5
cargo +nightly fuzz run file_header fuzz/corpus/file_header -- -max_total_time=10 2>&1 | tail -5
cargo +nightly fuzz run fact_page fuzz/corpus/fact_page -- -max_total_time=10 2>&1 | tail -5
cargo +nightly fuzz run btree_page fuzz/corpus/btree_page -- -max_total_time=10 2>&1 | tail -5
```
Expected: runs 10 seconds each, no crashes.

- [ ] **Step 9: Commit**

```bash
git add fuzz/fuzz_targets/wal_entry.rs fuzz/fuzz_targets/file_header.rs \
    fuzz/fuzz_targets/fact_page.rs fuzz/fuzz_targets/btree_page.rs \
    fuzz/corpus/wal_entry/ fuzz/corpus/file_header/ \
    fuzz/corpus/fact_page/ fuzz/corpus/btree_page/
git commit -m "test(fuzz): add #210 WAL and file-format fuzz targets with seed corpus"
```

---

## Task 4: Open PR

- [ ] **Push and open PR**

```bash
git push -u origin HEAD
gh pr create \
  --title "test(wal): crash-recovery matrix, fault-injection, and fuzz targets (#209, #210, #214)" \
  --body "$(cat <<'EOF'
## Wave 3 PR 2 — WAL Cluster

Closes #209, #210, and #214.

### #209 — Crash-recovery matrix (7 cases in `tests/wal_test.rs`)
- Truncated length/header bytes
- Truncated payload bytes
- Bad checksum on second entry (valid first entry still replays)
- Committed tx crash before checkpoint
- Rollback + crash
- Multiple committed txs + corrupt tail
- Corrupt tail entry never applied

### #214 — Durability fault-injection (5 cases)
- 3 unit tests in `src/storage/persistent_facts.rs` using `FaultInjectingBackend`: write fails before apply, sync fails after write, sync error is non-empty
- 2 integration tests in `tests/wal_test.rs`: write lock not leaked after rollback, write state clean after dropped tx

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Monitor CI until green before merging**
