# Per-Query Complexity Limits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `:max-derived-facts N` and `:max-results N` as optional per-query keys in the Datalog query syntax, and raise `DEFAULT_MAX_DERIVED_FACTS` from 100,000 to 1,000,000.

**Architecture:** Two new `Option<usize>` fields on `DatalogQuery` carry per-query overrides through the parse → execute pipeline. In `execute_query_with_rules`, effective limits are computed as `query.field.unwrap_or(self.field)` and passed to `StratifiedEvaluator` for that invocation only — no shared state mutation. All other paths are unaffected.

**Tech Stack:** Rust, `cargo test`

---

## File Map

| File | Role |
|---|---|
| `src/query/datalog/types.rs` | Add two `Option<usize>` fields to `DatalogQuery`; update both constructors |
| `src/query/datalog/parser.rs` | Parse `:max-derived-facts` and `:max-results` in `parse_query` |
| `src/query/datalog/executor.rs` | Compute effective limits in `execute_query_with_rules`; pass to `StratifiedEvaluator` |
| `src/query/datalog/evaluator.rs` | Raise `DEFAULT_MAX_DERIVED_FACTS` constant to `1_000_000` |
| `src/db.rs` | Update `OpenOptions::max_derived_facts` doc comment to reflect new default |
| `tests/grammar/grammar.pest` | Add `max_derived_facts_section` / `max_results_section` to `query_section` |
| `.wiki/Datalog-Reference.md` | Update EBNF `query-section` + add narrative for new keywords |
| `.wiki/Performance-Tuning.md` | Update default table (100,000 → 1,000,000) + add per-query override note |

---

### Task 1: Raise `DEFAULT_MAX_DERIVED_FACTS` and update doc comments

**Files:**
- Modify: `src/query/datalog/evaluator.rs:42`
- Modify: `src/db.rs:103-110`

- [ ] **Step 1: Change the constant in `evaluator.rs`**

In `src/query/datalog/evaluator.rs`, change line 42:

```rust
// Before:
pub const DEFAULT_MAX_DERIVED_FACTS: usize = 100_000;

// After:
/// Default maximum facts that can be derived per iteration
pub const DEFAULT_MAX_DERIVED_FACTS: usize = 1_000_000;
```

- [ ] **Step 2: Update the `OpenOptions` doc comment in `db.rs`**

In `src/db.rs`, the builder method `max_derived_facts` currently says "Defaults to 100_000". Change it:

```rust
    /// Set the maximum facts that can be derived per recursive rule iteration.
    ///
    /// Defaults to 1_000_000. Use lower values to prevent runaway recursive rules
    /// from consuming excessive memory. Can be overridden per-query using
    /// `:max-derived-facts N` in the query vector.
    pub fn max_derived_facts(mut self, n: usize) -> Self {
        self.max_derived_facts = n;
        self
    }
```

Also update the field doc comment in the `OpenOptions` struct (around line 71-73):

```rust
    /// Maximum facts that can be derived per recursive rule iteration.
    /// Defaults to 1_000_000. Use to prevent runaway recursive rules.
    pub max_derived_facts: usize,
```

- [ ] **Step 3: Verify the existing test at `db.rs:1796` still compiles**

The test `test_max_derived_facts_limit_enforced` sets `.max_derived_facts(100_000)` explicitly — that's fine, it will continue to work with the new default unchanged.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass (the constant change is backwards-compatible — all existing tests either use the default or set it explicitly).

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/evaluator.rs src/db.rs
git commit -m "perf(query): raise DEFAULT_MAX_DERIVED_FACTS from 100K to 1M (#288)"
```

---

### Task 2: Add `max_derived_facts` / `max_results` fields to `DatalogQuery`

**Files:**
- Modify: `src/query/datalog/types.rs:522-556`

This task has no test (the field is internal; tests come in Task 3 via the parser). The struct change must compile cleanly before adding parser support.

- [ ] **Step 1: Add fields to the struct**

In `src/query/datalog/types.rs`, change the `DatalogQuery` struct (line 522):

```rust
pub struct DatalogQuery {
    /// Variables to return (from :find clause)
    pub find: Vec<FindSpec>,
    /// Where clauses: patterns and rule invocations
    pub where_clauses: Vec<WhereClause>,
    /// Optional transaction-time snapshot (:as-of)
    pub as_of: Option<AsOf>,
    /// Optional valid-time filter (:valid-at)
    pub valid_at: Option<ValidAt>,
    /// Grouping variables that participate in grouping but are excluded from output rows.
    pub with_vars: Vec<String>,
    /// Per-query override for maximum derived facts per recursive rule iteration.
    /// `None` falls back to the executor's configured limit (from `OpenOptions`).
    pub max_derived_facts: Option<usize>,
    /// Per-query override for maximum total query results.
    /// `None` falls back to the executor's configured limit (from `OpenOptions`).
    pub max_results: Option<usize>,
}
```

- [ ] **Step 2: Update `DatalogQuery::new`**

```rust
    pub fn new(find: Vec<FindSpec>, where_clauses: Vec<WhereClause>) -> Self {
        DatalogQuery {
            find,
            where_clauses,
            as_of: None,
            valid_at: None,
            with_vars: Vec::new(),
            max_derived_facts: None,
            max_results: None,
        }
    }
