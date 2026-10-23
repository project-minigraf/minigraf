# Streaming transactions (#550) — Plan

Spec: `docs/superpowers/specs/2026-10-21-streaming-transactions-design.md` (merged as #683),
called "the spec" below. PRs target `v3`, merge commits. None uses a closing keyword for #473 or
#550; #550 closes by hand after PR 3.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–9: lexer, statement reader (pass 1 and pass 2), `cow_update` (spec §3, §6) | `feat/550-parser` |
| 2 | 10–15: `PageAllocator::release`, large `Minigraf::execute` statements, WAL guard, open-time dirty rule, crash and fault tests, 1M acceptance (spec §4) | `feat/550-large-statements` |
| 3 | 16–21: statement bases, snapshot reads in a `WriteTransaction`, measurement, `TX_CHUNK_FACTS`, docs (spec §5, §7, §8.2) | `feat/550-statement-bases` |

Each PR leaves `v3` with no unused code outside `cfg(test)` and updates the test counts. Only
PR 3 changes `CHANGELOG.md` and the docs site: PRs 1 and 2 change no behaviour a user sees
(PR 2 changes when a large transaction is written, documented with PR 3).

## Design decisions not fixed by the spec

1. **One `Parser`, over a `Lexer`.** `Lexer { input: &str, pos: usize, error: Option<Error> }`
   yields one `Token` at a time; after its first error it yields nothing more and keeps the
   error. `Parser` reads from a `Lexer` instead of a `Vec<Token>`, with one token of lookahead.
   So every command reads tokens on demand. `parse_edn` and the non-statement path of
   `parse_datalog_command` keep today's errors by finishing the lexer before they report: a
   lexer error anywhere in the input wins over a structure error, as `tokenize` before
   `Parser` gave. `tokenize` stays as `Lexer::new(input).collect()` for tests (`cfg(test)`).
   One non-generic `Parser` keeps the binary from carrying two copies.
2. **A `keep` flag on `Parser`.** With `keep` off, `parse_value` checks structure exactly as
   with it on but vectors, lists and maps hold no elements. The statement reader skips every
   element of a statement other than the facts and the transaction-level map this way, so an
   extra element (an error, `PRS-075`/`PRS-076`) is never held whole either.
3. **Where the reader lives.** `src/query/datalog/parser/statement.rs`, a child module of the
   parser like `verifier.rs`, so it reads the lexer and `Parser` without widening their
   visibility. It holds the per-fact phase 4 checks (`transact_pattern`, `retract_pattern`,
   moved out of `parse_transact`/`parse_retract`) and phase 5, `materialize_assertion` and
   `materialize_retraction_of` (moved from `db.rs`; `Minigraf::materialize_transaction` and
   `materialize_retraction` stay for `BrowserDb` and loop over them).
4. **Dispatch.** `parse_datalog_command` lexes the first two tokens. `(` then the symbol
   `transact` or `retract` goes to the statement reader; anything else, including a lexer
   error in those two tokens, takes today's path. The statement reader builds the same
   `DatalogCommand::Transact`/`Retract` for `parse_datalog_command`'s callers (`BrowserDb`,
   `Minigraf::query`, `prepare`), holding every `Pattern` as today. Writes use
   `read_statement` instead, which keeps no `Pattern`s.
5. **Phase 3 order.** The spec says "first error in text order" within a phase. Phase 3 is
   not in text order today: `PRS-075` (extra elements, after the facts) is checked before
   `PRS-042` (facts not a vector). The reader records what phase 3 needs (element count, the
   map's result, the facts element's kind) and runs today's checks in today's order at the
   end of the input.
6. **Phases 5 and 6 are reported when the facts are taken.** `read_statement` returns `Err`
   for a phase 1–4 error and a `Statement` otherwise; a phase 5 or 6 error is kept in the
   `Statement` and returned by `Statement::facts`/`Statement::chunks`. `Minigraf::execute`
   checks `reject_if_read_only` between the two, as it does between parsing and
   materialising today, so a statement on a read-only database still gets `API-014` before
   `INT-002`.
7. **Pass 1 keeps `TX_CHUNK_FACTS` facts, not `TX_CHUNK_FACTS + 1`.** It records the byte
   offset after the last kept fact and marks the statement large when one more fact follows.
   Pass 2 resumes there. Same result, and the acceptance bound "at most `TX_CHUNK_FACTS`
   materialised facts" holds exactly.
8. **`TX_CHUNK_FACTS`** is `16_384` until PR 3's measurement. `cfg(test)` (and, from PR 2,
   `test-oracle`) gets a thread-local override `set_tx_chunk_facts`.
