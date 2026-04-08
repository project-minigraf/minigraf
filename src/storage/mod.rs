/// Storage backend abstraction for cross-platform support.
///
/// This module provides a trait-based abstraction over different storage backends,
/// allowing Minigraf to run on native platforms (file-based), WASM (IndexedDB),
/// and in-memory (testing/embedded).
///
/// Inspired by SQLite's VFS (Virtual File System) architecture.
pub mod backend;
pub mod btree;
pub mod btree_v6;
pub mod cache;
pub mod index;
pub mod packed_pages;
pub mod persistent_facts;

use anyhow::Result;

/// Page size for the storage engine (4KB like SQLite)
pub const PAGE_SIZE: usize = 4096;

/// Magic number for .graph files: "MGRF" (Minigraf)
pub const MAGIC_NUMBER: [u8; 4] = *b"MGRF";

/// Current file format version
pub const FORMAT_VERSION: u32 = 7;

/// fact_page_format: legacy one-per-page (v4 and earlier, or unset byte = 0x00).
pub const FACT_PAGE_FORMAT_ONE_PER_PAGE: u8 = 0x01;
/// fact_page_format: packed pages (v5+).
pub const FACT_PAGE_FORMAT_PACKED: u8 = 0x02;

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
    #[allow(dead_code)]
    fn close(&mut self) -> Result<()>;

    /// Get a human-readable name for this backend (for debugging).
    #[allow(dead_code)]
    fn backend_name(&self) -> &'static str;

    /// Returns true if this is a newly created empty storage.
    ///
    /// This is used to determine whether a header read failure should
    /// create a fresh header (new storage) or return an error (corrupted existing storage).
    fn is_new(&self) -> bool;
}

