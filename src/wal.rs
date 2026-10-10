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
//!   magic:           [u8; 4]  = b"MWAL"
//!   version:         u32 LE   = 2
//!   base_generation: u64 LE   generation of the meta page that was active when
//!                             this WAL was created (v2; spec §4.1.1)
//!   reserved:        [u8; 16]
//!
//! A version 1 header (v2.x) has no base_generation. It can only sit next to a
//! file migrated from format v7 whose next checkpoint has not completed, so it
//! reads as base generation 1.
//!
//! WAL entries (variable length, sequential):
//!   checksum:  u32 LE  CRC32 of everything after this field in this entry
//!   tx_count:  u64 LE  monotonic counter from FactStorage
//!   num_facts: u64 LE
//!   facts:     for each fact: fact_len: u32 LE | fact_bytes: [u8; fact_len]
//! ```

use crate::db::SyncMode;
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::{Fact, Value};
use crate::storage::dir_sync::sync_parent_dir;
#[cfg(test)]
use crate::storage::fault::{self, Action, Site};
use crate::storage::keys::{MAX_IDENT_BYTES, MAX_VALUE_BYTES};
use anyhow::Result;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const WAL_MAGIC: [u8; 4] = *b"MWAL";
const WAL_VERSION: u32 = 2;
/// Format v7 (Minigraf v2.x) WAL version, read as base generation 1.
const WAL_VERSION_V1: u32 = 1;
const WAL_HEADER_SIZE: usize = 32;

// ─── WAL Header ─────────────────────────────────────────────────────────────

fn write_wal_header(file: &mut File, base_generation: u64) -> Result<()> {
    let mut buf = [0u8; WAL_HEADER_SIZE];
    buf[0..4].copy_from_slice(&WAL_MAGIC);
    buf[4..8].copy_from_slice(&WAL_VERSION.to_le_bytes());
    buf[8..16].copy_from_slice(&base_generation.to_le_bytes());
    // bytes 16..32 are reserved zeros
    file.seek(SeekFrom::Start(0))?;
    wal_write(file, &buf)?;
    wal_sync(file, true, 0)?;
    Ok(())
}

/// Write `bytes` at the file's position. Every WAL byte goes through here.
fn wal_write(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    match fault::on(Site::WalWrite) {
        Action::Fail(e) => return Err(e),
        Action::Tear(n) => {
            file.write_all(&bytes[..n.min(bytes.len())])?;
            return Err(fault::eio());
        }
        Action::Proceed | Action::Lose => {}
    }
    file.write_all(bytes)
}

/// Sync the WAL (`sync_all` when `all`, else `sync_data`). `synced_len` is
/// its length at the last good sync: a test fault that loses unsynced
/// writes cuts the file back to it.
#[cfg_attr(not(test), allow(unused_variables))]
fn wal_sync(file: &mut File, all: bool, synced_len: u64) -> io::Result<()> {
    #[cfg(test)]
    match fault::on(Site::WalSync) {
        Action::Fail(e) => return Err(e),
        Action::Lose => {
            file.set_len(synced_len)?;
            return Err(fault::eio());
        }
        Action::Proceed | Action::Tear(_) => {}
    }
    if all {
        file.sync_all()
    } else {
        file.sync_data()
    }
}

/// Check magic and version and return the header's base generation.
fn validate_wal_header(file: &mut File) -> Result<u64> {
    let mut buf = [0u8; WAL_HEADER_SIZE];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut buf)?;

    if buf[0..4] != WAL_MAGIC {
        bail_coded!(ErrorCode::Wal001);
    }
    let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    match version {
        WAL_VERSION => {
            let mut base = [0u8; 8];
            base.copy_from_slice(&buf[8..16]);
            Ok(u64::from_le_bytes(base))
        }
        WAL_VERSION_V1 => Ok(1),
        _ => bail_coded!(ErrorCode::Wal002, version, WAL_VERSION),
    }
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

/// Reject a fact a checkpoint could not store (WAL-003): a string value longer
/// than one value page's payload, or an attribute or keyword longer than an
/// ident may be (spec §6.3).
pub(crate) fn check_fact_size(fact: &Fact) -> Result<()> {
    if fact.attribute.len() > MAX_IDENT_BYTES {
        bail_coded!(ErrorCode::Wal003, fact.attribute.len(), MAX_IDENT_BYTES);
    }
    match &fact.value {
        Value::String(s) if s.len() > MAX_VALUE_BYTES => {
            bail_coded!(ErrorCode::Wal003, s.len(), MAX_VALUE_BYTES)
        }
        Value::Keyword(k) if k.len() > MAX_IDENT_BYTES => {
            bail_coded!(ErrorCode::Wal003, k.len(), MAX_IDENT_BYTES)
        }
        _ => Ok(()),
    }
}

