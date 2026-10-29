//! Snapshot-certified one-group aggregation (#641).
use super::{BATCH_ROWS, Counters};
use crate::{Minigraf, Value};

fn fixture(n: usize, size: usize) -> (tempfile::TempDir, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("groups.graph")).unwrap();
    let facts: String = (0..n)
        .map(|i| format!("[:e{i} :bucket {}] [:e{i} :item {}]", i / size, i % 3))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    (dir, db)
}

fn collect(db: &Minigraf, query: &str) -> (Vec<Vec<Value>>, Vec<Counters>, usize) {
    let mut cursor = db.query(&format!("(query {query})")).unwrap();
    let stream = cursor.stream_mut().unwrap();
    let meter = stream.meter().clone();
    let mut rows = Vec::new();
    while let Some(batch) = stream.next_rows().unwrap() {
        rows.extend(batch);
    }
    assert_eq!(meter.used(), 0, "drain releases operator state");
    (rows, stream.counters(), meter.peak())
}

fn ordered(counters: &[Counters]) -> bool {
    counters.iter().any(|c| c.op == "AggregateOrdered")
}

#[test]
fn generalized_avg_streams_and_stops_at_group_demand() {
    let (_dir, db) = fixture(4101, 3);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let query = "[:find ?g (avg ?v) :where [?e :bucket ?g] [?e :item ?v] \
                 :order-by [?g] :offset 2 :limit 3]";
    let (expected, _, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, query)
    };
    let (rows, counters, _) = collect(&db, query);
    assert!(ordered(&counters), "avg releases completed groups");
    assert!(rows == expected);
    assert!(
        rows == (2..5)
            .map(|g| vec![Value::Integer(g), Value::Float(1.0)])
            .collect::<Vec<_>>()
    );
    assert!(counters.iter().all(|c| c.op != "Sort"));
    assert!(
        counters
            .iter()
            .find(|c| c.op == "AggregateOrdered")
            .unwrap()
            .rows_in
            <= (15 + BATCH_ROWS) as u64
    );
}

#[test]
fn generalized_constant_and_repeated_group_columns_keep_projection_and_sort_order() {
    let (_dir, db) = fixture(4101, 3);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for (query, sorted) in [
        (
            "[:find ?k ?g (count ?e) :where [?e :bucket ?g] [(+ 1 1) ?k] :order-by [?k ?g] :limit 3]",
            false,
        ),
        (
            "[:find ?k ?g (count ?e) :where [?e :bucket ?g] [(+ 1 1) ?k] :order-by [?k ?g :desc] :limit 3]",
            true,
        ),
        (
            "[:find ?g ?k (count ?e) :where [?e :bucket ?g] [(= 1 1) ?k] :order-by [?g] :limit 3]",
            false,
        ),
        (
            "[:find ?g ?g (count-distinct ?e) :where [?e :bucket ?g] :order-by [?g] :limit 3]",
            false,
        ),
    ] {
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, query)
        };
        let (rows, counters, _) = collect(&db, query);
        assert!(ordered(&counters), "complete scalar tuple streams");
        assert!(
            rows == expected,
            "complete tuple order and projection stay unchanged"
        );
        assert_eq!(counters.iter().any(|c| c.op == "Sort"), sorted);
        assert_eq!(rows.len(), 3);
    }
}

#[test]
fn generalized_hidden_groups_stream_but_keep_visible_tie_sorting() {
    let (_dir, db) = fixture(4100, 3);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for query in [
        "[:find (count ?e) :with ?g :where [?e :bucket ?g]]",
        "[:find ?k (count ?e) :with ?g :where [?e :bucket ?g] [(+ 1 1) ?k] :order-by [?k (count ?e)] :limit 3]",
        "[:find ?g (avg ?v) :with ?k :where [?e :bucket ?g] [?e :item ?v] [(= 1 1) ?k] :order-by [?g] :offset 2 :limit 3]",
    ] {
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, query)
        };
        let (rows, counters, _) = collect(&db, query);
        assert!(
            ordered(&counters),
            "proved hidden key belongs to effective tuple"
        );
        assert!(rows == expected);
        assert_eq!(
            counters.iter().any(|c| c.op == "Sort"),
            query.contains(":order-by")
        );
    }
}

#[test]
fn generalized_empty_hidden_count_groups_emit_one_zero_row() {
    let (_dir, db) = fixture(1, 1);
    db.execute("(retract [[:e0 :bucket 0]])").unwrap();
    db.checkpoint().unwrap();
    for (query, expected) in [
        (
            "[:find (count ?e) (count-distinct ?e) :with ?g :where [?e :bucket ?g]]",
            vec![vec![Value::Integer(0), Value::Integer(0)]],
        ),
        ("[:find (avg ?g) :with ?g :where [?e :bucket ?g]]", vec![]),
    ] {
        let (rows, counters, _) = collect(&db, query);
        assert!(
            ordered(&counters),
            "raw history still certifies the empty range"
        );
        assert!(
            rows == expected,
            "empty hidden groups preserve zero_row semantics"
        );
    }
}

#[test]
fn generalized_unproved_tuple_components_stay_materialized() {
    let (_dir, db) = fixture(64, 4);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for query in [
        "[:find ?g ?v (avg ?v) :where [?e :bucket ?g] [?e :item ?v]]",
        "[:find ?g (avg ?v) :with ?v :where [?e :bucket ?g] [?e :item ?v]]",
        "[:find ?g ?k (count ?e) :where [?e :bucket ?g] [(+ 1.0 1.0) ?k]]",
        "[:find ?e ?g (count ?e) :where [?e :bucket ?g]]",
        "[:find ?k (avg ?g) :where [?e :bucket ?g] [(+ 1 1) ?k]]",
    ] {
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, query)
        };
        let (rows, counters, _) = collect(&db, query);
        assert!(!ordered(&counters), "complete tuple is not certified");
        assert!(rows == expected);
    }
}

