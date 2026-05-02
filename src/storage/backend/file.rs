/// File-based storage backend for native platforms.
use crate::storage::{FileHeader, PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Advisory file lock to prevent multi-process corruption.
///
/// Uses a sidecar `.lock` file with exclusive creation semantics.
/// The lock is released (file deleted) on drop.
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    /// Attempt to acquire an exclusive lock for the given database path.
    /// Returns `Err` if another process already holds the lock.
    ///
    /// If a stale lock file exists (the holder PID is no longer running),
    /// it is automatically removed and a new lock is acquired. This handles
    /// the case where the previous process crashed without cleaning up.
    fn acquire(db_path: &Path) -> Result<Self> {
        let lock_path = db_path.with_extension("graph.lock");
        // create_new fails with AlreadyExists if the lock file is present
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                // Write PID for diagnostics (best-effort)
                let _ = write!(f, "{}", std::process::id());
                Ok(FileLock { path: lock_path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Check if the holder process is still alive
                let holder = std::fs::read_to_string(&lock_path).unwrap_or_default();
                if let Ok(pid) = holder.trim().parse::<u32>() {
                    let our_pid = std::process::id();
                    if pid == our_pid || !Self::is_process_alive(pid) {
                        // Stale lock — either our own leaked handle or previous
                        // process crashed. Remove and retry.
                        let _ = std::fs::remove_file(&lock_path);
                        return Self::acquire(db_path);
                    }
                }
                anyhow::bail!(
                    "Database is locked by another process (lock file: {}, holder PID: {}). \
                     If no other process is using this database, delete the lock file manually.",
                    lock_path.display(),
                    holder.trim()
                );
            }
            Err(e) => {
                anyhow::bail!(
                    "Failed to acquire database lock at {}: {}",
                    lock_path.display(),
                    e
                );
            }
        }
    }

    /// Check if a process with the given PID is still running.
    fn is_process_alive(pid: u32) -> bool {
        // On Linux/Android, /proc/<pid> exists iff the process is alive.
        let proc_path = format!("/proc/{}", pid);
        if std::path::Path::new(&proc_path).exists() {
            return true;
        }
        // On Linux, if /proc exists but /proc/<pid> doesn't, the process is dead.
        if std::path::Path::new("/proc").exists() {
            return false;
        }
        // On non-procfs systems (macOS, Windows), assume alive (conservative).
        // Users must manually delete stale lock files on these platforms.
        true
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

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
    #[allow(dead_code)]
    path: PathBuf,
    file: File,
    header: FileHeader,
    is_new: bool,
    _lock: FileLock,
}

impl FileBackend {
    /// Open or create a .graph file at the given path.
    ///
    /// If the file doesn't exist, creates it with an initial header.
    /// If it exists, validates and loads the header.
    ///
    /// Acquires an advisory file lock (sidecar `.graph.lock` file) to prevent
    /// multi-process corruption. Returns an error if the database is already
    /// opened by another process.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Acquire advisory lock before touching the database file.
        let lock = FileLock::acquire(&path)?;

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
            _lock: lock,
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
    #[allow(dead_code)]
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn test_file_backend_create() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_create.graph");

        let backend = FileBackend::open(&temp_path).unwrap();
        assert_eq!(backend.backend_name(), "file");
        assert_eq!(backend.page_count().unwrap(), 1); // Header page
        assert!(backend.is_new(), "newly created file should be new");
    }

    #[test]
    fn test_file_backend_existing_file_not_new() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_existing.graph");

        {
            let backend = FileBackend::open(&temp_path).unwrap();
            assert!(backend.is_new(), "first open should be new");
        }

        {
            let backend = FileBackend::open(&temp_path).unwrap();
            assert!(
                !backend.is_new(),
                "reopening existing file should not be new"
            );
        }

        {
            let backend = FileBackend::open(&temp_path).unwrap();
            assert!(!backend.is_new(), "third open should still not be new");
        }
    }

    #[test]
    fn test_file_backend_write_read() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_write_read.graph");

        let mut backend = FileBackend::open(&temp_path).unwrap();

        let data = vec![42u8; PAGE_SIZE];
        backend.write_page(1, &data).unwrap(); // Page 0 is header

        let read_data = backend.read_page(1).unwrap();
        assert_eq!(data, read_data);
    }

    #[test]
    fn test_file_backend_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_file_backend_persistence.graph");

        // Write data
        {
            let mut backend = FileBackend::open(&temp_path).unwrap();
            let data = vec![99u8; PAGE_SIZE];
            backend.write_page(1, &data).unwrap();
            backend.close().unwrap();
        }

        // Read data after reopening
        {
            let backend = FileBackend::open(&temp_path).unwrap();
            let read_data = backend.read_page(1).unwrap();
            assert_eq!(read_data[0], 99);
        }
    }

    #[test]
    fn test_file_backend_page_count() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_page_count.graph");

        let mut backend = FileBackend::open(&temp_path).unwrap();
        assert_eq!(backend.page_count().unwrap(), 1);

        backend.write_page(1, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 2);

        backend.write_page(2, &vec![0u8; PAGE_SIZE]).unwrap();
        assert_eq!(backend.page_count().unwrap(), 3);
    }
}
