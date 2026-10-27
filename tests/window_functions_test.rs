use minigraf::db::Minigraf;
use minigraf::{QueryResult, Value};

fn setup_employees() -> Minigraf {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(concat!(
        r#"(transact ["#,
        r#"  [:e1 :employee/name "Alice"]"#,
        r#"  [:e1 :employee/dept "Engineering"]"#,
        r#"  [:e1 :employee/salary 90000]"#,
        r#"  [:e2 :employee/name "Bob"]"#,
        r#"  [:e2 :employee/dept "Engineering"]"#,
        r#"  [:e2 :employee/salary 110000]"#,
        r#"  [:e3 :employee/name "Carol"]"#,
        r#"  [:e3 :employee/dept "Product"]"#,
        r#"  [:e3 :employee/salary 95000]"#,
        r#"  [:e4 :employee/name "Dave"]"#,
        r#"  [:e4 :employee/dept "Product"]"#,
        r#"  [:e4 :employee/salary 85000]"#,
        r#"])"#,
    ))
    .expect("transact employees");
    db
}

fn get_results(r: QueryResult) -> Vec<Vec<Value>> {
    if let QueryResult::QueryResults { results, .. } = r {
        results
    } else {
        panic!("expected QueryResults");
    }
}

// ── row-number ──────────────────────────────────────────────────────────────

#[test]
fn row_number_assigns_sequential_positions() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (row-number :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4, "expected 4 rows");
    // Each salary should appear exactly once
    // The row-number for the smallest salary (85000) should be 1
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000));
    assert!(row_85k.is_some(), "salary 85000 not found");
    assert_eq!(
        row_85k.unwrap()[1],
        Value::Integer(1),
        "85000 should be row 1"
    );
    // The row-number for 110000 should be 4
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000));
    assert_eq!(
        row_110k.unwrap()[1],
        Value::Integer(4),
        "110000 should be row 4"
    );
}

// ── rank ────────────────────────────────────────────────────────────────────

#[test]
fn rank_assigns_same_rank_to_ties() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(concat!(
        r#"(transact ["#,
        r#"  [:a :item/score 10]"#,
        r#"  [:b :item/score 10]"#,
        r#"  [:c :item/score 20]"#,
        r#"])"#,
    ))
    .expect("transact");
    let result = db
        .execute(
            r#"(query [:find ?score (rank :over (:order-by ?score))
                   :where [?e :item/score ?score]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 3);
    // Both score=10 rows get rank 1; score=20 gets rank 3
    let tied: Vec<_> = rows.iter().filter(|r| r[0] == Value::Integer(10)).collect();
    assert_eq!(tied.len(), 2);
    for r in &tied {
        assert_eq!(r[1], Value::Integer(1), "tied scores should both be rank 1");
    }
    let top = rows.iter().find(|r| r[0] == Value::Integer(20)).unwrap();
    assert_eq!(
        top[1],
        Value::Integer(3),
        "score 20 should be rank 3 (gap after tie)"
    );
}

// ── cumulative sum ──────────────────────────────────────────────────────────

#[test]
fn cumulative_sum_over_whole_result() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (sum ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // sorted asc: 85000, 90000, 95000, 110000
    // cumulative:  85000, 175000, 270000, 380000
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(85000));
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(row_110k[1], Value::Integer(380000));
}

#[test]
fn bounded_rows_sum_uses_current_and_one_previous_row() {
    let db = setup_employees();
    let rows = get_results(
        db.execute(
            r#"(query [:find ?salary (sum ?salary :over (:order-by ?salary :rows 1 preceding))
                       :where [?e :employee/salary ?salary]])"#,
        )
        .expect("bounded window query"),
    );
    assert_eq!(rows.len(), 4);
    for (salary, sum) in [
        (85000, 85000),
        (90000, 175000),
        (95000, 185000),
        (110000, 205000),
    ] {
        let row = rows.iter().find(|r| r[0] == Value::Integer(salary));
        assert_eq!(row.map(|r| &r[1]), Some(&Value::Integer(sum)));
    }
}

