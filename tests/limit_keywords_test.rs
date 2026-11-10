#![cfg(not(target_arch = "wasm32"))]
//! `:query-memory-limit`, bind slots on the limit keywords and the
//! `PreparedQuery` limit setters (#870, spec #862 §3–§5).

use minigraf::{BindValue, Minigraf, MinigrafError, OpenOptions, Overridable, QueryResult, Value};

const RULES: [&str; 2] = [
    "(rule [(reach ?x ?y) [?x :edge ?y]])",
    "(rule [(reach ?x ?y) [?x :edge ?z] (reach ?z ?y)])",
];
/// Holds aggregate groups: fails under a small memory limit.
const GROUPED: &str = "(query [:find ?x (count ?y) :where [?e :v ?x] [?e :w ?y]])";
/// Projects only integers: holds no state.
const VALUES: &str = "(query [:find ?x :where [?e :v ?x] [(> ?x 1)]])";
/// Derives 10 facts: a chain of 4 edges reaches 4 + 3 + 2 + 1 pairs.
const REACH: &str = "(query [:find ?x ?y :where (reach ?x ?y)])";

fn with_memory_keyword() -> OpenOptions {
    OpenOptions::new().allow_overrides(
        Overridable::DEFAULTS
            .iter()
            .copied()
            .chain([Overridable::QueryMemoryLimit]),
    )
}

fn load(db: &Minigraf) {
    let facts: String = (0..40)
        .map(|i| format!("[:p{i} :v {}] [:p{i} :w {i}]", i % 5))
        .collect();
    db.execute(&format!("(transact [{facts}])"))
        .expect("transact");
    db.execute("(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d] [:d :edge :e]])")
        .expect("edges");
    for rule in RULES {
        db.execute(rule).expect("rule");
    }
}

fn setup(opts: OpenOptions) -> Minigraf {
    let db = opts.open_memory().expect("open");
    load(&db);
    db
}

fn rows(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    }
}

fn code(r: Result<QueryResult, MinigrafError>) -> &'static str {
    r.err().expect("an error").code()
}

fn with(query: &str, opt: &str) -> String {
    let (head, _) = query.rsplit_once("])").expect("query vector");
    format!("{head} {opt}])")
}

fn int(n: i64) -> BindValue {
    BindValue::Val(Value::Integer(n))
}

// ─── :query-memory-limit ─────────────────────────────────────────────────────

#[test]
fn memory_keyword_lowers_the_database_limit() {
    let db = setup(with_memory_keyword());
    assert_eq!(rows(db.execute(GROUPED).expect("unlimited")), 5);
    assert_eq!(
        code(db.execute(&with(GROUPED, ":query-memory-limit 64"))),
        "QRY-012"
    );
}

#[test]
fn memory_keyword_raises_the_database_limit() {
    let db = setup(with_memory_keyword().query_memory_limit(64));
    assert_eq!(code(db.execute(GROUPED)), "QRY-012");
    let q = with(GROUPED, ":query-memory-limit 100000000");
    assert_eq!(rows(db.execute(&q).expect("raised")), 5);
    assert_eq!(db.query(&q).expect("cursor").count(), 5);
}

#[test]
fn memory_keyword_zero_refuses_every_stateful_query() {
    let db = setup(with_memory_keyword());
    // Uncheckpointed facts take provisional ids, which is state.
    assert_eq!(
        code(db.execute(&with(VALUES, ":query-memory-limit 0"))),
        "QRY-012"
    );
    assert_eq!(
        code(db.execute(&with(GROUPED, ":query-memory-limit 0"))),
        "QRY-012"
    );
    assert_eq!(
        code(db.execute(&with(REACH, ":query-memory-limit 0"))),
        "QRY-012"
    );
}

#[test]
fn memory_keyword_applies_in_a_write_transaction() {
    let db = setup(with_memory_keyword());
    let mut tx = db.begin_write().expect("begin");
    assert_eq!(
        code(tx.execute(&with(GROUPED, ":query-memory-limit 64"))),
        "QRY-012"
    );
    tx.rollback();
}

