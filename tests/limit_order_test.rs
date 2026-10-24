//! `:limit`, `:offset` and `:order-by` (#306, spec `docs/superpowers/specs/2026-10-24-limit-order-by-design.md`).
//!
//! Without `:order-by`, `:limit n` returns `min(n, |answer|)` rows, and
//! they are a sub-bag of the answer the query gives without it: which rows is
//! unspecified (spec §1, decision 2). Each shape runs through
//! `Minigraf::execute`, a `Minigraf::query` cursor and a prepared `:limit $n`,
//! with clause reordering on and off (`minigraf::__reorder`).
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

use minigraf::{BindValue, Minigraf, QueryResult, Value};
use std::cmp::Ordering;
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
        Value::Set(s) => format!("S:{{{}}}", s.iter().map(key).collect::<Vec<_>>().join(" ")),
        _ => "?".to_string(),
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
    assert_eq!(code(BindValue::Val(Value::String("2".into()))), "API-024");
    assert_eq!(code(BindValue::TxCount(2)), "API-024");
}

// ── :order-by and :offset ─────────────────────────────────────────────────

/// The class of a value in the full order (spec §2).
fn class(v: &Value) -> u8 {
    match v {
        Value::Integer(_) | Value::Float(_) => 0,
        Value::String(_) => 1,
        Value::Boolean(_) => 2,
        Value::Ref(_) => 3,
        Value::Keyword(_) => 4,
        Value::Null => 5,
        _ => 6,
    }
}

/// The test's own order of values: numbers numerically (an integer before
/// an equal float, `-0.0` before `0.0`), then by class.
fn order(a: &Value, b: &Value) -> Ordering {
    let num = |v: &Value| match v {
        Value::Integer(i) => Some((*i as f64, 0)),
        Value::Float(f) => Some((*f, 1)),
        _ => None,
    };
    match (a, b, num(a), num(b)) {
        (_, _, Some((x, kx)), Some((y, ky))) => x
            .partial_cmp(&y)
            .unwrap()
            .then(kx.cmp(&ky))
            .then(x.total_cmp(&y)),
        (Value::String(x), Value::String(y), ..) => x.cmp(y),
        (Value::Boolean(x), Value::Boolean(y), ..) => x.cmp(y),
        (Value::Ref(x), Value::Ref(y), ..) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Keyword(x), Value::Keyword(y), ..) => x.cmp(y),
        _ => class(a).cmp(&class(b)),
    }
}

