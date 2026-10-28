//! Ordered input optimizations compared with the original materialized path.

use super::{BATCH_ROWS, Counters};
use crate::{Minigraf, Value};

fn fixture(n: usize) -> (tempfile::TempDir, Minigraf) {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("ordered.graph")).unwrap();
    let facts: String = (0..n)
        .map(|i| format!("[:e{i} :score {}]", (i * 337) % n))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    (dir, db)
}

fn collect(db: &Minigraf, query: &str) -> (Vec<Vec<Value>>, Vec<Counters>) {
    let mut cursor = db.query(&format!("(query {query})")).unwrap();
    // Cursor discards its stream at EOF; retain the stream itself so its
    // closed-operator counters can still establish how much input was read.
    let stream = cursor.stream_mut().unwrap();
    let mut rows = Vec::new();
    while let Some(batch) = stream.next_rows().unwrap() {
        rows.extend(batch);
    }
    (rows, stream.counters())
}

#[test]
fn scalar_order_omits_sort_and_restores_projection_slots() {
    let (_dir, db) = fixture(4096);
    let (rows, counters) = collect(
        &db,
        "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 10]",
    );
    let expected: Vec<_> = (0..10).map(|n| vec![Value::Integer(n)]).collect();
    assert!(rows == expected, "the original value slot is projected");
    assert!(
        counters.iter().all(|c| c.op != "Sort"),
        "a proved complete order omits Sort"
    );
}

#[test]
fn ordered_scan_limit_offset_reads_only_a_prefix() {
    let (_dir, db) = fixture(4096);
    let (rows, counters) = collect(
        &db,
        "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 10 :offset 7]",
    );
    let expected: Vec<_> = (7..17).map(|n| vec![Value::Integer(n)]).collect();
    assert!(rows == expected, "offset applies in semantic value order");
    let scan = counters.iter().find(|c| c.op == "Scan").unwrap();
    assert!(
        scan.rows_in <= (17 + BATCH_ROWS) as u64,
        "ordered Limit stops before reading all input"
    );
}

#[test]
fn uuid_tie_suffix_and_descending_order_keep_sort() {
    let (_dir, db) = fixture(32);
    for query in [
        "[:find ?e ?v :where [?e :score ?v] :order-by [?v] :limit 10]",
        "[:find ?v :where [?e :score ?v] :order-by [?v :desc] :limit 10]",
    ] {
        let (_, counters) = collect(&db, query);
        assert!(counters.iter().any(|c| c.op == "Sort"), "Sort is retained");
    }
}

fn has_sort(counters: &[Counters]) -> bool {
    counters.iter().any(|c| c.op == "Sort")
}

fn same_rows(a: &[Vec<Value>], b: &[Vec<Value>]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| match (a, b) {
                    (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
                    _ => a == b,
                })
        })
}

#[test]
fn forced_fallback_restores_sort_and_gives_the_same_ordered_list() {
    let (_dir, db) = fixture(128);
    let q = "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 10 :offset 7]";
    let (expected, fallback) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    let (actual, optimized) = collect(&db, q);
    assert!(has_sort(&fallback), "the switch forces the original Sort");
    assert!(
        !has_sort(&optimized),
        "dropping the guard restores optimization"
    );
    assert!(same_rows(&actual, &expected), "identical ordered rows");
}

