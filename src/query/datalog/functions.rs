use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Why a user-defined function failed. Its message is in the query's
/// `QRY-014` error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfError {
    message: String,
}

impl UdfError {
    /// A failure with this message.
    pub fn new(message: impl Into<String>) -> Self {
        UdfError {
            message: message.into(),
        }
    }

    /// The message the function gave.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for UdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UdfError {}

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

/// Type alias for the type-erased UDF accumulator init closure.
pub type UdfInitFn = Arc<dyn Fn() -> Result<Box<dyn Any + Send>, UdfError> + Send + Sync>;

/// Type alias for the type-erased UDF accumulator step closure.
pub type UdfStepFn =
    Arc<dyn Fn(&mut Box<dyn Any + Send>, &Value) -> Result<(), UdfError> + Send + Sync>;

/// Type alias for the type-erased UDF accumulator finalise closure.
pub type UdfFinaliseFn =
    Arc<dyn Fn(&Box<dyn Any + Send>, usize) -> Result<Value, UdfError> + Send + Sync>;

/// Type alias for a predicate closure.
pub type PredicateFn = Arc<dyn Fn(&Value) -> Result<bool, UdfError> + Send + Sync>;

/// Closure-based aggregate ops for UDFs.
/// The accumulator is type-erased as `Box<dyn Any + Send>`. Every closure
/// may fail; a failure fails the query with `QRY-014` (#180).
#[derive(Clone)]
pub struct UdfOps {
    pub init: UdfInitFn,
    pub step: UdfStepFn,
    pub finalise: UdfFinaliseFn,
}

/// Implementation discriminator for aggregate functions.
#[derive(Clone)]
pub enum AggImpl {
    /// Built-in: computed by `aggregate::Agg` and `aggregate::Running`.
    Builtin(Builtin),
    /// User-defined: uses `Arc<dyn Fn>` closures and `Box<dyn Any + Send>`.
    Udf(UdfOps),
}

/// Descriptor for a registered predicate function.
#[derive(Clone)]
pub struct PredicateDesc {
    pub f: PredicateFn,
    pub is_builtin: bool,
}

/// Descriptor for one registered aggregate function.
#[derive(Clone)]
pub struct AggregateDesc {
    pub impl_: AggImpl,
}

/// Registry of aggregate function descriptors, keyed by hyphenated name.
///
/// In 7.7a this holds only built-ins. Phase 7.7b adds
/// `register_aggregate_desc` / `register_predicate_desc` public methods.
/// A clone copies the maps; the closures are shared (#180).
#[derive(Clone)]
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
        ] {
            reg.predicates.insert(
                name.to_string(),
                PredicateDesc {
                    f: Arc::new(|_| Ok(false)), // sentinel; never called via registry
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

/// The registry a database shares between its handles (#180, spec §5). A
/// query reads its functions from a [`snapshot`](Functions::snapshot) taken
/// when it opens, so no lock is held while a user function runs, and a
/// function registered later is not visible to it. Registration swaps in a
/// new registry.
pub struct Functions(RwLock<Arc<FunctionRegistry>>);

impl Functions {
    pub(crate) fn with_builtins() -> Self {
        Functions(RwLock::new(Arc::new(FunctionRegistry::with_builtins())))
    }

    /// The registry now; a poisoned lock is `QRY-008`.
    pub(crate) fn snapshot(&self) -> anyhow::Result<Arc<FunctionRegistry>> {
        let reg = self.0.read().map_err(|_| err_coded!(ErrorCode::Qry008))?;
        Ok(Arc::clone(&reg))
    }

    /// Apply `f` to a copy of the registry and swap the copy in if `f`
    /// succeeds; a poisoned lock is `API-008`.
    pub(crate) fn register(
        &self,
        f: impl FnOnce(&mut FunctionRegistry) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut reg = self.0.write().map_err(|_| err_coded!(ErrorCode::Api008))?;
        let mut next = FunctionRegistry::clone(&reg);
        f(&mut next)?;
        *reg = Arc::new(next);
        Ok(())
    }

    /// Poison the lock, for the tests of `QRY-008` and `API-008`.
    #[cfg(test)]
    pub(crate) fn poison(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.0.write();
            panic!("poison the functions lock");
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;

    #[test]
    fn udf_error_displays_its_message() {
        let e = UdfError::new("not a string");
        assert_eq!(e.message(), "not a string");
        assert_eq!(e.to_string(), "not a string");
        assert_eq!(e.clone(), e);
    }

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
                    init: Arc::new(|| Ok(Box::new(0i64) as Box<dyn Any + Send>)),
                    step: Arc::new(|acc, v| {
                        if let (Some(n), Value::Integer(i)) = (acc.downcast_mut::<i64>(), v) {
                            *n += i;
                        }
                        Ok(())
                    }),
                    finalise: Arc::new(|acc, _n| {
                        Ok(acc
                            .downcast_ref::<i64>()
                            .map(|n| Value::Integer(*n))
                            .unwrap_or(Value::Null))
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
                init: Arc::new(|| Ok(Box::new(0i64) as Box<dyn Any + Send>)),
                step: Arc::new(|_acc, _v| Ok(())),
                finalise: Arc::new(|_acc, _n| Ok(Value::Null)),
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
                    init: Arc::new(|| Ok(Box::new(0i64) as Box<dyn Any + Send>)),
                    step: Arc::new(|_acc, _v| Ok(())),
                    finalise: Arc::new(|_acc, _n| Ok(Value::Null)),
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
                f: Arc::new(|v| Ok(matches!(v, Value::String(s) if s.contains('@')))),
                is_builtin: false,
            },
        )
        .expect("first registration ok");
        assert!(reg.get_predicate("email?").is_some());
        let second = reg.register_predicate_desc(
            "email?".to_string(),
            PredicateDesc {
                f: Arc::new(|_v| Ok(false)),
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
                f: Arc::new(|_v| Ok(false)),
                is_builtin: false,
            },
        );
        assert!(result.is_err());
    }
}
