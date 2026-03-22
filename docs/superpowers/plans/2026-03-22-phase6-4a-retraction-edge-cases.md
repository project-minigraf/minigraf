# Phase 6.4a — Retraction Semantics Fix + Edge Case Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the retraction semantics bug in `filter_facts_for_query`, extract shared net-view logic as a single source of truth, add early fact-size validation for file-backed inserts, and cover both with integration tests.

**Architecture:** `net_asserted_facts` is added to `src/graph/storage.rs` and used by both `get_current_value` (fixing its ordering key from `tx_id` to `tx_count`) and `filter_facts_for_query` (fixing Step 2 to compute net view instead of naively filtering `asserted=true`). A `check_fact_sizes` guard is added to both write paths in `src/db.rs` before the WAL write. A `MAX_FACT_BYTES` constant is exported from `src/storage/packed_pages.rs`.

**Tech Stack:** Rust, postcard (already a dependency), no new crates.

---

## File Map

| File | Role |
|---|---|
| `src/storage/packed_pages.rs` | Export `MAX_FACT_BYTES = 4080`; update inline check to reference it |
| `src/graph/storage.rs` | Add `net_asserted_facts` helper; refactor `get_current_value` to use it; remove `tx_id`-ordering sleeps from one unit test |
| `src/query/datalog/executor.rs` | Replace Step 2 (`filter(is_asserted)`) with `net_asserted_facts` |
| `src/db.rs` | Add `check_fact_sizes` helper; call it in both `execute()` and `WriteTransaction::commit()` before WAL write; add rustdoc for size limit |
| `README.md` | Add `### Fact Size Limit` note under `## Performance` (or a new `## Limitations` section) |
| `tests/retraction_test.rs` | New — 6 retraction scenario integration tests |
| `tests/edge_cases_test.rs` | New — 4 edge case tests (2 oversized-fact, 1 WriteTransaction, 1 checkpoint-during-crash) |

---

## Task 1: Export `MAX_FACT_BYTES` from `packed_pages.rs`

**Files:**
- Modify: `src/storage/packed_pages.rs:33,58-64`

This is a pure refactor — same value, just named. The existing unit test at line 254 continues to verify the error path.

- [ ] **Step 1: Add the constant after `PACKED_HEADER_SIZE`**

In `src/storage/packed_pages.rs`, after line 33 (`pub const PACKED_HEADER_SIZE: usize = 12;`), add:

```rust
/// Maximum serialised size (postcard bytes) for a single fact in a packed page.
///
/// Derived from the page layout: `PAGE_SIZE (4096) - PACKED_HEADER_SIZE (12) - 4`
/// (4 bytes for one record-directory entry).
///
/// In practice the usable space for a `Value::String` is roughly 3 900–4 000 bytes
/// after accounting for the fixed overhead of the other `Fact` fields (two UUIDs,
/// attribute string, counters, timestamps, boolean flag).
///
/// File-backed databases reject facts that exceed this limit at insertion time.
/// In-memory databases (`Minigraf::in_memory()`) have no size constraint.
pub const MAX_FACT_BYTES: usize = PAGE_SIZE - PACKED_HEADER_SIZE - 4;
```

- [ ] **Step 2: Replace the inline computation in `pack_facts`**

In `src/storage/packed_pages.rs`, replace lines 57–64:
```rust
        // Check if this fact exceeds the maximum slot size.
        let max_slot_size = PAGE_SIZE - PACKED_HEADER_SIZE - 4; // 4 = one directory entry
        if len > max_slot_size {
            anyhow::bail!(
                "Fact serialised size {} bytes exceeds maximum slot size {} bytes",
                len,
                max_slot_size
            );
        }
```
with:
```rust
        // Check if this fact exceeds the maximum slot size.
        if len > MAX_FACT_BYTES {
            anyhow::bail!(
                "Fact serialised size {} bytes exceeds maximum slot size {} bytes",
                len,
                MAX_FACT_BYTES
            );
        }
```

- [ ] **Step 3: Run tests to verify no regression**

```bash
cargo test --lib storage::packed_pages
```
Expected: all tests pass (including `test_oversized_fact_returns_error`).

- [ ] **Step 4: Commit**

```bash
git add src/storage/packed_pages.rs
git commit -m "refactor(storage): export MAX_FACT_BYTES constant from packed_pages"
```

