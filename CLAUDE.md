# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable **bi-temporal graph database with Datalog queries** written in Rust. Designed as the embedded graph memory layer for AI agents, mobile apps, and the browser — built on the SQLite philosophy: embedded, single-file, reliable, with time travel.

See `ROADMAP.md` for planned work and current direction. See `CHANGELOG.md` for completed-release history.

## Core Philosophy - CRITICAL

**Before implementing ANY feature or change, you MUST assess it against the design philosophy in PHILOSOPHY.md.**

Minigraf follows the "SQLite for bi-temporal graph databases" philosophy:
- **Zero-configuration** - No setup, no config files, just works
- **Embedded-first** - Library not server, in-process execution
- **Single-file database** - One portable `.graph` file
- **Self-contained** - Minimal dependencies, small binary (~1.2MB budget as of #277; see PHILOSOPHY.md §4)
- **Cross-platform** - Native, WASM, mobile, embedded
- **Reliability over features** - Do less, do it perfectly
- **Bi-temporal first-class** - Time travel is a core feature, not addon
- **Datalog queries** - Simpler, more powerful for graphs than SQL/GQL
- **Stability** - Backwards compatibility, stable file format
- **Long-term support** - Decades-long commitment

### Philosophy Compliance Check

**CRITICAL INSTRUCTION**: When the user requests a feature or change, you MUST:

1. **First**, assess whether it aligns with the philosophy in PHILOSOPHY.md
2. **If it violates the philosophy**, WARN the user BEFORE implementing:
   - Explain which principle(s) it violates
   - Explain why it's problematic
   - Suggest alternatives that align with the philosophy
   - Ask for explicit confirmation to proceed despite the violation
3. **If it aligns**, proceed with implementation

**Examples of philosophy violations to warn about**:
- Adding client-server architecture (violates embedded-first)
- Requiring external services or complex setup (violates zero-configuration)
- Large dependencies that bloat binary size (violates self-contained)
- Features only useful for distributed systems (violates target use cases)
- Breaking changes to API or file format (violates stability)
- Complex features before basics are reliable (violates reliability-first)
- Multi-file storage (violates single-file philosophy)

**Your response format when detecting a violation**:
```
⚠️ PHILOSOPHY WARNING ⚠️

The requested feature/change may violate Minigraf's core philosophy:

**Violated Principle**: [principle name from PHILOSOPHY.md]

**Why this is problematic**: [explanation]

**Philosophy-aligned alternatives**:
- [alternative 1]
- [alternative 2]

Do you want to proceed anyway? This would be a deviation from the "SQLite for graph databases" philosophy.
```

See PHILOSOPHY.md for complete design principles and decision framework.

## Build and Run Commands

```bash
# Build
cargo build
cargo build --release

# Run the REPL
cargo run

# Run tests
cargo test

# Run specific test suite
cargo test --test bitemporal_test -- --nocapture
cargo test --test window_functions_test -- --nocapture
cargo test --test complex_queries_test -- --nocapture
cargo test --test recursive_rules_test -- --nocapture
cargo test --test concurrency_test

# Run examples
cargo run --example embedded
cargo run --example file_storage

# Run demo scripts
cargo run < demos/demo_commands.txt
cargo run < demos/demo_recursive.txt
cargo run < demos/demo_bitemporal.txt
cargo run < demos/demo_negation.txt
```

## Architecture

### Module Structure

1. **`src/graph/`** — EAV fact store with bi-temporal support
   - `types.rs`: `Fact`, `Value`, `EntityId`, `TxId`, `VALID_TIME_FOREVER`
   - `storage.rs`: `FactStorage` — in-memory store, `transact_batch`, `retract`, `get_facts_as_of`, `get_facts_valid_at`, `net_asserted_facts`, `get_facts_by_entity_attribute_indexed`

2. **`src/storage/`** — Persistence layer
   - `mod.rs`: `StorageBackend` trait, `LegacyHeaderV7` (v7 migration only), `CommittedReader` trait
   - `backend/file.rs`: Single `.graph` file backend (4KB pages, cross-platform)
   - `backend/memory.rs`: In-memory backend for testing
   - `backend/fault_inject.rs`: `FaultInjectingBackend` — injects I/O errors for durability tests (test builds only)
   - `fault.rs`: thread-local fault plan (test builds only, #390) — EIO, torn/short writes, sticky ENOSPC, failed fsync with or without lost writes, at the k-th write or sync of `FileBackend`, the WAL and `sync_parent_dir`; driven by `src/fault_matrix.rs`
   - `index.rs`: pending (uncheckpointed) EAVT/AEVT keys over UUIDs and strings, `encode_value`
   - `keys.rs`: byte-comparable v8 keys (FDB integers, value tags, `tx↓`, FOREVER), DICT keys, `MAX_VALUE_BYTES`, `MAX_IDENT_BYTES`
   - `node.rs`: prefix-compressed leaf and shortest-separator internal node codecs
   - `btree.rs`: On-disk B+tree over byte keys (`build_btree`, `cow_insert`, `LeafCursor` with `seek`, `prefix_scan`, `get`, `MutexStorageBackend`)
   - `dict.rs`: `DictReader` (id ↔ UUID/ident, tx timestamps, long values) and `Encoder` (checkpoint-time id assignment and key building)
   - `value_pages.rs`: append-only value pages for strings over 64 bytes
   - `reader.rs`: `OnDiskReader` — covering reads of committed facts (`CommittedReader`); `live_facts` decides net-assert on index keys (#379)
   - `verify.rs`: integrity walk for `Minigraf::verify` (tree order, index digests, DICT, page accounting) and the source choice for `rebuild_indexes` (#373)
   - `meta.rs` / `page.rs` / `freelist.rs`: meta pages A/B, the common page header and allocator, free-list chain
   - `cache.rs`: LRU page cache (`PageCache`, default 256 pages)
   - `dir_sync.rs`: `sync_parent_dir` — fsyncs the parent directory after creating the `.graph`/WAL or deleting the WAL (no-op off Unix)
   - `packed_pages.rs`: v7 fact pages, read only for migration
   - `persistent_facts.rs`: `PersistentFactStorage` — v8 save/load, auto-migration v7→v8

3. **`src/query/datalog/`** — Datalog engine
   - `parser.rs`: EDN/Datalog parser — `transact`, `retract`, `query`, `rule`, `:as-of`, `:valid-at`, `not`, `not-join`
   - `executor.rs`: Query executor — temporal filter (tx-time → net-assert → valid-time), not/not-join post-filter
   - `matcher.rs`: Pattern matching with variable unification; `edn_to_value`, `edn_to_entity_id`
   - `evaluator.rs`: `RecursiveEvaluator` (semi-naive), `StratifiedEvaluator`, `evaluate_not_join`
   - `stratification.rs`: `DependencyGraph`, `stratify()` — negative edges + cycle detection
   - `rules.rs`: `RuleRegistry` — thread-safe rule management
   - `types.rs`: `EdnValue`, `Pattern`, `DatalogQuery`, `AsOf`, `ValidAt`, `WhereClause` (incl. `Not`, `NotJoin`); `PseudoAttr` enum, `AttributeSpec` wrapper
   - `optimizer.rs`: Selectivity-based join reordering; disabled under `wasm` feature
   - `prepared.rs`: `BindValue`, `PreparedQuery` — parse-once/execute-many with named `$slot` bind slots; `prepare_query`, `substitute`
   - `magic_sets.rs`: Magic-sets rewriting for demand-driven recursive rule evaluation (not applied to rules with `not`/`not-join`)

4. **`src/temporal.rs`** — UTC-only timestamp parsing (avoids chrono CVE GHSA-wcg3-cvx6-7396)

5. **`src/repl.rs`** — Interactive REPL; TTY-aware (suppresses prompts/banner for piped input)

6. **`src/db.rs`** — Public API: `Minigraf::open/execute/query/fact_log/prepare/begin_write/checkpoint/save/verify/rebuild_indexes`, `WriteTransaction`, `IntegrityReport`, `OpenOptions::page_cache_size`, `OpenOptions::read_only` (#429: shared lock, nothing written, writes are `API-014`)
   - `src/cursor.rs`: `Cursor` / `Batch` — owned, `Send` cursor returned by `Minigraf::query` and `PreparedQuery::query` (#432); answer computed at open today, shaped for streaming operators
   - `src/fact_log.rs`: `FactLog` / `FactFilter` / `FactRecord` / `FactOrder` — every fact version streamed by `Minigraf::fact_log` (#430): tx order (multi-pass, bounded window) or storage order, key-level filters; an open log pins the committed generation, deferring checkpoints (`API-013`)
   - `src/log_writer.rs`: `LogWriter` — builds a new file from `FactRecord`s with their tx and valid-time kept (#431): tx-ordered appends with holes (`API-015`/`API-016`), batched copy-on-write checkpoints at tx boundaries, no WAL, built at `<path>.partial` and renamed by `finish` (`STG-043` if the target exists)

7. **`src/wal.rs`** — Fact-level sidecar WAL, CRC32-protected entries, crash recovery. Like `storage::verify` and `storage::dir_sync`, not compiled for wasm32 (no file backend there)

8. **`src/error.rs`** — Structured error codes: `MinigrafError`, `ErrorCategory`, `ErrorCode` registry (PRS/QRY/STG/WAL/API/INT codes); must match `docs/ERROR_REFERENCE.md`. `MinigrafError::invalid_argument` (API-017) and `MinigrafError::closed` (API-018) exist for language bindings

9. **`src/browser/`** — Browser WASM backend (`browser` feature): `buffer.rs` (`BrowserBufferBackend`, in-memory pages with dirty tracking), `indexeddb.rs` (IndexedDB persistence); `BrowserDb.query` returns a `BrowserCursor` (JSON row batches). Tests here must be `#[wasm_bindgen_test]` (the module only builds for wasm32); `.github/workflows/wasm.yml` runs them in headless browsers, runs the WASI tests under Wasmtime, and lints both wasm targets. New file-backed integration test files need `#![cfg(not(target_arch = "wasm32"))]`

### Data Model

```rust
struct Fact {
    entity: EntityId,  // Uuid
    attribute: String, // e.g. ":person/name"
    value: Value,
    tx_id: TxId,       // Unix ms timestamp
    tx_count: u64,     // Monotonic counter — used by :as-of N
    valid_from: i64,   // Unix ms; defaults to tx_id
    valid_to: i64,     // Unix ms; i64::MAX = forever (VALID_TIME_FOREVER)
    asserted: bool,
}

enum Value { String(String), Integer(i64), Float(f64), Boolean(bool),
             Ref(Uuid), Keyword(String), Null }
```

**Important**: `tx_count` (sequential 1, 2, 3…) is what `:as-of N` compares against. The REPL displays `tx_id` (Unix ms). A single `(transact [...])` command increments `tx_count` once regardless of how many facts it contains (`transact_batch`).

### File Format (v8)

```
Page 0, 1: Meta pages A/B (alternating commits: odd generations in page 0, even in 1).
           Magic "MGRF"/"META", generation, page_count, five tree roots (EAVT, AEVT,
           AVET, VAET, DICT), free-list head, next_eid/next_iid, required_features;
           CRC over the whole page. The only commit point.
Page 2+:   Any mix of B+tree leaf/internal pages (0x61/0x62), value pages (0x51) and
           free-list pages (0x81). Every one has a 24-byte header (type, count, CRC32,
           page id, generation), verified on load.
Sidecar:   <db>.wal — v2 header records the base generation; CRC32-protected entries;
           replayed on open; deleted on checkpoint
```

Covering indexes: every index entry is a whole fact as a byte-comparable key
(`e a v tx↓ vf vt op` in EAVT order), with entities and idents as sequential ids from
the DICT tree, assigned at checkpoint. Strings over 64 bytes live once in value pages.
Committed scans return facts in id order. A checkpoint is copy-on-write: it rewrites
only touched leaves and their paths (`btree::cow_insert`), takes pages from the free
list lazily and pushes freed ones (`PageAllocator::from_chain` / `finish_free_list`),
so its cost follows the change. It writes only pages the active meta does not
reference, syncs, then writes the other meta page and syncs, so a crash at any point
keeps the previous checkpoint.
Auto-migrates v7 → v8 on open (spec §9, with a backup meta page). v1–v6 are rejected
(STG-028). Design: `docs/superpowers/specs/2026-10-05-v8-storage-format-design.md`.

## Test Coverage

**1424 tests** (1414 passing, 10 ignored; unit + integration + doc).
See `docs/TEST_COVERAGE.md` for the full per-file breakdown.

**Testing conventions** — see the Testing Conventions section below before writing any tests.

## Current Maintainer Context

**v2.0.4 released** (2026-10-08) — `BrowserDb` IndexedDB flush returns an error for an unreadable dirty page (#470); golden-file v7 corpus (#391) and SIGKILL committed-data crash test (#384). **v2.0.3** (2026-10-06) — data-integrity patch under the support policy: indexed queries with >65,535 uncheckpointed facts (#445), tx counter rewound on reopen next to an already-checkpointed WAL (#447). **v2.0.2** (2026-09-27) — last planned v2.x feature/fix release: query fixes (#297, #405, #407, #410), directory fsync (#389), point-query/attribute-scan/checkpoint perf mitigations (#323, #381, #315), CI policy checks (#395), support policy and tiers (#397, #399, #400). **v2.0.1** (2026-09-25) fixed index rebuild on open (#370). **v2.0.0** introduced kernel file locking (#317, #304), structured runtime error codes (#277), `OpenOptions` `#[non_exhaustive]`, and an MSRV of Rust 1.89. See `CHANGELOG.md` for the full rationale and release history.

Current plan (see `ROADMAP.md` and the milestones):
- **v2.x** (`main`, format v7) now gets only data-integrity and security fixes as patch releases (v2.0.3 is the first); v2.0.2 was the last planned v2.x release.
- **v3.0.0** (format v8 and data integrity, tracker #383) is developed on the long-lived `v3` branch. All v3 PRs target `v3`; `main` stays untouched. CI runs on PRs to `v3` the same as on PRs to `main`. Merge `main` into `v3` regularly. At the first v3 release, the current `main` is copied to `v2` and `v3` becomes the new `main`.
- **v3.1.0** holds new features (query profiler #185, `:limit` #306/#310, lag/lead #182, UniFFI additions).
- Known v2.x data issue: same-transaction multi-valued facts (#371). Fix ships in v3.0.0. All v2.x known issues are listed in pinned issue #421 (`known-issue` label); `data-integrity`/`corruption`/`durability` issues stay open until the fix is in a published release (CONTRIBUTING.md).
- Support policy and tiers: PHILOSOPHY.md §7 and §10. v2.x gets data-integrity and security fixes for 12 months after v3.0.0. Tier 1 = Rust + Python; other bindings are Tier 2.

## Testing Conventions

**Never use `{:?}` debug format of `Result`, `Fact`, `Value`, `EdnValue`, or any type that may transitively contain `Uuid` in `assert!`/`assert_eq!` message strings.**

CodeQL flags this as `rust/cleartext-logging`. It is a false positive in tests, but it pollutes the security scan and blocks CI.

```rust
// BAD — triggers CodeQL:
assert!(result.is_ok(), "parse failed: {:?}", result);

// GOOD — plain string message:
assert!(result.is_ok(), "parse failed");

// GOOD — use unwrap/expect instead:
result.unwrap();
result.expect("parse failed");

// GOOD — assert on count/bool only:
assert_eq!(results.len(), 3, "expected 3 results");
```

Applies to all `#[cfg(test)]` modules and all `tests/*.rs` files.

## Important Reminders

1. **Always use git worktrees for new features/bugfixes** — Never make changes directly on main. Create an isolated worktree (in `.worktrees/` directory) using the `using-git-worktrees` skill before implementing any feature or fixing any issue.
2. **Datalog is the query language** — no other query language
2. **Bi-temporal is first-class** — not an afterthought
3. **Single file is sacred** — never break this
4. **Simplicity over features** — do less, do it perfectly
5. **Test everything** — no untested code
6. **Think SQLite** — would SQLite do this?
7. **Long-term vision** — building for decades
8. **Keep documentation synchronized** — when a release or planned-work item changes, update and cross-check ALL of: `CLAUDE.md` (status line, test count), `ROADMAP.md`, `README.md`, `docs/TEST_COVERAGE.md`, `CHANGELOG.md`. No doc should contradict another.
   Also update the docs site, [project-minigraf/minigraf-docs](https://github.com/project-minigraf/minigraf-docs) (clone at `../minigraf-docs`): `content/architecture.md` (module/format/model changes), `content/datalog-reference.md` (new syntax), `content/comparison.md` (feature matrix), `content/use-cases.md` (deployment targets). It documents every release at once: wrap text that applies only from or until a version in `<!-- @since vX.Y.Z -->` / `<!-- @until vX.Y.Z -->` … `<!-- @end -->` rather than rewriting shared text, and add each release to its `site.toml` (`CHANGELOG.md` and `docs/ERROR_REFERENCE.md` are read from this repo per tag, never copied). Open a PR there; its CI checks every link in every version and deploys `main`. The GitHub wiki is retired: its pages only link to the site.
9. **Tag every version bump** — after the final doc-sync commit: `git tag -a v<x.y.z> -m "<release> — <summary>"` then `git push origin v<x.y.z>`.

---

*When in doubt, refer to PHILOSOPHY.md and ROADMAP.md. The goal is not to be the most feature-complete graph database. The goal is to be the one that's always there when you need it, works reliably, and never gets in your way.*

*Be boring. Be reliable. Be Minigraf.*
