pub mod types;
pub mod storage;

pub use types::{Node, Edge, Property, PropertyValue, NodeId, EdgeId};
pub use storage::{GraphStorage, FactStorage};
