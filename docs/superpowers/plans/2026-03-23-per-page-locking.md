# Per-Page Locking for Concurrent B+Tree Range Scans — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the whole-scan backend mutex hold in `OnDiskIndexReader::range_scan_*` with a per-page-read lock so concurrent readers don't block each other on cache-warm pages.

**Architecture:** Add a private `MutexStorageBackend<B>` newtype to `src/storage/btree_v6.rs` that implements `StorageBackend` by locking `Arc<Mutex<B>>` only for each individual `read_page` call. The four `range_scan_*` methods on `OnDiskIndexReader` pass this adapter instead of a pre-locked guard. No other files change.

**Tech Stack:** Rust, `std::sync::{Arc, Mutex}`, existing `StorageBackend` trait (`src/storage/mod.rs`).

---

## File Structure

| File | Change |
|---|---|
| `src/storage/btree_v6.rs` | Add `MutexStorageBackend<B>` struct (~30 lines); update 4 `range_scan_*` methods (1-line change each); add 1 unit test |

---

### Task 1: Add `MutexStorageBackend`, update `range_scan_*`, add concurrency test

**Files:**
- Modify: `src/storage/btree_v6.rs:459–529` (the `OnDiskIndexReader` section and its `CommittedIndexReader` impl)
- Test: `src/storage/btree_v6.rs` (inside existing `#[cfg(test)]` block at the bottom of the file)

---

- [ ] **Step 1: Write the failing test first**

Open `src/storage/btree_v6.rs`. Scroll to the bottom, inside the `#[cfg(test)] mod tests` block (currently ends around line 841). Add the following test **before the closing `}`**:

```rust
#[test]
fn test_concurrent_range_scans_correctness() {
    use crate::storage::CommittedIndexReader;
    use std::sync::{Arc, Barrier};
    use std::thread;

    let mut backend = MemoryBackend::new();
    // build_btree takes &PageCache (not Arc), so construct without Arc first
    let cache = PageCache::new(256);
    // 50 entries — enough to span multiple leaf pages
    let input: Vec<(EavtKey, FactRef)> =
        (0u128..50).map(|n| make_eavt(n, ":x", n as u64 + 1)).collect();
    let (eavt_root, _) =
        build_btree(input.iter().cloned(), &mut backend, &cache, 1).unwrap();

    // Wrap in Arc after build_btree is done — OnDiskIndexReader requires Arc<PageCache>
    let reader = Arc::new(OnDiskIndexReader::new(
        Arc::new(Mutex::new(backend)),
        Arc::new(cache),
        eavt_root,
        0,
        0,
        0,
    ));

    // Scan entities 10..19 (10 entries expected)
    let start = EavtKey {
        entity: Uuid::from_u128(10),
        attribute: String::new(),
        valid_from: i64::MIN,
        valid_to: i64::MIN,
        tx_count: 0,
    };
    let end = EavtKey {
        entity: Uuid::from_u128(20),
        attribute: String::new(),
        valid_from: i64::MIN,
        valid_to: i64::MIN,
        tx_count: 0,
    };

    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let r = Arc::clone(&reader);
            let b = Arc::clone(&barrier);
            let s = start.clone();
            let e = end.clone();
            thread::spawn(move || {
                b.wait(); // all 8 threads start simultaneously
                r.range_scan_eavt(&s, Some(&e)).unwrap()
            })
        })
        .collect();

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let expected_len = results[0].len();
    assert!(expected_len > 0, "expected non-empty scan results");
    for (i, res) in results.iter().enumerate() {
        assert_eq!(
            res.len(),
            expected_len,
            "thread {} returned {} refs, expected {}",
            i,
            res.len(),
            expected_len
        );
    }
}
```

**Note on TDD here:** This test validates *correctness under concurrency* — all 8 threads must see the same result set. It passes before and after the fix (old code serialises correctly; new code runs concurrently). The fix is a performance change, not a correctness change, so the test documents the contract rather than gating the change. The performance improvement is validated by the existing `bench_concurrent_btree_scan` benchmark.

- [ ] **Step 2: Verify the test compiles and passes before any implementation change**

```bash
cd /path/to/worktree   # .worktrees/phase-6.5-btree
cargo test test_concurrent_range_scans_correctness -- --nocapture
```

Expected: `test storage::btree_v6::tests::test_concurrent_range_scans_correctness ... ok`

