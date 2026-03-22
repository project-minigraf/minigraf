# Phase 6.4a — Retraction Semantics Fix + Edge Case Tests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix a known correctness bug where retracted facts remain visible in Datalog query results, extract shared retraction logic as a single source of truth, and add tests for retraction semantics plus two additional edge cases (oversized facts, checkpoint-during-crash).

**Architecture:** Three coherent pieces: (1) extract `net_asserted_facts` helper in the storage layer, (2) apply it in the query executor to fix Step 2 of `filter_facts_for_query`, (3) add integration tests covering retraction scenarios and edge cases.

**Tech Stack:** Rust, existing `minigraf` crate, no new dependencies.

---

## Background

### The Bug

`filter_facts_for_query` in `src/query/datalog/executor.rs` applies a 3-step temporal filter:

1. **tx-time window** — discard facts outside the `:as-of` tx_count range
2. **asserted exclusion** — keep only `asserted=true` facts ← **BUG HERE**
3. **valid-time window** — discard facts outside the `:valid-at` range

Step 2 currently does:
```rust
facts.retain(|f| f.asserted);
```

This discards retraction records (`asserted=false`) without checking whether they cancel an earlier assertion. The original assertion record (with `asserted=true`) remains in the append-only log and is still returned — retracted facts stay visible.

### Correct Semantics

For each unique `(entity, attribute, value)` triple in the fact set, the net view is:
- **Asserted** if the record with the highest `tx_count` for that triple has `asserted=true`
- **Retracted** (absent from results) if that record has `asserted=false`

### Existing Implementation in `get_current_value`

`get_current_value()` in `src/graph/storage.rs` handles this correctly today but uses `tx_id` (a UUID derived from system time) for ordering — it sorts by `tx_id` descending and checks the first record. This is weaker than `tx_count` because UUIDs are only millisecond-granular: two transactions within the same millisecond may sort non-deterministically.

The fix promotes `tx_count` as the canonical ordering key for both call sites, which is strictly correct (monotonically incremented per transaction). This is a deliberate semantic improvement to `get_current_value` as well, not just a side effect. After the refactor, the 2 ms `sleep` in the `test_fact_storage_get_current_value` unit test (which was inserted to force distinct `tx_id` timestamps) becomes unnecessary and should be removed.

---

## File Structure

| File | Change |
|---|---|
| `src/graph/storage.rs` | Extract `net_asserted_facts` helper; update `get_current_value` to use it; remove `sleep` from affected unit test |
| `src/query/datalog/executor.rs` | Replace Step 2 `retain(asserted)` with `net_asserted_facts` call |
| `src/storage/packed_pages.rs` | Export `MAX_FACT_BYTES` constant; no logic changes |
| `src/db.rs` | Add early size validation in the file-backed transact path |
| `tests/retraction_test.rs` | New — 6 retraction scenario integration tests |
| `tests/edge_cases_test.rs` | New — oversized-fact and checkpoint-during-crash tests |

---

## Piece 1 — `net_asserted_facts` helper

**File:** `src/graph/storage.rs`

Extract a module-level `pub(crate)` function. Use `encode_value` from `crate::storage::index` as the canonical byte-level key for the value component — this handles `Float` NaN canonicalisation and `±0.0` disambiguation correctly, consistent with how facts are keyed throughout the storage layer.

```rust
/// Compute the net-asserted view of a fact set.
///
/// For each unique (entity, attribute, value) triple, keeps the fact only if
/// the record with the highest `tx_count` for that triple has `asserted=true`.
/// Facts whose most recent record is a retraction are excluded entirely.
///
/// Uses `encode_value` for the value key to ensure correct handling of
/// floating-point edge cases (NaN canonicalisation, ±0.0 disambiguation).
///
/// This is the single source of truth for retraction semantics, used by both
/// `get_current_value` and `filter_facts_for_query`.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use std::collections::HashMap;
    use crate::storage::index::encode_value;
    // key: (entity, attribute, canonical value bytes) → highest-tx_count fact
    let mut latest: HashMap<(EntityId, Attribute, Vec<u8>), Fact> = HashMap::new();
    for fact in facts {
        let key = (fact.entity, fact.attribute.clone(), encode_value(&fact.value));
        match latest.get(&key) {
            None => { latest.insert(key, fact); }
            Some(existing) if fact.tx_count > existing.tx_count => {
                latest.insert(key, fact);
            }
            _ => {}
        }
    }
    latest.into_values().filter(|f| f.asserted).collect()
}
```

