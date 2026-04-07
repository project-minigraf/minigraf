# Phase 7.9 — Publish Prep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare Minigraf for its first crates.io publish by narrowing the public API surface, writing rustdoc with doctests, achieving zero clippy warnings, adding a cross-platform CI matrix, and publishing v0.19.0.

**Architecture:** API narrowing first (breaking change isolated before docs), then rustdoc sweep against the final public surface, then clippy/CI polish, then publish. The `Repl` is refactored to hold `&Minigraf` so it stays public without leaking `FactStorage`.

**Tech Stack:** Rust 2024 edition, cargo, GitHub Actions, crates.io

---

## Pre-flight

Before starting, create an isolated worktree using the `superpowers:using-git-worktrees` skill. Work from that worktree for all tasks below. The `.worktrees/` directory under the repo root is the conventional location.

---

## Task 1: Refactor `Repl` to use `&Minigraf`

This must happen before visibility narrowing because `main.rs` currently constructs `Repl::new(storage)`. After this task, `Repl<'_>` holds `&Minigraf` and is constructed via `db.repl()`.

**Files:**
- Modify: `src/repl.rs`
- Modify: `src/db.rs` (add `repl()` method, remove `inner_fact_storage()`)
- Modify: `src/main.rs` (use new API)

- [ ] **Step 1: Write a compile-time test for the new API shape**

Add to `src/db.rs` inside the existing `#[cfg(test)]` block at the bottom of the file:

```rust
#[test]
fn repl_constructed_from_db() {
    let db = Minigraf::in_memory().unwrap();
    // Verify db.repl() compiles and returns a Repl that borrows from db.
    let _repl = db.repl();
    // If this compiles, the lifetime relationship is correct.
}
```

- [ ] **Step 2: Run the test to verify it fails (method does not exist yet)**

```bash
cargo test repl_constructed_from_db 2>&1 | head -20
```

Expected: compile error — `no method named 'repl' found for struct 'Minigraf'`

- [ ] **Step 3: Add `Minigraf::repl()` to `src/db.rs`**

Add after the `inner_fact_storage` method (around line 614) and mark `inner_fact_storage` as `pub(crate)`:

```rust
/// Return an interactive REPL that reads commands from stdin.
///
/// The REPL borrows the database for the duration of the session.
/// Call [`Repl::run`] to start the interactive loop.
///
/// # Example
/// ```no_run
/// # use minigraf::Minigraf;
/// let db = Minigraf::in_memory().unwrap();
/// db.repl().run();
/// ```
pub fn repl(&self) -> crate::repl::Repl<'_> {
    crate::repl::Repl::new(self)
}
```

Change the visibility of `inner_fact_storage`:

```rust
#[doc(hidden)]
pub(crate) fn inner_fact_storage(&self) -> crate::graph::FactStorage {
    self.inner.fact_storage.clone()
}
```

- [ ] **Step 4: Refactor `src/repl.rs` to hold `&Minigraf`**

Replace the entire file with:

```rust
use crate::db::Minigraf;
use std::io::{self, IsTerminal, Write};

pub struct Repl<'a> {
    db: &'a Minigraf,
}

impl<'a> Repl<'a> {
    pub(crate) fn new(db: &'a Minigraf) -> Self {
        Repl { db }
    }

    pub fn run(&self) {
        if io::stdin().is_terminal() {
            println!(
                "Minigraf v{} - Interactive Datalog Console",
                env!("CARGO_PKG_VERSION")
            );
            println!();
            println!("Data operations:");
            println!("  (transact [...])                    - assert facts");
            println!("  (transact {{:valid-from ... :valid-to ...}} [...]) - with valid time");
            println!("  (retract [...])                     - retract facts");
            println!();
            println!("Queries:");
            println!("  (query [:find ?x :where ...])       - basic query");
            println!("  (rule [(name ?a ?b) [?a :attr ?b]]) - define a rule");
            println!();
            println!("Temporal queries:");
            println!(
                "  (query [:find ?x :as-of 50 :where ...])                     - state as of tx counter 50"
            );
            println!(
                "  (query [:find ?x :as-of \"2024-01-15T10:00:00Z\" :where ...]) - state as of UTC timestamp"
            );
            println!(
                "  (query [:find ?x :valid-at \"2023-06-01\" :where ...])        - facts valid on date"
            );
            println!(
                "  (query [:find ?x :valid-at :any-valid-time :where ...])     - all facts, ignoring validity"
            );
            println!();
            println!("Note: queries without :valid-at return only currently valid facts.");
            println!();
            println!("Type EXIT to quit.\n");
        }

        let mut command_buffer = String::new();
        let mut is_multiline = false;
        let interactive = io::stdin().is_terminal();

        loop {
            if interactive {
                if is_multiline {
                    print!("       .> ");
                } else {
                    print!("minigraf> ");
                }
                io::stdout().flush().ok();
            }

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }

                    let line = input.trim();

                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }

                    if line.to_uppercase() == "EXIT" {
                        break;
                    }

                    if !command_buffer.is_empty() {
                        command_buffer.push(' ');
                    }
                    command_buffer.push_str(line);

                    if self.is_command_complete(&command_buffer) {
                        match self.db.execute(&command_buffer) {
                            Ok(result) => {
                                self.print_result(result);
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                            }
                        }

                        command_buffer.clear();
                        is_multiline = false;
                        if interactive {
                            println!();
                        }
                    } else {
                        is_multiline = true;
                    }
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    break;
                }
            }
        }
    }

    fn is_command_complete(&self, input: &str) -> bool {
        let mut depth = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for ch in input.chars() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_string => {
                    escape_next = true;
                }
                '"' => {
                    in_string = !in_string;
                }
                '(' if !in_string => {
                    depth += 1;
                }
                ')' if !in_string => {
                    depth -= 1;
                }
                _ => {}
            }
        }

        depth == 0 && input.contains('(')
    }

    fn print_result(&self, result: crate::query::datalog::QueryResult) {
        use crate::query::datalog::QueryResult as DResult;

        match result {
            DResult::Transacted(tx_id) => {
                println!("✓ Transacted successfully (tx: {})", tx_id);
            }
            DResult::Retracted(tx_id) => {
                println!("✓ Retracted successfully (tx: {})", tx_id);
            }
            DResult::QueryResults { vars, results } => {
                if results.is_empty() {
                    println!("No results found.");
                } else {
                    println!("{}", vars.join("\t"));
                    println!("{}", "-".repeat(vars.len() * 20));

                    for row in &results {
                        let formatted_row: Vec<String> =
                            row.iter().map(|v| self.format_value(v)).collect();
                        println!("{}", formatted_row.join("\t"));
                    }

                    println!("\n{} result(s) found.", results.len());
                }
            }
            DResult::Ok => {
                println!("✓ OK");
            }
        }
    }

    fn format_value(&self, value: &crate::graph::types::Value) -> String {
        use crate::graph::types::Value;

        match value {
            Value::String(s) => format!("\"{}\"", s),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Boolean(b) => b.to_string(),
            Value::Ref(uuid) => format!("#uuid {}", uuid),
            Value::Keyword(k) => k.clone(),
            Value::Null => "nil".to_string(),
        }
    }
}
```

- [ ] **Step 5: Update `src/main.rs` to use the new API**

Replace the entire file with:

```rust
use minigraf::{Minigraf, OpenOptions};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file_flag_pos = args.iter().position(|a| a == "--file");
    let db_path = file_flag_pos.and_then(|i| args.get(i + 1)).cloned();

    if file_flag_pos.is_some() && db_path.is_none() {
        eprintln!("error: --file requires a path argument");
        std::process::exit(1);
    }

    let db = if let Some(path) = db_path {
        OpenOptions::new().path(path).open()?
    } else {
        Minigraf::in_memory()?
    };

    db.repl().run();
    Ok(())
}
```

- [ ] **Step 6: Run the test suite to verify everything passes**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 780 passed; 0 failed`

- [ ] **Step 7: Commit**

```bash
git add src/repl.rs src/db.rs src/main.rs
git commit -m "refactor(repl): replace FactStorage constructor with Minigraf::repl(&self) -> Repl<'_>"
```

---

## Task 2: Narrow `lib.rs` re-exports and module visibility

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Replace the contents of `src/lib.rs`**

```rust
#![forbid(unsafe_code)]
#![warn(missing_docs)]
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
//! minigraf = "0.19"
//! ```
//!
//! # Quick Start
//!
//! ```
//! use minigraf::{Minigraf, OpenOptions, BindValue};
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
//!
//! // Prepared statements — parse once, execute many times
//! let pq = db.prepare(
//!     "(query [:find ?name :where [$entity :person/name ?name]])"
//! ).unwrap();
//! let alice_id = uuid::Uuid::new_v4(); // normally a real entity UUID
//! // pq.execute(&[("entity", BindValue::Entity(alice_id))]).unwrap();
//! ```

