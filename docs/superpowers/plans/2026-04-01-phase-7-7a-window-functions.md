# Phase 7.7a: Window Functions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `sum/count/min/max/avg/rank/row-number :over (partition-by … :order-by …)` window functions to the Datalog `:find` clause, backed by a `FunctionRegistry` that will accept UDFs in Phase 7.7b.

**Architecture:** Post-evaluation pass — the core evaluation engine (pattern matching, joins, not, or) is unchanged. A new `apply_post_processing` function replaces the existing `apply_aggregation` and handles regular aggregates (via `FunctionRegistry`), window functions (partition → sort → accumulate), and mixed queries (aggregates collapse first, windows annotate after). All built-in aggregate dispatch is migrated from the `AggFunc` enum to a string-keyed `FunctionRegistry`.

**Tech Stack:** Rust stable; no new dependencies.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src/query/datalog/functions.rs` | **Create** | `FunctionRegistry`, `AggState`, `WindowOps`, `AggregateDesc`, `apply_builtin_aggregate`, `value_lt`, `value_type_name` |
| `src/query/datalog/mod.rs` | **Modify** | Add `pub mod functions;` |
| `src/query/datalog/types.rs` | **Modify** | Remove `AggFunc`; add `WindowFunc`, `Order`, `WindowSpec`, `FindSpec::Window`; change `FindSpec::Aggregate.func: AggFunc` → `func: String` |
| `src/query/datalog/parser.rs` | **Modify** | Update `parse_aggregate` (string func names); add `parse_window_expr` |
| `src/query/datalog/executor.rs` | **Modify** | Add `functions` field; replace `apply_aggregation`+`apply_agg_func` with `apply_post_processing`+helpers |
| `src/db.rs` | **Modify** | Add `functions: Arc<RwLock<FunctionRegistry>>` to `Inner`; pass to executor |
| `tests/window_functions_test.rs` | **Create** | Integration tests for all window function behaviour |
| `ROADMAP.md` | **Modify** | Move `lag`/`lead`/sliding frames to post-1.0 backlog; update 7.7 sub-phase list |

---

## Task 1: Update ROADMAP.md

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Mark lag/lead/sliding frames as post-1.0 in ROADMAP.md**

In the Phase 7.7 section, update the 7.7a deliverable and add a note. Find the line:

```
- **7.7** Window Functions + UDFs (`sum/count/rank/lag/lead :over (partition-by … :order-by …)`; embedder-registered aggregate and predicate UDFs via `FunctionRegistry`)
```

Replace with:

```
- **7.7a** Window Functions — `sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number` with unbounded-preceding frame; `FunctionRegistry` introduced (built-ins only); `lag`/`lead` and sliding frames deferred to post-1.0 backlog
- **7.7b** User-Defined Functions (UDFs) — public `register_aggregate`/`register_predicate` API on the `FunctionRegistry` introduced in 7.7a
```

Also find the `#### 7.7a Window Functions` sub-section that lists `lag` and `lead` in the supported functions table. Remove those two rows and add a note:

```
> **Note:** `lag`, `lead`, and the `:rows N preceding` sliding frame are deferred to the post-1.0 backlog. 7.7a ships `sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number` with unbounded-preceding (cumulative from partition start to current row) only.
```

- [ ] **Step 2: Commit**

```bash
git add ROADMAP.md
git commit -m "docs: defer lag/lead/sliding frames to post-1.0 backlog in roadmap"
```

---

## Task 2: Create `src/query/datalog/functions.rs`

**Files:**
- Create: `src/query/datalog/functions.rs`
- Modify: `src/query/datalog/mod.rs`

- [ ] **Step 1: Write unit tests (they will fail until the module exists)**

Create `src/query/datalog/functions.rs` with just the test module first:

```rust
// src/query/datalog/functions.rs
// (stub — tests written first, implementation follows)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;

    #[test]
    fn registry_knows_all_builtins() {
        let reg = FunctionRegistry::with_builtins();
        for name in ["count", "count-distinct", "sum", "sum-distinct", "min", "max", "avg"] {
            assert!(reg.is_known(name), "expected '{}' to be registered", name);
        }
    }

    #[test]
    fn window_compatible_flags() {
        let reg = FunctionRegistry::with_builtins();
        for name in ["count", "sum", "min", "max", "avg"] {
            assert!(reg.is_window_compatible(name), "'{}' should be window-compatible", name);
        }
        for name in ["count-distinct", "sum-distinct"] {
            assert!(!reg.is_window_compatible(name), "'{}' should NOT be window-compatible", name);
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        let reg = FunctionRegistry::with_builtins();
        assert!(reg.get("nonexistent").is_none());
        assert!(!reg.is_known("nonexistent"));
    }

    #[test]
    fn apply_builtin_count() {
        let vals = vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("count", &refs).unwrap();
        assert_eq!(result, Value::Integer(3));
    }

    #[test]
    fn apply_builtin_sum_integers() {
        let vals = vec![Value::Integer(10), Value::Integer(20), Value::Integer(30)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("sum", &refs).unwrap();
        assert_eq!(result, Value::Integer(60));
    }

    #[test]
    fn apply_builtin_sum_floats() {
        let vals = vec![Value::Float(1.5), Value::Float(2.5)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("sum", &refs).unwrap();
        assert_eq!(result, Value::Float(4.0));
    }

    #[test]
    fn apply_builtin_min() {
        let vals = vec![Value::Integer(30), Value::Integer(10), Value::Integer(20)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("min", &refs).unwrap();
        assert_eq!(result, Value::Integer(10));
    }

    #[test]
    fn apply_builtin_max() {
        let vals = vec![Value::Integer(30), Value::Integer(10), Value::Integer(20)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("max", &refs).unwrap();
        assert_eq!(result, Value::Integer(30));
    }

    #[test]
    fn apply_builtin_count_distinct() {
        let vals = vec![Value::Integer(1), Value::Integer(2), Value::Integer(1)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("count-distinct", &refs).unwrap();
        assert_eq!(result, Value::Integer(2));
    }

    #[test]
    fn apply_builtin_min_empty_errors() {
        let result = apply_builtin_aggregate("min", &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no non-null values"));
    }

    #[test]
    fn value_lt_integers() {
        assert!(value_lt(&Value::Integer(1), &Value::Integer(2)));
        assert!(!value_lt(&Value::Integer(2), &Value::Integer(1)));
        assert!(!value_lt(&Value::Integer(1), &Value::Integer(1)));
    }

    #[test]
    fn value_lt_strings() {
        assert!(value_lt(&Value::String("a".into()), &Value::String("b".into())));
        assert!(!value_lt(&Value::String("b".into()), &Value::String("a".into())));
    }

    #[test]
    fn window_ops_sum_accumulator() {
        let reg = FunctionRegistry::with_builtins();
        let desc = reg.get("sum").unwrap();
        let ops = desc.window_ops.as_ref().unwrap();
        let mut state = (ops.init)();
        (ops.step)(&mut state, &Value::Integer(10));
        assert_eq!((ops.finalise)(&state), Value::Integer(10));
        (ops.step)(&mut state, &Value::Integer(20));
        assert_eq!((ops.finalise)(&state), Value::Integer(30));
    }

    #[test]
    fn window_ops_count_accumulator() {
        let reg = FunctionRegistry::with_builtins();
        let desc = reg.get("count").unwrap();
        let ops = desc.window_ops.as_ref().unwrap();
        let mut state = (ops.init)();
        (ops.step)(&mut state, &Value::Integer(1));
        (ops.step)(&mut state, &Value::Integer(2));
        assert_eq!((ops.finalise)(&state), Value::Integer(2));
    }

    #[test]
    fn window_ops_avg_accumulator() {
        let reg = FunctionRegistry::with_builtins();
        let desc = reg.get("avg").unwrap();
        let ops = desc.window_ops.as_ref().unwrap();
        let mut state = (ops.init)();
        (ops.step)(&mut state, &Value::Integer(10));
        (ops.step)(&mut state, &Value::Integer(20));
        assert_eq!((ops.finalise)(&state), Value::Float(15.0));
    }
}
```

