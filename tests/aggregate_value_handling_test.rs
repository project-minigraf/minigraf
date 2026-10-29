//! #789: ordinary and window aggregates share their argument domain.
use minigraf::{BindValue, Minigraf, QueryResult, Value};

fn rows(result: QueryResult) -> Vec<Vec<Value>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    }
}

fn query(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    rows(db.execute(query).expect("aggregate query succeeds"))
}

#[test]
fn sum_skips_nonnumbers_and_distinct_keeps_numeric_identity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :v 10] [:b :v "unknown"] [:c :v 20]
        [:d :v 10] [:e :v 10.0] [:f :v true] [:g :v :keyword]
        [:h :v #uuid "00000000-0000-0000-0000-000000000001"]
        [:i :v #{1 "x"}] [:j :v nil]])"#,
    )
    .unwrap();
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) (sum-distinct ?v) (count ?v) (count-distinct ?v) :where [?e :v ?v]])"
        ) == vec![vec![
            Value::Float(50.0),
            Value::Float(40.0),
            Value::Integer(9),
            Value::Integer(8)
        ]]
    );
}

#[test]
fn extrema_accept_mixed_numbers_and_every_nonnil_class() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :v 2] [:b :v 1.5] [:c :v "x"]
        [:d :v true] [:e :v :keyword] [:f :v nil]
        [:g :v #uuid "00000000-0000-0000-0000-000000000001"] [:h :v #{}]])"#,
    )
    .unwrap();
    let result = query(&db, "(query [:find (min ?v) (max ?v) :where [?e :v ?v]])");
    assert_eq!(result.len(), 1);
    assert!(result[0][0] == Value::Float(1.5));
    assert!(matches!(&result[0][1], Value::Set(set) if set.is_empty()));
}

#[test]
fn existing_nil_and_nonnumeric_groups_differ_from_no_matches() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:a :g "nil"] [:a :v nil] [:b :g "text"] [:b :v "x"]])"#)
        .unwrap();
    let prepared = db.prepare("(query [:find (sum ?v) (sum-distinct ?v) (count ?v) (count-distinct ?v) :where [?e :g $g] [?e :v ?v]])").unwrap();
    for (group, count) in [("nil", 0), ("text", 1)] {
        let binds = [("g", BindValue::Val(Value::String(group.into())))];
        let expected = vec![vec![
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(count),
            Value::Integer(count),
        ]];
        assert!(rows(prepared.execute(&binds).unwrap()) == expected);
        let mut cursor = prepared.query(&binds).unwrap();
        assert!(cursor.next_batch(1).unwrap().unwrap().into_rows() == expected);
        assert!(cursor.next_batch(1).unwrap().is_none());
    }
    for f in ["sum", "sum-distinct", "min", "max"] {
        assert!(
            query(
                &db,
                &format!("(query [:find ({f} ?v) :where [?e :missing ?v]])")
            )
            .is_empty()
        );
        assert!(
            query(
                &db,
                &format!("(query [:find ?e ({f} ?v) :where [?e :missing ?v]])")
            )
            .is_empty()
        );
    }
    for f in ["min", "max"] {
        assert!(
            query(
                &db,
                &format!("(query [:find (count ?v) ({f} ?v) :where [?e :g \"nil\"] [?e :v ?v]])")
            )
            .is_empty()
        );
        assert!(
            query(
                &db,
                &format!("(query [:find ({f} ?v) :where [?e :g \"text\"] [?e :v ?v]])")
            ) == vec![vec![Value::String("x".into())]]
        );
    }
    assert!(
        query(
            &db,
            "(query [:find (count ?v) (count-distinct ?v) :with ?e :where [?e :missing ?v]])"
        ) == vec![vec![Value::Integer(0), Value::Integer(0)]]
    );
}

#[test]
fn default_and_bounded_windows_match_each_ordinary_population() {
    let db = Minigraf::in_memory().unwrap();
    // Every variant, repeated numbers and nil occupy real frame positions.
    let values = [
        "nil",
        "10",
        "\"unknown\"",
        "20",
        "true",
        ":k",
        "#{1 2}",
        "#uuid \"00000000-0000-0000-0000-000000000001\"",
        "1.5",
        "10",
        "nil",
    ];
    let facts: String = values
        .iter()
        .enumerate()
        .map(|(n, v)| format!("[:e{n} :v {v}] [:e{n} :ord {n}] [:e{n} :g {}]", n / 6))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    for partitioned in [false, true] {
        for bounded in [false, true] {
            let partition = if partitioned { ":partition-by ?g " } else { "" };
            let frame = if bounded { " :rows 2 preceding" } else { "" };
            for f in ["sum", "min", "max", "count"] {
                let result = query(
                    &db,
                    &format!(
                        "(query [:find ?o ({f} ?v :over ({partition}:order-by ?o{frame})) :where [?e :v ?v] [?e :ord ?o] [?e :g ?g] :order-by [?o]])"
                    ),
                );
                assert_eq!(
                    result.len(),
                    values.len(),
                    "skipped arguments still have output rows"
                );
                for (n, row) in result.iter().enumerate() {
                    let first = if partitioned { n / 6 * 6 } else { 0 };
                    let start = if bounded {
                        n.saturating_sub(2).max(first)
                    } else {
                        first
                    };
                    let ordinary = query(
                        &db,
                        &format!(
                            "(query [:find ({f} ?v) :where [?e :v ?v] [?e :ord ?o] [(>= ?o {start})] [(<= ?o {n})]])"
                        ),
                    );
                    if ordinary.is_empty() {
                        assert!(f == "min" || f == "max");
                        assert!(
                            row[1] == Value::Null,
                            "empty extrema drop groups but retain window rows"
                        );
                    } else {
                        assert!(
                            row[1] == ordinary[0][0],
                            "same function and argument population"
                        );
                    }
                }
            }
        }
    }
    // Independent expectations for the bounded middle frame [10, text, 20].
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) (min ?v) (max ?v) :where [?e :v ?v] [?e :ord ?o] [(>= ?o 1)] [(<= ?o 3)]])"
        ) == vec![vec![
            Value::Integer(30),
            Value::Integer(10),
            Value::String("unknown".into())
        ]]
    );
}

