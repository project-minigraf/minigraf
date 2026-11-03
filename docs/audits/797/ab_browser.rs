//! BrowserDb runtime harness of the #797 reader A/B (measurement only).
//!
//! `run.py` copies this file to `examples/ab_browser.rs` in every variant,
//! builds it once (`cargo test --release --target wasm32-unknown-unknown
//! --features browser --example ab_browser --no-run`: the release profile
//! without LTO, which cannot link this harness; an example, because a test
//! target also links the `minigraf` binary, whose `.wasm` collides with the
//! library's cdylib, rust-lang/cargo#6313) and runs the module in headless
//! Firefox with `wasm-bindgen-test-runner`,
//! with `webdriver.json` beside it (full-precision `performance.now()`). One
//! run prints one JSON line per workload, prefixed `AB `.
//!
//! The database is `BrowserDb.openInMemory`, its facts checkpointed so that
//! queries read the committed `BrowserBufferBackend` reader (page cache
//! capacity 0: RAM reads). IndexedDB is not used: no open or flush is timed.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

/// Nothing to do natively; the harness is a wasm-bindgen test.
fn main() {}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
mod ab {
    use minigraf::browser::BrowserDb;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    const ENTITIES: usize = 5_000;
    const SECS: f64 = 1.5;

    struct Clock {
        perf: JsValue,
        now: js_sys::Function,
    }

    impl Clock {
        fn new() -> Self {
            let perf = js_sys::Reflect::get(&js_sys::global(), &"performance".into()).unwrap();
            let now = js_sys::Reflect::get(&perf, &"now".into()).unwrap().into();
            Clock { perf, now }
        }
        /// Milliseconds.
        fn now(&self) -> f64 {
            self.now.call0(&self.perf).unwrap().as_f64().unwrap()
        }
    }

    fn transactions() -> Vec<String> {
        let mut out = Vec::new();
        for lo in (0..ENTITIES).step_by(200) {
            let hi = (lo + 200).min(ENTITIES);
            let mut cmd = String::from("(transact [");
            for i in lo..hi {
                let follows = (i * 7919 + 1) % ENTITIES;
                cmd.push_str(&format!(
                    "[:p{i} :name \"p{i}\"][:p{i} :age {}][:p{i} :group {}][:p{i} :follows :p{follows}]",
                    i % 90,
                    i % 100
                ));
                if i > 0 {
                    cmd.push_str(&format!("[:p{i} :manager :p{}]", i / 10));
                }
            }
            cmd.push_str("])");
            out.push(cmd);
        }
        out
    }

    fn varied_keys(range: usize, n: usize) -> Vec<usize> {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                ((x >> 33) as usize) % range
            })
            .collect()
    }

    /// Rows of a cursor, as the JSON batches it returns.
    fn drain(db: &BrowserDb, q: &str, batch: u32, h: Option<&mut DefaultHasher>) -> usize {
        let mut c = db.query(q).unwrap();
        let mut n = 0;
        let mut h = h;
        while let Some(json) = c.next_batch(batch).unwrap() {
            let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
            n += rows.as_array().map_or(0, Vec::len);
            if let Some(h) = h.as_deref_mut() {
                json.hash(h);
            }
        }
        n
    }

    fn report(name: &str, samples: &mut [f64], digest: u64, rows: usize) {
        let ops = samples.len();
        let total: f64 = samples.iter().sum();
        samples.sort_by(f64::total_cmp);
        let at = |p: f64| samples[((p * ops as f64).ceil() as usize).clamp(1, ops) - 1];
        let min_step = samples
            .iter()
            .copied()
            .filter(|&s| s > 0.0)
            .fold(f64::INFINITY, f64::min);
        console_log!(
            "AB {{\"name\":\"{name}\",\"ops\":{ops},\"total_ns\":{:.0},\"mean_ns\":{:.1},\"p50_ns\":{:.0},\
             \"p99_ns\":{:.0},\"allocs_per_op\":0,\"digest\":\"{digest:016x}\",\"rows\":{rows},\"rss_kb\":0,\
             \"min_nonzero_sample_ns\":{:.0}}}",
            total * 1e6,
            total * 1e6 / ops as f64,
            at(0.50) * 1e6,
            at(0.99) * 1e6,
            min_step * 1e6
        );
    }

    /// Time `op` (returns its row count, checked against `expect`) for `SECS`.
    fn timed(clock: &Clock, name: &str, digest: u64, op: impl Fn(usize) -> usize, expect: impl Fn(usize) -> usize) {
        let start = clock.now();
        let mut i = 0;
        while i < 3 || clock.now() - start < 300.0 {
            assert_eq!(op(i), expect(i), "warm-up rows");
            i += 1;
        }
        let mut samples = Vec::new();
        let mut rows = 0;
        let start = clock.now();
        let mut i = 0;
        while samples.len() < 5 || clock.now() - start < SECS * 1000.0 {
            let t = clock.now();
            let r = op(i);
            samples.push(clock.now() - t);
            assert_eq!(r, expect(i), "rows changed");
            rows += r;
            i += 1;
        }
        report(name, &mut samples, digest, rows);
    }

    #[wasm_bindgen_test]
    async fn reader_ab() {
        let clock = Clock::new();
        let db = BrowserDb::open_in_memory().unwrap();
        for t in transactions() {
            db.execute(t).await.unwrap();
        }
        db.checkpoint().await.unwrap();
        let m = ENTITIES / 2;
        let keys = varied_keys(ENTITIES, 256);
        // Point families: fixed, varied and missing keys through a cursor
        // pulled in batches of one.
        let families: [(&str, Box<dyn Fn(usize) -> String>); 2] = [
            (
                "br_pt_ea",
                Box::new(move |i| {
                    let j = (i / 3) % 256;
                    match i % 3 {
                        0 => format!("(query [:find ?a :where [:p{m} :age ?a]])"),
                        1 => format!("(query [:find ?a :where [:p{} :age ?a]])", keys[j]),
                        _ => format!("(query [:find ?a :where [:missing{j} :age ?a]])"),
                    }
                }),
            ),
            (
                "br_pt_av",
                Box::new(move |i| {
                    let j = (i / 3) % 256;
                    match i % 3 {
                        0 => format!("(query [:find ?e :where [?e :name \"p{m}\"]])"),
                        1 => format!("(query [:find ?e :where [?e :name \"p{}\"]])", varied_keys(ENTITIES, 256)[j]),
                        _ => format!("(query [:find ?e :where [?e :name \"nope{j}\"]])"),
                    }
                }),
            ),
        ];
        for (name, text) in &families {
            let texts: Vec<String> = (0..3 * 256).map(text).collect();
            let mut h = DefaultHasher::new();
            let expect: Vec<usize> = texts.iter().map(|q| drain(&db, q, 1, Some(&mut h))).collect();
            timed(
                &clock,
                name,
                h.finish(),
                |i| drain(&db, &texts[i % texts.len()], 1, None),
                |i| expect[i % texts.len()],
            );
        }
        for (name, q) in [
            ("br_scan", "(query [:find ?e ?a :where [?e :age ?a]])".to_string()),
            (
                "br_join_2",
                format!("(query [:find ?n :where [:p{m} :follows ?f] [?f :name ?n]])"),
            ),
        ] {
            let mut h = DefaultHasher::new();
            let n = drain(&db, &q, 1_000, Some(&mut h));
            timed(&clock, name, h.finish(), |_| drain(&db, &q, 1_000, None), |_| n);
        }
    }
}
