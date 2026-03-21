pub mod storage;
pub mod types;

// Datalog EAV types (Phase 3+)
pub use storage::FactStorage;
pub use types::{
    Attribute, EntityId, Fact, TxId, Value, tx_id_from_system_time, tx_id_now, tx_id_to_system_time,
};