**Update `get_current_value`** to call `net_asserted_facts` internally instead of its inline sort+check. The ordering now uses `tx_count` (via the helper) rather than `tx_id`. Remove the `std::thread::sleep(Duration::from_millis(2))` call from `test_fact_storage_get_current_value` — it was inserted to force distinct `tx_id` timestamps, which are no longer the ordering criterion.

---

## Piece 2 — Fact size limit: constant, early validation, and documentation

### 2a — Export `MAX_FACT_BYTES` constant

**File:** `src/storage/packed_pages.rs`

The existing check computes `max_slot_size` inline. Promote it to a named public constant so callers can reference it:

```rust
/// Maximum serialised size (postcard bytes) for a single fact stored in a packed page.
///
/// Derived from the page layout: `PAGE_SIZE (4096) - PACKED_HEADER_SIZE (12) - 4` (one
/// directory entry). Facts whose postcard serialisation exceeds this limit cannot be
/// persisted to a `.graph` file and will be rejected at insertion time.
///
/// In practice the usable space for `Value` content is roughly 3900–4000 bytes,
/// after accounting for the fixed overhead of the other `Fact` fields (two UUIDs,
/// attribute string, counters, timestamps, boolean).
pub const MAX_FACT_BYTES: usize = PAGE_SIZE - PACKED_HEADER_SIZE - 4; // 4080
```

Update the existing `bail!` in `pack_facts` to reference this constant rather than recomputing it inline.

### 2b — Early size validation in `db.rs`

**File:** `src/db.rs`

For file-backed databases, validate the serialised fact size before writing to the WAL. Facts are serialised in **two separate code paths** that must both be guarded:

**Path 1 — implicit `execute()` transactions** (the common path): after facts are stamped and before `wal_write_stamped_batch` is called, iterate over the stamped batch and check each fact.

**Path 2 — explicit `WriteTransaction::commit()`**: after the stamping loop inside `commit()` and before the `wal_write_stamped_batch` call in that path, apply the same check.

Both paths use the same helper:

```rust
use crate::storage::packed_pages::MAX_FACT_BYTES;

fn check_fact_sizes(facts: &[Fact]) -> anyhow::Result<()> {
    for fact in facts {
        let bytes = postcard::to_allocvec(fact)
            .map_err(|e| anyhow::anyhow!("Failed to serialise fact: {}", e))?;
        if bytes.len() > MAX_FACT_BYTES {
            anyhow::bail!(
                "Fact serialised size {} bytes exceeds maximum {} bytes. \
                 Store large payloads externally and reference them with a \
                 Value::String URL/path or Value::Ref entity ID.",
                bytes.len(),
                MAX_FACT_BYTES
            );
        }
    }
    Ok(())
}
```

Call `check_fact_sizes(&stamped)?` in both paths, before the WAL write.

This check applies to **file-backed databases only**. In-memory databases (`Minigraf::in_memory()`) have no page size constraint and do not enforce this limit — large facts are accepted without error. The guard is implemented by checking `WriteContext` (or equivalent) before invoking `check_fact_sizes`.

**Note on double serialisation:** `check_fact_sizes` serialises each fact once for validation; `pack_facts` serialises again during checkpoint. This redundancy is acceptable — the check only fires on the error path (oversized facts are rare), and the fast path (normal-sized facts) pays no extra cost at checkpoint time since `pack_facts` was always going to serialise them.

### 2c — Documentation

**Rustdoc on `Minigraf::execute` / `OpenOptions`** (`src/db.rs`):

Add a `# Fact Size Limit` section to the rustdoc of `Minigraf` (or a top-level doc comment visible from `cargo doc`):

