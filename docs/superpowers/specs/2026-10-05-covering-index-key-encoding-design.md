# Covering indexes and key encoding for 1B facts (#433) — Design

**Issue:** #433 (v3.0.0 target: confident operation at 1B facts), format decisions only
**Milestone:** v3.0.0, branch `v3`, file format v8 (unreleased)
**Companion:** `2026-10-03-cow-btree-checkpoint-design.md` (page format, meta, checkpoint).
This spec defines what is stored in the trees. The companion defines how the trees are
paged, checksummed and committed.
**Must land before:** #391 golden-file corpus freezes v8

## 1. Problem

A fact costs about 520 B on disk at best today, and about 2 KB as measured. The fact
record is ~70 B of postcard data. Each of the four index entries is ~70–90 B, and each
one repeats a 16-byte UUID, the attribute string and 8-byte timestamps. #433 allows at
most 300 GB for 1B facts, which is 300 B per fact including history.

Two further costs grow with N:

- **Indexes are not covering.** `range_scan` returns `FactRef`s, so every match costs a
  read of a fact page. A streaming scan that matches 1M facts means 1M random page reads.
- **Keys land on random leaves.** Entity UUIDs derived from keywords (v5) are effectively
  random, and so are most values. A checkpoint of k facts touches about k different
  leaves in every index. At 1B facts that is ~40× write amplification, against #434's
  limit of 10.

## 2. Decisions

| # | decision | alternative rejected |
|---|---|---|
| D1 | **Covering indexes.** Every index entry holds the whole fact. Fact pages, `FactRef` and the fact directory are removed. | Fact pages plus reference indexes: one random read per match |
| D2 | **Sequential internal entity ids** (`eid: u64`), with a dictionary mapping UUID ↔ eid. The API still speaks UUIDs. | 16-byte UUIDs in keys: random leaf placement, larger keys |
| D3 | **Ident dictionary** (`iid: u32`) for attribute names and keyword values. | Strings in every key |
| D4 | **Out-of-line long values.** Values above 64 B are stored once (deduplicated) in value pages. Keys hold a 32-byte prefix, an 8-byte hash and a value ref. | Always inline: a 4 KB string copied into 3 indexes, leaf fanout of 1 |
| D5 | **Byte-comparable keys** (memcmp order). Integers use order-preserving variable-length encoding. Leaves are prefix-compressed. Separators are truncated to their shortest form. | postcard structs compared after decoding: no prefix compression, decode on every compare |
| D6 | **Datomic component order, newest transaction first:** `(e,a,v)` triples are contiguous, followed by `tx` descending. | Today's `(e,a,vf,vt,tx,v)`: history of one triple is scattered |

### 2.1 Covering does not multiply writes

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

**Fewer copies means fewer indexes.** Both options were rejected for 3.0.0:

- **Drop AEVT.** Attribute scans would use AVET, which returns entries in value order
  rather than entity order. Joins on `?e` would then need a sort or a hash. It saves
  about 30 % of index bytes.