#[test]
fn distinct_preserves_integer_order_but_float_canonicalization_keeps_sort() {
    let (_dir, db) = fixture(16);
    let q = "[:find ?v :where [?e :score ?v] :distinct :order-by [?v] :limit 3]";
    assert!(
        !has_sort(&collect(&db, q).1),
        "integer distinct keeps an ordered subsequence"
    );
    db.execute("(transact [[:a :f -0.0] [:b :f 0.0] [:c :f 1.0]])")
        .unwrap();
    db.checkpoint().unwrap();
    let q = "[:find ?v :where [?e :f ?v] :distinct :order-by [?v]]";
    let (actual, counters) = collect(&db, q);
    let (expected, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    assert!(
        has_sort(&counters),
        "canonicalization invalidates the float certificate"
    );
    assert!(
        same_rows(&actual, &expected),
        "canonical float bits are unchanged"
    );
}

#[test]
fn an_open_ordered_cursor_keeps_its_reader_across_checkpoint_and_reuse() {
    let (_dir, db) = fixture(32);
    let mut cursor = db
        .query("(query [:find ?v :where [?e :score ?v] :order-by [?v] :limit 10])")
        .unwrap();
    assert!(
        !has_sort(&cursor.counters()),
        "the original snapshot was certified"
    );
    for i in 1..=4 {
        db.execute(&format!("(transact [[:new{i} :score -{i}]])"))
            .unwrap();
        db.checkpoint().unwrap();
    }
    let mut rows = Vec::new();
    while let Some(batch) = cursor.next_batch(3).unwrap() {
        rows.extend(batch);
    }
    let expected: Vec<_> = (0..10).map(|n| vec![Value::Integer(n)]).collect();
    assert!(
        same_rows(&rows, &expected),
        "no rows from later generations leak in"
    );
    let (new, _) = collect(
        &db,
        "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 1]",
    );
    assert!(
        new == vec![vec![Value::Integer(-4)]],
        "new opens see the new generation"
    );
}

#[test]
fn prepared_queries_recheck_each_snapshot_and_rules_keep_original_strategy() {
    let (_dir, db) = fixture(32);
    let prepared = db
        .prepare("(query [:find ?v :where [?e :score ?v] :order-by [?v] :limit $n])")
        .unwrap();
    let bindings = [("n", crate::BindValue::Val(Value::Integer(1)))];
    let before = prepared.query(&bindings).unwrap();
    assert!(
        !has_sort(&before.counters()),
        "prepared execution is eligible"
    );
    drop(before);
    db.execute("(transact [[:new :score -1]])").unwrap();
    let mut pending = prepared.query(&bindings).unwrap();
    assert!(has_sort(&pending.counters()), "no stale prepared proof");
    assert!(pending.next_batch(1).unwrap().unwrap().rows() == [vec![Value::Integer(-1)]]);
    drop(pending);
    db.checkpoint().unwrap();
    assert!(
        !has_sort(&prepared.query(&bindings).unwrap().counters()),
        "checkpoint re-enables proof"
    );
    db.execute("(rule [(scored ?e ?v) [?e :score ?v]])")
        .unwrap();
    let (_, counters) = collect(
        &db,
        "[:find ?v :where (scored ?e ?v) :order-by [?v] :limit 3]",
    );
    assert!(has_sort(&counters), "rule construction is unchanged");
}

#[test]
fn snapshot_pending_existence_uses_the_captured_prefix_and_staged_view() {
    use crate::graph::storage::FactStorage;
    use crate::graph::types::Fact;
    let storage = FactStorage::new();
    let e = uuid::Uuid::from_u128(2);
    let other = uuid::Uuid::from_u128(1);
    let empty = storage.snapshot().unwrap();
    storage
        .transact(vec![(e, ":a".into(), Value::Integer(1))], None)
        .unwrap();
    assert!(
        !empty.has_pending(":a", None).unwrap(),
        "an old empty view stays empty"
    );
    let snapshot = storage.snapshot().unwrap();
    storage
        .transact(
            vec![
                (other, ":new".into(), Value::Integer(2)),
                (other, ":a".into(), Value::Integer(3)),
            ],
            None,
        )
        .unwrap();
    assert!(snapshot.has_pending(":a", None).unwrap());
    assert!(snapshot.has_pending(":a", Some(&e)).unwrap());
    assert!(snapshot.has_pending(":a", None).unwrap());
    assert!(snapshot.has_pending(":a", Some(&e)).unwrap());
    assert!(!snapshot.has_pending(":a", Some(&other)).unwrap());
    assert!(!snapshot.has_pending(":new", None).unwrap());
    assert!(!snapshot.has_pending(":new", Some(&e)).unwrap());
    let mut retract =
        Fact::with_valid_time(other, ":new".into(), Value::Integer(2), 20, 2, 0, i64::MAX);
    retract.asserted = false;
    let staged = snapshot.with_staged(vec![retract]);
    assert!(
        staged.has_pending(":new", None).unwrap(),
        "staged retractions also disqualify"
    );
    assert!(staged.has_pending(":new", Some(&other)).unwrap());
}

#[test]
fn unrelated_pending_facts_do_not_disable_order_but_in_range_records_do() {
    let (_dir, db) = fixture(32);
    let q = "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 3]";
    db.execute("(transact [[:unrelated :name \"pending\"]])")
        .unwrap();
    assert!(!has_sort(&collect(&db, q).1), "unrelated pending range");
    db.execute("(transact [[:new :score -1]])").unwrap();
    let (rows, counters) = collect(&db, q);
    assert!(has_sort(&counters), "a second input run needs Sort");
    assert!(
        rows == vec![
            vec![Value::Integer(-1)],
            vec![Value::Integer(0)],
            vec![Value::Integer(1)]
        ]
    );
    db.checkpoint().unwrap();
    assert!(
        !has_sort(&collect(&db, q).1),
        "each open proves its own snapshot"
    );
    db.execute("(retract [[:new :score -1]])").unwrap();
    assert!(
        has_sort(&collect(&db, q).1),
        "pending retractions also disqualify"
    );
}

#[test]
fn committed_history_of_another_kind_disqualifies_the_entire_range() {
    let (_dir, db) = fixture(32);
    db.execute("(transact [[:history :score \"old\"]])")
        .unwrap();
    db.execute("(retract [[:history :score \"old\"]])").unwrap();
    db.checkpoint().unwrap();
    let (_, counters) = collect(
        &db,
        "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 3]",
    );
    assert!(
        has_sort(&counters),
        "dead historical keys still prevent a kind proof"
    );
}

#[test]
fn bool_float_and_single_entity_value_ranges_match_the_original_sort() {
    let (_dir, db) = fixture(16);
    db.execute(
        "(transact [[:a :f -0.0] [:b :f 0.0] [:c :f -1.5] [:d :f 2.0] \
        [:a :b true] [:b :b false] [:one :s 8] [:one :s -2] [:one :s 3]])",
    )
    .unwrap();
    db.checkpoint().unwrap();
    for q in [
        "[:find ?v :where [?e :f ?v] :order-by [?v]]",
        "[:find ?v :where [?e :b ?v] :order-by [?v]]",
        "[:find ?v :where [:one :s ?v] :order-by [?v]]",
    ] {
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, q)
        };
        let (actual, counters) = collect(&db, q);
        assert!(!has_sort(&counters), "supported scalar range");
        assert!(same_rows(&actual, &expected), "same values and float bits");
    }
}

