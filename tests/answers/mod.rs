//! Frozen answers (#551). Until #551 every query the engine ran in these
//! tests was also run by the executor, and a different answer failed the
//! test. With the executor gone, each answer is compared with those
//! recorded while that check still ran, in `tests/answers/<suite>.txt`: one
//! line per test, query and answer (row count and a digest of the rows as a
//! bag). A query a test runs over several states has one line per answer,
//! and its answer must be one of them. Integers within a week of now (wall
//! clock `tx-id`s, default `valid-from`s, a query's own `valid-at`) are
//! compared as "now".
//!
//! `MINIGRAF_BLESS_ANSWERS=1` appends instead of comparing; then
//! `LC_ALL=C sort -u -o <file> <file>`. A changed file is a changed answer,
//! to be justified in review.
#![allow(dead_code)]

use minigraf::Value;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// A value as text, without `Debug` (CodeQL `rust/cleartext-logging`).
fn key(v: &Value) -> String {
    match v {
        Value::String(s) => format!("s:{s}"),
        Value::Integer(i) if now().abs_diff(*i) < WEEK_MS => "i:now".to_string(),
        Value::Integer(i) => format!("i:{i}"),
        Value::Float(f) => format!("f:{:016x}", f.to_bits()),
        Value::Boolean(b) => format!("b:{b}"),
        Value::Ref(u) => format!("r:{}", u.as_hyphenated()),
        Value::Keyword(k) => format!("k:{k}"),
        Value::Null => "nil".to_string(),
    }
}

const WEEK_MS: u64 = 7 * 86_400_000;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn digest(rows: &[Vec<Value>]) -> u64 {
    let mut bag: Vec<String> = rows
        .iter()
        .map(|r| r.iter().map(key).collect::<Vec<_>>().join("\u{1f}"))
        .collect();
    bag.sort();
    fnv(bag.join("\u{1e}").as_bytes())
}

fn path(suite: &str) -> String {
    format!("{}/tests/answers/{suite}.txt", env!("CARGO_MANIFEST_DIR"))
}

fn bless() -> bool {
    std::env::var_os("MINIGRAF_BLESS_ANSWERS").is_some_and(|v| v == "1")
}

/// The test running on this thread, without its module path.
fn test_name() -> String {
    let name = std::thread::current()
        .name()
        .filter(|n| *n != "main")
        .expect("frozen answers need the test's own thread")
        .to_string();
    name.rsplit("::").next().unwrap_or(&name).to_string()
}

/// Each test and query digest's recorded answers.
type Frozen = HashMap<(String, u64), Vec<(usize, u64)>>;

fn frozen(suite: &str) -> &'static Mutex<Frozen> {
    static FILES: OnceLock<Mutex<Frozen>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut map: Frozen = HashMap::new();
        if !bless() {
            let text = std::fs::read_to_string(path(suite)).expect("frozen answers file");
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                let hex = |s: &str| u64::from_str_radix(s, 16).expect("hex digest");
                map.entry((f[0].to_string(), hex(f[1])))
                    .or_default()
                    .push((f[2].parse().expect("row count"), hex(f[3])));
            }
        }
        Mutex::new(map)
    })
}

/// Compare `rows`, the answer to `query`, with the frozen ones (or record
/// it). One suite per test binary.
pub fn check(suite: &str, query: &str, rows: &[Vec<Value>]) {
    let test = test_name();
    let q = fnv(query.as_bytes());
    let (count, digest) = (rows.len(), digest(rows));
    let map = frozen(suite);
    if bless() {
        let _held = map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path(suite))
            .unwrap();
        file.write_all(format!("{test}\t{q:016x}\t{count}\t{digest:016x}\n").as_bytes())
            .unwrap();
        return;
    }
    let map = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(answers) = map.get(&(test.clone(), q)) else {
        panic!("{test}: no frozen answer to {query}");
    };
    assert!(
        answers.contains(&(count, digest)),
        "{test}: {count} rows, not a frozen answer to {query}"
    );
}
