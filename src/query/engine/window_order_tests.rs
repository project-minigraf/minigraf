//! Snapshot-certified partition windows compared with the full-input evaluator.
use super::{BATCH_ROWS, Counters};
use crate::{Minigraf, Value};
use std::mem::size_of;

fn fixture(n: usize, size: usize) -> (tempfile::TempDir, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("parts.graph")).unwrap();
    let facts: String = (0..n)
        .map(|i| format!("[:e{i} :part {}] [:e{i} :value {}]", i / size, i % 7))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    (dir, db)
}
fn collect(db: &Minigraf, q: &str) -> (Vec<Vec<Value>>, Vec<Counters>, usize) {
    let mut cursor = db.query(&format!("(query {q})")).unwrap();
    let stream = cursor.stream_mut().unwrap();
    let meter = stream.meter().clone();
    let mut rows = Vec::new();
    while let Some(batch) = stream.next_rows().unwrap() {
        rows.extend(batch);
    }
    assert_eq!(meter.used(), 0);
    (rows, stream.counters(), meter.peak())
}
fn partitioned(c: &[Counters]) -> bool {
    c.iter().any(|c| c.op == "WindowPartitioned")
}
fn same(a: &[Vec<Value>], b: &[Vec<Value>]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| match (a, b) {
                    (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
                    _ => a == b,
                })
        })
}
fn chosen_avet_oracle(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    use crate::query::datalog::{
        functions::FunctionRegistry, parser::parse_datalog_command, types::DatalogCommand,
    };
    let DatalogCommand::Query(q) = parse_datalog_command(&format!("(query {q})")).unwrap() else {
        panic!("query");
    };
    let mut plan = super::plan(&q).unwrap();
    let super::plan::Source::Scan(spec) = &mut plan.source else {
        panic!("scan");
    };
    spec.index = Some(crate::storage::keys::Index::Avet);
    let _off = super::order::set_enabled(false);
    let mut stream = super::open(
        db.snapshot().unwrap(),
        plan,
        std::sync::Arc::new(FunctionRegistry::with_builtins()),
        usize::MAX,
    )
    .unwrap();
    assert!(!partitioned(&stream.counters()));
    let mut rows = Vec::new();
    while let Some(batch) = stream.next_rows().unwrap() {
        rows.extend(batch);
    }
    rows
}

#[test]
fn compatible_windows_keep_independent_orders_peers_frames_and_neighbors() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let (_dir, db) = fixture(4099, 17);
    let q = "[:find ?e ?g ?v (row-number :over (:partition-by ?g :order-by ?v)) \
        (rank :over (:partition-by ?g :order-by ?v)) \
        (sum ?v :over (:partition-by ?g :order-by ?v)) \
        (count ?v :over (:partition-by ?g :order-by ?v)) \
        (min ?v :over (:partition-by ?g :order-by ?v :desc :rows 2 preceding)) \
        (max ?v :over (:partition-by ?g :order-by ?v :rows 0 preceding)) \
        (avg ?v :over (:partition-by ?g :order-by ?e :rows 2 preceding)) \
        (lag ?v :over (:partition-by ?g :order-by ?v)) \
        (lead ?v :over (:partition-by ?g :order-by ?v :desc)) \
        :where [?e :part ?g] [?e :value ?v] :order-by [?g ?e]]";
    let (expected, _, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    let (actual, counters, _) = collect(&db, q);
    assert!(partitioned(&counters));
    assert!(
        same(&actual, &expected),
        "shared evaluator preserves every window column"
    );
}

#[test]
fn a_window_limit_stops_after_first_partition_and_one_input_batch() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let (_dir, db) = fixture(8192, 17);
    let q = "[:find ?e ?g ?v (row-number :over (:partition-by ?g :order-by ?v)) \
        :where [?e :part ?g] [?e :value ?v] :limit 10]";
    let expected = chosen_avet_oracle(&db, q);
    let (actual, counters, _) = collect(&db, q);
    assert!(partitioned(&counters));
    assert!(
        same(&actual, &expected),
        "preserve the chosen input's order"
    );
    let window = counters
        .iter()
        .find(|c| c.op == "WindowPartitioned")
        .unwrap();
    assert!(window.rows_in <= (17 + BATCH_ROWS) as u64);
}

