//! Integration tests for Phase 7.3: or / or-join disjunction.

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue] [:e3 :tag :green]])"#)
        .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#,
        )
        .unwrap();
    assert_eq!(
        result_count(&r),
        2,
        "red and blue entities must both appear"
    );
}

/// (2) Single-branch or behaves like an inline pattern filter.
#[test]
fn test_or_single_branch_acts_as_filter() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red] [:e2 :tag :blue]])"#)
        .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r), 1, "only the red entity should match");
}

/// (3) Only the first branch matches; second branch finds nothing.
#[test]
fn test_or_only_first_branch_matches() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag :red]])"#).unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r), 1, "only the red entity should match");
}

/// (4) Both branches match the same entity — result must appear exactly once.
#[test]
fn test_or_deduplication_both_branches_match() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true] [:e1 :b true]])"#)
        .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :a ?_a]
                       (or [?e :a true] [?e :b true])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r), 1, "e1 must appear exactly once");
}

/// (5) Or with not inside branch: active-and-not-banned OR vip.
#[test]
fn test_or_with_not_inside_branch() {
    let db = db();
    db.execute(
        r#"(transact [[:e1 :status :active]
                              [:e2 :status :active]
                              [:e3 :status :active]
                              [:e1 :banned true]
                              [:e3 :vip true]])"#,
    )
    .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :status :active]
                       (or (and (not [?e :banned true]))
                           [?e :vip true])])"#,
        )
        .unwrap();
    // e1: active + banned → branch 1 fails (banned), branch 2 fails (no :vip) → excluded
    // e2: active + not banned → branch 1 passes → included
    // e3: active + not banned + vip → both branches pass → deduplicated once → included
    assert_eq!(
        result_count(&r),
        2,
        "e2 (not banned) and e3 (vip) should match"
    );
}

/// (6) Nested or inside or.
#[test]
fn test_or_nested() {
    let db = db();
    db.execute(r#"(transact [[:e1 :kind :a] [:e2 :kind :b] [:e3 :kind :c]])"#)
        .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :kind ?_k]
                       (or (or [?e :kind :a] [?e :kind :b])
                           [?e :kind :c])])"#,
        )
        .unwrap();
    assert_eq!(
        result_count(&r),
        3,
        "all three entities should match via nested or"
    );
}

// ── or-join tests ─────────────────────────────────────────────────────────────

/// (7) Basic or-join: branch-private vars stripped from output.
#[test]
fn test_or_join_strips_branch_private_vars() {
    let db = db();
    db.execute(
        r#"(transact [[:e1 :name "Alice"] [:e1 :tag :red]
                              [:e2 :name "Bob"]  [:e2 :badge :gold]])"#,
    )
    .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :name ?_n]
                       (or-join [?e]
                         [?e :tag :red]
                         [?e :badge :gold])])"#,
        )
        .unwrap();
    assert_eq!(
        result_count(&r),
        2,
        "both Alice (red tag) and Bob (gold badge) should match"
    );
}

/// (8) or-join with multiple join vars.
#[test]
fn test_or_join_multiple_join_vars() {
    let db = db();
    db.execute(
        r#"(transact [[:e1 :dept :eng] [:e1 :level :senior]
                              [:e2 :dept :eng] [:e2 :role :lead]
                              [:e3 :dept :hr]])"#,
    )
    .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :dept ?dept]
                       (or-join [?e ?dept]
                         (and [?e :dept ?dept] [?e :level :senior])
                         (and [?e :dept ?dept] [?e :role :lead]))])"#,
        )
        .unwrap();
    assert_eq!(
        result_count(&r),
        2,
        "e1 (senior) and e2 (lead) should match"
    );
}

/// (9) or-join where branches introduce different private vars — result has only join vars.
#[test]
fn test_or_join_different_private_vars_per_branch() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red]])"#).unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :where [?e :color ?_c]
                       (or-join [?e]
                         (and [?e :color ?priv1])
                         (and [?e :color ?priv2]))])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r), 1, "e1 should match via either branch");
}

// ── Safety / parse error tests ─────────────────────────────────────────────────

/// (10) or branches with mismatched new variables → parse error.
#[test]
fn test_or_safety_mismatched_vars_error() {
    let db = db();
    let r = db.execute(
        r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or [?e :a ?x] [?e :b ?y])])"#,
    );
    assert!(r.is_err(), "mismatched new vars should be a parse error");
    let err = r.unwrap_err().to_string();
    assert!(
        err.contains("same set of new variables"),
        "error was: {}",
        err
    );
}