---

## Task 2: Write 6 failing retraction integration tests

**Files:**
- Create: `tests/retraction_test.rs`

Write the tests **first** — they must fail before the fix. This is the TDD anchor for Tasks 3 and 4.

- [ ] **Step 1: Create `tests/retraction_test.rs`**

```rust
//! Integration tests for retraction semantics in Datalog queries.
//!
//! These tests verify that `filter_facts_for_query` computes the net-asserted
//! view per (entity, attribute, value) triple, correctly hiding retracted facts.

use minigraf::Minigraf;

// ── Test 1: Basic retraction ──────────────────────────────────────────────────

#[test]
fn test_retraction_hides_fact_from_query() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap();
    db.execute("(retract [[:alice :age 30]])").unwrap();
    let result = db
        .execute("(query [:find ?v :where [:alice :age ?v]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("[]") || !format!("{:?}", result).contains("30"),
        "retracted fact must not appear in query results"
    );
}

// ── Test 2: as-of before and after retraction ─────────────────────────────────

#[test]
fn test_retraction_as_of_before_shows_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap(); // tx_count = 1
    db.execute("(transact [[:alice :age 31]])").unwrap(); // tx_count = 2
    db.execute("(retract [[:alice :age 30]])").unwrap();  // tx_count = 3

    // as-of 2: retraction not yet in window — fact 30 must appear
    let result = db
        .execute("(query [:find ?v :as-of 2 :where [:alice :age ?v]])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("30"), "fact 30 must appear when as-of precedes retraction");
}

#[test]
fn test_retraction_as_of_after_hides_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :age 30]])").unwrap(); // tx_count = 1
    db.execute("(transact [[:alice :age 31]])").unwrap(); // tx_count = 2
    db.execute("(retract [[:alice :age 30]])").unwrap();  // tx_count = 3

    // as-of 3: retraction is in window — fact 30 must not appear
    let result = db
        .execute("(query [:find ?v :as-of 3 :where [:alice :age ?v]])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(!s.contains("30"), "fact 30 must not appear when as-of includes retraction");
    assert!(s.contains("31"), "fact 31 must still appear (it was not retracted)");
}

// ── Test 3: Assert → retract → re-assert ─────────────────────────────────────

#[test]
fn test_retraction_then_reassert() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:alice :status :active]])").unwrap();
    db.execute("(retract [[:alice :status :active]])").unwrap();
    db.execute("(transact [[:alice :status :active]])").unwrap();

    let result = db
        .execute("(query [:find ?s :where [:alice :status ?s]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("active"),
        "re-asserted fact must appear after retract+reassert"
    );
}

// ── Test 4: Retraction + :any-valid-time ─────────────────────────────────────

#[test]
fn test_retraction_with_any_valid_time() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        r#"(transact {:valid-from "2023-01-01"} [[:alice :role :engineer]])"#,
    )
    .unwrap();
    db.execute("(retract [[:alice :role :engineer]])").unwrap();

    let result = db
        .execute("(query [:find ?r :any-valid-time :where [:alice :role ?r]])")
        .unwrap();
    assert!(
        !format!("{:?}", result).contains("engineer"),
        "retracted fact must not appear even with :any-valid-time"
    );
}

// ── Test 5: Retraction + recursive rule — as-of before retraction ─────────────

#[test]
fn test_retraction_rule_as_of_before_sees_fact() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c]])").unwrap(); // tx_count = 1
    db.execute("(retract [[:a :next :b]])").unwrap();                 // tx_count = 2
    db.execute("(rule [(reach ?x ?y) [?x :next ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)])").unwrap();

    let result = db
        .execute("(query [:find ?to :as-of 1 :where (reach :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(s.contains("b"), "b must be reachable from a at as-of 1");
    assert!(s.contains("c"), "c must be reachable from a at as-of 1 (via b)");
}

// ── Test 6: Retraction + recursive rule — as-of after retraction ──────────────

#[test]
fn test_retraction_rule_as_of_after_breaks_chain() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c]])").unwrap(); // tx_count = 1
    db.execute("(retract [[:a :next :b]])").unwrap();                 // tx_count = 2
    db.execute("(rule [(reach ?x ?y) [?x :next ?y]])").unwrap();
    db.execute("(rule [(reach ?x ?y) [?x :next ?m] (reach ?m ?y)])").unwrap();

    // as-of 2: retraction is in window — :a→:b link is broken
    // Note: [:b :next :c] remains asserted but :c is unreachable from :a
    let result = db
        .execute("(query [:find ?to :as-of 2 :where (reach :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(!s.contains("\"b\"") && !s.contains(":b"),
        "b must not be reachable from a after retraction of :a→:b");
    assert!(!s.contains("\"c\"") && !s.contains(":c"),
        "c must not be reachable from a: chain is broken at :a→:b");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test --test retraction_test 2>&1 | grep -E "FAILED|ok|error"
```
Expected: Tests 1, 3, 4, 5, 6 fail (retracted facts are still visible). Tests 2 (as-of-before) may pass since the retraction isn't in the tx window.