- [ ] **Step 2: Add `pub mod functions;` to `src/query/datalog/mod.rs`**

Read the file first, then add the line. The file currently contains module declarations like `pub mod executor;`, `pub mod types;`, etc. Add:

```rust
pub mod functions;
```

alongside the other module declarations.

- [ ] **Step 3: Run tests to confirm they fail (module not yet implemented)**

```bash
cargo test --lib 2>&1 | head -30
```

Expected: compilation error — `FunctionRegistry`, `apply_builtin_aggregate`, `value_lt` not found.

- [ ] **Step 4: Implement `src/query/datalog/functions.rs`**

Replace the stub with the full implementation:

```rust
use crate::graph::types::Value;
use std::collections::HashMap;

/// Accumulator state for window aggregate functions (incremental computation).
/// Each variant is owned by one partition accumulator and cloned from `init()`.
#[derive(Clone, Debug)]
pub enum AggState {
    Sum { total: f64, is_float: bool },
    Count(i64),
    /// Shared by both min and max; semantics determined by the registered `step` fn.
    MinMax { current: Option<Value> },
    Avg { sum: f64, count: usize },
}

/// Incremental operations for window-compatible aggregate functions.
/// All three functions must be consistent with each other.
pub struct WindowOps {
    /// Create a fresh accumulator for a new partition.
    pub init: fn() -> AggState,
    /// Incorporate one more value into the accumulator.
    pub step: fn(&mut AggState, &Value),
    /// Read the current window value without consuming the accumulator.
    pub finalise: fn(&AggState) -> Value,
}

/// Descriptor for one registered aggregate function.
pub struct AggregateDesc {
    /// True if this function can appear inside an `:over` clause.
    pub window_compatible: bool,
    /// Present iff `window_compatible` is true.
    pub window_ops: Option<WindowOps>,
}

/// Registry of aggregate function descriptors, keyed by hyphenated name.
///
/// In 7.7a this holds only built-ins. Phase 7.7b will add
/// `register_aggregate` / `register_predicate` public methods.
pub struct FunctionRegistry {
    aggregates: HashMap<String, AggregateDesc>,
}

impl FunctionRegistry {
    /// Build a registry pre-populated with all built-in aggregate functions.
    pub fn with_builtins() -> Self {
        let mut reg = Self {
            aggregates: HashMap::new(),
        };

        // ── count (window-compatible) ───────────────────────────────────────
        reg.aggregates.insert(
            "count".into(),
            AggregateDesc {
                window_compatible: true,
                window_ops: Some(WindowOps {
                    init: || AggState::Count(0),
                    step: |state, v| {
                        if !matches!(v, Value::Null) {
                            if let AggState::Count(n) = state {
                                *n += 1;
                            }
                        }
                    },
                    finalise: |state| {
                        if let AggState::Count(n) = state {
                            Value::Integer(*n)
                        } else {
                            Value::Null
                        }
                    },
                }),
            },
        );

        // ── sum (window-compatible) ─────────────────────────────────────────
        reg.aggregates.insert(
            "sum".into(),
            AggregateDesc {
                window_compatible: true,
                window_ops: Some(WindowOps {
                    init: || AggState::Sum { total: 0.0, is_float: false },
                    step: |state, v| {
                        if let AggState::Sum { total, is_float } = state {
                            match v {
                                Value::Integer(i) => *total += *i as f64,
                                Value::Float(f) => {
                                    *total += f;
                                    *is_float = true;
                                }
                                _ => {}
                            }
                        }
                    },
                    finalise: |state| {
                        if let AggState::Sum { total, is_float } = state {
                            if *is_float {
                                Value::Float(*total)
                            } else {
                                Value::Integer(*total as i64)
                            }
                        } else {
                            Value::Null
                        }
                    },
                }),
            },
        );

        // ── min (window-compatible) ─────────────────────────────────────────
        reg.aggregates.insert(
            "min".into(),
            AggregateDesc {
                window_compatible: true,
                window_ops: Some(WindowOps {
                    init: || AggState::MinMax { current: None },
                    step: |state, v| {
                        if matches!(v, Value::Null) {
                            return;
                        }
                        if let AggState::MinMax { current } = state {
                            match current {
                                None => *current = Some(v.clone()),
                                Some(cur) => {
                                    if value_lt(v, cur) {
                                        *current = Some(v.clone());
                                    }
                                }
                            }
                        }
                    },
                    finalise: |state| {
                        if let AggState::MinMax { current } = state {
                            current.clone().unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    },
                }),
            },
        );

        // ── max (window-compatible) ─────────────────────────────────────────
        reg.aggregates.insert(
            "max".into(),
            AggregateDesc {
                window_compatible: true,
                window_ops: Some(WindowOps {
                    init: || AggState::MinMax { current: None },
                    step: |state, v| {
                        if matches!(v, Value::Null) {
                            return;
                        }
                        if let AggState::MinMax { current } = state {
                            match current {
                                None => *current = Some(v.clone()),
                                Some(cur) => {
                                    if value_lt(cur, v) {
                                        *current = Some(v.clone());
                                    }
                                }
                            }
                        }
                    },
                    finalise: |state| {
                        if let AggState::MinMax { current } = state {
                            current.clone().unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    },
                }),
            },
        );

        // ── avg (window-compatible) ─────────────────────────────────────────
        reg.aggregates.insert(
            "avg".into(),
            AggregateDesc {
                window_compatible: true,
                window_ops: Some(WindowOps {
                    init: || AggState::Avg { sum: 0.0, count: 0 },
                    step: |state, v| {
                        if let AggState::Avg { sum, count } = state {
                            match v {
                                Value::Integer(i) => {
                                    *sum += *i as f64;
                                    *count += 1;
                                }
                                Value::Float(f) => {
                                    *sum += f;
                                    *count += 1;
                                }
                                _ => {}
                            }
                        }
                    },
                    finalise: |state| {
                        if let AggState::Avg { sum, count } = state {
                            if *count == 0 {
                                Value::Null
                            } else {
                                Value::Float(*sum / *count as f64)
                            }
                        } else {
                            Value::Null
                        }
                    },
                }),
            },
        );

        // ── count-distinct (NOT window-compatible) ──────────────────────────
        reg.aggregates.insert(
            "count-distinct".into(),
            AggregateDesc { window_compatible: false, window_ops: None },
        );

        // ── sum-distinct (NOT window-compatible) ────────────────────────────
        reg.aggregates.insert(
            "sum-distinct".into(),
            AggregateDesc { window_compatible: false, window_ops: None },
        );

        reg
    }

    /// Look up a descriptor by function name. Returns `None` if unknown.
    pub fn get(&self, name: &str) -> Option<&AggregateDesc> {
        self.aggregates.get(name)
    }

    /// Returns `true` if `name` is a known registered function.
    pub fn is_known(&self, name: &str) -> bool {
        self.aggregates.contains_key(name)
    }

    /// Returns `true` if `name` is known AND window-compatible.
    pub fn is_window_compatible(&self, name: &str) -> bool {
        self.aggregates
            .get(name)
            .map(|d| d.window_compatible)
            .unwrap_or(false)
    }
}

/// Returns `true` if `a < b` by value ordering.
/// Mixed Integer/Float comparisons promote to f64.
/// Returns `false` for incomparable types (no panic).
pub(crate) fn value_lt(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x < y,
        (Value::Float(x), Value::Float(y)) => x < y,
        (Value::Integer(x), Value::Float(y)) => (*x as f64) < *y,
        (Value::Float(x), Value::Integer(y)) => *x < (*y as f64),
        (Value::String(x), Value::String(y)) => x < y,
        _ => false,
    }
}

/// Human-readable type name for error messages.
pub(crate) fn value_type_name(v: &Value) -> &'static str {
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

/// Apply a built-in aggregate function to a slice of non-null values (batch mode).
/// This is the group-by aggregation path used by regular (non-window) queries.
pub fn apply_builtin_aggregate(name: &str, values: &[&Value]) -> anyhow::Result<Value> {
    match name {
        "count" => Ok(Value::Integer(values.len() as i64)),

        "count-distinct" => {
            let mut seen: Vec<&Value> = Vec::new();
            for v in values {
                if !seen.contains(v) {
                    seen.push(v);
                }
            }
            Ok(Value::Integer(seen.len() as i64))
        }

        "sum" | "sum-distinct" => {
            let deduped: Vec<&Value> = if name == "sum-distinct" {
                let mut seen: Vec<&Value> = Vec::new();
                for v in values {
                    if !seen.contains(v) {
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
                            return Err(anyhow::anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ));
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
                            return Err(anyhow::anyhow!(
                                "sum: expected Integer, Float, or Null, got {}",
                                value_type_name(other)
                            ));
                        }
                    }
                }
                Ok(Value::Integer(sum))
            }
        }

        "min" | "max" => {
            if values.is_empty() {
                return Err(anyhow::anyhow!("min/max: no non-null values in group"));
            }
            let first = values[0];
            for v in &values[1..] {
                if std::mem::discriminant(*v) != std::mem::discriminant(first) {
                    return Err(anyhow::anyhow!(
                        "{}: cannot compare {} and {} values",
                        name,
                        value_type_name(first),
                        value_type_name(v)
                    ));
                }
            }
            let result = values.iter().try_fold((*values[0]).clone(), |acc, v| {
                let ordering = match (&acc, v) {
                    (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
                    (Value::Float(a), Value::Float(b)) => {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(a), Value::String(b)) => a.cmp(b),
                    (_, other) => {
                        return Err(anyhow::anyhow!(
                            "{}: expected Integer, Float, String, or Null, got {}",
                            name,
                            value_type_name(other)
                        ));
                    }
                };
                let replace = if name == "min" {
                    ordering == std::cmp::Ordering::Greater
                } else {
                    ordering == std::cmp::Ordering::Less
                };
                Ok::<Value, anyhow::Error>(if replace { (*v).clone() } else { acc })
            })?;
            Ok(result)
        }

        other => Err(anyhow::anyhow!("unknown aggregate function: '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    // (paste the test module from Step 1 here)
}
```

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cargo test query::datalog::functions
```

Expected: all tests in the module pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/functions.rs src/query/datalog/mod.rs
git commit -m "feat: add FunctionRegistry with built-in aggregate descriptors"
```

