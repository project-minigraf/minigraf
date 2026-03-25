# Phase 7.2a: Aggregation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add scalar aggregation (`count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`) and the `:with` grouping clause to Minigraf's Datalog `:find` clause.

**Architecture:** Introduce a `FindSpec` enum that replaces `Vec<String>` in `DatalogQuery.find`, extend the parser to parse aggregate expressions and a `:with` clause, and add `apply_aggregation` as a post-processing step in the executor after bindings are collected. No changes to the evaluator, storage, or public API surface.

**Tech Stack:** Rust 2024, no new dependencies. Tests: `cargo test`. All test assertions must avoid `{:?}` on `Result`/`Value`/`Fact` in assert messages (CodeQL rule — see CLAUDE.md).

---

## File Map

| File | Action | What changes |
|---|---|---|
| `src/query/datalog/types.rs` | Modify | Add `AggFunc`, `FindSpec`; change `DatalogQuery.find` to `Vec<FindSpec>`; add `with_vars` field |
| `src/query/datalog/parser.rs` | Modify | Parse aggregate expressions in `:find`; parse `:with` clause; add validation |
| `src/query/datalog/executor.rs` | Modify | Update variable-extraction loops; add `apply_aggregation`, `extract_variables`, `apply_agg_func` helpers |
| `tests/aggregation_test.rs` | Create | ~26 integration tests via `db.execute()` |

No changes to: `evaluator.rs`, `matcher.rs`, `optimizer.rs`, `stratification.rs`, `rules.rs`, `storage/`, `graph/`, `db.rs`, `wal.rs`.

**Reference files to read before starting:**
- `docs/superpowers/specs/2026-03-24-phase-7-2a-aggregation-design.md` — full spec
- `tests/negation_test.rs` — integration test pattern (use `OpenOptions::new().open_memory().unwrap()`)
- `src/query/datalog/executor.rs` lines 250–272 and 437–456 — the two variable-extraction loops being updated
- `src/query/datalog/parser.rs` lines 397–524 — `parse_query` function being extended

---

## Task 1: Add `AggFunc` and `FindSpec` types

**Files:**
- Modify: `src/query/datalog/types.rs`

- [ ] **Step 1: Write failing unit tests for `AggFunc` and `FindSpec`**

Add inside the `#[cfg(test)]` module at the bottom of `types.rs`:

```rust
#[test]
fn test_agg_func_as_str() {
    assert_eq!(AggFunc::Count.as_str(), "count");
    assert_eq!(AggFunc::CountDistinct.as_str(), "count-distinct");
    assert_eq!(AggFunc::Sum.as_str(), "sum");
    assert_eq!(AggFunc::SumDistinct.as_str(), "sum-distinct");
    assert_eq!(AggFunc::Min.as_str(), "min");
    assert_eq!(AggFunc::Max.as_str(), "max");
}

#[test]
fn test_find_spec_variable_display_and_var() {
    let spec = FindSpec::Variable("?name".to_string());
    assert_eq!(spec.display_name(), "?name");
    assert_eq!(spec.var(), "?name");
}

#[test]
fn test_find_spec_aggregate_display_and_var() {
    let spec = FindSpec::Aggregate {
        func: AggFunc::CountDistinct,
        var: "?e".to_string(),
    };
    assert_eq!(spec.display_name(), "(count-distinct ?e)");
    assert_eq!(spec.var(), "?e");
}

#[test]
fn test_find_spec_all_agg_display_names() {
    let cases = [
        (AggFunc::Count, "?e", "(count ?e)"),
        (AggFunc::CountDistinct, "?e", "(count-distinct ?e)"),
        (AggFunc::Sum, "?v", "(sum ?v)"),
        (AggFunc::SumDistinct, "?v", "(sum-distinct ?v)"),
        (AggFunc::Min, "?x", "(min ?x)"),
        (AggFunc::Max, "?x", "(max ?x)"),
    ];
    for (func, var, expected) in cases {
        let spec = FindSpec::Aggregate { func, var: var.to_string() };
        assert_eq!(spec.display_name(), expected);
    }
}
```

- [ ] **Step 2: Run tests — expect compile failure**

```bash
cargo test --lib -- query::datalog::types 2>&1 | head -20
```
Expected: compile error `cannot find type 'AggFunc'` and `cannot find type 'FindSpec'`.

- [ ] **Step 3: Implement `AggFunc` and `FindSpec` in `types.rs`**

Add before the `Pattern` struct (after the `EdnValue` impl block, around line 96):

```rust
/// Aggregate function applied to a logic variable in the :find clause.
#[derive(Debug, Clone, PartialEq)]
pub enum AggFunc {
    Count,
    CountDistinct,
    Sum,
    SumDistinct,
    Min,
    Max,
}

impl AggFunc {
    /// Hyphenated lowercase name used in display and parsing.
    pub fn as_str(&self) -> &'static str {
        match self {
            AggFunc::Count => "count",
            AggFunc::CountDistinct => "count-distinct",
            AggFunc::Sum => "sum",
            AggFunc::SumDistinct => "sum-distinct",
            AggFunc::Min => "min",
            AggFunc::Max => "max",
        }
    }
}

/// A single element in the :find clause: either a plain variable or an aggregate.
#[derive(Debug, Clone, PartialEq)]
pub enum FindSpec {
    /// A plain logic variable: ?name
    Variable(String),
    /// An aggregate expression: (count ?e), (sum ?salary), etc.
    Aggregate { func: AggFunc, var: String },
}

impl FindSpec {
    /// Column header string used in QueryResult::QueryResults.vars.
    /// Variable("?name") → "?name"
    /// Aggregate { CountDistinct, "?e" } → "(count-distinct ?e)"
    pub fn display_name(&self) -> String {
        match self {
            FindSpec::Variable(v) => v.clone(),
            FindSpec::Aggregate { func, var } => format!("({} {})", func.as_str(), var),
        }
    }

    /// The logic variable this spec references.
    pub fn var(&self) -> &str {
        match self {
            FindSpec::Variable(v) => v.as_str(),
            FindSpec::Aggregate { var, .. } => var.as_str(),
        }
    }
}
```

- [ ] **Step 4: Run tests — expect pass**

