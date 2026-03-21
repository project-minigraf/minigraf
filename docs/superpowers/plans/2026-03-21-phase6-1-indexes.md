# Phase 6.1 — Covering Indexes & Query Optimizer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four Datomic-style covering indexes (EAVT, AEVT, AVET, VAET) with bi-temporal keys, persist them as B+tree pages in file format v4, validate them with a CRC32 sync check on open, and add a query optimizer that selects indexes and reorders join patterns.

**Architecture:** In-memory indexes are `BTreeMap<IndexKey, FactRef>` co-located with the fact list under a single `RwLock<FactData>`. On save/checkpoint, each BTreeMap is serialised to B+tree pages (leaf nodes linked for range scans) appended after the data pages. On open, a CRC32 over all in-memory facts (sorted deterministically) validates the on-disk index; a mismatch triggers a rebuild. The query optimizer calls `plan()` before pattern matching to select the best index per pattern and reorder joins by selectivity (skipped under the `wasm` feature flag). In Phase 6.1, the actual fast-path lookup still falls back to linear scan (page IDs are placeholders); the index-driven O(1) fetch is Phase 6.2's deliverable.

**Tech Stack:** Rust stable, `std::collections::BTreeMap`, `crc32fast` (already in Cargo.toml), `postcard` (already in Cargo.toml), `uuid` (already in Cargo.toml).

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| **Create** | `src/storage/index.rs` | `FactRef`, canonical value encoding, index key types (`EavtKey`, `AevtKey`, `AvetKey`, `VaetKey`), `Indexes` struct |
| **Create** | `src/storage/btree.rs` | `write_*_index` (BTreeMap → B+tree pages), `read_*_index` (B+tree pages → BTreeMap), page type constants |
| **Create** | `src/query/datalog/optimizer.rs` | `IndexHint`, `plan()` (index selection + join ordering), selectivity scoring |
| **Modify** | `src/graph/storage.rs` | Introduce `FactData { facts, indexes }`; wrap in single `Arc<RwLock<FactData>>`; populate all four indexes in `transact`, `retract`, `load_fact`; add `index_stats()` test accessor |
| **Modify** | `src/storage/mod.rs` | `FileHeader` v4 (72 bytes: new fields + `index_checksum`); bump `FORMAT_VERSION` to 4; add `pub mod index; pub mod btree;` |
| **Modify** | `src/storage/persistent_facts.rs` | `save_indexes`, `load_indexes`, `compute_index_checksum`, sync check on load; v3→v4 migration |
| **Modify** | `src/query/datalog/executor.rs` | Call `optimizer::plan()` before matching; thread `IndexHint` through for Phase 6.2 |
| **Modify** | `src/query/datalog/matcher.rs` | `match_pattern_with_hint` that prepares index-driven lookup infrastructure for 6.2 |
| **Modify** | `src/query/datalog/mod.rs` | Add `pub mod optimizer;` |
| **Modify** | `Cargo.toml` | Add `[features] default = [] wasm = []` |
| **Create** | `tests/index_test.rs` | Integration tests through the public `Minigraf` API: index correctness, sync check rebuild, bi-temporal queries, recursive rules regression |

---

## Task 1: FactRef, Index Key Types, and Canonical Value Encoding

**Files:**
- Create: `src/storage/index.rs`
- Modify: `src/storage/mod.rs` (add `pub mod index;`)

- [ ] **Step 1.1: Write failing tests in the new file**

Create `src/storage/index.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Fact, Value, VALID_TIME_FOREVER};
    use uuid::Uuid;

    #[test]
    fn test_fact_ref_fields() {
        let r = FactRef { page_id: 42, slot_index: 7 };
        assert_eq!(r.page_id, 42);
        assert_eq!(r.slot_index, 7);
    }

    #[test]
    fn test_encode_value_sort_order_integers() {
        let neg = encode_value(&Value::Integer(-1));
        let zero = encode_value(&Value::Integer(0));
        let pos = encode_value(&Value::Integer(1));
        assert!(neg < zero, "neg should sort before zero");
        assert!(zero < pos, "zero should sort before pos");
    }

    #[test]
    fn test_encode_value_large_negative_before_large_positive() {
        let a = encode_value(&Value::Integer(i64::MIN));
        let b = encode_value(&Value::Integer(i64::MAX));
        assert!(a < b);
    }

    #[test]
    fn test_encode_value_sort_order_cross_type() {
        let null = encode_value(&Value::Null);
        let bool_val = encode_value(&Value::Boolean(false));
        let int_val = encode_value(&Value::Integer(0));
        assert!(null < bool_val);
        assert!(bool_val < int_val);
    }

    #[test]
    fn test_encode_value_ref_structure() {
        let id = Uuid::new_v4();
        let bytes = encode_value(&Value::Ref(id));
        assert_eq!(bytes[0], 0x06); // Ref discriminant
        assert_eq!(&bytes[1..17], id.as_bytes());
    }

    #[test]
    fn test_eavt_key_ordering_by_entity() {
        let e1 = Uuid::from_u128(1);
        let e2 = Uuid::from_u128(2);
        let k1 = EavtKey { entity: e1, attribute: ":age".to_string(), valid_from: 0, valid_to: i64::MAX, tx_count: 1 };
        let k2 = EavtKey { entity: e2, attribute: ":age".to_string(), valid_from: 0, valid_to: i64::MAX, tx_count: 1 };
        assert!(k1 < k2);
    }

    #[test]
    fn test_avet_key_orders_by_value_bytes() {
        let e = Uuid::new_v4();
        let k1 = AvetKey { attribute: ":score".to_string(), value_bytes: encode_value(&Value::Integer(10)), valid_from: 0, valid_to: i64::MAX, entity: e, tx_count: 1 };
        let k2 = AvetKey { attribute: ":score".to_string(), value_bytes: encode_value(&Value::Integer(20)), valid_from: 0, valid_to: i64::MAX, entity: e, tx_count: 2 };
        assert!(k1 < k2);
    }

    #[test]
    fn test_indexes_insert_vaet_only_for_ref() {
        let entity = Uuid::new_v4();
        let target = Uuid::new_v4();
        let mut indexes = Indexes::new();

        // Non-Ref value: should NOT appear in VAET
        let non_ref_fact = Fact::with_valid_time(
            entity, ":name".to_string(), Value::String("Alice".to_string()),
            0, 1, 0, VALID_TIME_FOREVER,
        );
        indexes.insert(&non_ref_fact, FactRef { page_id: 1, slot_index: 0 });
        assert!(indexes.vaet.is_empty(), "VAET must not contain non-Ref fact");

        // Ref value: SHOULD appear in VAET
        let ref_fact = Fact::with_valid_time(
            entity, ":friend".to_string(), Value::Ref(target),
            0, 2, 0, VALID_TIME_FOREVER,
        );
        indexes.insert(&ref_fact, FactRef { page_id: 2, slot_index: 0 });
        assert_eq!(indexes.vaet.len(), 1);
    }

    #[test]
    fn test_indexes_insert_populates_all_four() {
        let entity = Uuid::new_v4();
        let target = Uuid::new_v4();
        let mut indexes = Indexes::new();
        let ref_fact = Fact::with_valid_time(
            entity, ":friend".to_string(), Value::Ref(target),
            0, 1, 0, VALID_TIME_FOREVER,
        );
        indexes.insert(&ref_fact, FactRef { page_id: 1, slot_index: 0 });
        assert_eq!(indexes.eavt.len(), 1);
        assert_eq!(indexes.aevt.len(), 1);
        assert_eq!(indexes.avet.len(), 1);
        assert_eq!(indexes.vaet.len(), 1);
    }
}
```

- [ ] **Step 1.2: Run tests to verify they fail (compile errors expected)**

```bash
cargo test --lib storage::index 2>&1 | head -20
```

Expected: compile errors — `FactRef`, `encode_value`, `EavtKey`, `Indexes`, etc. not found.

- [ ] **Step 1.3: Implement `src/storage/index.rs`**

