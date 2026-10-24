//! `:limit` (#306, spec `docs/superpowers/specs/2026-10-24-limit-order-by-design.md`).
//!
//! Without `:order-by`, `:limit n` returns `min(n, |answer|)` rows, and
//! they are a sub-bag of the answer the query gives without it: which rows is
//! unspecified (spec §1, decision 2). Each shape runs through
//! `Minigraf::execute`, a `Minigraf::query` cursor and a prepared `:limit $n`,
//! with clause reordering on and off (`minigraf::__reorder`).
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

use minigraf::{BindValue, Minigraf, QueryResult, Value};
use std::collections::HashMap;

/// A value as text, without `Debug` (CodeQL `rust/cleartext-logging`).
fn key(v: &Value) -> String {
    match v {
        Value::String(s) => format!("s:{s}"),
        Value::Integer(i) => format!("i:{i}"),
        Value::Float(f) => format!("f:{:016x}", f.to_bits()),
        Value::Boolean(b) => format!("b:{b}"),
        Value::Ref(u) => format!("r:{}", u.as_hyphenated()),
        Value::Keyword(k) => format!("k:{k}"),
        Value::Null => "nil".to_string(),
    }
}

fn keys(rows: &[Vec<Value>]) -> Vec<Vec<String>> {
    rows.iter().map(|r| r.iter().map(key).collect()).collect()
}

fn results(r: QueryResult) -> Vec<Vec<String>> {
    match r {
        QueryResult::QueryResults { results, .. } => keys(&results),
        _ => panic!("expected QueryResults"),
    }
}

/// Whether every row of `part` is in `whole` at least as many times.
fn is_sub_bag(part: &[Vec<String>], whole: &[Vec<String>]) -> bool {
    let mut left: HashMap<&Vec<String>, usize> = HashMap::new();
    for r in whole {
        *left.entry(r).or_default() += 1;
    }
    part.iter().all(|r| match left.get_mut(r) {
        Some(n) if *n > 0 => {
            *n -= 1;
            true
        }
        _ => false,
    })
}

/// 50 people in five `:follows` chains, over two transactions, the first checkpointed on a file
/// database, so answers mix committed and pending facts.
fn people(db: &Minigraf, checkpoint: bool) {
    let facts: String = (0..50)
        .map(|i| {
            format!(
                "[:p{i} :name \"n{i}\"] [:p{i} :age {}] [:p{i} :status {}] [:p{i} :follows :p{}]",
                i % 7,
                if i % 3 == 0 { ":inactive" } else { ":active" },
                // Chains of 10, so `reach` stays small.
                if i % 10 == 9 { i } else { i + 1 }
            )
        })
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    if checkpoint {
        db.checkpoint().unwrap();
    }
    let more: String = (50..60)
        .map(|i| {
            format!(
                "[:p{i} :name \"n{i}\"] [:p{i} :age {}] [:p{i} :status :active]",
                i % 7
            )
        })
        .collect();
    db.execute(&format!("(transact [{more}])")).unwrap();
    db.execute("(retract [[:p1 :age 1]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :follows ?y]])")
        .unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :follows ?z] (reach ?z ?y)])")
        .unwrap();
}

/// Query bodies: every shape `plan_with` builds before its `Limit`.
const SHAPES: &[&str] = &[
    ":find ?e ?a :where [?e :age ?a]",
    ":find ?a :where [?e :age ?a]",
    ":find ?n ?f :where [?e :name ?n] [?e :follows ?f]",
    ":find ?e :where [?e :age ?a] (not [?e :status :inactive])",
    ":find ?e :where (or [?e :status :inactive] [?e :age 3])",
    ":find ?s (count ?e) :where [?e :status ?s]",
    ":find (count ?e) :where [?e :age ?a]",
    ":find (count ?e) :where [?e :no-such ?a]",
    ":find ?e (rank :over (:order-by ?a)) :where [?e :age ?a]",
    ":find ?y :where (reach :p0 ?y)",
    ":find ?x ?y :where (reach ?x ?y)",
    ":find ?e :as-of 1 :where [?e :age ?a]",
    ":find ?e ?a :any-valid-time :where [?e :age ?a]",
    ":find ?x :where [(+ 1 2) ?x]",
];

