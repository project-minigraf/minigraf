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

## PR 3

Planned in detail when PR 2 is merged, against the code it leaves: spec §5 and §7, the large
path for every `WriteTransaction` statement (decision 12), the measurement on the bench
machine, the final `TX_CHUNK_FACTS`, and the docs listed in spec §9.
