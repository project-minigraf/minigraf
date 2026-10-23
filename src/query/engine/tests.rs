//! Targeted shapes for the engine. Every query here also runs on the
//! executor, and the answers must agree (`oracle::check`), so these tests
//! check the engine against the executor over histories chosen to reach each
//! index choice, net-assert across committed and pending records, `:as-of`,
//! valid time, long strings, and cross-kind values.

use super::{Cause, FALLBACKS, RUNS};
use crate::QueryResult;
use crate::db::Minigraf;

fn runs() -> usize {
    RUNS.with(std::cell::Cell::get)
}

/// Rows of `query`, which must run on the engine.
fn rows(db: &Minigraf, query: &str) -> usize {
    let before = runs();
    let QueryResult::QueryResults { results, .. } = db.execute(query).expect(query) else {
        panic!("expected query results");
    };
    assert_eq!(runs(), before + 1, "query ran on the engine");
    results.len()
}

fn long(tag: u32) -> String {
    format!("{}{tag}", "a long string kept in a value page ".repeat(3))
}

/// The UUID a keyword entity such as `:p1` stands for.
fn uuid_of(entity: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, entity.as_bytes())
}

/// One batch of history, numbered `round`: assertions, retractions,
/// re-assertions in new windows, refs, keywords (one naming an entity by its
/// UUID, one naming an attribute), floats, long strings, attributes that
/// name entities.
fn write_round(db: &Minigraf, round: u32) {
    let scores = ["0.30000000000000004", "1.5", "-0.0", "0.0", "2.5"];
    for p in 0..6u32 {
        let s = (round + p) % 3;
        let f = (p + round) % 6;
        db.execute(&format!(
            "(transact [[:p{p} :person/status :s{s}] [:p{p} :person/age {age}] \
             [:p{p} :person/friend :p{f}] [:p{p} :person/name \"n{p}\"] \
             [:p{p} :person/bio \"{bio}\"] [:p{p} :person/ok {ok}] [:p{p} :person/self :p{p}] \
             [:p{p} :person/boss #uuid \"{boss}\"] [:p{p} :person/alias :{alias}] \
             [:p{p} :person/score {score}] [:p{p} :person/attr :person/age] [:p{p} :p{f} {p}]])",
            age = 20 + round % 4,
            bio = long(p % 2),
            ok = (round + p) % 2 == 0,
            boss = uuid_of(&format!(":p{f}")),
            alias = uuid_of(&format!(":p{}", (p + 1) % 6)),
            score = scores[((round + p) % 5) as usize],
        ))
        .unwrap();
        if (round + p) % 2 == 0 {
            db.execute(&format!(
                "(retract [[:p{p} :person/status :s{s}] [:p{p} :person/age {}] \
                 [:p{p} :person/score {}]])",
                20 + round % 4,
                scores[((round + p) % 5) as usize],
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
    let u1 = uuid_of(":p1");
    let mut shapes: Vec<String> = [
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
        // Cross-kind constants (#544 PR 2): a keyword matches the keyword and
        // a ref to the entity it names; a UUID matches the ref and keywords
        // that name it; floats match by IEEE `==` (#706).
        "[?e :person/status :s1]",
        "[?e :person/friend :p2]",
        "[?e :person/boss :p2]",
        "[?e ?a :p3]",
        "[:p0 ?a :p1]",
        "[:p0 :person/boss :p1]",
        "[?e :person/status :no-such-keyword]",
        "[?e :person/score 0.3]",
        "[?e :person/score 0.0]",
        "[?e :person/score -0.0]",
        "[?e :person/score 1.5000000000000002]",
        "[?e ?a 2.5]",
        "[:p1 ?a 1.5]",
        "[?e :person/age 21.0]",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for c in [format!("#uuid \"{u1}\""), format!(":{u1}")] {
        shapes.push(format!("[?e :person/alias {c}]"));
        shapes.push(format!("[?e :person/boss {c}]"));
        shapes.push(format!("[?e ?a {c}]"));
        shapes.push(format!("[:p0 :person/alias {c}]"));
    }
    let mut out = Vec::new();
    for shape in &shapes {
        let find = ["?e", "?a", "?v"]
            .into_iter()
            .filter(|v| shape.contains(v))
            .collect::<Vec<_>>();
        let find = if find.is_empty() {
            "?x".to_string()
        } else {
            find.join(" ")
        };
        // A constant-only pattern answers through an expression binding.
        let tail = if find == "?x" { " [(+ 1 2) ?x]" } else { "" };
        for temporal in temporals(tx) {
            out.push(format!(
                "(query [:find {find} {temporal} :where {shape}{tail}])"
            ));
        }
    }
    // Expressions after the scan, and pseudo-attributes of the scanned fact.
    let clauses = [
        ("?e ?v", "[?e :person/age ?v] [(> ?v 21)]"),
        ("?e ?w", "[?e :person/age ?v] [(+ ?v 1) ?w]"),
        ("?e ?v", "[?e :person/age ?v] [(+ ?v 1) ?v]"),
        (
            "?e ?w",
            "[?e :person/age ?v] [(+ ?v 1) ?w] [(* ?w 2) ?w] [(> ?w 44)]",
        ),
        ("?e ?w", "[?e :person/age ?v] [(+ ?v 1) ?w] [(+ ?v 2) ?w]"),
        ("?e ?v", "[?e :person/bio ?v] [(contains? ?v \"page\")]"),
        ("?e ?v", "[?e :person/bio ?v] [(+ ?v 1)]"),
        ("?e ?w", "[?e :person/bio ?v] [(/ 1 0) ?w]"),
        ("?e ?v", "[?e :person/friend ?v] [(= ?v :p1)]"),
        ("?e ?v", "[?e :person/score ?v] [(< ?v 1)]"),
        ("?v ?w", "[?e :person/score ?v] [(* ?v 2) ?w]"),
        ("?e ?a", "[?e ?a ?v] [(string? ?v)]"),
        ("?e", "[?e :person/ok ?v] [(> 2 1)]"),
        ("?e", "[?e :person/ok ?v] [(> 1 2)]"),
        ("?e ?v", "[?e :person/self ?v] [(= ?e ?v)]"),
    ];
    let pseudo = [
        ("?e ?v ?t", "[?e :person/age ?v] [?e :db/tx-count ?t]"),
        (
            "?e ?f ?u",
            "[?e :person/name ?v] [?e :db/valid-from ?f] [?e :db/valid-to ?u]",
        ),
        ("?v ?t", "[:p1 :person/name ?v] [:p1 :db/tx-id ?t]"),
        ("?a ?t", "[?e ?a ?v] [?e :db/tx-id ?t]"),
        ("?e ?v", "[?e :person/age ?v] [?e :db/tx-count 3]"),
        ("?e ?v", "[?e :person/age ?v] [?e :db/tx-count \"3\"]"),
        ("?e ?v", "[?e :person/age ?v] [?e :db/tx-count ?v]"),
        (
            "?e ?t",
            "[?e :person/age ?v] [?e :db/tx-count ?t] [?e :db/tx-count ?t]",
        ),
        (
            "?e ?t",
            "[?e :person/age ?v] [?e :db/valid-from ?t] [?e :db/valid-to ?t]",
        ),
        (
            "?e ?f",
            "[?e :person/age ?v] [?e :db/valid-from ?f] [(> ?f 0)]",
        ),
        (
            "?e ?t",
            "[?e :person/age ?v] [(- ?v ?v) ?t] [?e :db/tx-count ?t]",
        ),
        (
            "?e ?t",
            "[?e :person/age ?v] [?e :db/tx-count _]  [?e :db/tx-count ?t]",
        ),
    ];
    // Pseudo-attributes across joins (#545, #594): the metadata of their
    // source, the nearest pattern before them in the text with the same
    // entity term, whatever the plan runs first.
    let pseudo_joins = [
        "[?e :person/name ?n] [?e :person/age ?a] [?e :db/tx-count ?t]",
        "[?e :person/age ?a] [?e :db/tx-count ?t] [?e :person/name ?n] [?e :db/valid-from ?f]",
        "[?e :person/friend ?f] [?f :person/name ?n] [?f :db/tx-count ?t]",
        "[?e :person/friend ?f] [?f :person/name ?n] [?e :db/valid-from ?t]",
        "[?e :person/self ?x] [?x :person/age ?a] [?e :db/tx-count ?t]",
        "[?e :person/friend ?x] [?x :person/self ?y] [?x :db/valid-to ?t]",
        "[:p1 :person/age ?a] [?e :person/friend :p1] [:p1 :db/tx-count ?t]",
        "[?e :person/self ?x] [:p1 :person/age ?a] [:p1 :db/tx-count ?t]",
        "[?e :person/self ?x] [:p1 :person/age ?a] [?e :db/tx-count ?t]",
        "[?e :person/friend :p1] [:p1 :person/age ?a] [?e :db/tx-count ?t]",
        "[?e ?b _] [?e :db/tx-count ?t] [?e :person/age ?a]",
        "[?e ?b _] [?e :db/tx-count 1] [?e :person/name ?n]",
        "[?e ?b _] [?e :db/tx-count 2] [?f ?c _] [?f :db/tx-count 2] [?e :person/self ?f]",
        "[?e ?b _] [?e :db/valid-from ?f] [?e :db/valid-to ?u]",
        "[:p1 ?b _] [:p1 :db/tx-count ?t]",
        "[:p1 :person/age ?a] [#uuid \"{p1}\" :db/tx-count ?t]",
        "[?e :person/name ?n] [(+ 1 2) ?e] [?e :db/tx-count ?t]",
        "[?e :person/name \"n1\"] [_ :person/age ?a] [?e :db/tx-count ?t]",
        "[?e :person/age ?a] [?e :person/name ?n] [?e :db/tx-count ?t] [?e :db/tx-id ?i]",
        "[?e :person/age ?a] [?f :person/age ?a] [?f :db/tx-count ?t] [?e :db/tx-count ?t]",
        "[?e :person/age ?a] [(+ ?a 1) ?b] [?e :db/tx-id ?t] [(> ?t 0)]",
    ];
    // `:db/valid-at`: the query's valid time, once per visible fact of the
    // entity.
    let valid_at = [
        "[?e :person/name ?n] [?e :db/valid-at ?t]",
        "[?e :db/valid-at ?t]",
        "[:p1 :db/valid-at ?t] [:p1 :person/age ?a]",
        "[?e :person/friend ?f] [?f :db/valid-at ?t]",
        "[?e :person/name ?n] [?e :db/valid-at nil]",
        "[?e :person/name ?n] [?e :db/valid-at 5]",
        "[?e :person/age ?a] [?e :db/valid-at ?a]",
    ];
    let pseudo_temporals = [
        ":valid-at :any-valid-time".to_string(),
        format!(":as-of {} :valid-at :any-valid-time", tx / 2),
    ];
    for (find, clauses) in clauses {
        for temporal in temporals(tx) {
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    for clauses in joins() {
        let find = vars_of(clauses);
        for temporal in temporals(tx) {
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    for (find, clauses) in pseudo {
        for temporal in &pseudo_temporals {
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    for clauses in pseudo_joins {
        let clauses = clauses.replace("{p1}", &uuid_of(":p1").to_string());
        let find = vars_of(&clauses);
        for temporal in &pseudo_temporals {
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    for clauses in valid_at {
        let find = vars_of(clauses);
        for temporal in temporals(tx) {
            // Per-fact metadata needs `:any-valid-time` (INT-026).
            if clauses.contains(":db/tx-count") && !temporal.contains(":any-valid-time") {
                continue;
            }
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    out
}

/// Join shapes (#545): on the entity, value and attribute positions, cross
/// products, keywords and refs of one entity in both text orders, floats,
/// long strings, three patterns, expressions between joins.
fn joins() -> Vec<&'static str> {
    vec![
        // IndexNestedLoopJoin: the entity is bound.
        "[?e :person/name ?n] [?e :person/age ?a]",
        "[?e :person/age ?a] [?e ?b ?v]",
        "[?e :person/name ?n] [?e :person/name ?n]",
        // ... by a keyword value naming it, or a ref.
        "[?e :person/friend ?f] [?f :person/name ?n]",
        "[?f :person/name ?n] [?e :person/friend ?f]",
        "[?e :person/boss ?f] [?f :person/name ?n]",
        "[?e :person/alias ?x] [?x :person/age ?a]",
        "[?e :person/friend ?f] [?f :person/friend ?g] [?g :person/name ?n]",
        "[?e :person/self ?x] [?x :person/self ?x]",
        "[?e :person/friend ?f] [?f :person/self ?e]",
        "[?e :person/age ?a] [?e :person/score 0.0]",
        // HashJoin on the value position: keywords and refs, both orders.
        "[?e :person/boss ?b] [?f :person/friend ?b]",
        "[?f :person/friend ?b] [?e :person/boss ?b]",
        "[?e :person/alias ?b] [?f :person/boss ?b]",
        "[?e :person/friend ?b] [?b :person/self ?b]",
        "[?a :person/age ?v] [?b :person/age ?v]",
        "[?a :person/score ?s] [?b :person/score ?s]",
        "[?a :person/bio ?s] [?b :person/bio ?s]",
        "[?a :person/status ?s] [:p1 :person/status ?s]",
        "[:p0 :person/friend ?f] [?e :person/friend ?f]",
        // ... on the attribute position, from attributes or entities.
        "[?e ?a 21] [:p1 ?a ?w]",
        "[?e :person/boss ?x] [?y ?x ?v]",
        "[?e :person/friend ?x] [?y ?x ?v]",
        "[?e :person/attr ?x] [?y ?x ?v]",
        // Cross products.
        "[?e :person/ok true] [?f :person/status :s1]",
        "[:p1 :person/age ?a] [:p2 :person/age ?b]",
        "[?e :person/null ?x] [?f :person/null ?y]",
        // Expressions between joins.
        "[?e :person/age ?a] [?f :person/age ?b] [(< ?a ?b)]",
        "[?e :person/age ?a] [(+ ?a 1) ?b] [?f :person/age ?b]",
        "[?e :person/friend ?s] [(= ?s :p1)] [?f :person/boss ?s]",
        "[?e :person/name ?s] [(= ?s \"n2\")] [?f :person/name ?s]",
        "[?e :person/score ?s] [(- 0.0 ?s) ?t] [?f :person/score ?t]",
        "[?e :person/boss ?b] [(+ 18 3) ?b] [?f :person/age ?b]",
        "[?e :person/age ?a] [(+ ?a 1) ?a] [?f :person/age ?a]",
        // An expression binding a bound variable unifies (#596).
        "[?e :person/age ?a] [:p1 :person/age ?x] [(+ ?a 1) ?x]",
        "[?e :person/age ?a] [(+ ?a 0) ?a]",
        "[?e :person/age ?a] [(* ?a 2) ?x] [(+ ?a ?a) ?x]",
        "[?e :person/age ?a] [(* ?a 2) ?x] [(+ ?a 1) ?x]",
        "[?e :person/score ?s] [(* ?s 1.0) ?s]",
        "[?e :person/score ?s] [(- 0.0 ?s) ?t] [?f :person/score ?t] [(- 0.0 ?t) ?s]",
    ]
}

/// The variables of `clauses`, in order.
fn vars_of(clauses: &str) -> String {
    let mut vars: Vec<&str> = Vec::new();
    for t in clauses.split(|c: char| c.is_whitespace() || "[]()".contains(c)) {
        if t.starts_with('?') && !t.starts_with("?_") && !vars.contains(&t) {
            vars.push(t);
        }
    }
    vars.join(" ")
}

fn temporals(tx: u64) -> [String; 6] {
    [
        String::new(),
        ":valid-at :any-valid-time".to_string(),
        ":valid-at \"2023-03-01\"".to_string(),
        format!(":as-of {} :valid-at :any-valid-time", tx / 2),
        format!(":as-of {}", tx.saturating_sub(1)),
        ":as-of 1".to_string(),
    ]
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

/// A file built by `LogWriter` whose transaction `i + 1` has `tx_id`
/// `tx_ids[i]`: `:a :n 1` asserted, then retracted, `:a :n 2` and `:b :n 3`
/// asserted, `:b :n 3` retracted, `:a :n 1` asserted again.
fn written(dir: &std::path::Path, name: &str, tx_ids: [u64; 6]) -> std::path::PathBuf {
    use crate::{FactRecord, LogWriter, Value};
    let path = dir.join(name);
    let mut w = LogWriter::create(&path, crate::OpenOptions::new()).unwrap();
    let ops = [
        (":a", 1, true),
        (":a", 1, false),
        (":a", 2, true),
        (":b", 3, true),
        (":b", 3, false),
        (":a", 1, true),
    ];
    for (i, ((e, v, asserted), tx_id)) in ops.into_iter().zip(tx_ids).enumerate() {
        w.append(&FactRecord {
            entity: uuid_of(e),
            attribute: ":n".to_string(),
            value: Value::Integer(v),
            tx_count: i as u64 + 1,
            tx_id,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted,
        })
        .unwrap();
    }
    w.finish().unwrap();
    path
}

/// `:as-of` a timestamp runs on the engine (#551 plan, step 4) and keeps the
/// versions the executor's `tx_id <= t` filter keeps: on a file whose
/// `tx_id`s rise and on one whose `tx_id`s fall and rise (a `LogWriter`
/// copy), at timestamps before, between, at and after its transactions,
/// with committed, pending and staged facts, in plain and rule queries,
/// point and attribute scans. The oracle checks every answer.
#[test]
fn as_of_timestamps_run_on_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let iso = |ms: i64| {
        let secs = ms.div_euclid(1000);
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60,
            ms.rem_euclid(1000)
        )
    };
    let base: u64 = 1_600_000_000_000;
    let files = [
        ("rising.graph", [1, 2, 3, 4, 5, 6].map(|k| base + k * 1000)),
        ("falling.graph", [3, 1, 4, 2, 6, 5].map(|k| base + k * 1000)),
    ];
    let mut stamps: Vec<i64> = vec![-1, 0, 1_000_000_000_000, 253_402_300_799_000];
    for k in 0..=7 {
        let t = (base + k * 1000) as i64;
        stamps.extend([t - 500, t, t + 1]);
    }
    let queries = [
        "[:find ?e ?v :as-of \"{t}\" :where [?e :n ?v]]",
        "[:find ?v :as-of \"{t}\" :where [:a :n ?v]]",
        "[:find ?e :as-of \"{t}\" :where [?e :n 1]]",
        "[:find ?e ?v :as-of \"{t}\" :where (has ?e ?v)]",
    ];
    for (name, tx_ids) in files {
        let path = written(dir.path(), name, tx_ids);
        let db = Minigraf::open(&path).unwrap();
        db.execute("(rule [(has ?e ?v) [?e :n ?v]])").unwrap();
        let run_all = |db: &Minigraf| {
            for q in queries {
                for &t in &stamps {
                    let q = format!("(query {})", q.replace("{t}", &iso(t)));
                    rows(db, &q);
                }
            }
        };
        // Committed facts only.
        run_all(&db);
        // Pending facts on top, stamped now: after every committed one.
        db.execute("(transact [[:c :n 4] [:a :n 2]])").unwrap();
        db.execute("(retract [[:a :n 1]])").unwrap();
        run_all(&db);
        // Staged facts in a write transaction.
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:d :n 5]])").unwrap();
        for q in queries {
            for t in [stamps[3], (base + 3500) as i64] {
                let q = format!("(query {})", q.replace("{t}", &iso(t)));
                let before = runs();
                tx.execute(&q).unwrap();
                assert_eq!(runs(), before + 1, "staged reads run on the engine");
            }
        }
        tx.rollback();
    }
}

/// A pseudo-attribute pattern with no source is refused at parse time
/// (#594): another entity's pattern before it, a `_` entity, an entity an
/// expression binds, none before it.
#[test]
fn pseudo_attributes_without_a_source_are_prs_081() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    for q in [
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :person/friend ?f] [?f :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [_ :person/name ?e] [_ :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :person/age ?a] [(+ ?a 1) ?b] [?b :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :db/tx-count ?t] [?e :person/age ?a]])",
    ] {
        let err = db.execute(q).unwrap_err();
        assert_eq!(crate::MinigrafError::from(err).code(), "PRS-081");
    }
}

/// The executor's errors come first, from the same checks: a per-fact
/// pseudo-attribute without `:any-valid-time` (INT-026), an unknown
/// predicate (QRY-007).
#[test]
fn engine_queries_raise_the_executor_errors() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    for (q, code) in [
        (
            "(query [:find ?e ?t :where [?e :person/age ?v] [?e :db/tx-count ?t]])",
            "INT-026",
        ),
        (
            "(query [:find ?e :where [?e :person/age ?v] [(no-such-pred? ?v)]])",
            "QRY-007",
        ),
    ] {
        let before = runs();
        let err = db.execute(q).unwrap_err();
        assert_eq!(crate::MinigrafError::from(err).code(), code);
        assert_eq!(runs(), before, "no engine run");
    }
}

/// A registered predicate runs in the engine's filter.
#[test]
fn engine_filter_runs_registered_predicates() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    db.register_predicate(
        "even-age?",
        |v: &crate::Value| matches!(v, crate::Value::Integer(n) if n % 2 == 0),
    )
    .unwrap();
    rows(
        &db,
        "(query [:find ?e ?v :where [?e :person/age ?v] [(even-age? ?v)]])",
    );
}

/// Cross-kind constants over committed and pending facts, `:as-of`,
/// retractions and reopen (from #518's branch): each answer agrees with the
/// executor (the oracle), and the expected entities are found.
#[test]
fn cross_kind_constants_read_their_avet_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("avet.graph");
    let file = Minigraf::open(&path).unwrap();
    let mem = Minigraf::in_memory().unwrap();
    let both = |cmd: &str| {
        file.execute(cmd).unwrap();
        mem.execute(cmd).unwrap();
    };
    let (u1, u2) = (uuid_of(":p1"), uuid_of(":p2"));
    let long = "a long relation value ".repeat(4);
    let batches = [
        vec![
            "(transact [[:p0 :rel :p1] [:p1 :rel :k/x] [:p2 :rel 1.5] [:p3 :rel nil]])".to_string(),
            format!(r#"(transact [[:p4 :rel "{long}"] [:p5 :rel 7] [:p5 :rel true]])"#),
        ],
        vec![
            format!(r#"(transact [[:p1 :rel #uuid "{u2}"]])"#),
            "(retract [[:p5 :rel 7]])".to_string(),
            r#"(transact [[:p3 :rel "s"]])"#.to_string(),
        ],
        // Pending on top of committed facts.
        vec![
            "(transact [[:p5 :rel 7] [:p0 :rel 1.5]])".to_string(),
            "(retract [[:p1 :rel :k/x]])".to_string(),
        ],
    ];
    for (i, batch) in batches.iter().enumerate() {
        for cmd in batch {
            both(cmd);
        }
        if i + 1 < batches.len() {
            file.checkpoint().unwrap();
        }
    }
    let entities = |db: &Minigraf, c: &str| -> Vec<uuid::Uuid> {
        let before = runs();
        let QueryResult::QueryResults { results, .. } = db
            .execute(&format!("(query [:find ?e :where [?e :rel {c}]])"))
            .unwrap()
        else {
            panic!("expected query results");
        };
        assert_eq!(runs(), before + 1, "query ran on the engine");
        let mut es: Vec<uuid::Uuid> = results
            .iter()
            .map(|r| match r.first() {
                Some(crate::Value::Ref(u)) => *u,
                _ => panic!("expected an entity"),
            })
            .collect();
        es.sort();
        es
    };
    let ps = |names: &[&str]| -> Vec<uuid::Uuid> {
        let mut es: Vec<uuid::Uuid> = names.iter().map(|n| uuid_of(n)).collect();
        es.sort();
        es
    };
    let expected = [
        (":p1".to_string(), ps(&[":p0"])),
        (":p2".to_string(), ps(&[":p1"])),
        (format!(r#"#uuid "{u1}""#), ps(&[":p0"])),
        (format!(r#"#uuid "{u2}""#), ps(&[":p1"])),
        (":k/x".to_string(), ps(&[])),
        (":nope".to_string(), ps(&[])),
        ("1.5".to_string(), ps(&[":p0", ":p2"])),
        ("nil".to_string(), ps(&[":p3"])),
        ("7".to_string(), ps(&[":p5"])),
        ("true".to_string(), ps(&[":p5"])),
        (r#""s""#.to_string(), ps(&[":p3"])),
        (format!(r#""{long}""#), ps(&[":p4"])),
    ];
    let check = |db: &Minigraf| {
        for (c, want) in &expected {
            assert_eq!(&entities(db, c), want, "entities of a constant");
            let max = db.current_tx_count();
            rows(
                db,
                &format!("(query [:find ?e :as-of {} :where [?e :rel {c}]])", max - 2),
            );
        }
    };
    check(&file);
    check(&mem);
    file.checkpoint().unwrap();
    check(&file);
    drop(file);
    check(&Minigraf::open(&path).unwrap());
}

/// Gives `rows` integers in one batch, then fails.
struct FailAfter {
    rows: i64,
    sent: bool,
    closed: bool,
}

impl super::Operator for FailAfter {
    fn next_batch(&mut self, _: &mut super::Ctx<'_>) -> anyhow::Result<Option<super::Batch>> {
        assert!(!self.closed, "no pull after close");
        if self.sent {
            anyhow::bail!("read failed");
        }
        self.sent = true;
        let mut batch = super::Batch::new(1);
        for n in 0..self.rows {
            batch.push(&mut [Some(super::cell::Cell::Val(super::cell::Scalar::Int(n)))]);
        }
        Ok(Some(batch))
    }

    fn rewind(&mut self, _: &[super::cell::Cell]) {}

    fn close(&mut self) {
        self.closed = true;
    }

    fn counters(&self, _: &mut Vec<super::Counters>) {}
}

fn failing_cursor(rows: i64) -> crate::Cursor {
    let snapshot = crate::graph::storage::FactStorage::new()
        .snapshot()
        .unwrap();
    let stream = super::Stream::for_test(
        snapshot.clone(),
        Box::new(FailAfter {
            rows,
            sent: false,
            closed: false,
        }),
    );
    crate::Cursor::open(
        crate::query::datalog::executor::Answer::Stream(Box::new(stream)),
        snapshot,
    )
    .unwrap()
}

#[test]
fn cursor_returns_gathered_rows_before_a_read_error() {
    let mut cursor = failing_cursor(3);
    assert_eq!(cursor.next_batch(10).unwrap().expect("rows first").len(), 3);
    assert!(cursor.next_batch(10).is_err(), "then the error");
    assert!(cursor.next_batch(10).unwrap().is_none(), "then done");

    let mut cursor = failing_cursor(3);
    assert_eq!(cursor.next_batch(3).unwrap().expect("rows").len(), 3);
    assert!(
        cursor.next_batch(3).is_err(),
        "an error with no rows gathered"
    );
    assert!(cursor.next_batch(3).unwrap().is_none());

    let items: Vec<bool> = failing_cursor(2).map(|r| r.is_ok()).collect();
    assert_eq!(
        items,
        [true, true, false],
        "the iterator ends after the error"
    );

    let mut cursor = failing_cursor(2);
    assert_eq!(cursor.next_batch(5).unwrap().expect("rows").len(), 2);
    assert!(cursor.next().expect("the error").is_err());
    assert!(cursor.next().is_none());
}

/// DICT lookups are cached for one pull only, so a cursor's memory does not
/// grow with the rows it returns; pending entities keep their ids.
#[test]
fn dict_cache_is_dropped_after_each_pull() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("ids.graph")).unwrap();
    let facts: String = (0..3000)
        .map(|i| format!("[:p{i} :person/age {i}]"))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    db.execute("(transact [[:new :person/age 1] [:new :person/age 2]])")
        .unwrap();
    let mut cursor = db
        .query("(query [:find ?e ?a :where [?e :person/age ?a]])")
        .unwrap();
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(500).unwrap() {
        rows += batch.len();
        if let Some(stream) = cursor.stream_mut() {
            assert_eq!(stream.ids.dict_cached(), 0, "no DICT lookups kept");
        }
    }
    assert_eq!(rows, 3002);
}

/// A stream that ends in a `Window` keeps its DICT lookups until it ends:
/// the window looked up every row's ids in its first pull, and the rows it
/// returns later read them from the cache (#547).
#[test]
fn window_stream_keeps_dict_cache_until_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("ids.graph")).unwrap();
    let facts: String = (0..3000)
        .map(|i| format!("[:p{i} :person/age {}]", i % 7))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let mut cursor = db
        .query("(query [:find ?e (rank :over (:order-by ?a)) :where [?e :person/age ?a]])")
        .unwrap();
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(500).unwrap() {
        rows += batch.len();
        if let Some(stream) = cursor.stream_mut()
            && rows < 3000
        {
            assert!(stream.ids.dict_cached() >= 3000, "every row's entity kept");
        }
    }
    assert_eq!(rows, 3000);
    if let Some(stream) = cursor.stream_mut() {
        assert_eq!(stream.ids.dict_cached(), 0, "dropped at the end");
    }
}

/// The planner's join choice (decision 3 of the #545 plan): an
/// `IndexNestedLoopJoin` when the entity is bound, a `HashJoin` otherwise,
/// built on the side with the smaller estimate.
#[test]
fn planner_chooses_the_join() {
    use super::plan::{JoinKind, Step, plan};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    let kinds = |q: &str| -> Vec<String> {
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(q) else {
            panic!("expected a query");
        };
        let plan = plan(&query).expect("an engine plan");
        plan.steps
            .iter()
            .filter_map(|s| match s {
                Step::Join(j) => Some(match j.kind {
                    JoinKind::Probe(_) => "probe".to_string(),
                    JoinKind::Hash { build_inner } => format!("hash inner={build_inner}"),
                }),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        kinds("(query [:find ?e :where [?e :a ?x] [?e :b ?y]])"),
        ["probe"]
    );
    assert_eq!(
        kinds("(query [:find ?e :where [?e :a ?x] [?f :b ?x]])"),
        ["hash inner=true"]
    );
    assert_eq!(
        kinds("(query [:find ?e :where [?e :a 1] [?f :b ?y] [?f :c ?e]])"),
        ["hash inner=false", "probe"]
    );
    assert_eq!(
        kinds("(query [:find ?e :where [?e :a ?x] [?f ?x ?y]])"),
        ["hash inner=false"]
    );
    // With the test switch off the table is always built on the inner
    // pattern.
    let _off = crate::query::datalog::optimizer::set_reorder(false);
    assert_eq!(
        kinds("(query [:find ?e :where [?e :a 1] [?f :b ?y] [?f :c ?e]])"),
        ["hash inner=true", "probe"]
    );
}

#[test]
fn planner_binds_checks_or_replaces_an_expression_output() {
    use super::plan::{Out, Step, plan};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    let modes = |q: &str| -> Vec<Out> {
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(q) else {
            panic!("expected a query");
        };
        let plan = plan(&query).expect("an engine plan");
        plan.steps
            .iter()
            .filter_map(|s| match s {
                Step::Bind(_, _, _, m) => Some(*m),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        modes("(query [:find ?b :where [?e :a ?a] [(+ ?a 1) ?b]])"),
        [Out::New]
    );
    // The pattern binding ?x is first in the text and in the plan.
    assert_eq!(
        modes("(query [:find ?x :where [?f :b ?x] [?e :a ?a] [(+ ?a 1) ?x]])"),
        [Out::Check]
    );
    // [:p :b ?x] runs first (natively), but the expression is earlier in the text.
    let _on = crate::query::datalog::optimizer::set_reorder(true);
    if crate::query::datalog::optimizer::reorder() {
        assert_eq!(
            modes("(query [:find ?x :where [?e :a ?a] [(+ ?a 1) ?x] [:p :b ?x]])"),
            [Out::Replace]
        );
    }
    let _off = crate::query::datalog::optimizer::set_reorder(false);
    assert_eq!(
        modes("(query [:find ?x :where [?e :a ?a] [(+ ?a 1) ?x] [:p :b ?x]])"),
        [Out::New]
    );
}

/// Joins whose answers span many batches: one probe with more than a batch
/// of matches, a cross product, a table built on the outer side, over
/// committed and pending facts.
#[test]
fn engine_joins_read_many_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("joins.graph")).unwrap();
    let facts: String = (0..2000)
        .map(|i| {
            format!(
                "[:e{i} :big/n {i}] [:e{i} :big/tag :t{}] [:hub :big/has :e{i}]",
                i % 7
            )
        })
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.checkpoint().unwrap();
    let more: String = (2000..2600)
        .map(|i| format!("[:e{i} :big/n {i}] [:hub :big/has :e{i}]"))
        .collect();
    db.execute(&format!("(transact [{more}])")).unwrap();
    db.execute("(retract [[:e5 :big/n 5] [:e2001 :big/n 2001]])")
        .unwrap();
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e ?n ?t :where [?e :big/n ?n] [?e :big/tag ?t]])"
        ),
        1999
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?x ?n :where [:hub :big/has ?x] [?x :big/n ?n]])"
        ),
        2598
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?h ?x :where [?h :big/has ?x] [?x :big/tag :t3]])"
        ),
        286
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?a ?b :where [?a :big/tag :t1] [?b :big/tag :t2] [(< 0 1)]])"
        ),
        286 * 286
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?a ?b :where [?a :big/tag ?t] [?b :big/tag ?t]])"
        ),
        5 * 286 * 286 + 2 * 285 * 285
    );
}

