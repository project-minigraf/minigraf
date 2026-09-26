//! Parent-directory fsync (#389).
//!
//! On POSIX systems a newly created, deleted or renamed file survives a power
//! loss only once its parent directory has been fsynced: `fsync` on the file
//! itself makes the file's *contents* durable, not the directory entry that
//! names it. Without this, a power loss (not a process kill -- a kill leaves
//! the OS page cache intact) can lose a freshly created `.graph` or `<db>.wal`,
//! or bring back a WAL that `checkpoint()` deleted.
//!
//! Durability order:
//!
//! - Create: create file → write + `sync_all` its first bytes → sync parent dir.
//! - Delete: remove file → sync parent dir.
//!
//! Windows needs no directory sync (NTFS journals directory metadata), and
//! cannot open a directory as a `File` without extra flags, so this is a no-op
//! there and on every other non-Unix target.

use std::io;
use std::path::Path;

/// Fsync the directory that contains `child`, making `child`'s creation or
/// removal durable.
///
/// `child` is the path of the file that was just created or removed, not the
/// directory itself. A bare file name (no parent component) refers to the
/// current directory.
pub(crate) fn sync_parent_dir(child: &Path) -> io::Result<()> {
    let dir = match child.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };

    #[cfg(test)]
    record(dir, child);

    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

// ─── Test-only call log ─────────────────────────────────────────────────────
//
// A power loss cannot be simulated in a test, so tests instead check that the
// sync happens at all and in the right order relative to the file operation:
// each call records the directory and whether `child` existed at that moment.
// After a create it must exist; after a delete it must not. Thread-local, so
// concurrently running tests do not see each other's calls.

#[cfg(test)]
thread_local! {
    static SYNC_LOG: std::cell::RefCell<Vec<DirSync>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// One recorded `sync_parent_dir` call.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirSync {
    /// The directory that was synced.
    pub dir: std::path::PathBuf,
    /// The file the sync was made on behalf of.
    pub child: std::path::PathBuf,
    /// Whether `child` existed when the directory was synced.
    pub child_existed: bool,
}

#[cfg(test)]
fn record(dir: &Path, child: &Path) {
    SYNC_LOG.with(|log| {
        log.borrow_mut().push(DirSync {
            dir: dir.to_path_buf(),
            child: child.to_path_buf(),
            child_existed: child.exists(),
        })
    });
}

/// Return and clear this thread's recorded directory syncs.
#[cfg(test)]
pub(crate) fn take_sync_log() -> Vec<DirSync> {
    SYNC_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syncs_parent_of_child() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("a.graph");
        std::fs::write(&child, b"x").unwrap();
        take_sync_log();

        sync_parent_dir(&child).unwrap();

        let log = take_sync_log();
        assert_eq!(log.len(), 1, "expected exactly one directory sync");
        assert_eq!(log[0].dir, dir.path(), "must sync the child's parent");
        assert!(log[0].child_existed, "child existed at sync time");
    }

    #[test]
    fn test_bare_file_name_syncs_current_dir() {
        take_sync_log();
        sync_parent_dir(Path::new("no-such-file.graph")).unwrap();
        let log = take_sync_log();
        assert_eq!(log.len(), 1, "expected exactly one directory sync");
        assert_eq!(log[0].dir, Path::new("."), "bare name means current dir");
        assert!(!log[0].child_existed, "child does not exist");
    }

    #[cfg(unix)]
    #[test]
    fn test_missing_parent_dir_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("gone").join("a.graph");
        assert!(
            sync_parent_dir(&child).is_err(),
            "a directory that cannot be opened must surface as an error"
        );
    }
}
