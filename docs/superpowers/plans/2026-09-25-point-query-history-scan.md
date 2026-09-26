# Point-Query History Scan Mitigation (#323) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop bound-entity point queries from paying for other attributes' history, and cut the per-history-record constant on the churned attribute itself by ≥3×, without changing file format, API, or results.

**Architecture:** (1) an in-crate FxHash hasher; (2) `net_asserted_facts` rewritten to encode each value once, borrow keys, and use FxHash; (3) a new EAVT `(entity, attribute)` range lookup in `FactStorage`; (4) `selective_fact_fetch` narrows bound-entity lookups to the attributes the query references and drops its redundant dedup set.

**Tech Stack:** Rust 2024 edition, MSRV 1.89, criterion benches, cargo test.

**Spec:** `docs/superpowers/specs/2026-09-25-point-query-history-scan-design.md`

## Global Constraints

- No file-format change (stays v7), no public API change, no new dependencies (runtime or dev).
- Clippy workspace lints deny `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss` in non-test code. Tests are exempt (`src/lib.rs` `cfg_attr(test, allow(...))`; integration tests are separate crates, where these lints still apply only as configured — existing tests use `.unwrap()` freely).
- Never use `{:?}` of `Result`/`Fact`/`Value`/`EdnValue`/UUID-bearing types in `assert!`/`assert_eq!` message strings (CodeQL `rust/cleartext-logging`). `{:?}` elsewhere (e.g. as a sort key) is fine.
- `#![forbid(unsafe_code)]` — no `unsafe`.
- Query results must be identical to before for every query.
- Work in `.worktrees/perf-323-point-query` on branch `perf/323-point-query-history-scan`. `cargo test --release` is broken by design (panic=abort); use `cargo test` and `cargo bench`.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01BedF9ZmqVZLkvDwn91fuKy
  ```
  Commits must reference `#323` but never use a closing keyword for it until the final PR body.

## Review Focus

1. **Attribute prefix collision** (`:a` vs `:ab`, `:hash` vs `:hash2`) on a bound entity — only the exact attribute's facts may be returned, in pending *and* committed indexes. → Task 3 test `entity_attribute_indexed_excludes_prefix_sibling`, Task 4 test `prefix_sibling_attribute_not_leaked`.
2. **Bound entity whose query also uses a variable or pseudo-attribute** (`[:e ?a ?v]`, `[:e :db/valid-from ?vf]`) — must still see all of the entity's facts. → Task 4 tests `bound_entity_variable_attribute_sees_all`, `bound_entity_pseudo_attribute_matches_full_scan`.
3. **Many attributes on one bound entity (> threshold)** — must not regress to a full scan, and results must be correct. → Task 4 test `five_attributes_on_one_entity`.
4. **Facts visible in both pending and committed indexes / duplicated input records** — `net_asserted_facts` must be idempotent under duplicates since the executor dedup is removed. → Task 2 test `net_asserted_idempotent_under_duplicates`.
5. **Same-transaction assert + retract, and retraction followed by reassertion of the same value** across a checkpoint boundary. → Task 2 randomized equivalence test; Task 4 test `churned_attribute_live_value_file_backed`.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/graph/fxhash.rs` | Create | `FxHasher`, `FxBuildHasher` — fast non-cryptographic hasher |
| `src/graph/mod.rs` | Modify | register `pub(crate) mod fxhash;` |
| `src/graph/storage.rs` | Modify | rewrite `net_asserted_facts`; add `get_facts_by_entity_attribute_indexed`; tests |
| `src/query/datalog/executor.rs` | Modify | `selective_fact_fetch` narrowing + dedup removal |
| `tests/point_query_history_test.rs` | Create | end-to-end result-equivalence tests |
| `benches/minigraf_bench.rs` | Modify | `point_query_chain_depth` group |
| `docs/BENCHMARKS.md`, `CHANGELOG.md`, `docs/TEST_COVERAGE.md`, `CLAUDE.md` | Modify | doc sync |

---

### Task 1: FxHash hasher

**Files:**
- Create: `src/graph/fxhash.rs`
- Modify: `src/graph/mod.rs`

**Interfaces:**
- Produces: `crate::graph::fxhash::FxBuildHasher` (type alias `BuildHasherDefault<FxHasher>`), used as `HashMap<K, V, FxBuildHasher>` / `HashSet<K, FxBuildHasher>` created with `::default()`.

- [ ] **Step 1: Write the module with its failing-by-absence tests**

`src/graph/mod.rs` becomes:
```rust
pub(crate) mod fxhash;
pub(crate) mod storage;
pub(crate) mod types;
```

`src/graph/fxhash.rs`:
```rust
//! Small non-cryptographic hasher for internal hot-path hash maps (#323).
//!
//! Same multiply-rotate scheme as rustc's `FxHasher`. Not HashDoS-resistant;
//! only use it for maps keyed by data the embedding application already
//! controls (facts in its own database), never for untrusted network input.