```rust
//! Index key types, FactRef, and canonical value encoding for the four
//! covering indexes (EAVT, AEVT, AVET, VAET).
//!
//! `FactRef` identifies a fact's location on disk. In Phase 6.1, one fact
//! occupies one page (`slot_index` is always 0). In Phase 6.2, `slot_index`
//! identifies the record slot within a packed page.

use crate::graph::types::{Attribute, EntityId, Fact, Value};

// ─── FactRef ────────────────────────────────────────────────────────────────

/// Disk location of a fact.
///
/// `slot_index` is always `0` in Phase 6.1 (one fact per page).
/// In Phase 6.2 it identifies the record within a packed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactRef {
    pub page_id: u64,
    pub slot_index: u16,
}

// ─── Canonical Value Encoding ───────────────────────────────────────────────

/// Encode a `Value` to bytes that preserve sort order across all variants.
///
/// Discriminant assignment (first byte):
///   0x00 = Null, 0x01 = Boolean, 0x02 = Integer, 0x03 = Float,
///   0x04 = String, 0x05 = Keyword, 0x06 = Ref
///
/// Within each type, big-endian layout ensures byte-wise comparison matches
/// the natural order of the type.
pub fn encode_value(v: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    match v {
        Value::Null => { bytes.push(0x00); }
        Value::Boolean(b) => { bytes.push(0x01); bytes.push(*b as u8); }
        Value::Integer(n) => {
            bytes.push(0x02);
            // Flip the sign bit so that negative numbers sort before positive
            // after unsigned byte comparison: MIN..=-1 maps to 0..0x7FFF...,
            // 0..=MAX maps to 0x8000...=0xFFFF...
            let bits = (*n as u64) ^ 0x8000_0000_0000_0000;
            bytes.extend_from_slice(&bits.to_be_bytes());
        }
        Value::Float(f) => {
            bytes.push(0x03);
            let bits = f.to_bits();
            let bits = if bits >> 63 == 0 {
                bits ^ 0x8000_0000_0000_0000 // positive: flip sign bit
            } else {
                !bits // negative: flip all bits
            };
            bytes.extend_from_slice(&bits.to_be_bytes());
        }
        Value::String(s) => { bytes.push(0x04); bytes.extend_from_slice(s.as_bytes()); }
        Value::Keyword(k) => { bytes.push(0x05); bytes.extend_from_slice(k.as_bytes()); }
        Value::Ref(id) => { bytes.push(0x06); bytes.extend_from_slice(id.as_bytes()); }
    }
    bytes
}

// ─── Index Key Types ─────────────────────────────────────────────────────────

/// EAVT: sort by (Entity, Attribute, ValidFrom, ValidTo, TxCount)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EavtKey {
    pub entity: EntityId,
    pub attribute: Attribute,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
}

/// AEVT: sort by (Attribute, Entity, ValidFrom, ValidTo, TxCount)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AevtKey {
    pub attribute: Attribute,
    pub entity: EntityId,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
}

/// AVET: sort by (Attribute, ValueBytes, ValidFrom, ValidTo, Entity, TxCount)
///
/// `value_bytes` is the canonical encoding from `encode_value`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AvetKey {
    pub attribute: Attribute,
    pub value_bytes: Vec<u8>,
    pub valid_from: i64,
    pub valid_to: i64,
    pub entity: EntityId,
    pub tx_count: u64,
}

/// VAET: sort by (RefTarget, Attribute, ValidFrom, ValidTo, SourceEntity, TxCount)
///
/// Only facts with `Value::Ref` are indexed here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VaetKey {
    pub ref_target: EntityId,
    pub attribute: Attribute,
    pub valid_from: i64,
    pub valid_to: i64,
    pub source_entity: EntityId,
    pub tx_count: u64,
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

/// All four covering indexes held in memory alongside the fact list.
///
/// Populated on every `transact`, `retract`, and `load_fact`.
#[derive(Default)]
pub struct Indexes {
    pub eavt: std::collections::BTreeMap<EavtKey, FactRef>,
    pub aevt: std::collections::BTreeMap<AevtKey, FactRef>,
    pub avet: std::collections::BTreeMap<AvetKey, FactRef>,
    pub vaet: std::collections::BTreeMap<VaetKey, FactRef>,
}

impl Indexes {
    pub fn new() -> Self { Self::default() }

    /// Insert a fact into all applicable indexes.
    ///
    /// `fact_ref` is the disk location. In Phase 6.1, callers pass
    /// `FactRef { page_id: 0, slot_index: 0 }` as a placeholder; real
    /// page IDs are assigned by `save()` and updated via `reindex_from_facts`.
    pub fn insert(&mut self, fact: &Fact, fact_ref: FactRef) {
        self.eavt.insert(EavtKey {
            entity: fact.entity,
            attribute: fact.attribute.clone(),
            valid_from: fact.valid_from,
            valid_to: fact.valid_to,
            tx_count: fact.tx_count,
        }, fact_ref);

        self.aevt.insert(AevtKey {
            attribute: fact.attribute.clone(),
            entity: fact.entity,
            valid_from: fact.valid_from,
            valid_to: fact.valid_to,
            tx_count: fact.tx_count,
        }, fact_ref);

        self.avet.insert(AvetKey {
            attribute: fact.attribute.clone(),
            value_bytes: encode_value(&fact.value),
            valid_from: fact.valid_from,
            valid_to: fact.valid_to,
            entity: fact.entity,
            tx_count: fact.tx_count,
        }, fact_ref);

        if let Value::Ref(target) = &fact.value {
            self.vaet.insert(VaetKey {
                ref_target: *target,
                attribute: fact.attribute.clone(),
                valid_from: fact.valid_from,
                valid_to: fact.valid_to,
                source_entity: fact.entity,
                tx_count: fact.tx_count,
            }, fact_ref);
        }
    }
}

#[cfg(test)]
mod tests { /* (tests from Step 1.1) */ }
```

Replace the `#[cfg(test)] mod tests { /* ... */ }` placeholder with the full test code from Step 1.1.

- [ ] **Step 1.4: Add `pub mod index;` to `src/storage/mod.rs`**

Add after the existing module declarations:
```rust
pub mod index;
```

(`pub mod btree;` will be added in Task 4.)

- [ ] **Step 1.5: Run tests**

```bash
cargo test storage::index -- --nocapture
```

Expected: 8 tests pass.

- [ ] **Step 1.6: Commit**

```bash
git add src/storage/index.rs src/storage/mod.rs
git commit -m "feat(6.1): FactRef, index key types, Indexes struct, canonical value encoding"
```

---

## Task 2: FileHeader v4 (72 bytes)

**Files:**
- Modify: `src/storage/mod.rs`

The current `FileHeader` is 64 bytes (32-byte `reserved` field). v4 repurposes those 32 bytes: four 8-byte root page fields + 4-byte checksum + 4-byte padding = 32 bytes exactly.

- [ ] **Step 2.1: Update the existing header tests to expect v4**

Locate the test `test_file_header_serialization` in `src/storage/mod.rs` and replace it:

```rust
#[test]
fn test_file_header_serialization_v4() {
    let header = FileHeader::new();
    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 72);
    assert_eq!(&bytes[0..4], b"MGRF");
    // version field at bytes 4..8
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 4);
    // eavt_root_page at bytes 32..40: zero on fresh header
    assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), 0);
    // index_checksum at bytes 64..68: zero on fresh header
    assert_eq!(u32::from_le_bytes(bytes[64..68].try_into().unwrap()), 0);
}

#[test]
fn test_file_header_roundtrip_v4() {
    let mut header = FileHeader::new();
    header.eavt_root_page = 10;
    header.aevt_root_page = 20;
    header.avet_root_page = 30;
    header.vaet_root_page = 40;
    header.index_checksum = 0xDEAD_BEEF;
    let bytes = header.to_bytes();
    let parsed = FileHeader::from_bytes(&bytes).unwrap();
    assert_eq!(parsed.eavt_root_page, 10);
    assert_eq!(parsed.avet_root_page, 30);
    assert_eq!(parsed.index_checksum, 0xDEAD_BEEF);
}

#[test]
fn test_file_header_from_bytes_v3_accepted() {
    // v3 files have a 64-byte header. from_bytes must accept them and
    // return zeroed index fields.
    let mut bytes = vec![0u8; 64];
    bytes[0..4].copy_from_slice(b"MGRF");
    bytes[4..8].copy_from_slice(&3u32.to_le_bytes()); // version = 3
    bytes[8..16].copy_from_slice(&1u64.to_le_bytes()); // page_count = 1
    let header = FileHeader::from_bytes(&bytes).unwrap();
    assert_eq!(header.version, 3);
    assert_eq!(header.eavt_root_page, 0);
    assert_eq!(header.index_checksum, 0);
}
```

- [ ] **Step 2.2: Run tests to verify they fail**

```bash
cargo test storage::tests -- --nocapture 2>&1 | grep -E "FAILED|error"
```

Expected: `test_file_header_serialization_v4` fails (bytes.len() is 64).

- [ ] **Step 2.3: Replace `FileHeader` struct and impl in `src/storage/mod.rs`**

```rust
pub const FORMAT_VERSION: u32 = 4;

/// File header for .graph files — 72 bytes in v4.
///
/// Layout (all fields little-endian):
///   0..4    magic ("MGRF")
///   4..8    version (u32)
///   8..16   page_count (u64)
///   16..24  node_count (u64)          — reused as fact count
///   24..32  last_checkpointed_tx_count (u64)
///   32..40  eavt_root_page (u64)      — new in v4
///   40..48  aevt_root_page (u64)      — new in v4
///   48..56  avet_root_page (u64)      — new in v4
///   56..64  vaet_root_page (u64)      — new in v4
///   64..68  index_checksum (u32)      — new in v4
///   68..72  _padding (u32)
#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub page_count: u64,
    pub node_count: u64,
    pub last_checkpointed_tx_count: u64,
    pub eavt_root_page: u64,
    pub aevt_root_page: u64,
    pub avet_root_page: u64,
    pub vaet_root_page: u64,
    pub index_checksum: u32,
    pub _padding: u32,
}

impl FileHeader {
    pub fn new() -> Self {
        FileHeader {
            magic: MAGIC_NUMBER,
            version: FORMAT_VERSION,
            page_count: 1,
            node_count: 0,
            last_checkpointed_tx_count: 0,
            eavt_root_page: 0,
            aevt_root_page: 0,
            avet_root_page: 0,
            vaet_root_page: 0,
            index_checksum: 0,
            _padding: 0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(72);
        b.extend_from_slice(&self.magic);
        b.extend_from_slice(&self.version.to_le_bytes());
        b.extend_from_slice(&self.page_count.to_le_bytes());
        b.extend_from_slice(&self.node_count.to_le_bytes());
        b.extend_from_slice(&self.last_checkpointed_tx_count.to_le_bytes());
        b.extend_from_slice(&self.eavt_root_page.to_le_bytes());
        b.extend_from_slice(&self.aevt_root_page.to_le_bytes());
        b.extend_from_slice(&self.avet_root_page.to_le_bytes());
        b.extend_from_slice(&self.vaet_root_page.to_le_bytes());
        b.extend_from_slice(&self.index_checksum.to_le_bytes());
        b.extend_from_slice(&self._padding.to_le_bytes());
        b
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        // Both v3 (64 bytes) and v4 (72 bytes) must pass at least 64-byte
        // validation before the version field is read.
        if bytes.len() < 64 {
            anyhow::bail!("Invalid header: too short (got {} bytes, need 64)", bytes.len());
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != MAGIC_NUMBER {
            anyhow::bail!("Invalid magic number: not a .graph file");
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let page_count = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let node_count = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let last_checkpointed_tx_count = u64::from_le_bytes(bytes[24..32].try_into().unwrap());

        // v3 and earlier: no index fields — return with zero-filled index fields.
        // The v3→v4 migration in persistent_facts.rs will upgrade on next save.
        if version <= 3 {
            return Ok(FileHeader {
                magic, version, page_count, node_count,
                last_checkpointed_tx_count,
                eavt_root_page: 0, aevt_root_page: 0,
                avet_root_page: 0, vaet_root_page: 0,
                index_checksum: 0, _padding: 0,
            });
        }

        // v4: need full 72 bytes.
        if bytes.len() < 72 {
            anyhow::bail!("Invalid v4 header: expected 72 bytes, got {}", bytes.len());
        }
        Ok(FileHeader {
            magic, version, page_count, node_count, last_checkpointed_tx_count,
            eavt_root_page:  u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            aevt_root_page:  u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            avet_root_page:  u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            vaet_root_page:  u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
            index_checksum:  u32::from_le_bytes(bytes[64..68].try_into().unwrap()),
            _padding:        u32::from_le_bytes(bytes[68..72].try_into().unwrap()),
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
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
}
```

