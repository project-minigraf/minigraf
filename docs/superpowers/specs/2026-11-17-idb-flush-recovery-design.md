# Spec — 006-idb-flush-recovery (#894)

**Mission Type:** code (plus error-reference and docs-site text)

**Base branches:** PR 1 on `v3` (format v8). After PR 1 merges, PR 2 backports to `main` (v2.x, format v7). A `minigraf-docs` PR covers the docs site. Merge methods: `v3` uses a merge commit, `main` squash.

## 1. User Intent

> #894

Issue #894, "BrowserDb (IndexedDB): a failed flush can leave the store with a meta that references missing pages" (bug, data-integrity, known-issue, v3.0.0). The #896 spike reproduced it on `main`, v2.0.11 and `@minigraf/browser@2.0.11`: a realistic `QuotaExceededError` abort leaves the write's promise pending forever, the next write is acknowledged, and reopen fails with `INT-052`.

Planning decisions by the user (2026-11-17):

> After an IndexedDB flush fails, later operations on that handle follow the **failed-handle rule**: writes, commit, checkpoint and import reject with STG-046, as native and OPFS do; close + reopen recovers. (Chosen over cumulative recovery and over a checkpoint-retry hybrid.)

> The v2.x backport is part of this mission (PR 2 to `main`, after the v3 PR merges).

> On v2.x, register STG-046 and STG-048 with the v3 meanings.

> IndexedDB `importGraph()` keeps today's order: the live store is swapped before the flush, so a failed import leaves the imported graph visible in memory and the handle failed; a reopen shows the old store.

## 2. Goal

