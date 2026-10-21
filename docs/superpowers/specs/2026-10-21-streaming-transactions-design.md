# Streaming transactions — Design

**Issue:** #550 (streaming engine 8/9, part of #473)
**Parent design:** `docs/superpowers/specs/2026-10-11-streaming-engine-design.md` (§8), called "the #473 spec" below
**Milestone:** v3.0.0, branch `v3`. The `.graph` file format stays v8 and the WAL format stays v2.
**Status:** brainstormed 2026-10-21; not implemented

This spec refines §8 of the #473 spec against the code on `v3` at `df7282f1`. Where the two differ, this one applies. §10 lists every difference.

## 1. Decisions

Settled in the brainstorm on 2026-10-21:

1. **Errors are reported as today.** A statement with several errors reports the same code it reports today, whatever its size (§3.3).
2. **Memory first, then measurement.** No hard bar on wall time. `TX_CHUNK_FACTS` is set by measurement (§8.2), and the #394 envelope records wall time and peak RSS of large transactions.
3. **Three PRs** (§9).

## 2. Terms

The #473 spec's terms apply (§1 there). In addition:

- **Staged facts**: the facts a `WriteTransaction` holds that are not yet in the pending store or in any tree. `Minigraf::execute` of one `transact` or `retract` is a transaction of one statement.
- **Small / large**: a transaction is *small* while its staged facts number at most `TX_CHUNK_FACTS`, and *large* from the moment staging one more fact would exceed that, for the rest of its life (#473 spec §8.2).
- **Working trees**: a large transaction's five tree roots (EAVT, AEVT, AVET, VAET, DICT), which start as the active generation's and grow copy-on-write. They become the next generation's trees at commit.
- **`g`**: the active generation when the transaction starts. A large transaction's pages are sealed with generation `g + 1`.

## 3. Parsing

### 3.1 The problem

`parse_datalog_command` (`parser.rs:497`) holds the whole command in memory three times over before any fact exists:

1. `tokenize` builds a `Vec<Token>` of the whole input (`parser.rs:34`);
2. `Parser::parse_value` builds an `EdnValue` tree of the whole input;
3. `parse_transact` / `parse_retract` clone each fact's elements into a `Vec<Pattern>` (`parser.rs:1019`, `1148`).

Then `materialize_transaction` builds the `Vec<Fact>` (`db.rs:1273`). Each of the three is larger than the facts, so bounding only the facts would not bound the memory.

### 3.2 A lexer that reads on demand

`tokenize` becomes a `Lexer` that yields one `Token` at a time from the input `&str`. `tokenize(input)` stays as `Lexer::new(input).collect()`, so `query` and `rule` commands, `parse_edn`, `BrowserDb` and the grammar verifier see the same tokens from the same code.

`transact` and `retract` get a statement reader, `StatementReader`, over a `Lexer`. It reads the command head and the optional transaction-level map, then yields one fact (`[e a v]` or `[e a v {opts}]`) at a time as a `Pattern`, then checks the end of the command. It holds one fact's tokens at a time. Nesting depth is checked as `Parser` checks it today.

### 3.3 Error order

Today a statement's errors are found in phases, each over the whole statement before the next starts:

| Phase | Codes (examples) | Where today |
|---|---|---|
| 1. Tokens | `INT-009`, bad characters, unterminated strings | `tokenize` |
| 2. EDN structure | unbalanced brackets, `PRS-005`, `PRS-077` | `Parser` |
| 3. Command | `PRS-010`, `PRS-011`, `PRS-041`–`PRS-044`, `PRS-048`, `PRS-075`, `PRS-076`, the transaction-level map | `parse_datalog_command`, `parse_transact`, `parse_retract` |
| 4. Fact structure | `PRS-045`–`PRS-047`, `PRS-049`, `PRS-089`, `PRS-090`, `PRS-108`, per-fact maps | the per-fact loop of `parse_transact` / `parse_retract` |
| 5. Materialisation | `INT-002`, `INT-003`, `API-003`, `API-004` | `materialize_transaction` / `materialize_retraction` |
| 6. Valid windows | `API-019` | `check_valid_windows` |
| 7. One window per triple | `API-011` | `check_one_window_per_triple` |

Within a phase, the first error in text order is reported.

`StatementReader` keeps this order with one pass that keeps no values: it runs phases 1–6 on each fact as it reads it and records **the first error of each phase**. At the end of the statement it reports the error of the earliest phase that has one. That is the error today's code reports, because today each phase completes over the whole statement before the next begins.

The pass that checks (**pass 1**) runs for every `transact` and `retract`:

- It keeps the first `TX_CHUNK_FACTS + 1` materialised facts.
- If the statement has at most `TX_CHUNK_FACTS` facts, those facts are the statement. There is no second pass, and the statement goes on as today: `check_one_window_per_triple` (phase 7), then staging or commit.
- If it has more, the statement is large. Pass 1 still reads to the end of the statement and reports any phase 1–6 error before anything is written. **Pass 2** then yields the facts in chunks of `TX_CHUNK_FACTS`, starting with the kept facts and resuming the lexer from the byte offset after the last kept fact. Phase 7 runs per chunk against the working trees (§5).

So a large statement is lexed twice and a small one once.

Phase 6 needs `tx_id`, which a large transaction fixes when it becomes large (§4.1), after pass 1. Pass 1 checks with the current time, as staging does today (`db.rs:1522`). Pass 2 checks each chunk again with the fixed `tx_id`. The check is time-dependent today too. Only when a window ends in the milliseconds between the two checks can the second check reject a fact the first accepted.

For `Minigraf::execute`, pass 1 runs before the write lock is taken, as parsing does today (`db.rs:729`). A statement rejected in pass 1 never waits for the lock.

### 3.4 Memory

At any time a `transact` holds the input `&str` (the caller's, not counted) plus at most one chunk of materialised facts (§4.2) plus one fact's tokens. The #473 spec's acceptance test, "a 1M-fact `transact` holds at most `TX_CHUNK_FACTS` parsed facts", counts `Fact`s, `Pattern`s, `EdnValue`s and `Token`s.

## 4. Large transactions

### 4.1 Becoming large

A transaction becomes large under the write lock (always held by a `WriteTransaction`, and taken by `execute` after pass 1). In this order:

1. **The WAL guard.** If no WAL file exists, the transaction creates one with its header only, base generation `g` (`WalWriter::open_or_create_at`, which syncs the file and its directory). No entry is ever written to it by this transaction. See §4.5 for why.
2. **The cache bound.** The shared `PageCache`'s generation bound (`set_generation_bound`, `cache.rs:90`) is raised from `g` to `g + 1`, so pages the transaction writes, and evicts, can be read back. The allocator hands the transaction only ids that generation `g` does not reference, so no reader of `g` or earlier reaches one of them through its own trees. On rollback the bound is set back to `g`.
3. **`tx_count` and `tx_id`.** The transaction takes its real `tx_count` (`allocate_tx_count`) and `tx_id` (`tx_id_now`). Every fact it commits is stamped with both. A default `valid_from` is that `tx_id`.
4. **The allocator.** `PageAllocator::from_chain(..., g + 1).with_quarantine(pages_in_force())`, as `save` builds it (`persistent_facts.rs:432`). Pins taken during the transaction are on `g` and only lower the set of pages in force, so the set taken here is safe for the whole transaction.
5. **Earlier statements.** The staged facts of the statements before the current one are reduced by `keep_last_statement`, checked with `check_valid_windows` at the fixed `tx_id`, stamped, and written into the working trees as one chunk (§4.2). If the check fails, steps 1–4 are undone (the WAL guard stays), the current statement is rejected with that `API-019`, and the transaction stays small. Its `commit` would fail with the same error today, since the commit time is later still.
6. **The current statement.** Its statement base is recorded (§5.1), and its facts follow in chunks (§4.2, §5.2).

### 4.2 Writing a chunk

A chunk is at most `TX_CHUNK_FACTS` facts of one statement, sorted by EAVT key. Writing it:

1. **Encode.** A new `dict::Encoder` per chunk, created from the working counters (`next_eid`, `next_iid`). It stages every fact against a `DictReader` over the **working** DICT root, so an entity or long value that an earlier chunk added is found there and not given a second id. `Encoder::finish` writes the chunk's new long values to fresh value pages and returns each tree's entries. Afterwards the working counters take the `Encoder`'s.
2. **Apply** the entries to the five working roots with `btree::cow_update` (§6): the chunk's inserts, and the deletes §5.2 selects.
3. **Replaced pages** (`freed` from `cow_update`):
   - an id that generation `g` references goes to the transaction's `freed` list, which joins the new meta's free list and the quarantine at commit, as in `save`;
   - an id this transaction wrote (it is in the transaction's `written` set) is released to the transaction's allocator (`PageAllocator::release`, new) unless a statement base references it (§5.1), in which case it waits in the base's `held` list.

A transaction-wide `Encoder` would hold an entry for every new entity and ident, so 1M new entities would mean 1M entries. One `Encoder` per chunk holds at most the chunk's.

`written` holds one `u64` per page written, not per fact. A 1M-fact transaction writes on the order of 10⁵ pages (§8.2).

### 4.3 Commit

Commit is a checkpoint, in this order (#473 spec §8.2 step 5):

1. The pending store's facts are written into the working trees as one more chunk. They carry their own `tx_count`s, are encoded by their own `Encoder`, and need no statement rules.
2. `finish_free_list(freed ∪ released-but-unused)`, then `sync`.
3. The new meta page (generation `g + 1`) is written and synced. Its `last_checkpointed_tx_count` is this transaction's `tx_count`. **This write is the commit point.**
4. `FactStorage::publish(reader, g + 1, pending_len, freed)`: the committed reader and an empty pending store together (#473 spec §4.1). The quarantine records `freed` only. Pages the transaction wrote and replaced were never referenced by any generation.
5. The WAL is deleted (`delete_wal`), and `wal_entry_count` becomes 0.

### 4.4 Rollback and failures

**Rollback** (explicit, by drop, or by a failure before any page is written) discards the working roots, the allocator, the `written` set and the statement base, and sets the cache bound back to `g`. Nothing that generation `g` references changed. The pages the transaction wrote are free in `g` or lie past its `page_count`. The handle is marked dirty (`force_dirty`), so its next checkpoint commits a new meta even with nothing pending (§4.5). The WAL guard stays until then.

`PageAllocator::write` puts every page it writes into the cache (`page.rs:350`), so a reused id never serves stale bytes. The pages of a rolled-back transaction need no cache eviction.

**Failures** after a page may have been written, in steps 1–3 of §4.3 or while writing a chunk, mark the handle failed (`STG-046`), as a failed checkpoint does. A failure in step 5 comes after the commit point and is handled as a checkpoint's failed WAL deletion is today. A statement error (`API-011`, or a phase 1–6 error in a chunk of pass 2) is not a failure: the statement returns to its base (§5.3).

### 4.5 Crash recovery and the WAL guard

The #473 spec says that a crash before the commit point "leaves the previous meta page active". The code does not do that when only one meta slot is valid.

`select_meta` calls `check_single_valid` when one slot is valid (`persistent_facts.rs:843`). That case covers a new file, whose slot B stays empty until its second checkpoint, and a file whose other slot is damaged. With no WAL, `check_single_valid` looks for pages of generation `g + 1` on `g`'s free list and past its `page_count`. If it finds one, it refuses the file with `STG-033`, taking it as proof that a commit of `g + 1` happened and its meta page was lost. Today that inference holds, because every checkpoint has a WAL until its meta is durable. A large transaction writes `g + 1` pages without a WAL, so after a crash mid-transaction a new database would fail to open.

Two rules keep the inference sound:

1. **A large transaction creates the WAL before its first page write** (§4.1 step 1). On open, `check_single_valid` then sees a WAL base `≤ g` and opens at `g`. The WAL has no entries this transaction wrote, so replay applies only the entries of earlier transactions, if any.
2. **A WAL is deleted only after a meta of a later generation than its base is durable.** Two code paths change:
   - after a rollback, the handle is marked dirty (§4.4);
   - on open, when a WAL file exists, the handle is marked dirty even if replay applied no entry (`db.rs:569`), so the first checkpoint writes a meta before `delete_wal`. Today `do_checkpoint` deletes such a WAL without a new meta (`db.rs:1152`, #457). That is still safe today, but would not be after a crash in a large transaction. The cost is one meta write on the first checkpoint after an unclean shutdown.

With both rules:

| Crash point | On reopen |
|---|---|
| Before the WAL guard is durable | No `g + 1` page exists yet. Opens at `g`. |
| After it, before the meta write (§4.3 step 3) completes | WAL base `g`. Opens at `g`, replays earlier entries. The transaction is absent. The first checkpoint commits `g + 1`; the transaction's pages are free in it. |
| After the meta write, before the WAL is deleted | Opens at `g + 1`. Every WAL entry has `tx_count` ≤ this transaction's and is skipped (the existing rule). |
| After the WAL is deleted | Opens at `g + 1`. |
| Meta write lost (a lying `fsync`), WAL deleted | `g + 1` pages without a WAL: `STG-033`, as today for a lost checkpoint. |

The fault-matrix (`src/fault_matrix.rs`) and `tests/crash_kill_test.rs` cover each row (§7).

## 5. Statement bases

### 5.1 The base

When a statement of a large transaction starts, it records its base:

- the five working roots;
- the working counters `next_eid`, `next_iid`;
- the allocator's position: the ids handed out since the base are logged in the base's `handed` list;
- a `held` list (§4.2 step 3).

The value-page position needs no entry. `ValueWriter` appends a new page for each chunk (`value_pages.rs:44`), and its pages are covered by `handed`.

### 5.2 A chunk's records

The #473 spec §8.3 rules, made concrete. For each record (fact) of triple `T = (e, a, v)` in the chunk, in key order:

1. Read the working EAVT keys with prefix `(e, a, v)`. Keys are ordered `tx↓`, so this transaction's keys (its `tx_count` is the highest) come first, and the read stops at the first older `tx`.
2. For each such key `k`:
   - if `k` is in the base's EAVT tree, it came from an earlier statement: `k` and its AEVT, AVET and VAET keys (EAVT keys are covering) go into the chunk's deletes;
   - otherwise it came from this statement: if the record is an assertion and `k` is an assertion of `T` with a different window, the statement fails with `API-011`; if `k` equals the record's key, the record is not inserted again.
3. Otherwise the record's keys go into the chunk's inserts.

Records of one `T` within the chunk are checked against each other the same way before the tree is read, with `check_one_window_per_triple` over the chunk.

**Fast path.** While no earlier statement of the transaction has written to the working trees (always for `Minigraf::execute`, and for a `WriteTransaction` whose first statement is the large one), every key of this `tx_count` came from this statement, and step 2 does not look in the base.

The lookups in step 1 follow the chunk's key order, so they are one `LeafCursor::seek` each, mostly in leaves already cached.

### 5.3 End of a statement

- **Success.** The base is dropped. Its `held` ids are released to the allocator.
- **Failure** (`API-011`, a phase 1–6 error found in pass 2, or a value limit). The working roots and counters are set back to the base's. The ids in `handed` are released to the allocator, and the ids in `held` stay where they are, since the base still references them. The transaction stays usable, as for a rejected statement today.

A statement rejected in pass 1 never reaches the working trees and needs no base.

## 6. Copy-on-write delete

`btree::cow_update(root, inserts, deletes, …)` applies sorted, distinct inserts and sorted deletes in one descent. `cow_insert` becomes `cow_update` with no deletes, so a checkpoint's behaviour is unchanged.

- A delete of a key not in the tree is an error (`INT-049`): §5.2 only deletes keys it read.
- A leaf left empty is removed from its parent. An internal node left with no children is removed from its parent in turn. A root left with no keys makes the tree empty (root 0).
- A leaf left with fewer keys stays as it is. `verify()` checks key order and separator bounds, not fill (#473 spec §8.3).
- Separators are not rewritten for deletes. A separator above a deleted first key still bounds its subtree.

Property tests compare `cow_update` against a `BTreeSet` model, run `verify` after every step, and check that every page the old root reaches is unchanged (copy-on-write).

## 7. Reading your own writes

A query inside a `WriteTransaction` stops copying the database. Today `merged_query_facts` clones every committed and pending fact (`db.rs:1733`). Instead it reads a `Snapshot` (§4.1 of the #473 spec) with two changes:

- **committed:** for a large transaction, an `OnDiskReader` over the working roots and counters (`OnDiskReader::new` with a `MetaPage` built from them), unpinned: `WriteTransaction` has only `execute`, which drains the query before it returns, and nothing frees the working pages while the transaction holds the write lock. For a small transaction, the active generation as today.
- **staged** (new, optional): a `PendingStore` built at query time from the staged facts, at most `TX_CHUNK_FACTS` of them, with their synthetic `tx_count`s and a default `valid_from` resolved to their staged `tx_id`, as `merged_query_facts` does. `Scan` reads it after the pending store. It is empty for a large transaction, because queries run between statements, and every finished statement is then in the working trees.

The snapshot's `now` is `max(now, pending_read_now_floor())` as today. Each such query then runs on the engine, or on the executor for the clause kinds #551 has not moved yet, both through the snapshot.

## 8. Testing and measurement

### 8.1 Tests

- **Parser.** Every existing parser test and the grammar verifier pass unchanged. A differential test runs the old `parse_transact`/`parse_retract` path (kept under `cfg(test)` until #551) and `StatementReader` over the verifier's generated statements and a corpus of multi-error statements, and requires the same command or the same error code. Pass 2 is forced with a test-only chunk size.
- **Chunk size.** `TX_CHUNK_FACTS` is a `const`. Tests set a thread-local override (`set_tx_chunk_facts`, `cfg(test)` and `test-oracle` only) to 2–8, so the large path runs in every existing integration suite through the oracle hook. The model-based test (`tests/model_based_test.rs`) runs with the override at 3, so its generated transactions cross chunk boundaries and statement bases.
- **B+tree.** §6's property tests.
- **Statement bases.** Last statement wins and `API-011` across chunks and statements, against `keep_last_statement` and `check_one_window_per_triple` on the same statements as a small transaction. A failed statement leaves the transaction usable and the trees as at its base. A rolled-back transaction leaves the file passing `verify`.
- **Crash and faults.** The fault matrix gains a large transaction operation: a fault at every write and sync, on a new file (one valid meta slot) and on a file with both slots valid, with and without a WAL beforehand. `tests/crash_kill_test.rs` kills a child process during a large transaction, after its commit point, and after a rollback before the next checkpoint. Each reopen gives exactly the earlier transactions plus, only after the commit point, this one, and `verify` finds nothing.
- **Concurrency.** A cursor and a fact log opened before, during and after a large transaction see their snapshot only (#473 acceptance test 5).
- **Acceptance.** A 1M-fact `transact` on a file-backed database holds at most `TX_CHUNK_FACTS` materialised facts at any time, counted by a test-only high-water mark in `StatementReader` and the chunk writer. Peak RSS is recorded beside it.

### 8.2 Measurement

On the bench machine, idle: `transact` of 100K and 1M facts, as one `execute` and as a `WriteTransaction` of 10 statements, with sequential and random entities and short and long string values. Wall time, peak RSS, pages written and file size after commit, for `TX_CHUNK_FACTS` of 4,096, 16,384 and 65,536, against `v3` before #550.

Inserting chunk by chunk rewrites a leaf for each chunk that touches it. Where keys are spread out, as in AVET over random values or the DICT's UUID entries, a chunk of `n` facts touches up to `n` leaves of each such tree. A 1M-fact transaction at 4,096 can then write on the order of 10⁶ pages, against one WAL entry and one checkpoint of about 10⁵ pages today. The measurement picks the chunk size, and #551's envelope records the result. There is no bar on wall time (§1).

## 9. Delivery

Three PRs to `v3`, merge commits. None uses a closing keyword for #473 or #550.

1. **Parser and B+tree.** `Lexer`, `StatementReader` with pass 1 and pass 2 and the phase-ordered errors, used for every `transact`/`retract`. `btree::cow_update`, `PageAllocator::release`. No change in behaviour: every statement still takes the small path. Parser differential tests and B+tree property tests.
2. **Large single statements.** §4 for `Minigraf::execute` and for a `WriteTransaction` whose large statement is its first: becoming large, chunks, commit as a checkpoint, rollback, the WAL guard and the open-time dirty rule (§4.5), the fault-matrix and crash tests, the 1M acceptance test.
3. **Statement bases and reads.** §5 for any statement of a large transaction, §7 for every `WriteTransaction` query, the chunk-size measurement and the `TX_CHUNK_FACTS` value, docs: `CHANGELOG.md` (`tx_id` of a large transaction is when it became large; large transactions commit as a checkpoint and write no WAL entry; the first checkpoint after an unclean shutdown writes a meta), `docs/TEST_COVERAGE.md`, `CLAUDE.md`, the docs site's `content/architecture.md` with `<!-- @since v3.0.0 -->`.

## 10. Changes from the #473 spec §8

1. **The parser holds tokens and EDN for one fact only (§3).** The #473 spec bounds "parsed facts", but today's parser holds three whole-input structures before any fact exists.
2. **Errors in phase order (§3.3)**, with one checking pass over every statement and a second pass for large ones. The #473 spec did not say which error a large statement reports.
3. **One DICT `Encoder` per chunk (§4.2)**, reading the working DICT tree. A transaction-wide `Encoder` holds every new entity. The statement base then holds the working counters, not "the ids the Encoder assigned".
4. **The WAL guard and the open-time dirty rule (§4.5).** Without them, a crash in a large transaction on a file with one valid meta slot, such as any new file, is refused with `STG-033` on reopen. The #473 spec's recovery claim did not hold.
5. **The page cache's generation bound is raised to `g + 1` during a large transaction (§4.1).** Otherwise a working-tree page reloaded after eviction fails its generation check (`STG-031`).
6. **No cache eviction on statement rollback (§4.4).** `PageAllocator::write` already replaces a reused id's cache entry. The #473 spec asked for the entries of returned ids to be dropped.
7. **No value-page position in the base (§5.1).** Value pages are appended per chunk and are covered by the ids handed out.
8. **`cow_update` combines insert and delete (§6)**, and a delete of a missing key is an error.
9. **Small transactions also read through a snapshot (§7)**, with the staged facts as a second pending part, instead of `merged_query_facts`.
10. **A failed window check of earlier statements at the switch (§4.1 step 5)** rejects the current statement and leaves the transaction small. The #473 spec did not cover it.