/// `rows` in the full order: `keys` (column, descending), then every other
/// column ascending.
fn full_sort(mut rows: Vec<Vec<Value>>, keys: &[(usize, bool)]) -> Vec<Vec<Value>> {
    let width = rows.first().map_or(0, Vec::len);
    let mut all = keys.to_vec();
    for c in 0..width {
        if !all.iter().any(|&(k, _)| k == c) {
            all.push((c, false));
        }
    }
    rows.sort_by(|a, b| {
        all.iter()
            .map(|&(c, desc)| {
                let o = order(&a[c], &b[c]);
                if desc { o.reverse() } else { o }
            })
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    rows
}

fn values(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

/// Ordered shapes: a body, its `:order-by` vector, and the keys as
/// `(column, descending)`.
const ORDERED: &[(&str, &str, &[(usize, bool)])] = &[
    (
        ":find ?e ?a :where [?e :age ?a]",
        "[?a :desc]",
        &[(1, true)],
    ),
    (":find ?a :where [?e :age ?a]", "[?a]", &[(0, false)]),
    (
        ":find ?n ?f :where [?e :name ?n] [?e :follows ?f]",
        "[?f :desc ?n]",
        &[(1, true), (0, false)],
    ),
    (
        ":find ?e :where [?e :age ?a] (not [?e :status :inactive])",
        "[?e :desc]",
        &[(0, true)],
    ),
    (
        ":find ?e :where (or [?e :status :inactive] [?e :age 3])",
        "[?e]",
        &[(0, false)],
    ),
    (
        ":find ?s (count ?e) :where [?e :status ?s]",
        "[(count ?e) :desc]",
        &[(1, true)],
    ),
    (
        ":find (count ?e) :where [?e :age ?a]",
        "[(count ?e)]",
        &[(0, false)],
    ),
    (
        ":find (count ?e) :where [?e :no-such ?a]",
        "[(count ?e) :desc]",
        &[(0, true)],
    ),
    (
        ":find ?e (rank :over (:order-by ?a)) :where [?e :age ?a]",
        "[(rank :over (:order-by ?a)) :desc]",
        &[(1, true)],
    ),
    (":find ?y :where (reach :p0 ?y)", "[?y :desc]", &[(0, true)]),
    (
        ":find ?x ?y :where (reach ?x ?y)",
        "[?y ?x :desc]",
        &[(1, false), (0, true)],
    ),
    (
        ":find ?e :as-of 1 :where [?e :age ?a]",
        "[?e]",
        &[(0, false)],
    ),
    (
        ":find ?e ?a :any-valid-time :where [?e :age ?a]",
        "[?a :desc]",
        &[(1, true)],
    ),
    (":find ?x :where [(+ 1 2) ?x]", "[?x]", &[(0, false)]),
];

/// Every row of `cursor`, `n` at a time.
fn drain(mut cursor: minigraf::Cursor, n: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    while let Some(batch) = cursor.next_batch(n).expect("batch") {
        rows.extend(keys(batch.rows()));
    }
    rows
}

fn check_ordered(db: &Minigraf) {
    for (body, by, sort_keys) in ORDERED {
        let mut lists: Vec<Vec<Vec<String>>> = Vec::new();
        for reorder in [true, false] {
            let _g = minigraf::__reorder(reorder);
            let answer = values(db.execute(&format!("(query [{body}])")).expect(body));
            let sorted = keys(&full_sort(answer, sort_keys));
            let total = sorted.len();
            let prepared = db
                .prepare(&format!(
                    "(query [{body} :order-by {by} :offset $o :limit $n])"
                ))
                .expect(body);
            let mut cases: Vec<(Option<usize>, Option<usize>)> = vec![(None, None)];
            for n in [0, 1, 3, total] {
                cases.push((None, Some(n)));
                for o in [1, total.saturating_sub(1), total + 2] {
                    cases.push((Some(o), Some(n)));
                }
            }
            cases.push((Some(2), None));
            for (offset, limit) in cases {
                let mut q = format!("{body} :order-by {by}");
                if let Some(o) = offset {
                    q.push_str(&format!(" :offset {o}"));
                }
                if let Some(n) = limit {
                    q.push_str(&format!(" :limit {n}"));
                }
                let q = format!("(query [{q}])");
                let from = offset.unwrap_or(0).min(total);
                let to = limit.map_or(total, |n| (from + n).min(total));
                let want = &sorted[from..to];
                let executed = keys(&values(db.execute(&q).expect(&q)));
                let pulled = drain(db.query(&q).expect(&q), 1);
                assert_eq!(executed, want, "{q} via execute, reorder {reorder}");
                assert_eq!(pulled, want, "{q} via cursor, reorder {reorder}");
                if let Some(n) = limit {
                    let bind = [
                        (
                            "o",
                            BindValue::Val(Value::Integer(offset.unwrap_or(0) as i64)),
                        ),
                        ("n", BindValue::Val(Value::Integer(n as i64))),
                    ];
                    let got = keys(&values(prepared.execute(&bind).expect(&q)));
                    assert_eq!(got, want, "{q} via prepared, reorder {reorder}");
                    let got = drain(prepared.query(&bind).expect(&q), 3);
                    assert_eq!(got, want, "{q} via prepared cursor, reorder {reorder}");
                }
            }
            lists.push(sorted);
        }
        assert_eq!(
            lists[0], lists[1],
            "{body}: one order with reordering on and off"
        );
    }
}

#[test]
fn order_by_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    check_ordered(&db);
}

#[test]
fn order_by_on_committed_and_pending_facts() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("order.graph")).unwrap();
    people(&db, true);
    check_ordered(&db);
}

