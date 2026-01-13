pub mod datalog;

// Datalog query API (Phase 3+)
pub use datalog::{
    parse_datalog_command,
    parse_edn,
    DatalogCommand,
    DatalogExecutor,
    DatalogQuery,
    EdnValue,
    Pattern,
    PatternMatcher,
    QueryResult,
    Transaction
};
