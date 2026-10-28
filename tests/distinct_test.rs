//! Optional distinct output (#493). Default queries still return bags.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{BindValue, Minigraf, OpenOptions, QueryResult, Value, ValueSet};

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    output(db.execute(query).expect("query succeeds"))
}

fn output(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    }
}

fn seed(db: &Minigraf) {
    db.execute("(transact [[:e1 :c 1] [:e1 :c 2] [:e2 :c 1]])")
        .unwrap();
}

fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort();
    rows
}

#[test]
fn bag_default_and_distinct_in_any_section_order() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    assert_eq!(rows(&db, "(query [:find ?e :where [?e :c ?v]])").len(), 3);
    for q in [
        "(query [:distinct :find ?e :where [?e :c ?v]])",
        "(query [:find ?e :distinct :where [?e :c ?v]])",
        "(query [:find ?e :where [?e :c ?v] :distinct])",
    ] {
        assert_eq!(rows(&db, q).len(), 2, "distinct projected entities");
    }
}

#[test]
fn distinct_keys_are_whole_output_tuples() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    assert_eq!(
        rows(&db, "(query [:find ?v :where [?e :c ?v] :distinct])").len(),
        2
    );
    assert_eq!(
        rows(&db, "(query [:find ?e ?v :where [?e :c ?v] :distinct])").len(),
        3
    );
    assert_eq!(
        rows(&db, "(query [:find ?v ?v :where [?e :c ?v] :distinct])").len(),
        2
    );
}

#[test]
fn distinct_is_a_bare_non_repeating_section() {
    let db = Minigraf::in_memory().unwrap();
    for (extra, code) in [
        (":distinct :distinct", "INT-012"),
        (":distinct true", "PRS-061"),
        (":distinct false", "PRS-061"),
        (":distinct 1", "PRS-061"),
        (":distinct $d", "PRS-061"),
        (":distinct ?e", "PRS-061"),
    ] {
        let q = format!("(query [:find ?e :where [?e :c ?v] {extra}])");
        assert_eq!(db.prepare(&q).err().expect("rejected").code(), code);
    }
}

#[test]
fn distinct_precedes_order_offset_and_limit() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    let q = "(query [:find ?v :where [?e :c ?v] :distinct :order-by [?v] :offset 1 :limit 1])";
    assert!(
        rows(&db, q) == vec![vec![Value::Integer(2)]],
        "second unique value"
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :distinct :limit 2])"
        )
        .len(),
        2
    );
    assert!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :distinct :limit 0])"
        )
        .is_empty()
    );
}

#[test]
fn prepared_execute_and_cursors_keep_distinct() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    let q =
        "(query [:find ?v :where [?e :c ?v] [(>= ?v $min)] :distinct :order-by [?v] :limit $n])";
    let prepared = db.prepare(q).unwrap();
    for (min, expected) in [(1, 2), (2, 1), (3, 0), (1, 2)] {
        let bind = [
            ("min", BindValue::Val(Value::Integer(min))),
            ("n", BindValue::Val(Value::Integer(10))),
        ];
        let answer = output(prepared.execute(&bind).unwrap());
        assert_eq!(answer.len(), expected);
        let mut cursor = prepared.query(&bind).unwrap();
        let mut pulled = Vec::new();
        while let Some(batch) = cursor.next_batch(1).unwrap() {
            pulled.extend(batch.into_rows());
        }
        assert!(pulled == answer, "prepared cursor agrees");
    }
    let mut cursor = db
        .query("(query [:find ?v :where [?e :c ?v] :distinct])")
        .unwrap();
    let mut count = 0;
    while let Some(batch) = cursor.next_batch(1).unwrap() {
        count += batch.len();
    }
    assert_eq!(count, 2);
}

#[test]
fn aggregate_inputs_and_hidden_with_groups() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:e1 :c 1] [:e1 :c 2] [:e2 :c 1] [:e2 :c 2]])")
        .unwrap();
    let q = "(query [:find (count ?v) :where [?e :c ?v] :with ?e])";
    assert_eq!(rows(&db, q).len(), 2);
    let q = "(query [:find (count ?v) :where [?e :c ?v] :with ?e :distinct])";
    assert!(
        rows(&db, q) == vec![vec![Value::Integer(2)]],
        "dedup only after aggregation"
    );
    let q = "(query [:find (count ?v) :where [?e :c ?v] :distinct])";
    assert!(
        rows(&db, q) == vec![vec![Value::Integer(4)]],
        "all bindings counted"
    );
}

