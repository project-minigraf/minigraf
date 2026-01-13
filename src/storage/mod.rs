/// Storage backend abstraction for cross-platform support.
///
/// This module provides a trait-based abstraction over different storage backends,
/// allowing Minigraf to run on native platforms (file-based), WASM (IndexedDB),
/// and in-memory (testing/embedded).
///
/// Inspired by SQLite's VFS (Virtual File System) architecture.
pub mod backend;

use anyhow::Result;

/// Page size for the storage engine (4KB like SQLite)
pub const PAGE_SIZE: usize = 4096;

/// Magic number for .graph files: "MGRF" (Minigraf)
pub const MAGIC_NUMBER: [u8; 4] = *b"MGRF";

/// Current file format version
pub const FORMAT_VERSION: u32 = 1;

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

/// File header structure for .graph files.
///
/// Stored in page 0, provides metadata about the database.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    /// Magic number "MGRF"
    pub magic: [u8; 4],

    /// Format version number
    pub version: u32,

    /// Total number of pages in the file
    pub page_count: u64,

    /// Number of nodes in the graph
    pub node_count: u64,

    /// Number of edges in the graph
    pub edge_count: u64,

    /// Reserved for future use (padding to 64 bytes)
    pub reserved: [u8; 32],
}

impl FileHeader {
    /// Create a new file header with default values.
    pub fn new() -> Self {
        FileHeader {
            magic: MAGIC_NUMBER,
            version: FORMAT_VERSION,
            page_count: 1, // Just the header page initially
            node_count: 0,
            edge_count: 0,
            reserved: [0; 32],
        }
    }

    /// Serialize the header to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&self.magic);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.page_count.to_le_bytes());
        bytes.extend_from_slice(&self.node_count.to_le_bytes());
        bytes.extend_from_slice(&self.edge_count.to_le_bytes());
        bytes.extend_from_slice(&self.reserved);
        bytes
    }

    /// Deserialize the header from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 64 {
            anyhow::bail!("Invalid header: too short");
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);

        if magic != MAGIC_NUMBER {
            anyhow::bail!("Invalid magic number: not a .graph file");
        }

        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let page_count = u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
            bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        let node_count = u64::from_le_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19],
            bytes[20], bytes[21], bytes[22], bytes[23],
        ]);
        let edge_count = u64::from_le_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27],
            bytes[28], bytes[29], bytes[30], bytes[31],
        ]);

        let mut reserved = [0u8; 32];
        reserved.copy_from_slice(&bytes[32..64]);

        Ok(FileHeader {
            magic,
            version,
            page_count,
            node_count,
            edge_count,
            reserved,
        })
    }

    /// Validate the header.
    pub fn validate(&self) -> Result<()> {
        if self.magic != MAGIC_NUMBER {
            anyhow::bail!("Invalid magic number");
        }
        if self.version != FORMAT_VERSION {
            anyhow::bail!(
                "Unsupported format version: {} (expected {})",
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
    fn test_file_header_serialization() {
        let header = FileHeader::new();
        let bytes = header.to_bytes();

        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[0..4], b"MGRF");

        let deserialized = FileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(deserialized.magic, MAGIC_NUMBER);
        assert_eq!(deserialized.version, FORMAT_VERSION);
        assert_eq!(deserialized.page_count, 1);
    }

    #[test]
    fn test_file_header_validation() {
        let header = FileHeader::new();
        assert!(header.validate().is_ok());

        let mut invalid = header;
        invalid.magic = *b"XXXX";
        assert!(invalid.validate().is_err());
    }
}
