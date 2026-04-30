# Phase 8.1a: Browser WASM Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `BrowserDb` type that runs Minigraf in browser WASM using IndexedDB for storage, exposed as the `@minigraf/core` npm package.

**Architecture:** A new `BrowserDb` async façade (distinct from `Minigraf`) owns a synchronous `BrowserBufferBackend` (in-memory `HashMap` + dirty `HashSet`) that satisfies the existing `StorageBackend` trait, plus an async `IndexedDbBackend` that mirrors dirty pages to IndexedDB after every write. The Datalog engine (`PersistentFactStorage`, all query/executor code) is reused unchanged. `BrowserDb` uses `Rc<RefCell<...>>` (single-threaded browser WASM).

**Tech Stack:** Rust 2024 edition, `wasm-bindgen 0.2`, `wasm-bindgen-futures 0.4`, `web-sys 0.3`, `js-sys 0.3`, `serde_json 1.0` (optional dep), `wasm-pack` build tool, headless Chrome for integration tests.

**Spec:** `docs/superpowers/specs/2026-04-17-phase-8-1a-browser-wasm-design.md`
**Issue:** project-minigraf/minigraf#129

---

## File Map

| Action | Path | Responsibility |
|--------|------|---------------|
| Modify | `Cargo.toml` | Add `browser` feature + WASM target deps |
| Create | `src/browser/buffer.rs` | `BrowserBufferBackend`: sync `HashMap` pages + dirty `HashSet` |
| Create | `src/browser/indexeddb.rs` | `IndexedDbBackend`: async web-sys IDB open/read/write |
| Create | `src/browser/mod.rs` | `BrowserDb` struct + all `#[wasm_bindgen]` exports |
| Modify | `src/lib.rs` | Cfg-gated `pub mod browser` declaration |
| Modify | `src/storage/backend/mod.rs` | Cfg-gated re-export of `BrowserBufferBackend` |
| Modify | `src/db.rs` | Make `materialize_transaction` / `materialize_retraction` `pub(crate)` |
| Create | `.github/workflows/wasm-browser.yml` | CI: build + headless Chrome tests |
| Create | `examples/browser/index.html` | Demo page (no bundler) |
| Create | `examples/browser/app.js` | Plain JS demo: open → transact → query → log |
| Create | `examples/browser/README.md` | Build and serve instructions |

---

## Task 1: Feature flags and WASM dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add optional deps and `browser` feature**

  Replace the `[features]` and add target-specific deps in `Cargo.toml`:

  ```toml
  [features]
  default = []
  wasm = []
  browser = [
      "dep:wasm-bindgen",
      "dep:wasm-bindgen-futures",
      "dep:web-sys",
      "dep:js-sys",
      "dep:serde_json",
  ]

  [dependencies]
  # ... existing deps unchanged ...
  serde_json = { version = "1.0", optional = true }

  [target.'cfg(target_arch = "wasm32")'.dependencies]
  wasm-bindgen         = { version = "0.2", optional = true }
  wasm-bindgen-futures = { version = "0.4", optional = true }
  js-sys               = { version = "0.3", optional = true }
  web-sys              = { version = "0.3", optional = true, features = [
      "IdbDatabase",
      "IdbFactory",
      "IdbIndex",
      "IdbObjectStore",
      "IdbOpenDbRequest",
      "IdbRequest",
      "IdbTransaction",
      "IdbTransactionMode",
      "Window",
  ] }

  [dev-dependencies]
  # ... existing dev-deps unchanged — serde_json stays here too for tests ...
  ```

  Note: `serde_json` remains in `[dev-dependencies]` as-is. Adding it as an optional
  `[dependencies]` entry means it's available in both test builds (via dev-dep) and
  in the `browser` feature build (via the optional dep).

- [ ] **Step 2: Verify native build still compiles**

  ```bash
  cargo build
  ```

  Expected: compiles with no errors or new warnings. The `browser` feature is not
  active so no WASM deps are compiled.

- [ ] **Step 3: Commit**

  ```bash
  git add Cargo.toml
  git commit -m "feat(browser): add browser feature flag and wasm target dependencies"
  ```

---

## Task 2: `BrowserBufferBackend`

**Files:**
- Create: `src/browser/buffer.rs`

This file has zero WASM dependencies and is fully testable with `cargo test`.

- [ ] **Step 1: Write the failing tests first**

  Create `src/browser/buffer.rs` with only the tests (struct not yet defined):

  ```rust
  use crate::storage::{PAGE_SIZE, StorageBackend};
  use std::collections::{HashMap, HashSet};

  // TODO: struct definition goes here

  #[cfg(test)]
  mod tests {
      use super::*;

      fn page(byte: u8) -> Vec<u8> {
          vec![byte; PAGE_SIZE]
      }

      #[test]
      fn write_marks_dirty() {
          let mut buf = BrowserBufferBackend::new();
          buf.write_page(0, &page(1)).unwrap();
          let dirty = buf.take_dirty();
          assert!(dirty.contains(&0));
      }

      #[test]
      fn take_dirty_clears_set() {
          let mut buf = BrowserBufferBackend::new();
          buf.write_page(0, &page(1)).unwrap();
          let _ = buf.take_dirty();
          assert!(buf.take_dirty().is_empty());
      }

      #[test]
      fn read_after_write_returns_same_bytes() {
          let mut buf = BrowserBufferBackend::new();
          let p = page(42);
          buf.write_page(3, &p).unwrap();
          assert_eq!(buf.read_page(3).unwrap(), p);
      }

      #[test]
      fn page_count_reflects_distinct_ids() {
          let mut buf = BrowserBufferBackend::new();
          buf.write_page(0, &page(0)).unwrap();
          buf.write_page(1, &page(1)).unwrap();
          buf.write_page(0, &page(2)).unwrap(); // overwrite
          assert_eq!(buf.page_count().unwrap(), 2);
      }

      #[test]
      fn load_pages_starts_with_no_dirty() {
          let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
          let mut buf = BrowserBufferBackend::load_pages(pages);
          assert!(buf.take_dirty().is_empty());
      }

      #[test]
      fn load_pages_all_dirty_marks_all() {
          let pages = HashMap::from([(0u64, page(0)), (1u64, page(1))]);
          let mut buf = BrowserBufferBackend::load_pages_all_dirty(pages);
          let dirty = buf.take_dirty();
          assert!(dirty.contains(&0));
          assert!(dirty.contains(&1));
      }

      #[test]
      fn is_new_true_when_empty() {
          assert!(BrowserBufferBackend::new().is_new());
      }

      #[test]
      fn is_new_false_after_write() {
          let mut buf = BrowserBufferBackend::new();
          buf.write_page(0, &page(0)).unwrap();
          assert!(!buf.is_new());
      }

      #[test]
      fn wrong_page_size_errors() {
          let mut buf = BrowserBufferBackend::new();
          assert!(buf.write_page(0, &[0u8; 100]).is_err());
      }

      #[test]
      fn read_missing_page_errors() {
          let buf = BrowserBufferBackend::new();
          assert!(buf.read_page(99).is_err());
      }
  }
  ```

