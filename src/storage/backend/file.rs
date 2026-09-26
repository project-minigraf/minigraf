/// File-based storage backend for native platforms.
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::dir_sync::sync_parent_dir;
use crate::storage::{FileHeader, PAGE_SIZE, StorageBackend};
use anyhow::Result;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What `FileBackend::open` should do with the result of `File::try_lock`.
///
/// Split out from the I/O so the unsupported-filesystem branch is testable
/// without an exotic filesystem: no CI runner can be relied on to provide a
/// mount whose locks fail.
enum LockOutcome {
    /// We hold the lock.
    Acquired,
    /// Another open file description holds it. May be in this process.
    Held,
    /// The filesystem cannot lock, and the caller did not opt in to running
    /// without one.
    ///
    /// This branch is reachable only when the OS itself reports the lock
    /// call as unsupported (e.g. some FUSE mounts, or NFSv3 with `lockd`
    /// simply not running). It is NOT reachable on an NFSv3 export mounted
    /// with the explicit `nolock` option: there, the Linux NFS client
    /// serves `flock`/OFD locks out of its own local, in-kernel lock table
    /// instead of going over NLM to the server, so `try_lock` sees an
    /// ordinary local success and this crate cannot tell that mount apart
    /// from one where locking genuinely works. `allow_unlocked` is never
    /// consulted in that case, because `classify` never learns the mount
    /// can't really coordinate locks across hosts. `nolock` NFSv3 exports
    /// are an unsupported deployment for this reason — see #334 and
    /// docs/ERROR_REFERENCE.md STG-027.
    Unsupported(std::io::Error),
    /// The filesystem cannot lock and the caller accepted the risk.
    ProceedUnlocked,
}

/// Decide what a `try_lock` result means.
///
/// `allow_unlocked` is consulted ONLY for `TryLockError::Error`, never for
/// `WouldBlock`. Letting it bypass a live lock would reintroduce the
/// two-writers-one-file corruption the lock exists to prevent.
fn classify(result: Result<(), std::fs::TryLockError>, allow_unlocked: bool) -> LockOutcome {
    match result {
        Ok(()) => LockOutcome::Acquired,
        Err(std::fs::TryLockError::WouldBlock) => LockOutcome::Held,
        Err(std::fs::TryLockError::Error(_)) if allow_unlocked => LockOutcome::ProceedUnlocked,
        Err(std::fs::TryLockError::Error(e)) => LockOutcome::Unsupported(e),
    }
}

/// Canonicalised paths this process currently has open.
///
/// Correctness comes from the kernel lock, not this registry -- but this is
/// no longer *purely* diagnostic. Two things read it: which error message to
/// print (#304 was misdiagnosed for a long time when the generic "another
/// process" wording sent the investigation looking for a second process that
/// did not exist), and whether `open_with` retries a `WouldBlock` at all. A
/// same-process conflict is unwinnable by waiting, so that decision skips the
/// retry loop entirely; only this registry can tell the two cases apart. If
/// `already_open_here` ever wrongly returned `true`, `open_with` would both
/// report the wrong error *and* skip the retry that would have let it
/// succeed.
static OPEN_PATHS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// Removes this backend's path from `OPEN_PATHS` when the backend drops.
struct PathGuard(Option<PathBuf>);

impl Drop for PathGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take()
            && let Ok(mut open) = OPEN_PATHS.lock()
        {
            open.remove(&path);
        }
    }
}

/// True if this process already has `path` open.
///
/// Not diagnostic-only: besides choosing which error message to print, this
/// also gates whether `open_with` retries a `WouldBlock` at all (see
/// `OPEN_PATHS` above).
fn already_open_here(path: &Path) -> bool {
    OPEN_PATHS
        .lock()
        .map(|open| open.contains(path))
        .unwrap_or(false)
}

/// Atomically test-and-set `path` into `OPEN_PATHS`, admitting only the
/// first caller.
///
/// Used by the `ProceedUnlocked` arm, where there is no kernel lock and this
/// registry is the only thing preventing two handles on one unlockable path
/// (#304). Must be a single critical section: checking with
/// `already_open_here` and inserting afterwards would let two threads both
/// observe "absent" and both proceed. A poisoned mutex yields `false`, so
/// this fails CLOSED rather than admitting a second handle it cannot
/// account for.
///
/// Split out from `open_with` so the race can be exercised directly (#330)
/// instead of needing a genuinely lock-incapable filesystem.
fn claim_unlocked_path(path: &Path) -> bool {
    OPEN_PATHS
        .lock()
        .map(|mut open| open.insert(path.to_path_buf()))
        .unwrap_or(false)
}