```

- [ ] **Step 3: Update `DatalogQuery::from_patterns`**

```rust
    #[allow(dead_code)]
    pub fn from_patterns(find: Vec<FindSpec>, patterns: Vec<Pattern>) -> Self {
        DatalogQuery {
            find,
            where_clauses: patterns.into_iter().map(WhereClause::Pattern).collect(),
            as_of: None,
            valid_at: None,
            with_vars: Vec::new(),
            max_derived_facts: None,
            max_results: None,
        }
    }
```

- [ ] **Step 4: Build to confirm no struct-literal errors**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors. (All `DatalogQuery` construction goes through `new()` / `from_patterns()` — the struct is internal and never constructed with `{ field: val, ... }` syntax outside these two constructors.)

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(query): add max_derived_facts/max_results fields to DatalogQuery (#288)"
```

---

### Task 3: Parse `:max-derived-facts` and `:max-results` in `parse_query`

**Files:**
- Modify: `src/query/datalog/parser.rs` (inside `parse_query`, around line 726)
- Test: `src/query/datalog/parser.rs` (in the existing `#[cfg(test)]` block)

- [ ] **Step 1: Write the failing tests**

Add these tests inside the existing `#[cfg(test)]` block in `parser.rs`. Find an appropriate location near the existing `:as-of` / `:valid-at` parse tests.

```rust
#[test]
fn test_parse_max_derived_facts_valid() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 500000])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_ok(), "should parse :max-derived-facts");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert_eq!(q.max_derived_facts, Some(500_000));
            assert_eq!(q.max_results, None);
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_max_results_valid() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-results 9999])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_ok(), "should parse :max-results");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert_eq!(q.max_derived_facts, None);
            assert_eq!(q.max_results, Some(9_999));
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_both_limits_valid() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 1000000 :max-results 100])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_ok(), "should parse both limit keys");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert_eq!(q.max_derived_facts, Some(1_000_000));
            assert_eq!(q.max_results, Some(100));
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_max_derived_facts_zero_rejected() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 0])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err(), "0 should be rejected");
    let msg = result.unwrap_err();
    assert!(
        msg.contains(":max-derived-facts must be >= 1"),
        "wrong error: {}",
        msg
    );
}

#[test]
fn test_parse_max_results_zero_rejected() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-results 0])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err(), "0 should be rejected");
    let msg = result.unwrap_err();
    assert!(
        msg.contains(":max-results must be >= 1"),
        "wrong error: {}",
        msg
    );
}

#[test]
fn test_parse_max_derived_facts_duplicate_rejected() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 100 :max-derived-facts 200])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err(), "duplicate key should be rejected");
    let msg = result.unwrap_err();
    assert!(
        msg.contains("duplicate :max-derived-facts"),
        "wrong error: {}",
        msg
    );
}

#[test]
fn test_parse_max_results_duplicate_rejected() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-results 100 :max-results 200])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err(), "duplicate key should be rejected");
    let msg = result.unwrap_err();
    assert!(
        msg.contains("duplicate :max-results"),
        "wrong error: {}",
        msg
    );
}

#[test]
fn test_parse_max_derived_facts_non_integer_rejected() {
    let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts "a lot"])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_err(), "non-integer should be rejected");
}

#[test]
fn test_parse_limits_order_independent() {
    // :max-derived-facts before :find — order should not matter
    let input = r#"(query [:max-derived-facts 42 :find ?x :where [?x :a :b]])"#;
    let result = parse_datalog_command(input);
    assert!(result.is_ok(), "order should not matter");
    match result.unwrap() {
        DatalogCommand::Query(q) => assert_eq!(q.max_derived_facts, Some(42)),
        _ => panic!("expected Query"),
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test -p minigraf -- test_parse_max_derived_facts_valid test_parse_max_results_valid test_parse_both_limits_valid test_parse_max_derived_facts_zero_rejected test_parse_max_results_zero_rejected test_parse_max_derived_facts_duplicate_rejected test_parse_max_results_duplicate_rejected test_parse_max_derived_facts_non_integer_rejected test_parse_limits_order_independent 2>&1 | grep -E "FAILED|error"
```