fn serialize_entry(tx_count: u64, facts: &[Fact]) -> Result<Vec<u8>> {
    // Build payload (everything covered by the checksum)
    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(&tx_count.to_le_bytes());
    payload.extend_from_slice(&(facts.len() as u64).to_le_bytes());
    for fact in facts {
        check_fact_size(fact)?;
        let fact_bytes = postcard::to_allocvec(fact)?;
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
    /// An append failed, so the file may end in a partial entry. Later
    /// appends would land after it, where replay never reaches (#513).
    failed: bool,
    /// The file's length after the last successful append.
    end: u64,
}

impl WalWriter {
    /// [`WalWriter::open_or_create_at`] with base generation 1, for tests.
    #[cfg(test)]
    pub fn open_or_create(path: &Path, sync_mode: SyncMode) -> Result<Self> {
        Self::open_or_create_at(path, sync_mode, 1)
    }

    /// Open an existing WAL or create a new one.
    ///
    /// If creating, writes the WAL header with `base_generation`, the generation
    /// of the meta page active now. If opening, validates the header (keeping its
    /// base generation) and seeks to the end for appending.
    pub fn open_or_create_at(
        path: &Path,
        sync_mode: SyncMode,
        base_generation: u64,
    ) -> Result<Self> {
        // Try atomic create-new first (no TOCTOU window)
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                write_wal_header(&mut file, base_generation)?;
                // The header is durable; make the WAL's directory entry
                // durable too, or a power loss can lose the whole file (#389).
                sync_parent_dir(path)?;
                let end = file.seek(SeekFrom::End(0))?;
                return Ok(WalWriter {
                    file,
                    sync_mode,
                    failed: false,
                    end,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }

        // File exists — validate its header and seek to end for appending
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        match check_wal_header_length(&mut file)? {
            WalHeaderState::Present => {
                validate_wal_header(&mut file)?;
                // A crash during an append can leave a partial entry at the
                // end. Replay stops there, so an entry appended after it
                // would be lost: cut it off before appending (#513).
                let (_, valid_end) = scan_entries(&mut file)?;
                if file.metadata()?.len() > valid_end {
                    #[cfg(test)]
                    if let Action::Fail(e) = fault::on(Site::WalTruncate) {
                        return Err(e.into());
                    }
                    file.set_len(valid_end)?;
                    // A lost sync here would bring the cut bytes back,
                    // which the test fault cannot model: it fails instead.
                    wal_sync(&mut file, true, valid_end)?;
                }
            }
            // A previous crash landed between this file's creation and its
            // header write completing; no entry could have been appended
            // yet, so re-initialize it as a fresh, empty WAL.
            // The crash may also have preceded the directory sync that
            // follows creation, so repeat it (#389).
            WalHeaderState::Absent => {
                write_wal_header(&mut file, base_generation)?;
                sync_parent_dir(path)?;
            }
        }
        let end = file.seek(SeekFrom::End(0))?;
        Ok(WalWriter {
            file,
            sync_mode,
            failed: false,
            end,
        })
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
    ///
    /// After a failed write or sync, every later append fails with WAL-007:
    /// the file may end in a partial entry, and replay stops there (#513). A
    /// new writer (after a checkpoint deletes the WAL, or on reopen) starts
    /// clean. Whether the failed entry itself is durable is unknown: it is
    /// replayed on reopen if it reached the disk whole, and lost otherwise.
    pub fn append_entry(&mut self, tx_count: u64, facts: &[Fact]) -> Result<()> {
        if self.failed {
            bail_coded!(ErrorCode::Wal007);
        }
        let entry_bytes = serialize_entry(tx_count, facts)?;
        let written = self.write_entry(&entry_bytes);
        if written.is_err() {
            self.failed = true;
        }
        written
    }

    fn write_entry(&mut self, entry_bytes: &[u8]) -> Result<()> {
        wal_write(&mut self.file, entry_bytes)?;
        match self.sync_mode {
            SyncMode::Full => wal_sync(&mut self.file, false, self.end)?,
            SyncMode::Normal => {}
        }
        self.end = self.end.saturating_add(entry_bytes.len() as u64);
        Ok(())
    }

    /// Whether an append failed, so every later one is refused (WAL-007).
    pub(crate) fn failed(&self) -> bool {
        self.failed
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
            #[cfg(test)]
            if let Action::Fail(e) = fault::on(Site::WalRemove) {
                last_err = Some(e);
                break;
            }
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
    base_generation: Option<u64>,
}

impl WalReader {
    /// Open the WAL at `path` for reading.
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let base_generation = match check_wal_header_length(&mut file)? {
            WalHeaderState::Present => Some(validate_wal_header(&mut file)?),
            // Too short to contain a header, which can only mean a crash
            // landed before any entry was ever appended (see
            // `WalHeaderState::Absent`). `read_entries` already treats
            // hitting EOF while reading an entry as "no more entries", so
            // leaving the file as-is and letting that seek-past-end happen
            // naturally is correct -- there is nothing here to replay.
            WalHeaderState::Absent => None,
        };
        Ok(WalReader {
            file,
            base_generation,
        })
    }

    /// The generation of the meta page active when this WAL was created, or
    /// `None` for a header-less WAL (a crash before its header was written, so
    /// it holds no entries and counts as no WAL).
    pub fn base_generation(&self) -> Option<u64> {
        self.base_generation
    }

    /// Read all valid entries from the WAL.
    ///
    /// Reads sequentially from after the header. Stops at the first entry
    /// with an invalid CRC32 (partial write) or at EOF. Earlier entries are
    /// unaffected by a bad entry.
    pub fn read_entries(&mut self) -> Result<Vec<WalEntry>> {
        Ok(scan_entries(&mut self.file)?.0)
    }
}

/// Read every valid entry after the header, stopping at the first truncated
/// or corrupt one or at EOF. Also returns the offset where the valid entries
/// end: the end of the header if there are none.
fn scan_entries(file: &mut File) -> Result<(Vec<WalEntry>, u64)> {
    file.seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))?;
    let mut entries = Vec::new();
    let mut valid_end = WAL_HEADER_SIZE as u64;

    loop {
        // Read checksum (4 bytes); EOF here means no more entries
        let mut csum_buf = [0u8; 4];
        match file.read_exact(&mut csum_buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
            Ok(()) => {}
        }
        let expected_csum = u32::from_le_bytes(csum_buf);

        // Read tx_count (8 bytes)
        let mut tx_count_buf = [0u8; 8];
        if file.read_exact(&mut tx_count_buf).is_err() {
            break; // truncated
        }
        let tx_count = u64::from_le_bytes(tx_count_buf);

        // Read num_facts (8 bytes)
        let mut num_facts_buf = [0u8; 8];
        if file.read_exact(&mut num_facts_buf).is_err() {
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
            if file.read_exact(&mut len_buf).is_err() {
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
            if file.read_exact(&mut fact_bytes).is_err() {
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
        valid_end = file.stream_position()?;
    }

    Ok((entries, valid_end))
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

    // ── #513: torn tail and failed appends ─────────────────────────────────

    /// The bytes of a WAL holding entries `1..=n`.
    fn wal_with_entries(path: &Path, n: u64) -> Vec<u8> {
        let mut writer = WalWriter::open_or_create(path, SyncMode::Full).unwrap();
        for tx in 1..=n {
            let fact = make_fact(Uuid::new_v4(), ":name", Value::Integer(tx as i64), tx);
            writer.append_entry(tx, &[fact]).unwrap();
        }
        drop(writer);
        std::fs::read(path).unwrap()
    }

    fn tx_counts(path: &Path) -> Vec<u64> {
        let mut reader = WalReader::open(path).unwrap();
        let entries = reader.read_entries().unwrap();
        entries.iter().map(|e| e.tx_count).collect()
    }

    #[test]
    fn test_wal_writer_cuts_off_torn_tail_before_appending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        let two = wal_with_entries(&path, 2);
        let one_len = {
            let p = dir.path().join("one.wal");
            wal_with_entries(&p, 1).len()
        };
        let second = two.len() - one_len;
        for cut in [1, 4, 12, 20, second - 1] {
            std::fs::write(&path, &two[..one_len + cut]).unwrap();
            let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().len(),
                one_len as u64,
                "the partial entry is cut off on open"
            );
            let fact = make_fact(Uuid::new_v4(), ":name", Value::Integer(7), 7);
            writer.append_entry(7, &[fact]).unwrap();
            drop(writer);
            assert_eq!(tx_counts(&path), [1, 7], "the new entry is reachable");
        }
    }

    #[test]
    fn test_wal_writer_cuts_off_entry_with_bad_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        let mut bytes = wal_with_entries(&path, 3);
        let one_len = {
            let p = dir.path().join("one.wal");
            wal_with_entries(&p, 1).len()
        };
        // Damage entry 2; entry 3 after it is unreachable and goes too.
        bytes[one_len] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        drop(writer);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), one_len as u64);
        assert_eq!(tx_counts(&path), [1]);
    }

    #[test]
    fn test_wal_writer_keeps_an_intact_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        let bytes = wal_with_entries(&path, 3);
        let writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        drop(writer);
        assert!(std::fs::read(&path).unwrap() == bytes, "nothing to cut");
    }

    #[test]
    fn test_wal_failed_append_refuses_later_appends_with_wal_007() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        let fact = |tx: u64| make_fact(Uuid::new_v4(), ":name", Value::Integer(1), tx);
        writer.append_entry(1, &[fact(1)]).unwrap();

        fault::arm(fault::Fault::Torn(10), 0);
        writer
            .append_entry(2, &[fact(2)])
            .expect_err("the torn append fails");
        assert_eq!(fault::disarm(), Some(Site::WalWrite));
        let err: crate::error::MinigrafError = writer
            .append_entry(3, &[fact(3)])
            .expect_err("later appends are refused")
            .into();
        assert_eq!(err.code(), "WAL-007");
        drop(writer);
        assert_eq!(tx_counts(&path), [1], "nothing after the torn entry");

        // A new writer cuts off the partial entry and appends normally.
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        writer.append_entry(4, &[fact(4)]).unwrap();
        drop(writer);
        assert_eq!(tx_counts(&path), [1, 4]);
    }

    #[test]
    fn test_wal_rejected_entry_does_not_poison_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");
        let mut writer = WalWriter::open_or_create(&path, SyncMode::Full).unwrap();
        let huge = make_fact(
            Uuid::new_v4(),
            ":name",
            Value::String("x".repeat(MAX_VALUE_BYTES + 1)),
            1,
        );
        writer
            .append_entry(1, &[huge])
            .expect_err("WAL-003, before anything is written");
        let fact = make_fact(Uuid::new_v4(), ":name", Value::Integer(1), 2);
        writer.append_entry(2, &[fact]).unwrap();
        drop(writer);
        assert_eq!(tx_counts(&path), [2]);
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
        let huge_string = "a".repeat(MAX_VALUE_BYTES + 1);
        let fact = Fact::new(entity, ":test".to_string(), Value::String(huge_string), 1);

        let result = writer.append_entry(1, &[fact]);
        let err = result.expect_err("oversized fact should be rejected");
        let minigraf_err: crate::error::MinigrafError = err.into();
        assert_eq!(minigraf_err.code(), "WAL-003");
    }

    /// Attributes and keyword values are capped at 1024 bytes (WAL-003), so a
    /// DICT entry always fits in a node; 1024 itself is accepted.
    #[test]
    fn test_wal_ident_size_limit_is_wal_003() {
        use crate::graph::types::{Fact, Value};
        use uuid::Uuid;

        let dir = tempfile::tempdir().unwrap();
        let mut writer =
            WalWriter::open_or_create(&dir.path().join("ident.wal"), SyncMode::Full).unwrap();
        let e = Uuid::new_v4();
        let at = |n: usize| format!(":{}", "a".repeat(n - 1));
        let ok = Fact::new(
            e,
            at(MAX_IDENT_BYTES),
            Value::Keyword(at(MAX_IDENT_BYTES)),
            1,
        );
        writer.append_entry(1, &[ok]).unwrap();
        let long_attr = Fact::new(e, at(MAX_IDENT_BYTES + 1), Value::Integer(1), 2);
        let long_kw = Fact::new(e, ":a".into(), Value::Keyword(at(MAX_IDENT_BYTES + 1)), 2);
        for f in [long_attr, long_kw] {
            let err = writer.append_entry(2, &[f]).unwrap_err();
            assert_eq!(crate::error::MinigrafError::from(err).code(), "WAL-003");
        }
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

    #[test]
    fn test_wal_v2_header_records_base_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("base.wal");
        drop(WalWriter::open_or_create_at(&path, SyncMode::Full, 42).unwrap());
        assert_eq!(WalReader::open(&path).unwrap().base_generation(), Some(42));
        // Reopening for append keeps the original base generation.
        drop(WalWriter::open_or_create_at(&path, SyncMode::Full, 99).unwrap());
        assert_eq!(WalReader::open(&path).unwrap().base_generation(), Some(42));
    }

    #[test]
    fn test_wal_v1_header_reads_as_base_generation_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.wal");
        let mut header = [0u8; WAL_HEADER_SIZE];
        header[0..4].copy_from_slice(&WAL_MAGIC);
        header[4..8].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&path, header).unwrap();
        assert_eq!(WalReader::open(&path).unwrap().base_generation(), Some(1));
        // A v2.x WAL can still be appended to.
        let mut w = WalWriter::open_or_create_at(&path, SyncMode::Full, 5).unwrap();
        w.append_entry(1, &[]).unwrap();
        drop(w);
        let mut r = WalReader::open(&path).unwrap();
        assert_eq!(r.read_entries().unwrap().len(), 1);
    }

    #[test]
    fn test_wal_headerless_has_no_base_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.wal");
        std::fs::write(&path, [0u8; 5]).unwrap();
        assert_eq!(WalReader::open(&path).unwrap().base_generation(), None);
    }
}