#[test]
fn generalized_prepared_tuple_proofs_follow_bindings_and_pinned_snapshots() {
    let (_dir, db) = fixture(65, 4);
    let prepared = db.prepare("(query [:find ?k ?g (avg ?g) :where [?e :bucket ?g] [(+ $k 0) ?k] :order-by [?k ?g] :limit 2])").unwrap();
    let integers = [("k", crate::BindValue::Val(Value::Integer(2)))];
    let floats = [("k", crate::BindValue::Val(Value::Float(2.0)))];
    let mut pinned = prepared.query(&integers).unwrap();
    assert!(ordered(&pinned.counters()));
    assert!(
        !ordered(&prepared.query(&floats).unwrap().counters()),
        "prepared float constant invalidates the composite certificate"
    );
    db.execute("(transact [[:outside :other 1]])").unwrap();
    assert!(ordered(&prepared.query(&integers).unwrap().counters()));
    db.execute("(transact [[:new :bucket -1]])").unwrap();
    assert!(!ordered(&prepared.query(&integers).unwrap().counters()));
    db.checkpoint().unwrap();
    assert!(ordered(&prepared.query(&integers).unwrap().counters()));
    let mut old = Vec::new();
    while let Some(batch) = pinned.next_batch(1).unwrap() {
        old.extend_from_slice(batch.rows());
    }
    assert!(
        old == vec![
            vec![Value::Integer(2), Value::Integer(0), Value::Float(0.0)],
            vec![Value::Integer(2), Value::Integer(1), Value::Float(1.0)],
        ],
        "checkpoint does not change the pinned input proof"
    );
    let mut fresh = prepared.query(&integers).unwrap();
    assert!(
        fresh.next_batch(1).unwrap().unwrap().rows()
            == [vec![
                Value::Integer(2),
                Value::Integer(-1),
                Value::Float(-1.0)
            ]]
    );
}

#[test]
fn generalized_avg_tracks_aggregate_and_idspace_peaks_separately() {
    for refs in [false, true] {
        let mut peaks = Vec::new();
        for n in [1024, 8192] {
            let (_dir, db) = fixture(n, if refs { n } else { 1 });
            let arg = if refs { "?e" } else { "?g" };
            let mut cursor = db
                .query(&format!(
                    "(query [:find ?g (avg {arg}) :where [?e :bucket ?g]])"
                ))
                .unwrap();
            let stream = cursor.stream_mut().unwrap();
            assert!(ordered(&stream.counters()));
            let meter = stream.meter().clone();
            let mut rows = 0;
            while let Some(batch) = stream.next_rows().unwrap() {
                if refs {
                    assert!(batch == vec![vec![Value::Integer(0), Value::Null]]);
                }
                rows += batch.len();
            }
            assert_eq!(rows, if refs { 1 } else { n });
            assert_eq!(meter.used(), 0);
            let aggregate = meter.peak_for("Aggregate");
            let ids = meter.peak_for("IdSpace");
            assert!(aggregate > 0, "Aggregate is measured independently");
            if refs {
                assert!(ids > 0, "skipped ref arguments exercise dictionary lookups");
            }
            peaks.push((aggregate, ids));
        }
        assert!(peaks[1].0 <= peaks[0].0 + two_input_batches());
        assert!(peaks[1].1 <= peaks[0].1 + two_input_batches());
    }
}

#[test]
fn counts_stream_across_batch_boundaries_and_skip_the_unique_group_sort() {
    let (_dir, db) = fixture(4099, 17);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let query = "[:find (count ?e) ?g (count-distinct ?v) :where [?e :bucket ?g] \
                 [?e :item ?v] :order-by [?g]]";
    let (expected, _, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, query)
    };
    let (rows, counters, _) = collect(&db, query);
    assert!(
        rows == expected,
        "identical ordered group results and projection"
    );
    assert!(ordered(&counters), "one-group aggregation selected");
    super::aggregate::ORDERED_MAX_GROUPS.with(|n| assert_eq!(n.get(), 1));
    assert!(
        counters.iter().all(|c| c.op != "Sort"),
        "the group key is unique"
    );
    assert_eq!(rows.len(), 4099usize.div_ceil(17));
    for (g, row) in rows.iter().enumerate() {
        let count = 17.min(4099 - g * 17) as i64;
        assert!(
            row == &vec![
                Value::Integer(count),
                Value::Integer(g as i64),
                Value::Integer(count.min(3))
            ],
            "count and distinct membership reset at each group"
        );
    }
}

#[test]
fn a_group_limit_stops_after_the_requested_group_and_one_input_batch() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let (_dir, db) = fixture(4099, 17);
    let (rows, counters, _) = collect(&db, "[:find ?g (count ?e) :where [?e :bucket ?g] :limit 1]");
    assert!(rows == vec![vec![Value::Integer(0), Value::Integer(17)]]);
    assert!(
        ordered(&counters),
        "group adjacency is enough without a query Sort"
    );
    let scan = counters.iter().find(|c| c.op == "Scan").unwrap();
    assert!(
        scan.rows_in <= (17 + BATCH_ROWS) as u64,
        "do not drain all groups"
    );
    let (rows, counters, _) = collect(
        &db,
        "[:find ?g (count-distinct ?v) :where [?e :bucket ?g] [?e :item ?v] :order-by [?g] :offset 2 :limit 3]",
    );
    assert!(ordered(&counters));
    assert!(
        rows == (2..5)
            .map(|g| vec![Value::Integer(g), Value::Integer(3)])
            .collect::<Vec<_>>()
    );
    let aggregate = counters
        .iter()
        .find(|c| c.op == "AggregateOrdered")
        .unwrap();
    assert!(
        aggregate.rows_in <= (5 * 17 + BATCH_ROWS) as u64,
        "a probe join still stops at group demand plus one child batch"
    );
}

