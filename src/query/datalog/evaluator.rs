use super::types::{EdnValue, Pattern};
use crate::error::{ErrorCode, err_coded};
use anyhow::Result;

/// Default maximum rounds of one stratum of a rule query (#861): equal to
/// [`DEFAULT_MAX_DERIVED_FACTS`], since a round that continues derived a new
/// fact, so at the defaults the derived-facts limit is reached first.
pub const DEFAULT_MAX_ITERATIONS: usize = 1_000_000;
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
            "Max iterations ({limit}) exceeded. A stratum's rules were still deriving new facts after {limit} rounds; raise the limit with :max-iterations N or OpenOptions::max_iterations."
        )
    )
}

/// Convert a predicate name + argument list to a Pattern.
///
/// 1-arg: `(blocked ?x)` → `[?x :blocked ?_rule_value]`
/// 2-arg: `(reachable ?from ?to)` → `[?from :reachable ?to]`
pub(crate) fn rule_invocation_to_pattern(predicate: &str, args: &[EdnValue]) -> Result<Pattern> {
    let attribute = EdnValue::Keyword(format!(":{}", predicate));
    match args {
        [entity] => Ok(Pattern::new(
            entity.clone(),
            attribute,
            EdnValue::Symbol("?_rule_value".to_string()),
        )),
        [entity, value] => Ok(Pattern::new(entity.clone(), attribute, value.clone())),
        _ => Err(err_coded!(ErrorCode::Int028, predicate, args.len())),
    }
}
