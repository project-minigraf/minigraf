# Phase 7.9 — Publish Prep (crates.io) Design

**Date**: 2026-04-07
**Status**: Design approved
**Version target**: 0.19.0

---

## Context

Phase 7 (Datalog Completeness) is functionally complete. Phase 7.9 prepares Minigraf for its first crates.io publish. The work is cleanup and polish — no new query features. The goal is a clean, narrow, well-documented public API that can evolve to 1.0 without breaking embedders.

---

## Approach

API-first, then polish:

1. **API narrowing** — break the build on internal type leakage, fix it, verify tests green
2. **Rustdoc sweep** — document the final public surface
3. **Clippy + unwrap audit** — zero-warning clean
4. **CI matrix** — cross-platform test coverage
5. **Publish** — dry-run, version bump, tag

---

## Section 1: Public API Surface

### Stays `pub`

| Type / fn | Notes |
|---|---|
| `Minigraf` | Core database handle |
| `OpenOptions`, `OpenOptionsWithPath` | Builder entry points |
| `WriteTransaction` | Explicit transaction |
| `QueryResult` | Query return type |
| `Value` | EAV value type — users construct and pattern-match on this |
| `EntityId` | Uuid newtype — appears in query results |
| `BindValue` | Prepared statement bind values |
| `PreparedQuery` | Prepared statement handle |
| `AsOf`, `ValidAt` | Temporal filter types |
| `Repl<'_>` | Kept public for embedders who want an interactive REPL in their binary; constructed via `db.repl()` |

### Becomes `pub(crate)`

`FactStorage`, `FileBackend`, `MemoryBackend`, `PersistentFactStorage`, `FileHeader`, `PAGE_SIZE`,
`StorageBackend`, `DatalogExecutor`, `PatternMatcher`, `DatalogCommand`, `EdnValue`, `Pattern`,
`Transaction`, `parse_datalog_command`, `parse_edn`, `Fact`, `Attribute`, `TxId`, `tx_id_*`
functions, `VALID_TIME_FOREVER`, `Wal`, `CommittedFactReader`, `CommittedIndexReader`

### Repl construction change

**Problem**: `Repl::new(FactStorage)` takes an internal type — external users cannot construct a `Repl` without `FactStorage` being public.

**Solution**: Add `Minigraf::repl(&self) -> Repl<'_>` factory method. `Repl<'_>` holds `&'_ Minigraf` and calls `Minigraf::execute()` internally rather than accessing `FactStorage` directly. `inner_fact_storage()` and the old `Repl::new(FactStorage)` constructor are removed (or made `pub(crate)`).

**Ownership**: `Repl<'_>` borrows `Minigraf` for its lifetime — `db.repl().run()` holds the borrow for the duration of the interactive session. This is natural: you cannot drop the database while the REPL is running.

```rust
// Public API after change
let db = OpenOptions::new().path("myapp.graph").open()?;
db.repl().run();

// main.rs binary — same call, no change in behaviour
```

This works for both file-backed and in-memory databases. The Repl going through `Minigraf::execute()` is strictly better than the old direct `FactStorage` access — WAL, checkpointing, and all Minigraf machinery are exercised correctly in REPL sessions.

### `lib.rs` module visibility

- `repl` module: keep `pub` (Repl is public)
- `db` module: keep `pub`
- `graph` module: `pub(crate)`
- `storage` module: `pub(crate)`
- `temporal` module: `pub(crate)`
- `wal` module: `pub(crate)`
- `query` module: `pub(crate)` (query types surfaced via `db` re-exports only)

---

## Section 2: Rustdoc Sweep

Target: `cargo doc --no-deps` with zero warnings and `cargo test --doc` with all doc examples passing. Add `#![warn(missing_docs)]` to `lib.rs`.

### `Cargo.toml` — `[package.metadata.docs.rs]`

Add this section so docs.rs builds with all features and uses the correct rustdoc args:

```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

Also add to `lib.rs`:

```rust
#![cfg_attr(docsrs, feature(doc_cfg))]
```

The `wasm` feature disables the query optimizer (`optimizer.rs`). With `all-features = true`, docs.rs builds with `wasm` enabled, so the optimizer's absence is documented accurately. Any items gated on `#[cfg(feature = "wasm")]` should carry `#[doc(cfg(feature = "wasm"))]` so docs.rs renders feature badges.

### Crate-level (`lib.rs`)

Add `//!` module doc:
- 3–5 sentence overview (bi-temporal, Datalog, embedded, single-file)
- Quick-start code example (matches README: open, transact, query, write tx, time-travel, prepared statement)

### Per-type doc requirements