#[test]
fn windows_run_before_distinct_including_partitioned_row_numbers() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    for (find, expected) in [
        ("(rank :over (:order-by ?v))", vec![1, 3]),
        ("(sum ?v :over (:order-by ?v))", vec![2, 4]),
        (
            "(row-number :over (:partition-by ?e :order-by ?v))",
            vec![1, 2],
        ),
    ] {
        let q = format!("(query [:find {find} :where [?e :c ?v] :distinct])");
        let expected: Vec<Vec<Value>> = expected
            .into_iter()
            .map(|v| vec![Value::Integer(v)])
            .collect();
        assert!(
            sorted(rows(&db, &q)) == expected,
            "window evaluated on the full bag"
        );
    }
    let q = "(query [:find ?e (count ?v) (rank :over (:order-by ?e)) :where [?e :c ?v] :distinct])";
    assert_eq!(rows(&db, q).len(), 2, "mixed aggregates and windows");
}

#[test]
fn empty_input_and_aggregate_zero_row() {
    let db = Minigraf::in_memory().unwrap();
    assert!(rows(&db, "(query [:find ?e :where [?e :missing ?v] :distinct])").is_empty());
    let q = "(query [:find (count ?v) :where [?e :missing ?v] :distinct])";
    assert!(
        rows(&db, q) == vec![vec![Value::Integer(0)]],
        "keep aggregate zero row"
    );
}

#[test]
fn joins_disjunction_and_recursive_rules() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    db.execute("(transact [[:e1 :next :e2] [:e2 :next :e3] [:e1 :tag :yes] [:e2 :tag :yes]])")
        .unwrap();
    db.execute("(rule [(reach ?a ?b) [?a :next ?b]])").unwrap();
    db.execute("(rule [(reach ?a ?b) [?a :next ?m] (reach ?m ?b)])")
        .unwrap();
    for (body, n) in [
        ("[?e :c ?v] [?e :tag ?t]", 1),
        ("(or [?e :c ?v] [?e :c ?v]) [?e :tag ?t]", 1),
        ("(reach ?e ?dest) [?e :tag ?t]", 1),
    ] {
        let q = format!("(query [:find ?t :where {body} :distinct])");
        assert_eq!(rows(&db, &q).len(), n);
    }
}

#[test]
fn distinct_is_after_temporal_filtering() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :c 1 {:valid-from \"2020-01-01\" :valid-to \"2021-01-01\"}] [:b :c 1 {:valid-from \"2022-01-01\"}]])").unwrap();
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :any-valid-time :distinct])"
        )
        .len(),
        1
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :valid-at \"2020-06-01\" :distinct])"
        )
        .len(),
        1
    );
    db.execute("(transact [[:c :c 2]])").unwrap();
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :as-of 1 :any-valid-time :distinct])"
        )
        .len(),
        1
    );
}

#[test]
fn distinct_spans_internal_batches_and_file_generations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("distinct.graph");
    let db = Minigraf::open(&path).unwrap();
    let long = "long value ".repeat(40);
    let facts: String = (0..2200)
        .map(|i| format!("[:p{i} :c {}] [:p{i} :long \"{long}\"]", i % 3))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    db.execute(&format!(
        "(transact [[:pending :c 1] [:pending :long \"{long}\"]])"
    ))
    .unwrap();
    for (attr, count) in [(":c", 3), (":long", 1)] {
        let q = format!("(query [:find ?v :where [?e {attr} ?v] :distinct])");
        let mut cursor = db.query(&q).unwrap();
        let mut n = 0;
        while let Some(batch) = cursor.next_batch(1).unwrap() {
            n += batch.len();
        }
        assert_eq!(n, count, "unique across batches and committed/pending data");
    }
    drop(db);
    let db = Minigraf::open(&path).unwrap();
    assert_eq!(
        rows(&db, "(query [:find ?v :where [?e :long ?v] :distinct])").len(),
        1
    );
}