#[test]
fn bounded_rows_split_ties_and_reset_at_partitions() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(
        r#"(transact [[:a :part "p1"] [:a :name "A"] [:a :ord 10] [:a :amount 2]
                      [:b :part "p1"] [:b :name "B"] [:b :ord 10] [:b :amount 3]
                      [:c :part "p1"] [:c :name "C"] [:c :ord 10] [:c :amount 5]
                      [:d :part "p2"] [:d :name "D"] [:d :ord 10] [:d :amount 7]])"#,
    )
    .expect("transact");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?part ?name ?amount
                       (sum ?amount :over (:partition-by ?part :order-by ?ord :rows 1 preceding))
                       (count ?amount :over (:partition-by ?part :order-by ?ord :rows 1 preceding))
                       (min ?amount :over (:partition-by ?part :order-by ?ord :rows 1 preceding))
                       (max ?amount :over (:partition-by ?part :order-by ?ord :rows 1 preceding))
                       (avg ?amount :over (:partition-by ?part :order-by ?ord :rows 1 preceding))
                       :where [?e :part ?part] [?e :name ?name]
                              [?e :ord ?ord] [?e :amount ?amount]])"#,
        )
        .expect("bounded aggregate windows"),
    );
    let at = |name: &str| {
        rows.iter()
            .find(|r| r[1] == Value::String(name.to_string()))
            .expect("named row")
    };
    assert_eq!(
        &at("A")[3..],
        &[
            Value::Integer(2),
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(2),
            Value::Float(2.0)
        ]
    );
    assert_eq!(
        &at("B")[3..],
        &[
            Value::Integer(5),
            Value::Integer(2),
            Value::Integer(2),
            Value::Integer(3),
            Value::Float(2.5)
        ]
    );
    assert_eq!(
        &at("C")[3..],
        &[
            Value::Integer(8),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(5),
            Value::Float(4.0)
        ]
    );
    assert_eq!(
        &at("D")[3..],
        &[
            Value::Integer(7),
            Value::Integer(1),
            Value::Integer(7),
            Value::Integer(7),
            Value::Float(7.0)
        ]
    );
}

#[test]
fn bounded_rows_zero_and_huge_width_in_descending_order() {
    let db = setup_employees();
    let rows = get_results(
        db.execute(
            r#"(query [:find ?salary
                       (sum ?salary :over (:rows 0 preceding :order-by ?salary :desc))
                       (sum ?salary :over (:order-by ?salary :desc :rows 9223372036854775807 preceding))
                       :where [?e :employee/salary ?salary]])"#,
        )
        .expect("bounded windows"),
    );
    let at = |salary| {
        rows.iter()
            .find(|r| r[0] == Value::Integer(salary))
            .expect("salary")
    };
    for (salary, prefix) in [
        (110000, 110000),
        (95000, 205000),
        (90000, 295000),
        (85000, 380000),
    ] {
        assert_eq!(at(salary)[1], Value::Integer(salary));
        assert_eq!(at(salary)[2], Value::Integer(prefix));
    }
}

#[test]
fn bounded_rows_udf_prepared_cursor_and_pagination() {
    use minigraf::BindValue;
    let db = Minigraf::in_memory().expect("in-memory db");
    db.register_aggregate(
        "frame-total",
        || 0i64,
        |sum, value| {
            if let Value::Integer(n) = value {
                *sum += n;
            }
        },
        |sum, _| Value::Integer(*sum),
    )
    .expect("register UDF");
    db.execute(
        r#"(transact [[:a :group "g"] [:a :ord 1] [:a :v 2]
                      [:b :group "g"] [:b :ord 2] [:b :v nil]
                      [:c :group "g"] [:c :ord 3] [:c :v 5]
                      [:d :group "g"] [:d :ord 4] [:d :v 7]])"#,
    )
    .expect("facts");
    let q = r#"(query [:find ?ord
        (frame-total ?v :over (:order-by ?ord :rows 1 preceding))
        :where [?e :group $group] [?e :ord ?ord] [?e :v ?v]
        :order-by [?ord] :offset 1 :limit 2])"#;
    let prepared = db.prepare(q).expect("prepare");
    let bind = [("group", BindValue::Val(Value::String("g".into())))];
    let expected = vec![
        vec![Value::Integer(2), Value::Integer(2)],
        vec![Value::Integer(3), Value::Integer(5)],
    ];
    assert_eq!(
        get_results(prepared.execute(&bind).expect("execute")),
        expected
    );
    let mut cursor = prepared.query(&bind).expect("cursor");
    let mut pulled = Vec::new();
    while let Some(batch) = cursor.next_batch(1).expect("batch") {
        pulled.extend(batch.rows().iter().cloned());
    }
    assert_eq!(pulled, expected);
}