fn check(db: &Minigraf) {
    for body in SHAPES {
        for reorder in [true, false] {
            let _g = minigraf::__reorder(reorder);
            let answer = results(db.execute(&format!("(query [{body}])")).expect(body));
            let total = answer.len();
            let prepared = db
                .prepare(&format!("(query [{body} :limit $n])"))
                .expect(body);
            for n in [0, 1, 2, total.saturating_sub(1), total, total + 5] {
                let q = format!("(query [{body} :limit {n}])");
                let executed = results(db.execute(&q).expect(&q));

                let mut cursor = db.query(&q).expect(&q);
                let mut pulled = Vec::new();
                while let Some(batch) = cursor.next_batch(1).expect("batch") {
                    pulled.extend(keys(batch.rows()));
                }

                let bind = [("n", BindValue::Val(Value::Integer(n as i64)))];
                let from_prepared = results(prepared.execute(&bind).expect(&q));
                let mut cursor = prepared.query(&bind).expect(&q);
                let mut prepared_pulled = Vec::new();
                while let Some(batch) = cursor.next_batch(3).expect("batch") {
                    prepared_pulled.extend(keys(batch.rows()));
                }

                for (how, rows) in [
                    ("execute", &executed),
                    ("cursor", &pulled),
                    ("prepared", &from_prepared),
                    ("prepared cursor", &prepared_pulled),
                ] {
                    assert_eq!(rows.len(), n.min(total), "{q} via {how}, reorder {reorder}");
                    assert!(
                        is_sub_bag(rows, &answer),
                        "{q} via {how}, reorder {reorder}: rows not in the answer"
                    );
                }
            }
        }
    }
}

#[test]
fn limit_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    check(&db);
}

#[test]
fn limit_on_committed_and_pending_facts() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("limit.graph")).unwrap();
    people(&db, true);
    check(&db);
}

/// A `count`-only query with no input gives its zero row; `:limit 0`
/// removes it.
#[test]
fn limit_applies_to_the_zero_row() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let q = |n: usize| format!("(query [:find (count ?e) :where [?e :no-such ?a] :limit {n}])");
    assert_eq!(results(db.execute(&q(1)).unwrap()), [["i:0"]]);
    assert!(results(db.execute(&q(0)).unwrap()).is_empty());
}

/// A write transaction's own staged facts count toward its answer.
#[test]
fn limit_in_a_write_transaction() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:q0 :age 100] [:q1 :age 100] [:q2 :age 100]])")
        .unwrap();
    let all = results(
        tx.execute("(query [:find ?e :where [?e :age 100]])")
            .unwrap(),
    );
    assert_eq!(all.len(), 3);
    let some = results(
        tx.execute("(query [:find ?e :where [?e :age 100] :limit 2])")
            .unwrap(),
    );
    assert_eq!(some.len(), 2);
    assert!(is_sub_bag(&some, &all));
    tx.commit().unwrap();
}

/// An unbound `:limit $n` in `execute` is `API-010`; a bad bind value is
/// rejected at execute.
#[test]
fn limit_slot_errors() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let q = "(query [:find ?e :where [?e :age ?a] :limit $n])";
    assert_eq!(db.execute(q).expect_err("unbound").code(), "API-010");
    let p = db.prepare(q).unwrap();
    let code = |b: BindValue| p.execute(&[("n", b)]).expect_err("rejected").code();
    assert_eq!(code(BindValue::Val(Value::Integer(-2))), "PRS-115");
    assert_eq!(code(BindValue::Val(Value::String("2".into()))), "INT-035");
    assert_eq!(code(BindValue::TxCount(2)), "INT-035");
}
