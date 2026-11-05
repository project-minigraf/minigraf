//! Included only by instrument.py in the disposable audit build.
use super::*;
use crate::{Cursor, FactFilter, FactLog, FactOrder, FactRecord, LogWriter, Value};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;
use uuid::Uuid;

fn id(i: usize) -> Uuid {
    Uuid::from_u128(0x84100000000000000000000000000000 + i as u128)
}
fn rss() -> Json {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let field = |prefix: &str| {
        s.lines()
            .find(|l| l.starts_with(prefix))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    json!({"current_kib":field("VmRSS:"),"peak_kib":field("VmHWM:")})
}
fn emit(value: Json) {
    println!("SHAPE {value}");
}

fn meta(db: &Minigraf) -> crate::storage::meta::MetaPage {
    match &*db.inner.write_lock.lock().unwrap() {
        WriteContext::File { pfs, .. } | WriteContext::ReadOnly { pfs } => pfs.meta(),
        _ => panic!("file fixture required"),
    }
}
fn page_generation(file: &mut std::fs::File, page: u64) -> u64 {
    file.seek(SeekFrom::Start(page * 4096 + 16)).unwrap();
    let mut b = [0u8; 8];
    file.read_exact(&mut b).unwrap();
    u64::from_le_bytes(b)
}
const QUERY: &str = "(query [:find ?e ?v :where [?e :pin/value ?v]])";

struct HeldCursor {
    cursor: Cursor,
    expected: Vec<usize>,
}
struct HeldLog {
    log: FactLog,
    expected: Vec<FactRecord>,
}

fn check_cursor(mut h: HeldCursor) {
    let mut seen = vec![false; h.expected.len()];
    while let Some(batch) = h.cursor.next_batch(256).unwrap() {
        for row in batch.rows() {
            let Value::Ref(e) = row[0] else {
                panic!("entity expected")
            };
            let i = (e.as_u128() - id(0).as_u128()) as usize;
            assert!(!seen[i], "duplicate snapshot row");
            seen[i] = true;
            assert!(
                row[1] == Value::Integer(h.expected[i] as i64),
                "snapshot value changed"
            );
        }
    }
    assert!(seen.into_iter().all(|v| v), "missing snapshot row");
}

fn record_key(r: &FactRecord) -> (Uuid, u64, bool, i64) {
    let Value::Integer(v) = r.value else {
        panic!("integer log expected")
    };
    (r.entity, r.tx_count, r.asserted, v)
}
fn check_log(h: HeldLog) {
    let mut actual: Vec<_> = h.log.map(|r| r.unwrap()).collect();
    let mut expected = h.expected;
    actual.sort_unstable_by_key(record_key);
    expected.sort_unstable_by_key(record_key);
    assert!(actual == expected, "held fact log changed");
}

fn churn(
    n: usize,
    held_rounds: usize,
    baseline: usize,
    after: usize,
    mode: &str,
    path: &std::path::Path,
) {
    let mut writer = LogWriter::create(path, OpenOptions::new()).unwrap();
    for i in 0..n {
        writer
            .append(&FactRecord {
                entity: id(i),
                attribute: ":pin/value".into(),
                value: Value::Integer(i as i64),
                tx_count: (i / 1000 + 1) as u64,
                tx_id: 1_700_000_000_000 + (i / 1000) as u64,
                valid_from: 0,
                valid_to: i64::MAX,
                asserted: true,
            })
            .unwrap();
    }
    writer.finish().unwrap();
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()
        .unwrap();
    let mut values: Vec<usize> = (0..n).collect();
    let mut cursors = Vec::new();
    let mut logs = Vec::new();
    let mut protected = BTreeMap::new();
    let mut released = HashSet::new();
    let mut reused = HashSet::new();
    let mut release_gen = 0;
    let mut file = std::fs::File::open(path).unwrap();
    let open_log = |db: &Minigraf| {
        let filter = FactFilter::new().order(if mode == "log_storage" {
            FactOrder::Storage
        } else {
            FactOrder::Tx
        });
        let filter = if mode == "log_4096" {
            filter.window(4096)
        } else {
            filter
        };
        let expected = db.fact_log(&filter).unwrap().map(|r| r.unwrap()).collect();
        HeldLog {
            log: db.fact_log(&filter).unwrap(),
            expected,
        }
    };
    for round in 0..baseline + held_rounds + after {
        let holding = round >= baseline && round < baseline + held_rounds;
        if round == baseline {
            match mode {
                "cursor" | "mixed" => cursors.push(HeldCursor {
                    cursor: db.query(QUERY).unwrap(),
                    expected: values.clone(),
                }),
                _ => (),
            }
            if mode.starts_with("log_") || mode == "mixed" {
                logs.push(open_log(&db));
            }
        }
        if holding && ((mode == "many" && cursors.len() < 32) || mode == "rolling") {
            cursors.push(HeldCursor {
                cursor: db.query(QUERY).unwrap(),
                expected: values.clone(),
            });
            if cursors.len() > 32 {
                check_cursor(cursors.remove(0));
            }
        }
        if round == baseline + held_rounds {
            released = db.inner.fact_storage.pages_in_force();
            release_gen = meta(&db).generation;
            for c in cursors.drain(..) {
                check_cursor(c);
            }
            for l in logs.drain(..) {
                check_log(l);
            }
            assert!(
                db.inner.fact_storage.pages_in_force().is_empty(),
                "quarantine survived last pin drop"
            );
            assert_eq!(
                db.inner.fact_storage.sweep_pin_stats().0,
                0,
                "pin survived last drop"
            );
            emit(
                json!({"kind":"release","mode":mode,"n":n,"generation":release_gen,
                "released_pages":released.len(),"quarantine_pages":0,"rss":rss(),"verified":true}),
            );
        }
        let before_q = db.inner.fact_storage.pages_in_force();
        for &page in &before_q {
            protected
                .entry(page)
                .or_insert_with(|| page_generation(&mut file, page));
        }
        let mut retract = String::from("(retract [");
        let mut transact = String::from("(transact [");
        for j in 0..100.min(n) {
            let i = (round * 100 + j) % n;
            retract.push_str(&format!("[#uuid \"{}\" :pin/value {}]", id(i), values[i]));
            values[i] += n;
            transact.push_str(&format!("[#uuid \"{}\" :pin/value {}]", id(i), values[i]));
        }
        retract.push_str("])");
        transact.push_str("])");
        db.execute(&retract).unwrap();
        db.execute(&transact).unwrap();
        let start = Instant::now();
        db.checkpoint().unwrap();
        let checkpoint_ns = start.elapsed().as_nanos();
        let m = meta(&db);
        // Every page protected BEFORE this checkpoint must keep its generation.
        for page in before_q {
            assert_eq!(
                page_generation(&mut file, page),
                protected[&page],
                "checkpoint rewrote quarantined page"
            );
        }
        let q = db.inner.fact_storage.pages_in_force();
        protected.retain(|p, _| q.contains(p));
        if round >= baseline + held_rounds {
            for &page in &released {
                if !reused.contains(&page) && page_generation(&mut file, page) > release_gen {
                    reused.insert(page);
                }
            }
        }
        let (pin_count, oldest, capacity) = db.inner.fact_storage.sweep_pin_stats();
        check_cursor(HeldCursor {
            cursor: db.query(QUERY).unwrap(),
            expected: values.clone(),
        });
        emit(json!({"kind":"generation","mode":mode,"n":n,"round":round,
            "phase":if round<baseline {"baseline"} else if holding {"held"} else {"after_release"},
            "generation":m.generation,"checkpoint_ns":checkpoint_ns,"file_bytes":std::fs::metadata(path).unwrap().len(),
            "page_count":m.page_count,"history_records":m.fact_count,"freelist_count_including_quarantine":m.freelist_count,
            "quarantine_pages":q.len(),"quarantine_vector_capacity_bytes":capacity,
            "pins":pin_count,"oldest_pin":oldest,"reused_released_pages":reused.len(),"rss":rss(),"verified":true}));
    }
    if !released.is_empty() {
        assert!(
            !reused.is_empty(),
            "no quarantined page reused after release"
        );
    }
    assert!(db.verify().unwrap().is_ok(), "integrity failed after churn");
    drop(db);
    let db = Minigraf::open(path).unwrap();
    check_cursor(HeldCursor {
        cursor: db.query(QUERY).unwrap(),
        expected: values,
    });
    assert!(
        db.verify().unwrap().is_ok(),
        "integrity failed after reopen"
    );
    emit(json!({"kind":"completed","mode":mode,"n":n,"verified":true,"reused_pages":reused.len()}));
}

#[test]
fn smoke_churn_oracles() {
    for mode in [
        "none",
        "cursor",
        "many",
        "rolling",
        "log_storage",
        "log_default",
        "log_4096",
        "mixed",
    ] {
        let dir = tempfile::tempdir().unwrap();
        churn(200, 4, 2, 4, mode, &dir.path().join("test.graph"));
    }
}

#[test]
#[ignore = "manual X06; isolated process on idle benchmark host"]
fn run_churn() {
    let n = std::env::var("SWEEP_N").unwrap().parse().unwrap();
    let mode = std::env::var("SWEEP_MODE").unwrap();
    let path = std::env::var("SWEEP_PATH").unwrap();
    churn(n, 256, 32, 256, &mode, std::path::Path::new(&path));
}

#[test]
#[ignore = "manual X06; isolated process on idle benchmark host"]
fn query_memory() {
    let path = std::env::var("SWEEP_PATH").unwrap();
    let query = std::env::var("SWEEP_QUERY").unwrap();
    let cache = std::env::var("SWEEP_CACHE").unwrap().parse().unwrap();
    let db = OpenOptions::new()
        .page_cache_size(cache)
        .wal_checkpoint_threshold(usize::MAX)
        .path(&path)
        .open()
        .unwrap();
    if std::env::var("SWEEP_RECURSION").as_deref() == Ok("yes") {
        db.execute("(rule [(anc ?x ?a) [?x :s/parent ?a]])")
            .unwrap();
        db.execute("(rule [(anc ?x ?a) [?x :s/parent ?p] (anc ?p ?a)])")
            .unwrap();
    }
    let mut cursor = db.query(&query).unwrap();
    let meter = cursor
        .stream_mut()
        .expect("streaming query")
        .meter()
        .clone();
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(256).unwrap() {
        rows += batch.len();
    }
    drop(cursor);
    emit(
        json!({"kind":"memory","rows":rows,"meter_peak":meter.peak(),"idspace_peak":meter.peak_for("IdSpace"),
        "aggregate_peak":meter.peak_for("Aggregate"),"window_peak":meter.peak_for("Window"),
        "meter_after_drop":meter.used(),"rss":rss()}),
    );
    assert_eq!(meter.used(), 0, "query retained charged memory after drop");
    crate::query::engine::SWEEP_EVENTS
        .with(|e| emit(json!({"kind":"events","counts":*e.borrow()})));
}