#[test]
fn mixed_numeric_kinds_strings_keywords_refs_sets_and_nil_keep_sort() {
    let (_dir, db) = fixture(8);
    db.execute(
        "(transact [[:a :mixed 2] [:b :mixed 1.5] [:a :str \"b\"] [:b :str \"a\"] \
        [:a :kw :z] [:b :kw :a] [:a :ref #uuid \"00000000-0000-0000-0000-000000000002\"] \
        [:b :ref #uuid \"00000000-0000-0000-0000-000000000001\"] \
        [:a :set #{1 2}] [:b :set #{1}] [:a :nil nil]])",
    )
    .unwrap();
    db.checkpoint().unwrap();
    for a in ["mixed", "str", "kw", "ref", "set", "nil"] {
        let q = format!("[:find ?v :where [?e :{a} ?v] :order-by [?v]]");
        let (actual, counters) = collect(&db, &q);
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        assert!(has_sort(&counters), "uncertified value kind");
        assert!(
            same_rows(&actual, &expected),
            "fallback preserves full order"
        );
    }
}

#[test]
fn constant_and_repeated_projection_columns_preserve_the_complete_order() {
    let (_dir, db) = fixture(32);
    for q in [
        "[:find ?v ?v :where [?e :score ?v] :order-by [?v] :limit 3]",
        "[:find ?c ?v :where [?e :score ?v] [(+ 20 22) ?c] :order-by [?v] :limit 3]",
        "[:find ?c ?v :where [?e :score ?v] [(+ 20 22) ?c] :order-by [?c :desc ?v] :limit 3]",
    ] {
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, q)
        };
        let (actual, counters) = collect(&db, q);
        assert!(
            !has_sort(&counters),
            "constants/repetitions add no tie requirement"
        );
        assert!(
            same_rows(&actual, &expected),
            "nonidentity projection is restored"
        );
    }
}