#[test]
fn staged_chunked_transactions_and_long_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chunked.graph");
    let db = OpenOptions::new()
        .transaction_chunk_facts(2)
        .path(&path)
        .open()
        .unwrap();
    let long = "chunked long value ".repeat(30);
    let mut tx = db.begin_write().unwrap();
    let facts: String = (0..9)
        .map(|i| format!("[:p{i} :long \"{long}\"]"))
        .collect();
    tx.execute(&format!("(transact [{facts}])")).unwrap();
    let q = "(query [:find ?v :where [?e :long ?v] :distinct])";
    assert_eq!(output(tx.execute(q).unwrap()).len(), 1, "staged distinct");
    tx.commit().unwrap();
    assert_eq!(rows(&db, q).len(), 1);
    db.checkpoint().unwrap();
    drop(db);
    let db = Minigraf::open(&path).unwrap();
    assert_eq!(rows(&db, q).len(), 1);
}

#[test]
fn typed_values_nan_and_sets_use_group_equality_and_canonical_output() {
    use minigraf::{FactRecord, LogWriter};
    use uuid::Uuid;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed.graph");
    let mut writer = LogWriter::create(&path, OpenOptions::new()).unwrap();
    let nan = f64::from_bits(0xfff8_0000_0000_0001);
    let set = |f| Value::Set(ValueSet::new([Value::Float(f), Value::Float(nan)]).unwrap());
    let values = [
        Value::Integer(0),
        Value::Float(0.0),
        Value::Float(-0.0),
        Value::Float(f64::NAN),
        Value::Float(nan),
        Value::Keyword(":entity".into()),
        Value::Ref(Uuid::new_v5(&Uuid::NAMESPACE_OID, b":entity")),
        Value::Boolean(false),
        Value::Null,
        set(0.0),
        set(-0.0),
    ];
    for (i, value) in values.into_iter().enumerate() {
        writer
            .append(&FactRecord {
                entity: Uuid::from_u128(i as u128 + 1),
                attribute: ":c".into(),
                value,
                tx_id: 1,
                tx_count: 1,
                valid_from: 0,
                valid_to: i64::MAX,
                asserted: true,
            })
            .unwrap();
    }
    writer.finish().unwrap();
    let db = Minigraf::open(&path).unwrap();
    let answer = rows(&db, "(query [:find ?v :where [?e :c ?v] :distinct])");
    assert_eq!(
        answer.len(),
        8,
        "typed values stay separate; zeros, NaNs and sets collapse"
    );
    for row in &answer {
        let check_float = |v: &Value| {
            if let Value::Float(f) = v {
                if *f == 0.0 {
                    assert_eq!(f.to_bits(), (-0.0_f64).to_bits());
                }
                if f.is_nan() {
                    assert_eq!(f.to_bits(), f64::NAN.to_bits());
                }
            }
        };
        check_float(&row[0]);
        if let Value::Set(s) = &row[0] {
            s.iter().for_each(check_float);
        }
    }
    let count = rows(&db, "(query [:find (count-distinct ?v) :where [?e :c ?v]])");
    // count-distinct ignores nil; the projected result includes it.
    assert!(
        count == vec![vec![Value::Integer(7)]],
        "same grouping equality"
    );
}

