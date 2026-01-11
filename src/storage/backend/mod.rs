/// Backend implementations for different platforms.

pub mod memory;

#[cfg(not(target_arch = "wasm32"))]
pub mod file;

// Future: WASM backend
// #[cfg(target_arch = "wasm32")]
// pub mod indexeddb;

// Re-export backends
pub use memory::MemoryBackend;

#[cfg(not(target_arch = "wasm32"))]
pub use file::FileBackend;
