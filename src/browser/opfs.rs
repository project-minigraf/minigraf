//! OPFS persistence for `BrowserDb.openOpfs` (#441): the database is one
//! file in the Origin Private File System, read and written through a
//! `FileSystemSyncAccessHandle`, which browsers offer only in dedicated
//! workers.
//!
//! [`OpfsBackend`] is a [`StorageBackend`] over that handle: every page read
//! and write is a synchronous call into the browser, and nothing but the page
//! cache holds pages in memory. Commits are the storage layer's copy-on-write
//! protocol on the file itself; there is no WAL. The handle is exclusive: while
//! one is open on a file, acquiring another, from any worker or tab, fails with
//! `NoModificationAllowedError`, which is the database lock (STG-026).
//!
//! The database `name` is the file `<name>.graph` in the origin's root
//! directory. An import into OPFS mode stages the new database in
//! `<name>.graph-import` ([`STAGING_SUFFIX`]), a name no database maps to.
//!
//! The handle is a JavaScript object: `Send + Sync` only because wasm-bindgen
//! implements those for `JsValue` when wasm atomics are off. A build with
//! atomics therefore does not compile this backend, which is the sound
//! outcome: there is no threaded wasm here.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::storage::meta::{MetaPage, SlotState, slot_page};
use crate::storage::{PAGE_SIZE, StorageBackend};
use anyhow::Result;
use js_sys::Reflect;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetFileOptions,
    FileSystemReadWriteOptions, FileSystemSyncAccessHandle,
};

/// The file of database `name` is `name` followed by this.
pub(crate) const FILE_SUFFIX: &str = ".graph";
/// The staging file of an import is the database file's name followed by
/// this; no database file ends with it.
pub(crate) const STAGING_SUFFIX: &str = "-import";

/// The largest byte offset a JavaScript number holds exactly (2^53).
const MAX_SAFE_OFFSET: u64 = 1 << 53;

/// `[STG-048]`: a browser storage call failed. `detail` is the exception's
/// name, or a note such as "short read/write".
pub(crate) fn storage_error(
    op: impl std::fmt::Display,
    detail: impl std::fmt::Display,
) -> anyhow::Error {
    err_coded!(ErrorCode::Stg048, op, detail)
}

/// The `name` of a thrown `DOMException` (or any error object), else its
/// string form.
pub(crate) fn exception_name(e: &JsValue) -> String {
    Reflect::get(e, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| super::js_text(e))
}

fn js_failure(op: impl std::fmt::Display, e: &JsValue) -> anyhow::Error {
    storage_error(op, exception_name(e))
}

/// The byte offset of `page_id`, exact as a JavaScript number, or an error
/// before any I/O.
fn offset(page_id: u64) -> Result<f64> {
    match page_id.checked_mul(PAGE_SIZE as u64) {
        Some(off) if off <= MAX_SAFE_OFFSET => {
            // At most 2^53: exact as an f64.
            #[allow(clippy::cast_precision_loss)]
            Ok(off as f64)
        }
        _ => bail_coded!(
            ErrorCode::Int048,
            format!("page {page_id} is past the largest OPFS offset")
        ),
    }
}

fn at(off: f64) -> FileSystemReadWriteOptions {
    let opts = FileSystemReadWriteOptions::new();
    opts.set_at(off);
    opts
}

/// Bytes moved by a read or write, as the handle reports them.
fn transferred(n: f64) -> usize {
    // A count of at most PAGE_SIZE bytes, whole and non-negative.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = n as usize;
    n
}

/// One OPFS file and its open sync access handle, shared by every backend
/// and reader of one database. Closing it releases the lock; so does
/// dropping it.
pub(crate) struct OpfsFile {
    handle: FileSystemSyncAccessHandle,
    /// The file's entry name, for error messages.
    name: String,
    open: AtomicBool,
    #[cfg(test)]
    pub(crate) faults: std::sync::Mutex<tests::Faults>,
}

impl OpfsFile {
    fn new(handle: FileSystemSyncAccessHandle, name: String) -> OpfsFile {
        OpfsFile {
            handle,
            name,
            open: AtomicBool::new(true),
            #[cfg(test)]
            faults: std::sync::Mutex::default(),
        }
    }

    /// Close the handle, releasing the lock. Idempotent.
    pub(crate) fn close(&self) {
        if self.open.swap(false, Ordering::SeqCst) {
            self.handle.close();
        }
    }

    fn check_open(&self, op: &str) -> Result<()> {
        if !self.open.load(Ordering::SeqCst) {
            return Err(storage_error(
                format!("{op} {}", self.name),
                "handle closed",
            ));
        }
        Ok(())
    }