/// (11) or-join with unbound join var → parse error.
#[test]
fn test_or_join_unbound_join_var_error() {
    let db = db();
    let r = db.execute(
        r#"
        (query [:find ?e
                :where [?e :name ?n]
                       (or-join [?x] [?x :tag :red])])"#,
    );
    assert!(r.is_err(), "unbound join var should be a parse error");
    let err = r.unwrap_err().to_string();
    assert!(err.contains("not bound"), "error was: {}", err);
}

// ── Rule tests ─────────────────────────────────────────────────────────────────

/// (12) Rule with or in body routes to mixed_rules and produces correct results.
#[test]
fn test_rule_with_or_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tier :gold] [:e2 :tier :silver] [:e3 :tier :bronze]])"#)
        .unwrap();
    db.execute(r#"(rule [(valuable ?e) (or [?e :tier :gold] [?e :tier :silver])])"#)
        .unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where (valuable ?e)])"#)
        .unwrap();
    assert_eq!(
        result_count(&r),
        2,
        "e1 (gold) and e2 (silver) are valuable"
    );
}

/// (13) or-join in a rule body.
#[test]
fn test_rule_with_or_join_body() {
    let db = db();
    db.execute(r#"(transact [[:e1 :color :red] [:e2 :color :blue] [:e3 :color :green]])"#)
        .unwrap();
    db.execute(
        r#"(rule [(vivid ?e)
                         [?e :color ?_c]
                         (or-join [?e]
                           [?e :color :red]
                           [?e :color :blue])])"#,
    )
    .unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where (vivid ?e)])"#)
        .unwrap();
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
    let r = db
        .execute(
            r#"
        (query [:find ?e
                :as-of 1
                :where [?e :tag ?_t]
                       (or [?e :tag :red] [?e :tag :blue])])"#,
        )
        .unwrap();
    assert_eq!(result_count(&r), 1, "only e1 (red) was present at tx 1");
}

// ── Stratification tests ───────────────────────────────────────────────────────

/// (15) or containing not that would form a negative cycle → error at rule registration.
#[test]
fn test_or_with_not_cycle_rejected() {
    let db = db();
    // Bind ?x first so both or-branches have the same new-variable footprint (none).
    db.execute(r#"(rule [(p ?x) [?x :a true] (or (and (not (q ?x))) [?x :a true])])"#)
        .unwrap();
    let result = db.execute(r#"(rule [(q ?x) [?x :b true] (or (and (not (p ?x))) [?x :b true])])"#);
    assert!(
        result.is_err(),
        "negative cycle through or should be rejected"
    );
}

/// (16) or containing RuleInvocation → positive dep edge, correct stratification.
#[test]
fn test_or_with_rule_invocation_positive_dep() {
    let db = db();
    db.execute(r#"(transact [[:e1 :a true]])"#).unwrap();
    db.execute(r#"(rule [(base ?x) [?x :a true]])"#).unwrap();
    db.execute(r#"(rule [(derived ?x) (or (base ?x) [?x :b true])])"#)
        .unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where (derived ?e)])"#)
        .unwrap();
    assert_eq!(result_count(&r), 1, "e1 matches via base -> derived");
}

// ── Regression tests ───────────────────────────────────────────────────────────────

/// (17) Regression test for issue #82: or/or-join deduplication missed identical bindings.
///
/// The bug: HashMap::iter() returns entries in non-deterministic order. Two bindings
/// that are logically identical can produce different Vec orderings, causing
/// BTreeSet::insert to treat them as distinct and failing to deduplicate.
/// The fix: sort the key before inserting into BTreeSet (apply_or_clauses lines ~1205, ~1247).
#[test]
fn test_or_does_not_return_duplicate_bindings() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag-a true] [:e1 :tag-b true]])"#)
        .unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where (or [?e :tag-a true] [?e :tag-b true])])"#)
        .unwrap();
    assert_eq!(
        result_count(&r),
        1,
        "entity matching both branches must appear exactly once"
    );
}

/// (18) or-join also deduplicates correctly.
#[test]
fn test_or_join_does_not_return_duplicate_bindings() {
    let db = db();
    db.execute(r#"(transact [[:e1 :tag-a true] [:e1 :tag-b true]])"#)
        .unwrap();
    let r = db
        .execute(
            r#"(query [:find ?e
                    :where [?e :tag-a ?_a]
                           (or-join [?e] [?e :tag-a true] [?e :tag-b true])])"#,
        )
        .unwrap();
    assert_eq!(
        result_count(&r),
        1,
        "or-join entity matching both branches must appear exactly once"
    );
}

