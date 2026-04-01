# Phase 7.7a: Window Functions — Design

**Date**: 2026-04-01
**Status**: Approved
**Version target**: v0.16.0

---

## Goal

Expose `SUM OVER`–style window computations natively in Datalog `:find` clauses. Window functions preserve per-row output (unlike regular aggregates which collapse rows), and support optional partitioning and mandatory ordering within each partition.

---

## Scope

### In scope (7.7a)

| Function | Semantics |
|---|---|
| `sum ?v :over (…)` | Cumulative/partition sum |
| `count ?v :over (…)` | Cumulative/partition count |
| `min ?v :over (…)` | Running minimum |
| `max ?v :over (…)` | Running maximum |
| `avg ?v :over (…)` | Running average |
| `rank :over (…)` | Rank within partition |
| `row-number :over (…)` | Sequential row number within partition |

Frame type: unbounded-preceding (accumulate from first row in partition to current row) — the only supported frame in 7.7a.

### Deferred to post-1.0 backlog

- `lag ?v :over (…)` — previous row value in partition
- `lead ?v :over (…)` — next row value in partition
- Sliding frame: `:rows N preceding`

### Decisions

- **`FunctionRegistry` introduced in 7.7a**: all built-in aggregates (Phase 7.2) and window functions are registered into it at startup. 7.7b adds the public `register_aggregate`/`register_predicate` API on top of the same registry.
- **`:partition-by` optional**: absent means the whole result set is treated as one partition.
- **`:order-by` required**: clear parse error if missing.
- **Mixed queries allowed**: regular aggregates and window functions may appear in the same `:find` clause. Regular aggregates collapse rows first; window functions run over the collapsed rows.

---

## Architecture

Five areas, all additive:

1. `src/query/datalog/functions.rs` (new) — `FunctionRegistry`
2. `src/query/datalog/types.rs` — new `FindSpec::Window` variant + supporting types
3. `src/query/datalog/parser.rs` — parse `(func ?v :over (...))` in `:find`
4. `src/query/datalog/executor.rs` — unified `apply_post_processing` replaces inline aggregate logic
5. `src/db.rs` — `Minigraf` holds `Arc<RwLock<FunctionRegistry>>`

---

## `FunctionRegistry` (`src/query/datalog/functions.rs`)

```rust
pub struct AggregateDesc {
    pub init: fn() -> AggState,
    pub step: fn(&mut AggState, &Value),
    pub finalise: fn(AggState, usize) -> Value,
    pub window_compatible: bool,  // true = usable in :over clause
}

pub enum AggState {
    Numeric(f64),
    Count(u64),
    Values(Vec<Value>),  // for `distinct`
}

pub struct FunctionRegistry {
    aggregates: HashMap<String, AggregateDesc>,
}

impl FunctionRegistry {
    pub fn with_builtins() -> Self { /* registers all built-ins */ }
    pub(crate) fn get_aggregate(&self, name: &str) -> Option<&AggregateDesc> { … }
    // register_aggregate / register_predicate added in 7.7b
}
```

`AggState` is a closed enum in 7.7a. 7.7b will open it up for UDFs — that change is isolated to `functions.rs`.

All existing Phase 7.2 aggregates (`count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`, `distinct`) are migrated into the registry. The internal `AggregateFunc` enum in `types.rs` is removed; all dispatch goes through the registry. No public API impact.

---

## AST Changes (`src/query/datalog/types.rs`)

```rust
pub enum WindowFunc { Sum, Count, Min, Max, Avg, Rank, RowNumber }
pub enum Order { Asc, Desc }

pub struct WindowSpec {
    pub func: WindowFunc,
    pub var: Option<String>,      // None for rank/row-number
    pub partition_by: Option<String>,
    pub order_by: String,         // required
    pub order: Order,             // default Asc
}

pub enum FindSpec {
    Variable(String),
    Aggregate { func: String, var: String },  // name now a String, dispatched via registry
    Window(WindowSpec),
}
```

---

