# Phase 4: Bi-temporal Support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bi-temporal support to Minigraf — every fact gains transaction time (`tx_count`, `tx_id`) and valid time (`valid_from`, `valid_to`), enabling time travel and historical queries.

**Architecture:** Extend the existing `Fact` struct in place with three new fields. Add temporal filtering to the query executor (applied before pattern matching). Bump the file format to v2 with automatic migration of v1 files. Add `chrono` (UTC-only) for ISO 8601 timestamp parsing.

**Tech Stack:** Rust, `chrono` (UTC timestamps), `postcard` (serialization), `anyhow` (errors), existing `FactStorage`/`DatalogExecutor` infrastructure.

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | Modify | Add `chrono` dependency |
| `src/temporal.rs` | **Create** | ISO 8601 parse helpers, millis conversions |
| `src/lib.rs` | Modify | Expose `temporal` module and new public types |
| `src/graph/types.rs` | Modify | Extend `Fact`, add `VALID_TIME_FOREVER`, `TransactOptions` |
| `src/graph/storage.rs` | Modify | Add `tx_counter`, `load_fact()`, temporal query methods, updated `transact()`/`retract()` signatures |
| `src/storage/mod.rs` | Modify | Bump `FORMAT_VERSION` to 2, update `FileHeader::validate()` |
| `src/storage/persistent_facts.rs` | Modify | Fix `tx_id`-discard bug in load path; add `migrate_v1_to_v2()` |
| `src/query/datalog/types.rs` | Modify | Add `EdnValue::Map`, `AsOf`, `ValidAt` enums; extend `DatalogQuery`, `Transaction` |
| `src/query/datalog/parser.rs` | Modify | Add `{`/`}` tokens, map parsing, `:as-of`/`:valid-at` query clauses, `transact` with options |
| `src/query/datalog/executor.rs` | Modify | Apply temporal filters before pattern matching |
| `tests/bitemporal_test.rs` | **Create** | Integration tests for all bi-temporal behaviour |
| `CHANGELOG.md` | Modify | Document v0.4.0 changes and behaviour change note |

---

## Task 1: Add `chrono` and timestamp parsing utilities