#[test]
fn a_fixed_partition_bounds_metered_state_as_total_rows_grow() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let mut peaks = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, 17);
        let (rows, counters, peak) = collect(
            &db,
            "[:find ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]]",
        );
        assert_eq!(rows.len(), n);
        assert!(partitioned(&counters));
        peaks.push(peak);
    }
    use super::{
        batch::Batch,
        cell::{Cell, Scalar},
        meter::cell_bytes,
    };
    let two_batches = 2
        * (std::mem::size_of::<Batch>()
            + 2 * std::mem::size_of::<Vec<Cell>>()
            + BATCH_ROWS * (cell_bytes(&Cell::Eid(1)) + cell_bytes(&Cell::Val(Scalar::Int(0)))));
    assert!(
        peaks[1] <= peaks[0] + two_batches,
        "completed partitions and DICT caches are released"
    );
}

#[test]
fn one_growing_partition_is_an_explicit_linear_control() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let mut peaks = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, n);
        let (rows, counters, peak) = collect(
            &db,
            "[:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]]",
        );
        assert_eq!(rows.len(), n);
        assert!(partitioned(&counters));
        peaks.push(peak);
    }
    assert!(peaks[1] > 4 * peaks[0]);
}

#[test]
fn incompatible_unpartitioned_constant_and_aggregate_queries_keep_full_window() {
    let (_dir, db) = fixture(64, 4);
    for q in [
        "[:find ?g (row-number :over (:order-by ?v)) :where [?e :part ?g] [?e :value ?v]]",
        "[:find ?g (rank :over (:partition-by ?g :order-by ?v)) (row-number :over (:partition-by ?e :order-by ?v)) :where [?e :part ?g] [?e :value ?v]]",
        "[:find ?g (count ?e) (row-number :over (:partition-by ?g :order-by ?g)) :where [?e :part ?g]]",
        "[:find ?g (row-number :over (:partition-by ?p :order-by ?g)) :where [?e :part ?g] [(+ 0 1) ?p]]",
    ] {
        assert!(!partitioned(&collect(&db, q).1));
    }
}

#[test]
fn entity_prefix_preserves_input_order_with_arbitrary_value_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("entity.graph")).unwrap();
    let facts: String = (0..4099)
        .map(|i| format!("[:e{} :value {i}]", i / 17))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let long = "x".repeat(100);
    db.execute(&format!("(transact [[:e0 :value \"{long}\"] [:e0 :value :k] [:e0 :value nil] [:e0 :value false] [:e0 :value #{{1 :k}}]])")).unwrap();
    db.checkpoint().unwrap();
    let q = "[:find ?e ?v (row-number :over (:partition-by ?e :order-by ?v :desc)) (min ?v :over (:partition-by ?e :order-by ?v)) (lag ?v :over (:partition-by ?e :order-by ?v)) :where [?e :value ?v]]";
    let (expected, _, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    let (actual, counters, _) = collect(&db, q);
    assert!(
        partitioned(&counters),
        "entity adjacency doesn't need semantic value order"
    );
    assert!(
        same(&actual, &expected),
        "the unchanged AEVT scan's physical row order is preserved"
    );
}

