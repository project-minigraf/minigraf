use super::types::EdnValue;
use crate::graph::types::{EntityId, Value, ValueSet};
use uuid::Uuid;

#[cfg(test)]
thread_local! {
    /// #774: count conversions, independently of wall-clock timing.
    pub(crate) static KEYWORD_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Convert EdnValue to Value for storage
pub fn edn_to_value(edn: &EdnValue) -> Result<Value, String> {
    match edn {
        EdnValue::String(s) => Ok(Value::String(s.clone())),
        EdnValue::Integer(i) => Ok(Value::Integer(*i)),
        EdnValue::Float(f) => Ok(Value::Float(*f)),
        EdnValue::Boolean(b) => Ok(Value::Boolean(*b)),
        EdnValue::Keyword(k) => Ok(Value::Keyword(k.clone())),
        EdnValue::Uuid(u) => Ok(Value::Ref(*u)),
        EdnValue::Nil => Ok(Value::Null),
        EdnValue::Set(elements) => {
            let values = elements
                .iter()
                .map(edn_to_value)
                .collect::<Result<Vec<_>, String>>()?;
            ValueSet::new(values)
                .map(Value::Set)
                .map_err(|e| e.to_string())
        }
        EdnValue::Symbol(s) if s.starts_with('?') => {
            Err(format!("Cannot convert unbound variable {} to value", s))
        }
        _ => Err(format!("Cannot convert {:?} to Value", edn)),
    }
}

/// Convert EdnValue to EntityId (must be a keyword or UUID)
pub fn edn_to_entity_id(edn: &EdnValue) -> Result<EntityId, String> {
    match edn {
        EdnValue::Keyword(k) => {
            #[cfg(test)]
            KEYWORD_RESOLUTIONS.with(|n| n.set(n.get() + 1));
            // Convert keyword to deterministic UUID
            // For now, we'll use a simple hash-based approach
            // In production, you might want a more sophisticated method
            let hash = k.as_bytes();
            // Create a UUID from the keyword string
            // This is deterministic: same keyword always gives same UUID
            if let Ok(uuid) = Uuid::parse_str(k.trim_start_matches(':')) {
                Ok(uuid)
            } else {
                // Generate UUID from keyword name
                Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, hash))
            }
        }
        EdnValue::Uuid(u) => Ok(*u),
        _ => Err(format!(
            "Expected keyword or UUID for entity, got {:?}",
            edn
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edn_to_value() {
        assert_eq!(
            edn_to_value(&EdnValue::String("test".to_string())).unwrap(),
            Value::String("test".to_string())
        );
        assert_eq!(
            edn_to_value(&EdnValue::Integer(42)).unwrap(),
            Value::Integer(42)
        );
        assert_eq!(
            edn_to_value(&EdnValue::Boolean(true)).unwrap(),
            Value::Boolean(true)
        );

        // Variables should fail
        let result = edn_to_value(&EdnValue::Symbol("?x".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_edn_to_entity_id() {
        let uuid = Uuid::new_v4();
        assert_eq!(edn_to_entity_id(&EdnValue::Uuid(uuid)).unwrap(), uuid);

        // Keywords should generate deterministic UUIDs
        let result1 = edn_to_entity_id(&EdnValue::Keyword(":alice".to_string())).unwrap();
        let result2 = edn_to_entity_id(&EdnValue::Keyword(":alice".to_string())).unwrap();
        assert_eq!(result1, result2); // Same keyword = same UUID
    }
}

#[cfg(test)]
mod hash_join_tests {
    use crate::graph::FactStorage;
    use crate::query::datalog::executor::DatalogExecutor;

    fn make_join_db(n: usize, dept_count: usize) -> DatalogExecutor {
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(50) {
            let batch_end = (batch_start + 50).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
                cmd.push_str(&format!("[:e{i} :dept :d{d}]", i = i, d = i % dept_count));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap())
                .unwrap();
        }
        exec
    }

    /// Two-pattern join: every entity has both :dept and :val facts.
    /// Querying [:find ?e ?dept ?v :where [?e :dept ?dept] [?e :val ?v]] should
    /// return exactly one row per entity (N rows for N entities).
    #[test]
    fn two_pattern_join_returns_correct_count() {
        let n = 100;
        let exec = make_join_db(n, 10);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e ?dept ?v :where [?e :dept ?dept] [?e :val ?v]])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            // One row per entity — the join must not produce a cross-product.
            assert_eq!(results.len(), n, "expected one row per entity");
        } else {
            panic!("expected QueryResults");
        }
    }

    /// Correctness check: for 10 entities across 2 departments, the join must
    /// pair each entity with the right department and value (no mixing).
    #[test]
    fn two_pattern_join_values_are_correct() {
        // e0,e2,e4,e6,e8 → :d0;  e1,e3,e5,e7,e9 → :d1
        let n = 10;
        let exec = make_join_db(n, 2);
        let result = exec
            .execute(
                crate::query::datalog::parser::parse_datalog_command(
                    "(query [:find ?e ?dept ?v :where [?e :dept ?dept] [?e :val ?v]])",
                )
                .unwrap(),
            )
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults {
            vars, results, ..
        } = result
        {
            assert_eq!(results.len(), n, "expected one row per entity");
            let dept_idx = vars
                .iter()
                .position(|v| v == "?dept")
                .expect("?dept in vars");
            let val_idx = vars.iter().position(|v| v == "?v").expect("?v in vars");
            // Collect (dept, val) pairs; no entity should appear in both departments.
            let mut d0_vals: Vec<i64> = Vec::new();
            let mut d1_vals: Vec<i64> = Vec::new();
            for row in &results {
                if let (
                    crate::graph::types::Value::Keyword(dept),
                    crate::graph::types::Value::Integer(val),
                ) = (&row[dept_idx], &row[val_idx])
                {
                    if dept == ":d0" {
                        d0_vals.push(*val);
                    } else if dept == ":d1" {
                        d1_vals.push(*val);
                    }
                }
            }
            assert_eq!(d0_vals.len(), 5, "d0 should have 5 entities");
            assert_eq!(d1_vals.len(), 5, "d1 should have 5 entities");
            // All d0 vals are even (0,2,4,6,8) and d1 vals are odd (1,3,5,7,9).
            for v in &d0_vals {
                assert_eq!(v % 2, 0, "d0 entities have even vals");
            }
            for v in &d1_vals {
                assert_eq!(v % 2, 1, "d1 entities have odd vals");
            }
        } else {
            panic!("expected QueryResults");
        }
    }
}
