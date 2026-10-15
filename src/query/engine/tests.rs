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
        // that name it; floats match within an epsilon.
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
    // Pseudo-attributes across joins (#545): the metadata of the last
    // pattern before them, in plan order, that matched their entity, chosen
    // per row; pseudo-attribute patterns with no such pattern before them
    // join with every fact of their entity.
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
        "[?e :db/tx-count ?t] [?e :person/age ?a]",
        "[?e :db/tx-count 1] [?e :person/name ?n]",
        "[?e :db/tx-count 2] [?f :db/tx-count 2] [?e :person/self ?f]",
        "[?e :db/valid-from ?f] [?e :db/valid-to ?u]",
        "[:p1 :db/tx-count ?t]",
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
        let find = vars_of(clauses);
        for temporal in &pseudo_temporals {
            out.push(format!(
                "(query [:find {find} {temporal} :where {clauses}])"
            ));
        }
    }
    for clauses in valid_at {
        let find = vars_of(clauses);
        for temporal in temporals(tx) {
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

/// The executor still runs what the engine does not: aggregates, `:as-of` a
/// timestamp, a pseudo-attribute pattern whose metadata comes from a pattern
/// chosen per row in a way the engine does not reproduce (another entity's
/// pattern before it, a `_` entity), an expression rebinding an entity.
#[test]
fn other_queries_stay_on_the_executor() {
    let db = Minigraf::in_memory().unwrap();
    write_round(&db, 0);
    for q in [
        "(query [:find (count ?e) :where [?e :person/name ?n]])",
        "(query [:find ?e :as-of \"2099-01-01T00:00:00Z\" :where [?e :person/name ?n]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :person/friend ?f] [?f :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [_ :person/name ?e] [_ :db/tx-count ?t]])",
        "(query [:find ?n ?t :valid-at :any-valid-time \
         :where [?e :person/name ?n] [(+ 1 2) ?e] [?e :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :person/name \"n1\"] [_ :person/age ?a] [?e :db/tx-count ?t]])",
        "(query [:find ?e ?t :valid-at :any-valid-time \
         :where [?e :person/age ?a] [(+ ?a 1) ?b] [?b :db/tx-count ?t]])",
    ] {
        let before = runs();
        db.execute(q).unwrap();
        assert_eq!(runs(), before, "query stayed on the executor");
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
    // pattern (`wasm` keeps the cost choice).
    let _off = crate::query::datalog::optimizer::set_reorder(false);
    if !cfg!(feature = "wasm") {
        assert_eq!(
            kinds("(query [:find ?e :where [?e :a 1] [?f :b ?y] [?f :c ?e]])"),
            ["hash inner=true", "probe"]
        );
    }
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
