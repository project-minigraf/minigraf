use crate::graph::types::Value;
use std::collections::HashMap;

/// Accumulator state for window aggregate functions (incremental computation).
#[derive(Clone, Debug)]
pub enum AggState {
    Sum { total: f64, is_float: bool },
    Count(i64),
    /// Used by both min and max; semantics determined by the registered step fn.
    MinMax { current: Option<Value> },
    Avg { sum: f64, count: usize },
}

/// Incremental operations for window-compatible aggregate functions.
pub struct WindowOps {
    pub init: fn() -> AggState,
    pub step: fn(&mut AggState, &Value),
    pub finalise: fn(&AggState) -> Value,
}

/// Descriptor for one registered aggregate function.
pub struct AggregateDesc {
    pub window_compatible: bool,
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
    pub fn with_builtins() -> Self {
        let mut reg = Self {
            aggregates: HashMap::new(),
        };

        // count (window-compatible)
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

        // sum (window-compatible)
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

        // min (window-compatible)
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

        // max (window-compatible)
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

        // avg (window-compatible)
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

        // count-distinct (NOT window-compatible)
        reg.aggregates.insert(
            "count-distinct".into(),
            AggregateDesc { window_compatible: false, window_ops: None },
        );

        // sum-distinct (NOT window-compatible)
        reg.aggregates.insert(
            "sum-distinct".into(),
            AggregateDesc { window_compatible: false, window_ops: None },
        );

        reg
    }

    pub fn get(&self, name: &str) -> Option<&AggregateDesc> {
        self.aggregates.get(name)
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.aggregates.contains_key(name)
    }

    pub fn is_window_compatible(&self, name: &str) -> bool {
        self.aggregates
            .get(name)
            .map(|d| d.window_compatible)
            .unwrap_or(false)
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