9. **In PR 1 every statement takes the small path.** `Statement::facts` returns the kept facts
   for a small statement and, for a large one, runs pass 2 and appends every chunk. So pass 2
   runs on every statement over `TX_CHUNK_FACTS` facts, with today's memory, and the
   differential test drives it with the override.
10. **`cow_update(root, inserts, deletes, …)`.** Inserts and deletes are routed down the tree
    as two sorted runs per child. At a leaf the deletes are applied first (each must be
    present, else `INT-049`), then the inserts are merged, so a key deleted and inserted in
    one call is kept. A node whose result is empty returns no pieces and its parent drops
    the child slot with it: the next child's separator then starts the parent's slot list,
    where `write_internal_pieces` already ignores it. An internal node with no children left
    returns no pieces too; a root with none gives root 0. `cow_insert` stays as a thin call
    of `cow_update` with no deletes.
11. **`PageAllocator::release(id)`** pushes `id` onto the free ids the allocator hands out
    next. A released id that is not handed out again joins the new free list in
    `finish_free_list`, as every unused free id does today. It moves from PR 1 (spec §9) to
    PR 2, its first caller, so PR 1 adds no code only tests call.

## PR 1 steps

1. **Lexer.** Turn `tokenize` into `Lexer::next_token`, byte offset `pos`, `Iterator` impl,
   `error()`; `tokenize` under `cfg(test)`. Existing tokenizer tests pass unchanged.
2. **Parser over the lexer** (decision 1) with `keep` (decision 2) and a settable `depth`;
   `parse_edn` finishes the lexer before reporting. Grammar verifier passes unchanged.
3. **Old path kept for tests.** `parse_transact`/`parse_retract` and an
   `old_parse_datalog_command` move under `cfg(test)` (removed with the executor in #551).
4. **Statement reader** (`statement.rs`): the outer list and facts vector by hand, each fact
   by `Parser::parse_value`, every other element skipped with `keep` off. One `FirstError`
   (phase, error), replaced only by an earlier phase; phases run only while below the
   recorded one; a lexer error ends the pass; a structure error leaves only lexing.
5. **`parse_datalog_command`** dispatches statements to the reader (decision 4).
6. **Pass 1 and pass 2.** `read_statement(input, tx_id) -> Result<Statement>`: phases 5–6 per
   fact, kept facts, resume offset. `Statement::facts()` and `Statement::chunks()` (pass 2,
   `Iterator<Item = Result<Vec<Fact>>>`); `Statement::is_retract()`, `is_large()`.
7. **Callers.** `Minigraf::execute` and `WriteTransaction::execute` use `read_statement` for
   statements and `parse_datalog_command` for the rest; their phase 6 and 7 checks stay.
   `WriteTransaction` computes its staging `tx_id` before pass 1.
8. **Differential tests.** Old path against the reader, over every token sequence up to 3
   long (balanced or not) from the verifier's tokens plus statement tokens, in statement
   templates, and a corpus of multi-error statements: same `DatalogCommand` or same code;
   then phases 5–7 against `materialize_*` + `check_valid_windows` +
   `check_one_window_per_triple`, with the chunk size forced to 1–3. Unit tests for
   the deferred phase 5–6 error, the resume offset, the `keep` flag and the lexer's offset.
9. **`cow_update`** (decision 10): property tests against a `BTreeSet` model over random
   insert/delete batches on trees of 1–3 levels, a `verify`-style walk after every step,
   every page the old root reaches unchanged, `freed` exactly the dropped old nodes, deletes
   of missing, unsorted or repeated keys `INT-049`, delete-everything gives root 0.


## PR 2: large single statements

Checked against `v3` at `3ea155ba` (PR 1 merged). Line numbers are from that commit.

### Scope

A `transact` or `retract` run by `Minigraf::execute` on a writable file database
(`WriteContext::File`) with more than `TX_CHUNK_FACTS` facts goes through spec §4: it becomes
large, writes its facts in chunks into working trees, and commits as a checkpoint. Every other
statement takes today's path:

- **In-memory databases** (`WriteContext::Memory`) keep the small path. They have no pages, and
  the whole database is in memory anyway.
- **`BrowserDb`** is unchanged: it parses with `parse_datalog_command` and commits through its
  own path (`src/browser/mod.rs`), not `execute_statement`.
- **`WriteTransaction`** stays small in PR 2 (decision 12).

### Decisions not fixed by the spec (PR 2)

12. **`WriteTransaction` moves to PR 3** (user decision, 2026-10-22). Spec §9 puts a
    `WriteTransaction` whose first statement is large in PR 2. Its later statements would need
    §5.2's deletes of an earlier statement's keys (last statement wins), and its queries §7's
    reads of the working trees, both PR 3. So every `WriteTransaction` statement is staged as
    today until PR 3, which enables the large path for all of them at once.
13. **No statement base in PR 2.** The large statement is its transaction's only statement, so
    its base (§5.1) is the transaction's start. A statement error rolls the whole transaction
    back (§4.4), which is what §5.3 does for a first statement. With nothing written before
    the statement, §4.2 step 3 has `handed` = every id written and `held` empty: a replaced
    page this transaction wrote is released at once.
14. **`tx_count` is reserved, then allocated at the commit point.** Spec §4.1 step 3 calls
    `allocate_tx_count` when the transaction becomes large. Under the write lock nothing else
    allocates (the writable `Minigraf` paths that call `allocate_tx_count`, `db.rs:786` and `db.rs:1588`, hold it; `FactStorage::transact` is called only by tests), so the
    transaction uses `current_tx_count() + 1` and calls `allocate_tx_count` just after its meta
    write, which must return the same value (`INT-049` otherwise). A rolled-back large
    transaction then takes no `tx_count`, as a small one rejected before its WAL entry takes
    none today (`db.rs:781`); a small one rejected with `WAL-003` takes one, since the WAL
    entry checks sizes after the allocation.
15. **Where it lives.** `src/storage/persistent_facts/large_tx.rs`, a child module of
    `persistent_facts` (as `browser_access.rs` is), so it uses `PersistentFactStorage`'s
    private fields (`backend`, `page_cache`, `meta`, `failed`, `dirty`) and `activate`.
    `LargeTx` holds generation `g`'s meta, the working meta (five roots, `next_eid`,
    `next_iid`, `fact_count`), the allocator, `freed`, `tx_count` and `tx_id`;
    `PersistentFactStorage::begin_large`, `LargeTx::write_chunk`, `commit` and `rollback`.
    `execute_statement` drives it within one call under the write lock; PR 3 keeps the type
    in the `WriteTransaction`. The backend mutex is taken per chunk, not for the transaction
    (`save` holds it throughout, `persistent_facts.rs:431`), so reads go on between chunks.
