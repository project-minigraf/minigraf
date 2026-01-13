pub mod types;
pub mod storage;

// Datalog EAV types (Phase 3+)
pub use types::{Fact, Value, EntityId, TxId, Attribute, tx_id_now, tx_id_from_system_time, tx_id_to_system_time};
pub use storage::FactStorage;
