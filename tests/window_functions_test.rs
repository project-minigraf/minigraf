use minigraf::db::Minigraf;
use minigraf::query::datalog::executor::QueryResult;
use minigraf::graph::types::Value;

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
    )).expect("transact employees");
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
    let result = db.execute(
        r#"(query [:find ?salary (row-number :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4, "expected 4 rows");
    // Each salary should appear exactly once
    // The row-number for the smallest salary (85000) should be 1
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000));
    assert!(row_85k.is_some(), "salary 85000 not found");
    assert_eq!(row_85k.unwrap()[1], Value::Integer(1), "85000 should be row 1");
    // The row-number for 110000 should be 4
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000));
    assert_eq!(row_110k.unwrap()[1], Value::Integer(4), "110000 should be row 4");
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
    )).expect("transact");
    let result = db.execute(
        r#"(query [:find ?score (rank :over (:order-by ?score))
                   :where [?e :item/score ?score]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 3);
    // Both score=10 rows get rank 1; score=20 gets rank 3
    let tied: Vec<_> = rows.iter().filter(|r| r[0] == Value::Integer(10)).collect();
    assert_eq!(tied.len(), 2);
    for r in &tied {
        assert_eq!(r[1], Value::Integer(1), "tied scores should both be rank 1");
    }
    let top = rows.iter().find(|r| r[0] == Value::Integer(20)).unwrap();
    assert_eq!(top[1], Value::Integer(3), "score 20 should be rank 3 (gap after tie)");
}

// ── cumulative sum ──────────────────────────────────────────────────────────

#[test]
fn cumulative_sum_over_whole_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (sum ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // sorted asc: 85000, 90000, 95000, 110000
    // cumulative:  85000, 175000, 270000, 380000
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(85000));
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(380000));
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
    let eng_90k = rows.iter().find(|r| {
        r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(90000)
    }).unwrap();
    assert_eq!(eng_90k[2], Value::Integer(90000));

    let eng_110k = rows.iter().find(|r| {
        r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(110000)
    }).unwrap();
    assert_eq!(eng_110k[2], Value::Integer(200000));

    // Product: 85000, 95000 → cumulative 85000, 180000
    let prod_85k = rows.iter().find(|r| {
        r[0] == Value::String("Product".into()) && r[1] == Value::Integer(85000)
    }).unwrap();
    assert_eq!(prod_85k[2], Value::Integer(85000));

    let prod_95k = rows.iter().find(|r| {
        r[0] == Value::String("Product".into()) && r[1] == Value::Integer(95000)
    }).unwrap();
    assert_eq!(prod_95k[2], Value::Integer(180000));
}

// ── running count ───────────────────────────────────────────────────────────

#[test]
fn running_count_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (count ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(4), "last row should have count 4");
}

// ── running min/max ─────────────────────────────────────────────────────────

#[test]
fn running_min_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (min ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    // Running min: first row min = 85000, subsequent rows also min = 85000
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(85000));
}

// ── running avg ─────────────────────────────────────────────────────────────

#[test]
fn running_avg_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (avg ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // After all 4: avg(85000, 90000, 95000, 110000) = 380000/4 = 95000.0
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Float(95000.0));
}

// ── desc ordering ───────────────────────────────────────────────────────────

#[test]
fn row_number_desc_ordering() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (row-number :over (:order-by ?salary :desc))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    // desc: 110000 is row 1, 85000 is row 4
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(1));
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(4));
}

// ── mixed: regular aggregate + window ──────────────────────────────────────

#[test]
fn mixed_aggregate_and_window_in_same_find() {
    let db = setup_employees();
    // count(e) collapses by dept, then sum runs over collapsed rows
    let result = db.execute(
        r#"(query [:find ?dept (count ?e) (sum ?salary :over (:order-by ?salary))
                   :with ?e ?salary
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 2, "expected one row per dept");
    // Each dept row has [dept, count, cumulative-sum]
    let eng = rows.iter().find(|r| r[0] == Value::String("Engineering".into())).unwrap();
    assert_eq!(eng[1], Value::Integer(2), "Engineering has 2 employees");
}

// ── single-row partition edge case ─────────────────────────────────────────

#[test]
fn single_row_result_window_equals_row_value() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(r#"(transact [[:x :score 42]])"#).expect("transact");
    let result = db.execute(
        r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::Integer(42));
}

// ── empty result ────────────────────────────────────────────────────────────

#[test]
fn empty_result_no_panic() {
    let db = Minigraf::in_memory().expect("in-memory db");
    let result = db.execute(
        r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 0);
}

// ── parse-time error for lag/lead ──────────────────────────────────────────

#[test]
fn lag_rejected_at_parse_time() {
    let db = Minigraf::in_memory().expect("in-memory db");
    let result = db.execute(
        r#"(query [:find (lag ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not supported"));
}