// ── Issue #250: short-circuit or/or-join branch evaluation ─────────────────

/// (19) or-join short-circuit is always safe (branches are projected to outer_keys
/// before merging, so a branch can't smuggle in a new distinguishing variable):
/// once the cheap branch covers every incoming join-var key, the expensive branch
/// (guarded by a call-counting predicate) must never run.
#[test]
fn test_or_join_short_circuits_expensive_branch_when_fully_covered() {
    let db = db();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_inner = calls.clone();
    db.register_predicate("expensive?", move |_v: &Value| {
        calls_inner.fetch_add(1, Ordering::SeqCst);
        true
    })
    .unwrap();

    // e1 also has :flag true — if the expensive branch actually ran (i.e. the
    // short-circuit didn't fire), it would match e1 and invoke the counter, even
    // though e1 is already covered by the cheap branch. Without this overlap, the
    // expensive branch's own pattern would match nothing regardless of whether it
    // runs, and the `calls == 0` assertion below would pass vacuously.
    db.execute(
        r#"(transact [[:e1 :cheap true] [:e1 :flag true] [:e2 :cheap true] [:e3 :cheap true]])"#,
    )
    .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?e
                    :where [?e ?_a ?_v]
                           (or-join [?e]
                             [?e :cheap true]
                             (and [?e :flag true] [(expensive? ?e)]))])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&r),
        3,
        "all three entities match via the cheap branch alone"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "expensive branch must be skipped once the cheap branch covers every group"
    );
}

/// (20) or-join correctness when the cheap branch covers only some incoming keys:
/// the fallback branch must still run for the uncovered ones, and results must
/// include matches contributed by it.
#[test]
fn test_or_join_falls_back_to_expensive_branch_when_not_fully_covered() {
    let db = db();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_inner = calls.clone();
    db.register_predicate("expensive?", move |_v: &Value| {
        calls_inner.fetch_add(1, Ordering::SeqCst);
        true
    })
    .unwrap();

    db.execute(r#"(transact [[:e1 :cheap true] [:e2 :flag true]])"#)
        .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?e
                    :where [?e ?_a ?_v]
                           (or-join [?e]
                             [?e :cheap true]
                             (and [?e :flag true] [(expensive? ?e)]))])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&r),
        2,
        "e1 via the cheap branch, e2 via the fallback branch"
    );
    assert!(
        calls.load(Ordering::SeqCst) >= 1,
        "fallback branch must run for the group the cheap branch didn't cover"
    );
}

/// (20b) Proves branch reordering itself (not declaration order) drives the skip:
/// the expensive branch is declared FIRST, but has a genuinely higher `branch_cost`
/// (a 1-bound pattern, cost 100) than the cheap branch (which pairs a 2-bound
/// pattern with an unrelated fully-bound one, so `branch_cost`'s min-over-patterns
/// is 1). If `branch_cost`-based sorting were broken (e.g. reverted to declaration
/// order), the expensive branch — written first — would run before coverage is
/// established and invoke the counter.
#[test]
fn test_or_join_short_circuit_relies_on_cost_sort_not_declaration_order() {
    let db = db();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_inner = calls.clone();
    db.register_predicate("expensive?", move |_v: &Value| {
        calls_inner.fetch_add(1, Ordering::SeqCst);
        true
    })
    .unwrap();

    // :tracked seeds exactly {e1, e2, e3} as the incoming set — deliberately not
    // `[?e ?_a ?_v]`, which would also pull in :marker-ent (it has its own fact)
    // and produce two incoming rows for e1 (one per fact), muddying the coverage
    // check this test is actually after.
    db.execute(
        r#"(transact [
            [:e1 :tracked true] [:e1 :cheap true] [:e1 :flag true]
            [:e2 :tracked true] [:e2 :cheap true]
            [:e3 :tracked true] [:e3 :cheap true]
            [:marker-ent :m true]
        ])"#,
    )
    .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?e
                    :where [?e :tracked true]
                           (or-join [?e]
                             (and [?e ?attr true] [(expensive? ?e)])
                             (and [?e :cheap true] [:marker-ent :m true]))])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&r),
        3,
        "all three entities match via the cost-cheaper branch alone"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the cost-100 branch must be sorted after the cost-1 branch and then skipped, \
         even though it appears first in the query text"
    );
}

