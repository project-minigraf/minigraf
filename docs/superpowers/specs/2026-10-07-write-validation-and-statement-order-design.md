# Valid-time window validation and statement order in transactions — Design

**Issues:** #436, #477 (broadened), tracker #383
**Milestone:** v3.0.0, branch `v3` (no format change; writes that succeed today can fail)
**Builds on:** `2026-10-06-one-window-per-triple-design.md` (#435)

## 1. Rules

1. **Empty or inverted windows are rejected (#436).** A `transact` whose effective
   window for any fact has `valid_to <= valid_from` fails with the new **API-019** and
   writes nothing. "Effective" means after the defaults are applied: a fact's own
   bound, else the transaction's, else `valid_from` = the transaction time and
   `valid_to` = forever. So `(transact {:valid-to "2020-01-01"} ...)` is rejected,
   because its `valid_from` is now.
2. **Within one `(transact [...])`**, the same `(e, a, v)` with two different windows is
   still **API-011**. Identical repeats are allowed and stored once. Duplicate triples in
   one `retract` are allowed.
3. **Across statements of one `WriteTransaction`**, the last statement that writes an
   `(e, a, v)` decides it, whether it is a `transact` or a `retract` (#477). At commit,
   every record of a triple from an earlier statement is dropped. One `tx_id` cannot tell
   several versions of a fact apart, so the committed transaction holds each triple's
   records from one statement only. Reads inside the transaction already follow statement
   order, so they now agree with the committed result. This replaces API-011's
   cross-statement rejection.
4. **`LogWriter` keeps the same rules.** Every file it builds satisfies rules 1–3, as if
   written normally: an inverted window is API-019, a second window of one triple in one
   transaction API-011, and an assertion plus a retraction of one triple in one
   transaction (either order) the new **API-020**. Records of one transaction share a
   `tx_id` and have no order, so the pair cannot say which came last; no normal write
   produces it after rule 3. Each rejected record changes nothing, as with API-015 and
   API-016. An unfiltered copy is identical to a source that keeps the rules. A source
   written before them (v2.x via migration, earlier v3 builds) fails at the first such
   record, and a middleware step must repair it (drop the assertions where a retraction
   decides, keep one window, drop inverted windows).

`load_fact`, WAL replay and migration never check rules 1–3. Files that already contain
such data open and read as before: readers keep their tie rules (a retraction in the
group hides the triple; several windows are kept once each).

## 2. Changes

- `src/error.rs`, `docs/ERROR_REFERENCE.md`: **API-019** "Empty or inverted valid-time
  window", text `the valid-time window of {} ends at or before it starts; :valid-to must
  be later than :valid-from (the transaction time when :valid-from is omitted)`. API-011
  is narrowed to one `(transact ...)`, and its text says so.
- `src/graph/storage.rs`: `check_valid_windows(facts, tx_id)` rejects any assertion whose
  `valid_from` (`VALID_FROM_USE_TX_TIME` resolved to `tx_id`) is not before `valid_to`.
  `FactStorage::transact`/`transact_batch` call it next to `check_one_window_per_triple`.
- `Minigraf::execute`: take `tx_id` before `allocate_tx_count`, then run both checks, so a
  rejected transaction takes no `tx_count`. The same applies to the browser `apply_write`.
- `WriteTransaction::execute` (transact): run both checks on the statement (window
  check against the staged `tx_id`) before staging it. A rejected statement stages
  nothing, and the transaction stays usable.
- `WriteTransaction::commit`: keep each triple's records from its newest staged
  statement only (the highest synthetic `tx_count`), take the commit `tx_id`, run
  `check_valid_windows` on what is left (a default `valid_from` moves to the commit time,
  so a `:valid-to` between staging and commit is caught here), then allocate the
  `tx_count` and write.
- `src/log_writer.rs`: per open transaction, the window (or retraction) written for each
  triple; API-019, API-011 and API-020 are checked before the record is loaded. New
  **API-020** "Assertion and retraction of one fact in one log-writer transaction".

## 3. Tests

- Unit: `check_valid_windows` (inverted, empty, default `valid_from` against `tx_id`,
  retractions ignored, forever); the commit collapse for each mix of statements.
- `tests/valid_time_window_test.rs`: API-019 from `execute` (both bounds, equal bounds,
  `:valid-to` only in the past, a per-fact override inverting a transaction window),
  from `WriteTransaction::execute`, and at commit, each taking no `tx_count`; old data
  with an inverted window still opens. Retract-then-assert, assert-then-retract,
  assert-then-assert with another window, and retract-then-retract inside one
  `WriteTransaction` give the same rows before and after commit, in memory,
  checkpointed and reopened. API-011 inside one statement of a `WriteTransaction` is
  raised by `tx.execute`, and the transaction still commits its other statements.
- `tests/log_writer_test.rs`: API-019, API-011 and API-020 (both orders), each leaving
  the writer usable and the output exact; duplicate retractions, identical repeats and
  the same triple in the next transaction are accepted.
- `tests/model_based_test.rs`: the model applies rules 1–3 (rejections from `execute`
  and `tx.execute`, last statement wins at commit) and checks reads inside a write
  transaction against the committed result.
