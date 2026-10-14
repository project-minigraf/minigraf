//! Targeted shapes for the engine. Every query here also runs on the
//! executor, and the answers must agree (`oracle::check`), so these tests
//! check the engine against the executor over histories chosen to reach each
//! index choice, net-assert across committed and pending records, `:as-of`,
//! valid time, long strings, and cross-kind values.

use super::RUNS;
use crate::QueryResult;
use crate::db::Minigraf;

fn runs() -> usize {
    RUNS.with(std::cell::Cell::get)
}

/// Rows of `query`, which must run on the engine.
fn rows(db: &Minigraf, query: &str) -> usize {
    let before = runs();
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected query results");
    };
    assert_eq!(runs(), before + 1, "query ran on the engine");
    results.len()
}

fn long(tag: u32) -> String {
    format!("{}{tag}", "a long string kept in a value page ".repeat(3))
}

/// One batch of history, numbered `round`: assertions, retractions,
/// re-assertions in new windows, refs, keywords, long strings.
fn write_round(db: &Minigraf, round: u32) {
    for p in 0..6u32 {
        let s = (round + p) % 3;
        db.execute(&format!(
            "(transact [[:p{p} :person/status :s{s}] [:p{p} :person/age {age}] \
             [:p{p} :person/friend :p{f}] [:p{p} :person/name \"n{p}\"] \
             [:p{p} :person/bio \"{bio}\"] [:p{p} :person/ok {ok}] [:p{p} :person/self :p{p}]])",
            age = 20 + round % 4,
            f = (p + round) % 6,
            bio = long(p % 2),
            ok = (round + p) % 2 == 0,
        ))
        .unwrap();
        if (round + p) % 2 == 0 {
            db.execute(&format!(
                "(retract [[:p{p} :person/status :s{s}] [:p{p} :person/age {}]])",
                20 + round % 4
            ))
            .unwrap();
        }
        if (round + p) % 3 == 0 {
            db.execute(&format!(
                "(transact {{:valid-from \"2023-01-01\" :valid-to \"2023-06-30\"}} \
                 [[:p{p} :person/name \"n{p}\"] [:p{p} :person/null nil]])"
            ))
            .unwrap();
        }
    }
}

/// Every query shape the planner takes, at several `:as-of` and valid times.
fn queries(tx: u64) -> Vec<String> {
    let shapes = [
        "[?e ?a ?v]",
        "[?e :person/status ?v]",
        "[?e :person/bio ?v]",
        "[:p1 ?a ?v]",
        "[:p1 :person/age ?v]",
        "[?e :person/name \"n2\"]",
        "[?e :person/age 21]",
        "[?e :person/ok true]",
        "[?e :person/null nil]",
        "[?e ?a 22]",
        "[:p2 ?a \"n2\"]",
        "[?e :person/self ?e]",
        "[?e ?a ?a]",
        "[_ :person/friend ?v]",
        "[?_x :person/age ?v]",
        "[:nobody ?a ?v]",
        "[?e :no/such ?v]",
        "[?e :person/name \"no such name\"]",
    ];
    let mut out = Vec::new();
    for shape in shapes {
        let find = ["?e", "?a", "?v"]
            .into_iter()
            .filter(|v| shape.contains(v))
            .collect::<Vec<_>>();
        if find.is_empty() {
            continue;
        }
        let find = find.join(" ");
        for temporal in [
            String::new(),
            ":valid-at :any-valid-time".to_string(),
            ":valid-at \"2023-03-01\"".to_string(),
            format!(":as-of {} :valid-at :any-valid-time", tx / 2),
            format!(":as-of {}", tx.saturating_sub(1)),
            ":as-of 1".to_string(),
        ] {
            out.push(format!("(query [:find {find} {temporal} :where {shape}])"));
        }
    }
    out
}

fn check_all(db: &Minigraf) -> usize {
    let tx = db.current_tx_count();
    queries(tx).iter().map(|q| rows(db, q)).sum()
}

#[test]
fn engine_matches_executor_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    for round in 0..4 {
        write_round(&db, round);
        assert!(check_all(&db) > 0, "the history answers some queries");
    }
}

#[test]
fn engine_matches_executor_across_checkpoints_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        for round in 0..4 {
            write_round(&db, round);
            // Pending facts on top of committed ones.
            check_all(&db);
            db.checkpoint().unwrap();
            // Committed facts only.
            check_all(&db);
        }
        // Pending records that retract and re-window committed triples.
        write_round(&db, 4);
        check_all(&db);
    }
    let db = Minigraf::open(&path).unwrap();
    assert!(
        check_all(&db) > 0,
        "the reopened history answers some queries"
    );
    db.checkpoint().unwrap();
    check_all(&db);
}

/// Answers of more than one batch, read through the committed and pending
/// halves of a scan.
#[test]
fn engine_reads_many_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("big.graph")).unwrap();
    let facts: String = (0..3000)
        .map(|i| format!("[:e{i} :big/n {i}] [:e{i} :big/tag :t{}]", i % 7))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let more: String = (3000..4500)
        .map(|i| format!("[:e{i} :big/n {i}]"))
        .collect();
    db.execute(&format!("(transact [{more}])")).unwrap();
    db.execute("(retract [[:e5 :big/n 5] [:e3001 :big/n 3001]])")
        .unwrap();
    assert_eq!(
        rows(&db, "(query [:find ?e ?n :where [?e :big/n ?n]])"),
        4498
    );
    assert_eq!(
        rows(&db, "(query [:find ?e ?a ?v :where [?e ?a ?v]])"),
        7498
    );
    assert_eq!(rows(&db, "(query [:find ?t :where [:e3 :big/tag ?t]])"), 1);
    assert_eq!(rows(&db, "(query [:find ?e :where [?e :big/n 2999]])"), 1);
}

/// The executor still runs what the engine does not: two patterns, rules,
/// aggregates, `:as-of` a timestamp.
#[test]
fn other_queries_stay_on_the_executor() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    for q in [
        "(query [:find ?e ?n :where [?e :person/name ?n] [?e :person/age ?a]])",
        "(query [:find (count ?e) :where [?e :person/name ?n]])",
        "(query [:find ?e :as-of \"2099-01-01T00:00:00Z\" :where [?e :person/name ?n]])",
        "(query [:find ?v :where [?e :person/status ?v] [(= ?v :s1)]])",
        "(query [:find ?e :where [?e :person/status :s1]])",
    ] {
        let before = runs();
        db.execute(q).unwrap();
        assert_eq!(runs(), before, "query stayed on the executor");
    }
}