```
# Fact Size Limit (file-backed databases only)

Each fact stored in a `.graph` file must serialise to at most
[`MAX_FACT_BYTES`] bytes (currently 4080). This limit applies to the
postcard-encoded representation of the full [`Fact`] struct, including
entity ID, attribute name, value, and all temporal fields.

In practice, `Value::String` and `Value::Keyword` content is limited to
roughly **3900–4000 bytes** depending on entity and attribute name lengths.

**Workarounds for large payloads:**

- **External blob reference**: store the large payload in a file or object
  store and record its path, URL, or content-addressed hash as a
  `Value::String`. Example:
  ```
  (transact [[:doc123 :blob/sha256 "a3f5..."]])
  ```
- **Entity decomposition**: split the large value across multiple facts
  using a continuation-entity pattern.
- **In-memory database**: `Minigraf::in_memory()` has no fact size limit
  and is suitable for workloads that do not require persistence.

This limit does not apply to recursive rule derivations or query results —
only to facts written via `transact`.
```

**`README.md`** (`## Limitations` section, or append to `## Performance`):

Add a brief note:

```
### Fact Size Limit

File-backed databases enforce a maximum fact size of 4080 serialised bytes
per fact (roughly 3900–4000 bytes of string content after fixed overhead).
Facts that exceed this are rejected at insertion time with a clear error
message. Use `Value::String` to store a reference to an external blob for
large payloads. In-memory databases have no size limit.
```

---

## Piece 3 — Fix `filter_facts_for_query`

**File:** `src/query/datalog/executor.rs`

Locate Step 2 of the temporal filter (the `retain(|f| f.asserted)` call). Replace it with:

```rust
// Step 2: compute net-asserted view — retracted facts are excluded
let facts = crate::graph::storage::net_asserted_facts(facts);
```

The surrounding Steps 1 and 3 (tx-time window and valid-time window) are unchanged.

---

## Piece 4 — Tests

### `tests/retraction_test.rs` — 6 scenarios

All tests use `Minigraf::in_memory()`.

**Test 1 — Basic retraction:**
```
transact [[:alice :age 30]]
retract [[:alice :age 30]]
query [:find ?v :where [:alice :age ?v]]
→ empty result
```

**Test 2 — `:as-of` before and after retraction:**
```
transact [[:alice :age 30]]   ; tx_count = 1
transact [[:alice :age 31]]   ; tx_count = 2  (separate assertion, not an update)
retract  [[:alice :age 30]]   ; tx_count = 3
query [:find ?v :as-of 2 :where [:alice :age ?v]]
  → contains 30 (and also 31; both are asserted within the tx window)
query [:find ?v :as-of 3 :where [:alice :age ?v]]
  → does NOT contain 30 (retraction at tx_count=3 is now in window); contains 31
```

**Test 3 — Assert → retract → re-assert:**
```
transact [[:alice :status :active]]
retract  [[:alice :status :active]]
transact [[:alice :status :active]]
query [:find ?s :where [:alice :status ?s]]
→ contains :active
```

**Test 4 — Retraction + `:any-valid-time`:**
```
transact {:valid-from "2023-01-01"} [[:alice :role :engineer]]
retract  [[:alice :role :engineer]]
query [:find ?r :any-valid-time :where [:alice :role ?r]]
→ empty result  (retraction supersedes all valid-time windows)
```

**Test 5 — Retraction + recursive rule, `:as-of` before retraction (fact visible in derivation):**
```
transact [[:a :next :b] [:b :next :c]]   ; tx_count = 1
retract  [[:a :next :b]]                 ; tx_count = 2
rule [(reach ?x ?y) [?x :next ?y]]
rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)]
query [:find ?to :as-of 1 :where (reach :a ?to)]
→ contains :b and :c
```

**Test 6 — Retraction + recursive rule, `:as-of` after retraction (fact excluded from derivation):**
```
(same setup as Test 5)
query [:find ?to :as-of 2 :where (reach :a ?to)]
→ does NOT contain :b (retracted at tx_count=2, within window); does NOT contain :c
  Note: [:b :next :c] remains asserted in the store — :c is simply unreachable
  from :a because the :a→:b link is broken. The chain is broken at its first hop.
```

---

### `tests/edge_cases_test.rs` — 4 scenarios

**Test: oversized fact rejected at insertion time (file-backed)**

With the early size validation in Piece 2b, `execute()` must return `Err` immediately for a file-backed database when the fact exceeds `MAX_FACT_BYTES`. No checkpoint call is needed.

