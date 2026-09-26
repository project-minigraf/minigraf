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

use crate::db::SyncMode;
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::Fact;
use crate::storage::dir_sync::sync_parent_dir;
use crate::storage::packed_pages::MAX_FACT_BYTES;
use anyhow::Result;
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
    file.sync_all()?;
    Ok(())
}

fn validate_wal_header(file: &mut File) -> Result<()> {
    let mut buf = [0u8; WAL_HEADER_SIZE];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut buf)?;

    if buf[0..4] != WAL_MAGIC {
        bail_coded!(ErrorCode::Wal001);
    }
    let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if version != WAL_VERSION {
        bail_coded!(ErrorCode::Wal002, version, WAL_VERSION);
    }
    Ok(())
}

/// Whether an existing WAL file's header is intact, or too short to
/// contain one at all.
enum WalHeaderState {
    /// At least `WAL_HEADER_SIZE` bytes present; validated by
    /// `validate_wal_header` (magic/version checked separately).
    Present,
    /// Fewer than `WAL_HEADER_SIZE` bytes on disk. This can only happen
    /// from a crash between `create_new` and `write_wal_header` completing
    /// in `WalWriter::open_or_create` -- before any `WalWriter` existed to
    /// append an entry with. Safe to treat as an empty WAL (#308-class
    /// fix: the same "kill mid multi-step file write" window that
    /// corrupted the main `.graph` header applies here to the WAL sidecar).
    Absent,
}

/// Distinguishes a too-short WAL file from one long enough to validate.
/// Does not itself validate magic/version -- callers still run
/// `validate_wal_header` on the `Present` case.
fn check_wal_header_length(file: &mut File) -> Result<WalHeaderState> {
    let len = file.metadata()?.len();
    if len < WAL_HEADER_SIZE as u64 {
        Ok(WalHeaderState::Absent)
    } else {
        Ok(WalHeaderState::Present)
    }
}

// ─── Entry serialization ────────────────────────────────────────────────────

fn serialize_entry(tx_count: u64, facts: &[Fact]) -> Result<Vec<u8>> {
    // Build payload (everything covered by the checksum)
    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(&tx_count.to_le_bytes());
    payload.extend_from_slice(&(facts.len() as u64).to_le_bytes());
    for fact in facts {
        let fact_bytes = postcard::to_allocvec(fact)?;
        if fact_bytes.len() > MAX_FACT_BYTES {
            bail_coded!(ErrorCode::Wal003, fact_bytes.len(), MAX_FACT_BYTES);
        }
        let fact_len = u32::try_from(fact_bytes.len())
            .map_err(|_| err_coded!(ErrorCode::Wal004, fact_bytes.len()))?;
        payload.extend_from_slice(&fact_len.to_le_bytes());
        payload.extend_from_slice(&fact_bytes);
    }

    let checksum = crc32fast::hash(&payload);

    let mut entry = Vec::with_capacity(payload.len().saturating_add(4));
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
    sync_mode: SyncMode,
}

impl WalWriter {
    /// Open an existing WAL or create a new one.
    ///
    /// If creating, writes the WAL header.
    /// If opening, validates the header and seeks to the end for appending.
    pub fn open_or_create(path: &Path, sync_mode: SyncMode) -> Result<Self> {
        // Try atomic create-new first (no TOCTOU window)
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                write_wal_header(&mut file)?;
                // The header is durable; make the WAL's directory entry
                // durable too, or a power loss can lose the whole file (#389).
                sync_parent_dir(path)?;
                file.seek(SeekFrom::End(0))?;
                return Ok(WalWriter { file, sync_mode });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }

        // File exists — validate its header and seek to end for appending
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        match check_wal_header_length(&mut file)? {
            WalHeaderState::Present => validate_wal_header(&mut file)?,
            // A previous crash landed between this file's creation and its
            // header write completing; no entry could have been appended
            // yet, so re-initialize it as a fresh, empty WAL.
            // The crash may also have preceded the directory sync that
            // follows creation, so repeat it (#389).
            WalHeaderState::Absent => {
                write_wal_header(&mut file)?;
                sync_parent_dir(path)?;
            }
        }
        file.seek(SeekFrom::End(0))?;
        Ok(WalWriter { file, sync_mode })
    }

    /// The `SyncMode` this writer was opened with. Used only in tests, to
    /// verify that `SyncMode` reaches `WalWriter` from every call site.
    #[cfg(test)]
    pub(crate) fn sync_mode(&self) -> SyncMode {
        self.sync_mode
    }

    /// Serialize `facts` as a WAL entry and append it to the file.
    ///
    /// The entry is written atomically from the caller's perspective:
    /// a partial write produces a bad CRC32, which the reader discards.
    /// Then flushes to disk, unless `sync_mode` is `SyncMode::Normal` — see [`SyncMode`].
    pub fn append_entry(&mut self, tx_count: u64, facts: &[Fact]) -> Result<()> {
        let entry_bytes = serialize_entry(tx_count, facts)?;
        self.file.write_all(&entry_bytes)?;
        match self.sync_mode {
            SyncMode::Full => self.file.sync_data()?,
            SyncMode::Normal => {}
        }
        Ok(())
    }

    /// Delete the WAL file at `path`. Called after a successful checkpoint.
    ///
    /// Fsyncs the parent directory after the remove, so a power loss cannot
    /// bring the deleted WAL back (#389).
    ///
    /// Uses a short retry loop to tolerate Windows races where the OS file handle
    /// is not immediately released after the `WalWriter` is dropped.
    pub fn delete_file(path: &Path) -> Result<()> {
        // Retry up to ~500 ms on Windows where handle release can lag drop().
        let retries: u32 = if cfg!(windows) { 10 } else { 1 };
        let mut last_err = None;
        for i in 0..retries {
            match std::fs::remove_file(path) {
                Ok(()) => {
                    sync_parent_dir(path)?;
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                    if i.saturating_add(1) < retries {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        }
        Err(err_coded!(
            ErrorCode::Wal006,
            path.display(),
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        ))
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
        match check_wal_header_length(&mut file)? {
            WalHeaderState::Present => validate_wal_header(&mut file)?,
            // Too short to contain a header, which can only mean a crash
            // landed before any entry was ever appended (see
            // `WalHeaderState::Absent`). `read_entries` already treats
            // hitting EOF while reading an entry as "no more entries", so
            // leaving the file as-is and letting that seek-past-end happen
            // naturally is correct -- there is nothing here to replay.
            WalHeaderState::Absent => {}
        }
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
            let num_facts = usize::try_from(u64::from_le_bytes(num_facts_buf))
                .map_err(|_| err_coded!(ErrorCode::Wal005))?;

            // Sanity cap: no legitimate entry has more than 1M facts
            const MAX_FACTS_PER_ENTRY: usize = 1_000_000;
            if num_facts > MAX_FACTS_PER_ENTRY {
                break; // treat as corrupt entry
            }

            // Maximum fact size to prevent memory exhaustion from large facts
            const MAX_FACT_SIZE: usize = 10 * 1024 * 1024; // 10MB

            // Build payload for CRC32 verification
            let mut payload = Vec::new();
            payload.extend_from_slice(&tx_count_buf);
            payload.extend_from_slice(&num_facts_buf);

            // Read each fact
            let mut facts = Vec::new(); // grow dynamically instead of pre-allocating
            let mut truncated = false;
            for _ in 0..num_facts {
                let mut len_buf = [0u8; 4];
                if self.file.read_exact(&mut len_buf).is_err() {
                    truncated = true;
                    break;
                }
                let fact_len = u32::from_le_bytes(len_buf) as usize;
                if fact_len > MAX_FACT_SIZE {
                    truncated = true;
                    break;
                }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::graph::types::{VALID_TIME_FOREVER, Value};
    use uuid::Uuid;

    fn make_fact(entity: Uuid, attr: &str, value: Value, tx_count: u64) -> Fact {
        Fact::with_valid_time(
            entity,
            attr.to_string(),
            value,
            1000,
            tx_count,
            0,
            VALID_TIME_FOREVER,
        )
    }

    #[test]
    fn test_wal_empty_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let _writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

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

        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer.append_entry(1, std::slice::from_ref(&fact)).unwrap();

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
    fn test_wal_normal_mode_write_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let fact = make_fact(alice, ":name", Value::String("Alice".to_string()), 1);

        // Normal mode skips the per-write flush; the entry must still be
        // durable-within-process (write_all()'d) and correctly replayable.
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Normal).unwrap();
        writer.append_entry(1, std::slice::from_ref(&fact)).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tx_count, 1);
        assert_eq!(entries[0].facts.len(), 1);
        assert_eq!(entries[0].facts[0].entity, fact.entity);
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

        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer.append_entry(1, &facts).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].facts.len(), 2);
        assert_eq!(entries[0].facts[0].entity, facts[0].entity);
        assert_eq!(entries[0].facts[0].attribute, facts[0].attribute);
        assert_eq!(entries[0].facts[0].value, facts[0].value);
        assert_eq!(entries[0].facts[1].entity, facts[1].entity);
        assert_eq!(entries[0].facts[1].attribute, facts[1].attribute);
        assert_eq!(entries[0].facts[1].value, facts[1].value);
    }

    #[test]
    fn test_wal_multiple_entries_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer
            .append_entry(
                1,
                &[make_fact(
                    alice,
                    ":name",
                    Value::String("Alice".to_string()),
                    1,
                )],
            )
            .unwrap();
        writer
            .append_entry(
                2,
                &[make_fact(bob, ":name", Value::String("Bob".to_string()), 2)],
            )
            .unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tx_count, 1);
        assert_eq!(entries[1].tx_count, 2);
    }

    #[test]
    fn test_wal_reopen_and_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // First open: create WAL and write entry with tx_count=1
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer
            .append_entry(
                1,
                &[make_fact(
                    alice,
                    ":name",
                    Value::String("Alice".to_string()),
                    1,
                )],
            )
            .unwrap();
        drop(writer);

        // Second open: exercises the fallback branch (file already exists)
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer
            .append_entry(
                2,
                &[make_fact(bob, ":name", Value::String("Bob".to_string()), 2)],
            )
            .unwrap();
        drop(writer);

        // Read back and verify both entries are present with correct tx_count values
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

    /// #308-class fix: a crash between `create_new` and the header write
    /// finishing in `open_or_create` leaves a WAL file shorter than
    /// `WAL_HEADER_SIZE`. No entry could ever have been appended at that
    /// point (that requires a fully-open `WalWriter`), so this must be
    /// treated as an empty WAL rather than a hard read error.
    #[test]
    fn test_wal_reader_tolerates_header_shorter_than_wal_header_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.wal");

        // Simulates a kill mid-write_wal_header: only the magic made it out.
        std::fs::write(&path, b"MWAL").unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert!(
            entries.is_empty(),
            "a too-short WAL header must read back as empty, not error"
        );
    }

    /// As above, but for the writer side: re-opening a WAL left too short
    /// by a prior crash must re-initialize it with a fresh header instead
    /// of failing `open_or_create`, and the WAL must be fully usable
    /// afterward.
    #[test]
    fn test_wal_writer_reinitializes_header_shorter_than_wal_header_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.wal");

        std::fs::write(&path, b"MW").unwrap();

        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        let alice = Uuid::new_v4();
        let fact = make_fact(alice, ":name", Value::String("Alice".to_string()), 1);
        writer.append_entry(1, std::slice::from_ref(&fact)).unwrap();
        drop(writer);

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert_eq!(entries.len(), 1, "re-initialized WAL must accept entries");
    }

    /// A completely empty (0-byte) WAL file -- the earliest possible point
    /// in the crash window, right after `create_new` -- must be tolerated
    /// the same way.
    #[test]
    fn test_wal_reader_tolerates_zero_byte_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wal");
        std::fs::write(&path, []).unwrap();

        let mut reader = WalReader::open(&path).unwrap();
        let entries = reader.read_entries().unwrap();
        assert!(entries.is_empty(), "a 0-byte WAL must read back as empty");
    }

    #[test]
    fn test_wal_truncated_entry_stops_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let alice = Uuid::new_v4();
        let fact = make_fact(alice, ":name", Value::String("Alice".to_string()), 1);

        // Write a valid entry
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
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
        WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        assert!(path.exists());
        WalWriter::delete_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_wal_fact_size_limit() {
        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":test".to_string(),
            Value::String("x".to_string()),
            1,
        );

        // Write an entry with one fact
        writer.append_entry(1, &[fact]).unwrap();
        writer.file.sync_all().unwrap();

        // Manually corrupt the WAL to have a fact larger than MAX_FACT_SIZE
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(std::io::SeekFrom::End(-20)).unwrap();
        // Overwrite the fact length to be huge
        let huge_len: u32 = (10 * 1024 * 1024 + 1) as u32; // Just over MAX_FACT_SIZE
        file.write_all(&huge_len.to_le_bytes()).unwrap();

        drop(file);

        // Try to read - should fail gracefully
        let mut reader = WalReader::open(&path).unwrap();
        let result = reader.read_entries();
        assert!(
            result.is_err() || result.unwrap().is_empty(),
            "Should fail or return empty on corrupted large fact"
        );
    }

    // ── #360: WAL-0xx error code regression tests ──────────────────────────
    //
    // These assert the exact code carried by `MinigrafError::from(err)` for
    // each migrated call site, complementing `tests/error_codes_wal_test.rs`
    // (which exercises the same WAL-001/002/003 paths through the public
    // `Minigraf` API). WAL-004 and WAL-005 are documented as "practically
    // unreachable" (WAL-004 needs a ~4 GB single fact; WAL-005 can never
    // fail on a 64-bit target, where `usize` and `u64` are the same width)
    // and have no dedicated trigger test here for that reason — they are
    // still covered by `error::tests::registry_is_a_subset_of_error_reference_doc`
    // and `error::tests::every_error_code_variant_has_a_registry_entry`.

    #[test]
    fn test_wal_bad_magic_error_code_is_wal_001() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.wal");
        std::fs::write(&path, b"XXXX\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();

        let result = WalReader::open(&path);
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("bad magic should be rejected"),
        };
        let minigraf_err: crate::error::MinigrafError = err.into();
        assert_eq!(minigraf_err.code(), "WAL-001");
    }

    #[test]
    fn test_wal_bad_version_error_code_is_wal_002() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("badversion.wal");

        let mut header = [0u8; WAL_HEADER_SIZE];
        header[0..4].copy_from_slice(&WAL_MAGIC);
        header[4..8].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(&path, header).unwrap();

        let result = WalReader::open(&path);
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("bad version should be rejected"),
        };
        let minigraf_err: crate::error::MinigrafError = err.into();
        assert_eq!(minigraf_err.code(), "WAL-002");
    }

    #[test]
    fn test_wal_fact_size_limit_error_code_is_wal_003() {
        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized.wal");
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

        let entity = Uuid::new_v4();
        let huge_string = "a".repeat(MAX_FACT_BYTES + 1000);
        let fact = Fact::new(entity, ":test".to_string(), Value::String(huge_string), 1);

        let result = writer.append_entry(1, &[fact]);
        let err = result.expect_err("oversized fact should be rejected");
        let minigraf_err: crate::error::MinigrafError = err.into();
        assert_eq!(minigraf_err.code(), "WAL-003");
    }

    #[test]
    fn test_wal_delete_directory_error_code_is_wal_006() {
        let dir = tempfile::tempdir().unwrap();
        // A directory at the WAL path, rather than a file, makes
        // `std::fs::remove_file` fail regardless of permissions -- portable
        // across platforms without needing a permission-denial trick.
        let path = dir.path().join("not-a-file.wal");
        std::fs::create_dir(&path).unwrap();

        let result = WalWriter::delete_file(&path);
        let err = result.expect_err("deleting a directory should fail");
        let minigraf_err: crate::error::MinigrafError = err.into();
        assert_eq!(minigraf_err.code(), "WAL-006");

        std::fs::remove_dir(&path).unwrap();
    }

    // ── #389: parent-directory fsync ────────────────────────────────────────

    #[test]
    fn test_wal_create_syncs_parent_dir_after_create() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        take_sync_log();

        let _writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

        let log = take_sync_log();
        assert_eq!(
            log.len(),
            1,
            "creating the WAL must sync its directory once"
        );
        assert_eq!(
            log[0].dir,
            dir.path(),
            "must sync the WAL's parent directory"
        );
        assert!(
            log[0].child_existed,
            "directory sync must follow the create"
        );
    }

    #[test]
    fn test_wal_reopen_existing_does_not_sync_parent_dir() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        drop(WalWriter::open_or_create(&path, SyncMode::Full).unwrap());
        take_sync_log();

        let _writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

        assert!(
            take_sync_log().is_empty(),
            "reopening an intact WAL creates nothing, so needs no directory sync"
        );
    }

    #[test]
    fn test_wal_reinit_headerless_syncs_parent_dir() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        // A crash between `create_new` and the header write leaves a short
        // file whose directory entry may never have been synced either.
        std::fs::write(&path, b"MW").unwrap();
        take_sync_log();

        let _writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();

        let log = take_sync_log();
        assert_eq!(
            log.len(),
            1,
            "re-initializing the WAL must sync its directory"
        );
        assert!(
            log[0].child_existed,
            "directory sync must follow the header write"
        );
    }

    #[test]
    fn test_wal_delete_syncs_parent_dir_after_remove() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        drop(WalWriter::open_or_create(&path, SyncMode::Full).unwrap());
        take_sync_log();

        WalWriter::delete_file(&path).unwrap();

        let log = take_sync_log();
        assert_eq!(
            log.len(),
            1,
            "deleting the WAL must sync its directory once"
        );
        assert_eq!(
            log[0].dir,
            dir.path(),
            "must sync the WAL's parent directory"
        );
        assert!(
            !log[0].child_existed,
            "directory sync must follow the remove"
        );
    }

    #[test]
    fn test_wal_failed_delete_does_not_sync_parent_dir() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-file.wal");
        std::fs::create_dir(&path).unwrap();
        take_sync_log();

        assert!(WalWriter::delete_file(&path).is_err(), "delete must fail");
        assert!(
            take_sync_log().is_empty(),
            "nothing was removed, nothing to sync"
        );

        std::fs::remove_dir(&path).unwrap();
    }
}