#[test]
fn memory_keyword_is_refused_by_default() {
    let db = setup(OpenOptions::new());
    let q = with(VALUES, ":query-memory-limit 1000000");
    let refused = |e: MinigrafError| {
        assert_eq!(e.code(), "API-025");
        assert!(
            e.to_string().contains(":query-memory-limit"),
            "names the keyword"
        );
        assert!(
            e.to_string().contains("QueryMemoryLimit"),
            "names the variant"
        );
    };
    refused(db.execute(&q).err().expect("execute"));
    refused(db.query(&q).err().expect("query"));
    refused(db.prepare(&q).err().expect("prepare"));
    refused(
        db.prepare(&with(VALUES, ":query-memory-limit $m"))
            .err()
            .expect("slot"),
    );
    let mut tx = db.begin_write().expect("begin");
    refused(tx.execute(&q).err().expect("tx"));
    tx.rollback();
}

#[test]
fn memory_keyword_alone_in_the_list_refuses_the_others() {
    let db = setup(OpenOptions::new().allow_overrides([Overridable::QueryMemoryLimit]));
    assert_eq!(
        rows(
            db.execute(&with(VALUES, ":query-memory-limit 1000000"))
                .expect("allowed")
        ),
        24
    );
    assert_eq!(
        code(db.execute(&with(REACH, ":max-derived-facts 100"))),
        "API-025"
    );
}

#[test]
fn memory_keyword_validation() {
    let db = setup(with_memory_keyword());
    for (opt, want) in [
        (":query-memory-limit 1 :query-memory-limit 1", "INT-012"),
        (":query-memory-limit", "INT-013"),
        (":query-memory-limit -1", "INT-015"),
        (":query-memory-limit \"x\"", "INT-015"),
        (":query-memory-limit 1.5", "INT-015"),
    ] {
        assert_eq!(code(db.execute(&with(VALUES, opt))), want, "{opt}");
    }
}

#[test]
fn memory_keyword_on_a_file_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("k.graph");
    let db = with_memory_keyword().path(&path).open().expect("open");
    load(&db);
    db.checkpoint().expect("checkpoint");
    // Checkpointed facts need no ids unless a row returns an entity.
    assert_eq!(
        rows(
            db.execute(&with(VALUES, ":query-memory-limit 0"))
                .expect("stateless")
        ),
        24
    );
    let zero = db
        .prepare(VALUES)
        .expect("prepare")
        .with_query_memory_limit(0);
    assert_eq!(rows(zero.execute(&[]).expect("setter 0")), 24);
    let entities = "(query [:find ?e :where [?e :v ?x] [(> ?x 1)] :query-memory-limit 0])";
    assert_eq!(code(db.execute(entities)), "QRY-012");
    assert_eq!(
        code(db.execute(&with(GROUPED, ":query-memory-limit 0"))),
        "QRY-012"
    );
    assert_eq!(
        code(db.execute(&with(GROUPED, ":query-memory-limit 64"))),
        "QRY-012"
    );
}

// ─── Bind slots ──────────────────────────────────────────────────────────────

#[test]
fn derived_facts_slot_changes_per_execution() {
    let db = setup(OpenOptions::new());
    for kw in [":max-derived-facts", ":max-results"] {
        let pq = db
            .prepare(&with(REACH, &format!("{kw} $n")))
            .expect("prepare");
        assert_eq!(code(pq.execute(&[("n", int(1))])), "INT-020", "{kw} 1");
        assert_eq!(
            rows(pq.execute(&[("n", int(1000))]).expect("1000")),
            10,
            "{kw} 1000"
        );
        assert_eq!(pq.query(&[("n", int(1000))]).expect("cursor").count(), 10);
    }
}

#[test]
fn memory_slot_changes_per_execution() {
    let db = setup(with_memory_keyword());
    let pq = db
        .prepare(&with(GROUPED, ":query-memory-limit $m"))
        .expect("prepare");
    assert_eq!(code(pq.execute(&[("m", int(0))])), "QRY-012");
    assert_eq!(rows(pq.execute(&[("m", int(i64::MAX))]).expect("max")), 5);
}

