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

## Piece 2 — Fix `filter_facts_for_query`

**File:** `src/query/datalog/executor.rs`

Locate Step 2 of the temporal filter (the `retain(|f| f.asserted)` call). Replace it with:

```rust
// Step 2: compute net-asserted view — retracted facts are excluded
let facts = crate::graph::storage::net_asserted_facts(facts);
```

The surrounding Steps 1 and 3 (tx-time window and valid-time window) are unchanged.

---

## Piece 3 — Tests

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

### `tests/edge_cases_test.rs` — 2 scenarios

**Test: oversized fact returns `Err` on checkpoint, not panic**

A packed page slot must fit within a 4 KB page. `pack_facts` is called during checkpoint/save, not during in-memory insert — so this test requires a **file-backed** database and must trigger a checkpoint to reach the error path. (A unit test for `pack_facts` directly already exists in `src/storage/packed_pages.rs`; this is the integration-level complement.)

Generate a string value of ~8 KB, transact it (which succeeds in-memory), then checkpoint and expect the error:

```rust
#[test]
fn test_oversized_fact_returns_err_on_checkpoint_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new().path(path.to_str().unwrap()).open().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    db.execute(&cmd).unwrap(); // insert succeeds in-memory
    let result = db.checkpoint();
    assert!(result.is_err(), "oversized fact must return Err on checkpoint, not panic");
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
- Verify Tests 1–6 **fail** before the fix is applied and **pass** after (TDD).
- Verify the edge case tests pass independently of the retraction fix.

---

## Commit Strategy

1. `test(retraction): add 6 failing retraction scenario tests` — tests first (TDD)
2. `fix(executor): extract net_asserted_facts, fix filter_facts_for_query Step 2, update get_current_value`
3. `test(edge): add oversized-fact and checkpoint-during-crash tests`