    /// The file's length in bytes.
    pub(crate) fn size(&self) -> Result<u64> {
        self.check_open("size of")?;
        let size = self
            .handle
            .get_size()
            .map_err(|e| js_failure(format!("size of {}", self.name), &e))?;
        // A file length: whole, non-negative and at most 2^53.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(size as u64)
    }

    /// Read `len` bytes at byte offset `off`; fewer is an error.
    fn read_at(&self, page_id: u64, len: usize) -> Result<Vec<u8>> {
        let op = || format!("read page {page_id} of {}", self.name);
        self.check_open("read")?;
        let off = offset(page_id)?;
        #[cfg(test)]
        tests::count_read();
        let mut buf = vec![0u8; len];
        let n = self
            .handle
            .read_with_u8_array_and_options(&mut buf, &at(off))
            .map_err(|e| js_failure(op(), &e))?;
        if transferred(n) != len {
            return Err(storage_error(op(), "short read/write"));
        }
        Ok(buf)
    }

    /// Write one page at `page_id`; a short write is an error.
    pub(crate) fn write_page(&self, page_id: u64, data: &[u8]) -> Result<()> {
        if data.len() != PAGE_SIZE {
            bail_coded!(ErrorCode::Int051, data.len(), PAGE_SIZE);
        }
        let op = || format!("write page {page_id} of {}", self.name);
        self.check_open("write")?;
        let off = offset(page_id)?;
        #[cfg(test)]
        if let Some(fault) = self.faults.lock().unwrap().on_write() {
            return self.inject_write(off, data, fault, op());
        }
        let n = self
            .handle
            .write_with_u8_array_and_options(data, &at(off))
            .map_err(|e| js_failure(op(), &e))?;
        if transferred(n) != data.len() {
            return Err(storage_error(op(), "short read/write"));
        }
        Ok(())
    }

    /// Persist the writes so far (`flush()`): as durable as the browser
    /// makes it, which is not a power-loss guarantee.
    pub(crate) fn flush(&self) -> Result<()> {
        self.check_open("flush")?;
        #[cfg(test)]
        if self.faults.lock().unwrap().on_flush() {
            return Err(storage_error(
                format!("flush {}", self.name),
                "fault injection",
            ));
        }
        self.handle
            .flush()
            .map_err(|e| js_failure(format!("flush {}", self.name), &e))
    }

    /// Set the file's length to `pages` pages.
    pub(crate) fn truncate(&self, pages: u64) -> Result<()> {
        self.check_open("truncate")?;
        let len = offset(pages)?;
        self.handle
            .truncate_with_f64(len)
            .map_err(|e| js_failure(format!("truncate {}", self.name), &e))
    }

    /// Whole pages in the file.
    pub(crate) fn page_count(&self) -> Result<u64> {
        Ok(self.size()? / PAGE_SIZE as u64)
    }

    pub(crate) fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        let pages = self.page_count()?;
        if page_id >= pages {
            bail_coded!(
                ErrorCode::Int049,
                format!("Page {page_id} out of bounds (total pages: {pages})")
            );
        }
        self.read_at(page_id, PAGE_SIZE)
    }

    /// The bytes past the last whole page, if any.
    fn partial_page(&self) -> Result<Option<Vec<u8>>> {
        let size = self.size()?;
        let tail = usize::try_from(size % PAGE_SIZE as u64).unwrap_or(0);
        if tail == 0 {
            return Ok(None);
        }
        self.read_at(size / PAGE_SIZE as u64, tail).map(Some)
    }

    /// The highest generation among the file's valid meta slots.
    pub(crate) fn newest_generation(&self) -> Result<Option<u64>> {
        let pages = self.page_count()?;
        let mut newest = None;
        for slot in 0..pages.min(2) {
            if let SlotState::Valid(m) = MetaPage::decode(&self.read_page(slot)?) {
                newest = newest.max(Some(m.generation));
            }
        }
        Ok(newest)
    }
}

impl Drop for OpfsFile {
    fn drop(&mut self) {
        self.close();
    }
}

/// A database's OPFS file as a [`StorageBackend`]. Every backend clone and
/// reader shares one [`OpfsFile`].
pub(crate) struct OpfsBackend {
    file: Arc<OpfsFile>,
}

impl OpfsBackend {
    pub(crate) fn new(file: Arc<OpfsFile>) -> OpfsBackend {
        OpfsBackend { file }
    }
}

impl StorageBackend for OpfsBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        self.file.write_page(page_id, data)
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.file.read_page(page_id)
    }

    fn sync(&mut self) -> Result<()> {
        self.file.flush()
    }

    fn page_count(&self) -> Result<u64> {
        self.file.page_count()
    }

    fn partial_page(&self) -> Result<Option<Vec<u8>>> {
        self.file.partial_page()
    }

    /// The handle stays open: `BrowserDb.close()` releases it.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "opfs"
    }
}

// ── Opening ─────────────────────────────────────────────────────────────────

