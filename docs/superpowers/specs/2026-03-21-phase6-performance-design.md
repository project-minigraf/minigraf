# Phase 6: Performance & Indexes — Design Spec

**Date:** 2026-03-21
**Status:** Approved
**Phase:** 6 (follows Phase 5 ACID + WAL, complete at v0.5.0)

---

## Overview

Phase 6 makes Minigraf scale to arbitrary database sizes with bounded memory, following the same architectural model as SQLite: page-granular I/O, a configurable page cache, and B+tree indexes for fast lookups. It also adds a query optimizer that selects indexes and reorders join patterns.

Phase 6 is implemented as three sequential sub-phases, each independently shippable and testable:

| Sub-phase | Deliverable | File format |
|---|---|---|
| 6.1 — Indexes | EAVT/AEVT/AVET/VAET covering indexes + query optimizer | v3 → v4 |
| 6.2 — Page I/O | Packed pages + LRU page cache + on-demand loading | v4 → v5 |
| 6.3 — Benchmarks | Criterion benchmark suite + rkyv evaluation | no change |

After 6.1, queries are fast (index-driven, no full scans). After 6.2, memory usage is bounded regardless of database size. After 6.3, performance is validated and documented.

Both file format migrations follow the established pattern: auto-detect version on open, migrate forward, persist new format before handing the DB to the caller.

---

## Sub-phase 6.1: Covering Indexes + Query Optimizer

### Index Design

Four Datomic-style covering indexes. All four are **bi-temporally complete** — `valid_from` and `valid_to` are included in every key tuple so that `:valid-at` queries benefit from index range scans on `valid_from` rather than requiring a full fact-set scan.

| Index | Sort key | Primary use |
|---|---|---|
| EAVT | `(Entity, Attribute, ValidFrom, ValidTo, TxCount)` | Lookup by entity or entity+attribute |
| AEVT | `(Attribute, Entity, ValidFrom, ValidTo, TxCount)` | Lookup by attribute or attribute+entity |
| AVET | `(Attribute, Value, ValidFrom, ValidTo, Entity, TxCount)` | Equality/range lookup by attribute+value |
| VAET | `(RefTarget, Attribute, ValidFrom, ValidTo, Entity, TxCount)` | Reverse reference traversal (Ref values only) |

VAET only indexes facts where `Value` is `Value::Ref(EntityId)`.

`Value` requires a canonical byte encoding that preserves sort order across variants (type discriminant byte prepended) to enable correct `BTreeMap` ordering in AVET and VAET keys.

**Temporal range scan behaviour:** a `:valid-at ts` query uses `valid_from <= ts` as the upper range scan bound on the `ValidFrom` component of the index key, narrowing the candidate set efficiently. `valid_to > ts` is then applied as a residual filter on the narrowed entries — it cannot itself be a range scan prefix because `ValidTo` appears after `ValidFrom` in the key. This is still far better than a full fact-set scan; the residual filter operates on a small candidate set rather than all facts.

### In-Memory Structure

Each index is a `std::collections::BTreeMap<IndexKey, FactRef>` where `FactRef` is defined as:

```rust
struct FactRef {
    page_id:    u64,
    slot_index: u16,  // record slot within the page (always 0 in 6.1; used in 6.2)
}
```

Using `FactRef` (not a bare `PageId`) from the outset means no index restructuring is needed when sub-phase 6.2 packs multiple facts per page. In 6.1, `slot_index` is always `0` (one fact per page). In 6.2, `slot_index` identifies the fact's slot in a packed page.

### On-Disk Structure: B+tree Pages

On-disk indexes are stored as **B+tree page structures**. All pages share a unified `page_type` byte at offset 0:

```
Page type values (unified namespace, no overlaps):
  0x01 = fact data page (one-per-page, v4 format)
  0x02 = packed fact data page (v5 format)
  0x03 = overflow chain page (v5 format)
  0x10 = B+tree internal node
  0x11 = B+tree leaf node
```

