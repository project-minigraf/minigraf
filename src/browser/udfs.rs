//! User-defined functions from JavaScript: `BrowserDb.registerPredicate` and
//! `BrowserDb.registerAggregate` (#180, spec §6.2).
//!
//! A function receives each value as a `{ type, value }` object, as Node's
//! `FactValue`: `type` is `'string'`, `'integer'` (`value` a `bigint`),
//! `'float'`, `'boolean'`, `'ref'` (an entity UUID string), `'keyword'`,
//! `'null'` (no `value`) or `'set'` (`value` an array of these, in canonical
//! order). An aggregate's `finish` returns a value in any form a prepared
//! query's binding accepts (`binds.rs`): a plain value or a typed form.
//!
//! A function that throws fails the query with `QRY-014`, carrying the
//! thrown error's `message` (else `String(e)`), and so does a predicate that
//! returns anything but a boolean, or a `finish` whose result is not a value.
//!
//! The registry is shared by every thread that queries the database, so it
//! holds no JavaScript value: each function and accumulator lives in a
//! table local to the thread that registered it ([`Held`]), and the
//! registry keeps its id. A call made on any other thread fails with
//! `called off the JavaScript thread` (only a build with atomics could make
//! one). This needs no `unsafe`, which the crate forbids.
//!
//! Every call runs synchronously inside the query that makes it. A query
//! holds `inner.borrow()`, and no synchronous `BrowserDb` method takes
//! `borrow_mut`, so a function may call `query`, `prepare`, `exportGraph`
//! and the `register*` methods. `execute`, `checkpoint` and `importGraph`
//! are `async`: their bodies run from a microtask, after the outer query has
//! returned.

use super::BrowserDb;
use super::binds::read_value;
use crate::graph::types::Value;
use crate::query::datalog::functions::{UdfError, udf_aggregate, udf_predicate};
use js_sys::{Array, Function, Reflect};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

thread_local! {
    /// The JavaScript values this thread's functions hold, by id.
    static HELD: RefCell<HashMap<u64, JsValue>> = RefCell::new(HashMap::new());
}

/// Ids for [`Held`], unique across threads, so an id never names another
/// thread's value.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A JavaScript value in this thread's [`HELD`] table, removed when this
/// drops. `Send + Sync` as the registry needs, since it is only an id.
struct Held(u64);

impl Held {
    fn new(value: JsValue) -> Held {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        HELD.with(|h| h.borrow_mut().insert(id, value));
        Held(id)
    }

    /// The value; never borrowed across a call into JavaScript, which may
    /// register functions or open queries that hold new values.
    fn get(&self) -> Result<JsValue, UdfError> {
        HELD.with(|h| h.borrow().get(&self.0).cloned())
            .ok_or_else(|| UdfError::new("called off the JavaScript thread"))
    }

    fn set(&self, value: JsValue) {
        HELD.with(|h| h.borrow_mut().insert(self.0, value));
    }

    fn function(&self) -> Result<Function, UdfError> {
        // Only functions are held under a function's id.
        Ok(self.get()?.unchecked_into())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // On another thread, or while the table is torn down, nothing to do.
        let _ = HELD.try_with(|h| {
            if let Ok(mut h) = h.try_borrow_mut() {
                h.remove(&self.0);
            }
        });
    }
}

#[wasm_bindgen]
extern "C" {
    /// The global `String`, which converts any value, as a template
    /// literal does not for a `Symbol`.
    #[wasm_bindgen(catch, js_name = String)]
    fn js_string(v: &JsValue) -> Result<String, JsValue>;
}

