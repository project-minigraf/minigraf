//! OPFS mode (#441). Most tests need a dedicated worker, so they run only in
//! the worker build (`RUSTFLAGS='--cfg minigraf_browser_worker'`, see
//! `.github/workflows/wasm.yml`); the main-thread build checks that
//! `openOpfs` is refused there. Termination and cross-worker tests need a
//! page that owns workers: minigraf-wasm's packaged consumer test has them.

use super::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

#[cfg(not(minigraf_browser_worker))]
wasm_bindgen_test_configure!(run_in_browser);
#[cfg(minigraf_browser_worker)]
wasm_bindgen_test_configure!(run_in_dedicated_worker);

/// The `[CODE]` an error message starts with.
fn code_of(e: &JsValue) -> String {
    let s = e.as_string().expect("an error message");
    s.split(']')
        .next()
        .unwrap_or("")
        .trim_start_matches('[')
        .to_string()
}

fn rejection<T>(r: Result<T, JsValue>) -> JsValue {
    match r {
        Ok(_) => panic!("expected a rejection"),
        Err(e) => e,
    }
}

/// Every entry name in the origin's OPFS root.
async fn entries() -> Vec<String> {
    // `navigator.storage.getDirectory()` directly: it works on the main
    // thread too, where `opfs::root` refuses.
    let navigator = js_sys::Reflect::get(&js_sys::global(), &"navigator".into()).unwrap();
    let storage: web_sys::StorageManager = js_sys::Reflect::get(&navigator, &"storage".into())
        .unwrap()
        .unchecked_into();
    let dir: web_sys::FileSystemDirectoryHandle = JsFuture::from(storage.get_directory())
        .await
        .expect("root")
        .unchecked_into();
    let iter = dir.keys();
    let mut names = Vec::new();
    loop {
        let next = JsFuture::from(iter.next().expect("next"))
            .await
            .expect("entry");
        if js_sys::Reflect::get(&next, &"done".into())
            .unwrap()
            .is_truthy()
        {
            break;
        }
        names.push(
            js_sys::Reflect::get(&next, &"value".into())
                .unwrap()
                .as_string()
                .unwrap(),
        );
    }
    names.sort();
    names
}

#[cfg(not(minigraf_browser_worker))]
mod main_thread {
    use super::*;

    /// Spec AC2: on the main thread `openOpfs` rejects with API-026,
    /// creates nothing and traps nothing.
    #[wasm_bindgen_test]
    async fn open_opfs_on_the_main_thread_is_api_026_and_creates_nothing() {
        let before = entries().await;
        let err = rejection(BrowserDb::open_opfs("main-thread".into()).await);
        assert_eq!(code_of(&err), "API-026");
        assert!(
            err.as_string().unwrap().contains("dedicated worker"),
            "names what is missing"
        );
        assert_eq!(entries().await, before, "nothing created");
        // The instance still works.
        let db = BrowserDb::open_in_memory().expect("open_in_memory");
        db.execute("(transact [[:a :n 1]])".into())
            .await
            .expect("transact");
    }
}

#[cfg(minigraf_browser_worker)]
mod worker {
    use super::super::tests::unique;
    use super::*;
    use crate::browser::opfs::tests::{Faults, WriteFault, arm, reads};

    async fn open(name: &str) -> BrowserDb {
        BrowserDb::open_opfs(name.to_string())
            .await
            .expect("openOpfs")
    }

    async fn exec(db: &BrowserDb, q: &str) -> String {
        db.execute(q.to_string()).await.expect("execute")
    }

