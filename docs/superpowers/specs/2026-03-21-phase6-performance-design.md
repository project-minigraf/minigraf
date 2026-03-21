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

Four Datomic-style covering indexes. All four are **bi-temporally complete** — `valid_from` and `valid_to` are included in every key tuple so that `:valid-at` queries are answered via index range scans, not post-scan filters.

| Index | Sort key | Primary use |
|---|---|---|
| EAVT | `(Entity, Attribute, ValidFrom, ValidTo, TxCount)` | Lookup by entity or entity+attribute |
| AEVT | `(Attribute, Entity, ValidFrom, ValidTo, TxCount)` | Lookup by attribute or attribute+entity |
| AVET | `(Attribute, Value, ValidFrom, ValidTo, Entity, TxCount)` | Equality/range lookup by attribute+value |
| VAET | `(RefTarget, Attribute, ValidFrom, ValidTo, Entity, TxCount)` | Reverse reference traversal (Ref values only) |

VAET only indexes facts where `Value` is `Value::Ref(EntityId)`.

`Value` requires a canonical byte encoding that preserves sort order across variants (type discriminant byte prepended) to enable correct `BTreeMap` ordering in AVET and VAET keys.

### In-Memory Structure

Each index is a `std::collections::BTreeMap<IndexKey, PageId>`. `PageId` references the fact's storage page, not its position in a `Vec` — this keeps the structure valid and unchanged when sub-phase 6.2 switches from load-all to on-demand page loading.

### On-Disk Structure: B+tree Pages

On-disk indexes are stored as **B+tree page structures**, not a flat serialisation of the in-memory `BTreeMap`. This is designed this way from 6.1 so 6.2's page cache can exploit sequential leaf traversal immediately without reformatting.

```
Internal node page:
  [page_type: u8 = 0x10]
  [key_count: u16]
  [keys: key_count × IndexKey]
  [children: (key_count + 1) × PageId]

Leaf node page:
  [page_type: u8 = 0x11]
  [entry_count: u16]
  [next_leaf: PageId]  (0 = no next leaf)
  [entries: entry_count × (IndexKey, PageId)]
```

Leaf nodes are linked via `next_leaf` pointers, enabling sequential range scans without traversing the tree again.

### File Format v4

New fields added to `FileHeader`:

```rust
index_section_start: u64,  // first page ID of the index section
eavt_root_page:      u64,  // root page of EAVT B+tree
aevt_root_page:      u64,  // root page of AEVT B+tree
avet_root_page:      u64,  // root page of AVET B+tree
vaet_root_page:      u64,  // root page of VAET B+tree
index_checksum:      u32,  // CRC32 over all fact data pages
```

Index pages are stored as a contiguous section after all fact data pages:
```
Page 0:         Header
Pages 1..N:     Fact data pages (one fact per page, unchanged from v3)
Pages N+1..M:   Index B+tree pages (EAVT, AEVT, AVET, VAET sections)
```

### Sync Check on Load

Performed before the database is made available to the caller:

1. Read `index_checksum` from the file header
2. Compute CRC32 over all current fact data pages
3. **Mismatch:** discard the index section, rebuild all four indexes from facts in memory, re-persist (write new index pages + updated header checksum), then proceed
4. **Match:** deserialize indexes from B+tree pages directly (fast path)

This is conservative (any fact page change invalidates all indexes) but correct and simple.

### Write Maintenance

When `transact()` or `retract()` adds facts:
- All four in-memory `BTreeMap` indexes are updated immediately
- Indexes marked dirty
- On `save()` / `checkpoint()`: dirty indexes serialised to B+tree pages, `index_checksum` recomputed and written to header

### Query Optimizer

New module: `src/query/datalog/optimizer.rs`

**Index selection** — for each pattern, pick the most efficient index based on bound fields:

| Bound fields | Index chosen |
|---|---|
| Entity | EAVT |
| Entity + Attribute | EAVT |
| Attribute only | AEVT |
| Attribute + Entity | AEVT |
| Attribute + Value | AVET |
| Value is Ref (reverse traversal) | VAET |
| Nothing bound | EAVT (full scan, least bad) |

Temporal constraints (`:as-of`, `:valid-at`) are folded into the index scan as range bounds on `TxCount`, `ValidFrom`, and `ValidTo` — eliminating post-scan temporal filtering in the common case.

**Join ordering** — patterns are sorted by estimated selectivity (most bound variables = most selective = executes first). This shrinks the candidate set for each subsequent pattern. No statistics collection required — bound-variable counting is a zero-overhead approximation that delivers most of the benefit.

**WASM feature flag:**

```toml
[features]
default = []
wasm = []
```