16. **Written ids.** `PageAllocator::tracking(self)` turns on a set of every id `alloc` and
    `alloc_append` hand out; `handed(id)` asks it. `release(id)` (decision 11) pushes `id`
    onto `free`, the stack `alloc` pops next. A page `cow_update` replaces is in the set (this
    transaction wrote it: released) or not (generation `g` references it: `freed`); no third
    case without a base (decision 13). The set keeps released ids too: on rollback every id
    in it is evicted from the cache (decision 21).
17. **Phase 7 across chunks** (§5.2 fast path). `check_one_window_per_triple` runs on each
    chunk before stamping, as today on the whole statement. Across chunks, after the chunk is
    encoded, each EAVT entry's triple (`keys::triple_len`) is looked up in the working EAVT
    tree with one `LeafCursor` seeking forward through the sorted entries; keys of this
    `tx_count` come first (`tx↓`). A key equal to the entry is an earlier chunk's copy of the
    same fact: the entry is dropped from all four trees' inserts (its other keys come from
    `KeyFact::decode` + `KeyFact::key`). An assertion key with another window, when the entry
    is an assertion, is `API-011`. An assertion and a retraction of one triple both stay, as
    `check_one_window_per_triple` allows.
18. **Statement errors and failures** (§4.4). A chunk's checks run before it writes anything:
    `check_valid_windows` at the fixed `tx_id` (`API-019`), `check_one_window_per_triple`
    (`API-011`) and `wal::check_fact_size` per fact (`WAL-003`, the limit the WAL entry
    enforced and the `Encoder` enforces again). These, `API-011` from decision 17, and every
    pass 2 error are statement errors: the transaction rolls back, the handle is not failed.
    Any other error once `begin_large` returns (I/O, `INT-*`) also marks the handle failed
    (`STG-046`) before the rollback. `write_chunk` returns which kind it was.
19. **Error order across chunks.** Pass 1 reports every phase 1–6 error at read time, as PR 1
    does. Pass 2 reports a chunk's errors before a later chunk's. Today's whole-statement
    order would report a later chunk's `API-011` before an earlier chunk's `WAL-003` (the WAL
    serialises after the phase 7 check, `db.rs:779-806`), and a later chunk's `API-019` at the
    commit time before an earlier chunk's `API-011` (the millisecond window of spec §3.3).
    For a large statement both come in chunk order instead.
20. **Chunks from `Statement`.** `Statement::chunks(self) -> Result<Chunks<'a>>` returns the
    deferred phase 5–6 error, or an iterator of `Result<Vec<Fact>>`: the kept facts, then
    pass 2's chunks. `Statement::facts` collects it. `Statement::is_large()`.