**Files:**
- Modify: `Cargo.toml`
- Create: `src/temporal.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests for timestamp parsing**

Add to a new file `src/temporal.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_utc_datetime() {
        let ts = parse_timestamp("2024-01-15T10:00:00Z").unwrap();
        assert_eq!(ts, 1705312800000_i64);
    }

    #[test]
    fn test_parse_date_only() {
        let ts = parse_timestamp("2023-06-01").unwrap();
        // 2023-06-01 midnight UTC
        assert_eq!(ts, 1685577600000_i64);
    }

    #[test]
    fn test_reject_timezone_offset() {
        let err = parse_timestamp("2024-01-15T10:00:00+05:30").unwrap_err();
        assert!(err.to_string().contains("timezone offsets are not supported"));
        assert!(err.to_string().contains("GHSA-wcg3-cvx6-7396"));
    }

    #[test]
    fn test_reject_invalid_string() {
        assert!(parse_timestamp("not-a-date").is_err());
    }

    #[test]
    fn test_millis_to_timestamp_roundtrip() {
        let original = "2024-01-15T10:00:00Z";
        let millis = parse_timestamp(original).unwrap();
        let back = millis_to_timestamp_string(millis);
        assert_eq!(back, original);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test temporal
```

Expected: compile error (module doesn't exist yet)

- [ ] **Step 3: Add `chrono` to `Cargo.toml`**

```toml
chrono = { version = "0.4", features = ["serde"], default-features = false }
```

Note: `default-features = false` disables `chrono`'s local timezone code entirely, sidestepping GHSA-wcg3-cvx6-7396.

- [ ] **Step 4: Create `src/temporal.rs`**

```rust
//! Timestamp parsing utilities for bi-temporal support.
//!
//! Supports UTC ISO 8601 strings only. Timezone offsets are rejected
//! to avoid chrono's local timezone handling (GHSA-wcg3-cvx6-7396).

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

/// Parse an ISO 8601 UTC string to milliseconds since UNIX epoch.
///
/// Accepted formats:
/// - `"2024-01-15T10:00:00Z"` — UTC datetime
/// - `"2023-06-01"` — date only, interpreted as midnight UTC
///
/// Rejected:
/// - Any string with a timezone offset (e.g., `+05:30`) — use UTC (Z) only.
pub fn parse_timestamp(s: &str) -> Result<i64> {
    // Reject timezone offsets explicitly
    if s.contains('+') || (s.len() > 10 && s[10..].contains('-')) {
        return Err(anyhow!(
            "timezone offsets are not supported; use UTC (Z) timestamps only. \
             chrono's local timezone handling (GHSA-wcg3-cvx6-7396) is avoided by design."
        ));
    }

    // Try full datetime first
    if s.contains('T') {
        let dt = s.parse::<DateTime<Utc>>()
            .map_err(|e| anyhow!("invalid UTC timestamp '{}': {}", s, e))?;
        return Ok(dt.timestamp_millis());
    }

    // Try date-only (YYYY-MM-DD)
    let date = s.parse::<NaiveDate>()
        .map_err(|e| anyhow!("invalid date '{}': {}", s, e))?;
    let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
    Ok(dt.timestamp_millis())
}

/// Convert milliseconds since UNIX epoch back to a UTC ISO 8601 string.
pub fn millis_to_timestamp_string(millis: i64) -> String {
    let dt = DateTime::<Utc>::from_timestamp_millis(millis)
        .unwrap_or_else(|| Utc::now());
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
```

- [ ] **Step 5: Expose `temporal` module in `src/lib.rs`**

Add `pub mod temporal;` to `src/lib.rs`.

- [ ] **Step 6: Run tests and confirm they pass**

```bash
cargo test temporal
```

Expected: 5 tests pass.

- [ ] **Step 7: Run full test suite to confirm no regressions**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/temporal.rs src/lib.rs
git commit -m "feat: add chrono (UTC-only) and timestamp parsing utilities"
```

---

## Task 2: Extend `Fact` struct and add `TransactOptions`

**Files:**
- Modify: `src/graph/types.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)]` block in `src/graph/types.rs`:

```rust
#[test]
fn test_fact_has_valid_time_fields() {
    let entity = Uuid::new_v4();
    let fact = Fact::new(
        entity,
        ":person/name".to_string(),
        Value::String("Alice".to_string()),
        1000,
    );
    // Defaults: valid_from = tx_id as i64, valid_to = FOREVER, tx_count = 0
    assert_eq!(fact.valid_from, 1000_i64);
    assert_eq!(fact.valid_to, VALID_TIME_FOREVER);
    assert_eq!(fact.tx_count, 0);
}

#[test]
fn test_fact_with_explicit_valid_time() {
    let entity = Uuid::new_v4();
    let fact = Fact::with_valid_time(
        entity,
        ":employment/status".to_string(),
        Value::Keyword(":active".to_string()),
        1000,
        1,
        1672531200000_i64, // 2023-01-01
        1685577600000_i64, // 2023-06-01
    );
    assert_eq!(fact.valid_from, 1672531200000_i64);
    assert_eq!(fact.valid_to, 1685577600000_i64);
    assert_eq!(fact.tx_count, 1);
}

#[test]
fn test_valid_time_forever_constant() {
    assert_eq!(VALID_TIME_FOREVER, i64::MAX);
}

#[test]
fn test_transact_options_defaults() {
    let opts = TransactOptions::default();
    assert!(opts.valid_from.is_none());
    assert!(opts.valid_to.is_none());
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_fact_has_valid_time_fields test_fact_with_explicit_valid_time test_valid_time_forever_constant test_transact_options_defaults
```

Expected: compile errors.

- [ ] **Step 3: Update `Fact` struct in `src/graph/types.rs`**

Replace the `Fact` struct and its `impl` block:

```rust
pub const VALID_TIME_FOREVER: i64 = i64::MAX;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub entity: EntityId,
    pub attribute: Attribute,
    pub value: Value,
    pub tx_id: TxId,
    pub tx_count: u64,     // monotonically incrementing batch counter
    pub valid_from: i64,   // millis since epoch (i64 allows pre-1970)
    pub valid_to: i64,     // millis since epoch; VALID_TIME_FOREVER = open-ended
    pub asserted: bool,
}

impl Fact {
    /// Create a new asserted fact with default valid time (valid_from=tx_id, valid_to=FOREVER).
    pub fn new(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted: true,
        }
    }

    /// Create an asserted fact with explicit valid time and tx_count.
    pub fn with_valid_time(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        tx_count: u64,
        valid_from: i64,
        valid_to: i64,
    ) -> Self {
        Fact { entity, attribute, value, tx_id, tx_count, valid_from, valid_to, asserted: true }
    }

    /// Create a retraction with default valid time.
    pub fn retract(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity, attribute, value, tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted: false,
        }
    }

    /// Create a fact with explicit asserted flag and default valid time.
    pub fn with_asserted(
        entity: EntityId, attribute: Attribute, value: Value,
        tx_id: TxId, asserted: bool,
    ) -> Self {
        Fact {
            entity, attribute, value, tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted,
        }
    }

    pub fn is_asserted(&self) -> bool { self.asserted }
    pub fn is_retracted(&self) -> bool { !self.asserted }
}

/// Options for controlling valid time on a transact/retract call.
#[derive(Debug, Clone, Default)]
pub struct TransactOptions {
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

impl TransactOptions {
    pub fn new(valid_from: Option<i64>, valid_to: Option<i64>) -> Self {
        TransactOptions { valid_from, valid_to }
    }
}
```

- [ ] **Step 4: Run the new tests**

```bash
cargo test test_fact_has_valid_time_fields test_fact_with_explicit_valid_time test_valid_time_forever_constant test_transact_options_defaults
```

Expected: pass.

- [ ] **Step 5: Fix all compilation errors in the rest of the codebase**

The struct field additions are non-breaking (existing constructors still work), but any code that constructs `Fact { ... }` with field syntax will need the new fields. Run:

```bash
cargo build 2>&1 | grep "error"
```

Fix any struct literal construction errors by adding the new fields with appropriate defaults (`tx_count: 0`, `valid_from: tx_id as i64`, `valid_to: VALID_TIME_FOREVER`). Also update the existing `test_fact_equality` test in `types.rs` — `Fact` structs with different `tx_count` or valid time fields are no longer equal.

- [ ] **Step 6: Run full test suite**

```bash
cargo test
```

Expected: 123 tests pass (no new tests added yet, just struct updates).

- [ ] **Step 7: Commit**

```bash
git add src/graph/types.rs
git commit -m "feat: extend Fact with tx_count, valid_from, valid_to; add TransactOptions"
```

---

## Task 3: Update `FactStorage` with `tx_counter`, `load_fact()`, and temporal query methods

**Files:**
- Modify: `src/graph/storage.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)]` in `src/graph/storage.rs`:

```rust
#[test]
fn test_tx_count_increments_per_call() {
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();

    storage.transact(vec![
        (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
    ], None).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));

    storage.transact(vec![
        (alice, ":person/age".to_string(), Value::Integer(30)),
    ], None).unwrap();

    let facts = storage.get_all_facts().unwrap();
    let name_fact = facts.iter().find(|f| f.attribute == ":person/name").unwrap();
    let age_fact = facts.iter().find(|f| f.attribute == ":person/age").unwrap();

    assert_eq!(name_fact.tx_count, 1);
    assert_eq!(age_fact.tx_count, 2);
}

#[test]
fn test_batch_facts_share_tx_count() {
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();

    storage.transact(vec![
        (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
        (alice, ":person/age".to_string(), Value::Integer(30)),
    ], None).unwrap();

    let facts = storage.get_all_facts().unwrap();
    assert!(facts.iter().all(|f| f.tx_count == 1));
}

#[test]
fn test_load_fact_preserves_tx_id_and_tx_count() {
    let storage = FactStorage::new();
    let entity = Uuid::new_v4();

    let original_fact = Fact::with_valid_time(
        entity,
        ":person/name".to_string(),
        Value::String("Alice".to_string()),
        12345_u64,  // original tx_id
        7,          // original tx_count
        12345_i64,
        VALID_TIME_FOREVER,
    );

    storage.load_fact(original_fact.clone()).unwrap();

    let facts = storage.get_all_facts().unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].tx_id, 12345);
    assert_eq!(facts[0].tx_count, 7);
}

#[test]
fn test_get_facts_as_of_counter() {
    use crate::query::datalog::types::{AsOf};

    let storage = FactStorage::new();
    let alice = Uuid::new_v4();

    // tx_count = 1
    storage.transact(vec![
        (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
    ], None).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));

    // tx_count = 2
    storage.transact(vec![
        (alice, ":person/age".to_string(), Value::Integer(30)),
    ], None).unwrap();

    // as-of tx 1: only name fact visible
    let snapshot = storage.get_facts_as_of(&AsOf::Counter(1)).unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].attribute, ":person/name");
}

#[test]
fn test_get_facts_valid_at() {
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();

    let opts = TransactOptions::new(
        Some(1672531200000_i64), // 2023-01-01
        Some(1685577600000_i64), // 2023-06-01
    );

    storage.transact(vec![
        (alice, ":employment/status".to_string(), Value::Keyword(":active".to_string())),
    ], Some(opts)).unwrap();

    // Valid on 2023-03-01 (inside range)
    let inside = storage.get_facts_valid_at(1677628800000_i64).unwrap();
    assert_eq!(inside.len(), 1);

    // Valid on 2024-01-01 (outside range)
    let outside = storage.get_facts_valid_at(1704067200000_i64).unwrap();
    assert_eq!(outside.len(), 0);
}

#[test]
fn test_tx_counter_restored_after_load_fact() {
    let storage = FactStorage::new();
    let entity = Uuid::new_v4();

    // Load a fact with tx_count = 5 (simulating migration/load)
    let fact = Fact::with_valid_time(
        entity, ":a".to_string(), Value::Integer(1),
        1000, 5, 1000_i64, VALID_TIME_FOREVER,
    );
    storage.load_fact(fact).unwrap();
    storage.restore_tx_counter().unwrap();

    // Next transact should get tx_count = 6
    storage.transact(vec![
        (entity, ":b".to_string(), Value::Integer(2)),
    ], None).unwrap();

    let facts = storage.get_all_facts().unwrap();
    let b_fact = facts.iter().find(|f| f.attribute == ":b").unwrap();
    assert_eq!(b_fact.tx_count, 6);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_tx_count test_batch_facts test_load_fact test_get_facts_as_of test_get_facts_valid_at test_tx_counter_restored
```

Expected: compile errors (methods don't exist yet, signatures differ).

- [ ] **Step 3: Update `src/graph/storage.rs`**

Update `FactStorage` struct:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

pub struct FactStorage {
    facts: Arc<RwLock<Vec<Fact>>>,
    tx_counter: Arc<AtomicU64>,
}
```

Update `FactStorage::new()`:

```rust
pub fn new() -> Self {
    FactStorage {
        facts: Arc::new(RwLock::new(Vec::new())),
        tx_counter: Arc::new(AtomicU64::new(0)),
    }
}
```

Update `transact()` signature and body:

```rust
pub fn transact(
    &self,
    fact_tuples: Vec<(EntityId, Attribute, Value)>,
    opts: Option<TransactOptions>,
) -> Result<TxId> {
    let tx_id = tx_id_now();
    let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;
    let opts = opts.unwrap_or_default();

    let facts: Vec<Fact> = fact_tuples
        .into_iter()
        .map(|(entity, attribute, value)| {
            let valid_from = opts.valid_from.unwrap_or(tx_id as i64);
            let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
            Fact::with_valid_time(entity, attribute, value, tx_id, tx_count, valid_from, valid_to)
        })
        .collect();

    let mut storage = self.facts.write().unwrap();
    storage.extend(facts);
    Ok(tx_id)
}
```

Update `retract()` signature similarly (retractions always use `valid_from=tx_id`, `valid_to=FOREVER` — opts not accepted):

```rust
pub fn retract(
    &self,
    fact_tuples: Vec<(EntityId, Attribute, Value)>,
) -> Result<TxId> {
    let tx_id = tx_id_now();
    let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;

    let retractions: Vec<Fact> = fact_tuples
        .into_iter()
        .map(|(entity, attribute, value)| {
            let mut f = Fact::retract(entity, attribute, value, tx_id);
            f.tx_count = tx_count;
            f
        })
        .collect();

    let mut storage = self.facts.write().unwrap();
    storage.extend(retractions);
    Ok(tx_id)
}
```

Add new methods:

```rust
/// Insert a fact with its original tx_id and tx_count preserved.
/// Used by the load and migration paths only — bypasses tx_counter.
pub fn load_fact(&self, fact: Fact) -> Result<()> {
    let mut storage = self.facts.write().unwrap();
    storage.push(fact);
    Ok(())
}

/// Set tx_counter to max(tx_count) across all loaded facts.
/// Must be called after all load_fact() calls complete.
pub fn restore_tx_counter(&self) -> Result<()> {
    let storage = self.facts.read().unwrap();
    let max = storage.iter().map(|f| f.tx_count).max().unwrap_or(0);
    self.tx_counter.store(max, Ordering::SeqCst);
    Ok(())
}

/// Return all facts visible as of the given transaction point.
pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
    let storage = self.facts.read().unwrap();
    let filtered = storage.iter().filter(|f| match as_of {
        AsOf::Counter(n) => f.tx_count <= *n,
        AsOf::Timestamp(t) => f.tx_id <= *t as u64,
    }).cloned().collect();
    Ok(filtered)
}

/// Return all asserted facts valid at the given timestamp.
pub fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> {
    let storage = self.facts.read().unwrap();
    let filtered = storage.iter()
        .filter(|f| f.is_asserted() && f.valid_from <= ts && ts < f.valid_to)
        .cloned()
        .collect();
    Ok(filtered)
}
```

Note: `AsOf` is defined in `src/query/datalog/types.rs` (Task 5). Add `use crate::query::datalog::types::AsOf;` once Task 5 is done. For now, the methods can use inline logic.

- [ ] **Step 4: Update all existing `transact()` call sites in tests and code**

The `transact()` signature now takes an extra `Option<TransactOptions>` parameter. Search for all callers:

```bash
cargo build 2>&1 | grep "error\[E"
```

In `src/graph/storage.rs` tests and `src/storage/persistent_facts.rs`, add `None` as the second argument to all `storage.transact(...)` calls. The `retract()` signature is unchanged.

- [ ] **Step 5: Run failing tests**

```bash
cargo test test_tx_count test_batch_facts test_load_fact test_get_facts_as_of test_get_facts_valid_at test_tx_counter_restored
```

Expected: pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/graph/storage.rs
git commit -m "feat: add tx_counter, load_fact(), temporal query methods to FactStorage"
```

---

## Task 4: Fix file format — version bump and `FileHeader::validate()`

**Files:**
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/storage/mod.rs` tests:

```rust
#[test]
fn test_format_version_is_2() {
    assert_eq!(FORMAT_VERSION, 2);
}

#[test]
fn test_validate_accepts_version_1_and_2() {
    let mut header = FileHeader::new();
    header.version = 1;
    assert!(header.validate().is_ok());

    header.version = 2;
    assert!(header.validate().is_ok());
}

#[test]
fn test_validate_rejects_version_0_and_3() {
    let mut header = FileHeader::new();
    header.version = 0;
    assert!(header.validate().is_err());

    header.version = 3;
    assert!(header.validate().is_err());
}

#[test]
fn test_new_header_has_version_2() {
    let header = FileHeader::new();
    assert_eq!(header.version, FORMAT_VERSION);
    assert_eq!(header.version, 2);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_format_version test_validate_accepts test_validate_rejects test_new_header
```

Expected: failures (`FORMAT_VERSION` is 1, `validate()` rejects version 1).

- [ ] **Step 3: Update `src/storage/mod.rs`**

Change:
```rust
pub const FORMAT_VERSION: u32 = 1;
```
to:
```rust
pub const FORMAT_VERSION: u32 = 2;
```

Update `FileHeader::validate()`:
```rust
pub fn validate(&self) -> Result<()> {
    if self.magic != MAGIC_NUMBER {
        anyhow::bail!("Invalid magic number");
    }
    if self.version < 1 || self.version > FORMAT_VERSION {
        anyhow::bail!(
            "Unsupported format version: {} (supported: 1-{})",
            self.version, FORMAT_VERSION
        );
    }
    Ok(())
}
```

- [ ] **Step 4: Run failing tests**

```bash
cargo test test_format_version test_validate_accepts test_validate_rejects test_new_header
```

Expected: pass.

- [ ] **Step 5: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat: bump FORMAT_VERSION to 2, update FileHeader::validate() for v1/v2"
```

---

## Task 5: Fix `PersistentFactStorage` load path and add migration

**Files:**
- Modify: `src/storage/persistent_facts.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/storage/persistent_facts.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::memory::MemoryBackend;

    fn make_v1_backend() -> MemoryBackend {
        // Build a backend with a v1-format header and two serialized FactV1 facts
        use crate::storage::{PAGE_SIZE, MAGIC_NUMBER};
        use crate::graph::types::{Value};
        use uuid::Uuid;

        let alice = Uuid::new_v4();

        #[derive(serde::Serialize)]
        struct FactV1Ser {
            entity: Uuid,
            attribute: String,
            value: Value,
            tx_id: u64,
            asserted: bool,
        }

        let fact1 = FactV1Ser {
            entity: alice,
            attribute: ":person/name".to_string(),
            value: Value::String("Alice".to_string()),
            tx_id: 1000,
            asserted: true,
        };
        let fact2 = FactV1Ser {
            entity: alice,
            attribute: ":person/age".to_string(),
            value: Value::Integer(30),
            tx_id: 1000,
            asserted: true,
        };

        let mut backend = MemoryBackend::new();

        // Write v1 header (version=1, page_count=3)
        let mut header_bytes = vec![0u8; PAGE_SIZE];
        header_bytes[0..4].copy_from_slice(b"MGRF");
        header_bytes[4..8].copy_from_slice(&1u32.to_le_bytes()); // version = 1
        header_bytes[8..16].copy_from_slice(&3u64.to_le_bytes()); // page_count = 3
        backend.write_page(0, &header_bytes).unwrap();

        // Write facts
        for (i, fact) in [&fact1, &fact2].iter().enumerate() {
            let data = postcard::to_allocvec(*fact).unwrap();
            let mut page = vec![0u8; PAGE_SIZE];
            page[..data.len()].copy_from_slice(&data);
            backend.write_page((i + 1) as u64, &page).unwrap();
        }

        backend
    }

    #[test]
    fn test_load_preserves_original_tx_id() {
        // Build a v2 backend with a fact that has a known tx_id
        use crate::storage::backend::memory::MemoryBackend;
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new()).unwrap();

        let alice = uuid::Uuid::new_v4();
        pfs.storage().transact(vec![
            (alice, ":person/name".to_string(), Value::String("Alice".to_string())),
        ], None).unwrap();

        let original_tx_id = pfs.storage().get_all_facts().unwrap()[0].tx_id;

        pfs.save().unwrap();

        // Reload from the backend
        let backend = pfs.into_backend();
        let pfs2 = PersistentFactStorage::new(backend).unwrap();
        let loaded_tx_id = pfs2.storage().get_all_facts().unwrap()[0].tx_id;

        assert_eq!(original_tx_id, loaded_tx_id, "tx_id must survive save/load round-trip");
    }

    #[test]
    fn test_migrate_v1_to_v2_assigns_defaults() {
        let backend = make_v1_backend();
        let pfs = PersistentFactStorage::new(backend).unwrap();
        let facts = pfs.storage().get_all_facts().unwrap();

        assert_eq!(facts.len(), 2);
        // Both facts have tx_id=1000 (same batch) → same tx_count
        assert_eq!(facts[0].tx_count, facts[1].tx_count);
        assert_eq!(facts[0].valid_to, VALID_TIME_FOREVER);
        assert_eq!(facts[0].valid_from, 1000_i64);
    }

    #[test]
    fn test_migrate_v1_tx_counter_set_correctly() {
        let backend = make_v1_backend();
        let pfs = PersistentFactStorage::new(backend).unwrap();

        let alice = uuid::Uuid::new_v4();
        pfs.storage().transact(vec![
            (alice, ":new/fact".to_string(), Value::Boolean(true)),
        ], None).unwrap();

        let new_fact = pfs.storage().get_all_facts().unwrap()
            .into_iter()
            .find(|f| f.attribute == ":new/fact")
            .unwrap();

        // After migrating 1 unique tx_id (tx_count=1), next tx gets tx_count=2
        assert_eq!(new_fact.tx_count, 2);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_load_preserves test_migrate_v1
```

Expected: compile errors (methods don't exist, `storage()` accessor missing).

- [ ] **Step 3: Update `src/storage/persistent_facts.rs`**

Add a `FactV1` deserialization struct (private, for migration only):

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FactV1 {
    entity: EntityId,
    attribute: Attribute,
    value: Value,
    tx_id: TxId,
    asserted: bool,
}
```

Fix the `load()` method to use `load_fact()`:

```rust
fn load(&mut self) -> Result<()> {
    let header_page = self.backend.read_page(0)?;
    let header = FileHeader::from_bytes(&header_page)?;
    header.validate()?;

    if header.version == 1 {
        return self.migrate_v1_to_v2();
    }

    self.storage.clear()?;

    let page_count = header.page_count;
    for page_id in 1..page_count {
        let page = self.backend.read_page(page_id)?;
        if let Ok(fact) = postcard::from_bytes::<Fact>(&page) {
            self.storage.load_fact(fact)?;
        }
    }

    self.storage.restore_tx_counter()?;
    self.dirty = false;
    Ok(())
}
```

Add `migrate_v1_to_v2()`:

```rust
fn migrate_v1_to_v2(&mut self) -> Result<()> {
    use crate::graph::types::VALID_TIME_FOREVER;

    let header_page = self.backend.read_page(0)?;
    let header = FileHeader::from_bytes(&header_page)?;
    let page_count = header.page_count;

    // Read all v1 facts
    let mut v1_facts: Vec<FactV1> = Vec::new();
    for page_id in 1..page_count {
        let page = self.backend.read_page(page_id)?;
        if let Ok(fact) = postcard::from_bytes::<FactV1>(&page) {
            v1_facts.push(fact);
        }
    }

    // Sort by tx_id ascending
    v1_facts.sort_by_key(|f| f.tx_id);

    // Assign tx_count, grouping by tx_id
    let mut tx_count: u64 = 0;
    let mut prev_tx_id: Option<TxId> = None;
    let mut migrated: Vec<Fact> = Vec::new();

    for v1 in v1_facts {
        if prev_tx_id != Some(v1.tx_id) {
            tx_count += 1;
            prev_tx_id = Some(v1.tx_id);
        }
        let fact = Fact::with_valid_time(
            v1.entity, v1.attribute, v1.value,
            v1.tx_id, tx_count,
            v1.tx_id as i64, VALID_TIME_FOREVER,
        );
        // Preserve asserted flag
        let mut fact = fact;
        fact.asserted = v1.asserted;
        migrated.push(fact);
    }

    self.storage.clear()?;
    for fact in migrated {
        self.storage.load_fact(fact)?;
    }
    self.storage.restore_tx_counter()?;

    self.dirty = true;
    self.save()?;
    Ok(())
}
```

Add a `storage()` accessor for tests:

```rust
pub fn storage(&self) -> &FactStorage {
    &self.storage
}
```

- [ ] **Step 4: Run the failing tests**

```bash
cargo test test_load_preserves test_migrate_v1
```

Expected: pass.

- [ ] **Step 5: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/storage/persistent_facts.rs
git commit -m "fix: preserve tx_id on load; add v1→v2 migration to PersistentFactStorage"
```

---

## Task 6: Add EDN map support to the parser

**Files:**
- Modify: `src/query/datalog/types.rs`
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/query/datalog/parser.rs` tests:

```rust
#[test]
fn test_parse_edn_map() {
    let result = parse("{:valid-from \"2023-01-01\" :valid-to \"2023-06-30\"}");
    let map = match result.unwrap() {
        EdnValue::Map(pairs) => pairs,
        _ => panic!("expected map"),
    };
    assert_eq!(map.len(), 2);
    assert_eq!(map[0].0, EdnValue::Keyword(":valid-from".to_string()));
    assert_eq!(map[0].1, EdnValue::String("2023-01-01".to_string()));
}

#[test]
fn test_parse_empty_map() {
    let result = parse("{}");
    assert!(matches!(result.unwrap(), EdnValue::Map(pairs) if pairs.is_empty()));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_parse_edn_map test_parse_empty_map
```

Expected: compile errors / failures.

- [ ] **Step 3: Add `EdnValue::Map` variant to `src/query/datalog/types.rs`**

```rust
pub enum EdnValue {
    // ... existing variants ...
    /// Map: {:key val ...}
    Map(Vec<(EdnValue, EdnValue)>),
}
```

Add accessor:
```rust
pub fn as_map(&self) -> Option<&Vec<(EdnValue, EdnValue)>> {
    match self {
        EdnValue::Map(m) => Some(m),
        _ => None,
    }
}
```

- [ ] **Step 4: Add `{`/`}` tokens and map parsing to `src/query/datalog/parser.rs`**

Add to `Token` enum:
```rust
LeftBrace,
RightBrace,
```

Add to `tokenize()` match:
```rust
'{' => { tokens.push(Token::LeftBrace); chars.next(); }
'}' => { tokens.push(Token::RightBrace); chars.next(); }
```

Add `parse_map()` to the parser:
```rust
fn parse_map(tokens: &[Token], pos: &mut usize) -> Result<EdnValue, String> {
    // consume '{'
    *pos += 1;
    let mut pairs = Vec::new();
    while *pos < tokens.len() {
        if tokens[*pos] == Token::RightBrace {
            *pos += 1;
            return Ok(EdnValue::Map(pairs));
        }
        let key = parse_value(tokens, pos)?;
        let val = parse_value(tokens, pos)?;
        pairs.push((key, val));
    }
    Err("Unterminated map: missing '}'".to_string())
}
```

Update `parse_value()` to dispatch on `Token::LeftBrace`:
```rust
Token::LeftBrace => parse_map(tokens, pos),
```

- [ ] **Step 5: Run the failing tests**

```bash
cargo test test_parse_edn_map test_parse_empty_map
```

Expected: pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/parser.rs
git commit -m "feat: add EdnValue::Map and { } token support to EDN parser"
```

---

## Task 7: Add `AsOf`, `ValidAt` types; extend `DatalogQuery` and `Transaction`

**Files:**
- Modify: `src/query/datalog/types.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/query/datalog/types.rs` tests:

```rust
#[test]
fn test_datalog_query_with_temporal_fields() {
    use crate::query::datalog::types::{AsOf, ValidAt};

    let query = DatalogQuery::new(
        vec!["?name".to_string()],
        vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::Symbol("?name".to_string()),
        ))],
    );

    assert!(query.as_of.is_none());
    assert!(query.valid_at.is_none());

    let query_with_time = DatalogQuery {
        as_of: Some(AsOf::Counter(5)),
        valid_at: Some(ValidAt::AnyValidTime),
        ..query
    };

    assert!(matches!(query_with_time.as_of, Some(AsOf::Counter(5))));
    assert!(matches!(query_with_time.valid_at, Some(ValidAt::AnyValidTime)));
}

#[test]
fn test_transaction_with_valid_time() {
    let tx = Transaction {
        facts: vec![],
        valid_from: Some(1672531200000_i64),
        valid_to: None,
    };
    assert_eq!(tx.valid_from, Some(1672531200000_i64));
    assert!(tx.valid_to.is_none());
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_datalog_query_with_temporal test_transaction_with_valid_time
```

Expected: compile errors.

- [ ] **Step 3: Add `AsOf` and `ValidAt` to `src/query/datalog/types.rs`**

```rust
/// Transaction time specifier for :as-of queries
#[derive(Debug, Clone, PartialEq)]
pub enum AsOf {
    /// Match facts where tx_count <= n
    Counter(u64),
    /// Match facts where tx_id <= t (millis since epoch)
    Timestamp(i64),
}

/// Valid time specifier for :valid-at queries
#[derive(Debug, Clone, PartialEq)]
pub enum ValidAt {
    /// Match facts where valid_from <= t < valid_to (millis since epoch)
    Timestamp(i64),
    /// No valid time filter — return facts regardless of valid_from/valid_to
    AnyValidTime,
}
```

Update `DatalogQuery`:
```rust
pub struct DatalogQuery {
    pub find: Vec<String>,
    pub where_clauses: Vec<WhereClause>,
    pub as_of: Option<AsOf>,
    pub valid_at: Option<ValidAt>,
}

impl DatalogQuery {
    pub fn new(find: Vec<String>, where_clauses: Vec<WhereClause>) -> Self {
        DatalogQuery { find, where_clauses, as_of: None, valid_at: None }
    }
    // Keep existing helpers unchanged
}
```

Update `Transaction`:
```rust
pub struct Transaction {
    pub facts: Vec<Pattern>,
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

impl Transaction {
    pub fn new(facts: Vec<Pattern>) -> Self {
        Transaction { facts, valid_from: None, valid_to: None }
    }
}
```

- [ ] **Step 4: Fix compilation errors from struct changes**

```bash
cargo build 2>&1 | grep "error"
```

Any `Transaction { facts: ... }` struct literals need `valid_from: None, valid_to: None` added. Any `DatalogQuery { find, where_clauses }` literals need `as_of: None, valid_at: None`.

- [ ] **Step 5: Run tests**

```bash
cargo test test_datalog_query_with_temporal test_transaction_with_valid_time
```

Expected: pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/types.rs
git commit -m "feat: add AsOf, ValidAt enums; extend DatalogQuery and Transaction with temporal fields"
```

---

## Task 8: Update the parser for `:as-of`, `:valid-at`, and `transact` with options

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/query/datalog/parser.rs` tests:

```rust
#[test]
fn test_parse_as_of_counter() {
    let cmd = parse_command("(query [:find ?name :as-of 50 :where [?e :person/name ?name]])").unwrap();
    let query = match cmd { DatalogCommand::Query(q) => q, _ => panic!() };
    assert_eq!(query.as_of, Some(AsOf::Counter(50)));
}

#[test]
fn test_parse_as_of_timestamp() {
    let cmd = parse_command(
        "(query [:find ?name :as-of \"2024-01-15T10:00:00Z\" :where [?e :person/name ?name]])"
    ).unwrap();
    let query = match cmd { DatalogCommand::Query(q) => q, _ => panic!() };
    assert!(matches!(query.as_of, Some(AsOf::Timestamp(_))));
}

#[test]
fn test_parse_valid_at_timestamp() {
    let cmd = parse_command(
        "(query [:find ?s :valid-at \"2023-06-01\" :where [:alice :employment/status ?s]])"
    ).unwrap();
    let query = match cmd { DatalogCommand::Query(q) => q, _ => panic!() };
    assert!(matches!(query.valid_at, Some(ValidAt::Timestamp(_))));
}

#[test]
fn test_parse_valid_at_any() {
    let cmd = parse_command(
        "(query [:find ?name :valid-at :any-valid-time :where [?e :person/name ?name]])"
    ).unwrap();
    let query = match cmd { DatalogCommand::Query(q) => q, _ => panic!() };
    assert_eq!(query.valid_at, Some(ValidAt::AnyValidTime));
}

#[test]
fn test_parse_transact_with_tx_level_valid_time() {
    let cmd = parse_command(
        "(transact {:valid-from \"2023-01-01\" :valid-to \"2023-06-30\"} [[:alice :employment/status :active]])"
    ).unwrap();
    let tx = match cmd { DatalogCommand::Transact(t) => t, _ => panic!() };
    assert!(tx.valid_from.is_some());
    assert!(tx.valid_to.is_some());
}

#[test]
fn test_parse_transact_with_per_fact_valid_time() {
    let cmd = parse_command(
        "(transact {:valid-from \"2023-01-01\"} [[:alice :employment/status :active {:valid-to \"2023-06-30\"}] [:alice :person/name \"Alice\"]])"
    ).unwrap();
    let tx = match cmd { DatalogCommand::Transact(t) => t, _ => panic!() };
    assert_eq!(tx.facts.len(), 2);
    // Per-fact metadata is stored in the Pattern's metadata field (see Step 3)
}

#[test]
fn test_parse_reject_timezone_offset_in_as_of() {
    let result = parse_command(
        "(query [:find ?n :as-of \"2024-01-15T10:00:00+05:30\" :where [?e :person/name ?n]])"
    );
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("timezone offsets are not supported"));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_parse_as_of test_parse_valid_at test_parse_transact_with
```

Expected: failures.

- [ ] **Step 3: Update `parse_query()` in `src/query/datalog/parser.rs`**

In the query vector parsing loop, add handling for `:as-of` and `:valid-at` keywords:

```rust
// In the loop that processes the query vector elements:
":as-of" => {
    let val = parse_value(tokens, pos)?;
    let as_of = match &val {
        EdnValue::Integer(n) => AsOf::Counter(*n as u64),
        EdnValue::String(s) => {
            let ts = parse_timestamp(s)
                .map_err(|e| e.to_string())?;
            AsOf::Timestamp(ts)
        }
        _ => return Err(":as-of must be an integer (counter) or ISO 8601 string".to_string()),
    };
    query_as_of = Some(as_of);
}
":valid-at" => {
    let val = parse_value(tokens, pos)?;
    let valid_at = match &val {
        EdnValue::String(s) => {
            let ts = parse_timestamp(s)
                .map_err(|e| e.to_string())?;
            ValidAt::Timestamp(ts)
        }
        EdnValue::Keyword(k) if k == ":any-valid-time" => ValidAt::AnyValidTime,
        _ => return Err(":valid-at must be an ISO 8601 string or :any-valid-time".to_string()),
    };
    query_valid_at = Some(valid_at);
}
```

Set `as_of` and `valid_at` on the constructed `DatalogQuery`.

- [ ] **Step 4: Update `parse_transact()` to accept optional map**

```rust
fn parse_transact(tokens: &[Token], pos: &mut usize) -> Result<Transaction, String> {
    // Check if first argument is a map (transaction-level valid time options)
    let (tx_valid_from, tx_valid_to) = if *pos < tokens.len()
        && tokens[*pos] == Token::LeftBrace
    {
        let map = parse_map(tokens, pos)?;
        let pairs = map.as_map().unwrap();
        let mut vf = None;
        let mut vt = None;
        for (k, v) in pairs {
            match (k.as_keyword(), v) {
                (Some(":valid-from"), EdnValue::String(s)) => {
                    vf = Some(parse_timestamp(s).map_err(|e| e.to_string())?);
                }
                (Some(":valid-to"), EdnValue::String(s)) => {
                    vt = Some(parse_timestamp(s).map_err(|e| e.to_string())?);
                }
                _ => {}
            }
        }
        (vf, vt)
    } else {
        (None, None)
    };

    // Parse facts vector: expect Token::LeftBracket, then zero or more fact vectors,
    // then Token::RightBracket. Each fact vector is itself a LeftBracket-delimited
    // 3-element or 4-element vector. The 4th element (if present) must be a Map with
    // :valid-from/:valid-to overrides.
    //
    // Concretely: refactor the existing fact-parsing loop out of `parse_transact` into a
    // helper `parse_fact_vector(tokens, pos) -> Result<Vec<Pattern>, String>` by extracting
    // the current "consume '[', loop until ']', parse each inner '[e a v]'" logic. Then:
    //   - For each inner vector, call `parse_value` three times (entity, attr, value).
    //   - If the next token is LeftBrace (not RightBracket), call `parse_map` and extract
    //     `:valid-from`/`:valid-to` from the map into per-fact metadata.
    //   - Store per-fact overrides by resolving them immediately against tx-level defaults:
    //     effective_valid_from = per_fact_valid_from.or(tx_valid_from)
    //     effective_valid_to   = per_fact_valid_to.or(tx_valid_to)
    //   - Store the resolved values in `Pattern` (add `valid_from: Option<i64>` and
    //     `valid_to: Option<i64>` fields to `Pattern` in types.rs).
    //
    // Then set the tx-level defaults on the Transaction:
    let mut tx = parse_fact_vector_with_overrides(tokens, pos, tx_valid_from, tx_valid_to)?;
    tx.valid_from = tx_valid_from;
    tx.valid_to = tx_valid_to;
    Ok(tx)
}
```

- [ ] **Step 5: Run failing tests**

```bash
cargo test test_parse_as_of test_parse_valid_at test_parse_transact_with test_parse_reject_timezone
```

Expected: pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "feat: parse :as-of, :valid-at, and transact with valid-time options"
```

---

## Task 9: Update executor with temporal filtering

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Write failing tests**

Add to `src/query/datalog/executor.rs` tests:

```rust
#[test]
fn test_default_query_filters_to_currently_valid() {
    use crate::graph::types::{TransactOptions, VALID_TIME_FOREVER};

    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage.clone());
    let alice = Uuid::new_v4();

    // Fact valid forever (default)
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        )],
        valid_from: None,
        valid_to: None,
    })).unwrap();

    // Fact with valid_to in the past (expired)
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":employment/status".to_string()),
            EdnValue::Keyword(":active".to_string()),
        )],
        valid_from: Some(1000_i64),
        valid_to: Some(2000_i64),  // expired long ago
    })).unwrap();

    // Default query (no :valid-at) should only return the forever-valid fact
    let result = executor.execute(DatalogCommand::Query(DatalogQuery::new(
        vec!["?attr".to_string()],
        vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Symbol("?attr".to_string()),
            EdnValue::Symbol("_".to_string()),
        ))],
    ))).unwrap();

    let rows = match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!(),
    };
    assert_eq!(rows.len(), 1); // only the name fact
}

