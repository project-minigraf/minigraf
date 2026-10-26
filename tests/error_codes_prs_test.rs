//! Regression tests for issue #358 (#277 2/6): PRS-0xx parser error codes.
//!
//! `src/query/datalog/parser.rs` now carries a real, documented `PRS-0xx`
//! code (see `docs/ERROR_REFERENCE.md`) on every reachable parse-error site,
//! via `bail_coded!`/`err_coded!`, instead of the generic `INT-000` fallback
//! from the #277 foundation PR (#357).
//!
//! Coverage: every PRS code that is actually reachable through the public
//! API is exercised here. Two documented codes — PRS-028 ("window
//! expression cannot be empty") and PRS-049 ("unexpected end of fact
//! vector") — are guarded by conditions that can never be false given how
//! their call sites are reached (the surrounding `has_over`/`len() >= 4`
//! checks already guarantee the "bad" case can't occur), so no input
//! reaches them; they are intentionally not tested here. That's a
//! pre-existing property of the parser logic, not something this PR
//! changed.

use minigraf::{ErrorCategory, Minigraf};

fn db() -> Minigraf {
    Minigraf::in_memory().unwrap()
}

/// Execute `input` and assert the resulting `MinigrafError` carries
/// `expected_code` in the `Parser` category.
fn assert_execute_code(input: &str, expected_code: &str) {
    let db = db();
    let result = db.execute(input);
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!(
            "expected {} for input, but execute() succeeded: {}",
            expected_code, input
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Parser,
        "expected Parser category for {}",
        expected_code
    );
    assert_eq!(err.code(), expected_code, "wrong code for input: {}", input);
}