| Type | Required |
|---|---|
| `Minigraf` | Struct overview + doc example on every public method |
| `OpenOptions` | Builder pattern overview + chained example |
| `WriteTransaction` | Commit/rollback semantics + example |
| `PreparedQuery` | Parse-once/execute-many explanation + bind-slot example |
| `Value` | Each variant + corresponding Datalog literal |
| `BindValue` | Each variant + which query position it targets |
| `Repl<'_>` | One-liner + `db.repl().run()` example |
| `QueryResult` | Explain result structure |
| `AsOf`, `ValidAt` | Temporal filter semantics (brief) |
| `EntityId` | One-liner (Uuid newtype for entity identity) |

### Intra-doc links

Use `[TypeName]` and `[TypeName::method]` cross-references throughout — e.g. `[WriteTransaction]` in `Minigraf::begin_write` docs, `[PreparedQuery::execute]` in `Minigraf::prepare` docs. `cargo doc` validates these resolve; broken links become warnings (or errors under `-D warnings`).

### Doctests

Every `/// # Examples` block must compile and run under `cargo test --doc`. Use `# use minigraf::*;` preamble lines (hidden from rendered output) to avoid boilerplate repetition. Examples that require a filesystem path should use `tempfile::tempdir()` or `Minigraf::in_memory()` so they're self-contained.

---

## Section 3: Clippy + Unwrap Audit

### Clippy

- `cargo clippy -- -D warnings` must pass clean
- Update `rust-clippy.yml` workflow to run on every PR (not just scheduled)

### Unwrap audit

| Location | Current | Action |
|---|---|---|
| `repl.rs` — `io::stdout().flush().unwrap()` | Panics on flush failure | Replace with `.ok()` |
| `db.rs` — 2 `expect()` in `register_aggregate` | Justified downcast guards with SAFETY comments | Keep — infallible by construction |
| `cache.rs`, `evaluator.rs` — RwLock `.read().unwrap()` / `.write().unwrap()` (~15 total) | No message | Replace with `.expect("lock poisoned")` — panicking is correct for an embedded DB in this state |

Tests and `src/main.rs` binary are exempt from the unwrap audit.

---

## Section 4: CI Matrix

Add a strategy matrix to `rust.yml` only:

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
runs-on: ${{ matrix.os }}
```

All other workflows (clippy, tarpaulin coverage, benchmarks, binary-size) stay Linux-only.

**Known risks on Windows:**
- WAL sidecar path construction (`.wal` suffix appended as a string) — verify `Path` usage is correct
- Windows file locking semantics for the WAL sidecar — may require a follow-up fix if tests fail

---

## Section 5: Publish Sequence

1. `cargo package --list` — verify no secrets or large test fixtures bundled
2. `cargo doc --no-deps` — zero warnings
3. CI green on all three OS
4. `cargo clippy -- -D warnings` — zero warnings
5. `cargo publish --dry-run` — catch Cargo.toml issues
6. Bump version: `0.18.0` → `0.19.0` (API narrowing removes public types — breaking change under semver, increment `0.x`)
7. `cargo publish`
8. `git tag -a v0.19.0 -m "Phase 7.9 complete — publish prep, API narrowing, crates.io publish"`
9. `git push origin v0.19.0`
10. Sync docs: `CLAUDE.md` (test count, status), `ROADMAP.md` (7.9 complete), `README.md`, `TEST_COVERAGE.md`, `CHANGELOG.md`
11. Update wiki pages: `Architecture.md` (API surface changes), `Datalog-Reference.md` (no changes needed)
12. Commit and push wiki repo

---

## Files Affected

| File | Change |
|---|---|
| `src/lib.rs` | Narrow module visibility, narrow re-exports, add `#![warn(missing_docs)]`, add crate-level doc |
| `src/db.rs` | Add `Minigraf::repl()`, remove `inner_fact_storage()`, add rustdoc to all public items |
| `src/repl.rs` | Change constructor to work via `Minigraf`, fix stdout flush unwrap |
| `src/graph/storage.rs` | `pub` → `pub(crate)` on types no longer in public API |
| `src/storage/mod.rs` | `pub` → `pub(crate)` on internal types |
| `src/storage/backend/file.rs` | `pub` → `pub(crate)` |
| `src/storage/cache.rs` | RwLock `.expect()` messages |
| `src/query/datalog/executor.rs` | `pub` → `pub(crate)` where applicable |
| `src/query/datalog/evaluator.rs` | RwLock `.expect()` messages |
| `src/wal.rs` | `pub` → `pub(crate)` |
| `.github/workflows/rust.yml` | Add OS matrix |
| `.github/workflows/rust-clippy.yml` | Add PR trigger |
| `Cargo.toml` | Bump to 0.19.0, add `[package.metadata.docs.rs]` section |

---

## Verification

```bash
# After API narrowing
cargo build                          # must compile
cargo test                           # must pass (780 tests)

# After rustdoc sweep
cargo doc --no-deps                  # zero warnings
cargo test --doc                     # all doc examples compile and run

# After clippy audit
cargo clippy -- -D warnings          # zero warnings

# Before publish
cargo package --list
cargo publish --dry-run

# Final
cargo test                           # full suite green (unit + integration + doc)
```