#[test]
fn late_callbacks_and_collect_set_errors_still_fail_before_answers() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let (_dir, db) = fixture(4096, 17);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    db.try_register_predicate("late?", move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) == 1499 {
            Err(crate::UdfError::new("late callback"))
        } else {
            Ok(true)
        }
    })
    .unwrap();
    let q = "(query [:find ?g (row-number :over (:partition-by ?g :order-by ?v)) :where [?e :part ?g] [?e :value ?v] [(= (late? ?g) true)] :limit 1])";
    let mut cursor = db.query(q).unwrap();
    assert!(!partitioned(&cursor.stream_mut().unwrap().counters()));
    assert_eq!(
        cursor
            .next_batch(1)
            .expect_err("late callback remains visible")
            .code(),
        "QRY-014"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1500);
    db.execute("(transact [[:e4095 :nested #{1}]])").unwrap();
    db.checkpoint().unwrap();
    let q = "(query [:find ?g (row-number :over (:partition-by ?g :order-by ?v)) :where [?e :part ?g] [?e :value ?v] [(collect-set ?e :nested) ?s] :limit 1])";
    assert_eq!(
        db.execute(q)
            .expect_err("late nested set remains visible")
            .code(),
        "QRY-015"
    );
}

#[test]
fn pending_scope_and_prepared_cursors_choose_fresh_strategies() {
    let (_dir, db) = fixture(64, 4);
    let q = "[:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g] :order-by [?g ?e]]";
    let prepared = db.prepare(&format!("(query {q})")).unwrap();
    let mut old = prepared.query(&[]).unwrap();
    assert!(partitioned(&old.stream_mut().unwrap().counters()));
    let expected = collect(&db, q).0;
    db.execute("(transact [[:outside :other 1]])").unwrap();
    assert!(partitioned(&collect(&db, q).1));
    db.execute("(transact [[:new :part -1]])").unwrap();
    let mut fresh = prepared.query(&[]).unwrap();
    assert!(!partitioned(&fresh.stream_mut().unwrap().counters()));
    drop(fresh);
    db.checkpoint().unwrap();
    let mut rows = Vec::new();
    while let Some(batch) = old.next_batch(3).unwrap() {
        rows.extend(batch);
    }
    assert!(
        same(&rows, &expected),
        "pinned partition cursor survives checkpoint and reuse"
    );
    let mut fresh = prepared.query(&[]).unwrap();
    assert!(partitioned(&fresh.stream_mut().unwrap().counters()));
}

#[test]
fn close_drop_and_rewind_release_partition_lookahead_and_results() {
    let (_dir, db) = fixture(4099, 17);
    let q = "(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]])";
    for close in [false, true] {
        let mut cursor = db.query(q).unwrap();
        let stream = cursor.stream_mut().unwrap();
        assert!(partitioned(&stream.counters()));
        let meter = stream.meter().clone();
        let first = stream.next_rows().unwrap().unwrap();
        assert!(meter.used() > 0);
        stream.root.rewind(&[]);
        stream.ids.end_pull();
        assert_eq!(meter.used(), 0);
        assert!(same(&first, &stream.next_rows().unwrap().unwrap()));
        if close {
            cursor.close();
        } else {
            drop(cursor);
        }
        assert_eq!(meter.used(), 0);
    }
}

#[test]
fn small_budget_streams_fixed_partitions_and_full_fallback_refuses() {
    let (dir, db) = fixture(4096, 17);
    drop(db);
    let db = crate::OpenOptions::new()
        .query_memory_limit(128 << 10)
        .path(dir.path().join("parts.graph"))
        .open()
        .unwrap();
    let q = "[:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]]";
    let (rows, counters, _) = collect(&db, q);
    assert_eq!(rows.len(), 4096);
    assert!(partitioned(&counters));
    let _off = super::order::set_enabled(false);
    assert_eq!(
        db.execute(&format!("(query {q})"))
            .expect_err("full window exceeds budget")
            .code(),
        "QRY-012"
    );
}