#[test]
fn prs_codes_cover_tokenizer_and_edn_errors() {
    let cases: &[(&str, &str)] = &[
        ("", "PRS-001"),
        ("@", "PRS-002"),
        (")", "PRS-003"),
        ("[1", "PRS-004"),
        ("(this is not valid datalog", "PRS-005"),
        ("{:a 1", "PRS-006"),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_007_string_exceeds_maximum_length() {
    let long_string = "\"".to_string() + &"x".repeat(1_048_577) + "\"";
    assert_execute_code(&long_string, "PRS-007");
}

#[test]
fn prs_008_keyword_exceeds_maximum_length() {
    let long_keyword = ":".to_string() + &"x".repeat(1025);
    assert_execute_code(&long_keyword, "PRS-008");
}

#[test]
fn prs_009_tagged_literal_exceeds_maximum_length() {
    let long_tag = "#".to_string() + &"x".repeat(1025);
    assert_execute_code(&long_tag, "PRS-009");
}

#[test]
fn prs_074_bind_slot_name_exceeds_maximum_length() {
    let long_slot = "$".to_string() + &"x".repeat(1025);
    assert_execute_code(&long_slot, "PRS-074");
}

#[test]
fn prs_codes_cover_command_dispatch() {
    let cases: &[(&str, &str)] = &[
        ("(1 2 3)", "PRS-010"),
        ("(bogus [1])", "PRS-011"),
        ("42", "PRS-012"),
        ("(query)", "PRS-013"),
        ("(transact)", "PRS-041"),
        ("(retract)", "PRS-043"),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_as_of_and_valid_at() {
    let cases: &[(&str, &str)] = &[
        ("(query [:find ?e :where [?e :name ?n] :as-of])", "PRS-014"),
        (
            "(query [:find ?e :where [?e :name ?n] :as-of -1])",
            "PRS-015",
        ),
        (
            "(query [:find ?e :where [?e :name ?n] :as-of :now])",
            "PRS-016",
        ),
        (
            "(query [:find ?e :where [?e :name ?n] :valid-at])",
            "PRS-017",
        ),
        (
            "(query [:find ?e :where [?e :name ?n] :valid-at 42])",
            "PRS-018",
        ),
        (
            r#"(transact {:valid-from 42} [[:alice :name "Alice"]])"#,
            "PRS-019",
        ),
        (
            r#"(transact {:valid-to 42} [[:alice :name "Alice"]])"#,
            "PRS-020",
        ),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_with_and_aggregates() {
    let cases: &[(&str, &str)] = &[
        (
            r#"(query [:find ?e ?name :with ?order :where [?e :name ?name] [?e :order ?order]])"#,
            "PRS-021",
        ),
        (
            r#"(query [:find (count ?e) :with ?unbound :where [?e :name ?n]])"#,
            "PRS-022",
        ),
        (
            r#"(query [:find (sum ?amount) :where [?e :name ?n]])"#,
            "PRS-023",
        ),
        (
            r#"(query [:find (sum ?amount :distinct) :where [?e :a ?amount]])"#,
            "PRS-024",
        ),
        (
            r#"(query [:find (:sum ?amount) :where [?e :a ?amount]])"#,
            "PRS-025",
        ),
        (r#"(query [:find (sum 42) :where [?e :a ?v]])"#, "PRS-026"),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_window_functions() {
    let cases: &[(&str, &str)] = &[
        (
            r#"(query [:find (rank ?score) :where [?e :score ?score]])"#,
            "PRS-027",
        ),
        (
            r#"(query [:find (:rank :over (:order-by ?score)) :where [?e :score ?score]])"#,
            "PRS-029",
        ),
        (
            r#"(query [:find (lag :over (:order-by ?v)) :where [?e :x ?v]])"#,
            "PRS-032",
        ),
        (
            r#"(query [:find (lead ?v ?extra :over (:order-by ?v)) :where [?e :x ?v]])"#,
            "PRS-033",
        ),
        (
            r#"(query [:find (lag ?v :over (:partition-by ?v)) :where [?e :x ?v]])"#,
            "PRS-084",
        ),
        (
            r#"(query [:find (count-distinct ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
            "PRS-031",
        ),
        (
            r#"(query [:find (ntile :over (:order-by ?score)) :where [?e :score ?score]])"#,
            "PRS-032",
        ),
        (
            r#"(query [:find (ntile ?bucket ?extra :over (:order-by ?score)) :where [?e :score ?score]])"#,
            "PRS-033",
        ),
        (
            r#"(query [:find (rank ?score :over (:order-by ?score)) :where [?e :score ?score]])"#,
            "PRS-034",
        ),
        (
            r#"(query [:find (rank :over :order-by) :where [?e :score ?score]])"#,
            "PRS-035",
        ),
        (
            r#"(query [:find (rank :over (:order-by ?score) :extra) :where [?e :score ?score]])"#,
            "PRS-036",
        ),
        (
            r#"(query [:find (rank :over (:partition-by :dept :order-by ?score)) :where [?e :dept ?d] [?e :score ?score]])"#,
            "PRS-037",
        ),
        (
            r#"(query [:find (rank :over (:order-by "score")) :where [?e :score ?score]])"#,
            "PRS-038",
        ),
        (
            r#"(query [:find (rank :over (:order-by ?score :ascending)) :where [?e :score ?score]])"#,
            "PRS-039",
        ),
        (
            r#"(query [:find (rank :over (:order-by ?score 5)) :where [?e :score ?score]])"#,
            "PRS-040",
        ),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_transact_and_retract_shape() {
    let cases: &[(&str, &str)] = &[
        ("(transact :bogus)", "PRS-042"),
        ("(retract :bogus)", "PRS-044"),
        ("(transact [:not-a-vector])", "PRS-045"),
        (r#"(transact [[:alice :name]])"#, "PRS-046"),
        (r#"(transact [[:alice :name "Alice" "bogus"]])"#, "PRS-047"),
        (
            r#"(transact {:valid-from "2023-01-01T00:00:00Z"})"#,
            "PRS-048",
        ),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_where_clause_structure() {
    let cases: &[(&str, &str)] = &[
        (r#"(query [:find ?x :where [?x :a ?v] ()])"#, "PRS-050"),
        (
            r#"(query [:find ?x :where [?x :a ?v] (not (not [?x :b true]))])"#,
            "PRS-051",
        ),
        (r#"(query [:find ?x :where [?x :a ?v] (not)])"#, "PRS-052"),
        (r#"(query [:find ?x :where [?x :a ?v] (or)])"#, "PRS-053"),
        (
            r#"(query [:find ?x :where [?x :a ?v] (or-join)])"#,
            "PRS-054",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] (or-join ?x [?x :b true])])"#,
            "PRS-055",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] (or-join [:x] [?x :b true])])"#,
            "PRS-056",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] (or [?x :b ?y] [?x :c ?z])])"#,
            "PRS-057",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] (or (and) [?x :b true])])"#,
            "PRS-058",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] (not-join)])"#,
            "PRS-059",
        ),
        (r#"(query [:find ?x :where [?x :a ?v] 42])"#, "PRS-060"),
        (r#"(query [42 :find ?x :where [?x :a ?v]])"#, "PRS-061"),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_expressions() {
    let cases: &[(&str, &str)] = &[
        (r#"(query [:find ?x :where [?x :a ?v] [()]])"#, "PRS-062"),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(:plus ?v 1)]])"#,
            "PRS-063",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(string? ?v ?v)]])"#,
            "PRS-064",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [?x :b ?w] [(+ ?v ?w 1)]])"#,
            "PRS-065",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [?x :b ?w] [(matches? ?v ?w)]])"#,
            "PRS-066",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [?x :b ?w] [(floor-div ?v ?w)]])"#,
            "PRS-067",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(< ?v 100) ?y :extra]])"#,
            "PRS-068",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(+ ?v 1) :result]])"#,
            "PRS-069",
        ),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(+ ?v {:x 1}) ?y]])"#,
            "PRS-070",
        ),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_tagged_literals() {
    let cases: &[(&str, &str)] = &[
        (r#"(transact [[#uuid :bogus :name "Alice"]])"#, "PRS-071"),
        (
            r#"(transact [[#uuid "not-a-valid-uuid" :name "Alice"]])"#,
            "PRS-072",
        ),
        (r#"(transact [[:alice :data #base64 "hello"]])"#, "PRS-073"),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

#[test]
fn prs_codes_cover_trailing_argument_rejection() {
    let cases: &[(&str, &str)] = &[
        (
            r#"(transact [[:alice :employment/status :active]] {:valid-from "2023-01-01T00:00:00Z"})"#,
            "PRS-075",
        ),
        (
            r#"(retract [[:alice :employment/status :active]] {:valid-from "2023-01-01T00:00:00Z"})"#,
            "PRS-076",
        ),
        (
            r#"(query [:find ?e :where [?e :a :b]]) garbage-trailing-tokens"#,
            "PRS-077",
        ),
        (
            r#"(query [:find ?e :where [?e :a :b]] :max-results 5)"#,
            "PRS-078",
        ),
        (
            r#"(rule [(reachable ?a ?b) [?a :edge ?b]] :unexpected-extra)"#,
            "PRS-079",
        ),
    ];
    for (input, code) in cases {
        assert_execute_code(input, code);
    }
}

// ── PRS-080: a :find variable must be bound in :where (#406) ───────────────

#[test]
fn prs_080_rejects_unbound_find_variables() {
    let cases: &[&str] = &[
        // Plain :find variable that appears nowhere in :where (a typo).
        r#"(query [:find ?n :where [?e :y ?v]])"#,
        // One bound, one unbound.
        r#"(query [:find ?e ?n :where [?e :y ?v]])"#,
        // `?_` variables are wildcards and never bind.
        r#"(query [:find ?_x :where [?_x :y 1]])"#,
        // Window function argument, :partition-by and :order-by variables.
        r#"(query [:find ?e (sum ?n :over (:order-by ?v)) :where [?e :y ?v]])"#,
        r#"(query [:find ?e (rank :over (:partition-by ?p :order-by ?v)) :where [?e :y ?v]])"#,
        r#"(query [:find ?e (row-number :over (:order-by ?o)) :where [?e :y ?v]])"#,
        r#"(query [:find ?e (lag ?missing :over (:order-by ?v)) :where [?e :y ?v]])"#,
    ];
    for input in cases {
        assert_execute_code(input, "PRS-080");
        assert_prepare_code(input, "PRS-080");
    }
}

#[test]
fn prs_080_names_the_unbound_variable() {
    let err = db()
        .execute(r#"(query [:find ?e ?typo :where [?e :y ?v]])"#)
        .expect_err("unbound :find variable must fail");
    assert!(err.to_string().contains("?typo"), "message names ?typo");
}

#[test]
fn find_variables_bound_by_any_binding_clause_are_accepted() {
    let db = db();
    db.execute(r#"(transact [[:a :y 1] [:a :z 2]])"#).unwrap();
    db.execute(r#"(rule [(r ?x ?y) [?x :y ?y]])"#).unwrap();
    let cases: &[&str] = &[
        r#"(query [:find ?e ?v :where [?e :y ?v]])"#,
        r#"(query [:find ?e ?a :where [?e ?a 1]])"#,
        r#"(query [:find ?x ?y :where (r ?x ?y)])"#,
        r#"(query [:find ?e ?w :where [?e :y ?v] [(+ ?v 1) ?w]])"#,
        r#"(query [:find ?e ?v :where [?e :y ?v] (or-join [?e] [?e :z 2] [?e :y 3])])"#,
        r#"(query [:find ?e ?v :where (or [?e :y ?v] [?e :z ?v])])"#,
        r#"(query [:find ?e ?vf :any-valid-time :where [?e :y ?v] [?e :db/valid-from ?vf]])"#,
        r#"(query [:find ?e (sum ?v :over (:partition-by ?e :order-by ?v)) :where [?e :y ?v]])"#,
    ];
    for input in cases {
        db.execute(input).expect("query with bound :find variables");
    }
    let pq = db
        .prepare(r#"(query [:find ?e :where [?e :y $v]])"#)
        .expect("bind slots are values, not :find variables");
    pq.execute(&[("v", minigraf::BindValue::Val(minigraf::Value::Integer(1)))])
        .expect("prepared query executes");
}

// ── db.prepare() surfaces the same PRS codes as db.execute() ────────────────

fn assert_prepare_code(input: &str, expected_code: &str) {
    let db = db();
    let result = db.prepare(input);
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!(
            "expected {} for input, but prepare() succeeded: {}",
            expected_code, input
        ),
    };
    assert_eq!(
        err.category(),
        ErrorCategory::Parser,
        "expected Parser category for {}",
        expected_code
    );
    assert_eq!(err.code(), expected_code, "wrong code for input: {}", input);
}

#[test]
fn prs_codes_reach_through_prepare() {
    let cases: &[(&str, &str)] = &[
        ("", "PRS-001"),
        ("(query)", "PRS-013"),
        ("(query [:find ?e :where [?e :name ?n] :as-of])", "PRS-014"),
        (r#"(query [:find ?x :where [?x :a ?v] 42])"#, "PRS-060"),
        (
            r#"(query [:find ?x :where [?x :a ?v] [(:plus ?v 1)]])"#,
            "PRS-063",
        ),
        (
            r#"(query [:find ?e :where [?e :a :b]] :max-results 5)"#,
            "PRS-078",
        ),
    ];
    for (input, code) in cases {
        assert_prepare_code(input, code);
    }
}

// ── Non-query commands still can't be prepared (API-0xx, not PRS) ──────────

#[test]
fn prepare_rejects_transact_command_not_prs() {
    let db = db();
    let result = db.prepare(r#"(transact [[:alice :name "Alice"]])"#);
    let err = result.expect_err("transact should not be preparable");
    // This is an Api-category rejection (only query commands can be
    // prepared), not a parser error — the input parses fine as a valid
    // Transact command, it's just the wrong command kind for prepare().
    assert_ne!(err.category(), ErrorCategory::Parser);
}

// ── PRS-083..PRS-107: parse errors that were INT-019 (#643) ─────────────────

/// Execute `input`, assert the Parser-category `expected_code`, and that the
/// message carries `needle` (the error's argument, or its fixed text).
fn assert_code_and_message(input: &str, expected_code: &str, needle: &str) {
    assert_execute_code(input, expected_code);
    let err = db().execute(input).expect_err("input must be rejected");
    let text = err.to_string();
    assert!(
        text.contains(needle),
        "message for {} lacks {}: {}",
        expected_code,
        needle,
        text
    );
    assert!(
        !text.contains("datalog parser:"),
        "{} message keeps the INT-019 prefix",
        expected_code
    );
}

#[test]
fn prs_083_bind_slot_without_name() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :name $]])",
        "PRS-083",
        "Bind slot '$'",
    );
}

#[test]
fn prs_084_over_without_order_by() {
    assert_code_and_message(
        "(query [:find ?e (rank :over (:partition-by ?d)) :where [?e :dept ?d]])",
        "PRS-084",
        "':order-by' is required",
    );
}

#[test]
fn prs_085_query_argument_not_a_vector() {
    assert_code_and_message(
        "(query (:find ?e :where [?e :a ?v]))",
        "PRS-085",
        "Query argument must be a vector",
    );
}

#[test]
fn prs_086_with_takes_only_variables() {
    assert_code_and_message(
        "(query [:find (count ?e) :with 7 :where [?e :a ?v]])",
        "PRS-086",
        "7",
    );
}

#[test]
fn prs_087_find_element_not_variable_or_aggregate() {
    assert_code_and_message("(query [:find 7 :where [?e :a ?v]])", "PRS-087", "7");
}

#[test]
fn prs_088_unknown_valid_time_key() {
    assert_code_and_message(
        r#"(transact {:valid-at "2024-01-01"} [[:alice :name "Alice"]])"#,
        "PRS-088",
        ":valid-at",
    );
}

#[test]
fn prs_089_retract_fact_not_a_vector() {
    assert_code_and_message(
        "(retract [:alice])",
        "PRS-089",
        "Each fact must be a vector",
    );
}

#[test]
fn prs_090_retract_fact_wrong_arity() {
    assert_code_and_message("(retract [[:alice :name]])", "PRS-090", "got 2");
    assert_code_and_message(
        r#"(retract [[:alice :name "Alice" 1]])"#,
        "PRS-090",
        "got 4",
    );
}

#[test]
fn prs_091_invalid_regex() {
    assert_code_and_message(
        r#"(query [:find ?e :where [?e :name ?n] [(matches? ?n "(")]])"#,
        "PRS-091",
        "\"(\"",
    );
}

#[test]
fn prs_092_not_body_item_not_a_clause() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (not 7)])",
        "PRS-092",
        "7",
    );
}

