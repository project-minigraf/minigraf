# v8 storage format: copy-on-write checkpoints and covering indexes — Design

**Issues:** #374 (crash-atomic `save()`), #434 (checkpoint cost O(change)), #388 (per-page
checksums), #433 (format decisions for 1B facts)
**Milestone:** v3.0.0, branch `v3`, file format v8 (unreleased; this redefines its layout)
**Must land before:** #391 golden-file corpus freezes v8
**Builds on:** #315 / `2026-09-26-incremental-checkpoint-design.md` (balanced leaf split, routing)

## 1. Problem

**Checkpoints.** `PersistentFactStorage::save()` has three defects that share one cause:
fact pages must stay contiguous at `1..=fact_page_count`, and the indexes sit directly
after them.

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

**Density and access cost (#433).** A fact costs about 520 B on disk at best today, and
about 2 KB as measured. The fact record is ~70 B of postcard data. Each of the four
index entries is ~70–90 B, and each one repeats a 16-byte UUID, the attribute string and
8-byte timestamps. #433 allows at most 300 GB for 1B facts, which is 300 B per fact
including history. Two further costs grow with N:

- **Indexes are not covering.** `range_scan` returns `FactRef`s, so every match costs a
  read of a fact page. A streaming scan that matches 1M facts means 1M random page reads.
- **Keys land on random leaves.** Entity UUIDs derived from keywords (v5) and
  caller-supplied UUIDs are effectively random in index order, and so are most values.
  A checkpoint of k facts touches about k different leaves in every index. At 1B facts
  that is ~40× write amplification, against #434's limit of 10.

## 2. Goals and non-goals

**Goals**

- G1. A crash at any point of a checkpoint (any write, any sync, a torn page write)
  reopens to the last committed checkpoint plus WAL replay. No checkpointed fact is lost.
  No index rebuild happens on open. (#374)
- G2. A checkpoint writes only new value pages, the copied-on-write path from each
  modified leaf to its root in each tree, free-list pages and one meta page. Bytes
  written are O(k · log N) after k new facts and never depend on N. This bound covers
  `checkpoint()`, the auto-checkpoint and the close-time checkpoint in
  `Drop for Inner`. (#434 acceptance 1, 2, 4)
- G3. Every non-meta page carries a CRC32, its own page id and the checkpoint
  generation that wrote it. Every read checks them. A failed check returns a structured
  `STG-0xx` error and never returns the page's data. (#388)
- G4. Open is O(1) in graph size: no full-file checksum pass.
- G5. v7 files migrate on open. The migration is crash-safe and writes checksums
  (#388 migration clause). Pre-release v8 files written by `v3` builds before this
  change are not supported: v8 was never released, so they fail to open with the
  "no valid meta page" error (§12).
- G6. At most 300 B per fact on disk at 1B facts, including history (estimate in §10:
  110–140 B). A query match never needs a page read beyond the index leaf, except for
  long values. (#433)

**Non-goals (separate issues)**

- Shrinking the file or compacting free pages. The free list is reused. Vacuum is
  a later, separate operation (#434: "space reclamation can be a later operation").
- The public streaming cursor API (#432). §7 adds an internal leaf cursor with `seek`
  that #432 can build on.
- Bε-style buffered internal nodes. They are the v3.x fallback behind a feature bit
  (§4.1.2) if #394 still measures write amplification above 10.
- Lost-write detection, where the device acknowledges a write that never lands and an
  older valid page stays at that location. Parent-held checksums were considered and
  rejected (§13).

## 3. Decisions

| # | decision | alternative rejected |
|---|---|---|
| D1 | **Two alternating meta pages** with a generation counter and CRC | Single header page: a torn write makes the file unopenable |
| D2 | **In-page CRC32 + own page id + generation** on every non-meta page | Parent-held checksums: +4 B per pointer and per value ref |
| D3 | **Copy-on-write B+trees with a free list**, no leaf sibling pointers | Today's relocate-everything checkpoint |
| D4 | **Covering indexes.** Every index entry holds the whole fact. Fact pages, `FactRef` and the fact directory are removed. | Fact pages plus reference indexes: one random read per match |
| D5 | **Sequential internal entity ids** (`eid: u64`), with a dictionary mapping UUID ↔ eid. The API still speaks UUIDs. | 16-byte UUIDs in keys: random leaf placement, larger keys |
| D6 | **Ident dictionary** (`iid: u32`) for attribute names and keyword values | Strings in every key |
| D7 | **Out-of-line long values.** Values above 64 B are stored once (deduplicated) in value pages. Keys hold a 32-byte prefix, an 8-byte hash and a value ref. | Always inline: a 4 KB string copied into 3 indexes, leaf fanout of 1 |
| D8 | **Byte-comparable keys** (memcmp order). Integers use order-preserving variable-length encoding. Leaves are prefix-compressed. Separators are truncated to their shortest form. | postcard structs compared after decoding: no prefix compression, decode on every compare |
| D9 | **Datomic component order, newest transaction first:** `(e,a,v)` triples are contiguous, followed by `tx` descending | Today's `(e,a,vf,vt,tx,v)`: history of one triple is scattered |
| D10 | **Close-time checkpoint kept** (it becomes O(change)) | Removing it: slower reopens, binding behaviour change (#322) |

### 3.1 Covering does not multiply writes

Each fact is written into 3–4 index entries: EAVT, AEVT and AVET always, and VAET only
for `Ref` values. That is not an added cost. An index ordered by any permutation of
`e a v` must already hold `e`, `a`, `v` and `tx` in its key to order entries and keep
them distinct (#371). Covering adds only `vf`, `vt` and `op`, about 9 B (`vt` FOREVER is
1 byte). That is about the size of the `FactRef` it replaces.

| per fact | today (v7 / v8 dev) | covering (this design) |
|---|---|---|
| fact record | 1 × ~70 B | none |
| index entries | 4 × ~80 B: already nearly whole facts, plus a `FactRef` | ~3.3 × ~25 B after prefix compression |
| raw bytes written | ~390 B | ~80 B |
| random page reads per query match | 1 (fact fetch) | 0 |

Checkpoint cost is set by leaves touched per tree, not by entry size. Any design with
four sort orders touches four trees.

Leaves cannot be shared between the four trees. A leaf is a run of entries that are
contiguous in its own tree's sort order, and the four orders disagree about which facts
are neighbours. Keeping each fact once and pointing at it from the trees is the
clustered-plus-secondary design that D4 rejects. Copy-on-write does share nodes, but
across generations of the same tree (§8).

## 4. On-disk layout

All integers in page headers and meta fields are little-endian. Key bytes follow §6.
Page size stays 4096.

```
Page 0, 1   Meta pages A and B (alternating commits)
Page 2+     Any mix of: B+tree nodes (EAVT/AEVT/AVET/VAET/DICT), value pages,
            free-list pages, free pages
Sidecar     <db>.wal — header gains base_generation (§4.1.1); entries unchanged
```

### 4.1 Meta pages

Each checkpoint writes the inactive slot: odd generations go to page 0 (slot A) and even
generations to page 1 (slot B), so `slot = (generation − 1) % 2`. On open, both slots
are read and one is chosen by §4.1.1.

A new file is created with an empty generation-1 meta in slot A, synced before anything
else is written, so every file that can hold committed data has a valid meta. If
neither slot is valid and the file has at most two pages, no commit can have written
data (every commit with data writes a page ≥ 2), and open initialises the file again.

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
| 80..88 | dict_root `u64` | §5.2 |
| 88..96 | freelist_head `u64` | §4.4; 0 = empty |
| 96..104 | freelist_count `u64` | number of free page ids in the chain |
| 104..112 | next_eid `u64` | next entity id (§5.1) |
| 112..120 | required_features `u64` | bitmask; v3.0.0 writes 0 (§4.1.2) |
| 120..124 | next_iid `u32` | next ident id (§5.1) |
| 124..PAGE_SIZE | zero | reserved, covered by meta_crc |

The CRC covers the whole page, so the reserved bytes must be zero and future fields fit
without a layout change. The 84-byte `FileHeader` and its `index_checksum` and
`header_checksum` are removed.

#### 4.1.1 Choosing the meta on open

Each slot is classified as one of:

- **empty:** no `"MGRF"`/`"META"` magic. This is slot B before generation 2.
- **valid:** magic, version and `meta_crc` all check out.
- **damaged:** magic present, but the CRC or a field fails.

If both slots are valid, the higher generation wins. If neither is valid, see §9
(a v7 header, the migration backup meta) or fail with "no valid meta page".

If exactly one slot is valid, with generation `g`, the other slot either held a newer
commit `g+1` that is torn or rotted, or held the older `g-1` and was damaged later.
Open must never silently drop a checkpoint whose WAL is already gone, so it decides
with two extra facts:

1. **WAL `base_generation`.** The WAL header records the generation of the meta that
   was active when the WAL file was created; it is reserved bytes today, so this is
   WAL version 2. A WAL is deleted only after a commit is durable, so an interrupted
   commit of `g+1` always leaves a WAL with `base_generation ≤ g`. The base can be
   below `g` when a crash came between a commit and the WAL delete: the reopened
   session keeps appending to that WAL, which still holds every fact not in `g`.
   A version 1 WAL (v2.x) can only sit next to a migrated file whose generation-2
   commit has not finished, so it reads as base generation 1.
2. **Evidence of `g+1` in the data pages.** Generation `g+1` can only have written to
   pages on `M_g`'s free list or at or above `M_g.page_count` (§8.1). A valid page in
   one of those places stamped `generation == g+1` means `g+1` wrote its data. A
   checkpoint with any new data writes at least one such page. The check reads `M_g`'s
   free-list pages and the pages from `M_g.page_count` to the end of the file. That cost
   is proportional to the last change, and it is paid only in this rare case.

| WAL | page stamped `g+1` where `g+1` could write | meaning | action |
|---|---|---|---|
| `base_generation ≤ g` | any | torn commit of `g+1`, or the older slot was damaged | open at `g`, replay the WAL |
| `base_generation > g` | any | a later commit existed and its meta is lost | error: meta damaged after commit |
| none | found | `g+1` committed (its WAL was deleted), then its meta rotted | error: meta damaged after commit |
| none | not found | the older slot was damaged, or `g == 1` with B empty | open at `g` |

The error is a new STG code. The file is not modified. #373's tooling can recover at
`g` explicitly, and that loses the last checkpoint's facts. The next commit always
rewrites the damaged slot, because it is the inactive one.

The browser backend writes through IndexedDB transactions and has no torn writes. Its
flush keeps the meta page in the same transaction as the data pages (§12).

#### 4.1.2 Feature bits

`required_features` lets later v3.x releases add structures without a format v9.
Examples are Bε-style buffered internal nodes, opt-in AVET (§13) or a new leaf encoding,
each as a new page type. Bit `i` set means a reader must understand feature `i` to read
the file correctly. Open fails with a new STG code ("unsupported file feature", naming
the bits) if any set bit is unknown to the running version. Nothing else is checked, so
the format version stays 8.

- A writer sets a bit at the first commit that writes a page that needs it. Once set,
  a bit is never cleared.
- v3.0.0 defines no bits and always writes 0.
- The bit registry lives next to the error-code registry and follows the same rule:
  bits are never reused.

### 4.2 Common page header (all non-meta pages)

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

Any failure returns a new structured error (§12) and the page is not cached.

| page_type | name | body after the 24-byte header |
|---|---|---|
| 0x51 | value page | record directory + long values (§6.3) |
| 0x52 | value overflow | reserved, not written |
| 0x61 | B+tree leaf | prefix-compressed entries + restart array (§4.3); **no `next_leaf`** |
| 0x62 | B+tree internal | rightmost_child `u64` + truncated separators/children (§4.3) |
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

During development, between delivery PRs 2 and 3 (§14), fact pages still exist and use
the interim type 0x41 with this header. Files carrying it are pre-release and are
rejected like any other pre-release v8 file. 0x41 is retired with them.

### 4.3 B+tree nodes

Leaves have no sibling pointer. With copy-on-write, a `next_leaf` pointer means changing
one leaf forces a rewrite of its left neighbour, then that neighbour's left neighbour,
and so on: the O(N) cascade this design removes. Scans use a cursor with a parent stack
instead (§7).

**Leaf body:**

- Entries in key order. Each entry is
  `varint shared_prefix_len ‖ varint suffix_len ‖ suffix ‖ varint value_len ‖ value`.
  `value_len` is always 0 for the four index trees.
- Every 16th entry is a **restart point**: `shared_prefix_len = 0`, and its offset is
  stored in a restart array at the end of the page. A lookup binary-searches the
  restart points, then scans at most 16 entries.

**Internal body:**

- `rightmost_child u64`, then `(separator, child u64)` pairs.
- Separators are the shortest byte string `s` with `max(left) < s ≤ min(right)`. They
  are usually a few bytes, so fanout is 200 or more.

**Splits** are balanced by bytes, as in #315. There is no merging: nothing is ever
removed from a tree.

### 4.4 Free list

The free list is a singly linked chain of free-list pages. It holds ids of pages that the
active meta does not reference. The meta records the head and the total count.

- **Pop (allocation):** whole pages are taken from the head. A free-list page that is
  emptied is itself free. A partly used head page is rewritten copy-on-write with the
  rest of its ids.
- **Push (release):** page ids freed by this checkpoint, plus the free-list pages it
  emptied or rewrote, go into new pages pushed onto the head.
- Tail pages that are not touched are shared unchanged with the new meta.

Cost is O(pages allocated + pages freed) per checkpoint, never O(free-list size).

## 5. Data model

### 5.1 Identifiers

- **eid** (`u64`, starting at 1): assigned to each entity UUID the first time it appears
  as an entity or as a `Ref` value. `next_eid` is stored in the meta page.
- **iid** (`u32`, starting at 1): assigned to each attribute name and each keyword value
  the first time it appears. Attribute and keyword values share one namespace, so the
  ident `:status/active` gets one id whether it is used as an attribute or a value.
  `next_iid` is stored in the meta page.
- **Assignment time.** Ids are assigned in memory at transact time, in WAL order. WAL
  replay reassigns them in the same order, starting from the meta page's counters, so the
  result is deterministic and crash-safe without logging ids. Assignment at transact
  time lets the in-memory pending index use the same encoded keys as the on-disk trees,
  so merging pending and committed data is a plain merge of two sorted streams.
- **Translation boundary.** The query layer receives facts with UUIDs and strings, as
  today. The reader translates ids through cached dictionary lookups. The streaming
  engine (#432) should work on raw ids and translate only at output; this design keeps
  that possible but does not require it in 3.0.0.

### 5.2 Trees

Five B+trees, all with the node format of §4.3 and all copy-on-write:

| tree | key | entry value | holds |
|---|---|---|---|
| EAVT | `e a v tx↓ vf vt op` | — | every fact |
| AEVT | `a e v tx↓ vf vt op` | — | every fact |
| AVET | `a v e tx↓ vf vt op` | — | every fact |
| VAET | `v a e tx↓ vf vt op` | — | facts whose value is a `Ref` (`v` = target eid) |
| DICT | tag-prefixed, below | per tag | dictionaries and the tx table |

EAVT and AEVT each hold the complete fact set. Either one can rebuild every other tree
(#373 `rebuild_indexes`), so a single corrupt index never loses data. There are no fact
pages and no fact directory: EAVT is the complete, ordered list of facts.

DICT entries:

| tag | key | value | used for |
|---|---|---|---|
| 0x01 | `uuid[16]` | `eid` | UUID → eid (transact, query constants) |
| 0x02 | `eid` | `uuid[16]` | eid → UUID (results) |
| 0x03 | ident bytes (escaped) | `iid` | name → iid |
| 0x04 | `iid` | ident bytes | iid → name |
| 0x05 | `tx_count` | `tx_id` (wall-clock ms) | `Fact.tx_id`, once per transaction |
| 0x06 | `hash64` of a long value | value ref | dedup of long values (§6.3) |

Tags 0x02, 0x04 and 0x05 receive increasing keys, so their inserts touch only the
rightmost path. Tags 0x01 and 0x06 are random-key inserts, but they happen once per new
entity or new long value, not once per fact.

### 5.3 Component order (D9)

- `(e, a, v)` triples are contiguous in EAVT, and the history of a triple follows its
  key, newest first. "Latest assertion per triple" (#435) and "live facts only" (#379)
  become streaming operations: read the first entry per triple and skip the rest with
  `seek` (§7).
- `:as-of N` seeks to `(e, a, v, ↓N)` and lands directly on the newest assertion at or
  before N, in O(log) per triple instead of a scan of its history.
- Valid time (`vf`, `vt`) and `op` (assert or retract) follow `tx`. They are filtered
  per entry, as today.
- This changes the order of committed scan results compared with v7: eid order instead
  of UUID order, iid order instead of attribute-name order. Query results are sets, but
  every test or binding that relies on a particular order must be found and fixed
  (§11).

## 6. Key encoding (D8)

All components are encoded so that comparing the bytes with memcmp gives the logical
order. Encoded keys are compared without decoding.

### 6.1 Integers

Integers use the FoundationDB tuple-layer encoding. A type byte holds the sign and the
byte length, followed by the minimal big-endian magnitude (one's complement for
negatives).

- 0 takes 1 byte. Values up to 255 take 2 bytes. A ms timestamp (~2^41) takes 7 bytes.
- `eid`, `iid` and `tx_count` use the unsigned form.
- `vt == VALID_TIME_FOREVER`, the common case, is the single byte `0xFF`. It sorts after
  every finite time.
- `tx↓` is the encoding of `u64::MAX - tx_count`, so newer transactions sort first.

### 6.2 Values

A type tag is followed by the payload. The tag order is unchanged from v7, so
cross-type ordering stays Null < Boolean < Integer < Float < String < Keyword < Ref.

| type | payload |
|---|---|
| Null | — |
| Boolean | 1 byte |
| Integer | §6.1 signed |
| Float | 8 bytes, the existing order-preserving bit transform, NaN canonicalised |
| String ≤ 64 B | bytes with 0x00 escaped as `00 FF`, terminated by `00 00` |
| String > 64 B | first 32 bytes escaped, then marker `00 01`, `hash64`, value ref (`page u64` + `slot u16`) |
| Keyword | `iid` (§6.1). Keywords sort by id, not by name |
| Ref | `eid` (§6.1). Refs sort by eid, not by UUID |

**Long strings.** Ordering is exact among short strings, and between strings whose first
32 bytes differ. Strings that share the first 32 bytes and of which at least one is long
are ordered by marker and hash, not by content. Range predicates (`<`, `>`, prefix) over
such values must re-check the full value. Equality is exact: encode the probe the same
way, seek to `prefix ‖ marker ‖ hash`, and compare the full value of each candidate to
rule out hash collisions.

Comparisons on keywords and refs never relied on the order of names or UUIDs. The query
layer must not use AVET order for keyword ranges; §11 audits this.

### 6.3 Value pages

Long values are stored in append-only value pages (0x51), packed like today's fact
pages. They are never rewritten and never freed, because a retraction stores the same
value again rather than deleting it. They are reachable only through value refs in index
keys, so verify (#373) finds them by walking the indexes.

**Dedup:** before a new long value is written, DICT tag 0x06 is looked up by `hash64`,
and each candidate's full value is compared. A retraction or re-assertion of the same
long value therefore costs no new value page. The 3.0.0 maximum value length is one
value page's payload (4068 B). That is about today's limit, but it now applies to the
value alone rather than the whole fact. `MAX_FACT_BYTES` is replaced by
`MAX_VALUE_BYTES`, a public API change recorded in the CHANGELOG. Values spanning
several pages can come later behind a feature bit.

## 7. Reads

- **Cursor.** `LeafCursor` holds a stack of `(internal page, child index)`. To start,
  it descends to the first key ≥ `start`. To advance at the end of a leaf, it pops up to
  the nearest ancestor with a next child, then descends to that child's leftmost leaf.
  The amortised cost per step is O(1), and memory is O(depth). `range_scan`,
  `stream_all_entries` and the `CommittedIndexReader` methods are rewritten on top of
  it. #432 exposes it later.
- **Seek.** `LeafCursor::seek(key)` moves to the first entry ≥ `key`, never backwards.
  If `key` is in the current leaf, it binary-searches that leaf. Otherwise it pops the
  stack to the lowest ancestor whose key range still covers `key`, and descends from
  there, not from the root. A seek to a nearby key costs O(1) page visits, and one to a
  key d leaves away costs O(log d). A seek to a key below the current position is a
  no-op. Merge joins and leapfrog-style worst-case-optimal joins in the streaming
  engine (#432) call `seek` on each index ordering constantly, so it must stay cheap.
- **Covering reads.** Index entries decode to whole facts. Ids are translated through
  DICT (§5.1). No other page is read except a value page for a long value.
- **Full fact scans** are an EAVT cursor scan.
- **Open** reads two meta pages and nothing else. Pages are verified when first read
  (§4.2). The rebuild-on-checksum-mismatch branch in `load()` is deleted. A corrupt
  page is now an error on the read that hits it. Repairs are explicit through #373:
  `verify`, and `rebuild_indexes` from EAVT or AEVT, each of which holds every fact.

## 8. Checkpoint

Let `M` be the active meta, with generation `g`. The new checkpoint commits generation
`g' = g + 1` into slot `(g' − 1) % 2`.

### 8.1 Invariant

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
never freed. A test pins this down (§11).

### 8.2 Allocation policy

- **Value pages** always append at `page_count`. They are never freed.
- **B+tree and free-list pages** take from `M`'s free list first and append only when
  it is empty. In steady state, tree churn reuses the pages freed one checkpoint
  earlier, and the file grows only by net tree growth and new long values.
- **Layout over time:** page kinds interleave freely. Nothing needs to be contiguous;
  every page is reached from a root in the meta page or from the free-list head.

### 8.3 Steps

1. **Snapshot.** Take `M` and the pending facts. Build an allocator from `M`'s free list
   and `M.page_count`.
2. **Long values.** Write new long values (after dedup, §6.3) into new value pages,
   appended and stamped with `g'`. Their value refs complete the pending keys.
3. **Trees.** For each of the five trees (the four indexes, then DICT with the new
   eid/iid/tx/dedup entries), run a copy-on-write batch insert of the sorted pending
   entries: descend from the root, route entries to leaves by separator keys, merge
   each touched leaf and split it by balanced bytes, then rewrite each touched internal
   node into a new page. A root split adds a level. Every page replaced along the way
   goes into `freed`.
4. **Free list.** Pop is already done by the allocator. Push `freed` and the consumed
   free-list pages as new head pages (§4.4).
5. **Data sync.** All pages above have been written, each stamped with `g'` and its CRC.
   Call `backend.sync()`.
6. **Commit.** Write meta `g'` (with the new roots, `next_eid` and `next_iid`) to slot
   `(g' − 1) % 2`, then call `backend.sync()`. This is the only commit point.
7. **Publish.** Swap the in-memory readers to the new roots and clear the pending facts.
   Then delete the WAL in `do_checkpoint`, as today.

`fact_prefix_crc`, `rebuild_btree_incremental`'s copy-every-leaf path,
`collect_leaf_pages`, `invalidate_from` and the full-file checksum are removed. The page
cache stays write-through (`put_dirty` on every write), so a reused page id never serves
stale content.

### 8.4 Close-time checkpoint

Kept (D10). Under G2 it costs O(change), so #434 acceptance 2 is met through the bound
and reopen stays fast.

## 9. Migration from v7

A legacy file has an 84-byte v7 `FileHeader` at page 0 (version 7, header CRC valid)
and no valid meta page. Only v7 is migrated. A page 0 that is neither a valid meta page
nor a v7 header, including a pre-release v8 single-header file, is rejected with
"no valid meta page".

1. Read the legacy header (header CRC checked) and all facts from pages
   `1..=fact_page_count` (legacy 12-byte fact header).
2. Assign eids and iids in `tx_count` order (§5.1). From `old_page_count` up, append
   value pages for long values, then the five trees built by `build_btree` (bulk build
   from sorted input, stamped and checksummed). Set the free list to old pages
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
free pages that later tree growth reuses. The file does not shrink until a vacuum
exists. That is an accepted one-time cost, listed in the CHANGELOG.

## 10. Estimates

### 10.1 Size at 1B facts

| component | bytes per fact |
|---|---|
| EAVT entry after prefix compression (e, a usually shared with the previous entry) | ~22 |
| AEVT | ~24 |
| AVET | ~26 |
| VAET (≈30 % of facts are refs) | ~8 |
| leaf fill 75 %, internal nodes ~1 % | × 1.35 |
| DICT (2 entries per entity, 1 per tx, amortised) | ~5 |
| **total, excluding long values** | **~110–140 B → 110–140 GB** |

This leaves headroom under the 300 GB budget (G6) for long values and for history. The
benchmark in #394 checks the estimate at each decade.

### 10.2 Write amplification

New entities get the next eid, so their EAVT and AEVT entries land at the right edge (of
the tree, and of each attribute's range). AVET entries and VAET entries pointing at old
targets stay random; this is the remaining source of write amplification. If #394
measures more than 10× at steady state, feature bits (§4.1.2) allow Bε-style buffered
internal nodes in v3.x without a format v9.

## 11. Testing

TDD per component. No new dependencies. Test assert messages follow the CodeQL rule.

**Pages and commit**

- **Page header:** encode/verify round trip; a flipped bit, a wrong `page_id` and a
  future `generation` each give their own STG code; a failed page is not cached.
- **Meta selection:** both valid (highest generation wins). Each row of the §4.1.1
  table: torn newer slot with WAL base `g` (open at `g`, replay); WAL base `> g`
  (error); no WAL with a `g+1` page on `M_g`'s free list or past `M_g.page_count`
  (error, each location tested); no WAL and no `g+1` page (open at `g`); `g == 1` with
  slot B empty. Neither valid: v7 header (migration); torn migration commit with backup
  meta (recovered); pre-release v8 single header or anything else (error).
- **Feature bits:** a file with an unknown `required_features` bit fails with the new
  STG code and is not modified. v3.0.0 writes 0.
- **Allocator and free list:** pop/push across many generations. Invariant check after
  each checkpoint: the reachable set (all five trees, the value pages their keys
  reference, and the free-list chain) and the free ids are disjoint, and together they
  cover `2..page_count` exactly. This also detects leaks.
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
- **Corruption surfacing:** corrupt a leaf, a value page and a free-list page in turn. A
  query or a checkpoint returns the STG code and never wrong data.

**Trees and cursor**

- **Copy-on-write insert equivalence:** random committed sets and pending batches
  (including splits, keys below the first leaf and above the last, root splits).
  `stream_all_entries` of the result must equal the merged sorted set. The old root
  must still stream its old contents unchanged, which proves it was not written to.
- **Prefix compression and restarts:** random keys round-trip through a leaf; lookup
  agrees with a linear scan; separators satisfy `max(left) < s ≤ min(right)`.
- **Cursor:** `range_scan` with random bounds, compared with a filter over the
  expected set, across depth 1–4 trees.
- **Seek:** random monotone seek sequences give the same positions as a fresh descent
  from the root. This covers seeks within a leaf, to a sibling, across subtrees, past
  the end, and backwards (no-op). A page-read counter shows that a seek to the
  neighbouring leaf reads no internal page that the cursor already holds.

**Keys and data model**

- **Encoding:** memcmp order equals logical order on random component tuples for every
  value type: negative, zero and large integers, NaN and ±0.0 floats, strings containing
  0x00, strings at exactly 64 and 65 bytes, FOREVER. Round trip of every component.
- **Long values:** equality lookup finds exactly one match despite a forced hash
  collision (test hook). A range predicate over strings sharing a 32-byte prefix gives
  the same result as a full sort. Dedup: re-asserting and retracting a long value adds
  no value page.
- **Ids:** assignment is deterministic across WAL replay, with a crash between transact
  and checkpoint. eids and iids are never reused. Keyword-as-attribute and
  keyword-as-value share an iid.
- **Covering:** every query result comes from index entries; a counting backend shows
  no value-page reads for facts without long values.
- **Order audit:** grep tests and bindings for assumptions about result order; sort
  results in tests that compare lists.

**Migration, size, benchmarks**

- **Migration:** v7 fixtures give identical query results (as sets) and the same
  `tx_count` floor after migration; crash at every point of the migration; a pre-release
  v8 single-header file is rejected.
- **Size:** generate 1M facts in #433's acceptance shape (~10 attributes per entity,
  20 % multi-valued, 10 % retracted and re-asserted). Bytes per fact must be ≤ 200 at
  1M. #394 extends the measurement to 1B.
- **Benchmark:** `checkpoint/after_1_fact` and `checkpoint/after_100k_facts` at 10k,
  100k and 1M facts, recording bytes written per checkpoint. Write amplification is
  reported for #394, and 1B is measured there.

## 12. Errors, docs, compatibility

- New STG codes: page checksum mismatch, page id mismatch, page generation ahead of
  meta, no valid meta page, meta damaged after commit (§4.1.1), unsupported file
  feature (§4.1.2), free-list/allocator inconsistency. A page that does not start with
  the magic in either meta slot is still `STG-002`. `INT-053` (header CRC mismatch)
  stays in use for a v7 header whose own CRC fails, so it is not deprecated. Codes are
  never recycled. Update `docs/ERROR_REFERENCE.md`.
- WAL format version 2: the header gains `base_generation u64` at bytes 8..16, taken
  from the reserved bytes. A v1 WAL is accepted only next to a v7 file being migrated.
- Public API: `MAX_FACT_BYTES` → `MAX_VALUE_BYTES` (§6.3). Committed scan order changes
  (§5.3).
- Browser: `BrowserBufferBackend` dirty sets become O(change). The IndexedDB flush must
  write the meta page in the same IDB transaction as the data pages. Verify this, and
  fix it if it is not already the case.
- Docs: the `Minigraf::checkpoint` and `wal_checkpoint_threshold` rustdoc drop "copies
  pages in proportion to the total index size". Also update the CLAUDE.md "File Format"
  section, `.wiki/Architecture.md`, CHANGELOG (format, `MAX_VALUE_BYTES`, migration
  file growth, result order) and ROADMAP.
- Philosophy: aligned. Single file, reliability first, no dependencies, and the format
  change goes into the unreleased v8, not a v9.

## 13. Alternatives rejected

- **Parent-held child checksums (ZFS style).** These would catch lost writes, but every
  internal pointer and every value ref would grow by 4 bytes, against the density goal.
  In-page CRC plus page id plus generation catches torn, corrupt and misdirected pages.
- **Single header page.** A torn header write would make the file unopenable.
- **Removing the close-time checkpoint.** It is unnecessary once checkpoints are
  O(change). Removing it would slow reopens and change binding behaviour (#322).
- **Appending to the last partial value page in place.** Under per-page CRCs, a torn
  in-place write would destroy values that were already committed. Each checkpoint
  that writes long values therefore starts a fresh value page. The space lost to
  partly filled pages is bounded by one page per checkpoint that writes long values.
- **Fact pages plus reference indexes.** One random read per query match, and the fact
  stored once more on top of four near-complete index entries (§3.1).
- **Leaves shared across trees.** Not possible: the four sort orders put different
  facts next to each other (§3.1).
- **LSM tree.** High ingest rate, but background compaction typically rewrites data
  10–30×, needs background threads and space management, and adds many moving parts.
- **Fewer indexes** (both rejected for 3.0.0):
  - **Drop AEVT.** Attribute scans would use AVET, which returns entries in value order
    rather than entity order. Joins on `?e` would then need a sort or a hash. It saves
    about 30 % of index bytes.
  - **Opt-in AVET per attribute** (Datomic's `:db/index`). Value lookups on attributes
    without it become full attribute scans. Turning it on per attribute is a schema
    setting, which goes against zero-configuration. If #394 shows that index writes
    dominate, opt-in AVET can come later behind a feature bit (§4.1.2).

## 14. Delivery (PRs into `v3`)

1. **Cursor scans:** add `LeafCursor` with `seek` and move every scan onto it, still on
   the current format. Behaviour does not change.
2. **v8 page format:** common page header with CRC/id/generation, verify on read, meta
   pages A/B with meta selection, feature bits, WAL v2, a full-rewrite free list, and
   v7 migration with the backup meta (page 0 changes here, so migration must too).
   `save()` rebuilds the trees into pages the active meta does not reference, which
   is atomic but O(N). (#388, #374 atomicity)
3. **Covering keys (#433):** byte-comparable key encoding, prefix-compressed nodes, DICT,
   value pages, eid/iid assignment, covering reads; migration moves to the new keys.
4. **Copy-on-write insert, allocator, free list:** O(change) checkpoints. (#434)
5. **Cost and crash test hardening, benchmarks, docs.**

The format freezes for #391 once PRs 2 and 3 have merged. PRs 4 and 5 do not change it.