- [ ] **Step 2: Run tests — confirm they fail to compile**

  ```bash
  cargo test -p minigraf browser::buffer 2>&1 | head -20
  ```

  Expected: compile error — `BrowserBufferBackend` not defined.

- [ ] **Step 3: Implement `BrowserBufferBackend`**

  Add the struct and implementations above the `#[cfg(test)]` block in `src/browser/buffer.rs`:

  ```rust
  use crate::storage::{PAGE_SIZE, StorageBackend};
  use anyhow::Result;
  use std::collections::{HashMap, HashSet};

  /// Synchronous in-memory page buffer with dirty-page tracking.
  ///
  /// Implements `StorageBackend` so it can be used with `PersistentFactStorage`.
  /// After `PersistentFactStorage::save()` writes updated pages here, call
  /// `take_dirty()` to retrieve the page IDs that must be flushed to IndexedDB.
  pub struct BrowserBufferBackend {
      pages: HashMap<u64, Vec<u8>>,
      dirty: HashSet<u64>,
  }

  impl BrowserBufferBackend {
      /// Create an empty buffer (new database).
      pub fn new() -> Self {
          Self {
              pages: HashMap::new(),
              dirty: HashSet::new(),
          }
      }

      /// Load pages from an existing snapshot. Dirty set starts empty.
      /// Used during `BrowserDb::open()` after fetching all pages from IndexedDB.
      pub fn load_pages(pages: HashMap<u64, Vec<u8>>) -> Self {
          Self {
              pages,
              dirty: HashSet::new(),
          }
      }

      /// Load pages and mark every page dirty.
      /// Used during `BrowserDb::import_graph()` so all pages are flushed to IDB.
      pub fn load_pages_all_dirty(pages: HashMap<u64, Vec<u8>>) -> Self {
          let dirty: HashSet<u64> = pages.keys().copied().collect();
          Self { pages, dirty }
      }

      /// Drain and return the set of page IDs written since the last call.
      /// Clears the dirty set. Call after `pfs.save()` to get pages to flush.
      pub fn take_dirty(&mut self) -> HashSet<u64> {
          std::mem::take(&mut self.dirty)
      }
  }

  impl Default for BrowserBufferBackend {
      fn default() -> Self {
          Self::new()
      }
  }

  impl StorageBackend for BrowserBufferBackend {
      fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
          if data.len() != PAGE_SIZE {
              anyhow::bail!(
                  "Invalid page size: {} bytes (expected {})",
                  data.len(),
                  PAGE_SIZE
              );
          }
          self.pages.insert(page_id, data.to_vec());
          self.dirty.insert(page_id);
          Ok(())
      }

      fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
          self.pages
              .get(&page_id)
              .cloned()
              .ok_or_else(|| anyhow::anyhow!("Page {} not found", page_id))
      }

      fn sync(&mut self) -> Result<()> {
          Ok(()) // no-op: durability handled by IndexedDbBackend
      }

      fn page_count(&self) -> Result<u64> {
          Ok(self.pages.len() as u64)
      }

      fn close(&mut self) -> Result<()> {
          Ok(()) // no-op
      }

      fn backend_name(&self) -> &'static str {
          "browser-buffer"
      }

      fn is_new(&self) -> bool {
          self.pages.is_empty()
      }
  }
  ```

- [ ] **Step 4: Run tests — confirm they pass**

  ```bash
  cargo test browser::buffer
  ```

  Expected: all 10 tests pass.

- [ ] **Step 5: Commit**

  ```bash
  git add src/browser/buffer.rs
  git commit -m "feat(browser): add BrowserBufferBackend with dirty page tracking"
  ```

---

## Task 3: Module wiring

**Files:**
- Create: `src/browser/mod.rs` (stub only — expanded in later tasks)
- Modify: `src/lib.rs`
- Modify: `src/storage/backend/mod.rs`

- [ ] **Step 1: Create the browser module stub**

  Create `src/browser/mod.rs`:

  ```rust
  //! Browser WASM support: `BrowserDb` async façade backed by IndexedDB.
  //!
  //! This module is only compiled for `wasm32-unknown-unknown` with the `browser`
  //! feature enabled. It is **not** compatible with Node.js or any server-side
  //! runtime. For Node.js, use `@minigraf/node` (Phase 8.3).

  pub mod buffer;
  pub mod indexeddb;
  ```

- [ ] **Step 2: Declare the browser module in `src/lib.rs`**

  Find the module declarations block in `src/lib.rs` and add:

  ```rust
  #[cfg(all(target_arch = "wasm32", feature = "browser"))]
  pub mod browser;
  ```

  Place it after the existing `pub mod db;` line.

- [ ] **Step 3: Re-export `BrowserBufferBackend` from `src/storage/backend/mod.rs`**

  Add at the end of `src/storage/backend/mod.rs`:

  ```rust
  #[cfg(all(target_arch = "wasm32", feature = "browser"))]
  pub use crate::browser::buffer::BrowserBufferBackend;
  ```

- [ ] **Step 4: Confirm native build is clean**

  ```bash
  cargo build && cargo test
  ```

  Expected: all existing tests pass, no new warnings.

- [ ] **Step 5: Commit**

  ```bash
  git add src/browser/mod.rs src/lib.rs src/storage/backend/mod.rs
  git commit -m "feat(browser): wire browser module into crate (cfg-gated)"
  ```

---

## Task 4: `IndexedDbBackend` — open and load

**Files:**
- Create: `src/browser/indexeddb.rs`

