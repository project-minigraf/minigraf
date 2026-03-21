/// Storage backend abstraction for cross-platform support.
///
/// This module provides a trait-based abstraction over different storage backends,
/// allowing Minigraf to run on native platforms (file-based), WASM (IndexedDB),
/// and in-memory (testing/embedded).
///
/// Inspired by SQLite's VFS (Virtual File System) architecture.
pub mod backend;
pub mod index;
pub mod persistent_facts;

use anyhow::Result;

/// Page size for the storage engine (4KB like SQLite)
pub const PAGE_SIZE: usize = 4096;

/// Magic number for .graph files: "MGRF" (Minigraf)
pub const MAGIC_NUMBER: [u8; 4] = *b"MGRF";

/// Current file format version
pub const FORMAT_VERSION: u32 = 4;

/// Storage backend trait.
///
/// Implementations provide platform-specific storage mechanisms:
/// - FileBackend: Native filesystem (Linux, macOS, Windows, iOS, Android)
/// - IndexedDbBackend: Browser storage (WASM)
/// - MemoryBackend: In-memory storage (testing, embedded)
pub trait StorageBackend: Send + Sync {
    /// Write a page of data at the given page ID.
    ///
    /// Page IDs start at 0. Page 0 is reserved for the file header.
    /// Data must be exactly PAGE_SIZE bytes.
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()>;

    /// Read a page of data at the given page ID.
    ///
    /// Returns exactly PAGE_SIZE bytes.
    fn read_page(&self, page_id: u64) -> Result<Vec<u8>>;

    /// Sync all pending writes to stable storage.
    ///
    /// Ensures durability - data is persisted even after crash.
    fn sync(&mut self) -> Result<()>;

    /// Get the total number of pages in the storage.
    fn page_count(&self) -> Result<u64>;

    /// Close the storage backend.
    ///
    /// Performs final sync and cleanup.
    fn close(&mut self) -> Result<()>;

    /// Get a human-readable name for this backend (for debugging).
    fn backend_name(&self) -> &'static str;
}

/// File header for .graph files — 72 bytes in v4.
///
/// Layout (all fields little-endian):
///   0..4    magic ("MGRF")
///   4..8    version (u32)
///   8..16   page_count (u64)
///   16..24  node_count (u64)          — reused as fact count
///   24..32  last_checkpointed_tx_count (u64)
///   32..40  eavt_root_page (u64)      — new in v4
///   40..48  aevt_root_page (u64)      — new in v4
///   48..56  avet_root_page (u64)      — new in v4
///   56..64  vaet_root_page (u64)      — new in v4
///   64..68  index_checksum (u32)      — new in v4
///   68..72  _padding (u32)
#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub page_count: u64,
    pub node_count: u64,
    pub last_checkpointed_tx_count: u64,
    pub eavt_root_page: u64,
    pub aevt_root_page: u64,
    pub avet_root_page: u64,
    pub vaet_root_page: u64,
    pub index_checksum: u32,
    pub(crate) _padding: u32,
}

impl FileHeader {
    /// Create a new file header with default values.
    pub fn new() -> Self {
        FileHeader {
            magic: MAGIC_NUMBER,
            version: FORMAT_VERSION,
            page_count: 1, // Just the header page initially
            node_count: 0,
            last_checkpointed_tx_count: 0,
            eavt_root_page: 0,
            aevt_root_page: 0,
            avet_root_page: 0,
            vaet_root_page: 0,
            index_checksum: 0,
            _padding: 0,
        }
    }