#[test]
fn unproved_hidden_groups_multiple_keys_and_arguments_keep_the_hash_path() {
    let (_dir, db) = fixture(64, 4);
    for query in [
        "[:find ?g (count ?e) :with ?v :where [?e :bucket ?g] [?e :item ?v]]",
        "[:find ?g ?v (count ?e) :where [?e :bucket ?g] [?e :item ?v]]",
        "[:find ?g (min ?e) :where [?e :bucket ?g] :limit 1]",
        "[:find ?g (sum-distinct ?v) :where [?e :bucket ?g] [(+ ?g 0.5) ?v] :limit 1]",
    ] {
        let (_, counters, _) = collect(&db, query);
        assert!(
            !ordered(&counters),
            "unproved aggregate scope retains its original strategy"
        );
    }
}

#[test]
fn descending_or_aggregate_first_orders_keep_sort_after_streaming_groups() {
    let (_dir, db) = fixture(67, 4);
    for query in [
        "[:find ?g (count ?e) :where [?e :bucket ?g] :order-by [?g :desc] :limit 3]",
        "[:find ?g (count ?e) :where [?e :bucket ?g] :order-by [(count ?e)] :limit 3]",
    ] {
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, query)
        };
        let (actual, counters, _) = collect(&db, query);
        assert!(ordered(&counters));
        assert!(counters.iter().any(|c| c.op == "Sort"));
        assert!(actual == expected);
    }
}

fn two_input_batches() -> usize {
    use super::cell::{Cell, Scalar};
    use super::meter::cell_bytes;
    2 * (std::mem::size_of::<super::batch::Batch>()
        + 2 * std::mem::size_of::<Vec<Cell>>()
        + BATCH_ROWS * (cell_bytes(&Cell::Eid(1)) + cell_bytes(&Cell::Val(Scalar::Int(0)))))
}

#[test]
fn group_state_does_not_grow_with_the_number_of_finished_groups() {
    let mut peaks = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, 1);
        let (rows, counters, peak) =
            collect(&db, "[:find ?g (count-distinct ?e) :where [?e :bucket ?g]]");
        assert_eq!(rows.len(), n);
        assert!(ordered(&counters));
        peaks.push(peak);
    }
    assert!(
        peaks[1] <= peaks[0] + two_input_batches(),
        "finished groups are not retained"
    );
}

#[test]
fn a_single_distinct_group_remains_an_explicit_linear_memory_control() {
    let mut peaks = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, n);
        let (rows, counters, peak) =
            collect(&db, "[:find ?g (count-distinct ?e) :where [?e :bucket ?g]]");
        assert!(rows == vec![vec![Value::Integer(0), Value::Integer(n as i64)]]);
        assert!(ordered(&counters));
        peaks.push(peak);
    }
    assert!(
        peaks[1] > 4 * peaks[0],
        "members within one group still occupy memory"
    );
}

#[test]
fn a_proved_range_with_no_live_rows_emits_no_group() {
    let (_dir, db) = fixture(1, 1);
    db.execute("(retract [[:e0 :bucket 0]])").unwrap();
    db.checkpoint().unwrap();
    let (rows, counters, _) = collect(&db, "[:find ?g (count ?e) :where [?e :bucket ?g]]");
    assert!(rows.is_empty());
    assert!(
        ordered(&counters),
        "raw integer history proves a range even when its rows were retracted"
    );
}

#[test]
fn fallible_finishes_keep_later_group_errors_and_dropped_group_precedence() {
    let (_dir, db) = fixture(4096, 16);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    db.execute("(transact [[:e4095 :item \"bad\"]])").unwrap();
    db.checkpoint().unwrap();
    db.try_register_aggregate(
        "checked",
        || Ok(false),
        |bad: &mut bool, v| {
            *bad |= matches!(v, Value::String(_));
            Ok(())
        },
        |bad: &bool, _| {
            if *bad {
                Err(crate::UdfError::new("late value"))
            } else {
                Ok(Value::Integer(0))
            }
        },
    )
    .unwrap();
    for func in ["sum", "sum-distinct", "min", "max"] {
        for tail in [":limit 1", ":order-by [?g] :offset 1 :limit 1"] {
            let q = format!(
                "(query [:find ?g ({func} ?v) (checked ?v) :where [?e :bucket ?g] [?e :item ?v] {tail}])"
            );
            let mut cursor = db.query(&q).unwrap();
            assert!(!ordered(&cursor.stream_mut().unwrap().counters()));
            let err = cursor
                .next_batch(1)
                .expect_err("late finalizer must fail before any answer");
            assert_eq!(err.code(), "QRY-014");
        }
    }
    db.execute("(transact [[:drop :bucket 999] [:drop :empty nil] [:drop :bad \"bad\"]])")
        .unwrap();
    db.checkpoint().unwrap();
    let (rows, counters, _) = collect(
        &db,
        "[:find ?g (min ?n) (checked ?v) :where [?e :bucket ?g] [?e :empty ?n] [?e :bad ?v] :limit 1]",
    );
    assert!(
        rows.is_empty(),
        "an earlier empty min drops the group before its failing finalizer"
    );
    assert!(!ordered(&counters));
    let err = db.execute("(query [:find ?g (checked ?v) (min ?n) :where [?e :bucket ?g] [?e :empty ?n] [?e :bad ?v] :limit 1])").expect_err("find order chooses the first aggregate error");
    assert_eq!(err.code(), "QRY-014");
}

