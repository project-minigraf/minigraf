# Streaming query engine — Design

**Issue:** #473 (streaming, pull-based operator pipeline behind the cursor API; follow-up to #432)
**Sub-issues:** #543–#551 (§12)
**Milestone:** v3.0.0, branch `v3`. The `.graph` file format stays v8 and the WAL format stays v2.
**Folds in:** #518 (AVET reads, parked on branch `perf/518-avet-lookup`), #530 (ids until projection)
**Followed by:** #306/#310 (`:limit`/`:offset` syntax and the 10M-fact `:limit` acceptance test), #185 (profiler)
**Status:** design approved 2026-10-11, revised the same day (§11 lists the changes from the approved summary); not implemented

## 1. Terms

- **Committed facts**: facts in the B+trees that the active meta page references.
- **Generation**: the number in a meta page. Each checkpoint writes a meta page with the next generation. A generation names one set of tree roots.
- **Pending store**: `FactData` inside `FactStorage` (`src/graph/storage.rs`): an in-memory `Vec<Fact>` plus in-memory EAVT/AEVT `BTreeMap` indexes from key to position in that `Vec`. It holds every fact committed after the active generation. It is append-only between checkpoints; a checkpoint clears it (`post_checkpoint_clear`). For a file-backed database the WAL holds the same facts durably, and replay rebuilds the pending store on open. An in-memory database (`Minigraf::in_memory`) has no B+trees: every fact is in its pending store, permanently.
- **Checkpoint**: writes the pending store into the B+trees copy-on-write, writes a new meta page, deletes the WAL and clears the pending store. It runs when the WAL holds `wal_checkpoint_threshold` *entries* (transactions, default 1,000), on `Minigraf::checkpoint`, and on close.
- **Statement**: one `transact` or `retract` command, i.e. one `execute` call. A `WriteTransaction` holds one or more statements.
- **Chunk**: a bounded group of facts parsed from one statement (§8.1).

## 2. Problem

