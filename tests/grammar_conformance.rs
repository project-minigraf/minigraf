//! Grammar conformance tests.
//!
//! Three test functions verify that `grammar.pest` and the real parser agree:
//!
//! - `valid_corpus`:            pest ACCEPTS  + parser ACCEPTS
//! - `invalid_syntax_corpus`:   pest REJECTS  + parser REJECTS
//! - `invalid_semantic_corpus`: pest ACCEPTS  + parser REJECTS
//!
//! Run with: cargo test grammar_conformance -- --nocapture

use pest::Parser;
use pest_derive::Parser;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[grammar = "tests/grammar/grammar.pest"]
struct DatalogGrammar;

/// Returns true when the pest grammar accepts the full input as a `command`.
fn pest_accepts(input: &str) -> bool {
    DatalogGrammar::parse(Rule::command, input.trim()).is_ok()
}

/// Returns true when the real Minigraf parser accepts the input.
///
/// Uses `db.prepare()` for `(query ...)` commands so that bind-slot
/// templates parse correctly without needing substituted values at test time.
/// Uses `db.execute()` for transact / retract / rule.
fn parser_accepts(input: &str) -> bool {
    let input = input.trim();
    let Ok(db) = minigraf::Minigraf::in_memory() else {
        return false;
    };
    if input.starts_with("(query") {
        db.prepare(input).is_ok()
    } else {
        db.execute(input).is_ok()
    }
}

/// Read every `.edn` file in `dir`, returning `(filename, content)` pairs
/// sorted by filename for deterministic test output.
fn load_corpus(dir: &str) -> Vec<(String, String)> {
    let path = Path::new(dir);
    let Ok(read_dir) = fs::read_dir(path) else {
        return vec![];
    };
    let mut files: Vec<(String, String)> = read_dir
        .filter_map(|e| {
            let e = e.ok()?;
            let p = e.path();
            if p.extension()?.to_str()? == "edn" {
                let name = p.file_name()?.to_str()?.to_string();
                let content = fs::read_to_string(&p).ok()?;
                Some((name, content))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

// ── VALID ──────────────────────────────────────────────────────────────────

#[test]
#[allow(clippy::panic)]
fn valid_corpus() {
    let files = load_corpus("tests/grammar/valid");
    assert!(
        !files.is_empty(),
        "valid/ corpus is empty — add .edn fixtures"
    );
    let mut failures: Vec<String> = vec![];
    for (name, content) in &files {
        if !pest_accepts(content) {
            failures.push(format!(
                "FAIL valid/{name}: pest rejected (expected accept)"
            ));
        }
        if !parser_accepts(content) {
            failures.push(format!(
                "FAIL valid/{name}: parser rejected (expected accept)"
            ));
        }
    }
    if !failures.is_empty() {
        panic!("\n{}", failures.join("\n"));
    }
}

// ── INVALID SYNTAX ─────────────────────────────────────────────────────────

#[test]
#[allow(clippy::panic)]
fn invalid_syntax_corpus() {
    let files = load_corpus("tests/grammar/invalid/syntax");
    assert!(
        !files.is_empty(),
        "invalid/syntax/ corpus is empty — add .edn fixtures"
    );
    let mut failures: Vec<String> = vec![];
    for (name, content) in &files {
        if pest_accepts(content) {
            failures.push(format!(
                "FAIL invalid/syntax/{name}: pest accepted (expected reject)"
            ));
        }
        if parser_accepts(content) {
            failures.push(format!(
                "FAIL invalid/syntax/{name}: parser accepted (expected reject)"
            ));
        }
    }
    if !failures.is_empty() {
        panic!("\n{}", failures.join("\n"));
    }
}

// ── INVALID SEMANTIC ───────────────────────────────────────────────────────

#[test]
#[allow(clippy::panic)]
fn invalid_semantic_corpus() {
    let files = load_corpus("tests/grammar/invalid/semantic");
    assert!(
        !files.is_empty(),
        "invalid/semantic/ corpus is empty — add .edn fixtures"
    );
    let mut failures: Vec<String> = vec![];
    for (name, content) in &files {
        if !pest_accepts(content) {
            failures.push(format!(
                "FAIL invalid/semantic/{name}: pest rejected (expected accept)"
            ));
        }
        if parser_accepts(content) {
            failures.push(format!(
                "FAIL invalid/semantic/{name}: parser accepted (expected reject)"
            ));
        }
    }
    if !failures.is_empty() {
        panic!("\n{}", failures.join("\n"));
    }
}
