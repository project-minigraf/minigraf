//! `Minigraf::fact_log` streams every fact record with its transaction and
//! valid-time bounds (#430). These tests check the records against a model of
//! the writes, in every split between checkpointed and pending facts, both
//! orders and small windows, the filters, the snapshot, and the checkpoint pin.

use minigraf::{FactFilter, FactLog, FactOrder, FactRecord, Minigraf, OpenOptions, QueryResult};
use std::path::Path;
use uuid::Uuid;

const NAMES: [&str; 4] = ["e1", "e2", "e3", "e4"];

fn uuid(i: usize) -> Uuid {
    Uuid::from_u128(0x1000 + i as u128)
}

fn name_of(u: &Uuid) -> &'static str {
    (0..NAMES.len())
        .find(|&i| uuid(i) == *u)
        .map_or("?", |i| NAMES[i])
}

const LONG: &str = "a long string value that is stored in a value page, not in the key: \
                    0123456789abcdefghijklmnopqrstuvwxyz";
const SALARY_FROM: i64 = 1_577_836_800_000; // 2020-01-01
const SALARY_TO: i64 = 1_640_995_200_000; // 2022-01-01
const BULK: usize = 20;

/// One record as the model knows it: `(tx_count, entity, attribute, value, asserted, tx_id)`.
type Row = (u64, String, String, String, bool, u64);

fn row(r: &FactRecord) -> Row {
    (
        r.tx_count,
        name_of(&r.entity).to_string(),
        r.attribute.clone(),
        format!("{:?}", r.value),
        r.asserted,
        r.tx_id,
    )
}

fn tx_id_of(result: QueryResult) -> u64 {
    match result {
        QueryResult::Transacted(t) | QueryResult::Retracted(t) => t,
        _ => panic!("expected a write result"),
    }
}

/// The writes, one transaction per step: `(command, records)` where each record
/// is `(entity, attribute, value as Debug, asserted)`.
fn steps() -> Vec<(String, Vec<(usize, &'static str, String, bool)>)> {
    let e = |i: usize| format!("#uuid \"{}\"", uuid(i));
    let s = |v: &str| format!("String({v:?})");
    let mut steps = vec![
        (
            format!(
                r#"(transact [[{} :person/name "Alice"] [{} :person/age 30]
                              [{} :person/name "Bob"] [{} :ingestion/src "x"]])"#,
                e(0),
                e(0),
                e(1),
                e(1)
            ),
            vec![
                (0, ":person/name", s("Alice"), true),
                (0, ":person/age", "Integer(30)".to_string(), true),
                (1, ":person/name", s("Bob"), true),
                (1, ":ingestion/src", s("x"), true),
            ],
        ),
        (
            format!(
                r#"(transact [[{} :person/age 31] [{} :ingestion/batch 7]
                              [{} :person/active true] [{} :person/role :role/admin]
                              [{} :person/score 2.5]])"#,
                e(0),
                e(2),
                e(0),
                e(1),
                e(1)
            ),
            vec![
                (0, ":person/age", "Integer(31)".to_string(), true),
                (2, ":ingestion/batch", "Integer(7)".to_string(), true),
                (0, ":person/active", "Boolean(true)".to_string(), true),
                (
                    1,
                    ":person/role",
                    "Keyword(\":role/admin\")".to_string(),
                    true,
                ),
                (1, ":person/score", "Float(2.5)".to_string(), true),
            ],
        ),
        (
            format!("(retract [[{} :person/age 30]])", e(0)),
            vec![(0, ":person/age", "Integer(30)".to_string(), false)],
        ),
        (
            format!("(transact [[{} :doc/body \"{LONG}\"]])", e(2)),
            vec![(2, ":doc/body", s(LONG), true)],
        ),
        (
            format!(
                r#"(transact {{:valid-from "2020-01-01" :valid-to "2022-01-01"}} [[{} :person/salary 100]])"#,
                e(1)
            ),
            vec![(1, ":person/salary", "Integer(100)".to_string(), true)],
        ),
    ];
    let bulk: String = (0..BULK)
        .map(|i| format!("[{} :bulk/n {i}]", e(3)))
        .collect();
    steps.push((
        format!("(transact [{bulk}])"),
        (0..BULK)
            .map(|i| (3, ":bulk/n", format!("Integer({i})"), true))
            .collect(),
    ));
    steps.push((
        format!(r#"(retract [[{} :ingestion/src "x"]])"#, e(1)),
        vec![(1, ":ingestion/src", s("x"), false)],
    ));
    steps
}