#[test]
fn test_as_of_counter_shows_past_state() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);
    let alice = Uuid::new_v4();

    // tx_count=1: assert name
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        )],
        valid_from: None, valid_to: None,
    })).unwrap();

    // tx_count=2: assert age
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":person/age".to_string()),
            EdnValue::Integer(30),
        )],
        valid_from: None, valid_to: None,
    })).unwrap();

    // :as-of 1 → only name fact visible (age was added at tx_count=2)
    let result = executor.execute(DatalogCommand::Query(DatalogQuery {
        find: vec!["?attr".to_string()],
        where_clauses: vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Symbol("?attr".to_string()),
            EdnValue::Symbol("_".to_string()),
        ))],
        as_of: Some(AsOf::Counter(1)),
        valid_at: Some(ValidAt::AnyValidTime),
    })).unwrap();

    let rows = match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    };
    assert_eq!(rows.len(), 1);
}

#[test]
fn test_valid_at_any_valid_time_shows_all() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);
    let alice = Uuid::new_v4();

    // Fact valid forever (default)
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        )],
        valid_from: None, valid_to: None,
    })).unwrap();

    // Fact with valid_to already in the past
    executor.execute(DatalogCommand::Transact(Transaction {
        facts: vec![Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Keyword(":employment/status".to_string()),
            EdnValue::Keyword(":active".to_string()),
        )],
        valid_from: Some(1000_i64),
        valid_to: Some(2000_i64),  // expired
    })).unwrap();

    // :valid-at :any-valid-time → both facts returned
    let result = executor.execute(DatalogCommand::Query(DatalogQuery {
        find: vec!["?attr".to_string()],
        where_clauses: vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Uuid(alice),
            EdnValue::Symbol("?attr".to_string()),
            EdnValue::Symbol("_".to_string()),
        ))],
        as_of: None,
        valid_at: Some(ValidAt::AnyValidTime),
    })).unwrap();

    let rows = match result {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query results"),
    };
    assert_eq!(rows.len(), 2);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_default_query_filters test_as_of_counter test_valid_at_any
