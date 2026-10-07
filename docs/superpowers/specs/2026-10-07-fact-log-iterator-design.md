# Fact-log iterator (#430, v3.0.0) — design

**Issue:** #430 (streaming fact-log iterator with full history and coarse filters).
**Branch:** `v3`. **Related:** #432 (cursor API, merged as `Cursor`), #429 (read-only
open), #431 (raw log writer), #462 (binding cursors), temporal_reasoning#378.

## Goal

Read every fact version in a graph (assertions and retractions, with `tx_count`, `tx_id`
and valid-time bounds) as a stream, without Datalog and with memory bounded
independently of graph size. This is the extract step of an offline migration and a
rebuild path for derived stores, such as the temporal_reasoning FTS sidecar, that does
not depend on query evaluation.

## API

```rust
impl Minigraf {
    pub fn fact_log(&self, filter: &FactFilter) -> Result<FactLog, MinigrafError>;
}

pub struct FactRecord {          // plain data, public fields
    pub entity: EntityId,
    pub attribute: String,
    pub value: Value,
    pub tx_count: u64,           // what `:as-of N` compares against
    pub tx_id: u64,              // transaction wall-clock time (Unix ms)
    pub valid_from: i64,
    pub valid_to: i64,           // i64::MAX = forever
    pub asserted: bool,          // false = retraction record
}

#[derive(Clone, Debug, Default)]
pub struct FactFilter { /* private */ }
impl FactFilter {
    pub fn new() -> Self;
    pub fn attributes<I, S>(self, idents: I) -> Self;     // exact idents
    pub fn attribute_prefix(self, prefix: &str) -> Self;  // e.g. ":ingestion/"; repeatable
    pub fn entities<I: IntoIterator<Item = EntityId>>(self, ids: I) -> Self;
    pub fn tx_range(self, range: impl RangeBounds<u64>) -> Self;
    pub fn order(self, order: FactOrder) -> Self;
    pub fn window(self, facts: usize) -> Self;            // Tx order memory bound
}

#[non_exhaustive]
pub enum FactOrder { Tx /* default */, Storage }

impl FactLog {                    // owned, Send, no lifetime
    pub fn next_batch(&mut self, max: usize) -> Result<Option<Vec<FactRecord>>, MinigrafError>;
    pub fn close(self);
}
impl Iterator for FactLog { type Item = Result<FactRecord, MinigrafError>; }
```

`FactLog` follows the shape of `Cursor`: `max == 0` is treated as 1, batches are never
empty, and `None` is the only end signal (it repeats). After an error the log is
finished.

## Decisions

1. **Order, chosen by the caller.**
   - `FactOrder::Tx` (default): ascending `tx_count`. Within one transaction the order
     is unspecified but deterministic for a given file state. Format v8 does not record
     insertion order once facts are checkpointed (EAVT key order replaces it), and
     transactions are sets, so `:as-of N` means the same for a consumer that copies the
     records in this order.
   - `FactOrder::Storage`: one pass in the order the storage reads most cheaply. Records
     still carry `tx_count`. This suits consumers that sort or index the records
     themselves.
