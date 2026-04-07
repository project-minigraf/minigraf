/// Backend implementations for different platforms.
pub mod memory;

#[cfg(not(target_arch = "wasm32"))]
pub mod file;

// Future: WASM backend
// #[cfg(target_arch = "wasm32")]
// pub mod indexeddb;

// Re-export backends
pub use memory::MemoryBackend;

// FileBackend is used in tests (persistent_facts tests use crate::storage::backend::FileBackend).
// The dead-code lint fires because non-test builds don't see the test-only usages.
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use file::FileBackend;