/// How many times to retry a cross-process `WouldBlock` before concluding
/// another process genuinely holds the lock.
///
/// `Command::spawn` (in this process or any other) forks before exec'ing the
/// child; until the child reaches `execve` and its close-on-exec descriptors
/// close, it transiently holds a duplicate of every flock its parent has
/// open. A concurrent `try_lock()` elsewhere -- another thread in this
/// process, a sibling process, or bindings that shell out (Python/Node
/// commonly do) -- can observe `WouldBlock` during that few-microsecond
/// window even though no other process is really contending for the file.
/// SQLite has the same class of problem and solves it with `busy_timeout`;
/// this is our version, bounded so a database that is genuinely locked by
/// another process still fails -- after paying the full retry budget below,
/// not instantly -- rather than `open` hanging indefinitely.
const LOCK_RETRY_ATTEMPTS: u32 = 10;

/// Initial backoff between retries, doubled each attempt up to
/// `LOCK_RETRY_MAX_DELAY`.
const LOCK_RETRY_INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(5);

/// Backoff cap. With `LOCK_RETRY_ATTEMPTS` retries doubling from
/// `LOCK_RETRY_INITIAL_DELAY` and capping here, the worst case is
/// 5+10+20+40+50+50+50+50+50+50 = 375ms of total sleep -- comfortably longer
/// than a fork-to-`execve` window, comfortably shorter than a human notices.
const LOCK_RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

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
    // `_path_guard` must drop before `file`: dropping `file` releases the
    // kernel lock, and if the registry entry were still present after that,
    // another thread in this process could win the lock and insert its own
    // entry between the two drops -- which this guard would then delete,
    // leaving the registry claiming "not open" while a live handle exists.
    // Rust drops struct fields in declaration order, so this field must be
    // declared before `file`.
    _path_guard: PathGuard,
    file: File,
    header: FileHeader,
    is_new: bool,
}