/// `not` / `not-join` shapes (#546): hashed and correlated bodies, keys on
/// the entity, value and attribute positions, floats and keywords imported
/// into a body, expressions over imported variables, per-fact
/// pseudo-attributes and `:db/valid-at` in bodies, deferred and
/// variable-free negations.
fn negations() -> Vec<&'static str> {
    vec![
        // Correlated: the body's entity is imported.
        "[?e :person/name ?n] (not [?e :person/ok true])",
        "[?e :person/friend ?f] (not [?f :person/ok true])",
        "[?e :person/name ?n] (not-join [?e] [?e :person/friend ?f] [?f :person/ok true])",
        "[?e :person/age ?a] (not-join [?e] [?e :person/score ?s] [(> ?s 1.0)])",
        // Hashed: keyed on a value or an attribute.
        "[?e :person/age ?a] (not-join [?a] [?f :person/age ?a] [?f :person/ok true])",
        "[?e :person/score ?s] (not-join [?s] [?f :person/score ?s] [?f :person/ok false])",
        "[?e :person/boss ?b] (not-join [?b] [?x :person/friend ?b])",
        "[?e :person/alias ?b] (not-join [?b] [?x :person/boss ?b])",
        "[?e :person/bio ?b] (not-join [?b] [?f :person/bio ?b] [?f :person/ok true])",
        "[?e :person/attr ?x] (not-join [?x] [:p1 ?x 21])",
        "[?e ?a 21] (not-join [?a] [?x :person/attr ?a])",
        // A body that reads an imported variable it does not bind.
        "[?e :person/age ?a] (not-join [?a] [?f :person/age ?b] [(< ?b ?a)])",
        "[?e :person/age ?a] (not [(> ?a 21)])",
        // A body binding no join variable; one reading no variable.
        "[?e :person/name ?n] (not-join [?e] [?x :person/ok true])",
        "[?e :person/name ?n] (not [:p1 :person/ok true])",
        // Metadata of an outer source, and the valid time.
        "[?e :person/name ?n] (not-join [?e] [?e :db/tx-count ?t] [(> ?t 3)])",
        "[?e :person/name ?n] (not-join [?e] [?e :db/valid-at ?t] [(> ?t 0)])",
        "[?e :person/name ?n] (not-join [?e] [?e :db/valid-at ?t])",
        // Deferred: an expression binds the join variable.
        "[?e :person/age ?a] [(+ ?a 1) ?b] (not-join [?b] [?f :person/age ?b])",
        // Two negations.
        "[?e :person/name ?n] (not [?e :person/ok true]) (not [?e :person/status :s1])",
    ]
}

