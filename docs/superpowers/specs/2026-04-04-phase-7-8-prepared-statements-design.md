# Phase 7.8 — Prepared Statements Design

**Date**: 2026-04-04  
**Status**: Approved  
**Roadmap ref**: Phase 7.8

---

## Goal

Parse and plan a query once; execute it repeatedly with different bind values — including temporal filters — without re-parsing or re-planning on each call.

Primary use case: an agent running the same query pattern thousands of times per session with different entity IDs and temporal coordinates.

---

## Syntax

`$identifier` tokens in any bind-able position are treated as named slots:

```datalog
(query [:find ?status
        :as-of $tx
        :valid-at $date
        :where [$entity :employment/status ?status]
               [(>= ?status $min-level)]])
```

### Permitted slot positions

| Position | Permitted `BindValue` variants |
|---|---|
| Entity position in pattern | `Entity(Uuid)` |
| Value position in pattern | `Val(Value)` |
| `:as-of` | `TxCount(u64)`, `Timestamp(i64)` |
| `:valid-at` | `Timestamp(i64)`, `AnyValidTime` |
| `Expr` literal in expression clause | `Val(Value)` |

Attribute positions are **not** parameterisable — the optimizer selects an index based on the attribute at prepare time; a slot there makes index selection impossible. Rejected at `prepare()` time with a clear error.

---

## New types

### `EdnValue::BindSlot` (in `types.rs`)

```rust
/// A named bind slot: `$identifier`. Only valid in a PreparedQuery AST —
/// rejected if encountered by the executor without prior substitution.
BindSlot(String),
```

### `AsOf::Slot` / `ValidAt::Slot` (in `types.rs`)

```rust
pub enum AsOf {
    Counter(u64),
    Timestamp(i64),
    Slot(String),   // $name — resolved at execute() time
}

pub enum ValidAt {
    Timestamp(i64),
    AnyValidTime,
    Slot(String),   // $name — resolved at execute() time
}
```

### `Expr::Slot` (in `types.rs`)

```rust
pub enum Expr {
    Var(String),
    Lit(Value),
    Slot(String),   // $name — substituted before execution
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}
```

Keeping slots out of `Value` prevents unsubstituted slots from silently reaching the expression evaluator.

### `BindValue` (in `prepared.rs`, public)

```rust
pub enum BindValue {
    Entity(Uuid),       // entity position in a pattern
    Val(Value),         // value position in a pattern or expr literal
    TxCount(u64),       // :as-of counter
    Timestamp(i64),     // :as-of wall-clock OR :valid-at millis
    AnyValidTime,       // :valid-at :any-valid-time sentinel
}
```

### `PreparedQuery` (in `prepared.rs`, public)

```rust
pub struct PreparedQuery {
    /// The parsed, optimized query template (contains BindSlot nodes).
    template: DatalogQuery,
    /// Slot names present in this query (for validation at execute time).
    slot_names: Vec<String>,
    // Shared handles from the originating Minigraf instance.
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
}
```

---

## API surface

### `Minigraf::prepare()`

```rust
/// Parse and plan a query once; bind slots (`$name`) are left unresolved.
/// Returns a `PreparedQuery` that can be executed many times with different bindings.
///
/// # Errors
/// - Parse failure
/// - Attribute position used as a bind slot (rejected at prepare time)
/// - Non-query command (transact/retract/rule are not preparable)
pub fn prepare(&self, query_str: &str) -> Result<PreparedQuery>
```

### `PreparedQuery::execute()`

```rust
/// Substitute bind values and execute the query against the current database state.
///
/// # Errors
/// - Missing bind value for a slot present in the query
/// - Type mismatch (e.g. `Val` supplied for an `:as-of` slot)
/// Extra bind values not referenced by any slot are silently ignored.
pub fn execute(&self, bindings: &[(&str, BindValue)]) -> Result<QueryResult>
```

`db.execute(str)` is unchanged — no breaking change.

---

## Implementation approach

**Option A — Inline substitution at `execute()` time.**

Parse into a `DatalogQuery` with `BindSlot` / `Slot` variants. On each `execute()`, deep-clone the AST and walk it replacing slot nodes with concrete values, then pass the filled-in query to the existing executor unchanged.

- Executor, evaluator, matcher, optimizer, and parser are not modified.
- Plan reuse: `optimizer::plan()` runs once at `prepare()` time; the optimized pattern order is stored in the template and reused.
- Clone cost is negligible — a typical query AST is tens of nodes.