#[test]
fn late_callbacks_and_collect_set_failures_cannot_be_hidden_by_group_limit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_dir, db) = fixture(4096, 16);
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
    for aggregate in ["(count ?e)", "(avg ?g)"] {
        calls.store(0, Ordering::SeqCst);
        let err = db
            .execute(&format!(
                "(query [:find ?g {aggregate} :where [?e :bucket ?g] [(= (late? ?g) true)] :limit 1])"
            ))
            .expect_err("callback must be reached");
        assert_eq!(err.code(), "QRY-014");
        assert_eq!(calls.load(Ordering::SeqCst), 1500);
    }
    db.execute("(transact [[:e4095 :nested #{1}]])").unwrap();
    db.checkpoint().unwrap();
    let err = db.execute("(query [:find ?g (count ?e) :where [?e :bucket ?g] [(collect-set ?e :nested) ?s] :limit 1])").expect_err("late nested set must be reached");
    assert_eq!(err.code(), "QRY-015");
}

#[test]
fn small_budget_succeeds_for_many_finished_groups_and_fallback_still_refuses() {
    let (dir, db) = fixture(8192, 1);
    drop(db);
    let db = crate::OpenOptions::new()
        .query_memory_limit(128 << 10)
        .path(dir.path().join("groups.graph"))
        .open()
        .unwrap();
    let query = "[:find ?g (count ?e) :where [?e :bucket ?g]]";
    let (rows, counters, _) = collect(&db, query);
    assert_eq!(rows.len(), 8192);
    assert!(ordered(&counters));
    let _off = super::order::set_enabled(false);
    let err = db
        .execute(&format!("(query {query})"))
        .expect_err("all groups exceed this budget");
    assert_eq!(err.code(), "QRY-012");
}

#[test]
fn pending_scope_and_prepared_queries_recompute_each_snapshot_proof() {
    let (_dir, db) = fixture(64, 4);
    let query = "[:find ?g (count ?e) :where [?e :bucket ?g]]";
    let prepared = db.prepare(&format!("(query {query})")).unwrap();
    assert!(ordered(&collect(&db, query).1));
    db.execute("(transact [[:outside :other 1]])").unwrap();
    assert!(
        ordered(&collect(&db, query).1),
        "unrelated pending facts allow the proof"
    );
    db.execute("(transact [[:new :bucket -1]])").unwrap();
    let (expected, counters, _) = collect(&db, query);
    assert!(
        !ordered(&counters),
        "in-range pending facts keep full grouping"
    );
    let crate::QueryResult::QueryResults { results, .. } = prepared.execute(&[]).unwrap() else {
        panic!("rows");
    };
    assert!(results == expected);
    db.checkpoint().unwrap();
    let (actual, counters, _) = collect(&db, query);
    assert!(ordered(&counters));
    assert!(actual == expected);
    let crate::QueryResult::QueryResults { results, .. } = prepared.execute(&[]).unwrap() else {
        panic!("rows");
    };
    assert!(results == expected);
}

/// Query-only meter diagnostic over separately built committed fixtures.
/// The bench driver supplies the path/row count; never builds a fixture here.
#[test]
#[ignore]
fn committed_group_memory_probe() {
    let path = std::env::var("MINIGRAF_GROUP_FIXTURE").expect("fixture path");
    let n: usize = std::env::var("MINIGRAF_GROUP_ROWS")
        .unwrap()
        .parse()
        .unwrap();
    let group_size: usize = std::env::var("MINIGRAF_GROUP_SIZE")
        .unwrap()
        .parse()
        .unwrap();
    let db = crate::OpenOptions::new()
        .read_only(true)
        .path(path)
        .open()
        .unwrap();
    for (mode, func, limited) in [
        ("count", "count", false),
        ("distinct", "count-distinct", false),
        ("count_limit", "count", true),
        ("distinct_limit", "count-distinct", true),
    ] {
        let query = format!(
            "(query [:find ?g ({func} ?e) :where [?e :group ?g] :order-by [?g] {}])",
            if limited { ":limit 10" } else { "" }
        );
        let mut cursor = db.query(&query).unwrap();
        let stream = cursor.stream_mut().unwrap();
        assert!(ordered(&stream.counters()));
        let meter = stream.meter().clone();
        let mut rows = 0;
        while let Some(batch) = stream.next_rows().unwrap() {
            rows += batch.len();
        }
        let expected = n.div_ceil(group_size);
        assert_eq!(rows, if limited { 10.min(expected) } else { expected });
        assert_eq!(meter.used(), 0);
        let highwater = super::aggregate::ORDERED_MAX_GROUPS.with(|n| n.get());
        assert_eq!(highwater, 1);
        let scan = stream
            .counters()
            .into_iter()
            .find(|c| c.op == "Scan")
            .unwrap();
        if limited {
            assert!(scan.rows_in <= (10 * group_size + BATCH_ROWS) as u64);
        }
        println!(
            "{{\"mode\":\"{mode}\",\"facts\":{n},\"group_size\":{group_size},\"rows\":{rows},\"meter_peak_bytes\":{},\"active_groups\":{highwater},\"scan_rows\":{}}}",
            meter.peak(),
            scan.rows_in
        );
    }
}

#[test]
fn registered_aggregate_keeps_full_drain_under_group_limit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_dir, db) = fixture(4096, 16);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    db.try_register_aggregate(
        "late-count",
        || Ok(0i64),
        move |acc: &mut i64, _| {
            *acc += 1;
            if seen.fetch_add(1, Ordering::SeqCst) == 1499 {
                Err(crate::UdfError::new("late aggregate callback"))
            } else {
                Ok(())
            }
        },
        |acc: &i64, _| Ok(Value::Integer(*acc)),
    )
    .unwrap();
    let err = db
        .execute("(query [:find ?g (late-count ?e) :where [?e :bucket ?g] :limit 1])")
        .expect_err("must reach the aggregate callback");
    assert_eq!(err.code(), "QRY-014");
    assert_eq!(calls.load(Ordering::SeqCst), 1500);
}

