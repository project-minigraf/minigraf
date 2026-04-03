use crate::graph::types::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// Accumulator state for window aggregate functions (incremental computation).
#[derive(Clone, Debug)]
pub enum AggState {
    Sum {
        total: f64,
        is_float: bool,
    },
    Count(i64),
    /// Used by both min and max; semantics determined by the registered step fn.
    MinMax {
        current: Option<Value>,
    },
    Avg {
        sum: f64,
        count: usize,
    },
}

/// Incremental operations for window-compatible aggregate functions.
pub struct WindowOps {
    pub init: fn() -> AggState,
    pub step: fn(&mut AggState, &Value),
    pub finalise: fn(&AggState) -> Value,
}

/// Type alias for the type-erased UDF accumulator step closure.
pub type UdfStepFn = Arc<dyn Fn(&mut Box<dyn Any + Send>, &Value) + Send + Sync>;

/// Type alias for the type-erased UDF accumulator finalise closure.
pub type UdfFinaliseFn = Arc<dyn Fn(&Box<dyn Any + Send>, usize) -> Value + Send + Sync>;

/// Closure-based aggregate ops for UDFs.
/// The accumulator is type-erased as `Box<dyn Any + Send>`.
pub struct UdfOps {
    pub init:     Arc<dyn Fn() -> Box<dyn Any + Send> + Send + Sync>,
    pub step:     UdfStepFn,
    pub finalise: UdfFinaliseFn,
}

/// Implementation discriminator for aggregate functions.
pub enum AggImpl {
    /// Built-in: uses `fn()` function pointers and `AggState`.
    Builtin(WindowOps),
    /// User-defined: uses `Arc<dyn Fn>` closures and `Box<dyn Any + Send>`.
    Udf(UdfOps),
}

/// Descriptor for a registered predicate function.
pub struct PredicateDesc {
    pub f:          Arc<dyn Fn(&Value) -> bool + Send + Sync>,
    pub is_builtin: bool,
}

/// Descriptor for one registered aggregate function.
pub struct AggregateDesc {
    pub impl_:      AggImpl,
    /// True for built-in functions handled by `apply_builtin_aggregate`.
    pub is_builtin: bool,
}

/// Registry of aggregate function descriptors, keyed by hyphenated name.
///
/// In 7.7a this holds only built-ins. Phase 7.7b adds
/// `register_aggregate_desc` / `register_predicate_desc` public methods.
pub struct FunctionRegistry {
    aggregates: HashMap<String, AggregateDesc>,
    /// Registered filter predicates, including built-in name sentinels.
    predicates: HashMap<String, PredicateDesc>,
}