```rust
#[test]
fn test_oversized_fact_rejected_at_insertion_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new().path(path.to_str().unwrap()).open().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    let result = db.execute(&cmd);
    assert!(result.is_err(), "oversized fact must be rejected at insertion for file-backed DB");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("4080"), "error message must cite the byte limit");
}
```

**Test: oversized fact rejected via `WriteTransaction::commit()` (file-backed)**

Covers the explicit write transaction code path — the validation must also fire here:

```rust
#[test]
fn test_oversized_fact_rejected_via_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new().path(path.to_str().unwrap()).open().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    let mut tx = db.begin_write().unwrap();
    tx.execute(&cmd).unwrap(); // buffered, not yet validated
    let result = tx.commit();
    assert!(result.is_err(), "oversized fact must be rejected at commit for file-backed DB");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("4080"), "error message must cite the byte limit");
}
```

**Test: oversized fact accepted in-memory (no page size constraint)**

In-memory databases have no fact size limit — the same large fact must be accepted:

```rust
#[test]
fn test_oversized_fact_accepted_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    assert!(db.execute(&cmd).is_ok(), "oversized fact must be accepted in in-memory DB");
}
```

**Test: checkpoint-during-crash (stale WAL replay is idempotent)**

Simulates a crash that left the pre-checkpoint WAL on disk after a successful checkpoint by manually restoring the stale WAL file. Uses `tempfile::tempdir()` consistent with the rest of the test suite.

```rust
#[test]
fn test_stale_wal_after_checkpoint_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let path = path.to_str().unwrap();
    let wal_path = format!("{}.wal", path);

    // Phase 1: insert alice via WAL only (checkpoint suppressed), save WAL bytes
    {
        let db = OpenOptions {
            wal_checkpoint_threshold: usize::MAX,
            ..Default::default()
        }
        .path(path)
        .open()
        .unwrap();
        db.execute("(transact [[:alice :age 30]])").unwrap();
    }
    let stale_wal = std::fs::read(&wal_path).expect("WAL must exist after insert");

    // Phase 2: reopen, insert bob, checkpoint (writes alice+bob to packed pages, deletes WAL)
    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute("(transact [[:bob :age 40]])").unwrap();
        db.checkpoint().unwrap();
    }
    assert!(
        !std::path::Path::new(&wal_path).exists(),
        "WAL must be gone after checkpoint"
    );

    // Phase 3: restore stale WAL — simulates crash that skipped WAL deletion
    std::fs::write(&wal_path, &stale_wal).unwrap();

    // Phase 4: reopen — WAL replay must skip alice (already in packed pages, tx_count ≤ last_checkpointed_tx_count)
    let db = OpenOptions::new().path(path).open().unwrap();

    // Alice must appear exactly once
    let alice_result = db
        .execute("(query [:find ?age :where [:alice :age ?age]])")
        .unwrap();
    let alice_rows = match alice_result {
        minigraf::QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    };
    assert_eq!(alice_rows.len(), 1, "alice:age must appear exactly once after stale WAL replay");

    // Bob must be present (survived checkpoint)
    let bob_result = db
        .execute("(query [:find ?age :where [:bob :age ?age]])")
        .unwrap();
    let bob_rows = match bob_result {
        minigraf::QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    };
    assert_eq!(bob_rows.len(), 1, "bob:age must survive checkpoint");
}
```

---

## Testing Strategy

- All new tests use `Minigraf::in_memory()` or `OpenOptions::new().path(...)`.
- Run `cargo test` after each piece is implemented.
- Verify Tests 1–6 **fail** before the retraction fix is applied and **pass** after (TDD).
- Verify edge case tests pass independently of the retraction fix.
- Verify the oversized-fact file-backed test fails before Piece 2b (early validation) is added and passes after.
- Verify the oversized-fact in-memory test passes before and after Piece 2b (no regression).

---

## Commit Strategy

1. `test(retraction): add 6 failing retraction scenario tests` — tests first (TDD)
2. `fix(executor): extract net_asserted_facts, fix filter_facts_for_query Step 2, update get_current_value`
3. `feat(storage): export MAX_FACT_BYTES, add early size validation for file-backed inserts`
4. `docs: document fact size limit and external blob workarounds`
5. `test(edge): add oversized-fact (file-backed + in-memory) and checkpoint-during-crash tests`