#[test]
fn limit_slot_errors() {
    let db = setup(with_memory_keyword());
    let derived = db
        .prepare(&with(REACH, ":max-derived-facts $n"))
        .expect("prepare");
    let memory = db
        .prepare(&with(GROUPED, ":query-memory-limit $m"))
        .expect("prepare");
    let string = BindValue::Val(Value::String("x".into()));
    assert_eq!(code(derived.execute(&[("n", string.clone())])), "API-024");
    assert_eq!(
        code(derived.execute(&[("n", BindValue::TxCount(5))])),
        "API-024"
    );
    assert_eq!(code(derived.execute(&[])), "API-022");
    assert_eq!(code(derived.execute(&[("n", int(0))])), "INT-014");
    assert_eq!(code(memory.execute(&[("m", string)])), "API-024");
    assert_eq!(code(memory.execute(&[("m", int(-1))])), "INT-015");
    assert_eq!(code(memory.execute(&[])), "API-022");
    let results = db
        .prepare(&with(REACH, ":max-results $n"))
        .expect("prepare");
    assert_eq!(code(results.execute(&[("n", int(-3))])), "INT-014");
}

#[test]
fn unbound_limit_slots_are_refused_by_execute() {
    let db = setup(with_memory_keyword());
    for opt in [
        ":max-derived-facts $n",
        ":max-results $n",
        ":query-memory-limit $n",
    ] {
        let q = with(REACH, opt);
        assert_eq!(code(db.execute(&q)), "API-010", "{opt}");
        assert_eq!(
            db.query(&q).err().expect("query").code(),
            "API-010",
            "{opt}"
        );
    }
}

#[test]
fn empty_list_refuses_slots_at_prepare() {
    let db = setup(OpenOptions::new().allow_overrides([]));
    for opt in [":max-derived-facts $n", ":max-results $n"] {
        let err = db.prepare(&with(REACH, opt)).err().expect("refused");
        assert_eq!(err.code(), "API-025", "{opt}");
    }
}

// ─── Setters ─────────────────────────────────────────────────────────────────

#[test]
fn derived_facts_setter_replaces_the_database_value() {
    let db = setup(OpenOptions::new());
    let low = db
        .prepare(REACH)
        .expect("prepare")
        .with_max_derived_facts(1);
    assert_eq!(code(low.execute(&[])), "INT-020");
    assert_eq!(low.query(&[]).err().expect("cursor").code(), "INT-020");

    let db = setup(OpenOptions::new().max_derived_facts(1));
    assert_eq!(code(db.execute(REACH)), "INT-020");
    let high = db
        .prepare(REACH)
        .expect("prepare")
        .with_max_derived_facts(1000);
    assert_eq!(rows(high.execute(&[]).expect("raised")), 10);
}

#[test]
fn keyword_overrides_the_setter() {
    let db = setup(OpenOptions::new());
    let pq = db
        .prepare(&with(REACH, ":max-derived-facts 1000"))
        .expect("prepare")
        .with_max_derived_facts(1);
    assert_eq!(rows(pq.execute(&[]).expect("keyword wins")), 10);

    let db = setup(with_memory_keyword());
    let pq = db
        .prepare(&with(GROUPED, ":query-memory-limit $m"))
        .expect("prepare")
        .with_query_memory_limit(usize::MAX);
    assert_eq!(code(pq.execute(&[("m", int(64))])), "QRY-012");
}

#[test]
fn memory_setter_replaces_the_database_value() {
    let db = setup(OpenOptions::new());
    let low = db
        .prepare(GROUPED)
        .expect("prepare")
        .with_query_memory_limit(64);
    assert_eq!(code(low.execute(&[])), "QRY-012");

    let db = setup(OpenOptions::new().query_memory_limit(64));
    let high = db
        .prepare(GROUPED)
        .expect("prepare")
        .with_query_memory_limit(usize::MAX);
    assert_eq!(rows(high.execute(&[]).expect("raised")), 5);
    assert_eq!(high.query(&[]).expect("cursor").count(), 5);
}

#[test]
fn setters_are_not_gated() {
    let db = setup(OpenOptions::new().allow_overrides([]).max_derived_facts(1));
    let pq = db
        .prepare(REACH)
        .expect("prepare")
        .with_max_derived_facts(1000)
        .with_query_memory_limit(usize::MAX);
    assert_eq!(rows(pq.execute(&[]).expect("setters")), 10);
}