```

Expected: failures (no temporal filtering implemented yet).

- [ ] **Step 3: Update `execute_query()` in `src/query/datalog/executor.rs`**

Add a `filter_facts_for_query()` helper that takes a `DatalogQuery` and returns a filtered `FactStorage` view:

```rust
fn filter_facts_for_query(&self, query: &DatalogQuery) -> Result<FactStorage> {
    let now = tx_id_now() as i64;

    // Step 1: transaction time filter
    let tx_filtered: Vec<Fact> = match &query.as_of {
        Some(as_of) => self.storage.get_facts_as_of(as_of)?,
        None => self.storage.get_all_facts()?,
    };

    // Step 2: asserted=false exclusion within the tx window
    let asserted: Vec<Fact> = tx_filtered.into_iter()
        .filter(|f| f.is_asserted())
        .collect();

    // Step 3: valid time filter
    let valid_filtered: Vec<Fact> = match &query.valid_at {
        Some(ValidAt::Timestamp(t)) => asserted.into_iter()
            .filter(|f| f.valid_from <= *t && *t < f.valid_to)
            .collect(),
        Some(ValidAt::AnyValidTime) => asserted,
        None => asserted.into_iter()
            .filter(|f| f.valid_from <= now && now < f.valid_to)
            .collect(),
    };

    // Build a temporary FactStorage with the filtered facts
    let filtered_storage = FactStorage::new();
    for fact in valid_filtered {
        filtered_storage.load_fact(fact)?;
    }
    Ok(filtered_storage)
}
```

Update `execute_query()` and `execute_query_with_rules()` to use `filter_facts_for_query()` instead of `self.storage` directly.

- [ ] **Step 4: Update `execute_transact()` to pass `TransactOptions` from `Transaction`**

```rust
fn execute_transact(&self, tx: Transaction) -> Result<QueryResult> {
    // ... existing entity/attribute/value parsing ...

    let opts = if tx.valid_from.is_some() || tx.valid_to.is_some() {
        Some(TransactOptions::new(tx.valid_from, tx.valid_to))
    } else {
        None
    };

    // Per-fact opts take precedence (handled during Pattern parsing in Task 8)

    let tx_id = self.storage.transact(fact_tuples, opts)?;
    Ok(QueryResult::Transacted(tx_id))
}
```

- [ ] **Step 5: Run failing tests**

```bash
cargo test test_default_query_filters test_as_of_counter test_valid_at_any
```

Expected: pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test
```