This file is WASM-only and cannot be tested with native `cargo test`. The integration
tests in Task 9 (headless Chrome) cover it.

- [ ] **Step 1: Create `src/browser/indexeddb.rs`**

  ```rust
  //! Async IndexedDB backend for browser WASM.
  //!
  //! This is NOT a `StorageBackend` implementor — it is async-only.
  //! Called directly by `BrowserDb` after synchronous `PersistentFactStorage::save()`.

  use js_sys::{Array, Promise, Uint8Array};
  use std::collections::HashMap;
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen::prelude::*;
  use wasm_bindgen::JsCast;
  use wasm_bindgen_futures::JsFuture;
  use web_sys::{IdbDatabase, IdbRequest, IdbTransaction, IdbTransactionMode};

  /// Converts an `IdbRequest` into a JS `Promise` that resolves with the request result.
  fn request_to_promise(request: &IdbRequest) -> Promise {
      let req = request.clone();
      Promise::new(&mut |resolve, reject| {
          let req_ok = req.clone();
          let on_success: Closure<dyn FnMut(web_sys::Event)> =
              Closure::once(move |_: web_sys::Event| {
                  let result = req_ok.result().unwrap_or(JsValue::NULL);
                  resolve.call1(&JsValue::NULL, &result).ok();
              });
          let on_error: Closure<dyn FnMut(web_sys::Event)> =
              Closure::once(move |_: web_sys::Event| {
                  reject
                      .call1(&JsValue::NULL, &JsValue::from_str("IdbRequest failed"))
                      .ok();
              });
          req.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
          req.set_onerror(Some(on_error.as_ref().unchecked_ref()));
          on_success.forget();
          on_error.forget();
      })
  }

  /// Converts an `IdbTransaction` completion into a JS `Promise`.
  fn transaction_to_promise(tx: &IdbTransaction) -> Promise {
      let tx = tx.clone();
      Promise::new(&mut |resolve, reject| {
          let on_complete: Closure<dyn FnMut(web_sys::Event)> =
              Closure::once(move |_: web_sys::Event| {
                  resolve.call0(&JsValue::NULL).ok();
              });
          let on_error: Closure<dyn FnMut(web_sys::Event)> =
              Closure::once(move |_: web_sys::Event| {
                  reject
                      .call1(&JsValue::NULL, &JsValue::from_str("IdbTransaction failed"))
                      .ok();
              });
          tx.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
          tx.set_onerror(Some(on_error.as_ref().unchecked_ref()));
          on_complete.forget();
          on_error.forget();
      })
  }

  /// Async wrapper around a browser IndexedDB database.
  ///
  /// Object store schema:
  ///   name:  `<db_name>`
  ///   key:   page_id (u64 stored as JS number — f64, safe up to 2^53)
  ///   value: 4096-byte Uint8Array
  pub struct IndexedDbBackend {
      db: IdbDatabase,
      store_name: String,
  }

  impl IndexedDbBackend {
      /// Open (or create) an IndexedDB database with a single object store.
      ///
      /// If the object store does not exist, it is created in `onupgradeneeded`.
      /// `db_name` is used as both the database name and the object store name.
      pub async fn open(db_name: &str) -> Result<Self, JsValue> {
          let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window object"))?;
          let idb_factory = window
              .indexed_db()?
              .ok_or_else(|| JsValue::from_str("IndexedDB not available"))?;

          let store_name = db_name.to_string();
          let store_name_upgrade = store_name.clone();

          let open_request = idb_factory.open_with_u32(db_name, 1)?;

          // Create the object store if this is a fresh database (version upgrade).
          let on_upgrade: Closure<dyn FnMut(web_sys::Event)> =
              Closure::once(move |event: web_sys::Event| {
                  let target = event.target().unwrap();
                  let request: web_sys::IdbOpenDbRequest = target.dyn_into().unwrap();
                  let db: IdbDatabase = request.result().unwrap().dyn_into().unwrap();
                  if !db
                      .object_store_names()
                      .contains(&store_name_upgrade)
                  {
                      db.create_object_store(&store_name_upgrade).unwrap();
                  }
              });
          open_request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
          on_upgrade.forget();

          // Wait for the open to succeed.
          JsFuture::from(request_to_promise(open_request.as_ref())).await?;

          let db: IdbDatabase = open_request.result()?.dyn_into()?;
          Ok(Self { db, store_name })
      }

      /// Load all pages from IndexedDB into a `HashMap<page_id, bytes>`.
      ///
      /// Uses `getAllKeys()` + `getAll()` in a single read transaction, then zips
      /// the two result arrays. Both calls share the same `IdbTransaction` to
      /// guarantee consistency (no writes can interleave between them).
      pub async fn load_all_pages(&self) -> Result<HashMap<u64, Vec<u8>>, JsValue> {
          let tx = self
              .db
              .transaction_with_str_and_mode(&self.store_name, IdbTransactionMode::Readonly)?;
          let store = tx.object_store(&self.store_name)?;

          let keys_req = store.get_all_keys()?;
          let keys_val = JsFuture::from(request_to_promise(keys_req.as_ref())).await?;
          let keys_arr: Array = keys_val.dyn_into()?;

          let vals_req = store.get_all()?;
          let vals_val = JsFuture::from(request_to_promise(vals_req.as_ref())).await?;
          let vals_arr: Array = vals_val.dyn_into()?;

          let mut pages = HashMap::with_capacity(keys_arr.length() as usize);
          for i in 0..keys_arr.length() {
              let key = keys_arr.get(i);
              let page_id = key
                  .as_f64()
                  .ok_or_else(|| JsValue::from_str("page_id is not a number"))? as u64;
              let val = vals_arr.get(i);
              let arr: Uint8Array = val.dyn_into()?;
              pages.insert(page_id, arr.to_vec());
          }
          Ok(pages)
      }
  }
  ```

- [ ] **Step 2: Confirm the crate compiles for the WASM target**

  ```bash
  cargo check --target wasm32-unknown-unknown --features browser
  ```

  Expected: no errors. (Warnings about unused items are acceptable at this stage.)

- [ ] **Step 3: Commit**

  ```bash
  git add src/browser/indexeddb.rs
  git commit -m "feat(browser): add IndexedDbBackend open and load_all_pages"
  ```

---

## Task 5: `IndexedDbBackend::write_pages`

