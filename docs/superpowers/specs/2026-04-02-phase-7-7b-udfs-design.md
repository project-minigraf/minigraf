# Phase 7.7b — User-Defined Functions (UDFs) Design Spec

**Date**: 2026-04-02  
**Phase**: 7.7b  
**Version target**: v0.17.0  
**Depends on**: Phase 7.7a (Window Functions, v0.16.0)

---

## Overview

Phase 7.7b exposes the `FunctionRegistry` introduced in 7.7a as a public extension point, letting embedders register custom aggregate functions and filter predicates at runtime. UDFs plug into the same evaluation paths as built-ins — grouping aggregation, window computation, and `:where` predicate filtering — without modifying the query engine core.

---

## Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Accumulator representation | `Box<dyn Any + Send>` (type-erased) for UDFs; `AggState` retained for built-ins | Avoids refactoring proven 7.7a built-in path; clean separation via `AggImpl` enum |
| Registry structure | One `HashMap<String, AggregateDesc>` with `AggImpl` discriminator; separate `HashMap<String, PredicateDesc>` | Single lookup path; built-ins and UDFs coexist in one map |
| Validation timing | **Runtime** — unknown names produce `QueryError` at execution time, not parse errors | Parser stays stateless; consistent with how rule names are resolved |
| Window compatibility | All UDF aggregates are automatically window-compatible | Simpler API; embedder doesn't manage a flag |
| Name collision | Any duplicate name (built-in or UDF) returns `Err` on registration | Predictable; no silent overrides |
| Predicate syntax | `[(email? ?addr)]` — square brackets, consistent with built-in predicates | No ambiguity with rule invocations; reuses existing `Expr` path |

---

## Architecture

### `functions.rs` changes

**New types:**

```rust
/// Closure-based ops for a UDF aggregate.
/// Accumulator is type-erased as Box<dyn Any + Send>.
pub struct UdfOps {
    pub init:     Arc<dyn Fn() -> Box<dyn Any + Send> + Send + Sync>,
    pub step:     Arc<dyn Fn(&mut Box<dyn Any + Send>, &Value) + Send + Sync>,
    pub finalise: Arc<dyn Fn(&Box<dyn Any + Send>, usize) -> Value + Send + Sync>,
}

/// Discriminates between the built-in fn()-pointer path and the UDF closure path.
pub enum AggImpl {
    Builtin(WindowOps),  // existing fn() pointers + AggState; unchanged
    Udf(UdfOps),         // type-erased closures; Box<dyn Any + Send> accumulator
}

/// Descriptor for a single registered predicate function.
pub struct PredicateDesc {
    pub f:          Arc<dyn Fn(&Value) -> bool + Send + Sync>,
    pub is_builtin: bool,
}
```

**`AggregateDesc` updated:**

```rust
pub struct AggregateDesc {
    pub impl_:      AggImpl,   // replaces (window_compatible, window_ops) pair
    pub is_builtin: bool,
}
```

All built-ins keep `AggImpl::Builtin(WindowOps)` and `is_builtin: true`. The `window_compatible` flag is implicit: `AggImpl::Builtin` is always window-compatible; `AggImpl::Udf` is always window-compatible.

**`FunctionRegistry` updated:**

```rust
pub struct FunctionRegistry {
    aggregates: HashMap<String, AggregateDesc>,
    predicates: HashMap<String, PredicateDesc>,  // new
}
```

New internal methods:
- `register_aggregate_desc(name: String, desc: AggregateDesc) -> Result<()>` — rejects duplicates
- `register_predicate_desc(name: String, desc: PredicateDesc) -> Result<()>` — rejects duplicates
- `get_predicate(name: &str) -> Option<&PredicateDesc>`

`with_builtins()` is updated to also populate the `predicates` map with built-in predicate names (`string?`, `integer?`, `float?`, `boolean?`, `nil?`, `starts-with?`, `ends-with?`, `contains?`, `matches?`) as `PredicateDesc { is_builtin: true }` entries. This ensures `register_predicate("string?", ...)` correctly returns `Err` — the `predicates` map is the single source of truth for collision detection.

Existing methods updated to work with `AggImpl`:
- `is_window_compatible`: always `true` (both variants are window-compatible)
- `is_known`: checks `aggregates` map only (predicates are separate)

---

### `types.rs` changes

**`WindowFunc` gains one variant:**

```rust
pub enum WindowFunc {
    Sum, Count, Min, Max, Avg, Rank, RowNumber,
    Udf(String),  // any unrecognised name in :over position
}
```

**`UnaryOp` gains one variant:**

```rust
pub enum UnaryOp {
    StringQ, IntegerQ, FloatQ, BooleanQ, NilQ,
    Udf(String),  // any unrecognised bare predicate name in [(name? ?var)] position
}
```

No other AST types change.

---

### `parser.rs` changes

**Window function parsing (`parse_window_expr`):**

- Keeps explicit rejection of `lag` / `lead` (deferred to post-1.0)
- Any other unrecognised function name emits `WindowFunc::Udf(name)` instead of a parse error

**Predicate expression parsing (`parse_expr`):**

- Currently has a whitelist of known predicate names mapping to `UnaryOp` variants
- Any bare name not in the whitelist, in a **1-arg form** `[(name arg)]`, emits `UnaryOp::Udf(name)`
- Unknown 2-arg forms `[(name arg1 arg2)]` still produce a parse error (no UDF binary operators)
- No parse error for unknown predicate names in 1-arg position

---

### `db.rs` changes

Two new public methods on `Minigraf`:

```rust
/// Register a custom aggregate function.
///
/// The accumulator type `Acc` is erased to `Box<dyn Any + Send>` internally.
/// The registered function is usable in both `:find` grouping position and
/// `:over` (window) position.
///
/// Returns `Err` if `name` is already registered (built-in or UDF).
pub fn register_aggregate<Acc>(
    &self,
    name: &str,
    init:     impl Fn() -> Acc              + Send + Sync + 'static,
    step:     impl Fn(&mut Acc, &Value)     + Send + Sync + 'static,
    finalise: impl Fn(&Acc, usize) -> Value + Send + Sync + 'static,
) -> Result<()>
where
    Acc: Any + Send + 'static,

/// Register a custom single-argument filter predicate.
///
/// The predicate is usable in `[(name? ?var)]` `:where` clauses.
/// Returns `Err` if `name` is already registered (built-in or UDF).
pub fn register_predicate(
    &self,
    name: &str,
    f: impl Fn(&Value) -> bool + Send + Sync + 'static,
) -> Result<()>
```

**Internal implementation of `register_aggregate`:**

The three user-supplied closures are wrapped into type-erased `Arc<dyn Fn...>` shims:

```rust
// Example — step shim
let step_boxed: Arc<dyn Fn(&mut Box<dyn Any + Send>, &Value) + Send + Sync> =
    Arc::new(move |acc, v| {
        // downcast is safe: type fixed at registration time, never changes
        step(acc.downcast_mut::<Acc>().expect("UDF acc type mismatch"), v);
    });
```

Both methods acquire a write lock on `self.inner.functions`, build the appropriate descriptor, and call the internal registry method. The `expect` in the shim is a logic invariant (the same `init` that creates the accumulator determines its type), not a user-visible error path.

---

### `executor.rs` changes

**Grouping aggregation (`apply_aggregation`):**

```
match registry.get(func_name).map(|d| &d.impl_) {
    Some(AggImpl::Builtin(_)) => existing built-in path (unchanged)
    Some(AggImpl::Udf(ops))   => init → step each value → finalise(acc, count)
    None                       => QueryError::UnknownFunction(func_name)
}
```

Empty result sets return `Value::Null` for UDF aggregates (same as built-ins).

**Window aggregation (`apply_window_functions`):**

New arm for `WindowFunc::Udf(name)`:
1. Look up `name` in registry → `QueryError::UnknownFunction` if absent
2. Extract `UdfOps` from `AggImpl::Udf` (guaranteed by registry invariant)
3. Run partition → sort → cumulative step → per-row finalise loop, calling `UdfOps` closures — identical structure to built-in loop

**Predicate evaluation (`eval_expr` / `is_truthy`):**

New arm for `UnaryOp::Udf(name)`:
1. Look up `name` in predicate registry → `QueryError::UnknownPredicate` if absent
2. Evaluate argument expression to a `Value`
3. Call `(desc.f)(&value)` → return bool as row keep/drop decision

---

### `lib.rs` changes

No new types need to be added to the public re-export list. `Value` is already exported. `register_aggregate` and `register_predicate` are methods on `Minigraf`, which is already exported. `Any` is from `std`.

---

## Error Cases

| Situation | Error |
|---|---|
| `register_aggregate("sum", ...)` | `Err` — name collides with built-in |
| `register_aggregate("myfn", ...)` called twice | `Err` — name collides with existing UDF |
| `register_predicate("string?", ...)` | `Err` — name collides with built-in predicate |
| `(query [:find (nosuchfn ?x) :where ...])` | `QueryError::UnknownFunction("nosuchfn")` |
| `[(nosuchpred? ?x)]` in `:where` | `QueryError::UnknownPredicate("nosuchpred?")` |
| `(geomean ?score :over (...))` before registration | `QueryError::UnknownFunction("geomean")` |

All errors surface as `Result::Err` return values — no panics visible to embedders.

---

## Tests

All tests live in `tests/udf_test.rs`. No `{:?}` format of `Result`/`Value`/`Fact` in assert messages (CodeQL rule — use `.unwrap()` / `.expect("message")` / assert on count/bool).

| # | Name | Description |
|---|------|-------------|
| 1 | `custom_aggregate_geometric_mean` | Register `geomean`; query over fact set; verify correct value |
| 2 | `custom_aggregate_empty_result` | `geomean` over zero matching rows returns `Value::Null` |
| 3 | `custom_predicate_filter` | Register `email?`; `[(email? ?addr)]` keeps/drops rows correctly |
| 4 | `udf_as_window_function` | `(geomean ?score :over (:partition-by ?dept :order-by ?score))` returns correct per-row cumulative value |
| 5 | `name_collision_builtin_aggregate` | `register_aggregate("sum", ...)` returns `Err` |
| 6 | `name_collision_udf_on_udf` | Registering same UDF name twice returns `Err` |
| 7 | `unknown_function_runtime_error` | `(query [:find (nosuchfn ?x) :where ...])` returns `Err`, not panic |
| 8 | `unknown_predicate_runtime_error` | `[(nosuchpred? ?x)]` in `:where` returns `Err`, not panic |
| 9 | `thread_safety` | `Arc<Minigraf>` shared across threads; concurrent reads + one UDF registration; no deadlock or data race |

---

## Scope Explicitly Excluded

- UDF predicates with more than one argument (multi-arg predicates deferred)
- Persisting UDF registrations to the `.graph` file (functions are code, not data — matches SQLite philosophy)
- Deregistering / replacing a registered UDF (rejected by name-collision rule)
- Built-in aggregate migration to `Box<dyn Any>` (built-ins keep `AggState` path)
- `lag` / `lead` / sliding window frames (post-1.0 backlog, unchanged)
