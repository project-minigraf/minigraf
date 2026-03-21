# Phase 4: Bi-temporal Support — Design Spec

**Date**: 2026-03-21
**Status**: Approved
**Phase**: 4 (follows Phase 3 Datalog Core, v0.3.0)
**Deliverable**: v0.4.0

---

## Overview

Phase 4 adds full bi-temporal support to Minigraf. Every fact carries two independent time dimensions:

- **Transaction time** — when the fact was recorded in the database (`tx_id`, `tx_count`)
- **Valid time** — when the fact was true in the real world (`valid_from`, `valid_to`)

This enables time travel queries ("what did the database look like at tx 50?"), corrections without data loss ("Alice was actually employed Jan–Jun 2023, even though we only recorded it in July"), and audit trails.

---

## Approach

**Extend `Fact` in place** (Approach A). Add `valid_from`, `valid_to`, and `tx_count` directly to the existing `Fact` struct. Bump the file format version from 1 to 2 and migrate old facts on open.

---

## Section 1: Data Model

### `Fact` struct

```rust
pub struct Fact {
    pub entity: EntityId,
    pub attribute: Attribute,
    pub value: Value,
    pub tx_id: TxId,       // wall-clock millis since epoch (u64) — unchanged
    pub tx_count: u64,     // NEW: monotonically incrementing counter (1, 2, 3…)
    pub valid_from: i64,   // NEW: millis since epoch (i64, allows pre-1970 dates)
    pub valid_to: i64,     // NEW: millis since epoch; i64::MAX = "forever"
    pub asserted: bool,    // unchanged
}
```

### Constants

```rust
pub const VALID_TIME_FOREVER: i64 = i64::MAX;
```

### Defaults when no valid time is supplied

- `valid_from` = `tx_id as i64` (fact becomes valid when it is recorded)
- `valid_to` = `VALID_TIME_FOREVER`

### `tx_count`

`FactStorage` gains a `tx_counter: Arc<AtomicU64>`, starting at 0 and incrementing by 1 on each `transact()` or `retract()` call. All facts in a single batch share the same `tx_count`.

**Note on contention**: Under heavy concurrent load, a single `AtomicU64` can become a hot cache-line bottleneck as threads compete for exclusive ownership. This is not a concern in Phase 4 because `FactStorage` already serializes all writes through `Arc<RwLock<Vec<Fact>>>` — the write lock is the true serialization point and no two concurrent writes can be in flight simultaneously. If Phase 5 introduces finer-grained locking (e.g., per-entity locks) that enables true concurrent writes, the `tx_counter` approach should be revisited — options include a per-thread counter with a merge step, or a lock-free sequence based on `tx_id` (wall-clock) alone.

### Role of `asserted`

`asserted=false` (retraction) and `valid_to` serve different purposes:

- `asserted=false` — the fact was removed from the database at transaction time `tx_id`
- `valid_to` — the fact stopped being true in the real world at that timestamp

They are independent. A fact may be valid in the real world (`valid_to = MAX`) but retracted from the database (`asserted=false`). Transaction-time travel requires `asserted` to distinguish "not yet retracted" from "retracted as of tx N". Both fields are necessary for a correct bi-temporal model.

### Migration from version 1

On open, if file format version == 1:
1. Deserialize old facts using a `FactV1` struct (no `tx_count`, `valid_from`, `valid_to`) — pages that fail to deserialize as `FactV1` are skipped
2. Sort facts by `tx_id` ascending
3. Assign `tx_count` sequentially, **grouping by `tx_id`**: all facts sharing the same `tx_id` receive the same `tx_count` value (preserving the batch-atomicity invariant). Each unique `tx_id` increments the counter by 1.
4. Set `valid_from = tx_id as i64`, `valid_to = VALID_TIME_FOREVER`
5. Rewrite all facts in new format using `load_fact()`
6. Set `tx_counter` to the highest `tx_count` assigned in step 3, so subsequent transactions start from `N+1` and do not collide with migrated facts
7. Update header to version 2 and save

