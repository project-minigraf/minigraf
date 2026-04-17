//! Async IndexedDB backend for browser WASM.
//!
//! This is NOT a `StorageBackend` implementor — it is async-only.
//! Called directly by `BrowserDb` after synchronous `PersistentFactStorage::save()`.

use js_sys::{Array, Promise, Uint8Array};
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
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
    pub(crate) db: IdbDatabase,
    pub(crate) store_name: String,
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
                if !db.object_store_names().contains(&store_name_upgrade) {
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
                .ok_or_else(|| JsValue::from_str("page_id is not a number"))?
                as u64;
            let val = vals_arr.get(i);
            let arr: Uint8Array = val.dyn_into()?;
            pages.insert(page_id, arr.to_vec());
        }
        Ok(pages)
    }

    /// Clone the underlying IdbDatabase handle (cheap — it's a JS object reference).
    pub fn clone_handle(&self) -> Self {
        Self {
            db: self.db.clone(),
            store_name: self.store_name.clone(),
        }
    }

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
}
