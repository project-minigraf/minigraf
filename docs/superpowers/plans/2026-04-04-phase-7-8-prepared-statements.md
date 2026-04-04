# Phase 7.8 Prepared Statements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `Minigraf::prepare()` and `PreparedQuery::execute()` — parse once, bind `$slot` values, execute many times.

**Architecture:** Option A (inline substitution). Parse into a `DatalogQuery` with `BindSlot`/`Slot` variants; on each `execute()` deep-clone and walk the AST replacing slot nodes with concrete values, then pass the filled query to the existing executor unchanged. No changes to executor, evaluator, matcher, or optimizer logic.

**Tech Stack:** Rust, `uuid`, `anyhow`, existing `DatalogExecutor`

---

## File Map

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `EdnValue::BindSlot`, `AsOf::Slot`, `ValidAt::Slot`, `Expr::Slot` |
| `src/query/datalog/parser.rs` | Add `Token::BindSlot`, tokenize `$identifier`, `parse_value` arm, `:as-of`/`:valid-at` slot arms, `parse_expr_arg` arm |
| `src/graph/storage.rs` | Panic guard arm for `AsOf::Slot` in `get_facts_as_of` |
| `src/query/datalog/executor.rs` | Panic guard arms for `Expr::Slot` in `eval_expr`, `ValidAt::Slot` in `filter_facts_for_query` |
| `src/query/datalog/prepared.rs` | **New file.** `BindValue`, `PreparedQuery`, all validation/substitution logic |
| `src/query/datalog/mod.rs` | `pub mod prepared` |
| `src/db.rs` | `Minigraf::prepare()` |
| `src/lib.rs` | Re-export `PreparedQuery`, `BindValue` |
| `tests/prepared_statements_test.rs` | **New file.** All integration tests |

---

## Task 1: Add new type variants to `types.rs`

**Files:**
- Modify: `src/query/datalog/types.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` module at the bottom of `src/query/datalog/types.rs`:

```rust
#[test]
fn test_bind_slot_edn_variant_exists() {
    let v = EdnValue::BindSlot("entity".to_string());
    assert!(matches!(v, EdnValue::BindSlot(_)));
    // BindSlot is not a logic variable — it is not a ?-prefixed symbol
    assert!(!v.is_variable());
    assert!(v.as_variable().is_none());
}

#[test]
fn test_as_of_slot_variant_exists() {
    let a = AsOf::Slot("tx".to_string());
    assert!(matches!(a, AsOf::Slot(_)));
}

#[test]
fn test_valid_at_slot_variant_exists() {
    let v = ValidAt::Slot("date".to_string());
    assert!(matches!(v, ValidAt::Slot(_)));
}

#[test]
fn test_expr_slot_variant_exists() {
    let e = Expr::Slot("threshold".to_string());
    assert!(matches!(e, Expr::Slot(_)));
}
```

- [ ] **Step 2: Run to verify tests fail**

```bash
cargo test --lib -- types::tests::test_bind_slot_edn_variant_exists types::tests::test_as_of_slot_variant_exists types::tests::test_valid_at_slot_variant_exists types::tests::test_expr_slot_variant_exists 2>&1 | tail -20
```

Expected: compile error — variants do not exist yet.

- [ ] **Step 3: Add `EdnValue::BindSlot`**

In `src/query/datalog/types.rs`, find the `EdnValue` enum and add after `Nil`:

```rust
    /// A named bind slot: `$identifier`.
    /// Only valid in a `PreparedQuery` template AST — must be replaced by
    /// `substitute()` before the query reaches the executor.
    BindSlot(String),
```

- [ ] **Step 4: Add `AsOf::Slot`**

Find the `AsOf` enum and add:

```rust
    /// Named bind slot: `$name` — resolved to `Counter` or `Timestamp` at execute time.
    Slot(String),
```

- [ ] **Step 5: Add `ValidAt::Slot`**

Find the `ValidAt` enum and add:

```rust
    /// Named bind slot: `$name` — resolved to `Timestamp` or `AnyValidTime` at execute time.
    Slot(String),
```

- [ ] **Step 6: Add `Expr::Slot`**

Find the `Expr` enum and add between `Lit` and `BinOp`:

```rust
    /// Named bind slot: `$name` — substituted to `Expr::Lit` before execution.
    Slot(String),
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
cargo test --lib -- types::tests::test_bind_slot_edn_variant_exists types::tests::test_as_of_slot_variant_exists types::tests::test_valid_at_slot_variant_exists types::tests::test_expr_slot_variant_exists 2>&1 | tail -20
```

Expected: 4 tests pass. There will also be compile errors in other files that match on these enums — fix them in the next task.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add BindSlot/Slot variants for prepared statement AST nodes"
```

---

## Task 2: Add panic guards for new variants in executor and storage

The new enum variants cause non-exhaustive match errors. Add arms that panic — these positions should never be reached after `substitute()` has run.

**Files:**
- Modify: `src/query/datalog/executor.rs`
- Modify: `src/graph/storage.rs`

- [ ] **Step 1: Fix `eval_expr` in `executor.rs`**

Find `pub(crate) fn eval_expr` (~line 1413). It matches on `expr`:

```rust
pub(crate) fn eval_expr(
    expr: &Expr,
    binding: &std::collections::HashMap<String, Value>,
    registry: Option<&FunctionRegistry>,
) -> Result<Value, ()> {
    match expr {
        Expr::Var(v) => ...
        Expr::Lit(val) => ...
        Expr::UnaryOp(...) => ...
        Expr::BinOp(...) => ...
    }
}
```

Add before the closing brace of the match:

```rust
        Expr::Slot(name) => {
            panic!("internal: unsubstituted bind slot '{}' reached eval_expr; call PreparedQuery::execute() instead of passing the template directly", name);
        }