#[test]
fn prs_093_not_join_nested_in_not() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (not (not-join [?e] [?e :b 1]))])",
        "PRS-093",
        "cannot appear inside another",
    );
}

#[test]
fn prs_094_not_join_first_argument_not_a_vector() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (not-join ?e [?e :b 1])])",
        "PRS-094",
        "first argument must be a vector",
    );
}

#[test]
fn prs_095_not_join_variable_not_logic_variable() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (not-join [7] [?e :b 1])])",
        "PRS-095",
        "7",
    );
}

#[test]
fn prs_096_not_join_body_item_not_a_clause() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (not-join [?e] 7)])",
        "PRS-096",
        "7",
    );
}

#[test]
fn prs_097_rule_invocation_head_not_a_symbol() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (7 ?e)])",
        "PRS-097",
        "7",
    );
}

#[test]
fn prs_098_pattern_wrong_arity() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] [?e :b]])",
        "PRS-098",
        "got 2",
    );
}

#[test]
fn prs_099_or_branch_item_not_a_clause() {
    assert_code_and_message(
        "(query [:find ?e :where [?e :a ?v] (or 7)])",
        "PRS-099",
        "7",
    );
}

#[test]
fn prs_100_rule_without_body() {
    assert_code_and_message("(rule)", "PRS-100", "Rule must have a body");
}

