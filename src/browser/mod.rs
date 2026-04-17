//! Browser WASM support: `BrowserDb` async façade backed by IndexedDB.
//!
//! This module is only compiled for `wasm32-unknown-unknown` with the `browser`
//! feature enabled. It is **not** compatible with Node.js or any server-side
//! runtime. For Node.js, use `@minigraf/node` (Phase 8.3).

pub mod buffer;