#[test]
fn bounded_rows_run_after_grouping_and_before_distinct() {
    let db = setup_employees();
    let grouped = get_results(
        db.execute(
            r#"(query [:find ?dept (count ?e)
                (count ?dept :over (:order-by ?dept :rows 1 preceding))
                :with ?e
                :where [?e :employee/dept ?dept]])"#,
        )
        .expect("grouped framed query"),
    );
    assert_eq!(grouped.len(), 2);
    assert!(grouped.contains(&vec![
        Value::String("Engineering".into()),
        Value::Integer(2),
        Value::Integer(1)
    ]));
    assert!(grouped.contains(&vec![
        Value::String("Product".into()),
        Value::Integer(2),
        Value::Integer(2)
    ]));

    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(r#"(transact [[:a :v 1] [:b :v 1] [:c :v 1]])"#)
        .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?v (count ?v :over (:order-by ?v :rows 1 preceding))
                :where [?e :v ?v] :distinct])"#,
        )
        .expect("distinct framed query"),
    );
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&vec![Value::Integer(1), Value::Integer(1)]));
    assert!(rows.contains(&vec![Value::Integer(1), Value::Integer(2)]));
}

#[cfg(feature = "test-hooks")]
#[test]
fn bounded_rows_answer_bag_is_plan_independent() {
    use minigraf::{FactRecord, LogWriter, OpenOptions};
    use uuid::Uuid;

    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("row-frame-plan.graph");
    let mut writer = LogWriter::create(&path, OpenOptions::new()).expect("fact writer");
    let amounts = [
        Value::Integer(1),
        Value::Float(1.0),
        Value::Float(-0.0),
        Value::Float(0.0),
        Value::Float(f64::NAN),
        Value::Integer(1),
    ];
    for (pos, amount) in amounts.into_iter().enumerate() {
        let entity = Uuid::from_u128(pos as u128 + 1);
        for (attribute, value) in [
            (":name", Value::String("T".into())),
            (":score", Value::Integer(1)),
            (":amount", amount),
        ] {
            writer
                .append(&FactRecord {
                    entity,
                    attribute: attribute.into(),
                    value,
                    tx_id: 1,
                    tx_count: 1,
                    valid_from: 0,
                    valid_to: i64::MAX,
                    asserted: true,
                })
                .expect("fact");
        }
    }
    writer.finish().expect("finish facts");
    let db = Minigraf::open(&path).expect("open fixture");
    let q = r#"(query [:find ?name ?amount
        (sum ?amount :over (:order-by ?score :rows 1 preceding))
        :where [?e :score ?score] [?e :name ?name] [?e :amount ?amount]])"#;
    let run = |reorder| {
        let _guard = minigraf::__reorder(reorder);
        let mut rows = get_results(db.execute(q).expect("query"));
        rows.sort();
        rows
    };
    let without_reorder = run(false);
    assert_eq!(
        without_reorder.len(),
        6,
        "every row stays in the answer bag"
    );
    assert_eq!(
        without_reorder
            .iter()
            .filter(|row| matches!(&row[1], Value::Float(f) if f.is_nan()))
            .count(),
        1,
        "NaN row participates in the window"
    );
    let with_reorder = run(true);
    assert_eq!(without_reorder.len(), with_reorder.len());
    for (left, right) in without_reorder.iter().zip(&with_reorder) {
        assert_eq!(left.len(), right.len());
        for (a, b) in left.iter().zip(right) {
            match (a, b) {
                (Value::Float(a), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
                (a, b) => assert_eq!(a, b),
            }
        }
    }
}

// ── partition-by ────────────────────────────────────────────────────────────