#[wasm_bindgen]
impl BrowserDb {
    /// Register `fn` as a predicate named `name`, called in a query as
    /// `[(name ?v)]`. `fn` receives a `{ type, value }` object and returns
    /// a boolean.
    ///
    /// ```js
    /// db.registerPredicate('adult?', (v) => v.type === 'integer' && v.value >= 18n);
    /// ```
    ///
    /// Synchronous. A query that is already open does not see the
    /// function; every query opened afterwards does. Throws a `TypeError`
    /// when `fn` is not a function, and `[INT-040]` when `name` is taken.
    #[wasm_bindgen(js_name = registerPredicate)]
    pub fn register_predicate(&self, name: &str, f: JsValue) -> Result<(), JsValue> {
        let f = Held::new(function(f, "the predicate")?.into());
        let desc = udf_predicate(move |v: &Value| {
            let result = f
                .function()?
                .call1(&JsValue::UNDEFINED, &value_to_js(v))
                .map_err(thrown)?;
            result.as_bool().ok_or_else(|| {
                UdfError::new(format!("returned {}, not a boolean", type_of(&result)))
            })
        });
        let functions = self.inner.borrow().functions.clone();
        functions
            .register(|reg| reg.register_predicate_desc(name.to_string(), desc))
            .map_err(super::to_js_error)
    }

    /// Register an aggregate named `name`, used in `:find` as `(name ?v)`
    /// and as a window function with `:over`. `aggregate` is
    /// `{ init, step, finish }`: `init()` returns a new accumulator (one per
    /// group or window partition), `step(acc, value)` returns the
    /// accumulator after `value`, and `finish(acc, count)` returns the
    /// result, `count` being the number of values stepped. They are called
    /// as plain functions, with no `this`.
    ///
    /// ```js
    /// db.registerAggregate('product', {
    ///   init: () => 1n,
    ///   step: (acc, v) => acc * v.value,
    ///   finish: (acc) => acc,
    /// });
    /// ```
    ///
    /// Synchronous. Throws a `TypeError` when `init`, `step` or `finish` is
    /// not a function, and `[INT-040]` when `name` is taken.
    #[wasm_bindgen(js_name = registerAggregate)]
    pub fn register_aggregate(&self, name: &str, aggregate: JsValue) -> Result<(), JsValue> {
        let member = |key: &str| -> Result<Held, JsValue> {
            let f = if aggregate.is_object() {
                Reflect::get(&aggregate, &JsValue::from_str(key))?
            } else {
                JsValue::UNDEFINED
            };
            function(f, &format!("the aggregate's {key}")).map(|f| Held::new(f.into()))
        };
        let (init, step, finish) = (member("init")?, member("step")?, member("finish")?);
        let desc = udf_aggregate(
            move || {
                let acc = init
                    .function()?
                    .call0(&JsValue::UNDEFINED)
                    .map_err(thrown)?;
                Ok(Held::new(acc))
            },
            move |acc: &mut Held, v: &Value| {
                let next = step
                    .function()?
                    .call2(&JsValue::UNDEFINED, &acc.get()?, &value_to_js(v))
                    .map_err(thrown)?;
                acc.set(next);
                Ok(())
            },
            move |acc: &Held, count: usize| {
                // A count past 2^53 is not exact as a number; no query gets near.
                #[allow(clippy::cast_precision_loss)]
                let count = JsValue::from_f64(count as f64);
                let result = finish
                    .function()?
                    .call2(&JsValue::UNDEFINED, &acc.get()?, &count)
                    .map_err(thrown)?;
                read_value("finish's result", &result).map_err(thrown)
            },
        );
        let functions = self.inner.borrow().functions.clone();
        functions
            .register(|reg| reg.register_aggregate_desc(name.to_string(), desc))
            .map_err(super::to_js_error)
    }
}

/// `f` as a function, or a `TypeError` naming `what`.
fn function(f: JsValue, what: &str) -> Result<Function, JsValue> {
    f.dyn_into::<Function>()
        .map_err(|_| js_sys::TypeError::new(&format!("{what} must be a function")).into())
}

fn type_of(v: &JsValue) -> String {
    v.js_typeof().as_string().unwrap_or_default()
}

