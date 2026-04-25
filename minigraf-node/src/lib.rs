#![deny(clippy::all)]

use minigraf::{QueryResult, Value};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::{Arc, Mutex};

// ─── Value → JSON ─────────────────────────────────────────────────────────────

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::String(s) => J::String(s.clone()),
        Value::Integer(i) => serde_json::json!(i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Boolean(b) => J::Bool(*b),
        Value::Ref(u) => J::String(u.to_string()),
        Value::Keyword(k) => J::String(k.clone()),
        Value::Null => J::Null,
    }
}

fn query_result_to_json(result: QueryResult) -> String {
    let val = match result {
        QueryResult::Transacted(tx) => serde_json::json!({"transacted": tx}),
        QueryResult::Retracted(tx) => serde_json::json!({"retracted": tx}),
        QueryResult::Ok => serde_json::json!({"ok": true}),
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<serde_json::Value>> = results
                .iter()
                .map(|r| r.iter().map(value_to_json).collect())
                .collect();
            serde_json::json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

// ─── MiniGrafDb ───────────────────────────────────────────────────────────────

#[napi]
pub struct MiniGrafDb {
    inner: Arc<Mutex<minigraf::Minigraf>>,
}

#[napi]
impl MiniGrafDb {
    /// Open a file-backed database. Throws on error.
    #[napi(constructor)]
    pub fn new(path: String) -> Result<Self> {
        let db = minigraf::Minigraf::open(&path)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(db)),
        })
    }

    /// Open an in-memory database. Throws on error.
    #[napi(factory)]
    pub fn in_memory() -> Result<Self> {
        let db = minigraf::Minigraf::in_memory()
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(db)),
        })
    }

    /// Execute a Datalog string. Returns a JSON string. Throws on error.
    #[napi]
    pub fn execute(&self, datalog: String) -> Result<String> {
        let result = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "mutex poisoned"))?
            .execute(&datalog)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(query_result_to_json(result))
    }

    /// Flush the WAL to disk. Throws on error.
    #[napi]
    pub fn checkpoint(&self) -> Result<()> {
        self.inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "mutex poisoned"))?
            .checkpoint()
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }
}
