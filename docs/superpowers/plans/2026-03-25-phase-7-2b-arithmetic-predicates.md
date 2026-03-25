# Phase 7.2b — Arithmetic & Predicate Expressions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `[(< ?v 100)]`-style filter predicates and `[(+ ?a ?b) ?result]`-style arithmetic bindings to Datalog `:where` clauses, backed by a unified `Expr` AST.

**Architecture:** A new `Expr` AST (`BinOp`/`UnaryOp`/`Lit`/`Var`) stored in a new `WhereClause::Expr` variant. The parser recognises `[(list-expr) ?out?]` vectors, validates at parse time (including regex compilation for `matches?`), and the executor applies expr clauses as a post-filter/binding pass after pattern matching — in both the plain query and rules-with-negation execution paths.

**Tech Stack:** Rust, `regex-lite = "0.1"` (already in `Cargo.toml`). Test runner: `cargo test`.

---

## File Map

| File | Role |
|---|---|
| `src/query/datalog/types.rs` | Add `BinOp`, `UnaryOp`, `Expr` enums; add `WhereClause::Expr` variant; patch exhaustive matches |
| `src/query/datalog/parser.rs` | Add `parse_expr_arg`, `parse_expr`, `parse_expr_clause`; update `:where` dispatch at two sites; update not/not-join body dispatch; update `outer_vars_from_clause`; add `check_expr_safety` |
| `src/query/datalog/executor.rs` | Add `eval_expr`, `eval_binop`, `to_float_pair`, `is_truthy`, `apply_expr_clauses`, `apply_expr_to_not_body`; wire into `execute_query` and `execute_query_with_rules` |
| `src/query/datalog/evaluator.rs` | Update `evaluate_rule` to apply exprs after pattern match; update `evaluate_not_join` to handle `WhereClause::Expr` |
| `tests/predicate_expr_test.rs` | New integration test file |

---

## Task 1: Types — `BinOp`, `UnaryOp`, `Expr`, `WhereClause::Expr`

**Files:**
- Modify: `src/query/datalog/types.rs`

### Background

`types.rs` currently only imports `uuid::Uuid`. `Expr::Lit` wraps `Value` (the graph type, `crate::graph::types::Value`) so the executor can work with bindings directly.

The existing `WhereClause` enum has four exhaustive-match sites that must each gain a new arm:
- `rule_invocations()` — line ~245
- `has_negated_invocation()` — line ~259
- `collect_rule_invocations_recursive()` — line ~339
- `outer_vars_from_clause()` in `parser.rs` — handled in Task 3

- [ ] **Step 1: Write failing unit tests for the new types**

Add inside the existing `#[cfg(test)] mod tests` block at the bottom of `types.rs`:

```rust
#[test]
fn test_binop_variants_exist() {
    let _ = BinOp::Lt;
    let _ = BinOp::Gt;
    let _ = BinOp::Lte;
    let _ = BinOp::Gte;
    let _ = BinOp::Eq;
    let _ = BinOp::Neq;
    let _ = BinOp::Add;
    let _ = BinOp::Sub;
    let _ = BinOp::Mul;
    let _ = BinOp::Div;
    let _ = BinOp::StartsWith;
    let _ = BinOp::EndsWith;
    let _ = BinOp::Contains;
    let _ = BinOp::Matches;
}

#[test]
fn test_unary_op_variants_exist() {
    let _ = UnaryOp::StringQ;
    let _ = UnaryOp::IntegerQ;
    let _ = UnaryOp::FloatQ;
    let _ = UnaryOp::BooleanQ;
    let _ = UnaryOp::NilQ;
}

#[test]
fn test_expr_var_and_lit() {
    use crate::graph::types::Value;
    let e = Expr::Var("?x".to_string());
    assert!(matches!(e, Expr::Var(_)));
    let l = Expr::Lit(Value::Integer(42));
    assert!(matches!(l, Expr::Lit(_)));
}

#[test]
fn test_expr_binop_nested() {
    use crate::graph::types::Value;
    let e = Expr::BinOp(
        BinOp::Add,
        Box::new(Expr::Var("?a".to_string())),
        Box::new(Expr::Lit(Value::Integer(1))),
    );
    assert!(matches!(e, Expr::BinOp(BinOp::Add, _, _)));
}

#[test]
fn test_where_clause_expr_filter_variant() {
    use crate::graph::types::Value;
    let clause = WhereClause::Expr {
        expr: Expr::BinOp(
            BinOp::Lt,
            Box::new(Expr::Var("?v".to_string())),
            Box::new(Expr::Lit(Value::Integer(100))),
        ),
        binding: None,
    };
    assert!(matches!(clause, WhereClause::Expr { binding: None, .. }));
}

#[test]
fn test_where_clause_expr_binding_variant() {
    use crate::graph::types::Value;
    let clause = WhereClause::Expr {
        expr: Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Var("?a".to_string())),
            Box::new(Expr::Var("?b".to_string())),
        ),
        binding: Some("?sum".to_string()),
    };
    assert!(matches!(clause, WhereClause::Expr { binding: Some(_), .. }));
}

#[test]
fn test_expr_clause_rule_invocations_empty() {
    use crate::graph::types::Value;
    let clause = WhereClause::Expr {
        expr: Expr::Lit(Value::Boolean(true)),
        binding: None,
    };
    assert!(clause.rule_invocations().is_empty());
    assert!(!clause.has_negated_invocation());
}
```

- [ ] **Step 2: Run tests — expect compile errors (types don't exist yet)**

```bash
cargo test --lib 2>&1 | head -30
```

Expected: compile errors about `BinOp`, `UnaryOp`, `Expr` not found.

- [ ] **Step 3: Add the new types to `types.rs`**

At the top of `types.rs`, add the import:

```rust
use crate::graph::types::Value;
```

After the existing `AggFunc` enum (around line 98), add:

```rust
/// Binary operators for expression clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    // Comparisons — return Boolean
    Lt, Gt, Lte, Gte, Eq, Neq,
    // Arithmetic — return numeric Value (Integer or Float, with int/float promotion)
    Add, Sub, Mul, Div,
    // String predicates — return Boolean
    StartsWith, EndsWith, Contains,
    /// Pattern must be a string literal validated at parse time via regex-lite.
    Matches,
}

/// Unary type-predicate operators — always return Boolean.
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    StringQ, IntegerQ, FloatQ, BooleanQ, NilQ,
}

/// Composable expression tree for `WhereClause::Expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Logic variable reference: `?v`
    Var(String),
    /// Literal value: `100`, `"foo"`, `true`
    Lit(Value),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}