/// Run the steps from `from` on, checkpointing after step `checkpoint_after`
/// (1-based), and return the model rows.
fn write(db: &Minigraf, checkpoint_after: Option<usize>) -> Vec<Row> {
    let mut model = Vec::new();
    for (i, (cmd, records)) in steps().into_iter().enumerate() {
        let tx_id = tx_id_of(db.execute(&cmd).unwrap());
        let tx = (i + 1) as u64;
        model.extend(records.into_iter().map(|(ent, a, v, asserted)| {
            (
                tx,
                NAMES[ent].to_string(),
                a.to_string(),
                v,
                asserted,
                tx_id,
            )
        }));
        if checkpoint_after == Some(i + 1) {
            db.checkpoint().unwrap();
        }
    }
    model
}

fn sorted(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort();
    rows
}

fn drain(log: &mut FactLog, max: usize) -> Vec<FactRecord> {
    let mut out = Vec::new();
    while let Some(batch) = log.next_batch(max).unwrap() {
        assert!(!batch.is_empty(), "a batch is never empty");
        assert!(batch.len() <= max.max(1), "batch exceeds max");
        out.extend(batch);
    }
    assert!(
        log.next_batch(max).unwrap().is_none(),
        "the end stays the end"
    );
    out
}

fn read(db: &Minigraf, filter: &FactFilter, max: usize) -> Vec<FactRecord> {
    drain(&mut db.fact_log(filter).unwrap(), max)
}

fn assert_tx_ordered(records: &[FactRecord]) {
    assert!(
        records.windows(2).all(|w| w[0].tx_count <= w[1].tx_count),
        "records out of tx order"
    );
}

fn file_db(path: &Path) -> Minigraf {
    let opts = OpenOptions::default().wal_checkpoint_threshold(usize::MAX);
    OpenOptions::path(opts, path).open().unwrap()
}

/// Every way the history can be split between committed and pending facts.
fn layouts(dir: &Path) -> Vec<(&'static str, Minigraf, Vec<Row>)> {
    let mem = Minigraf::in_memory().unwrap();
    let mem_model = write(&mem, None);
    let mut out = vec![("memory", mem, mem_model)];
    for (name, cp) in [
        ("pending", None),
        ("half", Some(4)),
        ("committed", Some(steps().len())),
    ] {
        let db = file_db(&dir.join(format!("{name}.graph")));
        let model = write(&db, cp);
        out.push((name, db, model));
    }
    let path = dir.join("reopened.graph");
    let model = {
        let db = file_db(&path);
        let model = write(&db, Some(2));
        db.checkpoint().unwrap();
        model
    };
    out.push(("reopened", file_db(&path), model));
    out
}

#[test]
fn every_record_in_every_layout_order_and_window() {
    let dir = tempfile::tempdir().unwrap();
    for (_name, db, model) in layouts(dir.path()) {
        for window in [1, 2, 3, 7, 1000] {
            for max in [1, 5, 1000] {
                let records = read(&db, &FactFilter::new().window(window), max);
                assert_tx_ordered(&records);
                let rows: Vec<Row> = records.iter().map(row).collect();
                assert_eq!(
                    sorted(rows),
                    sorted(model.clone()),
                    "tx order: wrong records"
                );
            }
        }
        for max in [1, 4, 1000] {
            let filter = FactFilter::new().order(FactOrder::Storage);
            let rows: Vec<Row> = read(&db, &filter, max).iter().map(row).collect();
            assert_eq!(
                sorted(rows),
                sorted(model.clone()),
                "storage order: wrong records"
            );
        }
    }
}

#[test]
fn records_are_identical_across_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let mut all = Vec::new();
    for (_name, db, _) in layouts(dir.path()) {
        let mut recs: Vec<(u64, String, String, String, bool, Option<i64>, i64)> =
            read(&db, &FactFilter::new(), 1000)
                .iter()
                .map(|r| {
                    (
                        r.tx_count,
                        name_of(&r.entity).to_string(),
                        r.attribute.clone(),
                        format!("{:?}", r.value),
                        r.asserted,
                        // `None` for the transaction time, so layouts written
                        // at different times compare equal.
                        (r.valid_from != r.tx_id as i64).then_some(r.valid_from),
                        r.valid_to,
                    )
                })
                .collect();
        recs.sort();
        all.push(recs);
    }
    assert!(all.windows(2).all(|w| w[0] == w[1]), "layouts disagree");
}

#[test]
fn valid_time_and_long_values_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    for (_name, db, _) in layouts(dir.path()) {
        let records = read(&db, &FactFilter::new(), 1000);
        let salary = records
            .iter()
            .find(|r| r.attribute == ":person/salary")
            .expect("salary record");
        assert_eq!(salary.valid_from, SALARY_FROM);
        assert_eq!(salary.valid_to, SALARY_TO);
        let name = records
            .iter()
            .find(|r| r.attribute == ":person/name")
            .expect("name record");
        assert_eq!(
            name.valid_from, name.tx_id as i64,
            "default valid_from is tx time"
        );
        assert_eq!(name.valid_to, i64::MAX, "default valid_to is forever");
        let body = records
            .iter()
            .find(|r| r.attribute == ":doc/body")
            .expect("long value record");
        assert_eq!(body.value, minigraf::Value::String(LONG.to_string()));
    }
}