**Files:**
- Modify: `src/browser/indexeddb.rs`

- [ ] **Step 1: Add `write_pages` to `IndexedDbBackend`**

  Add this method inside `impl IndexedDbBackend` in `src/browser/indexeddb.rs`:

  ```rust
  /// Write a batch of pages to IndexedDB in a single `readwrite` transaction.
  ///
  /// All `put` operations are queued synchronously on the store, then we wait
  /// for the transaction's `oncomplete` event. If any put fails, the transaction
  /// is aborted and an error is returned.
  ///
  /// `pages` is a list of `(page_id, page_bytes)` pairs. Empty input is a no-op.
  pub async fn write_pages(&self, pages: Vec<(u64, Vec<u8>)>) -> Result<(), JsValue> {
      if pages.is_empty() {
          return Ok(());
      }
      let tx = self
          .db
          .transaction_with_str_and_mode(&self.store_name, IdbTransactionMode::Readwrite)?;
      let store = tx.object_store(&self.store_name)?;

      for (page_id, data) in &pages {
          let key = JsValue::from_f64(*page_id as f64);
          let arr = Uint8Array::from(data.as_slice());
          store.put_with_key(&arr, &key)?;
      }

      // Wait for the transaction to commit. The IDB transaction commits
      // automatically once all put requests have been processed and no
      // new requests are made. We wait here to ensure durability before
      // returning to the caller.
      JsFuture::from(transaction_to_promise(&tx)).await?;
      Ok(())
  }
  ```

- [ ] **Step 2: Confirm compilation**

  ```bash
  cargo check --target wasm32-unknown-unknown --features browser
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add src/browser/indexeddb.rs
  git commit -m "feat(browser): add IndexedDbBackend::write_pages"
  ```

---

## Task 6: `BrowserDb` core — inner struct, `open_in_memory`, `open`

**Files:**
- Modify: `src/browser/mod.rs`

- [ ] **Step 1: Add `BrowserDb` struct and `open_in_memory`**

  Replace the stub contents of `src/browser/mod.rs` with:

  ```rust
  //! Browser WASM support: `BrowserDb` async façade backed by IndexedDB.
  //!
  //! This module is only compiled for `wasm32-unknown-unknown` with the `browser`
  //! feature enabled. It is **not** compatible with Node.js, Deno, Bun, or any
  //! server-side runtime. For server-side Node.js, use `@minigraf/node` (Phase 8.3).

  pub mod buffer;
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
  use crate::storage::PAGE_SIZE;
  use std::cell::RefCell;
  use std::collections::HashSet;
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
  }
  ```

- [ ] **Step 2: Confirm compilation**

  ```bash
  cargo check --target wasm32-unknown-unknown --features browser
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add src/browser/mod.rs
  git commit -m "feat(browser): add BrowserDb struct with open and open_in_memory"
  ```

---

## Task 7: Make `materialize_transaction` / `materialize_retraction` accessible

**Files:**
- Modify: `src/db.rs`

`BrowserDb::execute()` needs to convert `Transaction` AST nodes into `Fact` lists.
These functions already exist in `Minigraf` but are private. Make them `pub(crate)`.

- [ ] **Step 1: Change visibility in `src/db.rs`**

  Find these two lines in `src/db.rs` and change `fn` to `pub(crate) fn`:

  ```rust
  // Before:
  fn materialize_transaction(tx: &Transaction) -> Result<Vec<Fact>> {
  // After:
  pub(crate) fn materialize_transaction(tx: &Transaction) -> Result<Vec<Fact>> {
  ```

  ```rust
  // Before:
  fn materialize_retraction(tx: &Transaction) -> Result<Vec<Fact>> {
  // After:
  pub(crate) fn materialize_retraction(tx: &Transaction) -> Result<Vec<Fact>> {
  ```

- [ ] **Step 2: Run existing tests to confirm no regressions**

  ```bash
  cargo test
  ```

  Expected: all existing tests pass.

- [ ] **Step 3: Commit**

  ```bash
  git add src/db.rs
  git commit -m "refactor(db): make materialize_transaction/retraction pub(crate) for browser reuse"
  ```

---

## Task 8: `execute()` — read path and write path

**Files:**
- Modify: `src/browser/mod.rs`

`execute()` must **not** hold a `RefCell` borrow across any `.await` point.
The pattern: borrow → do all sync work → collect result → drop borrow → `.await`.

- [ ] **Step 1: Add the `query_result_to_json` helper**

  Add this free function at the bottom of `src/browser/mod.rs`
  (outside `impl BrowserDb`, not `#[wasm_bindgen]`):

  ```rust
  /// Serialise a `QueryResult` to a JSON string for the WASM boundary.
  ///
  /// Format:
  ///   Transacted  → `{"transacted":<tx_id>}`
  ///   Retracted   → `{"retracted":<tx_id>}`
  ///   Ok          → `{"ok":true}`
  ///   QueryResults → `{"variables":[...],"results":[[...],...]}`
  fn query_result_to_json(result: QueryResult) -> String {
      use crate::graph::types::Value;
      use serde_json::{Value as JVal, json};

      let val: JVal = match result {
          QueryResult::Transacted(tx_id) => json!({"transacted": tx_id}),
          QueryResult::Retracted(tx_id) => json!({"retracted": tx_id}),
          QueryResult::Ok => json!({"ok": true}),
          QueryResult::QueryResults { variables, results } => {
              let rows: Vec<Vec<JVal>> = results
                  .iter()
                  .map(|row| row.iter().map(value_to_json).collect())
                  .collect();
              json!({"variables": variables, "results": rows})
          }
      };
      val.to_string()
  }

  fn value_to_json(v: &crate::graph::types::Value) -> serde_json::Value {
      use crate::graph::types::Value;
      use serde_json::Value as JVal;
      match v {
          Value::String(s)  => JVal::String(s.clone()),
          Value::Integer(i) => JVal::Number((*i).into()),
          Value::Float(f)   => serde_json::Number::from_f64(*f)
              .map(JVal::Number)
              .unwrap_or(JVal::Null),
          Value::Boolean(b) => JVal::Bool(*b),
          Value::Ref(uuid)  => JVal::String(uuid.to_string()),
          Value::Keyword(k) => JVal::String(k.clone()),
          Value::Null       => JVal::Null,
      }
  }
  ```

