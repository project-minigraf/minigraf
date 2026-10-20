//! Rule queries on the streaming engine (#548 spec §7). Each query runs on
//! an in-memory database, on a file with its facts pending, and on the same
//! file after a checkpoint; under the `test-oracle` feature the executor
//! runs it too and must agree.
//
// File-backed (Minigraf::open, tempfile): native only, like the other file tests.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult, Value};

/// The databases a test reads: in memory, a file, and the file again once
/// checkpointed.
struct Dbs {
    mem: Minigraf,
    file: Minigraf,
    _dir: tempfile::TempDir,
}

impl Dbs {
    fn new(commands: &[&str]) -> Dbs {
        let dir = tempfile::tempdir().unwrap();
        let mem = Minigraf::in_memory().unwrap();
        let file = Minigraf::open(dir.path().join("rules.graph")).unwrap();
        for db in [&mem, &file] {
            for c in commands {
                db.execute(c).expect("setup command");
            }
        }
        Dbs {
            mem,
            file,
            _dir: dir,
        }
    }

    /// The rows of `query`, the same on every database.
    fn rows(&self, query: &str) -> Vec<Vec<Value>> {
        let mut all = Vec::new();
        for db in [&self.mem, &self.file] {
            all.push(sorted(rows(db, query)));
        }
        self.file.checkpoint().unwrap();
        all.push(sorted(rows(&self.file, query)));
        assert!(all[0] == all[1], "in memory and pending agree");
        assert!(all[1] == all[2], "pending and committed agree");
        all.swap_remove(0)
    }

    fn count(&self, query: &str) -> usize {
        self.rows(query).len()
    }

    /// The error code of `query`, the same on every database.
    fn err(&self, query: &str) -> String {
        let codes: Vec<String> = [&self.mem, &self.file]
            .iter()
            .map(|db| match db.execute(query) {
                Ok(_) => panic!("expected an error"),
                Err(e) => e.code().to_string(),
            })
            .collect();
        assert_eq!(codes[0], codes[1], "same error on both databases");
        codes[0].clone()
    }
}

fn rows(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    match db.execute(query).expect("query") {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    }
}

fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

const CHAIN: &str = "(transact [[:a :e :b] [:b :e :c] [:c :e :d]])";

#[test]
fn linear_and_non_linear_transitive_closure() {
    let db = Dbs::new(&[
        CHAIN,
        "(rule [(reach ?x ?y) [?x :e ?y]])",
        "(rule [(reach ?x ?z) [?x :e ?y] (reach ?y ?z)])",
        "(rule [(tc ?x ?y) [?x :e ?y]])",
        "(rule [(tc ?x ?z) (tc ?x ?y) (tc ?y ?z)])",
    ]);
    assert_eq!(db.count("(query [:find ?x ?y :where (reach ?x ?y)])"), 6);
    assert_eq!(db.count("(query [:find ?x ?y :where (tc ?x ?y)])"), 6);
    assert_eq!(
        db.count("(query [:find ?x ?y :where (reach ?x ?y) (tc ?x ?y)])"),
        6,
        "both relations in one body"
    );
}

#[test]
fn mutual_recursion() {
    let db = Dbs::new(&[
        "(transact [[:n0 :next :n1] [:n1 :next :n2] [:n2 :next :n3] [:n3 :next :n4] [:n4 :next :n5]])",
        "(rule [(odd ?x ?y) [?x :next ?y]])",
        "(rule [(odd ?x ?z) (even ?x ?y) [?y :next ?z]])",
        "(rule [(even ?x ?z) (odd ?x ?y) [?y :next ?z]])",
    ]);
    assert_eq!(db.count("(query [:find ?x ?y :where (odd ?x ?y)])"), 9);
    assert_eq!(db.count("(query [:find ?x ?y :where (even ?x ?y)])"), 6);
}

/// #633's cases: a stratum whose `not` and `or` rules recurse.
#[test]
fn recursion_through_not_and_or_rules() {
    let db = Dbs::new(&[
        CHAIN,
        "(transact [[:c :blocked true] [:b :f :x]])",
        "(rule [(r ?x ?y) [?x :e ?y]])",
        "(rule [(r ?x ?z) (r ?x ?y) [?y :e ?z] (not [?z :blocked true])])",
        "(rule [(s ?x ?y) (or [?x :e ?y] [?x :f ?y])])",
        "(rule [(s ?x ?z) (s ?x ?y) (or [?y :e ?z] [?y :f ?z])])",
        // Mutual recursion through a mixed rule.
        "(rule [(m ?x ?y) [?x :e ?y]])",
        "(rule [(m ?x ?z) (n ?x ?y) [?y :e ?z]])",
        "(rule [(n ?x ?y) (m ?x ?y) (not [?y :blocked true])])",
    ]);
    // a-b, b-c, c-d and b-d; a-c and a-d go through the blocked c.
    assert_eq!(db.count("(query [:find ?x ?y :where (r ?x ?y)])"), 4);
    // Every pair of the chain, and a-x, b-x.
    assert_eq!(db.count("(query [:find ?x ?y :where (s ?x ?y)])"), 8);
    // m: the three edges, a-c (n a-b, b-e-c) and b-d? n b-c is blocked: no.
    assert_eq!(db.count("(query [:find ?x ?y :where (m ?x ?y)])"), 4);
}