---

## Task 3: Update `src/query/datalog/types.rs`

**Files:**
- Modify: `src/query/datalog/types.rs`

- [ ] **Step 1: Read the current `AggFunc` and `FindSpec` definitions**

Read `src/query/datalog/types.rs` lines 99–195 to confirm the exact current definitions before editing.

- [ ] **Step 2: Replace `AggFunc` with string-based `FindSpec::Aggregate` and add window types**

In `types.rs`, make these changes:

**2a. Remove the `AggFunc` enum** (lines 99–122 approximately). Delete:

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
```

**2b. Add window types** in place of the removed `AggFunc` block:

```rust
/// Window aggregate functions usable inside an `:over` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowFunc {
    Sum,
    Count,
    Min,
    Max,
    Avg,
    Rank,
    RowNumber,
}

/// Sort direction for the `:order-by` key in a window spec.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    Asc,
    Desc,
}

/// The `:over (...)` clause of a window function expression.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    pub func: WindowFunc,
    /// The variable to accumulate (None for rank/row-number).
    pub var: Option<String>,
    /// Partition variable; None means whole result set is one partition.
    pub partition_by: Option<String>,
    /// Sort key variable (required).
    pub order_by: String,
    pub order: Order,
}

impl WindowSpec {
    /// Returns the registry key name for this function.
    pub fn func_name(&self) -> &'static str {
        match self.func {
            WindowFunc::Sum => "sum",
            WindowFunc::Count => "count",
            WindowFunc::Min => "min",
            WindowFunc::Max => "max",
            WindowFunc::Avg => "avg",
            WindowFunc::Rank => "rank",
            WindowFunc::RowNumber => "row-number",
        }
    }
}
```

**2c. Update `FindSpec`** — change the `Aggregate` variant and add `Window`:

Replace:
```rust
/// A single element in the :find clause: either a plain variable or an aggregate.
#[derive(Debug, Clone, PartialEq)]
pub enum FindSpec {
    /// A plain logic variable: ?name
    Variable(String),
    /// An aggregate expression: (count ?e), (sum ?salary), etc.
    Aggregate { func: AggFunc, var: String },
}
```

With:
```rust
/// A single element in the :find clause.
#[derive(Debug, Clone, PartialEq)]
pub enum FindSpec {
    /// A plain logic variable: ?name
    Variable(String),
    /// A regular aggregate: (count ?e), (sum ?salary), etc.
    /// `func` is the hyphenated name registered in `FunctionRegistry`.
    Aggregate { func: String, var: String },
    /// A window function: (sum ?salary :over (:order-by ?hire-date))
    Window(WindowSpec),
}
```

**2d. Update `FindSpec::display_name` and `FindSpec::var`**:

Replace:
```rust
impl FindSpec {
    pub fn display_name(&self) -> String {
        match self {
            FindSpec::Variable(v) => v.clone(),
            FindSpec::Aggregate { func, var } => format!("({} {})", func.as_str(), var),
        }
    }

