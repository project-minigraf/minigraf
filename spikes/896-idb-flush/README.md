# #896 spike: IndexedDB flush failures in `BrowserDb` (v2.x)

Reproduction for #896 (parent defect #894). This directory is evidence only and is not meant to be merged. It contains no fix.

## What was run

| | |
|---|---|
| Date | 2026-11-17 |
| Browser | Chromium 159.0.8154.6 (Playwright 1.65.1 headless shell), Linux x86_64 |
| Toolchain | rustc 1.94.0, wasm-pack 0.13.1, Node v26.13.0 |
| `main` build | core `19fd080a6`: `wasm-pack build --target web --features browser` (release). wasm sha256 `c4bffe16…2c76b` |
| `v2.0.11` build | core tag `v2.0.11` (`709be06c3`), same command. wasm sha256 `c4bffe16…2c76b`, **byte-identical to `main`** |
| `published` | `npm pack @minigraf/browser@2.0.11`, integrity `sha512-ycTP4OYO…pWaA==`. Built by minigraf-wasm `8b48111` from crates.io `minigraf =2.0.11` (lock checksum `09dff0a2…976c`). wasm sha256 `1f922c64…1e3f` (different crate name, `minigraf_wasm`) |

`src/browser/{mod,buffer,indexeddb}.rs` and `src/storage/persistent_facts.rs` have the same blob IDs at `19fd080a6` and `v2.0.11`.

Each case runs in a fresh, disposable Chromium profile (`launchPersistentContext` on a new temp directory, deleted afterwards) with a unique database name. "Reopen" means a full page reload, `init()` and `BrowserDb.open(name)`: a fresh handle on a fresh WASM instance.

```sh
npm ci                                   # playwright 1.65.1
npx playwright install chromium
PKGS=<dir with main/ v2.0.11/ published/> node run.mjs [build ...]   # CASES=a,b to filter
```

`PKGS/<build>/` is the wasm-pack output (`minigraf.js`, or `minigraf_wasm.js` for `published`).

## Fault injection

`harness.js` wraps `IDBDatabase.prototype.transaction` and `IDBObjectStore.prototype.put` before the WASM module loads. It records every readwrite transaction: page ids and FNV-1a hashes of its puts, and its complete/error/abort events. It also records when each API promise settles. Each fault applies to the next N readwrite transactions:

- **request-abort**: `tx.abort()` in the first put's success handler, while later puts are pending. The pending requests fire `error` (AbortError), which bubbles to the transaction, and then `abort` fires. This is a recoverable failure: later transactions on the same connection work.
- **abort-only** (synthetic): `tx.abort()` after the last put succeeds. Only `abort` fires.
- **quota** (realistic, no wrapper fault): `Storage.overrideQuotaForOrigin` via CDP, set to 100,000 bytes before the origin first uses storage. In this setup an override set after the origin had already written did not take effect, so the case sets it first. The over-quota transaction ends with only `abort` (`QuotaExceededError`) and no request error.
- **sync-throw** (synthetic, no production trigger established): the second `put()` call throws a `DOMException` synchronously.

A run counts as consistent only if all of these hold after reopen:

- it opens, and every acknowledged write is present;
- no rejected or unsettled write is visible;
- IndexedDB has every page below the header's `page_count`;
- the stored page set and every page's contents equal the last in-memory view (`exportGraph` on the old handle; a failed export counts as a mismatch);
- a later write and checkpoint resolve, and after a second reopen the names and the by-value (AVET) and by-entity (EAVT) query rows are exactly as expected.

"Acknowledged" is derived from the traces (each `transact` on the persisted handle whose promise resolved), not assumed by the case. The by-value query reads the found entity's name back, so a wrong entity cannot pass. `faults_seen` counts the faults that took effect: an injected abort only if that transaction then aborted, quota for each `QuotaExceededError` abort, sync-throw only if the transaction then committed. A case whose counts differ from what it is meant to exercise is marked `valid=NO` and fails. Every case in the committed run is `valid=yes`.

## Result matrix

The same results came out on all three builds (`main`, `v2.0.11`, `published`), 14 cases each. Only the order of page ids within a flush differs, because the dirty set is a `HashSet`. `matrix.txt` has the raw output and `results.json.gz` the traces, steps and page inspections.

