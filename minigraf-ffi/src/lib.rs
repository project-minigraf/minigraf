use minigraf::{QueryResult, Value};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

// ─── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MiniGrafError {
    #[error("storage error: {msg}")]
    Storage { msg: String },
    #[error("query error: {msg}")]
    Query { msg: String },
    #[error("parse error: {msg}")]
    Parse { msg: String },
    #[error("unknown error: {msg}")]
    Other { msg: String },
}

// ─── MiniGrafDb stub (needed for test compilation) ───────────────────────────

#[derive(uniffi::Object)]
pub struct MiniGrafDb {
    #[allow(dead_code)] // populated in Task 5
    inner: Arc<Mutex<minigraf::Minigraf>>,
}

#[uniffi::export]
impl MiniGrafDb {
    #[uniffi::constructor]
    pub fn open(_path: String) -> Result<Arc<Self>, MiniGrafError> {
        todo!()
    }

    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Arc<Self>, MiniGrafError> {
        todo!()
    }

    pub fn execute(&self, _datalog: String) -> Result<String, MiniGrafError> {
        todo!()
    }

    pub fn checkpoint(&self) -> Result<(), MiniGrafError> {
        todo!()
    }
}

// ─── JSON serialisation (internal helpers) ───────────────────────────────────

#[allow(dead_code)] // called by execute() in Task 5
fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as JVal;
    match v {
        Value::String(s) => JVal::String(s.clone()),
        Value::Integer(i) => JVal::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(JVal::Number)
            .unwrap_or(JVal::Null),
        Value::Boolean(b) => JVal::Bool(*b),
        Value::Ref(uuid) => JVal::String(uuid.to_string()),
        Value::Keyword(k) => JVal::String(k.clone()),
        Value::Null => JVal::Null,
    }
}

#[allow(dead_code)] // called by execute() in Task 5
fn query_result_to_json(result: QueryResult) -> String {
    use serde_json::json;
    let val = match result {
        QueryResult::Transacted(tx_id) => json!({"transacted": tx_id}),
        QueryResult::Retracted(tx_id) => json!({"retracted": tx_id}),
        QueryResult::Ok => json!({"ok": true}),
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<serde_json::Value>> = results
                .iter()
                .map(|row| row.iter().map(value_to_json).collect())
                .collect();
            json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_to_json_string() {
        let v = Value::String("hello".into());
        let j = value_to_json(&v);
        assert_eq!(j, serde_json::Value::String("hello".into()));
    }

    #[test]
    fn value_to_json_integer() {
        let v = Value::Integer(42);
        let j = value_to_json(&v);
        assert_eq!(j, serde_json::json!(42));
    }

    #[test]
    fn value_to_json_null() {
        let j = value_to_json(&Value::Null);
        assert_eq!(j, serde_json::Value::Null);
    }

    #[test]
    fn query_result_to_json_transacted() {
        let json = query_result_to_json(QueryResult::Transacted(12345));
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["transacted"], serde_json::json!(12345));
    }

    #[test]
    fn query_result_to_json_query_results() {
        let result = QueryResult::QueryResults {
            vars: vec!["?name".into()],
            results: vec![vec![Value::String("Alice".into())]],
        };
        let json = query_result_to_json(result);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["variables"][0], "?name");
        assert_eq!(v["results"][0][0], "Alice");
    }

    #[test]
    fn query_result_to_json_ok() {
        let json = query_result_to_json(QueryResult::Ok);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["ok"], serde_json::json!(true));
    }

    #[test]
    fn value_to_json_float_finite() {
        let j = value_to_json(&Value::Float(3.14));
        assert_eq!(j, serde_json::json!(3.14));
    }

    #[test]
    fn value_to_json_float_nan() {
        let j = value_to_json(&Value::Float(f64::NAN));
        assert_eq!(j, serde_json::Value::Null);
    }

    #[test]
    fn value_to_json_float_infinity() {
        let j = value_to_json(&Value::Float(f64::INFINITY));
        assert_eq!(j, serde_json::Value::Null);
    }

    #[test]
    fn value_to_json_boolean() {
        assert_eq!(value_to_json(&Value::Boolean(true)), serde_json::json!(true));
        assert_eq!(value_to_json(&Value::Boolean(false)), serde_json::json!(false));
    }

    #[test]
    fn value_to_json_ref() {
        let id = minigraf::EntityId::new_v4();
        let j = value_to_json(&Value::Ref(id));
        assert_eq!(j, serde_json::Value::String(id.to_string()));
    }

    #[test]
    fn value_to_json_keyword() {
        let j = value_to_json(&Value::Keyword(":status/active".into()));
        assert_eq!(j, serde_json::Value::String(":status/active".into()));
    }

    #[test]
    fn query_result_to_json_retracted() {
        let json = query_result_to_json(QueryResult::Retracted(99));
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["retracted"], serde_json::json!(99));
    }
}