- [ ] **Step 2: Add `execute()` to `impl BrowserDb`**

  Add inside `#[wasm_bindgen] impl BrowserDb`:

  ```rust
  /// Execute a Datalog string (transact, retract, query, rule).
  ///
  /// Returns a JSON string:
  /// - `(query ...)` → `{"variables":[...],"results":[[...],...]}`
  /// - `(transact ...)` → `{"transacted":<tx_id>}`
  /// - `(retract ...)` → `{"retracted":<tx_id>}`
  /// - `(rule ...)` → `{"ok":true}`
  ///
  /// Writes flush dirty pages to IndexedDB before returning.
  /// Reads never touch IndexedDB (served from in-memory `FactStorage`).
  pub async fn execute(&self, datalog: String) -> Result<String, JsValue> {
      let cmd = parse_datalog_command(&datalog)
          .map_err(|e| JsValue::from_str(&e.to_string()))?;

      match &cmd {
          DatalogCommand::Query(_) => {
              // Read path: no lock, no IDB access.
              let result = {
                  let inner = self.inner.borrow();
                  let mut executor = DatalogExecutor::new_with_rules_and_functions(
                      inner.fact_storage.clone(),
                      inner.rules.clone(),
                      inner.functions.clone(),
                  );
                  executor
                      .execute(cmd)
                      .map_err(|e| JsValue::from_str(&e.to_string()))?
              };
              Ok(query_result_to_json(result))
          }
          DatalogCommand::Rule(_) => {
              // Rule registration: mutates rule registry, no IDB flush needed.
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
              Ok(query_result_to_json(result))
          }
          DatalogCommand::Transact(tx) => {
              // Materialise facts from the parsed transaction.
              let facts = crate::db::Minigraf::materialize_transaction(tx)
                  .map_err(|e| JsValue::from_str(&e.to_string()))?;
              self.apply_write(facts, false).await
          }
          DatalogCommand::Retract(tx) => {
              let facts = crate::db::Minigraf::materialize_retraction(tx)
                  .map_err(|e| JsValue::from_str(&e.to_string()))?;
              self.apply_write(facts, true).await
          }
      }
  }

  /// Stamp facts, apply to FactStorage, save pages, flush dirty pages to IDB.
  ///
  /// IMPORTANT: The `RefCell` borrow is dropped before any `.await`. All sync
  /// work (stamp → apply → pfs.save → take_dirty) happens while the borrow is
  /// held; the async IDB flush happens after the borrow is released.
  async fn apply_write(
      &self,
      mut facts: Vec<crate::graph::types::Fact>,
      is_retract: bool,
  ) -> Result<String, JsValue> {
      use crate::graph::types::{tx_id_now, VALID_TIME_FOREVER};
      use crate::db::VALID_FROM_USE_TX_TIME;  // re-use the sentinel constant

      // --- Sync section: borrow inner, stamp facts, apply, save, take dirty ---
      let (dirty_ids, result_json, idb_ref) = {
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

          inner
              .pfs
              .save()
              .map_err(|e| JsValue::from_str(&e.to_string()))?;

          let dirty_ids: Vec<u64> = inner.pfs.backend_mut().take_dirty().into_iter().collect();

          let json = if is_retract {
              format!(r#"{{"retracted":{}}}"#, tx_id)
          } else {
              format!(r#"{{"transacted":{}}}"#, tx_id)
          };

          // Collect a reference to idb if present — we need it after the borrow drops.
          // We can't hold a reference across .await, so check existence only.
          let has_idb = inner.idb.is_some();
          (dirty_ids, json, has_idb)
      };
      // RefCell borrow is dropped here.

      // --- Async section: flush dirty pages to IndexedDB ---
      if idb_ref && !dirty_ids.is_empty() {
          let inner = self.inner.borrow();
          let idb = inner.idb.as_ref().unwrap();
          let pages: Vec<(u64, Vec<u8>)> = dirty_ids
              .into_iter()
              .map(|id| {
                  let data = inner
                      .pfs
                      .backend()
                      .read_page_raw(id)
                      .unwrap_or_else(|_| vec![0u8; PAGE_SIZE]);
                  (id, data)
              })
              .collect();
          // Drop borrow before await.
          drop(inner);

          let inner = self.inner.borrow();
          let idb = inner.idb.as_ref().unwrap();
          // We need to clone the pages because we drop the borrow.
          // Collect pages first while holding borrow.
          drop(inner);

          // Re-collect pages outside borrow: we already have them in `pages`.
          self.inner
              .borrow()
              .idb
              .as_ref()
              .unwrap()
              .write_pages(pages)  // <-- this is async
              // ERROR: can't .await while holding borrow from self.inner.borrow()
              ;
      }

      Ok(result_json)
  }
  ```

  **STOP** — the above `apply_write` has a borrow-across-await problem that must be
  fixed. The IDB handle (`idb`) is behind a `RefCell` borrow, and we can't `.await`
  while that borrow is held. The fix: collect the pages into an owned `Vec` and extract
  the `idb` reference before any `.await`.

  **Replace `apply_write` with this corrected version:**

  ```rust
  async fn apply_write(
      &self,
      facts: Vec<crate::graph::types::Fact>,
      is_retract: bool,
  ) -> Result<String, JsValue> {
      use crate::db::VALID_FROM_USE_TX_TIME;
      use crate::graph::types::tx_id_now;

      // ── Sync section ──────────────────────────────────────────────────────────
      // Borrow inner, do ALL sync work, collect owned data to use after borrow drops.
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

          inner
              .pfs
              .save()
              .map_err(|e| JsValue::from_str(&e.to_string()))?;

          // Collect dirty pages as owned Vec<(u64, Vec<u8>)> — no borrows escape.
          let dirty_ids: HashSet<u64> = inner.pfs.backend_mut().take_dirty();
          let dirty_pages: Vec<(u64, Vec<u8>)> = dirty_ids
              .into_iter()
              .filter_map(|id| {
                  inner.pfs.backend().read_page_raw(id).ok().map(|d| (id, d))
              })
              .collect();

          let json = if is_retract {
              format!(r#"{{"retracted":{}}}"#, tx_id)
          } else {
              format!(r#"{{"transacted":{}}}"#, tx_id)
          };

          (dirty_pages, json)
      };
      // ── Borrow dropped here ───────────────────────────────────────────────────

      // ── Async section: flush to IndexedDB (no RefCell borrow held) ────────────
      if !dirty_pages.is_empty() {
          // Temporarily take the idb out to call async write_pages.
          // We borrow briefly to check, collect the write, then release.
          let has_idb = self.inner.borrow().idb.is_some();
          if has_idb {
              // Clone the IDB handle — IdbDatabase is a JS object, Clone is cheap.
              let idb_clone = {
                  let inner = self.inner.borrow();
                  // IndexedDbBackend needs to be cloneable for this pattern.
                  // See note below — we derive Clone on IndexedDbBackend.
                  inner.idb.as_ref().unwrap().clone_handle()
              };
              idb_clone.write_pages(dirty_pages).await?;
          }
      }

      Ok(result_json)
  }
  ```

  This requires:
  1. `read_page_raw(id) -> Result<Vec<u8>>` on `BrowserBufferBackend` (delegates to
     `read_page` — add it as a public method)
  2. `backend()` / `backend_mut()` accessor on `PersistentFactStorage` (check if it
     exists; add it as `pub(crate)` if not)
  3. `clone_handle()` on `IndexedDbBackend` that clones the `IdbDatabase` JS handle

  **Add `read_page_raw` to `BrowserBufferBackend`** in `src/browser/buffer.rs`:

  ```rust
  /// Read a page by ID (alias for `StorageBackend::read_page`, for use without trait).
  pub fn read_page_raw(&self, page_id: u64) -> anyhow::Result<Vec<u8>> {
      self.read_page(page_id)
  }
  ```

  **Add `clone_handle` to `IndexedDbBackend`** in `src/browser/indexeddb.rs`:

  ```rust
  /// Clone the underlying IdbDatabase handle (cheap — it's a JS object reference).
  pub fn clone_handle(&self) -> Self {
      Self {
          db: self.db.clone(),
          store_name: self.store_name.clone(),
      }
  }
  ```

  **Check if `PersistentFactStorage` has `backend()` / `backend_mut()` accessors.**
  Look in `src/storage/persistent_facts.rs`. If they exist, use them.
  If not, add them in Task 8 Step 3 below.