**Internal node page:**
```
[page_type: u8 = 0x10]
[key_count: u16]
[keys:     key_count × IndexKey bytes]
[children: (key_count + 1) × u64 page_ids]
```

**Leaf node page:**
```
[page_type: u8 = 0x11]
[entry_count: u16]
[next_leaf: u64]   (0 = no next leaf)
[entries: entry_count × (IndexKey bytes, FactRef)]
```

Leaf nodes are linked via `next_leaf` pointers, enabling sequential range scans without re-traversing the tree.

**Node capacity:** EAVT and VAET index keys contain two UUIDs (16 bytes each) plus two i64 and one u64 — approximately 56 bytes per key. AVET keys are similar. With a 4KB page and the header overhead:
- **Leaf node:** `(56 + 10) × N ≤ 4096 - 11` → approximately 61 entries per leaf (10 bytes for `FactRef`: 8 for `page_id` + 2 for `slot_index`; 11-byte leaf node header: `page_type` u8 + `entry_count` u16 + `next_leaf` u64 = 1+2+8; note this is distinct from the 12-byte packed fact data page header)
- **Internal node:** `56k + 8(k+1) ≤ 4096 - 3` → approximately 62 keys and 63 child pointers per internal node

Individual keys are always smaller than one page so there is no key-level overflow.

### File Format v4

The v4 header grows beyond the current 64 bytes to accommodate five new fields (removing `index_section_start` as redundant — it can be derived as `min(eavt_root_page, aevt_root_page, avet_root_page, vaet_root_page)`). The new header layout:

```
Offset  Size  Field
0       4     magic ("MGRF")
4       4     version (u32 = 4)
8       8     page_count (u64)
16      8     node_count (u64, reused for fact count)
24      8     last_checkpointed_tx_count (u64)
32      8     eavt_root_page (u64)
40      8     aevt_root_page (u64)
48      8     avet_root_page (u64)
56      8     vaet_root_page (u64)
64      4     index_checksum (u32)
68      4     reserved / padding
Total: 72 bytes
```

`FileHeader::to_bytes()` and `FileHeader::from_bytes()` are updated to handle this 72-byte layout. `from_bytes` requires `bytes.len() >= 72` for v4 files (v3 files still require only 64 bytes, handled in migration).

File section layout:
```
Page 0:         Header (72 bytes in a 4KB page)
Pages 1..N:     Fact data pages (one fact per page, unchanged from v3)
Pages N+1..M:   Index B+tree pages (EAVT, AEVT, AVET, VAET trees)
```

The four `*_root_page` fields in the header point to the root of each B+tree; the full index section spans all pages reachable from those roots.

### Sync Check on Load

Performed before the database is made available to the caller. The checksum covers the **logical fact set** (a CRC32 over all fact byte representations after deserialisation and WAL replay), not raw page bytes. This ensures that after WAL replay the in-memory fact set is consistent and the checksum comparison works correctly — WAL replay advances the in-memory state ahead of the data pages, so checking raw pages would always mismatch after a crash.

1. Replay WAL into `FactStorage` (existing Phase 5 logic, unchanged)
2. Compute CRC32 over all in-memory facts (serialised with postcard, sorted by `(tx_count, entity_uuid_bytes, attribute_string)` — a total order that is stable across save/load/replay cycles, including when multiple facts share the same `tx_count`)
3. Read `index_checksum` from the file header
4. **Mismatch:** discard the index section, rebuild all four indexes from in-memory facts, re-persist (write new B+tree pages + updated header checksum), then hand DB to caller
5. **Match:** deserialise indexes from B+tree pages directly (fast path)

**Post-crash recovery:** after WAL replay the in-memory fact set is ahead of the on-disk data pages, so the logical-fact checksum will mismatch the stored checksum and a full index rebuild is triggered. This is correct and safe, though it means the fast path is not taken after a crash. This is an accepted trade-off: correctness over startup speed in the recovery case.