// ─── :max-iterations (#861) ──────────────────────────────────────────────────

/// Rounds the 4-edge chain's `reach` stratum needs (calibrated: one fewer fails).
const REACH_ROUNDS: i64 = 5;

fn with_iterations(query: &str, n: i64) -> String {
    with(query, &format!(":max-iterations {n}"))
}

#[test]
fn default_round_limit_allows_long_chains() {
    // 1,500 rounds: over the old fixed limit of 1,000, under 1,000,000.
    let db = Minigraf::in_memory().expect("open");
    let edges: String = (0..1500)
        .map(|i| format!("[:n{i} :next :n{}]", i + 1))
        .collect();
    db.execute(&format!("(transact [{edges}])")).expect("edges");
    db.execute("(rule [(down ?y) [:n0 :next ?y]])")
        .expect("rule");
    db.execute("(rule [(down ?y) (down ?x) [?x :next ?y]])")
        .expect("rule");
    let r = db
        .execute("(query [:find ?y :where (down ?y)])")
        .expect("long chain");
    assert_eq!(rows(r), 1500);
}

#[test]
fn open_option_sets_the_round_limit() {
    let db = setup(OpenOptions::new().max_iterations(REACH_ROUNDS as usize - 1));
    let err = db.execute(REACH).err().expect("over the limit");
    assert_eq!(err.code(), "INT-020");
    let text = err.to_string();
    assert!(text.contains(":max-iterations"), "says how to raise it");
    assert!(
        text.contains("OpenOptions::max_iterations"),
        "names the option"
    );
    let db = setup(OpenOptions::new().max_iterations(REACH_ROUNDS as usize));
    assert_eq!(rows(db.execute(REACH).expect("at the limit")), 10);
    let db = setup(OpenOptions::new().max_iterations(0));
    assert_eq!(code(db.execute(REACH)), "INT-020");
}

#[test]
fn iterations_keyword_replaces_the_database_value() {
    let db = setup(OpenOptions::new());
    assert_eq!(
        code(db.execute(&with_iterations(REACH, REACH_ROUNDS - 1))),
        "INT-020"
    );
    let db = setup(OpenOptions::new().max_iterations(1));
    assert_eq!(code(db.execute(REACH)), "INT-020");
    assert_eq!(
        rows(
            db.execute(&with_iterations(REACH, REACH_ROUNDS))
                .expect("raised")
        ),
        10
    );
    let mut tx = db.begin_write().expect("tx");
    assert_eq!(
        rows(
            tx.execute(&with_iterations(REACH, REACH_ROUNDS))
                .expect("in tx")
        ),
        10
    );
    tx.rollback();
}

#[test]
fn rounds_are_counted_per_stratum() {
    let db = setup(OpenOptions::new());
    db.execute("(transact [[:f :edge2 :g] [:g :edge2 :h] [:h :edge2 :i] [:i :edge2 :j]])")
        .expect("edges2");
    db.execute("(rule [(reach2 ?x ?y) [?x :edge2 ?y] (not (reach ?x ?y))])")
        .expect("rule");
    db.execute("(rule [(reach2 ?x ?y) [?x :edge2 ?z] (reach2 ?z ?y)])")
        .expect("rule");
    let q = "(query [:find ?x ?y :where (reach2 ?x ?y)])";
    // Two strata of REACH_ROUNDS each: a total count would need twice as many.
    assert_eq!(
        rows(
            db.execute(&with_iterations(q, REACH_ROUNDS))
                .expect("per stratum")
        ),
        10
    );
    assert_eq!(
        code(db.execute(&with_iterations(q, REACH_ROUNDS - 1))),
        "INT-020"
    );
}

