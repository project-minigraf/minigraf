# Binding write transactions — design (#824)

**Mission Type:** mixed (code in 9 repositories, plus documentation)
**Base branches:** `v3` in `project-minigraf/minigraf` (core) and `v3` in every binding repository. Nothing targets `main`.
**Approved:** 2026-11-10.

## 1. User Intent

> #824

Issue #824, "Expose explicit write transactions across all supported language bindings" (milestone v3.0.0):

> Expose explicit write transactions across all supported Minigraf language bindings, with behavior matching the Rust API: begin a transaction, run multiple reads and writes against its staged state, then commit or roll back.
>
> The immediate consumer is ldbc-benchmarks. FinBench conditional read-write operations need tentative writes, reads that see those writes, and a conditional commit or abort within one operation. Separate auto-committed `execute()` calls cannot preserve that boundary.

The issue's "Expected contract" and "Completion criteria" sections are part of the intent and are covered by the acceptance criteria below.

> [Asked how the bindings should hold a core write transaction across calls and threads] Owned transaction in core.

> [Asked what transaction-control contract the WASI package (the REPL) should have] Document auto-commit only.

## 2. Goal

An application in any binding can begin a write transaction, run several `transact`, `retract` and `query` statements in it with queries seeing the staged writes, and then commit everything atomically or roll it back. Abandoned, failed and misused transactions clean up and never leave the write lock held. To make that sound, core gains a transaction that owns what it needs and can move between threads, and `BrowserDb` gains its own transaction. WASI's contract (auto-commit per REPL command, no explicit transactions) is recorded and tested.

## 3. Terms