#[test]
fn a_large_transaction_uses_its_working_types_for_group_sort_proofs() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(dir.path().join("working.graph"))
        .open()
        .unwrap();
    db.execute("(transact [[:a :bucket 0] [:b :bucket 2]])")
        .unwrap();
    db.checkpoint().unwrap();
    let query = "(query [:find ?g (count ?e) :where [?e :bucket ?g] :order-by [?g]])";
    let mut staged = db.begin_write().unwrap();
    staged.execute("(transact [[:staged :bucket -1]])").unwrap();
    let before = super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
    staged.execute(query).unwrap();
    assert_eq!(
        super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
        before,
        "staged source stays materialized"
    );
    drop(staged);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:f :bucket -3.5] [:x :other 1] [:y :other 2]])")
        .unwrap();
    let before = super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
    let crate::QueryResult::QueryResults { results, .. } = tx.execute(query).unwrap() else {
        panic!("rows");
    };
    assert!(
        results
            == vec![
                vec![Value::Float(-3.5), Value::Integer(1)],
                vec![Value::Integer(0), Value::Integer(1)],
                vec![Value::Integer(2), Value::Integer(1)]
            ],
        "mixed working roots require numeric Sort after streaming groups"
    );
    assert_eq!(
        super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
        before + 1,
        "working grouping is actually streamed"
    );
    let err = tx.execute("(transact [[:bad :bucket \"rolled back\"] [:dup :other 1 {:valid-from \"2020-01-01\"}] [:fill :other 2] [:dup :other 1 {:valid-from \"2021-01-01\"}]])").expect_err("one triple with two windows");
    assert_eq!(err.code(), "API-011");
    let crate::QueryResult::QueryResults { results: again, .. } = tx.execute(query).unwrap() else {
        panic!("rows");
    };
    assert!(results == again);
    assert_eq!(
        super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
        before + 2,
        "rejected statement restores actual proof roots"
    );
    drop(tx);
    assert!(ordered(
        &collect(
            &db,
            "[:find ?g (count ?e) :where [?e :bucket ?g] :order-by [?g]]"
        )
        .1
    ));
}

#[test]
fn early_cursor_close_drop_and_distinct_budget_errors_release_all_state() {
    let (dir, db) = fixture(4096, 16);
    for (aggregate, close) in [
        ("(count-distinct ?e)", false),
        ("(count-distinct ?e)", true),
        ("(avg ?g)", false),
        ("(avg ?g)", true),
    ] {
        let mut cursor = db
            .query(&format!(
                "(query [:find ?g {aggregate} :where [?e :bucket ?g]])"
            ))
            .unwrap();
        assert!(ordered(&cursor.stream_mut().unwrap().counters()));
        let meter = cursor.stream_mut().unwrap().meter().clone();
        assert!(cursor.next_batch(1).unwrap().is_some());
        assert!(meter.used() > 0);
        if close {
            cursor.close();
        } else {
            drop(cursor);
        }
        assert_eq!(meter.used(), 0);
    }
    drop(db);
    let db = crate::OpenOptions::new()
        .query_memory_limit(1024)
        .path(dir.path().join("groups.graph"))
        .open()
        .unwrap();
    let mut cursor = db
        .query("(query [:find ?g (count-distinct ?e) :where [?e :bucket ?g]])")
        .unwrap();
    let stream = cursor.stream_mut().unwrap();
    assert!(ordered(&stream.counters()));
    let meter = stream.meter().clone();
    let err = stream
        .next_rows()
        .expect_err("one distinct group exceeds this budget");
    assert_eq!(crate::MinigrafError::from(err).code(), "QRY-012");
    assert_eq!(meter.used(), 0);
}

