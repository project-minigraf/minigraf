# v8 Storage Format Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the v8 storage format: crash-atomic copy-on-write checkpoints whose cost depends only on the change, per-page checksums, and covering indexes with dictionary-coded, byte-comparable keys. This closes #374, #434, #388 and #433's format decisions.

**Spec:** `docs/superpowers/specs/2026-10-05-v8-storage-format-design.md` (§ numbers below refer to it).

**Tech Stack:** Rust (MSRV 1.89), postcard (until PR 3), crc32fast, criterion. No new dependencies.

**Delivery:** five PRs into `v3` (spec §14). This plan details PR 1 task by task. PRs 2–5 are listed at task level. Each gets its own detailed plan section, written when the previous PR has merged, because each builds on the code shape the previous one leaves.

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

# PR 2 — v8 page format (outline)

Spec §4.1, §4.2, §12. Atomic but still O(N) `save()`.

- Common 24-byte page header (type, count, CRC, page id, generation): encode and verify helpers. Verification runs in `PageCache::get_or_load`, with new STG codes.
- Renumber page types (0x51, 0x61, 0x62, 0x81). Update constants and the type-check tests.
- Meta pages A/B: `MetaPage` struct, encode/decode, selection by §4.1.1. WAL v2 `base_generation` (`wal.rs` header). Search for evidence of an interrupted commit.
- `required_features` check and error code.
- `save()`: write everything into fresh pages (append), sync, write the inactive meta, sync. Delete `index_checksum`, `fact_prefix_crc` and the rebuild-on-open branch.
- `FaultInjectingBackend` torn-write mode. Extend the crash-at-every-point test, including a torn meta write.
- Browser flush check: the meta page goes in the same IDB transaction.

# PR 3 — Covering keys and dictionaries (outline)

Spec §4.3, §5, §6, §9. The format freezes after this PR.

- Key codec module: FoundationDB-style integers, value tags, string escaping, long-value prefix + hash + ref, FOREVER byte, `tx↓`. Property tests: memcmp order equals logical order.
- Node format: prefix-compressed leaves with restart points; internal nodes with shortest separators; generic `(key bytes, value bytes)` entries. `LeafCursor` moves to byte keys.
- DICT tree with tags 0x01–0x06. Id assignment at transact time in WAL order (`FactStorage` pending index keyed by encoded keys). `next_eid`/`next_iid` in the meta page.
- Value pages with dedup. `MAX_VALUE_BYTES` replaces `MAX_FACT_BYTES`.
- Covering reads: `CommittedIndexReader` returns facts, translating ids through cached DICT lookups. Delete `FactRef`, fact pages, `CommittedFactReader`.
- v7 migration with a backup meta page, and crash tests.
- Audit of result-order assumptions in tests and bindings.

# PR 4 — Copy-on-write insert, allocator, free list (outline)

Spec §4.4, §8.

- Allocator over `M`'s free list plus append. Free-list chain pop/push with copy-on-write head pages.
- Copy-on-write batch insert per tree, reusing #315's balanced split and freeing replaced pages.
- Reachability/free-list invariant checker, used in tests after every checkpoint.
- Cost-bound test with a page-counting backend; a guard that pages referenced by the previous meta are never written.
- Concurrency test: queries during checkpoints.

# PR 5 — Hardening, benchmarks, docs (outline)

Spec §10, §11, §12.

- Benchmarks: `checkpoint/after_1_fact` and `checkpoint/after_100k_facts` at 10k/100k/1M, bytes written per checkpoint, the 1M size test (≤ 200 B per fact).
- Corruption-surfacing tests for leaf, value page and free-list page.
- Docs: rustdoc of `checkpoint`/`wal_checkpoint_threshold`, `CLAUDE.md` File Format, `.wiki/Architecture.md`, CHANGELOG, ROADMAP, `ERROR_REFERENCE.md`, TEST_COVERAGE.