/// Rules that read their own stratum other than through a top-level
/// invocation run in full every round (spec §5.3).
#[test]
fn naive_rules() {
    let db = Dbs::new(&[
        CHAIN,
        "(transact [[:b :f :z]])",
        "(rule [(o ?x ?y) [?x :e ?y]])",
        "(rule [(o ?x ?z) [?x :e ?y] (or (o ?y ?z) [?y :f ?z])])",
        "(rule [(p ?x ?y) [?x :e ?y]])",
        "(rule [(p ?x ?z) [?x :e ?y] [?y :p ?z]])",
        "(rule [(q ?x ?y) [?x :e ?y]])",
        "(rule [(q ?x ?z) [?x :e ?y] [?y ?a ?z] [(= ?a :q)]])",
    ]);
    // The chain's 6 pairs, and a-z through b's :f.
    assert_eq!(db.count("(query [:find ?x ?y :where (o ?x ?y)])"), 7);
    assert_eq!(db.count("(query [:find ?x ?y :where (p ?x ?y)])"), 6);
    assert_eq!(db.count("(query [:find ?x ?y :where (q ?x ?y)])"), 6);
}

/// Derived facts are visible as stored ones (spec §3.1).
#[test]
fn merged_visibility() {
    let db = Dbs::new(&[
        CHAIN,
        "(transact [[:x :reach :y]])",
        "(rule [(reach ?x ?y) [?x :e ?y]])",
        "(rule [(reach ?x ?z) [?x :e ?y] (reach ?y ?z)])",
    ]);
    assert_eq!(
        db.count("(query [:find ?x ?y :where (reach ?x ?y)])"),
        7,
        "the stored :reach fact too"
    );
    let attrs: std::collections::BTreeSet<String> = db
        .rows("(query [:find ?a :where (reach :a ?y) [:a ?a ?v]])")
        .into_iter()
        .filter_map(|r| match r.first() {
            Some(Value::Keyword(k)) => Some(k.clone()),
            _ => None,
        })
        .collect();
    assert!(
        attrs.contains(":e") && attrs.contains(":reach") && attrs.len() == 3,
        ":e, the derived :reach and the magic seed's attribute"
    );
    assert_eq!(
        db.count("(query [:find ?y :where (reach ?x ?y) [?x :reach ?y]])"),
        7,
        "a pattern of the relation reads the derived facts"
    );
    // With a literal, the magic-set rewrite runs: only a's reach is derived.
    assert_eq!(db.count("(query [:find ?y :where (reach :a ?y)])"), 3);
    assert_eq!(
        db.count("(query [:find ?y :where (reach :b ?y) (not [:x :reach ?y])])"),
        2
    );
}

/// Relations are sets, values compared as stored (spec §3.2).
#[test]
fn identity_of_derived_facts() {
    let alice = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b":alice");
    let long = "a long string kept in a value page ".repeat(3);
    let db = Dbs::new(&[
        &format!(
            r#"(transact [[:k :kw :alice] [:k :rf #uuid "{alice}"] [:k :f 0.0] [:k :g -0.0]
                          [:k :s "{long}"] [:k :t "{long}"] [:k :p 1]])"#
        ),
        "(rule [(v ?x ?v) [?x :kw ?v]])",
        "(rule [(v ?x ?v) [?x :rf ?v]])",
        "(rule [(fl ?x ?v) [?x :f ?v]])",
        "(rule [(fl ?x ?v) [?x :g ?v]])",
        "(rule [(st ?x ?v) [?x :s ?v]])",
        "(rule [(st ?x ?v) [?x :t ?v]])",
        "(rule [(p ?x ?v) [?x :e ?v]])",
        "(rule [(p ?x ?one) [?x :kw ?v] [(+ 0 1) ?one]])",
    ]);
    assert_eq!(
        db.count("(query [:find ?v :where (v :k ?v)])"),
        2,
        "a keyword is not a ref"
    );
    assert_eq!(
        db.count("(query [:find ?x ?v :where (fl ?x ?v)])"),
        2,
        "-0.0 is not 0.0"
    );
    assert_eq!(
        db.count("(query [:find ?v :where (st :k ?v)])"),
        1,
        "one long string"
    );
    let QueryResult::QueryResults { results, .. } = db
        .mem
        .execute("(query [:find (count ?v) :where (p :k ?v)])")
        .unwrap()
    else {
        panic!("expected query results");
    };
    assert_eq!(
        results,
        vec![vec![Value::Integer(1)]],
        "a head equal to a stored fact is not derived again"
    );
}

