# Incremental Checkpoint Index Rebuild Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `checkpoint()` decode/re-encode only the index leaves the delta touches, and stop re-reading unchanged fact pages for the file checksum, without changing file format v7.

**Architecture:** `save()` snapshots each old B+tree's raw leaf pages, then `rebuild_btree_incremental` copies untouched leaves verbatim (patching only `next_leaf`), repacks only leaves that receive pending entries, and rebuilds internal levels with a helper extracted from `build_btree`. `PersistentFactStorage` caches the CRC32 state over its (immutable) fact pages so each save hashes only new fact pages plus the index region.

**Tech Stack:** Rust (MSRV 1.89), postcard, crc32fast, criterion.

**Spec:** `docs/superpowers/specs/2026-09-26-incremental-checkpoint-design.md`

## Global Constraints

- File format stays v7: node layout, header layout, and `index_checksum` value (CRC32 over pages `1..page_count`) are unchanged; files must open on v2.0.1 without the index-rebuild path.
- No public API change; no new dependencies (dev or runtime).
- Test assert messages must never format `Result`, `Fact`, `Value`, `EdnValue`, `EavtKey`, `FactRef` or anything containing a `Uuid` with `{:?}` (CodeQL `rust/cleartext-logging`). Use plain string messages.
- Match the surrounding clippy style: `btree_v6.rs` functions that index/do arithmetic carry `#[allow(clippy::arithmetic_side_effects)]` / `#[allow(clippy::indexing_slicing)]`; errors use `err_coded!`/`bail_coded!` with `ErrorCode::Int049` for B+tree invariant violations.
- `cargo test --release` does not work (panic=abort profile); use `cargo test` and `cargo bench`.
- Pre-push hook runs `cargo fmt --check`, `cargo clippy`, `cargo test`; run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before each commit.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01BedF9ZmqVZLkvDwn91fuKy
  ```
- Success target: 50k-fact graph, checkpoint after 1 fact ≤ 15 ms (was ~80 ms).

## Review Focus

1. **New index region overlaps the old one** — `save()` writes new fact pages over the start of the old index, and the new tree starts where the old tree's leaves were. Expect: entries identical to a from-scratch build. Pinned by `test_incremental_new_tree_overlaps_old_pages` (Task 3).
2. **Repeated checkpoints on an incrementally built tree** — the second, third… checkpoint consumes a tree whose leaves were raw-copied/split by the previous one. Expect: still exact. Pinned by `test_incremental_repeated_rounds` (Task 3) and `test_incremental_saves_keep_indexes_and_checksum_exact` (Task 4).
3. **Pending keys outside the old key range** (below the first leaf's first key, above the last leaf's last key). Expect: land in first/last leaf, range scans find them. Pinned by `test_incremental_pending_outside_old_range` (Task 3).
4. **Stale or wrong checksum-prefix cache** (after a failed save, a rebuild-on-open, or a page-count mismatch). Expect: silent fallback to a full pass and a correct stored checksum. Pinned by `test_prefix_cache_page_count_mismatch_falls_back` and `test_prefix_cache_absent_after_rebuild_on_open` (Task 4).
5. **Many pending entries into one leaf** (hot key range, e.g. all new facts for one entity). Expect: leaf splits into several ≤75%-fill leaves, no overflow. Pinned by `test_incremental_many_pending_into_one_leaf_splits` (Task 3).

---

### Task 1: Benchmark `checkpoint/after_1_fact` and record the baseline

**Files:**
- Modify: `benches/minigraf_bench.rs` (new function after `bench_checkpoint`, ~line 416; register in `criterion_group!` at ~line 1778)

**Interfaces:**
- Consumes: `helpers::populate_file_no_checkpoint(n, &path)`, `helpers::open_file_no_checkpoint(&path) -> Arc<Minigraf>`
- Produces: criterion group `checkpoint/after_1_fact` with ids `10k`, `100k`

- [ ] **Step 1: Add the bench function** after `bench_checkpoint`:

```rust
// ── checkpoint/after_1_fact (#315) ────────────────────────────────────────────

