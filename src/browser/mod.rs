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
        let pfs = PersistentFactStorage::new(buffer, 256)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
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
        let pfs = PersistentFactStorage::new(buffer, 256)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
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
        let cmd = parse_datalog_command(&datalog)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

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

    /// Flush all dirty pages to IndexedDB.
    ///
    /// Write-through means individual `execute()` calls already flush dirty pages,
    /// so `checkpoint()` is only needed after `import_graph()` or explicit bulk ops.
    /// No-op for in-memory databases.
    pub async fn checkpoint(&self) -> Result<(), JsValue> {
        let (dirty_pages, has_idb) = {
            let mut inner = self.inner.borrow_mut();
            inner.pfs.save()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let dirty_ids = inner.pfs.with_backend_mut(|b| b.take_dirty());
            let pages: Vec<(u64, Vec<u8>)> = dirty_ids
                .into_iter()
                .filter_map(|id| {
                    inner.pfs.with_backend(|b| b.read_page_raw(id).ok().map(|d| (id, d)))
                })
                .collect();
            (pages, inner.idb.is_some())
        };

        if has_idb && !dirty_pages.is_empty() {
            let idb = self.inner.borrow().idb.as_ref().unwrap().clone_handle();
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
        let page_count = inner.pfs.with_backend(|b| b.page_count_raw())
            .map_err(|e| JsValue::from_str(&e.to_string()))? as usize;

        let mut blob = Vec::with_capacity(page_count * crate::storage::PAGE_SIZE);
        for id in 0..page_count as u64 {
            let page = inner.pfs.with_backend(|b| b.read_page_raw(id))
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
        if bytes.len() % crate::storage::PAGE_SIZE != 0 {
            return Err(JsValue::from_str("import data length is not a multiple of PAGE_SIZE"));
        }

        let mut pages = std::collections::HashMap::new();
        for (i, chunk) in bytes.chunks(crate::storage::PAGE_SIZE).enumerate() {
            pages.insert(i as u64, chunk.to_vec());
        }

        // ── Sync section ──────────────────────────────────────────────────────────
        let (dirty_pages, has_idb) = {
            let mut inner = self.inner.borrow_mut();
            let buffer = BrowserBufferBackend::load_pages_all_dirty(pages);
            let mut new_pfs = PersistentFactStorage::new(buffer, 256)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let new_fact_storage = new_pfs.storage().clone();

            // Drain dirty set and collect owned page bytes before swapping inner.
            let dirty_ids = new_pfs.with_backend_mut(|b| b.take_dirty());
            let dirty_pages: Vec<(u64, Vec<u8>)> = dirty_ids
                .into_iter()
                .filter_map(|id| {
                    new_pfs.with_backend(|b| b.read_page_raw(id).ok().map(|d| (id, d)))
                })
                .collect();

            inner.pfs = new_pfs;
            inner.fact_storage = new_fact_storage;

            (dirty_pages, inner.idb.is_some())
        };
        // ── Borrow dropped ────────────────────────────────────────────────────────

        if has_idb && !dirty_pages.is_empty() {
            let idb = self.inner.borrow().idb.as_ref().unwrap().clone_handle();
            idb.write_pages(dirty_pages).await?;
        }
        Ok(())
    }
}

impl BrowserDb {
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

            let tx_count = inner.fact_storage.allocate_tx_count();
            let tx_id = tx_id_now();

            let stamped: Vec<crate::graph::types::Fact> = facts
                .into_iter()
                .map(|mut f| {
                    f.tx_id = tx_id;
                    f.tx_count = tx_count;
                    if f.asserted && f.valid_from == VALID_FROM_USE_TX_TIME {
                        f.valid_from = tx_id as i64;
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
            let dirty_ids = inner.pfs.with_backend_mut(|b| b.take_dirty());
            let dirty_pages: Vec<(u64, Vec<u8>)> = dirty_ids
                .into_iter()
                .filter_map(|id| {
                    inner
                        .pfs
                        .with_backend(|b| b.read_page_raw(id).ok().map(|d| (id, d)))
                })
                .collect();

            let json = if is_retract {
                format!(r#"{{"retracted":{}}}"#, tx_id)
            } else {
                format!(r#"{{"transacted":{}}}"#, tx_id)
            };

            (dirty_pages, json)
        };
        // ── Borrow dropped here ───────────────────────────────────────────────

        // ── Async section: flush to IDB (no RefCell borrow held) ─────────────
        if !dirty_pages.is_empty() {
            let has_idb = self.inner.borrow().idb.is_some();
            if has_idb {
                let idb = self
                    .inner
                    .borrow()
                    .idb
                    .as_ref()
                    .unwrap()
                    .clone_handle();
                idb.write_pages(dirty_pages).await?;
            }
        }

        Ok(result_json)
    }
}

// ── JSON serialisation helpers (free functions, not exported to WASM) ────────

fn query_result_to_json(result: QueryResult) -> String {
    use serde_json::{Value as JVal, json};

    let val: JVal = match result {
        QueryResult::Transacted(tx_id) => json!({"transacted": tx_id}),
        QueryResult::Retracted(tx_id) => json!({"retracted": tx_id}),
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
        assert_eq!(&bytes[0..4], b"MGRF", "exported blob must start with MGRF magic");

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
