//! Integration-level error-path tests for Phase 7.5.
//! Drives the full Minigraf::execute() API with invalid programs/data and
//! asserts that errors propagate correctly to the caller.

use minigraf::{Minigraf, OpenOptions};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

// ── Runtime type errors in aggregation ───────────────────────────────────────

/// sum over a string attribute fails at query execution time.
#[test]
fn sum_string_attribute_error() {
    let db = db();
    db.execute(r#"(transact [[:a :score "high"] [:b :score "low"]])"#)
        .unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#);
    assert!(r.is_err(), "sum of strings must fail at runtime");
}

/// sum fails when an attribute has mixed integer and string values.
#[test]
fn sum_mixed_int_string_error() {
    let db = db();
    // :score has both integer and string values across entities
    db.execute(r#"(transact [[:a :score 10] [:b :score "twenty"]])"#)
        .unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#);
    assert!(
        r.is_err(),
        "sum of mixed integer/string must fail at runtime"
    );
}

/// max over a boolean attribute fails at query execution time.
/// (min on boolean is already tested in aggregation_test.rs; this covers max.)
#[test]
fn max_boolean_attribute_error() {
    let db = db();
    db.execute(r#"(transact [[:a :flag true] [:b :flag false]])"#)
        .unwrap();
    let r = db.execute(r#"(query [:find (max ?f) :where [?e :flag ?f]])"#);
    assert!(r.is_err(), "max of booleans must fail at runtime");
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