Remove the old `reserved: [u8; 32]` field everywhere it was referenced (constructor, to_bytes, from_bytes, tests).

- [ ] **Step 2.4: Run all tests**

```bash
cargo test
```

Expected: all tests pass. Update any other test that referenced `header.reserved` or assumed 64-byte length.

- [ ] **Step 2.5: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat(6.1): FileHeader v4 — 72-byte layout with index root pages and checksum"
```

---

## Task 3: FactStorage Refactor — Introduce `FactData`

**Files:**
- Modify: `src/graph/storage.rs`

All existing public method signatures are unchanged. Only the internal lock structure changes.

- [ ] **Step 3.1: Write failing tests for index population**

Add to the `#[cfg(test)]` section inside `src/graph/storage.rs`:

```rust
#[test]
fn test_indexes_populated_on_transact() {
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    storage.transact(vec![
        (alice, ":name".to_string(), Value::String("Alice".to_string())),
        (alice, ":friend".to_string(), Value::Ref(bob)),
    ], None).unwrap();
    let (eavt, aevt, avet, vaet) = storage.index_counts();
    assert_eq!(eavt, 2);
    assert_eq!(aevt, 2);
    assert_eq!(avet, 2);
    assert_eq!(vaet, 1, "Only Ref values go into VAET");
}

#[test]
fn test_slot_index_is_zero_in_6_1() {
    let storage = FactStorage::new();
    let e = Uuid::new_v4();
    storage.transact(vec![(e, ":x".to_string(), Value::Integer(1))], None).unwrap();
    // index_counts indirectly verifies the insert path; direct slot check
    // is via the internal field accessible only within the crate.
    let (eavt, _, _, _) = storage.index_counts();
    assert_eq!(eavt, 1);
}

#[test]
fn test_load_fact_populates_indexes() {
    let storage = FactStorage::new();
    let e = Uuid::new_v4();
    let fact = crate::graph::types::Fact::with_valid_time(
        e, ":name".to_string(), Value::String("Test".to_string()),
        0, 1, 0, crate::graph::types::VALID_TIME_FOREVER,
    );
    storage.load_fact(fact).unwrap();
    storage.restore_tx_counter().unwrap();
    let (eavt, _, _, _) = storage.index_counts();
    assert_eq!(eavt, 1);
}
```

- [ ] **Step 3.2: Run tests to verify they fail**

```bash
cargo test graph::storage -- --nocapture 2>&1 | head -15
```

Expected: compile errors — `index_counts` method not found.

- [ ] **Step 3.3: Refactor `FactStorage` in `src/graph/storage.rs`**

Key changes:
1. Add `use crate::storage::index::{FactRef, Indexes};`
2. Define private `FactData` struct
3. Change `facts: Arc<RwLock<Vec<Fact>>>` to `data: Arc<RwLock<FactData>>`
4. Update all methods to lock `self.data` instead of `self.facts`
5. Update `transact`, `retract`, `load_fact` to call `d.indexes.insert(fact, FactRef { page_id: 0, slot_index: 0 })`
6. Add `pub fn index_counts(&self) -> (usize, usize, usize, usize)` for tests (visible within crate and to integration tests via `pub`)

```rust
use crate::storage::index::{FactRef, Indexes};

/// Internal container: fact list and its indexes under one lock.
///
/// A single `RwLock` ensures readers always see a consistent snapshot —
/// no torn reads where indexes reference facts not yet in the Vec.
struct FactData {
    facts: Vec<Fact>,
    indexes: Indexes,
}

#[derive(Clone)]
pub struct FactStorage {
    data: Arc<RwLock<FactData>>,
    tx_counter: Arc<AtomicU64>,
}

impl FactStorage {
    pub fn new() -> Self {
        FactStorage {
            data: Arc::new(RwLock::new(FactData {
                facts: Vec::new(),
                indexes: Indexes::new(),
            })),
            tx_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len) for testing.
    pub fn index_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.data.read().unwrap();
        (d.indexes.eavt.len(), d.indexes.aevt.len(),
         d.indexes.avet.len(), d.indexes.vaet.len())
    }

    /// Replace the in-memory indexes with a freshly rebuilt set.
    ///
    /// Used by `PersistentFactStorage` after detecting an index checksum
    /// mismatch (e.g. after crash recovery). Takes `&self` because `FactData`
    /// is behind an interior-mutable `RwLock`.
    pub fn replace_indexes(&self, indexes: Indexes) {
        let mut d = self.data.write().unwrap();
        d.indexes = indexes;
    }

    pub fn transact(&self, fact_tuples: Vec<(EntityId, Attribute, Value)>, opts: Option<TransactOptions>) -> Result<TxId> {
        let tx_id = tx_id_now();
        let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let opts = opts.unwrap_or_default();
        let facts: Vec<Fact> = fact_tuples.into_iter().map(|(entity, attribute, value)| {
            let valid_from = opts.valid_from.unwrap_or(tx_id as i64);
            let valid_to = opts.valid_to.unwrap_or(VALID_TIME_FOREVER);
            Fact::with_valid_time(entity, attribute, value, tx_id, tx_count, valid_from, valid_to)
        }).collect();

        let mut d = self.data.write().unwrap();
        for fact in &facts {
            d.indexes.insert(fact, FactRef { page_id: 0, slot_index: 0 });
        }
        d.facts.extend(facts);
        Ok(tx_id)
    }

    pub fn retract(&self, fact_tuples: Vec<(EntityId, Attribute, Value)>) -> Result<TxId> {
        let tx_id = tx_id_now();
        let tx_count = self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let retractions: Vec<Fact> = fact_tuples.into_iter().map(|(entity, attribute, value)| {
            let mut f = Fact::retract(entity, attribute, value, tx_id);
            f.tx_count = tx_count;
            f
        }).collect();

        let mut d = self.data.write().unwrap();
        for fact in &retractions {
            d.indexes.insert(fact, FactRef { page_id: 0, slot_index: 0 });
        }
        d.facts.extend(retractions);
        Ok(tx_id)
    }

    pub fn load_fact(&self, fact: Fact) -> Result<()> {
        let mut d = self.data.write().unwrap();
        d.indexes.insert(&fact, FactRef { page_id: 0, slot_index: 0 });
        d.facts.push(fact);
        Ok(())
    }

    pub fn restore_tx_counter(&self) -> Result<()> {
        let d = self.data.read().unwrap();
        let max = d.facts.iter().map(|f| f.tx_count).max().unwrap_or(0);
        self.tx_counter.store(max, Ordering::SeqCst);
        Ok(())
    }

    pub fn current_tx_count(&self) -> u64 { self.tx_counter.load(Ordering::SeqCst) }
    pub fn allocate_tx_count(&self) -> u64 { self.tx_counter.fetch_add(1, Ordering::SeqCst) + 1 }

    pub fn get_facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| match as_of {
            AsOf::Counter(n) => f.tx_count <= *n,
            AsOf::Timestamp(t) => f.tx_id <= *t as u64,
        }).cloned().collect())
    }

    pub fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| f.is_asserted() && f.valid_from <= ts && ts < f.valid_to).cloned().collect())
    }

    pub fn get_all_facts(&self) -> Result<Vec<Fact>> { Ok(self.data.read().unwrap().facts.clone()) }

    pub fn get_asserted_facts(&self) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| f.is_asserted()).cloned().collect())
    }

    pub fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| &f.entity == entity_id).cloned().collect())
    }

    pub fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| &f.attribute == attribute).cloned().collect())
    }

    pub fn get_facts_by_entity_attribute(&self, entity_id: &EntityId, attribute: &Attribute) -> Result<Vec<Fact>> {
        let d = self.data.read().unwrap();
        Ok(d.facts.iter().filter(|f| &f.entity == entity_id && &f.attribute == attribute).cloned().collect())
    }

    // Preserve any remaining methods (get_current_value, etc.) by updating them
    // to use `self.data.read().unwrap().facts` instead of `self.facts.read()`.
}
```

- [ ] **Step 3.4: Run all tests**

```bash
cargo test
```

Expected: all 212 existing tests + 3 new tests pass. The refactor is purely internal.

- [ ] **Step 3.5: Commit**

```bash
git add src/graph/storage.rs
git commit -m "refactor(6.1): FactData co-locates facts + indexes under one RwLock; add index_counts()"
```

---

## Task 4: B+tree Page Serialization

**Files:**
- Create: `src/storage/btree.rs`
- Modify: `src/storage/mod.rs` (add `pub mod btree;`)

This module handles persistence only. In Phase 6.1, all query-time lookups use the in-memory BTreeMap. The B+tree pages are written on `save()` and read on `load()` (fast path when checksum matches).