/// What a function threw, as a failed call: the error's `message` when it
/// has a string one, else `String(e)`. A message read from one of
/// `binds.rs`'s coded errors (`[API-017] ...`) drops the code, which
/// belongs to the bind API, not to the query's `QRY-014`.
fn thrown(e: JsValue) -> UdfError {
    let message = e
        .is_object()
        .then(|| Reflect::get(&e, &JsValue::from_str("message")).ok())
        .flatten()
        .and_then(|m| m.as_string())
        .or_else(|| e.as_string())
        .or_else(|| js_string(&e).ok())
        .unwrap_or_else(|| "user function failed".to_string());
    UdfError::new(strip_code(&message))
}

/// `message` without a leading `[ABC-123] ` error code.
fn strip_code(message: &str) -> &str {
    let code = message.get(1..8).unwrap_or_default();
    let is_code = code.len() == 7
        && code.char_indices().all(|(i, c)| match i {
            0..=2 => c.is_ascii_uppercase(),
            3 => c == '-',
            _ => c.is_ascii_digit(),
        });
    match message.get(10..) {
        Some(rest) if is_code && message.starts_with('[') && message.get(8..10) == Some("] ") => {
            rest
        }
        _ => message,
    }
}

/// `v` as a `{ type, value }` object.
fn value_to_js(v: &Value) -> JsValue {
    let (kind, value) = match v {
        Value::String(s) => ("string", Some(JsValue::from_str(s))),
        Value::Integer(i) => ("integer", Some(JsValue::from(*i))),
        Value::Float(f) => ("float", Some(JsValue::from_f64(*f))),
        Value::Boolean(b) => ("boolean", Some(JsValue::from_bool(*b))),
        Value::Ref(id) => ("ref", Some(JsValue::from_str(&id.to_string()))),
        Value::Keyword(k) => ("keyword", Some(JsValue::from_str(k))),
        Value::Null => ("null", None),
        Value::Set(set) => (
            "set",
            Some(set.iter().map(value_to_js).collect::<Array>().into()),
        ),
    };
    let object = js_sys::Object::new();
    // `Reflect::set` fails only on a frozen or proxied target.
    let _ = Reflect::set(
        &object,
        &JsValue::from_str("type"),
        &JsValue::from_str(kind),
    );
    if let Some(value) = value {
        let _ = Reflect::set(&object, &JsValue::from_str("value"), &value);
    }
    object.into()
}