```rust
// Used only during migration, not exported
#[derive(Deserialize)]
struct FactV1 {
    entity: EntityId,
    attribute: Attribute,
    value: Value,
    tx_id: TxId,
    asserted: bool,
}
```

---

## Section 2: Query Syntax

### ISO 8601 parsing

Timestamp strings (`:as-of "2024-01-15T10:00:00Z"`, `:valid-at "2023-06-01"`) are parsed using the **`chrono`** crate (UTC only — no local timezone handling, avoiding the `GHSA-wcg3-cvx6-7396` advisory). `chrono` adds ~150-200KB to the binary; this is acceptable given the `<1MB` philosophy target. If WASM support becomes a concern in Phase 7, evaluate switching to the `time` crate which has better WASM support.

Only UTC timestamps and date-only strings (`YYYY-MM-DD`, interpreted as midnight UTC) are supported. Timezone-offset strings (e.g., `+05:30`) are rejected with a descriptive error message referencing the UTC-only policy — e.g.: `"timezone offsets are not supported; use UTC (Z) timestamps only. chrono's local timezone handling (GHSA-wcg3-cvx6-7396) is avoided by design."`

### Parser extensions required

The `(transact {:valid-from ... :valid-to ...} [...])` syntax requires new parser support:
- New `Token::LeftBrace` / `Token::RightBrace` tokens
- New `EdnValue::Map(Vec<(EdnValue, EdnValue)>)` variant
- Map parsing in the EDN parser
- `parse_transact` updated to optionally accept a map as the first argument
- `Pattern::from_edn` updated to accept 3-element vectors (existing) or 4-element vectors (with per-fact metadata map as fourth element)
- 4-element fact vectors with valid-time metadata are **only supported in `transact`**, not in `retract` (retractions always use wall-clock time; per-fact valid time overrides on retractions are not meaningful)

These are non-trivial parser additions and must be included in the implementation plan.

### Transaction time travel — `:as-of`

Supports both a symbolic counter and a wall-clock timestamp:

```datalog
;; As of the 50th transaction (symbolic counter)
[:find ?name :as-of 50 :where [?e :person/name ?name]]

;; As of a wall-clock time (ISO 8601 string)
[:find ?name :as-of "2024-01-15T10:00:00Z" :where [?e :person/name ?name]]
```

### Valid time — `:valid-at`

Point-in-time filter only (range queries deferred to a future phase):

```datalog
;; Facts valid on a specific date
[:find ?status :valid-at "2023-06-01" :where [:alice :employment/status ?status]]

;; No valid time filter — include facts across all valid times
[:find ?name :valid-at :any-valid-time :where [?e :person/name ?name]]
```

`:any-valid-time` disables the valid time filter entirely.

### Combined bi-temporal

```datalog
[:find ?status
 :valid-at "2023-06-01"
 :as-of "2024-01-15T10:00:00Z"
 :where [:alice :employment/status ?status]]
```

### Transact with valid time

Transaction-level default with optional per-fact override:

```datalog
;; Transaction-level valid time (applies to all facts in the batch)
(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"}
          [[:alice :employment/status :active]
           [:alice :employment/org :acme]])

;; Per-fact override (overrides transaction-level for that fact)
(transact {:valid-from "2023-01-01"}
          [[:alice :employment/status :active {:valid-to "2023-06-30"}]
           [:alice :person/name "Alice"]])  ;; no override → valid_to = FOREVER

;; No valid time → defaults (valid_from = tx_time, valid_to = FOREVER)
(transact [[:alice :person/name "Alice"]])
```

---

## Section 3: Query Execution

### Temporal filtering

Temporal filtering happens in the executor **before** pattern matching. The executor narrows the fact set to a temporally-filtered view, then runs normal Datalog evaluation on that view.

### Transaction time filter

