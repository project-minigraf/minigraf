//! IndexedDB flush failures (#894). Faults wrap real browser transactions;
//! no injection controls are compiled into the library.
use super::*;
use js_sys::{Function, Promise, Reflect, Uint8Array};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use wasm_bindgen_test::*;

const QUERY: &str = "(query [:find ?n :where [?e :n ?n] :order-by [?n :asc]])";

fn name() -> String {
    format!(
        "flush-894-{}-{}",
        js_sys::Date::now(),
        js_sys::Math::random()
    )
}

fn write(first: usize, end: usize) -> String {
    let facts: String = (first..end).map(|i| format!("[:e{i} :n {i}]")).collect();
    format!("(transact [{facts}])")
}

fn code<T>(result: Result<T, JsValue>, expected: &str) {
    let Err(e) = result else {
        panic!("expected rejection {expected}")
    };
    let message = js_text(&e);
    assert!(message.starts_with(&format!("[{expected}]")), "{message}");
}

/// A bounded wait also catches the old abort-only hang. Clear its timer on
/// either outcome so a passing test leaves no work behind.
async fn deadline(p: Promise) -> Result<JsValue, JsValue> {
    let bounded: Promise = Function::new_with_args(
        "p",
        r#"
        return new Promise((resolve,reject) => {
            const timer = setTimeout(() => reject('deadline: flush did not settle'), 1500);
            p.then(v => { clearTimeout(timer); resolve(v); },
                   e => { clearTimeout(timer); reject(e); });
        });
    "#,
    )
    .call1(&JsValue::NULL, &p)
    .unwrap()
    .unchecked_into();
    JsFuture::from(bounded).await
}

fn execute(db: &BrowserDb, command: String) -> Promise {
    let inner = db.inner.clone();
    future_to_promise(async move {
        BrowserDb { inner }
            .execute(command)
            .await
            .map(JsValue::from)
    })
}

fn checkpoint(db: &BrowserDb) -> Promise {
    let inner = db.inner.clone();
    future_to_promise(async move {
        BrowserDb { inner }
            .checkpoint()
            .await
            .map(|()| JsValue::UNDEFINED)
    })
}

fn import(db: &BrowserDb, bytes: Uint8Array) -> Promise {
    let inner = db.inner.clone();
    future_to_promise(async move {
        BrowserDb { inner }
            .import_graph(bytes)
            .await
            .map(|()| JsValue::UNDEFINED)
    })
}

async fn tick() {
    JsFuture::from(Promise::resolve(&JsValue::NULL))
        .await
        .unwrap();
}

async fn rows(db: &BrowserDb) -> Vec<usize> {
    let json = db.execute(QUERY.into()).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_u64().unwrap() as usize)
        .collect()
}

fn adapter(db: &BrowserDb) -> IndexedDbBackend {
    db.inner.borrow().idb.as_ref().unwrap().clone_handle()
}