pub mod db;
pub(crate) mod graph;
pub(crate) mod query;
pub mod repl;
pub(crate) mod storage;
pub(crate) mod temporal;
pub(crate) mod wal;

pub use db::{Minigraf, OpenOptions, OpenOptionsWithPath, WriteTransaction};
pub use repl::Repl;

// EAV value types (users construct and match on these)
pub use graph::types::{EntityId, Value};

// Query result type
pub use query::datalog::executor::QueryResult;

// Bi-temporal query types
pub use query::datalog::types::{AsOf, ValidAt};

// Prepared statements
pub use query::datalog::prepared::{BindValue, PreparedQuery};
```

- [ ] **Step 2: Build and observe compile errors**

```bash
cargo build 2>&1 | grep "^error" | head -30
```

The build will fail with errors about types that are now `pub(crate)` but used in public positions. Note every error — these guide Tasks 3 and 4.

- [ ] **Step 3: Run full test suite to note all failures**

```bash
cargo test 2>&1 | grep "^error" | head -40
```

Document the list of modules/types that need fixing.

---

## Task 3: Fix visibility in `src/graph/` module

The `graph` module is now `pub(crate)`. Types used only internally need no change; types re-exported from `lib.rs` (`Value`, `EntityId`) must remain `pub` within the crate.

**Files:**
- Inspect then modify: `src/graph/mod.rs` (or `src/graph/storage.rs`, `src/graph/types.rs`)

- [ ] **Step 1: Read the current graph module structure**

```bash
cargo build 2>&1 | grep "src/graph" | head -20
```

- [ ] **Step 2: Ensure `Value` and `EntityId` remain `pub` in `src/graph/types.rs`**

These types are re-exported from `lib.rs` so they must stay `pub` (not `pub(crate)`). The rest of the types in `graph/types.rs` (`Fact`, `TxId`, `Attribute`, `TransactOptions`, `VALID_TIME_FOREVER`, `tx_id_*` functions) should become `pub(crate)`.

In `src/graph/types.rs`, change visibility on everything except `Value` and `EntityId`:

```rust
// KEEP pub:
pub struct Value { ... }
pub struct EntityId(pub uuid::Uuid);  // or newtype — keep pub

// CHANGE to pub(crate):
pub(crate) struct Fact { ... }
pub(crate) type TxId = i64;
pub(crate) type Attribute = String;
pub(crate) struct TransactOptions { ... }
pub(crate) const VALID_TIME_FOREVER: i64 = i64::MAX;
pub(crate) fn tx_id_now() -> TxId { ... }
pub(crate) fn tx_id_from_system_time(...) -> TxId { ... }
pub(crate) fn tx_id_to_system_time(...) -> ... { ... }
```

- [ ] **Step 3: Fix `src/graph/mod.rs` or `src/graph/storage.rs`**

`FactStorage` becomes `pub(crate)`. Check `src/graph/mod.rs` for the re-export:

```rust
pub(crate) use storage::FactStorage;
```

- [ ] **Step 4: Build to check graph errors are resolved**

```bash
cargo build 2>&1 | grep "src/graph" | head -10
```

Expected: no remaining errors in `src/graph/`.

---

## Task 4: Fix visibility in `src/storage/`, `src/query/`, `src/wal.rs`

**Files:**
- Modify: `src/storage/mod.rs`
- Modify: `src/storage/backend/file.rs`
- Modify: `src/storage/backend/mod.rs` (if it exists)
- Modify: `src/query/mod.rs`
- Modify: `src/wal.rs`

- [ ] **Step 1: Fix storage module visibility**

In `src/storage/mod.rs`, change `StorageBackend`, `FileHeader`, `PAGE_SIZE`, `CommittedFactReader`, `CommittedIndexReader` to `pub(crate)`:

```rust
pub(crate) trait StorageBackend { ... }
pub(crate) struct FileHeader { ... }
pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) trait CommittedFactReader { ... }
pub(crate) trait CommittedIndexReader { ... }
```

In `src/storage/backend/file.rs`:
```rust
pub(crate) struct FileBackend { ... }
```

In `src/storage/persistent_facts.rs`:
```rust
pub(crate) struct PersistentFactStorage<B> { ... }
```

In `src/storage/cache.rs`:
```rust
pub(crate) struct PageCache { ... }
```

- [ ] **Step 2: Fix query module visibility**

In `src/query/mod.rs` (or wherever the re-exports live), change all query-internal types to `pub(crate)`. The types re-exported from `lib.rs` (`QueryResult`, `AsOf`, `ValidAt`, `BindValue`, `PreparedQuery`) must stay `pub`.

```rust
// Keep pub (re-exported from lib.rs):
pub use datalog::executor::QueryResult;
pub use datalog::types::{AsOf, ValidAt};
pub use datalog::prepared::{BindValue, PreparedQuery};