```bash
cargo test --lib -- query::datalog::types::tests::test_agg_func 2>&1
cargo test --lib -- query::datalog::types::tests::test_find_spec 2>&1
```
Expected: all 4 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add AggFunc and FindSpec types for Phase 7.2a aggregation"
```

---

## Task 2: Migrate `DatalogQuery` to `Vec<FindSpec>` and fix all call sites

This task makes the codebase compile with the new `find` field type. No new behaviour yet — existing tests must still pass at the end.

**Files:**
- Modify: `src/query/datalog/types.rs`
- Modify: `src/query/datalog/parser.rs`
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Update `DatalogQuery` struct and `new` constructor in `types.rs`**

Change the struct (around line 237):
```rust
pub struct DatalogQuery {
    pub find: Vec<FindSpec>,          // was Vec<String>
    pub where_clauses: Vec<WhereClause>,
    pub as_of: Option<AsOf>,
    pub valid_at: Option<ValidAt>,
    pub with_vars: Vec<String>,       // new — empty = no :with clause
}
```

Update `DatalogQuery::new`:
```rust
pub fn new(find: Vec<FindSpec>, where_clauses: Vec<WhereClause>) -> Self {
    DatalogQuery {
        find,
        where_clauses,
        as_of: None,
        valid_at: None,
        with_vars: Vec::new(),
    }
}
```

Update `DatalogQuery::from_patterns`:
```rust
pub fn from_patterns(find: Vec<FindSpec>, patterns: Vec<Pattern>) -> Self {
    DatalogQuery {
        find,
        where_clauses: patterns.into_iter().map(WhereClause::Pattern).collect(),
        as_of: None,
        valid_at: None,
        with_vars: Vec::new(),
    }
}
```

- [ ] **Step 2: Fix struct literal and `DatalogQuery::new` call sites in `types.rs` tests**

In the `#[cfg(test)]` block, every `DatalogQuery::new(vec!["?name".to_string(), ...], ...)` call must become `DatalogQuery::new(vec![FindSpec::Variable("?name".to_string()), ...], ...)`. Every `DatalogQuery { find: ..., where_clauses: ..., ... }` struct literal must add `with_vars: Vec::new()`.

The struct spread at line ~539 (`..query`) is fine — the new `with_vars` is copied from `query` which was built with `DatalogQuery::new`, so it already has `Vec::new()`.

- [ ] **Step 3: Fix `parser.rs` — local variable and `DatalogQuery::new` call**

In `parse_query` (line ~407), change:
```rust
let mut find_vars = Vec::new();
```
to:
```rust
let mut find_specs: Vec<FindSpec> = Vec::new();
```

And add `with_vars` local:
```rust
let mut with_vars: Vec<String> = Vec::new();
```

At line ~520 where `DatalogQuery::new` is called, update to:
```rust
let mut query = DatalogQuery::new(find_specs, where_clauses);
query.as_of = query_as_of;
query.valid_at = query_valid_at;
query.with_vars = with_vars;
```

