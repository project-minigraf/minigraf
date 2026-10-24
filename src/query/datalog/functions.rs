use crate::error::{ErrorCode, bail_coded};
use crate::graph::types::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// A built-in aggregate; `aggregate.rs` holds its arithmetic (#547).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Count,
    Sum,
    Min,
    Max,
    Avg,
    CountDistinct,
    SumDistinct,
}

/// Type alias for the type-erased UDF accumulator step closure.
pub type UdfStepFn = Arc<dyn Fn(&mut Box<dyn Any + Send>, &Value) + Send + Sync>;

/// Type alias for the type-erased UDF accumulator finalise closure.
pub type UdfFinaliseFn = Arc<dyn Fn(&Box<dyn Any + Send>, usize) -> Value + Send + Sync>;

/// Closure-based aggregate ops for UDFs.
/// The accumulator is type-erased as `Box<dyn Any + Send>`.
pub struct UdfOps {
    pub init: Arc<dyn Fn() -> Box<dyn Any + Send> + Send + Sync>,
    pub step: UdfStepFn,
    pub finalise: UdfFinaliseFn,
}

/// Implementation discriminator for aggregate functions.
pub enum AggImpl {
    /// Built-in: computed by `aggregate::Agg` and `aggregate::Running`.
    Builtin(Builtin),
    /// User-defined: uses `Arc<dyn Fn>` closures and `Box<dyn Any + Send>`.
    Udf(UdfOps),
}

/// Descriptor for a registered predicate function.
pub struct PredicateDesc {
    pub f: Arc<dyn Fn(&Value) -> bool + Send + Sync>,
    pub is_builtin: bool,
}

/// Descriptor for one registered aggregate function.
pub struct AggregateDesc {
    pub impl_: AggImpl,
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

        for (name, b) in [
            ("count", Builtin::Count),
            ("sum", Builtin::Sum),
            ("min", Builtin::Min),
            ("max", Builtin::Max),
            ("avg", Builtin::Avg),
            ("count-distinct", Builtin::CountDistinct),
            ("sum-distinct", Builtin::SumDistinct),
        ] {
            reg.aggregates.insert(
                name.into(),
                AggregateDesc {
                    impl_: AggImpl::Builtin(b),
                },
            );
        }

        // Built-in predicate name sentinels — block user registration of these names.
        for name in [
            "string?",
            "integer?",
            "float?",
            "boolean?",
            "nil?",
            "starts-with?",
            "ends-with?",
            "contains?",
            "matches?",
            "in?",
            "not-in?",
            "any-in?",
            "all-in?",
            "none-in?",
            "collect-set",
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

    /// Register a UDF aggregate descriptor. Returns Err if the name is already taken.
    pub fn register_aggregate_desc(
        &mut self,
        name: String,
        desc: AggregateDesc,
    ) -> anyhow::Result<()> {
        if self.aggregates.contains_key(&name) {
            bail_coded!(ErrorCode::Int040, "aggregate function", name);
        }
        self.aggregates.insert(name, desc);
        Ok(())
    }

    /// Register a predicate descriptor. Returns Err if the name is already taken.
    pub fn register_predicate_desc(
        &mut self,
        name: String,
        desc: PredicateDesc,
    ) -> anyhow::Result<()> {
        if self.predicates.contains_key(&name) {
            bail_coded!(ErrorCode::Int040, "predicate", name);
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
            assert!(
                reg.get(name).is_some(),
                "expected '{}' to be registered",
                name
            );
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        let reg = FunctionRegistry::with_builtins();
        assert!(reg.get("nonexistent").is_none());
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
            },
        )
        .expect("register should succeed");
        assert!(reg.get("myfn").is_some());
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
        };
        reg.register_aggregate_desc("myfn".to_string(), make_desc())
            .expect("first ok");
        assert!(
            reg.register_aggregate_desc("myfn".to_string(), make_desc())
                .is_err()
        );
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
        )
        .expect("first registration ok");
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
}
