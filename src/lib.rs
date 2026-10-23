#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
    )
)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Zero-config, single-file, embedded graph database with bi-temporal Datalog queries.
//!
//! Minigraf is the SQLite of graph databases: embedded, no server, no configuration,
//! a single portable `.graph` file. It stores data as Entity-Attribute-Value facts,
//! queries them with [Datalog](https://en.wikipedia.org/wiki/Datalog), and tracks every
//! change with full bi-temporal history (transaction time + valid time).
//!
//! # Installation
//!
//! ```toml
//! [dependencies]
//! minigraf = "0.21"
//! ```
//!
//! # Quick Start
//!
//! ```
//! use minigraf::{Minigraf, BindValue};
//!
//! // Open (or create) a database
//! let db = Minigraf::in_memory().unwrap();
//!
//! // Assert facts
//! db.execute(r#"(transact [[:alice :person/name "Alice"]
//!                          [:alice :person/age 30]
//!                          [:alice :friend :bob]
//!                          [:bob   :person/name "Bob"]])"#).unwrap();
//!
//! // Query with Datalog
//! let results = db.execute(r#"
//!     (query [:find ?friend-name
//!             :where [:alice :friend ?friend]
//!                    [?friend :person/name ?friend-name]])
//! "#).unwrap();
//!
//! // Explicit transaction — all-or-nothing
//! let mut tx = db.begin_write().unwrap();
//! tx.execute(r#"(transact [[:alice :person/age 31]])"#).unwrap();
//! tx.commit().unwrap();
//!
//! // Time travel — query the state as of transaction 1
//! db.execute("(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])").unwrap();
//! ```
//!
//! # Feature Flags
//!
//! | Feature | Target | Description |
//! |---------|--------|-------------|
//! | *(default)* | native / WASI | File-backed and in-memory databases; full Datalog engine |
//! | `browser` | `wasm32-unknown-unknown` | Enables the `browser` module (`BrowserDb`), backed by IndexedDB for use inside a web browser via `wasm-pack` |
//!
//! The `browser` feature is only meaningful on the `wasm32-unknown-unknown` target.
//! When browsing docs on [docs.rs](https://docs.rs/minigraf), switch the target to
//! `wasm32-unknown-unknown` (top-right target selector) to see the full browser API.
//!
//! ## WebAssembly targets
//!
//! - **Browser** (`wasm32-unknown-unknown` + `browser` feature) — `wasm-pack build --target web --features browser`
//! - **WASI / server-side** (`wasm32-wasip1`) — `cargo build --target wasm32-wasip1 --release --bin minigraf`

// The engine's unit tests share `tests/answers/`, which names the crate.
#[cfg(all(test, not(target_arch = "wasm32")))]
extern crate self as minigraf;

/// Incremental delivery of query results: [`Cursor`] and [`Batch`].
pub mod cursor;
pub mod db;
pub mod error;
/// Every fact version, streamed: [`FactLog`], [`FactFilter`], [`FactRecord`].
pub mod fact_log;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fault_matrix;
pub(crate) mod graph;
/// Build a new database from explicit fact records: [`LogWriter`].
#[cfg(not(target_arch = "wasm32"))]
pub mod log_writer;
pub(crate) mod query;
/// Interactive REPL for exploring a [`Minigraf`] database from the command line.
pub mod repl;
pub(crate) mod storage;
pub(crate) mod temporal;
/// The sidecar WAL of file-backed databases; wasm32 targets have no file backend.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod wal;

#[cfg(all(target_arch = "wasm32", feature = "browser"))]
#[cfg_attr(docsrs, doc(cfg(all(target_arch = "wasm32", feature = "browser"))))]
pub mod browser;

#[cfg(not(target_arch = "wasm32"))]
pub use db::OpenOptionsWithPath;
pub use db::{IntegrityReport, Minigraf, OpenOptions, SyncMode, WriteTransaction};
pub use error::{ErrorCategory, MinigrafError};
pub use repl::Repl;

// EAV value types — users construct and match on these
pub use graph::types::{EntityId, Value};

// Query result types
pub use cursor::{Batch, Cursor};
pub use fact_log::{FactFilter, FactLog, FactOrder, FactRecord};
#[cfg(not(target_arch = "wasm32"))]
pub use log_writer::LogWriter;
pub use query::datalog::executor::QueryResult;

// Bi-temporal query types
pub use query::datalog::types::{AsOf, ValidAt};

// Prepared statements
pub use query::datalog::prepared::{BindValue, PreparedQuery};

// Plan-independence tests (#596): run queries without reordering.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use query::datalog::optimizer::ReorderGuard;

/// Turns clause reordering on or off for queries on this thread until the
/// returned guard drops. Only for tests that check no answer depends on the
/// plan (#596); built with the `test-hooks` feature only.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn __reorder(on: bool) -> ReorderGuard {
    query::datalog::optimizer::set_reorder(on)
}

// Large-transaction tests (#550): statements read and written in small chunks.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use query::datalog::parser::statement::TxChunkGuard;

/// Sets how many facts a `transact` or `retract` materialises at once on this
/// thread until the returned guard drops (`None`: the built-in size), so a
/// statement with more facts takes the large-transaction path (#550). Only
/// for tests; built with the `test-hooks` feature only.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn __tx_chunk_facts(n: Option<usize>) -> TxChunkGuard {
    query::datalog::parser::statement::set_tx_chunk_facts(n)
}

/// The most materialised statement facts this thread held at once since the
/// last call (#550 acceptance). Built with the `test-hooks` feature only.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn __statement_facts_high_water() -> usize {
    query::datalog::parser::statement::take_held_high_water()
}

// Plan-independence tests (#546): force how negation bodies and `or` branches run.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use query::datalog::optimizer::{Subplans, SubplansGuard};

/// Forces how the streaming engine runs `not` / `not-join` bodies (hashed or
/// correlated) on this thread until the returned guard drops. Only for tests
/// that check both forms give the same rows (#546); built with the
/// `test-hooks` feature only.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn __subplans(mode: Subplans) -> SubplansGuard {
    query::datalog::optimizer::set_subplans(mode)
}