#[test]
fn id_value_prefixes_partition_without_becoming_semantic_group_orders() {
    let (_dir, db) = fixture(64, 4);
    let long = "x".repeat(100);
    let facts:String=(0..64).map(|i|{
        let keyword=if i%2==0 {":z"}else{":a"};
        let id=if i%2==0 {2}else{1};
        format!("[:e{i} :kwpart {keyword}] [:e{i} :refpart #uuid \"00000000-0000-0000-0000-{id:012}\"] [:e{i} :boolpart {}] [:e{i} :longpart \"{long}{}\"] [:e{i} :setpart #{{{}}}]",i%2==0,i%2,i%2)
    }).collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    for (attribute, expected_mode) in [
        ("kwpart", true),
        ("refpart", true),
        ("boolpart", true),
        ("longpart", false),
        ("setpart", false),
    ] {
        let q = format!(
            "[:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :{attribute} ?g] :order-by [?g ?e]]"
        );
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        let (actual, counters, _) = collect(&db, &q);
        assert_eq!(partitioned(&counters), expected_mode);
        assert!(same(&actual, &expected));
        if ["kwpart", "refpart"].contains(&attribute) {
            let q = format!("[:find ?g (count ?e) :where [?e :{attribute} ?g]]");
            let (expected, _, _) = {
                let _off = super::order::set_enabled(false);
                collect(&db, &q)
            };
            let (actual, counters, _) = collect(&db, &q);
            assert!(counters.iter().all(|c| c.op != "AggregateOrdered"));
            assert!(
                same(&actual, &expected),
                "Hash Aggregate still emits semantic Value::Ord groups"
            );
        }
    }
}

#[test]
fn registered_window_callbacks_keep_full_evaluation_under_limit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_dir, db) = fixture(4096, 17);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    db.try_register_aggregate(
        "late-window",
        || Ok(0i64),
        move |acc: &mut i64, _| {
            if seen.fetch_add(1, Ordering::SeqCst) == 1499 {
                return Err(crate::UdfError::new("late window callback"));
            }
            *acc += 1;
            Ok(())
        },
        |acc: &i64, _| Ok(Value::Integer(*acc)),
    )
    .unwrap();
    let q = "(query [:find ?g (late-window ?v :over (:partition-by ?g :order-by ?v)) :where [?e :part ?g] [?e :value ?v] :limit 1])";
    let mut cursor = db.query(q).unwrap();
    assert!(!partitioned(&cursor.stream_mut().unwrap().counters()));
    assert_eq!(
        cursor
            .next_batch(1)
            .expect_err("must reach the late window callback")
            .code(),
        "QRY-014"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1500);
}

#[test]
fn empty_committed_history_and_literal_ranges_keep_their_contracts() {
    let (_dir, db) = fixture(1, 1);
    let (_, counters, _) = collect(
        &db,
        "[:find ?e (row-number :over (:partition-by ?e :order-by ?e)) :where [?e :part 0]]",
    );
    assert!(
        !partitioned(&counters),
        "literal value ranges aren't certified in this delivery"
    );
    db.execute("(retract [[:e0 :part 0]])").unwrap();
    db.checkpoint().unwrap();
    let (rows, counters, _) = collect(
        &db,
        "[:find ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]]",
    );
    assert!(rows.is_empty());
    assert!(partitioned(&counters));
}

#[test]
fn lookahead_is_charged_once_and_released_with_the_partition_cursor() {
    use super::{
        batch::Batch,
        cell::{Cell, Scalar},
        meter::{cell_bytes, row_bytes},
    };
    let (_dir, db) = fixture(4099, 1500);
    let mut cursor=db.query("(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]])").unwrap();
    let stream = cursor.stream_mut().unwrap();
    assert!(partitioned(&stream.counters()));
    let meter = stream.meter().clone();
    assert_eq!(stream.next_rows().unwrap().unwrap().len(), BATCH_ROWS);
    let charged = size_of::<Batch>()
        + 2 * size_of::<Vec<Cell>>()
        + BATCH_ROWS * (cell_bytes(&Cell::Eid(1)) + cell_bytes(&Cell::Val(Scalar::Int(0))));
    let charged = charged
        + (1500 - BATCH_ROWS)
            * row_bytes(&[
                Cell::Eid(1),
                Cell::Val(Scalar::Int(0)),
                Cell::Val(Scalar::Int(1)),
            ]);
    let window = stream
        .counters()
        .into_iter()
        .find(|c| c.op == "WindowPartitioned")
        .unwrap();
    assert_eq!(
        window.bytes_held, charged as u64,
        "lookahead plus remaining results of one partition"
    );
    assert_eq!(
        meter.used(),
        charged,
        "outer pull evicts committed lookup maps"
    );
    super::window::PARTITION_MAX_ROWS.with(|n| assert_eq!(n.get(), 1500));
    stream.close();
    assert_eq!(meter.used(), 0);
}

