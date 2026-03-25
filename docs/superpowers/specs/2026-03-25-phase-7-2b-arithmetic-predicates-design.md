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

**Comparisons** — return `bool`:

| Operator | Form |
|---|---|
| `<` | `(< ?a ?b)` |
| `>` | `(> ?a ?b)` |
| `<=` | `(<= ?a ?b)` |
| `>=` | `(>= ?a ?b)` |
| `=` | `(= ?a ?b)` |
| `!=` | `(!= ?a ?b)` |

**Arithmetic** — return numeric `Value`:

| Operator | Form |
|---|---|
| `+` | `(+ ?a ?b)` |
| `-` | `(- ?a ?b)` |
| `*` | `(* ?a ?b)` |
| `/` | `(/ ?a ?b)` |

**String predicates** — return `bool`:

| Operator | Form |
|---|---|
| `starts-with?` | `(starts-with? ?s "prefix")` |
| `contains?` | `(contains? ?s "needle")` |
| `matches?` | `(matches? ?s "^\\d{4}-\\d{2}-\\d{2}$")` |

**Unary operators** (one-argument type predicates, return `bool`):

| Operator | Form |
|---|---|
| `string?` | `(string? ?v)` |
| `integer?` | `(integer? ?v)` |
| `float?` | `(float? ?v)` |
| `boolean?` | `(boolean? ?v)` |
| `nil?` | `(nil? ?v)` |

### Error semantics

Type mismatches and division by zero **silently drop the row** — consistent with Datomic's approach and the principle that predicates are filters, not error sources.

### Numeric type semantics

**Mixed integer/float arithmetic**: When one operand is `Integer` and the other is `Float`, the `Integer` is promoted to `Float` and the result is `Float`. This matches Datomic's promotion behaviour.

**Integer division**: `(/ ?a ?b)` where both are `Integer` performs integer truncation (e.g., `5 / 2 = 2`). If `?b = 0`, the row is silently dropped.

**Comparison across types**: `<`, `>`, `<=`, `>=` require both operands to be numeric (`Integer` or `Float`); a string compared to an integer is a type mismatch → row dropped.

**`=` and `!=` semantics**: Structural equality on `Value` — works for `String`, `Integer`, `Float`, `Boolean`, `Keyword`, `Ref`, and `Null`. `(= "Alice" "Alice")` → true; `(= 1 1.0)` → false (different variant). Type mismatch does not drop the row for `=` / `!=` — they return `false` / `true` respectively (same as Datomic: `(= :a "a")` = false, not an error).

**`is_truthy` definition**: `Boolean(true)` → true; non-zero `Integer` or `Float` → true; everything else (including `Keyword`, `Ref`, `Null`, zero, empty string) → false.

---

## Syntax

Two clause forms inside `:where`, both written as an EDN vector:

```datalog
;; Filter — keeps row if expression is truthy; no new variable introduced
[(< ?age 30)]
[(>= ?salary ?min-salary)]
[(string? ?name)]
[(starts-with? ?tag "work")]
[(contains? ?bio "engineer")]
[(matches? ?email "^[^@]+@[^@]+$")]

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

**Shape disambiguation**: The vector-clause dispatcher checks element 0 before routing:
- Element 0 is a list (EDN `(...)`) → `Expr` clause path
- Otherwise → existing `Pattern` / `RuleInvocation` / `Not` / `NotJoin` paths

The existing `Pattern::from_edn` (which hard-fails on non-3-element vectors) is not called for `Expr` clauses — the dispatcher branches before reaching it.

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
    // Arithmetic — return numeric Value (Integer/Float with promotion)
    Add, Sub, Mul, Div,
    // String predicates — return bool
    StartsWith,
    Contains,
    Matches,  // regex via regex-lite; pattern compiled at parse time
}

/// Unary type-predicate operators — always return Boolean
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

**Required updates to existing exhaustive matches in `types.rs`**:

- `WhereClause::rule_invocations()` — add arm `WhereClause::Expr { .. } => vec![]`
- `WhereClause::has_negated_invocation()` — add arm `WhereClause::Expr { .. } => false`
- `collect_rule_invocations_recursive()` — add arm for `WhereClause::Expr { .. }` (no-op)

All three are trivial — `Expr` clauses contain no rule invocations.

### Parser (`src/query/datalog/parser.rs`)

**Two dispatch sites** must be updated: the query `:where` clause parser and the rule-body clause parser. Both currently dispatch vector clauses directly to `Pattern::from_edn`. Both must be updated to check element 0 first:

```
if vec[0] is a List:
    parse as Expr clause
else:
    existing Pattern / Not / NotJoin / RuleInvocation dispatch
```

`parse_expr` recurses on the inner list:

```
head symbol "+"        → BinOp(Add, parse_expr(arg0), parse_expr(arg1))
head symbol "<"        → BinOp(Lt,  parse_expr(arg0), parse_expr(arg1))
head symbol "string?"     → UnaryOp(StringQ, parse_expr(arg0))
head symbol "contains?"   → BinOp(Contains, parse_expr(arg0), parse_expr(arg1))
head symbol "matches?"    → BinOp(Matches, parse_expr(arg0), parse_expr(arg1))
                            (second arg must resolve to a string literal; regex compiled at parse time — invalid pattern → parse error)