/// Checkpoint cost when only one fact is dirty, on an already-checkpointed graph.
/// Before #315 this was flat in dirty bytes and proportional to graph size.
fn bench_checkpoint_after_1_fact(c: &mut Criterion) {
    use criterion::BatchSize;
    use tempfile::NamedTempFile;

    let mut group = c.benchmark_group("checkpoint/after_1_fact");
    group.sample_size(20);
    for &(label, n) in &[("10k", 10_000usize), ("100k", 100_000)] {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        helpers::populate_file_no_checkpoint(n, &path);
        let db = helpers::open_file_no_checkpoint(&path);
        db.checkpoint().unwrap();
        let mut i = 0u64;
        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter_batched(
                || {
                    i += 1;
                    db.execute(&format!("(transact [[:ck{i} :val {i}]])"))
                        .unwrap();
                },
                |()| db.checkpoint().unwrap(),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}
```

- [ ] **Step 2: Register it** — add `bench_checkpoint_after_1_fact,` to the `criterion_group!(benches, ...)` list directly after `bench_checkpoint,`.

- [ ] **Step 3: Record the baseline**

Run: `cargo bench --bench minigraf_bench -- checkpoint/after_1_fact`
Expected: completes; note the two `time:` medians (≈ tens of ms at 100k) in the task report — they are the "Before" column for Task 5.

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add benches/minigraf_bench.rs
git commit -m "bench: add checkpoint/after_1_fact group (#315)"
```

---

### Task 2: Extract leaf encoding, fill rule and internal-level build from `build_btree`

Pure refactor; existing `btree_v6` tests are the safety net.

**Files:**
- Modify: `src/storage/btree_v6.rs:62-420`

**Interfaces:**
- Produces (module-private, used by Task 3):
  - `fn encode_leaf_page(entries: &[Vec<u8>], next_leaf: u64) -> Result<Vec<u8>>`
  - `fn leaf_overflows(n_entries: usize, data_bytes: usize, entry_len: usize) -> bool`
  - `fn build_internal_levels(leaf_infos: Vec<(u64, Vec<u8>)>, backend: &mut dyn StorageBackend, cache: &PageCache, next_page: u64) -> Result<(u64, u64)>` — returns `(root_page_id, next_free_page_id)`

- [ ] **Step 1: Split `write_leaf_page`.** Rename the current body (everything up to but excluding `backend.write_page`) into `encode_leaf_page` returning the page, and make `write_leaf_page` a wrapper:

```rust
/// Encode a leaf page: fixed header, slot directory, entries written end-to-start.
///
/// `entries`: each element is the postcard-serialised `(K, FactRef)` bytes for
/// one index entry, in sort order.
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
fn encode_leaf_page(entries: &[Vec<u8>], next_leaf: u64) -> Result<Vec<u8>> {
    // ... existing body of write_leaf_page from `let entry_count = ...`
    //     through the slot-directory loop, unchanged ...
    Ok(page)
}

/// Write a single leaf page and insert it into the cache.
fn write_leaf_page(
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    page_id: u64,
    entries: &[Vec<u8>],
    next_leaf: u64,
) -> Result<()> {
    let page = encode_leaf_page(entries, next_leaf)?;
    backend.write_page(page_id, &page)?;
    cache.put_dirty(page_id, page);
    Ok(())
}
```

- [ ] **Step 2: Extract the fill rule.** Add below `write_internal_page`:

```rust
/// True when adding an entry of `entry_len` bytes to a leaf that already holds
/// `n_entries` entries totalling `data_bytes` would exceed the fill threshold.
#[allow(clippy::arithmetic_side_effects)]
fn leaf_overflows(n_entries: usize, data_bytes: usize, entry_len: usize) -> bool {
    LEAF_HEADER_SIZE + (n_entries + 1) * SLOT_SIZE + data_bytes + entry_len > PAGE_FILL_BYTES
}
```

and in `build_btree` phase 1 replace

```rust
        let projected = LEAF_HEADER_SIZE
            + (cur_entries.len() + 1) * SLOT_SIZE
            + cur_data_bytes
            + entry_bytes.len();

        if projected > PAGE_FILL_BYTES && !cur_entries.is_empty() {
```

with

```rust
        if !cur_entries.is_empty()
            && leaf_overflows(cur_entries.len(), cur_data_bytes, entry_bytes.len())
        {
```

- [ ] **Step 3: Extract phase 2.** Move everything in `build_btree` from `// Single leaf: it is the root` to the end of the function into:

```rust
/// Build internal levels bottom-up over `leaf_infos` (`(page_id, first_key_bytes)`
/// per leaf, in key order), writing nodes from `next_page` onward.
///
/// Returns `(root_page_id, next_free_page_id)`. A single leaf is its own root.
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
fn build_internal_levels(
    leaf_infos: Vec<(u64, Vec<u8>)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    next_page: u64,
) -> Result<(u64, u64)> {
    if leaf_infos.is_empty() {
        bail_coded!(
            ErrorCode::Int049,
            "build_internal_levels: no leaves".to_string()
        );
    }
    let mut next_page = next_page;
    let mut current_level = leaf_infos;
    loop {
        // ... existing phase-2 loop body, unchanged (it already returns
        //     `(current_level[0].0, next_page)` when current_level.len() == 1) ...
    }
}
```

Delete the now-redundant "Single leaf: it is the root" block (the loop handles `len() == 1`). End `build_btree` with:

```rust
    build_internal_levels(leaf_infos, backend, cache, next_page)
```

- [ ] **Step 4: Run the B+tree and storage tests**

Run: `cargo test --lib storage::`
Expected: all PASS (no behaviour change).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/storage/btree_v6.rs
git commit -m "refactor(btree_v6): extract leaf encoding, fill rule, internal-level build (#315)"
```

---

### Task 3: `collect_leaf_pages` and `rebuild_btree_incremental`

**Files:**
- Modify: `src/storage/btree_v6.rs` (new section after `merge_sorted_vecs`; tests in the existing `mod tests`)

**Interfaces:**
- Consumes (Task 2): `encode_leaf_page`, `leaf_overflows`, `build_internal_levels`; existing `build_btree`, `btree_entries`, `merge_sorted_vecs`, `read_leaf_entries`, `find_leftmost_leaf`, `read_u16_at`, `read_u64_at`.
- Produces (used by Task 4):
  - `pub fn collect_leaf_pages(root_page_id: u64, backend: &dyn StorageBackend, cache: &PageCache) -> Result<Vec<Arc<Vec<u8>>>>`
  - `pub fn rebuild_btree_incremental<K>(old_leaves: Vec<Arc<Vec<u8>>>, pending: Vec<(K, FactRef)>, backend: &mut dyn StorageBackend, cache: &PageCache, start_page_id: u64) -> Result<(u64, u64)> where K: Serialize + for<'de> Deserialize<'de> + Ord` — `pending` must be sorted; returns `(root_page_id, next_free_page_id)`.

- [ ] **Step 1: Write the failing tests** in `mod tests` (add `use std::sync::Arc;` if not already imported there):

```rust
    /// Deterministic xorshift64 so randomized tests are reproducible without deps.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    /// `n` EAVT entries with random entities; `counter` keeps keys unique.
    fn random_eavt(rng: &mut Rng, n: usize, counter: &mut u64) -> Vec<(EavtKey, FactRef)> {
        let mut v: Vec<(EavtKey, FactRef)> = (0..n)
            .map(|_| {
                *counter += 1;
                let entity = (u128::from(rng.next()) << 64) | u128::from(*counter);
                make_eavt(entity, ":attr", *counter)
            })
            .collect();
        v.sort();
        v
    }

    fn sorted_union(
        a: &[(EavtKey, FactRef)],
        b: &[(EavtKey, FactRef)],
    ) -> Vec<(EavtKey, FactRef)> {
        let mut v: Vec<_> = a.iter().chain(b.iter()).cloned().collect();
        v.sort();
        v
    }

    /// Build `committed` at page 1, then rebuild with `pending` starting at `start`.
    fn build_then_incremental(
        committed: &[(EavtKey, FactRef)],
        pending: &[(EavtKey, FactRef)],
        start: u64,
    ) -> (MemoryBackend, PageCache, u64, u64) {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let ser = btree_entries(committed.iter().cloned()).unwrap();
        let (old_root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        let leaves = collect_leaf_pages(old_root, &backend, &cache).unwrap();
        let (root, next_free) =
            rebuild_btree_incremental(leaves, pending.to_vec(), &mut backend, &cache, start)
                .unwrap();
        (backend, cache, root, next_free)
    }

    /// Stream equality, a strictly ascending next_leaf chain of non-empty leaves,
    /// and range_scan agreement on random bounds.
    fn assert_tree_exact(
        root: u64,
        backend: &MemoryBackend,
        cache: &PageCache,
        expected: &[(EavtKey, FactRef)],
        rng: &mut Rng,
    ) {
        let got: Vec<(EavtKey, FactRef)> = stream_all_entries(root, backend, cache).unwrap();
        assert_eq!(got.len(), expected.len(), "entry count differs");
        assert!(got == expected, "streamed entries differ from expected");

        let leaves = collect_leaf_pages(root, backend, cache).unwrap();
        if expected.is_empty() {
            assert_eq!(leaves.len(), 1, "empty tree is one empty leaf");
        } else {
            for page in &leaves {
                assert!(read_u16_at(&page[..], 2).unwrap() > 0, "empty leaf in tree");
            }
        }

        for _ in 0..20 {
            if expected.is_empty() {
                break;
            }
            let a = (rng.next() as usize) % expected.len();
            let b = (rng.next() as usize) % expected.len();
            let (lo, hi) = (a.min(b), a.max(b));
            let start = &expected[lo].0;
            let end = &expected[hi].0;
            let want: Vec<FactRef> = expected[lo..hi].iter().map(|(_, r)| *r).collect();
            let refs = range_scan(root, start, Some(end), backend, cache).unwrap();
            assert!(refs == want, "range_scan differs from expected slice");
        }
    }

    #[test]
    fn test_incremental_no_pending_copies_leaves_verbatim() {
        let mut rng = Rng(0x315);
        let mut ctr = 0;
        let committed = random_eavt(&mut rng, 2000, &mut ctr);
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let ser = btree_entries(committed.iter().cloned()).unwrap();
        let (old_root, old_next) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        let old_leaves = collect_leaf_pages(old_root, &backend, &cache).unwrap();
        let (root, _) = rebuild_btree_incremental(
            old_leaves.clone(),
            Vec::new(),
            &mut backend,
            &cache,
            old_next,
        )
        .unwrap();
        let new_leaves = collect_leaf_pages(root, &backend, &cache).unwrap();
        assert_eq!(new_leaves.len(), old_leaves.len(), "leaf count changed");
        for (o, n) in old_leaves.iter().zip(new_leaves.iter()) {
            assert!(o[..4] == n[..4], "leaf header prefix changed");
            assert!(o[12..] == n[12..], "leaf body changed");
        }
        assert_tree_exact(root, &backend, &cache, &committed, &mut rng);
    }

    #[test]
    fn test_incremental_matches_merge_random() {
        let mut rng = Rng(0xC0FFEE);
        let mut ctr = 0;
        for round in 0..40 {
            let committed = random_eavt(&mut rng, (rng.next() % 3000) as usize, &mut ctr);
            let pending = random_eavt(&mut rng, (rng.next() % 200) as usize, &mut ctr);
            let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
            let expected = sorted_union(&committed, &pending);
            assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
            let _ = round;
        }
    }

    #[test]
    fn test_incremental_pending_outside_old_range() {
        let mut rng = Rng(7);
        let committed: Vec<_> = (1000u128..3000).map(|n| make_eavt(n, ":a", n as u64)).collect();
        let pending = vec![
            make_eavt(1, ":a", 1),
            make_eavt(2, ":a", 2),
            make_eavt(9_000, ":a", 9_000),
            make_eavt(9_001, ":a", 9_001),
        ];
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
        assert_tree_exact(root, &backend, &cache, &sorted_union(&committed, &pending), &mut rng);
    }

    #[test]
    fn test_incremental_many_pending_into_one_leaf_splits() {
        let mut rng = Rng(11);
        // Committed entities are 0, 10_000, 20_000, ...: pending 0 < e < 10_000 all route to leaf 0.
        let committed: Vec<_> = (0u128..2000)
            .map(|n| make_eavt(n * 10_000, ":a", n as u64 + 1))
            .collect();
        let pending: Vec<_> = (1u128..3000)
            .map(|n| make_eavt(n, ":a", 100_000 + n as u64))
            .collect();
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 5000);
        assert_tree_exact(root, &backend, &cache, &sorted_union(&committed, &pending), &mut rng);
    }

    #[test]
    fn test_incremental_empty_old_tree_falls_back_to_bulk_build() {
        let mut rng = Rng(13);
        let mut ctr = 0;
        let pending = random_eavt(&mut rng, 500, &mut ctr);
        let (backend, cache, root, _) = build_then_incremental(&[], &pending, 5000);
        assert_tree_exact(root, &backend, &cache, &pending, &mut rng);
        // Nothing old, nothing new: still a valid single empty leaf.
        let (backend, cache, root, next) = build_then_incremental(&[], &[], 5000);
        assert_eq!(next, root + 1, "empty result is one page");
        assert_tree_exact(root, &backend, &cache, &[], &mut rng);
    }

    #[test]
    fn test_incremental_new_tree_overlaps_old_pages() {
        // save() writes the new tree over the old tree's pages; old leaves are
        // snapshotted first, so the result must still be exact.
        let mut rng = Rng(17);
        let mut ctr = 0;
        let committed = random_eavt(&mut rng, 3000, &mut ctr);
        let pending = random_eavt(&mut rng, 100, &mut ctr);
        let (backend, cache, root, _) = build_then_incremental(&committed, &pending, 3);
        assert_tree_exact(root, &backend, &cache, &sorted_union(&committed, &pending), &mut rng);
    }

    #[test]
    fn test_incremental_repeated_rounds() {
        // Each round consumes the previous round's incrementally built tree,
        // writing the new tree at the same start page like save() does.
        let mut rng = Rng(19);
        let mut ctr = 0;
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(4096);
        let mut expected = random_eavt(&mut rng, 1500, &mut ctr);
        let ser = btree_entries(expected.iter().cloned()).unwrap();
        let (mut root, _) = build_btree(ser.into_iter(), &mut backend, &cache, 1).unwrap();
        for round in 0..30u64 {
            let pending = random_eavt(&mut rng, 1 + (rng.next() % 150) as usize, &mut ctr);
            let leaves = collect_leaf_pages(root, &backend, &cache).unwrap();
            let start = 1 + round % 3;
            let (r, _) =
                rebuild_btree_incremental(leaves, pending.clone(), &mut backend, &cache, start)
                    .unwrap();
            root = r;
            expected = sorted_union(&expected, &pending);
            assert_tree_exact(root, &backend, &cache, &expected, &mut rng);
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib btree_v6::tests::test_incremental`
Expected: compile error — `collect_leaf_pages` / `rebuild_btree_incremental` not found.

- [ ] **Step 3: Implement.** Add after `merge_sorted_vecs`:

```rust
// ─── Incremental rebuild (#315) ───────────────────────────────────────────────

/// Collect the raw leaf pages of the B+tree at `root_page_id`, in key order.
///
/// `save()` calls this before writing anything: new fact pages overwrite the
/// start of the old index region, so the old leaves must be snapshotted first.
pub fn collect_leaf_pages(
    root_page_id: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<Arc<Vec<u8>>>> {
    let mut leaves = Vec::new();
    let mut leaf_id = find_leftmost_leaf(root_page_id, backend, cache)?;
    loop {
        let page = cache.get_or_load(leaf_id, backend)?;
        if page.first().copied() != Some(PAGE_TYPE_LEAF) {
            bail_coded!(
                ErrorCode::Int049,
                format!("collect_leaf_pages: expected leaf page at page_id={leaf_id}")
            );
        }
        let next_leaf = read_u64_at(&page[..], 4)?;
        leaves.push(page);
        if next_leaf == 0 {
            break;
        }
        leaf_id = next_leaf;
    }
    Ok(leaves)
}

/// Decode the key of a leaf's first entry; `None` for an empty leaf.
#[allow(clippy::arithmetic_side_effects)]
fn leaf_first_key<K>(page: &[u8]) -> Result<Option<K>>
where
    K: for<'de> Deserialize<'de>,
{
    if read_u16_at(page, 2)? == 0 {
        return Ok(None);
    }
    let offset = read_u16_at(page, LEAF_HEADER_SIZE)? as usize;
    let length = read_u16_at(page, LEAF_HEADER_SIZE + 2)? as usize;
    let slice = page
        .get(offset..offset.saturating_add(length))
        .ok_or_else(|| {
            err_coded!(
                ErrorCode::Int049,
                format!("first entry out of bounds: offset={offset} len={length}")
            )
        })?;
    let (key, _): (K, FactRef) = postcard::from_bytes(slice)?;
    Ok(Some(key))
}

/// Writes leaf pages at consecutive page ids, linking each to the next.
///
/// One page is held back so its `next_leaf` can be set to the following page
/// id, or to 0 once it is known to be the last leaf.
struct LeafEmitter<'a> {
    backend: &'a mut dyn StorageBackend,
    cache: &'a PageCache,
    next_page: u64,
    held: Option<(u64, Vec<u8>)>,
    infos: Vec<(u64, Vec<u8>)>,
}

impl LeafEmitter<'_> {
    #[allow(clippy::arithmetic_side_effects)]
    fn push(&mut self, page: Vec<u8>, first_key_bytes: Vec<u8>) -> Result<()> {
        let page_id = self.next_page;
        self.next_page += 1;
        self.release_held(page_id)?;
        self.held = Some((page_id, page));
        self.infos.push((page_id, first_key_bytes));
        Ok(())
    }

    fn release_held(&mut self, next_leaf: u64) -> Result<()> {
        if let Some((page_id, mut page)) = self.held.take() {
            page.get_mut(4..12)
                .ok_or_else(|| {
                    err_coded!(
                        ErrorCode::Int049,
                        "page too small to write next_leaf".to_string()
                    )
                })?
                .copy_from_slice(&next_leaf.to_le_bytes());
            self.backend.write_page(page_id, &page)?;
            self.cache.put_dirty(page_id, page);
        }
        Ok(())
    }

    /// Write the last held leaf and return `(leaf_infos, next_free_page_id)`.
    fn finish(mut self) -> Result<(Vec<(u64, Vec<u8>)>, u64)> {
        self.release_held(0)?;
        Ok((self.infos, self.next_page))
    }
}

/// Serialise sorted entries and emit them as leaves using `build_btree`'s fill rule.
#[allow(clippy::arithmetic_side_effects)]
fn emit_packed<K: Serialize>(
    emitter: &mut LeafEmitter<'_>,
    entries: impl Iterator<Item = (K, FactRef)>,
) -> Result<()> {
    let mut cur: Vec<Vec<u8>> = Vec::new();
    let mut cur_bytes = 0usize;
    let mut cur_first: Option<Vec<u8>> = None;
    for (key, fact_ref) in entries {
        let entry = postcard::to_allocvec(&(&key, &fact_ref))?;
        if !cur.is_empty() && leaf_overflows(cur.len(), cur_bytes, entry.len()) {
            let first = cur_first.take().ok_or_else(|| {
                err_coded!(ErrorCode::Int049, "BUG: leaf without first key".to_string())
            })?;
            emitter.push(encode_leaf_page(&cur, 0)?, first)?;
            cur.clear();
            cur_bytes = 0;
        }
        if cur_first.is_none() {
            cur_first = Some(postcard::to_allocvec(&key)?);
        }
        cur_bytes += entry.len();
        cur.push(entry);
    }
    if let Some(first) = cur_first {
        emitter.push(encode_leaf_page(&cur, 0)?, first)?;
    }
    Ok(())
}

/// Rebuild a B+tree from its old leaves plus sorted `pending` entries.
///
/// Leaves that receive no pending entry are copied verbatim (only `next_leaf`
/// is patched); only leaves that do are decoded, merged and repacked. Internal
/// levels are rebuilt from scratch. Pending keys route to leaf `i` when
/// `first_key[i] <= key < first_key[i + 1]`; keys below the first leaf's first
/// key go to leaf 0. With no non-empty old leaves this is a plain bulk build.
///
/// `old_leaves` must be snapshotted (see [`collect_leaf_pages`]) before any
/// page in `start_page_id..` is written. Returns `(root_page_id, next_free_page_id)`.
pub fn rebuild_btree_incremental<K>(
    old_leaves: Vec<Arc<Vec<u8>>>,
    pending: Vec<(K, FactRef)>,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
    start_page_id: u64,
) -> Result<(u64, u64)>
where
    K: Serialize + for<'de> Deserialize<'de> + Ord,
{
    let mut leaves: Vec<(Arc<Vec<u8>>, K)> = Vec::with_capacity(old_leaves.len());
    for page in old_leaves {
        if let Some(first_key) = leaf_first_key::<K>(&page[..])? {
            leaves.push((page, first_key));
        }
    }
    if leaves.is_empty() {
        return build_btree(
            btree_entries(pending.into_iter())?.into_iter(),
            backend,
            cache,
            start_page_id,
        );
    }

    let mut emitter = LeafEmitter {
        backend: &mut *backend,
        cache,
        next_page: start_page_id,
        held: None,
        infos: Vec::new(),
    };
    let mut pending = pending.into_iter().peekable();
    for (i, (page, first_key)) in leaves.iter().enumerate() {
        let upper = leaves.get(i.saturating_add(1)).map(|(_, k)| k);
        let mut batch = Vec::new();
        while let Some((key, _)) = pending.peek() {
            if upper.is_some_and(|u| key >= u) {
                break;
            }
            if let Some(entry) = pending.next() {
                batch.push(entry);
            }
        }
        if batch.is_empty() {
            emitter.push((**page).clone(), postcard::to_allocvec(first_key)?)?;
        } else {
            let old = read_leaf_entries::<K>(&page[..])?;
            emit_packed(&mut emitter, merge_sorted_vecs(old, batch))?;
        }
    }
    let (leaf_infos, next_page) = emitter.finish()?;
    build_internal_levels(leaf_infos, backend, cache, next_page)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib btree_v6::`
Expected: all PASS, including the seven `test_incremental_*`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/storage/btree_v6.rs
git commit -m "feat(btree_v6): incremental rebuild that copies untouched leaves (#315)"
```

---

### Task 4: Wire into `save()` and cache the fact-page checksum prefix

**Files:**
- Modify: `src/storage/persistent_facts.rs` — struct (~line 145), `new` (~line 166), `load` checksum block (~lines 277-301), `save` Steps A/D/checksum (~lines 842-970), `compute_page_checksum` (~line 1102), imports (lines 10-13), tests module

**Interfaces:**
- Consumes (Task 3): `collect_leaf_pages`, `rebuild_btree_incremental`.
- Produces: field `fact_prefix_crc: Option<(u64, Hasher)>`; `fn hash_pages(backend: &dyn StorageBackend, hasher: &mut Hasher, first_page: u64, num_pages: u64) -> Result<()>`.

- [ ] **Step 1: Write the failing tests** in the `#[cfg(test)] mod tests` of `persistent_facts.rs`:

```rust
    /// All four on-disk indexes equal a from-scratch derivation from the fact pages,
    /// and the stored checksum equals a full pass over pages 1..page_count.
    fn assert_indexes_and_checksum_exact<B: StorageBackend + 'static>(
        pfs: &PersistentFactStorage<B>,
    ) {
        let backend = pfs.backend.lock().unwrap();
        let header = FileHeader::from_bytes(&backend.read_page(0).unwrap()).unwrap();
        let full = compute_page_checksum(&*backend, 1, header.page_count - 1).unwrap();
        assert_eq!(header.index_checksum, full, "stored checksum != full pass");

        let (facts, refs) = crate::storage::packed_pages::read_all_with_refs(
            &*backend,
            1,
            header.fact_page_count,
        )
        .unwrap();
        let (eavt, aevt, avet, vaet) = build_sorted_index_entries(&facts, &refs);
        let cache = &pfs.page_cache;
        let got_eavt: Vec<(EavtKey, FactRef)> =
            stream_all_entries(header.eavt_root_page, &*backend, cache).unwrap();
        let got_aevt: Vec<(AevtKey, FactRef)> =
            stream_all_entries(header.aevt_root_page, &*backend, cache).unwrap();
        let got_avet: Vec<(AvetKey, FactRef)> =
            stream_all_entries(header.avet_root_page, &*backend, cache).unwrap();
        let got_vaet: Vec<(VaetKey, FactRef)> =
            stream_all_entries(header.vaet_root_page, &*backend, cache).unwrap();
        assert!(got_eavt == eavt, "EAVT differs from full derivation");
        assert!(got_aevt == aevt, "AEVT differs from full derivation");
        assert!(got_avet == avet, "AVET differs from full derivation");
        assert!(got_vaet == vaet, "VAET differs from full derivation");
    }

    /// One transact of `n` facts: new and reused entities, strings and refs (VAET).
    fn transact_mixed<B: StorageBackend + 'static>(
        pfs: &mut PersistentFactStorage<B>,
        entities: &mut Vec<Uuid>,
        n: usize,
        seed: u64,
    ) {
        let mut batch = Vec::new();
        for i in 0..n {
            let k = seed.wrapping_mul(31).wrapping_add(i as u64);
            let e = if entities.is_empty() || k % 3 == 0 {
                let e = Uuid::new_v4();
                entities.push(e);
                e
            } else {
                entities[(k as usize) % entities.len()]
            };
            let value = if k % 4 == 0 {
                Value::Ref(entities[(k as usize / 4) % entities.len()])
            } else {
                Value::String(format!("v{k}"))
            };
            batch.push((e, format!(":a{}", k % 5), value));
        }
        pfs.storage().transact(batch, None).unwrap();
        pfs.mark_dirty();
    }

    #[test]
    fn test_incremental_saves_keep_indexes_and_checksum_exact() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 256).unwrap();
        let mut entities = Vec::new();
        for round in 0..25u64 {
            let n = 1 + ((round * 37) % 60) as usize;
            transact_mixed(&mut pfs, &mut entities, n, round);
            pfs.save().unwrap();
            assert!(pfs.fact_prefix_crc.is_some(), "prefix cached after save");
            assert_indexes_and_checksum_exact(&pfs);
        }
    }

    #[test]
    fn test_reopen_after_incremental_saves_takes_no_rebuild() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let mut entities = Vec::new();
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            for round in 0..10u64 {
                transact_mixed(&mut pfs, &mut entities, 40, round);
                pfs.save().unwrap();
            }
        }
        let mut pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
        // Set only when the full checksum matched on load, i.e. no rebuild.
        assert!(pfs.fact_prefix_crc.is_some(), "full checksum must match on reopen");
        transact_mixed(&mut pfs, &mut entities, 40, 99);
        pfs.save().unwrap();
        assert_indexes_and_checksum_exact(&pfs);
    }

    #[test]
    fn test_prefix_cache_page_count_mismatch_falls_back() {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 256).unwrap();
        let mut entities = Vec::new();
        transact_mixed(&mut pfs, &mut entities, 50, 1);
        pfs.save().unwrap();
        pfs.fact_prefix_crc = Some((999, Hasher::new()));
        transact_mixed(&mut pfs, &mut entities, 50, 2);
        pfs.save().unwrap();
        assert_indexes_and_checksum_exact(&pfs);
    }

    #[test]
    fn test_prefix_cache_absent_after_rebuild_on_open() {
        use crate::storage::backend::FileBackend;
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let mut entities = Vec::new();
        {
            let mut pfs =
                PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
            for round in 0..3u64 {
                transact_mixed(&mut pfs, &mut entities, 30, round);
                pfs.save().unwrap();
            }
        }
        // Corrupt index_checksum (re-sealing the header) to force the rebuild path.
        {
            let mut backend = FileBackend::open(&path).unwrap();
            let mut page = backend.read_page(0).unwrap();
            page[64] ^= 0xFF;
            let cs = compute_header_checksum_from_bytes(&page);
            page[80..84].copy_from_slice(&cs.to_le_bytes());
            backend.write_page(0, &page).unwrap();
            backend.sync().unwrap();
        }
        let mut pfs = PersistentFactStorage::new(FileBackend::open(&path).unwrap(), 256).unwrap();
        assert!(pfs.fact_prefix_crc.is_none(), "no prefix after rebuild-on-open");
        transact_mixed(&mut pfs, &mut entities, 30, 7);
        pfs.save().unwrap();
        assert_indexes_and_checksum_exact(&pfs);
    }
```

Add any missing test-module imports (`FileHeader`, `StorageBackend`, `MemoryBackend`, `Value`, `Uuid`, `Hasher`, `EavtKey`/`AevtKey`/`AvetKey`/`VaetKey`, `FactRef`, `stream_all_entries`) — most are already imported via `use super::*;`; the compiler will list the rest.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib persistent_facts::tests::test_ -- incremental prefix reopen_after`
Expected: compile error — no field `fact_prefix_crc`.

- [ ] **Step 3: Add the field.** In `PersistentFactStorage`:

```rust
    committed_fact_pages: Arc<AtomicU64>,
    /// CRC32 state after hashing fact pages `1..=n` (`n` is the first element).
    /// Fact pages are never rewritten, so `save()` extends this instead of
    /// re-reading them; `None` (or a stale `n`) falls back to a full pass (#315).
    fact_prefix_crc: Option<(u64, Hasher)>,
```

and `fact_prefix_crc: None,` in the constructor literal in `new`.

- [ ] **Step 4: Add `hash_pages`** and re-express `compute_page_checksum` with it:

```rust
/// Feed pages `first_page..first_page + num_pages` into `hasher`.
fn hash_pages(
    backend: &dyn StorageBackend,
    hasher: &mut Hasher,
    first_page: u64,
    num_pages: u64,
) -> Result<()> {
    for i in 0..num_pages {
        let page_id = first_page
            .checked_add(i)
            .ok_or_else(|| err_coded!(ErrorCode::Stg021))?;
        hasher.update(&backend.read_page(page_id)?);
    }
    Ok(())
}

fn compute_page_checksum(
    backend: &dyn StorageBackend,
    first_page: u64,
    num_pages: u64,
) -> Result<u32> {
    let mut hasher = Hasher::new();
    hash_pages(backend, &mut hasher, first_page, num_pages)?;
    Ok(hasher.finalize())
}
```

- [ ] **Step 5: Capture the prefix in `load()`.** In the `needs_rebuild` block replace

```rust
            let full_checksum = compute_page_checksum(&*backend, 1, total_data_pages)?;
            if full_checksum == stored {
                false // new-style checksum matches: facts + indexes verified
```

with

```rust
            // Hash fact pages, keep that state for save() (#315), then the index pages.
            let (full_checksum, fact_prefix) = if total_data_pages >= num_fact_pages {
                let mut hasher = Hasher::new();
                hash_pages(&*backend, &mut hasher, 1, num_fact_pages)?;
                let prefix = hasher.clone();
                let index_start = num_fact_pages
                    .checked_add(1)
                    .ok_or_else(|| err_coded!(ErrorCode::Stg021))?;
                hash_pages(
                    &*backend,
                    &mut hasher,
                    index_start,
                    total_data_pages - num_fact_pages,
                )?;
                (hasher.finalize(), Some(prefix))
            } else {
                (compute_page_checksum(&*backend, 1, total_data_pages)?, None)
            };
            if full_checksum == stored {
                self.fact_prefix_crc = fact_prefix.map(|p| (num_fact_pages, p));
                false // new-style checksum matches: facts + indexes verified
```

(The subtraction is guarded by the `>=` check; add `#[allow(clippy::arithmetic_side_effects)]` only if clippy asks.)

- [ ] **Step 6: Step A of `save()`** — replace the four `committed_* = stream_all_entries(...)` blocks with raw-leaf snapshots:

```rust
        // Snapshot old B+tree leaves BEFORE writing new pages that may overlap them.
        let old_leaves = |root: u64| -> Result<Vec<Arc<Vec<u8>>>> {
            if root == 0 {
                Ok(Vec::new())
            } else {
                collect_leaf_pages(root, &*backend, &self.page_cache)
            }
        };
        let old_eavt = old_leaves(curr_header.eavt_root_page)?;
        let old_aevt = old_leaves(curr_header.aevt_root_page)?;
        let old_avet = old_leaves(curr_header.avet_root_page)?;
        let old_vaet = old_leaves(curr_header.vaet_root_page)?;
```

(If the closure's borrow of `backend` conflicts with the later `&mut *backend`, it ends at its last use; if the borrow checker still objects, inline the four `if` expressions.)

- [ ] **Step 7: Step D of `save()`** — replace the four `*_ser` / `build_btree` pairs with:

```rust
        let (eavt_root, next1) = rebuild_btree_incremental(
            old_eavt,
            pending_eavt,
            &mut *backend,
            &self.page_cache,
            index_start,
        )?;
        let (aevt_root, next2) =
            rebuild_btree_incremental(old_aevt, pending_aevt, &mut *backend, &self.page_cache, next1)?;
        let (avet_root, next3) =
            rebuild_btree_incremental(old_avet, pending_avet, &mut *backend, &self.page_cache, next2)?;
        let (vaet_root, next4) =
            rebuild_btree_incremental(old_vaet, pending_vaet, &mut *backend, &self.page_cache, next3)?;
```

- [ ] **Step 8: Checksum in `save()`** — replace

```rust
        let total_data_pages = next4.saturating_sub(1);
        let checksum = compute_page_checksum(&*backend, 1, total_data_pages)?;
```

with

```rust
        // Fact pages 1..=old count are unchanged since the last save/load, so extend
        // the cached CRC state instead of re-reading them (#315). Taking the cache
        // means a failed save leaves None and the next save does a full pass.
        let mut hasher = match self.fact_prefix_crc.take() {
            Some((n, prefix)) if n == old_fact_page_count => {
                let mut h = prefix;
                hash_pages(&*backend, &mut h, new_fact_start, new_pages_len)?;
                h
            }
            _ => {
                let mut h = Hasher::new();
                hash_pages(&*backend, &mut h, 1, new_total_fact_pages)?;
                h
            }
        };
        let new_prefix = hasher.clone();
        hash_pages(
            &*backend,
            &mut hasher,
            index_start,
            next4.saturating_sub(index_start),
        )?;
        let checksum = hasher.finalize();
```

and after `self.committed_fact_pages.store(new_total_fact_pages, ...)` add:

```rust
        self.fact_prefix_crc = Some((new_total_fact_pages, new_prefix));
```

- [ ] **Step 9: Fix imports** — add `collect_leaf_pages, rebuild_btree_incremental` to the `use crate::storage::btree_v6::{...}` list; drop names clippy reports unused in non-test code (e.g. `merge_sorted_vecs`; `stream_all_entries` if only tests use it — then import it inside `mod tests`).

- [ ] **Step 10: Run the tests**

Run: `cargo test --lib persistent_facts::` then `cargo test`
Expected: all PASS; total = baseline 1168 + 7 (Task 3) + 4 (this task) = 1179 passing, 8 ignored.

- [ ] **Step 11: Measure** — `cargo bench --bench minigraf_bench -- checkpoint/after_1_fact`. Expected: substantially below the Task 1 baseline. Also rerun the scratch harness if still present (50k facts target ≤ 15 ms). Record numbers in the task report.

- [ ] **Step 12: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/storage/persistent_facts.rs
git commit -m "perf: checkpoint copies untouched index leaves and skips re-hashing fact pages (#315)"
```

---

### Task 5: Docs, comment trim, doc sync

**Files:**
- Modify: `src/db.rs` (`OpenOptions::wal_checkpoint_threshold` field doc ~line 112; `Minigraf::checkpoint` doc ~line 684; `do_checkpoint` guard comment ~lines 730-742)
- Modify: `docs/BENCHMARKS.md`, `CHANGELOG.md`, `CLAUDE.md`, `README.md`, `docs/TEST_COVERAGE.md`

- [ ] **Step 1: `Minigraf::checkpoint` rustdoc** — replace the first paragraph with:

```rust
    /// Manually trigger a checkpoint: flush all in-memory facts to the main file
    /// and delete the WAL sidecar.
    ///
    /// A checkpoint is **not** a durability boundary: committed writes are already
    /// crash-durable in the `<db>.wal` sidecar and are replayed on the next open.
    /// Checkpointing less often only makes that replay (reopen) slower.
    ///
    /// Cost: every checkpoint rewrites the four covering indexes after the fact
    /// pages, so it copies pages in proportion to the total index size; only the
    /// index leaves that receive new entries are decoded and re-encoded. Prefer
    /// checkpointing on a size or time budget over once per logical unit of work.
    ///
    /// No-op for in-memory databases.
```

- [ ] **Step 2: `wal_checkpoint_threshold` field doc** — append:

```rust
    ///
    /// A checkpoint's cost grows with the total size of the database (see
    /// [`Minigraf::checkpoint`]), while durability does not depend on it: the WAL
    /// is crash-durable. Raise this for write-heavy workloads on large graphs.
```

- [ ] **Step 3: Trim the `do_checkpoint` guard comment** — replace the paragraph starting "The kernel file lock taken by FileBackend::open…" through "…concurrent writers." with:

```rust
                // Kernel file locking refuses a second writer (another process, or a
                // second handle in this one — #304/#314). The guard still matters where
                // the filesystem cannot lock and the caller set `allow_unlocked`.
```

- [ ] **Step 4: `docs/BENCHMARKS.md`** — add after the "Point Query vs. Version-Chain Depth (#323)" section:

```markdown
### Checkpoint After One Dirty Fact (#315)

**Date**: <today> · **Command**: `cargo bench --bench minigraf_bench -- checkpoint/after_1_fact` · same host as above.

An already-checkpointed file database of `n` facts receives one new fact, then `checkpoint()` runs. "Before" is v2.0.1 behaviour (every index entry decoded and re-encoded); "after" copies untouched index leaves verbatim.

| Facts | Before | After |
|---:|---:|---:|
| 10k | <Task 1 median> | <Task 4 median> |
| 100k | <Task 1 median> | <Task 4 median> |

Cost still grows with graph size (index pages are copied every checkpoint); O(delta) checkpoints need copy-on-write pages and are tracked with #374 for v3.0.0.
```

Fill `<…>` with the recorded numbers — no placeholders may remain.

- [ ] **Step 5: `CHANGELOG.md`** — under `## Unreleased` → `### Performance` (create headings if absent), add:

```markdown
- **Checkpoints no longer re-encode the whole index (#315).** `checkpoint()` used to decode and re-serialise every entry of all four covering indexes and re-read every page for the file checksum, so checkpointing after one new fact cost as much as after thousands. Index leaves that receive no new entries are now copied verbatim, and unchanged fact pages are not re-hashed. On a 100k-fact file, checkpoint after one fact drops from <before> to <after>. Cost still scales with graph size in page copies; the file format is unchanged. The `checkpoint()` docs now state that a checkpoint is not a durability boundary.
```

- [ ] **Step 6: Test counts** — run `cargo test 2>&1 | grep "^test result"` and sum; update `CLAUDE.md` (`**N tests passing** (P passing, 8 ignored…)`), `README.md` (`cargo test         # run N tests`), `docs/TEST_COVERAGE.md` (`**Verified**` date, `**Result**` line, and one sentence appended to the suite summary: "Since v2.0.1, incremental B+tree rebuild tests check that checkpoints which copy untouched leaves produce exactly the indexes and checksum a full rebuild would (#315).").

- [ ] **Step 7: Verify and commit**

Run: `cargo test` and `cargo doc --no-deps 2>&1 | grep -i warning` (expect none new)

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/db.rs docs/BENCHMARKS.md CHANGELOG.md CLAUDE.md README.md docs/TEST_COVERAGE.md
git commit -m "docs: checkpoint cost and durability semantics; #315 benchmarks and doc sync"
```

- [ ] **Step 8 (at PR time, with the user's go-ahead):** comment on #374 that copy-on-write pages there also own #315's remaining O(size) page-copy term. The PR body uses "Mitigates #315" wording chosen with the user, never a closing keyword for #374.
