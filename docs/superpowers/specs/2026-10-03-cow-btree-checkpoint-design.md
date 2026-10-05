# Copy-on-write B+tree checkpoints (#374, #434, #388) — Design

**Issues:** #374 (crash-atomic `save()`), #434 (checkpoint cost O(change)), #388 (per-page checksums)
**Milestone:** v3.0.0, branch `v3`, file format v8 (unreleased; this redefines its layout)
**Must land before:** #391 golden-file corpus freezes v8
**Builds on:** #315 / `2026-09-26-incremental-checkpoint-design.md` (balanced leaf split, routing)
**Companion:** `2026-10-05-covering-index-key-encoding-design.md` (#433). It defines what
the trees store: covering keys, dictionaries and value pages. This spec defines how pages
are laid out, checksummed and committed.

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
- G2. A checkpoint writes only new value pages, the copied-on-write path from each
  modified leaf to its root in each tree, free-list pages and one meta page.
  Bytes written are O(k · log N) after k new facts and never depend on N. This bound
  covers `checkpoint()`, the auto-checkpoint and the close-time checkpoint in
  `Drop for Inner`. (#434 acceptance 1, 2, 4)
- G3. Every non-meta page carries a CRC32, its own page id and the checkpoint
  generation that wrote it. Every read checks them. A failed check returns a structured
  `STG-0xx` error and never returns the page's data. (#388)
- G4. Open is O(1) in graph size: no full-file checksum pass.
- G5. v7 files migrate on open. The migration is crash-safe and writes checksums
  (#388 migration clause). Pre-release v8 files written by `v3` builds before this
  change are not supported: v8 was never released, so they fail to open with the
  "no valid meta page" error (§8).

**Non-goals (separate issues)**

- Shrinking the file or compacting free pages. The free list is reused. Vacuum is
  a later, separate operation (#434: "space reclamation can be a later operation").
- Key encoding, dictionaries, covering indexes and node layout. These are settled in the
  companion #433 spec. Bε-style buffered internal nodes (§3.1.2) are the v3.x fallback
  if #394 still measures write amplification > 10.
- The public streaming cursor API (#432). §5 adds an internal leaf cursor that #432
  can build on.
- Lost-write detection, where the device acknowledges a write that never lands and an
  older valid page stays at that location. Parent-held checksums were considered and
  rejected (§9).

## 3. On-disk layout (v8)

All integers are little-endian. Page size stays 4096.

```
Page 0, 1   Meta pages A and B (alternating commits)
Page 2+     Any mix of: B+tree nodes (EAVT/AEVT/AVET/VAET/DICT), value pages,
            free-list pages, free pages
Sidecar     <db>.wal — header gains base_generation (§3.1.1); entries unchanged
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
| 80..88 | dict_root `u64` | §3.4 |
| 88..96 | freelist_head `u64` | §3.5; 0 = empty |
| 96..104 | freelist_count `u64` | number of free page ids in the chain |
| 104..112 | next_eid `u64` | next entity id (#433 §3.1) |
| 112..120 | required_features `u64` | bitmask; v3.0.0 writes 0 (§3.1.2) |
| 120..124 | next_iid `u32` | next ident id (#433 §3.1) |
| 124..PAGE_SIZE | zero | reserved, covered by meta_crc |

The CRC covers the whole page, so the reserved bytes must be zero and future fields fit
without a layout change. The 84-byte `FileHeader` and its `index_checksum` and
`header_checksum` are removed.

#### 3.1.1 Choosing the meta on open

Each slot is classified as one of:

- **empty:** no `"MGRF"`/`"META"` magic. This is slot B before generation 2.
- **valid:** magic, version and `meta_crc` all check out.
- **damaged:** magic present, but the CRC or a field fails.

If both slots are valid, the higher generation wins. If neither is valid, see §6
(a v7 header, the migration backup meta) or fail with "no valid meta page".

If exactly one slot is valid, with generation `g`, the other slot either held a newer
commit `g+1` that is torn or rotted, or held the older `g-1` and was damaged later.
Open must never silently drop a checkpoint whose WAL is already gone, so it decides
with two extra facts:

1. **WAL `base_generation`.** The WAL header records the generation of the meta that
   was active when the WAL file was created; it is reserved bytes today, so this is
   WAL version 2. A WAL is deleted only after a commit is durable, so an interrupted
   commit of `g+1` always leaves a WAL with `base_generation == g`.
2. **Evidence of `g+1` in the data pages.** Generation `g+1` can only have written
   to pages on `M_g`'s free list or at or above `M_g.page_count` (§4.1). A valid page in
   one of those places stamped `generation == g+1` means `g+1` wrote its data. A
   checkpoint with any new data writes at least one such page. The check reads
   `M_g`'s free-list pages and the pages from `M_g.page_count` to the end of the file.
   That cost is proportional to the last change, and it is paid only in this rare
   case.

| WAL | page stamped `g+1` where `g+1` could write | meaning | action |
|---|---|---|---|
| `base_generation == g` | any | torn commit of `g+1`, or the older slot was damaged | open at `g`, replay the WAL |
| `base_generation > g` | any | a later commit existed and its meta is lost | error: meta damaged after commit |
| none | found | `g+1` committed (its WAL was deleted), then its meta rotted | error: meta damaged after commit |
| none | not found | the older slot was damaged, or `g == 1` with B empty | open at `g` |

The error is a new STG code. The file is not modified. #373's tooling can recover at
`g` explicitly, and that loses the last checkpoint's facts. The next commit always
rewrites the damaged slot, because it is the inactive one.

The browser backend writes through IndexedDB transactions and has no torn writes. Its
flush keeps the meta page in the same transaction as the data pages (§8).

#### 3.1.2 Feature bits

`required_features` lets later v3.x releases add structures without a format v9.
Examples are Bε-style buffered internal nodes or a new leaf encoding, each as a new
page type. Bit `i` set means a reader must understand feature `i` to read the file
correctly. Open fails with a new STG code ("unsupported file feature", naming the
bits) if any set bit is unknown to the running version. Nothing else is checked, so
the format version stays 8.

- A writer sets a bit at the first commit that writes a page that needs it. Once set,
  a bit is never cleared.
- v3.0.0 defines no bits and always writes 0.
- The bit registry lives next to the error-code registry and follows the same rule:
  bits are never reused.

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

1. `page_type` is a known v8 type (table below). This is a cheap first-byte check, so a
   legacy or foreign page fails before any CRC is computed.
2. crc32 matches.
3. `page_id` equals the requested id.
4. `generation ≤ active_meta.generation`.
5. `page_type` is the specific type the caller expects. Callers check this after the
   cache returns, as they do today.

Any failure returns a new structured error (§8) and the page is not cached.

| page_type | name | body after the 24-byte header |
|---|---|---|
| 0x51 | value page | record directory + long values (#433 §4.3) |
| 0x52 | value overflow | reserved, not written |
| 0x61 | B+tree leaf | prefix-compressed entries + restart array (#433 §5); **no `next_leaf`** |
| 0x62 | B+tree internal | rightmost_child `u64` + truncated separators/children (#433 §5) |
| 0x81 | free-list page | next `u64` + `count` × page id `u64` (up to 508 per page) |

All five trees (EAVT, AEVT, AVET, VAET, DICT) use 0x61/0x62. A node does not record
which tree it belongs to; its tree is given by the root it was reached from.

Type values follow the existing convention: the high nibble is the page family and the
low nibble is the variant. No v8 value reuses one from an earlier format. Those are
0x02/0x03 for v5–v7 fact pages, 0x11 for v5 index pages, and 0x21/0x22 for the v6/v7
B+tree. Every v8 page has a different header from its v7 counterpart, so the type byte
alone tells a v8 page from a legacy one. A legacy page read where a v8 page is expected
fails the type check before the CRC is computed. #373's verify and salvage can classify
any page from its first byte, and v7 migration never confuses an old page with a new
one. Retired values stay reserved and are never reassigned.

### 3.3 B+trees

The node contents are defined in #433 §5. Leaves have no sibling pointer. With copy-on-write, a `next_leaf` pointer means changing
one leaf forces a rewrite of its left neighbour, then that neighbour's left neighbour,
and so on: the O(N) cascade this design removes. Scans use a cursor with a parent stack
instead (§5). Fill and split rules carry over from #315: balanced split by bytes, no
merging, because nothing is ever removed from an index.

### 3.4 DICT tree and value pages

DICT holds the entity and ident dictionaries, the tx table and the long-value dedup
index (#433 §3.2). Value pages hold long values. They are append-only, never rewritten
and never freed, and they are reachable only through value refs in index keys, so
verify (#373) finds them by walking the indexes. There are no fact pages and no fact
directory: EAVT is the complete, ordered list of facts.

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
`set_committed_index_reader` needs. Long-value reads go only to value pages, which are
never freed. A test pins this down (§7).

### 4.2 Allocation policy

- **Value pages** always append at `page_count`. They are never freed.
- **B+tree and free-list pages** take from `M`'s free list first and append only when
  it is empty. In steady state, tree churn reuses the pages freed one checkpoint
  earlier, and the file grows only by net tree growth and new long values.
- **Layout over time:** page kinds interleave freely. Nothing needs to be contiguous;
  every page is reached from a root in the meta page or from the free-list head.

### 4.3 Steps

1. **Snapshot.** Take `M` and the pending facts. Build an allocator from `M`'s free list
   and `M.page_count`.
2. **Long values.** Write new long values (after dedup, #433 §4.3) into new value pages,
   appended and stamped with `g'`. Their value refs complete the pending keys.
3. **Trees.** For each of the five trees (the four indexes, then DICT with the new
   eid/iid/tx/dedup entries), run a copy-on-write batch insert of the sorted pending
   entries: descend from the root, route entries to leaves by separator keys,
   merge each touched leaf and split it by balanced bytes, then rewrite each touched
   internal node into a new page. A root split adds a level. Every page replaced along
   the way goes into `freed`.
4. **Free list.** Pop is already done by the allocator. Push `freed` and the consumed
   free-list pages as new head pages (§3.5).
5. **Data sync.** All pages above have been written, each stamped with `g'` and its CRC.
   Call `backend.sync()`.
6. **Commit.** Write meta `g'` (with the new roots, `next_eid` and `next_iid`) to slot `g' % 2`, then call `backend.sync()`. This is the
   only commit point.
7. **Publish.** Swap the in-memory readers to the new roots and clear the pending facts. Then delete the WAL in `do_checkpoint`, as
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
- **Seek.** `LeafCursor::seek(key)` moves to the first entry ≥ `key`, never backwards.
  If `key` is in the current leaf, it binary-searches that leaf. Otherwise it pops the
  stack to the lowest ancestor whose key range still covers `key`, and descends from
  there, not from the root. A seek to a nearby key costs O(1) page visits, and one to a
  key d leaves away costs O(log d). A seek to a key below the current position is a
  no-op. Merge joins and leapfrog-style worst-case-optimal joins in the streaming
  engine (#432) call `seek` on each index ordering constantly, so it must stay cheap.
- **Full fact scans** are an EAVT cursor scan. Covering keys mean no other page is read
  except for long values.
- **Open** reads two meta pages and nothing else. Pages are verified when first read
  (§3.2). The rebuild-on-checksum-mismatch branch in `load()` is deleted. A corrupt
  page is now an error on the read that hits it. Repairs are explicit through #373:
  `verify`, and `rebuild_indexes` from EAVT or AEVT, each of which holds every fact.

## 6. Migration from legacy files

A legacy file has an 84-byte v7 `FileHeader` at page 0 (version 7, header CRC valid)
and no valid meta page. Only v7 is migrated. A page 0 that is neither a valid meta page
nor a v7 header, including a pre-release v8 single-header file, is rejected with
"no valid meta page".

1. Read the legacy header (header CRC checked) and all facts from pages
   `1..=fact_page_count` (legacy 12-byte fact header).
2. Assign eids and iids in `tx_count` order (#433 §3.1). From `old_page_count` up,
   append value pages for long values, then the five trees built by `build_btree` (bulk
   build from sorted input, stamped and checksummed). Set the free list to old pages
   `2..old_page_count`. They become unreferenced at commit. Page 1 is excluded because
   it is meta slot B.
3. Append a **backup copy of meta generation 1** as the last page, sync, then write meta
   generation 1 to page 0 and sync. The commit overwrites the legacy header. Page 1
   still holds old fact bytes, which are not a valid meta, so it is ignored until
   generation 2 writes it. The backup page is on the generation-1 free list.
4. A crash before the page 0 write leaves the legacy header intact, and the appended
   pages lie beyond its `page_count`. The next open runs the migration again.
5. A torn page 0 write leaves neither a valid meta nor a v7 header, and page 1 is not a
   meta either. Only at this point does open read the file's last page. If it is a
   valid meta with generation 1 whose `page_count` matches the file, open copies it to
   page 0 and continues. Otherwise it fails with "no valid meta page". Generation 2
   reuses the backup page from the free list, so the fallback never sees a stale
   backup.

Cost: one O(N) pass. The file temporarily holds both copies, and the old region becomes
free pages that later index growth reuses. The file does not shrink until a vacuum
exists. That is an accepted one-time cost, listed in the CHANGELOG.

## 7. Testing

TDD per component. No new dependencies. Test assert messages follow the CodeQL rule.

- **Page header:** encode/verify round trip; a flipped bit, a wrong `page_id` and a
  future `generation` each give their own STG code; a failed page is not cached.
- **Meta selection:** both valid (highest generation wins). Each row of the §3.1.1
  table: torn newer slot with WAL base `g` (open at `g`, replay); WAL base `> g`
  (error); no WAL with a `g+1` page on `M_g`'s free list or past `M_g.page_count`
  (error, each location tested); no WAL and no `g+1` page (open at `g`); `g == 1` with slot B empty. Neither valid: v7 header (migration);
  torn migration commit with backup meta (recovered); pre-release v8 single header or
  anything else (error).
- **Copy-on-write insert equivalence:** random committed sets and pending batches
  (including splits, keys below the first leaf and above the last, root splits).
  `stream_all_entries` of the result must equal the merged sorted set. The old root
  must still stream its old contents unchanged, which proves it was not written to.
- **Cursor:** `range_scan` with random bounds, compared with a filter over the
  expected set, across depth 1–4 trees.
- **Seek:** random monotone seek sequences give the same positions as a fresh descent
  from the root. This covers seeks within a leaf, to a sibling, across subtrees, past
  the end, and backwards (no-op). A page-read counter shows that a seek to the
  neighbouring leaf reads no internal page that the cursor already holds.
- **Feature bits:** a file with an unknown `required_features` bit fails with the new
  STG code and is not modified. v3.0.0 writes 0.
- **Allocator and free list:** pop/push across many generations. Invariant check after
  each checkpoint: the reachable set (all five trees, the value pages their keys reference, and the free-list chain) and the free
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
  crash at every point of the migration; a pre-release v8 single-header file is rejected.
- **Corruption surfacing:** corrupt a leaf, a value page and a free-list page in turn. A
  query or a checkpoint returns the STG code and never wrong data.
- **Benchmark:** `checkpoint/after_1_fact` and `checkpoint/after_100k_facts` at 10k,
  100k and 1M facts, recording bytes written per checkpoint. Write amplification is
  reported for #394, and 1B is measured there.

## 8. Errors, docs, compatibility

- New STG codes: page checksum mismatch, page id mismatch, page generation ahead of
  meta, no valid meta page, meta damaged after commit (§3.1.1), unsupported file
  feature (§3.1.2), free-list/allocator inconsistency. The codes for the removed paths
  (header CRC mismatch `INT-053`, index rebuild) stay registered and are marked
  deprecated, never recycled. Update `docs/ERROR_REFERENCE.md`.
- WAL format version 2: the header gains `base_generation u64` at bytes 8..16, taken
  from the reserved bytes. A v1 WAL is accepted only next to a v7 file being migrated.
- Browser: `BrowserBufferBackend` dirty sets become O(change). The IndexedDB flush must
  write the meta page in the same IDB transaction as the data pages. Verify this, and
  fix it if it is not already the case.
- Docs: the `Minigraf::checkpoint` and `wal_checkpoint_threshold` rustdoc drop "copies
  pages in proportion to the total index size". Also update the CLAUDE.md "File Format"
  section, `.wiki/Architecture.md`, CHANGELOG (format, `MAX_FACT_BYTES` →
  `MAX_VALUE_BYTES`, migration file growth) and ROADMAP.
- Philosophy: aligned. Single file, reliability first, no dependencies, and the format
  change goes into the unreleased v8, not a v9.

## 9. Alternatives rejected

- **Parent-held child checksums (ZFS style).** These would catch lost writes, but every
  internal pointer and every value ref would grow by 4 bytes, against #433's density
  goal. In-page CRC plus page id plus generation catches torn, corrupt and
  misdirected pages.
- **Single header page.** A torn header write would make the file unopenable.
- **Removing the close-time checkpoint.** It is unnecessary once checkpoints are
  O(change). Removing it would slow reopens and change binding behaviour (#322).
- **Appending to the last partial value page in place.** Under per-page CRCs, a torn
  in-place write would destroy values that were already committed. Each checkpoint
  that writes long values therefore starts a fresh value page. The space lost to
  partly filled pages is bounded by one page per checkpoint that writes long values.

## 10. Delivery (PRs into `v3`)

1. **Cursor scans:** add `LeafCursor` with `seek` and move every scan onto it, still on
   the current format. Behaviour does not change.
2. **v8 page format:** common page header with CRC/id/generation, verify on read, meta
   pages A/B with meta selection, feature bits, WAL v2. `save()` still does a full rebuild
   into fresh pages, which is atomic but O(N). (#388, #374 atomicity)
3. **Covering keys (#433):** byte-comparable key encoding, prefix-compressed nodes, DICT,
   value pages, eid/iid assignment, covering reads, v7 migration.
4. **Copy-on-write insert, allocator, free list:** O(change) checkpoints. (#434)
5. **Cost and crash test hardening, benchmarks, docs.**

The format freezes for #391 once PRs 2 and 3 have merged. PRs 4 and 5 do not change it.
