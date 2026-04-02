/// File-based storage backend for native platforms.
use crate::storage::{FileHeader, StorageBackend, PAGE_SIZE};
use anyhow::Result;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// File-based storage backend for native platforms.
///
/// Stores graph data in a single `.graph` file with a page-based structure:
/// - Page 0: File header (metadata)
/// - Page 1+: Data pages (nodes, edges, indexes)
///
/// Supports:
/// - Linux, macOS, Windows (native desktop)
/// - iOS, Android (via FFI)
///
/// File format is cross-platform (endian-safe).
pub struct FileBackend {
    path: PathBuf,
    file: File,
    header: FileHeader,
    is_new: bool,
}

impl FileBackend {
    /// Open or create a .graph file at the given path.
    ///
    /// If the file doesn't exist, creates it with an initial header.
    /// If it exists, validates and loads the header.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // Check file size using the open file handle's metadata.
        // This is more reliable than checking path metadata separately,
        // as it uses the same file descriptor we'll be reading from.
        let file_len = file.metadata()?.len();

        // Determine if this is an existing file with data or a new/empty one.
        let is_new = file_len < PAGE_SIZE as u64;
        let header = if file_len >= PAGE_SIZE as u64 {
            // File has at least one page - try to read the header
            match Self::read_header(&mut file) {
                Ok(header) => header,
                Err(e) => {
                    // File has content but header is invalid - this is a real error
                    anyhow::bail!(
                        "Failed to read header from existing file (size={}): {}",
                        file_len,
                        e
                    );
                }
            }
        } else {
            // New file or empty file: write initial header
            let header = FileHeader::new();
            Self::write_header(&mut file, &header)?;
            header
        };

        Ok(FileBackend {
            path,
            file,
            header,
            is_new,
        })
    }

    /// Read the file header from page 0.
    fn read_header(file: &mut File) -> Result<FileHeader> {
        file.seek(SeekFrom::Start(0))?;

        let mut header_bytes = vec![0u8; PAGE_SIZE];
        file.read_exact(&mut header_bytes)?;

        let header = FileHeader::from_bytes(&header_bytes)?;
        header.validate()?;

        Ok(header)
    }

    /// Write the file header to page 0.
    fn write_header(file: &mut File, header: &FileHeader) -> Result<()> {
        file.seek(SeekFrom::Start(0))?;

        let header_bytes = header.to_bytes();
        let mut page = vec![0u8; PAGE_SIZE];
        page[..header_bytes.len()].copy_from_slice(&header_bytes);

        file.write_all(&page)?;
        file.sync_all()?;

        Ok(())
    }

    /// Get the file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl StorageBackend for FileBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        if data.len() != PAGE_SIZE {
            anyhow::bail!(
                "Invalid page size: {} bytes (expected {})",
                data.len(),
                PAGE_SIZE
            );
        }

        let offset = page_id * PAGE_SIZE as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(data)?;

        // If writing page 0 (header), update our in-memory header
        if page_id == 0 {
            // Page 0 is the header itself, parse it to update our cached copy
            self.header = FileHeader::from_bytes(data)?;
        } else if page_id >= self.header.page_count {
            // Update page count if this is a new page (but not page 0)
            self.header.page_count = page_id + 1;
            Self::write_header(&mut self.file, &self.header)?;
        }

        Ok(())
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        if page_id >= self.header.page_count {
            anyhow::bail!(
                "Page {} out of bounds (total pages: {})",
                page_id,
                self.header.page_count
            );
        }

        let offset = page_id * PAGE_SIZE as u64;
        let mut file = &self.file;
        file.seek(SeekFrom::Start(offset))?;

        let mut data = vec![0u8; PAGE_SIZE];
        file.read_exact(&mut data)?;

        Ok(data)
    }

    fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    fn page_count(&self) -> Result<u64> {
        Ok(self.header.page_count)
    }

    fn close(&mut self) -> Result<()> {
        self.sync()
    }

    fn backend_name(&self) -> &'static str {
        "file"
    }

    fn is_new(&self) -> bool {
        self.is_new
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_file_backend_create() {
        let temp_path = "/tmp/test_minigraf_create.graph";
        let _ = fs::remove_file(temp_path); // Clean up if exists

        let backend = FileBackend::open(temp_path).unwrap();
        assert_eq!(backend.backend_name(), "file");
        assert_eq!(backend.page_count().unwrap(), 1); // Header page

        // Clean up
        drop(backend);
        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_file_backend_write_read() {
        let temp_path = "/tmp/test_minigraf_write_read.graph";
        let _ = fs::remove_file(temp_path);

        let mut backend = FileBackend::open(temp_path).unwrap();

        let data = vec![42u8; PAGE_SIZE];
        backend.write_page(1, &data).unwrap(); // Page 0 is header

        let read_data = backend.read_page(1).unwrap();
        assert_eq!(data, read_data);

        // Clean up
        drop(backend);
        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_file_backend_persistence() {
        let temp_path = "/tmp/test_file_backend_persistence.graph";
        let _ = fs::remove_file(temp_path);

        // Write data
        {
            let mut backend = FileBackend::open(temp_path).unwrap();
            let data = vec![99u8; PAGE_SIZE];
            backend.write_page(1, &data).unwrap();
            backend.close().unwrap();
        }

        // Read data after reopening
        {
            let backend = FileBackend::open(temp_path).unwrap();
            let read_data = backend.read_page(1).unwrap();
            assert_eq!(read_data[0], 99);
        }

        // Clean up
        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_file_backend_page_count() {
        let temp_path = "/tmp/test_minigraf_page_count.graph";
        let _ = fs::remove_file(temp_path);

        let mut backend = FileBackend::open(temp_path).unwrap();
        assert_eq!(backend.page_count().unwrap(), 1);

        backend.write_page(1, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 2);

        backend.write_page(2, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 3);

        // Clean up
        drop(backend);
        fs::remove_file(temp_path).unwrap();
    }
}