- [ ] **Step 3: Add `backend()` / `backend_mut()` to `PersistentFactStorage` if missing**

  Check `src/storage/persistent_facts.rs` for existing accessors. If absent, find the
  `impl<B: StorageBackend> PersistentFactStorage<B>` block and add:

  ```rust
  /// Read-only access to the underlying storage backend.
  pub(crate) fn backend(&self) -> &B {
      &self.backend
  }

  /// Mutable access to the underlying storage backend.
  pub(crate) fn backend_mut(&mut self) -> &mut B {
      &mut self.backend
  }
  ```

  The field name may be `backend`, `storage`, or similar — check the struct definition
  in `persistent_facts.rs` and use the actual field name.

- [ ] **Step 4: Handle `VALID_FROM_USE_TX_TIME` visibility**

  `VALID_FROM_USE_TX_TIME` is a private constant in `src/db.rs`. It must be visible
  in `src/browser/mod.rs`. Change it from `const` to `pub(crate) const`:

  ```rust
  // In src/db.rs, line ~17:
  // Before:
  const VALID_FROM_USE_TX_TIME: i64 = i64::MIN;
  // After:
  pub(crate) const VALID_FROM_USE_TX_TIME: i64 = i64::MIN;
  ```

- [ ] **Step 5: Compile-check**

  ```bash
  cargo check --target wasm32-unknown-unknown --features browser
  ```

  Fix any remaining type errors until this passes.

- [ ] **Step 6: Run native tests to confirm no regressions**

  ```bash
  cargo test
  ```

  Expected: all existing tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add src/browser/mod.rs src/browser/buffer.rs src/browser/indexeddb.rs \
          src/storage/persistent_facts.rs src/db.rs
  git commit -m "feat(browser): add BrowserDb::execute() read and write paths"
  ```

---

## Task 9: `checkpoint()`, `export_graph()`, `import_graph()`

**Files:**
- Modify: `src/browser/mod.rs`

Add these three methods inside `#[wasm_bindgen] impl BrowserDb`:

- [ ] **Step 1: Add `checkpoint()`**

  ```rust
  /// Flush all dirty pages to IndexedDB.
  ///
  /// Write-through means individual `execute()` calls already flush dirty pages,
  /// so `checkpoint()` is only needed after `import_graph()` or explicit bulk ops.
  /// No-op for in-memory databases.
  pub async fn checkpoint(&self) -> Result<(), JsValue> {
      let (dirty_pages, has_idb) = {
          let mut inner = self.inner.borrow_mut();
          inner
              .pfs
              .save()
              .map_err(|e| JsValue::from_str(&e.to_string()))?;
          let dirty_ids: HashSet<u64> = inner.pfs.backend_mut().take_dirty();
          let pages: Vec<(u64, Vec<u8>)> = dirty_ids
              .into_iter()
              .filter_map(|id| inner.pfs.backend().read_page_raw(id).ok().map(|d| (id, d)))
              .collect();
          (pages, inner.idb.is_some())
      };

      if has_idb && !dirty_pages.is_empty() {
          let idb = self.inner.borrow().idb.as_ref().unwrap().clone_handle();
          idb.write_pages(dirty_pages).await?;
      }
      Ok(())
  }
  ```

- [ ] **Step 2: Add `export_graph()`**

  ```rust
  /// Serialise the current database to a portable `.graph` blob.
  ///
  /// The blob is byte-for-byte compatible with native `.graph` files opened by
  /// `Minigraf::open()`. Pages are always in ascending `page_id` order.
  ///
  /// Call `db.checkpoint()` on native before importing a file here to ensure
  /// no WAL entries are missing from the main file.
  #[wasm_bindgen(js_name = exportGraph)]
  pub fn export_graph(&self) -> Result<js_sys::Uint8Array, JsValue> {
      let inner = self.inner.borrow();
      let page_count = inner
          .pfs
          .backend()
          .page_count()
          .map_err(|e| JsValue::from_str(&e.to_string()))? as usize;

      let mut blob = Vec::with_capacity(page_count * PAGE_SIZE);
      for id in 0..page_count as u64 {
          let page = inner
              .pfs
              .backend()
              .read_page_raw(id)
              .map_err(|e| JsValue::from_str(&e.to_string()))?;
          blob.extend_from_slice(&page);
      }
      Ok(js_sys::Uint8Array::from(blob.as_slice()))
  }
  ```

  Note: `export_graph` is synchronous (no `async`) — it only reads in-memory data.
  The `pfs.save()` call to ensure the buffer is current is omitted because
  write-through means the buffer is always up-to-date after every `execute()`. If you
  want to guarantee freshness, call `checkpoint()` before `export_graph()`.

