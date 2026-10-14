# Streaming query engine — Design

**Issue:** #473 (streaming, pull-based operator pipeline behind the cursor API; follow-up to #432)
**Sub-issues:** #543–#551 (§12)
**Milestone:** v3.0.0, branch `v3`. The `.graph` file format stays v8 and the WAL format stays v2.
**Folds in:** #518 (AVET reads, parked on branch `perf/518-avet-lookup`), #530 (ids until projection)
**Followed by:** #306 (`:limit`/`:offset` syntax and the 10M-fact `:limit` acceptance test), #185 (profiler)
**Status:** design approved 2026-10-11, revised the same day (§11 lists the changes from the approved summary); not implemented

## 1. Terms

- **Committed facts**: facts in the B+trees that the active meta page references.
- **Generation**: the `u64` in a meta page. Each checkpoint writes a meta page whose generation is one more than the active one, so generations are totally ordered by number, and a lower number is an older generation. A generation names one set of tree roots.
- **`FactData`**: the struct behind `FactStorage`'s `Arc<RwLock<FactData>>` (`src/graph/storage.rs`). It holds the committed reader (`committed`, §4.1) and the pending store.
- **Pending store**: the part of `FactData` that holds facts committed after the active generation: an in-memory `Vec<Fact>` (`facts`), a duplicate-detection set (`pending_keys`), and in-memory EAVT/AEVT `BTreeMap` indexes from key to position in `facts` (`pending_indexes`). It is append-only between checkpoints; a checkpoint clears it (`post_checkpoint_clear`). For a file-backed database the WAL holds the same facts durably, and replay rebuilds the pending store on open. An in-memory database (`Minigraf::in_memory`) has no B+trees: every fact is in its pending store, permanently.
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

A cursor today retains no storage snapshot: `Minigraf::query` runs the executor to completion and then builds the cursor from the result (`db.rs:819–831`), so the answer cannot change. Once operators read lazily, two things can change under an open cursor:

- A checkpoint clears the pending store.
- The pages a checkpoint stops referencing go on the free list, and a later checkpoint can hand them out and write new contents to them while the cursor still reads the older generation.