integer literal        → Expr::Lit(Value::Integer(_))
string literal         → Expr::Lit(Value::String(_))
?var symbol            → Expr::Var(_)
nested list            → recurse
```

**Safety check (parse-time, post-hoc pass)**: After all `:where` clauses are parsed, the existing `outer_bound` set is built from `outer_vars_from_clause`. **`outer_vars_from_clause` must handle `WhereClause::Expr`**:

- `binding: None` → contributes no new variables to `outer_bound`
- `binding: Some(var)` → contributes `var` to `outer_bound` (the new variable is now in scope for subsequent clauses)

The safety check then verifies that all `Expr::Var` references in each `Expr` clause are present in `outer_bound` at the point that clause is processed. Because `outer_bound` is built as a forward pass, a variable used before it is bound will correctly fail the check.

**Note**: The safety check is post-hoc (full parse → then check), consistent with how the existing `not` safety check works. The check does correctly enforce ordering because `outer_vars_from_clause` accumulates variables in `:where` clause order.

**Applies inside `not` / `not-join` bodies** — the same clause-parsing helper is used for Not body parsing; the element-0 check applies there too.

### Executor (`src/query/datalog/executor.rs`)

`WhereClause::Expr` is dispatched in the same per-clause loop as `Pattern`, `Not`, and `NotJoin`. It is always applied after the patterns that produced its bound variables (guaranteed by the parse-time safety check).

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
- `Expr::BinOp(op, lhs, rhs)` → evaluate both sides, apply `op` with numeric promotion where applicable; `Err` on type mismatch or div/0
- `Expr::UnaryOp(op, arg)` → evaluate arg, apply type test; always returns `Value::Boolean(_)` (never errors)

**`is_truthy`**: `Boolean(true)` → true; non-zero `Integer` or `Float` → true; everything else (`Keyword`, `Ref`, `Null`, zero, `String`, `Boolean(false)`) → false.

**Interaction with `Not` / `NotJoin`**: `Expr` clauses inside negation bodies are evaluated by the same `apply_expr_clause` helper — no special handling needed.

### Optimizer (`src/query/datalog/optimizer.rs`)

`WhereClause::Expr` clauses are **passed through unchanged** — they are not reordered. The selectivity-based reordering applies only to `Pattern` clauses. This is correct and safe:

- Moving an `Expr` earlier risks evaluating it before its variables are bound
- Moving it later offers no benefit (it drives no index selection)

**Future extension point**: Phase 7.9b may assign selectivity estimates to predicate expressions and allow the optimizer to move them. The pass-through approach is forwards-compatible — no optimizer changes are needed today.

---

## Testing

### Unit tests

- **`types.rs`**: `Expr` / `BinOp` / `UnaryOp` construction; `WhereClause::Expr` variant; `rule_invocations()` returns empty for `Expr` variant; `has_negated_invocation()` returns false
- **`parser.rs`**: each operator parsed correctly; nested expression `(+ (* ?a 2) ?b)`; filter vs binding shape; ambiguity guard (3-element vector stays a `Pattern`); unbound variable rejected with clear error; `outer_vars_from_clause` correctly contributes binding variable to scope

### Integration tests (`tests/predicate_expr_test.rs`)

| Scenario | Description |
|---|---|
| Comparison filter | `[(< ?age 30)]` keeps only matching rows |
| Two-variable comparison | `[(>= ?a ?b)]` |
| Arithmetic binding | `[(* ?price ?qty) ?total]` |
| Nested arithmetic | `[(+ (* ?a 2) ?b) ?result]` |
| Integer division truncation | `[(/ ?a ?b) ?r]` where both are integers; `5 / 2 = 2` |
| Integer/float promotion | `[(+ ?int ?float) ?r]` returns `Float` |
| Type predicate filter | `[(string? ?v)]` |
| `starts-with?` filter | `[(starts-with? ?tag "work")]` |
| `contains?` filter | `[(contains? ?bio "engineer")]` |
| `matches?` filter | `[(matches? ?email "^[^@]+@[^@]+$")]` |
| `matches?` invalid regex → parse error | `[(matches? ?v "[unclosed")]` rejected at parse time |
| Predicate binding | `[(integer? ?v) ?is-int]` binds `true`/`false` |
| `=` across types | `(= ?name "Alice")` string equality works; `(= 1 1.0)` false |
| Type mismatch → drop | `[(< ?v 100)]` where `?v = "hello"` → row silently dropped |
| Division by zero → drop | `[(/ ?a ?b) ?r]` where `?b = 0` → row silently dropped |
| Expr inside `not` body | valid and evaluated correctly |
| Expr in rule body | `[(< ?a ?b)]` inside a rule body clause |
| Bi-temporal filter | `[(< ?age 30)]` combined with `:as-of` |
| Arithmetic binding into aggregate | `[(* ?price ?qty) ?total]` followed by `(sum ?total)` in `:find` |

---

## Files Modified

| File | Change |
|---|---|
| `Cargo.toml` | Add `regex-lite = "0.1"` dependency (~68K incremental binary cost; estimated total 812K, under 1MB goal) |
| `src/query/datalog/types.rs` | Add `BinOp`, `UnaryOp`, `Expr`; add `WhereClause::Expr` variant; add arms to exhaustive matches |
| `src/query/datalog/parser.rs` | Update both where-clause dispatch sites (query + rule body); add `parse_expr`; update `outer_vars_from_clause` for `Expr`; safety check |
| `src/query/datalog/executor.rs` | Dispatch `WhereClause::Expr`; add `apply_expr_clause`, `eval_expr`, `is_truthy` |
| `src/query/datalog/optimizer.rs` | Pass-through `WhereClause::Expr` unchanged |
| `tests/predicate_expr_test.rs` | New integration test file |

---

## Non-Goals (out of scope for 7.2b)

- Window functions (Phase 7.9a)
- UDF registration via `FunctionRegistry` (Phase 7.9b)
- Pseudo-attribute bindings (`:db/valid-from` etc.) — Phase 7.7
- String functions beyond `starts-with?`, `contains?`, `matches?` (e.g., `ends-with?`, `upper-case`) — can be added incrementally
- Three-or-more-argument expressions