fn negation_queries(tx: u64) -> Vec<String> {
    let mut out = Vec::new();
    for clauses in negations() {
        let find = vars_of(clauses.split(" (not").next().unwrap_or(clauses));
        let find = find
            .split(' ')
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        for temporal in temporals(tx) {
            // Per-fact metadata needs `:any-valid-time` (INT-026).
            if clauses.contains(":db/tx-count") && !temporal.contains(":any-valid-time") {
                continue;
            }
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    out
}

/// Every negation shape with each body form forced and reordering on and
/// off; the oracle compares each run with the executor.
fn check_negations(db: &Minigraf) -> usize {
    use crate::query::datalog::optimizer::{Subplans, set_reorder, set_subplans};
    let tx = db.current_tx_count();
    let mut total = 0;
    for mode in [Subplans::Choose, Subplans::Hash, Subplans::Correlated] {
        for on in [true, false] {
            let _m = set_subplans(mode);
            let _r = set_reorder(on);
            total += negation_queries(tx)
                .iter()
                .map(|q| rows(db, q))
                .sum::<usize>();
        }
    }
    total
}

#[test]
fn negation_matches_executor() {
    let dir = tempfile::tempdir().unwrap();
    let mem = Minigraf::in_memory().unwrap();
    let file = Minigraf::open(dir.path().join("not.graph")).unwrap();
    for round in 0..3 {
        write_round(&mem, round);
        assert!(check_negations(&mem) > 0, "negations keep some rows");
        write_round(&file, round);
        check_negations(&file);
        file.checkpoint().unwrap();
        check_negations(&file);
    }
}

/// The body forms the planner chooses (#546 spec §4.4, decision 7 of the
/// plan): correlated when the first pattern's entity is imported or the
/// body reads an outer variable it does not bind, hashed otherwise; the
/// test switch forces either where the body allows it.
#[test]
fn planner_chooses_the_body_form() {
    use super::plan::{Body, Step, plan};
    use crate::query::datalog::optimizer::{Subplans, set_subplans};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    let forms = |q: &str| -> Vec<&'static str> {
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(q) else {
            panic!("expected a query");
        };
        let plan = plan(&query).expect("an engine plan");
        plan.steps
            .iter()
            .filter_map(|s| match s {
                Step::Anti(Body::Hashed { .. }) => Some("hashed"),
                Step::Anti(Body::Correlated { .. }) => Some("correlated"),
                _ => None,
            })
            .collect()
    };
    let probe = "(query [:find ?e :where [?e :a ?x] (not [?e :b 1])])";
    let keyed = "(query [:find ?e :where [?e :a ?x] (not-join [?x] [?f :b ?x])])";
    let open = "(query [:find ?e :where [?e :a ?x] (not-join [?x] [?f :b ?y] [(< ?y ?x)])])";
    let expr_only = "(query [:find ?e :where [?e :a ?x] (not [(> ?x 1)])])";
    let constant = "(query [:find ?e :where [?e :a ?x] (not [:p :b 1])])";
    assert_eq!(forms(probe), ["correlated"]);
    assert_eq!(forms(keyed), ["hashed"]);
    assert_eq!(forms(open), ["correlated"]);
    assert_eq!(forms(expr_only), ["correlated"]);
    assert_eq!(forms(constant), ["hashed"]);
    {
        let _h = set_subplans(Subplans::Hash);
        assert_eq!(forms(probe), ["hashed"]);
        assert_eq!(forms(open), ["correlated"], "not closed: never hashed");
        assert_eq!(forms(expr_only), ["correlated"], "nothing to scan");
    }
    let _c = set_subplans(Subplans::Correlated);
    assert_eq!(forms(keyed), ["correlated"]);
    assert_eq!(forms(constant), ["correlated"]);
}

/// Which negations the engine runs (#546): every `not` / `not-join`, with
/// a rule invocation too since rule queries run here (#548).
#[test]
fn negation_routing() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    db.execute("(rule [(okay ?e) [?e :person/ok true]])")
        .unwrap();
    for q in [
        "(query [:find ?e :where [?e :person/name ?n] (not [?e :person/ok true])])",
        "(query [:find ?e :where [?e :person/name ?n] (not [:p1 :person/ok true])])",
        "(query [:find ?e :where [?e :person/age ?a] [(+ ?a 1) ?b] (not-join [?b] [?f :person/age ?b])])",
        "(query [:find ?e :where [?e :person/name ?n] (not (okay ?e))])",
    ] {
        rows(&db, q);
    }
}