- [ ] **Step 3: Commit the failing tests**

```bash
git add tests/retraction_test.rs
git commit -m "test(retraction): add 6 failing retraction scenario tests (TDD)"
```

---

## Task 3: Add `net_asserted_facts` helper + update `get_current_value`

**Files:**
- Modify: `src/graph/storage.rs:1,414-432,677-731`

- [ ] **Step 1: Add `encode_value` to the existing import on line 5**

Change line 5 from:
```rust
use crate::storage::index::{FactRef, Indexes};
```
to:
```rust
use crate::storage::index::{FactRef, Indexes, encode_value};
```

- [ ] **Step 2: Add `net_asserted_facts` after `get_current_value` (after line 432)**

Insert this function immediately after the closing `}` of `get_current_value` (line 432):

```rust
/// Compute the net-asserted view of a fact set.
///
/// For each unique `(entity, attribute, value)` triple, keeps the fact only if
/// the record with the highest `tx_count` for that triple has `asserted=true`.
/// Facts whose most recent record is a retraction are excluded entirely.
///
/// Uses [`encode_value`] for the value key to handle floating-point edge cases
/// (NaN canonicalisation, ±0.0 disambiguation) consistently with the rest of
/// the storage layer.
///
/// This is the single source of truth for retraction semantics, shared by
/// `get_current_value` and `filter_facts_for_query`.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use std::collections::HashMap;
    // key: (entity, attribute, canonical value bytes) → fact with highest tx_count
    let mut latest: HashMap<(EntityId, Attribute, Vec<u8>), Fact> = HashMap::new();
    for fact in facts {
        let key = (fact.entity, fact.attribute.clone(), encode_value(&fact.value));
        match latest.get(&key) {
            None => {
                latest.insert(key, fact);
            }
            Some(existing) if fact.tx_count > existing.tx_count => {
                latest.insert(key, fact);
            }
            _ => {}
        }
    }
    latest.into_values().filter(|f| f.asserted).collect()
}
```

- [ ] **Step 3: Refactor `get_current_value` (lines 414–432) to use the helper**

Replace the body of `get_current_value`:
```rust
    pub fn get_current_value(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Option<Value>> {
        let mut relevant_facts = self.get_facts_by_entity_attribute(entity_id, attribute)?;

        // Sort by transaction ID (timestamp) descending
        relevant_facts.sort_by(|a, b| b.tx_id.cmp(&a.tx_id));

        // Return the value if the most recent fact is an assertion
        Ok(relevant_facts.first().and_then(|f| {
            if f.is_asserted() {
                Some(f.value.clone())
            } else {
                None
            }
        }))
    }
```
with:
```rust
    pub fn get_current_value(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Option<Value>> {
        let relevant_facts = self.get_facts_by_entity_attribute(entity_id, attribute)?;
        // net_asserted_facts groups by (entity, attribute, value) triple and keeps
        // only triples whose most-recent record (by tx_count) is an assertion.
        // Sort the survivors by tx_count descending to return the latest value.
        let mut net = net_asserted_facts(relevant_facts);
        net.sort_by(|a, b| b.tx_count.cmp(&a.tx_count));
        Ok(net.first().map(|f| f.value.clone()))
    }
```

- [ ] **Step 4: Remove the two `sleep` calls from `test_fact_storage_get_current_value`**