21. **Cache on rollback** (§4.4). `PageCache::evict(&HashSet<u64>)` drops those entries in
    one pass over the LRU order (`invalidate`, `cache.rs:200`, scans the order per id). The
    spec evicts `written`; evicting released ids too costs nothing more and leaves no page of
    a generation that never existed. The bound goes back to `g` (`set_generation_bound`,
    `cache.rs:89`).
22. **The WAL guard** is the lazy open `wal_write_stamped_batch` does (`db.rs:1655`), moved
    to a helper both call. `WalWriter::open_or_create_at` writes and syncs the header
    (`wal.rs:55-56`) and syncs the directory (`wal.rs:238`); an existing file is reopened as
    is, so its base stays `≤ g`.
23. **Open-time dirty rule** (§4.5 rule 2). `open_with_options_inner` calls `pfs.force_dirty()`
    when the WAL file exists on a writable open (`db.rs:581`). `do_checkpoint`'s early
    return (`db.rs:1155`) then deletes a WAL without a meta only when no WAL existed at open
    and this handle wrote none, i.e. never with a WAL a large transaction could have guarded.
24. **Test hooks.** `set_tx_chunk_facts` is built under `cfg(any(test, feature =
    "test-oracle"))`; `minigraf::__tx_chunk_facts(Option<usize>) -> TxChunkGuard` exposes it
    under `test-oracle` (as `__reorder`, `src/lib.rs:134`). The acceptance count is a
    thread-local high-water mark of materialised `Fact`s held at once by pass 1, pass 2 and
    the chunk writer (`minigraf::__statement_facts_high_water`, `test-oracle`). `Token`s,
    `EdnValue`s and `Pattern`s are one fact's worth by PR 1's construction and are not
    counted.

### Deviations from the spec (PR 2)

1. `WriteTransaction` keeps the small path until PR 3 (decision 12; spec §9).
2. `tx_count` is allocated at the commit point, not when the transaction becomes large
   (decision 14; spec §4.1 step 3). `tx_id` is still fixed when it becomes large.
3. Rollback evicts every id handed out, not only those still written (decision 21; spec §4.4).
4. Errors of different chunks come in chunk order (decision 19; spec §3.3 for pass 2).
5. Two records of one triple in different chunks, one with a default `valid_from` and one with
   an explicit `valid_from` equal to the fixed `tx_id`, compare equal after stamping and are
   kept once; within one chunk, as today, they are `API-011`. Hitting it needs the exact
   millisecond the transaction became large.
6. The acceptance count is `Fact`s only (decision 24; spec §3.4 / §8.1 also name `Pattern`,
   `EdnValue`, `Token`).

### PR 2 steps

10. **Allocator and cache.** `PageAllocator::tracking`, `handed`, `release` (decisions 11,
    16) and `PageCache::evict` (decision 21). Unit tests: a released id is handed out next,
    an unused released id is on the new free list, `handed` covers `alloc` and
    `alloc_append` but not ids before `tracking`, `evict` leaves other entries.
11. **Statement chunks and hooks.** `Statement::chunks`, `is_large`, `facts` over `chunks`
    (decision 20); the high-water mark and the `test-oracle` hooks (decision 24). PR 1's
    differential tests run over `chunks` too.
12. **`LargeTx`** (decision 15). `begin_large(tx_id)`: bound `g + 1`, allocator
    `from_chain(.., g + 1).with_quarantine(pages_in_force()).tracking()`, working meta =
    `g`'s. `write_chunk(facts)`: the checks (decision 18), stamp (`tx_id`, `tx_count`,
    default `valid_from`), a new `Encoder` from the working counters staging against
    `DictReader::new(working.dict_root, ..)` with no `SharedDictCache`, `finish`, the
    cross-chunk check (decision 17), `cow_update` on the five working roots with no deletes,
    replaced pages released or `freed` (decision 16), working counters and `fact_count`
    updated. `commit(pending)`: the pending store's facts (`get_pending_facts`) as one more
    chunk (own `tx_count`s, own `Encoder`, no checks), `finish_free_list(freed)`, `sync`,
    meta `g + 1` with `last_checkpointed_tx_count = tx_count`, `check_written`, meta write,
    `sync` (commit point), `allocate_tx_count` (decision 14), `activate`, `publish(reader,
    g + 1, pending_len, freed)`, `dirty = false`. `rollback`: bound `g`, evict, drop the rest.
    Unit tests on `MemoryBackend` with chunk sizes 1–4: same records as the small path for
    the same statements (fact log), `verify` clean, every page accounted for
    (`assert_space_accounted`), pages `g` references never written, a rolled-back
    transaction leaves the backend's `g` pages and the active meta unchanged, a generation
    pinned before `begin_large` keeps its pages, repeated facts across chunks stored once,
    `API-011` across chunks.
