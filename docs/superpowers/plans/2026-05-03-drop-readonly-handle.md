# Fix Drop on Read-Only Handle Writes to File (issue #226) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent `Drop` on a read-only (no-write) file-backed handle from modifying the `.graph` file.

**Architecture:** Add an early-return guard in `do_checkpoint` (`src/db.rs`) that skips the `force_dirty()` + `save()` path when `wal_entry_count == 0 && !pfs.is_dirty()`. A comment explains that file locking already prevents multi-process exposure in the common case; the guard closes the remaining same-process and lock-bypass edge cases.

**Tech Stack:** Rust, `cargo test`

---

### Task 1: Write the failing test

**Files:**
- Modify: `src/db.rs` — add test in the existing `#[cfg(all(test, not(target_arch = "wasm32")))]` mod at the bottom

- [ ] **Step 1: Write the failing test**

  Locate the `mod tests { ... }` block near the end of `src/db.rs` (around line 1074). Add this test inside it, after the last existing test:

  ```rust
  #[test]
  fn test_readonly_handle_drop_does_not_modify_file() {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("test.graph");

      // Write a fact and checkpoint so the main file is clean and WAL is gone.
      {
          let db = Minigraf::open(&path).unwrap();
          db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
          db.checkpoint().unwrap();
      }
      // First handle is dropped here; Drop triggers checkpoint again, but wal_entry_count==0
      // after the explicit checkpoint — this is already "broken" pre-fix.

      // Record file metadata before the read-only handle opens.
      let meta_before = std::fs::metadata(&path).unwrap();
      let mtime_before = meta_before.modified().unwrap();
      let len_before = meta_before.len();

      // Open a second handle, do a read-only query, then drop it.
      {
          let db2 = Minigraf::open(&path).unwrap();
          let result = db2
              .execute(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
              .unwrap();
          match result {
              QueryResult::QueryResults { results, .. } => {
                  assert_eq!(results.len(), 1, "Alice must be visible");
              }
              _ => panic!("expected QueryResults"),
          }
          // db2 dropped here — Drop must NOT write to the file
      }

      // File must be byte-for-byte identical (same mtime and size).
      let meta_after = std::fs::metadata(&path).unwrap();
      assert_eq!(
          meta_after.len(),
          len_before,
          "file size must not change after read-only handle drop"
      );
      assert_eq!(
          meta_after.modified().unwrap(),
          mtime_before,
          "file mtime must not change after read-only handle drop"
      );
  }
  ```

  > **Note:** This test requires `QueryResult` to be in scope. It is already imported at the top of the `mod tests` block via `use super::*;` which brings in everything from `db.rs`; `QueryResult` is re-exported from `crate::query::datalog::executor`. Confirm it is already used in existing tests in the same block (e.g. `test_write_transaction_read_your_own_writes`) — it is, so no new import is needed.

- [ ] **Step 2: Run the test to confirm it fails**

  ```bash
  cargo test --lib test_readonly_handle_drop_does_not_modify_file -- --nocapture 2>&1 | tail -20
  ```

  Expected output: test **FAILS** with an assertion error about `file mtime must not change` or `file size must not change`. (The exact failure depends on filesystem mtime resolution; size is the more reliable signal — pre-fix the file grows by 4 KB.)

  If the test passes already, something is wrong — do not proceed until it fails.

- [ ] **Step 3: Commit the failing test**

  ```bash
  git add src/db.rs
  git commit -m "test: add failing test for read-only handle Drop writing to file (#226)"
  ```

---

### Task 2: Implement the fix

**Files:**
- Modify: `src/db.rs:545–576` — `do_checkpoint` function, `WriteContext::File` arm

- [ ] **Step 1: Locate the exact lines to change**

  Open `src/db.rs`. Find `fn do_checkpoint`. The `WriteContext::File` arm currently reads:

  ```rust
  #[cfg(not(target_arch = "wasm32"))]
  WriteContext::File {
      pfs,
      wal,
      db_path,
      wal_entry_count,
  } => {
      // Force a full save even if no new writes since last checkpoint.
      pfs.force_dirty();
      pfs.save()?;
  ```

- [ ] **Step 2: Apply the guard**

  Replace the `WriteContext::File` arm body (from the opening `{` of the arm through `pfs.save()?;`) with:

  ```rust
  #[cfg(not(target_arch = "wasm32"))]
  WriteContext::File {
      pfs,
      wal,
      db_path,
      wal_entry_count,
  } => {
      // Skip checkpoint if nothing to flush.
      //
      // `wal_entry_count` is non-zero when this handle has made writes *or*
      // replayed WAL entries on open (crash-recovery path).  `pfs.is_dirty()`
      // catches any facts marked dirty via the normal write path.
      //
      // File locking (`.graph.lock` sidecar, acquired by FileBackend::open)
      // already prevents a second *process* from opening the file while this
      // handle holds the lock, which covers the main multi-process exposure
      // described in issue #226.  This guard closes the remaining edge cases:
      // same-process double-opens (same PID bypasses the stale-lock check)
      // and environments where the advisory lock can be bypassed (e.g.
      // network filesystems, manual lock deletion).
      if *wal_entry_count == 0 && !pfs.is_dirty() {
          return Ok(());
      }
      // `force_dirty` is needed for the WAL-replay case: facts were loaded
      // into memory during `replay_wal` but `pfs.dirty` was not set because
      // no write path was exercised.  Without it `save()` would no-op and
      // the replayed facts would never reach the main file.
      pfs.force_dirty();
      pfs.save()?;
  ```

  Leave everything after `pfs.save()?;` (WAL deletion, `wal_entry_count = 0`) unchanged.

- [ ] **Step 3: Run the previously failing test to confirm it now passes**

  ```bash
  cargo test --lib test_readonly_handle_drop_does_not_modify_file -- --nocapture 2>&1 | tail -20
  ```

  Expected: **PASS**.

- [ ] **Step 4: Run the full test suite to confirm no regressions**

  ```bash
  cargo test 2>&1 | tail -20
  ```

  Expected: all tests pass (currently 795+). If any test fails, investigate before committing.

- [ ] **Step 5: Commit the fix**

  ```bash
  git add src/db.rs
  git commit -m "fix: skip checkpoint on Drop when handle made no writes (issue #226)

  Inner::drop() called do_checkpoint() unconditionally, which called
  force_dirty() + save() even for read-only handles — growing the file
  by 4 KB and invalidating other open handles.

  Add an early-return guard in do_checkpoint: skip when wal_entry_count
  is zero and pfs is not dirty. force_dirty() is still called when the
  guard passes, preserving the crash-recovery path where WAL entries are
  replayed into memory on open (wal_entry_count > 0, dirty == false).

  File locking already prevents multi-process exposure in the common
  case; this guard closes same-process double-opens and lock-bypass
  edge cases."
  ```