impl FileBackend {
    /// Open or create a .graph file at the given path.
    ///
    /// If the file doesn't exist, creates it with an initial header.
    /// If it exists, validates and loads the header.
    ///
    /// Takes a kernel file lock on the `.graph` file itself, which prevents
    /// both multi-process corruption and a second handle within this process.
    /// The kernel releases the lock when the process exits, however it exits,
    /// so a crashed holder never leaves the database unopenable (#317).
    ///
    /// A `WouldBlock` from a possible cross-process conflict is retried with
    /// bounded backoff (see `LOCK_RETRY_ATTEMPTS`) before being treated as a
    /// real conflict, to ride out the transient window where a forked but
    /// not-yet-exec'd subprocess holds a duplicate of someone else's lock.
    ///
    /// Production code calls `open_with` directly. This wrapper survives as a
    /// convenience for the ~40 in-crate test call sites.
    #[cfg(test)]
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with(path, false)
    }

    /// As `FileBackend::open` (test-only), but `allow_unlocked` permits opening on a
    /// filesystem that cannot lock at all.
    ///
    /// `allow_unlocked` does NOT override a lock held by someone else. It
    /// applies only when the filesystem rejects locking outright.
    pub fn open_with<P: AsRef<Path>>(path: P, allow_unlocked: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // Canonicalise only after the file exists. Used for the diagnostic
        // registry; if it fails we fall back to the generic message.
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

        let mut lock_result = file.try_lock();

        // Retry a `WouldBlock` with backoff, but ONLY when the conflict might
        // be with another process. If this process already has the path
        // open, waiting is pointless: we are the only thing that could ever
        // release that lock, and we are standing right here not releasing
        // it, so every retry would just burn the budget on a certain
        // failure. Skipping this check keeps `test_second_open_in_same_process_is_refused`
        // fast instead of paying the full retry budget on every same-process
        // rejection.
        if matches!(lock_result, Err(std::fs::TryLockError::WouldBlock))
            && !already_open_here(&canonical)
        {
            let mut delay = LOCK_RETRY_INITIAL_DELAY;
            for _ in 0..LOCK_RETRY_ATTEMPTS {
                std::thread::sleep(delay);
                lock_result = file.try_lock();
                if !matches!(lock_result, Err(std::fs::TryLockError::WouldBlock)) {
                    break;
                }
                delay = (delay * 2).min(LOCK_RETRY_MAX_DELAY);
            }
        }

        let path_guard = match classify(lock_result, allow_unlocked) {
            LockOutcome::Acquired => {
                if let Ok(mut open) = OPEN_PATHS.lock() {
                    open.insert(canonical.clone());
                }
                PathGuard(Some(canonical))
            }
            LockOutcome::Held if already_open_here(&canonical) => {
                bail_coded!(ErrorCode::Stg025, path.display());
            }
            LockOutcome::Held => {
                bail_coded!(ErrorCode::Stg026, path.display());
            }
            LockOutcome::Unsupported(e) => {
                bail_coded!(ErrorCode::Stg027, path.display(), e);
            }
            LockOutcome::ProceedUnlocked => {
                // There is no kernel lock in this arm, so the registry is not
                // merely diagnostic here: it is the only thing standing
                // between us and the #304 two-page-tables corruption if a
                // second handle in this process opens the same unlockable
                // path. See `claim_unlocked_path` for why the check and
                // insert must be one atomic operation.
                if !claim_unlocked_path(&canonical) {
                    bail_coded!(ErrorCode::Stg025, path.display());
                }
                PathGuard(Some(canonical))
            }
        };

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
                    // File has content but header is invalid - this is a real error.
                    // If `e` already carries a more specific code (e.g. STG-002 for a
                    // bad magic number, raised inside `FileHeader::from_bytes`/
                    // `validate`), propagate it as-is rather than burying it under
                    // the generic STG-010 label — `read_header` never wraps with
                    // `.context()`, so a coded error from deeper down comes back
                    // here unmodified and downcasts directly.
                    if e.downcast_ref::<crate::error::CodedError>().is_some() {
                        return Err(e);
                    }
                    bail_coded!(ErrorCode::Stg010, e);
                }
            }
        } else {
            // New file or empty file: write initial header, then make the
            // file's directory entry durable. Without the directory sync a
            // power loss can lose the whole file despite the header fsync
            // (#389). An empty file left by a crash before this point takes
            // this branch again on the next open, so the sync is repeated.
            let header = FileHeader::new();
            Self::write_header(&mut file, &header)?;
            sync_parent_dir(&path)?;
            header
        };

        Ok(FileBackend {
            path,
            file,
            header,
            is_new,
            _path_guard: path_guard,
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
        let hlen = header_bytes.len();
        page.get_mut(..hlen)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "header bytes exceed page size"))?
            .copy_from_slice(&header_bytes);

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
            bail_coded!(ErrorCode::Int051, data.len(), PAGE_SIZE);
        }

        let offset = page_id.checked_mul(PAGE_SIZE as u64).ok_or_else(|| {
            err_coded!(
                ErrorCode::Int048,
                format!("page offset overflow for page_id {page_id}")
            )
        })?;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(data)?;

        // If writing page 0 (header), update our in-memory header
        if page_id == 0 {
            // Page 0 is the header itself, parse it to update our cached copy
            self.header = FileHeader::from_bytes(data)?;
        } else if page_id >= self.header.page_count {
            // Bump and persist the page count so a plain reopen (no
            // deliberate commit in between) still sees pages written this
            // session -- callers like `test_file_backend_persistence`
            // depend on that. #308: the bug here was writing the bumped
            // page_count with a *stale* header_checksum left over from the
            // last real commit, so a crash before that commit finished left
            // a permanently checksum-invalid header with no recovery path.
            // The fix is to recompute the checksum over the whole header
            // every time, so the on-disk header is self-consistent at
            // every single-page write, not just at the deliberate commits
            // in `PersistentFactStorage::save`.
            self.header.page_count = page_id.checked_add(1).ok_or_else(|| {
                err_coded!(
                    ErrorCode::Int048,
                    format!("page_count overflow for page_id {page_id}")
                )
            })?;
            self.header.header_checksum =
                crate::storage::persistent_facts::compute_header_checksum(&self.header);
            Self::write_header(&mut self.file, &self.header)?;
        }

        Ok(())
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        if page_id >= self.header.page_count {
            bail_coded!(
                ErrorCode::Int049,
                format!(
                    "Page {page_id} out of bounds (total pages: {})",
                    self.header.page_count
                )
            );
        }

        let offset = page_id.checked_mul(PAGE_SIZE as u64).ok_or_else(|| {
            err_coded!(
                ErrorCode::Int048,
                format!("page offset overflow for page_id {page_id}")
            )
        })?;
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

    /// #304: a second handle on a file this process already has open must be
    /// refused. `flock` and OFD locks attach to the open file description, not
    /// the process, so the kernel denies this without any bookkeeping from us.
    #[test]
    fn test_second_open_in_same_process_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_self_steal.graph");

        let first = FileBackend::open(&temp_path).unwrap();

        // FileBackend is not Debug, so match on the Result rather than
        // expect_err.
        let msg = match FileBackend::open(&temp_path) {
            Ok(_) => panic!("a second handle in the same process must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("already open in this process"),
            "error should name the same-process case"
        );

        drop(first);
        // Once the only handle is dropped, the kernel releases the lock.
        FileBackend::open(&temp_path).unwrap();
    }

    /// #308: whenever `write_page` bumps `page_count` for a newly appended
    /// page (exactly what `save()`/`checkpoint()` do for every fact and
    /// index page, ahead of the deliberate final header commit), the header
    /// it persists to disk must have a `header_checksum` matching its own
    /// content. Before this fix, the page_count byte moved but the checksum
    /// was left stale from the last real commit -- a `SIGKILL` landing in
    /// that window (which fires on nearly every appended page) left a
    /// permanently checksum-invalid header with no recovery path.
    #[test]
    fn test_write_page_keeps_disk_header_checksum_consistent_on_page_count_bump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_header_atomicity.graph");
        let mut backend = FileBackend::open(&path).unwrap();

        // Append a page well beyond the current page_count (1) -- exactly
        // the pattern save()/checkpoint() use when writing fact and index
        // pages ahead of the final atomic header commit.
        let page_data = vec![0xABu8; PAGE_SIZE];
        backend.write_page(5, &page_data).unwrap();

        // Read back through the same handle rather than opening the path
        // independently: `backend` still holds the file lock, and Windows
        // (unlike Unix) refuses a second, unrelated open of a locked file
        // even from the same process.
        let raw = backend.read_page(0).unwrap();
        let on_disk = FileHeader::from_bytes(&raw).unwrap();
        let computed = crate::storage::persistent_facts::compute_header_checksum(&on_disk);
        assert_eq!(
            on_disk.header_checksum, computed,
            "on-disk header checksum must match its own content after a page-count bump"
        );
        assert_eq!(on_disk.page_count, 6);

        // In-memory bookkeeping must agree with what's now on disk.
        assert_eq!(backend.page_count().unwrap(), 6);
    }

    /// The child half of [`test_cross_process_lock_retries_then_fails_within_budget`]:
    /// opens `db_path`, holds the lock for `MINIGRAF_HOLD_MILLIS`, signalling
    /// readiness by creating `MINIGRAF_HOLD_READY_MARKER` right after
    /// acquiring it. A no-op when the marker environment variable is absent
    /// -- same convention as `tests/wal_test.rs`'s `crash_child_entrypoint`,
    /// which this mirrors for the same reason: `#[cfg(test)]` items are
    /// invisible outside this crate, so a real second process is the only
    /// way to exercise a genuinely external lock holder.
    #[test]
    fn hold_lock_entrypoint() {
        let Ok(db_path) = std::env::var("MINIGRAF_HOLD_DB") else {
            return;
        };
        let ready_marker = std::env::var("MINIGRAF_HOLD_READY_MARKER").expect("ready marker");
        let millis: u64 = std::env::var("MINIGRAF_HOLD_MILLIS")
            .expect("hold millis")
            .parse()
            .expect("hold millis is a number");

        let backend = FileBackend::open(&db_path).expect("child holds lock");
        std::fs::write(&ready_marker, "").expect("write ready marker");
        std::thread::sleep(std::time::Duration::from_millis(millis));
        drop(backend);
        // Falling off the end of the test runs Drop normally (unlike the
        // WAL crash tests, this one is not simulating a crash -- it is
        // simulating an ordinary second process that is simply still open),
        // which releases the kernel lock exactly as the retrying parent
        // needs to see happen once its budget is spent... except the parent
        // stops retrying before that, by design: see the driver test.
    }

    /// A genuinely cross-process lock must still refuse `open` -- with the
    /// cross-process wording, not the same-process one -- and must do so
    /// within the bounded retry budget rather than hang.
    ///
    /// This is the production-code counterpart to `LOCK_RETRY_ATTEMPTS`
    /// and `LOCK_RETRY_MAX_DELAY`: nothing else on this branch spawns a
    /// second process to hold the lock while another tries to open it, so
    /// nothing else exercises either the cross-process error wording or the
    /// bound on retry duration. Point (c) below matters beyond timing: if
    /// `already_open_here` ever wrongly returned `true` for a path this
    /// process does not actually have open, this is the only test that
    /// would catch it -- everything else in this module opens and closes
    /// within a single process.
    #[test]
    fn test_cross_process_lock_retries_then_fails_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_cross_process_hold.graph");
        let ready_marker = {
            let mut p = db_path.clone().into_os_string();
            p.push(".ready");
            PathBuf::from(p)
        };
        let _ = std::fs::remove_file(&ready_marker);

        let exe = std::env::current_exe().expect("test binary path");
        let mut holder = std::process::Command::new(&exe)
            .args([
                "storage::backend::file::tests::hold_lock_entrypoint",
                "--exact",
                "--nocapture",
            ])
            .env("MINIGRAF_HOLD_DB", &db_path)
            .env("MINIGRAF_HOLD_READY_MARKER", &ready_marker)
            // Comfortably longer than LOCK_RETRY_ATTEMPTS' ~375ms budget, so
            // this process's retries must exhaust while the lock is still
            // genuinely held.
            //
            // 3s rather than something nearer the budget: at 600ms only
            // ~215ms of slack separated the two, and if this process were
            // descheduled for longer than that anywhere across its ten retry
            // iterations, the child would release mid-loop, this open would
            // SUCCEED, and the test would fail claiming a cross-process lock
            // was not refused. CI runs this on three platforms with 34 test
            // binaries in flight, so that margin was not safe.
            .env("MINIGRAF_HOLD_MILLIS", "3000")
            .spawn()
            .expect("spawn lock holder");

        // Wait for the child to actually hold the lock before racing it --
        // otherwise this test would be racing its own setup, not the retry.
        let setup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready_marker.exists() {
            assert!(
                std::time::Instant::now() < setup_deadline,
                "lock-holding child never signalled readiness"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let start = std::time::Instant::now();
        let result = FileBackend::open(&db_path);
        let elapsed = start.elapsed();

        // (a) a real cross-process lock is still refused, with the coded
        // STG-026 error (#359).
        let (msg, code) = match result {
            Ok(_) => panic!("a genuinely cross-process lock must be refused"),
            Err(e) => {
                let msg = e.to_string();
                let code = crate::error::MinigrafError::from(e).code().to_string();
                (msg, code)
            }
        };
        assert_eq!(
            code, "STG-026",
            "cross-process lock conflict must be STG-026"
        );

        // (b) the retry budget is bounded: this must fail in roughly the
        // ~375ms budget, NOT hang for the holder's full 3s hold. The 2s bound
        // is far below the hold, so a regression that waited for the holder
        // instead of giving up would fail here.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "retry budget must be bounded, not hang"
        );

        // (c) the error names the cross-process case, not the same-process
        // one -- this process never had the path open.
        assert!(
            msg.contains("locked by another process"),
            "must report the cross-process wording"
        );
        assert!(
            !msg.contains("already open in this process"),
            "must not report the same-process wording for a real second process"
        );

        // Once the holder exits, the lock -- and the holder's own registry
        // entry, which only the holder's process could ever remove -- are
        // both gone, so a fresh open succeeds without retrying at all.
        //
        // Kill rather than waiting out the remaining hold: it keeps the test
        // fast, and release-on-SIGKILL is the property that actually matters
        // here. A holder that exits cleanly is the easy case; #317 was about
        // one that never got to run any cleanup at all.
        holder.kill().expect("kill lock holder");
        holder.wait().expect("reap lock holder");
        FileBackend::open(&db_path).expect("open must succeed once the holder exits");

        let _ = std::fs::remove_file(&ready_marker);
    }

    /// The refusal must not regress the sequential open/drop/reopen cycle that
    /// `temporal_reasoning` relies on between commits.
    #[test]
    fn test_reopen_after_drop_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_reopen.graph");

        for _ in 0..3 {
            let backend = FileBackend::open(&temp_path).unwrap();
            drop(backend);
        }
        assert!(FileBackend::open(&temp_path).is_ok());
    }

    /// A leftover sidecar from v1.2.x must not block an open. We neither read
    /// nor delete it: a still-running old process may rely on it.
    #[test]
    fn test_leftover_sidecar_is_ignored_and_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_leftover.graph");
        let legacy = temp_path.with_extension("graph.lock");
        std::fs::write(&legacy, "1").unwrap();

        let backend = FileBackend::open(&temp_path).unwrap();
        drop(backend);

        assert!(
            legacy.exists(),
            "a leftover sidecar must be left alone, not deleted"
        );
    }

    /// The path registry must not leak entries: a refused open must leave the
    /// set exactly as it found it, so a later refusal still reports the
    /// same-process case rather than degrading to the generic
    /// "locked by another process" wording.
    #[test]
    fn test_refused_open_does_not_corrupt_path_registry() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("test_minigraf_registry.graph");

        let first = FileBackend::open(&temp_path).unwrap();
        assert!(FileBackend::open(&temp_path).is_err()); // first refusal

        // A refused open must not have removed first's registry entry: a
        // second refusal must still see it and report the same-process case.
        let msg = match FileBackend::open(&temp_path) {
            Ok(_) => panic!("must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("already open in this process"),
            "registry entry must survive a refused open"
        );

        drop(first);
        FileBackend::open(&temp_path).unwrap();
    }

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

    #[test]
    fn test_classify_ok_is_acquired() {
        assert!(matches!(classify(Ok(()), false), LockOutcome::Acquired));
        assert!(matches!(classify(Ok(()), true), LockOutcome::Acquired));
    }

    #[test]
    fn test_classify_would_block_is_held_regardless_of_allow_unlocked() {
        // allow_unlocked must NOT override a live lock: doing so reintroduces
        // exactly the cross-process corruption the lock exists to prevent.
        assert!(matches!(
            classify(Err(std::fs::TryLockError::WouldBlock), false),
            LockOutcome::Held
        ));
        assert!(matches!(
            classify(Err(std::fs::TryLockError::WouldBlock), true),
            LockOutcome::Held
        ));
    }

    #[test]
    fn test_classify_unsupported_fails_closed_by_default() {
        let e = std::io::Error::from_raw_os_error(95); // ENOTSUP
        assert!(matches!(
            classify(Err(std::fs::TryLockError::Error(e)), false),
            LockOutcome::Unsupported(_)
        ));
    }

    #[test]
    fn test_classify_unsupported_proceeds_when_allowed() {
        let e = std::io::Error::from_raw_os_error(37); // ENOLCK
        assert!(matches!(
            classify(Err(std::fs::TryLockError::Error(e)), true),
            LockOutcome::ProceedUnlocked
        ));
    }

    #[test]
    fn test_claim_unlocked_path_race_admits_only_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("race.graph"));

        const THREADS: usize = 16;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = barrier.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_unlocked_path(&path)
                })
            })
            .collect();

        let admitted = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&admitted| admitted)
            .count();

        OPEN_PATHS.lock().unwrap().remove(path.as_path());

        assert_eq!(
            admitted, 1,
            "exactly one concurrent claimant should be admitted"
        );
    }

    // ── #389: parent-directory fsync ────────────────────────────────────────

    #[test]
    fn test_create_syncs_parent_dir_after_header_write() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.graph");
        take_sync_log();

        let backend = FileBackend::open(&path).unwrap();

        let log = take_sync_log();
        assert_eq!(
            log.len(),
            1,
            "creating the file must sync its directory once"
        );
        assert_eq!(
            log[0].dir,
            dir.path(),
            "must sync the file's parent directory"
        );
        assert!(
            log[0].child_existed,
            "directory sync must follow the create"
        );
        drop(backend);
    }

    #[test]
    fn test_reopen_existing_does_not_sync_parent_dir() {
        use crate::storage::dir_sync::take_sync_log;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.graph");
        drop(FileBackend::open(&path).unwrap());
        take_sync_log();

        let backend = FileBackend::open(&path).unwrap();

        assert!(
            take_sync_log().is_empty(),
            "reopening an initialized file creates nothing, so needs no directory sync"
        );
        drop(backend);
    }
}