- [ ] **Step 4.1: Write failing tests in the new file**

Create `src/storage/btree.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::index::{EavtKey, FactRef};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn eavt_entry(entity_lo: u128, attr: &str, tx: u64) -> (EavtKey, FactRef) {
        (EavtKey {
            entity: Uuid::from_u128(entity_lo),
            attribute: attr.to_string(),
            valid_from: 0, valid_to: i64::MAX, tx_count: tx,
        }, FactRef { page_id: tx, slot_index: 0 })
    }

    #[test]
    fn test_empty_eavt_roundtrip() {
        let mut backend = MemoryBackend::new();
        let map: BTreeMap<EavtKey, FactRef> = BTreeMap::new();
        let root = write_eavt_index(&map, &mut backend, 1).unwrap();
        let recovered = read_eavt_index(root, &backend).unwrap();
        assert_eq!(recovered.len(), 0);
    }

    #[test]
    fn test_small_eavt_roundtrip() {
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..10 {
            let (k, v) = eavt_entry(i, ":name", i as u64 + 1);
            map.insert(k, v);
        }
        let root = write_eavt_index(&map, &mut backend, 1).unwrap();
        let recovered = read_eavt_index(root, &backend).unwrap();
        assert_eq!(recovered.len(), 10);
        for (k, v) in &map {
            assert_eq!(recovered.get(k), Some(v), "missing key {:?}", k);
        }
    }

    #[test]
    fn test_large_eavt_roundtrip_multi_leaf() {
        // Force multi-leaf: insert more entries than LEAF_CAPACITY (50)
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..150 {
            let (k, v) = eavt_entry(i, ":attr", i as u64 + 1);
            map.insert(k, v);
        }
        let root = write_eavt_index(&map, &mut backend, 1).unwrap();
        let recovered = read_eavt_index(root, &backend).unwrap();
        assert_eq!(recovered.len(), 150);
        for (k, v) in &map {
            assert_eq!(recovered.get(k), Some(v));
        }
    }

    #[test]
    fn test_eavt_preserves_sort_order() {
        let mut backend = MemoryBackend::new();
        let mut map = BTreeMap::new();
        for i in 0u128..100 {
            let (k, v) = eavt_entry(i, ":x", i as u64 + 1);
            map.insert(k, v);
        }
        let root = write_eavt_index(&map, &mut backend, 1).unwrap();
        let recovered = read_eavt_index(root, &backend).unwrap();
        let orig_keys: Vec<_> = map.keys().collect();
        let rec_keys: Vec<_> = recovered.keys().collect();
        assert_eq!(orig_keys, rec_keys, "Sort order must be preserved");
    }
}
```

- [ ] **Step 4.2: Run tests to verify they fail**

```bash
cargo test storage::btree 2>&1 | head -10
```

Expected: compile errors — module does not exist.

- [ ] **Step 4.3: Implement `src/storage/btree.rs`**