- [ ] **Step 3: Add `import_graph()`**

  ```rust
  /// Replace the current database with a `.graph` blob.
  ///
  /// The blob must be a checkpointed native `.graph` file (no pending WAL sidecar).
  /// All existing data is overwritten. After import, the new data is immediately
  /// queryable and all dirty pages are flushed to IndexedDB.
  #[wasm_bindgen(js_name = importGraph)]
  pub async fn import_graph(&self, data: js_sys::Uint8Array) -> Result<(), JsValue> {
      let bytes = data.to_vec();
      if bytes.len() % PAGE_SIZE != 0 {
          return Err(JsValue::from_str("import data length is not a multiple of PAGE_SIZE"));
      }

      // Split into 4 KB pages and build buffer with all pages dirty.
      let mut pages = std::collections::HashMap::new();
      for (i, chunk) in bytes.chunks(PAGE_SIZE).enumerate() {
          pages.insert(i as u64, chunk.to_vec());
      }

      // ── Sync section ──────────────────────────────────────────────────────────
      let (dirty_pages, has_idb) = {
          let mut inner = self.inner.borrow_mut();
          let buffer = BrowserBufferBackend::load_pages_all_dirty(pages);
          // `buffer` is moved into `new_pfs`; take dirty pages from `new_pfs.backend_mut()`
          // AFTER construction, not from `buffer` directly (it has been moved).
          let mut new_pfs = PersistentFactStorage::new(buffer, 256)
              .map_err(|e| JsValue::from_str(&e.to_string()))?;
          let new_fact_storage = new_pfs.storage().clone();

          // Drain dirty set and collect owned page bytes before swapping inner.
          let dirty_ids = new_pfs.backend_mut().take_dirty();
          let dirty_pages: Vec<(u64, Vec<u8>)> = dirty_ids
              .into_iter()
              .filter_map(|id| {
                  new_pfs.backend().read_page_raw(id).ok().map(|d| (id, d))
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
  ```

  **Note on `import_graph` dirty collection:** The above collects dirty pages from
  `new_pfs` before swapping it into `inner`. After the swap, `inner.pfs` is the new
  one and `dirty_ids` have already been drained. This is correct: the pages are owned
  `Vec<u8>` values, not references.

- [ ] **Step 4: Compile-check**

  ```bash
  cargo check --target wasm32-unknown-unknown --features browser
  ```

- [ ] **Step 5: Native tests still pass**

  ```bash
  cargo test
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/browser/mod.rs
  git commit -m "feat(browser): add checkpoint, export_graph, import_graph to BrowserDb"
  ```

---

## Task 10: `wasm-bindgen-test` integration tests

**Files:**
- Modify: `src/browser/mod.rs`
- Modify: `Cargo.toml` (add `wasm-bindgen-test` dev-dep)

- [ ] **Step 1: Add `wasm-bindgen-test` dev-dependency**

  In `Cargo.toml` under `[dev-dependencies]`:

  ```toml
  wasm-bindgen-test = "0.3"
  ```

- [ ] **Step 2: Add integration tests to `src/browser/mod.rs`**

  Append at the end of `src/browser/mod.rs`:

  ```rust
  #[cfg(all(target_arch = "wasm32", feature = "browser", test))]
  mod tests {
      use super::*;
      use wasm_bindgen_test::*;

      wasm_bindgen_test_configure!(run_in_browser);

      // ── open_in_memory smoke test ─────────────────────────────────────────────

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

      // ── read does not require prior write ─────────────────────────────────────

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

      // ── export/import round-trip ──────────────────────────────────────────────

      #[wasm_bindgen_test]
      async fn export_import_round_trip() {
          let db = BrowserDb::open_in_memory().expect("open");
          db.execute(r#"(transact [[:bob :role "admin"]])"#.to_string())
              .await
              .expect("transact");

          let blob = db.export_graph().expect("export");
          // Verify magic bytes "MGRF" at offset 0
          let bytes = blob.to_vec();
          assert_eq!(&bytes[0..4], b"MGRF", "exported blob must start with MGRF magic");

          // Import into a fresh in-memory db and query
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

      // ── export byte size is a multiple of PAGE_SIZE ───────────────────────────

      #[wasm_bindgen_test]
      async fn export_size_is_page_aligned() {
          let db = BrowserDb::open_in_memory().expect("open");
          db.execute(r#"(transact [[:e :v 1]])"#.to_string())
              .await
              .expect("transact");
          let blob = db.export_graph().expect("export");
          assert_eq!(blob.byte_length() as usize % PAGE_SIZE, 0);
      }

      // ── IndexedDB persistence round-trip ─────────────────────────────────────
      // Uses a unique db_name per test run to avoid cross-test interference.

      #[wasm_bindgen_test]
      async fn idb_persistence_round_trip() {
          let db_name = "minigraf-test-persistence";

          // Write some data
          let db1 = BrowserDb::open(db_name).await.expect("open db1");
          db1.execute(r#"(transact [[:carol :dept "eng"]])"#.to_string())
              .await
              .expect("transact");
          drop(db1);

          // Reopen and verify data persisted
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
  ```

- [ ] **Step 3: Run tests with headless Chrome**

  First install `wasm-pack` if not already present:

  ```bash
  curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
  ```

  Run the tests:

  ```bash
  wasm-pack test --headless --chrome --features browser
  ```

  Expected: all 5 tests pass. If Chrome is not installed, install it:

  ```bash
  # Ubuntu/Debian:
  wget -q -O - https://dl.google.com/linux/linux_signing_key.pub | sudo apt-key add -
  sudo apt-get install -y google-chrome-stable
  ```

- [ ] **Step 4: Commit**

  ```bash
  git add src/browser/mod.rs Cargo.toml
  git commit -m "test(browser): add wasm-bindgen-test integration tests for BrowserDb"
  ```