/// `not` filters and never deduplicates (spec §3.1): rows with the same
/// `:find` values both survive. A failing expression drops its body row, so
/// it cannot exclude the outer row (spec §3.3).
#[test]
fn negation_keeps_multiplicity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :n 1] [:b :n 1] [:c :n 2] [:d :x 0.0]])")
        .unwrap();
    assert_eq!(
        rows(
            &db,
            "(query [:find ?v :where [?e :n ?v] (not-join [?v] [?f :n ?v] [(> ?v 1)])])"
        ),
        2
    );
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e :where [?e :x ?v] (not-join [?v] [?f :x ?v] [(/ 1 0) ?z])])"
        ),
        1
    );
}

/// An unknown predicate in a body fails on every body row, so the `not`
/// keeps every outer row (spec §3.3), on the engine as on the executor.
#[test]
fn negation_unknown_predicate_keeps_rows() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :n 1] [:b :n 2]])").unwrap();
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e :where [?e :n ?v] (not [?e :n ?v] [(no-such-pred? ?v)])])"
        ),
        2
    );
}

/// A correlated body probes once per outer row from its seed; a hashed one
/// is read once, however many outer rows there are (#546 spec §4.2, §4.3).
#[test]
fn negation_bodies_run_per_row_or_once() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..50)
        .map(|i| format!("[:e{i} :n {i}] [:e{i} :tag :t{}]", i % 5))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let ops = |q: &str| {
        let mut cursor = db.query(q).unwrap();
        cursor.next_batch(1).unwrap().expect("a row");
        cursor.counters()
    };
    let c = ops("(query [:find ?e :where [?e :n ?v] (not [?e :tag :t0])])");
    let seed = c.iter().find(|c| c.op == "Seed").expect("a seed");
    let anti = c.iter().find(|c| c.op == "AntiJoin").expect("an AntiJoin");
    assert_eq!(seed.rows_out, 50, "one seed row per outer row");
    assert_eq!(anti.rows_in, 50);
    assert_eq!(anti.rows_out, 40);
    assert!(c.iter().any(|c| c.op == "IndexNestedLoopJoin"));

    let c = ops("(query [:find ?e :where [?e :n ?v] (not-join [?v] [?f :n ?v] [?f :tag :t0])])");
    assert!(!c.iter().any(|c| c.op == "Seed"), "hashed: no seed");
    let anti = c.iter().find(|c| c.op == "AntiJoin").expect("an AntiJoin");
    assert_eq!(anti.rows_out, 40);
    // The outer scan, the body's scan and its probes: each key read once.
    let read: u64 = c.iter().filter(|c| c.op == "Scan").map(|c| c.rows_in).sum();
    assert!(read <= 150, "the body is read once");
}

/// `or` / `or-join` shapes (#546 PR 2), as `(find, clauses)`: branches
/// probed and keyed, new variables, keywords meeting refs, attributes, floats
/// and long strings as keys, branches reading outer variables, metadata and
/// the valid time, a branch with `not`, branches naming different outer
/// variables or no join variable, an `or` with nothing before it, a cross
/// product, deferred `not`s and expressions after an `or`.
fn disjunctions() -> Vec<(&'static str, &'static str)> {
    vec![
        // Correlated: the branch's entity is imported.
        (
            "?e ?n",
            "[?e :person/name ?n] (or [?e :person/ok true] [?e :person/age 21])",
        ),
        (
            "?e ?x",
            "[?e :person/name ?n] (or [?e :person/age ?x] [?e :person/score ?x])",
        ),
        (
            "?e",
            "[?e :person/friend ?f] (or [?f :person/ok true] [?f :person/status :s1])",
        ),
        // Hashed: keyed on a value, a ref, an attribute, a float, a long
        // string.
        (
            "?e ?f",
            "[?e :person/age ?a] (or [?f :person/age ?a] [?f :person/score ?a])",
        ),
        (
            "?e",
            "[?e :person/age ?a] (or-join [?a] (and [?f :person/age ?a] [?f :person/ok true]) [?f :person/score ?a])",
        ),
        (
            "?e ?b",
            "[?e :person/boss ?b] (or-join [?b] [?x :person/friend ?b] [?x :person/self ?b])",
        ),
        (
            "?e",
            "[?e :person/alias ?b] (or-join [?b] [?x :person/boss ?b] [?x :person/friend ?b])",
        ),
        (
            "?e ?x",
            "[?e :person/attr ?x] (or-join [?x] [:p1 ?x 21] [:p2 ?x 22])",
        ),
        (
            "?e ?s",
            "[?e :person/score ?s] (or-join [?s] [?f :person/score ?s] [?f :person/age ?s])",
        ),
        (
            "?e",
            "[?e :person/bio ?b] (or-join [?b] (and [?f :person/bio ?b] [?f :person/ok true]) [?f :person/name ?b])",
        ),
        // A branch reading an outer variable it does not bind.
        (
            "?e",
            "[?e :person/age ?a] (or [(> ?a 21)] [?e :person/ok true])",
        ),
        (
            "?e",
            "[?e :person/age ?a] (or-join [?a] (and [?f :person/age ?b] [(< ?b ?a)]) [?f :person/score ?a])",
        ),
        // A branch with `not`.
        (
            "?e ?a",
            "[?e :person/name ?n] (or (and [?e :person/age ?a] (not [?e :person/ok true])) [?e :person/score ?a])",
        ),
        (
            "?e",
            "[?e :person/age ?a] (or-join [?a] (and [?f :person/age ?a] (not-join [?f] [?f :person/ok true])) [?f :person/score ?a])",
        ),
        // Branches naming different outer variables; a branch binding no
        // join variable.
        (
            "?e ?f",
            "[?e :person/friend ?f] (or [?e :person/ok true] [?f :person/ok false])",
        ),
        (
            "?e",
            "[?e :person/name ?n] (or-join [?e] [?x :person/age 23] [?e :person/ok true])",
        ),
        // Nothing before the `or`; a cross product.
        ("?e", "(or [?e :person/ok true] [?e :person/age 21])"),
        (
            "?e ?f",
            "[?e :person/ok true] (or [?f :person/age 21] [?f :person/age 22])",
        ),
        // Metadata of an outer source, and the valid time.
        (
            "?e",
            "[?e :person/name ?n] (or-join [?e] (and [?e :db/tx-count ?t] [(> ?t 3)]) [?e :person/ok true])",
        ),
        (
            "?e ?t",
            "[?e :person/name ?n] (or [?e :db/valid-at ?t] [?e :person/age ?t])",
        ),
        // A `not` and an expression on what the `or` binds; two `or`s.
        (
            "?e ?a",
            "[?e :person/name ?n] (or [?e :person/age ?a] [?e :person/score ?a]) (not-join [?a] [?f :person/age ?a] [?f :person/ok true])",
        ),
        (
            "?e ?a",
            "[?e :person/name ?n] (or [?e :person/age ?a] [?e :person/score ?a]) [(> ?a 0)]",
        ),
        (
            "?e",
            "[?e :person/name ?n] (or [?e :person/ok true] [?e :person/age 21]) (or-join [?e] [?e :person/status :s0] [?e :person/status :s2])",
        ),
        // Duplicate outer rows collapse.
        (
            "?e",
            "[?e :person/friend ?f] [?g :person/friend ?f] (or [?e :person/ok true] [?e :person/ok false])",
        ),
    ]
}

