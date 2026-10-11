# Streaming query engine — Design

**Issue:** #473 (streaming, pull-based operator pipeline behind the cursor API; follow-up to #432)
**Milestone:** v3.0.0, branch `v3`, file format v8 (no format change)
**Folds in:** #518 (AVET lookups, parked on `perf/518-avet-lookup`), #530 (ids until projection)
**Followed by:** #306/#310 (`:limit`/`:offset` syntax and the 10M acceptance test), #185 (profiler)
**Status:** design approved 2026-10-11; not implemented

## 1. Problem

`Minigraf::query` returns a `Cursor` (#432), but the answer is computed when the cursor
opens: `execute_query` loads the facts its patterns touch into a `Vec<Fact>`, the
`PatternMatcher` joins them into `Vec<HashMap<String, Value>>` bindings, and the cursor
walks the final `Vec`. Memory follows input size, not answer size:

- `[?e :entity-type :type/function]` over 10M facts reads every fact of the attribute,
  materialises each one with UUIDs and owned strings, then keeps ten rows.
- `count` over 10M facts holds 10M bindings before counting.
- A recursive rule holds its whole input in a `FactStorage` plus a `BTreeSet` of seen facts.
- A 1M-fact `transact` is parsed into one `Vec<Fact>`, staged, and written as one WAL entry.
- Closing a cursor early saves nothing: the work is already done.

A cursor also has no snapshot of its own. The answer is fixed because it is computed at
open. Once it is computed lazily, a checkpoint that frees and reuses pages under an open
cursor would corrupt its reads. The fact log (#430) avoids this today by refusing
checkpoints while a log is open (`API-013`).

## 2. Goals and non-goals

Goals are #473's:

1. **Memory follows operator state, not input size.** Pipelined operators hold one batch.
   Only the stateful operators hold more, and the planner avoids that state where index
   order allows.
2. **Incremental delivery** with early termination, cancellation and backpressure.
3. **Streaming aggregation**, one pass, hash- or sort-grouped.
4. **Consistent snapshots** at open; writes committed later do not change the answer.
5. **Streaming transactions**: a large `transact` never holds more than one chunk of
   parsed facts.

Non-goals:
- No Datalog semantics change. `:find` keeps returning a bag.
- No file-format change. The `.graph` layout stays v8.
- No parallel execution (the operator model does not preclude it).
- No spilling and no `spill_dir`.
- No public API change: `Cursor`, `Batch`, `PreparedQuery` and every binding are untouched.

## 3. Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Scope for v3.0.0 | All of #473: every operator, `Recursive`, streaming transact |
| D2 | Snapshot | Pin the committed generation; copy the pending facts the plan reads; quarantine freed pages in memory (§4) |
| D3 | Fact log | Moves to the same pin; `API-013` stays registered, deprecated |
| D4 | Migration | New engine alongside the old one; the old one is a test-only oracle, deleted before v3.0.0 (§9) |
| D5 | `:limit` | #473 builds the `Limit` operator; #306/#310 add the syntax afterwards and take the 10M acceptance test |
| D6 | Large state | No default budget, no spill; byte accounting, `try_reserve` growth, an optional cap that raises one new QRY code (§7) |
| D7 | Row format | Columnar batches with plan-time variable slots; entities and idents stay DICT ids until `Project` (§5) |

## 4. Snapshots

### 4.1 What a cursor holds

At open, under the storage read lock, a cursor takes:

1. **A committed snapshot**: the active meta page's generation and its five tree roots,
   as an `OnDiskReader` over those roots. The reader never looks at a newer meta.
2. **A pin** on that generation (§4.2), released on `close` or drop.
3. **The pending facts the plan reads.** Each `Scan` in the plan copies the pending facts
   in its range (the same ranges `FactStorage::get_live_facts` serves today). Pending
   facts are those not yet checkpointed, so the copy is bounded by
   `wal_checkpoint_threshold` (default 1,000 entries). With checkpointing disabled the
   copy follows the pending set; that is the documented cost of disabling it.
4. **The `tx_count` bound** at open, applied to the copied pending facts.
5. **Provisional ids** (§5.3) for pending entities and idents that the DICT does not have.

After that the cursor needs no lock. Writers and checkpoints proceed.

### 4.2 Page quarantine

A checkpoint is copy-on-write: it writes only pages the active meta does not reference,
and the pages the new meta stops referencing go on the free list (`finish_free_list`).
The *next* checkpoint may hand them out and overwrite them. A cursor pinned to an older
generation may still read them.

The allocator therefore keeps, in memory only, a **quarantine**: for each checkpoint,
the generation it produced and the ids it freed. An id freed by the checkpoint that
produced generation `g` is reachable from every generation `< g`. While any pin `< g`
exists, `PageAllocator::alloc` skips that id: it stays listed as free, and
`finish_free_list` lists it again with the read-but-unused ids. When the oldest pin
drops, the quarantine entries at or below the new oldest pin are released.

Properties:
- **No format change.** On disk, a quarantined page is an ordinary free page.
- **Crash-safe.** After a crash nothing is pinned, so every free page is reusable.
- **Growth is bounded by the pin.** While a cursor is open the file grows by the pages
  the intervening checkpoints free, as in LMDB with a long reader. `verify()` already
  counts free pages; nothing changes there.
- **Page cache.** A quarantined id is never rewritten, so a cached page can never serve
  another generation's bytes.

The in-memory (`MemoryBackend`) and browser (`BrowserBufferBackend`) backends go through
the same allocator, so they quarantine the same way. A browser flush writes dirty pages
only, which never include a quarantined page.

### 4.3 Fact log (D3)

`Minigraf::fact_log` pins its generation the same way and copies its pending facts at
open. Checkpoints no longer fail while a log is open, and `log_pins()` stops gating the
automatic checkpoint (`db.rs:767`, `db.rs:1602`). `API-013` stays in the registry and in
`docs/ERROR_REFERENCE.md`, marked deprecated, and is never raised again.

`rebuild_indexes` rewrites every tree; it also proceeds, since pinned pages are
quarantined like any other freed page.

## 5. Rows

### 5.1 Batches

A `Batch` (internal; the public `Batch` wraps it) holds up to 1,024 rows as one column
per variable slot. The planner numbers the variables of each operator's output, so a
column is found by index, never by name. `HashMap<String, Value>` bindings are gone from
the new engine.

### 5.2 Cells

```rust
enum Cell {
    Eid(u64),       // entity: DICT id or provisional id
    Iid(u64),       // ident (attribute or keyword value): DICT id or provisional id
    Val(ValueRef),  // string, integer, float, boolean, null; long strings by vref
}
```

Ids stay ids through joins, filters, sorts and aggregation. `Project` turns the cells of
the rows it returns into `Value`s, so a query that keeps ten rows translates ten rows
(#530). Literals in the query are resolved to ids once, at plan time; a literal the DICT
and the snapshot do not know resolves to "no match", so its `Scan` is empty.

### 5.3 Provisional ids

Pending facts carry UUIDs and strings. At open the snapshot gives each pending entity
and ident that the DICT does not hold a provisional id from `next_eid` / `next_iid`
upwards, in a map private to the cursor. The whole snapshot then has one id space.
Provisional ids are never written; a checkpoint assigns real ids independently.

### 5.4 Cross-kind matching

The matcher today lets a keyword constant `:k` match `Keyword(k)` and `Ref(uuidv5(k))`,
lets a UUID constant match a ref or a keyword naming that entity, compares floats within
an epsilon, and matches `nil` to `Null`. The new engine keeps every one of these rules:

- **Constants** resolve to every id they can match: `:k` to `Iid(k)` and to the `Eid`
  of `uuidv5(k)` if that entity exists. These are the AVET probes #518 built
  (`ValueProbe`), now in id form.
- **Join keys** compare `Iid(k)` equal to `Eid(e)` exactly when `e = uuidv5(k)`. The
  snapshot resolves that per ident lazily and caches it; the number of idents bounds it.
- **Floats** keep the matcher's epsilon rule. A join on a float-valued variable uses the
  same comparison as today; the oracle tests (§9) pin which rows match.

## 6. Operators

All operators implement `open` / `next_batch() -> Result<Option<Batch>>` / `close`.
`close` propagates to the children, so a closed cursor stops all upstream work.

### 6.1 Pipelined (hold one batch)

- **`Scan`** over EAVT, AEVT, AVET or VAET, chosen by which pattern positions are bound.
  It merges the committed snapshot's keys with the copied pending facts per triple and
  decides net-assert on keys (#379), with the `:as-of` bound and `:valid-at` /
  `:any-valid-time` pushed into the walk. Attribute+value patterns read AVET (#518's
  `Scan::AttributeValue` / `AttributeKind` and its string/float/keyword/ref rules).
  Output is in index order, and the plan knows that order.
- **`IndexNestedLoopJoin`**: for each outer batch, probes the inner pattern's index with
  the bound positions. The default join when the inner pattern has a usable index.
- **`Filter`** / **`Expr`**: predicates and bindings from expression clauses.
- **`Project`**: column selection and id→value translation (§5.2).
- **`Union`** for `or` / `or-join`.
- **`Limit`** / **`Offset`**: stop pulling, then `close` the child.

### 6.2 Stateful (hold more than a batch)

- **`HashJoin`**: builds on the smaller side when neither side has a usable index.
- **`NotJoin`** / anti-join: hashes the right side.
- **`Distinct`**: hash on the row. Used only where the engine deduplicates today (`or`
  branches, `count-distinct`, the recursive relation). Plain `:find` stays a bag.
- **`Sort`**: in memory; skipped when index order already gives the order; becomes
  top-k under a `Limit`.
- **`Aggregate`**: ungrouped aggregates run in constant memory; grouped ones hash on the
  key, or stream sort-grouped when the child is ordered on it.
- **`Window`**: today's window functions over a partitioned, ordered input.
- **`Recursive`**: semi-naive fixpoint per stratum. The delta and the accumulated
  relation are hash relations of id rows, replacing `FactStorage` plus
  `BTreeSet seen_facts`. Magic sets and stratification stay as rewrites in front of the
  planner. `max_derived_facts` (`INT-020`) still bounds the accumulated relation.

### 6.3 Planner

`optimizer.rs` and `prepared.rs` produce an operator tree instead of a clause order for
the matcher. Rules:

1. Pick each pattern's index from its bound positions; prefer AVET for `[?e :a C]`.
2. Order joins by the existing selectivity estimates; use `IndexNestedLoopJoin` when the
   inner side is indexable, else `HashJoin` on the smaller side.
3. Use sort-grouped aggregation and skip `Sort` when the input order allows.
4. Push `Limit` into `Sort` (top-k).

A `PreparedQuery` keeps its plan and re-resolves only its bind slots and literals per
execution. Each operator counts rows in, rows out, batches and bytes held; the profiler
(#185) reads these counters.

`optimizer.rs` is disabled under the `wasm` feature today. The new planner keeps that
gate on join reordering only; index choice and operator selection run on every target.

## 7. Memory (D6)

- **No default budget and no spilling.** Streaming makes large state rare: only the
  operators in §6.2 hold more than a batch.
- **Accounting.** Each stateful operator tracks the bytes it holds and grows its tables
  with `try_reserve`, so an allocation failure becomes an error instead of an abort
  where the allocator reports it. (With Linux overcommit it rarely does; the cap below
  is the dependable control.)
- **Optional cap.** `OpenOptions::query_memory_limit(bytes)`, off by default
  (`usize::MAX`). When the bytes held by one query's stateful operators would pass it,
  the query fails with a new code, **`QRY-012`: query memory limit exceeded**, naming the
  operator and the limit. The cursor is closed; the database is unaffected.
- `max_results` keeps its meaning for `execute` only. A cursor has no result cap.
- **Bindings.** Every binding gets `query_memory_limit` on its `OpenOptions` record and
  the `QRY-012` code. UniFFI shims map the `usize::MAX` sentinel to `i64::MAX`, as for
  `wal_checkpoint_threshold`. Adding an `ErrorCode` variant breaks the UniFFI bindings
  until they regenerate, so the binding PRs follow the core PR.

## 8. Transactions

### 8.1 Parsing

`transact` and `retract` go through a streaming EDN parser that yields statements, which
are materialised into facts in chunks (default 4,096 facts). Validation (value limits,
valid windows, `check_one_window_per_triple`, `API-011`) runs per chunk; the duplicate
check across chunks reads the transaction's own facts through `Scan` (§8.3).

### 8.2 Where chunks go

A transaction below `wal_checkpoint_threshold` facts is written exactly as today: one
WAL entry at commit, then applied to the pending store.

A transaction that reaches the threshold switches to **writing its chunks into
copy-on-write pages**, the way `LogWriter` (#431) builds batched checkpoints:

1. On the switch it fixes its `tx_count` (known: the write lock is held) and its
   `tx_id` (the current time; the lock keeps it monotonic). Facts staged so far are
   stamped and inserted first.
2. Each later chunk is encoded and `cow_insert`ed into working copies of the trees.
   Pages written by this transaction and superseded within it are reused at once; pages
   of the active generation it supersedes are freed at commit.
3. **Commit** checkpoints the pending store into the same working trees, writes the new
   meta page and deletes the WAL: the transaction commits as a checkpoint.
4. **Rollback** or a crash before the meta write leaves the active meta and its free
   list untouched. Every page the transaction wrote was either on that free list or past
   the committed `page_count`, so the next checkpoint reuses or overwrites it as usual,
   exactly as after a crash mid-checkpoint. The allocator skips quarantined ids here too.

This replaces the "append each chunk to the WAL" wording in #473. That wording still
left all facts in the in-memory pending store after commit, so a 1M-fact `transact`
would hold 1M facts in memory and fail the acceptance test in §10. Writing large
transactions as a checkpoint keeps the WAL format and the small-transaction path
unchanged.

A transaction's `tx_id` is the switch time instead of the commit time for large
transactions. Both lie inside the write lock, so the order of transactions and their
`tx_count`s do not change. The change is noted in `CHANGELOG.md`.

### 8.3 Reading your own writes

A `WriteTransaction`'s queries see a snapshot whose committed part is the working trees
(when it has switched) and whose pending part is the pending store plus the chunks not
yet inserted. It is the same `Scan` over a different snapshot.

Schema validation and the v8 multi-value fix (#371, #287) are unaffected.

## 9. Migration and testing (D4)

- The new engine is built in `src/query/engine/` beside the existing executor. A query
  runs on the new engine when every clause it uses has an operator; otherwise on the old
  path. Each sub-issue widens that set.
- **Oracle.** The old path stays as a test-only oracle (`#[cfg(test)]` and a
  `test-oracle` feature for integration tests). Differential tests run every query in
  these suites through both engines and compare row bags:
  - the model-based test (`tests/model_based_test.rs`, `tests/reference_model/`),
  - the property tests (`tests/property_test.rs`),
  - the golden corpus (`tests/golden_corpus_test.rs`),
  - the existing integration suites, through a test hook on `execute`.
- When every query runs on the new engine, the old path and the oracle hook are deleted.
  This happens before v3.0.0.
- `tests/cursor_test.rs` and every binding suite pass unchanged throughout.
- **Measurements.** The #394 envelope at 100K/1M/10M on an idle machine before and after
  each phase; the #379 point queries are the regression guard.

## 10. Acceptance tests

From #473, with the `:limit` test moved to #306/#310 (D5):

- [ ] `count` over 10M facts runs in constant memory.
- [ ] Grouped `count` with 1M groups completes with memory proportional to the groups;
      with `query_memory_limit` below that, it fails with `QRY-012`.
- [ ] A recursive rule deriving 5M facts completes with memory proportional to the
      derived relation; `max_derived_facts` still stops a runaway rule.
- [ ] Closing a cursor after the first batch stops upstream work (operator counters).
- [ ] A cursor's answer is unchanged by writes and checkpoints made while it is open,
      including checkpoints that free pages it reads; the fact log likewise.
- [ ] A 1M-fact `transact` never holds more than one chunk of parsed facts.
- [ ] Python binding round trip with batches of 1,000; wall time and RSS in the envelope.
- [ ] Every existing `execute`, cursor and binding test passes unchanged.
- [ ] `[?e :a C]` reads only `C`'s AVET range (#518's envelope numbers hold).

## 11. Delivery

Sub-issues under #473, each a PR to `v3` with no closing keyword for #473:

1. **Snapshots and page quarantine** (§4): pins, quarantine in `PageAllocator`, cursor
   and fact-log snapshots, `API-013` deprecated.
2. **Operator core** (§5, §6.1 minus joins, §6.3 v1, §9 harness): `Batch`/`Cell`,
   provisional ids, `Scan` over all four indexes including #518, `Filter`, `Project`,
   `Limit`/`Offset`, single-pattern queries on the new engine, the oracle harness.
3. **Joins**: `IndexNestedLoopJoin`, `HashJoin`, join planning.
4. **Negation and disjunction**: `NotJoin`, `Union`, `Distinct`.
5. **Aggregation and ordering**: `Aggregate`, `Sort`, `Window`.
6. **Recursion**: `Recursive` with magic sets and stratification; rule queries move over.
7. **Memory accounting and cap** (§7): `QRY-012`, `OpenOptions::query_memory_limit`,
   bindings.
8. **Streaming transactions** (§8).
9. **Removal and measurement**: delete the old engine and oracle hook, run the envelope
   at 100K/1M/10M, update docs.

After #473: `:limit`/`:offset` syntax with the 10M acceptance test (#306/#310), then the
profiler (#185).
