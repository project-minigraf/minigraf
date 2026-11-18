//! Browser WASM support: `BrowserDb` async façade backed by IndexedDB.
//!
//! This module is only compiled for `wasm32-unknown-unknown` with the `browser`
//! feature enabled. It is **not** compatible with Node.js, Deno, Bun, or any
//! server-side runtime. For server-side Node.js, use `@minigraf/node` (Phase 8.3).

/// Synchronous in-memory page buffer with dirty-page tracking.
pub mod buffer;
#[cfg(test)]
mod flush_tests;
/// Async IndexedDB backend for browser WASM persistence.
pub mod indexeddb;

use crate::browser::buffer::BrowserBufferBackend;
use crate::browser::indexeddb::IndexedDbBackend;
use crate::error::{ErrorCode, err_coded};
use crate::graph::FactStorage;
use crate::query::datalog::executor::{DatalogExecutor, QueryResult};
use crate::query::datalog::functions::FunctionRegistry;
use crate::query::datalog::parser::parse_datalog_command;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::DatalogCommand;
use crate::storage::persistent_facts::PersistentFactStorage;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use wasm_bindgen::prelude::*;

/// Internal state shared by all `BrowserDb` clones.
struct BrowserDbInner {
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
    pfs: PersistentFactStorage<BrowserBufferBackend>,
    /// `None` for in-memory databases (no IDB backing).
    idb: Option<IndexedDbBackend>,
    /// First failed flush, independent of the storage replaced by import.
    failed_flush: Option<u64>,
    /// Only the last outstanding barrier; settled batches are released.
    flush_tail: Option<(u64, js_sys::Promise)>,
    flush_sequence: u64,
}

impl BrowserDbInner {
    fn refuse_failed_flush(&self) -> Result<(), JsValue> {
        if self.failed_flush.is_some() {
            return Err(to_js_error(err_coded!(ErrorCode::Stg046)));
        }
        Ok(())
    }

    fn fail_flush(&mut self, sequence: u64) {
        self.failed_flush = Some(self.failed_flush.map_or(sequence, |old| old.min(sequence)));
    }
}

/// Memory was applied; a preparation error keeps its engine code and fails
/// the handle, unlike validation errors before application.
struct FlushWork(Result<Vec<(u64, Vec<u8>)>, JsValue>);

/// Browser-only Minigraf database handle backed by IndexedDB.
///
/// Writes, checkpoint and open return `Promise`s. Use `await` in JavaScript.
///
/// IndexedDB flushes are atomic and ordered. A failed flush rejects with
/// `[STG-048]`; page-preparation errors keep their engine code. Later writes,
/// checkpoints and imports reject with `[STG-046]`. Reads, rules and export
/// still work, including rejected work already applied in memory. Discard the
/// failed handle and reopen to recover the successful prefix before failure.
///
/// **Not compatible with Node.js.** Use `@minigraf/node` for server-side use.
#[wasm_bindgen]
pub struct BrowserDb {
    inner: Rc<RefCell<BrowserDbInner>>,
}