fn disjunction_queries(tx: u64) -> Vec<String> {
    let mut out = Vec::new();
    for (find, clauses) in disjunctions() {
        for temporal in temporals(tx) {
            // Per-fact metadata needs `:any-valid-time` (INT-026).
            if clauses.contains(":db/tx-count") && !temporal.contains(":any-valid-time") {
                continue;
            }
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    out
}

/// Every disjunction shape with each branch form forced and reordering on
/// and off; the oracle compares each run with the executor.
fn check_disjunctions(db: &Minigraf) -> usize {
    use crate::query::datalog::optimizer::{Subplans, set_reorder, set_subplans};
    let tx = db.current_tx_count();
    let mut total = 0;
    for mode in [Subplans::Choose, Subplans::Hash, Subplans::Correlated] {
        for on in [true, false] {
            let _m = set_subplans(mode);
            let _r = set_reorder(on);
            total += disjunction_queries(tx)
                .iter()
                .map(|q| rows(db, q))
                .sum::<usize>();
        }
    }
    total
}

#[test]
fn disjunction_matches_executor() {
    let dir = tempfile::tempdir().unwrap();
    let mem = Minigraf::in_memory().unwrap();
    let file = Minigraf::open(dir.path().join("or.graph")).unwrap();
    for round in 0..3 {
        write_round(&mem, round);
        assert!(check_disjunctions(&mem) > 0, "disjunctions return rows");
        write_round(&file, round);
        check_disjunctions(&file);
        file.checkpoint().unwrap();
        check_disjunctions(&file);
    }
}

/// Which disjunctions the engine runs (#546 PR 2): `or` / `or-join` whose
/// branches hold patterns, expressions, `not`s and rule invocations (#548);
/// a nested `or`, a predicate the registry may lack, an `or` whose branches
/// bind their own sources' metadata, and a branch `not` reading what only
/// the branch's expression binds stay on the executor (decision 11).
#[test]
fn disjunction_routing() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    db.execute("(rule [(okay ?e) [?e :person/ok true]])")
        .unwrap();
    for q in [
        "(query [:find ?e :where [?e :person/name ?n] (or [?e :person/ok true] [?e :person/age 21])])",
        "(query [:find ?e :where (or [?e :person/ok true] [?e :person/age 21])])",
        "(query [:find ?e :where [?e :person/name ?n] (or (and [?e :person/ok true] (not [?e :person/age 21])) [?e :person/age 22])])",
        "(query [:find ?e :any-valid-time :where [?e :person/name ?n] (or-join [?e] (and [?f :person/age ?a] [?f :db/tx-count ?t]) [?e :person/ok true])])",
        "(query [:find ?e :where [?e :person/name ?n] (or (okay ?e) [?e :person/age 21])])",
    ] {
        rows(&db, q);
    }
    for q in [
        "(query [:find ?e :where [?e :person/name ?n] (or (or [?e :person/ok true] [?e :person/age 22]) [?e :person/age 21])])",
        "(query [:find ?e :where [?e :person/age ?a] (or (and [?e :person/ok true] [(no-such-pred? ?a)]) [?e :person/age 21])])",
        "(query [:find ?e :any-valid-time :where [?e :person/name ?n] (or (and [?e :person/age ?a] [?e :db/tx-count ?t]) (and [?e :person/score ?a] [?e :db/tx-id ?t]))])",
        "(query [:find ?e :where [?e :person/age ?a] (or (and [(+ ?a 1) ?b] (not [?f :person/age ?b])) (and [(+ ?a 2) ?b] [?e :person/ok true]))])",
    ] {
        let before = runs();
        let _ = db.execute(q);
        assert_eq!(runs(), before, "query stayed on the executor");
    }
}

/// The branch forms the planner chooses: as a `not` body's (spec §4.4,
/// §5.1), with a branch's `not` reading an imported variable the branch's
/// patterns do not bind keeping it correlated.
#[test]
fn planner_chooses_the_branch_form() {
    use super::plan::{Body, Step, plan};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    let forms = |q: &str| -> Vec<&'static str> {
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(q) else {
            panic!("expected a query");
        };
        let plan = plan(&query).expect("an engine plan");
        plan.steps
            .iter()
            .flat_map(|s| match s {
                Step::Union(branches) => branches
                    .iter()
                    .map(|b| match b.body {
                        Body::Hashed { .. } => "hashed",
                        Body::Correlated { .. } => "correlated",
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
    };
    assert_eq!(
        forms("(query [:find ?e :where [?e :a ?x] (or-join [?e ?x] [?e :b 1] [?f :c ?x])])"),
        ["correlated", "hashed"]
    );
    assert_eq!(
        forms(
            "(query [:find ?e :where [?e :a ?x] (or-join [?x] [?f :b ?x] (and [?f :c ?y] [(< ?y ?x)]))])"
        ),
        ["hashed", "correlated"]
    );
    assert_eq!(
        forms(
            "(query [:find ?e :where [?e :a ?x] (or-join [?x] (and [?f :b ?x] (not [?f :c 1])) (and [?f :b ?y] (not [?f :c ?x])))])"
        ),
        ["hashed", "correlated"]
    );
    assert_eq!(
        forms("(query [:find ?e :where (or [?e :b 1] [?e :c 2])])"),
        ["hashed", "hashed"]
    );
}

/// `Distinct` after `Union` collapses equal rows, as the executor's `or`
/// does, and keeps rows that differ only in a variable `:find` leaves out;
/// `-0.0` equals `0.0`, a keyword never equals a ref.
#[test]
fn disjunction_rows_are_distinct() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact [[:a :n 1] [:a :m 1] [:a :k :b] [:a :k #uuid \"8bd6a0fe-0f33-5b3c-a8c4-0b5ba4ea0d42\"] \
         [:z :f 0.0] [:z :f -0.0] [:y :f 0.0]])",
    )
    .unwrap();
    // Both branches give `:a`: one row.
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e :where [?e :n _] (or [?e :n 1] [?e :m 1])])"
        ),
        1
    );
    // Rows differing in `?k` (a keyword and a ref) are two rows.
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e :where [?e :k ?k] (or [?e :n 1] [?e :m 1])])"
        ),
        2
    );
    // `-0.0` and `0.0`: `:z` has two facts but one row per `?f` value.
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e ?f :where [?e :f ?f] (or [?e :f 0.0] [?e :f 1.0])])"
        ),
        2
    );
}

/// A correlated branch runs per outer row from its seed; a hashed one is
/// read once (#546 spec §5.1); with no new variable an outer row is
/// returned once, from the first branch that has a row.
#[test]
fn disjunction_branches_run_per_row_or_once() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..50)
        .map(|i| format!("[:e{i} :n {i}] [:e{i} :tag :t{}]", i % 5))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let ops = |q: &str| {
        // One pull reads every outer row: there are fewer than a batch.
        let mut cursor = db.query(q).unwrap();
        cursor.next_batch(1).unwrap().expect("a row");
        cursor.counters()
    };
    let c = ops("(query [:find ?e :where [?e :n ?v] (or [?e :tag :t0] [?e :tag :t1])])");
    let union = c.iter().find(|c| c.op == "Union").expect("a Union");
    assert_eq!(union.rows_in, 50);
    assert_eq!(union.rows_out, 20);
    assert_eq!(
        c.iter()
            .filter(|c| c.op == "Seed")
            .map(|c| c.rows_out)
            .sum::<u64>(),
        50 + 40,
        "the second branch runs only for rows the first did not match"
    );

    let c = ops(
        "(query [:find ?e :where [?e :n ?v] (or-join [?v] (and [?f :n ?v] [?f :tag :t0]) (and [?f :n ?v] [?f :tag :t1]))])",
    );
    assert!(!c.iter().any(|c| c.op == "Seed"), "hashed: no seed");
    let union = c.iter().find(|c| c.op == "Union").expect("a Union");
    assert_eq!(union.rows_out, 20);
    let read: u64 = c.iter().filter(|c| c.op == "Scan").map(|c| c.rows_in).sum();
    assert!(read <= 250, "each branch is read once");
}

/// Aggregate shapes (#547 PR 2): ungrouped and grouped, by keywords, refs,
/// booleans, floats (`-0.0` and `0.0` one group) and long strings; every
/// built-in aggregate over each kind of value, `nil` values, `:with`.
fn aggregates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("(count ?e)", "[?e :person/name ?n]"),
        ("(count ?v) (count-distinct ?v)", "[?e ?a ?v]"),
        ("(count ?v) (count-distinct ?v)", "[?e :person/null ?v]"),
        ("?s (count ?e)", "[?e :person/status ?s]"),
        (
            "?f (count ?e) (min ?g) (max ?g)",
            "[?e :person/friend ?f] [?e :person/age ?g]",
        ),
        (
            "?ok (sum ?g) (sum-distinct ?g)",
            "[?e :person/ok ?ok] [?e :person/age ?g]",
        ),
        (
            "?b (count ?e) (min ?n) (max ?n)",
            "[?e :person/bio ?b] [?e :person/name ?n]",
        ),
        ("?b (count-distinct ?b)", "[?e :person/bio ?b]"),
        ("(count-distinct ?b)", "[?e :person/bio ?b]"),
        (
            "(sum ?x) (min ?x) (max ?x) (count-distinct ?x) (sum-distinct ?x)",
            "[?e :person/score ?x]",
        ),
        ("?x (count ?e)", "[?e :person/score ?x]"),
        ("(count-distinct ?k)", "[?e :person/alias ?k]"),
        (
            "(count-distinct ?k)",
            "(or [?e :person/alias ?k] [?e :person/boss ?k])",
        ),
        ("(count-distinct ?a)", "[?e ?a ?v]"),
        ("(sum ?g) :with ?e", "[?e :person/age ?g]"),
        (
            "?s (sum ?g) :with ?e",
            "[?e :person/status ?s] [?e :person/age ?g]",
        ),
        ("(count ?g) :with ?e", "[?e :person/age ?g] [(> ?g 100)]"),
        (
            "(count ?e) (count-distinct ?e)",
            "[?e :person/age ?g] [(> ?g 100)]",
        ),
        ("(sum ?g)", "[?e :person/age ?g] [(> ?g 100)]"),
        ("?e (count ?g)", "[?e :person/age ?g] [(> ?g 100)]"),
        ("(min ?n) (count ?e)", "[?e :person/null ?n]"),
    ]
}