/// Patch only one connection (or, for open(), the prototype with a name
/// filter). Restoration also removes the page-error observer.
struct Fault {
    state: JsValue,
    restore: Function,
}
impl Fault {
    fn new(target: &JsValue, store: &str, mode: &str) -> Self {
        let state = js_sys::Object::new();
        let restore = Function::new_with_args("target,name,mode,state", r#"
            const descriptor = Object.getOwnPropertyDescriptor(target,'transaction');
            const original = target.transaction;
            state.sent = 0; state.errors = 0; state.pageErrors = [];
            const pageError = e => { state.pageErrors.push(e.message); e.preventDefault(); };
            globalThis.addEventListener('error',pageError);
            Object.defineProperty(target,'transaction',{configurable:true,value:function(...args) {
                const tx = original.apply(this,args);
                if (args[0] !== name || args[1] !== 'readwrite') return tx;
                state.sent++;
                if (state.sent !== 1) return tx;
                tx.addEventListener('error',() => state.errors++);
                if (mode === 'hold') {
                    const setter = Object.getOwnPropertyDescriptor(IDBTransaction.prototype,'oncomplete').set;
                    Object.defineProperty(tx,'oncomplete',{set(callback) {
                        setter.call(tx,event => { state.resume = () => callback(event); });
                    }});
                }
                const objectStore = tx.objectStore;
                tx.objectStore = function(...args) {
                    const s = objectStore.apply(this,args), put = s.put;
                    let queued = 0, last;
                    s.put = function(...args) {
                        if (mode === 'throw' && queued++ === 1)
                            throw new DOMException('injected queue failure','QuotaExceededError');
                        // add() of existing keys yields real request errors.
                        const req = mode === 'errors' ? s.add(...args) : put.apply(this,args);
                        last = req;
                        if (mode === 'abort' || mode === 'delayed-abort') {
                            req.addEventListener('success',() => {
                                if (req !== last) return;
                                // No outstanding requests: abort event only.
                                tx.abort();
                            });
                        }
                        return req;
                    };
                    return s;
                };
                if (mode === 'delayed-abort') {
                    const setter = Object.getOwnPropertyDescriptor(IDBTransaction.prototype,'onabort').set;
                    Object.defineProperty(tx,'onabort',{set(callback) {
                        setter.call(tx,event => { state.resume = () => callback(event); });
                    }});
                }
                return tx;
            }});
            return () => {
                if (descriptor) Object.defineProperty(target,'transaction',descriptor);
                else delete target.transaction;
                globalThis.removeEventListener('error',pageError);
            };
        "#).call4(&JsValue::NULL, target, &store.into(), &mode.into(), &state).unwrap().unchecked_into();
        Self {
            state: state.into(),
            restore,
        }
    }
    fn for_db(db: &BrowserDb, mode: &str) -> Self {
        let idb = adapter(db);
        Self::new(&idb.db, &idb.store_name, mode)
    }
    fn number(&self, key: &str) -> f64 {
        Reflect::get(&self.state, &key.into())
            .unwrap()
            .as_f64()
            .unwrap()
    }
    async fn resume(&self) {
        let p: Promise = Function::new_with_args(
            "s",
            r#"
            return new Promise((resolve,reject) => {
                const end = Date.now()+1500;
                function poll() {
                    if (s.resume) { s.resume(); resolve(); }
                    else if (Date.now()>end) reject('no held outcome');
                    else setTimeout(poll,0);
                }
                poll();
            });
        "#,
        )
        .call1(&JsValue::NULL, &self.state)
        .unwrap()
        .unchecked_into();
        JsFuture::from(p).await.unwrap();
    }
    fn no_page_errors(&self) {
        let errors: js_sys::Array = Reflect::get(&self.state, &"pageErrors".into())
            .unwrap()
            .unchecked_into();
        assert_eq!(errors.length(), 0, "callbacks must not throw");
    }
}
impl Drop for Fault {
    fn drop(&mut self) {
        self.restore.call0(&JsValue::NULL).unwrap();
    }
}

async fn reopen(n: &str, expected: Vec<usize>) {
    let db = BrowserDb::open(n).await.expect("fresh reopen");
    assert_eq!(rows(&db).await, expected);
    // The exact integrity walker used by native Minigraf::verify(), over a
    // native-format export; catches absent reachable pages, not free holes.
    assert!(db.inner.borrow().pfs.verify().unwrap().problems.is_empty());
    image_of(db.export_graph().unwrap().to_vec())
        .expect("export passes native image integrity walk");
    db.close().unwrap();
}

async fn refused(db: &BrowserDb) {
    let before = rows(db).await;
    let count = db.inner.borrow().fact_storage.current_tx_count();
    let bytes = db.export_graph().unwrap().to_vec();
    for _ in 0..2 {
        code(db.execute(write(900, 901)).await, "STG-046");
        code(db.execute("(retract [[:e0 :n 0]])".into()).await, "STG-046");
        code(db.checkpoint().await, "STG-046");
        code(
            db.import_graph(Uint8Array::new_with_length(0)).await,
            "STG-046",
        );
        for nonempty in [false, true] {
            let mut tx = db.begin_write().unwrap();
            if nonempty {
                tx.execute(&write(901, 902)).unwrap();
            }
            tx.execute(QUERY).unwrap();
            code(db.checkpoint().await, "INT-001");
            code(deadline(tx.commit()).await, "STG-046");
            code(tx.execute(QUERY), "API-018");
            code(deadline(tx.commit()).await, "API-018");
        }
    }
    assert_eq!(db.inner.borrow().fact_storage.current_tx_count(), count);
    assert_eq!(rows(db).await, before);
    assert_eq!(db.export_graph().unwrap().to_vec(), bytes);
    let prepared = db.prepare(QUERY).unwrap();
    prepared.execute(JsValue::UNDEFINED).unwrap();
    let mut cursor = db.query(QUERY).unwrap();
    cursor.next_batch(2).unwrap();
    db.execute("(rule [(number ?e ?n) [?e :n ?n]])".into())
        .await
        .unwrap();
}

#[wasm_bindgen_test]
async fn adapter_abort_only_settles() {
    let n = name();
    let idb = IndexedDbBackend::open(&n).await.unwrap();
    idb.write_pages(vec![(0, vec![1])], vec![]).await.unwrap();
    let before = idb.load_all_pages().await.unwrap();
    let fault = Fault::new(&idb.db, &n, "abort");
    let copy = idb.clone_handle();
    let p = future_to_promise(async move {
        copy.write_pages(vec![(0, vec![2]), (1, vec![3])], vec![])
            .await
            .map(|()| JsValue::UNDEFINED)
    });
    let error = deadline(p)
        .await
        .expect_err("abort rejects before deadline");
    assert!(js_text(&error).contains("STG-048"), "{}", js_text(&error));
    assert!(js_text(&error).contains("IndexedDB write"));
    assert!(js_text(&error).contains("AbortError"));
    assert_eq!(fault.number("errors"), 0.0, "abort-only, no request errors");
    assert_eq!(idb.load_all_pages().await.unwrap(), before);
    fault.no_page_errors();
    drop(fault);
    idb.write_pages(vec![(1, vec![4])], vec![])
        .await
        .expect("connection is recoverable");
    idb.db.close();
}

#[wasm_bindgen_test]
async fn adapter_queue_throw_is_atomic() {
    let n = name();
    let idb = IndexedDbBackend::open(&n).await.unwrap();
    idb.write_pages(vec![(0, vec![1]), (9, vec![9])], vec![])
        .await
        .unwrap();
    let before = idb.load_all_pages().await.unwrap();
    let fault = Fault::new(&idb.db, &n, "throw");
    let error = idb
        .write_pages(vec![(0, vec![2]), (1, vec![3])], vec![9])
        .await
        .unwrap_err();
    assert_eq!(
        idb.load_all_pages().await.unwrap(),
        before,
        "queued delete and put must both roll back"
    );
    assert!(js_text(&error).contains("STG-048"));
    assert!(js_text(&error).contains("QuotaExceededError"));
    fault.no_page_errors();
    idb.db.close();
}

#[wasm_bindgen_test]
async fn adapter_multiple_request_errors_settle_once() {
    let n = name();
    let idb = IndexedDbBackend::open(&n).await.unwrap();
    let pages = vec![(0, vec![1]), (1, vec![2]), (2, vec![3])];
    idb.write_pages(pages.clone(), vec![]).await.unwrap();
    let before = idb.load_all_pages().await.unwrap();
    let fault = Fault::new(&idb.db, &n, "errors");
    code(idb.write_pages(pages, vec![]).await, "STG-048");
    assert_eq!(idb.load_all_pages().await.unwrap(), before);
    assert!(fault.number("errors") >= 2.0);
    fault.no_page_errors();
    idb.db.close();
}

#[wasm_bindgen_test]
async fn multipage_failed_write_refuses_successors() {
    let n = name();
    let db = BrowserDb::open(&n).await.unwrap();
    db.execute(write(0, 2)).await.unwrap();
    let before = adapter(&db).load_all_pages().await.unwrap();
    let fault = Fault::for_db(&db, "throw");
    let failed = db.execute(write(2, 402)).await;
    assert!(
        adapter(&db).load_all_pages().await.unwrap() == before,
        "multi-page failure must leave every durable byte unchanged"
    );
    code(failed, "STG-048");
    assert_eq!(rows(&db).await.len(), 402);
    assert!(
        db.inner
            .borrow()
            .pfs
            .with_backend(|b| buffer_page_ids(b).any(|id| !before.contains_key(&id))),
        "write appended pages"
    );
    refused(&db).await;
    assert_eq!(fault.number("sent"), 1.0);
    assert_eq!(adapter(&db).load_all_pages().await.unwrap(), before);
    drop(fault);
    db.close().unwrap();
    reopen(&n, vec![0, 1]).await;
}

#[wasm_bindgen_test]
async fn commit_checkpoint_and_smaller_import_fail_atomically() {
    for operation in ["commit", "checkpoint", "import"] {
        let n = name();
        let db = BrowserDb::open(&n).await.unwrap();
        db.execute(write(0, 400)).await.unwrap();
        let before = adapter(&db).load_all_pages().await.unwrap();
        let fault = Fault::for_db(&db, "abort");
        let p = match operation {
            "commit" => {
                let mut tx = db.begin_write().unwrap();
                tx.execute(&write(400, 401)).unwrap();
                tx.commit()
            }
            "checkpoint" => {
                // Real dirty bytes of its own, without another flush.
                db.inner.borrow_mut().pfs.with_backend_mut(|b| {
                    let page = b.read_page(0).unwrap();
                    b.write_page(0, &page).unwrap();
                });
                checkpoint(&db)
            }
            _ => import(&db, Uint8Array::new_with_length(0)),
        };
        code(deadline(p).await, "STG-048");
        assert_eq!(
            rows(&db).await.len(),
            match operation {
                "commit" => 401,
                "import" => 0,
                _ => 400,
            }
        );
        refused(&db).await;
        assert_eq!(fault.number("sent"), 1.0);
        assert_eq!(adapter(&db).load_all_pages().await.unwrap(), before);
        drop(fault);
        db.close().unwrap();
        reopen(&n, (0..400).collect()).await;
    }
}

#[wasm_bindgen_test]
async fn preparation_error_keeps_code_and_fails_handle() {
    for operation in ["execute", "commit", "checkpoint"] {
        let n = name();
        let db = BrowserDb::open(&n).await.unwrap();
        db.execute(write(0, 2)).await.unwrap();
        let fault = Fault::for_db(&db, "hold");
        db.inner.borrow_mut().pfs.with_backend_mut(|b| {
            if let BrowserBackend::Buffer(buffer) = b {
                buffer.mark_dirty_for_test(999_999);
            }
        });
        let p = match operation {
            "commit" => {
                let mut tx = db.begin_write().unwrap();
                tx.execute(&write(2, 3)).unwrap();
                tx.commit()
            }
            "checkpoint" => checkpoint(&db),
            _ => execute(&db, write(2, 3)),
        };
        code(deadline(p).await, "INT-052");
        refused(&db).await;
        assert_eq!(fault.number("sent"), 0.0);
        db.close().unwrap();
        reopen(&n, vec![0, 1]).await;
    }
}

#[wasm_bindgen_test]
async fn overlapping_failure_never_submits_successor() {
    for pair in [
        "execute-execute",
        "commit-execute",
        "execute-import",
        "import-execute",
        "execute-checkpoint",
    ] {
        let n = name();
        let db = BrowserDb::open(&n).await.unwrap();
        db.execute(write(0, 2)).await.unwrap();
        let before = adapter(&db).load_all_pages().await.unwrap();
        let fault = Fault::for_db(&db, "delayed-abort");
        let first = if pair.starts_with("commit") {
            let mut tx = db.begin_write().unwrap();
            tx.execute(&write(2, 402)).unwrap();
            tx.commit()
        } else if pair.starts_with("import") {
            import(&db, Uint8Array::new_with_length(0))
        } else {
            execute(&db, write(2, 402))
        };
        tick().await;
        let second = if pair.ends_with("import") {
            import(&db, Uint8Array::new_with_length(0))
        } else if pair.ends_with("checkpoint") {
            checkpoint(&db)
        } else {
            execute(&db, write(500, 501))
        };
        tick().await;
        assert_eq!(fault.number("sent"), 1.0, "successor waits");
        assert_eq!(
            rows(&db).await.len(),
            match pair {
                "execute-import" => 0,
                "import-execute" => 1,
                "execute-checkpoint" => 402,
                _ => 403,
            }
        );
        fault.resume().await;
        code(deadline(first).await, "STG-048");
        code(deadline(second).await, "STG-046");
        refused(&db).await;
        assert_eq!(fault.number("sent"), 1.0);
        assert_eq!(adapter(&db).load_all_pages().await.unwrap(), before);
        drop(fault);
        db.close().unwrap();
        reopen(&n, vec![0, 1]).await;
    }
}

#[wasm_bindgen_test]
async fn successful_checkpoint_boundary_and_mixed_overlap() {
    let n = name();
    let db = BrowserDb::open(&n).await.unwrap();
    db.execute(write(0, 2)).await.unwrap();
    let fault = Fault::for_db(&db, "hold");
    let first = execute(&db, write(2, 402));
    tick().await;
    let boundary = checkpoint(&db);
    let observed = js_sys::Object::new();
    Function::new_with_args("p,s", "p.then(() => s.done=true)")
        .call2(&JsValue::NULL, &boundary, &observed)
        .unwrap();
    tick().await;
    assert!(
        !Reflect::get(&observed, &"done".into()).unwrap().is_truthy(),
        "empty checkpoint waits"
    );
    let replace = import(&db, Uint8Array::new_with_length(0));
    tick().await;
    let mut tx = db.begin_write().unwrap();
    tx.execute(&write(600, 602)).unwrap();
    let commit = tx.commit();
    let retract = execute(&db, "(retract [[:e600 :n 600]])".into());
    tick().await;
    assert_eq!(fault.number("sent"), 1.0);
    fault.resume().await;
    for p in [first, boundary, replace, commit, retract] {
        deadline(p).await.unwrap();
    }
    assert_eq!(rows(&db).await, vec![601]);
    drop(fault);
    db.close().unwrap();
    reopen(&n, vec![601]).await;
}

#[wasm_bindgen_test]
async fn close_discards_every_unstarted_flush() {
    for operation in ["execute", "commit", "checkpoint", "import"] {
        let n = name();
        let db = BrowserDb::open(&n).await.unwrap();
        db.execute(write(0, 2)).await.unwrap();
        let fault = Fault::for_db(&db, "hold");
        let first = execute(&db, write(2, 3));
        tick().await;
        let second = match operation {
            "commit" => {
                let mut tx = db.begin_write().unwrap();
                tx.execute(&write(3, 4)).unwrap();
                tx.commit()
            }
            "checkpoint" => checkpoint(&db),
            "import" => import(&db, Uint8Array::new_with_length(0)),
            _ => execute(&db, write(3, 4)),
        };
        tick().await;
        db.close().unwrap();
        let fresh = BrowserDb::open(&n).await.unwrap();
        fresh.execute(write(9, 10)).await.unwrap();
        fault.resume().await;
        deadline(first).await.unwrap();
        code(deadline(second).await, "API-018");
        assert_eq!(fault.number("sent"), 1.0);
        drop(fault);
        fresh.close().unwrap();
        reopen(&n, vec![0, 1, 2, 9]).await;
    }
}

#[wasm_bindgen_test]
async fn migration_flush_failure_leaves_original_pages() {
    let n = name();
    let idb = IndexedDbBackend::open(&n).await.unwrap();
    let bytes = include_bytes!("../../tests/golden/v7_basic.graph");
    let pages: Vec<_> = bytes
        .chunks(crate::storage::PAGE_SIZE)
        .enumerate()
        .map(|(i, p)| (i as u64, p.to_vec()))
        .collect();
    idb.write_pages(pages, vec![]).await.unwrap();
    let before = idb.load_all_pages().await.unwrap();
    let prototype = Reflect::get(&js_sys::global(), &"IDBDatabase".into()).unwrap();
    let prototype = Reflect::get(&prototype, &"prototype".into()).unwrap();
    let fault = Fault::new(&prototype, &n, "throw");
    code(BrowserDb::open(&n).await, "STG-048");
    assert_eq!(idb.load_all_pages().await.unwrap(), before);
    drop(fault);
    let db = BrowserDb::open(&n).await.unwrap();
    image_of(db.export_graph().unwrap().to_vec()).unwrap();
    db.close().unwrap();
    idb.db.close();
}

#[wasm_bindgen_test]
async fn checkpoint_does_not_wait_for_later_writes() {
    let n = name();
    let db = BrowserDb::open(&n).await.unwrap();
    let first_fault = Fault::for_db(&db, "hold");
    let first = execute(&db, write(0, 2));
    tick().await;
    let boundary = checkpoint(&db);
    tick().await;
    // Restore admission of transactions, retaining the first held callback.
    first_fault.restore.call0(&JsValue::NULL).unwrap();
    let later_fault = Fault::for_db(&db, "hold");
    let later = execute(&db, write(2, 3));
    tick().await;
    first_fault.resume().await;
    deadline(first).await.unwrap();
    deadline(boundary)
        .await
        .expect("boundary excludes later pending flush");
    later_fault.resume().await;
    deadline(later).await.unwrap();
    drop(later_fault);
    drop(first_fault);
    db.close().unwrap();
    reopen(&n, vec![0, 1, 2]).await;
}

#[wasm_bindgen_test]
async fn preparation_failure_preserves_earlier_successful_prefix() {
    let n = name();
    let db = BrowserDb::open(&n).await.unwrap();
    let fault = Fault::for_db(&db, "hold");
    let first = execute(&db, write(0, 2));
    tick().await;
    db.inner.borrow_mut().pfs.with_backend_mut(|b| {
        if let BrowserBackend::Buffer(buffer) = b {
            buffer.mark_dirty_for_test(999_999);
        }
    });
    let broken = execute(&db, write(2, 3));
    let observed = js_sys::Object::new();
    Function::new_with_args("p,s", "p.catch(() => s.done=true)")
        .call2(&JsValue::NULL, &broken, &observed)
        .unwrap();
    tick().await;
    assert!(
        !Reflect::get(&observed, &"done".into()).unwrap().is_truthy(),
        "even preparation failure waits for earlier work"
    );
    code(db.checkpoint().await, "STG-046");
    fault.resume().await;
    deadline(first).await.unwrap();
    code(deadline(broken).await, "INT-052");
    assert_eq!(fault.number("sent"), 1.0);
    assert!(
        db.inner.flush_tail.borrow().is_none(),
        "settled barriers are released"
    );
    drop(fault);
    db.close().unwrap();
    reopen(&n, vec![0, 1]).await;
}

#[wasm_bindgen_test]
async fn validation_errors_do_not_fail_handle_or_retain_flushes() {
    let n = name();
    let db = BrowserDb::open(&n).await.unwrap();
    assert!(db.execute("(transact [[:bad :n]])".into()).await.is_err());
    assert!(
        db.import_graph(Uint8Array::new_with_length(1))
            .await
            .is_err()
    );
    for i in 0..20 {
        db.execute(write(i, i + 1)).await.unwrap();
        db.checkpoint().await.unwrap();
        assert!(db.inner.failed_flush.get().is_none());
        assert!(
            db.inner.flush_tail.borrow().is_none(),
            "no settled history retained"
        );
    }
    db.close().unwrap();
    reopen(&n, (0..20).collect()).await;
}