---

## Module structure

### New file: `src/query/datalog/prepared.rs`

Contains:
- `BindValue` (public)
- `PreparedQuery` (public)
- `prepare_query(query: DatalogQuery, ...) -> Result<PreparedQuery>` — internal constructor
- `validate_no_attribute_slots(query: &DatalogQuery) -> Result<()>`
- `collect_slot_names(query: &DatalogQuery) -> Vec<String>` — recursive AST walk
- `substitute(template: &DatalogQuery, bindings: &HashMap<&str, &BindValue>) -> Result<DatalogQuery>` — clone + fill

### Changes to existing files

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `EdnValue::BindSlot`, `AsOf::Slot`, `ValidAt::Slot`, `Expr::Slot` |
| `src/query/datalog/parser.rs` | Parse `$identifier` tokens as `EdnValue::BindSlot` and `AsOf::Slot` / `ValidAt::Slot` |
| `src/query/datalog/mod.rs` | `pub mod prepared` |
| `src/db.rs` | Add `Minigraf::prepare()`, import `PreparedQuery` + `BindValue` |
| `src/lib.rs` | Re-export `PreparedQuery` + `BindValue` |

The executor, evaluator, matcher, optimizer, stratification, and rules modules are **not modified**.

---

## Substitution logic

### At `prepare()` time

1. **Parse** the query string into a `DatalogQuery` (slots present as `BindSlot`/`Slot` variants).
2. **Validate** no attribute-position slots — error immediately if found.
3. **Collect** all slot names from entity/value positions, `:as-of`, `:valid-at`, and `Expr::Slot` nodes.
4. **Run optimizer** (`optimizer::plan()`) on the template patterns — index hints and join order are fixed.
5. Return `PreparedQuery` with the optimized template and slot name list.

### At `execute()` time

1. **Completeness check** — every name in `slot_names` must appear in `bindings`. Missing → error with slot name.
2. **Type check** — position determines permitted `BindValue` variants (see table above).
3. **Clone + substitute** — deep-clone the template, walk the AST, replace every slot node with the concrete value.
4. **Execute** — pass the filled-in `DatalogQuery` to `DatalogExecutor::execute_query` unchanged.

Extra bindings (names not in `slot_names`) are silently ignored.

---

## Error messages

| Scenario | Message |
|---|---|
| Attribute position bind slot | `"bind slot '$name' is not permitted in attribute position"` |
| Non-query command prepared | `"only (query ...) commands can be prepared; got transact/retract/rule"` |
| Missing slot at execute | `"missing bind value for slot '$name'"` |
| Type mismatch | `"slot '$name' in :as-of position requires TxCount or Timestamp, got Val"` |
| Unsubstituted slot reaches executor | `"internal: unsubstituted bind slot '$name' in query AST"` (panic guard) |

---

## Tests

All tests live in `tests/prepared_statements_test.rs`.

| Test | What it covers |
|---|---|
| `prepare_and_execute_entity_slot` | `[$entity :attr ?v]` with `BindValue::Entity` |
| `prepare_and_execute_value_slot` | `[?e :attr $val]` with `BindValue::Val` |
| `prepare_and_execute_as_of_counter` | `:as-of $tx` with `BindValue::TxCount` |
| `prepare_and_execute_as_of_timestamp` | `:as-of $tx` with `BindValue::Timestamp` |
| `prepare_and_execute_valid_at` | `:valid-at $date` with `BindValue::Timestamp` |
| `prepare_and_execute_valid_at_any` | `:valid-at $date` with `BindValue::AnyValidTime` |
| `prepare_and_execute_expr_slot` | `[(>= ?v $threshold)]` with `BindValue::Val` |
| `prepare_and_execute_combined` | entity + `:as-of` + `:valid-at` + expr in one query |
| `prepare_rejects_attribute_slot` | `[?e $attr ?v]` → error at prepare |
| `prepare_rejects_transact` | `(transact [...])` → error at prepare |
| `execute_missing_slot` | Omit a required slot → error |
| `execute_type_mismatch_as_of` | Supply `Val` for `:as-of` slot → error |
| `execute_extra_bindings_ignored` | Supply an unreferenced slot name → succeeds |
| `plan_reused_across_executions` | Prepare once, execute 3× with different bindings, all return correct results |