#[test]
fn one_partition_spanning_many_batches_evicts_dict_lookups_internally() {
    let mut ids = Vec::new();
    let mut windows = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, n);
        let mut cursor=db.query("(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) (lag ?e :over (:partition-by ?g :order-by ?e)) (lead ?e :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]])").unwrap();
        let stream = cursor.stream_mut().unwrap();
        assert!(partitioned(&stream.counters()));
        let meter = stream.meter().clone();
        let mut rows = 0;
        while let Some(batch) = stream.next_rows().unwrap() {
            rows += batch.len();
        }
        assert_eq!(rows, n);
        assert_eq!(meter.used(), 0);
        assert!(
            meter.peak_for("IdSpace") > 0,
            "the fixture actually resolves UUIDs"
        );
        ids.push(meter.peak_for("IdSpace"));
        windows.push(meter.peak_for("Window"));
    }
    assert!(
        windows[1] > 4 * windows[0],
        "one growing partition remains linear"
    );
    // Projection can resolve three unrelated references per output row:
    // input EID order differs from the UUID order used by lag/lead. The
    // 1024-row control has only 1024 distinct entities; the larger partition
    // can therefore reach a higher, still one-output-batch lookup peak.
    let eid_entry = 2
        * (size_of::<crate::graph::types::EntityId>()
            + size_of::<u64>()
            + super::meter::HASH_ENTRY_OVERHEAD);
    for peak in ids {
        assert!(
            peak <= 3 * BATCH_ROWS * eid_entry,
            "internal window stages must not retain all DICT lookups"
        );
    }
}

#[test]
fn partition_budget_and_allocator_errors_release_every_holder() {
    let (dir, db) = fixture(4096, 4096);
    drop(db);
    let db = crate::OpenOptions::new()
        .query_memory_limit(128 << 10)
        .path(dir.path().join("parts.graph"))
        .open()
        .unwrap();
    let q = "(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g]])";
    let mut cursor = db.query(q).unwrap();
    let stream = cursor.stream_mut().unwrap();
    assert!(partitioned(&stream.counters()));
    let meter = stream.meter().clone();
    assert_eq!(
        crate::MinigrafError::from(
            stream
                .next_rows()
                .expect_err("growing partition exceeds budget")
        )
        .code(),
        "QRY-012"
    );
    assert_eq!(meter.used(), 0);
    let (_dir, db) = fixture(64, 4);
    let mut cursor = db.query(q).unwrap();
    let stream = cursor.stream_mut().unwrap();
    assert!(partitioned(&stream.counters()));
    let meter = stream.meter().clone();
    super::meter::FAIL_RESERVE.with(|f| f.set(Some("Window")));
    let result = stream.next_rows();
    super::meter::FAIL_RESERVE.with(|f| f.set(None));
    assert_eq!(
        crate::MinigrafError::from(result.expect_err("allocator refusal")).code(),
        "QRY-012"
    );
    assert_eq!(meter.used(), 0);
}

