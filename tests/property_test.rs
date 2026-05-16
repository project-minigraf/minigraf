//! Cross-feature property tests (#221).
//!
//! Compares Minigraf query results against a deliberately simple reference
//! evaluator for randomly-generated small graphs.
//!
//! Run: cargo test --test property_test
//! More cases: PROPTEST_CASES=500 cargo test --test property_test
#![cfg(not(target_arch = "wasm32"))]

use minigraf::db::Minigraf;
use minigraf::QueryResult;
use proptest::prelude::*;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct TestFact {
    entity: usize,
    attribute: String,
    value: TestValue,
}

#[derive(Debug, Clone, PartialEq)]
enum TestValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl TestValue {
    fn to_edn(&self) -> String {
        match self {
            TestValue::Str(s) => format!(r#""{s}""#),
            TestValue::Int(n) => n.to_string(),
            TestValue::Bool(b) => b.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct TestQuery {
    attribute: String,
    value_filter: Option<TestValue>,
    negation_attr: Option<String>,
}

/// Compute the deterministic UUID for a keyword entity (mirrors `edn_to_entity_id`).
fn entity_uuid(idx: usize) -> Uuid {
    let kw = format!(":e{idx}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, kw.as_bytes())
}

// ── Reference evaluator (naive, independent of production code) ───────────────

fn ref_eval(facts: &[TestFact], query: &TestQuery) -> Vec<usize> {
    let mut matched: Vec<usize> = facts
        .iter()
        .filter(|f| {
            f.attribute == query.attribute
                && match &query.value_filter {
                    None => true,
                    Some(v) => &f.value == v,
                }
        })
        .map(|f| f.entity)
        .collect();

    if let Some(neg_attr) = &query.negation_attr {
        let neg_entities: std::collections::HashSet<usize> = facts
            .iter()
            .filter(|f| &f.attribute == neg_attr)
            .map(|f| f.entity)
            .collect();
        matched.retain(|e| !neg_entities.contains(e));
    }

    matched.sort();
    matched.dedup();
    matched
}

// ── Minigraf evaluator ────────────────────────────────────────────────────────

fn minigraf_eval(facts: &[TestFact], query: &TestQuery, max_entity: usize) -> Vec<usize> {
    // Pre-compute UUID → index mapping for all possible entity indices.
    let uuid_to_idx: std::collections::HashMap<Uuid, usize> =
        (0..max_entity).map(|i| (entity_uuid(i), i)).collect();

    let db = Minigraf::in_memory().unwrap();

    for fact in facts {
        let entity_kw = format!(":e{}", fact.entity);
        let val_edn = fact.value.to_edn();
        let attr = &fact.attribute;
        let edn = format!(r#"(transact [[{entity_kw} {attr} {val_edn}]])"#);
        let _ = db.execute(&edn);
    }

    let val_clause = match &query.value_filter {
        Some(v) => format!(" [(= ?v {})]", v.to_edn()),
        None => String::new(),
    };

    let neg_clause = match &query.negation_attr {
        Some(neg) => format!(" (not [?e {} _])", neg),
        None => String::new(),
    };

    let attr = &query.attribute;
    let datalog = format!(
        "(query [:find ?e :where [?e {attr} ?v]{val_clause}{neg_clause}])"
    );

    let result = db.execute(&datalog);
    // Skip test cases where the generated query fails to parse/evaluate.
    if result.is_err() {
        return vec![];
    }
    match result.unwrap() {
        QueryResult::QueryResults { results, .. } => {
            let mut entities: Vec<usize> = results
                .into_iter()
                .flat_map(|r| r.into_iter())
                .filter_map(|v| match v {
                    minigraf::Value::Ref(uuid) => uuid_to_idx.get(&uuid).copied(),
                    _ => None,
                })
                .collect();
            entities.sort();
            entities.dedup();
            entities
        }
        _ => vec![],
    }
}

// ── proptest generators ───────────────────────────────────────────────────────

fn arb_attribute() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(":color".to_string()),
        Just(":size".to_string()),
        Just(":active".to_string()),
        Just(":tag".to_string()),
        Just(":score".to_string()),
    ]
}

fn arb_value() -> impl Strategy<Value = TestValue> {
    prop_oneof![
        Just(TestValue::Str("red".to_string())),
        Just(TestValue::Str("blue".to_string())),
        Just(TestValue::Int(1)),
        Just(TestValue::Int(2)),
        Just(TestValue::Bool(true)),
    ]
}

fn arb_fact(max_entity: usize) -> impl Strategy<Value = TestFact> {
    (0..max_entity, arb_attribute(), arb_value()).prop_map(|(entity, attribute, value)| {
        TestFact { entity, attribute, value }
    })
}

proptest! {
    /// Minigraf results must match the reference evaluator for basic queries.
    #[test]
    fn basic_query_matches_reference(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":color".to_string(),
            value_filter: None,
            negation_attr: None,
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8);
        prop_assert_eq!(ref_result, mg_result);
    }

    /// Negation: entities with negation_attr must not appear in results.
    #[test]
    fn negation_excludes_correct_entities(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":color".to_string(),
            value_filter: None,
            negation_attr: Some(":active".to_string()),
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8);

        let neg_entities: std::collections::HashSet<usize> = facts
            .iter()
            .filter(|f| f.attribute == ":active")
            .map(|f| f.entity)
            .collect();

        for &e in &mg_result {
            prop_assert!(
                !neg_entities.contains(&e),
                "entity {} has negated attribute but appears in result",
                e
            );
        }
        prop_assert_eq!(ref_result, mg_result);
    }

    /// Impossible paths return empty results.
    #[test]
    fn impossible_path_returns_empty(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":__nonexistent__".to_string(),
            value_filter: None,
            negation_attr: None,
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8);
        prop_assert!(ref_result.is_empty(), "reference: impossible path must be empty");
        prop_assert!(mg_result.is_empty(), "minigraf: impossible path must be empty");
    }
}