#[test]
fn prs_101_rule_body_not_a_vector() {
    assert_code_and_message(
        "(rule (reachable ?a ?b))",
        "PRS-101",
        "Rule body must be a vector",
    );
}

#[test]
fn prs_102_rule_body_empty() {
    assert_code_and_message("(rule [])", "PRS-102", "Rule body cannot be empty");
}

#[test]
fn prs_103_rule_head_not_a_list() {
    assert_code_and_message(
        "(rule [reachable [?a :edge ?b]])",
        "PRS-103",
        "Rule head must be a list",
    );
}

#[test]
fn prs_104_rule_head_empty() {
    assert_code_and_message(
        "(rule [() [?a :edge ?b]])",
        "PRS-104",
        "Rule head cannot be empty",
    );
}

#[test]
fn prs_105_rule_head_not_a_symbol() {
    assert_code_and_message(
        "(rule [(7 ?a) [?a :edge ?b]])",
        "PRS-105",
        "Rule head must start with a symbol",
    );
}

#[test]
fn prs_106_rule_body_clause_not_vector_or_list() {
    assert_code_and_message("(rule [(reachable ?a) 7])", "PRS-106", "7");
}

#[test]
fn prs_107_rule_body_has_no_clause() {
    assert_code_and_message(
        "(rule [(reachable ?a)])",
        "PRS-107",
        "at least one pattern or rule invocation",
    );
}