#[test]
fn working_reader_proofs_track_finished_and_rejected_statements() {
    fn rows(result: crate::QueryResult) -> Vec<Vec<Value>> {
        let crate::QueryResult::QueryResults { results, .. } = result else {
            panic!("rows");
        };
        results
    }
    let dir = tempfile::tempdir().unwrap();
    let db = crate::OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(dir.path().join("working.graph"))
        .open()
        .unwrap();
    db.execute("(transact [[:a :part 0] [:b :part 2]])")
        .unwrap();
    db.checkpoint().unwrap();
    let q = "(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) :where [?e :part ?g] :order-by [?g ?e]])";
    let proofs = || super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:staged :part -1]])").unwrap();
    let before = proofs();
    tx.execute(q).unwrap();
    assert_eq!(proofs(), before, "staged source stays full");
    drop(tx);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:new :part -1] [:x :other 1] [:y :other 2]])")
        .unwrap();
    let before = proofs();
    let expected = {
        let _off = super::order::set_enabled(false);
        rows(tx.execute(q).unwrap())
    };
    assert!(same(&rows(tx.execute(q).unwrap()), &expected));
    assert_eq!(proofs(), before + 1, "working integer roots partition");
    let err=tx.execute("(transact [[:bad :part \"rolled back\"] [:dup :other 1 {:valid-from \"2020-01-01\"}] [:fill :other 2] [:dup :other 1 {:valid-from \"2021-01-01\"}]])").expect_err("two windows for one triple");
    assert_eq!(err.code(), "API-011");
    assert!(same(&rows(tx.execute(q).unwrap()), &expected));
    assert_eq!(
        proofs(),
        before + 2,
        "rejected statement restores proof roots"
    );
    tx.execute("(transact [[:mixed :part \"string\"]])")
        .unwrap();
    let expected = {
        let _off = super::order::set_enabled(false);
        rows(tx.execute(q).unwrap())
    };
    assert!(same(&rows(tx.execute(q).unwrap()), &expected));
    assert_eq!(
        proofs(),
        before + 2,
        "working mixed strings disable value certificate"
    );
    drop(tx);
    assert!(partitioned(
        &collect(
            &db,
            q.strip_prefix("(query ")
                .unwrap()
                .strip_suffix(')')
                .unwrap()
        )
        .1
    ));
}

/// Query-only meter diagnostic over separately built committed fixtures.
#[test]
#[ignore]
fn committed_window_memory_probe() {
    let path = std::env::var("MINIGRAF_GROUP_FIXTURE").expect("fixture path");
    let n: usize = std::env::var("MINIGRAF_GROUP_ROWS")
        .unwrap()
        .parse()
        .unwrap();
    let size: usize = std::env::var("MINIGRAF_GROUP_SIZE")
        .unwrap()
        .parse()
        .unwrap();
    let db = crate::OpenOptions::new()
        .read_only(true)
        .path(path)
        .open()
        .unwrap();
    for (mode, partition, limited) in [
        ("value", "?g", false),
        ("value_limit", "?g", true),
        ("entity", "?e", false),
        ("entity_limit", "?e", true),
    ] {
        if size == n && partition == "?e" {
            continue;
        }
        let q = format!(
            "(query [:find ?e ?g (row-number :over (:partition-by {partition} :order-by ?e)) :where [?e :group ?g] {}])",
            if limited { ":limit 10" } else { "" }
        );
        let mut cursor = db.query(&q).unwrap();
        let stream = cursor.stream_mut().unwrap();
        assert!(partitioned(&stream.counters()));
        let meter = stream.meter().clone();
        let mut rows = 0;
        while let Some(batch) = stream.next_rows().unwrap() {
            rows += batch.len();
        }
        assert_eq!(rows, if limited { 10.min(n) } else { n });
        assert_eq!(meter.used(), 0);
        let max_rows = super::window::PARTITION_MAX_ROWS.with(|n| n.get());
        let expected = if partition == "?e" { 1 } else { size };
        assert_eq!(max_rows, expected.min(n));
        let input = stream
            .counters()
            .into_iter()
            .find(|c| c.op == "WindowPartitioned")
            .unwrap()
            .rows_in;
        if limited {
            assert!(input <= (10usize.max(expected) + BATCH_ROWS) as u64);
        }
        use super::{
            batch::Batch,
            cell::{Cell, Scalar},
            meter::cell_bytes,
        };
        let two_batches = 2
            * (size_of::<Batch>()
                + 2 * size_of::<Vec<Cell>>()
                + BATCH_ROWS
                    * (cell_bytes(&Cell::Eid(1)) + cell_bytes(&Cell::Val(Scalar::Int(0)))));
        println!(
            "{{\"mode\":\"{mode}\",\"facts\":{n},\"fixture_group_size\":{size},\"rows\":{rows},\"total_peak_bytes\":{},\"window_peak_bytes\":{},\"id_peak_bytes\":{},\"max_partition_rows\":{max_rows},\"input_rows\":{input},\"two_batch_allowance_bytes\":{two_batches}}}",
            meter.peak(),
            meter.peak_for("Window"),
            meter.peak_for("IdSpace")
        );
    }
}

