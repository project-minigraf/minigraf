# Raw log writer (#431, v3.0.0) — design

**Issue:** #431 (build a fresh graph from explicit `(tx, asserted, valid-time)` records).
**Branch:** `v3`. **Related:** #430 (fact log, merged as `FactLog`/`FactRecord`), #429
(read-only open), temporal_reasoning#378 (the migration tool).

## Goal

The load step of an offline migration: write a stream of `FactRecord`s (from
`Minigraf::fact_log` on a read-only source, after a transform) into a new graph file that
keeps each record's `tx_count`, `tx_id`, `asserted` flag and valid-time window. The
result must be an ordinary file, as if the transactions had been written normally and
checkpointed. Purging data is then "do not append it".

`transact`/`retract` cannot do this: they take the next `tx_count`, stamp `tx_id` and
`valid_from` with the current time, and write a WAL entry per transaction.

## API

```rust
pub struct LogWriter { /* private */ }          // not on wasm32

impl LogWriter {
    pub fn create(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self, MinigrafError>;
    pub fn append(&mut self, rec: &FactRecord) -> Result<(), MinigrafError>;
    pub fn advance_tx_count(&mut self, tx_count: u64) -> Result<(), MinigrafError>;
    pub fn tx_count(&self) -> u64;
    pub fn finish(self) -> Result<(), MinigrafError>;
}
```

```rust
let src = Minigraf::open_with_options("old.graph", OpenOptions::new().read_only(true))?;
let mut out = LogWriter::create("new.graph", OpenOptions::new())?;
for rec in src.fact_log(&FactFilter::new())? {
    let rec = rec?;
    if !rec.attribute.starts_with(":secret/") {
        out.append(&rec)?;
    }
}
out.advance_tx_count(src.current_tx_count())?;
out.finish()?;
```

## Decisions

1. **Built beside the target, renamed into place.** `create` refuses with the new
   **`STG-043`** if `path` or `<path>.wal` exists: a migration never overwrites a
   database, and a stale WAL would be replayed onto the new file. The file is built at
   `<path>.partial`, under the usual exclusive lock. A `.partial` left by a crashed or
   abandoned build is truncated and reused (a live builder holds its lock, so a second
   `create` for the same path fails with STG-025/STG-026). `finish` commits the last
   batch, closes the file, checks again that `path` does not exist (`STG-043`), renames
   `.partial` to `path` and syncs the directory. Dropping a writer without `finish`
   deletes the `.partial`. A crash at any point therefore never leaves a file at `path`
   that holds only part of the log; the single-file rule holds for every database that
   can be opened. The check before the rename does not lock the directory: the caller
   must not create `path` while the build runs.
2. **No WAL; batched checkpoints.** Records are loaded into the in-memory pending list
   and committed with the normal copy-on-write checkpoint (`PersistentFactStorage::save`)
   whenever at least 65,536 facts are pending and a new transaction starts, and once
   more by `finish`. Memory is O(batch + largest transaction), as for normal writes, which
   also hold a whole transaction in memory. A transaction is never split across
   checkpoints. The page cache uses `opts.page_cache_size`.
3. **Transaction order.** The writer keeps a counter (`tx_count()`, starting at 0) and the
   open transaction. A record must have `tx_count > counter`, which opens a new
   transaction, or equal the open transaction's `tx_count`, with the same `tx_id`.
   Otherwise the new **`API-015`** (out of order, including `tx_count` 0 and a record for
   a transaction that is already closed) or **`API-016`** (one transaction, two `tx_id`s)
   is returned, and nothing changes. Gaps are allowed and are simply absent: `:as-of N`
   on a hole compares `tx_count <= N` and so shows the preceding state.
   `tx_id` may go backwards between transactions; the source's clock is kept as it was.
4. **Trailing holes.** `advance_tx_count(n)` closes the open transaction and raises the
   counter to `n` without facts (an empty or purged transaction). `n` below the counter
   is `API-015`. `finish` writes the counter as `last_checkpointed_tx_count`, so the new
   file's first write after a purge of the last transactions still takes the source's
   next number, and `current_tx_count()` matches the source.