#[cfg(all(target_arch = "wasm32", feature = "browser", test))]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn js_fn(args: &str, body: &str) -> JsValue {
        Function::new_with_args(args, body).into()
    }

    /// A function of one argument from a Rust closure.
    fn rust_fn(f: impl FnMut(JsValue) -> JsValue + 'static) -> JsValue {
        Closure::<dyn FnMut(JsValue) -> JsValue>::new(f).into_js_value()
    }

    fn clone_db(db: &BrowserDb) -> BrowserDb {
        BrowserDb {
            inner: db.inner.clone(),
        }
    }

    async fn rows(db: &BrowserDb, q: &str) -> serde_json::Value {
        let r = db.execute(q.to_string()).await.expect("query");
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        v["results"].clone()
    }

    async fn rejection(db: &BrowserDb, q: &str) -> String {
        db.execute(q.to_string())
            .await
            .expect_err("query must reject")
            .as_string()
            .expect("rejection is a string")
    }

    async fn numbers(db: &BrowserDb) {
        db.execute(
            "(transact [[:a :n 1] [:b :n 2] [:c :n 3] [:a :g :x] [:b :g :x] [:c :g :y]])".into(),
        )
        .await
        .expect("transact");
    }

    #[wasm_bindgen_test]
    async fn a_predicate_filters_rows() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        db.register_predicate(
            "big?",
            js_fn("v", "return v.type === 'integer' && v.value > 1n"),
        )
        .expect("register");
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n] [(big? ?n)]])").await;
        let mut got: Vec<i64> = got
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_i64().unwrap())
            .collect();
        got.sort_unstable();
        assert_eq!(got, vec![2, 3]);
    }

    #[wasm_bindgen_test]
    async fn values_reach_javascript_as_typed_objects() {
        let db = BrowserDb::open_in_memory().unwrap();
        db.execute(
            r#"(transact [[:a :v "s"] [:a :v 7] [:a :v 1.5] [:a :v true] [:a :v :k] [:a :v nil] [:a :v #{2 "x"}]])"#.into(),
        )
        .await
        .expect("transact");
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let log = seen.clone();
        db.register_predicate(
            "see",
            rust_fn(move |v| {
                log.borrow_mut().push(
                    js_sys::JSON::stringify_with_replacer(
                        &v,
                        &js_fn(
                            "k, x",
                            "return typeof x === 'bigint' ? x.toString() + 'n' : x",
                        ),
                    )
                    .unwrap()
                    .into(),
                );
                JsValue::TRUE
            }),
        )
        .unwrap();
        rows(&db, "(query [:find ?v :where [:a :v ?v] [(see ?v)]])").await;
        let mut seen = seen.borrow().clone();
        seen.sort();
        assert_eq!(
            seen,
            vec![
                r#"{"type":"boolean","value":true}"#,
                r#"{"type":"float","value":1.5}"#,
                r#"{"type":"integer","value":"7n"}"#,
                r#"{"type":"keyword","value":":k"}"#,
                r#"{"type":"null"}"#,
                r#"{"type":"set","value":[{"type":"string","value":"x"},{"type":"integer","value":"2n"}]}"#,
                r#"{"type":"string","value":"s"}"#,
            ]
        );
    }

    #[wasm_bindgen_test]
    async fn a_ref_reaches_javascript_as_its_uuid() {
        let db = BrowserDb::open_in_memory().unwrap();
        let kind = Rc::new(RefCell::new(String::new()));
        let log = kind.clone();
        db.register_predicate(
            "ref?",
            rust_fn(move |v| {
                let t = Reflect::get(&v, &"type".into())
                    .unwrap()
                    .as_string()
                    .unwrap();
                let value = Reflect::get(&v, &"value".into())
                    .unwrap()
                    .as_string()
                    .unwrap();
                *log.borrow_mut() = format!("{t} {}", uuid::Uuid::parse_str(&value).is_ok());
                JsValue::TRUE
            }),
        )
        .unwrap();
        db.execute("(transact [[:a :friend :b] [:b :name \"b\"]])".into())
            .await
            .unwrap();
        rows(&db, "(query [:find ?e :where [?e :name _] [(ref? ?e)]])").await;
        assert_eq!(*kind.borrow(), "ref true");
    }

    #[wasm_bindgen_test]
    async fn an_aggregate_in_find_and_over() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let product = js_sys::Object::new();
        Reflect::set(&product, &"init".into(), &js_fn("", "return 1n")).unwrap();
        Reflect::set(
            &product,
            &"step".into(),
            &js_fn("acc, v", "return acc * v.value"),
        )
        .unwrap();
        Reflect::set(
            &product,
            &"finish".into(),
            &js_fn(
                "acc, n",
                "return { type: 'integer', value: acc * BigInt(n) }",
            ),
        )
        .unwrap();
        db.register_aggregate("product", product.into())
            .expect("register");

        let got = rows(
            &db,
            "(query [:find ?g (product ?n) :where [?e :n ?n] [?e :g ?g]])",
        )
        .await;
        let mut got: Vec<(String, i64)> = got
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_i64().unwrap()))
            .collect();
        got.sort();
        // x: 1 * 2, times 2 values; y: 3, times 1 value.
        assert_eq!(got, vec![(":x".to_string(), 4), (":y".to_string(), 3)]);

        let got = rows(
            &db,
            "(query [:find ?n (product ?n :over (:order-by ?n)) :where [?e :n ?n]])",
        )
        .await;
        let mut got: Vec<(i64, i64)> = got
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_i64().unwrap(), r[1].as_i64().unwrap()))
            .collect();
        got.sort_unstable();
        // Running product times running count: 1*1, 2*2, 6*3.
        assert_eq!(got, vec![(1, 1), (2, 4), (3, 18)]);
    }

    #[wasm_bindgen_test]
    async fn a_plain_finish_result_is_read_as_a_bind_value() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let agg = js_sys::Object::new();
        Reflect::set(&agg, &"init".into(), &js_fn("", "return 0")).unwrap();
        Reflect::set(
            &agg,
            &"step".into(),
            &js_fn("acc, v", "return acc + Number(v.value)"),
        )
        .unwrap();
        Reflect::set(&agg, &"finish".into(), &js_fn("acc, n", "return acc / n")).unwrap();
        db.register_aggregate("mean", agg.into()).unwrap();
        let got = rows(&db, "(query [:find (mean ?n) :where [?e :n ?n]])").await;
        assert_eq!(got, serde_json::json!([[2]]));
    }

    #[wasm_bindgen_test]
    async fn a_throw_fails_the_query_with_qry_014() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        db.register_predicate("boom?", js_fn("v", "throw new Error('bad ' + v.value)"))
            .unwrap();
        db.register_predicate("str?", js_fn("v", "throw 'plain'"))
            .unwrap();
        db.register_predicate("num?", js_fn("v", "return 1"))
            .unwrap();
        let q = |p: &str| format!("(query [:find ?n :where [:a :n ?n] [({p} ?n)]])");
        assert_eq!(
            rejection(&db, &q("boom?")).await,
            "[QRY-014] user function `boom?` failed: bad 1"
        );
        assert_eq!(
            rejection(&db, &q("str?")).await,
            "[QRY-014] user function `str?` failed: plain"
        );
        assert_eq!(
            rejection(&db, &q("num?")).await,
            "[QRY-014] user function `num?` failed: returned number, not a boolean"
        );

        // Through a cursor, a prepared query and a prepared cursor too.
        let mut cursor = db.query(&q("boom?")).expect("cursor opens");
        let err = cursor
            .next_batch(10)
            .err()
            .and_then(|e| e.as_string())
            .unwrap();
        assert!(err.starts_with("[QRY-014]"), "cursor error has QRY-014");
        let pq = db.prepare(&q("boom?")).unwrap();
        let err = pq
            .execute(JsValue::UNDEFINED)
            .err()
            .and_then(|e| e.as_string())
            .unwrap();
        assert!(err.starts_with("[QRY-014]"), "prepared error has QRY-014");
    }

    #[wasm_bindgen_test]
    async fn aggregate_failures_fail_the_query() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let ok_init = "return 0";
        let ok_step = "return acc";
        let ok_finish = "return 0";
        let bad = "throw new TypeError('no')";
        for (name, init, step, finish, message) in [
            ("bad-init", bad, ok_step, ok_finish, "no"),
            ("bad-step", ok_init, bad, ok_finish, "no"),
            ("bad-finish", ok_init, ok_step, bad, "no"),
            (
                "bad-ref",
                ok_init,
                ok_step,
                "return { type: 'ref', value: 'nope' }",
                r#"invalid argument: finish's result of type "ref": "nope" is not a UUID"#,
            ),
            (
                "bad-entity",
                ok_init,
                ok_step,
                "return { type: 'entity', value: '00000000-0000-0000-0000-000000000000' }",
                "invalid argument: finish's result is not a value",
            ),
        ] {
            let agg = js_sys::Object::new();
            Reflect::set(&agg, &"init".into(), &js_fn("", init)).unwrap();
            Reflect::set(&agg, &"step".into(), &js_fn("acc, v", step)).unwrap();
            Reflect::set(&agg, &"finish".into(), &js_fn("acc, n", finish)).unwrap();
            db.register_aggregate(name, agg.into()).unwrap();
            for q in [
                format!("(query [:find ({name} ?n) :where [?e :n ?n]])"),
                format!("(query [:find ?n ({name} ?n :over (:order-by ?n)) :where [?e :n ?n]])"),
            ] {
                assert_eq!(
                    rejection(&db, &q).await,
                    format!("[QRY-014] user function `{name}` failed: {message}")
                );
            }
        }
    }

    #[wasm_bindgen_test]
    fn registration_errors() {
        let db = BrowserDb::open_in_memory().unwrap();
        db.register_predicate("p?", js_fn("v", "return true"))
            .unwrap();
        let held = || HELD.with(|h| h.borrow().len());
        let before = held();
        let err = db.register_predicate("p?", JsValue::from(1)).unwrap_err();
        assert!(err.is_instance_of::<js_sys::TypeError>(), "not a function");
        let partial = js_sys::Object::new();
        Reflect::set(&partial, &"init".into(), &js_fn("", "return 0")).unwrap();
        let err = db.register_aggregate("a", partial.into()).unwrap_err();
        assert!(err.is_instance_of::<js_sys::TypeError>(), "step missing");
        let err = db.register_aggregate("a", JsValue::NULL).unwrap_err();
        assert!(err.is_instance_of::<js_sys::TypeError>(), "not an object");

        let err = db
            .register_predicate("p?", js_fn("v", "return true"))
            .unwrap_err()
            .as_string()
            .unwrap();
        assert!(err.starts_with("[INT-040]"), "duplicate name");
        let err = db
            .register_predicate("string?", js_fn("v", "return true"))
            .unwrap_err()
            .as_string()
            .unwrap();
        assert!(err.starts_with("[INT-040]"), "built-in name");
        // A failed registration keeps none of the functions it was given.
        assert_eq!(held(), before, "no entry left behind");
    }

    #[wasm_bindgen_test]
    async fn a_function_may_query_the_same_database() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let inner_db = clone_db(&db);
        // A nested query that runs a UDF too: true only for ?n = 1.
        db.register_predicate("one?", js_fn("v", "return v.value === 1n"))
            .unwrap();
        db.register_predicate(
            "nested?",
            rust_fn(move |v| {
                let n = Reflect::get(&v, &"value".into()).unwrap();
                let q = format!(
                    "(query [:find ?g :where [?e :n {n}] [?e :g ?g] [?e :n ?m] [(one? ?m)]])",
                    n = js_string(&n).unwrap()
                );
                let mut cursor = inner_db.query(&q).expect("nested query");
                let found = cursor.next_batch(10).expect("nested batch").is_some();
                JsValue::from_bool(found)
            }),
        )
        .unwrap();
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n] [(nested? ?n)]])").await;
        assert_eq!(got, serde_json::json!([[1]]));
    }

    #[wasm_bindgen_test]
    async fn a_function_may_register_a_function() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let inner_db = clone_db(&db);
        let registered = Rc::new(RefCell::new(false));
        let flag = registered.clone();
        db.register_predicate(
            "reg?",
            rust_fn(move |_| {
                if !*flag.borrow() {
                    inner_db
                        .register_predicate("later?", js_fn("v", "return true"))
                        .expect("register from a UDF");
                    *flag.borrow_mut() = true;
                }
                JsValue::TRUE
            }),
        )
        .unwrap();
        // The outer query opened before `later?` existed.
        let err = rejection(
            &db,
            "(query [:find ?n :where [?e :n ?n] [(reg? ?n)] [(later? ?n)]])",
        )
        .await;
        assert!(err.starts_with("[QRY-007]"), "unknown at open");
        rows(&db, "(query [:find ?n :where [?e :n ?n] [(reg? ?n)]])").await;
        assert!(*registered.borrow());
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n] [(later? ?n)]])").await;
        assert_eq!(
            got.as_array().unwrap().len(),
            3,
            "visible to the next query"
        );
    }

    /// `execute` and `checkpoint` started from a UDF run after
    /// the outer query, as they do when JavaScript calls them there: their
    /// promise's body starts from a microtask.
    #[wasm_bindgen_test]
    async fn async_methods_from_a_function_run_after_the_query() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let promises = Rc::new(RefCell::new(Vec::<js_sys::Promise>::new()));
        let started = promises.clone();
        let inner_db = clone_db(&db);
        db.register_predicate(
            "spawn?",
            rust_fn(move |v| {
                if Reflect::get(&v, &"value".into()).unwrap() == JsValue::from(1i64) {
                    let (a, b) = (clone_db(&inner_db), clone_db(&inner_db));
                    let mut started = started.borrow_mut();
                    started.push(wasm_bindgen_futures::future_to_promise(async move {
                        a.execute("(transact [[:d :n 4]])".into())
                            .await
                            .map(JsValue::from)
                    }));
                    started.push(wasm_bindgen_futures::future_to_promise(async move {
                        b.checkpoint().await.map(|()| JsValue::UNDEFINED)
                    }));
                }
                JsValue::TRUE
            }),
        )
        .unwrap();
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n] [(spawn? ?n)]])").await;
        assert_eq!(got.as_array().unwrap().len(), 3, "the outer query's answer");
        let started: Vec<_> = promises.borrow_mut().drain(..).collect();
        assert_eq!(started.len(), 2);
        for p in started {
            wasm_bindgen_futures::JsFuture::from(p)
                .await
                .expect("resolves");
        }
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n]])").await;
        assert_eq!(got.as_array().unwrap().len(), 4, "the write landed");
    }

    #[wasm_bindgen_test]
    async fn import_graph_from_a_function_runs_after_the_query() {
        let db = BrowserDb::open_in_memory().unwrap();
        numbers(&db).await;
        let other = BrowserDb::open_in_memory().unwrap();
        other
            .execute("(transact [[:z :n 9]])".into())
            .await
            .unwrap();
        let blob = other.export_graph().unwrap();
        let promise = Rc::new(RefCell::new(None::<js_sys::Promise>));
        let slot = promise.clone();
        let inner_db = clone_db(&db);
        db.register_predicate(
            "swap?",
            rust_fn(move |_| {
                if slot.borrow().is_none() {
                    let d = clone_db(&inner_db);
                    let blob = blob.clone();
                    *slot.borrow_mut() =
                        Some(wasm_bindgen_futures::future_to_promise(async move {
                            d.import_graph(blob).await.map(|()| JsValue::UNDEFINED)
                        }));
                }
                JsValue::TRUE
            }),
        )
        .unwrap();
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n] [(swap? ?n)]])").await;
        assert_eq!(got.as_array().unwrap().len(), 3, "the outer query's answer");
        let p = promise.borrow_mut().take().expect("started");
        wasm_bindgen_futures::JsFuture::from(p)
            .await
            .expect("resolves");
        let got = rows(&db, "(query [:find ?n :where [?e :n ?n]])").await;
        assert_eq!(got, serde_json::json!([[9]]), "the import landed");
    }

    /// Another thread has its own table, where the id is missing; this
    /// thread's table without the id stands in for it.
    #[wasm_bindgen_test]
    fn a_value_missing_from_the_table_fails_the_call() {
        let held = Held::new(js_fn("", "return true"));
        HELD.with(|h| h.borrow_mut().remove(&held.0));
        assert_eq!(
            held.function()
                .err()
                .map(|e| e.message().to_string())
                .as_deref(),
            Some("called off the JavaScript thread")
        );
    }

    #[wasm_bindgen_test]
    fn dropping_a_held_value_frees_its_entry() {
        let held = Held::new(JsValue::TRUE);
        let id = held.0;
        assert!(HELD.with(|h| h.borrow().contains_key(&id)));
        drop(held);
        assert!(!HELD.with(|h| h.borrow().contains_key(&id)));
    }

    #[wasm_bindgen_test]
    fn strip_code_drops_only_a_leading_code() {
        assert_eq!(strip_code("[API-017] bad"), "bad");
        assert_eq!(strip_code("[api-017] bad"), "[api-017] bad");
        assert_eq!(strip_code("bad [API-017] x"), "bad [API-017] x");
        assert_eq!(strip_code("[API-017]"), "[API-017]");
    }
}
