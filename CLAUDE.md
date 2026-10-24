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

# Kani proofs of the encoders (#627); ~1 GB. Never add decoder/string harnesses
# without a bench-machine run first: they went past 27 GB without finishing
cargo kani --lib -j --output-format terse -Z unstable-options --harness-timeout 10m

# Run examples
cargo run --example embedded
cargo run --example file_storage

# Performance envelope (#394): latency/RSS at 100K–10M facts; opt-level 3 profile
cargo run --profile envelope --example perf_envelope -- --scales 100k,1m

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
   - `storage.rs`: `FactStorage` — in-memory store, `snapshot()` → `Snapshot` (committed reader + pin + pending prefix, and a `WriteTransaction`'s staged facts read with the pending ones as one store, #550; every query/cursor/log read goes through one), `transact_batch`, `retract`, `get_facts_as_of`, `get_facts_valid_at`, `net_asserted_facts`, `get_facts_by_entity_attribute_indexed`

2. **`src/storage/`** — Persistence layer
   - `mod.rs`: `StorageBackend` trait, `LegacyHeaderV7` (v7 migration only), `CommittedReader` trait
   - `backend/file.rs`: Single `.graph` file backend (4KB pages, cross-platform)
   - `backend/memory.rs`: In-memory backend for testing
   - `backend/fault_inject.rs`: `FaultInjectingBackend` — injects I/O errors for durability tests (test builds only)
   - `fault.rs`: thread-local fault plan (test builds only, #390) — EIO, torn/short writes, sticky ENOSPC, failed fsync with or without lost writes, at the k-th write or sync of `FileBackend`, the WAL and `sync_parent_dir`; driven by `src/fault_matrix.rs`
   - `index.rs`: pending (uncheckpointed) EAVT/AEVT keys over UUIDs and strings, `encode_value`; `index/verification.rs`: Kani proofs of `encode_value` (`#[cfg(kani)]`, #627)
   - `keys.rs`: byte-comparable v8 keys (FDB integers, value tags, `tx↓`, FOREVER), DICT keys, `MAX_VALUE_BYTES`, `MAX_IDENT_BYTES`; `keys/verification.rs`: Kani proofs that the key encoders preserve order (`#[cfg(kani)]`, #627; `kani.yml` runs them on encoder PRs and nightly)
   - `node.rs`: prefix-compressed leaf and shortest-separator internal node codecs
   - `btree.rs`: On-disk B+tree over byte keys (`build_btree`, `cow_update` (copy-on-write inserts and deletes; an emptied leaf or node leaves its parent, an emptied tree is root 0; #550) and `cow_insert`, `LeafCursor` with `seek`, `prefix_scan`, `get`, `MutexStorageBackend`)
   - `dict.rs`: `DictReader` (id ↔ UUID/ident, tx timestamps, long values) and `Encoder` (checkpoint-time id assignment and key building)
   - `value_pages.rs`: append-only value pages for strings over 64 bytes
   - `reader.rs`: `OnDiskReader` — covering reads of committed facts (`CommittedReader`, `KeySource`); `live_facts` and the resumable `live_keys` decide net-assert on index keys (#379) among the versions a `TxBound` keeps; `tx_order` (cached per reader), `last_tx_at`, `tx_runs` over the DICT's transactions (#551)
   - `verify.rs`: integrity walk for `Minigraf::verify` (tree order, index digests, DICT, page accounting) and the source choice for `rebuild_indexes` (#373)
   - `meta.rs` / `page.rs` / `freelist.rs`: meta pages A/B, the common page header and allocator (skips quarantined ids), free-list chain
   - `pins.rs`: `Pins` / `GenerationPin` — generations held by open snapshots and the quarantine of pages checkpoints freed after them (#543)
   - `cache.rs`: LRU page cache (`PageCache`, default 256 pages)
   - `dir_sync.rs`: `sync_parent_dir` — fsyncs the parent directory after creating the `.graph`/WAL or deleting the WAL (no-op off Unix)
   - `packed_pages.rs`: v7 fact pages, read only for migration
   - `persistent_facts.rs`: `PersistentFactStorage` — v8 save/load, auto-migration v7→v8
   - `persistent_facts/large_tx.rs`: large transactions (#550): a transaction whose staged facts would pass `OpenOptions::transaction_chunk_facts` (default 65,536, clamped to 1..=1,000,000) on a file database (`Minigraf::execute` of a larger statement, or a `WriteTransaction` statement that takes it over) becomes large (WAL guard: the WAL exists, header only, before any page; cache generation bound `g + 1`; next `tx_count` reserved, allocated at the commit point; `tx_id` fixed; the pending store written as the first chunk, so in-transaction reads see the working trees alone), writes each chunk into copy-on-write working trees (per-chunk `Encoder` over the working DICT, no `SharedDictCache`; a fact the statement wrote kept once, another window of its triple `API-011`; an earlier statement's keys of a triple it writes deleted, and the triple recorded in the statement's claims tree; replaced pages: written since the statement's base released, written before it held, `g`'s freed), and commits as a checkpoint (meta `g + 1`, WAL deleted). In a `WriteTransaction` each statement has a base (roots, counters, `freed` length, `PageAllocator::begin_base` ids): a rejected statement returns to it and the transaction stays usable; staged statements are written as one chunk when it becomes large (their windows checked at its `tx_id` first). `Minigraf::execute`'s statement error rolls back (cache bound `g`, written pages evicted, handle dirty so the next checkpoint writes a meta before deleting the WAL); an I/O failure also marks the handle failed (`STG-046`). A WAL found at open also marks the handle dirty

3. **`src/query/datalog/`** — Datalog engine
   - `parser.rs`: EDN/Datalog parser — `transact`, `retract`, `query`, `rule`, `:as-of`, `:valid-at`, `not`, `not-join`; `resolve_pseudo_sources` names each per-fact pseudo-attribute's source (nearest earlier same-entity pattern, lexical scope) and its hidden metadata variable `?#<n>.<attr>`, else PRS-081 (#594)
   - `parser/statement.rs`: `transact`/`retract` read one fact at a time (#550): `kind_of` dispatches on the first two tokens; one pass keeps today's error order (phases 1 tokens … 6 valid windows, the earliest phase's first error, phase 3 in today's check order); `read_statement` (pass 1) keeps at most `TX_CHUNK_FACTS` materialised facts and the offset after the last, and returns phase 5–6 errors from `Statement::facts`/`Statement::chunks` (one `Chunk` at a time; pass 2 from that offset for a larger statement); `set_tx_chunk_facts` for tests (`minigraf::__tx_chunk_facts` and `__statement_facts_high_water` under `test-hooks`). The `Parser` reads tokens on demand from a `Lexer` (`keep` off checks structure without holding it); `parse_transact`/`parse_retract` remain under `cfg(test)` for the differential tests
   - `parser/verifier.rs`: test-only (not wasm32) bounded-exhaustive check of the parser against `tests/grammar/grammar.pest` (#643): every 1–3 character string (`parse_edn`) and every balanced token filler up to `MINIGRAF_GRAMMAR_BOUND` (default 3) in 15 command templates (`parse_datalog_command`); pest rejects ⇒ parser rejects, pest accepts ⇒ parser accepts or raises a `SEMANTIC` code; today's drift is `KNOWN_DRIFT`, one entry per cause, removed by its fix (empty: #649, #650, #651 fixed)
   - `executor.rs`: `DatalogExecutor` — `transact`/`retract`/`rule` commands, and every query on the engine (`open_query`, `open_rule_query`; planner errors after the pseudo-attribute check, then every predicate the query and its rules name, `QRY-007`); its own query path is gone (#551)
   - `matcher.rs`: `edn_to_value`, `edn_to_entity_id`
   - `aggregate.rs`: `Agg` (one aggregate of one group) and `Running` (a window's running value), used by the engine; exact sums (`i128` integers saturated once, fixed-point floats rounded once), `value_order` (one total order of values for window sorts, peers, `rank` and `min`/`max`), `named_vars` (a window's tie-break variables), `windows` (each window's value per row: partitions, sort, peers) (#547)
   - `evaluator.rs`: `rule_invocation_to_pattern`, the rule limits and their errors
   - `stratification.rs`: `DependencyGraph`, `stratify()` — negative edges + cycle detection; a positive edge to a rule with `not`/`not-join`/`or`/`or-join` also raises the caller, off a cycle (#628)
   - `rules.rs`: `RuleRegistry` — thread-safe rule management
   - `types.rs`: `EdnValue`, `Pattern`, `DatalogQuery`, `AsOf`, `ValidAt`, `WhereClause` (incl. `Not`, `NotJoin`); `PseudoAttr` enum, `AttributeSpec` wrapper
   - `optimizer.rs`: Selectivity-based join reordering on every build (the `wasm` feature is a deprecated no-op). `reorder()` gates every reordering site; tests turn it off per thread (`set_reorder`, `minigraf::__reorder` under `test-hooks`, #596); `plan_seeded` orders a body with its imported variables counted as bound, `plan_after` orders a rule body's clauses after its delta, each joining on a variable bound before it where one can (#694), and `subplans()` lets tests force the engine's negation bodies and `or` branches hashed or correlated (`set_subplans`, `minigraf::__subplans`, #546)
   - `prepared.rs`: `BindValue`, `PreparedQuery` — parse-once/execute-many with named `$slot` bind slots; `prepare_query`, `substitute`
   - `magic_sets.rs`: Magic-sets rewriting for demand-driven recursive rule evaluation (not applied to rules with `not`/`not-join`)

3a. **`src/query/engine/`** — Streaming query engine (#473), runs every query (#551): `plan.rs` (the operator tree of every query, or the executor's error code for it: patterns, expression clauses, `not`/`not-join` and `or`/`or-join`, and rule invocations, read as patterns of their relations (`plan_rules`, #548; a magic-set head constant is `HeadArg::Const`), in `optimizer::plan` order, then the `or`s in text order, then the deferred `not`s; an `or` branch or a `not` body is planned as a `:where` is; a clause list with no pattern first starts from `Seed`; a constant no cell equals is `Term::Never`, a scan that reads nothing; `:as-of` resolved when the query opens (`Snapshot::tx_bound`: a counter bound when `tx_id` rises with `tx_count`, else `TxBound::Timestamp`: committed `tx_count` ranges and pending `tx_id <= t`); a per-fact pseudo-attribute pattern reads the metadata column its source's scan fills (the hidden variable the parser named, #594), and `:db/valid-at` joins as `[e _ _]`; plain `:find` variables, aggregates with `:with`, and window functions, over an aggregate's groups too), `scan.rs` (one pattern over committed key ranges via `CommittedReader::key_source`/`live_keys`, AVET ranges for cross-kind constants, then the pending range by whole runs; one entity per probe for a join), `join.rs` (`IndexNestedLoopJoin` when the pattern's entity is bound, or, under a constant attribute and entity not constant, its value (#721: per outer row the AVET ranges of the values that join-equal it and the attribute's pending facts with its key, read once per query in `IdSpace::probes`; an entity cell needs the keyword naming it, known only where the query made the entity from a keyword (rule heads, magic seeds), else the join becomes the planner's `HashJoin`); `HashJoin` otherwise, and on a relation; `JoinKey`: floats by IEEE `==`, a keyword equals a ref to the entity it names), `filter.rs` (`Filter`/`Bind` and pseudo-attribute steps over batches; a `Bind` on a bound slot keeps the rows where the result is join-equal, #596), `anti.rs` (`AntiJoin`, #546: a body with the same rows for every outer row runs once into a hash set of its imported-variable keys; any other body, or one whose first pattern's entity is imported, runs per outer row from `seed.rs`'s one-row `Seed`, via `Operator::rewind`; never deduplicates), `aggregate.rs` (`Aggregate`, #547: a hash table of groups keyed on cells as `Value`'s `Ord` compares values, one entry when ungrouped; `count`/`count-distinct` on cells, other aggregates on values via `datalog::aggregate::Agg`; groups sorted by value and finished in that order; a float group shows its least value by `value_order`), `window.rs` (`Window`, #547: holds each input row's passed-on cells and its named variables' values, then computes every window with `datalog::aggregate::windows`, the executor's code, and returns the rows in input order), `union.rs`/`distinct.rs` (`Union` and `Distinct`, #546: each `or` branch planned as a `not` body is, hashed into a table or run per outer row; then one row per distinct binding, as the executor's `Value`s compare), `cell.rs`/`batch.rs` (id cells in columnar batches of 1,024), `ids.rs` (DICT and provisional ids, `keyword_eid`), `project.rs` (ids → values for returned rows only), `relation.rs`/`recursive.rs` (#548: rule queries; a relation per evaluated predicate holds its derived rows, a set by `encode_value` identity, appended per round so `Old`/`Delta`/`Full` are ranges; `Recursive` runs each stratum's semi-naive rounds, one run per same-stratum top-level invocation, a rule reading its stratum otherwise run in full every round; scans read derived rows after committed and pending ones, with `nil` metadata), `meter.rs` (#549: one `Meter` per query, shared by every holder of state through a `Held` — hash join build sides, `not`/`or` tables, distinct sets, aggregate groups, window rows, rule relations and their temporaries, `IdSpace`; each entry charged by a per-entry estimate before it is inserted, containers grown with `try_reserve`, released on `rewind`/`close`/drop; over `OpenOptions::query_memory_limit`, or a refused allocation, is `QRY-012`; `Stream::close` drops the operator tree, keeping its counters). `mod.rs`: `Operator` (`next_batch`/`rewind`/`close`/`counters`), `Stream` (the operator tree over an owned `Snapshot`, pulled one batch at a time). `DatalogExecutor::open` / `execute_query` run every query on it: a cursor gets the `Stream` (`Answer::Stream`), `execute` drains it.

4. **`src/temporal.rs`** — UTC-only timestamp parsing (avoids chrono CVE GHSA-wcg3-cvx6-7396)

5. **`src/repl.rs`** — Interactive REPL; TTY-aware (suppresses prompts/banner for piped input)

6. **`src/db.rs`** — Public API: `Minigraf::open/execute/query/fact_log/prepare/begin_write/checkpoint/save/verify/rebuild_indexes`, `WriteTransaction`, `IntegrityReport`, `OpenOptions::page_cache_size`, `OpenOptions::read_only` (#429: shared lock, nothing written, writes are `API-014`), `OpenOptions::query_memory_limit` (#549: `QRY-012`, off by default), `OpenOptions::transaction_chunk_facts` (#550)
   - `src/cursor.rs`: `Cursor` / `Batch` — owned, `Send` cursor returned by `Minigraf::query` and `PreparedQuery::query` (#432); pulls engine queries one internal batch at a time (`close`/drop stops the work; a read error after some rows comes on the next call), other queries computed at open; holds a `Snapshot` (one state of the store, its committed generation pinned, #543) until dropped
   - `src/fact_log.rs`: `FactLog` / `FactFilter` / `FactRecord` / `FactOrder` — every fact version streamed by `Minigraf::fact_log` (#430): tx order (multi-pass, bounded window) or storage order, key-level filters; an open log pins the committed generation (`storage::pins`), so checkpoints run but do not reuse its pages (`API-013` deprecated, #543)
   - `src/log_writer.rs`: `LogWriter` — builds a new file from `FactRecord`s with their tx and valid-time kept (#431): tx-ordered appends with holes (`API-015`/`API-016`), batched copy-on-write checkpoints at tx boundaries, no WAL, built at `<path>.partial` and renamed by `finish` (`STG-043` if the target exists)

7. **`src/wal.rs`** — Fact-level sidecar WAL, CRC32-protected entries, crash recovery. Like `storage::verify` and `storage::dir_sync`, not compiled for wasm32 (no file backend there)

8. **`src/error.rs`** — Structured error codes: `MinigrafError`, `ErrorCategory`, `ErrorCode` registry (PRS/QRY/STG/WAL/API/INT codes); must match `docs/ERROR_REFERENCE.md`. `MinigrafError::invalid_argument` (API-017) and `MinigrafError::closed` (API-018) exist for language bindings

9. **`src/browser/`** — Browser WASM backend (`browser` feature): `buffer.rs` (`BrowserBufferBackend`, in-memory pages with dirty tracking; free pages a pinned generation may read are withheld from release, `pins::split_released`), `indexeddb.rs` (IndexedDB persistence); `BrowserDb.query` returns a `BrowserCursor` (JSON row batches). Tests here must be `#[wasm_bindgen_test]` (the module only builds for wasm32); `.github/workflows/wasm.yml` runs them in headless browsers, runs the WASI tests under Wasmtime, and lints both wasm targets. New file-backed integration test files need `#![cfg(not(target_arch = "wasm32"))]`

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

**1723 tests** (1710 passing, 13 ignored; unit + integration + doc). Engine answers once checked by the executor are frozen in `tests/answers/` (#551).
See `docs/TEST_COVERAGE.md` for the full per-file breakdown.

**Testing conventions** — see the Testing Conventions section below before writing any tests.

## Current Maintainer Context

**v2.0.5 released** (2026-10-11) — short-file open refused with STG-045 (#506), commit after a torn WAL entry no longer lost, WAL-007 (#513); backported rule-query `max_results` (#528), all-checkpointed WAL removal (#457) and `usize::MAX` close-time save (#476); weekly soak test (#392), real coverage gates (#396). **v2.0.4** (2026-10-08) — `BrowserDb` IndexedDB flush returns an error for an unreadable dirty page (#470); golden-file v7 corpus (#391) and SIGKILL committed-data crash test (#384). **v2.0.3** (2026-10-06) — data-integrity patch under the support policy: indexed queries with >65,535 uncheckpointed facts (#445), tx counter rewound on reopen next to an already-checkpointed WAL (#447). **v2.0.2** (2026-09-27) — last planned v2.x feature/fix release: query fixes (#297, #405, #407, #410), directory fsync (#389), point-query/attribute-scan/checkpoint perf mitigations (#323, #381, #315), CI policy checks (#395), support policy and tiers (#397, #399, #400). **v2.0.1** (2026-09-25) fixed index rebuild on open (#370). **v2.0.0** introduced kernel file locking (#317, #304), structured runtime error codes (#277), `OpenOptions` `#[non_exhaustive]`, and an MSRV of Rust 1.89. See `CHANGELOG.md` for the full rationale and release history.

Current plan (see `ROADMAP.md` and the milestones):
- **v2.x** (`main`, format v7) now gets only data-integrity and security fixes as patch releases (v2.0.3 is the first); v2.0.2 was the last planned v2.x release.
- **v3.0.0** (format v8 and data integrity, plus `:limit` #306, query profiler #185, set membership predicates #316 and UniFFI `PreparedQuery` #181, UDF registration in every binding #180; tracker #383) is developed on the long-lived `v3` branch. All v3 PRs target `v3`; `main` stays untouched. CI runs on PRs to `v3` the same as on PRs to `main`. Do not merge `main` into `v3` routinely: `main` only gets backports of fixes already on `v3`. Shared CI and workflow changes land on both branches as separate PRs. Just before the v3.0.0 cut, merge `main` into `v3` once (merge commit; source conflicts resolved to `v3`, `main`'s v2.0.x CHANGELOG sections brought over) so the cut is a fast-forward. At the first v3 release, the current `main` is copied to `v2` and `v3` becomes the new `main`.
- **v3.next** (the release after v3.0.0) holds new features (lag/lead #182, sliding row frames #183, rule persistence #241, OPFS `BrowserDb` #441).
- Known v2.x data issue: same-transaction multi-valued facts (#371). Fix ships in v3.0.0. All v2.x known issues are listed in pinned issue #421 (`known-issue` label); `data-integrity`/`corruption`/`durability` issues fixed in a v2.x patch stay open until that patch is published; issues fixed on the unreleased `v3` branch close on merge (closing keywords do not fire on `v3`, so close them with `gh issue close`) and stay listed in #421 (CONTRIBUTING.md).
- Support policy and tiers: PHILOSOPHY.md §7 and §10. v2.x gets data-integrity and security fixes for 12 months after v3.0.0. Tier 1 = Rust + Python + browser WASM (from v3.0.0); other bindings are Tier 2.

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
