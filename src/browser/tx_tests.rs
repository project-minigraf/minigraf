//! `BrowserWriteTransaction` (#824): staging, read-your-own-writes, commit
//! and rollback against IndexedDB, the `[INT-001]` guard on the database
//! while a transaction is open, `[API-018]` after it finishes, and a
//! differential check against the native transaction.

use super::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const NAMES: &str = "(query [:find ?n :where [?e :person/name ?n]])";
const SETUP: &str = r#"(transact [[:alice :person/name "Alice"] [:alice :person/age 30]
    [:bob :person/name "Bob"] [:carol :person/name "Carol"]])"#;

/// A JSON result's rows, each encoded, sorted.
fn sorted_rows(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let mut rows: Vec<String> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    rows.sort();
    rows
}

/// The `[CODE]` an error message starts with.
fn code_of(e: &JsValue) -> String {
    let s = e.as_string().expect("an error message");
    s.split(']')
        .next()
        .unwrap_or("")
        .trim_start_matches('[')
        .to_string()
}

async fn commit(tx: &mut BrowserWriteTransaction) -> Result<JsValue, JsValue> {
    JsFuture::from(tx.commit()).await
}

async fn names(db: &BrowserDb) -> Vec<String> {
    sorted_rows(&db.execute(NAMES.to_string()).await.expect("query"))
}

async fn setup() -> BrowserDb {
    let db = BrowserDb::open_in_memory().expect("open_in_memory");
    db.execute(SETUP.to_string()).await.expect("setup");
    db
}

// ─── B2 ──────────────────────────────────────────────────────────────────────