```

- [ ] **Step 2: Fix `filter_facts_for_query` in `executor.rs`**

Find `fn filter_facts_for_query` (~line 195). It has two match sites for `ValidAt`. Add `Slot` arms to both.

First match (the `valid_filtered` match, ~line 214):

```rust
        let valid_filtered: Vec<Fact> = match &query.valid_at {
            Some(ValidAt::Timestamp(t)) => asserted
                .into_iter()
                .filter(|f| f.valid_from <= *t && *t < f.valid_to)
                .collect(),
            Some(ValidAt::AnyValidTime) => asserted,
            None => asserted
                .into_iter()
                .filter(|f| f.valid_from <= now && now < f.valid_to)
                .collect(),
            Some(ValidAt::Slot(name)) => {
                panic!("internal: unsubstituted :valid-at bind slot '{}' reached filter_facts_for_query", name);
            }
        };
```

Second match (`valid_at_value`, ~line 239):

```rust
        let valid_at_value = match &query.valid_at {
            Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
            Some(ValidAt::AnyValidTime) => Value::Null,
            None => Value::Integer(now),
            Some(ValidAt::Slot(name)) => {
                panic!("internal: unsubstituted :valid-at bind slot '{}' reached valid_at_value computation", name);
            }
        };
```

Search the file for all other match sites on `ValidAt` (there are several inside `execute_query_with_rules` and helper functions) and add a `Slot` panic arm to each. Run `cargo build` to find remaining non-exhaustive match errors and add arms until it compiles.

- [ ] **Step 3: Fix `get_facts_as_of` in `storage.rs`**

Find `pub fn get_facts_as_of` (~line 315):

```rust
    pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        let filtered = all
            .into_iter()
            .filter(|f| match as_of {
                AsOf::Counter(n) => f.tx_count <= *n,
                AsOf::Timestamp(t) => f.tx_id <= *t as u64,
            })
            .collect();
        Ok(filtered)
    }
```

Add the `Slot` arm:

```rust
    pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        let filtered = all
            .into_iter()
            .filter(|f| match as_of {
                AsOf::Counter(n) => f.tx_count <= *n,
                AsOf::Timestamp(t) => f.tx_id <= *t as u64,
                AsOf::Slot(name) => {
                    panic!("internal: unsubstituted :as-of bind slot '{}' reached get_facts_as_of", name);
                }
            })
            .collect();
        Ok(filtered)
    }
```

- [ ] **Step 4: Verify compilation**

```bash
cargo build 2>&1 | grep "error\|warning: unused"
```

Expected: zero errors. There may be warnings about `Slot` being unreachable — these are correct and expected.

- [ ] **Step 5: Verify existing tests still pass**

```bash
cargo test 2>&1 | tail -10
```

Expected: all previously passing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs src/graph/storage.rs
git commit -m "feat(executor,storage): add panic guards for unsubstituted BindSlot variants"
```

---

## Task 3: Tokenizer and parser support for `$identifier`

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing parser tests**

Add to the `#[cfg(test)]` module inside `parser.rs`:

```rust
#[test]
fn test_tokenize_bind_slot() {
    let result = parse_edn("$entity");
    assert!(matches!(result, Ok(EdnValue::BindSlot(ref s)) if s == "entity"), "expected BindSlot, got {:?}", result);
}

#[test]
fn test_tokenize_bind_slot_hyphenated() {
    let result = parse_edn("$min-level");
    assert!(matches!(result, Ok(EdnValue::BindSlot(ref s)) if s == "min-level"), "expected BindSlot(min-level), got {:?}", result);
}

#[test]
fn test_parse_bind_slot_in_entity_position() {
    let cmd = parse_datalog_command(
        "(query [:find ?name :where [$entity :person/name ?name]])"
    );
    assert!(cmd.is_ok(), "parse failed");
    match cmd.unwrap() {
        DatalogCommand::Query(q) => {
            match &q.where_clauses[0] {
                WhereClause::Pattern(p) => {
                    assert!(matches!(&p.entity, EdnValue::BindSlot(s) if s == "entity"));
                }
                _ => panic!("expected Pattern"),
            }
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_bind_slot_in_value_position() {
    let cmd = parse_datalog_command(
        "(query [:find ?e :where [?e :person/name $name]])"
    );
    assert!(cmd.is_ok(), "parse failed");
    match cmd.unwrap() {
        DatalogCommand::Query(q) => {
            match &q.where_clauses[0] {
                WhereClause::Pattern(p) => {
                    assert!(matches!(&p.value, EdnValue::BindSlot(s) if s == "name"));
                }
                _ => panic!("expected Pattern"),
            }
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_as_of_slot() {
    let cmd = parse_datalog_command(
        "(query [:find ?v :as-of $tx :where [?e :score ?v]])"
    );
    assert!(cmd.is_ok(), "parse failed");
    match cmd.unwrap() {
        DatalogCommand::Query(q) => {
            assert!(matches!(q.as_of, Some(AsOf::Slot(ref s)) if s == "tx"));
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_valid_at_slot() {
    let cmd = parse_datalog_command(
        "(query [:find ?v :valid-at $date :where [?e :score ?v]])"
    );
    assert!(cmd.is_ok(), "parse failed");
    match cmd.unwrap() {
        DatalogCommand::Query(q) => {
            assert!(matches!(q.valid_at, Some(ValidAt::Slot(ref s)) if s == "date"));
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_expr_bind_slot_in_binop() {
    let cmd = parse_datalog_command(
        "(query [:find ?v :where [?e :score ?v] [(>= ?v $threshold)]])"
    );
    assert!(cmd.is_ok(), "parse failed");
    match cmd.unwrap() {
        DatalogCommand::Query(q) => {
            let expr_clause = q.where_clauses.iter().find(|c| matches!(c, WhereClause::Expr { .. }));
            assert!(expr_clause.is_some(), "no Expr clause found");
            match expr_clause.unwrap() {
                WhereClause::Expr { expr: Expr::BinOp(_, _, rhs), .. } => {
                    assert!(matches!(rhs.as_ref(), Expr::Slot(s) if s == "threshold"));
                }
                other => panic!("expected BinOp Expr clause, got {:?}", other),
            }
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_bind_slot_empty_name_is_error() {
    let result = parse_edn("$");
    assert!(result.is_err(), "bare '$' should be a parse error");
}
```

- [ ] **Step 2: Run to verify tests fail**

```bash
cargo test --lib -- parser::tests::test_tokenize_bind_slot parser::tests::test_parse_as_of_slot parser::tests::test_parse_valid_at_slot 2>&1 | tail -10
```

Expected: errors — `$` is currently `Unexpected character`.