#[test]
fn a_transaction_larger_than_the_window_is_split_without_loss() {
    let db = Minigraf::in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = file_db(&dir.path().join("bulk.graph"));
    for db in [&db, &file] {
        write(db, None);
    }
    file.checkpoint().unwrap();
    for db in [&db, &file] {
        let filter = FactFilter::new().attributes([":bulk/n"]).window(3);
        let records = read(db, &filter, 2);
        assert_eq!(records.len(), BULK);
        let mut values: Vec<String> = records.iter().map(|r| format!("{:?}", r.value)).collect();
        values.sort();
        values.dedup();
        assert_eq!(values.len(), BULK, "no duplicates");
    }
}

fn check_filter(filter: FactFilter, keep: impl Fn(&Row) -> bool) {
    let dir = tempfile::tempdir().unwrap();
    for (_name, db, model) in layouts(dir.path()) {
        let expected = sorted(model.into_iter().filter(|r| keep(r)).collect());
        for f in [
            filter.clone(),
            filter.clone().window(2),
            filter.clone().order(FactOrder::Storage),
        ] {
            let records = read(&db, &f, 3);
            let rows: Vec<Row> = records.iter().map(row).collect();
            assert_eq!(sorted(rows), expected, "filtered records differ");
        }
    }
}

#[test]
fn filter_exact_attributes() {
    check_filter(
        FactFilter::new().attributes([":person/name", ":person/age"]),
        |r| r.2 == ":person/name" || r.2 == ":person/age",
    );
}

#[test]
fn filter_attribute_prefix() {
    check_filter(FactFilter::new().attribute_prefix(":ingestion/"), |r| {
        r.2.starts_with(":ingestion/")
    });
}

#[test]
fn filter_attributes_or_prefix() {
    check_filter(
        FactFilter::new()
            .attributes([":doc/body"])
            .attribute_prefix(":ingestion/"),
        |r| r.2 == ":doc/body" || r.2.starts_with(":ingestion/"),
    );
}

#[test]
fn filter_entities() {
    check_filter(FactFilter::new().entities([uuid(0), uuid(2)]), |r| {
        r.1 == "e1" || r.1 == "e3"
    });
}

#[test]
fn filter_entities_and_attribute_prefix() {
    check_filter(
        FactFilter::new()
            .entities([uuid(1)])
            .attribute_prefix(":person/"),
        |r| r.1 == "e2" && r.2.starts_with(":person/"),
    );
}

#[test]
fn filter_tx_ranges() {
    check_filter(FactFilter::new().tx_range(2..=4), |r| {
        (2..=4).contains(&r.0)
    });
    check_filter(FactFilter::new().tx_range(..3), |r| r.0 < 3);
    check_filter(FactFilter::new().tx_range(5..), |r| r.0 >= 5);
    check_filter(FactFilter::new().tx_range(3..3), |_| false);
}

#[test]
fn filter_tx_range_and_attributes() {
    check_filter(
        FactFilter::new().attributes([":person/age"]).tx_range(3..),
        |r| r.2 == ":person/age" && r.0 >= 3,
    );
}

#[test]
fn filters_on_unknown_names_match_nothing() {
    check_filter(FactFilter::new().attributes([":no/such"]), |_| false);
    check_filter(FactFilter::new().attribute_prefix(":no/"), |_| false);
    check_filter(FactFilter::new().entities([uuid(99)]), |_| false);
    check_filter(FactFilter::new().attributes(Vec::<String>::new()), |_| {
        false
    });
}

#[test]
fn zero_max_makes_progress_and_iterator_matches_batches() {
    let db = Minigraf::in_memory().unwrap();
    let model = write(&db, None);
    let mut log = db.fact_log(&FactFilter::new()).unwrap();
    let first = log.next_batch(0).unwrap().expect("a record");
    assert_eq!(first.len(), 1);
    let batches = read(&db, &FactFilter::new(), 4);
    let iterated: Vec<FactRecord> = db
        .fact_log(&FactFilter::new())
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(iterated.len(), model.len());
    assert!(iterated == batches, "iterator and batches differ");
}

#[test]
fn empty_database_has_no_records() {
    let db = Minigraf::in_memory().unwrap();
    assert!(read(&db, &FactFilter::new(), 10).is_empty());
    let dir = tempfile::tempdir().unwrap();
    let file = file_db(&dir.path().join("empty.graph"));
    assert!(read(&file, &FactFilter::new(), 10).is_empty());
}