#[test]
fn nested_and_binding_callbacks_keep_sort_and_late_errors_under_limit() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_dir, db) = fixture(4096);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    db.try_register_predicate("boom?", move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) == 1499 {
            Err(crate::UdfError::new("late failure"))
        } else {
            Ok(true)
        }
    })
    .unwrap();
    for body in [
        "[(boom? ?v)]",
        "[(= (boom? ?v) true)]",
        "[(boom? ?v) ?ok]",
        "(not [(= (boom? ?v) false)])",
    ] {
        calls.store(0, Ordering::SeqCst);
        let q = format!("(query [:find ?v :where [?e :score ?v] {body} :order-by [?v] :limit 10])");
        let err = db
            .execute(&q)
            .expect_err("Sort must reach the late callback error");
        assert_eq!(err.code(), "QRY-014");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1500,
            "no call after the failure"
        );
    }
}

#[test]
fn collect_set_data_errors_are_not_hidden_by_ordered_limit() {
    let (_dir, db) = fixture(4096);
    let bad = (0..4096).find(|i| i * 337 % 4096 == 4095).unwrap();
    db.execute(&format!("(transact [[:e{bad} :items #{{1}}]])"))
        .unwrap();
    db.checkpoint().unwrap();
    let err = db
        .execute(
            "(query [:find ?v :where [?e :score ?v] \
        [(collect-set ?e :items) ?s] :order-by [?v] :limit 10])",
        )
        .expect_err("a late nested set remains an error");
    assert_eq!(err.code(), "QRY-015");
}

#[test]
fn staged_negative_values_and_working_reader_type_changes_keep_correct_order() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(dir.path().join("large.graph"))
        .open()
        .unwrap();
    db.execute("(transact [[:a :score 0] [:b :score 2]])")
        .unwrap();
    db.checkpoint().unwrap();
    let q = "(query [:find ?v :where [?e :score ?v] :order-by [?v] :limit 1])";
    {
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:staged :score -1]])").unwrap();
        let crate::QueryResult::QueryResults { results, .. } = tx.execute(q).unwrap() else {
            panic!("rows");
        };
        assert!(
            results == vec![vec![Value::Integer(-1)]],
            "staged run is sorted with committed rows"
        );
    }
    {
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:new :score -0.5] [:x :other 1] [:y :other 2]])")
            .unwrap();
        let crate::QueryResult::QueryResults { results, .. } = tx.execute(q).unwrap() else {
            panic!("rows");
        };
        assert!(
            results == vec![vec![Value::Float(-0.5)]],
            "kind proof reads working roots, not published roots"
        );
    }
    assert!(
        !has_sort(&collect(&db, "[:find ?v :where [?e :score ?v] :order-by [?v]]").1),
        "rollback does not leave a stale type proof"
    );
}

#[test]
fn temporal_queries_match_fallback_with_duplicate_values_and_metadata() {
    let (_dir, db) = fixture(32);
    db.execute(
        "(transact {:valid-from \"2020-01-01\" :valid-to \"2021-01-01\"} \
        [[:hist1 :score -2]])",
    )
    .unwrap();
    db.execute("(transact {:valid-from \"2022-01-01\"} [[:hist2 :score -2]])")
        .unwrap();
    db.checkpoint().unwrap();
    for temporal in [
        ":as-of 1 :valid-at :any-valid-time",
        ":as-of 2 :valid-at :any-valid-time",
        ":valid-at \"2020-06-01\"",
        ":valid-at \"2022-06-01\"",
        ":valid-at :any-valid-time",
    ] {
        let q = format!("[:find ?v {temporal} :where [?e :score ?v] :order-by [?v] :limit 10]");
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        let (actual, counters) = collect(&db, &q);
        assert!(
            !has_sort(&counters),
            "time filtering preserves raw key order"
        );
        assert!(
            same_rows(&actual, &expected),
            "same temporal rows and multiplicities"
        );
    }
    for q in [
        "[:find ?v ?f :valid-at :any-valid-time :where [?e :score ?v] [?e :db/valid-from ?f] :order-by [?v ?f]]",
        "[:find ?t ?v :valid-at :any-valid-time :where [?e :score ?v] [?e :db/valid-at ?t] :order-by [?t ?v]]",
    ] {
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, q)
        };
        let (actual, _) = collect(&db, q);
        assert!(
            same_rows(&actual, &expected),
            "metadata columns and nil valid-at are preserved"
        );
    }
}