If it fails to compile, check:
- `EavtKey` must derive `Clone` (check `src/storage/index.rs`; it almost certainly already does — it's used as a BTreeMap key throughout).
- `build_btree` takes `cache: &PageCache`, not `Arc<PageCache>` — the test constructs `PageCache::new(256)` first, passes `&cache` to `build_btree`, then wraps in `Arc::new(cache)` for `OnDiskIndexReader::new`.

- [ ] **Step 3: Add `MutexStorageBackend<B>` struct**

In `src/storage/btree_v6.rs`, locate the `// ─── OnDiskIndexReader ───` section comment (around line 459). **Immediately above that section comment** (i.e., before the `// ─── OnDiskIndexReader` line itself), insert:

```rust
// ─── MutexStorageBackend ──────────────────────────────────────────────────────

/// Read-only [`StorageBackend`] adapter that locks `Arc<Mutex<B>>` only for the
/// duration of a single [`StorageBackend::read_page`] call.
///
/// Used exclusively by [`OnDiskIndexReader::range_scan_*`] so that the backend
/// mutex is held only while reading one cold page from disk, rather than for the
/// entire range scan. On a cache hit [`PageCache::get_or_load`] never calls
/// `read_page`, so no lock is acquired at all.
struct MutexStorageBackend<B>(Arc<Mutex<B>>);

impl<B: StorageBackend> StorageBackend for MutexStorageBackend<B> {
    fn read_page(&self, page_id: u64) -> anyhow::Result<Vec<u8>> {
        self.0.lock().unwrap().read_page(page_id)
    }

    fn write_page(&mut self, _page_id: u64, _data: &[u8]) -> anyhow::Result<()> {
        unreachable!("MutexStorageBackend is read-only; write_page must not be called")
    }

    fn sync(&mut self) -> anyhow::Result<()> {
        unreachable!("MutexStorageBackend is read-only; sync must not be called")
    }

    fn page_count(&self) -> anyhow::Result<u64> {
        unreachable!("MutexStorageBackend is read-only; page_count must not be called")
    }

    fn close(&mut self) -> anyhow::Result<()> {
        unreachable!("MutexStorageBackend is read-only; close must not be called")
    }

    fn backend_name(&self) -> &'static str {
        unreachable!("MutexStorageBackend is read-only; backend_name must not be called")
    }
}
```

- [ ] **Step 4: Update `range_scan_eavt`**

In `src/storage/btree_v6.rs`, find the `range_scan_eavt` method body (around line 495):

**Find:**
```rust
        if self.eavt_root == 0 { return Ok(vec![]); }
        let backend = self.backend.lock().unwrap();
        range_scan(self.eavt_root, start, end, &*backend, &self.cache)
```

**Replace with:**
```rust
        if self.eavt_root == 0 { return Ok(vec![]); }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.eavt_root, start, end, &adapter, &self.cache)
```

- [ ] **Step 5: Update `range_scan_aevt`**

**Find** (around line 504):
```rust
        if self.aevt_root == 0 { return Ok(vec![]); }
        let backend = self.backend.lock().unwrap();
        range_scan(self.aevt_root, start, end, &*backend, &self.cache)
```

**Replace with:**
```rust
        if self.aevt_root == 0 { return Ok(vec![]); }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.aevt_root, start, end, &adapter, &self.cache)
```

- [ ] **Step 6: Update `range_scan_avet`**

**Find** (around line 514):
```rust
        if self.avet_root == 0 { return Ok(vec![]); }
        let backend = self.backend.lock().unwrap();
        range_scan(self.avet_root, start, end, &*backend, &self.cache)
```

**Replace with:**
```rust
        if self.avet_root == 0 { return Ok(vec![]); }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.avet_root, start, end, &adapter, &self.cache)
```

- [ ] **Step 7: Update `range_scan_vaet`**

**Find** (around line 524):
```rust
        if self.vaet_root == 0 { return Ok(vec![]); }
        let backend = self.backend.lock().unwrap();
        range_scan(self.vaet_root, start, end, &*backend, &self.cache)
```

**Replace with:**
```rust
        if self.vaet_root == 0 { return Ok(vec![]); }
        let adapter = MutexStorageBackend(Arc::clone(&self.backend));
        range_scan(self.vaet_root, start, end, &adapter, &self.cache)
```

- [ ] **Step 8: Build and run all tests**

```bash
cargo test
```

Expected: all tests pass (the count increases by 1 — the new test). There must be no compilation errors or test failures.

If `MutexStorageBackend` fails to compile with a trait-method error, double-check the `StorageBackend` trait in `src/storage/mod.rs` — all methods must be covered. The trait has exactly 6 methods: `write_page`, `read_page`, `sync`, `page_count`, `close`, `backend_name`.

- [ ] **Step 9: Run clippy**

```bash
cargo clippy -- -D warnings
```

Expected: no warnings. If clippy flags `unreachable!()` usage (unlikely), suppress with `#[allow(unreachable_code)]` on the individual method only.

- [ ] **Step 10: Commit**

```bash
git add src/storage/btree_v6.rs
git commit -m "perf(btree): release backend mutex per page read in range_scan_*

OnDiskIndexReader previously held Arc<Mutex<B>> for the entire range scan.
Add MutexStorageBackend<B> adapter that locks only for individual read_page
calls on cache misses. On cache hits, no backend lock is acquired at all,
allowing concurrent readers to proceed in parallel.

Also add test_concurrent_range_scans_correctness to document the contract."
```
