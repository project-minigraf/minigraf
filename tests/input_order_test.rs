//! Differential oracle: identical ordered lists with input-order strategies
//! enabled and disabled, across the existing plan switches and snapshots.
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

use minigraf::{Minigraf, QueryResult, Value};

fn rows(db: &Minigraf, q: &str) -> Vec<Vec<Value>> {
    let QueryResult::QueryResults { results, .. } = db.execute(q).unwrap() else {
        panic!("query rows");
    };
    results
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

#[test]
fn ordered_results_are_independent_of_order_modes_and_physical_plans() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("plans.graph")).unwrap();
    let facts: String = (0..128)
        .map(|i| {
            format!(
                "[:e{i} :score {}] [:e{i} :flag {}] [:e{i} :group {}]",
                (i * 37) % 128,
                i % 2 == 0,
                i % 4
            )
        })
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.execute(
        "(transact [[:a :float -0.0] [:b :float 0.0] [:c :float -1.5] \
        [:one :multi 3] [:one :multi 1] [:one :multi 2]])",
    )
    .unwrap();
    db.execute("(rule [(scored ?e ?v) [?e :score ?v]])")
        .unwrap();
    let queries = [
        "(query [:find ?v :where [?e :score ?v] :order-by [?v] :limit 5 :offset 2])",
        "(query [:find ?b :where [?e :flag ?b] :order-by [?b] :limit 5])",
        "(query [:find ?v :where [:one :multi ?v] :order-by [?v]])",
        "(query [:find ?v :where [?e :score ?v] [(>= ?v 20)] :order-by [?v] :limit 5])",
        "(query [:find ?v :where [?e :score ?v] [?e :flag true] :order-by [?v] :limit 5])",
        "(query [:find ?v :where [?e :score ?v] (not [?e :hidden true]) :order-by [?v] :limit 5])",
        "(query [:find ?v :where [?e :score ?v] (or [?e :flag true] [?e :flag false]) :order-by [?v] :limit 5])",
        "(query [:find ?v :where [?e :float ?v] :distinct :order-by [?v]])",
        "(query [:find ?g (count ?e) :where [?e :group ?g] :order-by [?g]])",
        "(query [:find ?g (sum ?v) (avg ?v) (min ?v) (max ?v) (sum-distinct ?v) :where [?e :group ?g] [?e :score ?v] :order-by [?g] :offset 1 :limit 2])",
        "(query [:find ?k ?g (avg ?v) :where [?e :group ?g] [?e :score ?v] [(+ 1 1) ?k] :order-by [?k ?g] :limit 2])",
        "(query [:find ?k (avg ?v) :with ?g :where [?e :group ?g] [?e :score ?v] [(+ 1 1) ?k] :order-by [?k (avg ?v)] :limit 2])",
        "(query [:find ?g ?g (sum-distinct ?v) :where [?e :group ?g] [?e :score ?v] :distinct :order-by [?g] :limit 2])",
        "(query [:find ?g (min ?v) :where [?e :group ?g] (scored ?e ?v) :order-by [?g]])",
        "(query [:find ?g (sum ?v) (avg ?v) :where [?e :group ?g] (not [?e :hidden true]) [?e :score ?v] :order-by [?g]])",
        "(query [:find ?v (rank :over (:partition-by ?e :order-by ?v)) :where [?e :score ?v] :order-by [?v]])",
        "(query [:find ?v :where (scored ?e ?v) :order-by [?v] :limit 5])",
    ];
    for state in 0..4 {
        match state {
            1 => db.checkpoint().unwrap(),
            2 => {
                db.execute("(transact [[:unrelated :name \"pending\"]])")
                    .unwrap();
            }
            3 => {
                db.execute("(transact [[:early :score -1]])").unwrap();
            }
            _ => {}
        }
        for reorder in [false, true] {
            let _reorder = minigraf::__reorder(reorder);
            for mode in [
                minigraf::Subplans::Choose,
                minigraf::Subplans::Hash,
                minigraf::Subplans::Correlated,
            ] {
                let _mode = minigraf::__subplans(mode);
                for query in queries {
                    let expected = {
                        let _off = minigraf::__input_order(false);
                        rows(&db, query)
                    };
                    let actual = {
                        let _on = minigraf::__input_order(true);
                        rows(&db, query)
                    };
                    assert!(
                        same(&actual, &expected),
                        "identical ordered lists across physical strategies"
                    );
                }
            }
        }
    }
}