In `src/graph/storage.rs`, inside `test_fact_storage_get_current_value` (starting at line 677), remove both `std::thread::sleep` calls (lines 695 and 715). The test now relies on `tx_count` ordering which is monotonically incremented — no timing dependency.

The test structure after removal:
```rust
    fn test_fact_storage_get_current_value() {
        use uuid::Uuid;

        let storage = FactStorage::new();
        let alice = Uuid::new_v4();

        // Set initial value
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        // Update value
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice Smith".to_string()),
                )],
                None,
            )
            .unwrap();

        // Current value should be the most recent
        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, Some(Value::String("Alice Smith".to_string())));

        // Retract the value
        storage
            .retract(vec![(
                alice,
                ":person/name".to_string(),
                Value::String("Alice Smith".to_string()),
            )])
            .unwrap();

        // Current value should now be None (retracted)
        let current = storage
            .get_current_value(&alice, &":person/name".to_string())
            .unwrap();
        assert_eq!(current, None);
    }
```

- [ ] **Step 5: Run storage unit tests**

```bash
cargo test --lib graph::storage
```
Expected: all pass. If `test_fact_storage_get_current_value` fails, check that `net_asserted_facts` is calling the right comparison — `tx_count` not `tx_id`.

- [ ] **Step 6: Commit**

```bash
git add src/graph/storage.rs
git commit -m "feat(storage): add net_asserted_facts helper, refactor get_current_value to use tx_count ordering"
```

---

## Task 4: Fix `filter_facts_for_query` in `executor.rs`

**Files:**
- Modify: `src/query/datalog/executor.rs:139-143`

This single change fixes the retraction bug. Step 2 currently filters to `is_asserted()` — replace with the net-view computation.

- [ ] **Step 1: Replace Step 2 in `filter_facts_for_query` (lines 139–143)**

Current code:
```rust
        // Step 2: keep only asserted facts
        let asserted: Vec<Fact> = tx_filtered
            .into_iter()
            .filter(|f| f.is_asserted())
            .collect();
```

Replace with:
```rust
        // Step 2: compute net-asserted view — for each (entity, attribute, value) triple,
        // keep it only if the record with the highest tx_count is an assertion.
        // This correctly hides facts that have been retracted.
        let asserted = crate::graph::storage::net_asserted_facts(tx_filtered);
```

No other changes to this function — Steps 1 and 3 are unchanged.

- [ ] **Step 2: Run the retraction tests to verify they now pass**

```bash
cargo test --test retraction_test
```
Expected: all 6 tests pass.

- [ ] **Step 3: Run the full test suite to check for regressions**

```bash
cargo test
```
Expected: all tests pass (280+). If any existing tests fail, check whether they relied on the buggy behaviour of retracted facts appearing in queries.