#[test]
fn sum_resets_per_partition() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?dept ?salary (sum ?salary :over (:partition-by ?dept :order-by ?salary))
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);

    // Engineering: 90000, 110000 → cumulative 90000, 200000
    let eng_90k = rows
        .iter()
        .find(|r| r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(90000))
        .unwrap();
    assert_eq!(eng_90k[2], Value::Integer(90000));

    let eng_110k = rows
        .iter()
        .find(|r| r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(110000))
        .unwrap();
    assert_eq!(eng_110k[2], Value::Integer(200000));

    // Product: 85000, 95000 → cumulative 85000, 180000
    let prod_85k = rows
        .iter()
        .find(|r| r[0] == Value::String("Product".into()) && r[1] == Value::Integer(85000))
        .unwrap();
    assert_eq!(prod_85k[2], Value::Integer(85000));

    let prod_95k = rows
        .iter()
        .find(|r| r[0] == Value::String("Product".into()) && r[1] == Value::Integer(95000))
        .unwrap();
    assert_eq!(prod_95k[2], Value::Integer(180000));
}

// ── running count ───────────────────────────────────────────────────────────

#[test]
fn running_count_over_ordered_result() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (count ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(
        row_110k[1],
        Value::Integer(4),
        "last row should have count 4"
    );
}

// ── running min/max ─────────────────────────────────────────────────────────

#[test]
fn running_min_over_ordered_result() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (min ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    // Running min: first row min = 85000, subsequent rows also min = 85000
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(row_110k[1], Value::Integer(85000));
}

// ── running avg ─────────────────────────────────────────────────────────────

#[test]
fn running_avg_over_ordered_result() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (avg ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // After all 4: avg(85000, 90000, 95000, 110000) = 380000/4 = 95000.0
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(row_110k[1], Value::Float(95000.0));
}

// ── desc ordering ───────────────────────────────────────────────────────────

#[test]
fn row_number_desc_ordering() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (row-number :over (:order-by ?salary :desc))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    // desc: 110000 is row 1, 85000 is row 4
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(row_110k[1], Value::Integer(1));
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(4));
}

// ── mixed: regular aggregate + window ──────────────────────────────────────

#[test]
fn mixed_aggregate_and_window_in_same_find() {
    let db = setup_employees();
    // count(e) collapses by dept, then the window runs over the dept rows,
    // reading only the grouping variable ?dept (#547).
    let result = db
        .execute(
            r#"(query [:find ?dept (count ?e) (rank :over (:order-by ?dept :desc))
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 2, "expected one row per dept");
    let eng = rows
        .iter()
        .find(|r| r[0] == Value::String("Engineering".into()))
        .unwrap();
    assert_eq!(eng[1], Value::Integer(2), "Engineering has 2 employees");
    assert_eq!(eng[2], Value::Integer(2), "Product ranks first under :desc");
}

#[test]
fn window_in_aggregate_query_reading_a_non_grouping_variable_is_rejected() {
    // Before #547 ?salary read nil on every dept row, so the window summed
    // nothing; now it is PRS-082.
    let db = setup_employees();
    let err = db
        .execute(
            r#"(query [:find ?dept (count ?e) (sum ?salary :over (:order-by ?salary))
                   :with ?e ?salary
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
        )
        .err()
        .expect("rejected");
    assert_eq!(err.code(), "PRS-082");
    let err = db
        .execute(
            r#"(query [:find ?dept (count ?e) (lag ?salary :over (:order-by ?dept))
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
        )
        .err()
        .expect("lag on non-grouping variable rejected");
    assert_eq!(err.code(), "PRS-082");
}

// ── single-row partition edge case ─────────────────────────────────────────

#[test]
fn single_row_result_window_equals_row_value() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(r#"(transact [[:x :score 42]])"#)
        .expect("transact");
    let result = db
        .execute(
            r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::Integer(42));
}

// ── empty result ────────────────────────────────────────────────────────────

#[test]
fn empty_result_no_panic() {
    let db = Minigraf::in_memory().expect("in-memory db");
    let result = db
        .execute(
            r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 0);
}

// ── lag and lead ────────────────────────────────────────────────────────────