### Write Maintenance

When `transact()` or `retract()` adds facts:
- All four in-memory `BTreeMap` indexes are updated immediately
- Indexes marked dirty
- On `save()` / `checkpoint()`: dirty indexes serialised to B+tree pages, `index_checksum` recomputed over the full in-memory fact set and written to header

### Query Optimizer

New module: `src/query/datalog/optimizer.rs`

**Index selection** — for each pattern, pick the most efficient index based on bound fields:

| Bound fields | Index chosen |
|---|---|
| Entity | EAVT |
| Entity + Attribute | EAVT |
| Attribute only | AEVT |
| Attribute + Entity | AEVT |
| Attribute + Value::Ref | AVET (more selective than VAET) |
| Attribute + Value (non-Ref) | AVET |
| Value::Ref only (no attribute) | VAET (reverse traversal) |
| Nothing bound | EAVT (full scan, least bad) |

Temporal constraints (`:as-of`, `:valid-at`) are incorporated into the index scan: `:as-of` folds into a `TxCount` upper bound; `:valid-at ts` folds into a `valid_from <= ts` range scan bound with `valid_to > ts` applied as a residual filter on matched entries.

**Join ordering** — after index selection, patterns are sorted by estimated selectivity before execution. Selectivity is estimated by bound variable count: patterns with more bound fields are more selective and run first, shrinking the candidate set for subsequent patterns. No statistics collection required.

Example: a 3-pattern query `[?e :name "Alice"] [?e :age ?a] [?e :friend ?f]` — the first pattern (attribute + value both bound → AVET) runs first, binding `?e`; the remaining two patterns then have entity bound and use EAVT.

**WASM feature flag:**
```toml
[features]
default = []
wasm = []
```
Under `#[cfg(feature = "wasm")]`, join ordering is skipped — patterns execute in the order written by the user. Index selection still applies. This keeps the WASM optimizer lightweight.

Interface:
```rust
// optimizer.rs
pub fn plan(patterns: Vec<Pattern>, indexes: &Indexes) -> Vec<(Pattern, IndexHint)>;
```

The executor calls `optimizer::plan()` to get back an ordered `(Pattern, IndexHint)` list before pattern matching begins.

---

## Sub-phase 6.2: Page-Granular I/O + LRU Page Cache

### Problem with Current Format

One fact per 4KB page: a ~150-byte fact wastes ~97% of its page. A 100K-fact database occupies ~400MB on disk. The entire database is loaded into memory on open. Both problems are fixed by packed pages and on-demand loading.

### Packed Page Layout

All pages share the unified `page_type` namespace defined in 6.1. Packed fact data pages use `page_type = 0x02`:

```
[12-byte page header]
  page_type:    u8   (0x02 = packed fact data, 0x03 = overflow)
  _reserved:    u8   (padding for alignment)
  record_count: u16  (facts packed into this page)
  next_page:    u64  (overflow chain; 0 = none)

[record directory: record_count × 4 bytes each]
  offset: u16  (from page start to record bytes)
  length: u16  (byte length of this serialized record)

[record data: variable-length packed fact serializations]
```

With ~150 bytes per fact and 4KB pages, ~25 facts pack per page — a ~25× reduction in file size and I/O amplification.

`FactRef.slot_index` (introduced in 6.1) now carries the record slot number within the packed page. Index lookups resolve to `(page_id, slot_index)` pairs, giving O(1) fact access within a page via the record directory.

### LRU Page Cache

New module: `src/storage/cache.rs`

- Configurable capacity (default 256 pages = 1MB for 4KB pages); exposed via `OpenOptions::page_cache_size(usize)`
- Uses **interior mutability** (`RwLock` internally) so the public interface takes `&self` — preserving the Phase 5 concurrent-reader guarantee (multiple readers can access the cache simultaneously without serialising through a `&mut self` lock)
- Interface:
  ```rust
  fn get_page(&self, page_id: u64) -> Result<Arc<[u8; PAGE_SIZE]>>;
  fn mark_dirty(&self, page_id: u64);
  fn flush(&self) -> Result<()>;
  ```
