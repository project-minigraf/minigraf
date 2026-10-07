//! Browser WASM support: `BrowserDb` async façade backed by IndexedDB.
//!
//! This module is only compiled for `wasm32-unknown-unknown` with the `browser`
//! feature enabled. It is **not** compatible with Node.js, Deno, Bun, or any
//! server-side runtime. For server-side Node.js, use `@minigraf/node` (Phase 8.3).

/// Synchronous in-memory page buffer with dirty-page tracking.
pub mod buffer;
/// Async IndexedDB backend for browser WASM persistence.
pub mod indexeddb;

use crate::browser::buffer::BrowserBufferBackend;
use crate::browser::indexeddb::IndexedDbBackend;
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
}

/// Browser-only Minigraf database handle backed by IndexedDB.
///
/// All public methods return `Promise`s. Use `await` in JavaScript.
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
        let pfs =
            PersistentFactStorage::new(buffer, 0).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let fact_storage = pfs.storage().clone();

        Ok(BrowserDb {
            inner: Rc::new(RefCell::new(BrowserDbInner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                pfs,
                idb: None,
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
        let pfs =
            PersistentFactStorage::new(buffer, 0).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let fact_storage = pfs.storage().clone();

        Ok(BrowserDb {
            inner: Rc::new(RefCell::new(BrowserDbInner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                pfs,
                idb: Some(idb),
            })),
        })
    }

    /// Execute a Datalog command string and return a JSON-encoded result.
    ///
    /// Returns a `Promise<string>` in JavaScript. The JSON shape is:
    /// - Query: `{"variables": [...], "results": [[...], ...]}`
    /// - Transact: `{"transacted": <tx_id>}`
    /// - Retract: `{"retracted": <tx_id>}`
    /// - Rule: `{"ok": true}`
    #[wasm_bindgen(js_name = execute)]
    pub async fn execute(&self, datalog: String) -> Result<String, JsValue> {
        let cmd = parse_datalog_command(&datalog).map_err(|e| JsValue::from_str(&e.to_string()))?;

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
                .map_err(|e| JsValue::from_str(&e.to_string()))?
            };
            return Ok(query_result_to_json(result));
        }

        match cmd {
            DatalogCommand::Transact(tx) => {
                let facts = crate::db::Minigraf::materialize_transaction(&tx)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                self.apply_write(facts, false).await
            }
            DatalogCommand::Retract(tx) => {
                let facts = crate::db::Minigraf::materialize_retraction(&tx)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                self.apply_write(facts, true).await
            }
            // Handled above; unreachable but required for exhaustiveness.
            DatalogCommand::Query(_) | DatalogCommand::Rule(_) => unreachable!(),
        }
    }

    /// Open a cursor over a `(query ...)` and return its rows in batches.
    ///
    /// The answer is fixed when the cursor opens: later writes do not change
    /// it. Synchronous, since reading never touches IndexedDB.
    ///
    /// Throws `[API-012]` for a `transact`, `retract` or `rule`, and
    /// `[API-010]` for a query with `$slot` bind slots.
    #[wasm_bindgen(js_name = query)]
    pub fn query(&self, datalog: &str) -> Result<BrowserCursor, JsValue> {
        let cursor = self.query_inner(datalog).map_err(to_js_error)?;
        Ok(BrowserCursor {
            vars: cursor.vars().to_vec(),
            cursor: Some(cursor),
        })
    }

    /// Flush all dirty pages to IndexedDB.
    ///
    /// Write-through means individual `execute()` calls already flush dirty pages,
    /// so `checkpoint()` is only needed after `import_graph()` or explicit bulk ops.
    /// No-op for in-memory databases.
    pub async fn checkpoint(&self) -> Result<(), JsValue> {
        let (dirty_pages, idb) = {
            let mut inner = self.inner.borrow_mut();
            inner
                .pfs
                .save()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let pages = take_dirty_pages(&mut inner.pfs)?;
            (
                pages,
                inner.idb.as_ref().map(IndexedDbBackend::clone_handle),
            )
        };

        if let Some(idb) = idb
            && !dirty_pages.is_empty()
        {
            idb.write_pages(dirty_pages).await?;
        }
        Ok(())
    }

    /// Serialise the current database to a portable `.graph` blob.
    ///
    /// The blob is byte-for-bit compatible with native `.graph` files opened by
    /// `Minigraf::open()`. Pages are always in ascending `page_id` order.
    ///
    /// Call `checkpoint()` on native before importing a file here to ensure
    /// no WAL entries are missing from the main file.
    #[wasm_bindgen(js_name = exportGraph)]
    pub fn export_graph(&self) -> Result<js_sys::Uint8Array, JsValue> {
        let inner = self.inner.borrow();
        let page_count = inner
            .pfs
            .with_backend(|b| b.page_count_raw())
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let capacity = usize::try_from(page_count)
            .ok()
            .and_then(|n| n.checked_mul(crate::storage::PAGE_SIZE))
            .ok_or_else(|| JsValue::from_str("database is too large to export"))?;

        let mut blob = Vec::with_capacity(capacity);
        for id in 0..page_count {
            let page = inner
                .pfs
                .with_backend(|b| b.read_page_raw(id))
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            blob.extend_from_slice(&page);
        }
        Ok(js_sys::Uint8Array::from(blob.as_slice()))
    }

    /// Replace the current database with a `.graph` blob.
    ///
    /// The blob must be a checkpointed native `.graph` file (no pending WAL sidecar).
    /// All existing data is overwritten. After import, the new data is immediately
    /// queryable and all dirty pages are flushed to IndexedDB.
    #[wasm_bindgen(js_name = importGraph)]
    pub async fn import_graph(&self, data: js_sys::Uint8Array) -> Result<(), JsValue> {
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
        let (dirty_pages, idb) = {
            let mut inner = self.inner.borrow_mut();
            let buffer = BrowserBufferBackend::load_pages_all_dirty(pages);
            // Page cache capacity 0 — see comment in `open_in_memory` above.
            let mut new_pfs = PersistentFactStorage::new(buffer, 0)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let new_fact_storage = new_pfs.storage().clone();

            // Drain dirty set and collect owned page bytes before swapping inner.
            let dirty_pages = take_dirty_pages(&mut new_pfs)?;

            inner.pfs = new_pfs;
            inner.fact_storage = new_fact_storage;

            (
                dirty_pages,
                inner.idb.as_ref().map(IndexedDbBackend::clone_handle),
            )
        };
        // ── Borrow dropped ────────────────────────────────────────────────────────

        if let Some(idb) = idb
            && !dirty_pages.is_empty()
        {
            idb.write_pages(dirty_pages).await?;
        }
        Ok(())
    }
}

