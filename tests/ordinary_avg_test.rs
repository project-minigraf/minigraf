//! Ordinary avg shares window avg's numeric population and arithmetic (#777).
use minigraf::{BindValue, Minigraf, QueryResult, Value};

fn result_rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("query results"),
    }
}

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    result_rows(db.execute(query).expect("query"))
}

const AVG: &str = "(query [:find (avg ?v) :where [?e :n ?v]])";

#[test]
fn ordinary_avg_reproduction_returns_float() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :n 10] [:b :n 20]])").unwrap();
    assert_eq!(rows(&db, AVG), vec![vec![Value::Float(15.0)]]);
}

#[test]
fn ordinary_avg_groups_skip_nonnumbers_and_preserve_with_multiplicity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :g "x"] [:a :n 10] [:b :g "x"] [:b :n 20]
        [:c :g "x"] [:c :n "unknown"] [:d :g "y"] [:d :n nil]
        [:e :g "z"] [:e :n true]])"#,
    )
    .unwrap();
    let grouped = "(query [:find ?g (avg ?v) :where [?e :g ?g] [?e :n ?v]])";
    assert_eq!(
        rows(&db, grouped),
        vec![
            vec![Value::String("x".into()), Value::Float(15.0)],
            vec![Value::String("y".into()), Value::Null],
            vec![Value::String("z".into()), Value::Null],
        ]
    );
    let mut finer = rows(&db, "(query [:find (avg ?v) :with ?e :where [?e :n ?v]])");
    finer.sort();
    let mut expected = vec![
        vec![Value::Float(10.0)],
        vec![Value::Float(20.0)],
        vec![Value::Null],
        vec![Value::Null],
        vec![Value::Null],
    ];
    expected.sort();
    assert_eq!(finer, expected);
    assert_eq!(rows(&db, AVG), vec![vec![Value::Float(15.0)]]);
    for find in ["(avg ?v)", "(count ?e) (avg ?v)", "?e (avg ?v)"] {
        assert!(
            rows(
                &db,
                &format!("(query [:find {find} :where [?e :absent ?v]])")
            )
            .is_empty()
        );
    }
}

#[test]
fn ordinary_avg_matches_sum_results_and_preserves_dropped_groups() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:a :n "unknown"] [:a :empty nil]])"#)
        .unwrap();
    for (find, expected) in [
        ("(avg ?v) (sum ?v)", vec![Value::Null, Value::Integer(0)]),
        ("(sum ?v) (avg ?v)", vec![Value::Integer(0), Value::Null]),
    ] {
        assert!(rows(&db, &format!("(query [:find {find} :where [?e :n ?v]])")) == vec![expected]);
    }
    assert!(
        rows(
            &db,
            "(query [:find (avg ?v) (min ?n) :where [?e :n ?v] [?e :empty ?n]])"
        )
        .is_empty()
    );
}

#[test]
fn ordinary_avg_prepared_execute_and_cursor_use_numeric_count() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :g "x"] [:a :n 10] [:b :g "x"] [:b :n 20]
        [:c :g "x"] [:c :n "unknown"] [:d :g "y"] [:d :n nil]])"#,
    )
    .unwrap();
    let prepared = db
        .prepare("(query [:find (avg ?v) :where [?e :g $group] [?e :n ?v]])")
        .unwrap();
    for (group, expected) in [("x", Value::Float(15.0)), ("y", Value::Null)] {
        let binds = [("group", BindValue::Val(Value::String(group.into())))];
        assert_eq!(
            result_rows(prepared.execute(&binds).unwrap()),
            vec![vec![expected.clone()]]
        );
        let mut cursor = prepared.query(&binds).unwrap();
        assert_eq!(
            cursor.next_batch(1).unwrap().unwrap().into_rows(),
            vec![vec![expected]]
        );
        assert!(cursor.next_batch(1).unwrap().is_none());
    }
}