    pub fn var(&self) -> &str {
        match self {
            FindSpec::Variable(v) => v.as_str(),
            FindSpec::Aggregate { var, .. } => var.as_str(),
        }
    }
}
```

With:
```rust
impl FindSpec {
    pub fn display_name(&self) -> String {
        match self {
            FindSpec::Variable(v) => v.clone(),
            FindSpec::Aggregate { func, var } => format!("({} {})", func, var),
            FindSpec::Window(ws) => match &ws.var {
                Some(v) => format!("({} {} :over ...)", ws.func_name(), v),
                None => format!("({} :over ...)", ws.func_name()),
            },
        }
    }

    /// The logic variable this spec draws values from.
    /// For rank/row-number (no var), returns a synthetic placeholder.
    pub fn var(&self) -> &str {
        match self {
            FindSpec::Variable(v) => v.as_str(),
            FindSpec::Aggregate { var, .. } => var.as_str(),
            FindSpec::Window(ws) => ws.var.as_deref().unwrap_or("__window_var"),
        }
    }
}
```

- [ ] **Step 3: Fix compilation errors from `AggFunc` removal**

Run:
```bash
cargo build 2>&1 | grep "error\[" | head -30
```

The compiler will point to every file that still imports or uses `AggFunc`. Fix each:

- In `src/query/datalog/executor.rs` line ~6: remove `AggFunc` from the `use super::types::{...}` import.
- In `src/query/datalog/parser.rs`: remove `AggFunc` from its `use` import.
- Any remaining `AggFunc::Count` etc. patterns will need updating — these will be addressed in Tasks 4 and 5.

At this point it is acceptable for `executor.rs` and `parser.rs` to have compiler errors; the goal of this step is just to get `types.rs` clean. Patch the `use` lines in executor and parser to remove `AggFunc`, then add `#[allow(unused_imports)]` temporarily if needed to keep the build going.

- [ ] **Step 4: Run existing tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all 647 previously passing tests still pass. If any fail due to the `AggFunc` removal, fix them before proceeding.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/executor.rs src/query/datalog/parser.rs
git commit -m "refactor: replace AggFunc enum with String names in FindSpec; add WindowSpec/WindowFunc/Order"
```

---

## Task 4: Update `src/query/datalog/parser.rs`

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing parse-level tests**

Add a `#[cfg(test)]` block at the bottom of `parser.rs` (or extend the existing one). Locate where the existing parser tests live first:

```bash
grep -n "#\[cfg(test)\]" src/query/datalog/parser.rs | head -5
```

Add these tests to the existing test module (or create one if absent):

```rust
#[test]
fn parse_window_sum_no_partition() {
    let cmd = parse_datalog_command(
        r#"(query [:find ?name (sum ?salary :over (:order-by ?salary))
                   :where [?e :employee/name ?name]
                          [?e :employee/salary ?salary]])"#,
    );
    assert!(cmd.is_ok(), "parse failed");
    if let Ok(DatalogCommand::Query(q)) = cmd {
        assert_eq!(q.find.len(), 2);
        assert!(matches!(&q.find[1], FindSpec::Window(ws) if
            ws.func == WindowFunc::Sum &&
            ws.var == Some("?salary".into()) &&
            ws.partition_by.is_none() &&
            ws.order_by == "?salary" &&
            ws.order == Order::Asc
        ));
    } else {
        panic!("expected Query command");
    }
}

#[test]
fn parse_window_rank_desc() {
    let cmd = parse_datalog_command(
        r#"(query [:find (rank :over (:order-by ?score :desc))
                   :where [?e :item/score ?score]])"#,
    );
    assert!(cmd.is_ok(), "parse failed");
    if let Ok(DatalogCommand::Query(q)) = cmd {
        assert!(matches!(&q.find[0], FindSpec::Window(ws) if
            ws.func == WindowFunc::Rank &&
            ws.var.is_none() &&
            ws.order == Order::Desc
        ));
    } else {
        panic!("expected Query");
    }
}

#[test]
fn parse_window_with_partition_by() {
    let cmd = parse_datalog_command(
        r#"(query [:find ?dept (sum ?salary :over (:partition-by ?dept :order-by ?salary))
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
    );
    assert!(cmd.is_ok(), "parse failed");
    if let Ok(DatalogCommand::Query(q)) = cmd {
        assert!(matches!(&q.find[1], FindSpec::Window(ws) if
            ws.partition_by == Some("?dept".into())
        ));
    } else {
        panic!("expected Query");
    }
}

#[test]
fn parse_lag_rejected() {
    let result = parse_datalog_command(
        r#"(query [:find (lag ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not supported"));
}

#[test]
fn parse_unknown_window_function_rejected() {
    let result = parse_datalog_command(
        r#"(query [:find (bogus ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
    );
    assert!(result.is_err());
}

#[test]
fn parse_order_by_missing_rejected() {
    let result = parse_datalog_command(
        r#"(query [:find (sum ?v :over (:partition-by ?p)) :where [?e :x ?v] [?e :y ?p]])"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("order-by"));
}

#[test]
fn parse_non_window_compatible_in_over_rejected() {
    let result = parse_datalog_command(
        r#"(query [:find (count-distinct ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("window"));
}

#[test]
fn parse_existing_aggregate_still_works() {
    let cmd = parse_datalog_command(
        r#"(query [:find (count ?e) :where [?e :person/name _]])"#,
    );
    assert!(cmd.is_ok(), "parse failed");
    if let Ok(DatalogCommand::Query(q)) = cmd {
        assert!(matches!(&q.find[0], FindSpec::Aggregate { func, .. } if func == "count"));
    } else {
        panic!("expected Query");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test "parse_window" 2>&1 | tail -20
```

Expected: tests fail — `parse_window_expr` does not exist yet.

- [ ] **Step 3: Update `parse_aggregate` to use string func names**

Find the `parse_aggregate` function (around line 420). Replace the body:

```rust
fn parse_aggregate(elems: &[EdnValue]) -> Result<FindSpec, String> {
    // Detect window function: contains ":over" keyword anywhere in elems
    let has_over = elems
        .iter()
        .any(|e| matches!(e, EdnValue::Keyword(k) if k == ":over"));
    if has_over {
        return parse_window_expr(elems);
    }

    if elems.len() != 2 {
        return Err(format!(
            "Aggregate expression must have exactly 2 elements (func ?var), got {}",
            elems.len()
        ));
    }

    let func_name = match &elems[0] {
        EdnValue::Symbol(s) => s.clone(),
        other => {
            return Err(format!(
                "Aggregate function name must be a symbol, got {:?}",
                other
            ));
        }
    };

    const KNOWN_AGGREGATES: &[&str] =
        &["count", "count-distinct", "sum", "sum-distinct", "min", "max"];
    const WINDOW_ONLY: &[&str] = &["avg", "rank", "row-number"];
    const UNSUPPORTED: &[&str] = &["lag", "lead"];

    if UNSUPPORTED.contains(&func_name.as_str()) {
        return Err(format!(
            "'{}' is not supported in this version; lag/lead are planned for a future release",
            func_name
        ));
    }
    if WINDOW_ONLY.contains(&func_name.as_str()) {
        return Err(format!(
            "'{}' is a window function and requires an ':over (...)' clause",
            func_name
        ));
    }
    if !KNOWN_AGGREGATES.contains(&func_name.as_str()) {
        return Err(format!("Unknown aggregate function: '{}'", func_name));
    }

    let var = match &elems[1] {
        EdnValue::Symbol(s) if s.starts_with('?') => s.clone(),
        _ => return Err("Aggregate argument must be a variable (starting with ?)".to_string()),
    };

    Ok(FindSpec::Aggregate { func: func_name, var })
}
```

