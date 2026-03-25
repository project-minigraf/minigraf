//! Integration tests for Phase 7.2b: arithmetic and predicate expression clauses.

use minigraf::{Minigraf, QueryResult, Value as MgValue};

fn open() -> Minigraf {
    Minigraf::in_memory().expect("open")
}

fn count(result: QueryResult) -> usize {
    match result {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected QueryResults"),
    }
}

fn rows(result: QueryResult) -> Vec<Vec<MgValue>> {
    match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

// ── Comparison filters ────────────────────────────────────────────────────────

#[test]
fn test_lt_filter_keeps_matching_rows() {
    let db = open();
    db.execute("(transact [[:a :price 50] [:b :price 150] [:c :price 80]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :price ?p] [(< ?p 100)]])").expect("query");
    assert_eq!(count(r), 2);
}

#[test]
fn test_gt_filter() {
    let db = open();
    db.execute("(transact [[:a :score 10] [:b :score 90]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :score ?s] [(> ?s 50)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_two_variable_comparison_gte() {
    let db = open();
    db.execute("(transact [[:a :x 10] [:a :y 5] [:b :x 3] [:b :y 7]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :x ?x] [?e :y ?y] [(>= ?x ?y)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_eq_filter_string() {
    let db = open();
    db.execute("(transact [[:alice :name \"Alice\"] [:bob :name \"Bob\"]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :name ?n] [(= ?n \"Alice\")]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_neq_filter() {
    let db = open();
    db.execute("(transact [[:a :status :active] [:b :status :inactive] [:c :status :active]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :status ?s] [(!= ?s :inactive)]])").expect("query");
    assert_eq!(count(r), 2);
}

// ── Type mismatch → silent drop ───────────────────────────────────────────────

#[test]
fn test_lt_type_mismatch_drops_row_silently() {
    let db = open();
    // :v is a string for :a, integer for :b
    db.execute("(transact [[:a :v \"hello\"] [:b :v 50]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :v ?v] [(< ?v 100)]])").expect("query");
    // Only :b qualifies; :a's string silently drops
    assert_eq!(count(r), 1);
}

// ── Arithmetic bindings ───────────────────────────────────────────────────────

#[test]
fn test_multiply_binding() {
    let db = open();
    db.execute("(transact [[:a :price 10] [:a :qty 3] [:b :price 20] [:b :qty 2]])").expect("transact");
    let r = db.execute("(query [:find ?e ?total :where [?e :price ?p] [?e :qty ?q] [(* ?p ?q) ?total]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 2);
}

#[test]
fn test_add_binding() {
    let db = open();
    db.execute("(transact [[:a :x 3] [:a :y 4]])").expect("transact");
    let r = db.execute("(query [:find ?sum :where [:a :x ?x] [:a :y ?y] [(+ ?x ?y) ?sum]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0][0], MgValue::Integer(7));
}

#[test]
fn test_nested_arithmetic() {
    let db = open();
    db.execute("(transact [[:a :x 3] [:a :y 5]])").expect("transact");
    // (+ (* ?x 2) ?y) = 3*2+5 = 11
    let r = db.execute("(query [:find ?result :where [:a :x ?x] [:a :y ?y] [(+ (* ?x 2) ?y) ?result]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0][0], MgValue::Integer(11));
}

#[test]
fn test_integer_division_truncates() {
    let db = open();
    db.execute("(transact [[:a :n 5] [:a :d 2]])").expect("transact");
    let r = db.execute("(query [:find ?r :where [:a :n ?n] [:a :d ?d] [(/ ?n ?d) ?r]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs[0][0], MgValue::Integer(2));
}

#[test]
fn test_division_by_zero_drops_row() {
    let db = open();
    db.execute("(transact [[:a :n 5] [:a :d 0]])").expect("transact");
    let r = db.execute("(query [:find ?r :where [:a :n ?n] [:a :d ?d] [(/ ?n ?d) ?r]])").expect("query");
    assert_eq!(count(r), 0);
}

#[test]
fn test_int_float_promotion() {
    let db = open();
    db.execute("(transact [[:a :i 1] [:a :f 1.5]])").expect("transact");
    let r = db.execute("(query [:find ?r :where [:a :i ?i] [:a :f ?f] [(+ ?i ?f) ?r]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs[0][0], MgValue::Float(2.5));
}

// ── Type predicates ───────────────────────────────────────────────────────────

#[test]
fn test_string_predicate_filter() {
    let db = open();
    db.execute("(transact [[:a :v \"hello\"] [:b :v 42]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :v ?v] [(string? ?v)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_integer_predicate_filter() {
    let db = open();
    db.execute("(transact [[:a :v \"hello\"] [:b :v 42]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :v ?v] [(integer? ?v)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_nil_predicate_filter() {
    let db = open();
    db.execute("(transact [[:a :v nil] [:b :v 1]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :v ?v] [(nil? ?v)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_predicate_binding() {
    let db = open();
    db.execute("(transact [[:a :v \"hello\"] [:b :v 42]])").expect("transact");
    let r = db.execute("(query [:find ?e ?is-str :where [?e :v ?v] [(string? ?v) ?is-str]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 2);
}

// ── String predicates ─────────────────────────────────────────────────────────

#[test]
fn test_starts_with_filter() {
    let db = open();
    db.execute("(transact [[:a :tag \"work-project\"] [:b :tag \"personal\"]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :tag ?t] [(starts-with? ?t \"work\")]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_ends_with_filter() {
    let db = open();
    db.execute("(transact [[:a :file \"main.rs\"] [:b :file \"README.md\"]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :file ?f] [(ends-with? ?f \".rs\")]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_contains_filter() {
    let db = open();
    db.execute("(transact [[:a :bio \"senior engineer\"] [:b :bio \"designer\"]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :bio ?b] [(contains? ?b \"engineer\")]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_matches_filter() {
    let db = open();
    db.execute("(transact [[:a :email \"user@example.com\"] [:b :email \"not-an-email\"]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :email ?addr] [(matches? ?addr \"^[^@]+@[^@]+$\")]])").expect("query");
    assert_eq!(count(r), 1);
}

// ── Expr inside `not` body ────────────────────────────────────────────────────

#[test]
fn test_expr_inside_not_body() {
    let db = open();
    db.execute("(transact [[:a :price 50] [:b :price 150]])").expect("transact");
    // Exclude items where price > 100 (using expr inside not)
    let r = db.execute("(query [:find ?e :where [?e :price ?p] (not [(> ?p 100)])])").expect("query");
    assert_eq!(count(r), 1);
}

// ── Expr in rule body ─────────────────────────────────────────────────────────

#[test]
fn test_expr_in_rule_body() {
    let db = open();
    db.execute("(transact [[:a :score 90] [:b :score 40] [:c :score 75]])").expect("transact");
    db.execute("(rule [(passing ?e) [?e :score ?s] [(>= ?s 70)]])").expect("rule");
    let r = db.execute("(query [:find ?e :where (passing ?e)])").expect("query");
    assert_eq!(count(r), 2);
}

// ── Expr combined with :as-of (bi-temporal) ───────────────────────────────────

#[test]
fn test_expr_with_as_of() {
    let db = open();
    // Transact :a at tx_count=1
    db.execute("(transact [[:a :age 25]])").expect("transact 1");
    // Transact :b at tx_count=2
    db.execute("(transact [[:b :age 35]])").expect("transact 2");
    // :as-of 1 sees only :a (tx_count=1); [(< ?age 30)] keeps it → 1 result
    let r1 = db.execute("(query [:find ?e :as-of 1 :where [?e :age ?age] [(< ?age 30)]])").expect("query as-of 1");
    assert_eq!(count(r1), 1, "as-of 1 with expr filter should return 1");
    // :as-of 2 sees both :a and :b; [(< ?age 30)] keeps only :a → still 1 result
    let r2 = db.execute("(query [:find ?e :as-of 2 :where [?e :age ?age] [(< ?age 30)]])").expect("query as-of 2");
    assert_eq!(count(r2), 1, "as-of 2 with expr filter should return 1");
}

// ── Additional operator / predicate coverage ──────────────────────────────────

#[test]
fn test_sub_binding() {
    let db = open();
    db.execute("(transact [[:a :x 10] [:a :y 3]])").expect("transact");
    let r = db.execute("(query [:find ?r :where [:a :x ?x] [:a :y ?y] [(- ?x ?y) ?r]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0][0], MgValue::Integer(7));
}

#[test]
fn test_float_predicate_filter() {
    let db = open();
    db.execute("(transact [[:a :v 1.5] [:b :v 42]])").expect("transact");
    let r = db.execute("(query [:find ?e :where [?e :v ?v] [(float? ?v)]])").expect("query");
    assert_eq!(count(r), 1);
}

#[test]
fn test_eq_cross_type_is_false_not_error() {
    // (= 1 1.0) → false (different Value variants), not a row drop
    let db = open();
    db.execute("(transact [[:a :n 1]])").expect("transact");
    // Bind ?is-eq to (= ?n 1.0) — should be false (int != float structurally), not a drop
    let r = db.execute("(query [:find ?is-eq :where [:a :n ?n] [(= ?n 1.0) ?is-eq]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0][0], MgValue::Boolean(false));
}

#[test]
fn test_matches_invalid_regex_is_parse_error() {
    let db = open();
    // Invalid regex must be rejected at parse time, not at query time
    let result = db.execute("(query [:find ?e :where [?e :v ?v] [(matches? ?v \"[unclosed\")]])");
    assert!(result.is_err(), "invalid regex must be a parse error");
}

// ── Arithmetic binding into aggregate ────────────────────────────────────────

#[test]
fn test_arithmetic_binding_into_sum_aggregate() {
    let db = open();
    db.execute("(transact [[:a :price 10] [:a :qty 3] [:b :price 5] [:b :qty 4]])").expect("transact");
    // total = price * qty; :with ?e keeps each entity in its own group → 2 rows
    let r = db.execute("(query [:find (sum ?total) :with ?e :where [?e :price ?p] [?e :qty ?q] [(* ?p ?q) ?total]])").expect("query");
    let rs = rows(r);
    // :with ?e → one row per entity (each entity forms its own group)
    assert_eq!(rs.len(), 2);
    // Collect totals and sort to get deterministic order: [20, 30]
    let mut totals: Vec<i64> = rs
        .iter()
        .map(|row| match &row[0] {
            MgValue::Integer(n) => *n,
            _ => panic!("expected integer total"),
        })
        .collect();
    totals.sort_unstable();
    assert_eq!(totals, vec![20, 30]);
}
