#[allow(unused_imports)]
use crate::graph::types::{EntityId, Value};
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::DatalogQuery;
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};

#[allow(dead_code)]
pub(crate) fn rewrite(
    _query: &DatalogQuery,
    _registry: &RuleRegistry,
) -> Option<(RuleRegistry, Vec<(EntityId, String, Value)>)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::FindSpec;

    #[test]
    fn test_rewrite_empty_query_returns_none() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?x".to_string())],
            vec![],
        );
        let registry = RuleRegistry::new();
        assert!(rewrite(&query, &registry).is_none());
    }
}