- [ ] **Step 4: Add `parse_window_expr` function**

Add directly below `parse_aggregate`:

```rust
/// Parse a window function expression: `(func ?var :over (:partition-by ?p :order-by ?o :desc))`
/// or for rank/row-number: `(rank :over (:order-by ?o))`.
fn parse_window_expr(elems: &[EdnValue]) -> Result<FindSpec, String> {
    use super::types::{Order, WindowFunc, WindowSpec};

    let func_name = match &elems[0] {
        EdnValue::Symbol(s) => s.as_str(),
        _ => return Err("window function name must be a symbol".into()),
    };

    if matches!(func_name, "lag" | "lead") {
        return Err(format!(
            "'{}' is not supported in this version; lag/lead are planned for a future release",
            func_name
        ));
    }

    let func = match func_name {
        "sum" => WindowFunc::Sum,
        "count" => WindowFunc::Count,
        "min" => WindowFunc::Min,
        "max" => WindowFunc::Max,
        "avg" => WindowFunc::Avg,
        "rank" => WindowFunc::Rank,
        "row-number" => WindowFunc::RowNumber,
        // count-distinct and sum-distinct are not window-compatible
        "count-distinct" | "sum-distinct" => {
            return Err(format!(
                "'{}' is not window-compatible and cannot be used with ':over'",
                func_name
            ));
        }
        other => return Err(format!("unknown window function: '{}'", other)),
    };

    // rank and row-number take no ?var argument; others require one.
    let (var, over_keyword_idx) = match func {
        WindowFunc::Rank | WindowFunc::RowNumber => {
            // elems: [Symbol("rank"), Keyword(":over"), List(...)]
            if !matches!(elems.get(1), Some(EdnValue::Keyword(k)) if k == ":over") {
                return Err(format!(
                    "'{}' requires ':over' immediately after the function name (no variable argument)",
                    func_name
                ));
            }
            (None, 1usize)
        }
        _ => {
            // elems: [Symbol("sum"), Symbol("?salary"), Keyword(":over"), List(...)]
            let var = match elems.get(1) {
                Some(EdnValue::Symbol(s)) if s.starts_with('?') => s.clone(),
                _ => {
                    return Err(format!(
                        "'{}' requires a variable argument (starting with ?) before ':over'",
                        func_name
                    ));
                }
            };
            if !matches!(elems.get(2), Some(EdnValue::Keyword(k)) if k == ":over") {
                return Err(format!(
                    "'{}' requires ':over' after the variable argument",
                    func_name
                ));
            }
            (Some(var), 2usize)
        }
    };

    // Parse the :over clause list
    let over_list = match elems.get(over_keyword_idx + 1) {
        Some(EdnValue::List(l)) => l.as_slice(),
        _ => {
            return Err(
                "':over' must be followed by a list, e.g., (:order-by ?var)".to_string()
            );
        }
    };

    let mut partition_by: Option<String> = None;
    let mut order_by: Option<String> = None;
    let mut order = Order::Asc;

    let mut j = 0;
    while j < over_list.len() {
        match &over_list[j] {
            EdnValue::Keyword(k) => match k.as_str() {
                ":partition-by" => {
                    j += 1;
                    partition_by = match over_list.get(j) {
                        Some(EdnValue::Symbol(s)) if s.starts_with('?') => Some(s.clone()),
                        _ => {
                            return Err(
                                "':partition-by' requires a variable (starting with ?)".into()
                            );
                        }
                    };
                }
                ":order-by" => {
                    j += 1;
                    order_by = match over_list.get(j) {
                        Some(EdnValue::Symbol(s)) if s.starts_with('?') => Some(s.clone()),
                        _ => {
                            return Err(
                                "':order-by' requires a variable (starting with ?)".into()
                            );
                        }
                    };
                }
                ":desc" => {
                    order = Order::Desc;
                }
                ":asc" => {
                    order = Order::Asc;
                }
                other => {
                    return Err(format!("unknown option in ':over' clause: '{}'", other));
                }
            },
            other => {
                return Err(format!(
                    "unexpected element in ':over' clause: {:?}",
                    other
                ));
            }
        }
        j += 1;
    }

    let order_by = order_by
        .ok_or_else(|| "':order-by' is required in the ':over' clause".to_string())?;

    Ok(FindSpec::Window(WindowSpec {
        func,
        var,
        partition_by,
        order_by,
        order,
    }))
}
```

- [ ] **Step 5: Add the necessary `use` imports to `parser.rs`**

At the top of `parser.rs` where `FindSpec` is imported, add `Order`, `WindowFunc`, `WindowSpec` to the `use super::types::{...}` line. Also add `FindSpec` if not already present.

- [ ] **Step 6: Run parse tests**

```bash
cargo test "parse_window\|parse_lag\|parse_order_by\|parse_non_window\|parse_existing_aggregate" 2>&1 | tail -30
```

Expected: all new tests pass.

- [ ] **Step 7: Run full test suite to confirm no regressions**

```bash
cargo test 2>&1 | tail -10
```

Expected: all 647 previously passing tests still pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat: parse window function expressions in :find clause"
```

---

## Task 5: Wire `FunctionRegistry` through `db.rs` and `DatalogExecutor`

**Files:**
- Modify: `src/db.rs`
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Add `functions` field to `DatalogExecutor`**

In `executor.rs`, update the imports at the top to add the functions module:

```rust
use super::functions::FunctionRegistry;
```

Update the `DatalogExecutor` struct:

```rust
pub struct DatalogExecutor {
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
}
```

Update `DatalogExecutor::new`:

```rust
pub fn new(storage: FactStorage) -> Self {
    DatalogExecutor {
        storage,
        rules: Arc::new(RwLock::new(RuleRegistry::new())),
        functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
    }
}
```

Update `DatalogExecutor::new_with_rules` — rename to `new_with_rules_and_functions` and add the parameter:

```rust
pub fn new_with_rules_and_functions(
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
) -> Self {
    DatalogExecutor { storage, rules, functions }
}
```

Keep `new_with_rules` as a convenience wrapper:

```rust
pub fn new_with_rules(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>) -> Self {
    Self::new_with_rules_and_functions(
        storage,
        rules,
        Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
    )
}
```

- [ ] **Step 2: Add `functions` field to `Inner` in `db.rs`**

In `db.rs`, add the import:

```rust
use crate::query::datalog::functions::FunctionRegistry;
```

Update the `Inner` struct:

```rust
struct Inner {
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
    write_lock: Mutex<WriteContext>,
    options: OpenOptions,
}
```

Update `open_with_options` — in the `Ok(Minigraf { inner: Arc::new(Inner { ... }) })` block, add:

```rust
functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
```

Update `in_memory()` similarly.

- [ ] **Step 3: Pass registry to `DatalogExecutor` in `db.rs::execute`**

In `db.rs::execute`, find the read-only path (around line 449):

```rust
let executor = DatalogExecutor::new_with_rules(
    self.inner.fact_storage.clone(),
    self.inner.rules.clone(),
);
```

Replace with:

```rust
let executor = DatalogExecutor::new_with_rules_and_functions(
    self.inner.fact_storage.clone(),
    self.inner.rules.clone(),
    self.inner.functions.clone(),
);
```

Do the same for any other places in `db.rs` where `DatalogExecutor` is constructed (search for `DatalogExecutor::new`).

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/executor.rs src/db.rs
git commit -m "feat: wire FunctionRegistry through DatalogExecutor and Minigraf::Inner"
```