Expected: 123 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat: apply temporal filters in executor before pattern matching"
```

---

## Task 10: Integration tests

**Files:**
- Create: `tests/bitemporal_test.rs`

- [ ] **Step 1: Create `tests/bitemporal_test.rs`** with the following test cases:

```rust
use minigraf::{DatalogExecutor, FactStorage};
// ... imports ...

/// Helper: execute a query string against an executor
fn exec(executor: &DatalogExecutor, input: &str) -> QueryResult {
    let cmd = parse_command(input).expect("parse error");
    executor.execute(cmd).expect("execution error")
}

#[test]
fn test_tx_time_travel_via_counter() {
    let storage = FactStorage::new();
    let executor = DatalogExecutor::new(storage);

    exec(&executor, "(transact [[:alice :person/name \"Alice\"]])");
    exec(&executor, "(transact [[:alice :person/name \"Alice Smith\"]])");

    // :as-of 1 should see only the first name
    let result = exec(&executor,
        "(query [:find ?name :as-of 1 :where [:alice :person/name ?name]])");
    let rows = result_rows(result);
    assert_eq!(rows.len(), 1);
    assert!(rows[0][0] == Value::String("Alice".to_string()));
}

#[test]
fn test_tx_time_travel_via_timestamp() {
    // transact, record timestamp, sleep, transact again, query :as-of first timestamp
    // ... assert only first fact visible ...
}