/// (21) Plain `or` short-circuits too, when it's a pure filter (no branch introduces
/// a variable beyond what's already bound in the incoming scope).
#[test]
fn test_or_short_circuits_pure_filter_when_fully_covered() {
    let db = db();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_inner = calls.clone();
    db.register_predicate("expensive?", move |_v: &Value| {
        calls_inner.fetch_add(1, Ordering::SeqCst);
        true
    })
    .unwrap();

    db.execute(r#"(transact [[:e1 :status :active] [:e2 :status :active] [:e3 :flag true]])"#)
        .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?e
                    :where [?e :status ?_s]
                           (or [?e :status :active]
                               (and [?e :flag true] [(expensive? ?e)]))])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&r),
        2,
        "e1 and e2 both match via the cheap branch alone"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "expensive branch must be skipped once the cheap branch covers every incoming entity"
    );
}

/// (22) Correctness guard: a plain `or` whose branches introduce a *new* variable
/// must never short-circuit, even if every incoming key is already "covered" by the
/// cheaper branch — a later branch can legitimately bind that new variable to a
/// different value for the same shared key, and dropping it would silently lose a
/// valid result row. This is the scenario that makes `or` (unlike `or-join`) unsafe
/// to skip on coverage alone.
#[test]
fn test_or_never_skips_branches_that_introduce_new_variables() {
    let db = db();
    db.execute(r#"(transact [[:e1 :id true] [:e1 :typeA :va] [:e1 :typeB :vb]])"#)
        .unwrap();

    let r = db
        .execute(
            r#"(query [:find ?e ?v
                    :where [?e :id true]
                           (or [?e :typeA ?v] [?e :typeB ?v])])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&r),
        2,
        "both branches must contribute a row for ?e=:e1, since ?v differs between them"
    );
}

// ── #405: or-join with zero incoming rows ──────────────────────────────────────

const OR_JOIN_AFTER_PRODUCT: &str = r#"
    (query [:find ?item
            :where [?item :order-item/product ?p]
                   (or-join [?item]
                     [?item :order-item/order :a]
                     [?item :order-item/order :b])])"#;

/// #405: or-join on an empty database returns an empty result, not INT-031.
#[test]
fn test_or_join_empty_database_returns_empty() {
    let db = db();
    let r = db
        .execute(OR_JOIN_AFTER_PRODUCT)
        .expect("or-join on empty db must not error");
    assert_eq!(result_count(&r), 0, "empty db has no results");
}

/// #405: or-join after a pattern that matches nothing returns an empty result.
#[test]
fn test_or_join_after_unmatched_pattern_returns_empty() {
    let db = db();
    db.execute(r#"(transact [[:i1 :order-item/order :a]])"#)
        .unwrap();
    let r = db
        .execute(OR_JOIN_AFTER_PRODUCT)
        .expect("or-join after unmatched pattern must not error");
    assert_eq!(result_count(&r), 0, "no :order-item/product facts");

    db.execute(r#"(transact [[:i1 :order-item/product :p1]])"#)
        .unwrap();
    let r = db.execute(OR_JOIN_AFTER_PRODUCT).unwrap();
    assert_eq!(result_count(&r), 1, "i1 now matches");
}

/// #405: or-join after a `not` that removes every row returns an empty result.
#[test]
fn test_or_join_after_not_removing_all_rows_returns_empty() {
    let db = db();
    db.execute(
        r#"(transact [[:i1 :order-item/product :p1] [:i1 :order-item/order :a]
                      [:i1 :order-item/cancelled true]])"#,
    )
    .unwrap();
    let r = db
        .execute(
            r#"
        (query [:find ?item
                :where [?item :order-item/product ?p]
                       (not [?item :order-item/cancelled true])
                       (or-join [?item]
                         [?item :order-item/order :a]
                         [?item :order-item/order :b])])"#,
        )
        .expect("or-join after all-removing not must not error");
    assert_eq!(result_count(&r), 0, "the only item is cancelled");
}

/// #405: or-join in a rule body whose preceding pattern matches nothing.
#[test]
fn test_rule_with_or_join_body_no_matches_returns_empty() {
    let db = db();
    db.execute(
        r#"(rule [(vivid ?e)
                         [?e :color ?_c]
                         (or-join [?e]
                           [?e :color :red]
                           [?e :color :blue])])"#,
    )
    .unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where (vivid ?e)])"#)
        .expect("rule with or-join on empty db must not error");
    assert_eq!(result_count(&r), 0, "empty db has no vivid entities");

    db.execute(r#"(transact [[:e1 :shape :round]])"#).unwrap();
    let r = db
        .execute(r#"(query [:find ?e :where [?e :shape ?_s] (vivid ?e)])"#)
        .expect("rule with or-join and no colored entities must not error");
    assert_eq!(result_count(&r), 0, "no colored entities");
}