| `:as-of` value | Filter |
|---|---|
| `Counter(n)` | include facts where `tx_count <= n` |
| `Timestamp(t)` | include facts where `tx_id <= t` |
| absent | include all facts (no tx time filter) |

### Valid time filter

| `:valid-at` value | Filter |
|---|---|
| `Timestamp(t)` | include facts where `valid_from <= t < valid_to` |
| `AnyValidTime` | no valid time filter |
| absent (default) | include facts where `valid_from <= now < valid_to` |

**Current time source**: the executor uses `tx_id_now() as i64` (from `graph::types`) as the reference timestamp for the "no `:valid-at` = currently valid" default filter. No second time source is introduced.

**Default behaviour change**: queries without any temporal modifier automatically filter to currently valid facts. All migrated Phase 3 facts have `valid_to = MAX`, so this is safe for existing data. However, Phase 4 databases with facts carrying explicit `valid_to` in the past will silently omit those facts from unqualified queries — this is correct semantics but is a user-visible behaviour change. Add a note to the REPL help text and CHANGELOG.

**Ordering of filters**: the transaction time filter (`:as-of`) is applied first, producing a time-bounded fact set. The `asserted=false` exclusion is then applied within that window. This means `:as-of N` correctly surfaces facts that had not yet been retracted at transaction N — the retraction record (with its later `tx_count`) is simply not in the window.

### Updated `DatalogQuery` struct

```rust
pub struct DatalogQuery {
    pub find: Vec<String>,
    pub where_clauses: Vec<WhereClause>,
    pub as_of: Option<AsOf>,        // NEW
    pub valid_at: Option<ValidAt>,  // NEW
}

pub enum AsOf {
    Counter(u64),
    Timestamp(i64),  // millis since epoch
}

pub enum ValidAt {
    Timestamp(i64),  // millis since epoch
    AnyValidTime,
}
```

---

## Section 4: Storage & Persistence

### File format

- **Version** bumped from 1 to 2 in the `FileHeader` (`FORMAT_VERSION` constant in `src/storage/mod.rs`)
- Phase 3 never bumped the version beyond 1, so version-1 files may contain either old property-graph pages (pre-Phase 3) or serialized `Fact` structs (Phase 3). Since property-graph pages will fail to deserialize as `FactV1` (the Phase 3 struct), detection is straightforward: attempt `FactV1` deserialization; if it fails, the page is skipped (treated as empty)
- On open: detect version == 1 → run `migrate_v1_to_v2()` → save → continue; version > 2 → error
- postcard serialization handles the new fields transparently after migration

### Latent Phase 3 bug: `tx_id` is discarded on load

The current `PersistentFactStorage::load()` reconstructs facts by calling `self.storage.transact()`/`retract()`, which internally calls `tx_id_now()` — discarding the original `tx_id` stored on disk. This means time-travel queries against a loaded (persisted) database would be incorrect.

**Phase 4 must fix this** by adding a `FactStorage::load_fact(fact: Fact)` method that inserts a fact directly with its original `tx_id` and `tx_count` preserved, bypassing the normal `transact()` path. The load path in `PersistentFactStorage` switches to this method.

### `FactStorage` changes

- Adds `tx_counter: Arc<AtomicU64>` (starts at 0)
- `tx_count` is fetched-and-incremented **once per `transact()`/`retract()` call**, before any facts are constructed — not per fact, not after the write lock is released. All facts in the batch share the same `tx_count`
- `transact()` and `retract()` gain optional `valid_from: Option<i64>` and `valid_to: Option<i64>` parameters; defaults applied when `None`. **This is a breaking signature change** — all existing call sites (~120 tests + internal callers) must be updated. To minimise churn, add a `TransactOptions` struct with builder methods and update `transact()`/`retract()` to accept `Option<TransactOptions>` as a trailing parameter, defaulting to `None`.
- New method: `load_fact(fact: Fact) -> Result<()>` — inserts a fact with original `tx_id`/`tx_count` preserved (used by load path only)
- New method: `get_facts_as_of(as_of: &AsOf) -> Vec<Fact>` — facts visible at a given transaction time
- New method: `get_facts_valid_at(ts: i64) -> Vec<Fact>` — facts valid at a given timestamp