/// File header for .graph files — 84 bytes in v7.
///
/// Layout (all fields little-endian):
///   0..4    magic ("MGRF")
///   4..8    version (u32)
///   8..16   page_count (u64)
///   16..24  node_count (u64)          — reused as fact count
///   24..32  last_checkpointed_tx_count (u64)
///   32..40  eavt_root_page (u64)
///   40..48  aevt_root_page (u64)
///   48..56  avet_root_page (u64)
///   56..64  vaet_root_page (u64)
///   64..68  index_checksum (u32)
///   68      fact_page_format (u8)
///   69..72  _padding ([u8; 3])
///   72..80  fact_page_count (u64)     — new in v6
///   80..84  header_checksum (u32)    — new in v7
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
    /// fact_page_format (v5+): 0x00 = unset/legacy, 0x01 = one-per-page, 0x02 = packed.
    pub fact_page_format: u8,
    pub(crate) _padding: [u8; 3],
    /// Number of pages (starting at page 1) holding committed fact data.
    /// New in v6; zero-initialised when reading v5 or older headers.
    pub fact_page_count: u64,
    /// CRC32 checksum of the first 80 bytes of the header (excluding this field).
    /// New in v7; zero-initialised when reading v6 or older headers.
    pub header_checksum: u32,
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
            fact_page_format: FACT_PAGE_FORMAT_PACKED,
            _padding: [0; 3],
            fact_page_count: 0,
            header_checksum: 0,
        }
    }

    /// Serialize the header to bytes.
    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = Vec::with_capacity(84);
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
        b.push(self.fact_page_format);
        b.extend_from_slice(&self._padding);
        b.extend_from_slice(&self.fact_page_count.to_le_bytes());
        b.extend_from_slice(&self.header_checksum.to_le_bytes());
        b
    }

    /// Deserialize the header from bytes.
    ///
    /// Accepts v3 (64-byte), v4/v5 (72-byte), and v6 (80-byte) headers.
    /// v3 headers are returned with zero-filled index fields; the
    /// v3→v4 migration in persistent_facts.rs upgrades them on next save.
    /// v4 headers have fact_page_format = 0x00 (legacy/unset).
    /// v5 headers are returned with fact_page_count = 0.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        // Both v3 (64 bytes) and v4/v5/v6 (72+ bytes) must pass at least 64-byte
        // validation before the version field is read.
        if bytes.len() < 64 {
            anyhow::bail!(
                "Invalid header: too short (got {} bytes, need 64)",
                bytes.len()
            );
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
                fact_page_format: 0,
                _padding: [0; 3],
                fact_page_count: 0,
                header_checksum: 0,
            });
        }

        // v4, v5, v6: need at least 72 bytes
        if bytes.len() < 72 {
            anyhow::bail!(
                "Invalid v4/v5/v6 header: expected at least 72 bytes, got {}",
                bytes.len()
            );
        }

        let fact_page_count = if version >= 6 {
            if bytes.len() < 80 {
                anyhow::bail!("Invalid v6 header: expected 80 bytes, got {}", bytes.len());
            }
            u64::from_le_bytes(bytes[72..80].try_into().unwrap())
        } else {
            0
        };

        let header_checksum = if version >= 7 {
            if bytes.len() < 84 {
                anyhow::bail!("Invalid v7 header: expected 84 bytes, got {}", bytes.len());
            }
            u32::from_le_bytes(bytes[80..84].try_into().unwrap())
        } else {
            0
        };

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
            fact_page_format: bytes[68],
            _padding: [bytes[69], bytes[70], bytes[71]],
            fact_page_count,
            header_checksum,
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
        // Validate logical relationships
        if self.page_count == 0 {
            anyhow::bail!("page_count must be greater than 0");
        }
        if self.eavt_root_page != 0 && self.eavt_root_page >= self.page_count {
            anyhow::bail!(
                "eavt_root_page ({}) must be less than page_count ({})",
                self.eavt_root_page,
                self.page_count
            );
        }
        if self.fact_page_count > self.page_count {
            anyhow::bail!(
                "fact_page_count ({}) cannot exceed page_count ({})",
                self.fact_page_count,
                self.page_count
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

/// Reads committed (checkpointed) facts from persistent storage.
///
/// Implemented by `CommittedFactLoaderImpl` in `persistent_facts.rs` and set on
/// `FactStorage` after load, so index-driven reads resolve `FactRef`s to `Fact`
/// objects via the page cache without keeping the entire fact list in memory.
pub trait CommittedFactReader: Send + Sync {
    /// Resolve a single committed fact by its disk reference.
    #[allow(dead_code)]
    fn resolve(
        &self,
        fact_ref: crate::storage::index::FactRef,
    ) -> Result<crate::graph::types::Fact>;
    /// Stream all committed facts (for full scans, checksum verification, migration).
    fn stream_all(&self) -> Result<Vec<crate::graph::types::Fact>>;
    /// Number of committed fact pages (used for checksum + iteration bounds).
    #[allow(dead_code)]
    fn committed_page_count(&self) -> u64;
}

/// Provides bounded range scans over the four committed (on-disk) covering indexes.
///
/// Implemented by `OnDiskIndexReader` in `btree_v6.rs`. Set on `FactStorage`
/// after load/migration/checkpoint so query methods can merge committed and
/// pending index entries without loading the full index into RAM.
pub trait CommittedIndexReader: Send + Sync {
    /// Returns all committed EAVT entries in `[start, end)`. `end: None` means unbounded upper.
    #[allow(dead_code)]
    fn range_scan_eavt(
        &self,
        start: &crate::storage::index::EavtKey,
        end: Option<&crate::storage::index::EavtKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>>;

    /// Returns all committed AEVT entries in `[start, end)`. `end: None` means unbounded upper.
    #[allow(dead_code)]
    fn range_scan_aevt(
        &self,
        start: &crate::storage::index::AevtKey,
        end: Option<&crate::storage::index::AevtKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>>;

    /// Returns all committed AVET entries in `[start, end)`. `end: None` means unbounded upper.
    #[allow(dead_code)]
    fn range_scan_avet(
        &self,
        start: &crate::storage::index::AvetKey,
        end: Option<&crate::storage::index::AvetKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>>;

    /// Returns all committed VAET entries in `[start, end)`. `end: None` means unbounded upper.
    #[allow(dead_code)]
    fn range_scan_vaet(
        &self,
        start: &crate::storage::index::VaetKey,
        end: Option<&crate::storage::index::VaetKey>,
    ) -> anyhow::Result<Vec<crate::storage::index::FactRef>>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_format_version_is_7() {
        assert_eq!(FORMAT_VERSION, 7);
    }

    #[test]
    fn test_validate_accepts_version_7() {
        let mut h = FileHeader::new();
        h.version = 7;
        assert!(h.validate().is_ok());
    }

    #[test]
    fn test_validate_page_count_must_be_positive() {
        let mut h = FileHeader::new();
        h.page_count = 0;
        let result = h.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("page_count"));
    }

    #[test]
    fn test_validate_eavt_root_page_bounds() {
        let mut h = FileHeader::new();
        h.page_count = 10;
        h.eavt_root_page = 10; // equal to page_count, should fail
        let result = h.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("eavt_root_page"));

        // Should pass when 0
        h.eavt_root_page = 0;
        assert!(h.validate().is_ok());

        // Should pass when valid
        h.eavt_root_page = 5;
        assert!(h.validate().is_ok());
    }

    #[test]
    fn test_validate_fact_page_count_bounds() {
        let mut h = FileHeader::new();
        h.page_count = 10;
        h.fact_page_count = 11; // exceeds page_count
        let result = h.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("fact_page_count"));

        // Should pass when within bounds
        h.fact_page_count = 5;
        assert!(h.validate().is_ok());
    }

    #[test]
    fn test_new_header_has_version_7() {
        let header = FileHeader::new();
        assert_eq!(header.version, FORMAT_VERSION);
        assert_eq!(header.version, 7);
    }

    #[test]
    fn test_file_header_serialization_v7() {
        let header = FileHeader::new();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 84);
    }

    #[test]
    fn test_file_header_roundtrip_v7() {
        let mut header = FileHeader::new();
        header.header_checksum = 0xDEAD_BEEF;
        let bytes = header.to_bytes();
        let parsed = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.header_checksum, 0xDEAD_BEEF);
    }

    #[test]
    fn test_file_header_v7_byte_layout_all_fields() {
        let mut h = FileHeader::new();
        h.page_count = 0x0102_0304_0506_0708_u64;
        h.node_count = 0x1112_1314_1516_1718_u64;
        h.last_checkpointed_tx_count = 0x2122_2324_2526_2728_u64;
        h.eavt_root_page = 0x3132_3334_3536_3738_u64;
        h.aevt_root_page = 0x4142_4344_4546_4748_u64;
        h.avet_root_page = 0x5152_5354_5556_5758_u64;
        h.vaet_root_page = 0x6162_6364_6566_6768_u64;
        h.index_checksum = 0x7172_7374_u32;
        h.fact_page_format = 0x02;
        h._padding = [0x00; 3];
        h.fact_page_count = 0xA1A2_A3A4_A5A6_A7A8_u64;
        h.header_checksum = 0xC1C2_C3C4_u32;

        let b = h.to_bytes();
        assert_eq!(b.len(), 84, "v7 header must be exactly 84 bytes");

        assert_eq!(&b[0..4], b"MGRF");
        assert_eq!(&b[4..8], &7u32.to_le_bytes());
        assert_eq!(&b[8..16], &0x0102_0304_0506_0708_u64.to_le_bytes());
        assert_eq!(&b[16..24], &0x1112_1314_1516_1718_u64.to_le_bytes());
        assert_eq!(&b[24..32], &0x2122_2324_2526_2728_u64.to_le_bytes());
        assert_eq!(&b[32..40], &0x3132_3334_3536_3738_u64.to_le_bytes());
        assert_eq!(&b[40..48], &0x4142_4344_4546_4748_u64.to_le_bytes());
        assert_eq!(&b[48..56], &0x5152_5354_5556_5758_u64.to_le_bytes());
        assert_eq!(&b[56..64], &0x6162_6364_6566_6768_u64.to_le_bytes());
        assert_eq!(&b[64..68], &0x7172_7374_u32.to_le_bytes());
        assert_eq!(b[68], 0x02);
        assert_eq!(&b[69..72], &[0x00u8; 3]);
        assert_eq!(&b[72..80], &0xA1A2_A3A4_A5A6_A7A8_u64.to_le_bytes());
        assert_eq!(&b[80..84], &0xC1C2_C3C4_u32.to_le_bytes());
    }

    #[test]
    fn test_file_header_v6_reads_header_checksum_zero() {
        let mut bytes = vec![0u8; 80];
        bytes[0..4].copy_from_slice(b"MGRF");
        bytes[4..8].copy_from_slice(&6u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&2u64.to_le_bytes());
        let h = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.version, 6);
        assert_eq!(h.header_checksum, 0);
    }

    #[test]
    fn test_file_header_v7_truncated_rejected() {
        let mut bytes = vec![0u8; 80];
        bytes[0..4].copy_from_slice(b"MGRF");
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        assert!(FileHeader::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_validate_accepts_versions_1_to_7() {
        let mut h = FileHeader::new();
        for v in 1u32..=7 {
            h.version = v;
            assert!(h.validate().is_ok(), "version {} should be accepted", v);
        }
    }

    #[test]
    fn test_file_header_v7_header_checksum_roundtrip() {
        let mut h = FileHeader::new();
        h.header_checksum = 42;
        let bytes = h.to_bytes();
        assert_eq!(bytes.len(), 84);
        let parsed = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.header_checksum, 42);
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

    #[test]
    fn test_file_header_v5_fact_page_format_roundtrip() {
        let mut h = FileHeader::new();
        h.fact_page_format = FACT_PAGE_FORMAT_PACKED;
        let bytes = h.to_bytes();
        let parsed = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.fact_page_format, FACT_PAGE_FORMAT_PACKED);
    }

    #[test]
    fn test_v4_header_reads_fact_page_format_zero() {
        // v4 header has _padding = 0, so fact_page_format must come back as 0
        let mut bytes = vec![0u8; 72];
        bytes[0..4].copy_from_slice(b"MGRF");
        bytes[4..8].copy_from_slice(&4u32.to_le_bytes()); // version = 4
        bytes[8..16].copy_from_slice(&2u64.to_le_bytes()); // page_count = 2
        let h = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.fact_page_format, 0);
    }

    #[test]
    fn test_validate_accepts_version_5() {
        let mut h = FileHeader::new();
        h.version = 5;
        assert!(h.validate().is_ok());
    }
}
