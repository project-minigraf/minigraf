# SIGKILL crash test checks committed data — Design

**Issue:** #384, tracker #383
**Milestone:** v3.0.0, branch `v3` (test only; no library change)

## 1. Goal

`tests/crash_kill_test.rs` kills a child process mid-write and then only checks
that the file opens and that one AEVT query runs. It missed #370 (EAVT broken,
AEVT intact) and could not reach #371 (one fact per transaction). After this
change, every round checks that the data on disk is exactly the data the child
committed.

## 2. Shape

The test keeps its name and its parent/child structure, so `crash-kill.yml`
(nightly, 100 rounds on Linux, macOS and Windows) and the per-PR suite pick it
up unchanged. `sigkill_during_checkpoint_never_permanently_corrupts_file` is
replaced by `sigkill_keeps_exactly_the_committed_transactions`.

Each round:

1. Pick a seed and a workload variant (§4). Spawn the child with the database
   path, a confirmation-log path, the seed and the variant.
2. The child runs transaction `k = 1, 2, …` from a deterministic generator
   (§3). After `execute` returns, it appends `k\n` to the confirmation log with
   one `write_all`.
3. The parent waits until the log holds a random target of 1–60 confirmed
   transactions (poll every 1 ms, 30 s timeout), sleeps 0–20 ms more, then
   kills the child (`SIGKILL` on Unix, `TerminateProcess` on Windows).
4. The parent reads the log. `N` is the last complete line (a line with no
   newline is ignored).
5. Checks (§5) on: the first reopen; a second reopen; and a reopen after an
   explicit `checkpoint()`.

The confirmation log is not fsynced. A process kill does not drop the OS page
cache, so a written line survives the kill the same way the WAL entry does.
fsync would only slow the child down.

## 3. Workload

A transaction is one `execute`. The generator is a function of the seed and the
model state, so the parent rebuilds the same model by replaying it.

- Pools: 4 keyword entities `:e0`–`:e3`, 3 attributes (`:a0`, `:a1`,
  `:tag/丿`), values: integers 0–7 and two strings longer than the 64-byte
  inline limit.
- **Multi-valued batch** (`transact`, 1–6 facts, ~60%): several values of one
  entity and attribute in the same transaction (#371), plus random other facts.
  Never the same triple twice in one transaction.
- **Batched retract** (`retract`, ~30%): 1–4 currently live triples. When
  nothing is live, a batch is generated instead.
- **Re-assert** (~10%): a batch that asserts a triple retracted earlier.

All facts use the default valid time. Valid-time behaviour is the model-based
test's job (#385).

## 4. Variants

The round number picks one, so every per-PR run covers them all:

| Variant | Child loop | Kill lands in |
|---|---|---|
| `CheckpointEach` | transact, then `checkpoint()` | mostly `save()` |
| `CheckpointEvery(3)` | `checkpoint()` after every 3rd transaction | WAL append and `save()` |
| `WalOnly` | never checkpoints (`wal_checkpoint_threshold(usize::MAX)`) | WAL append; replay on reopen |
| `AutoCheckpoint` | `wal_checkpoint_threshold(2)` | auto-checkpoint inside `execute` |
| `NormalSync` | as `CheckpointEvery(3)` with `SyncMode::Normal` | same; no WAL fsync |

## 5. Checks

The model gives the live triples after `k` transactions, `M(k)`. On each
reopened handle:

- `current_tx_count()` is `N` or `N + 1`. Call it `T`. `N + 1` means the
  in-flight transaction reached the WAL or file before the kill but not the
  log. Nothing beyond it can exist.
- Full scan `[?e ?a ?v]`, AEVT `[?e <a> ?v]` per attribute, EAVT `[<e> ?a ?v]`
  per entity, and AVET `[?e <a> <v>]` per attribute and value: each, rendered
  without UUIDs, equals the matching part of `M(T)`. The union over entities
  and attributes must equal the full scan.
- `:as-of k` full scans for a few `k ≤ T` (1, `T/2`, `T - 1`) equal `M(k)`.
- `verify()` reports no problems.

Failure messages name the round, seed, variant, `N` and `T`. Setting
`MINIGRAF_CRASH_KILL_SEED` replays one seed. Assert messages print no `Uuid`
bearing types (CLAUDE.md).

## 6. Cost

Per-PR: 5 rounds (one per variant), each well under a second. The nightly
workflow keeps its 100 rounds per OS.

## 7. Out of scope

Kills inside `begin_write`/`commit` (covered by the in-process crash tests in
`tests/common`), valid-time windows, and fault injection (#390).