#[test]
fn lag_lead_read_adjacent_rows_and_nil_at_edges() {
    let db = setup_employees();
    let rows = get_results(
        db.execute(
            r#"(query [:find ?salary
        (lag ?salary :over (:order-by ?salary))
        (lead ?salary :over (:order-by ?salary))
        :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query"),
    );
    assert_eq!(rows.len(), 4);
    let at = |salary| {
        rows.iter()
            .find(|r| r[0] == Value::Integer(salary))
            .expect("salary row")
    };
    assert_eq!(&at(85000)[1..], &[Value::Null, Value::Integer(90000)]);
    assert_eq!(
        &at(90000)[1..],
        &[Value::Integer(85000), Value::Integer(95000)]
    );
    assert_eq!(
        &at(95000)[1..],
        &[Value::Integer(90000), Value::Integer(110000)]
    );
    assert_eq!(&at(110000)[1..], &[Value::Integer(95000), Value::Null]);
}

#[test]
fn lag_lead_reset_per_partition_in_descending_order() {
    let db = setup_employees();
    let rows = get_results(
        db.execute(
            r#"(query [:find ?dept ?salary
        (lag ?salary :over (:partition-by ?dept :order-by ?salary :desc))
        (lead ?salary :over (:partition-by ?dept :order-by ?salary :desc))
        :where [?e :employee/dept ?dept]
               [?e :employee/salary ?salary]])"#,
        )
        .expect("query"),
    );
    let at = |dept: &str, salary| {
        rows.iter()
            .find(|r| r[0] == Value::String(dept.to_string()) && r[1] == Value::Integer(salary))
            .expect("department salary row")
    };
    assert_eq!(
        &at("Engineering", 110000)[2..],
        &[Value::Null, Value::Integer(90000)]
    );
    assert_eq!(
        &at("Engineering", 90000)[2..],
        &[Value::Integer(110000), Value::Null]
    );
    assert_eq!(
        &at("Product", 95000)[2..],
        &[Value::Null, Value::Integer(85000)]
    );
    assert_eq!(
        &at("Product", 85000)[2..],
        &[Value::Integer(95000), Value::Null]
    );
}

#[test]
fn lag_lead_tied_keys_use_named_variable_order() {
    let db = Minigraf::in_memory().expect("db");
    db.execute(
        r#"(transact [[:b :name "Beta"] [:b :score 10]
                           [:a :name "Alpha"] [:a :score 10]
                           [:c :name "Gamma"] [:c :score 20]])"#,
    )
    .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?name ?score
        (lag ?name :over (:order-by ?score))
        (lead ?name :over (:order-by ?score))
        :where [?e :name ?name] [?e :score ?score]])"#,
        )
        .expect("query"),
    );
    let at = |name: &str| {
        rows.iter()
            .find(|r| r[0] == Value::String(name.into()))
            .expect("name")
    };
    assert_eq!(
        &at("Alpha")[2..],
        &[Value::Null, Value::String("Beta".into())]
    );
    assert_eq!(
        &at("Beta")[2..],
        &[Value::String("Alpha".into()), Value::String("Gamma".into())]
    );
    assert_eq!(
        &at("Gamma")[2..],
        &[Value::String("Beta".into()), Value::Null]
    );

    let descending = get_results(
        db.execute(
            r#"(query [:find ?name ?score
        (lag ?name :over (:order-by ?score :desc))
        (lead ?name :over (:order-by ?score :desc))
        :where [?e :name ?name] [?e :score ?score]])"#,
        )
        .expect("descending query"),
    );
    let at_desc = |name: &str| {
        descending
            .iter()
            .find(|r| r[0] == Value::String(name.into()))
            .expect("descending name")
    };
    assert_eq!(
        &at_desc("Gamma")[2..],
        &[Value::Null, Value::String("Alpha".into())]
    );
    assert_eq!(
        &at_desc("Alpha")[2..],
        &[Value::String("Gamma".into()), Value::String("Beta".into())]
    );
    assert_eq!(
        &at_desc("Beta")[2..],
        &[Value::String("Alpha".into()), Value::Null]
    );
}

