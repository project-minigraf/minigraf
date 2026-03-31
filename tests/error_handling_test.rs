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
    assert!(r.is_err(), "sum of mixed integer/string must fail at runtime");
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