fn check_aggregates(db: &Minigraf) -> usize {
    let tx = db.current_tx_count();
    let mut n = 0;
    for (find, clauses) in aggregates() {
        for t in temporals(tx) {
            n += rows(db, &format!("(query [:find {find} {t} :where {clauses}])"));
        }
    }
    n += rows(
        db,
        "(query [:find ?e (sum ?t) (max ?t) :any-valid-time :where [?e :person/name ?n] [?e :db/tx-count ?t]])",
    );
    n
}

#[test]
fn aggregates_match_executor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agg.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        for round in 0..3 {
            write_round(&db, round);
        }
        assert!(check_aggregates(&db) > 0, "some groups");
        db.checkpoint().unwrap();
        write_round(&db, 3);
        check_aggregates(&db);
    }
    let db = Minigraf::open(&path).unwrap();
    check_aggregates(&db);
}

/// An ungrouped aggregate holds one accumulator: it reads every row and
/// returns one; empty input gives the zero row of a `count`-only query, and
/// no row otherwise (#547 spec §5.2).
#[test]
fn ungrouped_aggregate_returns_one_row() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..3000).map(|i| format!("[:e{i} :n {i}]")).collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let mut cursor = db
        .query("(query [:find (count ?e) (sum ?v) :where [?e :n ?v]])")
        .unwrap();
    let batch = cursor.next_batch(1).unwrap().expect("a row");
    assert_eq!(batch.len(), 1);
    let c = cursor.counters();
    let agg = c
        .iter()
        .find(|c| c.op == "Aggregate")
        .expect("an Aggregate");
    assert_eq!(agg.rows_in, 3000);
    assert_eq!(agg.rows_out, 1);
    assert_eq!(agg.batches, 1);

    let result = |q: &str| {
        let before = runs();
        let QueryResult::QueryResults { results, .. } = db.execute(q).unwrap() else {
            panic!("expected query results");
        };
        assert_eq!(runs(), before + 1, "query ran on the engine");
        results
    };
    let zero =
        result("(query [:find (count ?e) (count-distinct ?v) :where [?e :n ?v] [(< ?v 0)]])");
    assert_eq!(
        zero,
        vec![vec![crate::Value::Integer(0), crate::Value::Integer(0)]]
    );
    let zero = result("(query [:find (count ?e) :with ?v :where [?e :n ?v] [(< ?v 0)]])");
    assert_eq!(zero, vec![vec![crate::Value::Integer(0)]]);
    assert!(result("(query [:find (sum ?v) :where [?e :n ?v] [(< ?v 0)]])").is_empty());
    assert!(result("(query [:find ?e (count ?v) :where [?e :n ?v] [(< ?v 0)]])").is_empty());
}

/// Groups come out in the order of their values, over several batches; an
/// error in a later group fails the query, and `min` of no value drops its
/// group (#547 spec §3.3, §5.3).
#[test]
fn grouped_aggregate_orders_groups_and_fails_in_a_later_group() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..3000)
        .map(|i| format!("[:e{i} :k {}] [:e{i} :n {i}]", 3000 - i))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let QueryResult::QueryResults { results, .. } = db
        .execute("(query [:find ?k (count ?e) :where [?e :k ?k]])")
        .unwrap()
    else {
        panic!("expected query results");
    };
    assert_eq!(results.len(), 3000);
    let keys: Vec<i64> = results
        .iter()
        .map(|r| match r.first() {
            Some(crate::Value::Integer(k)) => *k,
            _ => -1,
        })
        .collect();
    assert_eq!(keys, (1..=3000).collect::<Vec<_>>(), "groups in key order");

    db.execute("(transact [[:x :k 2999] [:x :n \"text\"] [:y :k 5] [:y :n nil]])")
        .unwrap();
    let before = runs();
    let err = db
        .execute("(query [:find ?k (sum ?n) :where [?e :k ?k] [?e :n ?n]])")
        .unwrap_err();
    assert_eq!(crate::MinigrafError::from(err).code(), "INT-041");
    assert_eq!(runs(), before + 1, "query ran on the engine");

    db.execute("(transact [[:z :k 7] [:z :m nil]])").unwrap();
    let QueryResult::QueryResults { results, .. } = db
        .execute("(query [:find ?k (min ?m) :where [?e :k ?k] [?e :m ?m]])")
        .unwrap()
    else {
        panic!("expected query results");
    };
    assert!(results.is_empty(), "the group with no value is dropped");
}

/// Window shapes (#547 PR 3): every built-in window over each kind of
/// value, partitions by keywords, refs, booleans, floats (`-0.0` and `0.0`
/// one partition) and long strings, ties broken by the named variables,
/// `:desc`, and windows over an aggregate's groups.
fn windows() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "?e (row-number :over (:order-by ?g))",
            "[?e :person/age ?g]",
        ),
        (
            "?e ?g (rank :over (:order-by ?g :desc)) (sum ?g :over (:order-by ?g))",
            "[?e :person/age ?g]",
        ),
        (
            "?s (count ?e :over (:partition-by ?s :order-by ?n)) (min ?n :over (:partition-by ?s :order-by ?n))",
            "[?e :person/status ?s] [?e :person/name ?n]",
        ),
        (
            "?f (max ?b :over (:partition-by ?f :order-by ?b :desc))",
            "[?e :person/friend ?f] [?e :person/bio ?b]",
        ),
        (
            "?x (sum ?x :over (:order-by ?x)) (avg ?x :over (:order-by ?x)) (row-number :over (:partition-by ?x :order-by ?x))",
            "[?e :person/score ?x]",
        ),
        (
            "?ok ?k (rank :over (:partition-by ?ok :order-by ?k))",
            "[?e :person/ok ?ok] [?e :person/alias ?k]",
        ),
        (
            "?v (count ?v :over (:order-by ?v)) (min ?v :over (:order-by ?v)) (max ?v :over (:order-by ?v))",
            "[?e ?a ?v]",
        ),
        (
            "?n (row-number :over (:order-by ?n))",
            "[?e :person/null ?n]",
        ),
        (
            "?s (count ?e) (rank :over (:order-by ?s :desc))",
            "[?e :person/status ?s]",
        ),
        (
            "?f (sum ?g) (sum ?f2 :over (:order-by ?f2)) ?f2",
            "[?e :person/friend ?f] [?e :person/age ?g] [?e :person/age ?f2]",
        ),
        (
            "?s (sum ?g) (row-number :over (:order-by ?s)) :with ?e",
            "[?e :person/status ?s] [?e :person/age ?g]",
        ),
        (
            "?g ?g (rank :over (:order-by ?g)) ?e",
            "[?e :person/age ?g]",
        ),
        (
            "?e (row-number :over (:order-by ?k))",
            "(or [?e :person/alias ?k] [?e :person/boss ?k])",
        ),
    ]
}

fn check_windows(db: &Minigraf) -> usize {
    let tx = db.current_tx_count();
    let mut n = 0;
    for (find, clauses) in windows() {
        for t in temporals(tx) {
            n += rows(db, &format!("(query [:find {find} {t} :where {clauses}])"));
        }
    }
    n += rows(
        db,
        "(query [:find ?e (max ?t :over (:partition-by ?e :order-by ?t)) :any-valid-time \
         :where [?e :person/name ?n] [?e :db/tx-count ?t]])",
    );
    n
}

#[test]
fn windows_match_executor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("win.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        for round in 0..3 {
            write_round(&db, round);
        }
        assert!(check_windows(&db) > 0, "some rows");
        db.checkpoint().unwrap();
        write_round(&db, 3);
        check_windows(&db);
    }
    let db = Minigraf::open(&path).unwrap();
    check_windows(&db);
}

/// `Window` returns each input row once, in input order, over several
/// batches; every row of a partition sees the whole partition, and peers
/// share a running value (#547 spec §4.2).
#[test]
fn window_returns_every_row_in_input_order() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..3000)
        .map(|i| format!("[:e{i} :k {}] [:e{i} :n {}]", i % 3, i / 2))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let mut cursor = db
        .query(
            "(query [:find ?e (sum ?n :over (:partition-by ?k :order-by ?n)) \
             (row-number :over (:order-by ?n)) :where [?e :k ?k] [?e :n ?n]])",
        )
        .unwrap();
    let mut rows = cursor.next_batch(1000).unwrap().expect("rows").into_rows();
    let c = cursor.counters();
    let window = c.iter().find(|c| c.op == "Window").expect("a Window");
    assert_eq!(window.rows_in, 3000, "the whole input is held");
    assert_eq!(window.batches, 1, "one batch passed on so far");
    while let Some(batch) = cursor.next_batch(1000).unwrap() {
        rows.extend(batch.into_rows());
    }
    assert_eq!(rows.len(), 3000);

    let mut numbers: Vec<i64> = rows
        .iter()
        .map(|r| match r.get(2) {
            Some(crate::Value::Integer(n)) => *n,
            _ => -1,
        })
        .collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (1..=3000).collect::<Vec<_>>(),
        "each row numbered once"
    );
    // A partition's last peers' running sum is the sum of the partition.
    let total = |k: i64| -> i64 { (0..3000).filter(|i| i % 3 == k).map(|i| i / 2).sum() };
    let max = rows
        .iter()
        .filter_map(|r| match r.get(1) {
            Some(crate::Value::Integer(n)) => Some(*n),
            _ => None,
        })
        .max();
    assert_eq!(
        max,
        (0..3).map(total).max(),
        "the last peers see the whole partition"
    );
}

/// Windows run on the engine, over an aggregate's groups too, and an empty
/// input gives no row.
#[test]
fn window_routing() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    for q in [
        "(query [:find ?e (rank :over (:order-by ?n)) :where [?e :person/name ?n]])",
        "(query [:find ?n (count ?e) (rank :over (:order-by ?n)) :where [?e :person/name ?n]])",
    ] {
        assert!(rows(&db, q) > 0, "rows");
    }
    assert_eq!(
        rows(
            &db,
            "(query [:find ?e (rank :over (:order-by ?n)) :where [?e :person/name ?n] [(= ?n \"none\")]])",
        ),
        0
    );
}