#[test]
fn lag_lead_preserve_long_strings_keywords_refs_sets_and_nil() {
    let db = Minigraf::in_memory().expect("db");
    let long = "x".repeat(100);
    db.execute(&format!(
        r#"(transact [[:a :ord 1] [:a :v "{long}"]
        [:b :ord 2] [:b :v :kind/b]
        [:c :ord 3] [:c :v #uuid "00000000-0000-0000-0000-000000000003"]
        [:d :ord 4] [:d :v #{{1 2}}]
        [:e :ord 5] [:e :v nil]])"#
    ))
    .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?ord ?v
        (lag ?v :over (:order-by ?ord)) (lead ?v :over (:order-by ?ord))
        :where [?e :ord ?ord] [?e :v ?v]])"#,
        )
        .expect("query"),
    );
    assert_eq!(rows.len(), 5);
    let at = |n| {
        rows.iter()
            .find(|r| r[0] == Value::Integer(n))
            .expect("order row")
    };
    assert_eq!(at(2)[2], Value::String(long));
    assert_eq!(at(3)[2], Value::Keyword(":kind/b".into()));
    assert_eq!(at(4)[2], at(3)[1]);
    assert_eq!(at(3)[3], at(4)[1]);
    assert_eq!(at(5)[2], at(4)[1]);
    assert_eq!(at(4)[3], Value::Null, "neighbor with nil value");
    assert_eq!(at(5)[3], Value::Null, "partition edge");
}

#[test]
fn lag_lead_on_grouped_rows_and_singleton_partition() {
    let db = setup_employees();
    db.execute(r#"(transact [[:e5 :employee/dept "Sales"] [:e5 :employee/salary 70000]])"#)
        .expect("fact");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?dept (count ?salary)
        (lag ?dept :over (:order-by ?dept))
        (lead ?dept :over (:order-by ?dept))
        :where [?e :employee/dept ?dept] [?e :employee/salary ?salary]])"#,
        )
        .expect("query"),
    );
    let at = |dept: &str| {
        rows.iter()
            .find(|r| r[0] == Value::String(dept.into()))
            .expect("dept")
    };
    assert_eq!(
        &at("Engineering")[1..],
        &[
            Value::Integer(2),
            Value::Null,
            Value::String("Product".into())
        ]
    );
    assert_eq!(
        &at("Product")[1..],
        &[
            Value::Integer(2),
            Value::String("Engineering".into()),
            Value::String("Sales".into())
        ]
    );
    assert_eq!(
        &at("Sales")[1..],
        &[
            Value::Integer(1),
            Value::String("Product".into()),
            Value::Null
        ]
    );

    let singleton = get_results(
        db.execute(
            r#"(query [:find ?dept ?salary
        (lag ?salary :over (:partition-by ?dept :order-by ?salary))
        (lead ?salary :over (:partition-by ?dept :order-by ?salary))
        :where [?e :employee/dept ?dept] [?e :employee/salary ?salary]])"#,
        )
        .expect("query"),
    );
    let sales = singleton
        .iter()
        .find(|r| r[0] == Value::String("Sales".into()))
        .expect("sales");
    assert_eq!(&sales[2..], &[Value::Null, Value::Null]);
}

#[test]
fn pagination_and_cursors_see_lag_before_offset_and_limit() {
    use minigraf::BindValue;
    let db = setup_employees();
    let q = r#"(query [:find ?salary (lag ?salary :over (:order-by ?salary))
        :where [?e :employee/salary ?salary]
        :order-by [?salary] :offset 1 :limit $n])"#;
    let prepared = db.prepare(q).expect("prepare");
    let bind = [("n", BindValue::Val(Value::Integer(2)))];
    let expected = vec![
        vec![Value::Integer(90000), Value::Integer(85000)],
        vec![Value::Integer(95000), Value::Integer(90000)],
    ];
    assert_eq!(
        get_results(prepared.execute(&bind).expect("execute")),
        expected
    );
    let mut cursor = prepared.query(&bind).expect("cursor");
    let mut pulled = Vec::new();
    while let Some(batch) = cursor.next_batch(1).expect("batch") {
        pulled.extend(batch.rows().iter().cloned());
    }
    assert_eq!(pulled, expected);
}