---

## Task 6: Write Failing Integration Tests

**Files:**
- Create: `tests/window_functions_test.rs`

Write the tests **before** implementing `apply_post_processing`, so they drive the implementation.

- [ ] **Step 1: Create `tests/window_functions_test.rs`**

```rust
use minigraf::db::Minigraf;
use minigraf::query::datalog::executor::QueryResult;
use minigraf::graph::types::Value;

fn setup_employees() -> Minigraf {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(concat!(
        r#"(transact ["#,
        r#"  [:e1 :employee/name "Alice"]"#,
        r#"  [:e1 :employee/dept "Engineering"]"#,
        r#"  [:e1 :employee/salary 90000]"#,
        r#"  [:e2 :employee/name "Bob"]"#,
        r#"  [:e2 :employee/dept "Engineering"]"#,
        r#"  [:e2 :employee/salary 110000]"#,
        r#"  [:e3 :employee/name "Carol"]"#,
        r#"  [:e3 :employee/dept "Product"]"#,
        r#"  [:e3 :employee/salary 95000]"#,
        r#"  [:e4 :employee/name "Dave"]"#,
        r#"  [:e4 :employee/dept "Product"]"#,
        r#"  [:e4 :employee/salary 85000]"#,
        r#"])"#,
    )).expect("transact employees");
    db
}

fn get_results(r: QueryResult) -> Vec<Vec<Value>> {
    if let QueryResult::QueryResults { results, .. } = r {
        results
    } else {
        panic!("expected QueryResults");
    }
}

// ── row-number ──────────────────────────────────────────────────────────────

#[test]
fn row_number_assigns_sequential_positions() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (row-number :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4, "expected 4 rows");
    // Each salary should appear exactly once
    // The row-number for the smallest salary (85000) should be 1
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000));
    assert!(row_85k.is_some(), "salary 85000 not found");
    assert_eq!(row_85k.unwrap()[1], Value::Integer(1), "85000 should be row 1");
    // The row-number for 110000 should be 4
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000));
    assert_eq!(row_110k.unwrap()[1], Value::Integer(4), "110000 should be row 4");
}

// ── rank ────────────────────────────────────────────────────────────────────

#[test]
fn rank_assigns_same_rank_to_ties() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(concat!(
        r#"(transact ["#,
        r#"  [:a :item/score 10]"#,
        r#"  [:b :item/score 10]"#,
        r#"  [:c :item/score 20]"#,
        r#"])"#,
    )).expect("transact");
    let result = db.execute(
        r#"(query [:find ?score (rank :over (:order-by ?score))
                   :where [?e :item/score ?score]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 3);
    // Both score=10 rows get rank 1; score=20 gets rank 3
    let tied: Vec<_> = rows.iter().filter(|r| r[0] == Value::Integer(10)).collect();
    assert_eq!(tied.len(), 2);
    for r in &tied {
        assert_eq!(r[1], Value::Integer(1), "tied scores should both be rank 1");
    }
    let top = rows.iter().find(|r| r[0] == Value::Integer(20)).unwrap();
    assert_eq!(top[1], Value::Integer(3), "score 20 should be rank 3 (gap after tie)");
}

// ── cumulative sum ──────────────────────────────────────────────────────────

#[test]
fn cumulative_sum_over_whole_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (sum ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // sorted asc: 85000, 90000, 95000, 110000
    // cumulative:  85000, 175000, 270000, 380000
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(85000));
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(380000));
}

// ── partition-by ────────────────────────────────────────────────────────────

#[test]
fn sum_resets_per_partition() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?dept ?salary (sum ?salary :over (:partition-by ?dept :order-by ?salary))
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);

    // Engineering: 90000, 110000 → cumulative 90000, 200000
    let eng_90k = rows.iter().find(|r| {
        r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(90000)
    }).unwrap();
    assert_eq!(eng_90k[2], Value::Integer(90000));

    let eng_110k = rows.iter().find(|r| {
        r[0] == Value::String("Engineering".into()) && r[1] == Value::Integer(110000)
    }).unwrap();
    assert_eq!(eng_110k[2], Value::Integer(200000));

    // Product: 85000, 95000 → cumulative 85000, 180000
    let prod_85k = rows.iter().find(|r| {
        r[0] == Value::String("Product".into()) && r[1] == Value::Integer(85000)
    }).unwrap();
    assert_eq!(prod_85k[2], Value::Integer(85000));

    let prod_95k = rows.iter().find(|r| {
        r[0] == Value::String("Product".into()) && r[1] == Value::Integer(95000)
    }).unwrap();
    assert_eq!(prod_95k[2], Value::Integer(180000));
}

// ── running count ───────────────────────────────────────────────────────────

#[test]
fn running_count_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (count ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(4), "last row should have count 4");
}

// ── running min/max ─────────────────────────────────────────────────────────

#[test]
fn running_min_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (min ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    // Running min: first row min = 85000, subsequent rows also min = 85000
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(85000));
}

// ── running avg ─────────────────────────────────────────────────────────────

#[test]
fn running_avg_over_ordered_result() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (avg ?salary :over (:order-by ?salary))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 4);
    // After all 4: avg(85000, 90000, 95000, 110000) = 380000/4 = 95000.0
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Float(95000.0));
}

// ── desc ordering ───────────────────────────────────────────────────────────

#[test]
fn row_number_desc_ordering() {
    let db = setup_employees();
    let result = db.execute(
        r#"(query [:find ?salary (row-number :over (:order-by ?salary :desc))
                   :where [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    // desc: 110000 is row 1, 85000 is row 4
    let row_110k = rows.iter().find(|r| r[0] == Value::Integer(110000)).unwrap();
    assert_eq!(row_110k[1], Value::Integer(1));
    let row_85k = rows.iter().find(|r| r[0] == Value::Integer(85000)).unwrap();
    assert_eq!(row_85k[1], Value::Integer(4));
}

// ── mixed: regular aggregate + window ──────────────────────────────────────

#[test]
fn mixed_aggregate_and_window_in_same_find() {
    let db = setup_employees();
    // count(e) collapses by dept, then sum runs over collapsed rows
    let result = db.execute(
        r#"(query [:find ?dept (count ?e) (sum ?salary :over (:order-by ?salary))
                   :with ?e ?salary
                   :where [?e :employee/dept ?dept]
                          [?e :employee/salary ?salary]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 2, "expected one row per dept");
    // Each dept row has [dept, count, cumulative-sum]
    let eng = rows.iter().find(|r| r[0] == Value::String("Engineering".into())).unwrap();
    assert_eq!(eng[1], Value::Integer(2), "Engineering has 2 employees");
}

// ── single-row partition edge case ─────────────────────────────────────────

#[test]
fn single_row_result_window_equals_row_value() {
    let db = Minigraf::in_memory().expect("in-memory db");
    db.execute(r#"(transact [[:x :score 42]])"#).expect("transact");
    let result = db.execute(
        r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::Integer(42));
}

// ── empty result ────────────────────────────────────────────────────────────

#[test]
fn empty_result_no_panic() {
    let db = Minigraf::in_memory().expect("in-memory db");
    let result = db.execute(
        r#"(query [:find ?v (sum ?v :over (:order-by ?v))
                   :where [?e :score ?v]])"#,
    ).expect("query");
    let rows = get_results(result);
    assert_eq!(rows.len(), 0);
}

// ── parse-time error for lag/lead ──────────────────────────────────────────

#[test]
fn lag_rejected_at_parse_time() {
    let db = Minigraf::in_memory().expect("in-memory db");
    let result = db.execute(
        r#"(query [:find (lag ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not supported"));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test --test window_functions_test 2>&1 | tail -20
```