#[test]
fn ordinary_avg_matches_window_without_removing_nonnumeric_rows() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :n 10] [:a :ord 1] [:b :n "unknown"] [:b :ord 2]
        [:c :n 20] [:c :ord 3]])"#,
    )
    .unwrap();
    let mut window = rows(
        &db,
        "(query [:find ?o (avg ?v :over (:order-by ?o))
        (avg ?v :over (:order-by ?o :rows 1 preceding)) :where [?e :n ?v] [?e :ord ?o]])",
    );
    window.sort();
    assert_eq!(
        window,
        vec![
            vec![Value::Integer(1), Value::Float(10.0), Value::Float(10.0)],
            vec![Value::Integer(2), Value::Float(10.0), Value::Float(10.0)],
            vec![Value::Integer(3), Value::Float(15.0), Value::Float(20.0)],
        ]
    );
    assert_eq!(rows(&db, AVG), vec![vec![Value::Float(15.0)]]);
}

#[test]
fn ordinary_avg_recursive_rule_and_transaction_reads() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c] [:b :n 10] [:c :n 20]])")
        .unwrap();
    db.execute("(rule [(reachable ?x ?y) [?x :next ?y]])")
        .unwrap();
    db.execute("(rule [(reachable ?x ?y) [?x :next ?z] (reachable ?z ?y)])")
        .unwrap();
    let q = "(query [:find (avg ?v) :where (reachable :a ?e) [?e :n ?v]])";
    assert_eq!(rows(&db, q), vec![vec![Value::Float(15.0)]]);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:c :next :d] [:d :n 30]])").unwrap();
    assert_eq!(
        result_rows(tx.execute(q).unwrap()),
        vec![vec![Value::Float(20.0)]]
    );
    tx.rollback();
    assert_eq!(rows(&db, q), vec![vec![Value::Float(15.0)]]);
}

#[test]
fn ordinary_avg_cursor_spans_batches_without_dropping_groups() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..1100)
        .map(|n| format!("[:e{n} :n 10] [:e{n} :group {n}] "))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let mut cursor = db
        .query("(query [:find ?g (avg ?v) :where [?e :group ?g] [?e :n ?v]])")
        .unwrap();
    let mut got = Vec::new();
    while let Some(batch) = cursor.next_batch(17).unwrap() {
        got.extend(batch.into_rows());
    }
    let expected: Vec<_> = (0..1100)
        .map(|n| vec![Value::Integer(n), Value::Float(10.0)])
        .collect();
    assert_eq!(got, expected);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ordinary_avg_pending_checkpointed_and_reopened_agree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("avg.graph");
    let expected = vec![vec![Value::Float(1.0)]];
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute("(transact [[:a :n 9007199254740993] [:b :n -9007199254740992] [:c :n 2]])")
            .unwrap();
        assert_eq!(rows(&db, AVG), expected);
        db.checkpoint().unwrap();
        assert_eq!(rows(&db, AVG), expected);
        db.execute(r#"(transact [[:d :n "unknown"] [:e :n #{1 2}]])"#)
            .unwrap();
        assert_eq!(rows(&db, AVG), expected);
        db.checkpoint().unwrap();
    }
    assert_eq!(rows(&Minigraf::open(&path).unwrap(), AVG), expected);
}

#[cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]
#[test]
fn ordinary_avg_is_independent_of_plan_choices() {
    use minigraf::Subplans;
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("plans.graph")).unwrap();
    db.execute(
        r#"(transact [[:a :n 10000000000000000.0] [:a :g 1]
        [:b :n 1.0] [:b :g 1] [:c :n -10000000000000000.0] [:c :g 1]
        [:d :n 1.0] [:d :g 1] [:e :n 0.5] [:e :g 1]
        [:f :n "unknown"] [:f :g 1] [:bad :n 999] [:bad :g 1] [:bad :excluded true]])"#,
    )
    .unwrap();
    let q = "(query [:find ?g (avg ?v) :where [?e :g ?g] [?e :n ?v]
        (not [?e :excluded true])])";
    for committed in [false, true] {
        if committed {
            db.checkpoint().unwrap();
        }
        for reorder in [false, true] {
            let _reorder = minigraf::__reorder(reorder);
            for input_order in [false, true] {
                let _input = minigraf::__input_order(input_order);
                for mode in [Subplans::Choose, Subplans::Hash, Subplans::Correlated] {
                    let _mode = minigraf::__subplans(mode);
                    let expected = vec![vec![Value::Integer(1), Value::Float(0.5)]];
                    assert_eq!(rows(&db, q), expected);
                    let mut cursor = db.query(q).unwrap();
                    let got: Vec<_> = cursor.by_ref().collect::<Result<_, _>>().unwrap();
                    assert_eq!(got, expected);
                }
            }
        }
    }
}