/// `[API-026]`: OPFS mode is not available here, because of `what`.
fn unavailable(what: impl std::fmt::Display) -> JsValue {
    super::to_js_error(err_coded!(ErrorCode::Api026, what))
}

fn is_function(v: &JsValue) -> bool {
    v.is_function()
}

/// The origin's OPFS root directory, after checking that this context can
/// use sync access handles at all. Creates nothing.
pub(crate) async fn root() -> Result<FileSystemDirectoryHandle, JsValue> {
    let global = js_sys::global();
    let get =
        |obj: &JsValue, key: &str| Reflect::get(obj, &key.into()).unwrap_or(JsValue::UNDEFINED);
    if get(&global, "DedicatedWorkerGlobalScope").is_undefined()
        || !global.is_instance_of::<web_sys::DedicatedWorkerGlobalScope>()
    {
        return Err(unavailable(
            "sync access handles exist only in a dedicated worker; call openOpfs from one",
        ));
    }
    if get(&global, "isSecureContext").as_bool() == Some(false) {
        return Err(unavailable(
            "this is not a secure context (https or localhost)",
        ));
    }
    let storage = get(&get(&global, "navigator"), "storage");
    if storage.is_undefined() || !is_function(&get(&storage, "getDirectory")) {
        return Err(unavailable(
            "this browser has no navigator.storage.getDirectory()",
        ));
    }
    let handle_class = get(&global, "FileSystemFileHandle");
    if handle_class.is_undefined()
        || !is_function(&get(
            &get(&handle_class, "prototype"),
            "createSyncAccessHandle",
        ))
    {
        return Err(unavailable(
            "this browser has no FileSystemFileHandle.createSyncAccessHandle()",
        ));
    }
    let storage: web_sys::StorageManager = storage.unchecked_into();
    match JsFuture::from(storage.get_directory()).await {
        Ok(dir) => Ok(dir.unchecked_into()),
        Err(e) => Err(unavailable(format!(
            "the browser denies access to this origin's storage ({})",
            exception_name(&e)
        ))),
    }
}

/// Check a database name: it becomes part of an OPFS file name, unchanged.
pub(crate) fn check_name(name: &str) -> Result<(), JsValue> {
    let bad = if name.is_empty() {
        Some("is empty")
    } else if name.contains('/') || name.contains('\\') {
        Some("contains '/' or '\\'")
    } else if name == "." || name == ".." {
        Some("is '.' or '..'")
    } else if name.contains('\0') {
        Some("contains a NUL character")
    } else {
        None
    };
    match bad {
        Some(why) => Err(JsValue::from_str(
            &crate::MinigrafError::invalid_argument(format!("openOpfs: the database name {why}"))
                .to_string(),
        )),
        None => Ok(()),
    }
}

/// Open the file `entry` in `dir` and acquire its sync access handle.
/// `None` when `create` is false and the file does not exist. A handle
/// already open on the file anywhere is STG-026.
pub(crate) async fn open_file(
    dir: &FileSystemDirectoryHandle,
    entry: &str,
    create: bool,
) -> Result<Option<Arc<OpfsFile>>> {
    let opts = FileSystemGetFileOptions::new();
    opts.set_create(create);
    let file: FileSystemFileHandle =
        match JsFuture::from(dir.get_file_handle_with_options(entry, &opts)).await {
            Ok(f) => f.unchecked_into(),
            Err(e) if !create && exception_name(&e) == "NotFoundError" => return Ok(None),
            Err(e) => return Err(js_failure(format!("open {entry}"), &e)),
        };
    match JsFuture::from(file.create_sync_access_handle()).await {
        Ok(h) => {
            let file = OpfsFile::new(h.unchecked_into(), entry.to_string());
            #[cfg(test)]
            if let Some(faults) = tests::take_armed(entry) {
                *file.faults.lock().unwrap() = faults;
            }
            Ok(Some(Arc::new(file)))
        }
        Err(e) if exception_name(&e) == "NoModificationAllowedError" => bail_coded!(
            ErrorCode::Stg026,
            format!("OPFS file {entry} is open in another worker or tab")
        ),
        Err(e) => Err(js_failure(format!("acquire a handle on {entry}"), &e)),
    }
}

/// Remove `entry` from `dir`, ignoring failure: a later open retries.
pub(crate) async fn remove_quietly(dir: &FileSystemDirectoryHandle, entry: &str) {
    let _ = JsFuture::from(dir.remove_entry(entry)).await;
}

// ── Import staging ──────────────────────────────────────────────────────────