#[test]
fn distinct_arguments_keep_all_value_kinds_and_group_pagination() {
    let (_dir, db) = fixture(64, 4);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let long = "x".repeat(100);
    db.execute(&format!("(transact [[:e0 :arg :k] [:e1 :arg :k] [:e2 :arg #uuid \"00000000-0000-0000-0000-000000000001\"] [:e3 :arg nil] [:e4 :arg \"short\"] [:e5 :arg \"{long}\"] [:e6 :arg \"{long}\"] [:e7 :arg #{{1 :k}}] [:e8 :arg #{{:k 1}}] [:e9 :arg false] [:e10 :arg -0.0] [:e11 :arg 0.0]])")).unwrap();
    db.checkpoint().unwrap();
    for tail in [
        "",
        ":order-by [?g] :offset 1 :limit 2",
        ":distinct :order-by [?g] :offset 1 :limit 2",
    ] {
        let q = format!(
            "[:find ?g (count ?v) (count-distinct ?v) :where [?e :bucket ?g] [?e :arg ?v] {tail}]"
        );
        let (expected, _, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        let (actual, counters, _) = collect(&db, &q);
        assert!(ordered(&counters));
        assert!(
            actual == expected,
            "count null handling and distinct values match fallback"
        );
        if !tail.is_empty() {
            let aggregate = counters
                .iter()
                .find(|c| c.op == "AggregateOrdered")
                .unwrap();
            assert!(
                aggregate.rows_in <= (3 * 4 + BATCH_ROWS) as u64,
                "group demand plus one child batch"
            );
        }
    }
}

#[test]
fn a_group_spanning_many_probe_batches_does_not_retain_all_dict_lookups() {
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let mut peaks = Vec::new();
    for n in [1024, 16384] {
        let (_dir, db) = fixture(n, n);
        let (rows, counters, peak) = collect(
            &db,
            "[:find ?g (count ?e) (count-distinct ?v) :where [?e :bucket ?g] [?e :item ?v]]",
        );
        assert!(ordered(&counters));
        assert!(
            rows == vec![vec![
                Value::Integer(0),
                Value::Integer(n as i64),
                Value::Integer(3)
            ]]
        );
        peaks.push(peak);
    }
    assert!(
        peaks[1] <= peaks[0] + two_input_batches(),
        "internal child pulls must evict committed entity lookups within one unfinished group"
    );
}

/// #781 holder measurements over a prebuilt committed fixture; result batches
/// are dropped as they are checked, never retained by the measuring process.
#[test]
#[ignore = "requires MINIGRAF_GROUP_FIXTURE/ROWS/SIZE; run separately from fixture creation"]
fn generalized_committed_memory_probe() {
    let path = std::env::var("MINIGRAF_GROUP_FIXTURE").expect("fixture path");
    let n: usize = std::env::var("MINIGRAF_GROUP_ROWS")
        .unwrap()
        .parse()
        .unwrap();
    let size: usize = std::env::var("MINIGRAF_GROUP_SIZE")
        .unwrap()
        .parse()
        .unwrap();
    let optimized = std::env::var("MINIGRAF_GROUP_FULL").is_err();
    let _mode = super::order::set_enabled(optimized);
    let db = crate::OpenOptions::new()
        .read_only(true)
        .path(path)
        .open()
        .unwrap();
    for (mode, find, with, extra, order) in [
        ("avg", "?g (avg ?g)", "", "", "?g"),
        ("sum", "?g (sum ?g)", "", "", "?g"),
        ("sum-float", "?g (sum ?v)", "", "[(+ ?g 0.5) ?v]", "?g"),
        ("extrema", "?g (min ?g) (max ?g)", "", "", "?g"),
        ("sum-distinct", "?g (sum-distinct ?g)", "", "", "?g"),
        ("avg-float", "?g (avg ?v)", "", "[(+ ?g 0.5) ?v]", "?g"),
        ("tuple-avg", "?k ?g (avg ?g)", "", "[(+ 1 1) ?k]", "?k ?g"),
        ("hidden-avg", "(avg ?g)", ":with ?g", "", ""),
        (
            "tuple-count",
            "?k ?g (count ?e)",
            "",
            "[(+ 1 1) ?k]",
            "?k ?g",
        ),
        ("tuple-distinct", "?g ?g (count-distinct ?e)", "", "", "?g"),
    ] {
        for limited in [false, true] {
            let offset = if limited && !order.is_empty() { 2 } else { 0 };
            let pagination = if limited {
                format!(":offset {offset} :limit 10")
            } else {
                String::new()
            };
            // An offset of zero still requires :order-by in the grammar.
            let pagination = if limited && order.is_empty() {
                ":limit 10".to_owned()
            } else {
                pagination
            };
            let order = if order.is_empty() {
                String::new()
            } else {
                format!(":order-by [{order}]")
            };
            let query = format!(
                "(query [:find {find} {with} :where [?e :group ?g] {extra} {order} {}])",
                pagination
            );
            let mut cursor = db.query(&query).unwrap();
            let stream = cursor.stream_mut().unwrap();
            assert_eq!(ordered(&stream.counters()), optimized);
            let meter = stream.meter().clone();
            let mut rows = 0;
            while let Some(batch) = stream.next_rows().unwrap() {
                for row in &batch {
                    let g = rows + offset;
                    let count = Value::Integer(size.min(n - g * size) as i64);
                    let key = Value::Integer(g as i64);
                    let avg = Value::Float(g as f64);
                    let expected = match mode {
                        "avg" => vec![key, avg],
                        "sum" => vec![key, Value::Integer((g * size.min(n - g * size)) as i64)],
                        "sum-float" => vec![
                            key,
                            Value::Float((g as f64 + 0.5) * size.min(n - g * size) as f64),
                        ],
                        "extrema" => vec![key.clone(), key.clone(), key],
                        "sum-distinct" => vec![key.clone(), key],
                        "avg-float" => vec![key, Value::Float(g as f64 + 0.5)],
                        "tuple-avg" => vec![Value::Integer(2), key, avg],
                        "hidden-avg" => vec![avg],
                        "tuple-count" => vec![Value::Integer(2), key, count],
                        "tuple-distinct" => vec![key.clone(), key, count],
                        _ => unreachable!(),
                    };
                    assert!(row == &expected);
                    rows += 1;
                }
            }
            assert_eq!(
                rows,
                if limited {
                    n.div_ceil(size).saturating_sub(offset).min(10)
                } else {
                    n.div_ceil(size)
                }
            );
            assert_eq!(meter.used(), 0);
            let input = stream
                .counters()
                .iter()
                .find(|c| c.op.starts_with("Aggregate"))
                .unwrap()
                .rows_in;
            if optimized && limited {
                assert!(input <= (12 * size + BATCH_ROWS) as u64);
            }
            println!(
                "{{\"mode\":\"{mode}\",\"facts\":{n},\"limited\":{limited},\"optimized\":{optimized},\"aggregate_peak\":{},\"idspace_peak\":{},\"meter_peak\":{},\"input_rows\":{input}}}",
                meter.peak_for("Aggregate"),
                meter.peak_for("IdSpace"),
                meter.peak()
            );
        }
    }
}

#[test]
fn certified_remaining_builtins_stream_scalar_probe_arguments() {
    let (_dir, db) = fixture(4101, 3);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let q = "[:find ?k ?g (sum ?v) (min ?v) (max ?v) (sum-distinct ?v) :where [?e :bucket ?g] [?e :item ?v] [(+ 1 1) ?k] :order-by [?k ?g] :offset 2 :limit 3]";
    let expected = {
        let _off = super::order::set_enabled(false);
        collect(&db, q).0
    };
    let (rows, counters, _) = collect(&db, q);
    assert!(ordered(&counters), "all four descriptors are certified");
    assert!(rows == expected);
    assert!(
        rows == (2..5)
            .map(|g| vec![
                Value::Integer(2),
                Value::Integer(g),
                Value::Integer(3),
                Value::Integer(0),
                Value::Integer(2),
                Value::Integer(3)
            ])
            .collect::<Vec<_>>()
    );
    assert!(counters.iter().all(|c| c.op != "Sort"));
    assert!(
        counters
            .iter()
            .find(|c| c.op == "AggregateOrdered")
            .unwrap()
            .rows_in
            <= (15 + BATCH_ROWS) as u64
    );
}

#[test]
fn certified_sum_skips_arbitrary_values_without_an_argument_domain_proof() {
    let (_dir, db) = fixture(4096, 16);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    db.execute("(transact [[:e0 :item nil] [:e1 :item true] [:e2 :item :tag] [:e3 :item \"text\"] [:e4 :item #{1 2}]])").unwrap();
    db.checkpoint().unwrap();
    for q in [
        "[:find ?g (sum ?v) (avg ?v) :where [?e :bucket ?g] [?e :item ?v]]",
        "[:find ?g (sum ?v) :where [?e :bucket ?g] [(+ ?g 0.5) ?v] :order-by [?g] :limit 3]",
    ] {
        let expected = {
            let _off = super::order::set_enabled(false);
            collect(&db, q).0
        };
        let (rows, counters, _) = collect(&db, q);
        assert!(ordered(&counters), "sum needs no argument type certificate");
        assert!(rows == expected);
    }
}

#[test]
fn certified_extrema_accept_mixed_scalars_and_preserve_dropped_groups() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("drops.graph")).unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let facts = (0..4100)
        .map(|i| {
            format!(
                "[:e{i} :bucket {i}] [:e{i} :item {}]",
                if i % 2 == 0 {
                    "nil".to_owned()
                } else {
                    i.to_string()
                }
            )
        })
        .collect::<String>();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let q = "[:find ?g (min ?v) (max ?v) (avg ?v) (count ?e) :where [?e :bucket ?g] [?e :item ?v] :order-by [?g] :offset 2 :limit 3]";
    let (rows, counters, _) = collect(&db, q);
    assert!(ordered(&counters));
    assert!(
        rows == [5, 7, 9]
            .into_iter()
            .map(|g| vec![
                Value::Integer(g),
                Value::Integer(g),
                Value::Integer(g),
                Value::Float(g as f64),
                Value::Integer(1)
            ])
            .collect::<Vec<_>>()
    );
    db.execute("(transact [[:e1 :item 1.5] [:e3 :item true]])")
        .unwrap();
    db.checkpoint().unwrap();
    let expected = {
        let _off = super::order::set_enabled(false);
        collect(&db, q).0
    };
    let (rows, counters, _) = collect(&db, q);
    assert!(
        ordered(&counters),
        "mixed fixed scalars are safe extrema payloads"
    );
    assert!(rows == expected);
    let (empty, counters, _) = collect(
        &db,
        "[:find ?g (min ?v) (count ?e) :where [?e :bucket ?g] [?e :item ?v] [(nil? ?v)] :limit 1]",
    );
    assert!(ordered(&counters));
    assert!(
        empty.is_empty(),
        "observed dropped groups are not empty-input zero rows"
    );
}

