# v8 Storage Format Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the v8 storage format: crash-atomic copy-on-write checkpoints whose cost depends only on the change, per-page checksums, and covering indexes with dictionary-coded, byte-comparable keys. This closes #374, #434, #388 and #433's format decisions.

**Spec:** `docs/superpowers/specs/2026-10-05-v8-storage-format-design.md` (§ numbers below refer to it).

**Tech Stack:** Rust (MSRV 1.89), postcard (until PR 3), crc32fast, criterion. No new dependencies.

**Delivery:** five PRs into `v3` (spec §14). PR 1 and PR 2 are detailed task by task. PRs 3–5 are listed at task level. Each gets its own detailed plan section, written when the previous PR has merged, because each builds on the code shape the previous one leaves.

## Global Constraints

- Every PR targets `v3` and must pass full CI. Never merge without the user's explicit confirmation.
- Test assert messages never format `Result`, `Fact`, `Value`, `EdnValue`, index keys, `FactRef` or anything containing a `Uuid` with `{:?}` (CodeQL `rust/cleartext-logging`). Use plain string messages, `unwrap`/`expect`, or assertions on counts.
- Match the surrounding clippy style. Functions in `btree_v6.rs` that index or do arithmetic carry `#[allow(clippy::arithmetic_side_effects)]` / `#[allow(clippy::indexing_slicing)]`. Errors use `err_coded!`/`bail_coded!`. B+tree invariant violations use `ErrorCode::Int049`. A page of the wrong type reached during a scan is `ErrorCode::Stg013`.
- Error codes are never removed or recycled. New codes are registered in `src/error.rs` and in `docs/ERROR_REFERENCE.md`.
- `cargo test --release` does not work (panic=abort profile). Use `cargo test` and `cargo bench`.
- Before each commit, run `cargo fmt` and `cargo clippy --all-targets -- -D warnings`. CI's `stable` clippy may be newer than the local one.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_015GgLEs8YozJjYhwPEYgTUn
  ```
- PR descriptions end with:
  ```
  🤖 Generated with [Claude Code](https://claude.com/claude-code)

  https://claude.ai/code/session_015GgLEs8YozJjYhwPEYgTUn
  ```
- Issue references: PRs 1–3 use `Refs #374 #434 #388 #433`. Only the PR that completes an issue may say `Closes`.

---

# PR 1 — Cursor scans (no format change)

**Goal:** All B+tree reads go through a `LeafCursor` that descends with a parent stack and supports a forward `seek`. No read path follows `next_leaf` any more. Writers still set it, and the on-disk format is unchanged. This prepares for spec §4.3, where `next_leaf` is gone.

**Architecture:** `LeafCursor<K>` (new, in `btree_v6.rs`) keeps a stack of internal frames. Each frame holds the node's decoded separators, its child ids, the index of the child currently taken, and the node's own upper bound, inherited from its parent's separator. The current leaf's decoded entries and position are held alongside. `range_scan`, `stream_all_entries` and `collect_leaf_pages` are rewritten on top of it. `find_leaf_for_key` and `find_leftmost_leaf` become cursor internals.

## Review Focus

1. **Seek that crosses subtrees.** Seeking from the last leaf of one subtree into a key several subtrees to the right must pop exactly as far as needed, then descend. Expect: the same position as a fresh descent from the root. Pinned by `cursor_seek_matches_fresh_descent_random` (Task 3).
2. **Key in the gap between a leaf's last entry and its upper bound.** Expect: the cursor parks at the end of that leaf and `next()` moves to the next leaf's first entry. Pinned by `cursor_seek_into_gap_after_leaf_end` (Task 3).
3. **Backward seek.** Expect: a no-op that keeps the position. Pinned by `cursor_seek_backwards_is_noop` (Task 3).
4. **Corrupt child pointer.** A child id pointing at a non-B+tree page, or a cycle back up the tree. Expect: STG-013 for a wrong page type, and Int049 on exceeding the maximum depth, never a hang. Pinned by `cursor_child_pointing_at_fact_page_returns_stg_013` and `cursor_cycle_in_child_pointers_is_bounded` (Task 4).
5. **Seek cost.** A seek to the neighbouring leaf must not re-read internal pages the cursor already holds. Pinned by `cursor_seek_to_next_leaf_reads_one_page` (Task 3), using a page-read counting backend.

---

### Task 1: Page-read counting test backend

**Files:**
- Modify: `src/storage/btree_v6.rs` (test module, new helper near the top of `mod tests`)

**Interfaces:**
- Produces: `CountingBackend { inner: MemoryBackend, reads: AtomicU64 }`, implementing `StorageBackend`, which counts `read_page` calls. Used with a page cache of capacity 0 so every page access reaches the backend.

- [ ] **Step 1:** Add `CountingBackend` to the test module. It delegates every method to `inner`, and `read_page` increments `reads` (`Ordering::Relaxed`). Add `fn reads(&self) -> u64` and `fn reset(&self)`.
- [ ] **Step 2:** Check that `PageCache::new(0)` passes reads straight through (look at `cache.rs`). If capacity 0 still caches, use a cache of capacity 1 and account for that in the expected counts.
- [ ] **Step 3:** `cargo test -p minigraf storage::btree_v6` passes (no behaviour change yet). Commit: `test(btree): page-read counting backend for cursor tests`.

### Task 2: `LeafCursor` with descend and `next`

**Files:**
- Modify: `src/storage/btree_v6.rs` (new section `// ─── LeafCursor ───` after the leaf traversal helpers)

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct LeafCursor<'a, K> { /* backend, cache, stack, leaf state */ }
  impl<'a, K> LeafCursor<'a, K>
  where K: for<'de> Deserialize<'de> + Ord + Clone
  {
      /// Positions before the first entry ≥ `start` (`None` = leftmost).
      pub(crate) fn new(root: u64, start: Option<&K>, backend: &'a dyn StorageBackend, cache: &'a PageCache) -> Result<Self>;
      /// Next entry in key order, or `None` at the end of the tree.
      pub(crate) fn next(&mut self) -> Result<Option<(K, FactRef)>>;
      /// Moves forward to the first entry ≥ `key`; a key at or before the position is a no-op.
      pub(crate) fn seek(&mut self, key: &K) -> Result<()>;
      /// Raw page of the current leaf, for `collect_leaf_pages` (None before the first leaf / at end).
      pub(crate) fn current_leaf_page(&self) -> Option<&Arc<Vec<u8>>>;
      /// Moves to the start of the next leaf; false at the end of the tree.
      pub(crate) fn advance_leaf(&mut self) -> Result<bool>;
  }
  ```
- Internal: `struct Frame<K> { page_id: u64, children: Vec<u64>, seps: Vec<K>, child_idx: usize, upper: Option<K> }`. `children.len() == seps.len() + 1`, and the last child is `rightmost_child`. `seps[j]` is the first key of `children[j+1]`, as in `write_internal_page`.
- Internal: a `decode_internal(page) -> (Vec<u64>, Vec<K>)` helper that reuses `read_u16_at`/`read_u64_at` and the slot-directory layout. `find_leaf_for_key` already decodes the same way, so extract the shared logic instead of copying it.
- Constant: `const MAX_TREE_DEPTH: usize = 32;`. Descending deeper than this is `Int049` ("tree deeper than MAX_TREE_DEPTH: cycle or corruption").

- [ ] **Step 1: Write failing tests** in `mod tests`:
  - `cursor_full_scan_matches_stream` builds trees of 0, 1, 10, 500 and 5,000 entries with `build_btree`. `LeafCursor::new(root, None, ..)` drained with `next()` must equal the sorted input. The 5,000 case must have depth ≥ 2: assert that the root page type is `PAGE_TYPE_INTERNAL`.
  - `cursor_start_key_positions_at_first_ge` covers start keys below the minimum, equal to an entry, between entries, at a leaf boundary (the first key of leaf 2) and above the maximum.
- [ ] **Step 2:** Run them and watch them fail (`LeafCursor` doesn't exist).
- [ ] **Step 3: Implement.**
  - Descent from a frame: pick child `c` = first `i` with `key < seps[i]`, else `seps.len()` (rightmost). Push a frame with `upper = if c < seps.len() { Some(seps[c].clone()) } else { parent.upper.clone() }`.
  - At a leaf, decode its entries (`read_leaf_entries`) and set `pos` = `partition_point(|(k,_)| k < key)`.
  - Every page reached by descent must be `PAGE_TYPE_LEAF` or `PAGE_TYPE_INTERNAL`, otherwise `Stg013`. Enforce `MAX_TREE_DEPTH`.
  - `advance_leaf` pops frames while `child_idx == seps.len()`. At the first frame with a next child it increments `child_idx`, then descends leftmost, computing upper bounds the same way.
  - `next` returns the entry at `pos` and increments it. At the end of a leaf it calls `advance_leaf` and loops, because empty leaves can exist (an empty tree is a single empty leaf).
- [ ] **Step 4:** Tests pass. `cargo clippy --all-targets -- -D warnings`. Commit: `feat(btree): LeafCursor with parent-stack descent (#374 prep)`.

### Task 3: `seek`

**Files:**
- Modify: `src/storage/btree_v6.rs`

- [ ] **Step 1: Write failing tests:**
  - `cursor_seek_matches_fresh_descent_random`: a seeded LCG (no new dependency; copy the generator pattern from `test_incremental_matches_merge_random`) builds a 20,000-entry tree. Generate 2,000 ascending seek targets, including duplicates and keys that are not in the tree. After each `seek(t)`, `next()` must equal what `LeafCursor::new(root, Some(&t), ..).next()` gives.
  - `cursor_seek_within_leaf`, `cursor_seek_to_sibling_leaf`, `cursor_seek_across_subtrees` and `cursor_seek_past_end` (then `next() == None`).
  - `cursor_seek_into_gap_after_leaf_end`: seek to a key greater than leaf L's last entry and smaller than leaf L+1's first entry. `next()` must return L+1's first entry.
  - `cursor_seek_backwards_is_noop`: after `seek(k2)`, `seek(k1)` with `k1 < k2` leaves `next()` unchanged.
  - `cursor_seek_to_next_leaf_reads_one_page`: using `CountingBackend` with a cache of capacity 0 and a depth ≥ 2 tree. Position at leaf L, reset the counter, then seek to the first key of L+1. Expected reads: exactly 1 (the new leaf), because the parent frame already holds the separators and child ids. A seek into a different parent reads 2 pages (the parent and the leaf).
- [ ] **Step 2:** Run them and watch them fail.
- [ ] **Step 3: Implement `seek(key)`:**
  1. If `key <= ` the entry at `pos` (or the last returned key), return: no-op.
  2. If a leaf is loaded and `upper` of the leaf's frame is `None` or `key < upper`, the key is in this leaf: `pos = max(pos, partition_point(< key))`. Return.
  3. Otherwise pop frames until the top frame's `upper` is `None` or `key < upper`. At that frame, set `child_idx` to the first `i ≥ child_idx` with `key < seps[i]` (else rightmost), then descend as in Task 2.
  4. Track the leaf's own upper bound in the cursor: it is the upper bound computed when the leaf was entered.
- [ ] **Step 4:** Tests pass. Commit: `feat(btree): LeafCursor::seek from the current position`.

### Task 4: Move `range_scan`, `stream_all_entries` and `collect_leaf_pages` onto the cursor

**Files:**
- Modify: `src/storage/btree_v6.rs`: `range_scan` (~line 897), `stream_all_entries` (~line 849, test-only), `collect_leaf_pages` (~line 459), `find_leftmost_leaf`/`find_leaf_for_key` (delete if unused, or make them private wrappers over the cursor).
- Modify: tests that corrupt `next_leaf` (~lines 1219 and 1794–1850).

- [ ] **Step 1: Update tests first.**
  - `range_scan_corrupted_next_leaf_pointer_returns_stg_013` is replaced by `cursor_child_pointing_at_fact_page_returns_stg_013`. It overwrites a child id in the root internal node with a packed fact page's id, then calls `range_scan` with a start key routed to that child. It expects STG-013 (`MinigrafError::from(err).code() == "STG-013"`).
  - Add `range_scan_ignores_next_leaf`: corrupt every leaf's `next_leaf` to 0. `range_scan` and `collect_leaf_pages` still return everything. This proves no read path follows `next_leaf`.
  - `test_collect_leaf_pages_rejects_cyclic_chain` becomes `cursor_cycle_in_child_pointers_is_bounded`: point a child id of an internal node back at the root. Every scan returns Int049 and does not hang.
  - Keep `test_collect_leaf_pages_rejects_corrupt_slot_directory`. `collect_leaf_pages` still calls `validate_leaf_slots` on each leaf.
- [ ] **Step 2:** Run the new tests and watch them fail. Old behaviour follows `next_leaf`.
- [ ] **Step 3: Implement.**
  - `range_scan`: `let mut c = LeafCursor::new(root, Some(start), ..)?; while let Some((k, fr)) = c.next()? { if end.is_some_and(|e| k >= *e) { break } out.push(fr) }`.
  - `stream_all_entries`: drain `LeafCursor::new(root, None, ..)`.
  - `collect_leaf_pages`: iterate leaves with `advance_leaf`, calling `validate_leaf_slots` and pushing `current_leaf_page()`. Drop the `max_leaves` chain guard, which `MAX_TREE_DEPTH` and the tree structure now replace. Keep the doc comment's reason for snapshotting. `rebuild_btree_incremental` still writes `next_leaf` (format unchanged).
- [ ] **Step 4:** `cargo test` (full suite) passes. Commit: `refactor(btree): every read path uses LeafCursor; next_leaf is write-only`.

### Task 5: Benchmark guard and docs

**Files:**
- Modify: `docs/TEST_COVERAGE.md` (btree_v6 test counts), `CLAUDE.md` (test count line), `CHANGELOG.md` (`[Unreleased]` / v3.0.0 section: internal, "B+tree scans no longer follow leaf sibling links (prep for #374)").

- [ ] **Step 1:** `cargo bench --bench minigraf_bench -- 'query/|checkpoint/'` on `v3` (before) and on the branch (after). Record both in the PR description. A regression above 10 % on any `query/` group blocks the PR: investigate separator decoding per frame first.
- [ ] **Step 2:** Update the test counts from `cargo test 2>&1 | grep "test result"`, and update the docs above.
- [ ] **Step 3:** Commit `docs: test counts and changelog for cursor scans`. Push the branch and open a PR into `v3` titled `refactor(btree): cursor-based scans with seek (v8 PR 1, refs #374)`. Include this plan's Review Focus list in the description. Monitor CI until green and fix every failure. Ask the user before merging.

---

# PR 2 — v8 page format

Spec §4.1, §4.2, §4.4 (full-rewrite form), §9, §12. Branch `feat/v8-pr2-page-format`.

**Outcome:** every page carries a verified header, two alternating meta pages are the
only commit point, and a crash at any write, sync or torn write loses no checkpointed
fact and needs no index rebuild on open. `save()` still rewrites all four trees, so it
stays O(N). PR 4 makes it O(change).

**Shape decisions (from reading the v3 code):**

- **Fact pages stay until PR 3** with the interim type 0x41 and the 24-byte header.
  They always append at `page_count` and are never freed, as value pages will be
  (§8.2). Page id order is then insertion order, which keeps `stream_all` ordering.
- **`stream_all`** collects the distinct fact page ids referenced by EAVT, sorts them,
  and reads each page. No fact-page directory and no contiguity assumption.
- **Trees** are rebuilt each save with the existing `rebuild_btree_incremental`. It now
  takes a page allocator instead of a start id, and it restamps copied leaves with the
  new id, generation and CRC. Tree and free-list pages come from `M`'s free list first,
  then append.
- **Free list:** each save frees every page of `M`'s four trees and `M`'s free-list
  pages, and writes the whole new list as a fresh 0x81 chain. PR 4 replaces this with
  O(delta) push/pop. The on-disk format is already final.
- **FileBackend becomes format-agnostic.** Today it parses page 0 as a v7 header and
  rewrites it on every append, which is the #308 hazard. Its page count comes from the
  file length, rounded down so that a torn trailing append is invisible. Page 0 is just
  a page.
- **New file:** `PersistentFactStorage` writes an empty generation-1 meta to page 0 and
  syncs, before anything else.
- **v7 migration lands here**, because page 0 changes here. It follows §9 with
  postcard keys and 0x41 fact pages. PR 3 retargets it to covering keys.
- **Page cache** verifies on every miss against a generation bound held in an
  `AtomicU64`. Meta pages, legacy v7 pages and the evidence search read the backend
  directly and never go through the cache. Every page write updates the cache
  (`put_dirty`), so a reused page id never serves stale bytes.
- **Error codes (new):** STG-029 page checksum mismatch, STG-030 page id mismatch,
  STG-031 page generation ahead of meta, STG-032 no valid meta page, STG-033 meta damaged
  after commit, STG-034 unsupported file feature, STG-035 free-list inconsistency. An
  unknown page type is the existing STG-013. INT-053 (header CRC) stays in use for a
  damaged v7 header, and a file with no magic in page 0 is still STG-002. Check the
  next free numbers in `src/error.rs` before assigning.

## Review Focus

- Every page write in `save()` and migration targets a page `M` does not reference.
  Task 9 asserts this in tests.
- Meta selection, row by row against the §4.1.1 table.
- No path silently opens an older generation when the WAL is gone.

### Task 1: `src/storage/page.rs` — common header

- [ ] Constants: `PAGE_HEADER_SIZE = 24`, `PAGE_TYPE_FACT_INTERIM = 0x41`,
  `PAGE_TYPE_VALUE = 0x51`, `PAGE_TYPE_VALUE_OVERFLOW = 0x52`, `PAGE_TYPE_LEAF = 0x61`,
  `PAGE_TYPE_INTERNAL = 0x62`, `PAGE_TYPE_FREELIST = 0x81`, plus a `RETIRED_PAGE_TYPES`
  doc table (0x02, 0x03, 0x11, 0x21, 0x22).
- [ ] `fn seal(page: &mut [u8], page_id: u64, generation: u64)` writes the id and
  generation, zeroes the CRC field, computes CRC32 over the page and stores it.
- [ ] `fn verify(page: &[u8], expected_id: u64, max_generation: u64) -> Result<u8>` runs
  checks 1–4 from §4.2 in order and returns the page type.
- [ ] Register the STG codes in `src/error.rs` and `docs/ERROR_REFERENCE.md`.
- [ ] Tests: seal/verify round trip; flipped body bit → STG-029; wrong id → STG-030;
  future generation → STG-031; legacy type byte 0x21 → STG-013 before the CRC check
  (a page with a wrong CRC and type 0x21 still gives STG-013).

### Task 2: Page cache verifies on load

- [ ] `PageCache` gains `generation_bound: AtomicU64` (default `u64::MAX`) and
  `set_generation_bound`.
- [ ] `get_or_load` calls `page::verify` on every backend read, including the
  capacity-0 path, before inserting. A failed page is not cached.
- [ ] Remove `invalidate_from` (its only caller is the old `save`).
- [ ] Tests: a corrupt page returns STG-029 and `cached_page_count()` is unchanged; a
  page with `generation > bound` is rejected and accepted after the bound is raised.

### Task 3: Pages on the new header

- [ ] B+tree: types 0x61/0x62, header 24 bytes, `next_leaf` removed from the encoding
  and from `build_btree`'s patch pass and `HeldPage`. `count` lives at bytes 2..4.
  Internal: `rightmost_child` moves to body offset 24.
- [ ] `PageAllocator { free: Vec<u64>, next_append: u64, generation: u64 }` with
  `alloc()` (free list first, then append) and `alloc_append()`. Writers take
  `&mut PageAllocator` and seal every page with its generation.
  `rebuild_btree_incremental` restamps copied leaves.
- [ ] Fact pages: `packed_pages` writes 0x41 with the 24-byte header and
  `MAX_FACT_BYTES = PAGE_SIZE − 24 − 4`. Move the 12-byte 0x02 reader to
  `packed_pages::legacy_v7`, used only by migration.
- [ ] Update the type-check and layout tests and every `PAGE_TYPE_*` assertion.
  `range_scan_ignores_next_leaf` is deleted, since the field no longer exists.

### Task 4: Meta pages — `src/storage/meta.rs`

- [ ] `MetaPage` with the §4.1 fields, `encode() -> [u8; PAGE_SIZE]` and
  `decode(&[u8]) -> SlotState { Empty, Valid(MetaPage), Damaged }`.
  `fn slot_page(generation) = (generation − 1) % 2`.
- [ ] `KNOWN_FEATURES: u64 = 0`. A valid meta with unknown bits fails open with
  STG-034, naming the bits.
- [ ] Tests: round trip; each field offset matches the table; reserved bytes non-zero →
  Damaged; CRC flip → Damaged; no magic → Empty; v7 header bytes → Empty (it has no
  "META").

### Task 5: Free-list pages

- [ ] `freelist::write_chain(ids, alloc, backend, cache)`: up to 508 ids per page,
  `next u64` at body offset 24, chain pages allocated from `alloc`. The chain's own
  pages are not in the list.
- [ ] `freelist::read_chain(head, backend, cache, page_count) -> Vec<u64>`: verifies each
  page and bounds the walk by `page_count`. A cycle, an id < 2 or an id ≥ `page_count`
  gives STG-035.
- [ ] Tests: round trip at 0, 1, 508, 509 and 5 000 ids; a cycle → STG-035.

### Task 6: FileBackend format-agnostic

- [ ] Drop the `header` field and `read_header`/`write_header`. `page_count = len /
  PAGE_SIZE`, tracked in memory and bumped on append. `read_page` past it → INT-049.
- [ ] New file: create, `sync_parent_dir`, no page written. `is_new = page_count == 0`.
- [ ] `MemoryBackend::is_new` returns `page_count == 0`.
- [ ] Update `file.rs` tests. Header-validation tests move to Task 8.

### Task 7: WAL v2

- [ ] `WAL_VERSION = 2`. `base_generation u64` at header bytes 8..16.
  `WalWriter::open_or_create(path, sync, base_generation)` writes it on create. An
  existing file keeps its header.
- [ ] `WalReader::base_generation() -> u64`. A v1 header reads as 1: a v1 WAL can only
  sit next to a migrated file whose generation-2 commit has not finished.
  Other versions → WAL-002.
- [ ] `db.rs`: read the WAL header (if the file exists) before
  `PersistentFactStorage::open`, and pass `Option<u64>`. A lazily created WAL takes
  `pfs.generation()`.
- [ ] Tests: v2 round trip; v1 file → 1; version 3 → WAL-002.

### Task 8: Open and meta selection

- [ ] `PersistentFactStorage::open(backend, cache_cap, wal_base: Option<u64>)`
  replaces the load branch of `new`:
  1. `page_count == 0`, or neither slot valid with `page_count ≤ 2` and no v7
     header: write an empty generation-1 meta to page 0, sync.
  2. Both valid: the higher generation wins.
  3. One valid at `g`: apply the §4.1.1 table. The evidence search reads `M_g`'s free
     list and pages `M_g.page_count..page_count` raw, and looks for a page that passes
     `verify` with generation `g + 1`.
  4. Neither valid: a v7 header → Task 10 migration. Otherwise, a valid generation-1
     meta in the last page whose `page_count` equals the file's → copy it to page 0,
     sync, continue. Otherwise STG-032.
- [ ] Wire the readers from the chosen meta and set the cache generation bound. Delete
  `index_checksum`, `fact_prefix_crc`, `compute_*_checksum`, `hash_pages` and the
  rebuild-on-open branch. `FileHeader` becomes `LegacyHeaderV7` (read-only, migration
  only).
- [ ] Tests, one per row: both valid; torn newer slot with WAL base `g` (open at `g`,
  replay); WAL base `> g` → STG-033; no WAL with a `g+1` page on the free list → STG-033;
  no WAL with a `g+1` page past `page_count` → STG-033; no WAL and no evidence → open at
  `g`; `g == 1` with slot B empty; unknown feature bit → STG-034 and the file bytes are
  unchanged; pre-release v8 single header → STG-032; an error path never modifies the
  file.

### Task 9: `save()` — full rewrite, atomic

- [ ] Steps:
  1. Set `g' = g + 1` and build the allocator from `read_chain(M.freelist_head)` and
     `M.page_count`.
  2. Append fact pages for the pending facts.
  3. Rebuild the four trees through the allocator.
  4. `freed` = every page of `M`'s trees plus `M`'s chain pages. The new list is the
     unused part of `M`'s free list plus `freed`. `write_chain` allocates its pages from
     that unused part, or by appending.
  5. Sync. Write meta `g'` to `slot_page(g')`. Sync.
  6. Swap the readers, set `M = M'`, raise the cache bound, and clear pending.
- [ ] Concurrent queries hold `M`'s roots. They never read a page that `save` writes,
  which follows from §8.1.
- [ ] Test guard: `WriteLog` (a test backend recording written ids) asserts that no
  written id is reachable from `M`. Run it in the existing save tests.
- [ ] Invariant helper (`#[cfg(test)]`): the pages reachable from the trees plus the
  fact pages plus the chain pages, together with the free ids, are disjoint and cover
  `2..page_count`. Check it after each save in the multi-save tests.
- [ ] `stream_all` via EAVT page ids (see Shape decisions).

### Task 10: v7 migration

- [ ] Detection: a page 0 whose `LegacyHeaderV7` parses with version 7 and a valid
  header CRC.
- [ ] Steps (§9 with interim pages):
  1. Read the facts through `legacy_v7`.
  2. Append 0x41 fact pages and four trees from `old_page_count`.
  3. Set the free list to `2..old_page_count`, plus the backup page id.
  4. Append the backup meta.
  5. Sync, write page 0, sync.
- [ ] A WAL next to the v7 file is replayed after migration. Its v1 header reads as
  base 1.
- [ ] Tests: the existing `v7_header_forces_rebuild_and_upgrades_to_v8` and
  `v7_migration_does_not_rewind_tx_counter…`; a crash at every write/sync of migration
  leaves either the v7 file or a v8 file with every fact; a torn page-0 write recovers
  from the backup.

### Task 11: Torn writes and crash tests

- [ ] `FaultConfig::torn_write_at: Option<(u64, usize)>`: at write number `n`, write
  the first `k` bytes of the new page over the old one, then fail.
- [ ] Extend `crash_at_every_point_in_save_loses_no_checkpointed_fact`: for each write
  index, a failure and torn writes at `k ∈ {0, 512, 4095}`, including the meta write.
  After each: reopen with no rebuild, check the facts against the model, replay the
  WAL, and run the invariant helper.
- [ ] Browser: confirm that `BrowserDb`'s flush writes page 0/1 in the same
  `write_pages` call as the data pages. A test asserts that the dirty set after
  `save()` holds the meta page.

### Task 12: Docs and CI

- [ ] `CHANGELOG.md` Unreleased (format v8 header, meta pages, WAL v2, `MAX_FACT_BYTES`
  is 12 bytes smaller), `docs/ERROR_REFERENCE.md`, `docs/TEST_COVERAGE.md`, the
  `CLAUDE.md` test count, and the `CLAUDE.md` File Format section, which describes the
  interim layout.
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and
  `cargo bench --bench minigraf_bench -- checkpoint` against v3 (a regression is expected
  until PR 4; record it).
- [ ] Open the PR into `v3` with `Refs #374 #388 #434`. Own CI until it is green. Ask the
  user before merging.

# PR 3 — Covering keys and dictionaries

Spec §4.3, §5, §6, §9. Branch `feat/v8-pr3-covering-keys`. The format freezes after
this PR.

**Outcome:** the four index trees hold whole facts as byte-comparable keys, with
sequential entity ids and ident ids from a DICT tree, and long strings in deduplicated
value pages. Fact pages (0x41), `FactRef` and `CommittedFactReader` are gone. `save()`
is still a full rewrite (O(N)); PR 4 makes it O(change).

**Shape decisions (from reading the v3 code):**

- **The query layer reads committed data through four paths only:** every fact
  (`get_all_facts`), by entity, by entity and attribute (EAVT), and by attribute (AEVT).
  AVET and VAET range scans are never called outside tests, and the matcher's
  index-hint lookup only checks for emptiness before scanning every fact either way.
  `CommittedFactReader` and `CommittedIndexReader` are therefore replaced by one
  `CommittedReader` trait with those four methods, returning `Vec<Fact>`. AVET and VAET
  are still written (spec §5.2, #373 `rebuild_indexes`, #432) and checked in tests.
- **Ids, value refs and keys are assigned at checkpoint, not at transact (deviation from
  spec §5.1).** A long value's ref exists only once its value page is written, which
  happens at checkpoint, so a key containing it cannot be built earlier. Ids are never
  persisted before the commit that also writes the dictionary entries and keys using
  them, so assigning them at checkpoint, in pending order (which is WAL order), is
  equally deterministic and crash-safe. The transact path does no dictionary I/O. The
  pending side stays logical: `Vec<Fact>` plus `BTreeMap`s over the existing
  UUID/string keys, mapping to `usize` positions. #432 merges pending and committed
  streams at scan time. Spec §5.1 is amended.
- **`tx↓` is the byte complement of the encoding of `tx_count` (deviation from spec
  §6.1).** The integer encoding is prefix-free, so complementing every byte exactly
  reverses the order, and a typical `tx_count` costs 2–4 bytes instead of the 9 bytes
  of `u64::MAX − tx_count`. Spec §6.1 is amended.
- **DICT 0x06 key is `tag ‖ hash64 ‖ value ref`, with an empty value,** so several
  long values can share a hash. Dedup seeks to `tag ‖ hash64` and compares each
  candidate's full value. `hash64` is FNV-1a 64: stable, no dependency. A collision
  only costs an extra comparison, never a wrong match.
- **DICT 0x03 keys hold the raw ident bytes.** Nothing follows them in the key, so they
  need no escaping or terminator.
- **Idents are capped at `MAX_IDENT_BYTES = 1024`.** The parser already caps keywords
  at 1024 B. The cap is enforced at WAL write for every path, so a DICT entry always
  fits in a node.
- **Size limit:** strings are capped at `MAX_VALUE_BYTES = PAGE_SIZE − 24 − 4 = 4068`.
  This replaces `MAX_FACT_BYTES` (crate-private, so not a public API change). WAL-003
  keeps its code; its message now names the value. An index key is at most about
  200 B.
- **`tx_count → tx_id` (DICT 0x05).** Every commit path stamps one `tx_id` per
  `tx_count`, and the v1→v2 migration grouped facts by `tx_id`. A checkpoint or
  migration that finds two different `tx_id`s for one `tx_count` fails with a new STG
  code rather than altering a timestamp.
- **Node format (§4.3).** Leaves hold prefix-compressed entries,
  `varint shared ‖ varint suffix_len ‖ suffix ‖ varint value_len ‖ value`, with a
  restart point every 16 entries and a `u16` restart array at the end of the page.
  Internal nodes hold `rightmost_child u64` at offset 24, then
  `varint sep_len ‖ sep ‖ child u64` entries with a `u16` slot array at the end. The
  separator between two subtrees is the shortest prefix of the right subtree's first
  key that is greater than the left subtree's last key.
- **btree_v6.rs becomes btree.rs** with byte keys: `build_btree`,
  `rebuild_btree_incremental` (copy untouched leaves, restamped), `LeafCursor` (with
  `seek`), `collect_leaf_pages`. Entries are `(Vec<u8>, Vec<u8>)`; the index trees use
  empty values.
- **Committed reads translate through a per-call `Dict` reader** that memoises
  eid→UUID, iid→name, tx_count→tx_id and value-ref→string lookups. Memory is bounded by
  one call's results, not by the dictionary.
- **Migration** reuses the checkpoint pipeline from an empty dictionary and empty
  trees, with facts sorted stably by `tx_count` (§9 step 2). Value pages and trees
  append from the old `page_count`. The free list, backup meta and crash handling stay
  as in PR 2.
- **Committed scan order changes** to eid/iid order (§5.3). Tests that compare lists
  sort first; §11's order audit covers the bindings.

## Review Focus

- memcmp order equals logical order for every component and value type.
- The leaf and internal codecs reject any page overflow, and the separators satisfy
  `max(left) < s ≤ min(right)`.
- Every dictionary id is assigned exactly once and never reused, across checkpoints and
  migration.
- Committed reads return exactly the facts that were written (property test against
  the pending model).

### Task 1: Key codec — `src/storage/keys.rs`

- [ ] FDB integers: `encode_uint`/`encode_int`/`decode_*`; FOREVER byte for `vt`;
  `tx↓` as the complemented `tx_count` encoding.
- [ ] Value encoding (§6.2): tags in cross-type order; short strings escaped with
  `00 FF` and terminated by `00 00`; long strings as escaped 32-byte prefix,
  `00 01`, `hash64`, value ref (`page u64 BE`, `slot u16 BE`); keyword = iid; ref =
  eid; float transform with NaN canonicalised.
- [ ] Index key builders and decoders for EAVT/AEVT/AVET/VAET over an `EncodedFact`
  (`e, a, v, tx_count, vf, vt, op`). DICT key builders for tags 0x01–0x06.
- [ ] `hash64` (FNV-1a), `MAX_VALUE_BYTES`, `MAX_IDENT_BYTES`.
- [ ] Tests: random tuples, sorted by encoding versus by logical order, for every type,
  negative/zero/large integers, NaN and ±0.0, strings containing 0x00, strings of 64
  and 65 bytes, FOREVER. Round trip of every component. Complemented `tx` reverses
  order.

### Task 2: Node codecs

- [ ] Leaf encode/decode with prefix compression and restarts. `lookup` uses binary
  search over the restart points, then a scan of at most 16 entries.
- [ ] Internal encode/decode with the slot array; `route(key)`.
- [ ] `shortest_separator(left_last, right_first)`.
- [ ] Tests: random keys round-trip through a leaf; lookup agrees with a linear scan;
  separators satisfy the bound; an overflow is INT-049.

### Task 3: `btree.rs` on byte keys

- [ ] Port `build_btree`, `build_internal_levels` (infos carry first and last key),
  `rebuild_btree_incremental`, `collect_leaf_pages`, `LeafCursor` (`next`, `seek`) and
  `range_scan` to `(Vec<u8>, Vec<u8>)` entries. Equal keys merge to one entry.
- [ ] Port the existing tree and cursor tests (incremental equivalence, splits,
  repeated rounds, seek against a fresh descent, page-read counting, cycles,
  corruption).

### Task 4: Value pages — `src/storage/value_pages.rs`

- [ ] 0x51 page: records end-to-start with a `(offset u16, len u16)` directory.
  `ValueWriter` appends pages through the allocator (`alloc_append`); each checkpoint
  starts a fresh page (§13). `read_value(ref)`.
- [ ] Tests: round trip; a value of exactly `MAX_VALUE_BYTES`; a bad slot is an error.

### Task 5: Dictionary — `src/storage/dict.rs`

- [ ] `Dict` reader over the DICT root: `eid_of(uuid)`, `uuid_of(eid)`,
  `iid_of(name)`, `name_of(iid)`, `tx_id_of(tx_count)`, `long_value_refs(hash)`, each
  memoised per instance.
- [ ] `DictWriter`: assigns new eids/iids from the meta's counters, records new tx
  entries (STG conflict check), deduplicates long values against committed and new
  ones, and emits the sorted new DICT entries.
- [ ] Tests: assignment is deterministic and never reuses an id; a keyword used as an
  attribute and as a value shares one iid; dedup of a re-asserted and retracted long
  value adds no value page; a forced hash collision still finds the right value.

### Task 6: Encode pipeline and save

- [ ] `encode_facts(facts, dict_writer, value_writer) -> [Vec<key>; 4]` and the DICT
  entries; `save()` and `migrate_v7` use it, then `rebuild_btree_incremental` on all
  five trees. The meta gets `dict_root`, `next_eid` and `next_iid`.
- [ ] Delete fact pages from v8: `append_fact_pages`, `pack_facts` for v8 and type 0x41
  (stays reserved). `packed_pages.rs` keeps only the v7 reader.
- [ ] The space-accounting and index-exactness test helpers walk DICT and value refs.

### Task 7: Covering reads

- [ ] `CommittedReader` trait (`all_facts`, `facts_for_entity`,
  `facts_for_entity_attribute`, `facts_for_attribute`), implemented by
  `OnDiskIndexReader` with EAVT/AEVT cursors and a `Dict`. An unknown UUID or attribute
  means no committed facts.
- [ ] `FactStorage`: one `committed` reader; pending maps hold `usize` positions; only
  EAVT and AEVT pending maps remain. Delete `FactRef`, `CommittedFactReader`,
  `resolve_fact_ref`, and the matcher's and optimizer's unused index plumbing.
- [ ] Tests: a counting backend shows no value-page read for facts without long
  values; a property test compares committed reads against the in-memory model across
  checkpoints.

### Task 8: Size limits and WAL

- [ ] WAL write checks `MAX_VALUE_BYTES` for strings and `MAX_IDENT_BYTES` for
  attributes and keywords (WAL-003). Update `edge_cases_test`, `multi_value_test` and
  `error_codes_wal_test` boundaries.

### Task 9: Migration retarget and crash tests

- [ ] `migrate_v7` through the pipeline. Keep the PR 2 migration crash tests; add v7
  fixtures with long strings, keywords, refs and retractions, compared as sets.

### Task 10: Order audit, docs, PR

- [ ] Run the whole suite; fix order-dependent tests by sorting. Grep the bindings
  (`minigraf-*` repos are separate; record any order assumption found for the cascade).
- [ ] Docs: spec amendments (§5.1, §6.1, §6.2 DICT 0x03/0x06), CHANGELOG (result order,
  size limits), CLAUDE.md File Format and module list, ERROR_REFERENCE, TEST_COVERAGE.
- [ ] `cargo fmt`, clippy, `cargo test`; open the PR into `v3` with
  `Refs #433 #374 #434 #388`. Own CI until green. Ask before merging.

# PR 4 — Copy-on-write insert, allocator, free list

Spec §4.4, §8. Branch `feat/v8-pr4-cow`. The format does not change.

**Outcome:** a checkpoint writes only new value pages, the copied path from each touched
leaf to its root in each of the five trees, the free-list pages it pushes or rewrites,
and one meta page. Its cost depends on the change, not on the graph size (#434
acceptance 1, 2, 4).

**Shape decisions (from reading the v3 code after PR 3):**

- **Lazy free-list pop.** `PageAllocator` gains a chain source: it reads `M`'s free-list
  pages one at a time, only when it needs another id, and hands out ids in page order.
  A page whose ids are all handed out becomes free itself (it is unreferenced once the
  new meta commits). `alloc` therefore needs read access to the backend and cache:
  its signature becomes `alloc(&mut self, backend: &dyn StorageBackend, cache:
  &PageCache)`. The eager `Vec` form stays for the migration and for tests.
- **Push at the end of the checkpoint.** The pushed ids are:
  - every page the checkpoint replaced (old tree nodes on copied paths);
  - every chain page it emptied;
  - the head page it partly consumed, plus that page's remaining ids.

  They are written as new head pages whose `next` is the first untouched chain page,
  so the untouched tail is shared. The new head pages are allocated through the same
  allocator, which may pop more ids; that repeats until stable (it converges in one or
  two rounds). Nothing pushed by this checkpoint is reused within it (§8.1).
  `freelist_count` is computed as old count − ids in the pages read + ids pushed. The
  cost is O(pages allocated + pages freed).
- **Copy-on-write batch insert** (`btree::cow_insert(root, sorted entries, alloc, freed)`):
  - **Descent and routing:** descend from the root, partitioning the entries by
    `Internal::route`.
  - **Leaves:** a touched leaf is merged with its entries and repacked with
    `pack_leaves`, the balanced split from #315.
  - **Internal nodes:** a touched internal node is rewritten with each touched child
    replaced by its pieces. Separators between new pieces are the shortest separators
    of the adjacent pieces' last and first keys; existing separators stay valid because
    the routed keys lie within them. An internal node that outgrows a page is split,
    balanced by bytes, and the separator at the cut moves up.
  - **Root:** a root split adds a level.
  - **Freeing:** every replaced page goes into `freed`; untouched subtrees are shared
    by page id.
  - **Edge cases:** an empty tree (root 0) is a bulk build. Nothing is ever removed
    from a tree, so there is no merging.
- **`save()`** no longer collects the old trees' leaves. It calls `cow_insert` on each
  of the five trees, then pushes onto the free list. `rebuild_btree_incremental` and
  the full-tree `collect_leaf_pages` snapshot leave the checkpoint path:
  `collect_leaf_pages` stays for the reachability test helpers and for #373's verify.
- **Concurrency (§8.1).** A query holds the `FactStorage` read guard for its whole
  scan, and swapping in the new reader needs the write lock. So no query spans two
  checkpoints, and pages freed by checkpoint N are written only by N + 1, after every
  reader of N − 1 has finished. The page cache stays write-through.
- **Browser (§12).** `BrowserBufferBackend` already tracks dirty pages per write, so a
  flush is O(change) once checkpoints are. Verify it, with no code change expected.

## Review Focus

- A checkpoint never writes a page the previous meta references (tested on every save).
- After every save, the reachable pages and the free ids are disjoint and cover
  `2..page_count` exactly.
- Copy-on-write insert equals a sorted merge, and the old root still streams its old
  contents unchanged.

### Task 1: Lazy chain allocator and push

- [x] `PageAllocator::from_chain(head, count, next_append, generation)`; `alloc` reads
  chain pages on demand; `finish_free_list(freed, backend, cache) -> (head, count)`
  pushes and writes new head pages.
- [x] Tests: pop across page boundaries; partial head rewrite; untouched tail shared
  (same page ids); count arithmetic; no id handed out twice; pushed ids never handed
  out in the same generation.

### Task 2: `cow_insert`

- [x] Recursive insert returning replacement pieces `(Option<separator>, page id,
  first key, last key)`. Leaf merge and balanced split, internal rewrite and split,
  root growth.
- [x] Tests:
  - random committed sets and pending batches (below the first key, above the last,
    into one leaf, root splits), compared with a sorted union;
  - the old root streams its old contents unchanged;
  - `freed` equals exactly the old pages on the touched paths;
  - single inserts do not fragment leaves;
  - depth-1 to depth-4 trees.

### Task 3: `save()` on copy-on-write

- [x] Five `cow_insert` calls plus the free-list push. Remove `rebuild_btree_incremental`
  from `save()`; port its tests to `cow_insert` or delete those that duplicate.
- [x] The existing crash-at-every-point, space-accounting and never-write-committed-pages
  tests keep passing.

### Task 4: Cost bound

- [x] A page-counting backend counts pages written by a checkpoint after k ∈ {1, 100}
  facts on graphs of 10k and 100k facts. The 100k count may exceed the 10k count by at
  most the tree-depth difference per tree plus the free-list pages.
- [x] `checkpoint/after_1_fact` measured at 10k and 100k (3.1 / 3.3 ms). `after_100k_facts`
  and 1M move to PR 5.

### Task 5: Concurrency

- [x] `concurrency_test`: reader threads query in a loop while the writer transacts
  and checkpoints repeatedly. Every read succeeds (no CRC, page-id or generation
  error) and sees a consistent fact count.

### Task 6: Docs, PR

- [x] CHANGELOG (checkpoint cost, with numbers), `checkpoint()` and
  `wal_checkpoint_threshold` rustdoc ("copies pages in proportion to the total index
  size" goes), CLAUDE.md, TEST_COVERAGE.
- [x] `cargo fmt`, clippy, `cargo test`; open the PR into `v3` with `Refs #434 #374`. Own
  CI until green. Ask before merging.

# PR 5 — Hardening, benchmarks, docs (outline)

Spec §10, §11, §12.

- Benchmarks: `checkpoint/after_1_fact` and `checkpoint/after_100k_facts` at 10k/100k/1M, bytes written per checkpoint, the 1M size test (≤ 200 B per fact).
- Corruption-surfacing tests for leaf, value page and free-list page.
- Docs: rustdoc of `checkpoint`/`wal_checkpoint_threshold`, `CLAUDE.md` File Format, `.wiki/Architecture.md`, CHANGELOG, ROADMAP, `ERROR_REFERENCE.md`, TEST_COVERAGE.
