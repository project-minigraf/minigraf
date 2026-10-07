# Read-only open (#429, v3.0.0) — design

**Issue:** #429 (`OpenOptions::read_only` for offline tooling).
**Branch:** `v3`. **Related:** #430 (fact log, merged), #431 (raw log writer),
#462 (binding cursors), temporal_reasoning#378 (offline migration).

## Goal

Open a database so that nothing done through the handle can write to the `.graph`
file or its WAL, and so that several such handles (a migration reader, a backup
step, an inspector) can hold the file at the same time.

## API

```rust
pub struct OpenOptions {            // #[non_exhaustive], so adding a field is not a break
    /* existing fields */
    pub read_only: bool,            // default false
}
impl OpenOptions {
    pub fn read_only(self, read_only: bool) -> Self;
}
```

No other API changes. `Minigraf::open_with_options(path, OpenOptions::new().read_only(true))`
and `OpenOptions::new().read_only(true).path(p).open()` both work.

## Decisions

1. **Shared lock.** A read-only open takes the kernel lock in shared mode
   (`File::try_lock_shared`); a read-write open keeps the exclusive lock. Read-only
   handles coexist, in this process or others. A writer and a reader exclude each
   other, so a read-only handle sees a file and WAL that cannot change under it. The
   writer side gets the existing STG-025/STG-026 errors with the same retry budget.
   The in-process registry (`OPEN_PATHS`) becomes a map from path to holders (one
   writer, or any number of readers) so that STG-025 keeps meaning "this process
   already holds a conflicting handle", and `allow_unlocked` on a filesystem without
   locks admits many readers or one writer, never both.
2. **No file is created or modified.** The `.graph` file is opened for reading only:
   - Missing file: new **STG-042**, "database file not found ({}); a read-only open
     does not create one". Nothing is created.
   - Empty file, or a torn initial meta (a crash during creation): an empty database.
     Nothing is initialised.
   - Torn page 0 after a v7 → v8 migration: the backup meta is used, and page 0 is not
     restored. The next read-write open restores it.
   - Format v7 file: not migrated. Its facts are read into memory, as uncheckpointed
     facts on an empty committed state, and the migration happens on the next
     read-write open. Memory is O(facts) for such a file; a v8 file is read through
     the page cache as usual.
   - WAL: read and applied in memory, exactly as a read-write open does (same skip of
     already-checkpointed entries, same tx counter restore). The WAL file is not
     opened for writing, not recreated, and not deleted.
   The backend refuses `write_page` and `sync` with INT-056 (an invariant: no code path
   reaches it), so a missed check fails closed instead of writing.
3. **Writes are refused, reads are not.** New **API-014**, "database is open
   read-only; {} is not allowed", from:
   - `execute` of `transact` or `retract` (before a tx count is allocated),
   - `begin_write`,
   - `checkpoint` and `rebuild_indexes`.
   Queries, `query` cursors, `prepare`, `fact_log`, `verify`, `current_tx_count` and
   UDF registration work. Rule registration also works: rules live in the handle's
   memory and are never written to the file.
   Dropping the handle does not checkpoint.
4. **In-memory databases.** `read_only` on `open_memory`/`in_memory_with_options`
   gives an empty database that refuses writes with API-014, the same rules as above.
   It has no use, but it is consistent and needs no special case.
5. **Page cache.** `page_cache_size` already sizes the cache that serves every
   committed page read, so it applies unchanged. A tool that wants the whole file
   resident passes `file_size / 4096` pages. Docs say so.
6. **Bindings: out of scope here.** The bindings live in their own repositories. A
   follow-up issue (like #462 for cursors) exposes `open_with_options` with
   `read_only`, `page_cache_size` and `allow_unlocked` in every binding, Python first
   (Tier 1).

## Implementation

- `FileBackend::open_with(path, allow_unlocked, mode: LockMode)` with
  `LockMode::{Exclusive, Shared}`. `Shared` opens read-only without `create`, maps
  `NotFound` to STG-042, uses `try_lock_shared`, never syncs the parent directory, and
  sets `read_only` so `write_page`/`sync` fail with INT-056.
- `OPEN_PATHS: HashMap<PathBuf, Holders>`; `PathGuard` removes its own claim.
- `PersistentFactStorage::open_read_only(backend, cache, wal_base)`: the same meta
  selection with a `read_only` flag that skips the page-0 restore and `init_empty`, and
  loads a v7 file into memory instead of migrating.
- `WriteContext::ReadOnly { pfs }` holds the backend (and with it the lock) and serves
  `verify`. `do_checkpoint` and `wal_write_stamped_batch` return API-014 for it.
  `Minigraf` checks `options.read_only` up front in `execute`, `begin_write`,
  `checkpoint` and `rebuild_indexes`. `Inner::drop` skips the checkpoint.
- API-014, STG-042 and INT-056 are registered in `src/error.rs` and
  `docs/ERROR_REFERENCE.md`.

## Tests (`tests/read_only_open_test.rs`, unit tests in `file.rs`)

- Same view: a file with checkpointed facts and uncheckpointed WAL entries returns the
  same query results, `current_tx_count` and fact log read-only as read-write.
- Nothing written: the `.graph` and `.wal` bytes and mtimes are unchanged after a
  read-only session with queries, a cursor, a fact log, `verify` and drop; no WAL is
  created when none existed.
- Refusals: `transact`, `retract`, `begin_write`, `checkpoint`, `rebuild_indexes` give
  API-014 and `current_tx_count` is unchanged; a rule and a query using it work.
- Missing file: STG-042 and no file created. Empty file: empty database, file still
  0 bytes.
- Lock: two read-only handles at once (same process); read-write while read-only is
  open is refused (STG-025) and vice versa; both work after the other drops. A child
  process holding read-only does not block a read-only open here but does block a
  read-write one (STG-026).
- v7 file (golden corpus fixture): read-only open shows the same facts as a
  read-write open, and the file bytes are unchanged (still v7).
- Read-only file permissions (Unix, skipped as root): opens and queries.
- `page_cache_size` reaches the cache in read-only mode.
- In-memory `read_only`: writes give API-014.