13. **`execute_statement`.** After the lock and `refuse_if_failed`: `is_large()` on a `File`
    context takes the large path: WAL guard (decision 22), `begin_large(tx_id_now())`, each
    chunk through `write_chunk`, `commit`, then `delete_wal`. A statement error rolls back
    and `force_dirty`s; a failure marks the handle failed, rolls back and `force_dirty`s. The
    open-time dirty rule (decision 23). Tests: `execute` at chunk sizes 1–4 against chunk
    size `None` on the same statements (records, `current_tx_count`, `:as-of`), a large
    statement with a WAL of earlier entries commits them too and deletes the WAL, a failed
    large statement takes no `tx_count` and the next checkpoint writes a meta, reopen after
    a WAL with every entry checkpointed writes one meta on the first checkpoint (existing
    #457 tests updated for it).
14. **Crash, faults, isolation.** Fault matrix (`src/fault_matrix.rs`): `Op::LargeTransact`
    on a file with one valid meta slot (new, never checkpointed) and with both, each with and
    without a WAL beforehand, chunk size 4, a large `X` with long values, a repeat and a
    retraction; `is_whole_x` for it. `tests/crash_kill_test.rs`, under `test-oracle`: a
    variant whose child sets the chunk size to 2, so most transactions are large, and runs a
    statement that fails `API-011` in pass 2 every few transactions (a rollback with pages
    written, not logged); the kill lands during a large transaction, after its commit point,
    or after a rollback before the next checkpoint. A unit test reproduces spec §4.5: a large
    transaction killed after writing pages on a new file reopens (WAL base `g`) and, without
    the guard WAL, is `STG-033`. Cursor and fact log opened before, during (between chunks,
    from a test hook in `write_chunk`) and after a large transaction see their snapshot
    only. `tests/model_based_test.rs` under `test-oracle` runs with the chunk size at 2 (spec §8.1: 3), so its retractions, of 1–3 facts, are large too.
15. **Acceptance and docs.** `tests/large_transaction_acceptance_test.rs` (`test-oracle`,
    ignored, `.github/workflows/smoke.yml` nightly as `memory_limit_acceptance_test` is): a
    1M-fact `transact` on a file database; the high-water mark is at most `TX_CHUNK_FACTS`;
    peak RSS (`VmHWM`) printed. Test counts in `CLAUDE.md` and `docs/TEST_COVERAGE.md`;
    `CLAUDE.md`'s module notes for `large_tx.rs`. No `CHANGELOG.md` or docs-site change
    (PR 3).

## PR 3: statement bases, large `WriteTransaction`s, reads

Checked against `v3` at `75a3faca` (PR 2 merged as `9ecc02f5`). Line numbers are from that
commit.

### Scope

- A `WriteTransaction` on a writable file database becomes large (spec §2, §4.1) and keeps its
  `LargeTx` across `execute` calls; `commit`, `rollback` and drop go through `commit_large` and
  `rollback_large`. In-memory databases stay small, as for `execute` (PR 2 scope).
- Statement bases (spec §5) for every statement of a large transaction after its first write.
- Every `WriteTransaction` query reads a `Snapshot` (spec §7); `merged_query_facts`
  (`db.rs:1770`) and the executor's `facts_override` (`executor.rs:150`) go.
- The measurement (spec §8.2), the final `TX_CHUNK_FACTS`, and the docs of spec §9.

Not in PR 3: #686 (WAL entries over 1M facts, its own v3 PR and v2.x backport), #687
(`crash-kill.yml` on `main`), #551.

### Decisions not fixed by the spec (PR 3)

25. **When a `WriteTransaction` becomes large.** `WriteTransaction` gains `large:
    Option<LargeTx>` (not wasm32). A statement read by pass 1 makes a small transaction on a
    `WriteContext::File` large when it is large itself (`Statement::is_large`) or when its
    facts plus the staged ones number more than `tx_chunk_facts()` (spec §2). A small
    statement's phase 5–7 errors (`Statement::facts`, `check_one_window_per_triple`) are
    reported before the switch, so a statement rejected as today leaves the transaction small,
    as today. A large statement's errors come from its chunks, after the switch.
26. **Order of the switch** (spec §4.1). `tx_id = tx_id_now()`; then step 5's checks, before
    steps 1–4: the staged facts reduced by `keep_last_statement` are checked by
    `check_valid_windows` at `tx_id` and `wal::check_fact_size` (the checks `check_chunk`,
    `large_tx.rs:290`, runs; each statement passed `API-011` and the reduction leaves one
    statement per triple). A failure rejects the current statement with nothing to undo: no WAL
    guard, no bound, the staged facts as they were. Then `refuse_if_failed` (`STG-046` /
    `WAL-007`: a large transaction writes pages, so a failed handle is refused here, not at
    commit), the WAL guard, `begin_large(tx_id)`, and the staged facts as one chunk, taken from
    the transaction only now. An error writing them is a failure: they passed every statement
    check, and the fast path (§5.2) finds no key of this `tx_count`.
27. **The pending store is written when the transaction becomes large, not at commit** (spec
    §4.3 step 1). `begin_large` writes `get_pending_facts()` (own `tx_count`s, no checks) as
    its first chunk and records their number in `LargeTx::pending_len`; `commit_large` publishes
    with it. A query inside a large transaction (§7) then reads the working trees alone: the
    engine's scan treats the committed reader as older than the pending part (a committed
    triple with a pending record is shadowed, `scan.rs:448-456`) and so does the executor's
    net-assert over committed survivors plus pending records (`storage.rs:232`); with pending
    facts older than the working trees' newest, both would let a pending assertion hide this
    transaction's retraction. No fact joins the pending store while the write lock is held
    (`execute_statement`, `commit_inner` and WAL replay at open append under it), so the length
    at `begin_large` is the publish count. One path for `Minigraf::execute` and
    `WriteTransaction`; an error writing the pending chunk fails the handle and rolls back.