impl FunctionRegistry {
    pub fn with_builtins() -> Self {
        let mut reg = Self {
            aggregates: HashMap::new(),
            predicates: HashMap::new(),
        };

        // count (window-compatible)
        reg.aggregates.insert(
            "count".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
                    init: || AggState::Count(0),
                    step: |state, v| {
                        if !matches!(v, Value::Null) {
                            let AggState::Count(n) = state else { return };
                            *n += 1;
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

        // sum (window-compatible)
        reg.aggregates.insert(
            "sum".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
                    init: || AggState::Sum {
                        total: 0.0,
                        is_float: false,
                    },
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

        // min (window-compatible)
        reg.aggregates.insert(
            "min".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
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

        // max (window-compatible)
        reg.aggregates.insert(
            "max".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
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

        // avg (window-compatible)
        reg.aggregates.insert(
            "avg".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
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

        // count-distinct (NOT window-compatible) — stub WindowOps; never called in window position
        reg.aggregates.insert(
            "count-distinct".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
                    init: || AggState::Count(0),
                    step: |_state, _v| {},
                    finalise: |_state| Value::Null,
                }),
            },
        );

        // sum-distinct (NOT window-compatible) — stub WindowOps; never called in window position
        reg.aggregates.insert(
            "sum-distinct".into(),
            AggregateDesc {
                is_builtin: true,
                impl_: AggImpl::Builtin(WindowOps {
                    init: || AggState::Count(0),
                    step: |_state, _v| {},
                    finalise: |_state| Value::Null,
                }),
            },
        );

        // Built-in predicate name sentinels — block user registration of these names.
        for name in [
            "string?", "integer?", "float?", "boolean?", "nil?",
            "starts-with?", "ends-with?", "contains?", "matches?",
        ] {
            reg.predicates.insert(
                name.to_string(),
                PredicateDesc {
                    f: Arc::new(|_| false), // sentinel; never called via registry
                    is_builtin: true,
                },
            );
        }

        reg
    }

    pub fn get(&self, name: &str) -> Option<&AggregateDesc> {
        self.aggregates.get(name)
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.aggregates.contains_key(name)
    }

    /// count-distinct and sum-distinct are not window-compatible.
    /// All other registered aggregates (built-in or UDF) are window-compatible.
    pub fn is_window_compatible(&self, name: &str) -> bool {
        match name {
            "count-distinct" | "sum-distinct" => false,
            _ => self.aggregates.contains_key(name),
        }
    }

    /// Register a UDF aggregate descriptor. Returns Err if the name is already taken.
    pub fn register_aggregate_desc(&mut self, name: String, desc: AggregateDesc) -> anyhow::Result<()> {
        if self.aggregates.contains_key(&name) {
            anyhow::bail!("aggregate function '{}' is already registered", name);
        }
        self.aggregates.insert(name, desc);
        Ok(())
    }

    /// Register a predicate descriptor. Returns Err if the name is already taken.
    pub fn register_predicate_desc(&mut self, name: String, desc: PredicateDesc) -> anyhow::Result<()> {
        if self.predicates.contains_key(&name) {
            anyhow::bail!("predicate '{}' is already registered", name);
        }
        self.predicates.insert(name, desc);
        Ok(())
    }

    /// Look up a registered predicate by name.
    /// Returns None for built-in sentinels (they are not callable via registry).
    pub fn get_predicate(&self, name: &str) -> Option<&PredicateDesc> {
        self.predicates.get(name).filter(|d| !d.is_builtin)
    }
}

/// Returns true if a < b by value ordering. Mixed Integer/Float promotes to f64.
/// Returns false for incomparable types (no panic).
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

        "avg" => {
            if values.is_empty() {
                return Ok(Value::Null);
            }
            let mut sum = 0.0_f64;
            let mut count = 0usize;
            for v in values {
                match v {
                    Value::Integer(i) => {
                        sum += *i as f64;
                        count += 1;
                    }
                    Value::Float(f) => {
                        sum += f;
                        count += 1;
                    }
                    _ => {}
                }
            }
            if count == 0 {
                Ok(Value::Null)
            } else {
                Ok(Value::Float(sum / count as f64))
            }
        }

        other => Err(anyhow::anyhow!("unknown aggregate function: '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;

    #[test]
    fn registry_knows_all_builtins() {
        let reg = FunctionRegistry::with_builtins();
        for name in [
            "count",
            "count-distinct",
            "sum",
            "sum-distinct",
            "min",
            "max",
            "avg",
        ] {
            assert!(reg.is_known(name), "expected '{}' to be registered", name);
        }
    }

    #[test]
    fn window_compatible_flags() {
        let reg = FunctionRegistry::with_builtins();
        for name in ["count", "sum", "min", "max", "avg"] {
            assert!(
                reg.is_window_compatible(name),
                "'{}' should be window-compatible",
                name
            );
        }
        for name in ["count-distinct", "sum-distinct"] {
            assert!(
                !reg.is_window_compatible(name),
                "'{}' should NOT be window-compatible",
                name
            );
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
    fn apply_builtin_avg() {
        let vals = vec![Value::Integer(10), Value::Integer(20), Value::Integer(30)];
        let refs: Vec<&Value> = vals.iter().collect();
        let result = apply_builtin_aggregate("avg", &refs).unwrap();
        assert_eq!(result, Value::Float(20.0));
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no non-null values")
        );
    }

    #[test]
    fn value_lt_integers() {
        assert!(value_lt(&Value::Integer(1), &Value::Integer(2)));
        assert!(!value_lt(&Value::Integer(2), &Value::Integer(1)));
        assert!(!value_lt(&Value::Integer(1), &Value::Integer(1)));
    }

    #[test]
    fn value_lt_strings() {
        assert!(value_lt(
            &Value::String("a".into()),
            &Value::String("b".into())
        ));
        assert!(!value_lt(
            &Value::String("b".into()),
            &Value::String("a".into())
        ));
    }

    #[test]
    fn window_ops_sum_accumulator() {
        let reg = FunctionRegistry::with_builtins();
        let desc = reg.get("sum").unwrap();
        let AggImpl::Builtin(ops) = &desc.impl_ else { panic!("sum should be Builtin") };
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
        let AggImpl::Builtin(ops) = &desc.impl_ else { panic!("count should be Builtin") };
        let mut state = (ops.init)();
        (ops.step)(&mut state, &Value::Integer(1));
        (ops.step)(&mut state, &Value::Integer(2));
        assert_eq!((ops.finalise)(&state), Value::Integer(2));
    }

    #[test]
    fn window_ops_avg_accumulator() {
        let reg = FunctionRegistry::with_builtins();
        let desc = reg.get("avg").unwrap();
        let AggImpl::Builtin(ops) = &desc.impl_ else { panic!("avg should be Builtin") };
        let mut state = (ops.init)();
        (ops.step)(&mut state, &Value::Integer(10));
        (ops.step)(&mut state, &Value::Integer(20));
        assert_eq!((ops.finalise)(&state), Value::Float(15.0));
    }

    #[test]
    fn register_udf_aggregate_is_known() {
        let mut reg = FunctionRegistry::with_builtins();
        reg.register_aggregate_desc(
            "myfn".to_string(),
            AggregateDesc {
                impl_: AggImpl::Udf(UdfOps {
                    init: Arc::new(|| Box::new(0i64) as Box<dyn Any + Send>),
                    step: Arc::new(|acc, v| {
                        if let (Some(n), Value::Integer(i)) = (acc.downcast_mut::<i64>(), v) {
                            *n += i;
                        }
                    }),
                    finalise: Arc::new(|acc, _n| {
                        acc.downcast_ref::<i64>()
                            .map(|n| Value::Integer(*n))
                            .unwrap_or(Value::Null)
                    }),
                }),
                is_builtin: false,
            },
        ).expect("register should succeed");
        assert!(reg.is_known("myfn"));
        assert!(reg.is_window_compatible("myfn"));
    }

    #[test]
    fn register_udf_duplicate_rejected() {
        let mut reg = FunctionRegistry::with_builtins();
        let make_desc = || AggregateDesc {
            impl_: AggImpl::Udf(UdfOps {
                init: Arc::new(|| Box::new(0i64) as Box<dyn Any + Send>),
                step: Arc::new(|_acc, _v| {}),
                finalise: Arc::new(|_acc, _n| Value::Null),
            }),
            is_builtin: false,
        };
        reg.register_aggregate_desc("myfn".to_string(), make_desc()).expect("first ok");
        assert!(reg.register_aggregate_desc("myfn".to_string(), make_desc()).is_err());
    }

    #[test]
    fn register_builtin_name_rejected() {
        let mut reg = FunctionRegistry::with_builtins();
        let result = reg.register_aggregate_desc(
            "sum".to_string(),
            AggregateDesc {
                impl_: AggImpl::Udf(UdfOps {
                    init: Arc::new(|| Box::new(0i64) as Box<dyn Any + Send>),
                    step: Arc::new(|_acc, _v| {}),
                    finalise: Arc::new(|_acc, _n| Value::Null),
                }),
                is_builtin: false,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_predicate_works_and_rejects_duplicate() {
        let mut reg = FunctionRegistry::with_builtins();
        reg.register_predicate_desc(
            "email?".to_string(),
            PredicateDesc {
                f: Arc::new(|v| matches!(v, Value::String(s) if s.contains('@'))),
                is_builtin: false,
            },
        ).expect("first registration ok");
        assert!(reg.get_predicate("email?").is_some());
        let second = reg.register_predicate_desc(
            "email?".to_string(),
            PredicateDesc {
                f: Arc::new(|_v| false),
                is_builtin: false,
            },
        );
        assert!(second.is_err());
    }

    #[test]
    fn register_builtin_predicate_name_rejected() {
        let mut reg = FunctionRegistry::with_builtins();
        let result = reg.register_predicate_desc(
            "string?".to_string(),
            PredicateDesc {
                f: Arc::new(|_v| false),
                is_builtin: false,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn builtin_sum_accumulator_regression_guard() {
        let reg = FunctionRegistry::with_builtins();
        // Regression guard: existing window_ops path still works.
        let desc = reg.get("sum").unwrap();
        if let AggImpl::Builtin(ops) = &desc.impl_ {
            let mut acc = (ops.init)();
            (ops.step)(&mut acc, &Value::Integer(5));
            assert_eq!((ops.finalise)(&acc), Value::Integer(5));
        } else {
            panic!("sum should be Builtin");
        }
    }
}
