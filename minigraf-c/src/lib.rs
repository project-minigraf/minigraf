#![deny(unsafe_op_in_unsafe_fn)]

use minigraf::{QueryResult, Value};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::Mutex;

// ─── Handle ──────────────────────────────────────────────────────────────────

pub struct MiniGrafDb {
    db: Mutex<minigraf::Minigraf>,
    last_error: Mutex<Option<CString>>,
}

impl MiniGrafDb {
    fn set_error(&self, msg: String) {
        *self.last_error.lock().unwrap() =
            Some(CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap()));
    }

    fn clear_error(&self) {
        *self.last_error.lock().unwrap() = None;
    }
}

// ─── Lifecycle ────────────────────────────────────────────────────────────────

/// Open a file-backed Minigraf database. Returns NULL on error.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_open(path: *const c_char) -> *mut MiniGrafDb {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let path = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    match minigraf::Minigraf::open(path) {
        Ok(db) => Box::into_raw(Box::new(MiniGrafDb {
            db: Mutex::new(db),
            last_error: Mutex::new(None),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Open an in-memory Minigraf database. Returns NULL on error.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_open_in_memory() -> *mut MiniGrafDb {
    match minigraf::Minigraf::in_memory() {
        Ok(db) => Box::into_raw(Box::new(MiniGrafDb {
            db: Mutex::new(db),
            last_error: Mutex::new(None),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Close a database and free all associated memory.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_close(handle: *mut MiniGrafDb) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

// ─── Execute ─────────────────────────────────────────────────────────────────

/// Execute a Datalog string. Returns a JSON string on success (caller must free
/// with `minigraf_string_free`), or NULL on error (call `minigraf_last_error`).
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_execute(
    handle: *mut MiniGrafDb,
    datalog: *const c_char,
) -> *mut c_char {
    if handle.is_null() || datalog.is_null() {
        return std::ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let datalog = match unsafe { CStr::from_ptr(datalog) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            handle.set_error("invalid UTF-8 in datalog string".into());
            return std::ptr::null_mut();
        }
    };

    let result = handle.db.lock().unwrap().execute(datalog);
    match result {
        Ok(qr) => {
            handle.clear_error();
            let json = query_result_to_json(qr);
            match CString::new(json) {
                Ok(s) => s.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        }
        Err(e) => {
            handle.set_error(format!("{e:#}"));
            std::ptr::null_mut()
        }
    }
}

/// Free a string returned by `minigraf_execute`.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

// ─── Checkpoint ───────────────────────────────────────────────────────────────

/// Flush the WAL to the database file. Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_checkpoint(handle: *mut MiniGrafDb) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &*handle };
    match handle.db.lock().unwrap().checkpoint() {
        Ok(_) => {
            handle.clear_error();
            0
        }
        Err(e) => {
            handle.set_error(format!("{e:#}"));
            -1
        }
    }
}

// ─── Error ────────────────────────────────────────────────────────────────────

/// Return the last error message. Valid until the next call on the same handle.
/// Returns NULL if no error has occurred.
#[unsafe(no_mangle)]
pub extern "C" fn minigraf_last_error(handle: *mut MiniGrafDb) -> *const c_char {
    if handle.is_null() {
        return std::ptr::null();
    }
    let handle = unsafe { &*handle };
    let guard = handle.last_error.lock().unwrap();
    match guard.as_ref() {
        Some(s) => s.as_ptr(),
        None => std::ptr::null(),
    }
}

// ─── JSON helpers ─────────────────────────────────────────────────────────────

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
            let rows: Vec<Vec<serde_json::Value>> =
                results.iter().map(|r| r.iter().map(value_to_json).collect()).collect();
            serde_json::json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_returns_non_null() {
        let db = minigraf_open_in_memory();
        assert!(!db.is_null());
        minigraf_close(db);
    }

    #[test]
    fn execute_transact_returns_json() {
        let db = minigraf_open_in_memory();
        let datalog = CString::new(r#"(transact [[:alice :name "Alice"]])"#).unwrap();
        let result = minigraf_execute(db, datalog.as_ptr());
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(s.contains("transacted"), "expected transacted in: {s}");
        minigraf_string_free(result);
        minigraf_close(db);
    }

    #[test]
    fn execute_query_returns_results() {
        let db = minigraf_open_in_memory();
        let tx = CString::new(r#"(transact [[:alice :name "Alice"]])"#).unwrap();
        let r = minigraf_execute(db, tx.as_ptr());
        assert!(!r.is_null());
        minigraf_string_free(r);

        let q = CString::new("(query [:find ?n :where [?e :name ?n]])").unwrap();
        let result = minigraf_execute(db, q.as_ptr());
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(s.contains("Alice"), "expected Alice in: {s}");
        minigraf_string_free(result);
        minigraf_close(db);
    }

    #[test]
    fn execute_invalid_datalog_returns_null_and_sets_error() {
        let db = minigraf_open_in_memory();
        let bad = CString::new("not valid datalog !!!").unwrap();
        let result = minigraf_execute(db, bad.as_ptr());
        assert!(result.is_null(), "expected NULL for invalid datalog");

        let err = minigraf_last_error(db);
        assert!(!err.is_null(), "expected non-NULL error");
        let msg = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert!(!msg.is_empty(), "expected non-empty error message");
        minigraf_close(db);
    }

    #[test]
    fn checkpoint_returns_zero_on_success() {
        let db = minigraf_open_in_memory();
        let rc = minigraf_checkpoint(db);
        assert_eq!(rc, 0);
        minigraf_close(db);
    }

    #[test]
    fn string_free_null_is_safe() {
        // Should not panic or crash
        minigraf_string_free(std::ptr::null_mut());
    }

    #[test]
    fn close_null_is_safe() {
        minigraf_close(std::ptr::null_mut());
    }
}