Expected: tests fail — `apply_post_processing` not yet implemented (queries with `FindSpec::Window` hit the unimplemented path).

---

## Task 7: Implement `apply_post_processing` in `executor.rs`

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Add `use` imports for window types and functions module**

At the top of `executor.rs`, update the imports:

```rust
use super::functions::{FunctionRegistry, apply_builtin_aggregate, value_lt};
use super::types::{
    AsOf, AttributeSpec, BinOp, DatalogCommand, DatalogQuery, EdnValue, Expr, FindSpec,
    Order, Pattern, Rule, Transaction, UnaryOp, ValidAt, WhereClause, WindowFunc,
};
```

(Remove `AggFunc` which was already removed in Task 3.)

- [ ] **Step 2: Remove `apply_agg_func` and `value_type_name`**

Delete the `apply_agg_func` function (lines ~796–902) and the `value_type_name` function if it exists in `executor.rs`. These are now in `functions.rs`. Search for any remaining call sites and replace with `apply_builtin_aggregate` from `functions.rs`.

- [ ] **Step 3: Replace `apply_aggregation` with the new helper suite**

Delete the existing `apply_aggregation` function (lines ~689–793) and replace with these four functions:

```rust
type Binding = std::collections::HashMap<String, Value>;

/// Unified post-processing: handles plain-variable extraction, aggregation,
/// window functions, and mixed (aggregate + window) queries.
///
/// - Plain variables only → `extract_variables` (no change from current path).
/// - Aggregates only → group-by collapse, then project.
/// - Windows only → partition/sort/accumulate per spec, then project.
/// - Mixed → aggregate collapses first, window runs over collapsed rows.
fn apply_post_processing(
    bindings: Vec<Binding>,
    find_specs: &[FindSpec],
    with_vars: &[String],
    registry: &FunctionRegistry,
) -> Result<Vec<Vec<Value>>> {
    let has_aggregates = find_specs.iter().any(|s| matches!(s, FindSpec::Aggregate { .. }));
    let has_windows = find_specs.iter().any(|s| matches!(s, FindSpec::Window(_)));

    if !has_aggregates && !has_windows {
        return Ok(extract_variables(bindings, find_specs));
    }

    // Step 1: Aggregate (collapses rows, produces binding maps).
    let mut working: Vec<Binding> = if has_aggregates {
        compute_aggregation(bindings, find_specs, with_vars, registry)?
    } else {
        bindings
    };

    // Step 2: Window functions (annotate each row, no collapse).
    if has_windows {
        apply_window_functions(&mut working, find_specs, registry)?;
    }

    // Step 3: Project to output rows in find-spec order.
    Ok(project_find_specs(&working, find_specs))
}

/// Group bindings by non-aggregate find vars + with_vars, apply aggregate functions,
/// return one binding map per group. Aggregate results stored under `"__agg_{i}"`.
fn compute_aggregation(
    bindings: Vec<Binding>,
    find_specs: &[FindSpec],
    with_vars: &[String],
    registry: &FunctionRegistry,
) -> Result<Vec<Binding>> {
    let has_grouping_vars = find_specs.iter().any(|s| matches!(s, FindSpec::Variable(_)));

    // Special case: zero bindings + all-count specs → one zero row.
    if bindings.is_empty() {
        let all_count = !has_grouping_vars
            && find_specs.iter().all(|s| {
                matches!(s, FindSpec::Aggregate { func, .. }
                    if func == "count" || func == "count-distinct")
            });
        if all_count {
            let mut b = Binding::new();
            for (i, _) in find_specs.iter().enumerate() {
                b.insert(format!("__agg_{}", i), Value::Integer(0));
            }
            return Ok(vec![b]);
        }
        return Ok(vec![]);
    }

    // Grouping key = Variable find specs (in find order) + with_vars.
    let group_var_names: Vec<&str> = find_specs
        .iter()
        .filter_map(|s| match s {
            FindSpec::Variable(v) => Some(v.as_str()),
            _ => None,
        })
        .chain(with_vars.iter().map(|s| s.as_str()))
        .collect();

    // Group using Vec + PartialEq scan (Value::Float doesn't implement Hash).
    let mut groups: Vec<(Vec<Value>, Vec<Binding>)> = Vec::new();
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

    // Map of Variable spec name → its index in the group key Vec.
    let mut group_key_idx: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    {
        let mut var_pos = 0usize;
        for spec in find_specs {
            if let FindSpec::Variable(v) = spec {
                group_key_idx.insert(v.as_str(), var_pos);
                var_pos += 1;
            }
        }
    }

    let mut results: Vec<Binding> = Vec::new();
    for (key, group_bindings) in &groups {
        let mut binding = Binding::new();
        let mut skip = false;

        // Plain variable values from group key.
        for (v, &idx) in &group_key_idx {
            binding.insert((*v).to_string(), key[idx].clone());
        }

        // Aggregate values stored under "__agg_{i}".
        for (i, spec) in find_specs.iter().enumerate() {
            if let FindSpec::Aggregate { func, var } = spec {
                let non_null: Vec<&Value> = group_bindings
                    .iter()
                    .filter_map(|b| b.get(var.as_str()))
                    .filter(|v| !matches!(v, Value::Null))
                    .collect();
                match apply_builtin_aggregate(func, &non_null) {
                    Ok(v) => {
                        binding.insert(format!("__agg_{}", i), v);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("no non-null values in group") {
                            skip = true;
                            break;
                        }
                        return Err(e);
                    }
                }
            }
        }

        if !skip {
            results.push(binding);
        }
    }

    Ok(results)
}

/// Compute window function values for each row and store under `"__win_{i}"`.
/// Modifies `bindings` in place.
fn apply_window_functions(
    bindings: &mut Vec<Binding>,
    find_specs: &[FindSpec],
    registry: &FunctionRegistry,
) -> Result<()> {
    for (i, spec) in find_specs.iter().enumerate() {
        let FindSpec::Window(ws) = spec else {
            continue;
        };
        let key = format!("__win_{}", i);

        // Build partitions: (partition_key, sorted row indices).
        let mut partitions: Vec<(Option<Value>, Vec<usize>)> = Vec::new();
        for (row_idx, binding) in bindings.iter().enumerate() {
            let part_key = ws
                .partition_by
                .as_ref()
                .and_then(|pv| binding.get(pv))
                .cloned();
            if let Some(pos) = partitions.iter().position(|(k, _)| k == &part_key) {
                partitions[pos].1.push(row_idx);
            } else {
                partitions.push((part_key, vec![row_idx]));
            }
        }

        // For each partition: sort, compute window values, write back.
        for (_, row_indices) in &mut partitions {
            // Sort by order_by key.
            row_indices.sort_by(|&a, &b| {
                let va = bindings[a].get(&ws.order_by).unwrap_or(&Value::Null);
                let vb = bindings[b].get(&ws.order_by).unwrap_or(&Value::Null);
                let lt = value_lt(va, vb);
                let eq = va == vb;
                let cmp = if eq {
                    std::cmp::Ordering::Equal
                } else if lt {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
                match ws.order {
                    Order::Asc => cmp,
                    Order::Desc => cmp.reverse(),
                }
            });

            // Compute one window value per row in partition order.
            let window_values: Vec<Value> = match ws.func {
                WindowFunc::RowNumber => row_indices
                    .iter()
                    .enumerate()
                    .map(|(pos, _)| Value::Integer(pos as i64 + 1))
                    .collect(),

                WindowFunc::Rank => {
                    let mut values = Vec::with_capacity(row_indices.len());
                    let mut rank = 1i64;
                    let mut prev_order_val: Option<Value> = None;
                    let mut row_num = 1i64;
                    for &row_idx in row_indices.iter() {
                        let cur_val = bindings[row_idx].get(&ws.order_by).cloned();
                        if prev_order_val.as_ref() != cur_val.as_ref() {
                            rank = row_num;
                            prev_order_val = cur_val;
                        }
                        values.push(Value::Integer(rank));
                        row_num += 1;
                    }
                    values
                }

                _ => {
                    // Accumulator-based: sum, count, min, max, avg.
                    let func_name = ws.func_name();
                    let desc = registry.get(func_name).ok_or_else(|| {
                        anyhow::anyhow!("no descriptor for window function '{}'", func_name)
                    })?;
                    let ops = desc.window_ops.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("function '{}' is not window-compatible", func_name)
                    })?;

                    let mut acc = (ops.init)();
                    let mut values = Vec::with_capacity(row_indices.len());
                    for &row_idx in row_indices.iter() {
                        let val = ws
                            .var
                            .as_ref()
                            .and_then(|v| bindings[row_idx].get(v))
                            .unwrap_or(&Value::Null);
                        (ops.step)(&mut acc, val);
                        values.push((ops.finalise)(&acc));
                    }
                    values
                }
            };

            // Write window values back to rows.
            for (&row_idx, window_val) in row_indices.iter().zip(window_values.into_iter()) {
                bindings[row_idx].insert(key.clone(), window_val);
            }
        }
    }
    Ok(())
}

/// Project binding maps to output rows in find-spec order.
fn project_find_specs(bindings: &[Binding], find_specs: &[FindSpec]) -> Vec<Vec<Value>> {
    let mut results = Vec::new();
    for binding in bindings {
        let mut row = Vec::new();
        let mut complete = true;
        for (i, spec) in find_specs.iter().enumerate() {
            let val = match spec {
                FindSpec::Variable(v) => binding.get(v).cloned(),
                FindSpec::Aggregate { .. } => {
                    binding.get(&format!("__agg_{}", i)).cloned()
                }
                FindSpec::Window(_) => {
                    binding.get(&format!("__win_{}", i)).cloned()
                }
            };
            match val {
                Some(v) => row.push(v),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            results.push(row);
        }
    }
    results
}
```