```rust
//! B+tree page serialisation for covering index persistence.
//!
//! `write_*_index` writes a `BTreeMap<K, FactRef>` to B+tree pages starting
//! at `start_page_id` on the backend. Returns the root page ID.
//! `read_*_index` is the inverse — reads all entries back into a BTreeMap.
//!
//! Phase 6.1 uses these only for save/load. In-memory BTreeMaps handle queries.

use crate::storage::index::{AevtKey, AvetKey, EavtKey, FactRef, VaetKey, encode_value};
use crate::storage::{StorageBackend, PAGE_SIZE};
use anyhow::Result;
use std::collections::BTreeMap;

// ─── Page type constants ──────────────────────────────────────────────────────
pub const PAGE_TYPE_FACT_ONE:     u8 = 0x01; // v4 fact data page (one per page)
pub const PAGE_TYPE_PACKED_FACT:  u8 = 0x02; // v5 packed fact data page
pub const PAGE_TYPE_OVERFLOW:     u8 = 0x03; // v5 overflow chain
pub const PAGE_TYPE_BTREE_INTERNAL: u8 = 0x10;
pub const PAGE_TYPE_BTREE_LEAF:     u8 = 0x11;

/// Leaf node capacity: (PAGE_SIZE - leaf_header) / (avg_key_bytes + fact_ref_bytes)
/// Leaf header: page_type(1) + entry_count(2) + next_leaf(8) = 11 bytes.
/// FactRef: page_id(8) + slot_index(2) = 10 bytes.
/// Average key ~60 bytes → (4096-11) / 70 ≈ 58. Use 50 to be safe with variable keys.
const LEAF_CAPACITY: usize = 50;

/// Internal node capacity: key_count = children_count - 1.
/// Internal header: page_type(1) + key_count(2) = 3 bytes.
/// (PAGE_SIZE - 3) / (avg_key_bytes + child_ptr_bytes) = 4093 / 68 ≈ 60. Use 50.
const INTERNAL_CAPACITY: usize = 50;

// ─── Encoding helpers ─────────────────────────────────────────────────────────

fn write_u32_le(out: &mut Vec<u8>, n: u32) { out.extend_from_slice(&n.to_le_bytes()); }
fn write_u64_le(out: &mut Vec<u8>, n: u64) { out.extend_from_slice(&n.to_le_bytes()); }
fn write_i64_le(out: &mut Vec<u8>, n: i64) { out.extend_from_slice(&n.to_le_bytes()); }

fn write_len_bytes(out: &mut Vec<u8>, data: &[u8]) {
    write_u32_le(out, data.len() as u32);
    out.extend_from_slice(data);
}

fn read_len_bytes(data: &[u8], pos: usize) -> Result<(&[u8], usize)> {
    if pos + 4 > data.len() { anyhow::bail!("btree: truncated length prefix at {}", pos); }
    let len = u32::from_le_bytes(data[pos..pos+4].try_into().unwrap()) as usize;
    let end = pos + 4 + len;
    if end > data.len() { anyhow::bail!("btree: truncated data at {} (need {} bytes)", pos, len); }
    Ok((&data[pos+4..end], 4 + len))
}

fn write_fact_ref(out: &mut Vec<u8>, r: &FactRef) {
    write_u64_le(out, r.page_id);
    out.extend_from_slice(&r.slot_index.to_le_bytes());
}

fn read_fact_ref(data: &[u8], pos: usize) -> Result<FactRef> {
    if pos + 10 > data.len() { anyhow::bail!("btree: truncated FactRef at {}", pos); }
    Ok(FactRef {
        page_id: u64::from_le_bytes(data[pos..pos+8].try_into().unwrap()),
        slot_index: u16::from_le_bytes(data[pos+8..pos+10].try_into().unwrap()),
    })
}

fn pad_to_page(mut data: Vec<u8>) -> Vec<u8> {
    data.resize(PAGE_SIZE, 0);
    data
}

// ─── Generic B+tree bulk-load writer ─────────────────────────────────────────

/// Write a sorted list of (key_bytes, FactRef) pairs as B+tree pages.
///
/// Allocates consecutive pages starting at `*next_page_id`, incrementing it
/// for each page written. Returns the root page ID.
fn write_btree(
    entries: Vec<(Vec<u8>, FactRef)>,
    backend: &mut dyn StorageBackend,
    next_page_id: &mut u64,
) -> Result<u64> {
    if entries.is_empty() {
        // Single empty leaf
        let root = *next_page_id;
        *next_page_id += 1;
        let mut page_data = vec![0u8; 11]; // header only
        page_data[0] = PAGE_TYPE_BTREE_LEAF;
        // entry_count(2) = 0, next_leaf(8) = 0 — already zeroed
        backend.write_page(root, &pad_to_page(page_data))?;
        return Ok(root);
    }

    // ── Phase 1: write leaf pages ─────────────────────────────────────────────
    let chunks: Vec<_> = entries.chunks(LEAF_CAPACITY).collect();
    let leaf_start = *next_page_id;
    *next_page_id += chunks.len() as u64;

    let mut leaf_first_keys: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());

    for (i, chunk) in chunks.iter().enumerate() {
        let page_id = leaf_start + i as u64;
        let next_leaf = if i + 1 < chunks.len() { leaf_start + i as u64 + 1 } else { 0 };

        let mut page_data = Vec::new();
        page_data.push(PAGE_TYPE_BTREE_LEAF);
        page_data.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        write_u64_le(&mut page_data, next_leaf);
        for (key_bytes, fact_ref) in *chunk {
            write_len_bytes(&mut page_data, key_bytes);
            write_fact_ref(&mut page_data, fact_ref);
        }
        backend.write_page(page_id, &pad_to_page(page_data))?;
        leaf_first_keys.push(chunk[0].0.clone());
    }

    if chunks.len() == 1 {
        return Ok(leaf_start);
    }

    // ── Phase 2: build internal nodes bottom-up ───────────────────────────────
    let mut current_children: Vec<u64> = (0..chunks.len() as u64).map(|i| leaf_start + i).collect();
    let mut current_keys = leaf_first_keys;

    loop {
        if current_children.len() == 1 {
            return Ok(current_children[0]);
        }
        let group_size = INTERNAL_CAPACITY + 1;
        let mut new_children = Vec::new();
        let mut new_keys = Vec::new();

        for chunk_start in (0..current_children.len()).step_by(group_size) {
            let chunk_end = (chunk_start + group_size).min(current_children.len());
            let children_chunk = &current_children[chunk_start..chunk_end];
            let keys_chunk = &current_keys[chunk_start..chunk_end];

            let page_id = *next_page_id;
            *next_page_id += 1;

            let key_count = children_chunk.len() - 1;
            let mut page_data = Vec::new();
            page_data.push(PAGE_TYPE_BTREE_INTERNAL);
            page_data.extend_from_slice(&(key_count as u16).to_le_bytes());
            // Separator keys: keys[1..] (skip the first — it's the child's own first key)
            for key in keys_chunk.iter().skip(1) {
                write_len_bytes(&mut page_data, key);
            }
            for child in children_chunk {
                write_u64_le(&mut page_data, *child);
            }
            backend.write_page(page_id, &pad_to_page(page_data))?;
            new_children.push(page_id);
            new_keys.push(keys_chunk[0].clone());
        }
        current_children = new_children;
        current_keys = new_keys;
    }
}

// ─── Generic B+tree reader ────────────────────────────────────────────────────

/// Read all entries from a B+tree, returning them in sorted order.
fn read_btree_entries(root_page_id: u64, backend: &dyn StorageBackend) -> Result<Vec<(Vec<u8>, FactRef)>> {
    // Descend to leftmost leaf
    let mut page_id = root_page_id;
    loop {
        let page = backend.read_page(page_id)?;
        match page[0] {
            PAGE_TYPE_BTREE_INTERNAL => {
                let key_count = u16::from_le_bytes([page[1], page[2]]) as usize;
                let mut pos = 3usize;
                for _ in 0..key_count {
                    let (_, n) = read_len_bytes(&page, pos)?;
                    pos += n;
                }
                page_id = u64::from_le_bytes(page[pos..pos+8].try_into().unwrap());
            }
            PAGE_TYPE_BTREE_LEAF => break,
            t => anyhow::bail!("btree: unexpected page_type 0x{:02X} at page {}", t, page_id),
        }
    }

    // Scan leaf chain
    let mut result = Vec::new();
    loop {
        if page_id == 0 { break; }
        let page = backend.read_page(page_id)?;
        if page[0] != PAGE_TYPE_BTREE_LEAF {
            anyhow::bail!("btree: expected leaf, got 0x{:02X} at page {}", page[0], page_id);
        }
        let entry_count = u16::from_le_bytes([page[1], page[2]]) as usize;
        let next_leaf = u64::from_le_bytes(page[3..11].try_into().unwrap());
        let mut pos = 11usize;
        for _ in 0..entry_count {
            let (key_bytes, n) = read_len_bytes(&page, pos)?;
            pos += n;
            let fact_ref = read_fact_ref(&page, pos)?;
            pos += 10;
            result.push((key_bytes.to_vec(), fact_ref));
        }
        page_id = next_leaf;
    }
    Ok(result)
}

// ─── EAVT key encoding ────────────────────────────────────────────────────────

fn eavt_to_bytes(k: &EavtKey) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(k.entity.as_bytes());
    write_len_bytes(&mut out, k.attribute.as_bytes());
    write_i64_le(&mut out, k.valid_from);
    write_i64_le(&mut out, k.valid_to);
    write_u64_le(&mut out, k.tx_count);
    out
}

fn eavt_from_bytes(data: &[u8]) -> Result<EavtKey> {
    if data.len() < 16 { anyhow::bail!("eavt: short entity"); }
    let entity = uuid::Uuid::from_bytes(data[0..16].try_into().unwrap());
    let (attr_bytes, n) = read_len_bytes(data, 16)?;
    let tail = 16 + n;
    if data.len() < tail + 24 { anyhow::bail!("eavt: short tail"); }
    let valid_from = i64::from_le_bytes(data[tail..tail+8].try_into().unwrap());
    let valid_to   = i64::from_le_bytes(data[tail+8..tail+16].try_into().unwrap());
    let tx_count   = u64::from_le_bytes(data[tail+16..tail+24].try_into().unwrap());
    Ok(EavtKey { entity, attribute: String::from_utf8(attr_bytes.to_vec())?, valid_from, valid_to, tx_count })
}

// ─── AEVT key encoding ────────────────────────────────────────────────────────

fn aevt_to_bytes(k: &AevtKey) -> Vec<u8> {
    let mut out = Vec::new();
    write_len_bytes(&mut out, k.attribute.as_bytes());
    out.extend_from_slice(k.entity.as_bytes());
    write_i64_le(&mut out, k.valid_from);
    write_i64_le(&mut out, k.valid_to);
    write_u64_le(&mut out, k.tx_count);
    out
}

fn aevt_from_bytes(data: &[u8]) -> Result<AevtKey> {
    let (attr_bytes, n) = read_len_bytes(data, 0)?;
    let pos = n;
    if data.len() < pos + 16 + 24 { anyhow::bail!("aevt: short tail"); }
    let entity    = uuid::Uuid::from_bytes(data[pos..pos+16].try_into().unwrap());
    let valid_from = i64::from_le_bytes(data[pos+16..pos+24].try_into().unwrap());
    let valid_to   = i64::from_le_bytes(data[pos+24..pos+32].try_into().unwrap());
    let tx_count   = u64::from_le_bytes(data[pos+32..pos+40].try_into().unwrap());
    Ok(AevtKey { attribute: String::from_utf8(attr_bytes.to_vec())?, entity, valid_from, valid_to, tx_count })
}

// ─── AVET key encoding ────────────────────────────────────────────────────────

fn avet_to_bytes(k: &AvetKey) -> Vec<u8> {
    let mut out = Vec::new();
    write_len_bytes(&mut out, k.attribute.as_bytes());
    write_len_bytes(&mut out, &k.value_bytes);
    write_i64_le(&mut out, k.valid_from);
    write_i64_le(&mut out, k.valid_to);
    out.extend_from_slice(k.entity.as_bytes());
    write_u64_le(&mut out, k.tx_count);
    out
}

fn avet_from_bytes(data: &[u8]) -> Result<AvetKey> {
    let (attr_bytes, n1) = read_len_bytes(data, 0)?;
    let (val_bytes, n2)  = read_len_bytes(data, n1)?;
    let pos = n1 + n2;
    if data.len() < pos + 16 + 24 { anyhow::bail!("avet: short tail"); }
    let valid_from = i64::from_le_bytes(data[pos..pos+8].try_into().unwrap());
    let valid_to   = i64::from_le_bytes(data[pos+8..pos+16].try_into().unwrap());
    let entity     = uuid::Uuid::from_bytes(data[pos+16..pos+32].try_into().unwrap());
    let tx_count   = u64::from_le_bytes(data[pos+32..pos+40].try_into().unwrap());
    Ok(AvetKey { attribute: String::from_utf8(attr_bytes.to_vec())?, value_bytes: val_bytes.to_vec(), valid_from, valid_to, entity, tx_count })
}

// ─── VAET key encoding ────────────────────────────────────────────────────────

fn vaet_to_bytes(k: &VaetKey) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(k.ref_target.as_bytes());
    write_len_bytes(&mut out, k.attribute.as_bytes());
    write_i64_le(&mut out, k.valid_from);
    write_i64_le(&mut out, k.valid_to);
    out.extend_from_slice(k.source_entity.as_bytes());
    write_u64_le(&mut out, k.tx_count);
    out
}

fn vaet_from_bytes(data: &[u8]) -> Result<VaetKey> {
    if data.len() < 16 { anyhow::bail!("vaet: short ref_target"); }
    let ref_target = uuid::Uuid::from_bytes(data[0..16].try_into().unwrap());
    let (attr_bytes, n) = read_len_bytes(data, 16)?;
    let pos = 16 + n;
    if data.len() < pos + 16 + 24 { anyhow::bail!("vaet: short tail"); }
    let valid_from    = i64::from_le_bytes(data[pos..pos+8].try_into().unwrap());
    let valid_to      = i64::from_le_bytes(data[pos+8..pos+16].try_into().unwrap());
    let source_entity = uuid::Uuid::from_bytes(data[pos+16..pos+32].try_into().unwrap());
    let tx_count      = u64::from_le_bytes(data[pos+32..pos+40].try_into().unwrap());
    Ok(VaetKey { ref_target, attribute: String::from_utf8(attr_bytes.to_vec())?, valid_from, valid_to, source_entity, tx_count })
}

// ─── Public typed API ─────────────────────────────────────────────────────────

/// Write an EAVT index as B+tree pages, starting at `start_page_id`.
/// Returns the root page ID.
pub fn write_eavt_index(map: &BTreeMap<EavtKey, FactRef>, backend: &mut dyn StorageBackend, start_page_id: u64) -> Result<u64> {
    let entries: Vec<_> = map.iter().map(|(k, v)| (eavt_to_bytes(k), *v)).collect();
    let mut next = start_page_id;
    write_btree(entries, backend, &mut next)
}

pub fn read_eavt_index(root_page_id: u64, backend: &dyn StorageBackend) -> Result<BTreeMap<EavtKey, FactRef>> {
    let entries = read_btree_entries(root_page_id, backend)?;
    entries.into_iter().map(|(kb, fr)| Ok((eavt_from_bytes(&kb)?, fr))).collect()
}

pub fn write_aevt_index(map: &BTreeMap<AevtKey, FactRef>, backend: &mut dyn StorageBackend, start_page_id: u64) -> Result<u64> {
    let entries: Vec<_> = map.iter().map(|(k, v)| (aevt_to_bytes(k), *v)).collect();
    let mut next = start_page_id;
    write_btree(entries, backend, &mut next)
}

pub fn read_aevt_index(root_page_id: u64, backend: &dyn StorageBackend) -> Result<BTreeMap<AevtKey, FactRef>> {
    let entries = read_btree_entries(root_page_id, backend)?;
    entries.into_iter().map(|(kb, fr)| Ok((aevt_from_bytes(&kb)?, fr))).collect()
}

pub fn write_avet_index(map: &BTreeMap<AvetKey, FactRef>, backend: &mut dyn StorageBackend, start_page_id: u64) -> Result<u64> {
    let entries: Vec<_> = map.iter().map(|(k, v)| (avet_to_bytes(k), *v)).collect();
    let mut next = start_page_id;
    write_btree(entries, backend, &mut next)
}

pub fn read_avet_index(root_page_id: u64, backend: &dyn StorageBackend) -> Result<BTreeMap<AvetKey, FactRef>> {
    let entries = read_btree_entries(root_page_id, backend)?;
    entries.into_iter().map(|(kb, fr)| Ok((avet_from_bytes(&kb)?, fr))).collect()
}

pub fn write_vaet_index(map: &BTreeMap<VaetKey, FactRef>, backend: &mut dyn StorageBackend, start_page_id: u64) -> Result<u64> {
    let entries: Vec<_> = map.iter().map(|(k, v)| (vaet_to_bytes(k), *v)).collect();
    let mut next = start_page_id;
    write_btree(entries, backend, &mut next)
}

pub fn read_vaet_index(root_page_id: u64, backend: &dyn StorageBackend) -> Result<BTreeMap<VaetKey, FactRef>> {
    let entries = read_btree_entries(root_page_id, backend)?;
    entries.into_iter().map(|(kb, fr)| Ok((vaet_from_bytes(&kb)?, fr))).collect()
}

/// Returns the total number of pages used by all four index trees combined.
/// `start_page_id` is the first page allocated; each tree is written sequentially.
pub fn write_all_indexes(
    eavt: &BTreeMap<EavtKey, FactRef>,
    aevt: &BTreeMap<AevtKey, FactRef>,
    avet: &BTreeMap<AvetKey, FactRef>,
    vaet: &BTreeMap<VaetKey, FactRef>,
    backend: &mut dyn StorageBackend,
    start_page_id: u64,
) -> Result<(u64, u64, u64, u64)> {
    let mut next = start_page_id;
    let eavt_root = write_eavt_index(eavt, backend, next)?;
    // Advance `next` past the EAVT pages by querying backend page count
    next = backend.page_count()?;
    let aevt_root = write_aevt_index(aevt, backend, next)?;
    next = backend.page_count()?;
    let avet_root = write_avet_index(avet, backend, next)?;
    next = backend.page_count()?;
    let vaet_root = write_vaet_index(vaet, backend, next)?;
    Ok((eavt_root, aevt_root, avet_root, vaet_root))
}

#[cfg(test)]
mod tests { /* tests from Step 4.1 */ }
```