// Change to pub(crate):
pub(crate) use datalog::executor::DatalogExecutor;
pub(crate) use datalog::parser::{parse_datalog_command, parse_edn};
pub(crate) use datalog::types::{DatalogCommand, EdnValue, Pattern, Transaction};
pub(crate) use datalog::matcher::PatternMatcher;
```

- [ ] **Step 3: Fix `src/wal.rs`**

```rust
pub(crate) struct WalWriter { ... }
pub(crate) struct WalReader { ... }
// etc.
```

- [ ] **Step 4: Build to verify all errors resolved**

```bash
cargo build 2>&1 | grep "^error" | head -10
```

Expected: clean build.

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 780 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/graph/ src/storage/ src/query/ src/wal.rs
git commit -m "feat!: narrow public API — hide internal types, expose only core database surface"
```

---

## Task 5: Fix `unwrap`/`expect` in library code

**Files:**
- Modify: `src/storage/cache.rs` (7 RwLock unwraps)
- Modify: `src/query/datalog/evaluator.rs` (8 RwLock unwraps)
- Verify: `src/db.rs` line 942 (`wal.as_mut().unwrap()`)

- [ ] **Step 1: Fix RwLock unwraps in `src/storage/cache.rs`**

Replace every `.read().unwrap()` and `.write().unwrap()` on the cache's inner `RwLock` with an `.expect()` carrying a message. There are 7 occurrences. Example:

```rust
// Before:
let inner = self.inner.read().unwrap();
// After:
let inner = self.inner.read().expect("page cache lock poisoned");

// Before:
let mut inner = self.inner.write().unwrap();
// After:
let mut inner = self.inner.write().expect("page cache lock poisoned");
```

Apply the same pattern to the one-liner at the bottom of the file:
```rust
// Before:
self.inner.read().unwrap().entries.len()
// After:
self.inner.read().expect("page cache lock poisoned").entries.len()
```

- [ ] **Step 2: Fix RwLock unwraps in `src/query/datalog/evaluator.rs`**

There are 8 occurrences of `.read().unwrap()` on rule/function registries. Replace each:

```rust
// Before:
let registry = self.rules.read().unwrap();
// After:
let registry = self.rules.read().expect("rule registry lock poisoned");

// Before:
&self.functions.read().unwrap()
// After:
&self.functions.read().expect("function registry lock poisoned")

// Before:
let registry_guard = self.rules.read().unwrap();
// After:
let registry_guard = self.rules.read().expect("rule registry lock poisoned");
```

- [ ] **Step 3: Verify `src/db.rs` line 942**

The `wal.as_mut().unwrap()` at line 942 is logically infallible (we set `*wal = Some(...)` on the line immediately above it). Add an expect message for clarity:

```rust
// Before:
let wal_writer = wal.as_mut().unwrap();
// After:
let wal_writer = wal.as_mut().expect("WAL writer was just initialised above");
```

- [ ] **Step 4: Verify `src/repl.rs`** — already fixed in Task 1 (`.flush().ok()`)

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 780 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
git add src/storage/cache.rs src/query/datalog/evaluator.rs src/db.rs
git commit -m "fix: replace bare unwrap() with expect() messages on RwLock and WAL paths"
```

---

## Task 6: `Cargo.toml` — docs.rs metadata and version bump

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add `[package.metadata.docs.rs]` and bump version**

Edit `Cargo.toml`. Change the version line and add the metadata section after `[package]`:

```toml
version = "0.19.0"
```

Add after the existing `[features]` section (before `[profile.release]`):

```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

- [ ] **Step 2: Verify Cargo.toml is valid**

```bash
cargo metadata --no-deps --format-version 1 > /dev/null && echo "OK"
```

