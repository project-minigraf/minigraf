# Incremental checkpoint index rebuild (#315) — Design

**Issue:** #315 — `checkpoint()` is O(graph size) and flat in dirty bytes
**Milestone:** v2.3.0 (`main`, file format v7 — no format change)
**Follow-up:** true O(delta) checkpoints move to #374 on the `v3` branch (see §7)

## 1. Problem

Every `checkpoint()` streams all four covering indexes (EAVT/AEVT/AVET/VAET) off
disk, decodes every entry, merges in the pending entries, re-serialises everything
and bulk-builds four new B+trees. Cost scales with total graph size, not with the
number of dirty facts. Downstream (temporal_reasoning#241) this was ~half of
at-scale ingestion wall clock. The default `wal_checkpoint_threshold` (1000) makes
every default user pay `N_checkpoints × graph_size`.

Reproduced locally (50k facts, 13 MB, checkpoint after **one** new fact, mean of 20):
**~80 ms**.

## 2. Where the time goes

Phase timing of `PersistentFactStorage::save` (tmpfs, same harness):

| phase | ms |
|---|---:|
| merge + serialise + `build_btree` (Step D) | ~58 |
| `stream_all_entries` × 4 (Step A) | ~19 |
| full-file CRC re-read | ~3.5 |
| pack facts + fsyncs | <1 (a few ms on NVMe) |

The cost is CPU in decode/re-encode of every index entry, not I/O.

## 3. Why true O(delta) is out of scope here

In format v7, fact pages must be contiguous at pages `1..=fact_page_count`
(`read_all_with_refs`, `CommittedFactLoaderImpl::first_fact_page`), and indexes live
directly after them. New fact pages therefore always overwrite the start of the old
index region, so every checkpoint must relocate the index. Avoiding that needs
copy-on-write B+trees plus free-space management — a format change, and the same
change #374 (save() is not crash-atomic) requires. That work belongs on `v3`.

Dropping the unconditional `force_dirty()` (issue option 2) does not help: with one
dirty fact `dirty` is already true, and the zero-dirty case is already short-circuited
by the `wal_entry_count == 0 && !pfs.is_dirty()` guard.

## 4. Design

### 4.1 Leaf reuse — `src/storage/btree_v6.rs`

`btree_v6.rs` is the B+tree for both v6 and v7 (v7 changed only the header). The
node layout is unchanged by this design, so v2.x readers are unaffected.

New function:

```rust
pub fn rebuild_btree_incremental<K>(
    old_leaves: Vec<Arc<Vec<u8>>>,   // raw leaf pages of the old tree, in next_leaf order
    pending: Vec<(K, FactRef)>,      // sorted
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    start_page_id: u64,
) -> Result<(u64, u64)>              // (root_page_id, next_free_page_id)
where K: Serialize + for<'de> Deserialize<'de> + Ord
```

Algorithm:

1. For each old leaf decode **only the first entry's key** → `first_key[i]`.
2. Route each pending entry to leaf `i` where `first_key[i] ≤ key < first_key[i+1]`;
   keys below `first_key[0]` go to leaf 0.
3. Leaf with no pending entries → **raw copy**: write the 4 KB page verbatim to the
   next page id, patching only bytes 4..12 (`next_leaf`). No decode/encode.
4. Leaf with pending entries → decode all its entries, merge with its pending
   entries, serialise, and repack using `build_btree`'s 75% fill rule (may emit
   more than one leaf).
5. Leaves are written in order with `next_leaf = pid + 1` (0 for the last), so there
   is no second patch pass.
6. Collect `(page_id, first_key_bytes)` per new leaf (postcard of the first key) and
   build internal levels bottom-up via a helper **extracted from `build_btree`'s
   phase 2** (`build_internal_levels`), shared by both paths.

`build_btree` remains the path for the first save, for an empty or missing old tree
(root 0 or a single empty leaf), and for load-time index rebuild / migrations.

Collecting raw leaves: `collect_leaf_pages(root, backend, cache) -> Vec<Arc<Vec<u8>>>`
walks `find_leftmost_leaf` + the `next_leaf` chain, validating page type as
`stream_all_entries` does.