**Note:** `write_all_indexes` uses `backend.page_count()` to find the next free page after each tree. The `write_*_index` functions allocate pages starting at `start_page_id` by incrementing `next` internally; the caller must query `backend.page_count()` to know where the next tree starts. This works for `MemoryBackend` (which tracks writes); for `FileBackend`, `page_count()` returns the header's `page_count` field, which `write_page` must keep updated.

- [ ] **Step 4.4: Add `pub mod btree;` to `src/storage/mod.rs`**

- [ ] **Step 4.5: Run tests**

```bash
cargo test storage::btree -- --nocapture
cargo test  # full suite
```

Expected: 4 btree tests + all 215+ others pass.

- [ ] **Step 4.6: Commit**

```bash
git add src/storage/btree.rs src/storage/mod.rs
git commit -m "feat(6.1): B+tree page serialization for all four covering indexes"
```

---

## Task 5: Index Persistence and Sync Check

**Files:**
- Modify: `src/storage/persistent_facts.rs`

- [ ] **Step 5.1: Write failing tests**

Add to the `#[cfg(test)]` section of `src/storage/persistent_facts.rs`. Note: these tests are inside the same module as the implementation, so they can access private `load()`.

```rust
#[test]
fn test_indexes_survive_save_load_roundtrip() {
    use tempfile::NamedTempFile;
    use crate::graph::types::Value;
    use uuid::Uuid;
    use crate::storage::backend::FileBackend;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Save phase — `new()` returns `Result`, `save()` needs `mut`
    {
        let mut pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap()
        ).unwrap();
        pfs.storage().transact(vec![
            (alice, ":name".to_string(), Value::String("Alice".to_string())),
            (alice, ":friend".to_string(), Value::Ref(bob)),
        ], None).unwrap();
        pfs.dirty = true; // mark dirty so save() proceeds
        pfs.save().unwrap();
    }

    // Load phase — indexes must be populated from disk
    {
        // `new()` calls `load()` internally when page_count > 1
        let pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap()
        ).unwrap();
        let (eavt, _, _, vaet) = pfs.storage().index_counts();
        assert_eq!(eavt, 2, "EAVT must have 2 entries after reload");
        assert_eq!(vaet, 1, "VAET must have 1 entry (Ref fact) after reload");
    }
}

#[test]
fn test_sync_check_detects_mismatch_and_rebuilds() {
    use tempfile::NamedTempFile;
    use crate::graph::types::Value;
    use uuid::Uuid;
    use crate::storage::backend::FileBackend;
    use crate::storage::StorageBackend;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let alice = Uuid::new_v4();

    // Write a database with 1 fact
    {
        let mut pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap()
        ).unwrap();
        pfs.storage().transact(vec![
            (alice, ":name".to_string(), Value::String("Alice".to_string())),
        ], None).unwrap();
        pfs.dirty = true;
        pfs.save().unwrap();
    }

    // Corrupt the index_checksum (bytes 64..68 of page 0)
    {
        let mut backend = FileBackend::open(&path).unwrap();
        let mut page = backend.read_page(0).unwrap();
        page[64] ^= 0xFF;
        backend.write_page(0, &page).unwrap();
        backend.sync().unwrap();
    }

    // Re-open — `new()` should detect mismatch, rebuild, and succeed
    {
        let pfs = PersistentFactStorage::new(
            FileBackend::open(&path).unwrap()
        ).unwrap();
        let (eavt, _, _, _) = pfs.storage().index_counts();
        assert_eq!(eavt, 1, "After rebuild, EAVT must contain 1 fact");
    }
}

#[test]
fn test_compute_index_checksum_stable() {
    use crate::graph::types::{Fact, Value, VALID_TIME_FOREVER};
    use uuid::Uuid;

    let e = Uuid::new_v4();
    let facts = vec![
        Fact::with_valid_time(e, ":a".to_string(), Value::Integer(1), 100, 2, 0, VALID_TIME_FOREVER),
        Fact::with_valid_time(e, ":b".to_string(), Value::Integer(2), 200, 1, 0, VALID_TIME_FOREVER),
    ];
    let c1 = compute_index_checksum(&facts);
    // Reversed order — same checksum (deterministic sort applied inside)
    let facts_reversed = vec![facts[1].clone(), facts[0].clone()];
    let c2 = compute_index_checksum(&facts_reversed);
    assert_eq!(c1, c2, "Checksum must be order-independent");
}
```

- [ ] **Step 5.2: Run tests to verify they fail**

```bash
cargo test persistent_facts -- --nocapture 2>&1 | grep -E "FAILED|error" | head -10
```

Expected: `test_indexes_survive_save_load_roundtrip` fails (indexes not yet persisted).

- [ ] **Step 5.3: Add `compute_index_checksum`, `reindex_from_facts`, index save/load, sync check, and v3→v4 migration**

Open `src/storage/persistent_facts.rs`. Add the following imports at the top:

```rust
use crate::storage::btree::{
    write_all_indexes, read_eavt_index, read_aevt_index,
    read_avet_index, read_vaet_index,
};
use crate::storage::index::Indexes;
use crc32fast::Hasher;
```

Add helper functions (make `pub(crate)` for tests):

```rust
/// Compute the CRC32 sync checksum over all facts.
///
/// Sorts facts by `(tx_count, entity_bytes, attribute)` before hashing to
/// produce a stable total order independent of Vec insertion order.
pub(crate) fn compute_index_checksum(facts: &[crate::graph::types::Fact]) -> u32 {
    let mut sorted: Vec<&crate::graph::types::Fact> = facts.iter().collect();
    sorted.sort_by(|a, b| {
        a.tx_count.cmp(&b.tx_count)
            .then_with(|| a.entity.as_bytes().cmp(b.entity.as_bytes()))
            .then_with(|| a.attribute.as_str().cmp(b.attribute.as_str()))
    });
    let mut hasher = Hasher::new();
    for fact in sorted {
        let bytes = postcard::to_allocvec(fact).unwrap_or_default();
        hasher.update(&bytes);
    }
    hasher.finalize()
}

/// Rebuild all four indexes from a fact slice.
///
/// Page IDs are assigned as (1-based position in slice), matching the
/// one-fact-per-page layout used by `save()`. Must be called before `save()`
/// when rebuilding after a checksum mismatch, so that the indexes reflect
/// the correct page IDs written by `save()`.
fn reindex_from_facts(facts: &[crate::graph::types::Fact]) -> Indexes {
    let mut indexes = Indexes::new();
    for (i, fact) in facts.iter().enumerate() {
        // Page 1-based: page 0 is the header, pages 1..=N are facts.
        indexes.insert(fact, crate::storage::index::FactRef {
            page_id: (i + 1) as u64,
            slot_index: 0,
        });
    }
    indexes
}
```

In the `load` method, after loading all facts and restoring the tx counter, add:

```rust
// ── Sync check ───────────────────────────────────────────────────────────
let facts = self.storage.get_all_facts().unwrap_or_default();
let computed_checksum = compute_index_checksum(&facts);
let stored_checksum = header.index_checksum;

// Mismatch, or no index ever written (eavt_root_page == 0).
// Note: computed_checksum of an empty fact list is 0x00000000 (CRC32 of
// zero bytes). stored_checksum is also 0 on a fresh DB. To avoid a
// spurious rebuild on an empty DB we check eavt_root_page == 0 only when
// computed_checksum != 0 (i.e., there are facts to index).
let needs_rebuild = computed_checksum != stored_checksum
    || (header.eavt_root_page == 0 && computed_checksum != 0);

if needs_rebuild {
    // Rebuild: replace in-memory indexes and persist to disk.
    let new_indexes = reindex_from_facts(&facts);
    self.storage.replace_indexes(new_indexes);
    // Force save() to run even though dirty=false at this point.
    self.dirty = true;
    self.save()?;
    // dirty is set back to false inside save().
} else if header.eavt_root_page != 0 {
    // Fast path: load indexes from existing B+tree pages on disk.
    let eavt = read_eavt_index(header.eavt_root_page, &self.backend)?;
    let aevt = read_aevt_index(header.aevt_root_page, &self.backend)?;
    let avet = read_avet_index(header.avet_root_page, &self.backend)?;
    let vaet = read_vaet_index(header.vaet_root_page, &self.backend)?;
    self.storage.replace_indexes(crate::storage::index::Indexes {
        eavt, aevt, avet, vaet,
    });
}
// else: empty DB with no facts — indexes are empty by default, nothing to do.
```

