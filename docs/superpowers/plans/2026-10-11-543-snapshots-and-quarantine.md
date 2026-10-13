# Snapshots and page quarantine (#543) — Plan

Spec: `docs/superpowers/specs/2026-10-11-streaming-engine-design.md` §4 (#473, part 1 of 9).
Three PRs to `v3` (split agreed 2026-10-13; the first draft had one), each leaving
`v3` with no unused code. No closing keyword for #473, #543, #554 or #555.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1: `PendingStore` and the one-lock publish | `feat/543-snapshots` |
| 2 | 2, 3, 4, 6, 7: pins, allocator quarantine, checkpoint/rebuild entries, fact log on the pin, `API-013` deprecated | `feat/543-pins` |
| 3 | 5, 8, 9, 10: query and cursor snapshots, `BrowserDb`, crash test, docs | `feat/543-snapshots-read` |

Each PR updates the test counts and `CHANGELOG.md` for what it ships; step 10 holds
the rest of the docs.

## Already done

- `WriteTransaction::commit`, `execute` and WAL replay append a transaction with one
  `load_facts` call (#554, PR #556). Spec §4.1's first rule holds; only the checkpoint
  publish is left.
- Every lookup of one query reads under one `FactData` lock (#555, PR #557).
- `tests/snapshot_atomicity_test.rs` is on `v3`; `checkpoint_never_shows_a_fact_twice`
  is the one test still `#[ignore]`d for #543.

## Design decisions not fixed by the spec

1. **Where pins live.** A new `src/storage/pins.rs` holds `Pins` (behind
   `Arc<Mutex<..>>`): `pinned: BTreeMap<u64, usize>` (generation → open snapshots) and
   `quarantine: BTreeMap<u64, Vec<u64>>` (generation `g` → ids its checkpoint freed).
   `FactStorage` owns the `Arc<Pins>` (it replaces `log_pins`) because snapshots are
   taken there; `PersistentFactStorage` reaches it through `storage()`. Works for
   in-memory databases too (they never pin: no committed reader).
2. **Pin under the `FactData` lock.** A snapshot reads `committed`, `generation` and
   the pending length and takes its pin while holding `FactData`'s read lock. A
   checkpoint publishes (new reader, new generation, new `PendingStore`, quarantine
   entry) under `FactData`'s write lock. So a pin is always on the active generation
   at the moment it is taken, and a quarantine entry is never discarded before a pin
   that needs it exists. Lock order: `FactData` → `Pins`; `Pins` alone for
   `in_force()` and pin release. No cycle.
3. **In-force set is read once per checkpoint.** `save()` / `rebuild_indexes()` call
   `pins.in_force()` before allocating. Pins taken during the checkpoint are on the
   active generation, which no existing entry needs and which the entry this
   checkpoint creates already covers (its ids are listed free, not handed out, in this
   generation). Pins released meanwhile only shrink the set, so the stale copy is a
   safe superset.
4. **Entry discard.** On publish of generation `g`: if no generation below `g` is
   pinned, the entry is dropped at once. On pin release: drop entries with
   `g ≤ lowest pinned` (all, when nothing is pinned).
5. **`Snapshot` type.** `src/graph/storage.rs` gains
   `#[derive(Clone)] pub(crate) struct Snapshot { committed, pin: Option<Arc<GenerationPin>>, pending: Arc<PendingStore>, pending_len, now }`.
   It carries the read methods (`live_facts_of_scans`, `all_facts`, `facts_as_of`,
   `pending_range`). `FactStorage`'s read methods become `self.snapshot()?.…`, so
   every existing caller and test keeps working and each call is whole.
6. **Executor.** `DatalogExecutor` gets `snapshot: Option<Snapshot>`.
   `Minigraf::read_executor` and `PreparedQuery::query` take one snapshot and pass it;
   `filter_facts_for_query`, `live_source_facts` and `read_now` read from it (the
   overlay floor still applies). Writes keep using `FactStorage`. The executor is
   unchanged otherwise.
7. **Cursor.** `Cursor` gets a private `_snapshot: Option<Snapshot>` (`from_result`
   unchanged for `BrowserDb`'s path, a new `from_result_with(result, snapshot)` for the
   rest). The answer is still computed at open, so holding the pin buys nothing until
   #544; it is held now so the structure and the file-growth behaviour land and are
   tested with this sub-issue. Decided 2026-10-13: keep it (not dropped until #544).

## Steps

### 1. `PendingStore` and the one-lock publish
- Move `facts`, `pending_keys`, `pending_indexes` into `struct PendingStore` with its
  own `RwLock`; `FactData { committed, generation: u64, pending: Arc<PendingStore> }`.
- Appends (`transact_batch`, `retract`, `load_fact(s)`) hold `FactData`'s read lock
  and the pending write lock, so no append can land in a store after it is swapped
  out.
- Replace `post_checkpoint_clear` + `set_committed_reader` on the checkpoint path with
  `FactStorage::publish(reader, generation, quarantine_ids)`: one `FactData` write
  lock sets `committed`, `generation`, a fresh `PendingStore`, and records the entry.
- `PersistentFactStorage::activate` splits: build the reader; open/migration call
  `set_committed(reader, generation)`; `save` and `rebuild_indexes` call `publish`.
- Un-ignore `checkpoint_never_shows_a_fact_twice`; run it in a loop (it is a race
  test) before and after to see it fail, then pass.

### 2. `Pins` and `GenerationPin`
- `pins.rs` per decision 1, 4. `GenerationPin` is RAII (release on drop).
- Unit tests: entry in force while a lower generation is pinned; dropped when the pin
  goes; dropped at once with no pin; two pins on different generations release in
  either order.

### 3. `PageAllocator` skips ids in force
- `PageAllocator::with_quarantine(HashSet<u64>)`; `alloc` moves a popped id in the set
  to `skipped` and continues; `finish_free_list` lists `skipped` with the other unused
  ids. `new` filters its list the same way.
- Unit tests in `page.rs`: skipped ids are never handed out, appear in the new list,
  and the list's count matches.

### 4. Checkpoint and rebuild record quarantine
- `save()`: allocator with `pins.in_force()`; publish with `freed`.
- `rebuild_indexes()`: drop ids in force from `pool`; entry = old referenced pages
  (`keep`) minus `reached`; publish.
- Remove `refuse_while_logs_open` and the `log_pins() == 0` conditions in `execute`
  and `WriteTransaction::commit` (`db.rs:764`, `db.rs:1598`).
- `PFS` tests (`MemoryBackend`): pin generation `g`, run checkpoints that free and
  would reuse its pages (assert by page id), read the pinned reader after each and
  compare with its facts at open; drop the pin, next checkpoint reuses the ids;
  `verify()` clean throughout; same with `rebuild_indexes` (file grows by the index
  size, then reuse). `assert_space_accounted` holds with quarantined ids.

### 5. Snapshots for queries and cursors
- `Snapshot` (decision 5), `FactStorage::snapshot()`, executor (decision 6), cursor
  (decision 7), `PreparedQuery::query`.
- Test: a cursor open across writes and page-freeing checkpoints returns its answer;
  the pin holds pages (no reuse) until the cursor is dropped.

### 6. Fact log
- PR 2 adds `Snapshot` with only what the log needs (`committed`, the pin, the
  pending store and its length, `pending_range`) and `FactStorage::snapshot()`;
  step 5 adds `now`, the read methods and `Clone` (pin in an `Arc`).
- `FactLog` holds a `Snapshot` instead of `LogPin` + `FactStorage` + `pending_end`;
  pending reads go to `snapshot.pending_range`. `Minigraf::fact_log` no longer takes
  `write_lock`. Remove `LogPin`, `pin_log`, `log_pins`, `log_snapshot`.
- `tests/fact_log_test.rs`: `checkpoint_and_rebuild_are_api_013_while_a_log_is_open`
  becomes "checkpoint and rebuild proceed while a log is open, and the log still
  returns its snapshot". Add: log opened during a concurrent writer + checkpointer
  sees whole transactions only; in-memory log opens while transacting.
- Update rustdoc on `fact_log`, the `fact_log.rs` module doc and `FactLog` doc.

### 7. `API-013` deprecated
- `error.rs`: `#[allow(dead_code)]` with a comment, as for STG-003/004; keep the
  registry entries. `docs/ERROR_REFERENCE.md`: "(deprecated)" in the table, the
  deprecation note under the heading and in the deprecated-codes paragraph.
- `docs/DURABILITY.md` lines 125 and 188: logs no longer defer checkpoints; a
  long-lived log or cursor grows the file by the pages checkpoints replace meanwhile.

### 8. `BrowserDb`
- `take_released` returns `listed` minus ids in force and keeps the rest in a
  `withheld` list, returned at a later commit once out of force. Pure helper
  `split_released(listed, withheld, in_force)` with native unit tests; one
  `#[wasm_bindgen_test]`: a browser cursor/log held across two commits still reads.

### 9. Crash with an open snapshot
- `persistent_facts` crash test: open a pin, run checkpoints, crash at every write
  point (existing `crash_at_every_point_in_save` helper), reopen: every fact present,
  `verify()` clean, quarantined ids reused by the next checkpoint (no pins after
  reopen).

### 10. Docs and checks
- `CHANGELOG.md` (Unreleased, v3.0.0): fact logs no longer defer checkpoints, API-013
  deprecated, readers and checkpoints never see a half-published checkpoint (#554/#555
  v3 side), file growth with long-lived readers.
- `CLAUDE.md` (fact_log line, `pins.rs` in the module list, test count),
  `ROADMAP.md` line 84, `docs/TEST_COVERAGE.md`, `README.md` if it mentions API-013.
- Docs site PR (`../minigraf-docs`): `content/database-tools.md:46` and
  `content/architecture.md:443` wrapped in `@until v3.0.0` / `@since v3.0.0`.
- #421 entries for #554/#555: note the v3 side now fully fixed (issues stay open).
- `cargo fmt`, `cargo clippy` (and the wasm targets), `cargo test`; run
  `snapshot_atomicity_test` repeatedly; #394 envelope at 100K/1M on an idle machine
  (point queries must not regress: snapshot adds an `Arc` clone and a pin lock per
  query).
- Codex review (`codex exec -s read-only`, background, `-o`), then PR to `v3`.

## Bindings

No public API change and no new error code (API-013 stays registered), so no binding
cascade. Binding docs that mention API-013 for `fact_log` (#465/#462 work) get a
follow-up note once those land.