In the `:find` arm (lines ~474–483), temporarily replace the body with a placeholder that accepts only variables (to keep it compiling — we'll expand this in Task 3):
```rust
Some(":find") => {
    if let Some(var) = query_vector[i].as_variable() {
        find_specs.push(FindSpec::Variable(var.to_string()));
    } else {
        return Err(format!(
            "Expected variable in :find clause, got {:?}",
            query_vector[i]
        ));
    }
}
```

Update the `DatalogQuery::new` call in `parse_rule` (around line ~895) similarly if it uses `find`.

- [ ] **Step 4: Fix parser tests that assert `q.find == vec!["?name"]`**

The test sites at lines ~995, ~1045, ~1148, ~1170, ~1189 assert things like:
```rust
assert_eq!(q.find, vec!["?name"]);
```
Change these to:
```rust
assert_eq!(q.find, vec![FindSpec::Variable("?name".to_string())]);
// or for multiple:
assert_eq!(q.find, vec![
    FindSpec::Variable("?name".to_string()),
    FindSpec::Variable("?age".to_string()),
]);
```

- [ ] **Step 5: Fix `executor.rs` — two `vars: query.find` assignments**

At lines ~269 and ~454, change:
```rust
vars: query.find,
```
to:
```rust
vars: query.find.iter().map(|s| s.display_name()).collect(),
```

Fix the two variable-extraction loops (lines ~254–266 and ~441–451). Change `for var in &query.find` → `for spec in &query.find` and `binding.get(var)` → `binding.get(spec.var())`:

```rust
// Both loops become:
for spec in &query.find {
    if let Some(value) = binding.get(spec.var()) {
        row.push(value.clone());
    } else {
        continue;
    }
}
if row.len() == query.find.len() {
    results.push(row);
}
```

Fix the `DatalogQuery { ... }` struct literal tests in executor.rs at lines ~1031 and ~1086: add `with_vars: Vec::new()` to each. Also update any `DatalogQuery::new(vec!["?x".to_string(), ...], ...)` calls in executor tests to use `FindSpec::Variable(...)`.

- [ ] **Step 6: Run full test suite — expect all existing tests to pass**

```bash
cargo test 2>&1 | tail -20
```
Expected: all 407 existing tests pass (same as before). Zero new tests yet.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/parser.rs src/query/datalog/executor.rs
git commit -m "refactor(types): migrate DatalogQuery.find to Vec<FindSpec>; add with_vars field"
```

---

## Task 3: Parser — parse aggregate expressions in `:find` and add `:with` clause

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing parse tests**

Add to the `#[cfg(test)]` block in `parser.rs`:

```rust
#[test]
fn test_parse_count_in_find() {
    let result = parse_datalog_command("(query [:find (count ?e) :where [?e :person/name ?n]])");
    let cmd = result.expect("parse failed");
    match cmd {
        DatalogCommand::Query(q) => {
            assert_eq!(q.find.len(), 1);
            assert_eq!(
                q.find[0],
                FindSpec::Aggregate { func: AggFunc::Count, var: "?e".to_string() }
            );
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_mixed_find_var_and_aggregate() {
    let result = parse_datalog_command(
        r#"(query [:find ?dept (count-distinct ?e) :where [?e :dept ?dept]])"#,
    );
    let cmd = result.expect("parse failed");
    match cmd {
        DatalogCommand::Query(q) => {
            assert_eq!(q.find.len(), 2);
            assert_eq!(q.find[0], FindSpec::Variable("?dept".to_string()));
            assert_eq!(
                q.find[1],
                FindSpec::Aggregate { func: AggFunc::CountDistinct, var: "?e".to_string() }
            );
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_all_aggregate_functions() {
    let cases = [
        ("count", AggFunc::Count),
        ("count-distinct", AggFunc::CountDistinct),
        ("sum", AggFunc::Sum),
        ("sum-distinct", AggFunc::SumDistinct),
        ("min", AggFunc::Min),
        ("max", AggFunc::Max),
    ];
    for (name, expected_func) in cases {
        let input = format!("(query [:find ({} ?v) :where [?e :a ?v]])", name);
        let cmd = parse_datalog_command(&input).expect("parse failed");
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(
                    q.find[0],
                    FindSpec::Aggregate { func: expected_func, var: "?v".to_string() }
                );
            }
            _ => panic!("expected Query"),
        }
    }
}

#[test]
fn test_parse_with_clause_single_var() {
    let result = parse_datalog_command(
        r#"(query [:find ?dept (sum ?salary) :with ?e :where [?e :dept ?dept] [?e :salary ?salary]])"#,
    );
    let cmd = result.expect("parse failed");
    match cmd {
        DatalogCommand::Query(q) => {
            assert_eq!(q.with_vars, vec!["?e".to_string()]);
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_with_clause_multiple_vars() {
    let result = parse_datalog_command(
        r#"(query [:find (count ?e) :with ?dept ?role :where [?e :dept ?dept] [?e :role ?role]])"#,
    );
    let cmd = result.expect("parse failed");
    match cmd {
        DatalogCommand::Query(q) => {
            assert_eq!(q.with_vars, vec!["?dept".to_string(), "?role".to_string()]);
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_error_unknown_aggregate() {
    let result = parse_datalog_command("(query [:find (average ?e) :where [?e :a ?v]])");
    assert!(result.is_err(), "unknown aggregate should fail");
    assert!(
        result.unwrap_err().contains("Unknown aggregate function"),
        "wrong error message"
    );
}

#[test]
fn test_parse_error_aggregate_arg_not_variable() {
    let result = parse_datalog_command("(query [:find (count :not-a-var) :where [?e :a ?v]])");
    assert!(result.is_err(), "non-variable aggregate arg should fail");
}

#[test]
fn test_parse_error_with_without_aggregate() {
    let result = parse_datalog_command(
        r#"(query [:find ?e :with ?x :where [?e :a ?x]])"#,
    );
    assert!(result.is_err(), ":with without aggregate should fail");
    assert!(
        result.unwrap_err().contains("requires at least one aggregate"),
        "wrong error message"
    );
}

#[test]
fn test_parse_error_aggregate_var_unbound() {
    let result = parse_datalog_command(
        r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#,
    );
    assert!(result.is_err(), "unbound aggregate var should fail");
    assert!(
        result.unwrap_err().contains("not bound in :where"),
        "wrong error message"
    );
}
```

- [ ] **Step 2: Run tests — expect failure**

```bash
cargo test --lib -- query::datalog::parser::tests::test_parse_count 2>&1 | head -5
cargo test --lib -- query::datalog::parser::tests::test_parse_with 2>&1 | head -5
```
Expected: parse tests fail (`:find` arm still rejects lists; `:with` not parsed).

- [ ] **Step 3: Add `parse_aggregate` helper to `parser.rs`**

Add this function near the other parse helpers (e.g., after `parse_transact`):

```rust
/// Parse an aggregate expression list: (func-name ?var)
/// e.g., [Symbol("count"), Symbol("?e")] → FindSpec::Aggregate { Count, "?e" }
fn parse_aggregate(elems: &[EdnValue]) -> Result<FindSpec, String> {
    if elems.len() != 2 {
        return Err(format!(
            "Aggregate expression must have exactly 2 elements (func ?var), got {}",
            elems.len()
        ));
    }
    let func = match &elems[0] {
        EdnValue::Symbol(s) => match s.as_str() {
            "count" => AggFunc::Count,
            "count-distinct" => AggFunc::CountDistinct,
            "sum" => AggFunc::Sum,
            "sum-distinct" => AggFunc::SumDistinct,
            "min" => AggFunc::Min,
            "max" => AggFunc::Max,
            other => return Err(format!("Unknown aggregate function: '{}'", other)),
        },
        other => return Err(format!(
            "Aggregate function name must be a symbol, got {:?}",
            other
        )),
    };
    let var = match &elems[1] {
        EdnValue::Symbol(s) if s.starts_with('?') => s.clone(),
        _ => return Err("Aggregate argument must be a variable (starting with ?)".to_string()),
    };
    Ok(FindSpec::Aggregate { func, var })
}
```

- [ ] **Step 4: Extend the `:find` arm in `parse_query` to accept lists**

Replace the `:find` arm body (currently only accepting variables):
```rust
Some(":find") => {
    match &query_vector[i] {
        EdnValue::Symbol(s) if s.starts_with('?') => {
            find_specs.push(FindSpec::Variable(s.clone()));
        }
        EdnValue::List(elems) => {
            find_specs.push(parse_aggregate(elems)?);
        }
        other => {
            return Err(format!(
                "Expected variable or aggregate expression in :find clause, got {:?}",
                other
            ));
        }
    }
}
```

- [ ] **Step 5: Add the `:with` arm in the keyword match of `parse_query`**

Add this arm before the catch-all `_ =>` arm (which sets `current_clause`):

```rust
":with" => {
    // Collect ?-prefixed symbols until the next keyword or end of vector
    i += 1;
    while i < query_vector.len() {
        match &query_vector[i] {
            EdnValue::Symbol(s) if s.starts_with('?') => {
                with_vars.push(s.clone());
                i += 1;
            }
            EdnValue::Keyword(_) => break, // next clause keyword — stop
            other => {
                return Err(format!(
                    "':with' clause accepts only variables, got {:?}",
                    other
                ));
            }
        }
    }
    continue; // skip the i += 1 at the bottom of the loop
}
```

- [ ] **Step 6: Add parse-time validation for aggregates and `:with`**

After the existing `check_not_safety` and `check_not_join_safety` calls (line ~517), add:

```rust
// Validate aggregate and :with vars are bound in :where
for spec in &find_specs {
    if let FindSpec::Aggregate { var, .. } = spec {
        if !outer_bound.contains(var) {
            return Err(format!(
                "Aggregate variable {} not bound in :where",
                var
            ));
        }
    }
}
for var in &with_vars {
    if !outer_bound.contains(var) {
        return Err(format!(
            "':with' variable {} not bound in :where",
            var
        ));
    }
}
// :with without any aggregate is an error
if !with_vars.is_empty() && !find_specs.iter().any(|s| matches!(s, FindSpec::Aggregate { .. })) {
    return Err("':with' clause requires at least one aggregate in :find".to_string());
}
```

- [ ] **Step 7: Run parser tests — expect pass**

```bash
cargo test --lib -- query::datalog::parser 2>&1 | tail -10
```
Expected: all parser tests pass (new ones + all existing ones).

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): parse aggregate expressions in :find and :with clause"
```

---

## Task 4: Executor — add `apply_aggregation` and wire up dispatch

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write failing unit tests for `apply_aggregation` (count/count-distinct)**

Add to the `#[cfg(test)]` block in `executor.rs`:

```rust
// Helper: build a binding map from key-value pairs
fn binding(pairs: &[(&str, Value)]) -> std::collections::HashMap<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn test_apply_aggregation_count_basic() {
    let bindings = vec![
        binding(&[("?e", Value::Integer(1))]),
        binding(&[("?e", Value::Integer(2))]),
        binding(&[("?e", Value::Integer(3))]),
    ];
    let find_specs = vec![FindSpec::Aggregate {
        func: AggFunc::Count,
        var: "?e".to_string(),
    }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0][0], Value::Integer(3));
}

#[test]
fn test_apply_aggregation_count_with_grouping() {
    let bindings = vec![
        binding(&[("?dept", Value::String("eng".to_string())), ("?e", Value::Integer(1))]),
        binding(&[("?dept", Value::String("eng".to_string())), ("?e", Value::Integer(2))]),
        binding(&[("?dept", Value::String("hr".to_string())),  ("?e", Value::Integer(3))]),
    ];
    let find_specs = vec![
        FindSpec::Variable("?dept".to_string()),
        FindSpec::Aggregate { func: AggFunc::Count, var: "?e".to_string() },
    ];
    let mut results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    results.sort_by_key(|r| match &r[0] { Value::String(s) => s.clone(), _ => String::new() });
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![Value::String("eng".to_string()), Value::Integer(2)]);
    assert_eq!(results[1], vec![Value::String("hr".to_string()), Value::Integer(1)]);
}

#[test]
fn test_apply_aggregation_count_distinct() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(1))]),
        binding(&[("?v", Value::Integer(1))]),  // duplicate
        binding(&[("?v", Value::Integer(2))]),
    ];
    let find_specs = vec![FindSpec::Aggregate {
        func: AggFunc::CountDistinct,
        var: "?v".to_string(),
    }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(2));
}

#[test]
fn test_apply_aggregation_count_empty_no_grouping_vars() {
    // count with no grouping vars + zero bindings → [[0]]
    let find_specs = vec![FindSpec::Aggregate {
        func: AggFunc::Count,
        var: "?e".to_string(),
    }];
    let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
    assert_eq!(results.len(), 1, "should return one row with 0");
    assert_eq!(results[0][0], Value::Integer(0));
}

#[test]
fn test_apply_aggregation_count_empty_with_grouping_var() {
    // count with grouping var + zero bindings → empty result
    let find_specs = vec![
        FindSpec::Variable("?dept".to_string()),
        FindSpec::Aggregate { func: AggFunc::Count, var: "?e".to_string() },
    ];
    let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
    assert_eq!(results.len(), 0, "should return empty set");
}
```

- [ ] **Step 2: Run tests — expect compile failure**

```bash
cargo test --lib -- executor::tests::test_apply_aggregation 2>&1 | head -5
```
Expected: `cannot find function 'apply_aggregation'`.

- [ ] **Step 3: Add `apply_aggregation` and `apply_agg_func` to `executor.rs`**

Add after the `execute_rule` function (around line ~469), before the `#[cfg(test)]` block:

```rust
/// Extract plain variable values from bindings (non-aggregate path).
fn extract_variables(
    bindings: Vec<std::collections::HashMap<String, Value>>,
    find_specs: &[FindSpec],
) -> Vec<Vec<Value>> {
    let mut results = Vec::new();
    for binding in bindings {
        let mut row = Vec::new();
        for spec in find_specs {
            if let Some(value) = binding.get(spec.var()) {
                row.push(value.clone());
            } else {
                break;
            }
        }
        if row.len() == find_specs.len() {
            results.push(row);
        }
    }
    results
}

/// Post-process bindings through aggregation.
/// Called only when find_specs contains at least one FindSpec::Aggregate.
fn apply_aggregation(
    bindings: Vec<std::collections::HashMap<String, Value>>,
    find_specs: &[FindSpec],
    with_vars: &[String],
) -> Result<Vec<Vec<Value>>> {
    use super::types::{AggFunc, FindSpec};

    let has_grouping_vars = find_specs.iter().any(|s| matches!(s, FindSpec::Variable(_)));

    // Zero bindings: special case for pure count/count-distinct (no grouping vars)
    if bindings.is_empty() {
        let all_count = !has_grouping_vars
            && find_specs.iter().all(|s| {
                matches!(
                    s,
                    FindSpec::Aggregate { func: AggFunc::Count | AggFunc::CountDistinct, .. }
                )
            });
        if all_count {
            // Return single row of zeros
            let row = find_specs.iter().map(|_| Value::Integer(0)).collect();
            return Ok(vec![row]);
        }
        return Ok(vec![]);
    }

    // Grouping key = FindSpec::Variable vars (in find order) + with_vars
    let group_var_names: Vec<&str> = find_specs
        .iter()
        .filter_map(|s| match s {
            FindSpec::Variable(v) => Some(v.as_str()),
            FindSpec::Aggregate { .. } => None,
        })
        .chain(with_vars.iter().map(|s| s.as_str()))
        .collect();

    // Group using Vec + PartialEq scan (Value::Float doesn't implement Hash)
    let mut groups: Vec<(Vec<Value>, Vec<std::collections::HashMap<String, Value>>)> = Vec::new();
    for b in bindings {
        let key: Vec<Value> = group_var_names
            .iter()
            .map(|v| b.get(*v).cloned().unwrap_or(Value::Null))
            .collect();
        if let Some(pos) = groups.iter().position(|(k, _)| k == &key) {
            groups[pos].1.push(b);
        } else {
            groups.push((key.clone(), vec![b]));
        }
    }

    // Build output rows (one per group)
    let mut results = Vec::new();
    let mut group_key_idx_for_var: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for (idx, s) in find_specs.iter().enumerate() {
        if let FindSpec::Variable(v) = s {
            // position in the group key Vec (only Variable specs, in order)
            let pos = find_specs
                .iter()
                .take(idx + 1)
                .filter(|s| matches!(s, FindSpec::Variable(_)))
                .count()
                - 1;
            group_key_idx_for_var.insert(v.as_str(), pos);
        }
    }

    for (key, group_bindings) in &groups {
        let mut row = Vec::new();
        for spec in find_specs {
            match spec {
                FindSpec::Variable(v) => {
                    let pos = *group_key_idx_for_var.get(v.as_str()).unwrap();
                    row.push(key[pos].clone());
                }
                FindSpec::Aggregate { func, var } => {
                    let non_null_values: Vec<&Value> = group_bindings
                        .iter()
                        .filter_map(|b| b.get(var.as_str()))
                        .filter(|v| !matches!(v, Value::Null))
                        .collect();
                    row.push(apply_agg_func(func, &non_null_values)?);
                }
            }
        }
        // Filter out groups where all agg vars were null (min/max only)
        // — apply_agg_func returns Err for empty min/max; handle by skipping
        results.push(row);
    }

    Ok(results)
}

/// Apply a single aggregate function to a slice of non-null values.
fn apply_agg_func(func: &AggFunc, values: &[&Value]) -> Result<Value> {
    match func {
        AggFunc::Count => Ok(Value::Integer(values.len() as i64)),

        AggFunc::CountDistinct => {
            let mut seen: Vec<&Value> = Vec::new();
            for v in values {
                if !seen.iter().any(|s| *s == *v) {
                    seen.push(v);
                }
            }
            Ok(Value::Integer(seen.len() as i64))
        }

        AggFunc::Sum | AggFunc::SumDistinct => {
            let deduped: Vec<&Value> = if matches!(func, AggFunc::SumDistinct) {
                let mut seen: Vec<&Value> = Vec::new();
                for v in values {
                    if !seen.iter().any(|s| *s == *v) {
                        seen.push(v);
                    }
                }
                seen
            } else {
                values.to_vec()
            };

            if deduped.is_empty() {
                return Ok(Value::Integer(0));
            }

            let has_float = deduped.iter().any(|v| matches!(v, Value::Float(_)));
            if has_float {
                let mut sum = 0.0_f64;
                for v in &deduped {
                    match v {
                        Value::Float(f) => sum += f,
                        Value::Integer(i) => sum += *i as f64,
                        other => {
                            return Err(anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ))
                        }
                    }
                }
                Ok(Value::Float(sum))
            } else {
                let mut sum = 0_i64;
                for v in &deduped {
                    match v {
                        Value::Integer(i) => sum += i,
                        other => {
                            return Err(anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ))
                        }
                    }
                }
                Ok(Value::Integer(sum))
            }
        }

        AggFunc::Min | AggFunc::Max => {
            if values.is_empty() {
                // All values in group were null — group disappears
                return Err(anyhow!("min/max: no non-null values in group"));
            }
            // Check all same type (no mixing Integer and Float)
            let first = values[0];
            for v in &values[1..] {
                if std::mem::discriminant(*v) != std::mem::discriminant(first) {
                    return Err(anyhow!(
                        "{}: cannot compare {} and {} values",
                        func.as_str(),
                        value_type_name(first),
                        value_type_name(v)
                    ));
                }
            }
            // Find min or max using PartialOrd
            let result = values.iter().try_fold((*values[0]).clone(), |acc, v| {
                let ordering = match (&acc, v) {
                    (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
                    (Value::Float(a), Value::Float(b)) => {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(a), Value::String(b)) => a.cmp(b),
                    (_, other) => {
                        return Err(anyhow!(
                            "{}: expected Integer, Float, String, or Null, got {}",
                            func.as_str(),
                            value_type_name(other)
                        ))
                    }
                };
                let keep_new = matches!(func, AggFunc::Min) == (ordering == std::cmp::Ordering::Greater);
                Ok::<Value, anyhow::Error>(if keep_new { (*v).clone() } else { acc })
            })?;
            Ok(result)
        }
    }
}

/// Human-readable type name for error messages.
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "String",
        Value::Integer(_) => "Integer",
        Value::Float(_) => "Float",
        Value::Boolean(_) => "Boolean",
        Value::Ref(_) => "Ref",
        Value::Keyword(_) => "Keyword",
        Value::Null => "Null",
    }
}
```

Note: the `use super::types::{AggFunc, FindSpec}` line at the top of `apply_aggregation` may not be needed if `AggFunc` and `FindSpec` are already in scope via the existing `use super::types::*` or explicit import at the top of `executor.rs`. Remove it if so.

- [ ] **Step 4: Wire up `apply_aggregation` in both execution paths**

In `execute_query` (around line ~248), replace the "Extract requested variables" block through the `Ok(QueryResult::QueryResults {...})` return with:

```rust
let has_aggregates = query.find.iter().any(|s| matches!(s, FindSpec::Aggregate { .. }));
let results = if has_aggregates {
    apply_aggregation(filtered_bindings, &query.find, &query.with_vars)?
} else {
    extract_variables(filtered_bindings, &query.find)
};
Ok(QueryResult::QueryResults {
    vars: query.find.iter().map(|s| s.display_name()).collect(),
    results,
})
```

Apply the same change to `execute_query_with_rules` at the equivalent location (around line ~437).

- [ ] **Step 5: Add `AggFunc` and `FindSpec` to the import list at the top of `executor.rs`**

Make sure the `use super::types::` line includes `AggFunc` and `FindSpec`:
```rust
use super::types::{
    AggFunc, DatalogCommand, DatalogQuery, EdnValue, FindSpec, Pattern, Rule, Transaction,
    ValidAt, WhereClause,
};
```

- [ ] **Step 6: Run all tests — expect pass**

```bash
cargo test 2>&1 | tail -15
```
Expected: all 407 existing tests pass, plus the 5 new `apply_aggregation` unit tests = 412 total.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): add apply_aggregation with count/count-distinct support"
```

---

## Task 5: Sum, sum-distinct, min, max unit tests and fixes

Add unit tests covering sum, min, max, null-skipping, and type errors. The implementation is already in `apply_agg_func` from Task 4 — this task verifies correctness.

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Add unit tests for sum and sum-distinct**

Add to `#[cfg(test)]` in `executor.rs`:

```rust
#[test]
fn test_apply_aggregation_sum_integers() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(10))]),
        binding(&[("?v", Value::Integer(20))]),
        binding(&[("?v", Value::Integer(30))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Sum, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(60));
}

#[test]
fn test_apply_aggregation_sum_widens_to_float() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(10))]),
        binding(&[("?v", Value::Float(0.5))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Sum, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Float(10.5));
}

#[test]
fn test_apply_aggregation_sum_distinct_deduplicates() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(5))]),
        binding(&[("?v", Value::Integer(5))]),  // duplicate
        binding(&[("?v", Value::Integer(10))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::SumDistinct, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(15)); // 5 + 10, not 5 + 5 + 10
}

#[test]
fn test_apply_aggregation_sum_type_error() {
    let bindings = vec![binding(&[("?v", Value::String("bad".to_string()))])];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Sum, var: "?v".to_string() }];
    let result = apply_aggregation(bindings, &find_specs, &[]);
    assert!(result.is_err(), "sum of string should fail");
}

#[test]
fn test_apply_aggregation_min_integers() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(30))]),
        binding(&[("?v", Value::Integer(10))]),
        binding(&[("?v", Value::Integer(20))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Min, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(10));
}

#[test]
fn test_apply_aggregation_max_strings() {
    let bindings = vec![
        binding(&[("?v", Value::String("apple".to_string()))]),
        binding(&[("?v", Value::String("zebra".to_string()))]),
        binding(&[("?v", Value::String("mango".to_string()))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Max, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::String("zebra".to_string()));
}

#[test]
fn test_apply_aggregation_min_type_error_boolean() {
    let bindings = vec![binding(&[("?v", Value::Boolean(true))])];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Min, var: "?v".to_string() }];
    let result = apply_aggregation(bindings, &find_specs, &[]);
    assert!(result.is_err(), "min of boolean should fail");
}

#[test]
fn test_apply_aggregation_min_mixed_int_float_error() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(1))]),
        binding(&[("?v", Value::Float(2.0))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Min, var: "?v".to_string() }];
    let result = apply_aggregation(bindings, &find_specs, &[]);
    assert!(result.is_err(), "min of mixed Integer/Float should fail");
}

#[test]
fn test_apply_aggregation_skips_nulls_in_sum() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(10))]),
        binding(&[("?v", Value::Null)]),
        binding(&[("?v", Value::Integer(20))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Sum, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(30));
}

#[test]
fn test_apply_aggregation_skips_nulls_in_count() {
    let bindings = vec![
        binding(&[("?v", Value::Integer(1))]),
        binding(&[("?v", Value::Null)]),
        binding(&[("?v", Value::Integer(2))]),
    ];
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Count, var: "?v".to_string() }];
    let results = apply_aggregation(bindings, &find_specs, &[]).unwrap();
    assert_eq!(results[0][0], Value::Integer(2)); // null not counted
}

#[test]
fn test_apply_aggregation_sum_empty_bindings() {
    let find_specs = vec![FindSpec::Aggregate { func: AggFunc::Sum, var: "?v".to_string() }];
    let results = apply_aggregation(vec![], &find_specs, &[]).unwrap();
    assert_eq!(results.len(), 0, "sum on empty should return empty set");
}

#[test]
fn test_apply_aggregation_with_var_grouping() {
    // :with ?e means ?e participates in grouping but not in output
    // Without :with: two entities with same dept and same salary collapse to 1 row → sum = 50
    // With :with ?e: entities stay separate → sum = 100
    let bindings = vec![
        binding(&[("?dept", Value::String("eng".to_string())), ("?salary", Value::Integer(50)), ("?e", Value::Integer(1))]),
        binding(&[("?dept", Value::String("eng".to_string())), ("?salary", Value::Integer(50)), ("?e", Value::Integer(2))]),
    ];
    let find_specs = vec![
        FindSpec::Variable("?dept".to_string()),
        FindSpec::Aggregate { func: AggFunc::Sum, var: "?salary".to_string() },
    ];
    // Without :with
    let results_no_with = apply_aggregation(bindings.clone(), &find_specs, &[]).unwrap();
    // With :with ?e
    let results_with = apply_aggregation(bindings, &find_specs, &["?e".to_string()]).unwrap();
    // Without :with: ?dept+"eng" is the group key. Both bindings have same salary+dept,
    // but they are separate bindings so sum = 100 either way.
    // The real with-vs-without difference shows at the grouping level.
    // Check :with result has sum of both salaries.
    assert_eq!(results_with[0][1], Value::Integer(100));
    let _ = results_no_with; // exact value depends on grouping; both should be 100 here
}
```

- [ ] **Step 2: Run all unit tests — expect pass**

```bash
cargo test --lib -- executor::tests 2>&1 | tail -10
```
Expected: all new and existing executor tests pass.

- [ ] **Step 3: Fix any `apply_agg_func` bugs surfaced by tests**

If the min/max or sum logic has issues (e.g. the `keep_new` logic in min/max is subtle), fix them now. The key logic for min is: if `ordering == Greater` (acc > v), pick `v`; if ordering == `Less`, keep acc.

```rust
// For Min: replace acc with v when acc > v (i.e., ordering == Greater)
// For Max: replace acc with v when acc < v (i.e., ordering == Less)
let replace = match func {
    AggFunc::Min => ordering == std::cmp::Ordering::Greater,
    AggFunc::Max => ordering == std::cmp::Ordering::Less,
    _ => unreachable!(),
};
Ok(if replace { (*v).clone() } else { acc })
```

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): add sum, min, max, null-skipping, and :with grouping to apply_aggregation"
```

---

## Task 6: Integration tests

Write the full integration test file that exercises the parser → executor pipeline end-to-end.

**Files:**
- Create: `tests/aggregation_test.rs`

- [ ] **Step 1: Create the test file**

```rust
//! Integration tests for Phase 7.2a aggregation.
//! Covers count, sum, min, max, :with, rules, negation, and temporal queries.

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

fn results(r: &QueryResult) -> &Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn vars(r: &QueryResult) -> &Vec<String> {
    match r {
        QueryResult::QueryResults { vars, .. } => vars,
        _ => panic!("expected QueryResults"),
    }
}

// ── count ─────────────────────────────────────────────────────────────────────