- [ ] **Step 3: Add `Token::BindSlot` to the `Token` enum**

In `parser.rs`, find the `Token` enum (line ~12) and add:

```rust
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Keyword(String),
    Symbol(String),
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    TaggedLiteral(String),
    BindSlot(String),  // $identifier — named bind slot for PreparedQuery
    Nil,
}
```

- [ ] **Step 4: Handle `$` in `tokenize()`**

In the `tokenize` function, find the `_` catch-all arm at the end (line ~222):

```rust
            _ => {
                return Err(format!("Unexpected character: {}", ch));
            }
```

Insert a new arm **before** the catch-all:

```rust
            '$' => {
                chars.next(); // consume '$'
                let mut name = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                        if name.len() >= MAX_SYMBOL_LENGTH {
                            return Err(format!(
                                "Bind slot name exceeds maximum length of {} bytes",
                                MAX_SYMBOL_LENGTH
                            ));
                        }
                        name.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    return Err("Bind slot '$' must be followed by an identifier (e.g. $entity)".to_string());
                }
                tokens.push(Token::BindSlot(name));
            }
```

- [ ] **Step 5: Handle `Token::BindSlot` in `Parser::parse_value()`**

In the `parse_value` method (line ~317), add a new match arm after the `Token::Nil` arm:

```rust
            Some(Token::BindSlot(_)) => {
                if let Some(Token::BindSlot(name)) = self.advance() {
                    Ok(EdnValue::BindSlot(name))
                } else {
                    unreachable!()
                }
            }
```

- [ ] **Step 6: Handle `EdnValue::BindSlot` in `:as-of` parsing**

Find the `:as-of` parsing block inside `parse_query` (~line 664). The current match is:

```rust
                    let as_of = match &query_vector[i] {
                        EdnValue::Integer(n) if *n >= 0 => AsOf::Counter(*n as u64),
                        EdnValue::Integer(n) => {
                            return Err(format!(":as-of counter must be non-negative, got {}", n));
                        }
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s).map_err(|e| e.to_string())?;
                            AsOf::Timestamp(ts)
                        }
                        other => {
                            return Err(format!(
                                ":as-of must be an integer (counter) or ISO 8601 string, got {:?}",
                                other
                            ));
                        }
                    };
```

Add a `BindSlot` arm before the `other` catch-all:

```rust
                        EdnValue::BindSlot(name) => AsOf::Slot(name.clone()),
```

- [ ] **Step 7: Handle `EdnValue::BindSlot` in `:valid-at` parsing**

Find the `:valid-at` parsing block (~line 690). The current match is:

```rust
                    let valid_at = match &query_vector[i] {
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s).map_err(|e| e.to_string())?;
                            ValidAt::Timestamp(ts)
                        }
                        EdnValue::Keyword(k) if k == ":any-valid-time" => ValidAt::AnyValidTime,
                        other => {
                            return Err(format!(
                                ":valid-at must be an ISO 8601 string or :any-valid-time, got {:?}",
                                other
                            ));
                        }
                    };
```

Add before the `other` arm:

```rust
                        EdnValue::BindSlot(name) => ValidAt::Slot(name.clone()),
```

- [ ] **Step 8: Handle `EdnValue::BindSlot` in `parse_expr_arg()`**

Find `fn parse_expr_arg` (~line 974). Add a new arm before the `other` catch-all:

```rust
        EdnValue::BindSlot(name) => Ok(Expr::Slot(name.clone())),
```

- [ ] **Step 9: Run all new parser tests**

```bash
cargo test --lib -- parser::tests::test_tokenize_bind_slot parser::tests::test_tokenize_bind_slot_hyphenated parser::tests::test_parse_bind_slot_in_entity_position parser::tests::test_parse_bind_slot_in_value_position parser::tests::test_parse_as_of_slot parser::tests::test_parse_valid_at_slot parser::tests::test_parse_expr_bind_slot_in_binop parser::tests::test_parse_bind_slot_empty_name_is_error 2>&1 | tail -15
```

Expected: 8 tests pass.

- [ ] **Step 10: Verify no regressions**

```bash
cargo test 2>&1 | tail -10
```

Expected: all previously passing tests still pass.

- [ ] **Step 11: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): tokenize and parse \$identifier as BindSlot for prepared statements"
```

---

## Task 4: Create `prepared.rs`

**Files:**
- Create: `src/query/datalog/prepared.rs`

- [ ] **Step 1: Write unit tests first**

Create `src/query/datalog/prepared.rs` with only the test module:

```rust
use crate::graph::types::Value;
use crate::query::datalog::executor::QueryResult;
use crate::query::datalog::functions::FunctionRegistry;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogQuery, EdnValue, Expr, ValidAt, WhereClause,
};
use crate::graph::FactStorage;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ─── BindValue ────────────────────────────────────────────────────────────────

/// A concrete value supplied to a named bind slot (`$name`) in a `PreparedQuery`.
#[derive(Debug, Clone)]
pub enum BindValue {
    /// Substituted into an entity position: `[$entity :attr ?v]`.
    Entity(Uuid),
    /// Substituted into a value position `[?e :attr $val]` or an expression literal `[(>= ?v $threshold)]`.
    Val(Value),
    /// Substituted into an `:as-of $tx` slot (monotonic transaction counter).
    TxCount(u64),
    /// Substituted into an `:as-of $tx` slot (wall-clock millis since epoch)
    /// or a `:valid-at $date` slot.
    Timestamp(i64),
    /// Substituted into a `:valid-at $date` slot — disables valid-time filtering.
    AnyValidTime,
}

// ─── PreparedQuery ────────────────────────────────────────────────────────────

/// A parsed and optimized query template with named bind slots.
///
/// Obtain via [`crate::db::Minigraf::prepare`]. Execute many times via [`PreparedQuery::execute`].
pub struct PreparedQuery {
    template: DatalogQuery,
    slot_names: Vec<String>,
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
}