- [ ] **Step 4: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "fix(executor): use net_asserted_facts in filter_facts_for_query Step 2 — fixes retraction visibility bug"
```

---

## Task 5: Add `check_fact_sizes` + early validation in `db.rs`

**Files:**
- Modify: `src/db.rs`

Two insertion points: `Minigraf::execute()` (line 370, before the WAL write) and `WriteTransaction::commit()` (line 660, before the WAL write). Both call the same shared helper.

- [ ] **Step 1: Add the `check_fact_sizes` helper function in `db.rs`**

Add this function anywhere before the `impl Minigraf` block (e.g. after the imports, around line 25). It must be `fn` (not `pub`) — it's internal:

```rust
/// Validate that every fact in `facts` can fit in a single packed-page slot.
///
/// Called before writing to the WAL so that oversized facts are rejected at
/// insertion time rather than at checkpoint time.  Only invoked for file-backed
/// databases — in-memory databases have no page size constraint.
fn check_fact_sizes(facts: &[Fact]) -> anyhow::Result<()> {
    use crate::storage::packed_pages::MAX_FACT_BYTES;
    for fact in facts {
        let bytes = postcard::to_allocvec(fact)
            .map_err(|e| anyhow::anyhow!("Failed to serialise fact for size check: {}", e))?;
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

You also need `postcard` in scope. Add to the existing imports in `db.rs`:
```rust
use postcard;
```

- [ ] **Step 2: Call `check_fact_sizes` in `Minigraf::execute()` (file-backed path)**

In `Minigraf::execute()`, after the stamped batch is built (after line 366) and **before** the call to `wal_write_stamped_batch` (line 370), insert the guard. The check must only fire for file-backed DBs — `WriteContext::Memory` has no size limit.

After line 366 (the `.collect()` closing the `stamped` vector), add:

```rust
            // For file-backed databases, reject oversized facts before touching the WAL.
            if matches!(ctx, WriteContext::File { .. }) {
                check_fact_sizes(&stamped)?;
            }
```

- [ ] **Step 3: Call `check_fact_sizes` in `WriteTransaction::commit()` (file-backed path)**

In `WriteTransaction::commit()`, after the stamping loop (after line 656, the `.collect()` closing `stamped`) and **before** the call to `Self::wal_write_stamped_batch` (line 660), add:

```rust
            // For file-backed databases, reject oversized facts before touching the WAL.
            if matches!(*self.guard, WriteContext::File { .. }) {
                check_fact_sizes(&stamped)?;
            }
```

- [ ] **Step 4: Run the full test suite**

```bash
cargo test
```
Expected: all tests still pass (the guard is only triggered when facts are actually oversized).

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "feat(db): reject oversized facts at insertion time for file-backed databases"
```

---

## Task 6: Add documentation for the fact size limit

**Files:**
- Modify: `src/db.rs` (rustdoc on `Minigraf`)
- Modify: `README.md`

- [ ] **Step 1: Add rustdoc to `Minigraf` in `src/db.rs`**

Find the `/// ` doc comment block above `pub struct Minigraf` (or above `impl Minigraf` if there's no struct doc). Add a `# Fact Size Limit` section. If the struct already has a doc comment, append to it; otherwise add a new one. Example addition:

```rust
/// # Fact Size Limit (file-backed databases only)
///
/// Each fact persisted to a `.graph` file must serialise to at most
/// [`crate::storage::packed_pages::MAX_FACT_BYTES`] bytes (currently 4 080).
///
/// In practice, `Value::String` content is limited to roughly **3 900–4 000 bytes**
/// depending on entity and attribute name lengths.
///
/// Facts that exceed this limit are rejected at insertion time with a descriptive
/// error. This check does **not** apply to `Minigraf::in_memory()`.
///
/// ## Workarounds for large payloads
///
/// - **External blob reference** — store the payload in a file or object store
///   and record its path, URL, or content-addressed hash as a `Value::String`:
///   ```text
///   (transact [[:doc123 :blob/sha256 "a3f5c9..."]])
///   ```
/// - **Entity decomposition** — split large values across multiple facts using
///   a continuation-entity pattern.
/// - **In-memory database** — `Minigraf::in_memory()` has no fact size limit
///   and is suitable for workloads that do not require persistence.
```

- [ ] **Step 2: Add a fact size limit note to `README.md`**

Find `## Performance` in `README.md` and append a `### Fact Size Limit` subsection after the performance table. If there is no `## Performance` section, add it before `## Roadmap`.

```markdown
### Fact Size Limit

File-backed databases enforce a maximum fact size of **4 080 serialised bytes**
per fact (roughly 3 900–4 000 bytes of string content after fixed overhead).
Facts that exceed this limit are rejected at insertion time with a clear error
message.

For large payloads, store a reference to the external data as a `Value::String`
(e.g. a file path, URL, or content hash) and keep the payload outside Minigraf.
In-memory databases have no size limit.
```

- [ ] **Step 3: Verify doc build is clean**

```bash
cargo doc --no-deps 2>&1 | grep -i "warning\|error"
```
Expected: no warnings or errors.

- [ ] **Step 4: Commit**

```bash
git add src/db.rs README.md
git commit -m "docs: document MAX_FACT_BYTES limit and external-blob workarounds"
```

---

## Task 7: Write edge case integration tests

**Files:**
- Create: `tests/edge_cases_test.rs`

These tests require no further implementation changes — they verify behavior that Tasks 1 and 5 already established.

- [ ] **Step 1: Create `tests/edge_cases_test.rs`**

```rust
//! Edge case integration tests for Phase 6.4a.
//!
//! Covers:
//! - Oversized fact rejected at insertion (file-backed, both execute() and commit() paths)
//! - Oversized fact accepted in-memory (no page size constraint)
//! - Stale WAL after checkpoint is replayed idempotently (no duplicate facts)

use minigraf::{Minigraf, OpenOptions, QueryResult};

// ── Oversized fact — file-backed, execute() path ──────────────────────────────

#[test]
fn test_oversized_fact_rejected_at_insertion_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new().path(path.to_str().unwrap()).open().unwrap();

    let large_value = "x".repeat(8192); // well above MAX_FACT_BYTES = 4080
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    let result = db.execute(&cmd);

    assert!(result.is_err(), "oversized fact must be rejected at insertion for file-backed DB");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("4080"),
        "error message must cite the 4080-byte limit; got: {}",
        msg
    );
}

// ── Oversized fact — explicit WriteTransaction path ────────────────────────────

#[test]
fn test_oversized_fact_rejected_via_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let db = OpenOptions::new().path(path.to_str().unwrap()).open().unwrap();

    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);

    let mut tx = db.begin_write().unwrap();
    tx.execute(&cmd).unwrap(); // buffered in-memory — not yet validated
    let result = tx.commit();  // size check fires here

    assert!(result.is_err(), "oversized fact must be rejected at commit for file-backed DB");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("4080"),
        "error message must cite the 4080-byte limit; got: {}",
        msg
    );
}

// ── Oversized fact — in-memory (no constraint) ────────────────────────────────

#[test]
fn test_oversized_fact_accepted_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    let large_value = "x".repeat(8192);
    let cmd = format!("(transact [[:e :attr \"{}\"]])", large_value);
    assert!(
        db.execute(&cmd).is_ok(),
        "oversized fact must be accepted in an in-memory database (no page size constraint)"
    );
}

// ── Checkpoint-during-crash: stale WAL replay is idempotent ───────────────────

#[test]
fn test_stale_wal_after_checkpoint_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.graph");
    let path = path.to_str().unwrap();
    let wal_path = format!("{}.wal", path);

    // Phase 1: insert alice with checkpoint suppressed; save WAL bytes
    {
        let db = OpenOptions {
            wal_checkpoint_threshold: usize::MAX,
            ..Default::default()
        }
        .path(path)
        .open()
        .unwrap();
        db.execute("(transact [[:alice :age 30]])").unwrap();
        // Drop without checkpointing — WAL exists
    }
    let stale_wal = std::fs::read(&wal_path).expect("WAL must exist after insert");

    // Phase 2: reopen (replays WAL → alice loaded), insert bob, checkpoint
    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute("(transact [[:bob :age 40]])").unwrap();
        db.checkpoint().unwrap(); // alice + bob → packed pages; WAL deleted
    }
    assert!(
        !std::path::Path::new(&wal_path).exists(),
        "WAL must be deleted after checkpoint"
    );

    // Phase 3: simulate crash — restore stale WAL (alice only, tx_count=1)
    std::fs::write(&wal_path, &stale_wal).unwrap();

    // Phase 4: reopen — WAL replay must skip alice (tx_count=1 ≤ last_checkpointed_tx_count)
    let db = OpenOptions::new().path(path).open().unwrap();

    let alice_result = db
        .execute("(query [:find ?age :where [:alice :age ?age]])")
        .unwrap();
    let alice_rows = match alice_result {
        QueryResult::QueryResults { results, .. } => results,
        other => panic!("expected QueryResults, got {:?}", other),
    };
    assert_eq!(
        alice_rows.len(),
        1,
        "alice:age must appear exactly once — stale WAL replay must be idempotent"
    );

    let bob_result = db
        .execute("(query [:find ?age :where [:bob :age ?age]])")
        .unwrap();
    let bob_rows = match bob_result {
        QueryResult::QueryResults { results, .. } => results,
        other => panic!("expected QueryResults, got {:?}", other),
    };
    assert_eq!(bob_rows.len(), 1, "bob:age must survive the checkpoint");
}
```

- [ ] **Step 2: Run all edge case tests**

```bash
cargo test --test edge_cases_test
```
Expected: all 4 pass.

- [ ] **Step 3: Run the full test suite one final time**

```bash
cargo test
```
Expected: all tests pass (286+ tests: 280 existing + 6 retraction + 4 edge cases).

- [ ] **Step 4: Commit**

```bash
git add tests/edge_cases_test.rs
git commit -m "test(edge): add oversized-fact and checkpoint-during-crash integration tests"
```