28. **The base** (spec §5.1). `Base { roots: [u64; 5], next_eid, next_iid, fact_count,
    freed_len, held: Vec<u64>, claims: u64 }` in `LargeTx`, recorded when a statement of a
    large `WriteTransaction` starts (decision 33). `PageAllocator::begin_base` starts a second
    id set beside `handed`, `since`: `alloc` and `alloc_append` add to it, `release` removes
    from it; `handed_since(id)`; `end_base()` takes it.
29. **Replaced pages with a base** (spec §4.2 step 3): `handed_since` → released (and out of
    `since`); not `handed` (generation `g` references it) → `freed`; otherwise → `held`.
30. **Which statement wrote a key: claims** (deviation from spec §5.2). Spec §5.2 step 2 says a
    key of this `tx_count` that is in the base's EAVT tree came from an earlier statement. Not
    when this statement wrote the same key in an earlier chunk: statement 1 asserts `[:a :n 1]`,
    statement 2 asserts `[:a :n 1]` in chunk 1 (§5.2 deletes statement 1's key and inserts the
    same bytes) and `[:a :n 1 {other window}]` in chunk 2. The key is in the base, so chunk 2
    would delete it and accept the statement; a small transaction rejects it with `API-011`.
    The base holds the keys, not who wrote them last. So a statement also keeps **claims**: a
    copy-on-write B+tree (root 0 at the base, pages from the allocator, so in `since`) of the
    triples (EAVT key prefix up to `keys::triple_len`) whose base keys it deleted. For each
    triple `T` of a chunk, with `R` its working EAVT keys of this `tx_count` (`repeats`' run):
    - `R` empty, the fast path, or `T` in claims: `R` is this statement's (repeat or `API-011`,
      as in PR 2);
    - otherwise, if `R`'s first key is in the base's EAVT tree (`btree::get`): `R` is earlier
      statements' (this statement never touched `T`, or it would be in claims; and before it
      touches `T` every key of `T` is the base's) — every key of `R` and its AEVT, AVET and
      VAET keys go into the chunk's deletes, the records are inserted, and `T` joins claims;
    - otherwise `R` is this statement's: the base has no key of `T`, so it never will.
    Claims hold only triples two statements of the transaction share, on pages, not in memory.
    Their replaced pages are released at once. At the end of the statement every claims page
    is released (`btree::tree_pages`, new: the ids of a tree's nodes).
31. **A failed statement restores `freed`** (spec §5.3 gap). Pages of `g` that the statement
    replaced went to `freed`; the restored roots reference them again, so `freed` is truncated
    to its length at the base. Success releases `held` and the claims pages; failure restores
    the roots, counters and `fact_count`, truncates `freed`, releases `since` (claims pages
    included), keeps `held`.
32. **Statement errors and failures in a large `WriteTransaction`.** Statement errors
    (`API-011`, `API-019` at the fixed `tx_id`, `WAL-003`, pass 2 errors) restore the base; the
    transaction stays large and usable. Failures (decision 18) fail the handle; the transaction
    stays open, `commit` returns `STG-046` and rolls back, as `rollback` and drop do.
33. **Every statement of a large `WriteTransaction` gets a base**, the first written one
    included. A statement error in a first large statement must leave the transaction usable,
    which PR 2's rollback does not (`execute_large` ends the transaction). In the fast path
    (§5.2: no earlier statement in the working trees) the claims stay empty, since it never
    deletes. `Minigraf::execute`'s one statement keeps PR 2's rollback and no base (decision
    13). The staged chunk of decision 26 is written without a base: it cannot fail but as a
    failure.
34. **Phase 6 of a large transaction's later statements** runs in pass 1 at the fixed `tx_id`,
    which every fact is stamped with; a small transaction keeps the staging time.
35. **Reads** (spec §7). `Snapshot` gains `staged: Option<Arc<PendingStore>>`, built from the
    staged facts with each default `valid_from` resolved to its staged `tx_id`
    (`merged_query_facts`' rule), and `now` raised to the newest staged `tx_id`
    (`pending_read_now_floor`'s rule). Every pending read (`all_facts`, `live_facts_of_scans`,
    `pending_records`, `pending_live`, `pending_range`, `pending_len`) reads the pending store's
    first `pending_len` facts and then `staged` as one store: positions `pending_len..` are
    staged, and index walks merge the two `BTreeMap`s in key order, so `pending_live` decides a
    triple over both. Keys never collide: staged `tx_count`s are above every pending one. A
    large transaction's snapshot has `committed` = an `OnDiskReader` over the working meta
    (`last_checkpointed_tx_count` = its `tx_count`), unpinned working pages (the pin `snapshot()`
    takes on `g` stays; it costs nothing under the write lock), `pending_len` 0 (decision 27)
    and no `staged`. `DatalogExecutor::new_from_facts_with_rules_and_functions`,
    `facts_override` and `read_now_floor` are removed; `WriteTransaction` queries run on the
    engine where it plans them, with the oracle under `test`.
36. **Measurement** is a program outside the repository (public API only, so it builds on `v3`
    before #550, `d24fb585`, and on this branch), one process per run for `VmHWM`; pages
    written from `/proc/self/io` `write_bytes` / 4096 (WAL included). The chunk size is set by
    building with each value of the constant. Results (no machine details) go in the PR and in
    this plan.

### Measurement results (step 22)

`v3` before #550 (`d24fb585`) against this branch at each chunk size, 1M facts, an idle 88-vCPU
machine, one process per run, `opt-level = 3`. Facts are `[:e<k> :a<i % 10> v]`: sequential
entities `k = i / 10`, random entities `k` uniform over 100K; short values are integers, long
ones unique 100-byte strings. RSS is `VmHWM` above the peak while the input text was built
(the input itself is 31–475 MiB); 0 means no rise above it. Pages written are
`write_bytes / 4096`, WAL included; file pages after the commit and a checkpoint.

| Shape | v3 before | 4,096 | 16,384 | 65,536 |
|---|---|---|---|---|
| `execute`, sequential, short | 9.1 s, 1,312 MiB, 21,075 | 4.9 s, 0, 23,423 | 4.2 s, 0, 22,006 | 3.9 s, 0, 21,933 |
| `execute`, sequential, long | 15.2 s, 2,102 MiB, 90,715 | 21.8 s, 0, 95,855 | 20.2 s, 0, 95,284 | 21.0 s, 0, 98,430 |
| `execute`, random, short | 11.4 s, 1,293 MiB, 20,394 | 36.4 s, 0, 24,751 | 18.5 s, 0, 26,605 | 9.5 s, 0, 28,110 |
| `execute`, random, long | 17.6 s, 2,087 MiB, 82,678 | 51.0 s, 0, 90,095 | 39.7 s, 0, 97,778 | 28.1 s, 0, 104,613 |
| 10 statements, sequential, short | 10.1 s, 1,391 MiB, 21,075 | 5.0 s, 0, 24,195 | 4.5 s, 4, 22,920 | 3.8 s, 41, 22,832 |
| 10 statements, sequential, long | 17.5 s, 2,419 MiB, 90,715 | 22.1 s, 0, 109,067 | 20.6 s, 0, 107,432 | 18.9 s, 47, 107,477 |
| 10 statements, random, short | 12.0 s, 1,372 MiB, 20,394 | 38.2 s, 0, 36,912 | 22.5 s, 7, 38,596 | 10.1 s, 41, 41,418 |
| 10 statements, random, long | 22.3 s, 2,402 MiB, 82,678 | 51.3 s, 0, 139,352 | 41.3 s, 0, 146,629 | 34.0 s, 54, 150,977 |

At 100K facts every chunk size is within 2.3× of `v3` before in time and under 110 MiB of RSS
(before: 142–260 MiB). **The chunk size becomes `OpenOptions::transaction_chunk_facts`,
default 65,536** (user decision, 2026-10-22: configurable, default optimised for speed; this
deviates from spec decision 2's memory first): 65,536 takes about half the time of 16,384 with
random entities for 40–55 MiB more, and 4,096 is up to 3× slower than 16,384. Values are kept
between 1 and 1,000,000, the most facts WAL replay reads from one entry (`wal.rs`
`MAX_FACTS_PER_ENTRY`), since a small transaction is one entry (#686). Two costs the measurement shows: random entities are slower
than before (each chunk rewrites leaves all over each tree, spec §8.2), and a transaction of
several statements over random entities leaves a file up to 1.8× larger, since every page its
base references and a statement replaces stays held until the statement ends (§4.2 step 3);
the space is free in the file afterwards. #551's envelope records the result.

### Deviations from the spec (PR 3)

1. Step 5's checks run before steps 1–4, so a rejected statement has nothing to undo (decision
   26; spec §4.1 step 5).
2. The pending store is written when the transaction becomes large (decision 27; spec §4.3
   step 1).
3. Claims decide which statement wrote a key, not membership in the base alone (decision 30;
   spec §5.2 step 2).
4. A failed statement truncates `freed` (decision 31; spec §5.3 does not cover it).
5. A small statement's `API-011` is checked before the switch (decision 25).
6. Every statement of a large `WriteTransaction` has a base, the fast path included (decision 33).
7. A large transaction in which no statement wrote a fact commits nothing and takes no
   `tx_count`, as a small one with nothing staged: `commit` rolls it back (found by the
   model-based test).
8. Inside a large transaction, `:as-of` its own `tx_count` sees every finished statement; in a
   small one each staged statement has its own synthetic count until `commit`.
9. The chunk size is an open option, `OpenOptions::transaction_chunk_facts`, default 65,536,
   not a constant (spec §8.1, decision 2; user decision after the measurement).

### PR 3 steps

16. **Allocator and B+tree.** `PageAllocator::begin_base`, `handed_since`, `end_base`
    (decision 28); `btree::tree_pages`. Unit tests: `since` covers ids handed after
    `begin_base` and not before, `release` removes from it, `tree_pages` lists exactly the
    pages a `cow_update`d tree reaches.
17. **Pending chunk at the switch** (decision 27): `begin_large` writes the pending store;
    `commit_large` publishes `tx.pending_len`. PR 2's tests pass unchanged.
18. **Bases and claims** (decisions 28–31, 33) in `large_tx.rs`: `begin_statement`,
    `end_statement(ok)`, `repeats` generalised to `resolve` (drops, deletes for the four
    index trees, claims), `apply_chunk` with deletes and `fact_count` decremented. Unit tests on
    the file backend with chunk sizes 1–4: statement sequences in one large transaction
    against the same statements in a small one (records equal, `API-011` in the same
    statements), a failed statement leaves the trees, counters and `freed` at its base and the
    transaction usable, a rolled-back transaction leaves `verify` clean and every page
    accounted for after the next checkpoint.
19. **`WriteTransaction` large path** (decisions 25, 26, 32, 34): `execute_inner`,
    `commit_inner`, `rollback`, `Drop`. Tests: a `WriteTransaction` differential at chunk sizes
    1–4 (`large_statements_store_what_small_ones_do` with the workload's statements in one
    transaction, and split over several), the step 5 `API-019` case (a staged window that ends
    between staging and the switch), a failed handle refused at the switch, commit after a
    failure is `STG-046`.
20. **Reads** (decision 35): `Snapshot::staged`, the merged pending walk, the large
    transaction's working-tree snapshot, the executor's override removed. Tests:
    read-your-own-writes, `:as-of` of staged counts, retract-then-assert (#477) and
    assert-then-retract across statements, small and large, each against the committed result;
    a staged retraction hides a pending assertion of the same triple and vice versa; the
    existing `db.rs` transaction read tests pass on the engine.
21. **Crash and faults.** Fault matrix: `Op::Large { tx: true, .. }` (a `WriteTransaction` of
    a staged statement, the large `X`, a statement that fails `API-011` after writing a chunk,
    and one that repeats and changes earlier statements' triples; chunk size 3), on one and two
    meta slots, with and without a WAL. Crash-kill `Large` variant: every third transaction is a
    `WriteTransaction` of the opposite statement, the failing large statement and the
    transaction itself. The model-based test covers bases through its `WriteTransaction` steps
    at chunk size 2.
22. **Measurement and `TX_CHUNK_FACTS`** (decision 36): set the constant here and in
    `tests/large_transaction_acceptance_test.rs`.
23. **Docs.** `CHANGELOG.md`, `docs/TEST_COVERAGE.md`, `CLAUDE.md` (module notes, counts), and a
    docs-site PR (`content/architecture.md`, `<!-- @since v3.0.0 -->`).