#[test]
fn successful_working_tree_proofs_are_fresh_after_each_statement_and_rollback() {
    fn values(result: crate::QueryResult) -> Vec<Vec<Value>> {
        let crate::QueryResult::QueryResults { results, .. } = result else {
            panic!("rows");
        };
        results
    }
    let proofs = || super::order::OPTIMIZATIONS.with(std::cell::Cell::get);
    let dir = tempfile::tempdir().unwrap();
    let db = crate::OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(dir.path().join("working.graph"))
        .open()
        .unwrap();
    db.execute("(transact [[:a :score 0] [:b :score 2]])")
        .unwrap();
    db.checkpoint().unwrap();
    let q = "(query [:find ?v :where [?e :score ?v] :order-by [?v] :limit 1])";
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:new :score -1] [:x :other 1] [:y :other 2]])")
        .unwrap();
    let before = proofs();
    assert!(values(tx.execute(q).unwrap()) == vec![vec![Value::Integer(-1)]]);
    assert_eq!(proofs(), before + 1, "working-tree input really omits Sort");
    tx.execute("(transact [[:newer :score -2]])").unwrap();
    assert!(values(tx.execute(q).unwrap()) == vec![vec![Value::Integer(-2)]]);
    assert_eq!(
        proofs(),
        before + 2,
        "a new query reads the new working roots"
    );
    let err = tx
        .execute(
            "(transact [[:bad :score \"rolled back\"] \
        [:dup :other 1 {:valid-from \"2020-01-01\"}] [:filler :other 2] \
        [:dup :other 1 {:valid-from \"2021-01-01\"}]])",
        )
        .expect_err("one triple, two windows");
    assert_eq!(err.code(), "API-011");
    assert!(values(tx.execute(q).unwrap()) == vec![vec![Value::Integer(-2)]]);
    assert_eq!(
        proofs(),
        before + 3,
        "rejected statement restores the proof's actual reader"
    );
    tx.execute("(transact [[:mixed :score -3.5]])").unwrap();
    assert!(values(tx.execute(q).unwrap()) == vec![vec![Value::Float(-3.5)]]);
    assert_eq!(
        proofs(),
        before + 3,
        "a changed type invalidates subsequent proofs"
    );
    drop(tx);
    assert!(values(db.execute(q).unwrap()) == vec![vec![Value::Integer(0)]]);
    assert_eq!(
        proofs(),
        before + 4,
        "rollback leaves the published integer generation eligible"
    );
}

#[test]
fn filters_entity_probes_and_anti_joins_preserve_unchanged_order_columns() {
    let (_dir, db) = fixture(32);
    let facts: String = (0..32)
        .map(|i| format!("[:e{i} :flag true] [:m{i} :match {i}]"))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    for body in [
        "[(>= ?v 7)]",
        "[?e :flag true]",
        "[?other :match ?v]",
        "(not [?e :missing true])",
    ] {
        let q = format!("[:find ?v :where [?e :score ?v] {body} :order-by [?v] :limit 10]");
        let (expected, _) = {
            let _off = super::order::set_enabled(false);
            collect(&db, &q)
        };
        let (actual, counters) = collect(&db, &q);
        assert!(
            !has_sort(&counters),
            "a preserving operator keeps the outer order"
        );
        assert!(
            same_rows(&actual, &expected),
            "same complete ordered result"
        );
    }
}

#[test]
fn expression_priority_replacement_invalidates_the_ordered_slot() {
    let (_dir, db) = fixture(32);
    let _on = crate::query::datalog::optimizer::set_reorder(true);
    // The selective score pattern runs first. The earlier expression then
    // replaces its ?v after the broad entity probe binds ?x.
    let q = "[:find ?v :where [?e ?a ?x] [(+ ?x 0) ?v] [?e :score ?v] :order-by [?v] :limit 3]";
    assert!(
        query_plan(q).steps.iter().any(|s| matches!(
            s,
            super::plan::Step::Bind(_, _, _, super::plan::Out::Replace)
        )),
        "replacement path"
    );
    let (actual, counters) = collect(&db, q);
    let (expected, _) = {
        let _off = super::order::set_enabled(false);
        collect(&db, q)
    };
    assert!(
        has_sort(&counters),
        "replacement is not certified as order-preserving"
    );
    assert!(same_rows(&actual, &expected));
}

