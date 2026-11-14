# 005-browser-opfs — BrowserDb OPFS-backed persistent mode (#441)

**Mission Type:** mixed (Rust/WASM code, browser tests, documentation)

**Base branches:** core `v3` (read at `origin/v3` 108b99394; re-checked at 9fd321e7 with no change under `src/browser`, `src/storage` or `src/error.rs`; the Generator records the commit the worktree is cut from); minigraf-wasm `v3`; minigraf-docs `main`. Nothing lands on core `main`.

**Deliverables:** three PRs — core (`feat/441-browser-opfs`, worktree under `.worktrees/`), minigraf-wasm, minigraf-docs — merged in that order. #441 is closed by hand after the last merge. PRs say `Refs #441`, no closing keywords.

## 1. User Intent

> #441

GitHub issue project-minigraf/minigraf#441, "BrowserDb: OPFS-backed persistent mode (sync access handle, no in-memory buffer)", milestone v3.0.0. Its body and comments are part of the intent. Answers given during planning (2026-11-14):

> [Recovery without a WAL after an interrupted commit] Reopen at last commit.

> [IndexedDB failed-flush fix] Separate issue. *(filed as #894)*

> [Releasing the database / lock] close() on every mode.

> [Interrupted importGraph in OPFS mode] Old or new, never a mix.

> [Cursors and prepared queries created before an OPFS import] Invalidate them.

## 2. Goal

Give `BrowserDb` a second persistent mode that keeps the database as one real `.graph` file in the Origin Private File System, read and written through a `FileSystemSyncAccessHandle` inside a dedicated Web Worker. Unlike IndexedDB mode it holds no in-memory copy of the pages: the storage layer keeps at most a page cache's worth of pages, and opening a healthy database does not read all of it (AC13 states what this bound leaves out). Commits use the v8 copy-on-write protocol directly on the file, with no WAL and no separate flush step. IndexedDB mode stays as it is (and starts working inside workers); OPFS mode ships in v3.0.0 as a Tier 2 backend inside the Tier 1 browser binding.

## 3. Terms

- **IndexedDB mode**: `BrowserDb.open(name)`; every page in memory (`BrowserBufferBackend`), written through to IndexedDB after each commit.
- **OPFS mode**: the new mode, `BrowserDb.openOpfs(name)`.
- **Commit**: one `PersistentFactStorage::save()`: data pages, sync, meta page, sync.
- **Commit point**: a commit has reached its commit point when the file holds a valid meta image of its generation (one that open would select). This is a fact about the bytes, not about what the write call reported: a meta write that returned an error or a short count may still have left the complete new image (the meaningful bytes and CRC of a meta page are at its start, `src/storage/meta.rs:106-168`; the fault backend's 512-byte tear does exactly this, `persistent_facts.rs:2163-2203`). A crash before the commit point reopens at the previous committed state; after it, at the new one, even if the final sync or the caller's promise never completed.
- **Previous committed state**: the state of the last commit that reached its commit point before the attempt in question, whether or not its caller had observed the acknowledgment yet. Acknowledgments say what must be kept; they do not cap what recovery may return.
- **Uncertain commit**: a commit whose call was interrupted or rejected after it may have written. After reopen it is wholly present (it reached its commit point) or wholly absent; a rejection does not prove absence.
- **Acknowledged**: the promise of `execute`, `tx.commit()`, `checkpoint()` or `importGraph()` has resolved and the code that awaited it has observed that (in the termination tests: the page that owns the worker has received the worker's message for that commit).
- **Importing**: an OPFS-mode database is importing from the start of an `importGraph()` body until that body ends (resolve or reject), cleanup included.
- **Disposal operations**: generated `free()`, a cursor's `close()`, a transaction's `rollback()`, and `BrowserDb.close()` on an already closed database. They are always permitted and never throw the errors below.
- **Slot**: one of the two meta pages (pages 0 and 1). **Valid / damaged / empty** as in the v8 format spec §4.1.1.
- **Dependents**: `BrowserCursor`, `BrowserPreparedQuery` and `BrowserWriteTransaction` objects obtained from a `BrowserDb`.

## 4. Context & Investigation Findings

### Existing code

- `src/browser/mod.rs:35-48`: `BrowserDbInner` is typed to `PersistentFactStorage<BrowserBufferBackend>` plus `Option<IndexedDbBackend>`; every write path (`apply_write` :435, `commit_stamped` :495, `tx_commit` :640, `checkpoint` :231, `import_graph` :288, `open` :107) is "sync save into the buffer, then async IndexedDB flush". OPFS mode changes the facade, not only adds a backend.
- `src/storage/mod.rs:80-127`: `StorageBackend` (sync, `Send + Sync`), including `holds_every_page()` (file-length checks STG-044/INT-049, `persistent_facts.rs:810-830`) and `partial_page()` (short-file check STG-045, `:203-212`). A file-like backend must answer both truthfully.
- `src/storage/persistent_facts.rs:79-113`: the storage is generic over the backend and all backends share `select_meta` (:198-283). `save()` :433-520; a failed save sets a state that refuses later saves with STG-046 (:434, :442).
- `src/storage/persistent_facts.rs:855-893` `check_single_valid`: with one valid slot at generation `g` and no WAL, a valid page stamped `g+1` means STG-033. Native test `no_wal_with_next_generation_pages_is_stg_033` (:1612). This is the rule OPFS mode cannot keep (§7 D1).
- `src/browser/mod.rs:86,113`: browser storage opens with page cache capacity 0, right only for the buffer.
- `src/browser/mod.rs:500-507`: facts are loaded into `FactStorage` before `save()`, so after a failed commit they stay readable.
- `src/browser/mod.rs:716-719`: a prepared query holds the `Rc<RefCell<BrowserDbInner>>`; `:670-674` a cursor holds a `crate::Cursor`, which keeps storage alive through its snapshot and readers. Both extend the database's lifetime, by different routes, and there is no `close()`. `:327`: in IndexedDB mode, cursors opened before an import keep the old buffer.
- `src/browser/udfs.rs:22-27, 657, 696`: `execute`, `checkpoint` and `importGraph` are Rust `async` methods, so their bodies run from a microtask after the caller's synchronous code (and any outer query) has returned; tests `async_methods_from_a_function_run_after_the_query` and `import_graph_from_a_function_runs_after_the_query` pin this, and minigraf-wasm's README documents it. `tx.commit()` differs: it applies the transaction synchronously and only the flush is deferred (`mod.rs:552-574`).
- `src/storage/persistent_facts.rs:792-799`: dropping the storage may save dirty state and never calls `StorageBackend::close()`; `:2182-2203` the native crash test accepts either whole generation after a failed save and requires the new one after a successful save — the oracle AC5 follows.
- `src/browser/mod.rs:288-313`: IndexedDB-mode import validates only page alignment and the selected meta; other pages are checked lazily on read, and the buffer skips file-length checks. `src/error.rs:2206-2226`: an uncoded error reaches JS as `INT-000`.
- `src/browser/indexeddb.rs:75`: `web_sys::window()` makes IndexedDB mode fail in workers.
- `src/error.rs:1348`: STG-026, the kernel-lock contention code. `:1469` STG-046. `:1625` API-018 (`{} is closed`). `src/browser/mod.rs:864`: every engine error reaches JS as `[CODE] text` (#673); environment errors in `indexeddb.rs` are still uncoded strings.
- `src/storage/persistent_facts.rs:29-33`: large-transaction streaming is not built for wasm; `:311-325` v7 migration reads all facts into memory. Neither changes here.
- Tests today: `.github/workflows/wasm.yml:22-39` runs the lib's `wasm_bindgen_test`s in headless Firefox and Chrome on the main thread (`run_in_browser`); nothing runs in a worker. minigraf-wasm `consumer-test/browser/driver.mjs` drives the built package in Playwright Chromium; `Cargo.toml:16` there pins core by git rev.
- Dependencies: wasm-bindgen 0.2.126 / web-sys 0.3.103 (minigraf-wasm lockfile). The sync-access-handle bindings without options are stable in web-sys; `create_sync_access_handle_with_options` is unstable and must not be used. wasm-bindgen 0.2.126 already implements `Send`/`Sync` for `JsValue` when atomics are off, so the issue's unsafe wrapper may be unnecessary.
- Native size gate: `binary-size.yml:17`, 1,280,000 bytes (1,250 KiB), native `dist` binary only. Browser code is wasm-only.

### External facts

- [File System Standard](https://fs.spec.whatwg.org/#api-filesystemsyncaccesshandle): sync access handles exist only in dedicated workers in secure contexts; the default mode is exclusive and a second acquisition fails with `NoModificationAllowedError`; `read`/`write` may be short; `flush()` attempts to transfer cached changes to the underlying storage, which is not a power-loss guarantee, and nothing is said about torn writes. The standard does not define `FileSystemFileHandle.move()`; stable web-sys has no binding for it, and only Chrome documents it.
- [Storage Standard](https://storage.spec.whatwg.org/#persistence): OPFS shares the origin's best-effort bucket with IndexedDB; persistence is a permission the application requests.

### Stale statements to correct

`ROADMAP.md:150` (#441 under v3.next), `PHILOSOPHY.md:270` ("#441, v3.next"), docs site `content/use-cases.md:468` ("v3.1.0"), the issue's 2026-10-14 comment. The milestone is v3.0.0. The issue's "wiki" doc targets mean the docs site.

### Deviations from the issue text

1. Recovery is not "the native recovery path" unchanged: D1.
2. `close()` is added (the issue names no lifecycle API): D3.
3. The IndexedDB failed-flush fix is not in this mission: #894.
4. The unsafe `Send`/`Sync` wrapper is not required: D8.
5. "`getFile()` is blocked while a sync handle is open" is not asserted; export and import go through the held handle regardless.
6. Import invalidates earlier cursors and prepared queries in OPFS mode: D5.
7. The promise-based main-thread wrapper is left out: §9.

## 5. Acceptance Criteria

Test names are suggestions unless marked **(named)**. Every behavioural AC is covered by at least one automated test whose name states the behaviour, listed in `docs/TEST_COVERAGE.md`; AC13(c), AC15's WebKit and private-browsing records and AC17 are checked as evidence and documentation.

**AC1 Entry point.** In a dedicated worker with OPFS, `await BrowserDb.openOpfs(name)` opens or creates the database `name`; data written and acknowledged is there after `close()` and a new `openOpfs(name)`. `open(name)` (IndexedDB) and `openInMemory()` are unchanged. The same name in IndexedDB mode and OPFS mode are two separate databases; there is no automatic fallback or migration between them.

**AC2 Unsupported context and names.** `openOpfs` on the main thread, or where OPFS or sync access handles are unavailable, rejects with one new API-category code ("OPFS mode is not available here", naming what is missing); nothing is created and no wasm trap occurs. An invalid name (empty, containing `/` or `\`, `.` or `..`, or a name the import scheme reserves) rejects with `[API-017]` before any file is created.

**AC3 API parity.** Every `BrowserDb` method keeps its name, sync-or-Promise shape, result encoding and scheduling in OPFS mode: `execute`, `checkpoint` and `importGraph` bodies still run from a microtask (so called from a user function they run after the query, and an import then succeeds, as the two `udfs.rs` tests pin); `query`, `prepare`, `exportGraph`, `beginWrite` and the transaction's `execute` stay synchronous; `tx.commit()` applies synchronously and returns a Promise. The behaviours the IndexedDB-mode tests pin and that are not about the buffer or IndexedDB records pass against OPFS mode in a dedicated worker: persistence across reopen, cursors fixed at open, prepared binds and `with*` setters, temporal queries, rules, UDFs incl. re-entry, default override list, transaction staging/read-your-own-writes/one `tx_count` per commit/rollback/drop/`INT-001` guards/`API-018` after finish, and the native differential check of `src/browser/tx_tests.rs`.

**AC4 Acknowledged commits survive.** After a write is acknowledged, terminating the worker without `close()` and reopening shows that commit (its history: later valid retractions still apply). A rolled-back or dropped transaction leaves nothing after reopen. OPFS mode creates no WAL and, outside an import in progress or awaiting cleanup, no file other than the database file.

**AC5 Interrupted commit (D1).** For an OPFS-mode database, a crash at any point of a commit leaves a file that `openOpfs` opens. The reopened state is the previous committed state, or that plus the interrupted commit whole (when it reached its commit point). It always contains every acknowledged commit. Never a partial transaction, never a refused open. Verified at two levels:
- (a) natively, with the existing fault-injection approach, on a backend configured as OPFS mode is (every page stored, no WAL, WAL-less recovery rule): for every write/sync boundary of a first commit on a new database and of a later commit, including torn writes of the meta page and of a data page and a failed final sync, reopen succeeds and full query answers equal the previous committed state or the new state. Fixed cases: `save()` returned success ⇒ new state; a torn meta write whose bytes decode as the valid new meta ⇒ new state; a tear that leaves that meta invalid ⇒ previous committed state; commit B completes (unacknowledged), commit C then fails before its meta is written ⇒ B's state, not A's;
- (b) in a browser: a worker is terminated at arbitrary times during a stream of commits with sequential identifiers, while the owning page records each acknowledgment it receives. After reopen the database holds a gap-free prefix of the commits that includes every recorded one (it may hold more: a commit whose acknowledgment was lost), and no partial commit. Runs in CI on Chromium and Firefox from minigraf-wasm's packaged consumer test.

**AC6 Native rule unchanged.** A native open without a WAL of a file with one valid slot and next-generation pages still fails with STG-033 (`no_wal_with_next_generation_pages_is_stg_033` keeps passing), as do all other native recovery tests. In OPFS mode, every other open outcome matches native for the same bytes: two valid slots (higher generation wins), both slots damaged (STG-032), not a Minigraf file (STG-002), page CRC damage, unsupported feature bits, meta `page_count` past the file end (STG-044), a short non-Minigraf file (STG-045, file not modified), torn initial meta (initialised again). A refused open does not modify the file and releases the handle.

**AC7 Export after recovery.** The bytes `exportGraph()` returns from an OPFS database always open natively with `Minigraf::open()` (no WAL beside them), including when the database was last opened after an interrupted commit and has not committed since.

**AC8 Lock.** While a database is open in OPFS mode, a second `openOpfs` of the same name — from another worker or the same worker — rejects with `[STG-026]`. Different names open side by side. The lock is free again after `close()`, after a failed `openOpfs` (any reason past handle acquisition), and after the worker is terminated.

**AC9 close() (D3).** `BrowserDb.close()` exists in all three modes, is synchronous and idempotent.
- It does no durable work and saves nothing: in the persistent modes every acknowledged write is already stored (an in-memory database is simply discarded), an open write transaction is discarded, and facts of a failed commit are not retried. It therefore reports no storage error; unless refused (next item) the database always ends up closed, with its OPFS handle and lock released, including after a failed commit (AC10).
- `close()` is refused, with the new "database is busy" API-category code and the database left open (no panic), in exactly two situations: it is called from a user function while a query of this database is running, or the database is importing. Other calls from user functions keep today's behaviour (nested `query`, `prepare`, `exportGraph` and `register*` work; `beginWrite` fails as today).
- After `close()`, every method of the database and of its dependents fails with `[API-018]`, except the disposal operations. This holds while dependents are still referenced: AC8's reopen works at once.
- Operations issued before `close()`: (i) an `execute`/`checkpoint`/`importGraph` whose body has not started rejects with `[API-018]` and changes nothing; (ii) a `tx.commit()` whose synchronous commit phase succeeded (a commit that already failed keeps its own rejection) — OPFS mode and in-memory: its promise resolves (in OPFS mode the commit is already stored); IndexedDB mode with the flush not yet submitted: the flush is never submitted and the promise rejects with `[API-018]` (the commit is not stored); (iii) an IndexedDB flush already submitted settles by its own outcome. Nothing issued before `close()` writes into, or publishes state to, a database opened afterwards; tests reopen the same name before the old promise settles and assert both the promise's outcome and the reopened data.
- Freeing a `BrowserDb` and all its dependents without `close()` also releases the OPFS handle and saves nothing as a side effect.

**AC10 Failed commit.** When a commit fails after it may have written (an injected write or flush failure, a short write, quota exhaustion), the call rejects with the "browser storage operation failed" code (D9), whose text carries the operation and the browser exception's name (or "short read/write" when there is no exception) — never `INT-000`. Later writes on that handle reject with `[STG-046]`; reads keep working; `close()` works and frees the lock; a reopen opens cleanly at the previous committed state or, if the failed commit reached its commit point, with it whole. No short read or write is ever treated as a complete page.

**AC11 Portability.** (a) `exportGraph()` from OPFS mode returns a complete v8 `.graph` file: opened natively it passes `verify()` and answers a fixed set of queries (long values, retractions, valid-time and `:as-of`) the same as the browser did. (b) A checkpointed native v8 file and a supported v7 file import into OPFS mode, answer those queries, and still do after `close()` and reopen. (c) Importing a smaller database over a larger one leaves a file with no trailing content of the old one. The native half may use fixture bytes checked in from the browser half, with a test on each side.

**AC12 Import (D4, D5).** In OPFS mode an import has a **publication point**: before it the database is the old one, from it on the new one.
- *Validation, before anything is replaced.* Rejected: a length that is not a page multiple (`[API-017]`; IndexedDB mode keeps its uncoded message); a v8 image with no selectable meta, unsupported feature bits or version, a meta `page_count` beyond the image (`STG-044`), or for which the native verifier's checks (`src/storage/verify.rs`: page checks, index agreement, dictionary consistency, space accounting) report any finding; a v7 image that does not migrate completely. An image with several problems may be rejected with any one applicable registered code. Engine errors that both modes raise keep their codes; OPFS mode rejects more than IndexedDB mode (a page-aligned truncated image and a reachable page with a bad CRC are both tested). An empty blob gives an empty database, as in IndexedDB mode.
- *Failure before publication* (validation, staging I/O, quota): the import rejects with a coded error; the old database, its lock and its earlier dependents keep working; leftovers are removed then or at the next open.
- *Publication* is complete only when the new database is durable; a failure before that is the next item. From publication on, cursors and prepared queries created before the import fail with `[API-018]`. Cleanup follows publication; a cleanup failure does not fail the import (a later open finishes it). When the import has ended, new calls read the imported data.
- *Failure at the publication step itself* (outcome unknown): the import rejects with a coded error and the database becomes closed (`[API-018]` everywhere); reopening gives the complete old or the complete new database.
- *While importing:* admission is checked when a call starts work — at once for synchronous methods, when the body starts for `execute`/`checkpoint`/`importGraph` (which keep their deferred scheduling). A call admitted while the database is importing fails with the "database is busy" code and changes nothing: database methods including `close()` and a second import, and methods of dependents. A call issued during the import whose body starts after it has ended runs normally, against the database as it then is. Precedence when several apply: `[API-018]` (closed database, or a dependent invalidated by publication) over busy over `[INT-001]` (transaction open). Disposal operations are always permitted. An `openOpfs` of the same name from anywhere rejects with `[STG-026]` at every moment of the import and of any recovery or cleanup at open.
- *Interruption* at any point (fault injection at each staging/publication/cleanup boundary, plus worker termination in a browser), including a second interruption during the next open's recovery, reopens as the complete old or the complete new database. Transient files are kept until the chosen database is safely recoverable, are gone after an open that completes cleanup, and their names cannot be opened as databases.
- IndexedDB-mode and in-memory import behaviour is unchanged.

**AC13 Bounded reads and memory.** (a) **(named `opfs_open_read_bound`)** `openOpfs` of a healthy v8 database reads at most a fixed number of pages, the same bound for two databases whose fact counts differ tenfold; IndexedDB mode reads all. (b) **(named `opfs_cache_residency_bound`)** after a workload that touches more pages than the cache holds, with no cursor open, the page cache holds at most its capacity (256 entries, the default; `BrowserDb` takes no options) and the storage layer keeps no other page store. (c) A recorded measurement, `docs/benchmarks/441-opfs/README.md` with the script that produced it and the build profile: databases of at least 4 MiB and 40 MiB with the same schema, built outside the measured instance; each opened in a fresh wasm instance, one fixed point query (one entity, short values) run, then WebAssembly memory size, open time and pages read recorded for OPFS mode and IndexedDB mode. Required: OPFS memory for the larger database is within 4 MiB of the smaller one's (a larger difference fails this AC); the actual numbers are published, and wasm memory size is reported as a high-water mark, not as live memory. The docs state what the bound excludes: query working state, results and export blobs, import input, v7 migration, a transaction's facts, pages and quarantine metadata held for long-open cursors, and recovery scans.

**AC14 IndexedDB in a worker.** `BrowserDb.open(name)`, write, query, `checkpoint` and reopen work in a dedicated worker as on the main thread; existing main-thread IndexedDB tests still pass.

**AC15 Browser evidence.** In CI: the OPFS worker tests (AC1–AC4, AC8–AC12 where browser-level, AC14) run in headless Chrome and Firefox from core (`wasm.yml`); the built `@minigraf/browser` package is exercised in a real dedicated worker (open, write, reopen, lock, main-thread rejection, termination test AC5b) from minigraf-wasm's consumer test on Chromium and Firefox. WebKit: the same package test runs in CI, or the evidence file records the exact harness limitation and a reproducible manual run with its result; it is never reported as passed without a run. Private-browsing behaviour per engine is recorded in the docs from an actual check, not assumed.

**AC16 No regressions, no new dependencies.** Format stays v8. No new crate; only web-sys features are added; no `web_sys_unstable_apis`. Native, WASI and browser test suites, both wasm clippy jobs, coverage gates, the grammar verifier and the native size gate (1,250 KiB) pass. A build with wasm atomics enabled either compiles with sound handle confinement or fails to compile; it never shares the JS handle across threads through an unconditional unsafe impl. The `.wasm` package size before and after is reported in the PR.

**AC17 Documentation.** Updated together and consistent:
- core: `CHANGELOG.md`; `README.md` browser section; `ROADMAP.md` (#441 moves into v3.0.0); `PHILOSOPHY.md:270` (version; still "starts at Tier 2") and §"Single file" line on WASM if it now misstates storage; `docs/DURABILITY.md` (OPFS: durable as far as the browser's `flush()` goes; the WAL-less recovery rule and what it gives up; eviction and `navigator.storage.persist()`, which Minigraf never calls itself; failed-commit behaviour; lock and `close()`); `docs/ERROR_REFERENCE.md` + `src/error.rs` (new codes; STG-026, STG-033, STG-046 and API-018 entries mention OPFS mode); `docs/TEST_COVERAGE.md`; `CLAUDE.md` (module map, test counts, status); a dated note in `docs/superpowers/specs/2026-10-05-v8-storage-format-design.md` §4.1.1; the approved spec copied to `docs/superpowers/specs/2026-11-14-441-browser-opfs-design.md`.
- minigraf-wasm: core pin, `README.md` (OPFS mode, a complete module-worker example with `postMessage`, `close()`, choosing a mode, moving a database between modes by export/import), consumer tests, `readme_sync.py` kept passing.
- minigraf-docs: `content/use-cases.md`, `content/architecture.md`, `content/comparison.md` and the durability page, with `<!-- @since v3.0.0 -->` markers; link check passes.

## 6. Edge Cases & Boundaries

- **Short reads/writes and offsets:** a read or write that transfers fewer bytes than a page is an error or is completed, never a silently short page; page offsets are exact JS numbers or the operation fails before I/O.
- **Partial trailing page:** a file whose length is not a page multiple follows the native rules (`partial_page`), not a blanket rejection.
- **Read failures:** a page that cannot be read is an error; never zero-filled.
- **Empty transaction / no-op checkpoint:** same `tx_count` and write behaviour as IndexedDB mode.
- **Failed commit visibility:** the failed transaction's facts may stay readable on that handle until it is closed (as in IndexedDB mode today), and may or may not be stored (AC10); docs say a rejection means "not confirmed", not "absent".
- **Overlapping calls:** OPFS storage I/O is synchronous while an operation's body runs, so the bodies of un-awaited `execute` calls run one after another in call order, each complete before the next starts; none is acknowledged over a failed predecessor (STG-046). The order of synchronous calls relative to deferred bodies is the same as in IndexedDB mode (a `query` issued right after an un-awaited `execute` runs before that write; a `beginWrite` makes the deferred body reject with `INT-001`).
- **Long-open cursors:** keep their snapshot across commits (page quarantine, as native); they cost file space and quarantine metadata, not correctness.
- **Lock error text:** STG-026's message speaks of a process; the code is reused as is, and its reference entry explains the browser case (another tab or worker; released on `close()` or when the worker ends).
- **Storage pressure:** quota exhaustion and eviction are browser-managed; quota errors surface as AC10 failures, never as lock or corruption codes.
- **v7 import:** supported; may use memory in proportion to the database (existing migration).
- **Names:** distinct names never map to one file (no normalisation); the mapping from name to OPFS entry is documented.

## 7. Key Decisions

- **D1 WAL-less recovery rule (user).** An OPFS-mode database that has exactly one valid slot opens at it, whatever the other slot or later-generation pages look like. Why: with no WAL, an interrupted commit can leave that state (a kill during the first commit of a new database, or a torn meta write later), and the browser has no repair tool, so refusing would lose the database. Given up: if the newest meta page of an acknowledged commit later rots, open falls back to the previous commit silently (native refuses with STG-033). The rule applies to OPFS mode only; native opens are unchanged (AC6). Codex recommended keeping the refusal; the user chose reopening.
- **D2 Entry point `BrowserDb.openOpfs(name)`.** Explicit and additive; no options object; no fallback to IndexedDB (a silent fallback would show an empty database from the other namespace).
- **D3 `close()` on every mode (user).** It does no durable work, so it cannot fail on storage; it is refused only in the two situations AC9 names.
- **D4 Crash-atomic import (user).** A transient second OPFS file during an import is allowed; the single-file property holds outside that window.
- **D5 Import invalidates earlier cursors and prepared queries in OPFS mode (user).** Codex recommended refusing the import instead.
- **D6 No WAL, checkpoint per transaction,** as the issue says.
- **D7 Promise shapes kept.** `execute`, `checkpoint`, `importGraph` and `tx.commit()` stay Promise-returning in OPFS mode: one class and one TypeScript surface; they resolve after the synchronous durable write.
- **D8 Thread confinement without bespoke unsafe where the dependency already provides it** (AC16).
- **D9 Errors.** Lock contention (`NoModificationAllowedError` on acquiring the handle) is exactly STG-026. Invalid name and malformed import length are API-017. Three new registered codes, numbers assigned at implementation: (1) API, "OPFS mode is not available here": no dedicated worker, no OPFS or sync-access-handle API, insecure context, or the browser denies access to the origin's directory (for example `SecurityError` from `getDirectory()`); (2) API, "database is busy" (AC9, AC12); (3) storage, "browser storage operation failed": every other failed OPFS operation — creating or opening an entry, acquiring a handle for a reason other than the lock, read, write, flush, size, truncate, and staging steps of an import — with the operation and the exception name or a short-transfer note; quota exhaustion is here, never under (1) or STG-026. Post-publication cleanup failures are not reported (AC12). STG-003 is not reused. No uncoded or `INT-000` rejection from the new code paths; IndexedDB mode's existing uncoded messages are unchanged. Codes are never removed or recycled.
- **D10 Page cache:** the default 256 pages in OPFS mode.
- **D11 Tier:** OPFS mode is Tier 2 in v3.0.0.
- **D12 IndexedDB failed-flush gap** is #894 (v3.0.0, data-integrity), not this mission. This mission must not make it worse.

## 8. Out of Scope

- #894 (IndexedDB failed flush, abort handling); any other IndexedDB behaviour change beyond working in a worker and `close()`.
- A WAL in the browser; format changes; changes to native recovery.
- Multi-tab or multi-worker shared access, read-only OPFS handles, the Chrome-only handle modes, SharedWorker/ServiceWorker, threaded wasm.
- A packaged promise-based main-thread wrapper (file a follow-up issue in minigraf-wasm at the end of the mission; the README example covers hand-rolled `postMessage`).
- Automatic IndexedDB→OPFS migration; a database-delete or list API; `BrowserDb` open options; calling `navigator.storage.persist()`.
- Streaming import/export APIs; browser large-transaction streaming; bounded-memory v7 migration.
- New nightly benchmark lanes for OPFS (belongs to the #844 re-audit).
- Claims of power-loss durability from worker-termination tests.

## 9. Possible Directions & Ideas (non-binding)

- The file backend (`src/storage/backend/file.rs`) is the model for length, partial-page and lock behaviour; `src/storage/backend/fault_inject.rs` and the recovery tests around `persistent_facts.rs:1537-1645` are the model for AC5a/AC6.
- `wasm_bindgen_test_configure!(run_in_dedicated_worker)` is per test binary, so worker tests probably need their own test target; lock and termination tests need a page that owns workers, which the minigraf-wasm Playwright harness already has (`consumer-test/browser/driver.mjs`).
- For AC7, either repairing the leftover state at open or normalising on export would do; so would any commit protocol that never leaves it.
- For D4, `FileSystemFileHandle.move()` is only a candidate: it is not in the standard or in stable web-sys, and whatever scheme is used must show AC12's old-or-new and lock outcomes on Chromium and Firefox (and WebKit per AC15) before completion is claimed. Native `verify()` (`persistent_facts.rs:519`, `src/storage/verify.rs`) is `cfg(not(wasm32))` today.
- wasm linear memory never shrinks, which is why AC13(c) asks for a fresh instance per measurement.
- Related designs: `2026-10-05-v8-storage-format-design.md`, `2026-10-07-browser-page-reuse-design.md`, `2026-10-21-streaming-transactions-design.md` §WAL guard, `2026-11-10-824-binding-transactions-design.md`.

## 10. What the user should test manually

- The README worker example in a real page on Chrome, Firefox and Safari.
- Private-browsing behaviour on each engine, against what the docs record.

## Open Questions

None. Converged with Codex (gpt-6-astra) in four rounds; Codex-04 is APPROVED, and its two wording notes are applied here.