Expected: `OK`

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore: bump to v0.19.0, add docs.rs metadata"
```

---

## Task 7: Rustdoc sweep — crate-level and `db.rs`

The `lib.rs` crate-level doc was written in Task 2. This task adds doc comments to every public method in `db.rs` and verifies doctests pass.

**Files:**
- Modify: `src/db.rs`
- Modify: `src/lib.rs` (fix the quick-start doctest if needed)

- [ ] **Step 1: Check current doc state**

```bash
cargo doc --no-deps 2>&1 | grep "^warning" | head -20
```

Note every `missing_docs` warning. Each one becomes a doc comment to write.

- [ ] **Step 2: Add/update rustdoc on `OpenOptions` and `OpenOptionsWithPath`**

In `src/db.rs`, ensure `OpenOptions` has a struct-level doc and each field has a doc comment. `OpenOptionsWithPath` needs a brief doc. The example must be `no_run` since it touches the filesystem:

```rust
/// Builder for configuring and opening a [`Minigraf`] database.
///
/// Create with [`OpenOptions::new`], configure with the builder methods,
/// then open a file-backed database with [`OpenOptions::path`] or an
/// in-memory database with [`OpenOptions::open_memory`].
///
/// # Example
/// ```no_run
/// use minigraf::OpenOptions;
/// let db = OpenOptions::new()
///     .page_cache_size(512)
///     .path("myapp.graph")
///     .open()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct OpenOptions { ... }
```

- [ ] **Step 3: Add/update rustdoc on `Minigraf` methods**

Ensure every `pub fn` on `Minigraf` has a doc comment. The struct already has a doc block; update the existing `no_run` examples to use the crate root path (`minigraf::Minigraf`) not the module path (`minigraf::db::Minigraf`):

```rust
/// # Example
/// ```no_run
/// use minigraf::Minigraf;
/// let db = Minigraf::open("mydb.graph").unwrap();
/// ```
pub fn open(path: impl AsRef<Path>) -> Result<Self> { ... }
```

Add docs to `register_aggregate` and `register_predicate` with in-memory examples (they don't touch the filesystem):

```rust
/// Register a custom aggregate function callable from Datalog `:find` clauses.
///
/// Aggregates are not persisted — re-register on each `open()`, exactly as
/// SQLite requires for `sqlite3_create_function`.
///
/// # Example
/// ```
/// # use minigraf::{Minigraf, Value};
/// let db = Minigraf::in_memory().unwrap();
/// db.register_aggregate(
///     "sum2",
///     || 0_i64,
///     |acc: i64, v: &Value| match v {
///         Value::Integer(i) => acc + i,
///         _ => acc,
///     },
///     |acc: i64, _n| Value::Integer(acc),
/// ).unwrap();
/// ```
pub fn register_aggregate<Acc>(...) { ... }
```

- [ ] **Step 4: Add docs to `WriteTransaction`**

```rust
/// An explicit write transaction. All operations are buffered until [`WriteTransaction::commit`].
///
/// Obtained from [`Minigraf::begin_write`]. Drop without committing to roll back.
///
/// # Example
/// ```
/// # use minigraf::Minigraf;
/// let db = Minigraf::in_memory().unwrap();
/// let mut tx = db.begin_write().unwrap();
/// tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// tx.execute(r#"(transact [[:alice :person/age 30]])"#).unwrap();
/// tx.commit().unwrap();
/// ```
pub struct WriteTransaction<'a> { ... }
```

Add intra-doc links to `begin_write` pointing at `WriteTransaction`:

```rust
/// Begin an explicit write transaction. Returns a [`WriteTransaction`] that
/// buffers all writes until [`WriteTransaction::commit`] is called.
pub fn begin_write(&self) -> Result<WriteTransaction<'_>> { ... }
```

- [ ] **Step 5: Add docs to `PreparedQuery`**

`PreparedQuery` lives in `src/query/datalog/prepared.rs`. Add struct-level and method-level docs:

```rust
/// A parsed and planned query that can be executed many times with different bind values.
///
/// Obtain via [`Minigraf::prepare`]. Reuses the query plan across executions —
/// only the substituted values change.
///
/// # Example
/// ```
/// # use minigraf::{Minigraf, BindValue};
/// let db = Minigraf::in_memory().unwrap();
/// db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
///
/// let pq = db.prepare(
///     "(query [:find ?name :where [$entity :person/name ?name]])"
/// ).unwrap();
///
/// let alice = uuid::Uuid::nil(); // placeholder; use a real entity UUID
/// let results = pq.execute(&[("entity", BindValue::Entity(alice))]).unwrap();
/// ```
pub struct PreparedQuery { ... }
```

- [ ] **Step 6: Verify doctests compile and pass**

```bash
cargo test --doc 2>&1 | tail -10
```

Expected: all doc tests pass. Fix any compilation errors in examples (typically missing `use` statements or filesystem-touching examples that need `no_run`).

- [ ] **Step 7: Verify doc build is warning-free**

```bash
cargo doc --no-deps 2>&1 | grep "^warning" | head -10
```

Expected: no `missing_docs` warnings on public items. Fix any remaining gaps.

- [ ] **Step 8: Commit**

```bash
git add src/db.rs src/lib.rs src/query/datalog/prepared.rs
git commit -m "docs: rustdoc sweep for Minigraf, OpenOptions, WriteTransaction, PreparedQuery"
```

---

## Task 8: Rustdoc sweep — value and utility types

**Files:**
- Modify: `src/graph/types.rs` (Value, EntityId)
- Modify: `src/query/datalog/types.rs` (AsOf, ValidAt)
- Modify: `src/query/datalog/prepared.rs` (BindValue)
- Modify: `src/query/datalog/executor.rs` (QueryResult)
- Modify: `src/repl.rs` (Repl)

- [ ] **Step 1: Document `Value` in `src/graph/types.rs`**

```rust
/// A value stored in an Entity-Attribute-Value fact.
///
/// Values appear in Datalog patterns as literals:
///
/// | Variant | Datalog literal | Example |
/// |---|---|---|
/// | `String` | Quoted string | `"hello"` |
/// | `Integer` | Integer | `42` |
/// | `Float` | Float | `3.14` |
/// | `Boolean` | Boolean keyword | `true` |
/// | `Ref` | Entity reference | `:some-entity` (resolved UUID) |
/// | `Keyword` | Keyword | `:status/active` |
/// | `Null` | Nil | `nil` |
pub enum Value {
    /// A UTF-8 string.
    String(String),
    /// A 64-bit signed integer.
    Integer(i64),
    /// A 64-bit float.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// A reference to another entity (stored as a UUID).
    Ref(uuid::Uuid),
    /// A keyword such as `:status/active`.
    Keyword(String),
    /// Null / absent value.
    Null,
}
```

- [ ] **Step 2: Document `EntityId` in `src/graph/types.rs`**

```rust
/// The identity of an entity in the graph — a UUID.
///
/// Entity IDs appear as the first element of EAV patterns:
/// `[?entity :person/name ?name]`
pub struct EntityId(pub uuid::Uuid);
```

- [ ] **Step 3: Document `QueryResult` in `src/query/datalog/executor.rs`**

```rust
/// The result returned by [`Minigraf::execute`] and [`WriteTransaction::execute`].
pub enum QueryResult {
    /// A `transact` command succeeded. Contains the transaction ID.
    Transacted(/* tx_id: */ i64),
    /// A `retract` command succeeded. Contains the transaction ID.
    Retracted(/* tx_id: */ i64),
    /// A `query` returned rows. `vars` are the `:find` variable names;
    /// `results` is a list of rows, each a list of [`Value`]s.
    QueryResults {
        /// The variable names from the `:find` clause, in order.
        vars: Vec<String>,
        /// Each inner `Vec` is one result row, aligned to `vars`.
        results: Vec<Vec<crate::graph::types::Value>>,
    },
    /// A `rule` definition was registered successfully.
    Ok,
}
```

- [ ] **Step 4: Document `AsOf` and `ValidAt` in `src/query/datalog/types.rs`**

```rust
/// Temporal filter: constrains a query to the state as of a past transaction.
///
/// Use `:as-of N` (transaction counter) or `:as-of "2024-01-15T10:00:00Z"` (UTC timestamp)
/// in a query to travel back in transaction time.
pub enum AsOf { ... }