---

## Task 11: CI job

**Files:**
- Create: `.github/workflows/wasm-browser.yml`

- [ ] **Step 1: Create the workflow file**

  ```yaml
  name: WASM Browser

  on:
    push:
      branches: ["main"]
    pull_request:
      branches: ["main"]

  env:
    CARGO_TERM_COLOR: always

  permissions:
    contents: read

  jobs:
    wasm-browser:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4

        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: wasm32-unknown-unknown

        - name: Install wasm-pack
          run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

        - name: Install Chrome
          uses: browser-actions/setup-chrome@v1

        - name: Build (release)
          run: wasm-pack build --target web --features browser

        - name: Check gzipped binary size
          run: |
            SIZE=$(gzip -c pkg/minigraf_bg.wasm | wc -c)
            echo "Gzipped WASM size: ${SIZE} bytes"
            if [ "$SIZE" -gt 1048576 ]; then
              echo "WARNING: exceeds 1 MB gzipped budget (${SIZE} bytes)"
            fi

        - name: Test (headless Chrome)
          run: wasm-pack test --headless --chrome --features browser
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/wasm-browser.yml
  git commit -m "ci: add wasm-browser workflow for browser WASM build and tests"
  ```

---

## Task 12: Browser example

**Files:**
- Create: `examples/browser/index.html`
- Create: `examples/browser/app.js`
- Create: `examples/browser/README.md`

- [ ] **Step 1: Create `examples/browser/index.html`**

  ```html
  <!DOCTYPE html>
  <html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>Minigraf Browser Demo</title>
  </head>
  <body>
    <h1>Minigraf Browser Demo</h1>
    <p>Open the browser console (F12) to see query results.</p>
    <script type="module" src="app.js"></script>
  </body>
  </html>
  ```

- [ ] **Step 2: Create `examples/browser/app.js`**

  ```js
  // Minigraf browser demo — no bundler required.
  // Build first: wasm-pack build --target web --features browser
  // Then serve from repo root: python3 -m http.server 8080
  // Open: http://localhost:8080/examples/browser/

  import init, { BrowserDb } from "../../pkg/minigraf.js";

  async function main() {
    // Initialise the WASM module (loads minigraf_bg.wasm).
    await init();

    // Open a database backed by IndexedDB (persists across page reloads).
    const db = await BrowserDb.open("minigraf-demo");

    // Assert some facts.
    await db.execute(`(transact [
      [:alice :person/name "Alice"]
      [:alice :person/age  30]
      [:alice :friend      :bob]
      [:bob   :person/name "Bob"]
    ])`);

    // Query with Datalog.
    const raw = await db.execute(`
      (query [:find ?friend-name
              :where [:alice :friend ?f]
                     [?f :person/name ?friend-name]])
    `);
    const result = JSON.parse(raw);
    console.log("Alice's friends:", result.results.map(row => row[0]));
    // Expected: ["Bob"]

    // Export to a portable .graph blob.
    const blob = db.exportGraph();
    console.log(".graph blob size:", blob.byteLength, "bytes");

    // Import into a fresh in-memory db.
    const db2 = BrowserDb.openInMemory();
    await db2.importGraph(blob);
    const raw2 = await db2.execute(
      `(query [:find ?name :where [?e :person/name ?name]])`
    );
    console.log("After import, names:", JSON.parse(raw2).results.map(r => r[0]));
    // Expected: ["Alice", "Bob"] (order may vary)
  }

  main().catch(console.error);
  ```

- [ ] **Step 3: Create `examples/browser/README.md`**

  ```markdown
  # Minigraf Browser Demo

  Demonstrates `@minigraf/core` running in a plain browser page with no bundler.

  ## Build

  From the repo root:

  ```bash
  wasm-pack build --target web --features browser
  ```

  This produces `pkg/` containing `minigraf.js`, `minigraf_bg.wasm`, and
  `minigraf.d.ts`.

  ## Serve

  ```bash
  # From the repo root (not the examples/browser/ directory):
  python3 -m http.server 8080
  ```

  Open `http://localhost:8080/examples/browser/` in Chrome or Firefox.

  ## What it does

  - Opens an IndexedDB-backed database named `"minigraf-demo"`.
  - Transacts facts about Alice and Bob.
  - Queries Alice's friends with Datalog.
  - Exports the `.graph` blob and imports it into a fresh in-memory database.
  - Logs all results to the browser console (open with F12).

  ## Notes

  - Data persists across page reloads (stored in IndexedDB).
  - The `pkg/` directory is gitignored — rebuild after pulling changes.
  - This package (`@minigraf/core`) is **browser-only**. For Node.js, use
    `@minigraf/node` (Phase 8.3).
  ```

- [ ] **Step 4: Commit**

  ```bash
  git add examples/browser/
  git commit -m "docs(example): add browser WASM demo (examples/browser/)"
  ```

---

## Self-Review Checklist

After completing all tasks, verify the following before opening a PR:

- [ ] `cargo test` passes (native, all existing tests green)
- [ ] `cargo check --target wasm32-unknown-unknown --features browser` passes
- [ ] `wasm-pack build --target web --features browser` produces `pkg/`
- [ ] `wasm-pack test --headless --chrome --features browser` — all 5 tests green
- [ ] `gzip -c pkg/minigraf_bg.wasm | wc -c` is under 1,048,576 bytes
- [ ] `cargo clippy --features browser -- -D warnings` passes (fix any new warnings)
- [ ] `cargo fmt --check` passes
- [ ] The PR description references issue #129 and includes the checklist above as evidence

## Notes for the reviewer

- `BrowserDb` is intentionally `!Send + !Sync` (`Rc<RefCell<...>>` inside). This is correct for single-threaded browser WASM.
- `@minigraf/core` is browser-only. Any attempt to import it in Node.js will fail at the `IndexedDbBackend::open()` call (no `window.indexedDB`). This is by design — see spec.
- The `VALID_FROM_USE_TX_TIME` sentinel (`i64::MIN`) is now `pub(crate)` — the only change to `src/db.rs`.
- `Minigraf::materialize_transaction` and `::materialize_retraction` are now `pub(crate)` — no public API change.
- The `PersistentFactStorage::backend()` / `backend_mut()` accessors are `pub(crate)` — not exposed in the public API.