// Forward declarations — implemented below
fn validate_no_attribute_slots(query: &DatalogQuery) -> Result<()>;
fn collect_slot_names(query: &DatalogQuery) -> Vec<String>;
fn substitute(template: &DatalogQuery, bindings: &HashMap<&str, &BindValue>) -> Result<DatalogQuery>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{BinOp, FindSpec, Pattern};

    fn make_query_entity_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?name".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::BindSlot("entity".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::Symbol("?name".to_string()),
            ))],
        )
    }

    fn make_query_value_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?e".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::BindSlot("name".to_string()),
            ))],
        )
    }

    fn make_query_attr_slot() -> DatalogQuery {
        // This should be REJECTED at prepare time
        DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::BindSlot("attr".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        )
    }

    fn make_query_as_of_slot() -> DatalogQuery {
        let mut q = DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":score".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        );
        q.as_of = Some(AsOf::Slot("tx".to_string()));
        q
    }

    fn make_query_valid_at_slot() -> DatalogQuery {
        let mut q = DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":score".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        );
        q.valid_at = Some(ValidAt::Slot("date".to_string()));
        q
    }

    fn make_query_expr_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":score".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                )),
                WhereClause::Expr {
                    expr: Expr::BinOp(
                        BinOp::Gte,
                        Box::new(Expr::Var("?v".to_string())),
                        Box::new(Expr::Slot("threshold".to_string())),
                    ),
                    binding: None,
                },
            ],
        )
    }

    // ── validate_no_attribute_slots ──────────────────────────────────────────

    #[test]
    fn test_validate_rejects_attribute_slot() {
        let q = make_query_attr_slot();
        let result = validate_no_attribute_slots(&q);
        assert!(result.is_err(), "expected error for attribute slot");
        assert!(result.unwrap_err().to_string().contains("attribute position"));
    }

    #[test]
    fn test_validate_accepts_entity_slot() {
        let q = make_query_entity_slot();
        assert!(validate_no_attribute_slots(&q).is_ok());
    }

    #[test]
    fn test_validate_accepts_value_slot() {
        let q = make_query_value_slot();
        assert!(validate_no_attribute_slots(&q).is_ok());
    }

    // ── collect_slot_names ───────────────────────────────────────────────────

    #[test]
    fn test_collect_entity_slot_name() {
        let q = make_query_entity_slot();
        let names = collect_slot_names(&q);
        assert_eq!(names, vec!["entity"]);
    }

    #[test]
    fn test_collect_value_slot_name() {
        let q = make_query_value_slot();
        let names = collect_slot_names(&q);
        assert_eq!(names, vec!["name"]);
    }

    #[test]
    fn test_collect_as_of_slot_name() {
        let q = make_query_as_of_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"tx".to_string()));
    }

    #[test]
    fn test_collect_valid_at_slot_name() {
        let q = make_query_valid_at_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"date".to_string()));
    }

    #[test]
    fn test_collect_expr_slot_name() {
        let q = make_query_expr_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"threshold".to_string()));
    }

    #[test]
    fn test_collect_deduplicates() {
        // $entity appears in two patterns
        let q = DatalogQuery::new(
            vec![
                FindSpec::Variable("?name".to_string()),
                FindSpec::Variable("?age".to_string()),
            ],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::BindSlot("entity".to_string()),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::Symbol("?name".to_string()),
                )),
                WhereClause::Pattern(Pattern::new(
                    EdnValue::BindSlot("entity".to_string()),
                    EdnValue::Keyword(":person/age".to_string()),
                    EdnValue::Symbol("?age".to_string()),
                )),
            ],
        );
        let names = collect_slot_names(&q);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "entity");
    }

    // ── substitute ───────────────────────────────────────────────────────────

    #[test]
    fn test_substitute_entity_slot() {
        let q = make_query_entity_slot();
        let uuid = Uuid::new_v4();
        let bv = BindValue::Entity(uuid);
        let bindings: HashMap<&str, &BindValue> = [("entity", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        match &filled.where_clauses[0] {
            WhereClause::Pattern(p) => {
                assert_eq!(p.entity, EdnValue::Uuid(uuid));
            }
            _ => panic!("expected Pattern"),
        }
    }

    #[test]
    fn test_substitute_value_slot() {
        let q = make_query_value_slot();
        let bv = BindValue::Val(Value::String("Alice".to_string()));
        let bindings: HashMap<&str, &BindValue> = [("name", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        match &filled.where_clauses[0] {
            WhereClause::Pattern(p) => {
                assert_eq!(p.value, EdnValue::String("Alice".to_string()));
            }
            _ => panic!("expected Pattern"),
        }
    }

    #[test]
    fn test_substitute_as_of_counter() {
        let q = make_query_as_of_slot();
        let bv = BindValue::TxCount(42);
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.as_of, Some(AsOf::Counter(42))));
    }

    #[test]
    fn test_substitute_as_of_timestamp() {
        let q = make_query_as_of_slot();
        let bv = BindValue::Timestamp(1_685_577_600_000);
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.as_of, Some(AsOf::Timestamp(1_685_577_600_000))));
    }

    #[test]
    fn test_substitute_valid_at_timestamp() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::Timestamp(1_685_577_600_000);
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.valid_at, Some(ValidAt::Timestamp(1_685_577_600_000))));
    }

    #[test]
    fn test_substitute_valid_at_any() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::AnyValidTime;
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.valid_at, Some(ValidAt::AnyValidTime)));
    }

    #[test]
    fn test_substitute_expr_slot() {
        let q = make_query_expr_slot();
        let bv = BindValue::Val(Value::Integer(50));
        let bindings: HashMap<&str, &BindValue> = [("threshold", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        // The Expr::Slot in the BinOp rhs should now be Expr::Lit(Value::Integer(50))
        match &filled.where_clauses[1] {
            WhereClause::Expr { expr: Expr::BinOp(_, _, rhs), .. } => {
                assert!(matches!(rhs.as_ref(), Expr::Lit(Value::Integer(50))));
            }
            _ => panic!("expected BinOp Expr clause"),
        }
    }

    #[test]
    fn test_substitute_type_mismatch_entity_gets_val() {
        let q = make_query_entity_slot();
        let bv = BindValue::Val(Value::String("Alice".to_string()));
        let bindings: HashMap<&str, &BindValue> = [("entity", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(result.unwrap_err().to_string().contains("entity position"));
    }

    #[test]
    fn test_substitute_type_mismatch_as_of_gets_val() {
        let q = make_query_as_of_slot();
        let bv = BindValue::Val(Value::Integer(42));
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(result.unwrap_err().to_string().contains(":as-of position"));
    }

    #[test]
    fn test_substitute_type_mismatch_valid_at_gets_tx_count() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::TxCount(5);
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(result.unwrap_err().to_string().contains(":valid-at position"));
    }
}
```

- [ ] **Step 2: Run to verify tests fail**

Add `pub mod prepared;` to `src/query/datalog/mod.rs` first:

```rust
pub mod prepared;
```

Then run:

```bash
cargo test --lib -- prepared::tests 2>&1 | tail -15
```

Expected: compile errors — functions not yet implemented.

- [ ] **Step 3: Implement the module**

Replace the forward declarations in `prepared.rs` with the full implementation:

```rust
use crate::graph::types::Value;
use crate::graph::FactStorage;
use crate::query::datalog::executor::{DatalogExecutor, QueryResult};
use crate::query::datalog::functions::FunctionRegistry;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogCommand, DatalogQuery, EdnValue, Expr, ValidAt, WhereClause,
};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ─── BindValue ────────────────────────────────────────────────────────────────

/// A concrete value supplied to a named bind slot (`$name`) in a `PreparedQuery`.
#[derive(Debug, Clone)]
pub enum BindValue {
    /// Substituted into an entity position: `[$entity :attr ?v]`.
    Entity(Uuid),
    /// Substituted into a value position `[?e :attr $val]` or an expression literal.
    Val(Value),
    /// Substituted into an `:as-of $tx` slot (monotonic transaction counter).
    TxCount(u64),
    /// Substituted into an `:as-of $tx` slot (wall-clock millis) or `:valid-at $date` slot.
    Timestamp(i64),
    /// Substituted into a `:valid-at $date` slot — disables valid-time filtering.
    AnyValidTime,
}

fn bind_value_type_name(bv: &BindValue) -> &'static str {
    match bv {
        BindValue::Entity(_) => "Entity",
        BindValue::Val(_) => "Val",
        BindValue::TxCount(_) => "TxCount",
        BindValue::Timestamp(_) => "Timestamp",
        BindValue::AnyValidTime => "AnyValidTime",
    }
}

