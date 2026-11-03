#[path = "../fuzz_targets/datalog_eval/guard.rs"]
mod guard;

use guard::{join_rows_bound, MAX_JOIN_ROWS};

#[test]
fn rejects_issue_802_artifacts() {
    for input in [
        include_str!("fixtures/802-not-join-timeout.edn"),
        include_str!("fixtures/802-not-join-slow.edn"),
        include_str!("fixtures/802-not-join-slow-2.edn"),
        include_str!("fixtures/802-not-join-slow-3.edn"),
    ] {
        assert!(join_rows_bound(input) > MAX_JOIN_ROWS);
    }
}

#[test]
fn negation_local_variables_do_not_inherit_outer_constraints() {
    // The outer pattern fixes ?v, but only ?e is passed into not-join.
    // Its body still builds 2^15 rows for Alice before the final empty pattern.
    let mut input = String::from("(query [:find ?e :where [?e :age ?v] (not-join [?e] ");
    for i in 0..15 {
        input.push_str(&format!("[?e ?a{i} ?v{i}] "));
    }
    input.push_str("[?e :missing ?x])])");
    // Reuse the outer variable's name for the first inner value.
    let input = input.replace("?v0", "?v");
    assert!(join_rows_bound(&input) > MAX_JOIN_ROWS);
}

#[test]
fn keeps_long_selective_joins() {
    let input = format!(
        "(query [:find ?e :where {}])",
        "[?e :name ?name] ".repeat(80)
    );
    assert!(join_rows_bound(&input) <= MAX_JOIN_ROWS);
}

#[test]
fn rejects_disconnected_cross_products() {
    let input = "[_ _ _] ".repeat(11);
    assert!(join_rows_bound(&input) > MAX_JOIN_ROWS);
}

#[test]
fn parentheses_inside_strings_do_not_create_scopes() {
    assert_eq!(join_rows_bound(r#"[?e :name "(Alice)"]"#), 1);
}

#[test]
fn join_variable_lists_are_not_fact_patterns() {
    // A three-variable declaration is not an EAV constraint (?e = ?v).
    let input = format!(
        "(query [:find ?e :where [?e :age ?v] (not-join [?e ?v ?e] {}[?e :missing ?x])])",
        (0..16)
            .map(|i| format!("[?e ?a{i} ?v{i}] "))
            .collect::<String>()
    );
    assert!(join_rows_bound(&input) > MAX_JOIN_ROWS);
}

#[test]
fn returns_to_outer_scope_after_negation() {
    assert_eq!(
        join_rows_bound("(query [:find ?e :where [?e ?a ?v] (not [?e :missing ?x]) [?e ?a ?v]])"),
        6
    );
}

#[test]
fn sibling_branches_have_separate_scopes() {
    assert_eq!(
        join_rows_bound("(or-join [?e ?a ?v] (and [?e :age ?v]) (and [?e :name ?v]))"),
        2
    );
}