#[test]
fn certified_argument_proofs_are_snapshot_bound_and_include_raw_history() {
    let (_dir, db) = fixture(64, 4);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let q = "[:find ?g (min ?v) (sum-distinct ?v) :where [?e :bucket ?g] [?e :item ?v]]";
    assert!(ordered(&collect(&db, q).1));
    db.execute("(transact [[:elsewhere :unrelated 1]])")
        .unwrap();
    assert!(ordered(&collect(&db, q).1));
    db.execute("(transact [[:elsewhere :item nil]])").unwrap();
    assert!(
        !ordered(&collect(&db, q).1),
        "argument pending state prevents a stale certificate"
    );
    db.checkpoint().unwrap();
    assert!(ordered(&collect(&db, q).1), "nil is admitted for arguments");
    db.execute("(transact [[:outside :item 0.5]])").unwrap();
    db.checkpoint().unwrap();
    assert!(
        !ordered(&collect(&db, q).1),
        "float distinct sum remains materialized"
    );
    db.execute("(retract [[:outside :item 0.5]])").unwrap();
    db.checkpoint().unwrap();
    assert!(
        !ordered(&collect(&db, q).1),
        "retracted raw kinds still invalidate the proof"
    );
    let min = "[:find ?g (min ?v) :where [?e :bucket ?g] [?e :item ?v]]";
    assert!(ordered(&collect(&db, min).1));
    db.execute("(transact {:valid-from \"2999-01-01\"} [[:outside :item \"future\"]])")
        .unwrap();
    db.checkpoint().unwrap();
    assert!(
        !ordered(&collect(&db, min).1),
        "future raw payloads are not ignored by proof reads"
    );
}