/// A derived fact has no metadata (spec §3.3).
#[test]
fn derived_metadata_is_nil() {
    let db = Dbs::new(&[
        CHAIN,
        "(rule [(reach ?x ?y) [?x :e ?y]])",
        "(rule [(reach ?x ?z) [?x :e ?y] (reach ?y ?z)])",
    ]);
    let rows = db.rows(
        "(query [:find ?x ?y ?t :any-valid-time :where (reach ?x ?y) [?x :reach ?y] [?x :db/tx-count ?t]])",
    );
    assert_eq!(rows.len(), 6);
    assert!(rows.iter().all(|r| r[2] == Value::Null), "nil metadata");
    assert_eq!(
        db.count(
            "(query [:find ?x :any-valid-time :where (reach ?x ?y) [?x :reach ?y] [?x :db/tx-count 1]])"
        ),
        0,
        "a constant never matches nil"
    );
    assert_eq!(
        db.count("(query [:find ?x ?y :where (reach ?x ?y) [?x :db/valid-at ?v]])"),
        20,
        ":db/valid-at joins every fact of ?x, derived ones too: a 3 x 4, b 2 x 3, c 1 x 2"
    );
}

#[test]
fn heads() {
    let db = Dbs::new(&[
        CHAIN,
        r#"(transact [[:a :name "a"] [:b :ok true]])"#,
        "(rule [(flag ?x) [?x :ok true]])",
        "(rule [(bad ?n ?x) [?x :name ?n]])",
        "(rule [(gap ?x ?y) [?x :e ?z]])",
        "(rule [(kw ?y ?x) [?x :e ?y]])",
    ]);
    assert_eq!(
        db.rows("(query [:find ?x ?v :where (flag ?x) [?x :flag ?v]])")
            .first()
            .map(|r| r[1].clone()),
        Some(Value::Boolean(true)),
        "a one-argument head derives true"
    );
    assert_eq!(
        db.err("(query [:find ?x :where (bad ?x ?y)])"),
        "QRY-013",
        "a string is not an entity"
    );
    assert_eq!(
        db.err("(query [:find ?x :where (gap ?x ?y)])"),
        "INT-022",
        "an unbound head variable"
    );
    assert_eq!(
        db.count("(query [:find ?x ?y :where (kw ?x ?y)])"),
        3,
        "a keyword head entity names an entity"
    );
}

#[test]
fn limits() {
    let db = Dbs::new(&[
        "(transact [[:c :zero 0]])",
        "(rule [(cnt ?c ?n) [?c :zero ?n]])",
        "(rule [(cnt ?c ?m) (cnt ?c ?n) [(+ ?n 1) ?m]])",
    ]);
    assert_eq!(
        db.err("(query [:find ?n :where (cnt :c ?n)])"),
        "INT-020",
        "rounds"
    );
    assert_eq!(
        db.err("(query [:find ?n :where (cnt ?c ?n) :max-derived-facts 5])"),
        "INT-020",
        "derived facts"
    );
}

#[test]
fn rule_queries_with_every_operator() {
    let db = Dbs::new(&[
        CHAIN,
        "(transact [[:a :w 1] [:b :w 2] [:c :w 3] [:d :w 4]])",
        "(rule [(reach ?x ?y) [?x :e ?y]])",
        "(rule [(reach ?x ?z) [?x :e ?y] (reach ?y ?z)])",
    ]);
    assert_eq!(
        db.rows("(query [:find ?x (count ?y) :where (reach ?x ?y)])")
            .len(),
        3,
        "aggregate"
    );
    assert_eq!(
        db.count("(query [:find ?x ?y :where (reach ?x ?y) [?y :w ?w] [(> ?w 2)]])"),
        5,
        "expression"
    );
    assert_eq!(
        db.count("(query [:find ?x :where [?x :w ?w] (not-join [?x] (reach ?x ?y))])"),
        1,
        "not of an invocation"
    );
    assert_eq!(
        db.count("(query [:find ?x ?y :where [?x :w ?w] (or (reach ?x ?y) [?x :e ?y])])"),
        6,
        "or of an invocation"
    );
    assert_eq!(
        db.count("(query [:find ?x ?y (sum ?w :over (:partition-by ?x :order-by ?y)) :where (reach ?x ?y) [?y :w ?w]])"),
        6,
        "window"
    );
}
