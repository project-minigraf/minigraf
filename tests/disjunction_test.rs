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

// ── or-join tests ─────────────────────────────────────────────────────────────

/// (7) Basic or-join: branch-private vars stripped from output.
#[test]
fn test_or_join_strips_branch_private_vars() {
    let db = db();
    db.execute(r#"(transact [[:e1 :name "Alice"] [:e1 :tag :red]
                              [:e2 :name "Bob"]  [:e2 :badge :gold]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?_n]
                       (or-join [?e]
                         [?e :tag :red]
                         [?e :badge :gold])])"#).unwrap();
    assert_eq!(result_count(&r), 2, "both Alice (red tag) and Bob (gold badge) should match");
}

/// (8) or-join with multiple join vars.
#[test]
fn test_or_join_multiple_join_vars() {
    let db = db();
    db.execute(r#"(transact [[:e1 :dept :eng] [:e1 :level :senior]
                              [:e2 :dept :eng] [:e2 :role :lead]
                              [:e3 :dept :hr]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :dept ?dept]
                       (or-join [?e ?dept]
                         (and [?e :dept ?dept] [?e :level :senior])
                         (and [?e :dept ?dept] [?e :role :lead]))])"#).unwrap();
    assert_eq!(result_count(&r), 2, "e1 (senior) and e2 (lead) should match");
}

/// (9) or-join where branches introduce different private vars — result has only join vars.
#[test]
fn test_or_join_different_private_vars_per_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red]])"#).unwrap();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :color ?_c]
                       (or-join [?e]
                         (and [?e :color ?priv1])
                         (and [?e :color ?priv2]))])"#).unwrap();
    assert_eq!(result_count(&r), 1, "e1 should match via either branch");
}

// ── Safety / parse error tests ─────────────────────────────────────────────────

/// (10) or branches with mismatched new variables → parse error.
#[test]
fn test_or_safety_mismatched_vars_error() {
    let db = db();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or [?e :a ?x] [?e :b ?y])])"#);
    assert!(r.is_err(), "mismatched new vars should be a parse error");
    let err = r.unwrap_err().to_string();
    assert!(err.contains("same set of new variables"), "error was: {}", err);
}

/// (11) or-join with unbound join var → parse error.
#[test]
fn test_or_join_unbound_join_var_error() {
    let db = db();
    let r = db.execute(r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or-join [?x] [?x :tag :red])])"#);
    assert!(r.is_err(), "unbound join var should be a parse error");
    let err = r.unwrap_err().to_string();
    assert!(err.contains("not bound"), "error was: {}", err);
}

// ── Rule tests ─────────────────────────────────────────────────────────────────

/// (12) Rule with or in body routes to mixed_rules and produces correct results.
#[test]
fn test_rule_with_or_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tier :gold] [:e2 :tier :silver] [:e3 :tier :bronze]])"#).unwrap();
    db.execute(r#"(rule [(valuable ?e) (or [?e :tier :gold] [?e :tier :silver])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (valuable ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 2, "e1 (gold) and e2 (silver) are valuable");
}

/// (13) or-join in a rule body.
#[test]
fn test_rule_with_or_join_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red] [:e2 :color :blue] [:e3 :color :green]])"#).unwrap();
    db.execute(r#"(rule [(vivid ?e)
                         [?e :color ?_c]
                         (or-join [?e]
                           [?e :color :red]
                           [?e :color :blue])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (vivid ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 2, "e1 (red) and e2 (blue) are vivid");
}

// ── Bi-temporal tests ──────────────────────────────────────────────────────────

/// (14) or with :as-of — temporal filter applies across branches.
#[test]
fn test_or_with_as_of() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red]])"#).unwrap();
    db.execute(r#"(transact [[:e2 :tag :blue]])"#).unwrap();
    // At tx 1 only e1 (red) was present
    let r = db.execute(r#"
        (query [:find ?e
                :as-of 1
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#).unwrap();
    assert_eq!(result_count(&r), 1, "only e1 (red) was present at tx 1");
}

// ── Stratification tests ───────────────────────────────────────────────────────

/// (15) or containing not that would form a negative cycle → error at rule registration.
#[test]
fn test_or_with_not_cycle_rejected() {
    let db = db();
    // Bind ?x first so both or-branches have the same new-variable footprint (none).
    db.execute(r#"(rule [(p ?x) [?x :a true] (or (and (not (q ?x))) [?x :a true])])"#).unwrap();
    let result = db.execute(r#"(rule [(q ?x) [?x :b true] (or (and (not (p ?x))) [?x :b true])])"#);
    assert!(result.is_err(), "negative cycle through or should be rejected");
}

/// (16) or containing RuleInvocation → positive dep edge, correct stratification.
#[test]
fn test_or_with_rule_invocation_positive_dep() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true]])"#).unwrap();
    db.execute(r#"(rule [(base ?x) [?x :a true]])"#).unwrap();
    db.execute(r#"(rule [(derived ?x) (or (base ?x) [?x :b true])])"#).unwrap();
    let r = db.execute(r#"(query [:find ?e :where (derived ?e)])"#).unwrap();
    assert_eq!(result_count(&r), 1, "e1 matches via base -> derived");
}