- **Opt-in AVET per attribute** (Datomic's `:db/index`). Value lookups on attributes
  without it become full attribute scans. Turning it on per attribute is a schema
  setting, which goes against zero-configuration.

minigraf is schema-less, so every query pattern keeps an index. If #394 shows that
index writes dominate, opt-in AVET can come later behind a feature bit (companion §3.1.2)
without a format v9.

## 3. Data model in the file

### 3.1 Identifiers

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

### 3.2 Trees

Five B+trees, all with the same node format (§5) and all copy-on-write:

| tree | key | entry value | holds |
|---|---|---|---|
| EAVT | `e a v tx↓ vf vt op` | — | every fact |
| AEVT | `a e v tx↓ vf vt op` | — | every fact |
| AVET | `a v e tx↓ vf vt op` | — | every fact |
| VAET | `v a e tx↓ vf vt op` | — | facts whose value is a `Ref` (`v` = target eid) |
| DICT | tag-prefixed, below | per tag | dictionaries and the tx table |

EAVT and AEVT each hold the complete fact set. Either one can rebuild every other index
(#373 `rebuild_indexes`), so a single corrupt index never loses data.

DICT entries:

| tag | key | value | used for |
|---|---|---|---|
| 0x01 | `uuid[16]` | `eid` | UUID → eid (transact, query constants) |
| 0x02 | `eid` | `uuid[16]` | eid → UUID (results) |
| 0x03 | ident bytes (escaped) | `iid` | name → iid |
| 0x04 | `iid` | ident bytes | iid → name |
| 0x05 | `tx_count` | `tx_id` (wall-clock ms) | `Fact.tx_id`, once per transaction |
| 0x06 | `hash64` of a long value | value ref | dedup of long values (§4.3) |

Tags 0x02, 0x04 and 0x05 receive increasing keys, so their inserts touch only the
rightmost path. Tags 0x01 and 0x06 are random-key inserts, but they happen once per new
entity or new long value, not once per fact.

### 3.3 Component order (D6)

- `(e, a, v)` triples are contiguous in EAVT, and the history of a triple follows its
  key, newest first. "Latest assertion per triple" (#435) and "live facts only" (#379)
  become streaming operations: read the first entry per triple and skip the rest with
  `seek` (companion spec §5).
- `:as-of N` seeks to `(e, a, v, ↓N)` and lands directly on the newest assertion at or
  before N, in O(log) per triple instead of a scan of its history.
- Valid time (`vf`, `vt`) and `op` (assert or retract) follow `tx`. They are filtered
  per entry, as today.
- This changes the order of committed scan results compared with v7: eid order instead
  of UUID order, iid order instead of attribute-name order. Query results are sets, but
  every test or binding that relies on a particular order must be found and fixed
  (§8).

## 4. Key encoding (D5)

All components are encoded so that comparing the bytes with memcmp gives the logical
order. Encoded keys are compared without decoding.

### 4.1 Integers

Integers use the FoundationDB tuple-layer encoding. A type byte holds the sign and the
byte length, followed by the minimal big-endian magnitude (one's complement for
negatives).

- 0 takes 1 byte. Values up to 255 take 2 bytes. A ms timestamp (~2^41) takes 7 bytes.
- `eid`, `iid` and `tx_count` use the unsigned form.
- `vt == VALID_TIME_FOREVER`, the common case, is the single byte `0xFF`. It sorts after
  every finite time.
- `tx↓` is the encoding of `u64::MAX - tx_count`, so newer transactions sort first.

### 4.2 Values

A type tag is followed by the payload. The tag order is unchanged from v7, so
cross-type ordering stays Null < Boolean < Integer < Float < String < Keyword < Ref.

| type | payload |
|---|---|
| Null | — |
| Boolean | 1 byte |
| Integer | §4.1 signed |
| Float | 8 bytes, the existing order-preserving bit transform, NaN canonicalised |
| String ≤ 64 B | bytes with 0x00 escaped as `00 FF`, terminated by `00 00` |
| String > 64 B | first 32 bytes escaped, then marker `00 01`, `hash64`, value ref (`page u64` + `slot u16`) |
| Keyword | `iid` (§4.1). Keywords sort by id, not by name |
| Ref | `eid` (§4.1). Refs sort by eid, not by UUID |

**Long strings.** Ordering is exact among short strings, and between strings whose first
32 bytes differ. Strings that share the first 32 bytes and of which at least one is long
are ordered by marker and hash, not by content. Range predicates (`<`, `>`, prefix) over
such values must re-check the full value. Equality is exact: encode the probe the same
way, seek to `prefix ‖ marker ‖ hash`, and compare the full value of each candidate to
rule out hash collisions.

Comparisons on keywords and refs never relied on the order of names or UUIDs. The query
layer must not use AVET order for keyword ranges; §8 audits this.

### 4.3 Value pages

Long values are stored in append-only value pages (page type 0x51 in the companion spec),
packed like today's fact pages. They are never rewritten and never freed, because a
retraction stores the same value again rather than deleting it.

**Dedup:** before a new long value is written, DICT tag 0x06 is looked up by `hash64`,
and each candidate's full value is compared. A retraction or re-assertion of the same
long value therefore costs no new value page. The 3.0.0 maximum value length is one
value page's payload (4068 B). That is about today's limit, but it now applies to the
value alone rather than the whole fact. `MAX_FACT_BYTES` is replaced by
`MAX_VALUE_BYTES`, a public API change recorded in the CHANGELOG. Values spanning
several pages can come later behind a feature bit.

## 5. Node format

Every B+tree page (0x61 leaf, 0x62 internal) follows the companion spec's 24-byte header.

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

**Splits** are balanced by bytes, as in #315. Inserts never shrink a node, because
nothing is removed from an index.

## 6. Size estimate (1B facts)

| component | bytes per fact |
|---|---|
| EAVT entry after prefix compression (e, a usually shared with the previous entry) | ~22 |
| AEVT | ~24 |
| AVET | ~26 |
| VAET (≈30 % of facts are refs) | ~8 |
| leaf fill 75 %, internal nodes ~1 % | × 1.35 |
| DICT (2 entries per entity, 1 per tx, amortised) | ~5 |
| **total, excluding long values** | **~110–140 B → 110–140 GB** |

This leaves headroom under the 300 GB budget for long values and for history. The
benchmark in #394 checks the estimate at each decade.

**Write amplification.** New entities get the next eid, so their EAVT and AEVT entries
land at the right edge (of the tree, and of each attribute's range). AVET entries and
VAET entries pointing at old targets stay random; this is the remaining source of write
amplification. If #394 measures more than 10× at steady state, the companion spec's
feature bits allow Bε-style buffered internal nodes in v3.x without a format v9.

## 7. Effect on the companion spec

- No fact pages, `FactRef`s or fact directory. Value pages (0x51) take the role of fact
  pages for long values only. DICT replaces the fact-directory root in the meta page.
- The meta page gains `next_eid` and `next_iid`, and drops `fact_page_format`.
- A full fact scan is an EAVT cursor scan.
- The meta-selection evidence (companion §3.1.1) looks for a page stamped `g+1` anywhere
  generation `g+1` could have written: `M_g`'s free-list pages, or at or above
  `M_g.page_count`. The rule that fact pages are allocated first is dropped.
- v7 migration reads the facts, assigns eids and iids in `tx_count` order, writes the
  value pages, and bulk-builds the five trees.

## 8. Testing

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
- **Prefix compression and restarts:** random keys round-trip through a leaf; lookup
  agrees with a linear scan; separators satisfy `max(left) < s ≤ min(right)`.
- **Size:** generate 1M facts in #433's acceptance shape (~10 attributes per entity,
  20 % multi-valued, 10 % retracted and re-asserted). Bytes per fact must be ≤ 200 at
  1M. #394 extends the measurement to 1B.
- **Migration:** v7 fixtures give identical query results (as sets) after migration.

## 9. Delivery

This spec and the companion ship in the same PR sequence (companion §10). The key
encoding, DICT, value pages and covering indexes form one PR after the page-format PR.
The format freezes when both have merged.
