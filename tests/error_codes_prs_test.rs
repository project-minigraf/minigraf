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
            r#"(query [:find (lag ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
            "PRS-030",
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
            r#"(query [:find ?x :where [?x :a ?v] [(:+ ?v 1)]])"#,
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
            r#"(query [:find ?x :where [?x :a ?v] [(:+ ?v 1)]])"#,
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

// ── PRS-108: a fact with more than four elements (#649) ─────────────────────

/// Execute `input` and assert it fails with `expected_code` and a message
/// containing `needle`.
fn assert_code_and_message(input: &str, expected_code: &str, needle: &str) {
    assert_execute_code(input, expected_code);
    let err = db().execute(input).expect_err("input must be rejected");
    assert!(
        err.to_string().contains(needle),
        "message for {} lacks {}",
        expected_code,
        needle
    );
}

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
