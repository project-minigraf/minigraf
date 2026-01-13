pub mod evaluator;
pub mod executor;
pub mod matcher;
pub mod parser;
pub mod rules;
pub mod types;

pub use evaluator::RecursiveEvaluator;
pub use executor::{DatalogExecutor, QueryResult};
pub use matcher::{edn_to_entity_id, edn_to_value, Bindings, PatternMatcher};
pub use parser::{parse_datalog_command, parse_edn};
pub use rules::RuleRegistry;
pub use types::{
    DatalogCommand, DatalogQuery, EdnValue, Pattern, Rule, Transaction, WhereClause,
};
