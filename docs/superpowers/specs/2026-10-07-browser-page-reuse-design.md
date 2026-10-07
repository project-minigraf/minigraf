# BrowserDb: write back migrated pages, keep only live pages — Design

Issues: #411 (migrated pages are not written back to IndexedDB), #440 (bound page
growth via v8 free-list reuse; avoid v7→v8 migration bloat). Milestone v3.0.0, branch
`v3`.

## 1. Problem

- `BrowserDb::open()` runs `PersistentFactStorage::new` on the pages loaded from
  IndexedDB. A v7 store is migrated to v8 in the buffer, but nothing is flushed, so the
  stored copy stays v7 and every open migrates again (#411).
- v8 checkpoints reuse freed pages (spec 2026-10-05 §8), so the *live* page set is
  bounded. But a free page keeps its old bytes in `BrowserBufferBackend` and its key in
  IndexedDB. After a v7→v8 migration the whole v7 region is free: one extra copy of the
  database in the WASM heap and in IndexedDB, for good (#440 §2).

## 2. Decisions

D1. **Free pages hold no bytes in the browser.** After open and after each commit,
`BrowserDb` drops the bytes of every page on the active free list from the buffer and
deletes its IndexedDB key. Chosen over "migrate into a fresh object store" because it
also covers ordinary churn, needs no IndexedDB schema version bump, and has one code
path. Safe because:
  - A free page is never read before it is written: the allocator hands out ids and the
    writer fills them; only free-list *chain* pages are read, and those are referenced
    by the meta, so never dropped.
  - The single-valid-slot probe (§4.1.1) already skips an unreadable page.
  - A read of an absent page is still `INT-052`, free or not.

D2. **One IndexedDB transaction per flush.** `IndexedDbBackend::write_pages` takes the
puts *and* the deletes. The meta page is in the dirty set of the same commit, so data
pages, meta and deletes commit or abort together (v8 spec §12, verified).

D3. **O(change) per commit.** `PersistentFactStorage::save` records the ids it lists in
the new free-list head pages (freed pages, chain pages read, ids read but not handed
out). The untouched tail was already released by an earlier commit or by open.
`take_released()` hands that list to the caller. Open releases the whole free list once
(`free_page_ids()`).

D4. **Open flushes (#411).** After `PersistentFactStorage::new`, `open()` releases the
free list and flushes dirty pages and deletes, like `import_graph()`. A normal open of a
v8 store with nothing to release does no IndexedDB write.

D5. **Export pads free pages with zeros.** `exportGraph()` writes
`max(meta.page_count, buffer high-water)` pages. A page the buffer released is written
as zeros; any other absent page is an error. Natively a zero page on the free list is
never read (allocator writes first; probe and verify do not decode free pages), so the
blob opens and verifies with `Minigraf::open()`.

D6. **Buffer bookkeeping.** `BrowserBufferBackend` gains `release(ids) -> Vec<u64>`
(drops bytes, clears dirty, returns ids that had bytes, i.e. the IndexedDB keys to
delete) and a `released` id set used only by export. A later `write_page` to a released
id removes it from the set.

## 3. Failure model

IndexedDB transactions are atomic, so a crash leaves the store at either the previous
or the new commit. A save that reaches the buffer but never the store (tab closed
before the flush) reopens at the previous commit, whose pages were never deleted (the
deletes are in the unflushed transaction).

Out of scope (pre-existing): if a flush *fails*, the drained dirty set is not restored,
so a later flush can miss pages. Tracked separately.

## 4. Tests

wasm (`wasm-pack test --headless --firefox --features browser`):
1. v7 fixture pages written straight into IndexedDB → `open()` + query only → page 0
   in IndexedDB is a v8 meta; IndexedDB key count equals live pages; no v7 bytes left
   (#411, #440 §2).
2. N transacts on an IndexedDB store → buffer page count and IndexedDB key count equal
   `meta.page_count − freelist_count` and stay below a bound independent of N.
3. A save applied to the buffer but not flushed, then reopen → previous commit, free
   list consistent, a further transact succeeds.
4. Export after churn → page 0 meta valid, length `meta.page_count` pages, imports back.
5. Buffer unit tests for `release`.

Native:
6. `PersistentFactStorage`: zeroing every free page after save, then reopen, verify and
   more saves → no findings, facts intact (what an exported blob looks like).
7. `take_released()` after save lists exactly the ids newly put on the free list.

## 5. Docs

CHANGELOG (v3.0.0, browser), TEST_COVERAGE, CLAUDE.md test count. No new error codes.

## 6. Philosophy

Aligned: reliability (no lost migrations), self-contained (no new deps), single store,
no format change.