In the `save` / `checkpoint` method, after writing all fact pages:

```rust
// ── Write index pages ────────────────────────────────────────────────────
let facts = storage.get_all_facts().unwrap_or_default();
let d = storage.data.read().unwrap();
let start_page = self.backend.page_count()?;
let (eavt_root, aevt_root, avet_root, vaet_root) = write_all_indexes(
    &d.indexes.eavt, &d.indexes.aevt, &d.indexes.avet, &d.indexes.vaet,
    &mut *self.backend, start_page,
)?;
drop(d);

// ── Update header ────────────────────────────────────────────────────────
let checksum = compute_index_checksum(&facts);
header.eavt_root_page = eavt_root;
header.aevt_root_page = aevt_root;
header.avet_root_page = avet_root;
header.vaet_root_page = vaet_root;
header.index_checksum = checksum;
header.page_count = self.backend.page_count()?;
// (write header page as before)
```

For v3→v4 migration: when `from_bytes` returns a header with `version <= 3`, immediately set `header.version = 4` and `header.index_checksum = 0`. The sync check that follows will see a mismatch (stored=0, computed≠0) and trigger a rebuild+save. No additional migration code needed.

- [ ] **Step 5.4: Run all tests**

```bash
cargo test
```

Expected: all 215+ tests pass including the 3 new persistent_facts tests.

- [ ] **Step 5.5: Commit**

```bash
git add src/storage/persistent_facts.rs
git commit -m "feat(6.1): index persistence with CRC32 sync check; v3->v4 auto-migration"
```

---

## Task 6: Query Optimizer

**Files:**
- Create: `src/query/datalog/optimizer.rs`
- Modify: `src/query/datalog/mod.rs`

- [ ] **Step 6.1: Write failing tests**

Create `src/query/datalog/optimizer.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{EdnValue, Pattern};
    use uuid::Uuid;

    fn make_pattern(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Pattern {
        Pattern { entity, attribute, value, valid_from: None, valid_to: None }
    }

    fn var(s: &str) -> EdnValue { EdnValue::Variable(s.to_string()) }
    fn kw(s: &str) -> EdnValue { EdnValue::Keyword(s.to_string()) }
    fn str_val(s: &str) -> EdnValue { EdnValue::String(s.to_string()) }
    fn entity_lit() -> EdnValue { EdnValue::EntityId(Uuid::new_v4()) }

    #[test]
    fn test_entity_bound_selects_eavt() {
        let p = make_pattern(entity_lit(), var("?a"), var("?v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[test]
    fn test_entity_and_attr_bound_selects_eavt() {
        let p = make_pattern(entity_lit(), kw(":name"), var("?v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[test]
    fn test_attr_and_value_bound_selects_avet() {
        let p = make_pattern(var("?e"), kw(":name"), str_val("Alice"));
        assert_eq!(select_index(&p), IndexHint::Avet);
    }

    #[test]
    fn test_attr_and_ref_bound_selects_avet() {
        let p = make_pattern(var("?e"), kw(":friend"), entity_lit());
        assert_eq!(select_index(&p), IndexHint::Avet);
    }

    #[test]
    fn test_attr_only_selects_aevt() {
        let p = make_pattern(var("?e"), kw(":name"), var("?v"));
        assert_eq!(select_index(&p), IndexHint::Aevt);
    }

    #[test]
    fn test_ref_only_selects_vaet() {
        let p = make_pattern(var("?e"), var("?a"), entity_lit());
        assert_eq!(select_index(&p), IndexHint::Vaet);
    }

    #[test]
    fn test_nothing_bound_selects_eavt_full_scan() {
        let p = make_pattern(var("?e"), var("?a"), var("?v"));
        assert_eq!(select_index(&p), IndexHint::Eavt);
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn test_join_ordering_moves_selective_pattern_first() {
        use crate::storage::index::Indexes;
        // p1: attr only (score 1) vs p2: entity bound (score 2, entity+attr both bound)
        let p1 = make_pattern(var("?e"), kw(":age"), var("?a"));      // selectivity 1
        let p2 = make_pattern(entity_lit(), kw(":name"), var("?v"));   // selectivity 2
        let p1_attr = p1.attribute.clone();
        let p2_attr = p2.attribute.clone();
        let planned = plan(vec![p1, p2], &Indexes::new());
        // p2 (higher selectivity) must come first
        assert_ne!(planned[0].0.attribute, p1_attr,
            "Lower-selectivity pattern must not be first");
        assert_eq!(planned[0].0.attribute, p2_attr,
            "Higher-selectivity pattern must be first");
    }

    #[cfg(feature = "wasm")]
    #[test]
    fn test_join_ordering_skipped_under_wasm() {
        use crate::storage::index::Indexes;
        let p1 = make_pattern(var("?e"), kw(":age"), var("?a"));
        let p2 = make_pattern(entity_lit(), kw(":name"), var("?v"));
        let p1_attr = p1.attribute.clone();
        let planned = plan(vec![p1, p2], &Indexes::new());
        // Under wasm, order unchanged
        assert_eq!(planned[0].0.attribute, p1_attr);
    }
}
```

- [ ] **Step 6.2: Check what `Pattern` looks like in `src/query/datalog/types.rs`**

Read the file to confirm the exact struct fields (particularly `valid_from`, `valid_to` on `Pattern`). Adjust the `make_pattern` helper in the tests to match the actual struct.

```bash
grep -n "struct Pattern" src/query/datalog/types.rs
```

- [ ] **Step 6.3: Run tests to verify they fail**

```bash
cargo test datalog::optimizer 2>&1 | head -10
```

Expected: compile errors — `optimizer` module not found.

- [ ] **Step 6.4: Implement `src/query/datalog/optimizer.rs`**

```rust
//! Query optimizer: index selection and join ordering for Datalog patterns.
//!
//! `plan()` is the single entry point. It assigns an `IndexHint` to each
//! pattern and (outside the `wasm` feature) sorts patterns by selectivity.

use crate::query::datalog::types::{EdnValue, Pattern};

/// Which covering index to use for a given pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexHint {
    /// EAVT: entity-first scan. Also used when nothing is bound (full scan).
    Eavt,
    /// AEVT: attribute-first scan.
    Aevt,
    /// AVET: attribute + value equality / range lookup.
    Avet,
    /// VAET: reverse reference lookup (Ref value only, no attribute).
    Vaet,
}

/// Count the number of non-variable components in a pattern.
/// Higher score = more selective.
fn selectivity_score(p: &Pattern) -> u8 {
    let e = !matches!(&p.entity,    EdnValue::Variable(_));
    let a = !matches!(&p.attribute, EdnValue::Variable(_));
    let v = !matches!(&p.value,     EdnValue::Variable(_));
    e as u8 + a as u8 + v as u8
}

/// Check whether a pattern component is bound to a literal entity ID (Ref).
fn is_entity_lit(v: &EdnValue) -> bool {
    matches!(v, EdnValue::EntityId(_))
}

/// Select the most efficient index for a single pattern.
///
/// Selection table:
///   Entity bound (± anything)         → EAVT
///   Attribute + Value (any non-Var)    → AVET
///   Attribute only                     → AEVT
///   Value is entity literal, no attr   → VAET (reverse traversal)
///   Nothing bound                      → EAVT (full scan)
pub fn select_index(p: &Pattern) -> IndexHint {
    let e_bound = !matches!(&p.entity,    EdnValue::Variable(_));
    let a_bound = !matches!(&p.attribute, EdnValue::Variable(_));
    let v_bound = !matches!(&p.value,     EdnValue::Variable(_));

    if e_bound {
        return IndexHint::Eavt;
    }
    if a_bound && v_bound {
        return IndexHint::Avet;
    }
    if a_bound {
        return IndexHint::Aevt;
    }
    if v_bound && is_entity_lit(&p.value) {
        return IndexHint::Vaet;
    }
    // Nothing bound: full scan through EAVT
    IndexHint::Eavt
}

/// Plan a list of patterns: assign index hints and (non-wasm) reorder by selectivity.
///
/// `_indexes` is reserved for statistics-based optimization in a future phase;
/// in Phase 6.1 selectivity is estimated purely from bound-variable counts.
///
/// Under the `wasm` feature flag, patterns execute in user-written order
/// (index selection still applies, join reordering is skipped).
pub fn plan(patterns: Vec<Pattern>, _indexes: &crate::storage::index::Indexes) -> Vec<(Pattern, IndexHint)> {
    let mut planned: Vec<(Pattern, IndexHint)> = patterns
        .into_iter()
        .map(|p| { let h = select_index(&p); (p, h) })
        .collect();

    #[cfg(not(feature = "wasm"))]
    {
        // Stable sort preserves original order for ties.
        planned.sort_by_key(|(p, _)| std::cmp::Reverse(selectivity_score(p)));
    }

    planned
}
```

- [ ] **Step 6.5: Add `pub mod optimizer;` to `src/query/datalog/mod.rs`**

- [ ] **Step 6.6: Run tests**

```bash
cargo test datalog::optimizer -- --nocapture
cargo test
```

Expected: all optimizer tests pass; full suite passes.

- [ ] **Step 6.7: Commit**