use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Word-at-a-time multiply-rotate hasher.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(chunk);
            self.add_to_hash(u64::from_le_bytes(buf));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            for (dst, src) in buf.iter_mut().zip(rem) {
                *dst = *src;
            }
            self.add_to_hash(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `BuildHasher` for [`FxHasher`]; use with `HashMap::default()`.
pub(crate) type FxBuildHasher = BuildHasherDefault<FxHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::hash::BuildHasher;

    #[test]
    fn equal_inputs_hash_equal() {
        let b = FxBuildHasher::default();
        assert_eq!(b.hash_one(("abc", 1u64)), b.hash_one(("abc", 1u64)));
    }

    #[test]
    fn remainder_bytes_affect_hash() {
        let b = FxBuildHasher::default();
        // 9 bytes: one full word + 1 remainder byte that differs.
        assert_ne!(b.hash_one(b"abcdefghX"), b.hash_one(b"abcdefghY"));
        assert_ne!(b.hash_one(":a"), b.hash_one(":ab"));
    }

    #[test]
    fn usable_as_map_and_set_hasher() {
        let mut m: HashMap<(&str, u64), u64, FxBuildHasher> = HashMap::default();
        m.insert((":x", 1), 10);
        m.insert((":x", 2), 20);
        assert_eq!(m.get(&(":x", 2)).copied(), Some(20));
        let s: HashSet<u64, FxBuildHasher> = (0..1000).collect();
        assert_eq!(s.len(), 1000);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib graph::fxhash`
Expected: 3 passed. (Until Task 2 uses it, non-test builds warn `dead_code` for `FxBuildHasher`; that is resolved in Task 2 — do not add `#[allow(dead_code)]`.)

- [ ] **Step 3: Commit**
```bash
git add src/graph/fxhash.rs src/graph/mod.rs
git commit -m "perf: add in-crate FxHash hasher for hot-path maps (#323)"
```

---

### Task 2: Faster `net_asserted_facts`

**Files:**
- Modify: `src/graph/storage.rs` (function at ~line 528; tests module at ~line 827)

**Interfaces:**
- Consumes: `crate::graph::fxhash::FxBuildHasher` (Task 1).
- Produces: `pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact>` — same signature and semantics; output now preserves input order of surviving facts. Idempotent under duplicated input records (Task 4 relies on this).

- [ ] **Step 1: Move the current implementation into the test module as the reference oracle**

Cut the current body of `net_asserted_facts` and paste it into `mod tests` as:
```rust
    /// Pre-#323 implementation, kept verbatim as the oracle for the
    /// randomized equivalence test.
    fn net_asserted_facts_reference(facts: Vec<Fact>) -> Vec<Fact> {
        use std::collections::HashMap;

        type EavKey = (EntityId, Attribute, Vec<u8>);
        type WindowKey = (EntityId, Attribute, Vec<u8>, i64, i64);

        let mut max_retract_tx: HashMap<EavKey, u64> = HashMap::new();
        let mut by_window: HashMap<WindowKey, Fact> = HashMap::new();

        for fact in facts {
            let eav_key = (
                fact.entity,
                fact.attribute.clone(),
                encode_value(&fact.value),
            );

            if fact.asserted {
                let window_key = (
                    eav_key.0,
                    eav_key.1,
                    eav_key.2,
                    fact.valid_from,
                    fact.valid_to,
                );
                match by_window.get(&window_key) {
                    None => {
                        by_window.insert(window_key, fact);
                    }
                    Some(existing) if fact.tx_count > existing.tx_count => {
                        by_window.insert(window_key, fact);
                    }
                    _ => {}
                }
            } else {
                let tx_count = fact.tx_count;
                max_retract_tx
                    .entry(eav_key)
                    .and_modify(|max_tx| *max_tx = (*max_tx).max(tx_count))
                    .or_insert(tx_count);
            }
        }

        by_window
            .into_iter()
            .filter_map(|((entity, attribute, value, _, _), fact)| {
                let retract_tx = max_retract_tx
                    .get(&(entity, attribute, value))
                    .copied()
                    .unwrap_or(0);
                (fact.tx_count > retract_tx).then_some(fact)
            })
            .collect()
    }
```
(Copy it from the file rather than from here if they differ — the file is the source of truth.)

- [ ] **Step 2: Add the new tests to `mod tests`**

```rust
    /// Order-independent comparison key for a fact set.
    fn sorted_keys(facts: &[Fact]) -> Vec<String> {
        let mut v: Vec<String> = facts
            .iter()
            .map(|f| {
                format!(
                    "{}|{}|{:?}|{}|{}|{}|{}",
                    f.entity, f.attribute, f.value, f.tx_count, f.valid_from, f.valid_to, f.asserted
                )
            })
            .collect();
        v.sort();
        v
    }

    /// Deterministic xorshift so the test needs no RNG dependency.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn net_asserted_matches_reference_on_random_histories() {
        let entities = [uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2)];
        let attrs = [":a", ":ab", ":b"];
        let windows = [
            (0_i64, VALID_TIME_FOREVER),
            (1_000, 2_000),
            (1_500, VALID_TIME_FOREVER),
        ];
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _case in 0..500 {
            let n = rng.below(40) + 1;
            let mut facts = Vec::new();
            let mut tx = 0_u64;
            for _ in 0..n {
                // ~25% of records share the previous tx_count (same-transaction batches).
                if rng.below(4) != 0 {
                    tx += 1;
                }
                let e = entities[rng.below(2) as usize];
                let a = attrs[rng.below(3) as usize];
                let v = Value::Integer(i64::try_from(rng.below(3)).unwrap());
                if rng.below(3) == 0 {
                    facts.push(make_retract(e, a, v, tx));
                } else {
                    let (vf, vt) = windows[rng.below(3) as usize];
                    facts.push(make_assert(e, a, v, tx, vf, vt));
                }
            }
            let expected = sorted_keys(&net_asserted_facts_reference(facts.clone()));
            let actual = sorted_keys(&net_asserted_facts(facts));
            assert_eq!(actual, expected, "new net_asserted_facts diverged from reference");
        }
    }

    /// The executor's selective fetch no longer dedups (#323); it relies on
    /// net_asserted_facts collapsing duplicated input records.
    #[test]
    fn net_asserted_idempotent_under_duplicates() {
        let e = uuid::Uuid::from_u128(7);
        let facts = vec![
            make_assert(e, ":hash", Value::String("h0".into()), 1, 0, VALID_TIME_FOREVER),
            make_retract(e, ":hash", Value::String("h0".into()), 2),
            make_assert(e, ":hash", Value::String("h1".into()), 3, 0, VALID_TIME_FOREVER),
            make_assert(e, ":other", Value::String("o".into()), 3, 0, VALID_TIME_FOREVER),
        ];
        let mut doubled = facts.clone();
        doubled.extend(facts.clone());
        let once = sorted_keys(&net_asserted_facts(facts));
        let twice = sorted_keys(&net_asserted_facts(doubled));
        assert_eq!(once.len(), 2, "h1 and o are live");
        assert_eq!(twice, once, "duplicated records must not change the result");
    }

    #[test]
    fn net_asserted_preserves_input_order() {
        let e = uuid::Uuid::from_u128(9);
        let facts = vec![
            make_assert(e, ":z", Value::Integer(1), 1, 0, VALID_TIME_FOREVER),
            make_assert(e, ":a", Value::Integer(2), 2, 0, VALID_TIME_FOREVER),
            make_assert(e, ":m", Value::Integer(3), 3, 0, VALID_TIME_FOREVER),
        ];
        let out = net_asserted_facts(facts);
        let attrs: Vec<&str> = out.iter().map(|f| f.attribute.as_str()).collect();
        assert_eq!(attrs, vec![":z", ":a", ":m"]);
    }
```

- [ ] **Step 3: Run the new tests against the unchanged production function**

At this point the production `net_asserted_facts` still has the old body (Step 1 *copied* it into the test module; do not delete it from production yet).
Run: `cargo test --lib graph::storage::tests::net_asserted`
Expected: `net_asserted_preserves_input_order` FAILS (old code returns HashMap order); `net_asserted_matches_reference_on_random_histories` and `net_asserted_idempotent_under_duplicates` PASS.

- [ ] **Step 4: Replace the production body**

Replace the whole `pub(crate) fn net_asserted_facts` body (keep the existing doc comment, but replace its `# Implementation note` section with the text below):
```rust
/// # Implementation note
///
/// Hot path for every non-`:as-of` query (#323). Each value is encoded once;
/// the two group maps borrow `(entity, attribute, value_bytes)` from the input
/// instead of cloning them, and use [`FxBuildHasher`]. `by_window` stores the
/// index and `tx_count` of the winning assertion per validity window; survivors
/// are moved out of `facts` at the end, preserving input order.
///
/// Idempotent under duplicated input records (a duplicate assertion ties with
/// the original and loses; a duplicate retraction leaves the max unchanged).
/// `selective_fact_fetch` relies on this instead of deduplicating.
pub(crate) fn net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact> {
    use crate::graph::fxhash::FxBuildHasher;
    use std::collections::HashMap;

    type EavKey<'a> = (&'a EntityId, &'a str, &'a [u8]);
    type WindowKey<'a> = (&'a EntityId, &'a str, &'a [u8], i64, i64);

    let encoded: Vec<Vec<u8>> = facts.iter().map(|f| encode_value(&f.value)).collect();
    let mut keep = vec![false; facts.len()];

    {
        let mut max_retract_tx: HashMap<EavKey<'_>, u64, FxBuildHasher> = HashMap::default();
        let mut by_window: HashMap<WindowKey<'_>, (usize, u64), FxBuildHasher> =
            HashMap::default();

        for (idx, (fact, value_bytes)) in facts.iter().zip(encoded.iter()).enumerate() {
            let entity = &fact.entity;
            let attribute = fact.attribute.as_str();
            let value = value_bytes.as_slice();
            if fact.asserted {
                by_window
                    .entry((entity, attribute, value, fact.valid_from, fact.valid_to))
                    .and_modify(|winner| {
                        if fact.tx_count > winner.1 {
                            *winner = (idx, fact.tx_count);
                        }
                    })
                    .or_insert((idx, fact.tx_count));
            } else {
                max_retract_tx
                    .entry((entity, attribute, value))
                    .and_modify(|max_tx| *max_tx = (*max_tx).max(fact.tx_count))
                    .or_insert(fact.tx_count);
            }
        }

        for ((entity, attribute, value, _, _), (idx, tx_count)) in &by_window {
            let retract_tx = max_retract_tx
                .get(&(*entity, *attribute, *value))
                .copied()
                .unwrap_or(0);
            if *tx_count > retract_tx
                && let Some(slot) = keep.get_mut(*idx)
            {
                *slot = true;
            }
        }
    }

    facts
        .into_iter()
        .zip(keep)
        .filter_map(|(fact, kept)| kept.then_some(fact))
        .collect()
}
```
Also add `use crate::graph::fxhash::FxBuildHasher;` only inside the function as shown (keeps the top-level imports untouched). Update the doc-comment link to `[`FxBuildHasher`](crate::graph::fxhash::FxBuildHasher)` if rustdoc complains about the intra-doc link.

- [ ] **Step 5: Run the storage tests, then the whole suite**

Run: `cargo test --lib graph::storage`
Expected: all pass, including the 3 new tests.
Run: `cargo test 2>&1 | grep -E "^test result|FAILED|panicked"`
Expected: no failures. If any test fails because it indexed `net_asserted_facts` output assuming HashMap order, that test was already order-fragile: fix the test to compare sorted/set results, never the production code.

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings` → clean.
```bash
git add src/graph/storage.rs
git commit -m "perf: borrow keys and use FxHash in net_asserted_facts (#323)"
```

---

### Task 3: `(entity, attribute)` EAVT range lookup

**Files:**
- Modify: `src/graph/storage.rs` — add method inside `impl FactStorage` block that starts with `/// Production helpers on FactStorage` (~line 617), right after `get_facts_by_entity`; tests in `mod tests`.

**Interfaces:**
- Produces: `pub(crate) fn get_facts_by_entity_attribute_indexed(&self, entity_id: &EntityId, attribute: &Attribute) -> Result<Vec<Fact>>` — every stored record (assert and retract, all versions) whose `entity == entity_id && attribute == attribute`, pending + committed. Task 4 calls it.

- [ ] **Step 1: Write failing tests in `mod tests`**

```rust
    #[test]
    fn entity_attribute_indexed_pending_only() {
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        let other = uuid::Uuid::from_u128(2);
        storage
            .transact(vec![
                (e, ":hash".to_string(), Value::String("h0".into())),
                (e, ":other".to_string(), Value::String("o".into())),
                (other, ":hash".to_string(), Value::String("x".into())),
            ], None)
            .unwrap();
        storage
            .retract(vec![(e, ":hash".to_string(), Value::String("h0".into()))])
            .unwrap();
        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":hash".to_string())
            .unwrap();
        assert_eq!(facts.len(), 2, "assert + retract of :hash for e only");
        assert!(facts.iter().all(|f| f.entity == e && f.attribute == ":hash"));
    }

    #[test]
    fn entity_attribute_indexed_excludes_prefix_sibling() {
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        storage
            .transact(vec![
                (e, ":a".to_string(), Value::Integer(1)),
                (e, ":ab".to_string(), Value::Integer(2)),
                (e, ":a/b".to_string(), Value::Integer(3)),
            ], None)
            .unwrap();
        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":a".to_string())
            .unwrap();
        assert_eq!(facts.len(), 1, "only :a, not :ab or :a/b");
        assert_eq!(facts[0].value, Value::Integer(1));
    }

    #[test]
    fn entity_attribute_indexed_no_index_fallback() {
        // FactStorage with facts but empty pending indexes and no committed reader
        // exercises the fallback branch.
        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        storage
            .transact(vec![
                (e, ":a".to_string(), Value::Integer(1)),
                (e, ":b".to_string(), Value::Integer(2)),
            ], None)
            .unwrap();
        storage.replace_pending_indexes(crate::storage::index::Indexes::new());
        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":b".to_string())
            .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, Value::Integer(2));
    }

    #[test]
    fn entity_attribute_indexed_committed_and_pending() {
        use crate::storage::CommittedFactReader;
        use crate::storage::index::{FactRef, Indexes};
        use std::sync::Arc;

        struct MockLoader {
            facts: Vec<Fact>,
        }
        impl CommittedFactReader for MockLoader {
            fn resolve(&self, fr: FactRef) -> anyhow::Result<Fact> {
                self.facts
                    .get(fr.slot_index as usize)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no fact at slot"))
            }
            fn stream_all(&self) -> anyhow::Result<Vec<Fact>> {
                Ok(self.facts.clone())
            }
            fn committed_page_count(&self) -> u64 {
                1
            }
        }

        let storage = FactStorage::new();
        let e = uuid::Uuid::from_u128(1);
        let committed = vec![
            make_assert(e, ":a", Value::Integer(1), 1, 0, VALID_TIME_FOREVER),
            make_assert(e, ":ab", Value::Integer(9), 1, 0, VALID_TIME_FOREVER),
        ];
        let mut indexes = Indexes::new();
        for (slot, f) in committed.iter().enumerate() {
            indexes.insert(
                f,
                FactRef {
                    page_id: 1,
                    slot_index: u16::try_from(slot).unwrap(),
                },
            );
        }
        storage.replace_pending_indexes(indexes);
        storage.set_committed_reader(Arc::new(MockLoader { facts: committed }));

        let facts = storage
            .get_facts_by_entity_attribute_indexed(&e, &":a".to_string())
            .unwrap();
        assert_eq!(facts.len(), 1, "committed :a only, :ab excluded");
        assert_eq!(facts[0].value, Value::Integer(1));
    }
```
`FactStorage::transact(&self, Vec<(EntityId, Attribute, Value)>, Option<TransactOptions>) -> Result<TxId>` and `FactStorage::retract(&self, Vec<(EntityId, Attribute, Value)>) -> Result<(TxId, u64)>` are the real signatures (verified). Each test transacts distinct attributes per entity, so the v2 same-transaction multi-value key collision (#371) cannot interfere.

Note: the mock uses `page_id: 1` entries in the *pending* index map; this exercises the committed-resolve branch of `resolve_fact_ref` through the pending range loop, mirroring `test_committed_reader_resolves_facts`. The on-disk committed index path (`range_scan_eavt`) is covered end-to-end by Task 4's file-backed tests.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib entity_attribute_indexed`
Expected: compile error `no method named get_facts_by_entity_attribute_indexed`.

- [ ] **Step 3: Implement**

Insert after `get_facts_by_entity`:
```rust
    /// Get every stored record for one `(entity, attribute)` pair (index-driven, #323).
    ///
    /// Range-scans EAVT over `[(e, a, …), (e, next_prefix(a), …))` so other
    /// attributes of the same entity — and their version history — are never
    /// resolved. Both index sources post-filter on the exact attribute because a
    /// prefix range also covers longer attributes (`:a` → `:ab`).
    pub(crate) fn get_facts_by_entity_attribute_indexed(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Vec<Fact>> {
        use crate::storage::index::EavtKey;
        let d = self.data.read().unwrap_or_else(|e| e.into_inner());
        let matches = |f: &Fact| &f.entity == entity_id && &f.attribute == attribute;

        // Fallback: no indexes built yet
        if d.pending_indexes.eavt.is_empty() && d.committed_index_reader.is_none() {
            let mut result: Vec<Fact> = d.facts.iter().filter(|f| matches(f)).cloned().collect();
            if let Some(loader) = &d.committed {
                result.extend(loader.stream_all()?.into_iter().filter(|f| matches(f)));
            }
            return Ok(result);
        }

        let start = EavtKey {
            entity: *entity_id,
            attribute: attribute.clone(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        // None only for the empty attribute; then the committed scan is unbounded
        // above and relies on the post-filter.
        let end_opt: Option<EavtKey> = next_string_prefix(attribute).map(|next_attr| EavtKey {
            entity: *entity_id,
            attribute: next_attr,
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        });

        let mut facts = Vec::new();

        // Pending: walk forward from `start` while still on this exact (e, a).
        for (key, &fr) in d.pending_indexes.eavt.range(start.clone()..) {
            if key.entity != *entity_id || key.attribute != *attribute {
                break;
            }
            facts.push(resolve_fact_ref(&d, fr)?);
        }

        // Committed: on-disk B+tree range scan, post-filtered on the exact pair.
        if let Some(reader) = &d.committed_index_reader {
            for fr in reader.range_scan_eavt(&start, end_opt.as_ref())? {
                let fact = resolve_fact_ref(&d, fr)?;
                if matches(&fact) {
                    facts.push(fact);
                }
            }
        }

        Ok(facts)
    }
```
Why the pending loop can `break`: EAVT orders by `(entity, attribute, …)`, so all keys for the exact pair are contiguous starting at `start`; the first key with a different entity or attribute ends the run (`:ab` sorts after every `:a` key).

- [ ] **Step 4: Run tests**

Run: `cargo test --lib entity_attribute_indexed`
Expected: 4 passed.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy --all-targets -- -D warnings`. `get_facts_by_entity_attribute_indexed` is unused outside tests until Task 4 → a `dead_code` warning is expected here; if clippy's `-D warnings` blocks the commit hook, fold this commit into Task 4 instead of adding `#[allow(dead_code)]`.
```bash
git add src/graph/storage.rs
git commit -m "perf: add (entity, attribute) EAVT range lookup (#323)"
```

---

### Task 4: Narrow `selective_fact_fetch` and drop its dedup

**Files:**
- Modify: `src/query/datalog/executor.rs` — `selective_fact_fetch` (~lines 369–455) and the doc comment on `filter_facts_for_query` if it mentions dedup
- Create: `tests/point_query_history_test.rs`

**Interfaces:**
- Consumes: `FactStorage::get_facts_by_entity_attribute_indexed` (Task 3); `FactStorage::get_facts_by_entity`, `get_facts_by_attribute` (existing); `net_asserted_facts` idempotence (Task 2).
- Produces: same signature `fn selective_fact_fetch(&self, patterns: &[Pattern], threshold: usize) -> Option<Vec<Fact>>`.

- [ ] **Step 1: Write the integration tests**

`tests/point_query_history_test.rs`:
```rust
//! #323 — bound-entity point queries must return exactly what a full scan returns,
//! while only reading the queried attributes' history.
//!
//! Oracle: the same query with `:as-of 1000000000` goes through the full-scan
//! `get_facts_as_of` path (never the selective path) and, since every tx_count in
//! these tests is far below that bound, must produce identical rows.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult};

const ORACLE_AS_OF: &str = ":as-of 1000000000";

fn rows(r: QueryResult) -> Vec<String> {
    match r {
        QueryResult::QueryResults { results, .. } => {
            let mut v: Vec<String> = results.iter().map(|row| format!("{row:?}")).collect();
            v.sort();
            v
        }
        _ => panic!("expected QueryResults"),
    }
}

/// Run `(query [:find <find> <opts> :where <body>])` via the selective path and via
/// the full-scan oracle; assert equal; return the selective rows.
fn same_as_full_scan(db: &Minigraf, find: &str, opts: &str, body: &str) -> Vec<String> {
    let selective = rows(
        db.execute(&format!("(query [:find {find} {opts} :where {body}])"))
            .unwrap(),
    );
    let oracle = rows(
        db.execute(&format!(
            "(query [:find {find} {ORACLE_AS_OF} {opts} :where {body}])"
        ))
        .unwrap(),
    );
    assert_eq!(selective, oracle, "selective path diverged from full scan");
    selective
}

fn churn(db: &Minigraf, depth: usize) {
    db.execute("(transact [[:e/hot :other \"o\"]])").unwrap();
    db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h0\"]])")
        .unwrap();
    for i in 1..depth {
        db.execute(&format!("(retract [[:e/hot :hash \"h{}\"]])", i - 1))
            .unwrap();
        db.execute(&format!(
            "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [[:e/hot :hash \"h{i}\"]])"
        ))
        .unwrap();
    }
}

#[test]
fn churned_attribute_live_value_in_memory() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 50);
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
    assert_eq!(r.len(), 1, "exactly one live value");
    assert!(r[0].contains("h49"));
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :other ?v]");
    assert_eq!(r.len(), 1);
}

#[test]
fn churned_attribute_live_value_file_backed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.graph");
    {
        let db = Minigraf::open(&path).unwrap();
        churn(&db, 30);
        db.checkpoint().unwrap();
        // Pending on top of committed: more churn after the checkpoint,
        // including reasserting a previously retracted value.
        db.execute("(retract [[:e/hot :hash \"h29\"]])").unwrap();
        db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h3\"]])")
            .unwrap();
        let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
        assert_eq!(r.len(), 1);
        assert!(r[0].contains("\"h3\""));
    }
    let db = Minigraf::open(&path).unwrap();
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v]");
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("\"h3\""));
}

#[test]
fn two_attributes_on_one_entity() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 10);
    let r = same_as_full_scan(&db, "?h ?o", "", "[:e/hot :hash ?h] [:e/hot :other ?o]");
    assert_eq!(r.len(), 1);
}

#[test]
fn five_attributes_on_one_entity() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:e/x :a 1] [:e/x :b 2] [:e/x :c 3] [:e/x :d 4] [:e/x :f 5]])")
        .unwrap();
    let r = same_as_full_scan(
        &db,
        "?a ?b ?c ?d ?f",
        "",
        "[:e/x :a ?a] [:e/x :b ?b] [:e/x :c ?c] [:e/x :d ?d] [:e/x :f ?f]",
    );
    assert_eq!(r.len(), 1);
}

#[test]
fn bound_entity_variable_attribute_sees_all() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    let r = same_as_full_scan(&db, "?a ?v", "", "[:e/hot ?a ?v]");
    assert_eq!(r.len(), 2, ":hash h4 and :other o");
    // Mixed: one narrowed pattern + one variable-attribute pattern on the same entity.
    let r = same_as_full_scan(&db, "?a ?h", "", "[:e/hot :hash ?h] [:e/hot ?a _]");
    assert_eq!(r.len(), 2);
}

#[test]
fn bound_entity_pseudo_attribute_matches_full_scan() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    same_as_full_scan(
        &db,
        "?v ?vf",
        ":any-valid-time",
        "[:e/hot :hash ?v] [:e/hot :db/valid-from ?vf]",
    );
}

#[test]
fn not_and_or_on_narrowed_entity() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    db.execute("(transact [[:e/hot :flag true]])").unwrap();
    same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v] (not [:e/hot :flag false])");
    let r = same_as_full_scan(&db, "?v", "", "[:e/hot :hash ?v] (not [:e/hot :flag true])");
    assert!(r.is_empty(), "not must see :flag although only :hash is projected");
    same_as_full_scan(
        &db,
        "?v",
        "",
        "(or [:e/hot :hash ?v] [:e/hot :other ?v])",
    );
}

#[test]
fn prefix_sibling_attribute_not_leaked() {
    let dir = tempfile::tempdir().unwrap();
    let db = Minigraf::open(dir.path().join("p.graph")).unwrap();
    db.execute("(transact [[:e/x :hash \"a\"] [:e/x :hash2 \"b\"] [:e/x :hash/sub \"c\"]])")
        .unwrap();
    db.checkpoint().unwrap();
    db.execute("(transact [[:e/x :hashx \"d\"]])").unwrap();
    let r = same_as_full_scan(&db, "?v", "", "[:e/x :hash ?v]");
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("\"a\""));
}

#[test]
fn entity_and_attribute_patterns_mixed() {
    let db = Minigraf::in_memory().unwrap();
    churn(&db, 5);
    db.execute("(transact [[:e/a :hash \"z\"]])").unwrap();
    same_as_full_scan(&db, "?e ?v ?o", "", "[?e :hash ?v] [:e/hot :other ?o]");
}

#[test]
fn valid_at_past_on_churned_attribute() {
    let db = Minigraf::in_memory().unwrap();
    db.execute(
        "(transact {:valid-from \"2020-01-01T00:00:00Z\" :valid-to \"2021-01-01T00:00:00Z\"} [[:e/v :hash \"old\"]])",
    )
    .unwrap();
    db.execute("(transact {:valid-from \"2021-01-01T00:00:00Z\"} [[:e/v :hash \"new\"]])")
        .unwrap();
    db.execute("(transact [[:e/v :other 1]])").unwrap();
    let r = same_as_full_scan(&db, "?v", ":valid-at \"2020-06-01T00:00:00Z\"", "[:e/v :hash ?v]");
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("old"));
}
```
Adjust syntax only if the parser rejects something (e.g. option order `:as-of` / `:valid-at` / `:any-valid-time` — check `tests/bitemporal_test.rs` for accepted forms); never weaken an assertion to make it pass.

- [ ] **Step 2: Run tests against the current code**

Run: `cargo test --test point_query_history_test`
Expected: all PASS already (current code is correct, just slow). These are regression guards for Step 3. If one fails now, stop and investigate — it is a pre-existing bug, report it rather than fixing it inside this task.

- [ ] **Step 3: Rewrite `selective_fact_fetch`**

Replace the function (doc comment included) with:
```rust
    /// Attempt a selective index-backed fact fetch for the given patterns.
    ///
    /// Patterns with a bound entity literal (UUID or keyword → deterministic UUID) are
    /// fetched by entity; if every pattern on that entity also binds a concrete attribute
    /// keyword, only those `(entity, attribute)` EAVT ranges are read, so other attributes'
    /// version history is never resolved (#323). An entity also referenced with a variable
    /// or pseudo-attribute is read whole. Patterns without a bound entity are fetched by
    /// attribute. If any pattern has neither, returns `None` (full scan).
    ///
    /// Lookup budget: if narrowing needs more than `threshold` lookups, every entity falls
    /// back to a single whole-entity scan; only if that still exceeds `threshold` does this
    /// return `None`. Narrowing therefore never turns a selective query into a full scan.
    ///
    /// Results are not deduplicated: the sole caller feeds them to `net_asserted_facts`,
    /// which is idempotent under duplicated records.
    fn selective_fact_fetch(&self, patterns: &[Pattern], threshold: usize) -> Option<Vec<Fact>> {
        use std::collections::{BTreeMap, BTreeSet};

        // Per bound entity: Some(attrs) = only these attributes are referenced;
        // None = the whole entity is needed. BTree* for deterministic lookup order.
        let mut entity_attrs: BTreeMap<uuid::Uuid, Option<BTreeSet<String>>> = BTreeMap::new();
        let mut attributes: BTreeSet<String> = BTreeSet::new();

        for pattern in patterns {
            let bound_entity = match &pattern.entity {
                EdnValue::Uuid(u) => Some(*u),
                EdnValue::Keyword(_) => edn_to_entity_id(&pattern.entity).ok(),
                _ => None,
            };

            if let Some(uid) = bound_entity {
                let slot = entity_attrs
                    .entry(uid)
                    .or_insert_with(|| Some(BTreeSet::new()));
                match &pattern.attribute {
                    AttributeSpec::Real(EdnValue::Keyword(attr)) => {
                        if let Some(attrs) = slot {
                            attrs.insert(attr.clone());
                        }
                    }
                    _ => *slot = None,
                }
                continue;
            }

            if let AttributeSpec::Real(EdnValue::Keyword(attr)) = &pattern.attribute {
                attributes.insert(attr.clone());
            } else {
                return None;
            }
        }

        let narrowed_lookups: usize = entity_attrs
            .values()
            .map(|a| a.as_ref().map_or(1, BTreeSet::len))
            .sum::<usize>()
            + attributes.len();
        let narrow = narrowed_lookups <= threshold;
        let total = if narrow {
            narrowed_lookups
        } else {
            entity_attrs.len() + attributes.len()
        };
        if total == 0 || total > threshold {
            return None;
        }

        let mut all_facts: Vec<Fact> = Vec::new();

        for (uid, attrs) in &entity_attrs {
            match attrs {
                Some(attrs) if narrow => {
                    for attr in attrs {
                        all_facts.extend(
                            self.storage
                                .get_facts_by_entity_attribute_indexed(uid, attr)
                                .ok()?,
                        );
                    }
                }
                _ => all_facts.extend(self.storage.get_facts_by_entity(uid).ok()?),
            }
        }

        for attr in &attributes {
            all_facts.extend(self.storage.get_facts_by_attribute(attr).ok()?);
        }

        Some(all_facts)
    }
```
Behaviour kept from the old version: any storage error → `None` (caller falls back to full scan).

- [ ] **Step 4: Run the targeted and full suites**

Run: `cargo test --test point_query_history_test` → all pass.
Run: `cargo test 2>&1 | grep -E "^test result|FAILED|panicked"` → no failures.
Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` → clean.

- [ ] **Step 5: Commit**
```bash
git add src/query/datalog/executor.rs tests/point_query_history_test.rs
git commit -m "perf: narrow bound-entity fetch to queried attributes, drop redundant dedup (#323)"
```

---

### Task 5: Benchmark, measure, docs, follow-up issue

**Files:**
- Modify: `benches/minigraf_bench.rs`, `docs/BENCHMARKS.md`, `CHANGELOG.md`, `docs/TEST_COVERAGE.md`, `CLAUDE.md`

- [ ] **Step 1: Add the bench group**

Before `// ── query/predicate_pushdown` add:
```rust
// ── point_query_chain_depth (Issue #323) ─────────────────────────────────────
//
// One entity whose :hash is retracted/reasserted `depth` times (exactly one live
// value), plus a never-churned :other on the same entity; checkpointed file DB.

fn bench_point_query_chain_depth(c: &mut Criterion) {
    const DEPTHS: &[usize] = &[1, 500, 2000];
    const QUERIES: &[(&str, &str)] = &[
        ("churned_attr", "(query [:find ?v :where [:e/hot :hash ?v]])"),
        ("sibling_attr", "(query [:find ?v :where [:e/hot :other ?v]])"),
        ("attr_scan", "(query [:find ?v :where [?e :hash ?v]])"),
    ];
    for &(name, q) in QUERIES {
        let mut group = c.benchmark_group(format!("point_query_chain_depth/{name}"));
        group.sample_size(20);
        for &depth in DEPTHS {
            let dir = tempfile::tempdir().unwrap();
            let db = minigraf::Minigraf::open(dir.path().join("b.graph")).unwrap();
            for i in 0..2000 {
                db.execute(&format!("(transact [[:f/{i} :x {i}]])")).unwrap();
            }
            db.execute("(transact [[:e/hot :other \"o\"]])").unwrap();
            db.execute(
                "(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h0\"]])",
            )
            .unwrap();
            for i in 1..depth {
                db.execute(&format!("(retract [[:e/hot :hash \"h{}\"]])", i - 1))
                    .unwrap();
                db.execute(&format!(
                    "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [[:e/hot :hash \"h{i}\"]])"
                ))
                .unwrap();
            }
            db.checkpoint().unwrap();
            group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
                b.iter(|| black_box(db.execute(q).unwrap()));
            });
        }
        group.finish();
    }
}
```
Register `bench_point_query_chain_depth, // Issue #323` in `criterion_group!` after `bench_predicate_pushdown`. Confirm `tempfile` is usable from benches (it is a non-wasm dev-dependency; benches are native).

- [ ] **Step 2: Baseline vs after**

Commit the bench first, then measure the baseline in a throwaway worktree at the branch's merge-base (pre-Task-1 `src/`) with the new bench file copied in. Do not use `git stash` (shared across worktrees).
```bash
git add benches/minigraf_bench.rs && git commit -m "bench: point_query_chain_depth (#323)"
BASE=$(git merge-base HEAD origin/main)
git worktree add /tmp/claude-1000/-home-aditya-Work-AMC-Minigraf-minigraf/008662fe-0225-4a06-a393-915f375e4da0/scratchpad/base "$BASE"
cp benches/minigraf_bench.rs /tmp/claude-1000/-home-aditya-Work-AMC-Minigraf-minigraf/008662fe-0225-4a06-a393-915f375e4da0/scratchpad/base/benches/
(cd /tmp/claude-1000/-home-aditya-Work-AMC-Minigraf-minigraf/008662fe-0225-4a06-a393-915f375e4da0/scratchpad/base && cargo bench --bench minigraf_bench -- point_query_chain_depth --save-baseline before)
cargo bench --bench minigraf_bench -- point_query_chain_depth
git worktree remove --force /tmp/claude-1000/-home-aditya-Work-AMC-Minigraf-minigraf/008662fe-0225-4a06-a393-915f375e4da0/scratchpad/base
```
Record the median for each (query, depth) before and after.

Acceptance (spec §5): `sibling_attr` at depth 2000 within 2× of depth 1; `churned_attr` and `attr_scan` at depth 2000 ≥3× faster than before. If not met, stop and report the numbers — do not tune further without discussing.

- [ ] **Step 3: Docs**

- `CHANGELOG.md`: add at top, above `## v2.0.1`:
  ```markdown
  ## Unreleased

  ### Performance

  - **Bound-entity point queries no longer pay for other attributes' history (#323).** `[:e :attr ?v]` now range-scans only `(e, :attr)` in the EAVT index instead of every record the entity has ever had. The remaining per-record cost on a heavily retracted/reasserted attribute is cut by <N>× (cheaper net-assert grouping, redundant dedup removed); attribute scans (`[?e :attr ?v]`) benefit from the same change. File format and results unchanged. Resolve cost on a churned attribute is still proportional to its history on v2.x; the structural fix needs the v8 index keys and is tracked in #<v3-issue>.
  ```
  Fill `<N>` from Step 2 and `<v3-issue>` from Step 4.
- `docs/BENCHMARKS.md`: add a "Point query vs. version-chain depth (#323)" table (before/after medians) under the query latency section, following that file's existing table style.
- `docs/TEST_COVERAGE.md` and `CLAUDE.md` test count: run `cargo test 2>&1 | grep "^test result"` and sum passed/ignored; update both to the new totals and add `tests/point_query_history_test.rs` to the per-file breakdown.
- Also add `fxhash.rs` to the `src/graph/` module list in `CLAUDE.md`'s Architecture section.

- [ ] **Step 4: File the v3 follow-up issue**

```bash
gh issue create --milestone "v3.0.0" --title "perf: compute net-assert on v8 index keys before resolving facts (O(live) point queries)" --body "$(cat <<'EOF'
Follow-up to #323 (v2.x mitigation).

v8 `EavtKey`/`AevtKey` carry `value_bytes` and `asserted`. `net_asserted_facts` semantics can therefore be computed over index keys **before** `resolve_fact_ref`, and only surviving records resolved. Resolve cost becomes O(live facts); the key walk stays O(history) but over B+tree leaf pages, which is far cheaper than a page resolve + postcard decode per record.

Scope: `get_facts_by_entity_attribute_indexed`, `get_facts_by_entity`, `get_facts_by_attribute` on the `v3` branch; the executor's selective path. `:as-of` needs `tx_count <= N` applied on keys first.

Refs #323
EOF
)"
```
If the milestone name differs, run `gh api repos/:owner/:repo/milestones --jq '.[].title'` and use the v3.0.0 one.

- [ ] **Step 5: Commit docs**
```bash
git add CHANGELOG.md docs/BENCHMARKS.md docs/TEST_COVERAGE.md CLAUDE.md
git commit -m "docs: changelog, benchmarks and test counts for #323"
```

- [ ] **Step 6: PR** — push the branch and open a PR against `main` whose body ends with `Closes #323` and the attribution footer; then monitor CI until green (never merge without explicit confirmation).