#[test]
fn canonical_output_also_applies_to_singletons_and_prepared_floats() {
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    let p = db
        .prepare("(query [:find ?v :where [?e :c ?n] [(+ $x 0.0) ?v] :distinct])")
        .unwrap();
    let answer = output(
        p.execute(&[("x", BindValue::Val(Value::Float(0.0)))])
            .unwrap(),
    );
    assert_eq!(answer.len(), 1);
    assert_eq!(
        answer[0][0].as_float().unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    // Arithmetic drops nonfinite results; an aggregate can return a NaN.
    db.register_aggregate(
        "nan-result",
        || (),
        |_, _| {},
        |_, _| Value::Float(f64::from_bits(0xfff8_0000_0000_0001)),
    )
    .unwrap();
    let answer = rows(
        &db,
        "(query [:find (nan-result ?v) :where [?e :c ?v] :distinct])",
    );
    assert_eq!(answer.len(), 1);
    assert_eq!(
        answer[0][0].as_float().unwrap().to_bits(),
        f64::NAN.to_bits()
    );
}

#[test]
fn distinct_memory_limit_and_cursor_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        let facts: String = (0..12000).map(|i| format!("[:p{i} :c {i}]")).collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
        db.checkpoint().unwrap();
    }
    let db = OpenOptions::new()
        .query_memory_limit(128 * 1024)
        .path(&path)
        .open()
        .unwrap();
    let bag = "(query [:find ?v :where [?e :c ?v]])";
    assert_eq!(rows(&db, bag).len(), 12000, "the scan fits");
    let mut cursor = db
        .query("(query [:find ?v :where [?e :c ?v] :distinct])")
        .unwrap();
    loop {
        match cursor.next_batch(1024) {
            Ok(Some(_)) => {}
            Ok(None) => panic!("distinct should exceed its memory bound"),
            Err(e) => {
                assert_eq!(e.code(), "QRY-012");
                assert!(e.to_string().contains("Distinct"));
                break;
            }
        }
    }
    assert!(
        cursor.next_batch(1).unwrap().is_none(),
        "error closes the cursor"
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :c ?v] :distinct :limit 1])"
        )
        .len(),
        1
    );
    let q = "(query [:find ?v :where [?e :c ?v] :distinct :order-by [?v] :limit 1])";
    assert_eq!(
        rows(&db, q).len(),
        1,
        "ordered input stops before the bound"
    );
    let q = "(query [:find ?v :where [?e :c ?v] :distinct :order-by [?v :desc] :limit 1])";
    assert_eq!(
        db.execute(q)
            .expect_err("distinct retains keys even under top-k")
            .code(),
        "QRY-012"
    );
}

#[test]
fn retained_set_payloads_count_toward_the_memory_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("set-memory.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        let facts: String = (0..64)
            .map(|i| {
                let elements: String = (0..32)
                    .map(|j| format!("\"payload-{i:03}-{j:03}-{}\" ", "x".repeat(40)))
                    .collect();
                format!("[:p{i} :sets #{{{elements}}}]")
            })
            .collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
        db.checkpoint().unwrap();
    }
    let db = OpenOptions::new()
        .query_memory_limit(64 * 1024)
        .path(&path)
        .open()
        .unwrap();
    assert_eq!(
        rows(&db, "(query [:find ?s :where [?e :sets ?s]])").len(),
        64,
        "plain scan fits"
    );
    for q in [
        "(query [:find ?s :where [?e :sets ?s] :distinct])",
        "(query [:find ?s (count ?e) :where [?e :sets ?s]])",
        "(query [:find ?s (row-number :over (:order-by ?e)) :where [?e :sets ?s]])",
        "(query [:find (rank :over (:order-by ?s)) :where [?e :sets ?s]])",
    ] {
        let e = db
            .execute(q)
            .err()
            .expect("retained set payloads must exceed the bound");
        assert_eq!(e.code(), "QRY-012");
    }
}

#[cfg(feature = "test-hooks")]
#[test]
fn distinct_is_independent_of_join_and_subplan_choices() {
    use minigraf::Subplans;
    let db = Minigraf::in_memory().unwrap();
    seed(&db);
    db.execute("(transact [[:e1 :f 0.0] [:e2 :f -0.0] [:e1 :s #{0.0}] [:e2 :s #{-0.0}]])")
        .unwrap();
    for reorder in [true, false] {
        let _r = minigraf::__reorder(reorder);
        for subplans in [Subplans::Hash, Subplans::Correlated] {
            let _s = minigraf::__subplans(subplans);
            let q = "(query [:find ?v :where [?e :c ?v] (or [?e :c ?n] [?e :c ?n]) :distinct :order-by [?v]])";
            assert!(
                rows(&db, q) == vec![vec![Value::Integer(1)], vec![Value::Integer(2)]],
                "same output in every plan"
            );
            let q = "(query [:find ?f ?s :where [?e :f ?f] [?e :s ?s] (or [?e :c ?v] [?e :c ?v]) :distinct :order-by [?f]])";
            let answer = rows(&db, q);
            assert_eq!(answer.len(), 1);
            assert_eq!(
                answer[0][0].as_float().unwrap().to_bits(),
                (-0.0_f64).to_bits()
            );
            let set = answer[0][1].as_set().unwrap();
            assert_eq!(
                set.iter().next().unwrap().as_float().unwrap().to_bits(),
                (-0.0_f64).to_bits()
            );
        }
    }
}
