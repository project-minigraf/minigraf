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
- **quota** (realistic, no wrapper fault): `Storage.overrideQuotaForOrigin` via CDP, set to 100,000 bytes before the origin first uses storage (Chromium ignores a later override). The over-quota transaction ends with only `abort` (`QuotaExceededError`) and no request error.
- **sync-throw** (synthetic, no production trigger established): the second `put()` call throws a `DOMException` synchronously.

A run counts as consistent only if all of these hold after reopen: it opens, every acknowledged write is present, IndexedDB has every page below the header's `page_count`, the stored pages equal the last in-memory view (`exportGraph`), and a later write, checkpoint, reopen and index-path queries (by value and by entity) all succeed.

## Result matrix

The same results came out on all three builds (`main`, `v2.0.11`, `published`), 14 cases each. `matrix.txt` has the raw output and `results.json.gz` the traces, steps and page inspections.

| Case | API result of the faulted operation | After reopen | Conclusion |
|---|---|---|---|
| baseline (2 writes incl. multi-page, checkpoint) | all resolve | exact | control passes |
| overlap_control (2 concurrent writes) | both resolve | exact | control passes |
| fail_then_checkpoint | write **rejects**, but the fact stays visible in memory. `checkpoint()` **resolves without opening an IDB transaction** | opens with pre-failure state, so the handle's view and storage disagree | confirmed: rejected write stays visible; checkpoint reports success without retrying |
| fail_then_write (1-page failure) | rejects; successor resolves | opens; acknowledged data present; failed write's fact page slot holds a **stale old index page**; header `node_count` counts the failed fact. Load takes the checksum-mismatch rebuild path silently | confirmed divergence, no acknowledged loss |
| repeated_fail_then_write | 2× reject; successor + checkpoint resolve | as above, 2 stale slots | same |
| **fail_big_then_write** (400-fact failure) | rejects; **successor resolves** | **`BrowserDb.open` fails: `[INT-052] Page 6 not found`**. Every acknowledged write is unreachable | **confirmed acknowledged-data loss** |
| overlap (1-page) | predecessor rejects; overlapping successor resolves | as fail_then_write | successor delta captured before the failure, omits predecessor pages |
| **overlap_big** (400-fact) | predecessor rejects; overlapping successor resolves | **`INT-052`, database does not open** | **confirmed loss**; no handle-level workaround (delta captured before failure) |
| abort_only (synthetic) | **promise still pending after 5 s**; a later write resolves | opens; stale slot as fail_then_write | confirmed: an abort with no request error never settles the promise |
| **quota** (realistic `QuotaExceededError`) | **pending after 5 s**; a later write resolves | **`INT-052`, database does not open** | **confirmed loss from a realistic trigger** |
| sync_throw (synthetic) | rejects | the transaction **commits the put queued before the throw** (one page changed in IDB under the old header); opens via rebuild | confirmed partial commit despite API failure. Synthetic trigger only |
| import_fail | `importGraph` rejects; memory now holds the imported graph; checkpoint resolves without a write; next write resolves | reopens as **old data + successor write** (the imported graph is gone, the old fact `A` is back), while the header counts the imported facts | confirmed: a rejected import leaves the handle on different data than storage, and later writes are acknowledged against it |
| workaround_fresh_handle | after a rejection, open a fresh handle before writing again | exact | **workaround works for rejected writes** |
| quota_workaround | time out the pending write (5 s), open a fresh handle, write | exact | **workaround works for the hang if the app bounds it with a timeout** |

### Mechanism (consistent with traces)

`apply_write` saves into the in-memory buffer, drains the dirty set, then awaits the IDB transaction. On failure the dirty IDs are not restored and `PersistentFactStorage` has already published the state and cleared `dirty`. The next save appends its own fact pages after the failed write's fact pages, rewrites every index page and the header, and flushes only its own dirty pages. The new header references the failed write's fact pages, which IndexedDB never received:

- If those slots held older index pages (a 1-page failure), the header's index checksum no longer matches, and load silently rebuilds from fact pages, skipping the non-fact page.
- If the failed write had more fact pages than the old page count (400 facts → fact pages 2–9 against an old `page_count` of 6), the slots do not exist, and load fails with `INT-052`.

## Not run

- v3 transaction, smaller-import deletion and page-ID-reuse cases. They need a separately pinned v3 build under #894, and these findings are not transferred to v3.
- Other browsers (Firefox, WebKit) and non-headless profiles.
- Recovery of a database that fails with `INT-052` (the pages of acknowledged writes are still in IndexedDB, but no v2 API reads them).
- No native or FinBench impact assumed: native file/WAL paths are not exercised here.