#[test]
fn prs_083_to_107_reach_through_prepare() {
    let cases: &[(&str, &str)] = &[
        ("(query [:find ?e :where [?e :name $]])", "PRS-083"),
        ("(query [:find 7 :where [?e :a ?v]])", "PRS-087"),
        ("(query [:find ?e :where [?e :a ?v] [?e :b]])", "PRS-098"),
    ];
    for (input, code) in cases {
        assert_prepare_code(input, code);
    }
}

// ── PRS-108: a fact with more than four elements (#649) ─────────────────────

#[test]
fn prs_108_fact_with_more_than_four_elements() {
    assert_code_and_message(
        "(transact [[:alice :person/age 1 {} 99 :junk]])",
        "PRS-108",
        "got 6",
    );
    assert_code_and_message(
        r#"(transact [[:alice :name "Alice" "bogus" 7]])"#,
        "PRS-108",
        "got 5",
    );
}

#[test]
fn prs_108_rejects_the_whole_transaction() {
    // Before #649 the elements after the map were dropped and the fact stored.
    let db = db();
    db.execute(r#"(transact [[:bob :person/age 2] [:alice :person/age 1 {} 99 :junk]])"#)
        .expect_err("a 6-element fact must be rejected");
    let result = db
        .execute("(query [:find ?e ?v :where [?e :person/age ?v]])")
        .expect("query must run");
    let minigraf::QueryResult::QueryResults { results, .. } = result else {
        panic!("expected query results");
    };
    assert_eq!(
        results.len(),
        0,
        "no fact of the rejected transaction is stored"
    );
}

// ── #651: inputs the parser accepted and the grammar rejects ───────────────

#[test]
fn prs_109_unterminated_string() {
    assert_code_and_message(r#"(transact [[:a :b "abc]])"#, "PRS-109", "Unterminated");
    assert_execute_code(r#"(transact [[:a :b "abc\"#, "PRS-109");
}

#[test]
fn prs_110_keyword_without_a_name() {
    assert_code_and_message("(transact [[:a : 1]])", "PRS-110", "':'");
    assert_execute_code("(query [:find ?e :where [?e :(]])", "PRS-110");
}

#[test]
fn prs_111_empty_query_vector() {
    assert_code_and_message("(query [])", "PRS-111", "empty");
}

#[test]
fn prs_112_empty_query_section() {
    assert_code_and_message("(query [:find :where [?e :a ?v]])", "PRS-112", ":find");
    assert_code_and_message("(query [:find ?e :where])", "PRS-112", ":where");
    assert_code_and_message("(query [:where])", "PRS-112", ":where");
    assert_code_and_message(
        "(query [:find (count ?e) :with :where [?e :a ?v]])",
        "PRS-112",
        ":with",
    );
    assert_execute_code("(query [:find ?e :where [?e :a ?v] :with])", "PRS-112");
}

#[test]
fn prs_113_unknown_query_keyword() {
    assert_code_and_message(
        "(query [:find ?e :foo :where [?e :a ?v]])",
        "PRS-113",
        ":foo",
    );
    // With a value after it the keyword used to open a section of its own (PRS-061).
    assert_code_and_message(
        "(query [:find ?e :in ?v :where [?e :a ?v]])",
        "PRS-113",
        ":in",
    );
}

#[test]
fn prs_114_rule_head_argument_not_a_variable() {
    assert_code_and_message("(rule [(r ?a :c) [?a :edge :c]])", "PRS-114", ":c");
    assert_execute_code("(rule [(r ?a 0) [?a :edge ?b]])", "PRS-114");
}

#[test]
fn prs_115_limit_not_a_non_negative_integer() {
    assert_code_and_message(
        "(query [:find ?a :where [?a :edge ?b] :limit -1])",
        "PRS-115",
        ":limit",
    );
    assert_execute_code(
        "(query [:find ?a :where [?a :edge ?b] :limit 2.5])",
        "PRS-115",
    );
}

#[test]
fn prs_116_offset_without_order_by() {
    assert_code_and_message(
        "(query [:find ?a :where [?a :edge ?b] :offset 2 :limit 1])",
        "PRS-116",
        ":order-by",
    );
}

#[test]
fn prs_117_order_by_entry_not_in_find() {
    assert_code_and_message(
        "(query [:find ?a :where [?a :edge ?b] :order-by [?b]])",
        "PRS-117",
        "?b",
    );
    assert_execute_code(
        "(query [:find ?a :where [?a :edge ?b] :order-by [(count ?a)]])",
        "PRS-117",
    );
}

#[test]
fn prs_118_malformed_order_by() {
    assert_code_and_message(
        "(query [:find ?a :where [?a :edge ?b] :order-by [?a ?a]])",
        "PRS-118",
        "twice",
    );
    for q in [
        "(query [:find ?a :where [?a :edge ?b] :order-by ?a])",
        "(query [:find ?a :where [?a :edge ?b] :order-by []])",
        "(query [:find ?a :where [?a :edge ?b] :order-by [:desc ?a]])",
    ] {
        assert_execute_code(q, "PRS-118");
    }
}

#[test]
fn prs_105_rule_name_is_a_variable_or_an_operator() {
    assert_execute_code("(rule [(?r ?a) [?a :edge ?b]])", "PRS-105");
    assert_execute_code("(rule [(< ?a) [?a :edge ?b]])", "PRS-105");
}

#[test]
fn prs_061_value_after_a_query_option() {
    // A value after an option and its argument used to join the section before it.
    for q in [
        "(query [:find ?a :any-valid-time ?b :where [?a :edge ?b]])",
        "(query [:find ?a :as-of 1 ?b :where [?a :edge ?b]])",
        r#"(query [:find ?a :valid-at "2024-01-01T00:00:00Z" ?b :where [?a :edge ?b]])"#,
        "(query [:find ?a :max-results 5 ?b :where [?a :edge ?b]])",
        "(query [:find ?a :where [?a :edge ?b] :max-derived-facts 5 [?b :edge ?c]])",
        "(query [:find ?a :limit 5 ?b :where [?a :edge ?b]])",
    ] {
        assert_execute_code(q, "PRS-061");
    }
}

#[test]
fn prs_097_call_head_is_a_variable_or_an_operator() {
    // `(< ?a ?b)` as a clause was a call of a rule named `<` and returned no rows.
    for q in [
        "(query [:find ?a :where [?a :edge ?b] (< ?a ?b)])",
        "(query [:find ?a :where [?a :edge ?b] (?r ?a)])",
        "(query [:find ?a :where [?a :edge ?b] (not (?r ?a))])",
        "(query [:find ?a :where [?a :edge ?b] (or (?r ?a) [?a :x 1])])",
    ] {
        assert_execute_code(q, "PRS-097");
    }
    assert_execute_code("(rule [(r ?a) [?a :edge ?b] (= ?a ?b)])", "PRS-097");
}

#[test]
fn prs_025_029_063_function_named_by_a_variable() {
    assert_execute_code("(query [:find (?f ?a) :where [?a :edge ?b]])", "PRS-025");
    assert_execute_code(
        "(query [:find ?a (?f ?b :over (:order-by ?b)) :where [?a :edge ?b]])",
        "PRS-029",
    );
    assert_execute_code(
        "(query [:find ?a :where [?a :edge ?b] [(?f ?a) ?z]])",
        "PRS-063",
    );
}

#[test]
fn accepted_forms_next_to_the_tightened_ones() {
    let db = db();
    db.execute("(transact [[:a :edge :b] [:a :n 5]])").unwrap();
    for q in [
        // An option between sections, then a keyword: the next section starts.
        "(query [:find ?a :any-valid-time :where [?a :edge ?b]])",
        "(query [:find ?a :as-of 1 :where [?a :edge ?b]])",
        // `-` alone is the subtraction operator.
        "(query [:find ?d :where [?a :n ?n] [(- ?n 1) ?d]])",
        // A rule call named by a symbol, and an aggregate named by an operator-free symbol.
        "(query [:find (count ?a) :where [?a :edge ?b]])",
    ] {
        db.execute(q).expect("query must parse and run");
    }
    db.execute("(rule [(linked ?a ?b) [?a :edge ?b]])").unwrap();
    db.execute("(rule [(none) [?a :edge ?b]])").unwrap();
    db.execute("(query [:find ?a :where (linked ?a ?b)])")
        .unwrap();
}