Under `#[cfg(feature = "wasm")]`, join ordering is skipped — patterns execute in user-written order. Index selection still applies. This keeps the WASM optimizer lightweight.

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

```
[8-byte page header]
  page_type:    u8   (0x01 = fact data, 0x02 = index, 0x03 = overflow)
  record_count: u16  (facts packed into this page)
  next_page:    u64  (overflow chain; 0 = none)

[record directory: record_count × 4 bytes each]
  offset: u16  (from page start to record bytes)
  length: u16  (byte length of this serialized record)

[record data: variable-length packed fact serializations]
```

With ~150 bytes per fact and 4KB pages, ~25 facts pack per page — a ~25× reduction in file size and I/O amplification. Overflow chains handle facts that exceed a single page (rare).

### LRU Page Cache

New module: `src/storage/cache.rs`

- Configurable capacity (default 256 pages = 1MB for 4KB pages)
- Exposed via `OpenOptions::page_cache_size(usize)`
- Interface:
  ```rust
  fn get_page(&mut self, page_id: u64) -> Result<&[u8]>;
  fn mark_dirty(&mut self, page_id: u64);
  fn flush(&mut self) -> Result<()>;
  ```
- LRU eviction: dirty pages are written back to `StorageBackend` before eviction

### On-Demand Loading

Current flow: load all → clone → filter → match
New flow: index lookup → resolve page_ids → cache fetch → deserialize only those pages → match

The executor uses `optimizer::plan()` to get `(Pattern, IndexHint)` pairs, resolves page_ids via the in-memory B+tree index, and requests only those pages from the page cache. Facts are deserialized from fetched pages only.

The `Vec<Fact>` field in `FactStorage` is replaced by a `PageCache` reference. Because the B+tree indexes already reference facts by `PageId` (designed this way in 6.1), no index restructuring is needed in 6.2.

### File Format v5

Same section layout as v4. Header gains one new field:

```rust
fact_page_format: u8,  // 0x00 = one-per-page (v4 compat), 0x01 = packed (v5)
```

**Migration v4 → v5:**
1. Read all facts from v4 pages (one per page)
2. Repack facts into v5 pages (multiple per page)
3. Rebuild and re-persist all four indexes (B+tree pages)
4. Write v5 header with `fact_page_format = 0x01`

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
- All four indexes return correct page_ids for known facts
- VAET only indexes `Value::Ref` facts
- `valid_from` and `valid_to` correctly included in index keys
- Sync check detects mismatch and rebuilds correctly
- Index survives save/load round-trip
- On-disk B+tree structure correct (leaf links, internal routing, range scan)

### 6.1 — Optimizer Tests (unit)
- Correct index selected for each pattern shape
- Join ordering produces most-selective-first ordering
- Under `wasm` feature flag, patterns are not reordered
- Temporal bounds folded into index scan range correctly

### 6.2 — Page Cache Tests (unit)
- Cache hit returns same bytes as direct backend read
- LRU eviction triggers write-back of dirty pages before eviction
- Configurable capacity is respected
- Cache miss fetches from backend and caches result

### 6.2 — Packed Page Tests (unit)
- Multiple facts pack into a single page correctly
- Overflow chain serialises and deserialises correctly
- v4 → v5 migration preserves all facts and indexes
- Round-trip: pack → write → read → unpack = identical facts

### Integration Tests (new `tests/performance_test.rs`)
- 1K, 10K, 100K fact datasets query correctly with indexes active
- `:valid-at` and `:as-of` queries return correct results via index path
- Recursive rules produce correct results after 6.2 restructuring
- Concurrent readers + writer remain safe with page cache in play

### Regression
All 212 existing tests must pass unchanged throughout 6.1 and 6.2. No behaviour changes — only performance improves.

---

## Key Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Scale target | SQLite model (disk-bounded, memory-bounded) | Philosophy alignment |
| Index persistence | On-disk, sync-checked on load | Faster startup for large DBs |
| Bi-temporal in indexes | Yes — valid_from/valid_to in all key tuples | Efficient `:valid-at` without post-scan filter |
| In-memory index structure | `std::collections::BTreeMap` | No new dependencies, range iteration works |
| On-disk index structure | B+tree (linked leaf nodes) | Sequential range scans, cache-friendly |
| Optimizer scope | Index selection + join ordering | 80-90% of gain, minimal complexity |
| WASM optimizer | Feature flag caps at index-selection only | Lightweight for browser environments |
| Serialization | Keep postcard; evaluate rkyv only if benchmarks justify | YAGNI, postcard is proven |
| File format bumps | v3→v4 (indexes), v4→v5 (packed pages) | Incremental, each migration independently testable |