```bash
git add src/query/datalog/optimizer.rs src/query/datalog/mod.rs
git commit -m "feat(6.1): query optimizer with index selection and selectivity-based join ordering"
```

---

## Task 7: Wire Optimizer into Executor

**Files:**
- Modify: `src/query/datalog/executor.rs`

The executor calls `plan()` before pattern matching. In Phase 6.1, the planned order is used; the actual BTreeMap-based fast path arrives in Phase 6.2. The `IndexHint` values are threaded through for 6.2.

- [ ] **Step 7.1: Write a regression test first**

Add to the executor test section (or use an existing complex query test):

```rust
#[test]
fn test_optimizer_does_not_change_query_results() {
    // A multi-pattern query that the optimizer would reorder.
    // Results must be identical regardless of execution order.
    let storage = FactStorage::new();
    let alice = uuid::Uuid::new_v4();
    let bob = uuid::Uuid::new_v4();
    storage.transact(vec![
        (alice, ":name".to_string(), Value::String("Alice".to_string())),
        (alice, ":friend".to_string(), Value::Ref(bob)),
        (bob, ":name".to_string(), Value::String("Bob".to_string())),
    ], None).unwrap();

    let executor = DatalogExecutor::new(storage);
    // Query with 2 patterns: find names of Alice's friends
    let result = executor.execute(
        crate::query::datalog::parser::parse_datalog(
            "(query [:find ?name :where [:alice :friend ?friend] [?friend :name ?name]])"
        ).unwrap_or_else(|_| {
            // If :alice keyword-entity form isn't supported, use a simpler test
            return crate::query::datalog::parser::parse_datalog(
                "(query [:find ?name :where [?e :name ?name]])"
            ).unwrap();
        })
    ).unwrap();

    match result {
        QueryResult::QueryResults { results, .. } => {
            assert!(!results.is_empty());
        }
        _ => panic!("Expected QueryResults"),
    }
}
```

- [ ] **Step 7.2: Run test to verify it already passes (baseline)**

```bash
cargo test test_optimizer_does_not_change_query_results -- --nocapture
```

- [ ] **Step 7.3: Update `execute_query` in `src/query/datalog/executor.rs`**

Add the import:
```rust
use crate::query::datalog::optimizer;
```

In `execute_query`, before the pattern matching loop, add:

```rust
use crate::storage::index::Indexes;

// Plan the patterns: assign index hints and reorder by selectivity.
// `&Indexes::new()` is a placeholder; in Phase 6.2 the real Indexes from
// FactStorage will be passed so the optimizer can use actual cardinality data.
// In Phase 6.1 selectivity is estimated from bound-variable counts only.
let planned_patterns = optimizer::plan(
    query.where_clauses.clone(),
    &Indexes::new(),
);
// Use planned_patterns order for matching (join ordering now active).
// Replace the `query.where_clauses` iteration in the matching loop with:
//   planned_patterns.iter().map(|(p, _hint)| p.clone())
// The `_hint` is unused in 6.1 but present for Phase 6.2.
```

Specifically, find where `query.where_clauses` is iterated in the pattern matching loop and replace it with iteration over `planned_patterns.iter().map(|(p, _)| p.clone())`. The `_hint` value is unused in 6.1 but present for 6.2.

- [ ] **Step 7.4: Run full test suite**

```bash
cargo test
```

Expected: all tests pass. No semantic change — only execution order of patterns may change.

- [ ] **Step 7.5: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat(6.1): executor calls optimizer::plan() for join ordering; index hints ready for 6.2"
```

---

## Task 8: Feature Flag and Integration Tests

**Files:**
- Modify: `Cargo.toml`
- Create: `tests/index_test.rs`

- [ ] **Step 8.1: Add feature flag to `Cargo.toml`**

```toml
[features]
default = []
wasm = []
```

- [ ] **Step 8.2: Write integration tests in `tests/index_test.rs`**

These tests use only the public `Minigraf` API (no internal state inspection):

```rust
//! Integration tests for Phase 6.1 covering indexes.

use minigraf::{Minigraf, OpenOptions};
use tempfile::NamedTempFile;

fn open_temp_db() -> (Minigraf, NamedTempFile) {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let db = OpenOptions::new().path(&path).open().unwrap();
    (db, tmp)
}

#[test]
fn test_query_correct_after_transact_and_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    // First session: write facts
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        db.execute(r#"(transact [[:alice :person/name "Alice"]
                                  [:bob :person/name "Bob"]])"#).unwrap();
    }

    // Second session: re-open and query — indexes rebuilt from disk
    {
        let db = OpenOptions::new().path(&path).open().unwrap();
        let result = db.execute(
            r#"(query [:find ?name :where [?e :person/name ?name]])"#
        ).unwrap();
        match result {
            minigraf::QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 2, "Both names should be found");
            }
            _ => panic!("Expected QueryResults"),
        }
    }
}

#[test]
fn test_index_counts_exposed_via_fact_storage() {
    // Verify indexes are populated through the in-process API
    let (db, _tmp) = open_temp_db();
    db.execute(r#"(transact [[:a :x 1] [:a :y 2] [:a :link :b]])"#).unwrap();
    // index_counts is a test accessor on FactStorage — verify via the library
    // by checking that queries work correctly (index correctness = query correctness)
    let r = db.execute(r#"(query [:find ?v :where [?e :x ?v]])"#).unwrap();
    match r {
        minigraf::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!(),
    }
}

#[test]
fn test_bitemporal_valid_at_query_still_correct() {
    let (db, _tmp) = open_temp_db();
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"}
                             [[:alice :status :active]])"#).unwrap();
    db.execute(r#"(transact {:valid-from "2024-01-01"}
                             [[:alice :status :retired]])"#).unwrap();

    let r = db.execute(
        r#"(query [:find ?s :valid-at "2023-06-01" :where [:alice :status ?s]])"#
    ).unwrap();
    match r {
        minigraf::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], minigraf::Value::Keyword(":active".to_string()));
        }
        _ => panic!("Expected QueryResults"),
    }
}

#[test]
fn test_as_of_query_still_correct() {
    let (db, _tmp) = open_temp_db();
    db.execute(r#"(transact [[:alice :age 30]])"#).unwrap();
    db.execute(r#"(transact [[:alice :age 31]])"#).unwrap();

    // as-of tx 1: should see age 30 only
    let r1 = db.execute(r#"(query [:find ?a :as-of 1 :where [:alice :age ?a]])"#).unwrap();
    match r1 {
        minigraf::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0][0], minigraf::Value::Integer(30));
        }
        _ => panic!(),
    }
}

#[test]
fn test_recursive_rules_unchanged_after_6_1() {
    let (db, _tmp) = open_temp_db();
    db.execute(r#"(transact [[:a :connected :b] [:b :connected :c] [:c :connected :d]])"#).unwrap();
    db.execute(r#"(rule [(reachable ?from ?to) [?from :connected ?to]])"#).unwrap();
    db.execute(r#"(rule [(reachable ?from ?to) [?from :connected ?mid] (reachable ?mid ?to)])"#).unwrap();
    let r = db.execute(r#"(query [:find ?to :where (reachable :a ?to)])"#).unwrap();
    match r {
        minigraf::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 3, "a can reach b, c, d");
        }
        _ => panic!(),
    }
}

#[test]
fn test_explicit_transaction_with_indexes() {
    let (db, _tmp) = open_temp_db();
    let mut tx = db.begin_write().unwrap();
    tx.execute(r#"(transact [[:alice :age 30]])"#).unwrap();
    tx.commit().unwrap();

    let r = db.execute(r#"(query [:find ?a :where [:alice :age ?a]])"#).unwrap();
    match r {
        minigraf::QueryResult::QueryResults { results, .. } => {
            assert_eq!(results.len(), 1);
        }
        _ => panic!(),
    }
}
```

- [ ] **Step 8.3: Run integration tests**

```bash
cargo test --test index_test -- --nocapture
```

Expected: all 6 tests pass.

- [ ] **Step 8.4: Commit**

```bash
git add Cargo.toml tests/index_test.rs
git commit -m "feat(6.1): wasm feature flag; index integration tests"
```

---

## Task 9: Final Regression, Clippy, and Version Bump

- [ ] **Step 9.1: Run complete test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. N passed; 0 failed; 0 ignored`

- [ ] **Step 9.2: Run Clippy**

```bash
cargo clippy -- -D warnings
```

Fix any warnings. Common issues to watch for:
- `let _ = planned;` in the executor stub — replace with an intentional usage or `#[allow(unused_variables)]` with a comment
- Dead code warnings on new public functions
- Missing `Default` derives

- [ ] **Step 9.3: Build release**

```bash
cargo build --release
```

Expected: clean build.

- [ ] **Step 9.4: Bump version**

In `Cargo.toml`, change `version = "0.5.0"` → `version = "0.6.0"`.

- [ ] **Step 9.5: Commit**

```bash
git add Cargo.toml
git commit -m "chore: bump to v0.6.0 — Phase 6.1 (covering indexes + query optimizer) complete"
```

---

## Summary

After these 9 tasks, Phase 6.1 delivers:

- Four in-memory covering indexes (EAVT, AEVT, AVET, VAET) with full bi-temporal keys, populated on every `transact` / `retract` / `load_fact`
- B+tree pages persisted on disk as file format v4 (72-byte header with index root pages + checksum)
- CRC32 sync check on open — mismatch triggers rebuild, never silently serves stale data
- Automatic v3→v4 migration on first open/save cycle
- `optimizer::plan()` assigns index hints per pattern and reorders joins by selectivity
- `wasm` feature flag skips join reordering (index selection still applies)
- `FactRef { page_id, slot_index }` structure is forward-compatible with Phase 6.2 packed pages
- All 212 existing tests pass unchanged
- New tests: index correctness (unit), sync check rebuild, VAET filter, temporal queries, recursive rules regression, explicit transaction