Make an IndexedDB-backed `BrowserDb` never acknowledge a write over pages that did not reach IndexedDB. Every IndexedDB flush must land whole or not at all, and must settle. Once a flush fails, the handle refuses further durable work with STG-046 until it is closed and reopened. That is the rule native (#390) and OPFS (#441) already follow. A reopen then loads a complete, consistent store: the state before the failed flush. Ship the fix in v3.0.0 and in the next v2.x patch.

## 3. Context & Investigation Findings

Read at `v3` `df7e8170d` and `main` `82e1ea5e6`. Round-1 reports: `Claude-01.md`, `Codex-01.md`.

### Mechanism (v3, `src/browser/mod.rs`)
- Every persistent write first commits in memory (`commit_stamped` :851: `load_facts`, `pfs.save()`, `take_commit_flush`), then drops the `RefCell` borrow and awaits `IndexedDbBackend::write_pages`. Paths: `apply_write` :791 (`execute` transact/retract), `BrowserWriteTransaction::commit` :914, `checkpoint` :469, IndexedDB `import_graph` :534 (swaps `inner.pfs` before the await), and the migration/free-page flush in `open` :276.
- `take_dirty_pages` :1216 drains the dirty set before reading bytes. `take_flush` :1259 and `BrowserBufferBackend::release` (`buffer.rs:68`) report each freed id once. A failed flush therefore loses its puts and deletes. The next save writes only its own delta and a meta that references the lost pages.
- `pfs.save()` has already succeeded when the IndexedDB flush fails, so `PersistentFactStorage::failed` (`persistent_facts.rs:46–53`, the STG-046 flag) is never set in IndexedDB mode. OPFS mode writes inside `save()`, so its failures already set it.
- Overlap: the borrow is released before every await, and `commit()` clears `tx_open` before its flush. A later operation can commit in memory and queue its IndexedDB transaction before an earlier flush has failed. IndexedDB runs overlapping readwrite transactions on one store in creation order, but it still commits the later one after the earlier one aborts.

### IndexedDB adapter (`src/browser/indexeddb.rs`)
- `transaction_to_promise` :38 listens for `complete` and `error`, not `abort`. An abort with no request error, which the realistic quota case produces, never settles. Its `onerror` is `Closure::once`, so each further bubbling request `AbortError` throws "FnOnce called more than once" (spike).
- `write_pages` :155 queues deletes then puts with `?`. A synchronous throw returns without aborting the transaction, so the requests already queued commit (the spike's sync-throw partial commit).
- Flush failures are bare strings (`"IdbTransaction failed"`). OPFS failures are STG-048 (`opfs.rs:47`).

### Precedent
- STG-046 (ERROR_REFERENCE, v3): after a failed commit that may have written, every later write, checkpoint and index rebuild through the handle fails. Queries keep working, and reopening reads afresh. Its OPFS paragraph (#441) applies the same rule to BrowserDb OPFS mode. IndexedDB mode is the one persistent mode without it.
- STG-048 "Browser storage operation failed" (v3, #441) names the operation and the browser exception.
- `docs/superpowers/specs/2026-10-07-browser-page-reuse-design.md` (#411/#440) deferred failed-flush recovery. The #441 spec (`TandemKit/005-browser-opfs/Spec.md`, D12) delegated it to #894.

### v2.x (`main`)
- Same shape, format v7: `take_dirty_pages` drains, and `write_pages(dirty_pages)` sends puts only. The header (page 0) is rewritten, fact pages are appended, and index pages are rewritten. `BrowserDb` has `open`, `openInMemory`, `execute`, `checkpoint`, `exportGraph`, `importGraph`, with no transactions or `close()`. `error.rs` ends at STG-045, and native v2 has no failed-handle rule.

### Tests and CI
- `src/browser/tx_tests.rs:275` `failed_flush_rejects_like_execute` (v3) closes the IDB connection, an unrecoverable failure. Both the failed `execute` and the failed `commit()` reject, their facts stay visible in memory, and a reopen shows the previous state. Under AC4 the commit is admitted after the handle failed, so it must change nothing (AC15).
- `src/browser/opfs_tests.rs:681–741` `submitted_indexeddb_flush_settles_after_close_and_reopen` pins close/reopen behaviour with a submitted IndexedDB flush, and uses a `JsPatch` interception of `IDBDatabase.transaction`. Another test in the same file pins API-018 for checkpoint, import and execute pending at `close()`, in all three modes.
- `.github/workflows/wasm.yml`: `wasm-pack test --headless` in Firefox and Chrome (window and worker lanes) and Safari (`macos-15`). There is no wasm size gate. The browser code is wasm-only. The native size gate (`binary-size.yml`, v3 `SIZE_LIMIT_BYTES` 1,280,000 = 1,250 KiB) is affected only by the error registrations PR 2 adds on `main`.
- The spike harness (`.worktrees/spike-896/spikes/896-idb-flush/`, Playwright, Chromium, CDP quota override) is the only realistic `QuotaExceededError` trigger. Its oracle checks that every page below the v7 header's `page_count` is present. On v3, free pages are deliberately absent (#440), so any v3 use must check pages reachable from the meta instead.

## 4. Acceptance Criteria

Terms:
- A **flush** is the one IndexedDB readwrite transaction that stores a commit's pages and page deletes.
- A flush **fails** when its transaction aborts or errors, when queueing a request on it throws, or when building it after the in-memory commit fails (e.g. an unreadable dirty page, `INT-052`).
- A **failed handle** is a `BrowserDb` (and its transactions, cursors and prepared queries) on which a flush has failed.

**Flush atomicity and settlement**
- AC1. A flush lands whole or not at all. A synchronous throw while its requests are queued leaves IndexedDB exactly as it was, with no request committed.
- AC2. Every flush settles: `complete` resolves, and `error` or `abort` rejects, exactly once. An abort with no request error rejects; it does not hang. Several request errors on one transaction raise no page error or "FnOnce called more than once" exception.
- AC3. A flush failure in IndexedDB rejects its operation with **STG-048**. The message names the operation (IndexedDB write) and the DOM exception name when one exists (e.g. `QuotaExceededError`, `AbortError`). A failure while building the flush after the in-memory commit (e.g. an unreadable dirty page) keeps its own engine code (e.g. INT-052) and fails the handle in the same way.

**Failed-handle rule**
- AC4. Once a handle is known to have failed, every **newly admitted** `execute` transact/retract, `importGraph()`, `checkpoint()` and `BrowserWriteTransaction.commit()` (empty or not) rejects with **STG-046**. It starts no IndexedDB transaction and changes no database state: no facts, no transaction counter, no live store swap. A commit refused with STG-046 finishes its transaction the way a refused commit does today: the staged facts are discarded, the INT-001 exclusion is cleared, and later calls on that transaction give API-018. `beginWrite()` and a transaction's `execute` (staging, reading) still work. Queries, cursors, prepared queries, rule registration, `exportGraph()` and `close()` keep working. The existing admission rules come first and are unchanged: API-018 after `close()` or on a finished transaction, INT-001 while a transaction is open, API-027 where it applies.
- AC5. Work already applied in memory before the failure became known stays readable on the failed handle. That covers the failed write itself and any overlapping successor applied behind it (AC6), as `failed_flush_rejects_like_execute` asserts today for the failed write.
- AC6. **Persistence order.** Each persistent operation's outcome is ordered after every operation of the handle applied in memory before it. Its flush, if it has one, reaches IndexedDB only after every earlier flush has settled successfully. If an earlier flush fails, a later operation already applied in memory sends nothing and rejects with STG-046. When the earlier flushes succeed, the later one is sent and resolves as today, so overlapping successful writes keep working.
- AC7. **Checkpoint boundary.** `checkpoint()` resolves only once every write applied before it was called is stored. With no earlier durability work outstanding and no pages of its own to flush, it resolves without waiting, as today. Behind an in-flight flush it waits: it resolves if that flush succeeds and rejects with STG-046 if it fails, even when the checkpoint has no pages of its own. Writes applied after the checkpoint was called are not part of its boundary.
- AC8. `close()` is unchanged and does no durable work. A flush not yet started when `close()` runs, including one queued behind a delayed predecessor, is never started, and its operation rejects with API-018 (the existing `commit()` check extended to every path). A flush already submitted settles by its own outcome, and its late completion never overwrites writes made through a fresh handle (`opfs_tests.rs` `submitted_indexeddb_flush_settles_after_close_and_reopen` keeps passing).
- AC9. **Store oracle.** Tests are quiescent: every promise has settled, the handle is closed, and a fresh `BrowserDb.open` of the name follows. That reopen opens without error. Its logical content equals the result of the successful operation prefix up to the first failure (successful transacts, retractions and imports, an import replacing earlier data), and holds nothing from a rejected operation. Every page the selected meta reaches is present. An `exportGraph()` of the reopened database opens natively and passes the native integrity check (v3 `verify()`, run from native test code or an equivalent route; no new `BrowserDb` method). After a failed import, the store's key-to-bytes map equals its map before the import: no imported bytes and no deletes took effect.
- AC10. *(v3 only)* An `open` whose migration/free-page flush fails rejects with STG-048 and returns no handle. A later `open` of the same name succeeds on the unchanged store, migrating again if needed.
- AC11. In-memory and OPFS modes behave as before, and all their existing tests pass.

**Tests** (wasm-bindgen, `--features browser`, every lane `wasm.yml` runs; names are suggestions)
- AC12. Deterministic tests run against real browser IndexedDB transactions and cover these faults: **a recoverable abort** (the connection stays usable), **an abort-only failure** (no request error), **a throw while queueing after earlier requests were queued**, **several request errors on one transaction**, and the existing **closed-connection** case. Production builds carry no fault-injection control.
- AC13. For each of `execute`, `commit()`, `checkpoint()` (its own flush, and an empty checkpoint behind an in-flight write, both success and failure per AC7) and a **smaller-file `importGraph()`**: the operation whose own flush fails rejects with STG-048 (an empty checkpoint waiting behind it rejects with STG-046 per AC7), the next write, commit, checkpoint and import each reject with STG-046, and the reopen satisfies AC9.
- AC14. Also covered:
  - repeated refusal after one injected fault, with only one transaction reaching IndexedDB;
  - a **multi-page write** (≥ 400 facts, enough to append pages IndexedDB never held: the spike's INT-052 shape);
  - an abort-only fault settling within the test deadline;
  - a **flush-preparation failure** (AC3): its error code is asserted, and no later operation submits;
  - **overlap** cases, each asserting that no successor transaction is sent, the AC5 memory contract and the unchanged store: execute then execute, commit then execute, a failed write with an import applied behind it, a failed import with a write applied behind it, and checkpoint behind a write;
  - an all-success mixed-operation control, in which every overlapping operation resolves and the reopened database equals the logical result of the operations in order (AC9).
- AC15. `failed_flush_rejects_like_execute` is updated. The first failure asserts STG-048. Erin's commit, begun after the handle failed, rejects STG-046, finishes the transaction, and leaves Erin out of memory, so memory holds four names, not five.
- AC16. The adapter has its own tests: abort-only rejection, multiple request errors settling once, and a queue-time throw leaving the store unchanged.

**v2.x backport (PR 2, `main`)**
- AC17. Same behaviour on v7 for `execute`, `checkpoint` and `importGraph`: AC1–AC7, with AC4 restricted to the v2 API (no transactions, no `close()`); AC9 without `verify()`, with native open of the export as the integrity route; and AC11 for in-memory mode. `main`'s `open` performs no IndexedDB write (`main:src/browser/mod.rs:76–94`). Instead of AC10, test that a fresh open after a rejected write loads the unchanged store and that writes through the fresh handle work. Don't add v3's open-time writeback to `main`. STG-046 and STG-048 are registered on `main` with the v3 code, title and error text. Their ERROR_REFERENCE entries describe only what v2.x has: the IndexedDB browser path, since v2 native has no failed-handle rule. No other v3 code or API is backported.
- AC18. The v2 tests mirror AC12–AC16, minus transactions and `close()`.

**Evidence and gates**
- AC19. Before the fix, at least the multi-page and abort-only cases fail (or hang, bounded by the test deadline) on unmodified `v3`, and pass after it. The same holds on `main` for PR 2.
- AC20. Evaluator-run evidence: the spike harness's realistic quota case (CDP quota override, Chromium), adapted to the AC9 oracle (v3 free pages are absent by design, so the check covers pages the meta reaches, not a dense range), passes against a PR 1 build and a PR 2 build. The harness itself is not merged.
- AC21. Native `cargo test` and `cargo clippy` (native and `wasm32-unknown-unknown --features browser`) are clean on both branches. Each branch's enforced size gate passes (`binary-size.yml`: v3 `SIZE_LIMIT_BYTES` 1,280,000 = 1,250 KiB; `main`'s own limit), and the PR reports the actual native size delta. `main` gains native-compiled error registrations. No file format, IndexedDB schema/version or public method changes. The fix adds no whole-store rewrite to the healthy path. v3 keeps the page-reuse design's O(change) commits, and `main` keeps its existing write volume. No history or page batches grow with the number of settled flushes; constant-size status such as a sequence counter is fine.

**Documentation**
- AC22. Core, in the same PR: ERROR_REFERENCE STG-046 gains an IndexedDB paragraph (rule, what stays readable, reopen recovers to the state before the failed flush, a rejected write is never stored) and STG-048 names IndexedDB writes. `src/error.rs` messages match. The `BrowserDb` rustdoc (`execute`, `checkpoint`, `importGraph`, `commit`) states the rule. CHANGELOG, `docs/TEST_COVERAGE.md` and the CLAUDE.md test count are updated, and #421's #894 entry gets a Fix line naming v3.0.0 and the v2.x patch.
- AC23. Docs site (`../minigraf-docs`): the browser/IndexedDB durability text states the rule, tagged `@since` the v2.x patch version (and v3.0.0), without rewriting shared text.
- AC24. No commit or PR text uses a closing keyword for #894. It stays open until a published release has the fix.
- AC25. The approved spec is copied to `docs/superpowers/specs/2026-11-17-idb-flush-recovery-design.md` in PR 1.

## 5. Edge Cases & Boundaries

- Validation failures (parse errors, API-019 and the like) are not flush failures. They reject as today and do not fail the handle.
- A `checkpoint()` on a healthy handle with no outstanding earlier flush and no pages of its own resolves without waiting, as today. Behind an in-flight flush it follows AC7. On a failed handle it rejects STG-046, just as native `save` refuses before it checks for dirty state.
- Rule registration (`execute` of a `rule`) is in-memory only and allowed on a failed handle.
- `exportGraph()` on a failed handle exports the in-memory state, including rejected writes. It is not a durability acknowledgement, and the docs say so (the escape hatch the spike observed).
- A failed import: memory shows the imported graph, the handle is failed, and a reopen shows the old store, and no imported bytes or deletes have taken effect (user decision).
- Cursor pins (#543): withheld pages and pinned generations behave as today on a healthy handle. A failed handle frees nothing more.
- Independent handles or tabs writing the same IndexedDB name at the same time are a cross-handle problem and out of scope. Close followed by reopen while an old flush is submitted or queued is in scope (AC8).
- Persistent quota exhaustion: every attempt rejects. There is no background retry.
- A tab kill during a flush leaves IndexedDB at the previous or the new state (IndexedDB transactions are atomic). This needs no new test.

## 6. Key Decisions

- **D1 Failed handle, not cumulative recovery (user).** It's one rule across native, OPFS and IndexedDB. It needs no per-page intent bookkeeping under page reuse, import or pins, and it backports to v7 unchanged. A rejected write never becomes durable. Codex preferred cumulative recovery (the handle self-heals and rejected memory changes persist later). The user chose the failed handle.
- **D2 Ordered flushes, not busy rejection.** A later operation's flush waits for its predecessors and is dropped (STG-046) if one fails. Overlapping successful writes keep working as today, and no new API-027 path is added (Codex's recommendation, agreed).
- **D3 STG-048 for the failure itself, STG-046 afterwards.** This matches OPFS. Today's bare strings are replaced. Codex suggested keeping today's error conventions; coded errors are the project rule (#277), and STG-048 already means "browser storage operation failed".
- **D4 Applied work stays visible in memory** (unchanged, #894 "unless decided otherwise"). That covers the failed write and successors applied before the failure was known. Operations admitted after the handle is known to have failed change nothing (AC4).
- **D8 Preparation failures keep their code.** An engine error such as INT-052 while building a flush keeps its code and fails the handle. STG-048 is only for browser storage calls.
- **D5 Import order unchanged (user).**
- **D6 v2 codes (user).** STG-046 and STG-048 are registered on `main` with the v3 meanings. Codes are never recycled, and v3 already owns them.
- **D7 Backport in this mission (user).** PR 2 starts after PR 1 merges.

## 7. Out of Scope

- Cumulative retry or self-healing handles. Rolling back the in-memory state of a failed write.
- Repairing stores already damaged by #894 (the old handle's `exportGraph()` is only an observation).
- Cross-tab or cross-handle writer isolation for IndexedDB.
- OPFS, native WAL, file format, IndexedDB schema.
- Merging the #896 spike branch. Closing #894 (it closes after release).
- Binding repos: `minigraf-wasm` re-exports `BrowserDb`, and its v3/v2.x pins pick the fix up at the release cascade. Its README text about failures is updated only if it describes the old behaviour (check, don't assume).

## 8. Possible Directions & Ideas (non-binding)

- The failure state could live on `Db` (next to `closed`) or reuse `PersistentFactStorage::failed`. Note that an IndexedDB import replaces `inner.pfs` (`mod.rs:577`), so a flag held only on the old storage would be lost.
- Fault injection can be done from test code by intercepting `IDBDatabase.transaction` / `IDBObjectStore.put`, as `opfs_tests.rs:681–741` (`JsPatch`) and the spike's `harness.js` do, or with a `#[cfg(test)]` hook in `write_pages`. Either way the result must not reach production builds (AC12).
- A per-handle flush chain (a promise or a sequence number of the last settled flush) is one way to meet AC6.
- An abort listener plus idempotent settle (`Closure` that ignores repeats), and `tx.abort()` on any queue-time error, cover AC1–AC2.
- `superpowers:test-driven-development` fits AC19 (failing first on unmodified branches).