#[wasm_bindgen]
impl BrowserDb {
    /// Open an in-memory database (no IndexedDB — for testing only).
    ///
    /// Data is lost when the page is closed. Use `BrowserDb.open()` for persistence.
    #[wasm_bindgen(js_name = openInMemory)]
    pub fn open_in_memory() -> Result<BrowserDb, JsValue> {
        let buffer = BrowserBufferBackend::new();
        // Page cache capacity 0: `BrowserBufferBackend` is already an in-memory
        // page store, so the LRU cache would only duplicate pages and add
        // HashMap overhead for zero benefit. See #275.
        let pfs = PersistentFactStorage::new(buffer, 0).map_err(to_js_error)?;
        let fact_storage = pfs.storage().clone();

        Ok(BrowserDb {
            inner: Rc::new(RefCell::new(BrowserDbInner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                pfs,
                idb: None,
                failed_flush: None,
                flush_tail: None,
                flush_sequence: 0,
            })),
        })
    }

    /// Open or create a database backed by IndexedDB.
    ///
    /// `db_name` is used as both the IndexedDB database name and object store name.
    /// Called as `await BrowserDb.open("mydb")` — NOT `new BrowserDb()`.
    #[wasm_bindgen(js_name = open)]
    pub async fn open(db_name: &str) -> Result<BrowserDb, JsValue> {
        let idb = IndexedDbBackend::open(db_name).await?;
        let existing = idb.load_all_pages().await?;

        let buffer = BrowserBufferBackend::load_pages(existing);
        // Page cache capacity 0 — see comment in `open_in_memory` above.
        let pfs = PersistentFactStorage::new(buffer, 0).map_err(to_js_error)?;
        let fact_storage = pfs.storage().clone();

        Ok(BrowserDb {
            inner: Rc::new(RefCell::new(BrowserDbInner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                pfs,
                idb: Some(idb),
                failed_flush: None,
                flush_tail: None,
                flush_sequence: 0,
            })),
        })
    }

    /// Execute a Datalog command string and return a JSON-encoded result.
    ///
    /// In IndexedDB mode, a failed flush rejects with `[STG-048]` and later
    /// transact/retract calls reject with `[STG-046]` without changing memory.
    /// Queries and rules remain available; discard the handle and reopen to
    /// recover. Page-preparation failures retain their original engine code.
    ///
    /// Returns a `Promise<string>` in JavaScript. The JSON shape is:
    /// - Query: `{"variables": [...], "results": [[...], ...]}`
    /// - Transact: `{"transacted": <tx_id>}`
    /// - Retract: `{"retracted": <tx_id>}`
    /// - Rule: `{"ok": true}`
    #[wasm_bindgen(js_name = execute)]
    pub async fn execute(&self, datalog: String) -> Result<String, JsValue> {
        let cmd = parse_datalog_command(&datalog).map_err(to_js_error)?;

        // Peek at the discriminant before consuming `cmd`.
        let is_read = matches!(cmd, DatalogCommand::Query(_) | DatalogCommand::Rule(_));

        if is_read {
            let result = {
                let inner = self.inner.borrow();
                DatalogExecutor::new_with_rules_and_functions(
                    inner.fact_storage.clone(),
                    inner.rules.clone(),
                    inner.functions.clone(),
                )
                .execute(cmd)
                .map_err(to_js_error)?
            };
            return Ok(query_result_to_json(result));
        }

        self.inner.borrow().refuse_failed_flush()?;
        match cmd {
            DatalogCommand::Transact(tx) => {
                let facts =
                    crate::db::Minigraf::materialize_transaction(&tx).map_err(to_js_error)?;
                self.apply_write(facts, false).await
            }
            DatalogCommand::Retract(tx) => {
                let facts =
                    crate::db::Minigraf::materialize_retraction(&tx).map_err(to_js_error)?;
                self.apply_write(facts, true).await
            }
            // Handled above; unreachable but required for exhaustiveness.
            DatalogCommand::Query(_) | DatalogCommand::Rule(_) => unreachable!(),
        }
    }

    /// Flush all dirty pages to IndexedDB.
    ///
    /// Waits for every IndexedDB write applied before this call, even if it
    /// has no pages of its own. Later writes are outside this boundary. Its
    /// own flush failure is `[STG-048]`; an earlier failed flush is `[STG-046]`.
    /// Discard the handle and reopen to recover. With no earlier work or dirty
    /// pages it resolves immediately. No-op for in-memory databases.
    pub async fn checkpoint(&self) -> Result<(), JsValue> {
        let work = {
            let mut inner = self.inner.borrow_mut();
            inner.refuse_failed_flush()?;
            FlushWork(
                inner
                    .pfs
                    .save()
                    .map_err(to_js_error)
                    .and_then(|()| take_dirty_pages(&mut inner.pfs)),
            )
        };
        wasm_bindgen_futures::JsFuture::from(self.flush(work)).await?;
        Ok(())
    }

    /// Serialise the current database to a portable `.graph` blob.
    ///
    /// The blob is byte-for-bit compatible with native `.graph` files opened by
    /// `Minigraf::open()`. Pages are always in ascending `page_id` order.
    /// On a failed IndexedDB handle this includes rejected work already applied
    /// in memory; export does not acknowledge durability.
    ///
    /// Call `checkpoint()` on native before importing a file here to ensure
    /// no WAL entries are missing from the main file.
    #[wasm_bindgen(js_name = exportGraph)]
    pub fn export_graph(&self) -> Result<js_sys::Uint8Array, JsValue> {
        let inner = self.inner.borrow();
        // page_count_raw returns the HashMap's usize length widened to u64; this round-trips.
        #[allow(clippy::cast_possible_truncation)]
        let page_count = inner
            .pfs
            .with_backend(|b| b.page_count_raw())
            .map_err(to_js_error)? as usize;

        let mut blob = Vec::with_capacity(page_count * crate::storage::PAGE_SIZE);
        for id in 0..page_count as u64 {
            let page = inner
                .pfs
                .with_backend(|b| b.read_page_raw(id))
                .map_err(to_js_error)?;
            blob.extend_from_slice(&page);
        }
        Ok(js_sys::Uint8Array::from(blob.as_slice()))
    }

    /// Replace the current database with a `.graph` blob.
    ///
    /// The blob must be a checkpointed native `.graph` file (no pending WAL sidecar).
    /// All existing data is overwritten. After import, the new data is immediately
    /// queryable and all dirty pages are flushed to IndexedDB.
    /// A flush failure is `[STG-048]`: memory retains the imported graph, while
    /// IndexedDB retains the previous graph. Later durable work is `[STG-046]`,
    /// including imports (no live swap), until the handle is discarded and reopened.
    #[wasm_bindgen(js_name = importGraph)]
    pub async fn import_graph(&self, data: js_sys::Uint8Array) -> Result<(), JsValue> {
        self.inner.borrow().refuse_failed_flush()?;
        let bytes = data.to_vec();
        if !bytes.len().is_multiple_of(crate::storage::PAGE_SIZE) {
            return Err(JsValue::from_str(
                "import data length is not a multiple of PAGE_SIZE",
            ));
        }

        let mut pages = std::collections::HashMap::new();
        for (i, chunk) in bytes.chunks(crate::storage::PAGE_SIZE).enumerate() {
            pages.insert(i as u64, chunk.to_vec());
        }

        // ── Sync section ──────────────────────────────────────────────────────────
        let work = {
            let mut inner = self.inner.borrow_mut();
            let buffer = BrowserBufferBackend::load_pages_all_dirty(pages);
            // Page cache capacity 0 — see comment in `open_in_memory` above.
            let new_pfs = PersistentFactStorage::new(buffer, 0).map_err(to_js_error)?;
            let new_fact_storage = new_pfs.storage().clone();

            // Preserve the memory contract even if flush preparation fails.
            // The failed-flush state belongs to the handle, not this new pfs.
            inner.pfs = new_pfs;
            inner.fact_storage = new_fact_storage;

            FlushWork(take_dirty_pages(&mut inner.pfs))
        };
        // ── Borrow dropped ────────────────────────────────────────────────────────

        wasm_bindgen_futures::JsFuture::from(self.flush(work)).await?;
        Ok(())
    }
}