// ─── PreparedQuery ────────────────────────────────────────────────────────────

/// A parsed and optimized query template with named bind slots (`$name`).
///
/// Obtain via [`crate::db::Minigraf::prepare`].
/// Execute many times via [`PreparedQuery::execute`].
pub struct PreparedQuery {
    template: DatalogQuery,
    slot_names: Vec<String>,
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
}

impl PreparedQuery {
    /// Substitute bind values and execute the query against the current database state.
    ///
    /// Slot names in `bindings` that are not referenced by the prepared query are silently ignored.
    ///
    /// # Errors
    /// - Missing bind value for a slot present in the query.
    /// - Type mismatch (e.g. `Val` supplied for an `:as-of` slot).
    pub fn execute(&self, bindings: &[(&str, BindValue)]) -> Result<QueryResult> {
        let binding_map: HashMap<&str, &BindValue> =
            bindings.iter().map(|(name, val)| (*name, val)).collect();

        // Completeness check
        for name in &self.slot_names {
            if !binding_map.contains_key(name.as_str()) {
                anyhow::bail!("missing bind value for slot '${}'", name);
            }
        }

        // Clone + substitute
        let filled_query = substitute(&self.template, &binding_map)?;

        // Execute against the current fact store state
        let executor = DatalogExecutor::new_with_rules_and_functions(
            self.fact_storage.clone(),
            self.rules.clone(),
            self.functions.clone(),
        );
        executor.execute(DatalogCommand::Query(filled_query))
    }
}

// ─── Internal constructor ─────────────────────────────────────────────────────

/// Parse-time constructor called by `Minigraf::prepare()`.
pub(crate) fn prepare_query(
    query: DatalogQuery,
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
) -> Result<PreparedQuery> {
    validate_no_attribute_slots(&query)?;
    let slot_names = collect_slot_names(&query);
    Ok(PreparedQuery {
        template: query,
        slot_names,
        fact_storage,
        rules,
        functions,
    })
}

// ─── Validation ───────────────────────────────────────────────────────────────

fn validate_no_attribute_slots(query: &DatalogQuery) -> Result<()> {
    validate_clauses_no_attr_slots(&query.where_clauses)
}

fn validate_clauses_no_attr_slots(clauses: &[WhereClause]) -> Result<()> {
    for clause in clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let AttributeSpec::Real(EdnValue::BindSlot(name)) = &p.attribute {
                    anyhow::bail!(
                        "bind slot '${name}' is not permitted in attribute position; \
                         the query optimizer selects an index based on the attribute at \
                         prepare time and cannot handle a parameterised attribute"
                    );
                }
            }
            WhereClause::Not(inner) => validate_clauses_no_attr_slots(inner)?,
            WhereClause::NotJoin { clauses: inner, .. } => {
                validate_clauses_no_attr_slots(inner)?
            }
            WhereClause::Or(branches) => {
                for b in branches {
                    validate_clauses_no_attr_slots(b)?;
                }
            }
            WhereClause::OrJoin { branches, .. } => {
                for b in branches {
                    validate_clauses_no_attr_slots(b)?;
                }
            }
            WhereClause::Expr { .. } | WhereClause::RuleInvocation { .. } => {}
        }
    }
    Ok(())
}

// ─── Slot collection ──────────────────────────────────────────────────────────

fn collect_slot_names(query: &DatalogQuery) -> Vec<String> {
    let mut names: HashSet<String> = HashSet::new();
    if let Some(AsOf::Slot(name)) = &query.as_of {
        names.insert(name.clone());
    }
    if let Some(ValidAt::Slot(name)) = &query.valid_at {
        names.insert(name.clone());
    }
    collect_slots_from_clauses(&query.where_clauses, &mut names);
    let mut result: Vec<String> = names.into_iter().collect();
    result.sort(); // deterministic order for tests
    result
}

