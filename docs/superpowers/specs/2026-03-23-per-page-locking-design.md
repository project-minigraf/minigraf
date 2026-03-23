# Per-Page Locking for Concurrent B+Tree Range Scans — Design Spec

**Date:** 2026-03-23
**Phase:** 6.5 extension (performance tuning)
**Status:** Approved

---

## Problem

`OnDiskIndexReader::range_scan_*` in `src/storage/btree_v6.rs` acquires `Arc<Mutex<B>>` once and holds the `MutexGuard` for the entire scan:

```rust
let backend = self.backend.lock().unwrap();   // held for entire scan
range_scan(root, start, end, &*backend, &self.cache)
```

Inside `range_scan`, `cache.get_or_load(leaf_id, backend)` is called for each B+tree leaf page. On a cache hit, `PageCache::get_or_load` returns via its read-lock fast path without ever calling `backend.read_page` — but the backend mutex is still held, serialising all concurrent callers.

Benchmark evidence (`bench_concurrent_btree_scan`, 10K facts):
- 2 → 4 threads: ~1.4× latency increase
- 4 → 8 threads: ~2.2× latency increase

Both are above ideal linear scaling, confirming backend mutex contention as the bottleneck.

---

## Goal

Eliminate backend mutex contention between concurrent readers performing B+tree range scans. On a cache hit, no lock should be acquired at all. On a cache miss, the lock should be held only for the duration of a single `backend.read_page` call.

---

## Non-Goals

- No changes to `PageCache`, `range_scan`, or any caller outside `btree_v6.rs`.
- No changes to the `StorageBackend` trait.
- No per-page locking inside `PageCache` itself (deferred to Phase 8 if needed).
- No file format changes.

---

## Design

### New Type: `MutexStorageBackend<B>`

A private newtype inside `btree_v6.rs` that wraps `Arc<Mutex<B>>` and implements `StorageBackend`. It acquires and releases the mutex per individual `read_page` call.

```rust
/// Read-only `StorageBackend` adapter that locks `Arc<Mutex<B>>` per page read.
///
/// Used exclusively by `OnDiskIndexReader::range_scan_*` so that the backend
/// mutex is held only for the duration of a single page read on a cache miss,
/// rather than for the entire range scan.
struct MutexStorageBackend<B>(Arc<Mutex<B>>);

impl<B: StorageBackend> StorageBackend for MutexStorageBackend<B> {
    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        self.0.lock().unwrap().read_page(page_id)
    }

    fn write_page(&mut self, _page_id: u64, _data: &[u8]) -> Result<()> {
        unreachable!("MutexStorageBackend is read-only; write_page must not be called")
    }

    fn page_count(&self) -> Result<u64> {
        self.0.lock().unwrap().page_count()
    }
}
```

`write_page` uses `unreachable!()` because `MutexStorageBackend` is only ever passed to `range_scan`, which calls `cache.get_or_load` — the only path that invokes `read_page`. `write_page` is never reachable through that code path.

### Change: `OnDiskIndexReader::range_scan_*`

Each of the four `range_scan_*` methods drops its pre-locked guard and uses the adapter instead:

**Before:**
```rust
fn range_scan_eavt(&self, start: &EavtKey, end: Option<&EavtKey>) -> Result<Vec<FactRef>> {
    if self.eavt_root == 0 { return Ok(vec![]); }
    let backend = self.backend.lock().unwrap();
    range_scan(self.eavt_root, start, end, &*backend, &self.cache)
}
```

**After:**
```rust
fn range_scan_eavt(&self, start: &EavtKey, end: Option<&EavtKey>) -> Result<Vec<FactRef>> {
    if self.eavt_root == 0 { return Ok(vec![]); }
    let adapter = MutexStorageBackend(Arc::clone(&self.backend));
    range_scan(self.eavt_root, start, end, &adapter, &self.cache)
}
```

Same pattern for `range_scan_aevt`, `range_scan_avet`, `range_scan_vaet`.

### Locking Behaviour After Change

| Scenario | Before | After |
|---|---|---|
| Cache hit (all pages warm) | Backend mutex held for entire scan | No backend mutex acquired |
| Cache miss (cold page) | Backend mutex held for entire scan | Backend mutex held for one `read_page` call, then released |
| Concurrent readers (all cache-warm) | Fully serialised | Fully parallel |
| Concurrent readers (mixed warm/cold) | Fully serialised | Serialised only on cold-page I/O |

---

## Files Changed

| File | Change |
|---|---|
| `src/storage/btree_v6.rs` | Add `MutexStorageBackend<B>` struct; update 4 `range_scan_*` methods |

No other files change.

---

## Testing

### New Unit Test: `test_concurrent_range_scans_no_deadlock`

Location: `src/storage/btree_v6.rs` (inside `#[cfg(test)]` block)

Purpose: verify correctness under concurrent access (not a performance test).

Outline:
1. Build a B+tree with 50 `EavtKey` entries using `MemoryBackend` — enough to span multiple leaf pages.
2. Wrap in `Arc<Mutex<MemoryBackend>>` and construct an `OnDiskIndexReader`.
3. Spawn 8 threads, each synchronised via a `Barrier`.
4. Each thread calls `reader.range_scan_eavt(&start, Some(&end))`.
5. Assert all threads return the same non-empty `Vec<FactRef>` and no thread panics.

This test would deadlock or panic before the fix (if the mutex were re-acquired recursively) and passes after.

### Existing Benchmark

`bench_concurrent_btree_scan` in `benches/minigraf_bench.rs` (added in Phase 6.5 Task 7) already measures wall-clock latency for 2/4/8 concurrent EAVT range scans. Re-running it after the fix should show improved scaling (target: 4→8 threads closer to 2× than the current 2.2×).

---

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| `write_page` called unexpectedly | `unreachable!()` panics immediately — detectable in tests, not a silent failure |
| Cache miss storm on first open (many misses in rapid succession) | Individual page reads are still serialised by the mutex; this is correct and safe — it only means cold-cache concurrent performance is bounded by I/O throughput, which is the right bottleneck |
| Deadlock if `read_page` internally re-acquires the same mutex | `StorageBackend` implementations (`FileBackend`, `MemoryBackend`) do not use the outer `Mutex` — no re-entrancy risk |