Expected: tests fail because the parse logic doesn't exist yet.

- [ ] **Step 3: Add parse logic to `parse_query`**

In `src/query/datalog/parser.rs`, inside `parse_query` (around line 726), add two new `Option<usize>` locals alongside the existing ones:

```rust
    let mut query_as_of: Option<AsOf> = None;
    let mut query_valid_at: Option<ValidAt> = None;
    let mut query_max_derived_facts: Option<usize> = None;
    let mut query_max_results: Option<usize> = None;
```

Then add two new `match` arms inside the `match keyword { ... }` block, after the `:any-valid-time` arm and before the `:with` arm:

```rust
                ":max-derived-facts" => {
                    if query_max_derived_facts.is_some() {
                        return Err("duplicate :max-derived-facts".to_string());
                    }
                    i += 1;
                    let val = query_vector
                        .get(i)
                        .ok_or_else(|| ":max-derived-facts requires a value".to_string())?;
                    match val {
                        EdnValue::Integer(n) if *n >= 1 => {
                            query_max_derived_facts = Some(*n as usize);
                        }
                        EdnValue::Integer(0) | EdnValue::Integer(_) => {
                            return Err(":max-derived-facts must be >= 1".to_string());
                        }
                        other => {
                            return Err(format!(
                                ":max-derived-facts must be a positive integer, got {:?}",
                                other
                            ));
                        }
                    }
                    i += 1;
                    continue;
                }
                ":max-results" => {
                    if query_max_results.is_some() {
                        return Err("duplicate :max-results".to_string());
                    }
                    i += 1;
                    let val = query_vector
                        .get(i)
                        .ok_or_else(|| ":max-results requires a value".to_string())?;
                    match val {
                        EdnValue::Integer(n) if *n >= 1 => {
                            query_max_results = Some(*n as usize);
                        }
                        EdnValue::Integer(0) | EdnValue::Integer(_) => {
                            return Err(":max-results must be >= 1".to_string());
                        }
                        other => {
                            return Err(format!(
                                ":max-results must be a positive integer, got {:?}",
                                other
                            ));
                        }
                    }
                    i += 1;
                    continue;
                }
```

Then, at the end of `parse_query` where the fields are assigned (around line 903):

```rust
    query.as_of = query_as_of;
    query.valid_at = query_valid_at;
    query.with_vars = with_vars;
    query.max_derived_facts = query_max_derived_facts;
    query.max_results = query_max_results;
```

- [ ] **Step 4: Note on the integer match pattern**

The `EdnValue::Integer(n) if *n >= 1` guard matches positive integers. The catch-all `EdnValue::Integer(0) | EdnValue::Integer(_)` arm is unreachable for positive `n` but Rust requires exhaustiveness — the compiler may warn. If so, simplify to:

```rust
                        EdnValue::Integer(n) if *n >= 1 => {
                            query_max_derived_facts = Some(*n as usize);
                        }
                        EdnValue::Integer(_) => {
                            return Err(":max-derived-facts must be >= 1".to_string());
                        }
```

- [ ] **Step 5: Run the parser tests**

```bash
cargo test -p minigraf -- test_parse_max_derived_facts_valid test_parse_max_results_valid test_parse_both_limits_valid test_parse_max_derived_facts_zero_rejected test_parse_max_results_zero_rejected test_parse_max_derived_facts_duplicate_rejected test_parse_max_results_duplicate_rejected test_parse_max_derived_facts_non_integer_rejected test_parse_limits_order_independent 2>&1 | tail -10
```

Expected: all 9 tests pass.