#[test]
fn test_valid_at_inside_range() {
    // transact with valid_from=2023-01-01, valid_to=2023-06-30
    // query :valid-at "2023-03-01" → match
}

#[test]
fn test_valid_at_outside_range() {
    // transact with valid_from=2023-01-01, valid_to=2023-06-30
    // query :valid-at "2024-01-01" → no match
}

#[test]
fn test_no_valid_at_returns_only_current() {
    // transact expired fact and forever fact
    // default query → only forever fact returned
}

#[test]
fn test_valid_at_any_valid_time_returns_all() {
    // transact expired and forever facts
    // :valid-at :any-valid-time → both returned
}

#[test]
fn test_bitemporal_combined_query() {
    // transact, then transact correction with past valid time
    // query :valid-at past-date :as-of recent-tx → correct historical answer
}

#[test]
fn test_migration_v1_file_defaults() {
    // Write a v1-format .graph file to a temp MemoryBackend
    // Open via PersistentFactStorage → verify migrated facts have correct defaults
    // ... (same helper as in Task 5, but as a higher-level integration test) ...
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --test bitemporal_test
```

Expected: all pass.

- [ ] **Step 3: Run full suite**

```bash
cargo test
```

Expected: ≥150 tests pass (123 original + ~30 new).

- [ ] **Step 4: Commit**

```bash
git add tests/bitemporal_test.rs
git commit -m "test: add bi-temporal integration tests"
```

---

## Task 11: REPL help text and CHANGELOG

**Files:**
- Modify: `src/repl.rs`
- Create or modify: `CHANGELOG.md`

- [ ] **Step 1: Update REPL help text in `src/repl.rs`**

Find the `(help)` output section and add:

```
Temporal queries:
  (query [:find ?x :as-of 50 :where ...])          - state as of tx counter 50
  (query [:find ?x :as-of "2024-01-15T10:00:00Z" :where ...]) - state as of timestamp
  (query [:find ?x :valid-at "2023-06-01" :where ...])  - facts valid on date
  (query [:find ?x :valid-at :any-valid-time :where ...]) - all facts, ignoring validity

  Note: queries without :valid-at return only currently valid facts.

Transact with valid time:
  (transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [...])
  (transact {:valid-from "2023-01-01"} [[:e :a :v {:valid-to "2023-03-01"}] ...])
```

- [ ] **Step 2: Create/update `CHANGELOG.md`**

```markdown
## [0.4.0] - 2026-03-21

### Added
- Bi-temporal support: every fact now carries transaction time (`tx_id`, `tx_count`)
  and valid time (`valid_from`, `valid_to`)
- `:as-of N` query modifier for transaction time travel (counter or ISO 8601 timestamp)
- `:valid-at "date"` query modifier for valid time point-in-time queries
- `:valid-at :any-valid-time` to disable valid time filtering
- `(transact {:valid-from ... :valid-to ...} [...])` syntax for specifying valid time
- Per-fact valid time override in transact (4-element fact vectors)
- File format version 2 with automatic migration from version 1

### Changed
- **Breaking behaviour**: queries without `:valid-at` now return only currently valid
  facts (`valid_from <= now < valid_to`). Existing Phase 3 databases are unaffected
  because all migrated facts have `valid_to = MAX`.
- `FactStorage::transact()` now accepts an optional `TransactOptions` parameter

### Fixed
- `PersistentFactStorage::load()` previously discarded original `tx_id` when loading
  facts from disk, making time-travel queries on persisted databases incorrect
```

- [ ] **Step 3: Run full suite one final time**

```bash
cargo test
```

Expected: ≥150 tests pass, 0 failures.

- [ ] **Step 4: Final commit**

```bash
git add src/repl.rs CHANGELOG.md
git commit -m "docs: update REPL help and CHANGELOG for Phase 4 bi-temporal support"
```

---

## Done

At this point all Phase 4 deliverables are complete:
- ✅ Bi-temporal `Fact` struct
- ✅ `tx_counter`, `load_fact()`, temporal query methods on `FactStorage`
- ✅ File format v2 with v1 migration
- ✅ EDN map parsing
- ✅ `:as-of` and `:valid-at` query clauses
- ✅ Temporal filtering in executor
- ✅ ≥150 tests passing
- ✅ CHANGELOG and REPL help updated
