# Query memory accounting and the optional limit — Design

**Issue:** #549 (part 7 of 9 of #473), acceptance criterion from #575
**Milestone:** v3.0.0, branch `v3` (no format change; one new public option and one new error code)
**Builds on:** the streaming engine design (`2026-10-11-streaming-engine-design.md` §6.2, §7), and the
#545–#548 engine work now on `v3`

## 1. Scope

Each engine operator that holds more than one batch counts the bytes it holds. Its tables grow through
`try_reserve`. An optional per-query limit, `OpenOptions::query_memory_limit`, turns a query that would
hold more into a `QRY-012` error instead of letting the process abort or be killed. The limit is off by
default. Hash joins with no shared variable build on the side with the smaller product estimate, so the
#575 shape runs in bounded memory.

Settled with the maintainer on 2026-10-21:

1. **Per-entry estimates.** An operator adds a fixed per-entry cost plus the heap bytes of the strings an
   entry holds when it inserts that entry. It subtracts them when it lets the entry go (§3). The count is
   deterministic and the same on every target. A string that several entries share through an `Arc` is
   counted once per entry, so the count can be too high but is never too low.
2. **One error code.** `QRY-012` covers both the configured limit and an allocation the allocator refuses
   (`try_reserve` fails). The message says which (§4).
3. **Engine only.** The limit applies to queries the engine runs, rule evaluation included. Queries that
   still fall back to the executor (§2) are not counted. #551 deletes the executor before v3.0.0, so the
   gap does not ship.
4. **The #575 acceptance criterion drains a cursor.** `:find` returns a bag (one row per binding), not a
   set, so the issue's "distinct set" does not exist. A query that returns 1.6M rows through `execute`
   holds 1.6M rows by design. The test drains `Minigraf::query` without keeping the rows (§6.2). The
   issue text is corrected to match.

## 2. What runs where

The engine runs every query `engine::plan` / `engine::plan_rules` accepts. The executor still runs:

- queries inside a write transaction (`facts_override`, #550);
- shapes the planner returns `None` for.

None of these are counted. Each moves to the engine in #550 / #551, and so comes under the limit then.

## 3. Accounting

### 3.1 The meter

A query has one `Meter`: the limit and the bytes counted so far. Every stateful operator of the query shares
it, including those in `not` bodies, `or` branches and rewound correlated sub-plans, as do the query's
`IdSpace` (provisional ids) and, for a rule query, its `Relations`. `Operator::rewind` and `close` take no
`Ctx`, so the meter is an `Arc` with an atomic count. A query runs on one thread at a time, so `Relaxed`
ordering suffices. Each operator also keeps its own `bytes_held` in its `Counters` (spec §6.3, for the
profiler #185).

Before it inserts an entry, an operator calls `meter.charge(op, bytes)`:

1. If `used + bytes > limit`, it returns `QRY-012` naming `op` and the limit, and counts nothing.
2. Otherwise it adds `bytes` to `used` and to the operator's `bytes_held`.

Then the operator calls `try_reserve(1)` on the table or vector it is about to grow. That call does
nothing while the container has room, and when the container grows it asks for the whole new capacity. A
refusal becomes `QRY-012` (allocator form, §4), and the charge from step 2 is taken back.

On `rewind`, `close` and `Drop`, an operator subtracts its `bytes_held` from the meter and sets it to 0. A
`Stream` drops its meter when it closes, and a cursor's stream closes when the cursor is dropped.

### 3.2 Per-entry cost

`cell_bytes(c)` is `size_of::<Cell>()`, plus the string's length for `Scalar::Str`.
`key_bytes(k)` is `size_of::<JoinKey>()`, plus the length for `JoinKey::Str`. A hash table entry also
costs `size_of` of its key and value types plus `HASH_ENTRY_OVERHEAD` (16 bytes: the control byte, and
spare capacity at hashbrown's 7/8 load factor, rounded up). A vector entry costs `size_of` of its element.

| Holder | Charged per entry |
|---|---|
| `Join` (`HashJoin` build side) | a new key's bucket: its key, a `usize`, a `Vec<Row>` header and the overhead; each row: its cells |
| `AntiJoin` (hashed body) | each distinct key: its `Vec<JoinKey>` |
| `Union` (hashed branch) | as `Join` |
| `Distinct` | each kept row's key |
| `Aggregate` | each new group: key, index, the `Group` (its cells and accumulators); each new member of a `count-distinct` set: its key; the finished output rows |
| `Window` | each row's cells and values |
| `Relations` (rule queries) | each derived row: the row, its identity key, its entity-index slot, its `next` slot |
| `IdSpace` | each provisional id: both directions of its map, with the UUID or the name |

`IdSpace`'s DICT cache is cleared after every pull and is not charged, because it holds at most one batch.
Batches passing between operators, and the rows of the current batch inside an operator, are bounded by
`BATCH_ROWS` and are not charged.

The estimate is checked against measured RSS in the acceptance tests (§6). It is meant to be within a
factor of two of the real figure, not exact.

## 4. `QRY-012`

**Query memory limit exceeded** (category Query Execution). Two forms of the text:

- `query memory limit exceeded: {op} needs {n} more bytes and the query already holds {used} of its {limit}-byte limit`
- `query memory limit exceeded: the allocator refused {n} bytes for {op}`

After either one, the stream closes and releases what it held. The database, other queries and other
cursors are unaffected. Through a cursor, the rows returned before the error stay returned, and the error
comes on the next call, as a read error does today (`cursor.rs`). For a rule query the error can come while
the rules are evaluated, that is when the query is opened.

`docs/ERROR_REFERENCE.md` gets the table row and a section with cause, resolution and an example. The
"(QRY-012 is reserved.)" note in `QRY-013` is removed. `ErrorCode::Qry012` joins the registry and the
category tests.

## 5. API

- `OpenOptions::query_memory_limit: usize`, a public field like `max_derived_facts`, default `usize::MAX`
  (off), and the builder `OpenOptions::query_memory_limit(bytes)`. It applies to every query of the
  database: `execute`, `query`, prepared queries and rule queries. A value of 0 fails any query that holds
  state.
- The option reaches the executor as `max_derived_facts` does, and goes into each engine query's meter.
- The test oracle (`oracle::check`) accepts an engine `QRY-012` against any executor answer, because the
  executor does not count.

**Bindings** (PRs after the core PR, on each binding's `v3` branch): the option on every `OpenOptions`
record (UniFFI maps `usize::MAX` to `i64::MAX`, as for `wal_checkpoint_threshold`), and `QRY-012`. A new
`ErrorCode` variant breaks the UniFFI bindings (Python, Java, Android, Swift) until they regenerate. Node,
WASM and C take the option where they expose `OpenOptions`.

## 6. Planner change and acceptance

### 6.1 Build side of a keyless join

A `HashJoin` with no join key is a cartesian product, and the outer input's size is the product of its
patterns' sizes, not that of its cheapest pattern. For such a join the planner compares the inner pattern's
`pattern_cost` with the saturating product of the outer patterns' `pattern_cost`s, and builds on the
smaller. Joins with keys keep today's `branch_cost` comparison. With the reorder test switch off it still
builds on the inner pattern (#596 decision 3). Answers do not change, only the build side.

### 6.2 Acceptance tests

Both tests go in a new `tests/memory_limit_acceptance_test.rs`. They are ignored by default and run nightly
in `smoke.yml` as `cargo test --profile bench ... -- --ignored`, without `test-oracle`, because the executor
oracle would itself run out of memory on the #575 shape. Peak RSS comes from `/proc/self/status` after
resetting `VmHWM` (Linux only), as in `recursive_acceptance_test.rs`.

1. **Grouped count, 1M groups:** 1M entities, each with one `:g` value, and
   `[:find ?g (count ?e) :where [?e :g ?g]]`. With no limit it returns 1M rows. With the limit set below
   the meter's peak for that query (read from the counters of a first run), it fails with `QRY-012`
   naming `Aggregate`.
2. **#575 cartesian, k = 13:** the fuzz harness's 3 facts, a query of 13 patterns that share no variable,
   and `:find ?n` from one of them (3^13 ≈ 1.6M rows). Draining `Minigraf::query` without keeping the rows
   counts 1.6M rows, and peak RSS stays under 64 MiB, against more than 5.7 GB on the executor. With the
   limit set to 64 bytes, below one build side, it fails with `QRY-012` naming `HashJoin`.

Unit tests (always run) cover:

- each counted holder charges on insert and releases on `rewind` and `close`;
- the meter is back at 0 after a stream is drained, after it is closed early, and after an error;
- the limit is reached in each operator (`QRY-012` names it);
- the allocator form, through a test hook that makes `try_reserve` report failure;
- a cursor that has returned rows and then hits the limit returns the error on the next call, after which
  the database answers other queries;
- the keyless build-side choice;
- the option's default, builder, and that it applies to prepared and rule queries.

## 7. Delivery

- **PR 1** (`feat/549-memory-limit` off `v3`): the meter and accounting in every holder of §3.2,
  `try_reserve`, `QRY-012`, the option, the oracle rule, the keyless build side, unit tests, `CHANGELOG`,
  `ERROR_REFERENCE`, `CLAUDE.md`/`TEST_COVERAGE` counts. Plus a docs-site PR for `OpenOptions` and the
  error, merged after PR 1.
- **PR 2** (`feat/549-acceptance`): the two acceptance tests, the `smoke.yml` job, and the measured figures
  in the PR and `docs/` (meter peak against RSS).
- **Binding PRs** after PR 1: Python, Java, Android, Swift (UniFFI), then Node, WASM, C.
- #575's acceptance criterion is met by PR 2. Close #549 by hand after PR 2. No closing keywords for #473.
