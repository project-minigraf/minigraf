pub mod datalog;

// Datalog query API (Phase 3+)
pub use datalog::{
    DatalogCommand, DatalogExecutor, DatalogQuery, EdnValue, Pattern, PatternMatcher, QueryResult,
    Transaction, parse_datalog_command, parse_edn,
};