- LRU eviction: dirty pages are written back to `StorageBackend` before eviction

### FactStorage API Migration

In 6.2, `FactStorage`'s internal `Vec<Fact>` is replaced by a `PageCache` reference. The existing public API is migrated as follows:

| Method | Fate |
|---|---|
| `transact()`, `retract()`, `load_fact()`, `restore_tx_counter()` | Unchanged — write path unaffected |
| `get_facts_by_entity()`, `get_facts_by_attribute()`, `get_facts_by_entity_attribute()` | Replaced by index-driven lookups via EAVT/AEVT |
| `get_facts_as_of()`, `get_facts_valid_at()` | Replaced by index range scans with temporal bounds |
| `get_current_value()` | Replaced by EAVT lookup |
| `get_all_facts()` | Retained as a full-scan method, documented as "avoid in hot paths — use index-driven queries instead"; used internally for sync check and migration only |
| `get_asserted_facts()` | Deprecated; callers use index-driven queries with implicit assertion filter |

The `get_all_facts()` method materialises the full fact set and is explicitly marked as a non-hot-path operation in rustdoc.

### On-Demand Loading

Current flow: load all → clone → filter → match
New flow: index lookup → resolve `FactRef(page_id, slot_index)` → cache fetch → read record at slot → deserialise → match

The executor uses `optimizer::plan()` to get `(Pattern, IndexHint)` pairs, resolves `FactRef` values via the in-memory B+tree index, and requests only those pages from the page cache. Because `FactRef` was designed this way in 6.1, no index restructuring is needed.

### File Format v5

Same section layout as v4. Header gains one new field, replacing the 4-byte reserved block at offsets 68–71:

```
Offset  Size  Field
...     ...   (all v4 fields unchanged)
68      1     fact_page_format (u8): 0x01 = one-per-page (v4), 0x02 = packed (v5)
69      3     reserved / padding
Total: 72 bytes (same size as v4 header)
```

**Migration v4 → v5:**
1. Read all facts from v4 pages (one per page, `page_type = 0x01`) into a `Vec<Fact>`
2. Repack facts into v5 pages (`page_type = 0x02`, multiple per page), recording each fact's `(page_id, slot_index)` as facts are written
3. Rebuild all four in-memory indexes from scratch by re-inserting every fact with its new `FactRef { page_id, slot_index }` into fresh `BTreeMap`s — in-place mutation of existing index entries is not used, as a full rebuild is simpler and avoids partial-update bugs
4. Re-persist all four indexes (B+tree pages, same structure as 6.1)
5. Write v5 header with `fact_page_format = 0x02`

---

## Sub-phase 6.3: Benchmarks

