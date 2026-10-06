//! Query results over committed history equal those over the same history held
//! in memory (#379). The file-backed database checkpoints between batches, so
//! its committed facts go through the key-level net-assert walk; the in-memory
//! database runs the same commands with every fact pending.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::QueryResult;
use minigraf::Value;
use minigraf::db::Minigraf;

fn rows(db: &Minigraf, query: &str) -> Vec<String> {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => {
            let mut out: Vec<String> = results
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            Value::Integer(n) => n.to_string(),
                            Value::Keyword(k) => k.clone(),
                            Value::Boolean(b) => b.to_string(),
                            Value::Float(f) => f.to_string(),
                            Value::Null => "nil".to_string(),
                            Value::Ref(_) => "ref".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect();
            out.sort();
            out
        }
        _ => panic!("expected query results"),
    }
}

/// Commands in batches; the file-backed database checkpoints after each batch.
fn history() -> Vec<Vec<String>> {
    let long = "a long status description ".repeat(4);
    let mut batches = Vec::new();
    for round in 0..5 {
        let mut b = Vec::new();
        for p in 0..6 {
            let status = (round + p) % 3;
            b.push(format!(
                r#"(transact [[:p{p} :person/status :s{status}] [:p{p} :person/age {}] [:p{p} :person/friend :p{}]])"#,
                20 + round,
                (p + 1) % 6
            ));
            if (round + p) % 2 == 0 {
                b.push(format!(
                    r#"(retract [[:p{p} :person/status :s{status}] [:p{p} :person/age {}]])"#,
                    20 + round
                ));
            }
            b.push(format!(
                r#"(transact {{:valid-from "2023-01-01" :valid-to "2023-06-30"}} [[:p{p} :person/role "{long}{}"]])"#,
                round % 2
            ));
            b.push(format!(
                r#"(transact {{:valid-from "2024-01-01"}} [[:p{p} :person/role "{long}{}"]])"#,
                round % 2
            ));
            if round == 3 {
                b.push(format!(r#"(retract [[:p{p} :person/role "{long}1"]])"#));
            }
        }
        batches.push(b);
    }
    batches
}

fn queries(max_tx: u64) -> Vec<String> {
    let mut q = vec![
        "(query [:find ?s :where [:p1 :person/status ?s]])".to_string(),
        "(query [:find ?a :where [:p2 :person/age ?a]])".to_string(),
        "(query [:find ?attr ?v :where [:p3 ?attr ?v]])".to_string(),
        "(query [:find ?p ?s :where [?p :person/status ?s]])".to_string(),
        "(query [:find ?p ?r :valid-at :any-valid-time :where [?p :person/role ?r]])".to_string(),
        r#"(query [:find ?p ?r :valid-at "2023-03-01" :where [?p :person/role ?r]])"#.to_string(),
        "(query [:find ?p ?f ?a :where [?p :person/friend ?f] [?f :person/age ?a]])".to_string(),
        "(query [:find ?e ?a ?v :where [?e ?a ?v]])".to_string(),
    ];
    for n in [1, max_tx / 3, max_tx / 2, max_tx - 1, max_tx] {
        q.push(format!(
            "(query [:find ?s :as-of {n} :where [:p0 :person/status ?s]])"
        ));
        q.push(format!(
            "(query [:find ?p ?a :as-of {n} :valid-at :any-valid-time :where [?p :person/age ?a]])"
        ));
        q.push(format!(
            "(query [:find ?e ?a ?v :as-of {n} :valid-at :any-valid-time :where [?e ?a ?v]])"
        ));
    }
    q
}

#[test]
fn committed_and_in_memory_histories_answer_alike() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.graph");
    let file = Minigraf::open(&path).unwrap();
    let mem = Minigraf::in_memory().unwrap();
    for (i, batch) in history().iter().enumerate() {
        for cmd in batch {
            file.execute(cmd).unwrap();
            mem.execute(cmd).unwrap();
        }
        // Leave the last batch pending so committed and pending facts mix.
        if i + 1 < history().len() {
            file.checkpoint().unwrap();
        }
    }
    let rule = "(rule [(knows ?a ?b) [?a :person/friend ?b]])";
    file.execute(rule).unwrap();
    mem.execute(rule).unwrap();
    let max_tx = file.current_tx_count();
    assert_eq!(max_tx, mem.current_tx_count());
    let mut all = queries(max_tx);
    all.push("(query [:find ?a ?b :where (knows ?a ?b)])".to_string());
    for q in &all {
        assert_eq!(rows(&file, q), rows(&mem, q), "query: {q}");
    }
    file.checkpoint().unwrap();
    drop(file);
    let file = Minigraf::open(&path).unwrap();
    file.execute(rule).unwrap();
    for q in &all {
        assert_eq!(rows(&file, q), rows(&mem, q), "after reopen: {q}");
    }
}