impl BrowserDb {
    fn query_inner(&self, datalog: &str) -> anyhow::Result<crate::Cursor> {
        use crate::error::{ErrorCode, bail_coded};

        let cmd = parse_datalog_command(datalog)?;
        match &cmd {
            DatalogCommand::Query(q) => crate::query::datalog::prepared::reject_unbound_slots(q)?,
            DatalogCommand::Transact(_) => bail_coded!(ErrorCode::Api012, "transact"),
            DatalogCommand::Retract(_) => bail_coded!(ErrorCode::Api012, "retract"),
            DatalogCommand::Rule(_) => bail_coded!(ErrorCode::Api012, "rule"),
        }
        let result = {
            let inner = self.inner.borrow();
            DatalogExecutor::new_with_rules_and_functions(
                inner.fact_storage.clone(),
                inner.rules.clone(),
                inner.functions.clone(),
            )
            .execute(cmd)?
        };
        crate::Cursor::from_result(result)
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
        let (dirty_pages, result_json) = {
            let mut inner = self.inner.borrow_mut();

            // Before allocating: a rejected transaction takes no tx_count (#435).
            crate::graph::storage::check_one_window_per_triple(&facts)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
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

            for fact in &stamped {
                inner
                    .fact_storage
                    .load_fact(fact.clone())
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
            }

            inner.pfs.mark_dirty();
            inner
                .pfs
                .save()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;

            // Collect dirty pages as owned Vec<(u64, Vec<u8>)> — no borrows escape
            let dirty_pages = take_dirty_pages(&mut inner.pfs)?;

            let json = if is_retract {
                format!(r#"{{"retracted":{}}}"#, tx_id)
            } else {
                format!(r#"{{"transacted":{}}}"#, tx_id)
            };

            (dirty_pages, json)
        };
        // ── Borrow dropped here ───────────────────────────────────────────────

        // ── Async section: flush to IDB (no RefCell borrow held) ─────────────
        let idb = self
            .inner
            .borrow()
            .idb
            .as_ref()
            .map(IndexedDbBackend::clone_handle);
        if let Some(idb) = idb
            && !dirty_pages.is_empty()
        {
            idb.write_pages(dirty_pages).await?;
        }

        Ok(result_json)
    }
}

// ── BrowserCursor ───────────────────────────────────────────────────────────

/// A cursor from `BrowserDb.query()`: the rows of one query answer, in batches.
///
/// ```js
/// const cursor = db.query("(query [:find ?n :where [?e :name ?n]])");
/// try {
///   let batch;
///   while ((batch = cursor.nextBatch(1000)) !== undefined) {
///     for (const row of JSON.parse(batch)) console.log(row);
///   }
/// } finally {
///   cursor.close();
/// }
/// ```
#[wasm_bindgen]
pub struct BrowserCursor {
    vars: Vec<String>,
    /// `None` after `close()`.
    cursor: Option<crate::Cursor>,
}

#[wasm_bindgen]
impl BrowserCursor {
    /// The query's `:find` variables, in column order.
    pub fn vars(&self) -> Vec<String> {
        self.vars.clone()
    }

    /// The next batch of at most `maxRows` rows (`0` counts as 1), as a JSON
    /// array of rows encoded like `execute()`'s `results`, or `undefined` at
    /// the end. A batch is never empty. After `close()` it returns
    /// `undefined`.
    #[wasm_bindgen(js_name = nextBatch)]
    pub fn next_batch(&mut self, max_rows: u32) -> Result<Option<String>, JsValue> {
        let Some(cursor) = self.cursor.as_mut() else {
            return Ok(None);
        };
        let batch = cursor
            .next_batch(max_rows as usize)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(batch.map(|b| rows_to_json(b.rows())))
    }

    /// Release the cursor's rows. Later `nextBatch()` calls return `undefined`.
    pub fn close(&mut self) {
        self.cursor = None;
    }
}

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

fn to_js_error(e: anyhow::Error) -> JsValue {
    JsValue::from_str(&crate::MinigrafError::from(e).to_string())
}

// ── JSON serialisation helpers (free functions, not exported to WASM) ────────

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

fn rows_to_json(rows: &[Vec<crate::graph::types::Value>]) -> String {
    let rows: Vec<Vec<serde_json::Value>> = rows
        .iter()
        .map(|row| row.iter().map(value_to_json).collect())
        .collect();
    serde_json::Value::from(rows).to_string()
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
    async fn cursor_batches_match_execute() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        for i in 0..20 {
            db.execute(format!("(transact [[:e{i} :n {i}]])"))
                .await
                .expect("transact");
        }
        let q = "(query [:find ?n :where [?e :n ?n]])";
        let all: serde_json::Value =
            serde_json::from_str(&db.execute(q.to_string()).await.expect("execute")).unwrap();
        let mut expected: Vec<i64> = all["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_i64().unwrap())
            .collect();
        expected.sort_unstable();
        for size in [1u32, 7, 1000] {
            let mut cursor = db.query(q).expect("query");
            assert_eq!(cursor.vars(), vec!["?n".to_string()]);
            let mut got = Vec::new();
            while let Some(batch) = cursor.next_batch(size).expect("next_batch") {
                let rows: Vec<Vec<i64>> = serde_json::from_str(&batch).unwrap();
                assert!(!rows.is_empty() && rows.len() <= size as usize);
                got.extend(rows.into_iter().map(|r| r[0]));
            }
            got.sort_unstable();
            assert_eq!(got, expected, "batch size {size}");
        }
    }

    #[wasm_bindgen_test]
    async fn cursor_is_fixed_at_open_and_closes() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        db.execute(r#"(transact [[:a :n 1]])"#.to_string())
            .await
            .expect("transact");
        let mut cursor = db
            .query("(query [:find ?n :where [?e :n ?n]])")
            .expect("query");
        db.execute(r#"(transact [[:b :n 2]])"#.to_string())
            .await
            .expect("transact");
        assert_eq!(
            cursor.next_batch(10).expect("batch").as_deref(),
            Some("[[1]]")
        );
        assert_eq!(cursor.next_batch(10).expect("end"), None);

        let mut cursor = db
            .query("(query [:find ?n :where [?e :n ?n]])")
            .expect("query");
        assert!(cursor.next_batch(1).expect("batch").is_some());
        cursor.close();
        assert_eq!(cursor.next_batch(1).expect("closed"), None);
    }

    #[wasm_bindgen_test]
    fn cursor_rejects_non_queries() {
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        let err = db
            .query(r#"(transact [[:a :n 1]])"#)
            .err()
            .and_then(|e| e.as_string())
            .expect("error string");
        assert!(err.starts_with("[API-012]"), "expected API-012");
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

        let exported = db.export_graph().expect("export after import");
        let bytes = exported.to_vec();
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            8,
            "v7 fixture must be upgraded to v8 on import"
        );
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