impl BrowserDb {
    /// Reserve the durability boundary before deferring its work. Barriers
    /// always resolve; the operation itself reports any error, so an internal
    /// predecessor cannot cause an unhandled promise rejection.
    fn flush(&self, work: FlushWork) -> js_sys::Promise {
        let mut inner = self.inner.borrow_mut();
        let Some(idb) = inner.idb.as_ref().map(IndexedDbBackend::clone_handle) else {
            return match work.0 {
                Ok(_) => js_sys::Promise::resolve(&JsValue::UNDEFINED),
                Err(error) => js_sys::Promise::reject(&error),
            };
        };
        inner.flush_sequence += 1;
        let sequence = inner.flush_sequence;
        if work.0.is_err() {
            // Fail admission now, but let the earlier successful prefix finish.
            inner.fail_flush(sequence);
        }
        let previous = inner.flush_tail.take();
        if previous.is_none() && work.0.as_ref().is_ok_and(Vec::is_empty) {
            return js_sys::Promise::resolve(&JsValue::UNDEFINED);
        }
        let mut release = None;
        let barrier = js_sys::Promise::new(&mut |resolve, _| release = Some(resolve));
        inner.flush_tail = Some((sequence, barrier));
        let db = self.inner.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            if let Some((_, previous)) = previous {
                let _ = wasm_bindgen_futures::JsFuture::from(previous).await;
            }
            let result = async {
                if db
                    .borrow()
                    .failed_flush
                    .is_some_and(|failed| failed < sequence)
                {
                    return Err(to_js_error(err_coded!(ErrorCode::Stg046)));
                }
                let pages = work.0?;
                idb.write_pages(pages)
                    .await
                    .inspect_err(|_| db.borrow_mut().fail_flush(sequence))
            }
            .await;
            let mut inner = db.borrow_mut();
            if inner
                .flush_tail
                .as_ref()
                .is_some_and(|(last, _)| *last == sequence)
            {
                inner.flush_tail.take();
            }
            drop(inner);
            if let Some(release) = release {
                let _ = release.call0(&JsValue::NULL);
            }
            result.map(|()| JsValue::UNDEFINED)
        })
    }

    /// Apply a batch of pre-materialized facts to the in-memory store and
    /// flush dirty pages to IndexedDB (if present).
    ///
    /// The `RefCell` borrow is fully released before the `.await` so that no
    /// borrow is held across the async boundary.
    async fn apply_write(
        &self,
        facts: Vec<crate::graph::types::Fact>,
        is_retract: bool,
    ) -> Result<String, JsValue> {
        use crate::db::VALID_FROM_USE_TX_TIME;
        use crate::graph::types::tx_id_now;

        // ── Sync section: hold borrow, do ALL sync work, collect owned data ──
        let (work, result_json) = {
            let mut inner = self.inner.borrow_mut();

            let tx_count = inner.fact_storage.allocate_tx_count();
            let tx_id = tx_id_now();

            let stamped: Vec<crate::graph::types::Fact> = facts
                .into_iter()
                .map(|mut f| {
                    f.tx_id = tx_id;
                    f.tx_count = tx_count;
                    if f.asserted && f.valid_from == VALID_FROM_USE_TX_TIME {
                        f.valid_from = tx_id.cast_signed();
                    }
                    f
                })
                .collect();

            let work = FlushWork((|| {
                // One batch, so no query sees part of the transaction (#554).
                inner
                    .fact_storage
                    .load_facts(stamped)
                    .map_err(to_js_error)?;
                inner.pfs.mark_dirty();
                inner.pfs.save().map_err(to_js_error)?;
                take_dirty_pages(&mut inner.pfs)
            })());

            let json = if is_retract {
                format!(r#"{{"retracted":{}}}"#, tx_id)
            } else {
                format!(r#"{{"transacted":{}}}"#, tx_id)
            };

            (work, json)
        };
        // ── Borrow dropped here ───────────────────────────────────────────────

        // ── Async section: flush to IDB (no RefCell borrow held) ─────────────
        wasm_bindgen_futures::JsFuture::from(self.flush(work)).await?;

        Ok(result_json)
    }
}

// ── JSON serialisation helpers (free functions, not exported to WASM) ────────

/// The bytes of every page the last `save()` dirtied, clearing the dirty set.
/// A page that cannot be read is an error, never a page silently left out of
/// the flush to IndexedDB.
fn take_dirty_pages(
    pfs: &mut PersistentFactStorage<BrowserBufferBackend>,
) -> Result<Vec<(u64, Vec<u8>)>, JsValue> {
    let ids = pfs.with_backend_mut(|b| b.take_dirty());
    pfs.with_backend(|b| {
        ids.into_iter()
            .map(|id| b.read_page_raw(id).map(|d| (id, d)))
            .collect::<anyhow::Result<Vec<_>>>()
    })
    .map_err(to_js_error)
}

/// Every error a `BrowserDb` method rejects with goes through this, so the
/// message carries its code (`[PRS-001] ...`), as on every other API (#673).
fn to_js_error(e: anyhow::Error) -> JsValue {
    JsValue::from_str(&crate::MinigrafError::from(e).to_string())
}

fn query_result_to_json(result: QueryResult) -> String {
    use serde_json::{Value as JVal, json};

    let val: JVal = match result {
        QueryResult::Transacted(tx_id) => {
            json!({"transacted": tx_id})
        }
        QueryResult::Retracted(tx_id) => {
            json!({"retracted": tx_id})
        }
        QueryResult::Ok => json!({"ok": true}),
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<JVal>> = results
                .iter()
                .map(|row| row.iter().map(value_to_json).collect())
                .collect();
            json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

fn value_to_json(v: &crate::graph::types::Value) -> serde_json::Value {
    use crate::graph::types::Value;
    use serde_json::Value as JVal;
    match v {
        Value::String(s) => JVal::String(s.clone()),
        Value::Integer(i) => JVal::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(JVal::Number)
            .unwrap_or(JVal::Null),
        Value::Boolean(b) => JVal::Bool(*b),
        Value::Ref(uuid) => JVal::String(uuid.to_string()),
        Value::Keyword(k) => JVal::String(k.clone()),
        Value::Null => JVal::Null,
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser", test))]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn take_dirty_pages_returns_written_pages() {
        use crate::storage::StorageBackend;
        let mut pfs = PersistentFactStorage::new(BrowserBufferBackend::new(), 0).unwrap();
        let _ = pfs.with_backend_mut(|b| b.take_dirty());
        pfs.with_backend_mut(|b| b.write_page(3, &[7u8; crate::storage::PAGE_SIZE]))
            .unwrap();
        let pages = take_dirty_pages(&mut pfs).expect("dirty pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].0, 3);
        assert!(take_dirty_pages(&mut pfs).expect("second call").is_empty());
    }

    #[wasm_bindgen_test]
    fn unreadable_dirty_page_is_an_error_not_skipped() {
        let mut pfs = PersistentFactStorage::new(BrowserBufferBackend::new(), 0).unwrap();
        // A dirty id with no page behind it: before the fix it was dropped
        // from the flush and the caller still saw success.
        pfs.with_backend_mut(|b| b.mark_dirty_for_test(99));
        assert!(take_dirty_pages(&mut pfs).is_err());
    }

    // #673: a rejection carries the same error code as the native API.
    #[wasm_bindgen_test]
    async fn execute_errors_carry_their_code() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        for (datalog, code) in [
            ("(nosuchcommand [])", "[PRS-011] "),
            (
                "(query [:find ?x :where [?e :a ?x] [(nosuchfn ?x)]])",
                "[QRY-007] ",
            ),
            ("(transact [[:a :b]])", "[PRS-046] "),
        ] {
            let err = db
                .execute(datalog.to_string())
                .await
                .expect_err("invalid command must reject");
            let text = err.as_string().expect("rejection is a string");
            assert!(
                text.starts_with(code),
                "rejection does not start with {code}"
            );
        }
    }

    #[wasm_bindgen_test]
    async fn in_memory_transact_and_query() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        let transact_result = db
            .execute(r#"(transact [[:alice :name "Alice"] [:alice :age 30]])"#.to_string())
            .await
            .expect("transact");
        let v: serde_json::Value = serde_json::from_str(&transact_result).unwrap();
        assert!(v.get("transacted").is_some());

        let query_result = db
            .execute(r#"(query [:find ?name :where [:alice :name ?name]])"#.to_string())
            .await
            .expect("query");
        let v: serde_json::Value = serde_json::from_str(&query_result).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0][0], serde_json::Value::String("Alice".into()));
    }

    #[wasm_bindgen_test]
    async fn empty_query_returns_empty_results() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        let result = db
            .execute(r#"(query [:find ?e :where [?e :name _]])"#.to_string())
            .await
            .expect("query");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["results"].as_array().unwrap().len(), 0);
    }

    #[wasm_bindgen_test]
    async fn export_import_round_trip() {
        let db = BrowserDb::open_in_memory().expect("open");
        db.execute(r#"(transact [[:bob :role "admin"]])"#.to_string())
            .await
            .expect("transact");

        let blob = db.export_graph().expect("export");
        let bytes = blob.to_vec();
        assert_eq!(
            &bytes[0..4],
            b"MGRF",
            "exported blob must start with MGRF magic"
        );

        let db2 = BrowserDb::open_in_memory().expect("open2");
        db2.import_graph(blob).await.expect("import");

        let result = db2
            .execute(r#"(query [:find ?role :where [:bob :role ?role]])"#.to_string())
            .await
            .expect("query after import");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0][0], serde_json::Value::String("admin".into()));
    }

    #[wasm_bindgen_test]
    async fn export_size_is_page_aligned() {
        let db = BrowserDb::open_in_memory().expect("open");
        db.execute(r#"(transact [[:e :v 1]])"#.to_string())
            .await
            .expect("transact");
        let blob = db.export_graph().expect("export");
        assert_eq!(blob.byte_length() as usize % crate::storage::PAGE_SIZE, 0);
    }

    /// Load the committed binary fixture (produced by `cargo run --example
    /// generate_compat_fixture` from the native build) into a `BrowserDb` via
    /// `import_graph` and verify the known facts are queryable.
    ///
    /// This is the **native → browser** direction of the cross-platform
    /// compatibility test.  The companion native side lives in
    /// `tests/cross_platform_compat_test.rs`.
    #[wasm_bindgen_test]
    async fn native_fixture_readable_by_browser_db() {
        let fixture: &[u8] = include_bytes!("../../tests/fixtures/compat.graph");
        let db = BrowserDb::open_in_memory().expect("open in-memory");
        let arr = js_sys::Uint8Array::from(fixture);
        db.import_graph(arr).await.expect("import native fixture");

        let r = db
            .execute(r#"(query [:find ?name :where [?e :name ?name]])"#.to_string())
            .await
            .expect("query name");
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(
            results.len(),
            1,
            "expected 1 name result from native fixture"
        );
        assert_eq!(results[0][0], serde_json::Value::String("Alice".into()));

        let r2 = db
            .execute("(query [:find ?age :where [?e :age ?age]])".to_string())
            .await
            .expect("query age");
        let v2: serde_json::Value = serde_json::from_str(&r2).unwrap();
        let results2 = v2["results"].as_array().unwrap();
        assert_eq!(
            results2.len(),
            1,
            "expected 1 age result from native fixture"
        );
        assert_eq!(results2[0][0], serde_json::Value::Number(30.into()));
    }

    /// #275: `BrowserBufferBackend` is already an in-memory page store, so the
    /// `PersistentFactStorage` LRU page cache is redundant on top of it and
    /// should be disabled (capacity 0) for all three `BrowserDb` constructors.
    #[wasm_bindgen_test]
    async fn open_in_memory_disables_page_cache() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        assert_eq!(db.inner.borrow().pfs.page_cache_capacity(), 0);
    }

    #[wasm_bindgen_test]
    async fn open_disables_page_cache() {
        let db = BrowserDb::open("minigraf-test-zero-page-cache-open")
            .await
            .expect("open");
        assert_eq!(db.inner.borrow().pfs.page_cache_capacity(), 0);
    }

    #[wasm_bindgen_test]
    async fn import_graph_disables_page_cache() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        db.execute(r#"(transact [[:e :v 1]])"#.to_string())
            .await
            .expect("transact");
        let blob = db.export_graph().expect("export");

        let db2 = BrowserDb::open_in_memory().expect("open_in_memory 2");
        db2.import_graph(blob).await.expect("import");
        assert_eq!(db2.inner.borrow().pfs.page_cache_capacity(), 0);
    }

    #[wasm_bindgen_test]
    async fn idb_persistence_round_trip() {
        let db_name = "minigraf-test-persistence";

        let db1 = BrowserDb::open(db_name).await.expect("open db1");
        db1.execute(r#"(transact [[:carol :dept "eng"]])"#.to_string())
            .await
            .expect("transact");
        drop(db1);

        let db2 = BrowserDb::open(db_name).await.expect("open db2");
        let result = db2
            .execute(r#"(query [:find ?dept :where [:carol :dept ?dept]])"#.to_string())
            .await
            .expect("query after reopen");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0][0], serde_json::Value::String("eng".into()));
    }
}
