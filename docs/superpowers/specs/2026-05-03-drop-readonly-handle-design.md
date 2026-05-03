# Design: Fix `Drop` on Read-Only Handles Writing to File (issue #226)

**Date:** 2026-05-03  
**Status:** Approved

## Problem

`Inner::drop()` unconditionally calls `do_checkpoint()`, which calls `pfs.force_dirty()` before `pfs.save()`. The `force_dirty()` call bypasses the `dirty` guard in `save()`, causing a file write (and 4 KB size increase) even when the handle never performed any mutations. This corrupts other open handles to the same file.

**Reproduction context:** A persistent MCP server holds handle A with active writes. A per-turn hook subprocess opens handle B, performs a read-only query, then exits (`__del__` fires `Drop`). Handle A subsequently fails with `Serde Deserialization Error` or `Page N out of bounds (total pages: N)`.

## Root Cause

`do_checkpoint` (`src/db.rs:545–576`) always calls `pfs.force_dirty()` then `pfs.save()`, regardless of whether any writes or WAL replays have occurred on this handle. The `force_dirty()` was introduced to handle the WAL-replay-on-open case (where `pfs.dirty == false` but in-memory facts exceed what's in the main file). However it has the side-effect of making every `Drop` write to the file.

## Design

### Change 1 — Early-return guard in `do_checkpoint` (`src/db.rs`)

In the `WriteContext::File` arm of `do_checkpoint`, add a guard before `force_dirty()` + `save()`:

```rust
if *wal_entry_count == 0 && !pfs.is_dirty() {
    return Ok(());
}
// force_dirty handles the WAL-replay case:
// wal_entry_count > 0 but dirty == false (facts replayed into memory, not yet on disk)
pfs.force_dirty();
pfs.save()?;
```

**Invariant preserved:** `force_dirty()` is only reachable when `wal_entry_count > 0` (writes were made or WAL was replayed on open) or `pfs.is_dirty() == true` (facts marked dirty via write path). In both cases there is genuinely something to write.

**Scenarios:**

| Scenario | `wal_entry_count` | `pfs.is_dirty()` | Result |
|---|---|---|---|
| Read-only handle, no WAL replay | 0 | false | Early return — no file write ✓ |
| Normal write handle | > 0 | true | Checkpoint proceeds ✓ |
| Crash recovery (WAL replayed on open) | > 0 | false | `force_dirty()` called, checkpoint proceeds ✓ |
| Post-checkpoint drop | 0 | false | Early return — no file write ✓ |

This fix applies uniformly to Drop, manual `checkpoint()`, and auto-checkpoint paths.

### Change 2 — Integration test (`src/db.rs` test module)

New test `test_readonly_handle_drop_does_not_modify_file`:

1. Open a file-backed DB, write one fact, call `db.checkpoint()` explicitly (flushes to main file, deletes WAL, resets `wal_entry_count` to 0).
2. Record the file's `mtime` and `len`.
3. Open a second `Minigraf` handle to the same path, execute one read-only query, drop it.
4. Assert: file `mtime` and `len` are unchanged.

## Files Changed

- `src/db.rs` — 5-line guard in `do_checkpoint`, one new test

## Non-changes

- `src/storage/persistent_facts.rs` — no changes; `force_dirty()` / `is_dirty()` / `save()` behaviour unchanged
- `src/wal.rs` — no changes
- Public API — no changes; `Minigraf::checkpoint()` signature and semantics unchanged (still a no-op when nothing to flush)