5. **Checks: structure, not schema.** Per record:
   - order (`API-015`, `API-016`, above);
   - size: attribute and keyword idents and string values within the format limits
     (`WAL-003`, as for a normal write);
   - one valid-time window per `(entity, attribute, value)` assertion in one transaction
     (`API-011`, #435). Reads assume it, and a normal write refuses it the same way.
   An exact repeat of a record already in the open transaction is dropped, as a normal
   write drops a repeated fact. Nothing else is checked: `valid_from > valid_to`,
   retractions without an assertion, and attribute semantics are the caller's business;
   a migration checks its result with `verify()` and its own audit.
6. **`finish` output.** After the final checkpoint the file has every record in
   EAVT/AEVT/AVET/VAET and the dictionary, a meta at the last generation, and no WAL. It
   opens read-write or read-only, `verify()` is clean, and queries, `:as-of` and
   `fact_log()` give the same answers as a database built by `transact` with the same
   records. The tests check exactly this against #370's failure mode (indexes missing
   facts after reopen).
7. **No `copy_pages`.** The issue offered a page-range fast path "if the format permits".
   Format v8 shares one dictionary across all trees and uses copy-on-write pages, so a
   range of pages from another file is not self-contained (ids, value references and
   free-list state all differ). The fact-log design (#430, decision 5) reached the same
   conclusion. Every record goes through `append`.
8. **Errors leave the writer usable.** A rejected record changes nothing. An I/O error
   from a checkpoint inside `append` is returned and the writer should be dropped (which
   deletes the `.partial`). `opts.read_only` is `API-014`.
9. **Bindings** (`LogWriter` in the FFI, so the tool can live in the temporal_reasoning
   Python package) follow in the binding repositories, as with #429.

## Implementation

- `src/log_writer.rs`: `LogWriter` holds a `PersistentFactStorage<FileBackend>`, the
  target and partial paths, the counter, the open transaction `(tx_count, tx_id)`, and
  the open transaction's windows `HashMap<(EntityId, String, Vec<u8>), (i64, i64)>`.
  `append` converts the record to a `Fact`, runs the checks, `load_fact`s it and marks
  the storage dirty. Before each `save`, the storage's tx counter is set to the
  writer's counter. Re-exported from `lib.rs` (not on wasm32).
- `FileBackend::truncate()` resets a reused `.partial` to zero pages.
- `PersistentFactStorage::discard()` clears the dirty flag so an abandoned writer's drop
  does not commit.
- `wal::check_fact_size` becomes `pub(crate)`.
- `STG-043`, `API-015`, `API-016` in `src/error.rs` and `docs/ERROR_REFERENCE.md`.

## Tests (`tests/log_writer_test.rs`)

- Round trip: records from a source built with transacts, retracts, explicit valid-time
  windows, long strings, refs and keywords, written through `LogWriter`. The new file's
  `fact_log` equals the source's; queries (current, `:as-of N` for every N,
  `:valid-at`) match; `current_tx_count` matches; `verify()` is clean; a read-only open
  works; and after a reopen, indexed lookups by entity, attribute and value return every
  fact (#370).
- Many batches (more than one checkpoint) give the same result.
- Holes: purge one middle transaction; `:as-of` the hole equals `:as-of` the one before.
  Trailing holes with `advance_tx_count`; the next `transact` takes the following number.
- An empty log gives an empty database with the advanced counter.
- Rejections: `tx_count` 0, decreasing, a closed transaction (`API-015`), two `tx_id`s
  (`API-016`), two windows (`API-011`), oversize (`WAL-003`), `advance_tx_count` below the
  counter. Each leaves the writer usable and the output unchanged.
- Duplicate records in one transaction are written once.
- `create` refuses an existing target or WAL (`STG-043`), a second writer for the same
  path (`STG-025`), and `read_only` (`API-014`); a stale `.partial` is reused.
- Dropping without `finish` leaves no `.partial` and no target. `finish` refuses a target
  created during the build and leaves it untouched.