- [ ] **Step 6: Run the full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(query): parse :max-derived-facts and :max-results per-query keys (#288)"
```

---

### Task 4: Apply per-query limits in `execute_query_with_rules`

**Files:**
- Modify: `src/query/datalog/executor.rs:934-941`
- Test: `src/query/datalog/executor.rs` (in `#[cfg(test)]` block)

- [ ] **Step 1: Write the failing tests**

Add these to the `#[cfg(test)]` block in `executor.rs`:

```rust
#[test]
fn test_per_query_max_derived_facts_overrides_executor_default() {
    // Executor default is 1M; per-query override is 5 — should fail.
    let storage = FactStorage::new();
    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
    let mut executor = DatalogExecutor::new_with_rules_and_functions(
        storage,
        rules.clone(),
        functions.clone(),
    );
    executor.set_limits(1_000_000, 1_000_000);

    // Register a recursive rule
    let rule_cmd = parse_datalog_command(
        "(rule [(reachable ?x ?y)] [?x :edge ?y])"
    ).unwrap();
    executor.execute(rule_cmd).unwrap();
    let rule_cmd2 = parse_datalog_command(
        "(rule [(reachable ?x ?z)] [?x :edge ?y] (reachable ?y ?z))"
    ).unwrap();
    executor.execute(rule_cmd2).unwrap();

    // Transact some facts
    executor.execute(parse_datalog_command(
        r#"(transact [[:a :edge :b] [:b :edge :c]])"#
    ).unwrap()).unwrap();

    // Query with per-query limit of 1 — should fail
    let result = executor.execute(parse_datalog_command(
        "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1])"
    ).unwrap());
    assert!(result.is_err(), "per-query limit of 1 should fail");

    // Query with per-query limit of 1M — should succeed
    let result = executor.execute(parse_datalog_command(
        "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1000000])"
    ).unwrap());
    assert!(result.is_ok(), "per-query limit of 1M should succeed");
}

#[test]
fn test_per_query_limit_does_not_bleed_into_next_query() {
    // A tight per-query limit on one query should not affect the next.
    let storage = FactStorage::new();
    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let functions = Arc::new(RwLock::new(FunctionRegistry::with_builtins()));
    let executor = DatalogExecutor::new_with_rules_and_functions(
        storage,
        rules.clone(),
        functions.clone(),
    );

    executor.execute(parse_datalog_command(
        "(rule [(reachable ?x ?y)] [?x :edge ?y])"
    ).unwrap()).unwrap();

    executor.execute(parse_datalog_command(
        r#"(transact [[:a :edge :b]])"#
    ).unwrap()).unwrap();

    // First query: tight limit, will fail
    let _ = executor.execute(parse_datalog_command(
        "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1])"
    ).unwrap());

    // Second query: no per-query limit — should use executor default (1M) and succeed
    let result = executor.execute(parse_datalog_command(
        "(query [:find ?x ?y :where (reachable ?x ?y)])"
    ).unwrap());
    assert!(result.is_ok(), "next query should not inherit the tight per-query limit");
}
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test -p minigraf -- test_per_query_max_derived_facts_overrides_executor_default test_per_query_limit_does_not_bleed_into_next_query 2>&1 | grep -E "FAILED|error\[" | head -10
```

Expected: tests fail because the override logic is not wired in yet.

- [ ] **Step 3: Implement the override in `execute_query_with_rules`**

In `src/query/datalog/executor.rs`, in `execute_query_with_rules` (line ~934), change the `StratifiedEvaluator::new` call:

```rust
        // Compute effective limits: per-query override takes precedence over executor default.
        let effective_max_derived = query.max_derived_facts.unwrap_or(self.max_derived_facts);
        let effective_max_results = query.max_results.unwrap_or(self.max_results);

        // Create StratifiedEvaluator — handles negation, stratification, and positive-only rules
        let evaluator = StratifiedEvaluator::new(
            filtered_storage,
            self.rules.clone(),
            self.functions.clone(),
            1000, // max iterations
            effective_max_derived,
            effective_max_results,
        );
```

The two new locals must be declared before the `StratifiedEvaluator::new` call. The existing `self.max_derived_facts` / `self.max_results` fields are read, not mutated — no state bleeds between queries.

- [ ] **Step 4: Run the new tests**

```bash
cargo test -p minigraf -- test_per_query_max_derived_facts_overrides_executor_default test_per_query_limit_does_not_bleed_into_next_query 2>&1 | tail -10
```

Expected: both pass.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(query): apply per-query limits in execute_query_with_rules (#288)"
```

---

### Task 5: Integration test via `Minigraf::execute`

**Files:**
- Test: `src/db.rs` (in `#[cfg(test)]` block)

This task confirms the full path from `Minigraf::execute` through the parser and executor works end-to-end.

- [ ] **Step 1: Write the failing test**

Add this to the `#[cfg(test)]` block in `src/db.rs`, near the existing `test_max_derived_facts_limit_enforced` test:

```rust
#[test]
fn test_per_query_max_derived_facts_via_execute() {
    let db = OpenOptions::new()
        .max_derived_facts(1_000_000)
        .open_memory()
        .unwrap();

    // Register a recursive rule: reachable via edges
    db.execute("(rule [(reachable ?x ?y)] [?x :edge ?y])").unwrap();
    db.execute("(rule [(reachable ?x ?z)] [?x :edge ?y] (reachable ?y ?z))").unwrap();

    // Transact a small graph: a -> b -> c
    db.execute(r#"(transact [[:a :edge :b] [:b :edge :c]])"#).unwrap();

    // Per-query limit of 1 — too tight, must fail
    let result = db.execute(
        "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1])"
    );
    assert!(result.is_err(), "per-query limit of 1 should fail");

    // Per-query limit of 1M — should succeed
    let result = db.execute(
        "(query [:find ?x ?y :where (reachable ?x ?y) :max-derived-facts 1000000])"
    );
    assert!(result.is_ok(), "per-query limit of 1M should succeed");

    // No per-query limit — should fall back to OpenOptions default (1M) and succeed
    let result = db.execute(
        "(query [:find ?x ?y :where (reachable ?x ?y)])"
    );
    assert!(result.is_ok(), "no per-query limit should use database default");
}

#[test]
fn test_per_query_max_results_via_execute() {
    let db = Minigraf::in_memory().unwrap();

    db.execute(r#"(transact [[:a :v 1] [:b :v 2] [:c :v 3]])"#).unwrap();

    // Per-query :max-results 1 — should not error (max_results on non-rules path
    // is enforced in StratifiedEvaluator; for non-rules queries the field is parsed
    // and carried through but the result set is not forcibly truncated — this test
    // confirms the field parses cleanly and the query succeeds).
    let result = db.execute(
        "(query [:find ?e :where [?e :v ?v] :max-results 1])"
    );
    assert!(result.is_ok(), "query with :max-results should parse and execute");
}
```

- [ ] **Step 2: Run to confirm they pass immediately** (the implementation is already wired)

```bash
cargo test -p minigraf -- test_per_query_max_derived_facts_via_execute test_per_query_max_results_via_execute 2>&1 | tail -10
```

Expected: both pass (Tasks 1–4 already supply the implementation).

- [ ] **Step 3: Run full suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/db.rs
git commit -m "test(db): add integration tests for per-query :max-derived-facts and :max-results (#288)"
```

---

### Task 6: Update `tests/grammar/grammar.pest`

**Files:**
- Modify: `tests/grammar/grammar.pest:90-92`

The `query_section` rule must include the two new keywords or the grammar conformance tests will diverge from the actual parser.

- [ ] **Step 1: Add the new section rules**

In `tests/grammar/grammar.pest`, change the `query_section` rule (lines 90–92) and add two new rules below `with_section`:

```pest
query_section = {
    find_section | where_section | as_of_section |
    valid_at_section | any_valid_time_section | with_section |
    max_derived_facts_section | max_results_section
}

// ...existing rules stay unchanged...

with_section           = { ":with" ~ variable+ }
max_derived_facts_section = { ":max-derived-facts" ~ integer_lit }
max_results_section    = { ":max-results" ~ integer_lit }
```

(The semantic check that the integer is ≥ 1 happens in the Rust parser, not in pest.)

- [ ] **Step 2: Run any grammar conformance tests**

```bash
cargo test grammar 2>&1 | tail -10
```

Expected: pass (or "no tests matched" if the suite doesn't auto-test the PEG file — either is fine as long as there are no compile errors).

- [ ] **Step 3: Commit**

```bash
git add tests/grammar/grammar.pest
git commit -m "docs(grammar): add :max-derived-facts and :max-results to query_section (#288)"
```

---

### Task 7: Update `.wiki/Datalog-Reference.md`

**Files:**
- Modify: `.wiki/Datalog-Reference.md`

Two changes: (1) EBNF `query-section` rule, (2) a new narrative section documenting the keywords.

- [ ] **Step 1: Update the EBNF `query-section` rule**

Find the block (around line 28–37):

```ebnf
query-section ::=
    find-section | where-section | as-of-section |
    valid-at-section | any-valid-time-section | with-section
```

Change to:

```ebnf
query-section ::=
    find-section | where-section | as-of-section |
    valid-at-section | any-valid-time-section | with-section |
    max-derived-facts-section | max-results-section

max-derived-facts-section ::= ":max-derived-facts" integer
max-results-section       ::= ":max-results" integer
```

- [ ] **Step 2: Add a narrative section**

Find the `:valid-at` narrative section (around line 329). After the `:valid-at` section, add:

```markdown
### `:max-derived-facts` and `:max-results` — per-query complexity limits

Override the database-level `OpenOptions` complexity limits for a single query:

```datalog
;; Run a transitive-closure query that would normally hit the 1M default
(query [:find ?ancestor
        :where (ancestor ?ancestor "abc123")
        :max-derived-facts 5000000
        :max-results 10000])
```

Both keys are optional and order-independent. Omitting a key falls back to the `OpenOptions`
value for that limit. Values must be positive integers (≥ 1).

- `:max-derived-facts N` — caps how many facts the recursive rule engine can derive
  internally before returning an error. Use when a legitimate recursive query exceeds
  the database default.
- `:max-results N` — caps the maximum number of result rows returned. Applies inside
  the rule evaluator; for non-recursive queries results are not truncated by this value.

The limits are applied for that query only and do not affect subsequent queries.
```

- [ ] **Step 3: Commit the wiki**

```bash
cd .wiki && git add Datalog-Reference.md && git commit -m "docs: add :max-derived-facts and :max-results to Datalog Reference (#288)" && git push && cd ..
```

---

### Task 8: Update `.wiki/Performance-Tuning.md`

**Files:**
- Modify: `.wiki/Performance-Tuning.md`

Two changes: (1) update the default in the table, (2) add a per-query override note.

- [ ] **Step 1: Update the default in the configuration table**

Find the table (around line 46–50):

```markdown
| `max_derived_facts` | 100,000 | derived facts per rule iteration |
```

Change to:

```markdown
| `max_derived_facts` | 1,000,000 | derived facts per rule iteration |
```

- [ ] **Step 2: Update the prose below the table**

Find (around line 64–66):

```markdown
**`max_derived_facts` / `max_results`** — Safety limits, not performance knobs. Lower them to fail
fast on runaway recursive rules or unexpectedly large result sets; raise only if a legitimate query
hits the ceiling.
```

Change to:

```markdown
**`max_derived_facts` / `max_results`** — Safety limits, not performance knobs. Lower them to fail
fast on runaway recursive rules or unexpectedly large result sets; raise only if a legitimate query
hits the ceiling. For one-off queries that need a different limit without reconfiguring the database,
use `:max-derived-facts N` or `:max-results N` directly in the query vector — see the
[Datalog Reference](Datalog-Reference#max-derived-facts-and-max-results--per-query-complexity-limits).
```

- [ ] **Step 3: Commit the wiki**

```bash
cd .wiki && git add Performance-Tuning.md && git commit -m "docs: update max_derived_facts default to 1M; add per-query override note (#288)" && git push && cd ..
```

---

### Task 9: Final verification and close

- [ ] **Step 1: Run the complete test suite one final time**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass, no regressions.

- [ ] **Step 2: Smoke-test in the REPL**

```bash
echo '(query [:find ?x :where [?x :a :b] :max-derived-facts 999999 :max-results 50])' | cargo run --quiet
```

Expected: `No results` (no facts match) — confirms the query parses and executes without error.

- [ ] **Step 3: Commit the final plan doc**

```bash
git add docs/superpowers/plans/2026-06-04-per-query-limits.md
git commit -m "docs: add per-query limits implementation plan (#288)"
```

- [ ] **Step 4: Push and create PR**

```bash
git push origin <worktree-branch>
gh pr create --title "feat(query): per-query :max-derived-facts and :max-results; raise default to 1M (#288)" \
  --body "$(cat <<'EOF'
## Summary

- Adds `:max-derived-facts N` and `:max-results N` as optional per-query keys in the Datalog query syntax
- Per-query values override the `OpenOptions` database-level limits for that single query only; no shared state mutation
- Raises `DEFAULT_MAX_DERIVED_FACTS` from 100,000 to 1,000,000
- Updates `tests/grammar/grammar.pest`, `.wiki/Datalog-Reference.md`, `.wiki/Performance-Tuning.md`

## Test plan

- [ ] All existing tests pass
- [ ] Parser tests: valid keys, `0` rejected, duplicate rejected, non-integer rejected, order-independent
- [ ] Executor tests: per-query override takes effect; does not bleed into next query
- [ ] Integration tests via `Minigraf::execute` for both `:max-derived-facts` and `:max-results`

Closes #288
Tracks long-term magic sets work in #289

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