fn collect_slots_from_clauses(clauses: &[WhereClause], names: &mut HashSet<String>) {
    for clause in clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let EdnValue::BindSlot(name) = &p.entity {
                    names.insert(name.clone());
                }
                if let EdnValue::BindSlot(name) = &p.value {
                    names.insert(name.clone());
                }
            }
            WhereClause::Not(inner) => collect_slots_from_clauses(inner, names),
            WhereClause::NotJoin { clauses: inner, .. } => {
                collect_slots_from_clauses(inner, names)
            }
            WhereClause::Or(branches) => {
                for b in branches {
                    collect_slots_from_clauses(b, names);
                }
            }
            WhereClause::OrJoin { branches, .. } => {
                for b in branches {
                    collect_slots_from_clauses(b, names);
                }
            }
            WhereClause::Expr { expr, .. } => collect_slots_from_expr(expr, names),
            WhereClause::RuleInvocation { args, .. } => {
                for arg in args {
                    if let EdnValue::BindSlot(name) = arg {
                        names.insert(name.clone());
                    }
                }
            }
        }
    }
}

fn collect_slots_from_expr(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::Slot(name) => {
            names.insert(name.clone());
        }
        Expr::BinOp(_, l, r) => {
            collect_slots_from_expr(l, names);
            collect_slots_from_expr(r, names);
        }
        Expr::UnaryOp(_, arg) => collect_slots_from_expr(arg, names),
        Expr::Var(_) | Expr::Lit(_) => {}
    }
}

// ─── Substitution ─────────────────────────────────────────────────────────────

fn substitute(template: &DatalogQuery, bindings: &HashMap<&str, &BindValue>) -> Result<DatalogQuery> {
    let mut query = template.clone();

    // :as-of slot
    if let Some(AsOf::Slot(name)) = &query.as_of.clone() {
        query.as_of = Some(resolve_as_of_slot(name, bindings)?);
    }

    // :valid-at slot
    if let Some(ValidAt::Slot(name)) = &query.valid_at.clone() {
        query.valid_at = Some(resolve_valid_at_slot(name, bindings)?);
    }

    // where clauses
    for clause in &mut query.where_clauses {
        substitute_where_clause(clause, bindings)?;
    }

    Ok(query)
}

fn substitute_where_clause(
    clause: &mut WhereClause,
    bindings: &HashMap<&str, &BindValue>,
) -> Result<()> {
    match clause {
        WhereClause::Pattern(p) => substitute_pattern(p, bindings),
        WhereClause::Not(clauses) => {
            for c in clauses {
                substitute_where_clause(c, bindings)?;
            }
            Ok(())
        }
        WhereClause::NotJoin { clauses, .. } => {
            for c in clauses {
                substitute_where_clause(c, bindings)?;
            }
            Ok(())
        }
        WhereClause::Or(branches) => {
            for branch in branches {
                for c in branch {
                    substitute_where_clause(c, bindings)?;
                }
            }
            Ok(())
        }
        WhereClause::OrJoin { branches, .. } => {
            for branch in branches {
                for c in branch {
                    substitute_where_clause(c, bindings)?;
                }
            }
            Ok(())
        }
        WhereClause::Expr { expr, .. } => substitute_expr(expr, bindings),
        WhereClause::RuleInvocation { args, .. } => {
            for arg in args {
                substitute_edn_value(arg, bindings)?;
            }
            Ok(())
        }
    }
}

fn substitute_pattern(
    p: &mut crate::query::datalog::types::Pattern,
    bindings: &HashMap<&str, &BindValue>,
) -> Result<()> {
    if let EdnValue::BindSlot(name) = &p.entity.clone() {
        p.entity = resolve_entity_slot(name, bindings)?;
    }
    if let EdnValue::BindSlot(name) = &p.value.clone() {
        p.value = resolve_value_slot(name, bindings)?;
    }
    Ok(())
}

fn substitute_expr(expr: &mut Expr, bindings: &HashMap<&str, &BindValue>) -> Result<()> {
    match expr {
        Expr::Slot(name) => {
            let name = name.clone();
            *expr = Expr::Lit(resolve_val_slot(&name, bindings)?);
            Ok(())
        }
        Expr::BinOp(_, lhs, rhs) => {
            substitute_expr(lhs, bindings)?;
            substitute_expr(rhs, bindings)
        }
        Expr::UnaryOp(_, arg) => substitute_expr(arg, bindings),
        Expr::Var(_) | Expr::Lit(_) => Ok(()),
    }
}

fn substitute_edn_value(val: &mut EdnValue, bindings: &HashMap<&str, &BindValue>) -> Result<()> {
    if let EdnValue::BindSlot(name) = val.clone() {
        *val = resolve_value_slot(&name, bindings)?;
    }
    Ok(())
}

// ─── Slot resolvers ───────────────────────────────────────────────────────────

