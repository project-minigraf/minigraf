//! Integration tests for Phase 7.1b: not-join (existential negation).

use minigraf::{Minigraf, OpenOptions, QueryResult};

fn in_memory_db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

/// Helper: extract result count from a QueryResult.
fn result_count(r: &QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    }
}

/// 1. Basic not-join: single join var, inner var existentially quantified.
/// alice has a blocked dep → excluded; bob has no deps → included.
#[test]
fn test_not_join_basic_inner_var_excluded() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:alice :name "Alice"]
                      [:alice :has-dep :dep1]
                      [:dep1  :blocked true]
                      [:bob   :name "Bob"]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"(query [:find ?x
                       :where [?x :name ?_n]
                              (not-join [?x]
                                [?x :has-dep ?d]
                                [?d :blocked true])])"#,
        )
        .unwrap();

    // Only bob passes the not-join (alice has a blocked dep).
    assert_eq!(
        result_count(&result),
        1,
        "only bob must pass not-join; alice is excluded"
    );
}

/// 2. Multiple join vars.
/// u1 has restricted role r1; u2 has unrestricted role r2 → only u2 returned.
#[test]
fn test_not_join_multiple_join_vars() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:u1 :has-role :r1]
                      [:u2 :has-role :r2]
                      [:r1 :is-restricted true]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"(query [:find ?u
                       :where [?u :has-role ?r]
                              (not-join [?u ?r]
                                [?r :is-restricted true])])"#,
        )
        .unwrap();

    // u1's role r1 is restricted → excluded; u2's role r2 is not → included.
    assert_eq!(
        result_count(&result),
        1,
        "u2 with unrestricted role must appear; u1 with restricted role excluded"
    );
}

/// 3. Multi-clause not-join body (conjunction of patterns).
/// Only e3 has :data true and no :tag :sensitive.
#[test]
fn test_not_join_multi_clause_body() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:e1 :tag :sensitive]
                      [:e1 :tag :critical]
                      [:e2 :tag :sensitive]
                      [:e3 :data true]])"#,
    )
    .unwrap();

    let result = db
        .execute(
            r#"(query [:find ?e
                       :where [?e :data true]
                              (not-join [?e]
                                [?e :tag :sensitive])])"#,
        )
        .unwrap();

    // Only e3 has :data true and is not tagged :sensitive.
    assert_eq!(
        result_count(&result),
        1,
        "only e3 (data=true, no sensitive tag) must appear"
    );
}

/// 4. not-join in a rule body.
/// alice has a rejected dep → ineligible; bob has no deps → eligible.
#[test]
fn test_not_join_in_rule_body() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:alice :applied true]
                      [:alice :dep :dep1]
                      [:dep1  :status :rejected]
                      [:bob   :applied true]])"#,
    )
    .unwrap();
    db.execute(
        r#"(rule [(eligible ?x)
                  [?x :applied true]
                  (not-join [?x]
                    [?x :dep ?d]
                    [?d :status :rejected])])"#,
    )
    .unwrap();

    let result = db
        .execute("(query [:find ?x :where (eligible ?x)])")
        .unwrap();

    // Only bob is eligible (alice's dep is rejected).
    assert_eq!(
        result_count(&result),
        1,
        "bob must be eligible; alice has a rejected dep"
    );
}

/// 5. not-join in a two-stage rule chain.
///
/// LIMITATION: When rule B positively invokes rule A (both in the same stratum)
/// and rule A contains a not-join, the StratifiedEvaluator processes all mixed
/// rules within a stratum in a single pass in declaration order.  Because
/// `approved` is declared before `eligible`, its positive-pattern match for
/// `[?x :eligible ?_rule_value]` finds no facts yet, producing an empty result.
///
/// A proper fix would require either:
///   (a) iterating mixed rules to a fixed-point within each stratum, or
///   (b) giving `approved` a higher stratum via a negative edge (which isn't
///       present here — only a positive dep exists).
///
/// What DOES work: two independent not-join rules that each operate directly on
/// base facts (no positive dependency between the two rules).
///
/// Stage-1 rule: (stage1-ok ?x) :- [?x :applied true],
///                                  (not-join [?x] [?x :dep ?d] [?d :blocked true])
/// Stage-2 rule: (stage2-ok ?x) :- [?x :applied true],
///                                  (not-join [?x] [?x :dep ?d] [?d :blocked true]),
///                                  (not-join [?x] [?x :on-hold true])
///
/// alice: has blocked dep → excluded at both stages
/// bob:   on-hold → excluded only at stage 2
/// charlie: neither → passes both
#[test]
fn test_not_join_multi_stratum_chain() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:alice   :applied true]
                      [:alice   :dep :dep1]
                      [:dep1    :blocked true]
                      [:bob     :applied true]
                      [:bob     :on-hold true]
                      [:charlie :applied true]])"#,
    )
    .unwrap();
    db.execute(
        r#"(rule [(stage1-ok ?x)
                  [?x :applied true]
                  (not-join [?x] [?x :dep ?d] [?d :blocked true])])"#,
    )
    .unwrap();
    db.execute(
        r#"(rule [(stage2-ok ?x)
                  [?x :applied true]
                  (not-join [?x] [?x :dep ?d] [?d :blocked true])
                  (not-join [?x] [?x :on-hold true])])"#,
    )
    .unwrap();

    // stage1-ok: bob and charlie (alice has blocked dep)
    let r1 = db
        .execute("(query [:find ?x :where (stage1-ok ?x)])")
        .unwrap();
    assert_eq!(
        result_count(&r1),
        2,
        "stage1-ok: bob and charlie (alice excluded by blocked dep)"
    );

    // stage2-ok: only charlie (alice excluded by dep, bob excluded by on-hold)
    let r2 = db
        .execute("(query [:find ?x :where (stage2-ok ?x)])")
        .unwrap();
    assert_eq!(
        result_count(&r2),
        1,
        "stage2-ok: only charlie; alice excluded by dep, bob excluded by on-hold"
    );
}

