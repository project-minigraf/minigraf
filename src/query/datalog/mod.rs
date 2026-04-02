pub mod evaluator;
pub mod executor;
pub mod functions;
pub mod matcher;
pub mod optimizer;
pub mod parser;
pub mod rules;
pub mod stratification;
pub mod types;

pub use evaluator::RecursiveEvaluator;
pub use evaluator::StratifiedEvaluator;
pub use executor::{DatalogExecutor, QueryResult};
pub use matcher::{Bindings, PatternMatcher, edn_to_entity_id, edn_to_value};
pub use parser::{parse_datalog_command, parse_edn};
pub use rules::RuleRegistry;
pub use types::{DatalogCommand, DatalogQuery, EdnValue, Pattern, Rule, Transaction, WhereClause};