#[test]
fn with_groups_and_join_multiplicity_remain_intact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :v 10] [:a :g "x"] [:b :v 10] [:b :g "x"]
        [:c :v "unknown"] [:c :g "y"] [:t1 :tag true] [:t2 :tag true]])"#,
    )
    .unwrap();
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) (sum-distinct ?v) (count ?v) :where [?e :v ?v] [?t :tag true]])"
        ) == vec![vec![
            Value::Integer(40),
            Value::Integer(10),
            Value::Integer(6)
        ]]
    );
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) :with ?g :where [?e :v ?v] [?e :g ?g]])"
        ) == vec![vec![Value::Integer(20)], vec![Value::Integer(0)]]
    );
}

#[test]
fn rule_and_temporal_queries_use_the_same_aggregate_contract() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :v 10] [:b :v "unknown"] [:c :v 20]
        [:a :next :b] [:b :next :c]])"#,
    )
    .unwrap();
    db.execute("(rule [(walk ?e ?v) [?e :v ?v]])").unwrap();
    db.execute("(rule [(walk ?e ?v) [?e :next ?n] (walk ?n ?v)])")
        .unwrap();
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) (sum-distinct ?v) (min ?v) (max ?v) :where (walk :a ?v)])"
        ) == vec![vec![
            Value::Integer(30),
            Value::Integer(30),
            Value::Integer(10),
            Value::String("unknown".into())
        ]]
    );
    db.execute("(transact [[:d :v true]])").unwrap();
    assert!(
        query(
            &db,
            "(query [:find (sum ?v) (max ?v) :as-of 1 :where [?e :v ?v]])"
        ) == vec![vec![Value::Integer(30), Value::String("unknown".into())]]
    );
    assert!(
        query(&db, "(query [:find (sum ?v) (max ?v) :where [?e :v ?v]])")
            == vec![vec![Value::Integer(30), Value::Boolean(true)]]
    );
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:f :v 5.5]])").unwrap();
    assert!(
        rows(
            tx.execute("(query [:find (sum ?v) (min ?v) :where [?e :v ?v]])")
                .unwrap()
        ) == vec![vec![Value::Float(35.5), Value::Float(5.5)]]
    );
    // Rollback does not retain the staged numeric argument.
    drop(tx);
    assert!(
        query(&db, "(query [:find (sum ?v) :where [?e :v ?v]])") == vec![vec![Value::Integer(30)]]
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn grouped_cursor_and_prepared_queries_survive_checkpoint_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("values.graph");
    let db = Minigraf::open(&path).unwrap();
    let facts: String = (0..1100)
        .map(|n| {
            format!(
                "[:e{n} :g {n}] [:e{n} :v {}]",
                if n % 2 == 0 { "10" } else { "\"unknown\"" }
            )
        })
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let q = "(query [:find ?g (sum ?v) (sum-distinct ?v) (min ?v) (max ?v) :where [?e :g ?g] [?e :v ?v] :order-by [?g]])";
    let expected: Vec<_> = (0..1100)
        .map(|n| {
            let v = if n % 2 == 0 {
                Value::Integer(10)
            } else {
                Value::String("unknown".into())
            };
            let sum = Value::Integer(if n % 2 == 0 { 10 } else { 0 });
            vec![Value::Integer(n), sum.clone(), sum, v.clone(), v]
        })
        .collect();
    let check = |db: &Minigraf| {
        assert!(query(db, q) == expected);
        let prepared = db.prepare(q).unwrap();
        assert!(rows(prepared.execute(&[]).unwrap()) == expected);
        let mut cursor = prepared.query(&[]).unwrap();
        let mut actual = Vec::new();
        while let Some(batch) = cursor.next_batch(17).unwrap() {
            actual.extend(batch.into_rows());
        }
        assert!(actual == expected);
        cursor.close();
    };
    check(&db);
    db.checkpoint().unwrap();
    check(&db);
    drop(db);
    check(&Minigraf::open(&path).unwrap());
}

#[cfg(feature = "test-hooks")]
#[test]
fn value_handling_is_independent_of_join_and_negation_plans() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact [[:a :v 10] [:b :v "x"] [:c :v 1.5]
        [:a :keep true] [:b :keep true] [:c :keep true] [:z :blocked true]])"#,
    )
    .unwrap();
    let expected = vec![vec![
        Value::Float(11.5),
        Value::Float(1.5),
        Value::String("x".into()),
        Value::Integer(3),
    ]];
    for mode in [
        minigraf::Subplans::Choose,
        minigraf::Subplans::Hash,
        minigraf::Subplans::Correlated,
    ] {
        let _mode = minigraf::__subplans(mode);
        for reorder in [false, true] {
            let _order = minigraf::__reorder(reorder);
            assert!(
                query(
                    &db,
                    "(query [:find (sum-distinct ?v) (min ?v) (max ?v) (count-distinct ?v) :where [?e :keep true] [?e :v ?v] (not [?e :blocked true])])"
                ) == expected
            );
        }
    }
}