#[test]
fn lag_lead_keep_nil_and_mixed_partition_keys_separate() {
    let db = Minigraf::in_memory().expect("db");
    db.execute(
        r#"(transact [[:a :part nil] [:a :ord 1] [:a :v "A"]
        [:b :part nil] [:b :ord 2] [:b :v "B"]
        [:c :part 1] [:c :ord 1] [:c :v "C"]
        [:d :part "1"] [:d :ord 1] [:d :v "D"]])"#,
    )
    .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?v ?part
        (lag ?v :over (:partition-by ?part :order-by ?ord))
        (lead ?v :over (:partition-by ?part :order-by ?ord))
        :where [?e :part ?part] [?e :ord ?ord] [?e :v ?v]])"#,
        )
        .expect("query"),
    );
    let at = |name: &str| {
        rows.iter()
            .find(|r| r[0] == Value::String(name.into()))
            .expect("name")
    };
    assert_eq!(&at("A")[2..], &[Value::Null, Value::String("B".into())]);
    assert_eq!(&at("B")[2..], &[Value::String("A".into()), Value::Null]);
    assert_eq!(&at("C")[2..], &[Value::Null, Value::Null]);
    assert_eq!(&at("D")[2..], &[Value::Null, Value::Null]);
}

#[test]
fn distinct_runs_after_lag_on_duplicate_rows() {
    let db = Minigraf::in_memory().expect("db");
    db.execute(r#"(transact [[:a :v 1] [:b :v 1] [:c :v 2]])"#)
        .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find ?v
        (lag ?v :over (:order-by ?v)) :where [?e :v ?v] :distinct])"#,
        )
        .expect("query"),
    );
    assert_eq!(rows.len(), 3);
    assert!(rows.contains(&vec![Value::Integer(1), Value::Null]));
    assert!(rows.contains(&vec![Value::Integer(1), Value::Integer(1)]));
    assert!(rows.contains(&vec![Value::Integer(2), Value::Integer(1)]));
}

#[test]
fn plain_lag_and_lead_names_remain_available_for_registered_aggregates() {
    let db = Minigraf::in_memory().expect("db");
    db.register_aggregate(
        "lag",
        || 0i64,
        |sum, v| {
            if let Value::Integer(n) = v {
                *sum += n;
            }
        },
        |sum, _| Value::Integer(*sum),
    )
    .expect("register lag");
    db.register_aggregate(
        "lead",
        || 0i64,
        |sum, v| {
            if let Value::Integer(n) = v {
                *sum += n;
            }
        },
        |sum, _| Value::Integer(*sum),
    )
    .expect("register lead");
    db.execute(r#"(transact [[:a :v 2] [:b :v 3]])"#)
        .expect("facts");
    let rows = get_results(
        db.execute(
            r#"(query [:find (lag ?v) (lead ?v)
        :where [?e :v ?v]])"#,
        )
        .expect("query"),
    );
    assert_eq!(rows, vec![vec![Value::Integer(5), Value::Integer(5)]]);
    let window_rows = get_results(
        db.execute(
            r#"(query [:find ?v (lag ?v :over (:order-by ?v))
                              (lead ?v :over (:order-by ?v))
                         :where [?e :v ?v]])"#,
        )
        .expect("window query"),
    );
    assert!(window_rows.contains(&vec![Value::Integer(2), Value::Null, Value::Integer(3)]));
    assert!(window_rows.contains(&vec![Value::Integer(3), Value::Integer(2), Value::Null]));
}

// ── running max ─────────────────────────────────────────────────────────────

#[test]
fn running_max_over_ordered_result() {
    let db = setup_employees();
    let result = db
        .execute(
            r#"(query [:find ?salary (max ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
        )
        .expect("query");
    let rows = get_results(result);
    // Running max: sorted asc 85000, 90000, 95000, 110000
    // The last row (110000) should have running max = 110000
    let row_110k = rows
        .iter()
        .find(|r| r[0] == Value::Integer(110000))
        .unwrap();
    assert_eq!(row_110k[1], Value::Integer(110000));
}
