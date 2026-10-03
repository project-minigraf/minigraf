# Copy-on-write B+tree checkpoints (#374, #434, #388) — Design

**Issues:** #374 (crash-atomic `save()`), #434 (checkpoint cost O(change)), #388 (per-page checksums)
**Milestone:** v3.0.0, branch `v3`, file format v8 (unreleased; this redefines its layout)
**Must land before:** #391 golden-file corpus freezes v8
**Builds on:** #315 / `2026-09-26-incremental-checkpoint-design.md` (balanced leaf split, routing)

## 1. Problem

`PersistentFactStorage::save()` has three defects that share one cause: fact pages
must stay contiguous at `1..=fact_page_count`, and the indexes sit directly after them.

1. **Not crash-atomic (#374).** New fact pages overwrite the start of the old index
   region before the header is written. A crash in between leaves the old header
   pointing at overwritten index pages. Open then sees a checksum mismatch and rebuilds
   every index from the fact pages, which is O(graph).
2. **O(graph) checkpoints (#434).** Every index page is copied on every checkpoint.
   Measured at 3.82 GB: ~3.27 GB written per checkpoint, ~13,000× write amplification.
   On top of that, the full-file `index_checksum` is O(file) on every save and every open.
3. **No per-page integrity (#388).** Only the header has a checksum. A torn write, a bad
   sector or a misdirected write is noticed only if postcard decoding happens to fail.

A fourth defect sits in the same code: the header is a single page. A torn write of
page 0 leaves a file whose header fails its CRC, and the file cannot be opened at all.

## 2. Goals and non-goals

**Goals**

- G1. A crash at any point of a checkpoint (any write, any sync, a torn page write)
  reopens to the last committed checkpoint plus WAL replay. No checkpointed fact is lost.
  No index rebuild happens on open. (#374)
- G2. A checkpoint writes only new fact pages, the copied-on-write path from each
  modified leaf to its root, the fact-directory path, free-list pages and one meta page.
  Bytes written are O(k · log N) after k new facts and never depend on N. This bound
  covers `checkpoint()`, the auto-checkpoint and the close-time checkpoint in
  `Drop for Inner`. (#434 acceptance 1, 2, 4)
- G3. Every non-meta page carries a CRC32, its own page id and the checkpoint
  generation that wrote it. Every read checks them. A failed check returns a structured
  `STG-0xx` error and never returns the page's data. (#388)
- G4. Open is O(1) in graph size: no full-file checksum pass.
- G5. v7 files and pre-release v8 files (single header, contiguous fact pages) migrate
  on open. The migration is crash-safe and writes checksums. (#388 migration clause)

**Non-goals (separate issues)**

- Shrinking the file or compacting free pages. The free list is reused. Vacuum is
  a later, separate operation (#434: "space reclamation can be a later operation").
- Fact-page density, dictionary coding and prefix compression (#433 format decisions).
  The page header in §3.2 leaves the payload encoding free to change.
- The public streaming cursor API (#432). §5 adds an internal leaf cursor that #432
  can build on.
- Lost-write detection, where the device acknowledges a write that never lands and an
  older valid page stays at that location. Parent-held checksums were considered and
  rejected (§9).

## 3. On-disk layout (v8)

All integers are little-endian. Page size stays 4096.

```
Page 0, 1   Meta pages A and B (alternating commits)
Page 2+     Any mix of: fact pages, index nodes (EAVT/AEVT/AVET/VAET),
            fact-directory nodes, free-list pages, free pages
Sidecar     <db>.wal — unchanged
```

### 3.1 Meta pages

Each checkpoint writes the inactive slot, `slot = generation % 2`. On open, both slots
are read. A slot is valid if its magic, version and `meta_crc` check out. The valid
slot with the highest `generation` wins. Neither valid → §6.

| offset | field | notes |
|---|---|---|
| 0..4 | magic `"MGRF"` | |
| 4..8 | version `u32` = 8 | |
| 8..12 | meta_magic `"META"` | distinguishes a v8 meta page from a legacy header |
| 12..16 | meta_crc `u32` | CRC32 of bytes 0..PAGE_SIZE with this field zeroed |
| 16..24 | generation `u64` | +1 per checkpoint; 1 = first commit |
| 24..32 | page_count `u64` | high-water mark; pages ≥ this are unallocated |
| 32..40 | fact_count `u64` | |
| 40..48 | last_checkpointed_tx_count `u64` | |
| 48..56 | eavt_root `u64` | 0 = empty tree |
| 56..64 | aevt_root `u64` | |
| 64..72 | avet_root `u64` | |
| 72..80 | vaet_root `u64` | |
| 80..88 | factdir_root `u64` | §3.4 |
| 88..96 | freelist_head `u64` | §3.5; 0 = empty |
| 96..104 | freelist_count `u64` | number of free page ids in the chain |
| 104 | fact_page_format `u8` | |
| 105..PAGE_SIZE | zero | reserved, covered by meta_crc |

The CRC covers the whole page, so the reserved bytes must be zero and future fields fit
without a layout change. The 84-byte `FileHeader` and its `index_checksum` and
`header_checksum` are removed.

### 3.2 Common page header (all non-meta pages)

| offset | field |
|---|---|
| 0 | page_type `u8` |
| 1 | reserved `u8` (0) |
| 2..4 | count `u16` (entries, keys, records or ids, by type) |
| 4..8 | crc32 `u32`, over the full page with this field zeroed |
| 8..16 | page_id `u64`, the page's own id (catches misdirected writes) |
| 16..24 | generation `u64`, the checkpoint generation that wrote it |

Read checks, in the page-cache load path so that every reader gets them:

1. crc32 matches.
2. `page_id` equals the requested id.
3. `generation ≤ active_meta.generation`.
4. `page_type` is the type the caller expects. Callers already check this.

Any failure returns a new structured error (§8) and the page is not cached.

| page_type | name | body after the 24-byte header |
|---|---|---|
| 0x02 | fact page | record directory + postcard facts, as today (the 8-byte `next_page` is dropped) |
| 0x21 | index leaf | slot directory + `(K, FactRef)` entries; **no `next_leaf`** |
| 0x22 | index internal | rightmost_child `u64` + keys/children, as today |
| 0x31 | fact-dir leaf | `(start_page u64, len u64)` extents |
| 0x32 | fact-dir internal | same layout as 0x22 with `u64` keys |
| 0x41 | free-list page | next `u64` + `count` × page id `u64` (up to 508 per page) |

The header grows from 12 to 24 bytes, so `MAX_FACT_BYTES` drops by 12, to 4056. That
is a public constant change and goes in the CHANGELOG.

### 3.3 Index trees

Leaves have no sibling pointer. With copy-on-write, a `next_leaf` pointer means changing
one leaf forces a rewrite of its left neighbour, then that neighbour's left neighbour,
and so on: the O(N) cascade this design removes. Scans use a cursor with a parent stack
instead (§5). Fill and split rules carry over from #315: balanced split by bytes, no
merging, because nothing is ever removed from an index.

### 3.4 Fact directory

A small B+tree keyed by `start_page`, with extent entries `(start_page, len)`. It is the
authoritative list of committed fact pages. It is used for full scans in page order
(`CommittedFactReader::stream_all`), and for #373's verify and `rebuild_indexes`.
Pages that a crashed checkpoint wrote are never in it, because only the committed meta's
directory counts.

New fact pages are allocated by appending (§4.2), so a checkpoint usually extends the
last extent. That rewrites only the rightmost path. A non-contiguous allocation adds a
new extent, which also touches only the rightmost path.

### 3.5 Free list

The free list is a singly linked chain of free-list pages. It holds ids of pages that the
active meta does not reference. The meta records the head and the total count.

- **Pop (allocation):** whole pages are taken from the head. A free-list page that is
  emptied is itself free. A partly used head page is rewritten copy-on-write with the
  rest of its ids.
- **Push (release):** page ids freed by this checkpoint, plus the free-list pages it
  emptied or rewrote, go into new pages pushed onto the head.
- Tail pages that are not touched are shared unchanged with the new meta.

Cost is O(pages allocated + pages freed) per checkpoint, never O(free-list size).

## 4. Checkpoint algorithm

Let `M` be the active meta, with generation `g`. The new checkpoint commits generation
`g' = g + 1` into slot `g' % 2`.

### 4.1 Invariant

A checkpoint writes only to pages that `M` does not reference. Those are pages on `M`'s
free list and pages at or above `M.page_count`. Three things follow:

- A crash before the meta write leaves `M` and everything it references untouched (G1).
- Queries running concurrently through the in-memory `OnDiskIndexReader` (still on `M`'s
  roots) never read a page that is being overwritten.
- Pages freed by this checkpoint are not reusable until the next one. The reader swap at
  the end of this checkpoint happens before that.

The implementation must confirm that no query keeps an `OnDiskIndexReader` across two
checkpoints. Today each `range_scan_*` runs under the `FactStorage` guard that
`set_committed_index_reader` needs, and `resolve()` reads only fact pages, which are
never freed. A test pins this down (§7).

### 4.2 Allocation policy

- **Fact pages** always append at `page_count`. Extents stay long and full scans stay
  sequential. Facts are never freed, so appending is the file's unavoidable growth.
- **Index, fact-directory and free-list pages** take from `M`'s free list first and
  append only when it is empty. In steady state, index churn reuses the pages freed one
  checkpoint earlier, and the file grows only by fact pages and net index growth.

### 4.3 Steps

1. **Snapshot.** Take `M` and the pending facts. Build an allocator from `M`'s free list
   and `M.page_count`.
2. **Facts.** Pack the pending facts into new fact pages, appended, stamped with `g'`.
   The `FactRef`s come from the allocated ids.
3. **Indexes.** For each of the four trees, run a copy-on-write batch insert of the sorted
   pending entries: descend from the root, route entries to leaves by separator keys,
   merge each touched leaf and split it by balanced bytes, then rewrite each touched
   internal node into a new page. A root split adds a level. Every page replaced along
   the way goes into `freed`.
4. **Fact directory.** Extend or add an extent on the rightmost path (copy-on-write) and
   add the replaced pages to `freed`.
5. **Free list.** Pop is already done by the allocator. Push `freed` and the consumed
   free-list pages as new head pages (§3.5).
6. **Data sync.** All pages above have been written, each stamped with `g'` and its CRC.
   Call `backend.sync()`.
7. **Commit.** Write meta `g'` to slot `g' % 2`, then call `backend.sync()`. This is the
   only commit point.
8. **Publish.** Swap the in-memory `CommittedFactReader` and `OnDiskIndexReader` to the
   new roots and clear the pending facts. Then delete the WAL in `do_checkpoint`, as
   today.

`fact_prefix_crc`, `rebuild_btree_incremental`'s copy-every-leaf path,
`collect_leaf_pages`, `invalidate_from` and the full-file checksum are removed.
The page cache stays write-through (`put_dirty` on every write). A reused page id
therefore never serves stale content.

### 4.4 Close-time checkpoint

Kept. Under G2 it costs O(change), so #434 acceptance 2 is met through the bound and
reopen stays fast.

## 5. Reads

- **Cursor.** `LeafCursor<K>` holds a stack of `(internal page, child index)`. To start,
  it descends to the first key ≥ `start`. To advance at the end of a leaf, it pops up to
  the nearest ancestor with a next child, then descends to that child's leftmost leaf.
  The amortised cost per step is O(1), and memory is O(depth). `range_scan`,
  `stream_all_entries` and the four `CommittedIndexReader` methods are rewritten on top
  of it, and their signatures stay the same. #432 exposes it later.
- **Full fact scans** walk the fact-directory extents in page order.
- **Open** reads two meta pages and nothing else. Pages are verified when first read
  (§3.2). The rebuild-on-checksum-mismatch branch in `load()` is deleted. A corrupt
  page is now an error on the read that hits it. Repairs are explicit through #373
  (`verify`, `rebuild_indexes` over the fact directory).

## 6. Migration from legacy files

A legacy file has an 84-byte `FileHeader` at page 0 and no valid meta page. It is either
v7, or a pre-release v8 file written by `v3` builds before this change. The two have the
same layout, and both are migrated by rebuilding indexes from the fact pages, so no
separate path is needed.

1. Read the legacy header (header CRC checked) and all facts with their old refs from
   pages `1..=fact_page_count` (legacy 12-byte fact header).
2. From `old_page_count` up, append new fact pages (new header, CRC, generation 1), then
   the fact directory and the four indexes built by `build_btree` (bulk build, now
   stamped and checksummed). Set the free list to old pages `2..old_page_count`. They
   become unreferenced at commit. Page 1 is excluded because it is meta slot B.
3. Sync, write meta generation 1 to page 0, sync. The commit overwrites the legacy
   header. Page 1 still holds old fact bytes, which are not a valid meta, so it is
   ignored until generation 2 writes it.
4. A crash before step 3 leaves the legacy header intact, and the appended pages lie
   beyond its `page_count`. The next open runs the migration again.

Cost: one O(N) pass. The file temporarily holds both copies, and the old region becomes
free pages that later index growth reuses. The file does not shrink until a vacuum
exists. That is an accepted one-time cost, listed in the CHANGELOG.

## 7. Testing

TDD per component. No new dependencies. Test assert messages follow the CodeQL rule.

- **Page header:** encode/verify round trip; a flipped bit, a wrong `page_id` and a
  future `generation` each give their own STG code; a failed page is not cached.
- **Meta selection:** both valid (highest generation wins); a torn newer slot (older
  wins); neither valid with a legacy header (migration); neither valid without one
  (error).
- **Copy-on-write insert equivalence:** random committed sets and pending batches
  (including splits, keys below the first leaf and above the last, root splits).
  `stream_all_entries` of the result must equal the merged sorted set. The old root
  must still stream its old contents unchanged, which proves it was not written to.
- **Cursor:** `range_scan` with random bounds, compared with a filter over the
  expected set, across depth 1–4 trees.
- **Allocator and free list:** pop/push across many generations. Invariant check after
  each checkpoint: the reachable set (all trees plus the free-list chain) and the free
  ids are disjoint, and together they cover `2..page_count` exactly. This also detects
  leaks.
- **Crash atomicity:** extend `crash_at_every_point_in_save_loses_no_checkpointed_fact`.
  Inject a failure at every write and sync, and add torn writes, including a torn meta
  write, using a new `FaultInjectingBackend` mode that writes a prefix of the page. After
  each, reopen with no index rebuild and check: checkpointed facts equal the model, WAL
  replay restores the rest, and the invariant above holds.
- **Cost bound (#434 acceptance 1, 4):** a page-counting backend measures pages written
  by a checkpoint after k ∈ {1, 100} facts, on graphs of 10k and 100k facts. The count
  must grow by at most the tree-depth difference between the two sizes. A guard asserts
  that no page referenced by the previous meta was written.
- **Concurrency:** queries running during checkpoints in `concurrency_test` still return
  consistent results, and no read ever fails CRC.
- **Migration:** v7 fixture → v8, with equal facts, query results and `tx_count` floor;
  pre-release v8 → v8; crash at every point of the migration.
- **Corruption surfacing:** corrupt a leaf, a fact page and a free-list page in turn. A
  query or a checkpoint returns the STG code and never wrong data.
- **Benchmark:** `checkpoint/after_1_fact` and `checkpoint/after_100k_facts` at 10k,
  100k and 1M facts, recording bytes written per checkpoint. Write amplification is
  reported for #394, and 1B is measured there.

## 8. Errors, docs, compatibility

- New STG codes: page checksum mismatch, page id mismatch, page generation ahead of
  meta, no valid meta page, free-list/allocator inconsistency. The codes for the removed
  paths (header CRC mismatch `INT-053`, index rebuild) stay registered and are marked
  deprecated, never recycled. Update `docs/ERROR_REFERENCE.md`.
- Browser: `BrowserBufferBackend` dirty sets become O(change). The IndexedDB flush must
  write the meta page in the same IDB transaction as the data pages. Verify this, and
  fix it if it is not already the case.
- Docs: the `Minigraf::checkpoint` and `wal_checkpoint_threshold` rustdoc drop "copies
  pages in proportion to the total index size". Also update the CLAUDE.md "File Format"
  section, `.wiki/Architecture.md`, CHANGELOG (format, `MAX_FACT_BYTES`, migration file
  growth) and ROADMAP.
- Philosophy: aligned. Single file, reliability first, no dependencies, and the format
  change goes into the unreleased v8, not a v9.

## 9. Alternatives rejected

- **Parent-held child checksums (ZFS style).** These would catch lost writes, but every
  internal pointer and every `FactRef` in all four indexes would grow by 4 bytes, against
  #433's density goal. In-page CRC plus page id plus generation catches torn, corrupt and
  misdirected pages.
- **Single header page.** A torn header write would make the file unopenable.
- **Typed-page scan instead of a fact directory.** Full scans would read fact pages in
  random order, and salvage would have to trust the free list to rule out orphan pages
  left by a crashed checkpoint.
- **Removing the close-time checkpoint.** It is unnecessary once checkpoints are
  O(change). Removing it would slow reopens and change binding behaviour (#322).
- **Appending to the last partial fact page in place.** Under per-page CRCs, a torn
  in-place write would destroy facts that were already committed. Each checkpoint
  therefore starts a fresh fact page, and density is #433's concern.

## 10. Delivery (PRs into `v3`)

1. **Cursor scans:** add `LeafCursor` and move every scan onto it, still on the current
   format. Behaviour does not change.
2. **v8 page format:** common page header with CRC/id/generation, verify on read, meta
   pages A/B, fact directory, legacy migration. `save()` still does a full rebuild into
   fresh pages, which is atomic but O(N). (#388, #374 atomicity)
3. **Copy-on-write insert, allocator, free list:** O(change) checkpoints. (#434)
4. **Cost and crash test hardening, benchmarks, docs.**

PR 2 freezes the page format for #391. PRs 3 and 4 do not change it.
