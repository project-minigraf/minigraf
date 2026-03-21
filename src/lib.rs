pub mod graph;
pub mod query;
pub mod repl;
pub mod storage;
pub mod temporal;

// Datalog EAV storage (Phase 3+)
pub use graph::FactStorage;

// Datalog EAV types (Phase 3+)
pub use graph::types::{
    Fact, Value, EntityId, TxId, Attribute,
    tx_id_now, tx_id_from_system_time, tx_id_to_system_time,
    TransactOptions, VALID_TIME_FOREVER,
};

// REPL
pub use repl::Repl;

// Storage backend (Phase 2+)
pub use storage::backend::file::FileBackend;
pub use storage::persistent_facts::PersistentFactStorage;
pub use storage::{FileHeader, StorageBackend, PAGE_SIZE};

// Datalog query API (Phase 3+)
pub use query::{
    parse_datalog_command,
    parse_edn,
    DatalogCommand,
    DatalogExecutor,
    DatalogQuery,
    EdnValue,
    Pattern,
    PatternMatcher,
    QueryResult,
    Transaction,
};

// Bi-temporal query types (Phase 4+)
pub use query::datalog::types::{AsOf, ValidAt};