Leaf fill: untouched leaves keep their existing fill; touched leaves are repacked at
75%. The tree shape differs from a fresh bulk build but is a valid B+tree for
`range_scan` / `stream_all_entries` / `find_leaf_for_key`. No merging of
under-filled leaves (YAGNI).

### 4.2 `save()` ordering — `src/storage/persistent_facts.rs`

Step A reads the raw leaf pages of all four old trees into memory **before** any
write (new fact pages overwrite the start of the old index region). This replaces
the decoded `Vec<(K, FactRef)>` streams; memory drops (raw pages ≤ decoded entries
+ serialised copies held today).

Step D calls `rebuild_btree_incremental` per index when the old root is non-zero,
otherwise `build_btree` as today.

Crash semantics are unchanged: the WAL is only deleted after the header write, and a
checksum mismatch on open rebuilds indexes from fact pages (#370 path). #374 remains
the real fix for atomicity.

### 4.3 Checksum prefix cache

`PersistentFactStorage` gains `fact_prefix_crc: Option<crc32fast::Hasher>` — CRC32
state after hashing fact pages `1..=committed_fact_pages`.

- **Set** in `load()`: the existing full-checksum pass is split into fact pages then
  index pages; the hasher is cloned between the two segments. Only set when the
  new-style full checksum matched.
- **Set** at the end of `save()`: prefix + the new fact pages (hashed from the
  in-memory page buffers).
- **Cleared (None)** on any other path that rewrites the header/indexes (index
  rebuild in `load()`, legacy/v5 migrations).
- **Use** in `save()`: clone the prefix, update with the new fact pages from memory,
  then read back and hash only the index pages. `None` → today's full pass.

The stored `index_checksum` value is identical to today's (CRC over pages
`1..page_count`), so older v2.x readers verify it unchanged.

### 4.4 Docs and cleanup

- `Minigraf::checkpoint` rustdoc: cost is O(graph size) page copies plus O(delta)
  decode; checkpoint is **not** a durability boundary (the WAL is crash-durable);
  batching trades only reopen latency (WAL replay).
- Same note on `OpenOptions::wal_checkpoint_threshold`.
- Trim the obsolete "same-process double-opens" rationale in `do_checkpoint`'s
  guard comment (now refused outright since #304/#314).
- At release: `CHANGELOG.md`, `ROADMAP.md`, `docs/BENCHMARKS.md`,
  `docs/TEST_COVERAGE.md`, `CLAUDE.md` test count.

## 5. Testing

No new dependencies; deterministic seeded loops.

- **Equivalence (unit, `btree_v6.rs`):** for random committed sets and random
  pending batches (including batches that force leaf splits, keys below the first
  leaf, keys after the last leaf, and many pending entries in one leaf),
  `stream_all_entries` of the incremental tree equals the merged sorted set.
- **Structure:** `next_leaf` chain is strictly ascending across leaves; `range_scan`
  over random bounds matches a filter over the expected set.
- **Multi-checkpoint (`persistent_facts.rs`):** many save cycles on a file backend;
  after each, all four indexes stream equal to a from-scratch build over all facts.
- **Checksum:** after incremental saves `header.index_checksum ==
  compute_page_checksum(1, page_count-1)`; reopening does not take the rebuild path.
  Prefix cache is `None` after a rebuild/migration path and a subsequent save still
  writes a correct checksum.
- **Regression:** full existing suite.
- **Benchmark:** new `checkpoint/after_1_fact` group (10k / 100k facts) in
  `benches/minigraf_bench.rs`; before/after numbers in `docs/BENCHMARKS.md`.

Test assert messages follow the CodeQL rule (no `{:?}` of UUID-bearing types).

## 6. Success criteria

- 50k facts, checkpoint after 1 fact: ~80 ms → **≤ 15 ms** on the §1 harness.
- No file-format or public-API change; files written by the new code open without
  index rebuild on v2.0.1.
- All existing tests pass.

## 7. Follow-up

Comment on #374 (v3.0.0): copy-on-write B+tree pages with a free list make save()
crash-atomic **and** give O(delta · log n) checkpoints; #315's remaining O(size)
page-copy term is owned there. #315 is closed by this work as mitigated, not fully
resolved (no closing keyword on #374).