```

- [ ] **Step 4: Add `WhereClause::Expr` variant**

In the `WhereClause` enum, after `NotJoin`, add:

```rust
/// Expression clause: `[(expr) ?out?]`
///
/// `binding = None`  → filter: keep binding iff `expr` evaluates to truthy.
/// `binding = Some`  → bind the result `Value` to the named variable.
///
/// Type mismatches and division by zero silently drop the row.
Expr { expr: Expr, binding: Option<String> },
```

- [ ] **Step 5: Patch the three exhaustive matches**

In `rule_invocations()`, add before the closing `}`:
```rust
WhereClause::Expr { .. } => vec![],
```

In `has_negated_invocation()`, add before the closing `}`:
```rust
WhereClause::Expr { .. } => false,
```

In `collect_rule_invocations_recursive()`, add before the closing `}` of the `for` loop match:
```rust
WhereClause::Expr { .. } => {}
```

- [ ] **Step 6: Run tests — all should pass**

```bash
cargo test --lib 2>&1 | tail -20
```

Expected: all existing tests pass plus the 7 new ones.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat(types): add BinOp, UnaryOp, Expr, WhereClause::Expr for Phase 7.2b"
```

---

## Task 2: Parser — `parse_expr_arg`, `parse_expr`, `parse_expr_clause`

**Files:**
- Modify: `src/query/datalog/parser.rs`

### Background

`parser.rs` imports `use super::types::*` so all new types are already in scope. Add `use crate::graph::types::Value;` after the existing imports.

Three new private functions:

- `parse_expr_arg(&EdnValue) -> Result<Expr, String>` — converts a single token to an `Expr` leaf or recurses for nested lists
- `parse_expr(&[EdnValue]) -> Result<Expr, String>` — converts a full list `(op arg arg?)` to an `Expr` tree; validates regex at parse time
- `parse_expr_clause(&[EdnValue]) -> Result<WhereClause, String>` — wraps `parse_expr` into a `WhereClause::Expr`

- [ ] **Step 1: Write failing unit tests for `parse_expr`**

Add to the existing `#[cfg(test)]` block at the bottom of `parser.rs`:

```rust
#[test]
fn test_parse_expr_lt_filter() {
    // [(< ?v 100)] — filter clause
    let input = "(query [:find ?e :where [?e :item/price ?v] [(< ?v 100)]])";
    let result = parse(input);
    assert!(result.is_ok(), "parse failed");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert_eq!(q.where_clauses.len(), 2);
            assert!(matches!(q.where_clauses[1], WhereClause::Expr { binding: None, .. }));
        }
        _ => panic!("expected query"),
    }
}

#[test]
fn test_parse_expr_add_binding() {
    // [(+ ?a ?b) ?sum] — binding clause
    let input = "(query [:find ?sum :where [?e :x ?a] [?e :y ?b] [(+ ?a ?b) ?sum]])";
    let result = parse(input);
    assert!(result.is_ok(), "parse failed");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert_eq!(q.where_clauses.len(), 3);
            assert!(matches!(
                q.where_clauses[2],
                WhereClause::Expr { binding: Some(_), .. }
            ));
        }
        _ => panic!("expected query"),
    }
}

#[test]
fn test_parse_expr_nested_arithmetic() {
    // [(+ (* ?a 2) ?b) ?result]
    let input = "(query [:find ?result :where [?e :x ?a] [?e :y ?b] [(+ (* ?a 2) ?b) ?result]])";
    let result = parse(input);
    assert!(result.is_ok(), "parse nested arithmetic");
}

#[test]
fn test_parse_expr_string_predicate() {
    let input = "(query [:find ?e :where [?e :item/tag ?tag] [(starts-with? ?tag \"work\")]])";
    let result = parse(input);
    assert!(result.is_ok(), "parse starts-with?");
}

#[test]
fn test_parse_expr_matches_valid_regex() {
    let input = "(query [:find ?e :where [?e :person/email ?addr] [(matches? ?addr \"^[^@]+@[^@]+$\")]])";
    let result = parse(input);
    assert!(result.is_ok(), "parse matches? with valid regex");
}

#[test]
fn test_parse_expr_matches_invalid_regex_is_error() {
    let input = "(query [:find ?e :where [?e :a ?v] [(matches? ?v \"[unclosed\")]])";
    let result = parse(input);
    assert!(result.is_err(), "invalid regex must be a parse error");
}

#[test]
fn test_parse_expr_unbound_variable_is_error() {
    // ?v is not bound by any pattern before the expr clause
    let input = "(query [:find ?e :where [?e :x ?a] [(< ?v 100)]])";
    let result = parse(input);
    assert!(result.is_err(), "unbound variable in expr must be parse error");
}

#[test]
fn test_parse_expr_three_element_vector_stays_pattern() {
    // [?e :a ?v] must still parse as a Pattern, not an Expr clause
    let input = "(query [:find ?v :where [?e :attr ?v]])";
    let result = parse(input);
    assert!(result.is_ok(), "three-element vector is a pattern");
    match result.unwrap() {
        DatalogCommand::Query(q) => {
            assert!(matches!(q.where_clauses[0], WhereClause::Pattern(_)));
        }
        _ => panic!(),
    }
}
```

- [ ] **Step 2: Run — expect failures**

```bash
cargo test --lib -q 2>&1 | grep -E "FAILED|error"
```

Expected: compile errors or test failures.

- [ ] **Step 3: Add `use crate::graph::types::Value;` to parser imports**

At the top of `parser.rs`, after `use super::types::*;`, add:

```rust
use crate::graph::types::Value;
```

- [ ] **Step 4: Implement `parse_expr_arg`**

Add before the existing `parse_list_as_where_clause` function (around line 747):

```rust
/// Convert a single EDN token to an Expr leaf, or recurse for a nested list.
fn parse_expr_arg(edn: &EdnValue) -> Result<Expr, String> {
    match edn {
        EdnValue::Symbol(s) if s.starts_with('?') => Ok(Expr::Var(s.clone())),
        EdnValue::Integer(n) => Ok(Expr::Lit(Value::Integer(*n))),
        EdnValue::Float(f) => Ok(Expr::Lit(Value::Float(*f))),
        EdnValue::String(s) => Ok(Expr::Lit(Value::String(s.clone()))),
        EdnValue::Boolean(b) => Ok(Expr::Lit(Value::Boolean(*b))),
        EdnValue::Nil => Ok(Expr::Lit(Value::Null)),
        EdnValue::Keyword(k) => Ok(Expr::Lit(Value::Keyword(k.clone()))),
        EdnValue::List(inner) => parse_expr(inner),
        other => Err(format!("unsupported expression argument: {:?}", other)),
    }
}
```

- [ ] **Step 5: Implement `parse_expr`**

Add immediately after `parse_expr_arg`:

```rust
/// Parse an EDN list `(op arg arg?)` into an Expr tree.
///
/// For `matches?`, the second argument must be a string literal and is
/// validated as a valid regex pattern at parse time.
fn parse_expr(list: &[EdnValue]) -> Result<Expr, String> {
    if list.is_empty() {
        return Err("expression list cannot be empty".to_string());
    }
    let head = match &list[0] {
        EdnValue::Symbol(s) => s.as_str(),
        other => return Err(format!("expression head must be a symbol, got {:?}", other)),
    };

    match head {
        // Unary type predicates
        "string?" | "integer?" | "float?" | "boolean?" | "nil?" => {
            if list.len() != 2 {
                return Err(format!("{} takes exactly 1 argument", head));
            }
            let op = match head {
                "string?" => UnaryOp::StringQ,
                "integer?" => UnaryOp::IntegerQ,
                "float?" => UnaryOp::FloatQ,
                "boolean?" => UnaryOp::BooleanQ,
                "nil?" => UnaryOp::NilQ,
                _ => unreachable!(),
            };
            let arg = parse_expr_arg(&list[1])?;
            Ok(Expr::UnaryOp(op, Box::new(arg)))
        }

        // Binary operators
        "<" | ">" | "<=" | ">=" | "=" | "!="
        | "+" | "-" | "*" | "/"
        | "starts-with?" | "ends-with?" | "contains?" | "matches?" => {
            if list.len() != 3 {
                return Err(format!("{} takes exactly 2 arguments", head));
            }
            let op = match head {
                "<"  => BinOp::Lt,  ">"  => BinOp::Gt,
                "<=" => BinOp::Lte, ">=" => BinOp::Gte,
                "="  => BinOp::Eq,  "!=" => BinOp::Neq,
                "+"  => BinOp::Add, "-"  => BinOp::Sub,
                "*"  => BinOp::Mul, "/"  => BinOp::Div,
                "starts-with?" => BinOp::StartsWith,
                "ends-with?"   => BinOp::EndsWith,
                "contains?"    => BinOp::Contains,
                "matches?"     => BinOp::Matches,
                _ => unreachable!(),
            };
            let lhs = parse_expr_arg(&list[1])?;
            let rhs = parse_expr_arg(&list[2])?;

            // matches? second arg must be a string literal; validate regex now.
            if op == BinOp::Matches {
                match &rhs {
                    Expr::Lit(Value::String(pattern)) => {
                        regex_lite::Regex::new(pattern).map_err(|e| {
                            format!("invalid regex pattern {:?}: {}", pattern, e)
                        })?;
                    }
                    _ => return Err(
                        "matches? second argument must be a string literal".to_string()
                    ),
                }
            }
            Ok(Expr::BinOp(op, Box::new(lhs), Box::new(rhs)))
        }

        other => Err(format!("unknown expression operator: {}", other)),
    }
}
```

- [ ] **Step 6: Implement `parse_expr_clause`**

Add immediately after `parse_expr`:

```rust
/// Parse a vector clause whose first element is a list: `[(expr)]` or `[(expr) ?out]`.
///
/// Called from `:where` dispatch when `vec[0]` is an `EdnValue::List`.
fn parse_expr_clause(vec: &[EdnValue]) -> Result<WhereClause, String> {
    let inner_list = match &vec[0] {
        EdnValue::List(l) => l.as_slice(),
        _ => return Err("parse_expr_clause called with non-list element 0".to_string()),
    };
    let expr = parse_expr(inner_list)?;
    let binding = match vec.len() {
        1 => None,
        2 => match &vec[1] {
            EdnValue::Symbol(s) if s.starts_with('?') => Some(s.clone()),
            other => return Err(format!(
                "expression output must be a ?variable, got {:?}", other
            )),
        },
        n => return Err(format!(
            "expression clause must be [(expr)] or [(expr) ?out], got {} elements", n
        )),
    };
    Ok(WhereClause::Expr { expr, binding })
}
```

- [ ] **Step 7: Run parser unit tests — should still fail (dispatch not wired yet)**

```bash
cargo test --lib -q 2>&1 | grep -E "test_parse_expr|FAILED"
```

Expected: compile success, but `test_parse_expr_*` tests fail (parse returns error or wrong variant).

- [ ] **Step 8: Commit the new parsing functions**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): add parse_expr, parse_expr_arg, parse_expr_clause for Phase 7.2b"
```

---

## Task 3: Parser — Wire Dispatch Sites, `outer_vars_from_clause`, Safety Check

**Files:**
- Modify: `src/query/datalog/parser.rs`

### Background

There are **four** dispatch sites that route vector clauses to `Pattern::from_edn`:

1. Query `:where` body (~line 544): `if let Some(pattern_vec) = query_vector[i].as_vector() { Pattern::from_edn(...) }`
2. Rule body (~line 961): `if let Some(vec) = item.as_vector() { Pattern::from_edn(...) }`
3. `not` body inside `parse_list_as_where_clause` (~line 763): `if let Some(vec) = item.as_vector() { Pattern::from_edn(...) }`
4. `not-join` body inside `parse_list_as_where_clause` (~line 812): `if let Some(vec) = item.as_vector() { Pattern::from_edn(...) }`

All four must check `vec[0]` before calling `Pattern::from_edn`.

`outer_vars_from_clause` (~line 846) is also an exhaustive match that needs a new arm.

`check_expr_safety` is a new function for the forward-pass variable safety check.

- [ ] **Step 1: Update `outer_vars_from_clause`**

In `outer_vars_from_clause`, add a new match arm before the closing `}`:

```rust
WhereClause::Expr { binding, .. } => match binding {
    Some(var) => vec![var.clone()],
    None => vec![],
},
```

- [ ] **Step 2: Add `expr_vars` helper and `check_expr_safety`**

Add after `check_not_join_safety` (around line 924):

```rust
/// Collect all Var names referenced in an Expr tree.
fn expr_vars(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::Var(v) => vec![v.clone()],
        Expr::Lit(_) => vec![],
        Expr::BinOp(_, lhs, rhs) => {
            let mut v = expr_vars(lhs);
            v.extend(expr_vars(rhs));
            v
        }
        Expr::UnaryOp(_, arg) => expr_vars(arg),
    }
}

