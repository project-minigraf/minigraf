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
use anyhow::{Result, bail};
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
        bail!("Invalid WAL magic number: not a .wal file");
    }
    let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if version != WAL_VERSION {
        bail!(
            "Unsupported WAL version: {} (expected {})",
            version,
            WAL_VERSION
        );
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
        // Try atomic create-new first (no TOCTOU window)
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                write_wal_header(&mut file)?;
                file.seek(SeekFrom::End(0))?;
                return Ok(WalWriter { file });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }

        // File exists — validate its header and seek to end for appending
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        validate_wal_header(&mut file)?;
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
    ///
    /// On some filesystems, deleting a recently-written file may not be durable
    /// until a sync occurs. We open the file and sync it before deletion to ensure
    /// durability.
    pub fn delete_file(path: &Path) -> Result<()> {
        let file = std::fs::File::open(path)?;
        file.sync_all()?;
        drop(file);
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

#[cfg(test)]
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

        let mut writer = WalWriter::open_or_create(&path).unwrap();
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
        let mut writer = WalWriter::open_or_create(&path).unwrap();
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
        let mut writer = WalWriter::open_or_create(&path).unwrap();
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

    #[test]
    fn test_wal_fact_size_limit() {
        use crate::graph::Fact;
        use crate::graph::types::Value;
        use uuid::Uuid;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut writer = WalWriter::open_or_create(&path).unwrap();

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
}