/// Pages of 3 read the whole ordered answer once each.
#[test]
fn pages_cover_the_ordered_answer() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let body = ":find ?n ?a :where [?e :name ?n] [?e :age ?a] :order-by [?a :desc]";
    let whole = results(db.execute(&format!("(query [{body}])")).unwrap());
    let mut paged = Vec::new();
    for page in 0.. {
        let q = format!("(query [{body} :offset {} :limit 3])", page * 3);
        let rows = results(db.execute(&q).unwrap());
        if rows.is_empty() {
            break;
        }
        paged.extend(rows);
    }
    assert_eq!(paged, whole);
    assert_eq!(whole.len(), 59, "every person with an age");
}

/// Values of every class in one column, ascending and descending.
#[test]
fn order_by_every_value_class() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact [[:v0 :v 1] [:v1 :v 1.0] [:v2 :v -0.0] [:v3 :v 0.0] [:v4 :v -5] \
         [:v5 :v 2.5] [:v6 :v \"a\"] [:v7 :v \"b\"] [:v8 :v true] [:v9 :v false] \
         [:v10 :v :k/a] [:v11 :v :k/b] [:v12 :v #{1 2}] \
         [:v13 :v #uuid \"550e8400-e29b-41d4-a716-446655440000\"]])",
    )
    .unwrap();
    let answer = values(db.execute("(query [:find ?v :where [?e :v ?v]])").unwrap());
    assert_eq!(answer.len(), 14);
    for (by, desc) in [("[?v]", false), ("[?v :desc]", true)] {
        let got = keys(&values(
            db.execute(&format!(
                "(query [:find ?v :where [?e :v ?v] :order-by {by}])"
            ))
            .unwrap(),
        ));
        let want = keys(&full_sort(answer.clone(), &[(0, desc)]));
        assert_eq!(got, want, "{by}");
    }
    let first = keys(&values(
        db.execute("(query [:find ?v :where [?e :v ?v] :order-by [?v] :limit 4])")
            .unwrap(),
    ));
    assert_eq!(
        first,
        [
            ["i:-5"],
            ["f:8000000000000000"],
            ["f:0000000000000000"],
            ["i:1"]
        ]
    );
}

/// A write transaction's staged facts are ordered with the rest.
#[test]
fn order_by_in_a_write_transaction() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:q0 :age 100] [:q1 :age 101]])")
        .unwrap();
    let top = results(
        tx.execute("(query [:find ?a :where [?e :age ?a] :order-by [?a :desc] :limit 2])")
            .unwrap(),
    );
    assert_eq!(top, [["i:101"], ["i:100"]]);
    tx.commit().unwrap();
}

/// `:offset` slots: unbound is `API-010`, a bad value is rejected at execute.
#[test]
fn offset_slot_errors() {
    let db = Minigraf::in_memory().unwrap();
    people(&db, false);
    let q = "(query [:find ?e :where [?e :age ?a] :order-by [?e] :offset $o])";
    assert_eq!(db.execute(q).expect_err("unbound").code(), "API-010");
    let p = db.prepare(q).unwrap();
    let code = |b: BindValue| p.execute(&[("o", b)]).expect_err("rejected").code();
    assert_eq!(code(BindValue::Val(Value::Integer(-2))), "PRS-115");
    assert_eq!(code(BindValue::TxCount(2)), "API-024");
}

/// A full sort holds the answer; top-k under a `:limit` holds `k` rows.
#[test]
fn sort_memory_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sort.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        let facts: String = (0..20_000).map(|i| format!("[:p{i} :a {i}]")).collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
        db.checkpoint().unwrap();
    }
    let db = minigraf::OpenOptions::new()
        .query_memory_limit(512 * 1024)
        .path(&path)
        .open()
        .unwrap();
    let e = db
        .execute("(query [:find ?e ?v :where [?e :a ?v] :order-by [?v]])")
        .expect_err("over the limit");
    assert_eq!(e.code(), "QRY-012");
    let top = results(
        db.execute("(query [:find ?v :where [?e :a ?v] :order-by [?v :desc] :limit 2])")
            .unwrap(),
    );
    assert_eq!(top, [["i:19999"], ["i:19998"]]);
}
