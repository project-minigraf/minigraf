# Model-based (stateful) storage test — Design

**Issue:** #385, tracker #383
**Milestone:** v3.0.0, branch `v3` (test and CI only; no library change)

## 1. Goal

Run random sequences of writes, checkpoints and reopens against a file-backed
database and compare every read with a plain in-memory reference model. The
storage and index bugs of 2026 (#285, #287, #323, #370, #371) only showed up after
such sequences.

## 2. Shape

`tests/model_based_test.rs`, one `proptest!` over `Vec<Step>`. A step is an
operation plus a probe (the random inputs of the checks that follow it). Plain
proptest replaces `proptest-state-machine`: a `Vec<Step>` already shrinks by
dropping and simplifying steps, every operation is valid in every state, and no
dependency is added.

Pools are small so that steps collide: 4 keyword entities, 3 attributes (one
non-ASCII), 18 values (integers 0–2, `i64::MIN`, booleans, a keyword, a float, short
strings, two 80-byte strings sharing a 40-byte prefix (the v8 long-string key
prefix is 32 bytes), a 3,000-byte string for the value pages, and refs to the
entities). A third of the triples are one hot triple, so that its history grows
past the on-disk scan's seek threshold (8 entries). A case has 1–49 steps.

## 3. Operations

| Op | API | Model |
|---|---|---|
| `Transact` | `execute("(transact {window} [[e a v {window}] …])")`, 1–6 facts, sometimes a repeated triple with another window | one tx; API-011 when a triple gets two windows |
| `Retract` | `execute("(retract [[e a v] …])")` | one tx (always, even if nothing matches) |
| `WriteTx` | `begin_write`, 0–4 transact/retract statements (half the time plus a retraction of a triple it asserts, before or after), then `commit` or `rollback` | one tx if committed and non-empty (a same-tx retraction wins); API-011 as above; nothing on rollback |
| `Checkpoint` | `checkpoint()`, then `verify()` | none |
| `Reopen` | drop and `open_with_options`: default, `page_cache_size(4)`, or `wal_checkpoint_threshold(2)`; then `verify()` | none |
| `Crash` | copy the `.graph` file and WAL while the handle is open, drop it, open the copy (same modes); then `verify()` | none |

`Crash` is the only way to reach WAL replay in-process: dropping a handle always
saves (`PersistentFactStorage::drop` saves when dirty, even with
`wal_checkpoint_threshold(usize::MAX)`), and the file lock forbids leaking it. On
Windows, where locks are mandatory, the copy is taken after the drop.

All handles use `SyncMode::Normal`: the test checks results, not fsync.

Valid-time windows come from a grid: `:valid-from` in {2000, 2002, 2004, 2006},
`:valid-to` in {2003, 2005, 2007, 2200} (January 1st UTC), each optional at the
transaction and the fact level. A fact window that would be empty or inverted
(#436) is rewritten to start in 2000, so every generated window is valid.

## 4. Model

A log of records `(tx, e, a, v, valid_from or tx-time, valid_to, asserted)` and a
transaction counter. At `as-of N`, a triple is live when its newest assertion
with `tx ≤ N` is newer than its newest retraction; its window is that
assertion's (one window per triple, #435). A window that starts at transaction
time is before every grid instant up to 2008, after 2100, and before "now".

## 5. Checks after each step

- `current_tx_count()` equals the model counter.
- Full rows `?e ?a ?v ?vf ?vt ?tc ?ti` with `:any-valid-time`, current and at a
  random `:as-of N` (N up to counter + 1): windows and tx counts match the model,
  a tx-time `valid_from` equals the fact's tx id, and tx count → tx id is a
  function.
- Entity-bound, entity+attribute-bound, attribute-bound, attribute+value-bound,
  value-bound and full-scan queries, each under: default (now), `:valid-at T`,
  `:as-of N :valid-at T`, `:as-of N`. T comes from a grid that hits every window
  bound plus 1999, 2008, 2100 and 2300.

Results are compared as sorted lists of rendered rows; entity UUIDs render as
their pool index, so failure messages carry no `Uuid` (CodeQL rule).

## 6. CI

Normal CI runs 24 cases (default, overridden by `PROPTEST_CASES`). The nightly
property-test job becomes a matrix over `property_test` (8,000,000 cases) and
`model_based_test` (30,000 cases, ~0.12 s each locally), so the two run in
parallel within their own 6-hour limits; the failure issue names the test.

Mutation checks during development: the test fails on each of ignored
retractions, an off-by-one `:as-of`, inclusive `:valid-to`, exclusive
`:valid-from`, a same-tx retraction that loses, a missing API-011, on-disk
retractions ignored, on-disk older transactions not hidden, a seek that skips the
`:as-of` transaction, a WAL replay that skips an entry, and a transaction counter
not restored after replay. Minimized failures become named regression
tests in the same file.
