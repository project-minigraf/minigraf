pub mod graph;
pub mod minigraf;
pub mod query;
pub mod repl;
pub mod storage;

// Property Graph exports (Phase 1-2)
pub use graph::{GraphStorage, Node, Edge, Property, PropertyValue};

// Datalog EAV exports (Phase 3+)
pub use graph::types::{
    Fact, Value, EntityId, TxId, Attribute,
    tx_id_now, tx_id_from_system_time, tx_id_to_system_time,
};

pub use minigraf::Minigraf;
pub use query::{parse_query, Query, QueryExecutor};
pub use repl::Repl;
pub use storage::{StorageBackend, FileHeader, PAGE_SIZE};