## Parser Changes (`src/query/datalog/parser.rs`)

- Detect `(func ?v :over (...))` form in `:find` position
- `:partition-by ?var` — optional; absent = whole result set as one partition
- `:order-by ?var` — required; clear parse error if missing
- `:desc` optional after `:order-by ?var`; default `:asc`
- `rank` and `row-number` take no `?var` argument: `(rank :over (...))`
- Unknown function name → parse error
- `lag`/`lead` → "not supported in this version" error
- Non-window-compatible aggregate used in `:over` → parse error
- Mixed `Aggregate` + `Window` in same `:find` → allowed, no validation

---

## Executor Post-Processing (`src/query/datalog/executor.rs`)

Replace inline aggregate logic with:

```rust
fn apply_post_processing(
    bindings: Vec<HashMap<String, Value>>,
    find_specs: &[FindSpec],
    registry: &FunctionRegistry,
) -> Vec<Vec<Value>>
```

**Algorithm:**

1. Partition `find_specs` into: plain variables, regular aggregates, window specs.
2. **If any regular aggregates**: group `bindings` by non-aggregate variables; apply `init`/`step`/`finalise` from registry per group → collapsed rows.
3. **If any window specs**: for each `WindowSpec`:
   - Partition rows by `partition_by` value (or treat all as one partition if absent)
   - Sort each partition by `order_by` key, respecting `Order`
   - Walk rows in order; maintain accumulator per partition; annotate each row
   - `rank`/`row-number` use position in sorted partition, not an accumulator
4. **Mixed case**: regular aggregates collapse first (step 2), window functions run over collapsed rows (step 3). Consistent with SQL semantics.
5. **Project output**: extract values in `:find` variable order.

Window result stored under a synthetic key (e.g. `"__window_0"`) in the row's binding map, projected to output position in the final step.

---

## `Minigraf` integration (`src/db.rs`)

`Minigraf` gains `Arc<RwLock<FunctionRegistry>>` field, initialized with `FunctionRegistry::with_builtins()` at `open()`. The registry is threaded into the executor via the existing query execution path.

---

## Testing (`tests/window_functions_test.rs`)

**Core window semantics:**
- `sum :over (order-by ?v)` — cumulative sum over whole result set
- `count :over (order-by ?v)` — running count
- `min`/`max :over (order-by ?v)` — running min/max
- `avg :over (order-by ?v)` — running average (f64 result)
- `rank :over (order-by ?v)` — rank within whole result set
- `row-number :over (order-by ?v)` — sequential row number

**Partition-by:**
- `sum :over (partition-by ?dept :order-by ?salary)` — resets per partition
- Multiple distinct partitions produce independent accumulators
- Single-row partition produces correct result

**Mixed queries:**
- Regular `(count ?e)` aggregate + `(sum ?salary :over (...))` window in same `:find`
- Window result appears in correct output column position

**Ordering:**
- `:desc` produces correct reverse-order rank/accumulation
- Ties in `order-by` key: `rank` assigns same rank; `row-number` stable (insertion order within tie)

**Edge cases:**
- Empty result set → empty output (no panic)
- Single-row result → window value equals the row's own value
- `lag`/`lead` at parse time → clear "not supported" error
- Unknown function name → parse error
- Non-window-compatible aggregate in `:over` → parse error

**`FunctionRegistry` unit tests** (in `src/query/datalog/functions.rs` `#[cfg(test)]`):
- All built-ins registered by `with_builtins()`
- `window_compatible` flag correct for each built-in
- Unknown name returns `None`

---

## File format / public API impact

- No file format changes (window computation is query-time only)
- No breaking public API changes (`FindSpec` and `AggregateFunc` are internal types)
- `Minigraf` gains `Arc<RwLock<FunctionRegistry>>` field — additive, not breaking

---

## Out of scope

- `lag`, `lead`, sliding frames — post-1.0 backlog (update ROADMAP.md at phase start)
- Public UDF registration API — Phase 7.7b
- Cost-based optimizer awareness of window functions — post-1.0 backlog