/// The staged database's meta page: valid, in one of the two slots, and
/// counting no page past the staging file's end. A staging file whose meta
/// was never completed has none: the import did not reach its publication
/// point.
pub(crate) fn staged_meta(stage: &OpfsFile) -> Result<Option<MetaPage>> {
    let pages = stage.page_count()?;
    let mut found = None;
    for slot in 0..pages.min(2) {
        if let SlotState::Valid(m) = MetaPage::decode(&stage.read_page(slot)?)
            && slot_page(m.generation) == slot
            && m.page_count <= pages
            && found.is_none_or(|f: MetaPage| f.generation < m.generation)
        {
            found = Some(m);
        }
    }
    Ok(found)
}

/// Make `dst` hold the image `src` stages, page by page, meta last: its
/// length first, then every other page, a flush, the meta page and a flush.
/// Until the meta is written `dst` has no valid meta of the image's
/// generation, so an interruption leaves the staging file in charge and the
/// copy is redone.
pub(crate) fn copy_staged(src: &OpfsFile, dst: &OpfsFile, meta: &MetaPage) -> Result<()> {
    let slot = slot_page(meta.generation);
    dst.truncate(meta.page_count)?;
    for id in (0..meta.page_count).filter(|&id| id != slot) {
        dst.write_page(id, &src.read_page(id)?)?;
    }
    dst.flush()?;
    dst.write_page(slot, &src.read_page(slot)?)?;
    dst.flush()
}

/// Finish an import a terminated worker left behind, under the database's
/// lock (`main` is open). A staged image newer than every meta of `main` is
/// copied into it; any staging file is then removed.
pub(crate) async fn recover_import(
    dir: &FileSystemDirectoryHandle,
    staging: &str,
    main: &OpfsFile,
) -> Result<()> {
    let Some(stage) = open_file(dir, staging, false).await? else {
        return Ok(());
    };
    if let Some(m) = staged_meta(&stage)?
        && main.newest_generation()?.is_none_or(|g| g < m.generation)
    {
        copy_staged(&stage, main, &m)?;
    }
    stage.close();
    drop(stage);
    remove_quietly(dir, staging).await;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    //! Fault injection and read counting for the OPFS tests.
    use std::cell::Cell;

    thread_local! {
        static READS: Cell<u64> = const { Cell::new(0) };
        static ARMED: std::cell::RefCell<Vec<(String, Faults)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Give the next file opened as `entry` on this thread these faults.
    pub(crate) fn arm(entry: &str, faults: Faults) {
        ARMED.with(|a| a.borrow_mut().push((entry.to_string(), faults)));
    }

    pub(crate) fn take_armed(entry: &str) -> Option<Faults> {
        ARMED.with(|a| {
            let mut a = a.borrow_mut();
            let i = a.iter().position(|(e, _)| e == entry)?;
            Some(a.remove(i).1)
        })
    }

    pub(crate) fn count_read() {
        READS.with(|r| r.set(r.get() + 1));
    }

    /// Pages (and partial tails) read from OPFS files on this thread.
    pub(crate) fn reads() -> u64 {
        READS.with(Cell::get)
    }

    /// What an injected write fault does.
    #[derive(Clone, Copy)]
    pub(crate) enum WriteFault {
        /// Throw after writing the first `n` bytes.
        Throw(usize),
        /// Report `n` bytes written, having written them.
        Short(usize),
    }

    /// Faults for one file: the write or flush after `n` more succeed.
    #[derive(Default, Clone)]
    pub(crate) struct Faults {
        pub write_after: Option<(u64, WriteFault)>,
        pub flush_after: Option<u64>,
    }

    impl Faults {
        pub(crate) fn on_write(&mut self) -> Option<WriteFault> {
            match &mut self.write_after {
                Some((0, f)) => {
                    let f = *f;
                    self.write_after = None;
                    Some(f)
                }
                Some((n, _)) => {
                    *n -= 1;
                    None
                }
                None => None,
            }
        }

        pub(crate) fn on_flush(&mut self) -> bool {
            match &mut self.flush_after {
                Some(0) => {
                    self.flush_after = None;
                    true
                }
                Some(n) => {
                    *n -= 1;
                    false
                }
                None => false,
            }
        }
    }
}

#[cfg(test)]
impl OpfsFile {
    /// The handle itself, for tests that write raw bytes.
    pub(crate) fn handle_for_tests(&self) -> &FileSystemSyncAccessHandle {
        &self.handle
    }

    fn inject_write(
        &self,
        off: f64,
        data: &[u8],
        fault: tests::WriteFault,
        op: String,
    ) -> Result<()> {
        let (n, throw) = match fault {
            tests::WriteFault::Throw(n) => (n.min(data.len()), true),
            tests::WriteFault::Short(n) => (n.min(data.len()), false),
        };
        if n > 0 {
            self.handle
                .write_with_u8_array_and_options(&data[..n], &at(off))
                .map_err(|e| js_failure(op.clone(), &e))?;
        }
        if throw {
            return Err(storage_error(op, "fault injection"));
        }
        Err(storage_error(op, "short read/write"))
    }
}