/// Which rule queries the engine runs (#548 spec §5.5): those whose body and
/// every evaluated rule body it plans, outside a write transaction and
/// without an `:as-of` timestamp.
#[test]
fn rule_query_routing() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :e :b] [:b :e :c] [:c :n 1]])")
        .unwrap();
    for rule in [
        "(rule [(reach ?x ?y) [?x :e ?y]])",
        "(rule [(reach ?x ?z) [?x :e ?y] (reach ?y ?z)])",
        "(rule [(one ?x ?v) [(+ 0 1) ?v] [(+ 0 2) ?x]])",
        "(rule [(deep ?x ?y) [?x :e ?y] (or (or [?y :e ?x] [?y :n 1]) [?x :n 2])])",
    ] {
        db.execute(rule).unwrap();
    }
    assert_eq!(rows(&db, "(query [:find ?x ?y :where (reach ?x ?y)])"), 3);
    assert_eq!(rows(&db, "(query [:find ?y :where (reach :a ?y)])"), 2);
    for (at, n) in [("2020-01-01T00:00:00Z", 0), ("2100-01-01T00:00:00Z", 3)] {
        let q = format!("(query [:find ?x ?y :as-of \"{at}\" :where (reach ?x ?y)])");
        assert_eq!(rows(&db, &q), n, "an :as-of timestamp runs on the engine");
    }
    assert_eq!(
        rows(
            &db,
            "(query [:find ?x :where [?x :e ?y] (not-join [?y] (reach ?y ?z))])"
        ),
        1
    );
    for (q, cause) in [
        ("(query [:find ?x ?v :where (one ?x ?v)])", Cause::NoPattern),
        ("(query [:find ?x :where (deep ?x ?y)])", Cause::Branch),
        (
            "(query [:find ?x :where (reach ?x ?y ?z)])",
            Cause::WideCall,
        ),
    ] {
        let before = runs();
        FALLBACKS.with(|f| f.borrow_mut().clear());
        let _ = db.execute(q);
        assert_eq!(runs(), before, "query stayed on the executor");
        assert_eq!(
            FALLBACKS.with(|f| f.borrow().first().copied()),
            Some(cause),
            "the planner names why"
        );
    }
    let before = runs();
    let mut tx = db.begin_write().unwrap();
    tx.execute("(transact [[:c :e :d]])").unwrap();
    tx.execute("(query [:find ?x ?y :where (reach ?x ?y)])")
        .unwrap();
    tx.rollback();
    assert!(
        runs() > before,
        "a write transaction reads on the engine (#550 spec §7)"
    );
}

/// The relations a rule query derives are read after the committed and
/// pending facts, in batches, on every scan kind (#548 spec §5.2).
#[test]
fn rule_queries_read_derived_rows_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("rules.graph")).unwrap();
    // A chain of 60 nodes: 1,770 reach facts, more than one batch.
    let edges: String = (0..59)
        .map(|i| format!("[:n{i} :e :n{}]", i + 1))
        .collect::<Vec<_>>()
        .join(" ");
    db.execute(&format!("(transact [{edges}])")).unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :e ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?z) (reach ?x ?y) [?y :e ?z]])")
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            rows(&db, "(query [:find ?x ?y :where (reach ?x ?y)])"),
            1770
        );
        assert_eq!(rows(&db, "(query [:find ?y :where (reach :n0 ?y)])"), 59);
        assert_eq!(
            rows(&db, "(query [:find ?x ?a :where [?x :e :n5] [?x ?a ?y]])"),
            1,
            "no rule invoked: no derived rows"
        );
        assert_eq!(
            rows(
                &db,
                "(query [:find ?x ?y :where [?x :reach ?y] (reach ?x ?y)])"
            ),
            1770,
            "a pattern of the relation"
        );
        db.checkpoint().unwrap();
    }
}

/// A rule body's delta run joins each later clause on a variable bound
/// before it where one can (#694). Magic sets turn `(reach :n0 ?to)` into
/// `(magic ?from) [?from :next ?mid] (reach ?mid ?to)`; the run from the
/// `reach` delta took `magic` next, sharing nothing with it, and joined
/// every delta row with every magic row.
#[test]
fn rule_delta_runs_join_on_a_bound_variable() {
    use super::plan::{JoinKind, Step};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::rules::RuleRegistry;
    use crate::query::datalog::types::DatalogCommand;
    let mut registry = RuleRegistry::new();
    for rule in [
        "(rule [(reach ?from ?to) [?from :next ?to]])",
        "(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])",
    ] {
        let DatalogCommand::Rule(r) = parse_datalog_command(rule).unwrap() else {
            panic!("expected a rule");
        };
        registry.register_rule("reach".to_string(), r).unwrap();
    }
    let DatalogCommand::Query(query) =
        parse_datalog_command("(query [:find ?to :where (reach :n0 ?to)])").unwrap()
    else {
        panic!("expected a query");
    };
    let (rewritten, seeds) =
        crate::query::datalog::magic_sets::rewrite(&query, &registry).expect("magic sets apply");
    let attrs: Vec<&str> = seeds.iter().map(|(_, a, _)| a.as_str()).collect();
    let plan = super::plan_rules(&query, &rewritten, &attrs).expect("the engine plans it");
    let mut runs = 0;
    for (_, rules) in &plan.strata {
        for rule in rules {
            for run in &rule.deltas {
                runs += 1;
                for step in &run.plan.steps {
                    if let Step::Join(j) = step {
                        assert!(
                            !(matches!(j.kind, JoinKind::Hash { .. }) && j.keys.is_empty()),
                            "a delta run of {} joins as a product",
                            rule.predicate
                        );
                    }
                }
            }
        }
    }
    assert!(runs >= 2, "the recursive rules have delta runs");

    // The answers on a fan-out tree, engine and executor agreeing.
    let db = Minigraf::in_memory().unwrap();
    let mut edges = String::new();
    for p in 0..11 {
        for c in 0..10 {
            edges.push_str(&format!("[:n{p} :next :n{}]", 1 + p * 10 + c));
        }
    }
    db.execute(&format!("(transact [{edges}])")).unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?to]])")
        .unwrap();
    db.execute("(rule [(reach ?from ?to) [?from :next ?mid] (reach ?mid ?to)])")
        .unwrap();
    assert_eq!(rows(&db, "(query [:find ?to :where (reach :n0 ?to)])"), 110);
    assert_eq!(rows(&db, "(query [:find ?to :where (reach :n1 ?to)])"), 10);
}

// ── #549: query memory accounting ──────────────────────────────────────────

/// A checkpointed database of `n` entities, each with integers `:a` and
/// `:b`, both its number. Queries over variable attributes
/// read ids straight from keys, so their first holder is an operator rather
/// than the query's `IdSpace`.
fn memory_db(dir: &std::path::Path, n: usize, limit: usize) -> Minigraf {
    let path = dir.join("memory.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        let facts: String = (0..n)
            .map(|i| format!("[:p{i} :a {i}] [:p{i} :b {i}]"))
            .collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
        db.checkpoint().unwrap();
    }
    crate::OpenOptions::new()
        .query_memory_limit(limit)
        .path(&path)
        .open()
        .unwrap()
}

/// One shape per operator that holds state, the operator that holds it
/// first, and the entities its database needs: more than one batch of
/// answer rows, few enough that the executor oracle's cross joins stay fast.
const MEMORY_SHAPES: [(&str, usize, &str); 5] = [
    (
        "HashJoin",
        300,
        "(query [:find ?e :where [?e ?a ?x] [?f ?b ?x]])",
    ),
    (
        "AntiJoin",
        300,
        "(query [:find ?e :where [?e ?a ?x] (not-join [?x] [?f ?b ?x] [(< ?x 100)])])",
    ),
    (
        "Union",
        300,
        "(query [:find ?e ?f :where [?e ?a ?x] \
         (or (and [?f ?b ?x] [(> ?x 5)]) (and [?f ?b ?x] [(< ?x 9)]))])",
    ),
    (
        "Aggregate",
        1100,
        "(query [:find ?x (count ?e) :where [?e ?a ?x]])",
    ),
    (
        "Window",
        1100,
        "(query [:find ?e (rank :over (:order-by ?x)) :where [?e ?a ?x]])",
    ),
];

/// Every holder charges while it holds state, and the meter is back at 0
/// once the stream is drained or dropped early (#549 spec §6.2).
#[test]
fn holders_charge_and_the_meter_returns_to_zero() {
    for (op, n, query) in MEMORY_SHAPES {
        let dir = tempfile::tempdir().unwrap();
        let db = memory_db(dir.path(), n, usize::MAX);
        let mut cursor = db.query(query).unwrap();
        let meter = cursor.stream_mut().expect("engine").meter().clone();
        assert!(cursor.next_batch(1).unwrap().is_some(), "{op}: a row");
        assert!(meter.peak() > 0, "{op}: charged");
        if op != "Union" {
            let held = cursor
                .stream_mut()
                .expect("engine")
                .counters()
                .iter()
                .filter(|c| c.op == op)
                .map(|c| c.bytes_held)
                .sum::<u64>();
            assert!(held > 0, "{op}: holds state between pulls");
        }
        while cursor.next_batch(4096).unwrap().is_some() {}
        assert_eq!(meter.used(), 0, "{op}: drained");

        let mut cursor = db.query(query).unwrap();
        let meter = cursor.stream_mut().expect("engine").meter().clone();
        cursor.next_batch(1).unwrap();
        drop(cursor);
        assert_eq!(meter.used(), 0, "{op}: dropped early");
    }
}

/// The holder that would go over the limit is named (#549 spec §4).
#[test]
fn qry_012_names_the_holder_over_the_limit() {
    for (op, n, query) in MEMORY_SHAPES {
        let dir = tempfile::tempdir().unwrap();
        let db = memory_db(dir.path(), n, 1);
        let err = db.execute(query).expect_err(op);
        assert_eq!(err.code(), "QRY-012", "{op}");
        let text = err.to_string();
        assert!(text.contains(&format!("{op} needs")), "{op}: {text}");
        assert!(text.contains("of its 1-byte limit"), "{op}: {text}");
    }

    // Pending entities take provisional ids.
    let db = crate::OpenOptions::new()
        .query_memory_limit(1)
        .open_memory()
        .unwrap();
    db.execute("(transact [[:x :a 1]])").unwrap();
    let err = db
        .execute("(query [:find ?e :where [?e :a ?v]])")
        .expect_err("ids");
    assert!(err.to_string().contains("IdSpace needs"), "IdSpace");

    // A rule query's candidate heads: enough for its ids, not its rows.
    let dir = tempfile::tempdir().unwrap();
    let db = memory_db(dir.path(), 300, 2000);
    db.execute("(rule [(r ?x ?y) [?x :a ?y]])").unwrap();
    let err = db
        .execute("(query [:find ?x ?y :where (r ?x ?y)])")
        .expect_err("rules");
    assert_eq!(err.code(), "QRY-012");
    assert!(err.to_string().contains("Recursive needs"), "Recursive");
}

/// A refused allocation is `QRY-012` naming the holder (spec §4, second
/// form), through the test hook.
#[test]
fn qry_012_allocator_form() {
    let dir = tempfile::tempdir().unwrap();
    let db = memory_db(dir.path(), 300, usize::MAX);
    for (op, _, query) in MEMORY_SHAPES {
        super::meter::FAIL_RESERVE.with(|f| f.set(Some(op)));
        let err = db.execute(query).expect_err(op);
        super::meter::FAIL_RESERVE.with(|f| f.set(None));
        assert_eq!(err.code(), "QRY-012", "{op}");
        let text = err.to_string();
        assert!(
            text.contains("the allocator refused") && text.ends_with(&format!("bytes for {op}")),
            "{op}: {text}"
        );
        db.execute(query).expect("the next query runs");
    }
}

/// A cursor that returned rows and then reaches the limit gives the error
/// on the next call; the database then answers other queries.
#[test]
fn cursor_returns_rows_then_qry_012() {
    let db = crate::OpenOptions::new()
        .query_memory_limit(120_000)
        .open_memory()
        .unwrap();
    let facts: String = (0..3000).map(|i| format!("[:p{i} :a {i}]")).collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    // Each new pending entity takes a provisional id, held to the end.
    let mut cursor = db.query("(query [:find ?e :where [?e :a ?v]])").unwrap();
    let mut rows = 0;
    let err = loop {
        match cursor.next_batch(1024) {
            Ok(Some(batch)) => rows += batch.len(),
            Ok(None) => panic!("expected QRY-012"),
            Err(e) => break e,
        }
    };
    assert!(rows >= 1024, "rows before the error");
    assert_eq!(err.code(), "QRY-012");
    assert!(cursor.next_batch(1024).unwrap().is_none(), "then done");
    let QueryResult::QueryResults { results, .. } =
        db.execute("(query [:find ?v :where [:p1 :a ?v]])").unwrap()
    else {
        panic!("expected query results");
    };
    assert_eq!(results.len(), 1, "the database answers");
}