#[test]
fn certified_unsupported_argument_origins_and_payloads_keep_full_input() {
    let (_dir, db) = fixture(64, 4);
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for q in [
        "[:find ?g (min ?e) :where [?e :bucket ?g]]",
        "[:find ?g (max ?v) :where [?e :bucket ?g] [(+ ?g 1) ?v]]",
        "[:find ?g (sum-distinct ?v) :where [?e :bucket ?g] [(+ ?g 1) ?v]]",
        "[:find ?g (sum-distinct ?v) :where [?e :bucket ?g] [(+ 1.0 1.0) ?v]]",
        "[:find ?g (min ?v) :where [?e :bucket ?g] [?e ?a ?v]]",
    ] {
        let expected = {
            let _off = super::order::set_enabled(false);
            collect(&db, q).0
        };
        let (rows, counters, _) = collect(&db, q);
        assert!(!ordered(&counters));
        assert!(rows == expected);
    }
    for (expr, integer) in [
        ("(= 1 1)", false),
        ("(+ 1 1)", true),
        ("(+ 1.0 1.0)", false),
    ] {
        for func in ["min", "sum-distinct"] {
            let q = format!("[:find ?g ({func} ?v) :where [?e :bucket ?g] [{expr} ?v]]");
            let (_, counters, _) = collect(&db, &q);
            assert_eq!(
                ordered(&counters),
                func == "min" || integer,
                "constant argument kind controls eligibility"
            );
        }
    }
    db.execute("(transact [[:e0 :arg \"payload\"]])").unwrap();
    db.checkpoint().unwrap();
    assert!(!ordered(
        &collect(
            &db,
            "[:find ?g (min ?v) :where [?e :bucket ?g] [?e :arg ?v]]"
        )
        .1
    ));
}

#[test]
fn certified_working_argument_roots_and_staged_values_choose_fresh_strategies() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(dir.path().join("working-args.graph"))
        .open()
        .unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    db.execute("(transact [[:a :bucket 0] [:b :bucket 1] [:a :arg 1] [:b :arg 2]])")
        .unwrap();
    db.checkpoint().unwrap();
    let min = "(query [:find ?g (min ?v) :where [?e :bucket ?g] [?e :arg ?v]])";
    let distinct = "(query [:find ?g (sum-distinct ?v) :where [?e :bucket ?g] [?e :arg ?v]])";
    {
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:outside :arg nil]])").unwrap();
        let before = super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
        tx.execute(min).unwrap();
        assert_eq!(
            super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
            before
        );
    }
    {
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:outside :arg 0.5] [:fill :other 1] [:fill :other 2]])")
            .unwrap();
        let before = super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
        tx.execute(min).unwrap();
        tx.execute(distinct).unwrap();
        assert_eq!(
            super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
            before + 1,
            "working float kind qualifies only for scalar extrema"
        );
        tx.execute("(transact [[:outside :arg \"bad\"] [:dup :other 1 {:valid-from \"2020-01-01\"}] [:fill :other 3] [:dup :other 1 {:valid-from \"2021-01-01\"}]])").expect_err("statement is rolled back");
        tx.execute(min).unwrap();
        assert_eq!(
            super::order::OPTIMIZATIONS.with(std::cell::Cell::get),
            before + 2,
            "rejected statement restores proof roots"
        );
    }
    let mut cursor = db.query(distinct).unwrap();
    assert!(
        ordered(&cursor.counters()),
        "rollback restores integer-only argument history"
    );
    assert_eq!(cursor.next_batch(10).unwrap().unwrap().len(), 2);
}

#[test]
#[ignore = "requires a one-group fixture with one fact per transaction"]
fn certified_growing_distinct_memory_probe() {
    let n: usize = std::env::var("MINIGRAF_GROUP_ROWS")
        .unwrap()
        .parse()
        .unwrap();
    let db = crate::OpenOptions::new()
        .read_only(true)
        .path(std::env::var("MINIGRAF_GROUP_FIXTURE").unwrap())
        .open()
        .unwrap();
    for func in ["count-distinct", "sum-distinct"] {
        let q = format!(
            "(query [:find ?g ({func} ?t) :any-valid-time :where [?e :group ?g] [?e :db/tx-count ?t]])"
        );
        let mut cursor = db.query(&q).unwrap();
        let stream = cursor.stream_mut().unwrap();
        assert!(ordered(&stream.counters()));
        let meter = stream.meter().clone();
        let expected = if func == "count-distinct" {
            n
        } else {
            n * (n + 1) / 2
        };
        assert!(
            stream.next_rows().unwrap().unwrap()
                == vec![vec![Value::Integer(0), Value::Integer(expected as i64)]]
        );
        assert!(stream.next_rows().unwrap().is_none());
        assert_eq!(meter.used(), 0);
        println!(
            "{{\"mode\":\"growing-{func}\",\"facts\":{n},\"aggregate_peak\":{},\"idspace_peak\":{},\"meter_peak\":{}}}",
            meter.peak_for("Aggregate"),
            meter.peak_for("IdSpace"),
            meter.peak()
        );
    }
}

#[test]
fn certified_metadata_arguments_preserve_integer_and_nil_results() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("metadata.graph")).unwrap();
    let facts: String = (0..12)
        .map(|i| format!("[:e{i} :bucket {}] [:e{i} :item {}]", i / 3, i % 3))
        .collect();
    db.execute(&format!(
        "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [{facts}])"
    ))
    .unwrap();
    db.checkpoint().unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for (q, expected) in [
        (
            "[:find ?g (max ?t) (sum-distinct ?t) :any-valid-time :where [?e :bucket ?g] [?e :db/tx-count ?t]]",
            (0..4)
                .map(|g| vec![Value::Integer(g), Value::Integer(1), Value::Integer(1)])
                .collect::<Vec<_>>(),
        ),
        (
            "[:find ?g (max ?t) :valid-at :any-valid-time :where [?e :bucket ?g] [?e :db/valid-at ?t]]",
            vec![],
        ),
        (
            "[:find ?g (max ?t) :valid-at \"2100-01-01T00:00:00Z\" :where [?e :bucket ?g] [?e :db/valid-at ?t]]",
            (0..4)
                .map(|g| vec![Value::Integer(g), Value::Integer(4_102_444_800_000)])
                .collect(),
        ),
    ] {
        let fallback = {
            let _off = super::order::set_enabled(false);
            collect(&db, q).0
        };
        assert!(fallback == expected);
        let (rows, counters, _) = collect(&db, q);
        assert!(ordered(&counters));
        assert!(rows == expected);
    }
}