#[wasm_bindgen_test]
async fn execute_stages_and_reads_own_writes() {
    let db = setup().await;
    let mut tx = db.begin_write().expect("begin");
    assert_eq!(
        tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
            .expect("transact"),
        r#"{"ok":true}"#
    );
    assert_eq!(
        tx.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
            .expect("retract"),
        r#"{"ok":true}"#
    );
    let inside = tx.execute(NAMES).expect("query");
    let v: serde_json::Value = serde_json::from_str(&inside).unwrap();
    assert_eq!(v["variables"], serde_json::json!(["?n"]), "execute's JSON");
    assert_eq!(
        sorted_rows(&inside),
        vec![r#"["Alice"]"#, r#"["Carol"]"#, r#"["Dave"]"#]
    );
    assert_eq!(
        tx.execute("(rule [(named ?e ?n) [?e :person/name ?n]])")
            .expect("rule"),
        r#"{"ok":true}"#
    );
    let ruled = tx
        .execute("(query [:find ?n :where (named ?e ?n)])")
        .expect("rule query");
    assert_eq!(sorted_rows(&ruled).len(), 3, "the rule reads staged facts");
    commit(&mut tx).await.expect("commit");
    assert_eq!(
        names(&db).await,
        vec![r#"["Alice"]"#, r#"["Carol"]"#, r#"["Dave"]"#]
    );
}

#[wasm_bindgen_test]
async fn failed_statement_stages_nothing_and_stays_open() {
    let db = setup().await;
    let mut tx = db.begin_write().expect("begin");
    assert_eq!(
        code_of(&tx.execute("(transact [[:a").unwrap_err()),
        "PRS-004"
    );
    let two_windows = r#"(transact [[:x :p/v 1 {:valid-from "2020-01-01T00:00:00Z"}]
        [:x :p/v 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#;
    assert_eq!(code_of(&tx.execute(two_windows).unwrap_err()), "API-011");
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .expect("a good statement after the failures");
    commit(&mut tx).await.expect("commit");
    assert_eq!(names(&db).await.len(), 4, "only the good statement");
    let v = db
        .execute("(query [:find ?e :where [?e :p/v _]])".to_string())
        .await
        .expect("query");
    assert_eq!(sorted_rows(&v).len(), 0, "nothing of the failed one");
}

// ─── B3: differential check against the native transaction ──────────────────

/// Sequences of statements, each run in one transaction and committed: the
/// browser database must end where an in-memory `Minigraf` ends.
const SEQUENCES: &[&[&str]] = &[
    &[],
    &[NAMES],
    &[
        r#"(transact [[:dave :person/name "Dave"] [:dave :person/age 40]])"#,
        NAMES,
        r#"(retract [[:bob :person/name "Bob"]])"#,
        "(transact [[:alice :person/age 31]])",
        "(retract [[:alice :person/age 30]])",
    ],
    &[
        // Latest statement wins for a triple written twice (#477).
        r#"(transact [[:erin :person/name "Erin" {:valid-from "2020-01-01T00:00:00Z"}]])"#,
        r#"(transact [[:erin :person/name "Erin" {:valid-from "2021-01-01T00:00:00Z" :valid-to "2030-01-01T00:00:00Z"}]])"#,
        // API-011 and an inverted window (API-019): rejected, nothing staged.
        r#"(transact [[:x :p/v 1 {:valid-from "2020-01-01T00:00:00Z"}] [:x :p/v 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#,
        r#"(transact [[:y :p/v 1 {:valid-from "2021-01-01T00:00:00Z" :valid-to "2020-01-01T00:00:00Z"}]])"#,
        "(transact [[:x :p/v",
        "(query [:find ?e :where [?e :person/name \"Erin\"]])",
    ],
    &[
        "(rule [(friendly ?e ?n) [?e :person/name ?n] [?e :person/friend _]])",
        r#"(transact [[:frank :person/name "Frank"] [:frank :person/friend :alice] [:gina :person/name "Gina"]])"#,
        "(query [:find ?n :where (friendly ?e ?n)])",
        r#"(retract [[:carol :person/name "Carol"]])"#,
        r#"(transact [[:carol :person/name "Carol"]])"#,
        "(query [:find ?n ?a :where [?e :person/name ?n] [?e :person/age ?a]])",
    ],
];

/// The checks run on both databases after each sequence.
const CHECKS: &[&str] = &[
    "(query [:find ?e ?a ?v :where [?e ?a ?v]])",
    "(query [:find ?e ?a ?v ?t :any-valid-time :where [?e ?a ?v] [?e :db/tx-count ?t]])",
    "(query [:find ?e ?v :valid-at \"2025-06-01T00:00:00Z\" :where [?e :person/name ?v]])",
    "(query [:find ?e ?v :as-of 1 :where [?e :person/name ?v]])",
];

/// What a statement returned, comparable across the two databases.
fn outcome(result: Result<String, String>) -> String {
    match result {
        Ok(json) if json.contains("\"results\"") => sorted_rows(&json).join(";"),
        Ok(json) => json,
        Err(code) => code,
    }
}

#[wasm_bindgen_test]
async fn commit_matches_native_transaction() {
    for (i, stmts) in SEQUENCES.iter().enumerate() {
        let browser = setup().await;
        let native = crate::Minigraf::in_memory().unwrap();
        native.execute(SETUP).unwrap();

        let mut btx = browser.begin_write().expect("begin");
        let mut ntx = native.begin_write().unwrap();
        for stmt in *stmts {
            let b = outcome(btx.execute(stmt).map_err(|e| code_of(&e)));
            let n = outcome(
                ntx.execute(stmt)
                    .map(query_result_to_json)
                    .map_err(|e| e.code().to_string()),
            );
            assert_eq!(b, n, "sequence {i}: statement outcomes differ");
        }
        commit(&mut btx).await.expect("browser commit");
        ntx.commit().unwrap();

        for check in CHECKS {
            let b = browser.execute((*check).to_string()).await.expect("check");
            let n = query_result_to_json(native.execute(check).unwrap());
            assert_eq!(
                sorted_rows(&b),
                sorted_rows(&n),
                "sequence {i}: results differ"
            );
        }
        let bcount = browser.inner.borrow().fact_storage.current_tx_count();
        assert_eq!(bcount, native.current_tx_count(), "sequence {i}: tx_count");
    }
}

// ─── B4, B5: commit, rollback and drop against IndexedDB ────────────────────

async fn fresh_db(name: &str) -> BrowserDb {
    drop(super::tests::empty_store(name).await);
    BrowserDb::open(name).await.expect("open")
}

#[wasm_bindgen_test]
async fn commit_is_one_transaction_and_survives_reopen() {
    let name = "minigraf-test-tx-commit";
    let db = fresh_db(name).await;
    db.execute(SETUP.to_string()).await.expect("setup");
    let before = db.inner.borrow().fact_storage.current_tx_count();
    let mut tx = db.begin_write().expect("begin");
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    tx.execute(r#"(transact [[:erin :person/name "Erin"]])"#)
        .unwrap();
    tx.execute(r#"(retract [[:bob :person/name "Bob"]])"#)
        .unwrap();
    let pending = tx.commit();
    // Finished and visible before the flush resolves.
    assert_eq!(
        names(&db).await,
        vec![r#"["Alice"]"#, r#"["Carol"]"#, r#"["Dave"]"#, r#"["Erin"]"#]
    );
    JsFuture::from(pending).await.expect("flush");
    assert_eq!(
        db.inner.borrow().fact_storage.current_tx_count(),
        before + 1,
        "one tx_count for the whole transaction"
    );
    let counts = db
        .execute(
            "(query [:find ?t :any-valid-time :where [?e :person/name _] [?e :db/tx-count ?t]])"
                .to_string(),
        )
        .await
        .unwrap();
    assert_eq!(
        sorted_rows(&counts),
        vec!["[1]", "[1]", "[2]", "[2]"],
        "Alice and Carol from the setup, Dave and Erin from the one commit"
    );
    drop(db);
    let db = BrowserDb::open(name).await.expect("reopen");
    assert_eq!(names(&db).await.len(), 4, "the commit after reopening");
}

#[wasm_bindgen_test]
async fn rollback_and_drop_discard_and_survive_reopen() {
    let name = "minigraf-test-tx-rollback";
    let db = fresh_db(name).await;
    db.execute(SETUP.to_string()).await.expect("setup");

    let mut tx = db.begin_write().expect("begin");
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    tx.rollback();
    assert_eq!(names(&db).await.len(), 3, "rolled back");

    let mut tx = db.begin_write().expect("begin after rollback");
    tx.execute(r#"(transact [[:erin :person/name "Erin"]])"#)
        .unwrap();
    drop(tx); // as JavaScript's free() does
    assert_eq!(names(&db).await.len(), 3, "dropped");

    drop(db);
    let db = BrowserDb::open(name).await.expect("reopen");
    assert_eq!(names(&db).await.len(), 3, "neither after reopening");
}

/// A failed IndexedDB flush rejects the commit's promise and leaves what a
/// failed flush of `execute` leaves: the facts in memory, not in the store.
#[wasm_bindgen_test]
async fn failed_flush_rejects_like_execute() {
    let name = "minigraf-test-tx-flush";
    let db = fresh_db(name).await;
    db.execute(SETUP.to_string()).await.expect("setup");
    db.inner.borrow().idb.as_ref().unwrap().db.close();

    let failed_execute = db
        .execute(r#"(transact [[:dave :person/name "Dave"]])"#.to_string())
        .await;
    assert!(failed_execute.is_err(), "execute's flush fails");
    let mut tx = db.begin_write().expect("begin");
    tx.execute(r#"(transact [[:erin :person/name "Erin"]])"#)
        .unwrap();
    assert!(commit(&mut tx).await.is_err(), "the commit's flush fails");
    assert_eq!(names(&db).await.len(), 5, "both in memory, as before");
    assert!(
        tx.execute(NAMES).is_err(),
        "the transaction is finished after a failed flush"
    );
    drop(db);
    let db = BrowserDb::open(name).await.expect("reopen");
    assert_eq!(names(&db).await.len(), 3, "neither reached the store");
}

#[wasm_bindgen_test]
async fn refused_commit_finishes_and_discards() {
    let db = setup().await;
    let mut tx = db.begin_write().expect("begin");
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    let soon = crate::graph::types::tx_id_now().saturating_add(20);
    for f in &mut tx.staged.as_mut().unwrap().pending_facts {
        f.valid_to = soon.cast_signed();
    }
    while crate::graph::types::tx_id_now() <= soon {}
    let err = commit(&mut tx)
        .await
        .expect_err("window ends before commit");
    assert_eq!(code_of(&err), "API-019");
    assert_eq!(names(&db).await.len(), 3, "nothing committed");
    assert_eq!(code_of(&tx.execute(NAMES).unwrap_err()), "API-018");
}

#[wasm_bindgen_test]
async fn empty_and_query_only_transactions() {
    let db = setup().await;
    let before = db.inner.borrow().fact_storage.current_tx_count();
    let mut tx = db.begin_write().expect("begin");
    commit(&mut tx).await.expect("empty commit");
    let mut tx = db.begin_write().expect("begin");
    tx.execute(NAMES).unwrap();
    commit(&mut tx).await.expect("query-only commit");
    let mut tx = db.begin_write().expect("begin");
    tx.execute(NAMES).unwrap();
    tx.rollback();
    assert_eq!(db.inner.borrow().fact_storage.current_tx_count(), before);
}

// ─── B6, B7: the guard and finished transactions ────────────────────────────

/// Every guarded `BrowserDb` call's error code, or `None` on success.
///
/// `importGraph` is called only while a transaction is open: it is refused
/// before it reads the blob, and would otherwise replace the database.
async fn guarded(
    db: &BrowserDb,
    pq: &BrowserPreparedQuery,
    open: bool,
) -> Vec<(&'static str, Option<String>)> {
    let code = |r: Result<(), JsValue>| r.err().map(|e| code_of(&e));
    let empty = js_sys::Object::new().into();
    let import = if open {
        code(
            db.import_graph(js_sys::Uint8Array::new_with_length(0))
                .await,
        )
    } else {
        None
    };
    vec![
        (
            "execute",
            code(db.execute(NAMES.to_string()).await.map(|_| ())),
        ),
        ("query", code(db.query(NAMES).map(|_| ()))),
        ("prepare", code(db.prepare(NAMES).map(|_| ()))),
        ("checkpoint", code(db.checkpoint().await)),
        ("importGraph", import),
        ("beginWrite", code(db.begin_write().map(|_| ()))),
        ("prepared execute", code(pq.execute(empty).map(|_| ()))),
        (
            "prepared query",
            code(pq.query(js_sys::Object::new().into()).map(|_| ())),
        ),
    ]
}

#[wasm_bindgen_test]
async fn database_calls_are_int001_while_open() {
    let db = setup().await;
    let pq = db.prepare(NAMES).expect("prepare");
    let mut cursor = db.query(NAMES).expect("cursor before begin");
    let mut tx = db.begin_write().expect("begin");
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .unwrap();
    for (name, code) in guarded(&db, &pq, true).await {
        assert_eq!(code.as_deref(), Some("INT-001"), "{name} while open");
    }
    let rows = cursor.next_batch(10).expect("cursor").expect("rows");
    assert_eq!(sorted_rows(&format!(r#"{{"results":{rows}}}"#)).len(), 3);
    commit(&mut tx).await.expect("commit");

    // After the commit everything works.
    for (name, code) in guarded(&db, &pq, false).await {
        assert_eq!(code, None, "{name} after the commit");
    }
    // After a rollback and a drop too.
    db.begin_write().expect("begin").rollback();
    drop(db.begin_write().expect("begin"));
    assert_eq!(names(&db).await.len(), 4);
}

#[wasm_bindgen_test]
async fn finished_transaction_is_api018() {
    let db = setup().await;
    let mut tx = db.begin_write().expect("begin");
    commit(&mut tx).await.expect("commit");
    assert_eq!(code_of(&tx.execute(NAMES).unwrap_err()), "API-018");
    assert_eq!(code_of(&commit(&mut tx).await.unwrap_err()), "API-018");
    tx.rollback(); // does nothing

    let mut tx = db.begin_write().expect("begin");
    tx.rollback();
    tx.rollback();
    assert_eq!(code_of(&tx.execute(NAMES).unwrap_err()), "API-018");
    assert_eq!(code_of(&commit(&mut tx).await.unwrap_err()), "API-018");
    // A finished transaction no longer guards the database.
    assert_eq!(names(&db).await.len(), 3);
}

#[wasm_bindgen_test]
async fn rules_stay_after_rollback() {
    let db = setup().await;
    let mut tx = db.begin_write().expect("begin");
    tx.execute("(rule [(named ?e ?n) [?e :person/name ?n]])")
        .unwrap();
    tx.rollback();
    let r = db
        .execute("(query [:find ?n :where (named ?e ?n)])".to_string())
        .await
        .expect("the rule is still registered");
    assert_eq!(sorted_rows(&r).len(), 3);
}

/// A user function of a transaction's query that calls the database (here
/// `beginWrite` and `query`) is refused as `[INT-001]` is, so the query fails with
/// `[QRY-014]`; nothing panics and the transaction stays usable.
#[wasm_bindgen_test]
async fn user_function_calling_the_database_inside_a_transaction() {
    let db = setup().await;
    let calls = [
        ("begins?", "db.beginWrite(); return true"),
        (
            "queries?",
            "db.query('(query [:find ?n :where [?e :person/name ?n]])'); return true",
        ),
    ];
    for (name, body) in calls {
        let other = BrowserDb {
            inner: db.inner.clone(),
        };
        let f = js_sys::Function::new_with_args("db", &format!("return (v) => {{ {body} }}"));
        let predicate = f
            .call1(&JsValue::NULL, &JsValue::from(other))
            .expect("make the predicate");
        db.register_predicate(name, predicate).expect("register");
    }
    let mut tx = db.begin_write().expect("begin");
    for (name, _) in calls {
        let q = format!("(query [:find ?n :where [?e :person/name ?n] [({name} ?n)]])");
        let e = tx.execute(&q).expect_err("a call back into the database");
        let msg = e.as_string().expect("a message");
        assert!(
            msg.starts_with("[QRY-014]")
                && msg.contains("write transaction is already in progress"),
            "{name}: {msg}"
        );
    }
    tx.execute(r#"(transact [[:dave :person/name "Dave"]])"#)
        .expect("still usable");
    commit(&mut tx).await.expect("commit");
    assert_eq!(names(&db).await.len(), 4);
}