- **Core**: the `minigraf` crate, branch `v3`. At planning time `origin/v3` is `d9755c02`.
- **Binding**: one of `minigraf-python`, `minigraf-java`, `minigraf-android`, `minigraf-swift`, `minigraf-node`, `minigraf-c`, `minigraf-wasm` (clones at `../minigraf-<name>`). **Template** is `minigraf-binding-template`. **Native bindings** are all of these except `minigraf-wasm`.
- **UniFFI bindings**: python, java, android, swift and the template. Each has a Rust crate (`src/lib.rs`) and, except the template, a language wrapper.
- **Borrowed transaction**: core's existing `WriteTransaction<'_>` from `begin_write` / `begin_write_with`.
- **Owned transaction**: the new core transaction of section A.
- **Binding transaction**: the object a binding returns from its begin call.
- **Open**: a transaction that has not finished. **Finished**: after `commit` (successful or failed), after `rollback`, or after the database it belongs to was closed (Node, C).
- **Staged**: written by a statement of an open transaction and not yet committed.
- **Begin thread**: the thread that called the begin function of an open transaction.
- **Guarded calls**: the database calls that core refuses with `INT-001` on a thread with an open borrowed transaction today: `execute`, `query`, `prepare` and prepared `execute`/`query`, `fact_log`, `checkpoint`, `verify`, `rebuild_indexes`, and a second begin (`src/db.rs`, callers of `is_write_tx_active`; `tests/write_tx_guard_test.rs`).
- **Scope form**: a wrapper's language construct that ends a transaction when a block is left (Python `with`, Kotlin/Java `AutoCloseable`, a Swift closure form).

## 4. Context & Investigation Findings

### Core (`origin/v3`)

- `src/db.rs:1183` `begin_write`, `:1217` `begin_write_with(TxOptions)`; `:452` `TxOptions { synchronous, chunk_facts }` (#871).
- `src/db.rs:1897` `WriteTransaction<'a>` holds a `MutexGuard<'a, WriteContext>` and `&'a Inner`: it borrows the database and is not `Send`.
- `src/db.rs:47-59`: the thread-local flag `WRITE_TX_ACTIVE` is set by begin and cleared by `commit`, `rollback` and `Drop` on the thread that runs them. A borrowed transaction cannot safely begin on one thread and end on another.
- `WriteTransaction::execute` (`:1941`): a `transact`/`retract` is staged and returns `QueryResult::Ok`; a `query` reads committed plus staged facts (`:2011`); a rule is registered at once in the shared registry and is not undone by a rollback. No cursor and no prepared query exists inside a transaction.
- `commit(self)` `:2170`, `rollback(self)` `:2298`, `Drop` rolls back `:2342`. A transaction that passes its chunk size on a file database becomes large (#550) and commits as a checkpoint.
- A write from another thread waits on the write lock until the transaction ends. A query from another thread reads the committed state without waiting.
- `docs/superpowers/specs/2026-11-06-862-override-audit-design.md` §6: `TxOptions` reaches the bindings with #824.

### Browser (core `src/browser/mod.rs`)

- `BrowserDb` (`:50`) is not built on `Minigraf`. It holds `Rc<RefCell<BrowserDbInner>>` with its own `FactStorage` and page store, takes no options, and has no transaction type. A write is applied and saved synchronously, then flushed to IndexedDB (`apply_write`, `:371`).
- `minigraf-wasm` re-exports core; its `wasi-pkg` is the REPL. `src/repl.rs` has no transaction commands.

### Bindings (`origin/v3` of each; all pin core `d9755c02`)

- UniFFI crates: `MiniGrafDb { inner: Arc<Mutex<minigraf::Minigraf>> }` (python `src/lib.rs:514`); each call clones the `Minigraf` out of that mutex (`db()`, `:651`) so that no binding lock is held while core runs. Objects must be `Send + Sync` and are freed by the host's collector on any thread. The four bindings' `src/lib.rs` are the same file.
- Node (`src/lib.rs:439`): `close()` empties the handle and closes its prepared queries (registry of weak references); calls are synchronous on the JS thread.
- C (`src/lib.rs:88`): per-object `*_last_error`; the database handle keeps a registry of its prepared handles; `include/minigraf.h` is generated by `cbindgen`.
- No binding exposes a transaction or `TxOptions`. Every binding has `consumer-test/` and a `package.yml` workflow.

### Constraints

- Core size gate on `v3`: 1,240 KiB. It is not raised by this change.
- Error codes are never removed or recycled. This change is expected to add none: `INT-001` and `API-018` are reused.
- Core PRs to `v3` merge with a merge commit. No PR is merged without the maintainer's explicit confirmation.
- Tests follow `CLAUDE.md` §Testing Conventions (no `{:?}` of `Result`/`Value`/… in assert messages).

## 5. Acceptance Criteria

### A. Core PR: owned transaction

- A1. `Minigraf` has two new public functions that begin an owned transaction, one with the database's options and one taking `TxOptions`. The owned transaction type is `Send` and `'static`, keeps the database open while it exists, and offers `execute`, `commit` and `rollback` with the results and errors of the borrowed transaction's. Dropping it open rolls it back. A compile-time test asserts `Send + 'static`.
- A2. For any sequence of statements, an owned transaction leaves the database in the state a borrowed transaction given the same statements and `TxOptions` leaves it in: after commit, after rollback and after drop; small and large (#550) transactions; file and in-memory databases. Test: `owned_tx_matches_borrowed_tx`, comparing query results and, on a file database, results after reopening.
- A3. An owned transaction may begin on one thread and run `execute`, `commit`, `rollback` or be dropped on another. Afterwards every guarded call succeeds again on every thread, including the begin thread. Tests: `owned_tx_commit_on_other_thread`, `owned_tx_drop_on_other_thread`.
- A4. While an owned transaction is open, on its database: every guarded call from the begin thread fails with `INT-001`; a write or a begin (owned or borrowed) from any other thread waits until the transaction ends and then proceeds; a query from another thread returns the committed state without waiting. A different database is not affected by it. Tests: `owned_tx_guards_begin_thread`, `owned_tx_other_thread_write_waits`, `owned_tx_other_thread_query_reads_committed`, `owned_tx_leaves_other_database_usable`.
- A5. Beginning an owned transaction fails with `INT-001` on a thread that has an open transaction of either kind on that database, with `API-014` on a read-only handle, and leaves the handle usable in both cases.
- A6. The borrowed transaction's public API and behaviour are unchanged: every existing test passes unmodified, including `tests/write_tx_guard_test.rs`, `tests/concurrency_test.rs`, the fault-injection and crash tests, and the lock-poisoning test (`API-001`). `cargo semver-checks` (v3 vs PR base) passes.
- A7. The model-based or fault-matrix test that covers `WriteTransaction` commits today also runs its transaction cases through the owned transaction, or the PR description states which existing suite covers the changed lock and why that is sufficient.
- A8. The size gate passes at 1,240 KiB. If it cannot, the work stops and the measured size is reported; it does not raise the gate.

### B. Core PR: browser transaction

- B1. `BrowserDb` has a synchronous begin function returning a transaction object with a synchronous `execute(datalog)`, an asynchronous `commit()` and a synchronous `rollback()`. It takes no options.
- B2. `execute` inside the transaction: a `transact`/`retract` is staged and returns the JSON the browser API uses for core's `QueryResult::Ok`; a `query` returns committed plus staged facts in the JSON shape of `BrowserDb.execute`; a rule is registered at once. A statement that fails stages nothing and leaves the transaction open.
- B3. For any sequence of statements, committing a browser transaction leaves `BrowserDb` returning the same query results as an in-memory core `Minigraf` returns after a transaction with the same statements, including the rejections (`API-011`, invalid valid-time windows) and the latest-statement-wins rule (#477). Test: a `wasm-bindgen` differential test over a fixed list of sequences.
- B4. `commit()` applies the staged facts as one transaction (one `tx_count`; no query sees part of it), and its promise resolves after the IndexedDB flush. After a resolved commit, reopening the database by name returns the committed facts. A failed flush rejects the promise and leaves the state a failed flush of `BrowserDb.execute` leaves today.
- B5. `rollback()`, and dropping or freeing the object while open, discard the staged facts: no query returns them and a reopen does not either.
- B6. While a browser transaction is open, `BrowserDb`'s `execute`, `query`, `prepare`, `checkpoint`, `importGraph`, a second begin, and `execute`/`query` of a prepared query throw or reject with an error whose message starts with `[INT-001]`. A cursor opened before the transaction began keeps returning its rows. After the transaction finishes, all of them work.
- B7. A finished browser transaction: `execute` and `commit` fail with `[API-018]`; `rollback` does nothing.
- B8. `wasm-bindgen` tests in `src/browser/` cover B2–B7, including a persistent (IndexedDB) commit-and-reopen and rollback-and-reopen.

### C. Core PR: documentation

- C1. `CHANGELOG.md`, `docs/TEST_COVERAGE.md`, the `CLAUDE.md` test count and module notes, and `README.md` where it describes transactions or the browser API.
- C2. `docs/ERROR_REFERENCE.md` and the `INT-001` registry text describe the owned-transaction case (the code is unchanged), and `API-018` names a finished transaction among its causes.
- C3. This spec is copied to `docs/superpowers/specs/2026-11-10-824-binding-transactions-design.md`.

### D. Every native binding (6 bindings + template): transaction object

- D1. The database object has a begin call returning a binding transaction, and a form of it taking transaction options: `synchronous` (the binding's existing spelling of `Full`/`Normal`) and `chunk_facts`, each optional, absent meaning the database's value. A negative `chunk_facts` is rejected as the binding rejects other negative counts. On a read-only database, begin fails with `API-014`.
- D2. The transaction has `execute(datalog)`, returning what the database's `execute` returns for the same core result (a staged write returns the binding's encoding of `QueryResult::Ok`), `commit()` and `rollback()`. It has no cursor and no prepared-query call.
- D3. Read-your-own-writes: after a `transact` and a `retract` in a transaction, a `query` through the transaction returns the staged state; the same query from another thread through the database returns the committed state (Node: checked before begin and after rollback instead).
- D4. `commit()` makes every staged change visible to database queries at once and, on a file database, present after close and reopen. `rollback()` leaves the database as it was, also after close and reopen.
- D5. A statement that fails (parse error, `API-011`, …) reports core's error code, stages nothing, and leaves the transaction open and usable.
- D6. A finished transaction: `execute` and `commit` fail with `API-018`; `rollback` and close/free do nothing and report no error. A `commit` that fails finishes the transaction and rolls it back.
- D7. Abandonment rolls back and releases the write lock: dropping the last reference in the Rust crate (UniFFI, Node), `minigraf_tx_free` on an open transaction (C), and leaving a scope form without a commit, normally or by an exception. After each, a write through the database succeeds from every thread. Tests name each path.
- D8. While a transaction is open, the database object behaves as A4 states: guarded calls on the begin thread fail with `INT-001`; a write or begin from another thread waits and then succeeds; a query from another thread reads the committed state. While a begin waits, queries from other threads through the same database object still run (the binding holds none of its own locks while it waits).
- D9. One transaction object used from two threads at once does not interleave within a statement and cannot corrupt state: each call runs whole, and a call that loses a race with `commit`/`rollback` fails with `API-018`. A transaction begun on one thread may be committed, rolled back or freed on another (A3).
- D10. Closing the database with a transaction open: in Node (`close()`) and C (`minigraf_close`) the transaction is rolled back and finished (`API-018` on later use; a C transaction handle must still be freed, and freeing it after the close is safe). In the UniFFI bindings the transaction keeps the database open, as prepared queries do.
- D11. `chunk_facts` works through the binding: on a file database, a transaction begun with a small `chunk_facts` that stages more facts than that commits, and every fact is returned after reopening.
- D12. A user function registered on the database runs in a query inside a transaction and sees staged values. A user function that makes a guarded call on the database fails that query with `QRY-014`; nothing deadlocks.
- D13. Rules registered inside a transaction remain after a rollback (tested once per binding implementation: UniFFI crate, Node, C).

### E. Public names

Names follow each binding's conventions. This table is the contract unless a name collides with an existing one, in which case the PR description states the name used.

| Where | Begin | Options | Transaction |
|---|---|---|---|
| Core (Rust) | `Minigraf::begin_write_owned()`, `begin_write_owned_with(TxOptions)` | `TxOptions` (existing) | `OwnedWriteTransaction`: `execute`, `commit`, `rollback` |
| Core browser | `BrowserDb.beginWrite()` | none | `BrowserWriteTransaction`: `execute`, `commit` (Promise), `rollback` |
| UniFFI crates (all 5) | `MiniGrafDb.begin_write()`, `begin_write_with(options)` | record `TxOptions { synchronous: SyncMode?, chunk_facts: i64? }` | `MiniGrafWriteTransaction`: `execute`, `commit`, `rollback` |
| Python wrapper | `db.begin_write(synchronous=None, chunk_facts=None)` | keyword arguments | context manager: `with db.begin_write() as tx:` rolls back on exit unless committed |
| Java, Android (Kotlin) | `beginWrite()`, `beginWrite(options)` | `TxOptions` with a builder or named arguments, as the binding's open options do | `AutoCloseable`; `close()` rolls back unless committed |
| Swift | `beginWrite(options:)` | `TxOptions` | plus a closure form that rolls back when the closure throws or returns without committing |
| Node | `db.beginWrite(options?)` | `{ synchronous?: 'full' \| 'normal', chunkFacts?: number }` | `WriteTransaction`: `execute`, `commit`, `rollback`; typed in `index.d.ts` |
| C | `minigraf_begin_write(db, options_json)` (`NULL` = defaults; unknown keys rejected) | JSON `{"synchronous": …, "chunk_facts": …}` | `MiniGrafTx*`: `minigraf_tx_execute`, `minigraf_tx_commit`, `minigraf_tx_rollback`, `minigraf_tx_last_error`, `minigraf_tx_free`; declared in `include/minigraf.h` |

### F. wasm binding

- F1. Pins the merged core commit; the generated TypeScript declarations list the begin call and the transaction class.
- F2. `consumer-test/browser/contract.js` runs, against the packed package: a commit with read-your-own-writes, a rollback, and one `[INT-001]` rejection from B6.
- F3. WASI contract: the `wasi-pkg` README states that the package is the REPL, that every command commits on its own, and that it has no explicit transactions. A `wasi-pkg` test pipes a `transact`, a command that fails, and a `query`, and checks that the first command's fact is returned (the failed command did not undo it). `wasi-pkg` builds and its existing tests pass at the new pin.

### G. Every binding PR (7 bindings + template)

- G1. `Cargo.toml` pins a commit that is on core `v3` and contains the core PR. No PR is marked ready for merge while it pins a commit that is only on a PR branch.
- G2. The binding's own tests, its `consumer-test`, its `package.yml` and its other CI jobs are green at the new pin. Anything the pin move breaks is fixed in the same PR and listed in the PR description.
- G3. Tests exist at two levels in each native binding: the Rust crate (all of D, including the thread cases D8 and D9 and the reopen cases D4 and D11) and the language wrapper or consumer test through the built package (commit with read-your-own-writes, rollback, scope-form exit by exception, a finished-handle error, commit surviving reopen).
- G4. The README documents: begin, the options, `execute`/`commit`/`rollback`, the scope form with one complete example, what happens on abandonment, the `INT-001` rule for the begin thread and waiting from other threads, database close with an open transaction, that rules are not rolled back, and that an open transaction holds the write lock until it finishes, so it should always be ended explicitly or through the scope form.
- G5. The README or the binding's changelog names the first binding version that has write transactions and the core commit it embeds.
- G6. The PR description references #824 without a closing keyword.

### H. Documentation outside the binding repositories

- H1. Docs-site PR (`../minigraf-docs`): the pages that describe the bindings and the browser API (`content/use-cases.md` and whichever page documents transactions) describe binding transactions with one example per binding, the browser difference (no WAL; a commit is durable when its promise resolves), and the WASI contract. `content/comparison.md` is updated if its matrix has a transactions row. New text is inside `<!-- @since v3.0.0 -->` where the surrounding text is shared with v2. Its link check passes.
- H2. No document contradicts another: core docs, the binding READMEs and the docs site agree on names and behaviour.

### I. Completion

- I1. Ten PRs exist and are green: core, 7 bindings, template, docs site.
- I2. #824 is closed by hand after the last PR merges, with a comment that lists the ten PRs and, per binding, the pinned core commit and the first transaction-capable version.

## 6. Edge Cases & Boundaries

- **Empty transaction.** Commit of a transaction with no staged write succeeds and changes nothing a query can see.
- **Only queries.** A transaction that ran only queries commits and rolls back without error.
- **Large transaction.** A binding transaction that becomes large behaves as core's does; its rollback and abandonment discard its working pages (D7 and D11 apply to it).
- **Failed handle.** After a failed checkpoint (`STG-046`), begin, `execute` and `commit` report what core reports; the binding adds nothing.
- **`synchronous`.** `Normal` cannot be shown to skip a sync from a binding test; it is enough that the option is accepted and the commit succeeds. In-memory databases accept and ignore both options.
- **Collector timing.** A collector may never run. No test may rely on a JVM, Python or JS collector to release a transaction; abandonment is tested through the Rust crate and the scope forms.
- **Waiting begin.** A begin from another thread blocks that thread for as long as the open transaction lasts. This is core's behaviour; the binding documents it and adds no timeout.
- **Transaction moved to another thread.** `INT-001` follows the begin thread, not the thread now using the transaction. A thread that uses a transaction it did not begin and then writes through the database waits on its own transaction. The READMEs warn against it.
- **Node.** There is one JS thread: every guarded call during an open transaction is `INT-001`, never a wait.
- **Browser awaits.** Other asynchronous code may run between a browser transaction's statements. B6 makes its database calls fail loudly instead of reading or writing outside the transaction.
- **Browser commit in flight.** Between `commit()` being called and its promise resolving, the transaction is finished and the facts are visible, as with `BrowserDb.execute` writes today.
- **Double free (C).** `minigraf_tx_free(NULL)` is a no-op, as the other `*_free` functions are.
- **Regressions to avoid.** `execute` and prepared queries outside a transaction behave as before; auto-committed writes keep their result JSON; the C header stays in sync with the exported symbols; Node's `index.d.ts` stays in sync with the addon; core durability behaviour (WAL sync modes, crash recovery, #554 whole-transaction visibility) is unchanged.

## 7. Key Decisions

- **Owned transaction in core** (maintainer, 2026-11-10). Core's borrowed transaction cannot be held by a binding object: it borrows the database, is not `Send`, and is tracked by a thread-local flag. The alternative, a worker thread inside each binding, would have meant three separate thread mechanisms and no way to call a Node user function from inside a transaction. The price is a change to the write lock used by every write path shortly before the v3.0.0 cut, which is why A2, A6 and A7 demand equivalence with the borrowed transaction and unchanged existing tests.
- **Guard follows the begin thread, per database.** It keeps Rust's rule (same thread: `INT-001`; other threads: wait) for a transaction that can move, and does not make one database's transaction block another database, which the thread-local flag does for borrowed transactions today. That existing quirk is left alone.
- **WASI: auto-commit only** (maintainer, 2026-11-10). The WASI package is the REPL; no transaction commands are added.
- **Surface equals core's.** `execute`, `commit`, `rollback` only. No cursor, no prepared query inside a transaction, no nested transactions, no savepoints.
- **`TxOptions` comes with the transactions**, as the #862 design §6 planned. The browser gets none: `BrowserDb` takes no options and has no WAL.
- **Finished objects fail with `API-018`**, the code bindings already use for closed objects.
- **Commit is always explicit.** A scope form never commits on normal exit; that keeps one rule in every language and makes a forgotten commit visible in tests instead of silently durable.
- **Browser calls fail during a transaction** rather than reading committed state, because the issue asks that ordinary calls are never mistaken for transaction-local ones.
- **Template included**, as in #180, #181, #316 and #872.

## 8. Out of Scope

- REPL transaction commands, and any library API for WASI.
- Cursors, prepared queries, savepoints or nested transactions inside a transaction.
- Options on `BrowserDb`; OPFS (#441).
- Rolling back rule registration.
- Lock timeouts or a non-blocking begin.
- Changing the borrowed transaction's cross-database `INT-001` behaviour.
- The ldbc-benchmarks switch from its JNI bridge to the Java binding.
- Changes to `main` or to v2.x binding branches; releasing or tagging any repository; merging any PR.

## 9. Possible Directions & Ideas (non-binding)

- The UniFFI crates' `src/lib.rs` files are the same; doing the template first and carrying the change to the other four may save time.
- A binding PR can be developed against the core PR's head commit and re-pinned to the `v3` merge commit once the maintainer merges core.
- Prior binding rollouts: #872, `docs/superpowers/plans/2026-10-24-181-prepared-ffi.md`, `2026-10-24-180-udf-uniffi.md`. The Node and C prepared-query registries show how `close()` reaches dependent objects.
- `src/db.rs:2011` (`read_snapshot`) shows how staged facts are layered over a snapshot; the browser transaction may be able to use the same snapshot call.
- The borrowed transaction could be expressed through the owned one or left as it is; either is acceptable if A6 holds.
- The laptop has 15 GB of RAM: build one binding at a time. Tests and timings run on the bench machine first.