    fn rows(json: &str) -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut rows: Vec<serde_json::Value> = v["results"].as_array().unwrap().clone();
        rows.sort_by_key(ToString::to_string);
        serde_json::Value::Array(rows)
    }

    async fn ns(db: &BrowserDb) -> Vec<i64> {
        let json = exec(db, "(query [:find ?n :where [?e :n ?n]])").await;
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut ns: Vec<i64> = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_i64().unwrap())
            .collect();
        ns.sort_unstable();
        ns
    }

    async fn put(db: &BrowserDb, range: std::ops::Range<i64>) {
        let facts: String = range.map(|i| format!("[:e{i} :n {i}]")).collect();
        exec(db, &format!("(transact [{facts}])")).await;
    }

    fn file(db: &BrowserDb) -> Arc<OpfsFile> {
        db.inner.opfs.as_ref().expect("OPFS mode").file.clone()
    }

    async fn raw_file(entry: &str) -> Arc<OpfsFile> {
        let dir = opfs::root().await.unwrap();
        opfs::open_file(&dir, entry, true).await.unwrap().unwrap()
    }

    /// Write `bytes` as the whole of OPFS entry `entry`.
    async fn write_raw(entry: &str, bytes: &[u8]) {
        let f = raw_file(entry).await;
        f.truncate(0).unwrap();
        let opts = FileSystemReadWriteOptionsExt::at(0.0);
        let n = f
            .handle_for_tests()
            .write_with_u8_array_and_options(bytes, &opts)
            .unwrap();
        assert_eq!(n as usize, bytes.len());
        f.flush().unwrap();
        f.close();
    }

    /// The whole of OPFS entry `entry`.
    async fn read_raw(entry: &str) -> Vec<u8> {
        let f = raw_file(entry).await;
        let len = f.size().unwrap() as usize;
        let mut buf = vec![0u8; len];
        let opts = FileSystemReadWriteOptionsExt::at(0.0);
        f.handle_for_tests()
            .read_with_u8_array_and_options(&mut buf, &opts)
            .unwrap();
        f.close();
        buf
    }

    struct FileSystemReadWriteOptionsExt;
    impl FileSystemReadWriteOptionsExt {
        fn at(off: f64) -> web_sys::FileSystemReadWriteOptions {
            let o = web_sys::FileSystemReadWriteOptions::new();
            o.set_at(off);
            o
        }
    }

    fn blob(bytes: &[u8]) -> js_sys::Uint8Array {
        js_sys::Uint8Array::from(bytes)
    }

    const PORTABILITY_COMMANDS: &str =
        include_str!("../../tests/fixtures/portability/commands.txt");
    const PORTABILITY_QUERIES: &str = include_str!("../../tests/fixtures/portability/queries.txt");

    async fn assert_portability_answers(db: &BrowserDb, source: &str) {
        for line in PORTABILITY_QUERIES.lines().filter(|l| !l.is_empty()) {
            let (q, expected) = line.split_once('\t').unwrap();
            let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
            assert_eq!(rows(&exec(db, q).await), expected, "{source}: {q}");
        }
    }

    async fn portability_db(name: &str) -> BrowserDb {
        let db = open(name).await;
        for c in PORTABILITY_COMMANDS.lines().filter(|l| !l.is_empty()) {
            exec(&db, c).await;
        }
        db
    }

    // ── AC1, AC4: entry point, persistence, files ──────────────────────────

    #[wasm_bindgen_test]
    async fn acknowledged_writes_survive_close_and_reopen() {
        let name = unique("ac1");
        let db = open(&name).await;
        put(&db, 0..5).await;
        exec(&db, "(retract [[:e0 :n 0]])").await;
        db.close().expect("close");
        let db = open(&name).await;
        assert_eq!(
            ns(&db).await,
            vec![1, 2, 3, 4],
            "history kept, retraction applied"
        );
        // Dropped without close(): the handle is released and nothing lost.
        put(&db, 5..6).await;
        drop(db);
        let db = open(&name).await;
        assert_eq!(ns(&db).await, vec![1, 2, 3, 4, 5]);
        // An IndexedDB database of the same name is another database.
        let idb = BrowserDb::open(&name).await.expect("IndexedDB open");
        assert!(ns(&idb).await.is_empty(), "no fallback or sharing");
        idb.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn the_database_is_one_file_and_no_wal() {
        let name = unique("files");
        let db = open(&name).await;
        put(&db, 0..50).await;
        db.checkpoint().await.expect("checkpoint");
        let exported = db.export_graph().expect("export");
        db.import_graph(exported).await.expect("import");
        put(&db, 50..60).await;
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:x :n 1000]])").unwrap();
        tx.rollback();
        let mine: Vec<String> = entries()
            .await
            .into_iter()
            .filter(|e| e.starts_with(&name))
            .collect();
        assert_eq!(mine, vec![format!("{name}.graph")], "one file, no WAL");
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn invalid_names_are_api_017_before_anything_is_created() {
        let before = entries().await;
        for bad in ["", "a/b", "a\\b", ".", "..", "nul\0"] {
            let err = rejection(BrowserDb::open_opfs(bad.into()).await);
            assert_eq!(code_of(&err), "API-017", "name {bad:?}");
        }
        assert_eq!(entries().await, before, "nothing created");
    }

    // ── AC8: lock ──────────────────────────────────────────────────────────

    #[wasm_bindgen_test]
    async fn a_second_open_is_stg_026_until_close() {
        let (a, b) = (unique("lock-a"), unique("lock-b"));
        let db = open(&a).await;
        let err = rejection(BrowserDb::open_opfs(a.clone()).await);
        assert_eq!(code_of(&err), "STG-026");
        let other = open(&b).await;
        put(&other, 0..1).await;
        // Dependents still referenced do not hold the lock after close().
        let cursor = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
        db.close().unwrap();
        let again = open(&a).await;
        drop(cursor);
        drop(again);
        // A failed open releases the lock: a foreign file is STG-002.
        let junk = unique("lock-junk");
        write_raw(&format!("{junk}.graph"), &vec![0xAA; 3 * 4096]).await;
        let err = rejection(BrowserDb::open_opfs(junk.clone()).await);
        assert_eq!(code_of(&err), "STG-002");
        let dir = opfs::root().await.unwrap();
        let f = opfs::open_file(&dir, &format!("{junk}.graph"), false)
            .await
            .expect("lock free after a failed open")
            .unwrap();
        f.close();
        other.close().unwrap();
    }

    // ── AC9: close() ───────────────────────────────────────────────────────

    async fn all_modes(name: &str) -> Vec<BrowserDb> {
        vec![
            BrowserDb::open_in_memory().unwrap(),
            BrowserDb::open(name).await.expect("IndexedDB"),
            open(name).await,
        ]
    }

    #[wasm_bindgen_test]
    async fn after_close_every_call_is_api_018_but_disposal() {
        let name = unique("closed");
        for db in all_modes(&name).await {
            put(&db, 0..3).await;
            let mut cursor = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
            let mut cursor2 = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
            let pq = db.prepare("(query [:find ?n :where [?e :n ?n]])").unwrap();
            let mut tx = db.begin_write().unwrap();
            tx.execute("(transact [[:t :n 99]])").unwrap();
            db.close().expect("close");
            db.close().expect("idempotent");
            let none = JsValue::UNDEFINED;
            let api018 = |e: JsValue| assert_eq!(code_of(&e), "API-018");
            api018(rejection(
                db.execute("(query [:find ?n :where [?e :n ?n]])".into())
                    .await,
            ));
            api018(rejection(db.execute("(transact [[:z :n 1]])".into()).await));
            api018(rejection(db.query("(query [:find ?n :where [?e :n ?n]])")));
            api018(rejection(
                db.prepare("(query [:find ?n :where [?e :n ?n]])"),
            ));
            api018(rejection(db.checkpoint().await));
            api018(rejection(db.export_graph()));
            api018(rejection(db.import_graph(blob(&[])).await));
            api018(rejection(db.begin_write()));
            api018(rejection(db.register_predicate(
                "p?",
                js_sys::Function::new_no_args("return true").into(),
            )));
            api018(rejection(cursor.next_batch(10)));
            api018(rejection(cursor.vars()));
            api018(rejection(pq.execute(none.clone())));
            api018(rejection(pq.query(none.clone())));
            api018(rejection(pq.with_max_iterations(1.0)));
            api018(rejection(
                tx.execute("(query [:find ?n :where [?e :n ?n]])"),
            ));
            api018(rejection(JsFuture::from(tx.commit()).await));
            // Disposal never throws.
            cursor2.close();
            tx.rollback();
            drop(tx);
        }
        // The OPFS lock is free at once, with the dependents dropped above.
        let db = open(&name).await;
        assert_eq!(
            ns(&db).await,
            vec![0, 1, 2],
            "the open transaction was discarded"
        );
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn close_from_a_user_function_during_a_query_is_busy() {
        let name = unique("busy-close");
        let db = open(&name).await;
        put(&db, 0..3).await;
        let inner = BrowserDb {
            inner: db.inner.clone(),
        };
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let log = seen.clone();
        let f = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |_v: JsValue| {
            let mut log = log.borrow_mut();
            log.push(match inner.close() {
                Ok(()) => "closed".into(),
                Err(e) => code_of(&e),
            });
            // Nested reads keep working.
            let nested = inner.query("(query [:find ?n :where [?e :n ?n]])").is_ok()
                && inner.export_graph().is_ok();
            log.push(nested.to_string());
            JsValue::TRUE
        })
        .into_js_value();
        db.register_predicate("probe?", f).unwrap();
        exec(&db, "(query [:find ?n :where [?e :n ?n] [(probe? ?n)]])").await;
        let seen = seen.borrow().clone();
        assert!(!seen.is_empty());
        assert_eq!(seen[0], "API-027", "refused while the query runs");
        assert_eq!(seen[1], "true", "nested calls work");
        // Through a cursor too.
        let mut cursor = db
            .query("(query [:find ?n :where [?e :n ?n] [(probe? ?n)]])")
            .unwrap();
        while cursor.next_batch(1).unwrap().is_some() {}
        db.close().expect("close once the query is over");
    }

    #[wasm_bindgen_test]
    async fn calls_issued_before_close_change_nothing_after_it() {
        // OPFS: an execute whose body has not started rejects and writes
        // nothing; a commit that applied resolves and is stored. Both are
        // checked against the database reopened before they settle.
        let name = unique("before-close");
        let db = open(&name).await;
        let late = {
            let d = BrowserDb {
                inner: db.inner.clone(),
            };
            wasm_bindgen_futures::future_to_promise(async move {
                d.execute("(transact [[:late :n 7]])".into())
                    .await
                    .map(JsValue::from)
            })
        };
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:c :n 1]])").unwrap();
        let commit = tx.commit();
        db.close().unwrap();
        let reopened = open(&name).await;
        let err = rejection(JsFuture::from(late).await);
        assert_eq!(code_of(&err), "API-018", "the body started after close()");
        JsFuture::from(commit)
            .await
            .expect("an applied OPFS commit resolves");
        assert_eq!(
            ns(&reopened).await,
            vec![1],
            "the commit, not the late write"
        );
        reopened.close().unwrap();

        // IndexedDB: a commit whose flush has not started is never flushed.
        let name = unique("before-close-idb");
        let db = BrowserDb::open(&name).await.unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:c :n 1]])").unwrap();
        let commit = tx.commit();
        db.close().unwrap();
        let reopened = BrowserDb::open(&name).await.unwrap();
        let err = rejection(JsFuture::from(commit).await);
        assert_eq!(code_of(&err), "API-018");
        assert!(ns(&reopened).await.is_empty(), "not stored");
        reopened.close().unwrap();
    }

    // ── AC10: failed commits ───────────────────────────────────────────────

    #[wasm_bindgen_test]
    async fn a_failed_commit_is_stg_048_then_stg_046() {
        let cases: Vec<(Faults, &str)> = vec![
            (
                Faults {
                    write_after: Some((0, WriteFault::Throw(0))),
                    ..Faults::default()
                },
                "fault injection",
            ),
            (
                Faults {
                    write_after: Some((1, WriteFault::Short(100))),
                    ..Faults::default()
                },
                "short read/write",
            ),
            (
                Faults {
                    flush_after: Some(1),
                    ..Faults::default()
                },
                "flush",
            ),
        ];
        for (faults, text) in cases {
            let name = unique("failed");
            let db = open(&name).await;
            put(&db, 0..3).await;
            *file(&db).faults.lock().unwrap() = faults;
            let err = rejection(db.execute("(transact [[:e9 :n 9]])".into()).await);
            let msg = err.as_string().unwrap();
            assert_eq!(code_of(&err), "STG-048", "{msg}");
            assert!(msg.contains(text), "{msg}");
            let err = rejection(db.execute("(transact [[:e10 :n 10]])".into()).await);
            assert_eq!(code_of(&err), "STG-046");
            let mut tx = db.begin_write().unwrap();
            tx.execute("(transact [[:e11 :n 11]])").unwrap();
            assert_eq!(
                code_of(&rejection(JsFuture::from(tx.commit()).await)),
                "STG-046"
            );
            // Reads keep working, failed facts included.
            assert!(ns(&db).await.contains(&9));
            db.close().expect("close after a failed commit");
            let db = open(&name).await;
            let got = ns(&db).await;
            assert!(
                got == vec![0, 1, 2] || got == vec![0, 1, 2, 9],
                "previous state, or the failed commit whole"
            );
            db.close().unwrap();
        }
    }

    // ── AC5, AC6, AC7: interrupted commits and open outcomes ──────────────

    /// A commit whose meta write tears before the CRC: a native open of
    /// these bytes refuses (STG-033); OPFS mode opens at the previous commit,
    /// and its export opens under the native rule.
    #[wasm_bindgen_test]
    async fn a_torn_meta_reopens_at_the_previous_commit() {
        // Tear writes of the commit until one leaves the meta invalid.
        let mut torn_once = false;
        for k in 0..64u64 {
            let name_k = unique("torn-k");
            let d = open(&name_k).await;
            put(&d, 0..20).await;
            *file(&d).faults.lock().unwrap() = Faults {
                write_after: Some((k, WriteFault::Throw(16))),
                ..Faults::default()
            };
            let ok = d.execute("(transact [[:z :n 99]])".into()).await.is_ok();
            d.close().unwrap();
            if ok {
                break;
            }
            let reopened = open(&name_k).await;
            let got = ns(&reopened).await;
            assert!(
                got.len() == 20 || got.len() == 21,
                "old or new, never refused"
            );
            let bytes = reopened.export_graph().unwrap().to_vec();
            reopened.close().unwrap();
            // The export opens under the native rule (no WAL).
            let mut pages = std::collections::HashMap::new();
            for (i, c) in bytes.chunks(4096).enumerate() {
                pages.insert(i as u64, c.to_vec());
            }
            let native = PersistentFactStorage::open(
                BrowserBackend::Buffer(BrowserBufferBackend::load_pages(pages)),
                0,
                None,
            );
            assert!(native.is_ok(), "the export opens natively");
            // The raw file after a meta tear: one valid slot, next-generation
            // pages. Native refuses it; OPFS mode just opened it.
            let raw = read_raw(&format!("{name_k}.graph")).await;
            let mut pages = std::collections::HashMap::new();
            for (i, c) in raw.chunks(4096).enumerate() {
                pages.insert(i as u64, c.to_vec());
            }
            if let Err(e) = PersistentFactStorage::open(
                BrowserBackend::Buffer(BrowserBufferBackend::load_pages(pages)),
                0,
                None,
            ) {
                assert_eq!(crate::MinigrafError::from(e).code(), "STG-033");
                assert_eq!(got.len(), 20, "the previous commit");
                torn_once = true;
            }
        }
        assert!(torn_once, "some tear left one valid slot and later pages");
    }

    #[wasm_bindgen_test]
    async fn open_outcomes_match_native_and_refusals_modify_nothing() {
        // A short file that is not a Minigraf database: STG-045, unchanged.
        let name = unique("short");
        write_raw(&format!("{name}.graph"), &[0x55; 100]).await;
        assert_eq!(
            code_of(&rejection(BrowserDb::open_opfs(name.clone()).await)),
            "STG-045"
        );
        assert_eq!(read_raw(&format!("{name}.graph")).await, vec![0x55; 100]);
        // Not a Minigraf file: STG-002, unchanged.
        let name = unique("foreign");
        write_raw(&format!("{name}.graph"), &vec![0xAA; 3 * 4096]).await;
        assert_eq!(
            code_of(&rejection(BrowserDb::open_opfs(name.clone()).await)),
            "STG-002"
        );
        assert_eq!(
            read_raw(&format!("{name}.graph")).await,
            vec![0xAA; 3 * 4096]
        );
        // A meta counting pages past the end: STG-044, unchanged.
        let name = unique("short-tail");
        let db = open(&name).await;
        put(&db, 0..300).await;
        let bytes = db.export_graph().unwrap().to_vec();
        db.close().unwrap();
        let cut = &bytes[..bytes.len() - 4096];
        write_raw(&format!("{name}.graph"), cut).await;
        assert_eq!(
            code_of(&rejection(BrowserDb::open_opfs(name.clone()).await)),
            "STG-044"
        );
        assert_eq!(read_raw(&format!("{name}.graph")).await, cut.to_vec());
        // Both slots damaged: STG-032.
        let name = unique("both-damaged");
        let mut damaged = bytes.clone();
        damaged[30] ^= 0xFF;
        damaged[4096 + 30] ^= 0xFF;
        write_raw(&format!("{name}.graph"), &damaged).await;
        assert_eq!(
            code_of(&rejection(BrowserDb::open_opfs(name.clone()).await)),
            "STG-032"
        );
        // A torn initial meta: initialised again.
        let name = unique("torn-initial");
        let mut page = vec![0u8; 4096];
        page[..40].copy_from_slice(&crate::storage::meta::MetaPage::empty(1).encode()[..40]);
        write_raw(&format!("{name}.graph"), &page).await;
        let db = open(&name).await;
        assert!(ns(&db).await.is_empty());
        db.close().unwrap();
    }

    // ── AC11: portability ──────────────────────────────────────────────────

    #[wasm_bindgen_test]
    async fn export_is_a_complete_file_that_verifies() {
        let name = unique("portable");
        let db = portability_db(&name).await;
        assert_portability_answers(&db, "OPFS").await;
        let bytes = db.export_graph().unwrap().to_vec();
        assert_eq!(&bytes[0..4], b"MGRF");
        image_of(bytes.clone()).expect("the export verifies");
        db.close().unwrap();
        // Back into a new OPFS database, across a reopen.
        let copy = unique("portable-copy");
        let db = open(&copy).await;
        db.import_graph(blob(&bytes)).await.unwrap();
        assert_portability_answers(&db, "imported export").await;
        db.close().unwrap();
        let db = open(&copy).await;
        assert_portability_answers(&db, "imported export, reopened").await;
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn native_v8_and_v7_files_import_and_survive_reopen() {
        let v8: &[u8] = include_bytes!("../../tests/fixtures/portability/native_v8.graph");
        let name = unique("native-v8");
        let db = open(&name).await;
        db.import_graph(blob(v8)).await.expect("import native v8");
        assert_portability_answers(&db, "native v8").await;
        db.close().unwrap();
        let db = open(&name).await;
        assert_portability_answers(&db, "native v8, reopened").await;
        db.close().unwrap();

        let v7: &[u8] = include_bytes!("../../tests/fixtures/compat.graph");
        let name = unique("native-v7");
        let db = open(&name).await;
        db.import_graph(blob(v7)).await.expect("import native v7");
        let names = exec(&db, "(query [:find ?n :where [?e :name ?n]])").await;
        assert_eq!(rows(&names), serde_json::json!([["Alice"]]));
        db.close().unwrap();
        let db = open(&name).await;
        let names = exec(&db, "(query [:find ?n :where [?e :name ?n]])").await;
        assert_eq!(rows(&names), serde_json::json!([["Alice"]]));
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn a_smaller_import_leaves_no_trailing_bytes() {
        let small = open(&unique("small")).await;
        put(&small, 0..2).await;
        let small_bytes = small.export_graph().unwrap().to_vec();
        small.close().unwrap();
        let name = unique("large");
        let db = open(&name).await;
        for i in 0..20 {
            put(&db, i * 100..(i + 1) * 100).await;
        }
        let large = file(&db).size().unwrap();
        db.import_graph(blob(&small_bytes)).await.unwrap();
        let size = file(&db).size().unwrap();
        assert!(size < large, "the file shrank");
        assert_eq!(
            size as usize,
            small_bytes.len(),
            "exactly the image's length"
        );
        assert_eq!(ns(&db).await, vec![0, 1]);
        db.close().unwrap();
    }

    // ── AC12: import ───────────────────────────────────────────────────────

    #[wasm_bindgen_test]
    async fn import_validation_rejects_before_replacing_anything() {
        let src = open(&unique("valid-src")).await;
        put(&src, 0..300).await;
        let good = src.export_graph().unwrap().to_vec();
        src.close().unwrap();

        let name = unique("valid");
        let db = open(&name).await;
        put(&db, 0..3).await;
        let cursor = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
        let pq = db.prepare("(query [:find ?n :where [?e :n ?n]])").unwrap();

        // Not a whole number of pages: API-017.
        let err = rejection(db.import_graph(blob(&good[..4097])).await);
        assert_eq!(code_of(&err), "API-017");
        // Page-aligned but truncated: STG-044 (IndexedDB mode accepts it).
        let truncated = &good[..good.len() - 4096];
        let err = rejection(db.import_graph(blob(truncated)).await);
        assert_eq!(code_of(&err), "STG-044");
        let mem = BrowserDb::open_in_memory().unwrap();
        mem.import_graph(blob(truncated))
            .await
            .expect("the buffer modes accept it");
        // A reachable page with a bad CRC: rejected (the buffer modes find
        // it only when reading it).
        let mut bad = good.clone();
        let page = 2 * 4096;
        bad[page + 100] ^= 0xFF;
        let err = rejection(db.import_graph(blob(&bad)).await);
        assert!(code_of(&err).starts_with("STG-"), "a storage code");
        mem.import_graph(blob(&bad)).await.expect("accepted lazily");

        // Nothing was replaced; the dependents still work; no leftovers.
        assert_eq!(ns(&db).await, vec![0, 1, 2]);
        assert!(pq.execute(JsValue::UNDEFINED).is_ok());
        drop(cursor);
        let mine: Vec<String> = entries()
            .await
            .into_iter()
            .filter(|e| e.starts_with(&name))
            .collect();
        assert_eq!(mine, vec![format!("{name}.graph")]);
        // An empty blob is an empty database.
        db.import_graph(blob(&[])).await.expect("empty import");
        assert!(ns(&db).await.is_empty());
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn a_published_import_invalidates_earlier_cursors_and_prepared_queries() {
        let name = unique("publish");
        let db = open(&name).await;
        put(&db, 0..3).await;
        let mut cursor = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
        let pq = db.prepare("(query [:find ?n :where [?e :n ?n]])").unwrap();
        let other = BrowserDb::open_in_memory().unwrap();
        put(&other, 10..12).await;
        db.import_graph(other.export_graph().unwrap())
            .await
            .expect("import");
        assert_eq!(code_of(&rejection(cursor.next_batch(10))), "API-018");
        assert_eq!(
            code_of(&rejection(pq.execute(JsValue::UNDEFINED))),
            "API-018"
        );
        assert_eq!(ns(&db).await, vec![10, 11], "new calls read the import");
        let pq2 = db.prepare("(query [:find ?n :where [?e :n ?n]])").unwrap();
        assert!(
            pq2.execute(JsValue::UNDEFINED).is_ok(),
            "new dependents work"
        );
        db.close().unwrap();
        let db = open(&name).await;
        assert_eq!(ns(&db).await, vec![10, 11]);
        db.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn calls_admitted_while_importing_are_busy() {
        let name = unique("importing");
        let db = open(&name).await;
        put(&db, 0..3).await;
        let mut cursor = db.query("(query [:find ?n :where [?e :n ?n]])").unwrap();
        let other = BrowserDb::open_in_memory().unwrap();
        put(&other, 10..12).await;
        let image = other.export_graph().unwrap();
        let import = {
            let d = BrowserDb {
                inner: db.inner.clone(),
            };
            wasm_bindgen_futures::future_to_promise(async move {
                d.import_graph(image).await.map(|()| JsValue::UNDEFINED)
            })
        };
        // Let the import's body start and wait on the browser.
        JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
            .await
            .unwrap();
        JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
            .await
            .unwrap();
        assert!(db.inner.importing.get(), "the import is in progress");
        let busy = |e: JsValue| assert_eq!(code_of(&e), "API-027");
        busy(rejection(db.query("(query [:find ?n :where [?e :n ?n]])")));
        busy(rejection(db.export_graph()));
        busy(rejection(db.begin_write()));
        busy(rejection(db.close()));
        busy(rejection(cursor.next_batch(1)));
        busy(rejection(db.execute("(transact [[:x :n 1]])".into()).await));
        assert_eq!(
            code_of(&rejection(BrowserDb::open_opfs(name.clone()).await)),
            "STG-026",
            "locked throughout"
        );
        cursor.close();
        JsFuture::from(import).await.expect("the import completes");
        assert_eq!(ns(&db).await, vec![10, 11]);
        db.close().unwrap();
    }

    /// An image staged like an import, ready for [`recover_import`] tests.
    fn staged_bytes(image: &[u8], generation: u64) -> Vec<u8> {
        let pfs = image_of(image.to_vec()).unwrap();
        let mut out = Vec::new();
        pfs.write_image(generation, OtherSlot::Zero, |_, page| {
            out.extend_from_slice(page);
            Ok(())
        })
        .unwrap();
        out
    }

    /// Spec AC12: an import interrupted at every write and flush of its
    /// staging file, of the database file's copy, and of the copy that an
    /// open redoes, reopens as the whole old or the whole new database; a
    /// failure before the publication point keeps the old one open, one
    /// after it closes the handle. Transient files are gone after an open
    /// that completes.
    #[wasm_bindgen_test]
    async fn an_interrupted_import_reopens_old_or_new() {
        let other = BrowserDb::open_in_memory().unwrap();
        put(&other, 100..400).await;
        let image = other.export_graph().unwrap().to_vec();
        let new: Vec<i64> = (100..400).collect();
        let old: Vec<i64> = (0..3).collect();
        let mut published_failures = 0;
        let mut kept_old = 0;
        for target in ["stage", "main"] {
            for flush in [false, true] {
                for k in 0..40u64 {
                    let name = unique("interrupt");
                    let db = open(&name).await;
                    put(&db, 0..3).await;
                    let faults = Faults {
                        write_after: (!flush).then_some((k, WriteFault::Throw(16))),
                        flush_after: flush.then_some(k),
                    };
                    match target {
                        "stage" => arm(&format!("{name}.graph-import"), faults),
                        _ => *file(&db).faults.lock().unwrap() = faults,
                    }
                    let result = db.import_graph(blob(&image)).await;
                    let _ = opfs::tests::take_armed(&format!("{name}.graph-import"));
                    if result.is_ok() {
                        assert_eq!(ns(&db).await, new);
                        db.close().unwrap();
                        break;
                    }
                    if db.inner.closed.get() {
                        published_failures += 1;
                    } else {
                        kept_old += 1;
                        assert_eq!(ns(&db).await, old, "the old database, still open");
                        db.close().unwrap();
                    }
                    drop(db);
                    let db = open(&name).await;
                    let got = ns(&db).await;
                    assert!(got == old || got == new, "old or new, whole");
                    db.close().unwrap();
                    let mine: Vec<String> = entries()
                        .await
                        .into_iter()
                        .filter(|e| e.starts_with(&name))
                        .collect();
                    assert_eq!(mine, vec![format!("{name}.graph")], "no leftovers");
                }
            }
        }
        assert!(
            published_failures > 0,
            "some failures came after publication"
        );
        assert!(kept_old > 0, "some came before it");

        // A second interruption, in the copy an open redoes.
        let name = unique("interrupt-twice");
        let db = open(&name).await;
        put(&db, 0..3).await;
        *file(&db).faults.lock().unwrap() = Faults {
            write_after: Some((3, WriteFault::Throw(0))),
            ..Faults::default()
        };
        assert!(db.import_graph(blob(&image)).await.is_err());
        assert!(db.inner.closed.get(), "failed while copying: closed");
        drop(db);
        arm(
            &format!("{name}.graph"),
            Faults {
                write_after: Some((5, WriteFault::Throw(0))),
                ..Faults::default()
            },
        );
        let err = rejection(BrowserDb::open_opfs(name.clone()).await);
        assert_eq!(code_of(&err), "STG-048");
        let db = open(&name).await;
        assert_eq!(ns(&db).await, new, "the redone copy completes");
        db.close().unwrap();
    }

    /// A staging file left after its copy finished (a failed cleanup) is
    /// removed at the next open, never copied over later commits; one that
    /// never reached its publication point is removed too.
    #[wasm_bindgen_test]
    async fn leftover_staging_files_are_removed_not_replayed() {
        let other = BrowserDb::open_in_memory().unwrap();
        put(&other, 100..110).await;
        let image = other.export_graph().unwrap().to_vec();

        let name = unique("consumed");
        let db = open(&name).await;
        db.import_graph(blob(&image)).await.unwrap();
        let g = file(&db).newest_generation().unwrap().unwrap();
        put(&db, 0..2).await;
        db.close().unwrap();
        write_raw(&format!("{name}.graph-import"), &staged_bytes(&image, g)).await;
        let db = open(&name).await;
        let mut want: Vec<i64> = (100..110).collect();
        want.extend([0, 1]);
        want.sort_unstable();
        assert_eq!(ns(&db).await, want, "later commits kept");
        db.close().unwrap();

        let name = unique("unpublished");
        let db = open(&name).await;
        put(&db, 0..2).await;
        db.close().unwrap();
        let mut staged = staged_bytes(&image, 99);
        staged[..4096].fill(0); // generation 99's slot is page 0: no meta
        write_raw(&format!("{name}.graph-import"), &staged).await;
        let db = open(&name).await;
        assert_eq!(
            ns(&db).await,
            vec![0, 1],
            "an unpublished import is dropped"
        );
        db.close().unwrap();
        for n in [&name] {
            let mine: Vec<String> = entries()
                .await
                .into_iter()
                .filter(|e| e.starts_with(n.as_str()))
                .collect();
            assert_eq!(mine, vec![format!("{n}.graph")]);
        }
    }

    // ── AC13: bounded reads and memory ─────────────────────────────────────

    /// Spec AC13(a): opening a healthy database reads a fixed number of
    /// pages, the same for databases whose fact counts differ tenfold.
    #[wasm_bindgen_test]
    async fn opfs_open_read_bound() {
        let mut opened = Vec::new();
        for facts in [500i64, 5_000] {
            let name = unique("read-bound");
            let db = open(&name).await;
            for chunk in (0..facts).step_by(500) {
                put(&db, chunk..chunk + 500).await;
            }
            db.close().unwrap();
            let before = reads();
            let db = open(&name).await;
            opened.push((reads() - before, file(&db).page_count().unwrap()));
            db.close().unwrap();
        }
        let (small, large) = (opened[0], opened[1]);
        assert!(large.1 > 5 * small.1, "the larger file is much larger");
        assert_eq!(small.0, large.0, "the same pages read at open");
        assert!(small.0 <= 4, "a fixed handful of pages: {}", small.0);
        // IndexedDB mode reads every page at open.
        let name = unique("read-bound-idb");
        let db = BrowserDb::open(&name).await.unwrap();
        put(&db, 0..500).await;
        db.close().unwrap();
        let db = BrowserDb::open(&name).await.unwrap();
        let held = db.inner.borrow().pfs.with_backend(|b| {
            b.buffer()
                .map_or(0, BrowserBufferBackend::stored_page_count)
        });
        assert!(held > 4, "IndexedDB mode holds every page");
        db.close().unwrap();
    }

    /// Spec AC13(b): after touching more pages than the cache holds, with no
    /// cursor open, the cache holds at most its capacity, and the backend no
    /// pages at all.
    #[wasm_bindgen_test]
    async fn opfs_cache_residency_bound() {
        let name = unique("residency");
        let db = open(&name).await;
        for chunk in 0..40 {
            put(&db, chunk * 500..(chunk + 1) * 500).await;
        }
        let pages = file(&db).page_count().unwrap();
        assert!(
            pages > 2 * OPFS_PAGE_CACHE as u64,
            "more pages than the cache"
        );
        exec(&db, "(query [:find (count ?e) :where [?e :n ?n]])").await;
        exec(&db, "(query [:find ?n :where [:e19999 :n ?n]])").await;
        let inner = db.inner.borrow();
        let cached = inner.pfs.cached_page_count();
        assert!(cached <= OPFS_PAGE_CACHE, "cache within capacity: {cached}");
        assert!(
            inner.pfs.with_backend(|b| b.buffer().is_none()),
            "no page buffer under OPFS mode"
        );
        drop(inner);
        db.close().unwrap();
    }

    // ── AC14: IndexedDB in a worker ────────────────────────────────────────

    #[wasm_bindgen_test]
    async fn indexeddb_mode_works_in_a_worker() {
        let name = unique("idb-worker");
        let db = BrowserDb::open(&name).await.expect("IndexedDB in a worker");
        put(&db, 0..3).await;
        db.checkpoint().await.unwrap();
        db.close().unwrap();
        let db = BrowserDb::open(&name).await.unwrap();
        assert_eq!(ns(&db).await, vec![0, 1, 2]);
        db.close().unwrap();
    }
}
