//! Integration tests for Phase 7.3: or / or-join disjunction.

use minigraf::{Minigraf, OpenOptions, QueryResult};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

fn result_count(r: &QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    }
}

/// (1) Two-branch or: entities with :tag :red OR :tag :blue both appear.
#[test]
fn test_or_union_two_branches() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue] [:e3 :tag :green]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 2, "red and blue entities must both appear");
}

/// (2) Single-branch or behaves like an inline pattern filter.
#[test]
fn test_or_single_branch_acts_as_filter() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "only the red entity should match");
}

/// (3) Only the first branch matches; second branch finds nothing.
#[test]
fn test_or_only_first_branch_matches() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "only the red entity should match");
}

/// (4) Both branches match the same entity — result must appear exactly once.
#[test]
fn test_or_deduplication_both_branches_match() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true] [:e1 :b true]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :a ?_a]
                       (or [?e :a true] [?e :b true])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "e1 must appear exactly once");
}

/// (5) Or with not inside branch: active-and-not-banned OR vip.
#[test]
fn test_or_with_not_inside_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :status :active]
                              [:e2 :status :active]
                              [:e3 :status :active]
                              [:e1 :banned true]
                              [:e3 :vip true]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :status :active]
                       (or (and (not [?e :banned true]))
                           [?e :vip true])])"#).unwrap();
    // e1: active + banned → branch 1 fails (banned), branch 2 fails (no :vip) → excluded
    // e2: active + not banned → branch 1 passes → included
    // e3: active + not banned + vip → both branches pass → deduplicated once → included
    assert_eq!(result_count(&r), 2, "e2 (not banned) and e3 (vip) should match");
}

/// (6) Nested or inside or.
#[test]
fn test_or_nested() {
    let db = db();
    db.execute(r#"(transact [[:e1 :kind :a] [:e2 :kind :b] [:e3 :kind :c]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :kind ?_k]
                       (or (or [?e :kind :a] [?e :kind :b])
                           [?e :kind :c])])"#).unwrap();
    assert_eq!(result_count(&r), 3, "all three entities should match via nested or");
}