/// Temporal filter: constrains a query to facts valid at a specific point in valid time.
///
/// Use `:valid-at "2024-01-15"` in a query, or `:valid-at :any-valid-time` to
/// return facts regardless of their validity interval.
pub enum ValidAt { ... }
```

- [ ] **Step 5: Document `BindValue` in `src/query/datalog/prepared.rs`**

```rust
/// A value to substitute into a [`PreparedQuery`] bind slot (`$slot`).
///
/// | Variant | Slot position |
/// |---|---|
/// | `Entity(Uuid)` | Entity position: `[$entity :attr ?val]` |
/// | `Val(Value)` | Value position: `[?e :attr $val]` |
/// | `TxCount(u64)` | `:as-of $tx` (transaction counter) |
/// | `Timestamp(i64)` | `:as-of $ts` or `:valid-at $date` (Unix milliseconds) |
/// | `AnyValidTime` | `:valid-at $vt` sentinel meaning "ignore valid-time filter" |
pub enum BindValue { ... }
```

- [ ] **Step 6: Document `Repl` in `src/repl.rs`**

```rust
/// An interactive Datalog REPL that reads commands from stdin and prints results to stdout.
///
/// Obtain via [`Minigraf::repl`]. The REPL detects whether stdin is a terminal
/// and suppresses prompts and the welcome banner when running non-interactively
/// (e.g. when stdin is a pipe or file).
///
/// # Example
/// ```no_run
/// use minigraf::Minigraf;
/// let db = Minigraf::in_memory().unwrap();
/// db.repl().run();
/// ```
pub struct Repl<'a> { ... }
```

- [ ] **Step 7: Verify doctests and doc build**

```bash
cargo test --doc 2>&1 | tail -5
cargo doc --no-deps 2>&1 | grep "^warning" | head -5
```

Expected: all tests pass, zero warnings.

- [ ] **Step 8: Commit**

```bash
git add src/graph/types.rs src/query/datalog/types.rs src/query/datalog/prepared.rs \
        src/query/datalog/executor.rs src/repl.rs
