# Phase 7.2b — Arithmetic & Predicate Expressions

**Date**: 2026-03-25
**Status**: Approved
**Phase**: 7.2b (sub-phase of Phase 7: Datalog Completeness)

---

## Overview

Add arithmetic and predicate expression clauses to the Datalog `:where` clause, expressed as a unified `Expr` AST. This unlocks filter predicates (`[(< ?v 100)]`) and arithmetic bindings (`[(+ ?a ?b) ?result]`) without requiring application-side post-processing.

This is a load-bearing dependency for Phase 7.7 (temporal range queries via `:db/valid-from` / `:db/valid-to` pseudo-attributes) and Phase 7.9b (UDF predicates via `FunctionRegistry`).

---

## Scope

### Operators

**Binary operators** (two-argument, written as `(op lhs rhs)`):

| Operator | Form | Returns |
|---|---|---|
| `<` | `(< ?a ?b)` | `bool` |
| `>` | `(> ?a ?b)` | `bool` |
| `<=` | `(<= ?a ?b)` | `bool` |
| `>=` | `(>= ?a ?b)` | `bool` |
| `=` | `(= ?a ?b)` | `bool` |
| `!=` | `(!= ?a ?b)` | `bool` |
| `+` | `(+ ?a ?b)` | numeric |
| `-` | `(- ?a ?b)` | numeric |
| `*` | `(* ?a ?b)` | numeric |
| `/` | `(/ ?a ?b)` | numeric |
| `starts-with?` | `(starts-with? ?s "prefix")` | `bool` |

**Unary operators** (one-argument type predicates):

| Operator | Form | Returns |
|---|---|---|
| `string?` | `(string? ?v)` | `bool` |
| `integer?` | `(integer? ?v)` | `bool` |
| `float?` | `(float? ?v)` | `bool` |
| `boolean?` | `(boolean? ?v)` | `bool` |
| `nil?` | `(nil? ?v)` | `bool` |

### Error semantics

Type mismatches and division by zero **silently drop the row** — consistent with Datomic's approach and the principle that predicates are filters, not error sources.

---

## Syntax

Two clause forms inside `:where`, both written as an EDN vector:

```datalog
;; Filter — keeps row if expression is truthy; no new variable introduced
[(< ?age 30)]
[(>= ?salary ?min-salary)]
[(string? ?name)]
[(starts-with? ?tag "work")]

;; Binding — evaluates expression, binds result to output variable
[(+ ?price ?tax) ?total]
[(* ?price ?qty) ?subtotal]
[(string? ?v) ?is-str]       ;; binds true or false
[(integer? ?v) ?is-int]

;; Nested arithmetic expressions
[(+ (* ?a 2) ?b) ?result]

;; Combine with temporal filters (key use case for Phase 7.7)
(query [:find ?e ?name
        :as-of 10
        :where [?e :person/name ?name]
               [?e :person/age ?age]
               [(< ?age 30)]])
```

**Shape disambiguation**: A vector clause is a `Pattern` if element 0 is a symbol/keyword/uuid. It is an `Expr` clause if element 0 is a list (expression). No overlap.

---

## Design

### Types (`src/query/datalog/types.rs`)

Three new enums, one new `WhereClause` variant:

```rust
/// Binary operators
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    // Comparisons — return bool
    Lt, Gt, Lte, Gte, Eq, Neq,
    // Arithmetic — return numeric Value
    Add, Sub, Mul, Div,
    // String predicate — return bool
    StartsWith,
}

/// Unary type-predicate operators — return bool
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    StringQ,
    IntegerQ,
    FloatQ,
    BooleanQ,
    NilQ,
}

/// Composable expression tree
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Var(String),                                   // ?v
    Lit(Value),                                    // 100, "foo", true
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}

// WhereClause gains one new variant:
pub enum WhereClause {
    Pattern(Pattern),
    RuleInvocation { predicate: String, args: Vec<EdnValue> },
    Not(Vec<WhereClause>),
    NotJoin { join_vars: Vec<String>, clauses: Vec<WhereClause> },
    /// [(expr) ?out?]
    /// binding = None  → filter: keep row iff expr evaluates to truthy
    /// binding = Some  → bind result Value to the named variable
    Expr { expr: Expr, binding: Option<String> },
}
```

**`WhereClause::rule_invocations` and `has_negated_invocation`** require no changes — `Expr` contains no rule invocations.

### Parser (`src/query/datalog/parser.rs`)

Recognition rule inside `:where` clause parsing:

- Vector with 1 element, element 0 is a list → `WhereClause::Expr { expr: parse_expr(list), binding: None }`
- Vector with 2 elements, element 0 is a list, element 1 is `?var` symbol → `WhereClause::Expr { expr: parse_expr(list), binding: Some(var) }`
- Otherwise → existing `Pattern` / `RuleInvocation` / `Not` / `NotJoin` paths

`parse_expr` recurses:

```
head symbol "+"  → BinOp::Add
head symbol "<"  → BinOp::Lt
head symbol "string?" → UnaryOp::StringQ
integer literal  → Expr::Lit(Value::Integer(_))
?var symbol      → Expr::Var(_)
list element     → recurse
```

**Safety check (parse time):** All `Expr::Var` references inside an expression clause must be bound by earlier `:where` clauses. Violation → `Err("variable ?x in expression clause is unbound")`. The output variable (in `binding: Some`) is exempt — it is the new binding produced.

**Applies inside `not` / `not-join` bodies** — `parse_expr_clause` is called from the same clause-parsing helper used for `Not` body parsing.

### Executor (`src/query/datalog/executor.rs`)

`WhereClause::Expr` is dispatched in the same per-clause loop as `Pattern`, `Not`, and `NotJoin`. It is always applied **after** the patterns that produced its bound variables (guaranteed by the parse-time safety check).

```rust
fn apply_expr_clause(
    bindings: Vec<Binding>,
    expr: &Expr,
    out: Option<&str>,
) -> Vec<Binding> {
    bindings.into_iter().filter_map(|mut b| {
        match eval_expr(expr, &b) {
            Ok(value) => match out {
                None => if is_truthy(&value) { Some(b) } else { None },
                Some(var) => { b.insert(var.to_string(), value); Some(b) }
            },
            Err(_) => None,   // type mismatch or div/0 — silently drop
        }
    }).collect()
}
```

`eval_expr` recurses the `Expr` tree:
- `Expr::Var(v)` → look up `v` in current binding; `Err` if absent
- `Expr::Lit(val)` → return `val.clone()`
- `Expr::BinOp(op, lhs, rhs)` → evaluate both sides, apply `op`; `Err` on type mismatch or div/0
- `Expr::UnaryOp(op, arg)` → evaluate arg, apply type test; always succeeds (returns `Boolean`)

**`is_truthy`**: `Boolean(true)` → true; non-zero `Integer`/`Float` → true; everything else → false.

**Interaction with `Not` / `NotJoin`**: `Expr` clauses inside negation bodies are evaluated by the same `apply_expr_clause` helper — no special handling needed.

### Optimizer (`src/query/datalog/optimizer.rs`)

`WhereClause::Expr` clauses are **passed through unchanged** — they are not reordered. The selectivity-based reordering applies only to `Pattern` clauses. This is correct and safe:

- Moving an `Expr` earlier risks evaluating it before its variables are bound
- Moving it later offers no benefit (it drives no index selection)

**Future extension point**: Phase 7.9b may assign selectivity estimates to predicate expressions and allow the optimizer to move them. The pass-through approach is forwards-compatible — no optimizer changes are needed today.

---

## Testing

### Unit tests

- **`types.rs`**: `Expr` / `BinOp` / `UnaryOp` construction; `WhereClause::Expr` variant round-trip; `rule_invocations()` returns empty for `Expr` variant
- **`parser.rs`**: each operator parsed correctly; nested expression `(+ (* ?a 2) ?b)`; filter vs binding shape; ambiguity guard (3-element vector stays a `Pattern`); unbound variable rejected with clear error

### Integration tests (`tests/predicate_expr_test.rs`)

| Scenario | Description |
|---|---|
| Comparison filter | `[(< ?age 30)]` keeps only matching rows |
| Two-variable comparison | `[(>= ?a ?b)]` |
| Arithmetic binding | `[(* ?price ?qty) ?total]` |
| Nested arithmetic | `[(+ (* ?a 2) ?b) ?result]` |
| Type predicate filter | `[(string? ?v)]` |
| `starts-with?` filter | `[(starts-with? ?tag "work")]` |
| Predicate binding | `[(integer? ?v) ?is-int]` binds `true`/`false` |
| Type mismatch → drop | `[(< ?v 100)]` where `?v = "hello"` → row silently dropped |
| Division by zero → drop | `[(/ ?a ?b) ?r]` where `?b = 0` → row silently dropped |
| Expr inside `not` body | valid and evaluated correctly |
| Bi-temporal filter | `[(< ?age 30)]` combined with `:as-of` |

---

## Files Modified

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `BinOp`, `UnaryOp`, `Expr`; add `WhereClause::Expr` variant |
| `src/query/datalog/parser.rs` | Parse `[(expr) ?out?]` clause shape; add `parse_expr`; safety check |
| `src/query/datalog/executor.rs` | Dispatch `WhereClause::Expr`; add `apply_expr_clause`, `eval_expr`, `is_truthy` |
| `src/query/datalog/optimizer.rs` | Pass-through `WhereClause::Expr` unchanged |
| `tests/predicate_expr_test.rs` | New integration test file |

---

## Non-Goals (out of scope for 7.2b)

- Window functions (Phase 7.9a)
- UDF registration via `FunctionRegistry` (Phase 7.9b)
- Pseudo-attribute bindings (`:db/valid-from` etc.) — Phase 7.7
- String functions beyond `starts-with?` (e.g., `ends-with?`, `contains?`, `upper-case`) — can be added incrementally
- Three-or-more-argument expressions