/// Forward-pass safety check: every Var in an Expr clause must be bound by
/// an earlier clause. Binding-form Expr clauses add their output var to scope.
///
/// Called for both query `:where` clauses and rule body clauses.
fn check_expr_safety(clauses: &[WhereClause]) -> Result<(), String> {
    let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    for clause in clauses {
        match clause {
            WhereClause::Expr { expr, binding } => {
                for var in expr_vars(expr) {
                    if !bound.contains(&var) {
                        return Err(format!(
                            "variable {} in expression clause is not bound by any earlier :where clause",
                            var
                        ));
                    }
                }
                if let Some(out) = binding {
                    bound.insert(out.clone());
                }
            }
            other => {
                for var in outer_vars_from_clause(other) {
                    bound.insert(var);
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Update query `:where` dispatch (site 1)**

Locate the block around line 544:
```rust
Some(":where") => {
    if let Some(pattern_vec) = query_vector[i].as_vector() {
        let pattern = Pattern::from_edn(pattern_vec)?;
        where_clauses.push(WhereClause::Pattern(pattern));
    } else if ...
```

Replace the inner `if let Some(pattern_vec)` branch with:

```rust
Some(":where") => {
    if let Some(pattern_vec) = query_vector[i].as_vector() {
        // Expr clause: [(list-expr) ?out?] — element 0 is a List
        if matches!(pattern_vec.first(), Some(EdnValue::List(_))) {
            let clause = parse_expr_clause(pattern_vec)?;
            where_clauses.push(clause);
        } else {
            let pattern = Pattern::from_edn(pattern_vec)?;
            where_clauses.push(WhereClause::Pattern(pattern));
        }
    } else if let Some(rule_list) = query_vector[i].as_list() {
        let clause = parse_list_as_where_clause(rule_list, true)?;
        where_clauses.push(clause);
    } else {
        return Err(format!(
            "Expected pattern vector or rule invocation in :where clause, got {:?}",
            query_vector[i]
        ));
    }
}
```

- [ ] **Step 4: Call `check_expr_safety` in `parse_query`**

After the existing `check_not_join_safety(&where_clauses, &outer_bound)?;` call (around line 575), add:

```rust
check_expr_safety(&where_clauses)?;
```

- [ ] **Step 5: Update rule body dispatch (site 2)**

Locate the block around line 960:
```rust
for item in &body_vec[1..] {
    if let Some(vec) = item.as_vector() {
        let pattern = Pattern::from_edn(vec)?;
        body_clauses.push(WhereClause::Pattern(pattern));
    } else if ...
```

Replace the `if let Some(vec)` branch:

```rust
for item in &body_vec[1..] {
    if let Some(vec) = item.as_vector() {
        if matches!(vec.first(), Some(EdnValue::List(_))) {
            let clause = parse_expr_clause(vec)?;
            body_clauses.push(clause);
        } else {
            let pattern = Pattern::from_edn(vec)?;
            body_clauses.push(WhereClause::Pattern(pattern));
        }
    } else if let Some(list) = item.as_list() {
        let clause = parse_list_as_where_clause(list, true)?;
        body_clauses.push(clause);
    } else {
        return Err(format!(
            "Rule body clause must be a vector (pattern) or list (rule invocation / not), got {:?}",
            item
        ));
    }
}
```

- [ ] **Step 6: Call `check_expr_safety` in `parse_rule`**

After the existing `check_not_join_safety(&body_clauses, &outer_bound)?;` call (around line 994), add:

```rust
check_expr_safety(&body_clauses)?;
```

- [ ] **Step 7: Update `not` body dispatch (site 3)**

Inside `parse_list_as_where_clause`, in the `"not"` arm, locate:
```rust
if let Some(vec) = item.as_vector() {
    let pattern = Pattern::from_edn(vec)?;
    inner.push(WhereClause::Pattern(pattern));
```

Replace with:
```rust
if let Some(vec) = item.as_vector() {
    if matches!(vec.first(), Some(EdnValue::List(_))) {
        let clause = parse_expr_clause(vec)?;
        inner.push(clause);
    } else {
        let pattern = Pattern::from_edn(vec)?;
        inner.push(WhereClause::Pattern(pattern));
    }
```

- [ ] **Step 8: Update `not-join` body dispatch (site 4)**

Same change in the `"not-join"` arm of `parse_list_as_where_clause` (around line 812):
```rust
if let Some(vec) = item.as_vector() {
    if matches!(vec.first(), Some(EdnValue::List(_))) {
        let clause = parse_expr_clause(vec)?;
        inner.push(clause);
    } else {
        let pattern = Pattern::from_edn(vec)?;
        inner.push(WhereClause::Pattern(pattern));
    }
```

- [ ] **Step 9: Run parser tests — all should pass**

```bash
cargo test --lib -q 2>&1 | grep -E "test_parse_expr|FAILED|ok"
```

Expected: all `test_parse_expr_*` tests pass. All previously passing tests still pass.

- [ ] **Step 10: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat(parser): wire expr clause dispatch, outer_vars_from_clause, check_expr_safety"
```

---

## Task 4: Executor — `eval_expr`, `is_truthy`, `apply_expr_clauses`

**Files:**
- Modify: `src/query/datalog/executor.rs`

### Background

Add `use regex_lite::Regex;` and import the new types. The executor's binding maps are `HashMap<String, Value>` (abbreviated `Binding` locally). All new helpers are module-level free functions (not methods).

`apply_expr_clauses` is used in **four** places:
1. `execute_query` — after not/not-join post-filter
2. `execute_query_with_rules` — after not/not-join post-filter
3. `execute_query` not-body evaluation — for Expr clauses inside `not` bodies
4. `execute_query_with_rules` not-body evaluation — same

For not-body Expr evaluation: the not body may contain Expr clauses referencing outer-binding variables. Since all not-body vars must be bound by outer clauses (safety check), we evaluate Expr clauses against the merged (outer + pattern-match) binding.

- [ ] **Step 1: Write failing unit tests for eval helpers**

Add to the existing `#[cfg(test)]` block in `executor.rs`:

```rust
#[cfg(test)]
mod expr_eval_tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::datalog::types::{BinOp, Expr, UnaryOp};
    use std::collections::HashMap;

    fn b(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn test_eval_lit() {
        let e = Expr::Lit(Value::Integer(42));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Integer(42)));
    }

    #[test]
    fn test_eval_var_bound() {
        let e = Expr::Var("?x".to_string());
        let binding = b(&[("?x", Value::Integer(10))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Integer(10)));
    }

    #[test]
    fn test_eval_var_unbound_is_err() {
        let e = Expr::Var("?x".to_string());
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_lt_true() {
        let e = Expr::BinOp(BinOp::Lt, Box::new(Expr::Var("?v".to_string())), Box::new(Expr::Lit(Value::Integer(100))));
        let binding = b(&[("?v", Value::Integer(50))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_lt_false() {
        let e = Expr::BinOp(BinOp::Lt, Box::new(Expr::Var("?v".to_string())), Box::new(Expr::Lit(Value::Integer(100))));
        let binding = b(&[("?v", Value::Integer(150))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_add_integers() {
        let e = Expr::BinOp(BinOp::Add, Box::new(Expr::Var("?a".to_string())), Box::new(Expr::Var("?b".to_string())));
        let binding = b(&[("?a", Value::Integer(3)), ("?b", Value::Integer(4))]);
        assert_eq!(eval_expr(&e, &binding), Ok(Value::Integer(7)));
    }

    #[test]
    fn test_eval_add_int_float_promotes() {
        let e = Expr::BinOp(BinOp::Add, Box::new(Expr::Lit(Value::Integer(1))), Box::new(Expr::Lit(Value::Float(1.5))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Float(2.5)));
    }

    #[test]
    fn test_eval_div_integer_truncates() {
        let e = Expr::BinOp(BinOp::Div, Box::new(Expr::Lit(Value::Integer(5))), Box::new(Expr::Lit(Value::Integer(2))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Integer(2)));
    }

    #[test]
    fn test_eval_div_by_zero_is_err() {
        let e = Expr::BinOp(BinOp::Div, Box::new(Expr::Lit(Value::Integer(5))), Box::new(Expr::Lit(Value::Integer(0))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_eq_strings() {
        let e = Expr::BinOp(BinOp::Eq, Box::new(Expr::Lit(Value::String("Alice".to_string()))), Box::new(Expr::Lit(Value::String("Alice".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_eq_int_float_false() {
        // Different Value variants → structural inequality
        let e = Expr::BinOp(BinOp::Eq, Box::new(Expr::Lit(Value::Integer(1))), Box::new(Expr::Lit(Value::Float(1.0))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_type_mismatch_comparison_is_err() {
        let e = Expr::BinOp(BinOp::Lt, Box::new(Expr::Lit(Value::String("hello".to_string()))), Box::new(Expr::Lit(Value::Integer(100))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Err(()));
    }

    #[test]
    fn test_eval_string_q_true() {
        let e = Expr::UnaryOp(UnaryOp::StringQ, Box::new(Expr::Lit(Value::String("hi".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_string_q_false() {
        let e = Expr::UnaryOp(UnaryOp::StringQ, Box::new(Expr::Lit(Value::Integer(1))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(false)));
    }

    #[test]
    fn test_eval_starts_with_true() {
        let e = Expr::BinOp(BinOp::StartsWith, Box::new(Expr::Lit(Value::String("foobar".to_string()))), Box::new(Expr::Lit(Value::String("foo".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_ends_with_true() {
        let e = Expr::BinOp(BinOp::EndsWith, Box::new(Expr::Lit(Value::String("foobar".to_string()))), Box::new(Expr::Lit(Value::String("bar".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_contains_true() {
        let e = Expr::BinOp(BinOp::Contains, Box::new(Expr::Lit(Value::String("engineer at co".to_string()))), Box::new(Expr::Lit(Value::String("engineer".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_eval_matches_true() {
        let e = Expr::BinOp(BinOp::Matches, Box::new(Expr::Lit(Value::String("test@example.com".to_string()))), Box::new(Expr::Lit(Value::String("^[^@]+@[^@]+$".to_string()))));
        assert_eq!(eval_expr(&e, &HashMap::new()), Ok(Value::Boolean(true)));
    }

    #[test]
    fn test_is_truthy() {
        assert!(is_truthy(&Value::Boolean(true)));
        assert!(!is_truthy(&Value::Boolean(false)));
        assert!(is_truthy(&Value::Integer(1)));
        assert!(!is_truthy(&Value::Integer(0)));
        assert!(is_truthy(&Value::Float(0.1)));
        assert!(!is_truthy(&Value::Float(0.0)));
        assert!(!is_truthy(&Value::Null));
        assert!(!is_truthy(&Value::String("hi".to_string())));
    }
}
```

- [ ] **Step 2: Run — expect compile errors**

```bash
cargo test --lib -q 2>&1 | head -20
```

Expected: `eval_expr`, `is_truthy` not found.

- [ ] **Step 3: Add imports to `executor.rs`**

At the top of `executor.rs`, update the `use super::types::` import:

```rust
use super::types::{
    AggFunc, BinOp, DatalogCommand, DatalogQuery, EdnValue, Expr, FindSpec, Pattern, Rule,
    Transaction, UnaryOp, ValidAt, WhereClause,
};
```

Also add:
```rust
use regex_lite::Regex;
```

- [ ] **Step 4: Implement `is_truthy`**

Add as a module-level function after `extract_variables`:

```rust
/// Returns true for Boolean(true), non-zero Integer, non-zero Float.
/// All other Value variants (String, Keyword, Ref, Null, Float(0.0)) → false.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Boolean(b) => *b,
        Value::Integer(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        _ => false,
    }
}
```

- [ ] **Step 5: Implement `to_float_pair`**

```rust
/// Promote both values to f64 for numeric comparison / float arithmetic.
/// Returns Err(()) if either operand is not Integer or Float.
fn to_float_pair(l: &Value, r: &Value) -> Result<(f64, f64), ()> {
    let lf = match l {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        _ => return Err(()),
    };
    let rf = match r {
        Value::Integer(i) => *i as f64,
        Value::Float(f) => *f,
        _ => return Err(()),
    };
    Ok((lf, rf))
}
```

- [ ] **Step 6: Implement `eval_binop`**

```rust
fn eval_binop(op: &BinOp, l: Value, r: Value) -> Result<Value, ()> {
    match op {
        // Structural equality — works for all Value variants; no type mismatch error.
        BinOp::Eq  => return Ok(Value::Boolean(l == r)),
        BinOp::Neq => return Ok(Value::Boolean(l != r)),
        _ => {}
    }

    match op {
        // Numeric comparisons — require both numeric; int/float promotion via to_float_pair.
        BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
            let (lf, rf) = to_float_pair(&l, &r)?;
            Ok(Value::Boolean(match op {
                BinOp::Lt  => lf < rf,
                BinOp::Gt  => lf > rf,
                BinOp::Lte => lf <= rf,
                BinOp::Gte => lf >= rf,
                _ => unreachable!(),
            }))
        }

        // Arithmetic: integer-integer stays integer; any float promotes result to float.
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            match (&l, &r) {
                (Value::Integer(a), Value::Integer(b)) => match op {
                    BinOp::Add => Ok(Value::Integer(a.wrapping_add(*b))),
                    BinOp::Sub => Ok(Value::Integer(a.wrapping_sub(*b))),
                    BinOp::Mul => Ok(Value::Integer(a.wrapping_mul(*b))),
                    BinOp::Div => {
                        if *b == 0 { Err(()) } else { Ok(Value::Integer(a / b)) }
                    }
                    _ => unreachable!(),
                },
                _ => {
                    let (lf, rf) = to_float_pair(&l, &r)?;
                    match op {
                        BinOp::Add => Ok(Value::Float(lf + rf)),
                        BinOp::Sub => Ok(Value::Float(lf - rf)),
                        BinOp::Mul => Ok(Value::Float(lf * rf)),
                        BinOp::Div => {
                            if rf == 0.0 { Err(()) } else { Ok(Value::Float(lf / rf)) }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        // String predicates — both operands must be String.
        BinOp::StartsWith => match (l, r) {
            (Value::String(s), Value::String(prefix)) => Ok(Value::Boolean(s.starts_with(prefix.as_str()))),
            _ => Err(()),
        },
        BinOp::EndsWith => match (l, r) {
            (Value::String(s), Value::String(suffix)) => Ok(Value::Boolean(s.ends_with(suffix.as_str()))),
            _ => Err(()),
        },
        BinOp::Contains => match (l, r) {
            (Value::String(s), Value::String(needle)) => Ok(Value::Boolean(s.contains(needle.as_str()))),
            _ => Err(()),
        },
        BinOp::Matches => match (l, r) {
            (Value::String(s), Value::String(pattern)) => {
                // Pattern was validated at parse time; compile here.
                let re = Regex::new(&pattern).map_err(|_| ())?;
                Ok(Value::Boolean(re.is_match(&s)))
            }
            _ => Err(()),
        },

        // Eq/Neq handled above
        BinOp::Eq | BinOp::Neq => unreachable!(),
    }
}
```

- [ ] **Step 7: Implement `eval_expr`**

```rust
/// Evaluate an Expr against a binding map.
///
/// Returns `Err(())` on: unbound variable, type mismatch, division by zero.
fn eval_expr(
    expr: &Expr,
    binding: &std::collections::HashMap<String, Value>,
) -> Result<Value, ()> {
    match expr {
        Expr::Var(v) => binding.get(v).cloned().ok_or(()),
        Expr::Lit(val) => Ok(val.clone()),
        Expr::UnaryOp(op, arg) => {
            let v = eval_expr(arg, binding)?;
            Ok(Value::Boolean(match op {
                UnaryOp::StringQ  => matches!(v, Value::String(_)),
                UnaryOp::IntegerQ => matches!(v, Value::Integer(_)),
                UnaryOp::FloatQ   => matches!(v, Value::Float(_)),
                UnaryOp::BooleanQ => matches!(v, Value::Boolean(_)),
                UnaryOp::NilQ     => matches!(v, Value::Null),
            }))
        }
        Expr::BinOp(op, lhs, rhs) => {
            let l = eval_expr(lhs, binding)?;
            let r = eval_expr(rhs, binding)?;
            eval_binop(op, l, r)
        }
    }
}
```

- [ ] **Step 8: Implement `apply_expr_clauses`**

```rust
type Binding = std::collections::HashMap<String, Value>;

/// Apply all WhereClause::Expr clauses from `where_clauses` to `bindings`.
///
/// Filter-form (`binding: None`) drops the row if the expr is not truthy or errors.
/// Binding-form (`binding: Some(var)`) extends the row with the computed value.
/// Type mismatches and errors silently drop the row.
fn apply_expr_clauses(mut bindings: Vec<Binding>, where_clauses: &[WhereClause]) -> Vec<Binding> {
    for clause in where_clauses {
        if let WhereClause::Expr { expr, binding: out } = clause {
            bindings = bindings
                .into_iter()
                .filter_map(|mut b| match eval_expr(expr, &b) {
                    Ok(value) => match out {
                        None => if is_truthy(&value) { Some(b) } else { None },
                        Some(var) => { b.insert(var.clone(), value); Some(b) }
                    },
                    Err(_) => None,
                })
                .collect();
        }
    }
    bindings
}
```

- [ ] **Step 9: Run unit tests — all eval tests should pass**

```bash
cargo test --lib -q 2>&1 | grep -E "expr_eval_tests|FAILED"
```

Expected: all `expr_eval_tests::*` pass.

- [ ] **Step 10: Commit eval helpers**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(executor): add eval_expr, eval_binop, is_truthy, apply_expr_clauses"
```

---

## Task 5: Executor — Wire into Execution Paths

**Files:**
- Modify: `src/query/datalog/executor.rs`
- Modify: `src/query/datalog/evaluator.rs`

### Background

Three wiring points in `executor.rs`:

**A. `execute_query` — top-level expr filter (after not/not-join)**

After `let filtered_bindings: Vec<_> = ...` (line ~249), add:
```rust
let filtered_bindings = apply_expr_clauses(filtered_bindings, &query.where_clauses);
```

**B. `execute_query` — not-body expr evaluation**

The existing not-body loop (lines ~222-239) only processes `WhereClause::Pattern`. When a `not` body contains Expr clauses:
- Extract patterns → match → get `not_bindings`
- If no patterns exist, start `not_bindings` with a single copy of the outer binding (since variables are all from outer scope)
- Apply not-body Expr clauses to `not_bindings`
- If result non-empty → not body matched → exclude outer binding

**C. Same for `execute_query_with_rules` — both top-level expr and not-body expr.**

**D. `evaluator.rs` — `evaluate_rule`**

After `let bindings = matcher.match_patterns(&patterns);`, apply expr clauses from the rule body before instantiating the head.

**E. `evaluator.rs` — `evaluate_not_join`**

After pattern matching, apply Expr clauses from the not-join body.

- [ ] **Step 1: Write failing integration test (smoke test)**

Add to the existing `#[cfg(test)]` block in `executor.rs`:

```rust
#[test]
fn test_execute_expr_filter_lt() {
    use crate::graph::storage::FactStorage;
    use std::sync::{Arc, RwLock};
    use crate::query::datalog::rules::RuleRegistry;

    let storage = FactStorage::new();
    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let executor = DatalogExecutor::new_with_rules(storage.clone(), rules);

    // Transact two items with different prices
    executor.execute(crate::query::datalog::parser::parse(
        "(transact [[:item1 :item/price 50] [:item2 :item/price 150]])"
    ).unwrap()).unwrap();

    // Query: find items where price < 100
    let result = executor.execute(crate::query::datalog::parser::parse(
        "(query [:find ?e :where [?e :item/price ?p] [(< ?p 100)]])"
    ).unwrap());

    assert!(result.is_ok(), "expr filter query failed");
    match result.unwrap() {
        QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1, "expected exactly one result");
        }
        _ => panic!("expected QueryResults"),
    }
}
```

- [ ] **Step 2: Run — expect test failure**

```bash
cargo test --lib test_execute_expr_filter_lt -- --nocapture 2>&1 | tail -10
```

Expected: test fails (expr clause not yet applied in executor).

- [ ] **Step 3: Wire `execute_query` — top-level expr filter**

In `execute_query`, locate the line after the not/not-join filter that produces `filtered_bindings` (around line 249). Add immediately after:

```rust
// Apply WhereClause::Expr clauses (filter and binding predicates)
let filtered_bindings = apply_expr_clauses(filtered_bindings, &query.where_clauses);
```

- [ ] **Step 4: Wire `execute_query` — not-body expr handling**

Replace the existing not-body loop in `execute_query` (lines ~222-239):

```rust
let filtered_bindings: Vec<_> = if not_clauses.is_empty() && not_join_clauses.is_empty() {
    bindings
} else {
    let not_storage = filtered_storage.clone();
    bindings
        .into_iter()
        .filter(|binding| {
            for not_body in &not_clauses {
                if not_body_matches(not_body, binding, &not_storage) {
                    return false;
                }
            }
            for (join_vars, nj_clauses) in &not_join_clauses {
                if evaluate_not_join(join_vars, nj_clauses, binding, &not_storage) {
                    return false;
                }
            }
            true
        })
        .collect()
};
```

Add the `not_body_matches` helper function (module-level):

```rust
/// Evaluate a `not` body against the current outer binding.
///
/// Returns true if the body "matches" (i.e., the outer binding should be excluded).
///
/// Algorithm:
/// 1. Extract Pattern clauses from the body, substitute outer binding, match against storage.
///    If no patterns, start with [outer_binding] as the initial binding set.
/// 2. Apply Expr clauses from the body to the resulting binding set.
/// 3. If the final binding set is non-empty → body matched → return true.
fn not_body_matches(
    not_body: &[WhereClause],
    outer: &Binding,
    storage: &crate::graph::FactStorage,
) -> bool {
    use crate::query::datalog::evaluator::substitute_pattern;

    let patterns: Vec<_> = not_body
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(substitute_pattern(p, outer)),
            _ => None,
        })
        .collect();

    let matcher = crate::query::datalog::matcher::PatternMatcher::new(storage.clone());
    let mut not_bindings: Vec<Binding> = if patterns.is_empty() {
        // Expr-only not body: start with the outer binding so variables resolve.
        vec![outer.clone()]
    } else {
        // Merge outer binding with pattern-match results.
        matcher
            .match_patterns(&patterns)
            .into_iter()
            .map(|mut nb| {
                for (k, v) in outer {
                    nb.entry(k.clone()).or_insert_with(|| v.clone());
                }
                nb
            })
            .collect()
    };

    // Apply Expr clauses from the not body.
    not_bindings = apply_expr_clauses(not_bindings, not_body);
    !not_bindings.is_empty()
}
```

- [ ] **Step 5: Wire `execute_query_with_rules` — top-level expr filter**

In `execute_query_with_rules`, after the not/not-join filter (around line 429), add:

```rust
let filtered_bindings = apply_expr_clauses(filtered_bindings, &query.where_clauses);
```

- [ ] **Step 6: Wire `execute_query_with_rules` — not-body expr handling**

In `execute_query_with_rules`, replace the existing not-body `filter` loop (lines ~352-428) to use `not_body_matches`, same as Step 4. The only difference: the not-body storage is `derived_storage` (which includes rule-derived facts), and rule invocations in the not body are handled by the existing code. Keep the existing `RuleInvocation` handling inside the loop; add Expr support by calling `not_body_matches` for bodies that contain Expr clauses, or keeping the existing path for pure-pattern/rule-invocation bodies.

Simplest approach: use `not_body_matches` for all `not` body evaluation in both paths (it only extracts `Pattern` clauses; RuleInvocation inside not bodies in the rules path are handled separately by the existing `filter_map` that converts them to patterns against `derived_storage`).

Replace the `execute_query_with_rules` not-body loop:

```rust
let filtered_bindings: Vec<_> = if not_clauses.is_empty() && not_join_clauses.is_empty() {
    bindings
} else {
    let not_storage = derived_storage.clone();
    bindings
        .into_iter()
        .filter(|binding| {
            for not_body in &not_clauses {
                // Build pattern list from both Pattern and RuleInvocation clauses
                let substituted: Vec<Pattern> = not_body
                    .iter()
                    .filter_map(|c| match c {
                        WhereClause::Pattern(p) => {
                            Some(crate::query::datalog::evaluator::substitute_pattern(p, binding))
                        }
                        WhereClause::RuleInvocation { predicate, args } => {
                            // (existing rule-invocation-to-pattern logic stays here)
                            let resolved_args: Vec<EdnValue> = args
                                .iter()
                                .map(|a| match a {
                                    EdnValue::Symbol(s) if s.starts_with('?') => {
                                        binding.get(s).map(|v| match v {
                                            Value::Keyword(k) => EdnValue::Keyword(k.clone()),
                                            Value::String(s) => EdnValue::String(s.clone()),
                                            Value::Integer(i) => EdnValue::Integer(*i),
                                            Value::Float(f) => EdnValue::Float(*f),
                                            Value::Boolean(b) => EdnValue::Boolean(*b),
                                            Value::Ref(u) => EdnValue::Uuid(*u),
                                            Value::Null => EdnValue::Nil,
                                        }).unwrap_or_else(|| a.clone())
                                    }
                                    other => other.clone(),
                                })
                                .collect();
                            let pattern = match resolved_args.len() {
                                1 => Pattern::new(resolved_args[0].clone(), EdnValue::Keyword(format!(":{}", predicate)), EdnValue::Symbol("?_rule_value".to_string())),
                                2 => Pattern::new(resolved_args[0].clone(), EdnValue::Keyword(format!(":{}", predicate)), resolved_args[1].clone()),
                                _ => return None,
                            };
                            Some(crate::query::datalog::evaluator::substitute_pattern(&pattern, binding))
                        }
                        _ => None,
                    })
                    .collect();

                let matcher = crate::query::datalog::matcher::PatternMatcher::new(not_storage.clone());
                let mut not_bindings: Vec<Binding> = if substituted.is_empty() {
                    vec![binding.clone()]
                } else {
                    matcher.match_patterns(&substituted).into_iter().map(|mut nb| {
                        for (k, v) in binding { nb.entry(k.clone()).or_insert_with(|| v.clone()); }
                        nb
                    }).collect()
                };
                not_bindings = apply_expr_clauses(not_bindings, not_body);
                if !not_bindings.is_empty() { return false; }
            }
            for (join_vars, nj_clauses) in &not_join_clauses {
                if evaluate_not_join(join_vars, nj_clauses, binding, &not_storage) {
                    return false;
                }
            }
            true
        })
        .collect()
};
```

- [ ] **Step 7: Update `evaluate_rule` in `evaluator.rs`**

In `evaluator.rs`, the `evaluate_rule` method currently returns an error for `WhereClause::Not` / `WhereClause::NotJoin` (they are handled by `StratifiedEvaluator`). Add a `WhereClause::Expr` arm that collects expr clauses for post-match application:

Replace the exhaustive match in `evaluate_rule` (lines ~188-212):

```rust
let mut patterns = Vec::new();
let mut expr_clauses: Vec<&WhereClause> = Vec::new();
for clause in &rule.body {
    match clause {
        WhereClause::Pattern(p) => patterns.push(p.clone()),
        WhereClause::RuleInvocation { predicate, args } => {
            let list: Vec<EdnValue> = std::iter::once(EdnValue::Symbol(predicate.clone()))
                .chain(args.iter().cloned())
                .collect();
            let pattern = self.rule_invocation_to_pattern(&list)?;
            patterns.push(pattern);
        }
        WhereClause::Expr { .. } => {
            expr_clauses.push(clause);
        }
        WhereClause::Not(_) => {
            return Err(anyhow!(
                "WhereClause::Not in evaluate_rule: use StratifiedEvaluator for rules with negation"
            ));
        }
        WhereClause::NotJoin { .. } => {
            return Err(anyhow!(
                "WhereClause::NotJoin in evaluate_rule: use StratifiedEvaluator for rules with negation"
            ));
        }
    }
}

if patterns.is_empty() && expr_clauses.is_empty() {
    return Ok(derived);
}

let matcher = PatternMatcher::new(current_facts.clone());
let mut bindings = matcher.match_patterns(&patterns);

// Apply rule-body Expr clauses to filter / extend bindings before head instantiation.
if !expr_clauses.is_empty() {
    bindings = apply_expr_clauses_in_evaluator(bindings, &expr_clauses);
}
```

Add `apply_expr_clauses_in_evaluator` as a free function in `evaluator.rs` (it mirrors `apply_expr_clauses` in `executor.rs` but lives here to avoid a cross-module dependency):

```rust
fn apply_expr_clauses_in_evaluator(
    bindings: Vec<Bindings>,
    expr_clauses: &[&WhereClause],
) -> Vec<Bindings> {
    use crate::query::datalog::executor::{eval_expr, is_truthy};
    bindings
        .into_iter()
        .filter_map(|mut b| {
            for clause in expr_clauses {
                if let WhereClause::Expr { expr, binding: out } = clause {
                    match eval_expr(expr, &b) {
                        Ok(value) => match out {
                            None => if !is_truthy(&value) { return None; }
                            Some(var) => { b.insert(var.clone(), value); }
                        },
                        Err(_) => return None,
                    }
                }
            }
            Some(b)
        })
        .collect()
}
```

**Note:** `evaluator.rs` is a sibling module of `executor.rs`, both under `src/query/datalog/`. To expose `eval_expr` and `is_truthy` from `executor.rs` to sibling modules, use `pub(crate)` (not `pub(super)` — that only grants visibility to the parent `datalog` module, not to siblings).

- [ ] **Step 8: Update `evaluate_not_join` in `evaluator.rs`**

`evaluate_not_join` currently returns `false` when `substituted.is_empty()` (line ~423). This is correct for pattern-only not-join bodies, but for Expr-containing not-join bodies, we need to apply the Expr clauses.

Replace the existing `evaluate_not_join` body:

```rust
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: &FactStorage,
) -> bool {
    let partial: Bindings = join_vars
        .iter()
        .filter_map(|v| binding.get(v.as_str()).map(|val| (v.clone(), val.clone())))
        .collect();

    let substituted: Vec<Pattern> = clauses
        .iter()
        .filter_map(|c| match c {
            WhereClause::Pattern(p) => Some(substitute_pattern(p, &partial)),
            WhereClause::RuleInvocation { predicate, args } => {
                rule_invocation_to_pattern(predicate, args)
                    .ok()
                    .map(|p| substitute_pattern(&p, &partial))
            }
            _ => None,
        })
        .collect();

    let matcher = PatternMatcher::new(storage.clone());
    let mut not_bindings: Vec<Bindings> = if substituted.is_empty() {
        // Expr-only not-join: start with partial binding.
        vec![partial.clone()]
    } else {
        matcher.match_patterns(&substituted)
    };

    // Apply Expr clauses from the not-join body.
    let expr_clauses: Vec<&WhereClause> = clauses
        .iter()
        .filter(|c| matches!(c, WhereClause::Expr { .. }))
        .collect();
    if !expr_clauses.is_empty() {
        not_bindings = apply_expr_clauses_in_evaluator(not_bindings, &expr_clauses);
    }

    !not_bindings.is_empty()
}
```

- [ ] **Step 9: Make `eval_expr` and `is_truthy` `pub(crate)` in `executor.rs`**

Change:
```rust
fn eval_expr(...) -> Result<Value, ()>
fn is_truthy(v: &Value) -> bool
```
to:
```rust
pub(crate) fn eval_expr(...) -> Result<Value, ()>
pub(crate) fn is_truthy(v: &Value) -> bool
```

(`pub(super)` would only grant visibility to the parent `datalog` module, not to sibling modules like `evaluator.rs`. `pub(crate)` is the correct visibility for cross-sibling access.)

- [ ] **Step 10: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass, `test_execute_expr_filter_lt` passes.

- [ ] **Step 11: Commit**

```bash
git add src/query/datalog/executor.rs src/query/datalog/evaluator.rs
git commit -m "feat(executor): wire expr clauses into execute_query, execute_query_with_rules, evaluate_rule, evaluate_not_join"
```

---

## Task 6: Integration Tests

**Files:**
- Create: `tests/predicate_expr_test.rs`

### Background

Each test:
1. Creates an in-memory `Minigraf`
2. Transacts a small dataset
3. Runs a query with an expression clause
4. Asserts the result count and/or values

Use `Minigraf::in_memory().unwrap()` and `db.execute(str).unwrap()`. Avoid `{:?}` on `Result`/`Value` in assert messages (CodeQL rule — use plain strings).

- [ ] **Step 1: Create `tests/predicate_expr_test.rs`**

```rust
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
    db.execute("(transact [[:a :age 25] [:b :age 35]])").expect("transact");
    let r = db.execute("(query [:find ?e :as-of 1 :where [?e :age ?age] [(< ?age 30)]])").expect("query");
    assert_eq!(count(r), 1);
}

// ── Arithmetic binding into aggregate ────────────────────────────────────────

#[test]
fn test_arithmetic_binding_into_sum_aggregate() {
    let db = open();
    db.execute("(transact [[:a :price 10] [:a :qty 3] [:b :price 5] [:b :qty 4]])").expect("transact");
    // total = price * qty; sum of totals
    let r = db.execute("(query [:find (sum ?total) :with ?e :where [?e :price ?p] [?e :qty ?q] [(* ?p ?q) ?total]])").expect("query");
    let rs = rows(r);
    assert_eq!(rs.len(), 1);
    // 10*3 + 5*4 = 30 + 20 = 50
    assert_eq!(rs[0][0], MgValue::Integer(50));
}
```

- [ ] **Step 2: Verify public exports in `src/lib.rs`**

```bash
grep -n "pub use" src/lib.rs | head -20
```

The integration test uses `use minigraf::{Minigraf, QueryResult, Value as MgValue};`. Confirm `QueryResult` and `Value` are re-exported at the crate root. If they are under a different path, adjust the import accordingly.

- [ ] **Step 3: Run all integration tests**

```bash
cargo test --test predicate_expr_test -- --nocapture 2>&1 | tail -40
```

Expected: all 25 tests pass.

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass (previously 461+).

- [ ] **Step 5: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
git add tests/predicate_expr_test.rs
git commit -m "test(predicate-expr): add integration test suite for Phase 7.2b"
```

---

## Done

After all 6 tasks are complete and all tests pass:

```bash
cargo test 2>&1 | tail -5
```

Expected output: `test result: ok. N passed; 0 failed`.

Update `CLAUDE.md` test count and `CHANGELOG.md` with Phase 7.2b entry before marking the phase complete.