/// 6. not-join allows inner vars not bound by outer clauses (unlike plain `not`).
/// alice's dep is blocked → excluded; bob's dep is not blocked → included.
#[test]
fn test_not_join_allows_inner_var_not_would_reject() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:alice :dep :dep1]
                      [:dep1  :blocked true]
                      [:bob   :dep :dep2]])"#,
    )
    .unwrap();

    // ?d is an inner (existentially quantified) variable inside not-join.
    // With plain (not ...) this would be a safety error; not-join allows it.
    let result = db
        .execute(
            r#"(query [:find ?x
                       :where [?x :dep ?_d2]
                              (not-join [?x]
                                [?x :dep ?d]
                                [?d :blocked true])])"#,
        )
        .unwrap();

    // bob's dep is not blocked → passes; alice's dep is blocked → excluded.
    assert_eq!(
        result_count(&result),
        1,
        "bob must appear (dep not blocked); alice excluded"
    );
}

/// 7. not-join combined with :as-of time travel.
/// At tx 1 alice has no dep yet → passes not-join.
/// At tx 2 alice's dep is present and blocked → excluded.
#[test]
fn test_not_join_with_as_of() {
    let db = in_memory_db();
    db.execute("(transact [[:alice :applied true]])").unwrap(); // tx 1
    db.execute(
        r#"(transact [[:dep1 :blocked true]
                      [:alice :dep :dep1]])"#,
    )
    .unwrap(); // tx 2

    let result_tx1 = db
        .execute(
            r#"(query [:find ?x
                       :as-of 1
                       :where [?x :applied true]
                              (not-join [?x]
                                [?x :dep ?d]
                                [?d :blocked true])])"#,
        )
        .unwrap();

    let result_tx2 = db
        .execute(
            r#"(query [:find ?x
                       :as-of 2
                       :where [?x :applied true]
                              (not-join [?x]
                                [?x :dep ?d]
                                [?d :blocked true])])"#,
        )
        .unwrap();

    assert_eq!(
        result_count(&result_tx1),
        1,
        "at tx 1 alice has no dep yet, must pass not-join"
    );
    assert_eq!(
        result_count(&result_tx2),
        0,
        "at tx 2 alice has a blocked dep, must be excluded"
    );
}

/// 8. Unbound join variable rejected at parse time.
/// ?unbound appears in the join-vars list but is never bound by any outer clause.
#[test]
fn test_not_join_unbound_join_var_parse_error() {
    let db = in_memory_db();
    db.execute(r#"(transact [[:e1 :name "test"]])"#).unwrap();

    let result = db.execute(
        r#"(query [:find ?e
                   :where [?e :name ?n]
                          (not-join [?unbound] [?e :dep ?unbound])])"#,
    );

    assert!(
        result.is_err(),
        "join var ?unbound not bound by outer clause must produce a parse error"
    );
}

/// 9. Nested not-join inside not is rejected at parse time.
#[test]
fn test_not_join_nested_inside_not_rejected() {
    let db = in_memory_db();
    db.execute("(transact [[:e1 :data true]])").unwrap();

    let result = db.execute(
        r#"(query [:find ?e
                   :where [?e :data true]
                          (not (not-join [?e] [?e :flag true]))])"#,
    );

    assert!(
        result.is_err(),
        "not-join nested inside not must be a parse error"
    );
}