#[test]
fn value_probe_runtime_fallback_preserves_order_only_when_it_streams_the_outer() {
    use super::plan::{JoinKind, Step};
    use crate::query::datalog::{
        functions::FunctionRegistry, parser::parse_datalog_command, types::DatalogCommand,
    };
    let (_dir, db) = fixture(32);
    let named = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b":named");
    let mut facts = String::new();
    for i in 0..32 {
        if i < 16 {
            facts.push_str(&format!("[:e{i} :friend {i}] [:m{i} :alias {i}]"));
        } else {
            facts.push_str(&format!("[:e{i} :friend #uuid \"{named}\"]"));
        }
    }
    facts.push_str("[:named-alias :alias :named]");
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let DatalogCommand::Query(q) = parse_datalog_command(
        "(query [:find ?v :where [?e :score ?v] [?e :friend ?target] \
        [?other :alias ?target] :order-by [?v]])",
    )
    .unwrap() else {
        panic!("query");
    };
    for build_inner in [true, false] {
        let mut plan = super::plan(&q).unwrap();
        let join = plan
            .steps
            .iter_mut()
            .rev()
            .find_map(|s| match s {
                Step::Join(j) if matches!(j.kind, JoinKind::ProbeValue { .. }) => Some(j),
                _ => None,
            })
            .expect("value probe");
        join.kind = JoinKind::ProbeValue {
            key: match join.kind {
                JoinKind::ProbeValue { key, .. } => key,
                _ => panic!("probe"),
            },
            build_inner,
        };
        let mut stream = super::open(
            db.snapshot().unwrap(),
            plan,
            std::sync::Arc::new(FunctionRegistry::with_builtins()),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            has_sort(&stream.counters()),
            !build_inner,
            "fallback orientation controls the proof"
        );
        let mut rows = Vec::new();
        while let Some(batch) = stream.next_rows().unwrap() {
            rows.extend(batch);
        }
        let expected: Vec<_> = (0..32).map(|v| vec![Value::Integer(v)]).collect();
        assert!(
            same_rows(&rows, &expected),
            "ambiguous ref/keyword probes still give the full ordered result"
        );
    }
}

/// Real reader with instrumentation only around optional proof reads.
struct ProofKeys<'a> {
    base: &'a dyn crate::storage::KeySource,
    calls: std::sync::atomic::AtomicUsize,
    fail_at: Option<usize>,
}

macro_rules! forward_keys {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> $ret { self.base.$name($($arg),*) })*
    };
}

impl crate::storage::KeySource for ProofKeys<'_> {
    fn any_key(&self, index: crate::storage::keys::Index, prefix: &[u8]) -> anyhow::Result<bool> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if self.fail_at == Some(call) {
            anyhow::bail!("optional proof read failed");
        }
        self.base.any_key(index, prefix)
    }

    forward_keys! {
        next_ids() -> (u64, u32);
        eid_of(e: &uuid::Uuid) -> anyhow::Result<Option<u64>>;
        iid_of(a: &str) -> anyhow::Result<Option<u32>>;
        uuid_of(e: u64) -> anyhow::Result<uuid::Uuid>;
        uuids_of(e: &[u64], visit: &mut dyn FnMut(u64, uuid::Uuid) -> anyhow::Result<()>) -> anyhow::Result<()>;
        ident_of(i: u32) -> anyhow::Result<String>;
        long_value(r: crate::storage::keys::ValueRef) -> anyhow::Result<String>;
        long_value_ref(s: &str) -> anyhow::Result<Option<crate::storage::keys::ValueRef>>;
        long_bytes(r: crate::storage::keys::ValueRef) -> anyhow::Result<Vec<u8>>;
        long_bytes_ref(s: &[u8]) -> anyhow::Result<Option<crate::storage::keys::ValueRef>>;
        tx_id_of(n: u64) -> anyhow::Result<u64>;
        tx_order() -> anyhow::Result<crate::storage::TxOrder>;
        last_tx_at(t: u64) -> anyhow::Result<u64>;
        tx_runs(t: u64, out: &mut Vec<(u64, u64)>) -> anyhow::Result<()>;
        live_keys(index: crate::storage::keys::Index, prefix: &[u8], resume: Option<&[u8]>,
            bound: &crate::storage::TxBound, max: usize, out: &mut Vec<crate::storage::keys::KeyFact>)
            -> anyhow::Result<Option<Vec<u8>>>;
    }
}