/// Integers `0..n` in slot 0, in batches.
struct Ints {
    n: i64,
    at: i64,
}

impl super::Operator for Ints {
    fn next_batch(&mut self, _: &mut super::Ctx<'_>) -> anyhow::Result<Option<super::Batch>> {
        if self.at >= self.n {
            return Ok(None);
        }
        let mut batch = super::Batch::new(1);
        while batch.len < super::BATCH_ROWS && self.at < self.n {
            batch.push(&mut [Some(super::cell::Cell::Val(super::cell::Scalar::Int(
                self.at,
            )))]);
            self.at += 1;
        }
        Ok(Some(batch))
    }

    fn rewind(&mut self, _: &[super::cell::Cell]) {
        self.at = 0;
    }

    fn close(&mut self) {
        self.at = self.n;
    }

    fn counters(&self, _: &mut Vec<super::Counters>) {}
}

/// Pull `op` once with a fresh context over `meter`.
fn pull_once(
    op: &mut dyn super::Operator,
    meter: &std::sync::Arc<super::Meter>,
) -> anyhow::Result<Option<super::Batch>> {
    let snapshot = crate::graph::storage::FactStorage::new()
        .snapshot()
        .unwrap();
    let functions = crate::query::datalog::functions::FunctionRegistry::with_builtins();
    let mut ids = super::ids::IdSpace::new(None, meter);
    let mut ctx = super::Ctx {
        snapshot: &snapshot,
        keys: None,
        ids: &mut ids,
        functions: &functions,
        derived: None,
        bound: &crate::storage::TxBound::All,
    };
    op.next_batch(&mut ctx)
}

/// `rewind` and `close` release what an operator holds (spec §3.1).
#[test]
fn rewind_and_close_release() {
    let meter = super::Meter::unlimited();
    let ints = || Box::new(Ints { n: 3000, at: 0 });
    let mut distinct = super::distinct::Distinct::new(ints(), vec![0], &meter);
    let mut aggregate = super::aggregate::Aggregate::new(
        ints(),
        vec![0],
        vec![("count".to_string(), 0)],
        false,
        &meter,
    );
    let ops: [&mut dyn super::Operator; 2] = [&mut distinct, &mut aggregate];
    for op in ops {
        pull_once(op, &meter).unwrap().expect("a batch");
        assert!(meter.used() > 0, "held after a pull");
        op.rewind(&[]);
        assert_eq!(meter.used(), 0, "released by rewind");
        pull_once(op, &meter).unwrap().expect("a batch");
        assert!(meter.used() > 0, "held again");
        op.close();
        assert_eq!(meter.used(), 0, "released by close");
    }
}

/// Each operator over a small limit names itself; a rule query's relation
/// too.
#[test]
fn small_limit_in_each_holder() {
    let meter = super::Meter::new(100);
    let mut distinct =
        super::distinct::Distinct::new(Box::new(Ints { n: 10, at: 0 }), vec![0], &meter);
    let err = pull_once(&mut distinct, &meter).unwrap_err();
    assert!(err.to_string().contains("Distinct needs"), "Distinct");
    super::Operator::close(&mut distinct);
    assert_eq!(meter.used(), 0, "nothing counted after the error and close");

    let mut aggregate = super::aggregate::Aggregate::new(
        Box::new(Ints { n: 10, at: 0 }),
        vec![0],
        vec![("count".to_string(), 0)],
        false,
        &meter,
    );
    let err = pull_once(&mut aggregate, &meter).unwrap_err();
    assert!(err.to_string().contains("Aggregate needs"), "Aggregate");

    let meter = super::Meter::new(300);
    let mut rels = super::relation::Relations::new([7], &meter);
    let row = |n| {
        [
            super::cell::Cell::Eid(n),
            super::cell::Cell::Val(super::cell::Scalar::Int(1)),
        ]
    };
    rels.add(0, row(1)).unwrap();
    let err = (2..10)
        .find_map(|n| rels.add(0, row(n)).err())
        .expect("limit");
    assert!(err.to_string().contains("Relations needs"), "Relations");
    assert!(meter.used() > 0);
    drop(rels);
    assert_eq!(meter.used(), 0, "released on drop");
}

/// A join with no key builds on its inner pattern unless the outer side's
/// product estimate is smaller (#549 spec §6.1).
#[test]
fn planner_builds_a_keyless_join_on_the_smaller_product() {
    use super::plan::{JoinKind, Step, plan};
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    let sides = |q: &str| -> Vec<bool> {
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(q) else {
            panic!("expected a query");
        };
        plan(&query)
            .expect("an engine plan")
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Join(j) => match j.kind {
                    JoinKind::Hash { build_inner } => Some(build_inner),
                    JoinKind::Probe(_) => None,
                },
                _ => None,
            })
            .collect()
    };
    // #575: the outer side is 100 rows, then at least 100 × 10,000.
    assert_eq!(
        sides("(query [:find ?n :where [?e0 :n ?n] [?e1 ?a1 ?v1] [?e2 ?a2 ?v2] [?e3 ?a3 ?v3]])"),
        [false, true, true]
    );
    assert_eq!(
        sides("(query [:find ?e :where [?e :name \"x\"] [?p ?q ?r]])"),
        [false],
        "a selective outer side"
    );
    assert_eq!(
        sides("(query [:find ?e :where [?e :a ?x] [?f :b ?y]])"),
        [true],
        "a tie builds inner"
    );
    let _off = crate::query::datalog::optimizer::set_reorder(false);
    assert_eq!(
        sides("(query [:find ?n :where [?e0 :n ?n] [?e1 ?a1 ?v1] [?e2 ?a2 ?v2]])"),
        [true, true],
        "reorder off builds inner"
    );
}

/// The two acceptance shapes of `tests/memory_limit_acceptance_test.rs`:
/// a checkpointed database at `path`, and the query.
fn acceptance_shape(name: &str, path: &std::path::Path) -> String {
    let db = Minigraf::open(path).unwrap();
    if name == "groups" {
        for chunk in 0..10 {
            let facts: String = (chunk * 100_000..(chunk + 1) * 100_000)
                .map(|i| format!("[:e{i} :g {i}]"))
                .collect();
            db.execute(&format!("(transact [{facts}])")).unwrap();
            db.checkpoint().unwrap();
        }
        "(query [:find ?g (count ?e) :where [?e :g ?g]])".to_string()
    } else {
        db.execute(r#"(transact [[:alice :name "Alice"] [:bob :name "Bob"] [:alice :age 30]])"#)
            .unwrap();
        db.checkpoint().unwrap();
        let patterns: String = (0..13).map(|i| format!("[?e{i} ?a{i} ?v{i}]")).collect();
        format!("(query [:find ?v0 :where {patterns}])")
    }
}

/// A `kB` field of `/proc/self/status`; 0 where it does not exist.
fn proc_status_kb(field: &str) -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(field)?.strip_prefix(':').map(str::to_owned))
        })
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// The meter's peak against the peak RSS the query adds, for both
/// acceptance shapes (#549 spec §3.2: the estimate is meant to be within a
/// factor of two). Opens the engine directly, without the executor oracle,
/// which would run out of memory on the cartesian shape. Prints only; run
/// alone, as the high-water mark is per process:
///
/// ```text
/// cargo test --profile bench --lib meter_peak_against_rss -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn meter_peak_against_rss() {
    use crate::query::datalog::functions::FunctionRegistry;
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;
    for name in ["groups", "cartesian"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shape.graph");
        let query = acceptance_shape(name, &path);
        let db = Minigraf::open(&path).unwrap();
        let Ok(DatalogCommand::Query(query)) = parse_datalog_command(&query) else {
            panic!("expected a query");
        };
        let plan = super::plan(&query).expect("an engine plan");
        let functions =
            std::sync::Arc::new(std::sync::RwLock::new(FunctionRegistry::with_builtins()));
        let snapshot = db.snapshot().unwrap();
        let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
        let before_kb = proc_status_kb("VmRSS");
        let mut stream =
            super::open(snapshot, plan, functions, usize::MAX).expect("the engine runs it");
        let meter = stream.meter().clone();
        let mut rows = 0;
        while let Some(batch) = stream.next_rows().unwrap() {
            rows += batch.len();
        }
        drop(stream);
        let added_kb = proc_status_kb("VmHWM").saturating_sub(before_kb);
        eprintln!(
            "{name}: {rows} rows; meter peak {} KiB, peak RSS added {added_kb} KiB{}",
            meter.peak() / 1024,
            if reset {
                ""
            } else {
                " (high-water mark not reset)"
            }
        );
        assert_eq!(meter.used(), 0, "released after the drain");
    }
}

/// `Aggregate::finish` charges its sorted map of groups until every row is
/// built, and lets the hash table and group rows go first, so once rows are
/// returned only the ones not yet returned stay charged (#549).
#[test]
fn aggregate_finish_charges_its_sorted_groups() {
    use super::meter::{HASH_ENTRY_OVERHEAD, row_bytes, value_bytes};
    use crate::graph::types::Value;
    use crate::query::datalog::aggregate::Agg;
    use std::mem::size_of;
    let n = 3000;
    let meter = super::Meter::unlimited();
    let mut aggregate = super::aggregate::Aggregate::new(
        Box::new(Ints { n, at: 0 }),
        vec![0],
        vec![("count".to_string(), 0)],
        false,
        &meter,
    );
    let batch = pull_once(&mut aggregate, &meter).unwrap().expect("a batch");
    let row = [
        super::cell::Cell::Val(super::cell::Scalar::Int(0)),
        super::cell::Cell::Val(super::cell::Scalar::Int(1)),
    ];
    let n = usize::try_from(n).unwrap();
    assert_eq!(
        meter.used(),
        (n - batch.len) * row_bytes(&row),
        "only the rows not yet returned"
    );
    let sorted = 2 * (size_of::<Vec<Value>>() + value_bytes(&Value::Integer(0)))
        + size_of::<Vec<Agg>>()
        + size_of::<Agg>()
        + HASH_ENTRY_OVERHEAD;
    assert!(
        meter.peak() >= n * (sorted + row_bytes(&row)),
        "the sorted groups and the finished rows at once"
    );
}

/// A `sum-distinct` set's members stay charged once, in the sorted map,
/// until the rows are built (#549).
#[test]
fn aggregate_finish_keeps_sum_distinct_members_charged_once() {
    use super::meter::row_bytes;
    let n = 3000;
    let meter = super::Meter::unlimited();
    let mut aggregate = super::aggregate::Aggregate::new(
        Box::new(Ints { n, at: 0 }),
        vec![],
        vec![("sum-distinct".to_string(), 0)],
        false,
        &meter,
    );
    pull_once(&mut aggregate, &meter).unwrap().expect("a batch");
    assert_eq!(meter.used(), 0, "the one row returned, nothing held");
    let members = usize::try_from(n).unwrap()
        * (super::meter::value_bytes(&crate::graph::types::Value::Integer(0))
            + super::meter::HASH_ENTRY_OVERHEAD);
    let row = [super::cell::Cell::Val(super::cell::Scalar::Int(0))];
    assert!(meter.peak() >= members, "members charged");
    assert!(
        meter.peak() < 2 * members + row_bytes(&row),
        "members not counted twice"
    );
}