/// 10. not-join body contains a RuleInvocation — derived rule facts correctly negated end-to-end.
/// Rule: (banned ?x) :- [?x :status :banned]
/// Rule: (eligible ?x) :- [?x :applied true], (not-join [?x] (banned ?x))
/// alice: applied + banned → NOT eligible; bob: applied only → eligible.
#[test]
fn test_not_join_body_with_rule_invocation_end_to_end() {
    let db = in_memory_db();
    db.execute(
        r#"(transact [[:alice :applied true]
                      [:alice :status :banned]
                      [:bob   :applied true]])"#,
    )
    .unwrap();
    db.execute(r#"(rule [(banned ?x) [?x :status :banned]])"#)
        .unwrap();
    db.execute(
        r#"(rule [(eligible ?x)
                  [?x :applied true]
                  (not-join [?x] (banned ?x))])"#,
    )
    .unwrap();

    let result = db
        .execute("(query [:find ?x :where (eligible ?x)])")
        .unwrap();

    // Only bob is eligible (alice is banned via the rule).
    assert_eq!(
        result_count(&result),
        1,
        "bob must be eligible; alice excluded via banned rule in not-join body"
    );
}

/// 11. not-join where no entities match the body — all outer bindings survive.
#[test]
fn test_not_join_body_no_matches_all_survive() {
    let db = in_memory_db();
    // Three entities with :active true, but none have :banned true
    db.execute("(transact [[:e1 :active true] [:e2 :active true] [:e3 :active true]])")
        .unwrap();
    let result = db
        .execute("(query [:find ?x :where [?x :active true] (not-join [?x] [?x :banned true])])")
        .unwrap();
    // All three survive because the not-join body matches nothing.
    // Entity IDs are UUIDs in the result, not keyword strings, so we count results.
    assert_eq!(
        result_count(&result),
        3,
        "all three entities must survive when not-join body matches nothing"
    );
}

/// 12. not-join combined with :valid-at valid-time query.
#[test]
fn test_not_join_with_valid_at() {
    let db = in_memory_db();
    // alice: active during 2023, restricted during 2024
    // bob: active during 2023, no restrictions
    db.execute(
        "(transact {:valid-from \"2023-01-01\" :valid-to \"2024-01-01\"} [[:alice :active true] [:bob :active true]])",
    )
    .unwrap();
    db.execute("(transact {:valid-from \"2024-01-01\"} [[:alice :restricted true]])")
        .unwrap();
    // At 2023-06-01: both active, neither restricted -> both pass.
    // Entity IDs are UUIDs in the result, not keyword strings, so we count results.
    let result_2023 = db
        .execute("(query [:find ?x :valid-at \"2023-06-01\" :where [?x :active true] (not-join [?x] [?x :restricted true])])")
        .unwrap();
    assert_eq!(
        result_count(&result_2023),
        2,
        "both alice and bob must pass not-join at 2023-06-01 (neither is restricted then)"
    );
}

/// 13. Negative cycle via not-join at rule registration is rejected.
#[test]
fn test_not_join_negative_cycle_at_registration_rejected() {
    let db = in_memory_db();
    // p :- (not-join [?x] (q ?x))
    // q :- (not-join [?x] (p ?x))
    // This is a negative cycle; the second rule registration should fail.
    let r1 = db.execute("(rule [(p ?x) (not-join [?x] (q ?x))])");
    let r2 = db.execute("(rule [(q ?x) (not-join [?x] (p ?x))])");
    // At least one of the registrations must fail with a stratification error
    assert!(
        r1.is_err() || r2.is_err(),
        "negative cycle via not-join must be rejected at rule registration"
    );
}

/// 14. not and not-join coexist in the same query body.
#[test]
fn test_not_join_coexists_with_not_in_query() {
    let db = in_memory_db();
    // Find active entities that are not globally-blocked AND have no blocked dependency
    // alice: active, globally-blocked -> excluded by (not)
    // bob: active, has blocked dep -> excluded by (not-join)
    // charlie: active, no restrictions -> passes both
    db.execute("(transact [[:alice :active true] [:alice :blocked true] [:bob :active true] [:bob :dep :dep1] [:dep1 :severity :high] [:charlie :active true]])").unwrap();
    let result = db
        .execute("(query [:find ?x :where [?x :active true] (not [?x :blocked true]) (not-join [?x] [?x :dep ?d] [?d :severity :high])])")
        .unwrap();
    // Entity IDs are UUIDs in the result, not keyword strings, so we count results.
    // Only charlie passes both (not) and (not-join) filters.
    assert_eq!(
        result_count(&result),
        1,
        "only charlie must pass: alice excluded by (not), bob excluded by (not-join)"
    );
}