| Case | API result of the faulted operation | After reopen | Conclusion |
|---|---|---|---|
| baseline (2 writes incl. multi-page, checkpoint) | all resolve | exact | control passes |
| overlap_control (2 concurrent writes) | both resolve | exact | control passes |
| fail_then_checkpoint | write **rejects**, but the fact stays visible in memory. `checkpoint()` **resolves without opening an IDB transaction** | opens with pre-failure state, so the handle's view and storage disagree | confirmed: rejected write stays visible; checkpoint reports success without retrying |
| fail_then_write (1-page failure) | rejects; successor resolves | opens; acknowledged data present; failed write's fact page slot holds a **stale old index page**; header `node_count` counts the failed fact. Load takes the checksum-mismatch rebuild path silently | confirmed divergence, no acknowledged loss |
| repeated_fail_then_write | 2× reject; successor + checkpoint resolve | as above, 2 stale slots | same |
| **fail_big_then_write** (400-fact failure) | rejects; **successor resolves** | **`BrowserDb.open` fails: `[INT-052] Page 6 not found`**. No acknowledged write can be read through a fresh handle | **confirmed: acknowledged data unreachable after reopen** |
| overlap (1-page) | predecessor rejects; overlapping successor resolves | as fail_then_write | successor delta captured before the failure, omits predecessor pages |
| **overlap_big** (400-fact) | predecessor rejects; overlapping successor resolves | **`INT-052`, database does not open** | **confirmed: unreachable after reopen**; a fresh handle cannot help, because the successor's delta was captured before the failure was known |
| abort_only (synthetic) | **promise still pending after 5 s**; a later write resolves | opens; stale slot as fail_then_write | confirmed: an abort with no request error never settles the promise |
| **quota** (realistic `QuotaExceededError`) | **pending after 5 s**; a later write resolves | **`INT-052`, database does not open** | **confirmed from a realistic trigger: acknowledged data unreachable after reopen** |
| sync_throw (synthetic) | rejects | the transaction **commits the put queued before the throw** (one page changed in IDB under the old header); opens via rebuild | confirmed partial commit despite API failure. Synthetic trigger only |
| import_fail | `importGraph` rejects; memory now holds the imported graph; checkpoint resolves without a write; next write resolves | reopens as **old data + successor write** (the imported graph is gone, the old fact `A` is back), while the header counts the imported facts | confirmed: a rejected import leaves the handle on different data than storage, and later writes are acknowledged against it |
| workaround_fresh_handle | after an atomic (request-abort) rejection, no further write on the old handle; open a fresh handle, then write | consistent | mitigation holds in this case |
| quota_workaround | 5 s timeout on the pending write, no further write on the old handle, quota restored, fresh handle, write | consistent | mitigation holds in this case |

### Mitigation and its limits

Tested only in the two cases above. The mitigation is: serialize writes (await each one before starting the next), bound each write with a timeout, and after any rejection or timeout stop using that handle and open a fresh one before the next write. A fresh handle reloads from what IndexedDB actually holds.

It is not shown to help in these cases:

- **Overlapping writes.** A successor started before the failure is known has already captured its delta (overlap_big).
- **Non-atomic failures.** sync-throw commits part of the flush.
- **A full origin.** The quota case restored the quota before writing again.
- **The old operation itself.** A timeout only stops waiting; it does not cancel the operation or prove it rolled back. The timeout case relies on the abort the trace shows.

After `INT-052` the acknowledged pages are still in IndexedDB. In every such run, the original handle, before the page was reloaded, still exported its full in-memory graph (`exportGraph`). No recovery path was tested beyond that observation.

### Second defect: transaction error handler is `FnOnce`

Every request-abort case raises `Error: FnOnce called more than once` page errors: 4 for a 1-page failure, 35 for the 400-fact one. `transaction_to_promise` in `src/browser/indexeddb.rs` registers `onerror` with `Closure::once`, and each pending request's `AbortError` bubbles to the transaction. The first call rejects the promise; each later one throws in the browser's event dispatch. The promise still rejects once, so the results above are unaffected, but the handler is wrong and it is part of #894. (Unhandled-rejection page errors from the harness's own overlap cases were removed by attaching a handler at start; they were artifacts.)

### Mechanism (consistent with traces)

`apply_write` saves into the in-memory buffer, drains the dirty set, then awaits the IDB transaction. On failure the dirty IDs are not restored and `PersistentFactStorage` has already published the state and cleared `dirty`. The next save appends its own fact pages after the failed write's fact pages, rewrites every index page and the header, and flushes only its own dirty pages. The new header references the failed write's fact pages, which IndexedDB never received:

- If those slots held older index pages (a 1-page failure), the header's index checksum no longer matches, and load silently rebuilds from fact pages, skipping the non-fact page.
- If the failed append reaches past the pages IndexedDB already holds, the slots do not exist, and load fails with `INT-052`. In the 400-fact cases, IndexedDB held pages 0–5 from A, the failed write's fact pages were 2–8 and the successor's fact page was 9, so pages 6–8 are missing.

## Not run

- v3 transaction, smaller-import deletion and page-ID-reuse cases. They need a separately pinned v3 build under #894, and these findings are not transferred to v3.
- Other browsers (Firefox, WebKit) and non-headless profiles.
- Recovery of a database that fails with `INT-052` from a fresh page. The old handle's `exportGraph` succeeded, but re-importing that export was not tested.
- No native or FinBench impact assumed: native file/WAL paths are not exercised here.