`Minigraf::query` returns a `Cursor` (#432), but `Cursor::from_result` wraps an answer that the executor has already computed in full:

1. `execute_query` fetches the facts its patterns touch as a `Vec<Fact>` (`filter_facts_for_query`, `selective_fact_fetch`).
2. `PatternMatcher` joins them into `Vec<Bindings>`, where `Bindings` is `HashMap<String, Value>`.
3. Post-processing (`not`, `or`, expressions, aggregation, window functions, projection) runs over that `Vec`.

Memory therefore follows the facts read, not the rows returned:

- `[?e :entity-type :type/function]` over 10M facts materialises every fact of the attribute, with UUIDs and owned strings, before returning any row.
- `count` over 10M facts holds 10M bindings before counting.
- A rule query copies its base facts into a `FactStorage` and keeps a `BTreeSet seen_facts` of every base and derived fact (`RecursiveEvaluator::evaluate_recursive_rules`).
- `execute` parses a whole `transact` into one `Vec<Fact>`. A `WriteTransaction` keeps all its staged facts in `pending_facts` until commit. The commit writes one WAL entry and then appends every fact to the pending store, where they stay until the next checkpoint.
- Closing a cursor early saves no work.

A cursor has no snapshot of its own today; its answer is fixed only because it is computed at open. Once operators read lazily, two things can change under an open cursor:

- A checkpoint clears the pending store.
- A later checkpoint can reuse pages that the cursor's generation still references.

`FactLog` (#430) avoids both by pinning: while a log is open, automatic checkpoints are skipped and `Minigraf::checkpoint` fails with `API-013`.

## 3. Goals and non-goals

Goals (#473):

1. Memory follows operator state, not input size. A pipelined operator holds at most one batch. Only the operators in §6.2 hold more.
2. Batches reach the caller as they are produced. `Limit` stops the work early, and `close` stops all work.
3. Aggregates run in one pass over their input.
4. A cursor's answer is the database as of the moment it opened, whatever is written or checkpointed afterwards.
5. A `transact` of any size holds at most one chunk of parsed facts in memory (file-backed databases; §8.4).

Non-goals:

- No change to Datalog semantics: every query returns the same rows, as a bag, as today.
- No change to the `.graph` format or the WAL format.
- No parallel execution.
- No spilling to disk and no `spill_dir`.
- No change to the public API: `Cursor`, `Batch`, `PreparedQuery`, `FactLog` and every binding keep their signatures.

## 4. Snapshots

### 4.1 Contents

A cursor's snapshot is taken once, at open, while holding the pending store's read lock (`FactData`'s `RwLock`). It consists of:

1. **The committed reader**: the `Arc<dyn CommittedReader>` of the active generation. It reads only the roots of that generation. An in-memory database has none.
2. **A pin** on that generation (§4.2), released when the cursor is closed or dropped.
3. **The pending store as of open**: an `Arc` of the current `FactData` and its length `n` at open. The cursor reads pending positions `0..n` only (§4.3).
4. **The valid-time "now"**: read once, at open, and used for the default `:valid-at`.

The lock is held only while these four are taken. After that the cursor holds no lock across `next_batch` calls. Writers and checkpoints proceed.

`Minigraf` handles and cursors share these objects by `Arc`. A cursor keeps the file and its lock open until the cursor is closed, as `FactLog` does today.

### 4.2 Pins and page quarantine

A checkpoint writes only pages that the active meta does not reference. The ids it stops referencing (`freed` in `btree::cow_insert`) go on the free list that the new meta records. Today the next checkpoint may hand those ids out again (`PageAllocator::alloc`) and write new contents to them, while a cursor of an older generation might still read them.

`PersistentFactStorage` therefore keeps two in-memory structures:

- **Pins**: a count of open cursors and fact logs per generation.
- **Quarantine**: for each checkpoint, the generation `g` it produced and the ids in its `freed` list. Those ids are referenced by generations below `g`, and by none from `g` on.

`PageAllocator::alloc` does not hand out an id that is in the quarantine entry of some generation `g`, while a pin on a generation below `g` exists. The id stays free on disk, and `finish_free_list` lists it again, as it already does for ids it read but did not hand out. When a pin is released, the quarantine entries no remaining pin needs are discarded.

Properties:

- **No format change.** On disk, a quarantined id is an ordinary free-list entry.
- **After a crash or reopen** there are no pins and no quarantine, so every free id can be reused.
- **File growth.** While a cursor is open, the file can grow by the pages that the checkpoints run meanwhile replace, as in LMDB with a long-lived reader. `verify()` still holds: quarantined ids are on the free list.
- **Page cache.** An id is not rewritten while a pinned generation can read it, so `PageCache` never returns another generation's page to a pinned reader.
- **Backends.** `FileBackend`, `MemoryBackend` and `BrowserBufferBackend` all go through `PageAllocator`, so the rule applies to each. A `BrowserDb` flush to IndexedDB writes only dirty pages, and a quarantined page is never dirty.

### 4.3 Pending facts under a checkpoint

A checkpoint today clears the pending store in place. It will instead replace `FactData`'s pending part (`facts`, `pending_keys`, `pending_indexes`) with a new, empty one. The old one stays alive while any cursor holds its `Arc`, and no writer appends to it after the swap.

A cursor's pending reads are therefore stable:

- Positions `0..n` of its `FactData` never move, because the store is append-only.
- Positions `≥ n` are facts committed after the cursor opened, so they are skipped.
- A pending index range is resumed by key after each batch, under a short read lock. A key inserted after open belongs to a position `≥ n`, so it is skipped too.

The snapshot copies nothing, so an in-memory database, whose facts are all pending, opens a cursor in constant time. This is the mechanism `FactLog` uses today (`log_snapshot` returns the reader and the pending length), extended so that a checkpoint can proceed.

### 4.4 Fact log

`Minigraf::fact_log` takes the same snapshot (§4.1) and the same pin. Checkpoints no longer wait for open logs: `log_pins()` no longer gates the automatic checkpoint (`db.rs:767`, `db.rs:1602`), and `Minigraf::checkpoint` and `rebuild_indexes` no longer fail with `API-013`. `API-013` stays in `ErrorCode` and in `docs/ERROR_REFERENCE.md`, marked deprecated, and is no longer raised. `rebuild_indexes` replaces every tree page, so with a pin held it grows the file by the size of the indexes until the pin is released.

## 5. Rows

### 5.1 Batches

Operators exchange an internal batch of at most 1,024 rows, stored as one column per variable. The planner assigns each variable a slot number, so operators address columns by index, never by name. The public `Batch` is built from internal batches by `Project` (§6.1).

### 5.2 Cells

```rust
enum Cell {
    Eid(u64),        // an entity: a DICT entity id, or a provisional id (§5.3)
    Iid(u64),        // an ident used as a value (a keyword): a DICT ident id, or a provisional id
    Val(ValueRef),   // a string, integer, float, boolean or null; long strings by value-page reference
    Meta(i64),       // a pseudo-attribute value (§5.5)
}
```

Joins, filters, sorts and aggregation compare and hash cells. `Project` converts the cells of the rows it returns into `Value`s. A query that returns ten rows converts ten rows (#530).

### 5.3 Ids of pending entities and idents

Pending facts hold UUIDs and strings, not ids. When a `Scan` reads a pending fact, it converts the entity, and a keyword value, to an id:

1. If the committed reader's DICT has the UUID or ident, use the DICT id.
2. Otherwise, use a **provisional id** from a map private to the cursor. Ids are assigned from the snapshot's `next_eid` / `next_iid` upwards, the first time each UUID or ident is met.

Every cell of a cursor is in one id space. Provisional ids are never written to disk. The provisional map is query state and counts towards the memory accounting of §7. For an in-memory database every entity and keyword is provisional, so the map grows with the distinct entities and keywords the query reads.

Query literals are converted the same way when the plan is built. A literal that the DICT and the pending store both lack gets a provisional id that no fact carries, so its `Scan` returns nothing.

### 5.4 Matching across kinds

The new engine reproduces the matcher's rules exactly (`PatternMatcher::match_component` and `normalize_join_value`, `src/query/datalog/matcher.rs`):

- **Keyword constant `:k`** matches the value `Keyword(:k)` and the value `Ref(uuidv5(:k))`. A `Scan` for `[?e :a :k]` reads both AVET ranges; these are #518's `ValueProbe`s, converted to ids.
- **UUID constant `u`** matches the value `Ref(u)` and any keyword value `:k` with `uuidv5(:k) = u`.
- **`nil`** matches `Null`.
- **Float constant** matches a float value within the matcher's epsilon. `Scan` reads every float of the attribute (#518's `ValueKind::Float`) and filters.
- **Join keys.** A keyword cell `Iid(k)` is hashed and compared as `Eid` of `uuidv5(k)`: the DICT id if that UUID has one, else a provisional id. The row keeps the cell bound first, as `matcher.rs` keeps the value bound first.
- **Float join keys.** Today the hash-join path compares floats with `Value`'s `Eq` (exact), and the nested-loop path compares them with `match_component` (epsilon). Which one applies depends on the plan. #545 fixes one rule for both paths, using the oracle tests (§9) to find the queries whose answer changes. Any change goes in `CHANGELOG.md`.

### 5.5 Pseudo-attributes

`:db/valid-from`, `:db/valid-to`, `:db/tx-count` and `:db/tx-id` read metadata of the fact matched by another pattern on the same entity. Today the matcher carries this metadata in hidden bindings named `__fvf_<uuid>`, `__fvt_<uuid>`, `__ftc_<uuid>` and `__fti_<uuid>`. In the new engine, `Scan` outputs these four values as `Meta` columns of the fact it matched, and the planner wires a pseudo-attribute pattern to them. `:db/valid-at` is a constant of the query.

## 6. Operators

Every operator implements `open`, `next_batch() -> Result<Option<Batch>>` and `close`. `close` closes the operator's children. Closing a cursor therefore closes every operator under it, and no operator does further work.

### 6.1 Operators that hold at most one batch

- **`Scan`** reads one key range of EAVT, AEVT, AVET or VAET, chosen from the pattern positions that are constants or bound:
  - It merges the committed reader's keys with the pending facts at positions `0..n` of the same range, per `(e, a, v)`.
  - It decides net-assert per `(e, a, v)` on keys, as `CommittedReader::live_facts` does (#379).
  - It applies the `:as-of` bound and the `:valid-at` / `:any-valid-time` test while reading.
  - Its output is in the index's order, and the planner records that order.
  - `[?e :a C]` reads the AVET range of `C`, with #518's encoding rules for strings, floats, keywords and refs.
  - `IndexNestedLoopJoin` (below) uses `Scan` to read one range per outer row.
- **`IndexNestedLoopJoin`**: for each outer batch, reads the inner pattern's range once per outer row, using the outer row's bound values.
- **`Filter`**: keeps the rows whose expression clause (`WhereClause::Expr` without a binding) is true.
- **`Bind`**: adds a column computed by an expression clause with a binding.
- **`Project`**: keeps the `:find` columns and converts cells to `Value`s (§5.2).
- **`Union`**: concatenates the outputs of the branches of an `or` / `or-join`. It is followed by `Distinct` (§6.2), because `or` removes duplicate rows today (`apply_or_clauses`).
- **`Limit`** / **`Offset`**: `Limit` stops pulling from its child after `n` rows and closes it.

### 6.2 Operators that hold more than one batch

Each of these holds state whose size depends on the data. They are the only operators counted by §7.

- **`HashJoin`**: builds a hash table on one input and streams the other. Used when neither input can be read by index for the join.
- **`AntiJoin`**: for `not` and `not-join`. Builds a hash set of the inner rows' join keys, then streams the outer rows whose key is absent.
- **`Distinct`**: a hash set of rows. Placed only where the engine removes duplicates today: after `or` / `or-join`, inside `count-distinct` and `sum-distinct`, and in the recursive relation. `:find` without these returns a bag, as today.
- **`Sort`**: sorts its input in memory. The planner omits it when the input order already satisfies it. Under a `Limit` of `n`, it keeps the best `n` rows only.
- **`Aggregate`**:
  - Without grouping variables it holds one accumulator per aggregate.
  - With grouping variables it holds one accumulator per group in a hash table.
  - When its input is ordered on the grouping variables, it holds one group at a time.
- **`Window`**: holds one partition at a time, ordered by its `:order-by`.
- **`Recursive`**: evaluates one stratum by semi-naive iteration:
  - It holds the accumulated relation and the current delta as hash sets of id rows. These replace the `FactStorage` copy and `seen_facts`.
  - Magic-set rewriting (`magic_sets.rs`) and stratification (`stratification.rs`) still run before planning.
  - The three existing `INT-020` limits keep their meaning: 1,000 iterations; `max_derived_facts` new facts per iteration; `max_results` facts derived in total (base facts excluded, #528).

### 6.3 Planner

`optimizer.rs` and `prepared.rs` produce an operator tree instead of a clause order for `PatternMatcher`. The planner:

1. Chooses each pattern's index from its constant or bound positions, choosing AVET for `[?e :a C]`.
2. Orders joins by the existing selectivity estimates. The join is an `IndexNestedLoopJoin` when the inner pattern has a constant or bound position that an index covers, otherwise a `HashJoin` that builds on the input with the smaller estimate.
3. Omits `Sort` when the input order satisfies it, and chooses the ordered `Aggregate` when the input is ordered on the grouping variables.
4. Places a `Limit` above a `Sort` inside the `Sort` (top `n`).

`optimizer.rs` does not reorder joins under the `wasm` feature today. The planner keeps that rule for join order only; index choice and operator choice run on every target.

A `PreparedQuery` keeps its operator tree and converts only its bind values and literals on each execution.

Each operator counts rows in, rows out, batches and, for §6.2 operators, bytes held. The profiler (#185) reads these counts.

## 7. Memory

- **No default limit and no spilling.** Only the §6.2 operators hold more than a batch, and the planner avoids them where index order allows.
- **Accounting.** Each §6.2 operator, and the provisional id map (§5.3), keeps a count of the bytes it holds. It grows its tables with `try_reserve`, so an allocation the allocator refuses becomes an error instead of an abort. On Linux with memory overcommit the allocator seldom refuses, so the limit below is the dependable control.
- **Optional limit.** `OpenOptions::query_memory_limit(bytes)` sets a limit, off by default (`usize::MAX`). When the bytes counted for one query would exceed it, the query fails with a new error, **`QRY-012` query memory limit exceeded**, naming the operator and the limit. The cursor is closed, and the database and other cursors are unaffected.
- **`max_results`** keeps its current meaning: the number of facts rules may derive in one query (§6.2). A cursor has no limit on the rows it returns.
- **Bindings.** Every binding gets `query_memory_limit` on its `OpenOptions` record and the `QRY-012` code. UniFFI shims map `usize::MAX` to `i64::MAX`, as for `wal_checkpoint_threshold`. A new `ErrorCode` variant breaks the UniFFI bindings until they regenerate, so the binding PRs follow the core PR.

## 8. Transactions

### 8.1 Parsing in chunks

`execute` passes `transact` and `retract` text to a parser that yields one statement at a time. Within a statement, it yields the statement's facts in chunks of `TX_CHUNK_FACTS` facts. This is a constant, not an option; #550 sets it by measurement, starting at 4,096.

The input text is the caller's string and is not counted; the bound is on parsed facts. Each chunk is validated as a statement is today:
- value limits;
- `check_valid_windows`: `API-019`, with a default `valid_from` taken as the transaction's `tx_id`;
- the one-window-per-triple rule (`API-011`) within the statement (§8.3).

### 8.2 Small and large transactions

A transaction stays **small** while its staged facts number at most `TX_CHUNK_FACTS`. A small transaction commits exactly as today:
1. `keep_last_statement`;
2. one `tx_id` and `tx_count`;
3. one WAL entry, written and synced;
4. the facts appended to the pending store.

When a staged fact would exceed `TX_CHUNK_FACTS`, the transaction becomes **large**, once and for good, while holding the write lock:

1. It fixes its `tx_count` (the next counter) and its `tx_id` (the current time). The write lock serialises transactions, so both are higher than every earlier transaction's.
2. It stamps the facts staged so far and writes them, as in step 3.
3. It writes each later chunk into **working trees**: copies of the active generation's five tree roots, extended copy-on-write with `btree::cow_insert` through the DICT `Encoder`, value pages and a `PageAllocator`. The working trees are the next generation's trees. Nothing moves their pages later. A page that a later chunk replaces is handled by its origin:
   - one the active generation references goes to the `freed` list of the commit;
   - one this transaction wrote returns to the transaction's allocator and can be reused within the transaction.
4. **Commit** is a checkpoint:
   - the pending store's facts are inserted into the same working trees;
   - the pages are synced;
   - the new meta page is written and synced;
   - the WAL is deleted;
   - the pending store is replaced (§4.3).

   Failure handling is that of a checkpoint today: a failure after anything may have been written marks the handle failed (`STG-046`), and the outcome is known only after reopening.
5. **Rollback** (explicit, or by dropping the transaction) discards the working roots, the transaction's allocator and the DICT `Encoder`.

Large transactions do not write to the WAL. Their durability point is the meta page write, as for a checkpoint.

`tx_id` changes meaning for large transactions only. It is the time the transaction became large, not the time it committed. Both lie inside the write lock, so the order of `tx_id`s and `tx_count`s across transactions is unchanged. `CHANGELOG.md` records this.

### 8.3 Last statement wins, and API-011, across chunks

Two rules apply to the facts of one transaction today:

- **One window per triple within a statement**: a statement that asserts one `(e, a, v)` with two valid-time windows is rejected with `API-011`. A rejected statement stages nothing, and the transaction stays usable.
- **The last statement wins**: when several statements write one `(e, a, v)`, only the last statement's facts for it are committed (`keep_last_statement`, #477).

A large transaction cannot keep its statements in memory, so it applies both rules to the working trees. When a statement starts, the transaction records the working roots as the **statement base**. Until the statement ends, pages that the base references are not reused (§4.2's rule, applied within the transaction).

For each fact of triple `T` in a chunk:

1. The EAVT keys of `T` with this transaction's `tx_count` that are in the statement base came from earlier statements. They are removed from the working trees in all four indexes. EAVT keys are covering, so each one gives the other three keys.
2. The EAVT keys of `T` with this transaction's `tx_count` that are in the working trees but not in the base came from this statement. If one has a different window, the statement fails with `API-011`. The working roots are set back to the statement base, the pages written since are returned to the allocator, and the transaction stays usable.

When the statement ends, its base is dropped.

This needs a copy-on-write delete in `btree.rs`, which does not exist today. #550 adds it, either as part of `cow_insert` or alongside it. A leaf that a delete leaves with no keys is removed from its parent. A leaf left with fewer keys stays as it is: `verify()` checks key order and separator bounds, not fill.

### 8.4 Reading your own writes

A query inside a `WriteTransaction` reads a snapshot (§4.1) that consists of:
- for a large transaction, the working trees, in place of the active generation;
- the pending store;
- the transaction's staged facts that are not yet in the working trees: all of them for a small transaction, at most one chunk for a large one.

### 8.5 Scope

§8.2–§8.4 apply to file-backed databases.

An in-memory database keeps every fact in its pending store (§1), so a large transaction's facts end up in memory however it is written. It uses the chunked parser (§8.1) and the small-transaction path at every size.

`BrowserDb` writes through its own path (`src/browser/mod.rs`), which checkpoints after every `transact` (`pfs.save()`) into in-memory pages (`BrowserBufferBackend`). It keeps that path; its pages are in memory in any case.

Schema validation and the v8 multi-value fix (#371, #287) are unaffected.

## 9. Migration and testing

- The new engine is built in `src/query/engine/`, beside the existing executor.
  - A query runs on the new engine when every clause it uses has an operator, and on the existing executor otherwise.
  - Each sub-issue moves more clause kinds over.
- **Oracle.** The existing executor stays available to tests as an oracle (under `#[cfg(test)]`, and under a `test-oracle` Cargo feature for `tests/`). Differential tests run each query through both engines and compare the rows as bags, over:
  - `tests/model_based_test.rs` with `tests/reference_model/`;
  - `tests/property_test.rs`;
  - `tests/golden_corpus_test.rs`;
  - the other integration suites, through a test hook on `execute`.
- When every clause kind runs on the new engine, the existing executor, the oracle feature and the hook are deleted. This happens before v3.0.0 (#551).
- `tests/cursor_test.rs` and every binding's test suite pass, unchanged, after each sub-issue.
- **Measurement.** After each sub-issue, the #394 envelope at 100K, 1M and 10M facts on an otherwise idle machine. The #379 point queries must not regress.

## 10. Acceptance tests

From #473. The `:limit` test moves to #306/#310, since it needs the syntax.

- [ ] `count` over 10M facts holds memory independent of the number of facts.
- [ ] Grouped `count` with 1M groups completes, holding memory proportional to the number of groups. With `query_memory_limit` below that, it fails with `QRY-012`.
- [ ] A recursive rule deriving 5M facts completes, holding memory proportional to the derived relation. `max_derived_facts` and `max_results` still stop a runaway rule with `INT-020`.
- [ ] Closing a cursor after its first batch: the operator counters show no further rows read.
- [ ] A cursor's answer is unchanged by writes and checkpoints made while it is open, including checkpoints that free pages it reads. The same holds for a fact log.
- [ ] A 1M-fact `transact` on a file-backed database holds at most `TX_CHUNK_FACTS` parsed facts at any time.
- [ ] Python binding round trip with batches of 1,000 rows; wall time and peak RSS recorded in the #394 envelope.
- [ ] Every existing `execute`, cursor and binding test passes unchanged.
- [ ] `[?e :a C]` reads only the AVET range of `C`; #518's envelope result (1M facts: 274 ms → 11 µs) holds.

## 11. Changes from the approved summary

The design approved on 2026-10-11 changed as follows when checked against the code:

1. **Pending facts are shared, not copied, at open (§4.3).** An in-memory database keeps every fact pending, so a copy would copy the database.
2. **Large transactions commit as a checkpoint (§8.2), not as WAL chunks.** Chunks in the WAL would still be appended to the pending store at commit, so a 1M-fact transaction would end up in memory. WAL chunks streamed straight into the trees at commit would also work, but would need a WAL format change, two writes per fact, and an unindexed WAL for reading your own writes (§8.4).
3. **Last statement wins and `API-011` across chunks (§8.3)** need a statement base and a B+tree delete.
4. **`max_results`** limits the facts rules derive (#528), not the rows a query returns. #473's text said otherwise.
5. **Float join keys (§5.4)** differ between today's two join paths. #545 settles one rule.

## 12. Delivery

Sub-issues of #473, in order. Each is one or more PRs to `v3`, and none uses a closing keyword for #473.

1. **#543 Snapshots and page quarantine** (§4): pins, quarantine in `PageAllocator`, the pending-store swap at checkpoint, cursor and fact-log snapshots, `API-013` deprecated.
2. **#544 Operator core** (§5, §6.1 without the joins, §6.3, §9): batches and cells, provisional ids, pseudo-attribute columns, `Scan` over all four indexes including #518 (rebased from `perf/518-avet-lookup`), `Filter`, `Bind`, `Project`, `Limit`/`Offset`, single-pattern queries on the new engine, the oracle tests.
3. **#545 Joins**: `IndexNestedLoopJoin`, `HashJoin`, join planning, the float join rule (§5.4).
4. **#546 Negation and disjunction**: `AntiJoin`, `Union`, `Distinct`.
5. **#547 Aggregation and ordering**: `Aggregate`, `Sort`, `Window`.
6. **#548 Recursion**: `Recursive`, with rule queries moved to the new engine.
7. **#549 Memory accounting and limit** (§7): `QRY-012`, `OpenOptions::query_memory_limit`, bindings.
8. **#550 Streaming transactions** (§8): chunked parser, large transactions, statement bases, the B+tree delete.
9. **#551 Removal and measurement**: delete the existing executor and the oracle, envelope at 100K/1M/10M, documentation.

After #473: `:limit`/`:offset` syntax with the 10M-fact acceptance test (#306/#310), then the profiler (#185).
