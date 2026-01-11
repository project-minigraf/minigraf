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
            .open(&path)?;

        // Try to read existing header. If file is empty/new, this will fail.
        let header = match Self::read_header(&mut file) {
            Ok(header) => {
                // Existing file with valid header
                header
            }
            Err(_) => {
                // New file or empty file: write initial header
                let header = FileHeader::new();
                Self::write_header(&mut file, &header)?;
                header
            }
        };

        Ok(FileBackend { path, file, header })
    }

    /// Read the file header from page 0.
    fn read_header(file: &mut File) -> Result<FileHeader> {
        file.seek(SeekFrom::Start(0))?;

        let mut header_bytes = vec![0u8; PAGE_SIZE];
        file.read_exact(&mut header_bytes)?;

        let header = FileHeader::from_bytes(&header_bytes)?;
        header.validate()?;

        eprintln!("FileBackend::read_header - read page_count={}", header.page_count);
        Ok(header)
    }

    /// Write the file header to page 0.
    fn write_header(file: &mut File, header: &FileHeader) -> Result<()> {
        file.seek(SeekFrom::Start(0))?;

        let header_bytes = header.to_bytes();
        let mut page = vec![0u8; PAGE_SIZE];
        page[..header_bytes.len()].copy_from_slice(&header_bytes);

        eprintln!("FileBackend::write_header - writing page_count={}", header.page_count);
        file.write_all(&page)?;
        file.sync_all()?;

        Ok(())
    }

    /// Update the header in memory and on disk.
    fn update_header(&mut self, header: FileHeader) -> Result<()> {
        Self::write_header(&mut self.file, &header)?;
        self.header = header;
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
            eprintln!("FileBackend::write_page(0) - updated in-memory header to page_count={}", self.header.page_count);
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
        eprintln!("FileBackend::close() - final sync and close");
        self.sync()?;
        // Re-read the header to verify it was written correctly
        let final_header = Self::read_header(&mut self.file)?;
        eprintln!("FileBackend::close() - verified page_count={}", final_header.page_count);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "file"
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
        let temp_path = "/tmp/test_minigraf_persistence.graph";
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
