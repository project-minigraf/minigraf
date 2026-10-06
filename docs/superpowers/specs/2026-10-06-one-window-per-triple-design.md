# One current valid-time window per triple — Design

**Issue:** #435, tracker #383
**Milestone:** v3.0.0, branch `v3` (no format change; changes query results)
**Builds on:** `2026-10-06-live-key-scans-design.md` (#379)

## 1. Model

For each `(e, a, v)`, transaction time N has exactly one current valid-time window: the
window of the latest assertion with `tx_count ≤ N`, provided it is newer than the
triple's last retraction. `:as-of N` selects that window; `:valid-at` is then checked
against it only. A later assertion replaces the earlier window, which stays visible
through `:as-of` of the earlier transaction. Closing, extending or reopening a window
is a `transact` with the new bounds. `retract` still withdraws the triple.

## 2. Defect

`net_asserted_facts` picked a winner per `(e, a, v, vf, vt)`, so every window ever
asserted stayed live until a retraction. `[vf, ∞)` followed by `[vf, D)` left both live.

## 3. Changes

- `net_asserted_facts` (`src/graph/storage.rs`): one map keyed by `(e, a, v)` holding
  the first latest assertion's index and `tx_count`, plus a `tied` flag set when another
  assertion has the same `tx_count`. Untied triples keep one record with no extra work.
  Tied triples (only from data written before §4) take a second pass that keeps each
  window of the latest transaction once, which also keeps the function idempotent
  under duplicated input.
- On-disk walk (`OnDiskReader::live_scan`, `src/storage/reader.rs`): the first
  transaction group of a triple at or before `as_of` decides it. A retraction in it
  hides the triple, otherwise its assertions are emitted. Every older entry is dead and
  skipped (with the existing seek after 8). The per-triple window set is gone.
- `check_one_window_per_triple`: two assertions of one triple with different
  `(vf, vt)` in one transaction are rejected with **API-011**. It runs on
  `Minigraf::execute` transacts and `WriteTransaction::commit` before a `tx_count` is
  allocated or the WAL is written, and in `FactStorage::transact`/`transact_batch`.
  Unresolved `valid_from`s all hold `VALID_FROM_USE_TX_TIME`, so repeats without bounds
  compare equal, as they will after stamping. Identical repeats and retractions are
  allowed. WAL replay, migration and `load_fact` are not checked: old data must open.

## 4. Data written before the check

v2.x and earlier v3 builds could store two windows of one triple in one transaction.
Both stay live (once each) until a later transaction of that triple. In-memory and
on-disk paths agree on this, and no open or migration fails because of it.

## 5. Tests

- Unit: replace, close, retraction-withdraws, same-transaction retraction, legacy tied
  windows; randomized histories against a per-triple model (replaces the pre-#323
  per-window oracle); API-011 accepts repeats, other values and retractions; a rejected
  `transact_batch` takes no `tx_count`.
- On disk: hand-written history (closing, same-transaction retract, legacy pair) at every
  `as_of`, before and after reopen; fails on the per-window walk.
- `tests/valid_time_window_test.rs`: the issue's table, close, extend/reopen,
  retract-then-reassert, API-011 from `transact` and `WriteTransaction`, in memory,
  checkpointed, and reopened, through EAVT and AEVT queries.
- Revised: `bitemporal_test::test_same_eav_later_valid_time_interval_replaces_earlier`,
  `multi_value_test::multi_value_time_stints_visible_at_correct_valid_time_on_every_path`.