#[test]
fn writes_after_open_are_not_in_the_log() {
    let dir = tempfile::tempdir().unwrap();
    for (_name, db, model) in layouts(dir.path()) {
        for order in [FactOrder::Tx, FactOrder::Storage] {
            let before = read(&db, &FactFilter::new(), 1000).len();
            let mut log = db
                .fact_log(&FactFilter::new().order(order).window(2))
                .unwrap();
            let first = log.next_batch(1).unwrap().expect("a record");
            db.execute(r#"(transact [[:late :person/name "Late"]])"#)
                .unwrap();
            let mut records = first;
            records.extend(drain(&mut log, 3));
            assert_eq!(records.len(), before, "late write leaked in");
            assert!(before >= model.len());
        }
    }
}

fn wal_exists(path: &Path) -> bool {
    let mut p = path.as_os_str().to_owned();
    p.push(".wal");
    Path::new(&p).exists()
}

#[test]
fn checkpoint_and_rebuild_are_api_013_while_a_log_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pin.graph");
    let db = file_db(&path);
    let model = write(&db, Some(3));
    let log = db.fact_log(&FactFilter::new()).unwrap();

    let err = db.checkpoint().expect_err("checkpoint must be refused");
    assert_eq!(err.code(), "API-013");
    let err = db.rebuild_indexes().expect_err("rebuild must be refused");
    assert_eq!(err.code(), "API-013");
    assert!(db.verify().unwrap().is_ok(), "verify still runs");
    assert!(wal_exists(&path), "nothing was checkpointed");

    // The refused calls left the log intact.
    let records: Vec<FactRecord> = log.map(Result::unwrap).collect();
    assert_eq!(records.len(), model.len());

    // Reading to the end released the database.
    db.checkpoint().unwrap();
    assert!(!wal_exists(&path));
    let rows: Vec<Row> = read(&db, &FactFilter::new(), 100).iter().map(row).collect();
    assert_eq!(sorted(rows), sorted(model));
}

#[test]
fn checkpoint_with_nothing_to_write_is_allowed_while_a_log_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let db = file_db(&dir.path().join("idle.graph"));
    write(&db, Some(steps().len()));
    let _log = db.fact_log(&FactFilter::new()).unwrap();
    db.checkpoint().unwrap();
}

#[test]
fn close_releases_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("close.graph");
    let db = file_db(&path);
    write(&db, None);
    let mut log = db.fact_log(&FactFilter::new()).unwrap();
    assert!(log.next_batch(1).unwrap().is_some());
    log.close();
    db.checkpoint().unwrap();
    assert!(!wal_exists(&path));
}

#[test]
fn auto_checkpoint_waits_for_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auto.graph");
    let opts = OpenOptions::default().wal_checkpoint_threshold(2);
    let db = OpenOptions::path(opts, &path).open().unwrap();
    db.execute(r#"(transact [[:a :n 1]])"#).unwrap();
    let log = db.fact_log(&FactFilter::new()).unwrap();
    for i in 2..6 {
        db.execute(&format!("(transact [[:a :n {i}]])")).unwrap();
    }
    assert!(wal_exists(&path), "auto-checkpoint must wait");
    drop(log);
    db.execute(r#"(transact [[:a :n 6]])"#).unwrap();
    assert!(
        !wal_exists(&path),
        "the first write after close checkpoints"
    );
    assert_eq!(read(&db, &FactFilter::new(), 100).len(), 6);
}

#[test]
fn write_transaction_commit_defers_auto_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wtx.graph");
    let opts = OpenOptions::default().wal_checkpoint_threshold(1);
    let db = OpenOptions::path(opts, &path).open().unwrap();
    let log = db.fact_log(&FactFilter::new()).unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:a :n 1]])"#).unwrap();
    tx.commit().unwrap();
    assert!(wal_exists(&path), "commit's auto-checkpoint must wait");
    drop(log);
    assert_eq!(read(&db, &FactFilter::new(), 100).len(), 1);
}

#[test]
fn fact_log_on_a_write_transaction_thread_is_int_001() {
    let db = Minigraf::in_memory().unwrap();
    let _tx = db.begin_write().unwrap();
    let err = db
        .fact_log(&FactFilter::new())
        .expect_err("must not open inside a write transaction");
    assert_eq!(err.code(), "INT-001");
}

#[test]
fn log_is_send_and_outlives_the_database_handle() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<FactLog>();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("send.graph");
    let db = file_db(&path);
    let model = write(&db, Some(4));
    let log = db.fact_log(&FactFilter::new().window(2)).unwrap();
    drop(db);
    let n = std::thread::spawn(move || log.map(Result::unwrap).count())
        .join()
        .unwrap();
    assert_eq!(n, model.len());

    // The pending writes were kept in the WAL and replay on the next open.
    let db = file_db(&path);
    let rows: Vec<Row> = read(&db, &FactFilter::new(), 100).iter().map(row).collect();
    assert_eq!(sorted(rows), sorted(model));
}
