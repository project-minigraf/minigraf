# Failure-path correctness coverage (#843)

This report covers [#843](https://github.com/project-minigraf/minigraf/issues/843),
X08 in the [#776 close-out](../../bench-coverage-v3.md#remaining-exclusions-and-follow-ups).
It maps each failure path excluded from latency timing to correctness,
recovery and fault tests at a pinned revision. It records the exact error and
data invariants those tests check, limits the C08 and C30 benchmark claims to
their fixtures, and states the standing exclusion.

**Decision:** X08 stays **EXCLUDED**. Invalid inputs, deliberate errors,
corruption, failed-handle refusal, legacy-format refusal and heavy recovery
edge cases belong to `cargo test`. CI runs it on every PR in both the plain and
`test-hooks` builds (`rust.yml`), and `smoke.yml` runs the ignored acceptance
tests nightly. No nightly benchmark ID is added or removed. No production code,
public API, test or nightly route changes. Nothing here is a timing, so nothing
was measured.

One test gap was found and filed separately as
[#865](https://github.com/project-minigraf/minigraf/issues/865): no test replays
one heavy WAL entry through `Minigraf::open` (H4 below). The fixed recursion
round bound (part of L2) also has no test, and
[#861](https://github.com/project-minigraf/minigraf/issues/861) already plans
one. Every other path has correctness coverage.

This follows PHILOSOPHY.md's reliability ("do less, do it perfectly") and
simplicity principles. A failure path's contract is its error code and the
state it leaves behind, checked once in tests. A latency series for each code
would add noisy nightly IDs without a performance contract behind them.

## Pinned inventory

| Input | Value |
| --- | --- |
| Core, tests, benchmarks and workflows | `v3` at `a95e51ffe506cb83d60a48da1622ab2e6b35340f` (2026-11-06, after #864) |
| Toolchain | rustc 1.89.0 (29483883e 2025-08-04), cargo 1.89.0 (the MSRV) |
| Plain inventory | `cargo test -- --list --format terse` → 2,062 entries ([list-plain.txt](evidence/list-plain.txt)) |
| `test-hooks` inventory | `cargo test --features test-hooks -- --list --format terse` → 2,099 entries: 2,079 Rust tests in `src/` and `tests/`, 19 doctests, 1 example test ([list-hooks.txt](evidence/list-hooks.txt)) |

`a95e51ffe` differs from #842's pin `5d85c36f` only in documentation, and the
listed tests are identical. The lists were taken with stderr included (`2>&1`),
so every binary's `Running` line precedes its tests. Local paths are replaced by
`<repo>`. The enumeration is a build plus `--list`, not a timing run. It ran on
an idle secondary benchmark host because the primary one was busy. Its
temporary checkout and target directory were deleted afterwards.

### Reproduce

```bash
git checkout a95e51ffe506cb83d60a48da1622ab2e6b35340f
cargo test -- --list --format terse > list-plain.txt 2>&1
cargo test --features test-hooks -- --list --format terse > list-hooks.txt 2>&1
python3 -I docs/benchmarks/843-failure-correctness/map.py . list-hooks.txt list-plain.txt out \
  docs/benchmarks/843-failure-correctness/targets.csv
```

From the committed lists, [map.py](map.py) (standard library only, adapted from
[#842's](../842-semantic-permutations/map.py)) writes
[tests.csv](evidence/tests.csv), [summary.json](evidence/summary.json) and
[targets.json](evidence/targets.json) byte for byte (checked). It exits 1 if
any curated target is missing.

## Method

There are two layers:

1. **Curated targets** ([targets.csv](targets.csv): 138 distinct tests in 35
   cells). Each one was chosen by reading it. Two of them share the name
   `limit_slot_errors`, one in `tests/limit_order_test.rs` and one in
   `src/query/datalog/prepared.rs`. `map.py` checks that each target
   is exactly one listed test defined in the named file, and records whether it
   exists only in the `test-hooks` build. All 138 resolve. Five are hooks-only:
   `limit_order_test.rs`'s three (B3, L1) and the two ignored
   `large_transaction_acceptance_test.rs` tests (H3).
2. **Lexical index** ([tests.csv](evidence/tests.csv), all 2,079 Rust tests).
   Each test is tagged by regexes for error codes (as strings such as
   `"STG-046"` or as `ErrorCode` variants such as `Stg046`), size constants,
   fault hooks and repair calls. A test's tags include those of the same-file
   helpers it calls, transitively, so `udf_test.rs`'s `assert_qry014` and
   `fault_matrix.rs`'s `run` lend their checks to their callers. String and
   comment contents are skipped when matching braces. In `src/`, only test code
   is read. Family counts are: size 34, window 62, bind 15, callback 38,
   limits 352, corrupt 109, retry 40, legacy 21, heavy WAL 14. These are
   lexical evidence for review, not proof that a test asserts a contract. For
   example, the limits count includes every parser test that mentions a `PRS`
   code, and the heavy-WAL tag matches the `1_000_000` checkpoint threshold
   that crash tests pass.

## Exclusions → correctness targets

Counts are curated targets. The invariants column is what the tests assert;
the full list with test paths is in [targets.json](evidence/targets.json).

### Invalid storage, value and ident size bounds (C04)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| S1 over-bound value, ident and set (WAL-003) | 7 | A string over 4,068 bytes, an attribute or keyword over 1,024 bytes, or a set whose element encoding is over 4,068 bytes is `WAL-003` with category `Wal`, through `execute`, a `WriteTransaction` commit and the WAL writer itself. The `edge_cases_test.rs` pair check the message cites 4,068 rather than the code |
| S2 exact bound accepted | 2 | `max_size_value_at_exact_boundary_checkpoint_and_read_back` binary-searches the limit, asserts it is exactly 4,068 bytes, and reads two values of that size back through EAVT, AEVT and AVET after checkpoint and reopen. 76 set elements (4,028 bytes) are accepted and all 76 read back after a checkpoint |
| S3 a rejected write changes nothing | 3 | The rejected WAL entry leaves the writer usable (later entries replay as `[2]`); a rejected large statement writes nothing and takes no `tx_count`, with the same code and message on the small and large paths; rejected `LogWriter` records change nothing |
| S4 parser length caps | 4 | `PRS-007` (string), `PRS-008` (keyword), `PRS-009` (tagged literal), `PRS-074` (bind slot name) |
| S5 in-memory databases | 1 | An 8,192-byte value is accepted in memory, where there is no page bound. This is the documented contract, not a gap |

### Valid-time windows, write ordering and log-writer rules (C10, C30, C32)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| W1 empty or inverted window (API-019) | 6 | `API-019` through `execute`, `WriteTransaction::execute` and commit (a window that ended before the commit time); no `tx_count` taken; a staged window that ended when the transaction became large rejects the statement and the commit, with no WAL guard left and no records |
| W2 two windows for one fact in one transaction (API-011) | 4 | `API-011`; nothing visible at either window; no `tx_count` taken, no fact written; in a `WriteTransaction`, the rejected statement leaves the earlier one to commit as one transaction |
| W3 last statement wins (#477) | 3 | The last statement decides each triple inside the transaction and after commit; other triples of an overridden statement survive; the commit writes one transaction without the overridden records |
| W4 `LogWriter` rules | 1 | `API-019`, `API-020` and `API-011` (a second window); a rejected record opens no transaction. S3's `rejected_records_change_nothing` checks `API-015`/`API-016` and that rejected records change nothing |
| W5 error phase order | 1 | Materialisation (`INT-002`) and window (`API-019`) errors come from the statement's facts in the documented phase order |

### Prepared binds (C34)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| B1 bind slots outside `prepare` (API-010) | 4 | `API-010` in every position, through `execute`, `query` cursors and `WriteTransaction::execute`; the message lists the slots sorted and points at `prepare()` |
| B2 missing, attribute-position and wrong-kind binds | 3 | `API-022`, `API-023` and `API-024` with category `Api`, in every position or slot kind |
| B3 count slots | 4 | An unbound `:limit`/`:offset` slot is `API-010`; a negative integer is `PRS-115`; a value of another kind (including `TxCount`) is `API-024` |

### Deliberate callback errors (C33)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| U1 failing predicate (QRY-014) | 3 | `QRY-014` naming the function and its message in every clause position, through a prepared query (with the exact call count), and through a cursor: earlier rows are returned, then the error, then nothing |
| U2 failing aggregate (QRY-014) | 1 | `QRY-014` in every phase of an aggregate, after the failing call was made |
| U3 late errors not hidden | 6 | Under a `:limit`, an ordered aggregate, a sort or a partitioned window, a late callback or `collect-set` failure still fails the query rather than returning a truncated answer; four tests also assert every one of the 1,500 calls was made; registered callbacks keep the full-drain strategy |
| U4 error text | 1 | `QRY-014`, category `Query`, names the function and its message |

`BrowserDb` UDF throws (`src/browser/udfs.rs`) are `wasm_bindgen_test`s that
`wasm.yml` runs in headless browsers. They are outside this native inventory.

### Query options, exhaustion and validation codes (C19, C37)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| L1 query memory (QRY-012) | 8 | `QRY-012` through `execute`, cursors (rows first, then the error, then nothing), prepared and rule queries, queries inside a `WriteTransaction` (before and after a staged write), sorts, and the allocator-refusal form; the database answers a small query afterwards |
| L2 rule exhaustion (INT-020) | 6 | `INT-020` when the derived-fact limit is passed: per query (`:max-derived-facts`) and per database, counting facts derived through a `not` rule and the total rather than one iteration, through cursors and prepared rule queries with the database's limits. The fixed 1,000-round bound has no test at the pin; see below |
| L3 invalid query options | 7 | `PRS-115` (`:limit`), `PRS-116` (`:offset` without `:order-by`), `PRS-117`, `PRS-118`, `PRS-084` with their messages; `:max-derived-facts 0` and duplicates rejected with the message |
| L4 query validation codes | 6 | `QRY-007`, `QRY-010`, `QRY-011` (with and without matching rows), `QRY-013`, `QRY-015` (and the stored set stays readable); engine planner errors keep their codes |
| L5 parser codes and grammar | 7 | The per-code `PRS` tables through `execute` and `prepare`; `PRS-108` rejects the whole transaction; the pest grammar's invalid syntax and semantic corpora; the bounded grammar verifier's structural level and semantic witnesses |
| L6 handle API codes | 1 | `API-014` for every write on a read-only handle; no `tx_count` allocated, data unchanged |
| L7 code registry | 2 | Every `ErrorCode` variant has a registry entry, and the registry matches `docs/ERROR_REFERENCE.md` in both directions |

`error_codes_prs_test.rs` holds 62 tests, mostly one per `PRS` code or family; L3
and L5 name only the ones relevant to options and the overall tables. "Valid
operations still pay validation cost" is covered by the ordinary nightly IDs,
which all run validated input.

### Corrupt pages, trees and files (C08)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| C1 page checksum, id, generation | 6 | `STG-029`, `STG-030`, `STG-031` from the page codec; a corrupt page is not cached; every query over damaged leaves fails with `STG-029` (no fallback to other indexes); damaged index pages surface on read while EAVT stays intact |
| C2 meta pages and short files | 7 | `STG-032` (torn initial meta), `STG-033` (newest meta damaged after commit, or a WAL newer than the valid meta), `STG-044` (truncated file, meta page count past the end) read-write and read-only; the file is unchanged, never grows, and no WAL is created |
| C3 trees, dictionary, free list | 5 | `STG-035` (free-list count or loop), `STG-036` (missing dictionary entry), corrupt leaves rejected by decode and validate, `INT-049` for a cursor through a tree cycle without repeating a key; an internal node pointing at itself fails each query instead of looping |
| C4 verify and repair of arbitrary damage | 6 | `verify` reports `STG-029`/`STG-038`/`STG-039` for each index's damage and `rebuild_indexes` leaves a clean file; dictionary damage (`STG-036`/`STG-040`/`STG-029`) is reported and not rebuilt (`STG-041`); a damaged free-list page fails the checkpoint, not the file; a crash at every point of a rebuild keeps every fact; faults during `rebuild_indexes` (fault matrix) |

### Failed-handle retry refusal (C06, C37)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| R1 failed checkpoint (STG-046) | 7 | For a fault at every k-th write or sync of a checkpoint, auto-checkpoint or large transaction (`fault_matrix.rs`'s `run`): no fact lost, no partial transaction, a committed transaction complete; a retried checkpoint is `STG-046`, and a later write is `STG-046` or `WAL-007` only after a failure. Disk full halfway and a failed meta fsync: no commit, `STG-046`. A failed handle refuses a statement that would become large and its commit; a failed large write transaction fails its commit, records are unchanged and `verify` is clean |
| R2 failed WAL append (WAL-007) | 3 | A torn append refuses later appends with `WAL-007` and nothing follows the torn entry; through `Minigraf`, the failed transaction is not applied and writes are refused until a checkpoint; faults during `transact` (fault matrix) |

### Unsupported v1–v6 files (C07)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| V1 v1–v6 refused (STG-028) | 6 | Every version 1 to 6 is `STG-028`, the message says how to upgrade through v2.x, and the file is left byte for byte unmodified; v2 and v5 files specifically; the header decoder and validator |
| V2 unknown versions | 2 | `STG-006` with category `Storage` |

Native v7 → v8 migration is a separate supported path (C07, #838).

### Heavy single-entry WAL replay (C06)

| Cell | Tests | Invariants checked |
| --- | ---: | --- |
| H1 entry of more than 1,000,000 facts | 1 | The WAL layer only: an entry of 1,000,001 facts and the entries after it replay, reopening the writer cuts off nothing, and an append lands after both (#686) |
| H2 damaged entry bounds | 2 | An entry claiming more facts than the bytes left, or whose facts are cut short, ends replay at the last good entry |
| H3 million-fact statements | 2 | Ignored; `smoke.yml` runs them nightly. A 1,000,000-fact `transact` holds at most one chunk, commits as a checkpoint with no WAL left, and survives reopen; the chunk option and its clamps up to 1,000,001 facts |
| H4 replay through `Minigraf::open` | 0 | **Missing.** See below |

## Missing test coverage

**H4: no test replays one heavy WAL entry through `Minigraf::open`**
([#865](https://github.com/project-minigraf/minigraf/issues/865)). H1 drives
`WalWriter` and `WalReader` directly. It does not open a database, so the
replay into the pending store, the queries over the replayed facts and the
checkpoint that follows are not exercised for a heavy entry. H3's statements
are larger than the chunk and commit as checkpoints, so they never write a WAL
entry. The `open/wal_replay` benchmarks use 100-fact statements.

The shape is reachable: a statement of exactly `transaction_chunk_facts` facts
(up to 1,000,000) stays small and is written as one WAL entry
(`parser/statement.rs`, pass 1), and v2.x writers could write larger entries
that a v7 → v8 migration replays. #865 proposes a correctness test only, likely
among the ignored nightly acceptance tests. No production change is expected
unless that test fails.

Follow-up: `one_million_fact_wal_entry_replays_on_open` in
`tests/large_transaction_acceptance_test.rs` now covers this path (#865,
nightly); it passes with no production change. The map above stays pinned to
`a95e51ffe`.

**The 1,000-round recursion bound has no test at the pin.** `INT-020`
"Max iterations (1000) exceeded" (`engine/recursive.rs`) is not triggered by
any test. A check on the alternate host at the pin confirmed it is reachable:
a 1,500-edge acyclic chain queried as `(reach :n0 ?b)` fails with that error
instead of returning 1,500 rows. It fails loudly and returns no partial rows.
This is already tracked by [#861](https://github.com/project-minigraf/minigraf/issues/861)
(v3.0.0, found by #841), which makes the bound configurable and lists the
tests: a 501-edge chain fails at the default and succeeds with a raised limit.
No separate issue was filed.

No other gap was found. Two weaker checks are noted, each backed by a stronger
target in the same cell:

- `edge_cases_test.rs`'s oversized-fact tests check the message cites 4,068
  rather than the code; S1's other tests assert `WAL-003`.
- `parser.rs`'s `:max-derived-facts` tests check the message rather than a
  code; L3's `error_codes_prs_test.rs` targets assert codes.

## C08 and C30 claims

Both benchmarks time successful work around one fixed failure. Their claims
stay limited to their exact fixtures, and both keep result-integrity checks
before timing.

| ID | Fixture and operation | Integrity checks | Does not establish |
| --- | --- | --- | --- |
| `checkpoint/storage_v3/verify/100k_history` (C08) | 100,000 populated facts, 100 updates and 50 retractions, checkpointed: 100,150 stored and 99,950 live facts; `verify()` on one open handle | Fixture setup checks `verify()` is clean with 100,150 facts and more than 0 pages, and the query returns 99,950 rows | Verify cost on damaged files or other sizes |
| `checkpoint/storage_v3/rebuild_damaged_avet/100k_history` (C08) | The same file with one byte flipped at offset 2,000 of the current AVET root (no checksum recomputed); a fresh copy opened per iteration, then `rebuild_indexes()` | Once, at setup: `verify()` must report the damage; after `rebuild_indexes()` it is clean with 100,150 facts and the query returns 99,950 rows. The timed iteration unwraps `rebuild_indexes()` | Rejection latency for any `STG` code; damage to other trees, the DICT, meta or free list; arbitrary corruption |
| `insert_file/chunked/statement_rollback/65537` (C30) | A `WriteTransaction` on a fresh file: a small statement, then a 65,537-fact statement whose last fact gives `:chunk/bad` a second window after the first default-size chunk reached the working trees, then a valid statement and commit | Every iteration asserts the rejection is `API-011`. Once, at setup: one transaction committed, no `:chunk/value` facts at any valid time, two control markers, and the file committed as a large transaction | Parser rejections, rejections at other chunk positions, I/O-failure rollback, or exhaustive failure timing |

## Standing exclusion and new routes

- X08 remains a deliberate standing exclusion. The 138 curated tests above and
  the wider suite own these contracts, and CI runs them on every PR (and the
  ignored acceptance tests nightly).
- No dedicated failure-latency route is proposed. A future one must state:
  - a concrete performance or durability hypothesis, for example "a damaged
    page is rejected in time bounded by the pages read, without scanning the
    file", that no existing ID covers and that a correctness test cannot check;
  - the failure it times and a preflight that checks the exact error code and
    the state left behind before timing, as C30 does for `API-011`;
  - a bounded nightly budget. The rebuild and rollback IDs build a fresh copy
    or fresh file per iteration in Criterion's untimed setup
    (`iter_batched_ref`, `PerIteration`), so their wall time exceeds the
    measured time; the verify ID reuses one open handle.
- X02 owns the actual memory and exhaustion acceptance bounds. This report does
  not waive them.

## Files

| File | Purpose |
| --- | --- |
| [map.py](map.py) | Reproduces the index and verifies targets (standard library) |
| [targets.csv](targets.csv) | 138 curated targets in 35 cells |
| [evidence/list-plain.txt](evidence/list-plain.txt), [evidence/list-hooks.txt](evidence/list-hooks.txt) | Pinned inventories |
| [evidence/tests.csv](evidence/tests.csv), [evidence/summary.json](evidence/summary.json) | Lexical index and family counts |
| [evidence/targets.json](evidence/targets.json) | Each target's listed path, tags and hooks-only flag |