- [ ] **Step 4: Update `execute_query` to call `apply_post_processing`**

Find the aggregate dispatch block in `execute_query` (around line 312–320):

```rust
let has_aggregates = query
    .find
    .iter()
    .any(|s| matches!(s, FindSpec::Aggregate { .. }));
let results = if has_aggregates {
    apply_aggregation(filtered_bindings, &query.find, &query.with_vars)?
} else {
    extract_variables(filtered_bindings, &query.find)
};
```

Replace with:

```rust
let registry = self.functions.read().unwrap();
let results = apply_post_processing(
    filtered_bindings,
    &query.find,
    &query.with_vars,
    &registry,
)?;
```

- [ ] **Step 5: Update `execute_query_with_rules` identically**

Find the same block in `execute_query_with_rules` (around line 561–569) and apply the same replacement.

- [ ] **Step 6: Run the integration tests**

```bash
cargo test --test window_functions_test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 7: Run the full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all 647 previously passing tests still pass (plus the new window function tests).

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/executor.rs tests/window_functions_test.rs
git commit -m "feat: implement window functions via apply_post_processing"
```

---

## Task 8: Final Checks and Version Bump

**Files:**
- Modify: `Cargo.toml`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Bump version in `Cargo.toml`**

Change the version line:

```toml
version = "0.16.0"
```

- [ ] **Step 2: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -30
```

Fix any warnings before continuing.

- [ ] **Step 3: Run the full test suite one final time**

```bash
cargo test 2>&1 | tail -5
```

Confirm the total test count reflects all new tests passing.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: bump version to 0.16.0 for Phase 7.7a window functions"
```

---

## Self-Review Checklist

- [x] **Spec coverage**: `FunctionRegistry` (Task 2) ✓; `WindowSpec`/`WindowFunc`/`Order` types (Task 3) ✓; parser (Task 4) ✓; registry wired (Task 5) ✓; `apply_post_processing` + window computation (Task 7) ✓; all core window functions tested (Task 6) ✓; `lag`/`lead` rejected at parse time (Tasks 4+6) ✓; mixed aggregate+window (Task 6) ✓; empty result (Task 6) ✓; partition-by optional (Task 7 impl) ✓; `order-by` required, error if missing (Task 4) ✓; `:desc` ordering (Task 6) ✓; rank with ties (Task 6) ✓; `FunctionRegistry` unit tests (Task 2) ✓.
- [x] **Placeholders**: none. All function bodies are shown in full.
- [x] **Type consistency**: `WindowSpec::func_name()` defined in Task 3 and called in Task 7 `apply_window_functions`. `FindSpec::Window(WindowSpec)` defined in Task 3, parsed in Task 4, dispatched in Task 7. `"__agg_{i}"` and `"__win_{i}"` synthetic keys used consistently in `compute_aggregation`, `apply_window_functions`, and `project_find_specs`.
- [x] **No Placeholders**: no TBD, no "similar to above", no missing code blocks.