Benchmark suite in `benches/` using [Criterion](https://github.com/bheisler/criterion.rs).

### Dataset Scales

| Scale | Facts | Represents |
|---|---|---|
| Small | 10K | Personal knowledge base |
| Medium | 100K | Small business app |
| Large | 1M | Production single-machine |

### Benchmark Scenarios

| Benchmark | What it measures |
|---|---|
| `insert_throughput` | Facts/sec for batch transact at each scale |
| `indexed_lookup` | Single-fact point lookup by entity+attribute (EAVT) |
| `pattern_match_single` | 1-pattern query with index at each scale |
| `pattern_match_multi` | 3-pattern join query with join ordering |
| `transitive_closure` | Recursive reachability on a 1K-node graph |
| `valid_at_query` | `:valid-at` temporal range query at each scale |
| `as_of_query` | `:as-of` transaction-time query at each scale |
| `checkpoint` | WAL flush + page write throughput |

### Performance Targets

| Scenario | Target |
|---|---|
| Indexed point lookup | < 1ms |
| Pattern match with index (100K facts) | < 10ms |
| Transitive closure (typical graph) | < 100ms |

### rkyv Evaluation

After benchmarks run against postcard: if serialization/deserialization appears in the top 3 hotspots (via `cargo flamegraph` or `perf`), implement the same benchmarks with rkyv and compare. Switch only if rkyv shows >20% improvement on the bottleneck scenario and the added complexity is justified.

---

## Testing Strategy

### 6.1 — Index Tests (unit)
- Index entries created correctly on `transact()` and `retract()`
- All four indexes return correct `FactRef` values for known facts
- VAET only indexes `Value::Ref` facts
- `valid_from` and `valid_to` correctly included in index keys
- Sync check detects mismatch (logical fact checksum) and rebuilds correctly
- Index survives save/load round-trip (persist + reload = identical BTreeMap)
- On-disk B+tree structure correct (leaf links, internal routing, range scan traversal)
- `slot_index` is always 0 in 6.1 for all index entries

### 6.1 — Optimizer Tests (unit)
- Correct index selected for each pattern shape (including Attribute+Ref→AVET, Ref-only→VAET)
- Join ordering produces most-selective-first ordering
- Under `wasm` feature flag, patterns are not reordered
- `:valid-at` temporal constraint becomes `valid_from <= ts` range bound + `valid_to > ts` residual filter

### 6.2 — Page Cache Tests (unit)
- Cache hit returns same bytes as direct backend read
- Concurrent readers can call `get_page(&self)` simultaneously without serialising
- LRU eviction triggers write-back of dirty pages before eviction
- Configurable capacity is respected
- Cache miss fetches from backend and caches result

### 6.2 — Packed Page Tests (unit)
- Multiple facts pack into a single page correctly
- `slot_index` in `FactRef` correctly identifies the record in the packed page
- Overflow chain serialises and deserialises correctly
- v4 → v5 migration preserves all facts, all indexes, and all `FactRef` slot values
- Round-trip: pack → write → read → unpack = identical facts

### Integration Tests (new `tests/performance_test.rs`)
- 1K, 10K, 100K fact datasets query correctly with indexes active
- `:valid-at` and `:as-of` queries return correct results via index path
- Recursive rules produce correct results after 6.2 restructuring
- Concurrent readers + writer remain safe with page cache in play

### Regression
All 212 existing query, storage, WAL, and concurrency tests must pass unchanged throughout 6.1 and 6.2 — no query semantic or API behaviour changes. Low-level `FileHeader` serialisation unit tests will be updated to match the new v4/v5 72-byte layout.

---

## Key Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Scale target | SQLite model (disk-bounded, memory-bounded) | Philosophy alignment |
| Index persistence | On-disk, sync-checked on load | Faster startup for large DBs |
| Bi-temporal in indexes | Yes — valid_from/valid_to in all key tuples | Efficient temporal range scans; valid_to is a residual filter |
| In-memory index structure | `std::collections::BTreeMap` | No new dependencies, range iteration works |
| On-disk index structure | B+tree (linked leaf nodes) | Sequential range scans, cache-friendly |
| Index value type | `FactRef { page_id, slot_index }` | Works in both 6.1 (slot=0) and 6.2 (slot=N) without refactoring |
| Page type namespace | Unified, non-overlapping byte values | Avoids ambiguity between B+tree node types and page section types |
| Optimizer scope | Index selection + join ordering | 80-90% of gain, minimal complexity |
| WASM optimizer | Feature flag caps at index-selection only | Lightweight for browser environments |
| PageCache concurrency | Interior mutability (`RwLock`), `&self` interface | Preserves Phase 5 concurrent-reader guarantee |
| Serialization | Keep postcard; evaluate rkyv only if benchmarks justify | YAGNI, postcard is proven |
| File format bumps | v3→v4 (indexes, 72-byte header), v4→v5 (packed pages) | Incremental, each migration independently testable |
| Post-crash index rebuild | Always triggered (logical checksum will mismatch) | Correct; fast path applies only to clean shutdown |