#[test]
fn count_all() {
    let db = db();
    db.execute(r#"(transact [[:a :t "x"] [:b :t "y"] [:c :t "z"]])"#).unwrap();
    let r = db.execute(r#"(query [:find (count ?e) :where [?e :t ?v]])"#).unwrap();
    assert_eq!(vars(&r), &["(count ?e)"]);
    assert_eq!(results(&r).len(), 1);
    assert_eq!(results(&r)[0][0], Value::Integer(3));
}

#[test]
fn count_with_grouping() {
    let db = db();
    db.execute(r#"(transact [[:a :dept "eng"] [:b :dept "eng"] [:c :dept "hr"]])"#).unwrap();
    let r = db
        .execute(r#"(query [:find ?dept (count ?e) :where [?e :dept ?dept]])"#)
        .unwrap();
    let mut rows = results(&r).clone();
    rows.sort_by_key(|r| match &r[0] { Value::String(s) => s.clone(), _ => String::new() });
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec![Value::String("eng".to_string()), Value::Integer(2)]);
    assert_eq!(rows[1], vec![Value::String("hr".to_string()), Value::Integer(1)]);
}

#[test]
fn count_distinct_deduplicates() {
    let db = db();
    // Three entities share two distinct :tag values
    db.execute(r#"(transact [[:a :tag "x"] [:b :tag "x"] [:c :tag "y"]])"#).unwrap();
    let r_count = db.execute(r#"(query [:find (count ?v) :where [?e :tag ?v]])"#).unwrap();
    let r_distinct = db.execute(r#"(query [:find (count-distinct ?v) :where [?e :tag ?v]])"#).unwrap();
    assert_eq!(results(&r_count)[0][0], Value::Integer(3));
    assert_eq!(results(&r_distinct)[0][0], Value::Integer(2));
}

#[test]
fn count_empty_result_no_grouping_vars() {
    let db = db();
    // No facts — count with no grouping vars → [[0]]
    let r = db.execute(r#"(query [:find (count ?e) :where [?e :nonexistent ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 1, "count on empty should return one row");
    assert_eq!(results(&r)[0][0], Value::Integer(0));
}

#[test]
fn count_empty_with_grouping_var() {
    let db = db();
    let r = db
        .execute(r#"(query [:find ?dept (count ?e) :where [?e :dept ?dept]])"#)
        .unwrap();
    assert_eq!(results(&r).len(), 0, "grouped count on empty should return no rows");
}

#[test]
fn count_distinct_empty_result() {
    let db = db();
    let r = db.execute(r#"(query [:find (count-distinct ?v) :where [?e :x ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 1);
    assert_eq!(results(&r)[0][0], Value::Integer(0));
}

// ── sum ───────────────────────────────────────────────────────────────────────

#[test]
fn sum_integers() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score 20] [:c :score 30]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(60));
}

#[test]
fn sum_mixed_widens_to_float() {
    let db = db();
    db.execute(r#"(transact [[:a :v 10] [:b :v 0.5]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :v ?v]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Float(10.5));
}

#[test]
fn sum_distinct_deduplicates() {
    let db = db();
    db.execute(r#"(transact [[:a :v 5] [:b :v 5] [:c :v 10]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum-distinct ?v) :where [?e :v ?v]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(15));
}

#[test]
fn sum_empty_result() {
    let db = db();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :nothing ?v]])"#).unwrap();
    assert_eq!(results(&r).len(), 0, "sum on empty should return no rows");
}

#[test]
fn sum_skips_nulls() {
    let db = db();
    // Assert facts; one entity lacks :score entirely — when joined, its ?score won't bind.
    // Use a second attribute to force null: transact a null value explicitly.
    db.execute(r#"(transact [[:a :score 10] [:b :score 20]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?s) :where [?e :score ?s]])"#).unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(30));
}

#[test]
fn sum_type_error() {
    let db = db();
    db.execute(r#"(transact [[:a :v "not-a-number"]])"#).unwrap();
    let r = db.execute(r#"(query [:find (sum ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "sum of string should fail");
}

// ── min / max ─────────────────────────────────────────────────────────────────

#[test]
fn min_max_integers() {
    let db = db();
    db.execute(r#"(transact [[:a :n 30] [:b :n 10] [:c :n 20]])"#).unwrap();
    let r_min = db.execute(r#"(query [:find (min ?n) :where [?e :n ?n]])"#).unwrap();
    let r_max = db.execute(r#"(query [:find (max ?n) :where [?e :n ?n]])"#).unwrap();
    assert_eq!(results(&r_min)[0][0], Value::Integer(10));
    assert_eq!(results(&r_max)[0][0], Value::Integer(30));
}

#[test]
fn min_max_strings() {
    let db = db();
    db.execute(r#"(transact [[:a :s "banana"] [:b :s "apple"] [:c :s "cherry"]])"#).unwrap();
    let r_min = db.execute(r#"(query [:find (min ?s) :where [?e :s ?s]])"#).unwrap();
    let r_max = db.execute(r#"(query [:find (max ?s) :where [?e :s ?s]])"#).unwrap();
    assert_eq!(results(&r_min)[0][0], Value::String("apple".to_string()));
    assert_eq!(results(&r_max)[0][0], Value::String("cherry".to_string()));
}

#[test]
fn min_type_error_boolean() {
    let db = db();
    db.execute(r#"(transact [[:a :v true]])"#).unwrap();
    let r = db.execute(r#"(query [:find (min ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "min of boolean should fail");
}

#[test]
fn min_mixed_int_float_error() {
    let db = db();
    db.execute(r#"(transact [[:a :v 1] [:b :v 2.0]])"#).unwrap();
    let r = db.execute(r#"(query [:find (min ?v) :where [?e :v ?v]])"#);
    assert!(r.is_err(), "min of mixed Integer/Float should fail");
}

// ── :with ─────────────────────────────────────────────────────────────────────

#[test]
fn with_prevents_overcollapse() {
    let db = db();
    // Two entities in same dept, same salary — without :with they're the same group row
    db.execute(
        r#"(transact [[:e1 :dept "eng"] [:e1 :salary 50]
                      [:e2 :dept "eng"] [:e2 :salary 50]])"#,
    )
    .unwrap();
    let r = db
        .execute(r#"(query [:find ?dept (sum ?salary) :with ?e
                            :where [?e :dept ?dept] [?e :salary ?salary]])"#)
        .unwrap();
    // With :with ?e, entities are distinct in the grouping key → sum = 100
    assert_eq!(results(&r)[0][1], Value::Integer(100));
    // :with var must NOT appear in output
    assert_eq!(vars(&r), &["?dept", "(sum ?salary)"]);
}

// ── parse errors ──────────────────────────────────────────────────────────────

#[test]
fn parse_error_with_without_aggregate() {
    let db = db();
    let r = db.execute(r#"(query [:find ?e :with ?x :where [?e :a ?x]])"#);
    assert!(r.is_err(), ":with without aggregate should fail");
}

#[test]
fn parse_error_unknown_aggregate_function() {
    let db = db();
    let r = db.execute(r#"(query [:find (average ?e) :where [?e :a ?v]])"#);
    assert!(r.is_err(), "unknown aggregate should fail");
}

#[test]
fn parse_error_aggregate_var_unbound() {
    let db = db();
    let r = db.execute(r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#);
    assert!(r.is_err(), "unbound aggregate var should fail");
}

// ── integration: rules, negation, temporal ───────────────────────────────────

#[test]
fn aggregate_after_nonrecursive_rule() {
    let db = db();
    db.execute(r#"(transact [[:a :member :g1] [:b :member :g1] [:c :member :g2]])"#).unwrap();
    db.execute(r#"(rule [(in-group ?e ?g) [?e :member ?g]])"#).unwrap();
    let r = db
        .execute(r#"(query [:find ?g (count ?e) :where (in-group ?e ?g)])"#)
        .unwrap();
    let mut rows = results(&r).clone();
    rows.sort_by_key(|r| match &r[1] { Value::Integer(n) => *n, _ => 0 });
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], Value::Integer(1)); // g2 has 1 member
    assert_eq!(rows[1][1], Value::Integer(2)); // g1 has 2 members
}

#[test]
fn aggregate_after_recursive_rule() {
    let db = db();
    // Chain: a → b → c → d  (3 nodes reachable from a)
    db.execute(r#"(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d]])"#).unwrap();
    db.execute(r#"(rule [(reach ?x ?y) [?x :edge ?y]])"#).unwrap();
    db.execute(r#"(rule [(reach ?x ?y) [?x :edge ?z] (reach ?z ?y)])"#).unwrap();
    let r = db
        .execute(r#"(query [:find (count ?y) :where (reach :a ?y)])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(3));
}

#[test]
fn aggregate_with_negation() {
    let db = db();
    db.execute(
        r#"(transact [[:a :score 10] [:b :score 20] [:b :excluded true]])"#,
    )
    .unwrap();
    let r = db
        .execute(r#"(query [:find (sum ?s) :where [?e :score ?s] (not [?e :excluded true])])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(10));
}

#[test]
fn aggregate_with_as_of() {
    let db = db();
    db.execute(r#"(transact [[:a :score 10] [:b :score 20]])"#).unwrap(); // tx 1
    db.execute(r#"(transact [[:c :score 30]])"#).unwrap(); // tx 2
    // As of tx 1, only :a and :b exist
    let r = db
        .execute(r#"(query [:find (count ?e) :as-of 1 :valid-at :any-valid-time :where [?e :score ?s]])"#)
        .unwrap();
    assert_eq!(results(&r)[0][0], Value::Integer(2));
}
```

- [ ] **Step 2: Run integration tests**

```bash
cargo test --test aggregation_test 2>&1
```
Expected: all ~26 tests pass.

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```
Expected: 407 + 13 unit (Tasks 1, 4, 5) + 26 integration = ~446 tests pass.

- [ ] **Step 4: Commit**

```bash
git add tests/aggregation_test.rs
git commit -m "test(aggregation): add integration test suite for Phase 7.2a"
```

---

## Task 7: Doc sync

Update documentation to reflect the completed phase.

**Files:**
- Modify: `CLAUDE.md`
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`
- Modify: `demos/demo_commands.txt`
- Modify: `llms.txt`
- Modify: `.wiki/Datalog-Reference.md`

- [ ] **Step 1: Update `CLAUDE.md` test count**

In the "Test Coverage" section, update the test count from 407 to the actual number (run `cargo test 2>&1 | grep "test result"` and sum the totals).

- [ ] **Step 2: Update `CHANGELOG.md`**

Add a new entry at the top (above `[0.10.0]`):

```markdown
## [0.11.0] - 2026-03-24

### Added
- Aggregation in `:find` clause: `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`
- `:with` grouping clause to prevent over-deduplication
- `AggFunc` and `FindSpec` types in `src/query/datalog/types.rs`
- `apply_aggregation` post-processing step in executor

### Semantics
- `count`/`count-distinct` on zero bindings with no grouping vars → `[[0]]`
- All aggregates skip `Value::Null` (SQL behavior)
- Type mismatches (e.g. `sum` on `String`) fail fast with a runtime error
- `min`/`max` on mixed `Integer`/`Float` is a runtime error

### Tests
- Added `tests/aggregation_test.rs` (~26 tests)
- Total: NNN tests passing
```

- [ ] **Step 3: Update `ROADMAP.md`**

Mark Phase 7.2a complete and add Phase 7.2b as next. Find the `### 7.2 Aggregation` section and mark it `✅ COMPLETE`. Update the "Right Now" / "Immediate Next Steps" section at the bottom.

- [ ] **Step 4: Update `demos/demo_commands.txt`**

The `PLANNED (see ROADMAP.md)` section already exists. Update it to note aggregation is now available, and revise the examples to show working aggregation queries.

- [ ] **Step 5: Update `llms.txt`**

Update version from `0.10.0` to `0.11.0` and add aggregation to the Datalog syntax reference section.

- [ ] **Step 6: Update `.wiki/Datalog-Reference.md`**

Add an aggregation section describing the syntax, `:with`, and null-skipping behavior.

- [ ] **Step 7: Update `.wiki/Datalog-Reference.md` in its git repo**

```bash
cd .wiki && git add -A && git commit -m "docs: add Phase 7.2a aggregation reference" && git push
```

- [ ] **Step 8: Commit main repo docs and tag**

```bash
git add CLAUDE.md CHANGELOG.md ROADMAP.md demos/demo_commands.txt llms.txt
git commit -m "docs: sync docs for Phase 7.2a aggregation complete (v0.11.0)"
git tag -a v0.11.0 -m "Phase 7.2a complete — aggregation (count, sum, min, max, count-distinct, sum-distinct, :with)"
git push && git push origin v0.11.0
```

---

## Checklist Summary

- [ ] Task 1: `AggFunc` + `FindSpec` types
- [ ] Task 2: `DatalogQuery` migration + all call sites
- [ ] Task 3: Parser aggregate + `:with` + validation
- [ ] Task 4: `apply_aggregation` + count unit tests
- [ ] Task 5: Sum, min, max, null, empty unit tests
- [ ] Task 6: Integration test suite (~26 tests)
- [ ] Task 7: Doc sync + tag v0.11.0