`FactLog` (#430) avoids both today: while a log is open, `execute` skips the automatic checkpoint (`db.rs:764–769`), and `PersistentFactStorage::save` and `rebuild_indexes` fail with `API-013` (`persistent_facts.rs:417–423`, `707–712`).

Two windows in today's code also matter once a snapshot can be taken at any moment:

- `WriteTransaction::commit` appends its facts to the pending store one `load_fact` call at a time (`db.rs:1594–1597`), each under its own write lock, so a reader between two calls sees part of the transaction.
- A checkpoint sets the new committed reader (`set_committed_reader`, via `activate`) and clears the pending store (`post_checkpoint_clear`) under two separate write locks, so a reader between the two sees the checkpointed facts both as committed and as pending.

§4.1 removes both windows.

## 3. Goals and non-goals

Goals (#473):

1. Memory follows operator state, not input size. A pipelined operator holds at most one batch. Only the operators in §6.2 hold more.
2. Batches reach the caller as they are produced. `Limit` stops the work early, and `close` stops all work.
3. Aggregates run in one pass over their input.
4. A cursor's answer is the database as it was when the cursor took its snapshot (§4.1): every transaction whose facts were in the pending store or the committed trees at that instant, and no other, whatever is written or checkpointed afterwards.
5. A `transact` of any size holds at most `TX_CHUNK_FACTS` parsed facts in memory (file-backed databases; §8).

Non-goals:

- No change to Datalog semantics: every query returns the same rows, as a bag, as today.
- No change to the `.graph` format or the WAL format.
- No parallel execution.
- No spilling to disk and no `spill_dir`.
- No change to the public API: `Cursor`, `Batch`, `PreparedQuery`, `FactLog` and every binding keep their signatures.

## 4. Snapshots

### 4.1 Contents

`FactData` changes shape: the pending store moves into its own `Arc<PendingStore>`, so that `FactData` holds `committed: Option<Arc<dyn CommittedReader>>` and `pending: Arc<PendingStore>`. `PendingStore` keeps `facts`, `pending_keys` and `pending_indexes` behind its own `RwLock`.

A cursor's snapshot is taken once, at open, while holding `FactData`'s read lock. It is not taken under the database's write lock (`Inner::write_lock`), so opening a cursor never waits for a write transaction. The snapshot consists of:

1. **The committed reader**: a clone of `committed`, the `Arc` of the active generation's reader. It reads only the roots of that generation. An in-memory database has none.
2. **A pin** on that generation (§4.2), released when the cursor is closed or dropped.
3. **The pending store**: a clone of `pending`, and its length `n` (`facts.len()`). The cursor reads pending positions `0..n` only (§4.3).
4. **The valid-time "now"**: read once, here, and used for the default `:valid-at`.

After that the cursor holds no lock across `next_batch` calls, and writers and checkpoints proceed.

Two rules make every snapshot whole:

- **A commit appends all of a transaction's facts to the pending store under one write lock.** `transact_batch` and `retract` already do (`storage.rs:172–177`, `274–279`). `WriteTransaction::commit` and WAL replay change from one `load_fact` per fact to one batch append.
- **A checkpoint publishes in one step.** After the new meta page is synced, it sets `committed` to the new generation's reader and `pending` to a new, empty `PendingStore`, both under one write lock on `FactData`. It no longer calls `post_checkpoint_clear`.

`Minigraf` handles and cursors share these objects by `Arc`. A cursor keeps the file and its lock open until the cursor is closed, as `FactLog` does today.

### 4.2 Pins and page quarantine

A checkpoint writes only pages that the active meta does not reference. The ids it stops referencing (`freed` in `btree::cow_insert`) go on the free list that the new meta records. Today the next checkpoint may hand those ids out again (`PageAllocator::alloc`) and write new contents to them, while a cursor of an older generation might still read them.

`PersistentFactStorage` therefore keeps two in-memory structures:

- **Pins**: for each generation, the number of open cursors and fact logs whose snapshot reads it.
- **Quarantine**: for each checkpoint, the generation `g` it produced and the ids it stopped referencing: its `freed` list, plus every page of the trees that `rebuild_indexes` replaces. Generation `g − 1` references those ids; generations `g` and later do not. An id that was already free in generation `g − 1` is not in the entry: no pinned reader of `g − 1` or later reads it, and a reader older than `g − 1` is protected by the entry of the checkpoint that freed it.

The rule: **an id in the entry of generation `g` is not handed out while any generation below `g` is pinned.** Equivalently, with `p` the lowest pinned generation, every entry with `g > p` is in force.

- `PageAllocator::alloc` skips an id in force. The id stays free on disk, and `finish_free_list` lists it again, as it already does for ids it read but did not hand out.
- `rebuild_indexes` builds its pool from the free list minus the pages it keeps (`persistent_facts.rs:540–567`). It also removes the ids in force.
- When a pin is released and `p` rises, each entry with `g ≤ p` is discarded and its ids become ordinary free ids. With no pins, every entry is discarded.

Properties:

- **No format change.** On disk, a quarantined id is an ordinary free-list entry.
- **After a crash or reopen** there are no pins and no quarantine, so every free id can be reused.
- **File growth.** While a cursor is open, the file can grow by the pages that the checkpoints run meanwhile replace, as in LMDB with a long-lived reader. `verify()` still holds: quarantined ids are on the free list.
- **Page cache.** An id in force is not rewritten, so `PageCache` never returns another generation's contents for it to a pinned reader.
- **Backends.** `FileBackend`, `MemoryBackend` and `BrowserBufferBackend` all allocate through `PageAllocator`, so the rule applies to each.
- **`BrowserDb`.** After each commit, `BrowserDb` today drops the bytes of the pages that commit freed, from the in-memory buffer and from IndexedDB (`take_released`, `BrowserBufferBackend::release`, `browser/mod.rs:358–362`, `464–470`; #440). That assumes a freed page is never read again, which a pinned cursor breaks. `take_released` will therefore withhold the ids in force and return them once their entry is discarded, so their bytes are released at a later commit. A quarantined page is not written, so it is never in the dirty set that a flush sends to IndexedDB.

### 4.3 Pending facts under a checkpoint

A checkpoint replaces `FactData`'s `pending` with a new, empty `PendingStore` (§4.1). The old `PendingStore` stays alive while any cursor holds its `Arc`, and no writer appends to it after the swap.

A cursor's pending reads are therefore stable:

- Positions `0..n` of its `PendingStore` never move, because the store is append-only.
- Positions `≥ n` are facts committed after the cursor opened, so they are skipped.
- A pending index range is resumed by key after each batch, under a short read lock. A key inserted after open belongs to a position `≥ n`, so it is skipped too.

The snapshot copies nothing, so an in-memory database, whose facts are all pending, opens a cursor in constant time. `FactLog` today snapshots the same way (`log_snapshot` returns the committed reader and the pending length), but it must block checkpoints because a checkpoint clears the pending store in place.

### 4.4 Fact log

`Minigraf::fact_log` takes the same snapshot (§4.1) and the same pin. It no longer holds the write lock while it opens (`db.rs:866–871`); the two rules of §4.1 give it a whole snapshot. Checkpoints no longer wait for open logs: `execute` and `WriteTransaction::commit` no longer skip the automatic checkpoint when `log_pins()` is non-zero (`db.rs:767`, `db.rs:1602`), and `PersistentFactStorage::save` and `rebuild_indexes` no longer fail with `API-013`. `API-013` stays in `ErrorCode` and in `docs/ERROR_REFERENCE.md`, marked deprecated, and is no longer raised. `rebuild_indexes` replaces every tree page, so with a pin held it grows the file by the size of the indexes until the pin is released.

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

Query literals are converted the same way when the plan is built, after the cross-kind expansion of §5.4: a keyword literal `:k` becomes both `Iid(:k)` and the `Eid` of `uuidv5(:k)`, and a UUID literal `u` becomes both `Eid(u)` and `Iid` of every keyword whose `uuidv5` is `u`, which the `Scan` finds through #518's `ValueKind::Keyword` range. Each converted form that the DICT and the pending store both lack gets a provisional id that no fact carries, so the range for that form is empty.

### 5.4 Matching across kinds

The new engine reproduces the matcher's rules exactly (`PatternMatcher::match_component` and `normalize_join_value`, `src/query/datalog/matcher.rs`):

- **Keyword constant `:k`** matches the value `Keyword(:k)` and the value `Ref(uuidv5(:k))`. A `Scan` for `[?e :a :k]` reads both AVET ranges; these are #518's `ValueProbe`s, converted to ids.
- **UUID constant `u`** matches the value `Ref(u)` and any keyword value `:k` with `uuidv5(:k) = u`.
- **`nil`** matches `Null`.
- **Float constant** matches a float value within the matcher's epsilon. `Scan` reads every float of the attribute (#518's `ValueKind::Float`) and filters.
- **Join keys.** For hashing and equality only, a keyword cell `Iid(k)` is treated as the `Eid` of `uuidv5(k)`: the DICT id if that UUID has one, else a provisional id. The cell stored in the output row is not changed: it is one of the two input cells, and `Project` returns its kind (a keyword or a ref).
- **Which input cell a row keeps.** When the two sides of a join hold different kinds for one variable, `matcher.rs` keeps the value bound by the pattern it evaluated first (`merged = existing`, `matcher.rs:428–453`). Evaluation order is the optimizer's order, so today the projected kind depends on the plan. #545 fixes one rule, defined on the query text rather than on operator order (the first pattern in `:where` order that binds the variable), and records the queries whose answer changes in `CHANGELOG.md`.
- **Float join keys.** Today the hash-join path compares floats with `Value`'s `Eq` (exact), and the nested-loop path compares them with `match_component` (epsilon). Which one applies depends on the plan. #545 fixes one rule for both paths, using the oracle tests (§9) to find the queries whose answer changes. Any change goes in `CHANGELOG.md`.

### 5.5 Pseudo-attributes

`:db/valid-from`, `:db/valid-to`, `:db/tx-count` and `:db/tx-id` read metadata of the fact matched by another pattern on the same entity. A query that uses one of these four without `:valid-at :any-valid-time` fails with `INT-026` before it reads anything (`executor.rs:502–507`); the new engine keeps that check. Today the matcher carries this metadata in hidden bindings named `__fvf_<uuid>`, `__fvt_<uuid>`, `__ftc_<uuid>` and `__fti_<uuid>`. In the new engine, `Scan` outputs these four values as `Meta` columns of the fact it matched, and the planner wires a pseudo-attribute pattern to them. `:db/valid-at` is a constant of the query.

## 6. Operators

The new engine reproduces every observable behaviour of the existing executor, including the rules stated in this section; the oracle tests (§9) check it. Where today's answer depends on the plan (§5.4), the sub-issue named there fixes one rule.

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
- **`Filter`**: keeps the rows for which an expression clause without a binding (`WhereClause::Expr { binding: None }`) evaluates to a truthy value (`is_truthy`).
- **`Bind`**: adds a column holding the value of an expression clause with a binding.
- **Expression errors.** For `Filter` and `Bind`, a row whose expression fails to evaluate (a type mismatch, division by zero, an unbound variable) is dropped, not reported (`executor.rs:565–580`). An unknown function name is reported as an error when the query is planned, before any row is read (`executor.rs:521–531`).
- **`Project`**: keeps the `:find` columns and converts cells to `Value`s (§5.2).
- **`Union`**: for `or` and `or-join`. `apply_or_clauses` (`executor.rs:1833–2169`) evaluates them in one of two ways today, and the new engine reproduces each:
  - **Seeded** (some branch contains `not` / `not-join`): each branch is evaluated with every incoming row as input; the branch outputs are concatenated, and duplicates are removed over the user-visible variables (names not starting with `__`).
  - **Unseeded** (otherwise): each branch is evaluated once, without input rows, and the branch outputs are concatenated with duplicates removed. The result is then hash-joined with the incoming rows on the user-visible variables that both bind, by exact `Value` equality; a merged row keeps the incoming row's value for every variable the incoming row binds, and duplicates are removed over the merged row. When no variable is shared, the result replaces the incoming rows. Branches may be skipped once earlier branches cover every incoming row (#250).
  - Both ways deduplicate, so `Union` is followed by `Distinct` (§6.2). Whether the two ways give the same rows for every query is not established; the oracle tests (§9) compare the new engine with whichever way today's executor takes.
- **`Limit`** / **`Offset`**: `Limit` stops pulling from its child after `n` rows and closes it.

### 6.2 Operators that hold more than one batch

Each of these holds state whose size depends on the data. They are the only operators counted by §7.

- **`HashJoin`**: builds a hash table on one input and streams the other. Used when neither input can be read by index for the join.
- **`AntiJoin`**: for `not` and `not-join`. Builds a hash set of the inner rows' join keys, then streams the outer rows whose key is absent.
- **`Distinct`**: a hash set of rows. Placed only where the engine removes duplicates today: after `or` / `or-join` (§6.1), inside `count-distinct` and `sum-distinct`, and in the recursive relation. A `:find` without these returns a bag, as today.
- **`Sort`**: sorts its input in memory. The planner omits it when the input order already satisfies it. Under a `Limit` of `n`, it keeps the best `n` rows only.
- **`Aggregate`**:
  - Without grouping variables it holds one accumulator per aggregate.
  - With grouping variables it holds one accumulator per group in a hash table.
  - When its input is ordered on the grouping variables, it holds one group at a time.
  - The grouping variables are the plain variables of `:find`, plus the `:with` variables when `:find` has no window function. With a window function, `:with` variables do not group (`executor.rs:1422–1443`).
  - With no input rows, a query whose `:find` is only `count` / `count-distinct` aggregates returns one row of zeros; any other aggregate query returns no rows (`executor.rs:1405–1419`).
  - `Null` inputs are skipped. Registered aggregates (`FunctionRegistry`) run through their init, step and finalise functions. Groups and errors are handled as `compute_aggregation` handles them (`executor.rs:1483–1525`).
- **`Window`**: holds one partition at a time, ordered by its `:order-by`. It computes `sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number` and registered window functions, with ties and output order as `apply_window_functions` computes them (`executor.rs:1540` onwards).
- **`Recursive`**: evaluates one stratum by semi-naive iteration:
  - It holds the accumulated relation and the current delta as hash sets of id rows. These replace the `FactStorage` copy and `seen_facts`.
  - Its base facts are the snapshot's facts after the `:as-of` and `:valid-at` filters, as today (`executor.rs:686–729`). Magic-set rewriting (`magic_sets.rs`) and its seed facts, and stratification (`stratification.rs`), still run before planning; the seed facts are visible only to that query.
  - It raises `INT-020` at two limits: 1,000 iterations, and `max_derived_facts` facts derived in total, base facts not counted. #553 merges today's `max_results` (total derived facts, #528) and `max_derived_facts` (new facts per iteration) into that one limit, keeping `max_results` as a deprecated alias. If #553 has not landed when #548 does, `Recursive` enforces both of today's limits.

### 6.3 Planner

`optimizer.rs` and `prepared.rs` produce an operator tree instead of a clause order for `PatternMatcher`. The planner:

1. Chooses each pattern's index from its constant or bound positions, choosing AVET for `[?e :a C]`.
2. Orders joins by the existing selectivity estimates. The join is an `IndexNestedLoopJoin` when the inner pattern has a constant or bound position that an index covers, otherwise a `HashJoin` that builds on the input with the smaller estimate.
3. Omits `Sort` when the input order satisfies it, and chooses the ordered `Aggregate` when the input is ordered on the grouping variables.
4. Places a `Limit` above a `Sort` inside the `Sort` (top `n`).

Under the `wasm` feature the executor today orders by cost neither joins (`optimizer.rs`), nor `or` branches, nor deferred negation clauses (`executor.rs:625–629`, `1844–1850`). The planner keeps those three exceptions; index choice and operator choice run on every target.

A `PreparedQuery` keeps its operator tree and converts only its bind values and literals on each execution.

Each operator counts rows in, rows out, batches and, for §6.2 operators, bytes held. The profiler (#185) reads these counts.

## 7. Memory

- **No default limit and no spilling.** Only the §6.2 operators hold more than a batch, and the planner avoids them where index order allows.
- **Accounting.** Each §6.2 operator, and the provisional id map (§5.3), keeps a count of the bytes it holds. It grows its tables with `try_reserve`, so an allocation the allocator refuses becomes an error instead of an abort. On Linux with memory overcommit the allocator seldom refuses, so the limit below is the dependable control.
- **Optional limit.** `OpenOptions::query_memory_limit(bytes)` sets a limit, off by default (`usize::MAX`). When the bytes counted for one query would exceed it, the query fails with a new error, **`QRY-012` query memory limit exceeded**, naming the operator and the limit. The cursor is closed, and the database and other cursors are unaffected.
- **`max_derived_facts`** (§6.2, #553) limits the facts rules derive in one query. Neither it nor `max_results` limits the rows a query returns, and a cursor has no such limit.
- **Bindings.** Every binding gets `query_memory_limit` on its `OpenOptions` record and the `QRY-012` code. UniFFI shims map `usize::MAX` to `i64::MAX`, as for `wal_checkpoint_threshold`. A new `ErrorCode` variant breaks the UniFFI bindings until they regenerate, so the binding PRs follow the core PR.

## 8. Transactions

### 8.1 Parsing in chunks

`execute` passes `transact` and `retract` text to a parser that yields one statement at a time. Within a statement, it yields the statement's facts in chunks of `TX_CHUNK_FACTS` facts. This is a constant, not an option; #550 sets it by measurement, starting at 4,096.

The input text is the caller's string and is not counted; the bound is on parsed facts. Each chunk is validated as a statement is today:
- value limits;
- `check_valid_windows`: `API-019`, with a default `valid_from` taken as the transaction's `tx_id`;
- the one-window-per-triple rule (`API-011`) within the statement (§8.3).

### 8.2 Small and large transactions

A `WriteTransaction` today stages each statement's facts in `pending_facts` with a **synthetic** `tx_count`, one per statement and increasing (`stage_pending_facts`, `db.rs:1524–1536`). Reads inside the transaction and `keep_last_statement` use these synthetic counters. At commit the transaction takes its one real `tx_count` and `tx_id`.

A transaction stays **small** while its staged facts number at most `TX_CHUNK_FACTS`. A small transaction commits as today, except that step 4 is one batch append (§4.1):
1. `keep_last_statement` over the staged facts;
2. one real `tx_id` and `tx_count`, stamped on every fact;
3. one WAL entry, written and synced;
4. the facts appended to the pending store under one write lock.

When staging a fact would make the count exceed `TX_CHUNK_FACTS`, the transaction becomes **large**, once and for good. The write lock is held throughout, as for every `WriteTransaction` today.

1. It fixes its real `tx_count` (the next counter) and its `tx_id` (the current time). The write lock serialises transactions, so both are higher than every earlier transaction's.
2. It applies `keep_last_statement` to the staged facts of the statements before the current one, stamps them, and inserts them into the **working trees**:
   - The working trees start as the active generation's five tree roots and grow copy-on-write through `btree::cow_insert`, the DICT `Encoder`, value pages and a `PageAllocator` of the transaction's own.
   - The working trees are the next generation's trees: no later step copies or moves their pages.
3. It records the statement base for the current statement (§8.3), then inserts the current statement's staged facts, and each later chunk, by the rules of §8.3.
4. A page that an insert replaces is handled by its origin:
   - a page the active generation references goes to the commit's `freed` list;
   - a page this transaction wrote returns to the transaction's allocator, except while a statement base references it (§8.3).
5. **Commit** is a checkpoint, in this order:
   1. The pending store's facts are inserted into the working trees.
   2. All pages written are synced.
   3. The new meta page is written and synced. Its `last_checkpointed_tx_count` is this transaction's `tx_count`. **This write is the commit point.**
   4. `committed` and `pending` are published (§4.1).
   5. The WAL is deleted, as a checkpoint deletes it.

**Crash recovery:**
- A crash before step 3 completes leaves the previous meta page active. The transaction is absent, and the WAL replays the earlier transactions as usual.
- A crash after step 3 leaves the new meta page active, and possibly the old WAL. Every WAL entry has a `tx_count` at most this transaction's, so replay skips all of them as already checkpointed. This is the existing rule for a WAL left behind by a checkpoint.

**Failures** are handled as for a checkpoint today:
- A failure in steps 1–3, after any page may have been written, marks the handle failed (`STG-046`). Whether the transaction committed is known only after reopening, as when a WAL sync fails today.
- A failure in step 5 happens after the commit point, so the transaction is committed, and the error is handled as a checkpoint's failed WAL deletion is today.

6. **Rollback** (explicit, or by dropping the transaction) discards the working roots, the transaction's allocator and its DICT `Encoder`. Nothing on disk changes: every page the transaction wrote was free in the active generation, or lay beyond its `page_count`.

Large transactions write nothing to the WAL. Their durability point is the meta page write.

`tx_id` changes meaning for large transactions only: it is the time the transaction became large, not the time it committed. A default `valid_from` is that `tx_id`. Both times lie inside the write lock, so the order of `tx_id`s and `tx_count`s across transactions is unchanged. `CHANGELOG.md` records this.

### 8.3 Last statement wins, and API-011, across chunks

Two rules apply to the facts of one transaction today:

- **One window per triple within a statement.** A statement that asserts one `(e, a, v)` with two different valid-time windows is rejected with `API-011` (`check_one_window_per_triple`, which looks at assertions only, `storage.rs:607–621`). A rejected statement stages nothing, and the transaction stays usable.
- **The last statement wins.** When several statements write records (assertions or retractions) of one `(e, a, v)`, only the records of the last of those statements are committed (`keep_last_statement`, `db.rs:1697–1722`, #477).

A `transact` statement holds only assertions and a `retract` statement only retractions, so the records of one triple within one statement are all of one kind.

A large transaction cannot keep its statements in memory, so it applies both rules to the working trees. When a statement starts, the transaction records its **statement base**, the state that a rejected statement returns to:
- the five working roots;
- the allocator's state;
- the DICT `Encoder`'s next ids;
- the value-page append position.

Until the statement ends, a page that the base references is not reused, even if the transaction wrote it.

For each record of triple `T` in a chunk, in order:

1. **Earlier statements.** The EAVT keys of `T` with this transaction's `tx_count` that are in the statement base came from earlier statements. They are removed from all four indexes of the working trees. EAVT keys are covering, so each one gives the other three keys.
2. **This statement.** For an assertion: if the working trees hold an assertion key of `T` with this `tx_count` that is not in the base and has a different window, the statement fails with `API-011`. A key equal to one already present is not inserted again.
3. Otherwise the record's keys are inserted into all four indexes.

When a statement fails (`API-011`, or any other validation error), the transaction returns to the statement base:
- the working roots are set back;
- the allocator's ids handed out since the base return to its free set;
- the `Encoder` forgets the ids it assigned since the base;
- the value-page append position is set back;
- the page-cache entries of the returned ids are dropped.

The transaction stays usable. When a statement ends without failing, its base is dropped, and the pages the base alone referenced can be reused.

This needs a copy-on-write delete in `btree.rs`, which does not exist today. #550 adds it, either as part of `cow_insert` or alongside it. A leaf that a delete leaves with no keys is removed from its parent. A leaf left with fewer keys stays as it is: `verify()` checks key order and separator bounds, not fill.

### 8.4 Reading your own writes

A query inside a `WriteTransaction` reads a snapshot (§4.1) whose parts are:
- **committed:** the working trees for a large transaction, in place of the active generation;
- **pending:** the pending store;
- **staged:** the transaction's staged facts not yet in the working trees. For a small transaction that is all of them, with their synthetic counters as today. For a large transaction it is empty: a query runs between statements, and by then every finished statement is in the working trees.

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

From #473. The `:limit` test moves to #306, since it needs the syntax.

- [ ] `count` over 10M facts holds memory independent of the number of facts.
- [ ] Grouped `count` with 1M groups completes, holding memory proportional to the number of groups. With `query_memory_limit` below that, it fails with `QRY-012`.
- [ ] A recursive rule deriving 5M facts completes, holding memory proportional to the derived relation. `max_derived_facts` (#553) still stops a runaway rule with `INT-020`.
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
4. **`max_results`** limits the facts rules derive (#528), not the rows a query returns; #473's text said otherwise. #553 merges it into `max_derived_facts` (v3.0.0).
5. **Float join keys (§5.4)** differ between today's two join paths. #545 settles one rule.

After an independent review against the code (Codex, 2026-10-11):

6. **Snapshots are whole (§4.1).** `WriteTransaction::commit` and WAL replay append a transaction in one batch, and a checkpoint publishes the new committed reader and the empty pending store under one lock. Today both happen in steps (§2).
7. **`rebuild_indexes` and `BrowserDb` respect the quarantine (§4.2).** `rebuild_indexes` builds its page pool from the free list, and `BrowserDb` drops the bytes of freed pages after each commit (#440); both would otherwise reuse or drop pages a pinned cursor reads.
8. **`FactLog` no longer takes the write lock to open (§4.4).**
9. **Behaviours of today's executor made explicit (§5.5, §6):** `INT-026` for pseudo-attributes, expression errors drop the row, the two `or` evaluation paths, `:with` grouping, the zero row for `count`-only queries, window functions, the `wasm` exceptions.
10. **Large-transaction commit order, crash recovery and statement rollback stated in full (§8.2, §8.3).**

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

After #473: `:limit`/`:offset` syntax with the 10M-fact acceptance test (#306), then the profiler (#185).
