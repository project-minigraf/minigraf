pub(crate) mod storage;
pub(crate) mod types;

// Datalog EAV types (Phase 3+)
pub(crate) use storage::FactStorage;
pub(crate) use types::{Attribute, Fact, TransactOptions, TxId, VALID_TIME_FOREVER,
    tx_id_from_system_time, tx_id_now, tx_id_to_system_time};
// EntityId and Value are part of the public API — re-exported from lib.rs
pub use types::{EntityId, Value};
