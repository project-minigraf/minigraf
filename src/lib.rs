#![forbid(unsafe_code)]

pub mod db;
pub mod graph;
pub mod query;
pub mod repl;
pub mod storage;
pub mod temporal;
pub mod wal;

pub use db::{Minigraf, OpenOptions, OpenOptionsWithPath, WriteTransaction};

// Datalog EAV storage (Phase 3+)
pub use graph::FactStorage;

// Datalog EAV types (Phase 3+)
pub use graph::types::{
    Attribute, EntityId, Fact, TransactOptions, TxId, VALID_TIME_FOREVER, Value,
    tx_id_from_system_time, tx_id_now, tx_id_to_system_time,
};

// REPL
pub use repl::Repl;

// Storage backend (Phase 2+)
pub use storage::backend::file::FileBackend;
pub use storage::persistent_facts::PersistentFactStorage;
pub use storage::{FileHeader, PAGE_SIZE, StorageBackend};

// Datalog query API (Phase 3+)
pub use query::{
    DatalogCommand, DatalogExecutor, DatalogQuery, EdnValue, Pattern, PatternMatcher, QueryResult,
    Transaction, parse_datalog_command, parse_edn,
};

// Bi-temporal query types (Phase 4+)
pub use query::datalog::types::{AsOf, ValidAt};

// Prepared statements (Phase 7.8)
pub use query::datalog::prepared::{BindValue, PreparedQuery};