git commit -m "docs: rustdoc sweep for Value, EntityId, QueryResult, AsOf, ValidAt, BindValue, Repl"
```

---

## Task 9: README — installation section and badges

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add crates.io and docs.rs badges**

Open `README.md`. The badge row is at lines 3–8. Add two badges after the existing ones:

```markdown
[![crates.io](https://img.shields.io/crates/v/minigraf.svg)](https://crates.io/crates/minigraf)
[![docs.rs](https://docs.rs/minigraf/badge.svg)](https://docs.rs/minigraf)
```

- [ ] **Step 2: Add Installation section before Quick Start**

Insert after the `## Why Datalog?` section and before `## Quick Start`:

```markdown
## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
minigraf = "0.19"
```

Or via cargo:

```sh
cargo add minigraf
```
```

- [ ] **Step 3: Update the Phase badge in README**

The badge at line 8 currently says `phase-7.8%20complete`. Update it to `phase-7.9%20complete`:

```markdown
[![Phase](https://img.shields.io/badge/phase-7.9%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)
```

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs(readme): add installation section, crates.io and docs.rs badges"
```

---

## Task 10: Clippy clean

**Files:**
- Run clippy, fix all warnings
- Modify: `.github/workflows/rust-clippy.yml`

- [ ] **Step 1: Run clippy with `-D warnings`**

```bash
cargo clippy -- -D warnings 2>&1 | grep "^error\|^warning" | head -30
```

Note every warning.

- [ ] **Step 2: Fix all clippy warnings**

Common patterns to expect:

- Unused imports surfaced by visibility narrowing — remove them
- `clippy::needless_pass_by_value` on `pub(crate)` functions — add `#[allow]` or fix
- `clippy::module_name_repetitions` — suppress with `#[allow(clippy::module_name_repetitions)]` at the module level if appropriate

Fix each warning until the command produces no output.

- [ ] **Step 3: Verify clean**

```bash
cargo clippy -- -D warnings 2>&1 | grep "^error" | wc -l
```

Expected: `0`

- [ ] **Step 4: Add PR trigger to `rust-clippy.yml`**

The current `rust-clippy.yml` already runs on push and PR to main. The issue is `continue-on-error: true` on the clippy step — it never fails the build. Add a separate step that fails on warnings:

Add after the existing `Run rust-clippy` step:

```yaml
      - name: Fail on clippy warnings
        run: cargo clippy -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/rust-clippy.yml
# Also add any src/ files changed for clippy fixes
git add src/
git commit -m "fix: resolve all clippy warnings; enforce -D warnings in CI"
```

---

## Task 11: CI matrix — cross-platform tests

**Files:**
- Modify: `.github/workflows/rust.yml`

- [ ] **Step 1: Update `rust.yml` to add OS matrix**

Replace the current `runs-on: ubuntu-latest` line and add a strategy matrix:

```yaml
jobs:
  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
      fail-fast: false
    runs-on: ${{ matrix.os }}

    steps:
    - uses: actions/checkout@v3
    - name: Build
      run: cargo build --verbose
    - name: Run tests
      run: cargo test --verbose
```

`fail-fast: false` means all three OS jobs complete even if one fails — useful for diagnosing platform-specific failures on the first run.

- [ ] **Step 2: Commit and push to trigger CI**

```bash
git add .github/workflows/rust.yml
git commit -m "ci: add macOS and Windows to test matrix"
git push
```

- [ ] **Step 3: Monitor CI results**

```bash
gh run list --limit 3
```

Check the run triggered by the push. If Windows or macOS tests fail, read the failure:

```bash
gh run view <run-id> --log-failed
```

Common Windows failure: WAL sidecar path. Look for `PathBuf` operations that append `.wal` as a string suffix. Fix if found, then re-run.

---

## Task 12: Publish

This task is a manual checklist. Do not proceed until CI is green on all three OS.

**Files:**
- Modify: `Cargo.toml` (version already bumped in Task 6)
- Modify: `CLAUDE.md`, `ROADMAP.md`, `CHANGELOG.md`, `TEST_COVERAGE.md`

- [ ] **Step 1: Verify the package contents**

```bash
cargo package --list 2>&1 | grep -v "\.rs$" | head -30
```

Check that no `.graph` test fixture files, `.wal` files, secrets, or large binaries are included. The list should contain only source files, `Cargo.toml`, `README.md`, and documentation.

- [ ] **Step 2: Final pre-publish checks**

```bash
cargo test 2>&1 | tail -3
cargo doc --no-deps 2>&1 | grep "^warning" | wc -l
cargo clippy -- -D warnings 2>&1 | grep "^error" | wc -l
cargo test --doc 2>&1 | tail -3
```

All must show zero errors/warnings and passing tests.

- [ ] **Step 3: Dry run**

```bash
cargo publish --dry-run 2>&1
```

Expected: `Uploading minigraf v0.19.0` with no errors. Fix any Cargo.toml metadata issues before proceeding.

- [ ] **Step 4: Publish**

```bash
cargo publish
```

Verify the crate appears at `https://crates.io/crates/minigraf`.

- [ ] **Step 5: Tag the release**

```bash
git tag -a v0.19.0 -m "Phase 7.9 complete — publish prep, API narrowing, crates.io publish"
git push origin v0.19.0
```

- [ ] **Step 6: Sync documentation files**

Update the following files to reflect Phase 7.9 completion:

**`ROADMAP.md`** — Mark Phase 7.9 as complete:
```markdown
## Phase 7: Datalog Completeness ✅ COMPLETE
...
- **7.9** ✅ Publish Prep (crates.io — API cleanup, rustdoc, clippy, CI matrix)
```

**`CLAUDE.md`** — Update the test count and any status lines referencing Phase 7.

**`CHANGELOG.md`** — Add a v0.19.0 entry summarising: API narrowing, `Minigraf::repl()`, rustdoc, doctests, CI matrix, first crates.io publish.

**`TEST_COVERAGE.md`** — Update if test count changed due to added doctests.

- [ ] **Step 7: Commit doc sync**

```bash
git add ROADMAP.md CLAUDE.md CHANGELOG.md TEST_COVERAGE.md README.md
git commit -m "docs: sync all documentation for Phase 7.9 / v0.19.0 completion"
```

- [ ] **Step 8: Update wiki**

```bash
cd .wiki
# Edit Architecture.md — update the Public API Surface section
# No changes needed to Datalog-Reference.md
git add -A
git commit -m "docs(wiki): update Architecture.md for v0.19.0 API surface"
git push
cd ..
```

---

## Verification Summary

```bash
# After Task 4 (API narrowing complete):
cargo build && cargo test
# Expected: clean build, 780 tests pass

# After Task 8 (all rustdoc written):
cargo doc --no-deps         # zero warnings
cargo test --doc            # all doctests pass

# After Task 10 (clippy clean):
cargo clippy -- -D warnings # zero errors

# Before Task 12 (publish):
cargo package --list        # no secrets or test fixtures
cargo publish --dry-run     # no metadata errors

# Final:
cargo test                  # full suite green (unit + integration + doc)
```