#[test]
fn iterations_keyword_is_in_the_defaults_and_gated() {
    assert!(Overridable::DEFAULTS.contains(&Overridable::MaxIterations));
    assert_eq!(Overridable::MaxIterations.keyword(), ":max-iterations");
    let db = setup(OpenOptions::new().allow_overrides([]));
    let q = with_iterations(REACH, 10);
    assert_eq!(code(db.execute(&q)), "API-025");
    assert_eq!(db.query(&q).err().expect("query").code(), "API-025");
    assert_eq!(db.prepare(&q).err().expect("prepare").code(), "API-025");
    let err = db.execute(&q).err().expect("refused").to_string();
    assert!(
        err.contains("Overridable::MaxIterations"),
        "names the variant"
    );
    let db = setup(OpenOptions::new().allow_overrides([Overridable::MaxIterations]));
    assert_eq!(rows(db.execute(&q).expect("listed")), 10);
}

#[test]
fn iterations_keyword_validation() {
    let db = setup(OpenOptions::new());
    for (opt, want) in [
        (":max-iterations 3 :max-iterations 4", "INT-012"),
        (":max-iterations", "INT-013"),
        (":max-iterations 0", "INT-014"),
        (":max-iterations -2", "INT-014"),
        (":max-iterations \"x\"", "INT-015"),
    ] {
        assert_eq!(code(db.execute(&with(REACH, opt))), want, "{opt}");
    }
}

#[test]
fn iterations_slot_changes_per_execution() {
    let db = setup(OpenOptions::new());
    let pq = db
        .prepare(&with(REACH, ":max-iterations $n"))
        .expect("prepare");
    assert_eq!(code(pq.execute(&[("n", int(REACH_ROUNDS - 1))])), "INT-020");
    assert_eq!(
        rows(pq.execute(&[("n", int(REACH_ROUNDS))]).expect("ok")),
        10
    );
    assert_eq!(
        pq.query(&[("n", int(REACH_ROUNDS))])
            .expect("cursor")
            .count(),
        10
    );
    assert_eq!(code(pq.execute(&[("n", int(0))])), "INT-014");
    let string = BindValue::Val(Value::String("x".into()));
    assert_eq!(code(pq.execute(&[("n", string)])), "API-024");
    assert_eq!(code(pq.execute(&[])), "API-022");
    assert_eq!(
        code(db.execute(&with(REACH, ":max-iterations $n"))),
        "API-010"
    );
}

#[test]
fn iterations_setter_replaces_the_database_value() {
    let db = setup(OpenOptions::new());
    let low = db
        .prepare(REACH)
        .expect("prepare")
        .with_max_iterations(REACH_ROUNDS as usize - 1);
    assert_eq!(code(low.execute(&[])), "INT-020");
    assert_eq!(low.query(&[]).err().expect("cursor").code(), "INT-020");

    let db = setup(OpenOptions::new().allow_overrides([]).max_iterations(1));
    let high = db
        .prepare(REACH)
        .expect("prepare")
        .with_max_iterations(REACH_ROUNDS as usize);
    assert_eq!(rows(high.execute(&[]).expect("ungated")), 10);

    let db = setup(OpenOptions::new());
    let pq = db
        .prepare(&with_iterations(REACH, REACH_ROUNDS))
        .expect("prepare")
        .with_max_iterations(1);
    assert_eq!(rows(pq.execute(&[]).expect("keyword wins")), 10);
}

// ─── Clone (#872) ────────────────────────────────────────────────────────────

#[test]
fn prepared_query_clone_is_independent() {
    let db = setup(OpenOptions::new());
    let pq = db.prepare(REACH).expect("prepare");
    let low = pq.clone().with_max_derived_facts(1);
    assert_eq!(code(low.execute(&[])), "INT-020");
    assert_eq!(rows(pq.execute(&[]).expect("source unchanged")), 10);

    let low = db.prepare(REACH).expect("prepare").with_max_iterations(1);
    let high = low.clone().with_max_iterations(REACH_ROUNDS as usize);
    assert_eq!(rows(high.execute(&[]).expect("copy raised")), 10);
    assert_eq!(code(low.execute(&[])), "INT-020");

    let grouped = db.prepare(GROUPED).expect("prepare");
    let small = grouped.clone().with_query_memory_limit(64);
    assert_eq!(code(small.execute(&[])), "QRY-012");
    assert_eq!(rows(grouped.execute(&[]).expect("source unchanged")), 5);
}