fn query_plan(query: &str) -> super::Plan {
    use crate::query::datalog::{parser::parse_datalog_command, types::DatalogCommand};
    let DatalogCommand::Query(q) = parse_datalog_command(&format!("(query {query})")).unwrap()
    else {
        panic!("query");
    };
    super::plan(&q).unwrap()
}

#[test]
fn optional_proofs_are_bounded_and_queries_without_a_usable_demand_do_no_probes() {
    use crate::query::datalog::functions::FunctionRegistry;
    let (_dir, db) = fixture(32);
    let snapshot = db.snapshot().unwrap();
    let base = snapshot.committed.as_ref().unwrap().key_source().unwrap();
    for (q, expected) in [
        (
            "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 3]",
            8,
        ),
        ("[:find ?v :where [?e :score ?v] :limit 3]", 0),
        (
            "[:find ?v :where [?e :score ?v] :order-by [?v :desc] :limit 3]",
            0,
        ),
        (
            "[:find ?v :where [?e :score ?v] :order-by [?v] :limit 0]",
            0,
        ),
        (
            "[:find ?v :where [?e :score ?v] :order-by [?v] :offset 7 :limit 0]",
            0,
        ),
        (
            "[:find ?e ?v :where [?e :score ?v] :order-by [?v] :limit 3]",
            0,
        ),
        (
            "[:find ?v :where [?e :missing ?v] :order-by [?v] :limit 3]",
            0,
        ),
        (
            "[:find ?e :where [?e :score ?e] :order-by [?e] :limit 3]",
            0,
        ),
    ] {
        let keys = ProofKeys {
            base,
            calls: 0.into(),
            fail_at: None,
        };
        let mut plan = query_plan(q);
        super::order::resolve(
            &snapshot,
            Some(&keys),
            &FunctionRegistry::with_builtins(),
            &mut plan,
        );
        assert_eq!(
            keys.calls.load(std::sync::atomic::Ordering::SeqCst),
            expected,
            "bounded optional proof reads"
        );
    }
}

#[test]
fn every_failed_optional_read_retains_the_original_sort_projection_and_index() {
    use super::plan::{Source, Step};
    use crate::query::datalog::functions::FunctionRegistry;
    let (_dir, db) = fixture(32);
    let snapshot = db.snapshot().unwrap();
    let base = snapshot.committed.as_ref().unwrap().key_source().unwrap();
    for fail_at in 1..=8 {
        let keys = ProofKeys {
            base,
            calls: 0.into(),
            fail_at: Some(fail_at),
        };
        let mut plan = query_plan("[:find ?v :where [?e :score ?v] :order-by [?v] :limit 3]");
        let original_find = plan.find.clone();
        super::order::resolve(
            &snapshot,
            Some(&keys),
            &FunctionRegistry::with_builtins(),
            &mut plan,
        );
        assert!(
            plan.steps.iter().any(|s| matches!(s, Step::Sort { .. })),
            "Sort remains"
        );
        assert_eq!(
            plan.find, original_find,
            "projection is not partially restored"
        );
        assert!(
            matches!(&plan.source, Source::Scan(s) if s.index.is_none()),
            "index choice is unchanged"
        );
        let _off = super::order::set_enabled(false);
        let mut fallback = super::open(
            snapshot.clone(),
            plan,
            std::sync::Arc::new(FunctionRegistry::with_builtins()),
            usize::MAX,
        )
        .unwrap();
        assert!(has_sort(&fallback.counters()));
        let mut rows = Vec::new();
        while let Some(batch) = fallback.next_rows().unwrap() {
            rows.extend(batch);
        }
        assert!(
            rows == vec![
                vec![Value::Integer(0)],
                vec![Value::Integer(1)],
                vec![Value::Integer(2)]
            ],
            "a failed optional proof still executes the original query"
        );
    }
}
