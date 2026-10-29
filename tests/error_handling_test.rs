//! Integration-level error paths and aggregate type-boundary regressions.
//! Invalid programs still propagate errors; aggregate values follow #789.

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

// ── Aggregate type boundaries ────────────────────────────────────────────────

fn single_value(db: &Minigraf, query: &str, expected: Value) {
    let QueryResult::QueryResults { results, .. } = db.execute(query).unwrap() else {
        panic!("expected query results");
    };
    assert!(results == vec![vec![expected]]);
}

#[test]
fn sum_string_attribute_returns_zero() {
    let db = db();
    db.execute(r#"(transact [[:a :score "high"] [:b :score "low"]])"#)
        .unwrap();
    single_value(
        &db,
        "(query [:find (sum ?s) :where [?e :score ?s]])",
        Value::Integer(0),
    );
}

#[test]
fn sum_mixed_int_string_skips_string() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score "twenty"]])"#)
        .unwrap();
    single_value(
        &db,
        "(query [:find (sum ?s) :where [?e :score ?s]])",
        Value::Integer(10),
    );
}

#[test]
fn max_boolean_attribute_returns_true() {
    let db = db();
    db.execute(r#"(transact [[:a :flag true] [:b :flag false]])"#)
        .unwrap();
    single_value(
        &db,
        "(query [:find (max ?f) :where [?e :flag ?f]])",
        Value::Boolean(true),
    );
}

// ── Rule-level errors ─────────────────────────────────────────────────────────

/// Registering two rules that form a negative cycle must fail.
/// p depends on not-q; q depends on not-p → unstratifiable.
#[test]
fn negative_cycle_pair_rejected() {
    let db = db();
    db.execute(r#"(rule [(p ?x) [?x :base true] (not (q ?x))])"#)
        .unwrap();
    let r = db.execute(r#"(rule [(q ?x) [?x :base true] (not (p ?x))])"#);
    assert!(
        r.is_err(),
        "negative cycle p not-q and q not-p must be rejected"
    );
    let msg = r.unwrap_err().to_string();
    assert!(
        msg.contains("negative cycle") || msg.contains("unstratifiable"),
        "error must mention the cycle"
    );
}

/// An or branch that creates a negative cycle must also be rejected.
#[test]
fn or_negative_cycle_rejected() {
    let db = db();
    // base rule: safe depends on not-unsafe
    db.execute(r#"(rule [(safe ?x) [?x :item true] (not (unsafe ?x))])"#)
        .unwrap();
    // This rule creates a cycle: unsafe depends on not-safe (via or)
    let r = db
        .execute(r#"(rule [(unsafe ?x) [?x :item true] (or (not (safe ?x)) [?x :flagged true])])"#);
    assert!(r.is_err(), "or-with-negative-cycle must be rejected");
}

// ── Parse / safety errors ─────────────────────────────────────────────────────

/// not-join with a join variable that is not bound in the outer query fails.
#[test]
fn not_join_unbound_join_var_rejected() {
    let db = db();
    // ?x is used as join var but never bound in an outer :where pattern
    let r = db.execute(
        r#"(query [:find ?e
                   :where [?e :a ?v]
                          (not-join [?x]
                            [?e :ref ?x]
                            [?x :blocked true])])"#,
    );
    assert!(
        r.is_err(),
        "not-join with unbound join var must fail at parse"
    );
}

/// or where the two branches introduce different new variables must fail.
/// Note: or-join branch-private vars are existential and need NOT match;
/// the safety check applies to plain `or` only.
#[test]
fn or_mismatched_new_vars_rejected() {
    let db = db();
    let r = db.execute(
        r#"(query [:find ?e
                   :where [?e :type ?_t]
                          (or [?e :a ?x]
                              [?e :b ?y])])"#,
    );
    assert!(r.is_err(), "or with mismatched new vars must fail at parse");
}

/// count on a variable not present in the :where clause must fail.
#[test]
fn aggregate_var_unbound_rejected() {
    let db = db();
    let r = db.execute(r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#);
    assert!(r.is_err(), "count on unbound variable must fail at parse");
}
