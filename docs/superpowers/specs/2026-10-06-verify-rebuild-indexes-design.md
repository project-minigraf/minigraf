# Integrity check and index rebuild — Design

**Issue:** #373 (split out of #370), tracker #383
**Milestone:** v3.0.0, branch `v3`, file format v8 (no format change)
**Builds on:** `2026-10-05-v8-storage-format-design.md` §4.2, §5.2, §7, §8, §11

## 1. Problem

#373 was written against v7: fact pages were the source of truth and four reference
indexes pointed into them. An index that disagreed with the fact pages was silent.
Entity-bound queries returned `[]` and the file checksum still passed (#370).

v8 removes fact pages. Every index entry holds the whole fact (D4), and EAVT, AEVT and
AVET each hold the complete fact set. Every page carries a CRC, its own id and its
generation, and every read checks them (G3). A torn or rotted page is therefore an error
on the read that hits it, never a wrong answer.

Two gaps remain:

1. **Logical damage is still silent.** A bug like #370's, or a lost write that leaves an
   older valid page in place, gives pages that pass their CRC but say the wrong thing:
   an index missing entries, two indexes that disagree, a leaked or doubly referenced
   page, a key whose ids have no DICT entry. Only a full cross-check finds these.
2. **There is no repair.** The spec promises `rebuild_indexes` from EAVT or AEVT
   (§5.2, §7). A checkpoint cannot repair: it inserts copy-on-write into the existing
   trees and fails on a damaged leaf.

## 2. API

```rust
impl Minigraf {
    /// Check the committed file. O(file).
    pub fn verify(&self) -> Result<IntegrityReport, MinigrafError>;
    /// Rebuild EAVT, AEVT, AVET and VAET from an intact index, then checkpoint.
    pub fn rebuild_indexes(&self) -> Result<(), MinigrafError>;
}

#[non_exhaustive]
pub struct IntegrityReport {
    pub facts: u64,              // EAVT entries
    pub pages: u64,              // pages checked
    pub problems: Vec<MinigrafError>,
}
impl IntegrityReport { pub fn is_ok(&self) -> bool; }
```

- `verify` returns `Err` only when it cannot run (lock poisoned, I/O error reading the
  meta). Damage goes into `problems`, one structured error per finding, so a caller can
  match on codes. It keeps going after a finding where it can.
- It checks the committed file only. Pending facts live in memory and in the WAL, whose
  entries carry their own CRC.
- In-memory databases have no committed pages: the report is empty and `is_ok()`.
- Both take the write lock, so the active meta and its pages cannot change underneath
  them. Readers keep running.
- No Datalog or REPL syntax. Datalog stays the query language; SQLite's equivalent is a
  pragma, not SQL. Bindings expose the two methods later (Python is Tier 1).
- No on-open sampling. v8 open is O(1) by design (G4), and per-page checks already turn
  physical damage into errors. Sampling a few entries cannot find a missing one.

## 3. Checks (`verify`)

All against the active meta `M`.

| # | check | code |
|---|---|---|
| C1 | Every node of the five trees verifies (CRC, id, generation, type), decodes, and is reached once across all trees | STG-029/030/031/013, STG-039 |
| C2 | Keys strictly increase across each tree (cursor walk) | STG-039 |
| C3 | Every index key decodes; VAET values are refs | STG-039 |
| C4 | EAVT count = `M.fact_count` | STG-038 |
| C5 | AEVT and AVET hold the same facts as EAVT; VAET holds exactly EAVT's ref facts | STG-038 |
| C6 | Every `tx_count` ≤ `M.last_checkpointed_tx_count` | STG-038 |
| C7 | Every eid (entity or ref) has 0x02, every iid (attribute or keyword) 0x04, every `tx_count` 0x05, every long value ref a 0x06 entry | STG-036 |
| C8 | 0x01 ↔ 0x02 and 0x03 ↔ 0x04 are inverse; ids below `next_eid` / `next_iid` | STG-040 |
| C9 | Each long value reads, and its hash and prefix match the key | STG-038 |
| C10 | Free list reads; ids distinct; count = `M.freelist_count`; free ∩ reachable = ∅; free ∪ reachable = `2..M.page_count` | STG-035 |

**C5 without O(N) memory.** Each entry is mapped to its canonical EAVT key bytes and
hashed. Per index the check keeps `(count, Σ h₁, ⊕ h₂)` with two independent 64-bit
hashes. Equal digests mean equal multisets except with probability ≈ 2⁻⁶⁴ per pair. This
is a corruption check, not an adversarial one. The digest also tells which index is the
odd one out (§4).

Memory: the reachable page set (8 B per page) and the DICT id sets (one entry per entity,
ident and transaction). No per-fact state.

## 4. Rebuild (`rebuild_indexes`)

**Choosing the source.** Walk EAVT, AEVT and AVET (C1–C3 and the C5 digest). An index
is *clean* if its walk finished without error.

1. If two clean indexes have the same digest, the first of them (EAVT, AEVT, AVET order)
   is the source.
2. Otherwise, if exactly one is clean, it is the source: it is the only copy left.
3. Otherwise fail with STG-041 and change nothing.

DICT is not rebuilt: UUIDs and ident names exist only there. A damaged DICT is not
repairable by this operation (STG-036 / STG-041); restore from backup.

**Writing.** Like `save()` (§8.3), with bulk builds instead of copy-on-write inserts:

1. Encode pending facts on top of `M`'s dictionary (new ids, new long values), as `save()`.
2. Stream the source, re-encode each entry as a `KeyFact`, and produce keys for all four
   indexes. Merge the pending index entries. Sort each list and `build_btree` it.
3. `cow_insert` the new DICT entries into `M`'s DICT.
4. New free list: `2..next_append` minus everything the new meta reaches (the four new
   trees, DICT nodes, value pages through DICT 0x06). This re-derives the list from
   scratch, so a leaked or wrong free list is repaired too. Chain pages are appended.
5. Sync, write meta `g+1` to the inactive slot, sync. Delete the WAL as a checkpoint does.

**Crash safety (§8.1 invariant).** Pages for steps 1–3 come from `M`'s free list only if
it reads cleanly and is disjoint from every page that the DICT tree, the value pages and
the source tree use; otherwise every page is appended. Either way nothing the old meta
needs to reopen is overwritten before the commit. A crash reopens at `g` plus the WAL.

**Cost.** O(N) time. Memory O(N) for the four key lists, as the v7 migration already
does. An external sort is not worth the code for a repair operation. The file can grow by
one copy of the indexes; those pages return to the free list at commit and are reused.

## 5. Errors

New codes, never recycled; `docs/ERROR_REFERENCE.md` updated:

- **STG-038** `Index {} disagrees with the committed facts: {}`
- **STG-039** `Tree {} is malformed: {}`
- **STG-040** `Dictionary is inconsistent: {}`
- **STG-041** `Cannot rebuild indexes: {}`

STG-035 (free list) and STG-036 (dictionary entry missing) gain the verify scenarios.

## 6. Testing

- Clean files of mixed facts (strings, long strings, refs, keywords, retractions,
  several checkpoints) verify with no problems; `facts` and `pages` are exact.
- Each check fires on a targeted damage, built with page-level helpers: a leaf whose
  entries are removed and resealed (C4/C5 for that index only), a dropped VAET entry,
  swapped keys (C2), a node linked from two trees (C1), a leaked page and a free id that
  is still referenced (C10), a deleted DICT entry (C7), a flipped bit (C1 / STG-029).
- Rebuild: after each damage above that leaves a source, `rebuild_indexes` succeeds,
  `verify` is clean, and queries return the model's facts. Pending facts and WAL entries
  made before the rebuild survive it and the WAL is gone.
- Source choice: EAVT damaged → AEVT; EAVT and AEVT disagree with AVET agreeing with one;
  all three disagree → STG-041 with the file unchanged.
- Crash at every write and sync of a rebuild (FaultInjectingBackend): reopen gives
  either the old or the rebuilt state, never a loss.
- Space accounting (`assert_space_accounted`) holds after every rebuild.

## 7. Philosophy

Aligned: reliability over features, single file, no format change, no dependency, no
configuration. Two methods, like SQLite's `integrity_check` and `REINDEX`.

## 8. Out of scope

- Recovering at generation `g` after STG-033 (§4.1.1): an open option, separate issue.
- Rebuilding DICT, vacuum, salvage of individual pages.
- Browser (`BrowserDb`) and binding exposure: follow-ups.
