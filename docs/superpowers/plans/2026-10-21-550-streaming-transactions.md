# Streaming transactions (#550) — Plan

Spec: `docs/superpowers/specs/2026-10-21-streaming-transactions-design.md` (merged as #683),
called "the spec" below. PRs target `v3`, merge commits. None uses a closing keyword for #473 or
#550; #550 closes by hand after PR 3.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–9: lexer, statement reader (pass 1 and pass 2), `cow_update` (spec §3, §6) | `feat/550-parser` |
| 2 | 10–15: `PageAllocator::release`, large single statements, WAL guard, open-time dirty rule, crash and fault tests, 1M acceptance (spec §4) | `feat/550-large-statements` |
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

## PR 2 and PR 3

Planned in detail when PR 1 is merged, against the code it leaves. PR 2: `PageAllocator::release` (decision 11), spec §4 for
`Minigraf::execute` and a first-statement-large `WriteTransaction` (the chunk writer,
working trees, commit as a checkpoint, rollback, WAL guard, open-time dirty rule), the oracle
hook for the chunk-size override, fault matrix and crash-kill cases, the 1M acceptance test.
PR 3: spec §5 and §7, the measurement on the bench machine, the final `TX_CHUNK_FACTS`, and
the docs listed in spec §9.