    /// Serialize the header to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(72);
        b.extend_from_slice(&self.magic);
        b.extend_from_slice(&self.version.to_le_bytes());
        b.extend_from_slice(&self.page_count.to_le_bytes());
        b.extend_from_slice(&self.node_count.to_le_bytes());
        b.extend_from_slice(&self.last_checkpointed_tx_count.to_le_bytes());
        b.extend_from_slice(&self.eavt_root_page.to_le_bytes());
        b.extend_from_slice(&self.aevt_root_page.to_le_bytes());
        b.extend_from_slice(&self.avet_root_page.to_le_bytes());
        b.extend_from_slice(&self.vaet_root_page.to_le_bytes());
        b.extend_from_slice(&self.index_checksum.to_le_bytes());
        b.extend_from_slice(&self._padding.to_le_bytes());
        b
    }

    /// Deserialize the header from bytes.
    ///
    /// Accepts both v3 (64-byte) and v4 (72-byte) headers.
    /// v3 headers are returned with zero-filled index fields; the
    /// v3→v4 migration in persistent_facts.rs upgrades them on next save.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        // Both v3 (64 bytes) and v4 (72 bytes) must pass at least 64-byte
        // validation before the version field is read.
        if bytes.len() < 64 {
            anyhow::bail!("Invalid header: too short (got {} bytes, need 64)", bytes.len());
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);

        if magic != MAGIC_NUMBER {
            anyhow::bail!("Invalid magic number: not a .graph file");
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let page_count = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let node_count = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let last_checkpointed_tx_count = u64::from_le_bytes(bytes[24..32].try_into().unwrap());

        // v3 and earlier: no index fields — return with zero-filled index fields.
        // The v3→v4 migration in persistent_facts.rs will upgrade on next save.
        if version <= 3 {
            return Ok(FileHeader {
                magic,
                version,
                page_count,
                node_count,
                last_checkpointed_tx_count,
                eavt_root_page: 0,
                aevt_root_page: 0,
                avet_root_page: 0,
                vaet_root_page: 0,
                index_checksum: 0,
                _padding: 0,
            });
        }

        // v4: need full 72 bytes.
        if bytes.len() < 72 {
            anyhow::bail!("Invalid v4 header: expected 72 bytes, got {}", bytes.len());
        }

        Ok(FileHeader {
            magic,
            version,
            page_count,
            node_count,
            last_checkpointed_tx_count,
            eavt_root_page: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            aevt_root_page: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            avet_root_page: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            vaet_root_page: u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
            index_checksum: u32::from_le_bytes(bytes[64..68].try_into().unwrap()),
            _padding: u32::from_le_bytes(bytes[68..72].try_into().unwrap()),
        })
    }

    /// Validate the header.
    pub fn validate(&self) -> Result<()> {
        if self.magic != MAGIC_NUMBER {
            anyhow::bail!("Invalid magic number");
        }
        if self.version < 1 || self.version > FORMAT_VERSION {
            anyhow::bail!(
                "Unsupported format version: {} (supported: 1-{})",
                self.version,
                FORMAT_VERSION
            );
        }
        Ok(())
    }
}

impl Default for FileHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_header_serialization_v4() {
        let header = FileHeader::new();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 72);
        assert_eq!(&bytes[0..4], b"MGRF");
        // version field at bytes 4..8
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 4);
        // eavt_root_page at bytes 32..40: zero on fresh header
        assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), 0);
        // index_checksum at bytes 64..68: zero on fresh header
        assert_eq!(u32::from_le_bytes(bytes[64..68].try_into().unwrap()), 0);
    }

    #[test]
    fn test_file_header_roundtrip_v4() {
        let mut header = FileHeader::new();
        header.eavt_root_page = 10;
        header.aevt_root_page = 20;
        header.avet_root_page = 30;
        header.vaet_root_page = 40;
        header.index_checksum = 0xDEAD_BEEF;
        let bytes = header.to_bytes();
        let parsed = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.eavt_root_page, 10);
        assert_eq!(parsed.aevt_root_page, 20);
        assert_eq!(parsed.avet_root_page, 30);
        assert_eq!(parsed.vaet_root_page, 40);
        assert_eq!(parsed.index_checksum, 0xDEAD_BEEF);
    }

    #[test]
    fn test_file_header_from_bytes_v3_accepted() {
        // v3 files have a 64-byte header. from_bytes must accept them and
        // return zeroed index fields.
        let mut bytes = vec![0u8; 64];
        bytes[0..4].copy_from_slice(b"MGRF");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes()); // version = 3
        bytes[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
        let header = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(header.version, 3);
        assert_eq!(header.eavt_root_page, 0);
        assert_eq!(header.index_checksum, 0);
    }

    #[test]
    fn test_file_header_validation() {
        let header = FileHeader::new();
        assert!(header.validate().is_ok());

        let mut invalid = header;
        invalid.magic = *b"XXXX";
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_format_version_is_4() {
        assert_eq!(FORMAT_VERSION, 4);
    }

    #[test]
    fn test_validate_accepts_versions_1_to_4() {
        let mut header = FileHeader::new();
        for v in 1u32..=4 {
            header.version = v;
            assert!(header.validate().is_ok(), "version {} should be accepted", v);
        }
    }

    #[test]
    fn test_validate_rejects_version_0_and_5() {
        let mut header = FileHeader::new();
        header.version = 0;
        assert!(header.validate().is_err());

        header.version = 5;
        assert!(header.validate().is_err());
    }

    #[test]
    fn test_new_header_has_version_4() {
        let header = FileHeader::new();
        assert_eq!(header.version, FORMAT_VERSION);
        assert_eq!(header.version, 4);
    }

    #[test]
    fn test_file_header_from_bytes_truncated_v4_rejected() {
        // A header that claims version=4 but has fewer than 72 bytes must be rejected.
        let mut bytes = vec![0u8; 68]; // only 68 bytes, not 72
        bytes[0..4].copy_from_slice(b"MGRF");
        bytes[4..8].copy_from_slice(&4u32.to_le_bytes()); // version = 4
        let result = FileHeader::from_bytes(&bytes);
        assert!(result.is_err(), "truncated v4 header must be rejected");
    }
}
