pub mod evaluator;
pub mod executor;
pub mod functions;
pub mod matcher;
pub mod optimizer;
pub mod parser;
pub mod prepared;
pub mod rules;
pub mod stratification;
pub mod types;

// Re-export QueryResult at the `crate::query::datalog` level so that
// repl.rs can use `crate::query::datalog::QueryResult` directly.
pub use executor::QueryResult;
