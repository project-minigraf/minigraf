use super::types::{EdnValue, Pattern};
use crate::error::{ErrorCode, err_coded};
use anyhow::Result;

/// Default maximum iterations for recursive evaluation (kept for reference).
#[allow(dead_code)]
pub const DEFAULT_MAX_ITERATIONS: usize = 1000;
/// Default maximum facts the rules of one query may derive in total; base
/// facts do not count (#553).
pub const DEFAULT_MAX_DERIVED_FACTS: usize = 1_000_000;

/// `INT-020` for a rule query that derived more than `limit` facts (#553).
pub(crate) fn derived_facts_exceeded(limit: usize) -> anyhow::Error {
    err_coded!(
        ErrorCode::Int020,
        format!(
            "Max derived facts ({limit}) exceeded. The rules derived more facts than the limit; base facts do not count."
        )
    )
}

/// `INT-020` for a stratum still deriving facts after `limit` rounds.
pub(crate) fn iterations_exceeded(limit: usize) -> anyhow::Error {
    err_coded!(
        ErrorCode::Int020,
        format!(
            "Max iterations ({limit}) exceeded. Possible infinite recursion or cycle in rules."
        )
    )
}

/// Convert a predicate name + argument list to a Pattern.
///
/// 1-arg: `(blocked ?x)` → `[?x :blocked ?_rule_value]`
/// 2-arg: `(reachable ?from ?to)` → `[?from :reachable ?to]`
pub(crate) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
    match args.len() {
        1 => {
            // Safety: length checked above
            let arg0 = args
                .first()
                .ok_or_else(|| err_coded!(ErrorCode::Int022, "missing arg 0"))?;
            Ok(Pattern::new(
                arg0.clone(),
                EdnValue::Keyword(format!(":{}", predicate)),
                EdnValue::Symbol("?_rule_value".to_string()),
            ))
        }
        2 => {
            let arg0 = args
                .first()
                .ok_or_else(|| err_coded!(ErrorCode::Int022, "missing arg 0"))?;
            let arg1 = args
                .get(1)
                .ok_or_else(|| err_coded!(ErrorCode::Int022, "missing arg 1"))?;
            Ok(Pattern::new(
                arg0.clone(),
                EdnValue::Keyword(format!(":{}", predicate)),
                arg1.clone(),
            ))
        }
        n => Err(err_coded!(ErrorCode::Int028, predicate, n)),
    }
}