#[test]
fn many_single_row_partitions_fill_a_bounded_output_batch() {
    let (_dir, db) = fixture(4099, 1);
    let q = "[:find ?e ?g (row-number :over (:partition-by ?e :order-by ?g)) :where [?e :part ?g]]";
    let (expected, _, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    let mut cursor = db.query(&format!("(query {q})")).unwrap();
    let stream = cursor.stream_mut().unwrap();
    assert!(partitioned(&stream.counters()));
    let first = stream.next_rows().unwrap().unwrap();
    assert!(
        first.len() >= BATCH_ROWS - 1 && first.len() <= BATCH_ROWS,
        "small partitions should share a passing output batch"
    );
    let mut rows = first;
    while let Some(batch) = stream.next_rows().unwrap() {
        rows.extend(batch);
    }
    assert!(same(&rows, &expected));
}

#[test]
fn partitions_on_and_across_input_batch_edges_match_full_evaluation() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for size in [16, BATCH_ROWS - 1, BATCH_ROWS, BATCH_ROWS + 1] {
        let (_dir, db) = fixture(3 * BATCH_ROWS + 7, size);
        let q = "[:find ?e ?g ?v (rank :over (:partition-by ?g :order-by ?v)) (lag ?v :over (:partition-by ?g :order-by ?v)) (sum ?v :over (:partition-by ?g :order-by ?v :rows 2 preceding)) :where [?e :part ?g] [?e :value ?v] :order-by [?g ?e]]";
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, q)
        };
        let (actual, counters, _) = collect(&db, q);
        assert!(partitioned(&counters));
        assert!(same(&actual, &expected));
        super::window::PARTITION_MAX_ROWS.with(|n| assert_eq!(n.get(), size));
    }
}

#[test]
fn selective_eavt_id_partitions_and_absent_attributes_keep_their_contracts() {
    let (_dir, db) = fixture(1, 1);
    db.execute("(transact [[:one :kw :z] [:one :kw :a] [:one :ref #uuid \"00000000-0000-0000-0000-000000000002\"] [:one :ref #uuid \"00000000-0000-0000-0000-000000000001\"] [:one :idmix :z] [:one :idmix #uuid \"00000000-0000-0000-0000-000000000002\"]])").unwrap();
    db.checkpoint().unwrap();
    for (attribute, mode) in [("kw", true), ("ref", true), ("idmix", false)] {
        let q = format!(
            "[:find ?g (row-number :over (:partition-by ?g :order-by ?g)) :where [:one :{attribute} ?g]]"
        );
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        let (actual, counters, _) = collect(&db, &q);
        assert_eq!(partitioned(&counters), mode);
        assert!(same(&actual, &expected));
    }
    let (rows, counters, _) = collect(
        &db,
        "[:find ?e (row-number :over (:partition-by ?e :order-by ?e)) :where [?e :missing ?v]]",
    );
    assert!(rows.is_empty());
    assert!(
        partitioned(&counters),
        "empty entity prefix needs no value-kind proof"
    );
    let (rows, counters, _) = collect(
        &db,
        "[:find ?g (row-number :over (:partition-by ?g :order-by ?g)) :where [:one :missing ?g]]",
    );
    assert!(rows.is_empty());
    assert!(
        !partitioned(&counters),
        "a value certificate needs the attribute id"
    );
}