2. **No tx-ordered index in v8, so Tx order is multi-pass.** The format is frozen (golden
   corpus, #391). Each pass walks the selected index ranges and reads `tx↓` and the ids
   from the key bytes, without decoding. It keeps the `window` smallest
   `(tx_count, key)` entries greater than the last one emitted, in a bounded max-heap,
   then decodes them as the consumer pulls batches. Memory is O(window) keys. The cost is
   `ceil(N / window)` key scans. The default window is 262,144, so 4M facts take 16
   scans. A transaction larger than the window is split across passes at a key boundary.
   Nothing is lost or repeated, because `(tx_count, key)` is a total order and keys are
   unique.
3. **Snapshot and checkpoint pin.** No reader may hold an on-disk reader across two
   checkpoints, because pages freed by checkpoint N are reused at N+1 (v8 spec §8.1).
   `fact_log()` briefly takes the write lock, then records the current committed reader
   and the length of the pending (uncheckpointed) fact list, and increments a pin count
   on the shared `FactStorage`. While the pin count is above zero:
   - auto-checkpoints (WAL threshold, `WriteTransaction` commit) and the checkpoint on
     drop are deferred. Writes still succeed and the WAL grows. The next write after the
     last log closes triggers the deferred auto-checkpoint.
   - explicit `checkpoint()` and `rebuild_indexes()` fail with the new **`API-013`**,
     "checkpoint deferred while {} fact log(s) are open; close them and checkpoint
     again". Nothing is written.
   - `verify()` and queries are unaffected.
   Dropping or closing the log releases the pin. Because the pending list is cleared
   only by a checkpoint and is append-only otherwise, the snapshot is exact: committed
   facts of the recorded reader, then pending facts `[0, len)`. Writes that commit after
   `fact_log()` returns are not seen. Taking the write lock at open also means a
   transaction is never seen half-applied. This works like a SQLite WAL-mode reader. The
   lock is held only during open, never for the life of the log, so the #432 decision
   "no lock held for a cursor's life" still holds.
4. **Committed before pending.** Every pending fact is newer than every committed one (an
   invariant `CommittedReader::live_facts` already relies on), and the pending list is in
   `tx_count` order with insertion order within a transaction. Both orders therefore emit
   committed facts first and pending facts after them, in list order.
5. **Filters** are optional and ANDed: the attribute filter (an exact set and prefixes,
   ORed), the entity set, and the tx range. They are applied to key bytes before any
   decoding. Range selection:
   - With entities: EAVT, one range per known entity id. Unknown UUIDs match nothing.
   - Otherwise, with exact attributes only: AEVT, one range per known attribute id.
   - Otherwise: the whole of EAVT. Attribute ids are tested against the filter once each
     (memoised; prefix tests translate the id through the dictionary).
   Not provided, from the issue:
   - **Entity ident prefix.** Keyword entities are stored as UUIDv5 hashes, so no ident
     can be recovered.
   - **Entity type.** It needs a join. Run a Datalog query for the UUIDs and pass them to
     `entities`.
   - **"Untouched ranges" for page-wise copy.** v8 trees share a dictionary and use
     copy-on-write pages, so a page range is not a self-contained unit to copy. #431
     owns the write side.
6. **`FactRecord` is new public data, not `Fact`.** `Fact` is crate-private. The record's
   field names follow `Fact`. #431's `LogWriter::append` will take the same type.
7. **Errors.** Called on a thread that holds a `WriteTransaction`, `fact_log()` fails
   with `INT-001`, because opening needs the write lock, as `execute()` does. A page read
   error mid-stream is returned by `next_batch` (or as an iterator item). In-memory
   databases have only pending facts and no pin effect.
8. **Lifetime.** A log owns `Arc`s to the committed reader and the fact store. It can
   outlive the `Minigraf` handle and move threads. Because of that, the backend, and
   with it the file lock, stays open until the log is dropped.
9. **Out of scope:** the FFI cursor (`next_batch(n)` in the bindings, tracked with
   #462), read-only open (#429) and the writer (#431).

## Implementation

- `src/fact_log.rs`: `FactRecord`, `FactFilter`, `FactOrder`, `FactLog`, re-exported
  from `lib.rs`. Committed part: `Storage` keeps `(range index, last key)` and resumes
  with `LeafCursor::seek(last ‖ 0x00)`. `Tx` keeps `after: (tx, key)`, runs a pass when
  its decoded buffer is empty, and is finished when a pass returns fewer than `window`
  entries. Pending part: index into the pending list, read in chunks under the read
  guard.
- `CommittedReader::log_source() -> Option<&dyn LogSource>` (default `None`).
  `OnDiskReader` implements `LogSource`: `last_tx()` (from the meta), `eid_of`,
  `iid_of`, `ident_of`, `walk(index, prefix, after, visit)` over key bytes, and
  `decode(index, keys) -> Vec<Fact>` with one `DictReader`.
- `FactStorage`: `log_pins: Arc<AtomicUsize>`, `pin_log() -> LogPin` (decrements on
  drop), `log_pins()`, `pending_len()`, `pending_range(from, to)`, and
  `committed_reader()`.
- `Minigraf`: `do_checkpoint` returns `API-013` when pinned. Auto-checkpoint call sites
  skip when pinned. `rebuild_indexes` refuses when pinned. `Inner::drop` already ignores
  errors.
- `API-013` is registered in `src/error.rs` and `docs/ERROR_REFERENCE.md`.

## Tests (`tests/fact_log_test.rs`)

- Full history: every assertion and retraction written, with the same `tx_count`,
  `tx_id`, valid bounds and `asserted`, in non-decreasing `tx_count` order (Tx).
  `Storage` returns the same multiset.
- The same records from an in-memory DB, a file DB with everything pending, everything
  checkpointed, half checkpointed, and after reopen.
- A small `window` (2, 3) with one transaction larger than the window: no loss, no
  duplicates, Tx order kept.
- Filters on committed and pending data: exact attributes, prefix, entities, tx range,
  combinations, unknown attribute or entity (empty), empty range.
- Long string values (value pages) and explicit valid-time windows round-trip.
- Snapshot: a write after `fact_log()` is not seen. `checkpoint()` and
  `rebuild_indexes()` fail with `API-013` while a log is open, and nothing changes.
  Auto-checkpoint is deferred (the WAL still holds the entries) and happens on the first
  write after close. `checkpoint()` succeeds after `close()`.
- `next_batch(0)` makes progress, batches are bounded and non-empty, and `None` repeats.
  The iterator equals the batch output.
- `FactLog: Send`. A log can be consumed on another thread after the `Minigraf` is
  dropped.
- `INT-001` inside a `WriteTransaction` thread.
