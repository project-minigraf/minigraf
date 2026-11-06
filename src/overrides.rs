//! Which query keywords may override the database's settings (#862, #869).
//!
//! The embedder lists them in [`OpenOptions::allow_overrides`]; a query that
//! uses a keyword the list leaves out is refused with `API-025` before it runs.
//!
//! [`OpenOptions::allow_overrides`]: crate::OpenOptions::allow_overrides

use anyhow::Result;

use crate::error::{ErrorCode, bail_coded};
use crate::query::datalog::types::DatalogQuery;

/// A setting that query text may override (#862).
///
/// `#[non_exhaustive]`: later releases add variants for new query keywords.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overridable {
    /// `:max-derived-facts N`, which overrides
    /// [`OpenOptions::max_derived_facts`](crate::OpenOptions::max_derived_facts).
    MaxDerivedFacts,
    /// `:max-results N`, the deprecated alias of `:max-derived-facts`.
    MaxResults,
}

impl Overridable {
    /// The list a database uses when
    /// [`OpenOptions::allow_overrides`](crate::OpenOptions::allow_overrides)
    /// is not set: every keyword that could override a setting before #862.
    pub const DEFAULTS: &'static [Overridable] =
        &[Overridable::MaxDerivedFacts, Overridable::MaxResults];

    /// The query keyword this variant gates, e.g. `":max-derived-facts"`.
    pub fn keyword(self) -> &'static str {
        match self {
            Overridable::MaxDerivedFacts => ":max-derived-facts",
            Overridable::MaxResults => ":max-results",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Overridable::MaxDerivedFacts => "MaxDerivedFacts",
            Overridable::MaxResults => "MaxResults",
        }
    }
}

/// Refuse a query that uses a keyword `allowed` does not list (`API-025`).
///
/// Checks the keywords in a fixed order and reports the first one refused.
/// Only the top-level query takes keywords; rule bodies take none.
pub(crate) fn check_overrides(query: &DatalogQuery, allowed: &[Overridable]) -> Result<()> {
    let present = [
        (
            Overridable::MaxDerivedFacts,
            query.max_derived_facts.is_some(),
        ),
        (Overridable::MaxResults, query.max_results.is_some()),
    ];
    for (setting, used) in present {
        if used && !allowed.contains(&setting) {
            bail_coded!(ErrorCode::Api025, setting.keyword(), setting.name());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::parser::parse_datalog_command;
    use crate::query::datalog::types::DatalogCommand;

    fn query(text: &str) -> DatalogQuery {
        match parse_datalog_command(text).expect("parse") {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected a query"),
        }
    }

    fn code(r: Result<()>) -> String {
        crate::MinigrafError::from(r.expect_err("refused"))
            .code()
            .to_string()
    }

    #[test]
    fn reports_max_derived_facts_first() {
        let q = query("(query [:find ?e :where [?e :a ?v] :max-derived-facts 5 :max-results 5])");
        let err = crate::MinigrafError::from(check_overrides(&q, &[]).expect_err("refused"));
        assert_eq!(err.code(), "API-025");
        assert!(err.to_string().contains(":max-derived-facts"));
        assert!(err.to_string().contains("Overridable::MaxDerivedFacts"));
        assert!(!err.to_string().contains(":max-results"));
    }

    #[test]
    fn listed_and_absent_keywords_pass() {
        let both =
            query("(query [:find ?e :where [?e :a ?v] :max-derived-facts 5 :max-results 5])");
        check_overrides(&both, Overridable::DEFAULTS).expect("defaults allow both");
        let plain = query("(query [:find ?e :where [?e :a ?v]])");
        check_overrides(&plain, &[]).expect("no keyword");
        let results = query("(query [:find ?e :where [?e :a ?v] :max-results 5])");
        assert_eq!(
            code(check_overrides(&results, &[Overridable::MaxDerivedFacts])),
            "API-025"
        );
    }
}