fn resolve_entity_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<EdnValue> {
    match bindings.get(name) {
        Some(BindValue::Entity(u)) => Ok(EdnValue::Uuid(*u)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in entity position requires Entity, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_value_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<EdnValue> {
    match bindings.get(name) {
        Some(BindValue::Val(v)) => Ok(value_to_edn(v)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in value position requires Val, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_val_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<Value> {
    match bindings.get(name) {
        Some(BindValue::Val(v)) => Ok(v.clone()),
        Some(other) => anyhow::bail!(
            "slot '${name}' in expression position requires Val, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_as_of_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<AsOf> {
    match bindings.get(name) {
        Some(BindValue::TxCount(n)) => Ok(AsOf::Counter(*n)),
        Some(BindValue::Timestamp(t)) => Ok(AsOf::Timestamp(*t)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in :as-of position requires TxCount or Timestamp, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_valid_at_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<ValidAt> {
    match bindings.get(name) {
        Some(BindValue::Timestamp(t)) => Ok(ValidAt::Timestamp(*t)),
        Some(BindValue::AnyValidTime) => Ok(ValidAt::AnyValidTime),
        Some(other) => anyhow::bail!(
            "slot '${name}' in :valid-at position requires Timestamp or AnyValidTime, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

// ─── Value ↔ EdnValue conversion ─────────────────────────────────────────────

fn value_to_edn(v: &Value) -> EdnValue {
    match v {
        Value::String(s) => EdnValue::String(s.clone()),
        Value::Integer(i) => EdnValue::Integer(*i),
        Value::Float(f) => EdnValue::Float(*f),
        Value::Boolean(b) => EdnValue::Boolean(*b),
        Value::Keyword(k) => EdnValue::Keyword(k.clone()),
        Value::Ref(u) => EdnValue::Uuid(*u),
        Value::Null => EdnValue::Nil,
    }
}
```

- [ ] **Step 4: Run unit tests**

```bash
cargo test --lib -- prepared::tests 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Verify no regressions**

```bash
cargo test 2>&1 | tail -10
```

Expected: all previously passing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/prepared.rs src/query/datalog/mod.rs
git commit -m "feat(prepared): add BindValue, PreparedQuery, validation, slot collection, and substitution"
```

---

## Task 5: Wire up `Minigraf::prepare()` and re-exports

**Files:**
- Modify: `src/db.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add `Minigraf::prepare()` to `db.rs`**

Find the `impl Minigraf` block in `src/db.rs`. After the `checkpoint()` method, add:

```rust
    /// Parse and plan a query once; bind slots (`$name`) are left unresolved.
    ///
    /// Returns a [`PreparedQuery`] that can be executed many times with different
    /// bind values via [`PreparedQuery::execute`].
    ///
    /// # Errors
    /// - Parse failure.
    /// - A bind slot appears in an attribute position (rejected at prepare time).
    /// - The command is not a `(query ...)` — `transact`, `retract`, and `rule`
    ///   are not preparable.
    pub fn prepare(
        &self,
        query_str: &str,
    ) -> Result<crate::query::datalog::prepared::PreparedQuery> {
        use crate::query::datalog::prepared::prepare_query;
        use crate::query::datalog::types::DatalogCommand;

        let cmd =
            parse_datalog_command(query_str).map_err(|e| anyhow::anyhow!("{}", e))?;

        let query = match cmd {
            DatalogCommand::Query(q) => q,
            DatalogCommand::Transact(_) => {
                anyhow::bail!(
                    "only (query ...) commands can be prepared; got transact"
                )
            }
            DatalogCommand::Retract(_) => {
                anyhow::bail!(
                    "only (query ...) commands can be prepared; got retract"
                )
            }
            DatalogCommand::Rule(_) => {
                anyhow::bail!(
                    "only (query ...) commands can be prepared; got rule"
                )
            }
        };

        prepare_query(
            query,
            self.inner.fact_storage.clone(),
            self.inner.rules.clone(),
            self.inner.functions.clone(),
        )
    }
```

- [ ] **Step 2: Add re-exports to `src/lib.rs`**

Add at the bottom of `src/lib.rs`:

```rust
// Prepared statements (Phase 7.8)
pub use query::datalog::prepared::{BindValue, PreparedQuery};
```

- [ ] **Step 3: Verify compilation**

```bash
cargo build 2>&1 | grep "^error"
```

Expected: zero errors.

- [ ] **Step 4: Verify no regressions**

```bash
cargo test 2>&1 | tail -10
```

Expected: all previously passing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs src/lib.rs
git commit -m "feat(db): add Minigraf::prepare() and re-export PreparedQuery + BindValue"
```

---

## Task 6: Integration tests

**Files:**
- Create: `tests/prepared_statements_test.rs`

- [ ] **Step 1: Write all integration tests**

Create `tests/prepared_statements_test.rs`:

```rust
use minigraf::db::Minigraf;
use minigraf::query::datalog::executor::QueryResult;
use minigraf::{BindValue, Value};
use uuid::Uuid;

// ─── Happy-path tests ─────────────────────────────────────────────────────────

#[test]
fn prepare_and_execute_entity_slot() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :person/name "Alice"]
                      [#uuid "{bob}"  :person/name "Bob"]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    let r1 = prepared
        .execute(&[("entity", BindValue::Entity(alice))])
        .unwrap();
    match r1 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }

    let r2 = prepared
        .execute(&[("entity", BindValue::Entity(bob))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Bob".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_value_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?e :where [?e :person/name $name]])")
        .unwrap();

    let r = prepared
        .execute(&[("name", BindValue::Val(Value::String("Alice".to_string())))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }

    let r2 = prepared
        .execute(&[("name", BindValue::Val(Value::String("Bob".to_string())))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_as_of_counter() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice-v2"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :as-of $tx :where [?e :person/name ?name]])")
        .unwrap();

    // At tx 1 only "Alice" exists
    let r = prepared
        .execute(&[("tx", BindValue::TxCount(1))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], Value::String("Alice".to_string()));
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_as_of_timestamp() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    // Use a very large timestamp — all facts should be visible
    let prepared = db
        .prepare("(query [:find ?name :as-of $ts :where [?e :person/name ?name]])")
        .unwrap();

    let r = prepared
        .execute(&[("ts", BindValue::Timestamp(i64::MAX))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert!(!results.is_empty());
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_valid_at() {
    let db = Minigraf::in_memory().unwrap();
    let t1: i64 = 1_000_000_000_000;
    let t2: i64 = 2_000_000_000_000;
    let t3: i64 = 3_000_000_000_000;

    db.execute(&format!(
        "(transact [[:alice :employment/status :active] \
                    {{:valid-from {t1} :valid-to {t3}}}])"
    ))
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?s :valid-at $date :where [?e :employment/status ?s]])")
        .unwrap();

    // Inside the valid window
    let r = prepared
        .execute(&[("date", BindValue::Timestamp(t2))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }

    // Before the valid window
    let r2 = prepared
        .execute(&[("date", BindValue::Timestamp(t1 - 1))])
        .unwrap();
    match r2 {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 0);
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_valid_at_any() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :valid-at $va :any-valid-time :where [?e :person/name ?name]])")
        .unwrap();

    let r = prepared
        .execute(&[("va", BindValue::AnyValidTime)])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert!(!results.is_empty());
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_expr_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:a :score 10] [:b :score 50] [:c :score 90]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:any-valid-time :find ?e :where [?e :score ?v] [(>= ?v $threshold)]])")
        .unwrap();

    let r = prepared
        .execute(&[("threshold", BindValue::Val(Value::Integer(50)))])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 2); // :b (50) and :c (90)
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn prepare_and_execute_combined() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :employment/status :active]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare(
            "(query [:find ?s \
                     :as-of $tx \
                     :valid-at $date \
                     :where [$entity :employment/status ?s] \
                            [(= ?s $expected-status)]])",
        )
        .unwrap();

    let r = prepared
        .execute(&[
            ("tx", BindValue::TxCount(100)),
            ("date", BindValue::Timestamp(i64::MAX)),
            ("entity", BindValue::Entity(alice)),
            ("expected-status", BindValue::Val(Value::Keyword(":active".to_string()))),
        ])
        .unwrap();
    match r {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!("expected QueryResults"),
    }
}

#[test]
fn plan_reused_across_executions() {
    let db = Minigraf::in_memory().unwrap();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let carol = Uuid::new_v4();

    db.execute(&format!(
        r#"(transact [[#uuid "{alice}" :person/name "Alice"]
                      [#uuid "{bob}"   :person/name "Bob"]
                      [#uuid "{carol}" :person/name "Carol"]])"#
    ))
    .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    for (uuid, expected) in [
        (alice, "Alice"),
        (bob, "Bob"),
        (carol, "Carol"),
    ] {
        let r = prepared
            .execute(&[("entity", BindValue::Entity(uuid))])
            .unwrap();
        match r {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0][0], Value::String(expected.to_string()));
            }
            _ => panic!("expected QueryResults"),
        }
    }
}

// ─── Error tests ──────────────────────────────────────────────────────────────

#[test]
fn prepare_rejects_attribute_slot() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare("(query [:find ?v :where [?e $attr ?v]])");
    assert!(result.is_err(), "expected error for attribute slot");
    assert!(
        result.unwrap_err().to_string().contains("attribute position"),
        "error should mention attribute position"
    );
}

#[test]
fn prepare_rejects_transact() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare(r#"(transact [[:alice :person/name "Alice"]])"#);
    assert!(result.is_err(), "expected error for transact");
    assert!(result.unwrap_err().to_string().contains("transact"));
}

#[test]
fn prepare_rejects_retract() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare(r#"(retract [[:alice :person/name "Alice"]])"#);
    assert!(result.is_err(), "expected error for retract");
    assert!(result.unwrap_err().to_string().contains("retract"));
}

#[test]
fn prepare_rejects_rule() {
    let db = Minigraf::in_memory().unwrap();
    let result = db.prepare("(rule [(reachable ?a ?b) [?a :edge ?b]])");
    assert!(result.is_err(), "expected error for rule");
    assert!(result.unwrap_err().to_string().contains("rule"));
}

#[test]
fn execute_missing_slot() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    // Intentionally omit the "entity" binding
    let result = prepared.execute(&[]);
    assert!(result.is_err(), "expected error for missing slot");
    assert!(
        result.unwrap_err().to_string().contains("entity"),
        "error should mention the missing slot name"
    );
}

#[test]
fn execute_type_mismatch_as_of() {
    let db = Minigraf::in_memory().unwrap();
    let prepared = db
        .prepare("(query [:find ?v :as-of $tx :where [?e :score ?v]])")
        .unwrap();

    let result = prepared.execute(&[("tx", BindValue::Val(Value::Integer(42)))]);
    assert!(result.is_err(), "expected type mismatch error");
    assert!(
        result.unwrap_err().to_string().contains(":as-of position"),
        "error should mention :as-of position"
    );
}

#[test]
fn execute_type_mismatch_entity() {
    let db = Minigraf::in_memory().unwrap();
    let prepared = db
        .prepare("(query [:find ?name :where [$entity :person/name ?name]])")
        .unwrap();

    let result = prepared.execute(&[("entity", BindValue::Val(Value::String("not-a-uuid".to_string())))]);
    assert!(result.is_err(), "expected type mismatch error");
    assert!(
        result.unwrap_err().to_string().contains("entity position"),
        "error should mention entity position"
    );
}

#[test]
fn execute_extra_bindings_ignored() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
        .unwrap();

    let prepared = db
        .prepare("(query [:find ?name :where [:alice :person/name ?name]])")
        .unwrap();

    // Provide an extra binding that the query doesn't reference
    let result = prepared.execute(&[("unused-slot", BindValue::Val(Value::Integer(99)))]);
    assert!(result.is_ok(), "extra bindings should be silently ignored");
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --test prepared_statements_test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 3: Run the full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass, no regressions.

- [ ] **Step 4: Commit**

```bash
git add tests/prepared_statements_test.rs
git commit -m "test(prepared): add integration tests for PreparedQuery (entity, value, temporal, expr slots)"
```

---

## Self-Review

**Spec coverage check:**

| Spec section | Covered by task |
|---|---|
| `EdnValue::BindSlot`, `AsOf::Slot`, `ValidAt::Slot`, `Expr::Slot` | Task 1 |
| `BindValue` enum | Task 4 |
| `PreparedQuery` struct | Task 4 |
| `validate_no_attribute_slots` | Task 4 |
| `collect_slot_names` | Task 4 |
| `substitute` (clone + fill) | Task 4 |
| Parser: `$identifier` tokenization | Task 3 |
| Parser: `:as-of $slot`, `:valid-at $slot` | Task 3 |
| Parser: `$slot` in `parse_expr_arg` | Task 3 |
| Panic guards in executor + storage | Task 2 |
| `Minigraf::prepare()` | Task 5 |
| Re-exports | Task 5 |
| Integration tests (all 14 from spec) | Task 6 |

**Placeholder scan:** No TBDs or "implement later" stubs.

**Type consistency:** `BindValue`, `PreparedQuery`, `prepare_query`, `substitute` — names consistent across all tasks. `resolve_entity_slot`, `resolve_value_slot`, `resolve_val_slot`, `resolve_as_of_slot`, `resolve_valid_at_slot` — defined in Task 4 and not referenced before.

---

One implementation note for the `prepare_and_execute_valid_at_any` test: the query string uses `:valid-at $va :any-valid-time`. The parser currently treats `:any-valid-time` as a standalone shorthand (line ~712), not as a value for `:valid-at`. If that conflicts, simplify the test to use only `:any-valid-time` without the `$va` slot, or restructure the query to not parameterise `valid-at` in that specific test (test AnyValidTime substitution via unit tests instead). The unit tests in Task 4 already cover `ValidAt::AnyValidTime` substitution directly.
