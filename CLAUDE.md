# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minigraf is a tiny, portable **bi-temporal graph database with Datalog queries** written in Rust. Designed as the embedded graph memory layer for AI agents, mobile apps, and the browser — built on the SQLite philosophy: embedded, single-file, reliable, with time travel.

See `ROADMAP.md` for the current phase, full plan, and publish gate. See `CHANGELOG.md` for per-phase implementation history.

## Core Philosophy - CRITICAL

**Before implementing ANY feature or change, you MUST assess it against the design philosophy in PHILOSOPHY.md.**

Minigraf follows the "SQLite for bi-temporal graph databases" philosophy:
- **Zero-configuration** - No setup, no config files, just works
- **Embedded-first** - Library not server, in-process execution
- **Single-file database** - One portable `.graph` file
- **Self-contained** - Minimal dependencies, small binary (<1MB goal)
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
   - `storage.rs`: `FactStorage` — in-memory store, `transact_batch`, `retract`, `get_facts_as_of`, `get_facts_valid_at`, `net_asserted_facts`

2. **`src/storage/`** — Persistence layer
   - `mod.rs`: `StorageBackend` trait, `FileHeader` v6 (80 bytes), `CommittedFactReader` / `CommittedIndexReader` traits
   - `backend/file.rs`: Single `.graph` file backend (4KB pages, cross-platform)
   - `backend/memory.rs`: In-memory backend for testing
   - `index.rs`: EAVT / AEVT / AVET / VAET key types, `FactRef`, `encode_value`
   - `btree_v6.rs`: On-disk B+tree (`build_btree`, `OnDiskIndexReader`, `MutexStorageBackend`)
   - `btree.rs`: Legacy v5 B+tree (migration only)
   - `cache.rs`: LRU page cache (`PageCache`, default 256 pages)
   - `packed_pages.rs`: Packed fact pages (~25 facts/4KB page), `MAX_FACT_BYTES`
   - `persistent_facts.rs`: `PersistentFactStorage` — v6 save/load, auto-migration v1–v5→v6

3. **`src/query/datalog/`** — Datalog engine
   - `parser.rs`: EDN/Datalog parser — `transact`, `retract`, `query`, `rule`, `:as-of`, `:valid-at`, `not`, `not-join`
   - `executor.rs`: Query executor — temporal filter (tx-time → net-assert → valid-time), not/not-join post-filter
   - `matcher.rs`: Pattern matching with variable unification; `edn_to_value`, `edn_to_entity_id`
   - `evaluator.rs`: `RecursiveEvaluator` (semi-naive), `StratifiedEvaluator`, `evaluate_not_join`
   - `stratification.rs`: `DependencyGraph`, `stratify()` — negative edges + cycle detection
   - `rules.rs`: `RuleRegistry` — thread-safe rule management
   - `types.rs`: `EdnValue`, `Pattern`, `DatalogQuery`, `AsOf`, `ValidAt`, `WhereClause` (incl. `Not`, `NotJoin`); `PseudoAttr` enum, `AttributeSpec` wrapper
   - `optimizer.rs`: Selectivity-based join reordering; disabled under `wasm` feature

4. **`src/temporal.rs`** — UTC-only timestamp parsing (avoids chrono CVE GHSA-wcg3-cvx6-7396)

5. **`src/repl.rs`** — Interactive REPL; TTY-aware (suppresses prompts/banner for piped input)

6. **`src/db.rs`** — Public API: `Minigraf::open/execute/begin_write/checkpoint/save`, `WriteTransaction`, `OpenOptions::page_cache_size`

7. **`src/wal.rs`** — Fact-level sidecar WAL, CRC32-protected entries, crash recovery

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

### File Format (v6)

```
Page 0:  Header (80 bytes) — magic "MGRF", version, page/fact counts,
         B+tree root pages (eavt/aevt/avet/vaet), index_checksum, fact_page_count
Page 1+: Packed fact pages (postcard-encoded, ~25 facts/4KB page)
After:   On-disk B+tree index pages (one node per 4KB page)
Sidecar: <db>.wal — CRC32-protected WAL entries; replayed on open; deleted on checkpoint
```

Auto-migrates v1/v2/v3/v4/v5 → v6 on open/checkpoint.

## Test Coverage

**707 tests passing** (unit + integration + doc).
See `TEST_COVERAGE.md` for the full per-file breakdown.

**Testing conventions** — see the Testing Conventions section below before writing any tests.

## Key Files for the Next Phase

See `ROADMAP.md` for the current next phase spec, including the relevant files and implementation details.

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
8. **Sync all docs at phase completion** — when a phase is marked complete, update and cross-check ALL of: `CLAUDE.md` (status line, test count), `ROADMAP.md`, `README.md`, `TEST_COVERAGE.md`, `CHANGELOG.md`. No doc should contradict another.
   Also update affected wiki pages in `.wiki/`: `Architecture.md` (module/format/model changes), `Datalog-Reference.md` (new syntax), `Comparison.md` (feature matrix), `Use-Cases.md` (deployment targets). Commit and push the wiki repo separately (`cd .wiki && git add -A && git commit -m "..." && git push`).
9. **Tag every version bump** — after the final doc-sync commit: `git tag -a v<x.y.z> -m "<phase> complete — <summary>"` then `git push origin v<x.y.z>`.

---

*When in doubt, refer to PHILOSOPHY.md and ROADMAP.md. The goal is not to be the most feature-complete graph database. The goal is to be the one that's always there when you need it, works reliably, and never gets in your way.*

*Be boring. Be reliable. Be Minigraf.*