**`Transaction` struct gains valid-time fields:**
```rust
pub struct Transaction {
    pub facts: Vec<Pattern>,
    pub valid_from: Option<i64>,  // NEW: transaction-level default
    pub valid_to: Option<i64>,    // NEW: transaction-level default
}
```
`execute_transact()` maps these into `TransactOptions` before calling `storage.transact()`. Per-fact overrides (parsed from the 4-element vector) take precedence over transaction-level defaults.

**`TransactOptions` struct:**
```rust
pub struct TransactOptions {
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

impl TransactOptions {
    pub fn new(valid_from: Option<i64>, valid_to: Option<i64>) -> Self { ... }
}
```

**`FileHeader::validate()` updated** to accept version 1 (migrated) and version 2 (current). Versions outside `[1, 2]` return an error. The migration path reads the raw version before calling `validate()`, so version-1 files proceed to `migrate_v1_to_v2()` rather than being rejected.

### `PersistentFactStorage`

- Load path updated to use `load_fact()` instead of `transact()`/`retract()`
- After all facts are loaded (v2 path), set `tx_counter` to `max(tx_count)` across all loaded facts, so new transactions start from `max+1` and do not collide with stored facts
- Migration logic added as `migrate_v1_to_v2()` called during `open()` when version == 1

---

## Section 5: Testing

### Unit tests

**`src/graph/types.rs`**:
- `Fact` construction with explicit valid time
- `Fact` defaults when no valid time supplied (`valid_from = tx_id`, `valid_to = MAX`)
- `VALID_TIME_FOREVER` sentinel value

**`src/graph/storage.rs`**:
- `tx_count` increments correctly across transactions
- `get_facts_as_of(Counter(n))` returns correct snapshot
- `get_facts_as_of(Timestamp(t))` returns correct snapshot
- `get_facts_valid_at(t)` filters correctly inside/outside valid range

**`src/query/datalog/parser.rs`**:
- Parse `:as-of` with counter
- Parse `:as-of` with ISO 8601 timestamp string
- Parse `:valid-at` with timestamp
- Parse `:valid-at :any-valid-time`
- Parse EDN map `{:key val ...}` — new `EdnValue::Map` variant
- Parse `(transact {...} [...])` with transaction-level valid time
- Parse per-fact valid time override (4-element fact vector)
- Reject invalid ISO 8601 strings with a clear error

**`src/query/datalog/executor.rs`**:
- Temporal filter applied before pattern matching
- Default "currently valid" filter when no `valid_at` specified
- `AsOf` and `ValidAt` enum variants handled correctly

### Integration tests (`tests/bitemporal_test.rs`)

- Transaction time travel via counter: assert facts, query `:as-of N` → see state at tx N
- Transaction time travel via timestamp: assert facts, query `:as-of "date"` → see past state
- Valid time range: transact with explicit range; query `:valid-at` inside → match; outside → no match
- Valid time default: no `:valid-at` → only currently valid facts returned
- Valid time `:any-valid-time`: all facts returned regardless of valid time
- Bi-temporal: combine `:as-of` and `:valid-at` in one query
- Migration: open a V1 file, verify migrated facts have correct defaults (`valid_from = tx_id as i64`, `valid_to = MAX`, `tx_count` assigned sequentially)

**Target: ~30 new tests** (existing baseline: 123 tests).

---

## Out of Scope (Deferred)

- Valid time range queries (`:valid-from` / `:valid-to` on queries) — future phase
- Indexes for temporal queries (EAVT, ValidTime) — Phase 6
- History query convenience syntax — future phase
