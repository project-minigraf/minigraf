# BrowserDb OPFS bounded-memory evidence (#441)

Measured locally on 2026-11-14 with fresh wasm instances. **AC13(c) passes**:
OPFS linear-memory high-water is 1,245,184 bytes for both fixtures, a zero-byte
increase (limit: 4 MiB). These are single-run observations under concurrent
native-test load, not latency benchmarks or a power-loss durability claim.

## Build and fixtures

The built companion package uses Rust 1.94.0, release profile with the packaging
shim's default `opt-level = 3`, wasm-bindgen 0.2.126 / web-sys 0.3.103, and
wasm-opt disabled. It was built from the minigraf-wasm feature worktree with a
temporary local core path; no path pin is committed. Core code is identical to
`6fa1582f8` (later commits contain documentation and evidence only).

- Raw wasm: **1,812,953 bytes**; SHA-256
  `32ba454e40152bd017d5a994529d4294cbf1d43a67ee3b7608ecd7e8189dfbc1`.
- `4mib.graph`: **4,239,360 bytes**, 2,688 payload entities, 1,035 file pages.
- `40mib.graph`: **42,135,552 bytes**, 27,392 payload entities, 10,287 file pages.

`fixture.rs` builds and verifies native v8 files with the same schema. Generation
and seeding happen outside the measured wasm instance. Each mode/size is seeded
in a worker which is destroyed, then opened in another worker with a fresh wasm
instance. The fixed query is `(query [:find ?a :where [:probe :answer ?a]])`,
whose single short answer must be `42`.

## Results

“Open reads” and “pages incl. query” count OPFS read calls/pages; IndexedDB's page
count is the number of records returned by `getAll()`, excluding unused file
pages. IndexedDB does not call the OPFS read API.

| Engine | Mode | Fixture | Open ms | Open reads | Pages incl. query | Wasm high-water bytes |
|---|---|---|---:|---:|---:|---:|
| Chromium 140.0.7339.16 | OPFS | 4 MiB | 21.6 | 2 | 7 | 1,245,184 |
| Chromium 140.0.7339.16 | OPFS | 40 MiB | 26.3 | 2 | 10 | 1,245,184 |
| Chromium 140.0.7339.16 | IndexedDB | 4 MiB | 136.1 | n/a | 989 | 5,373,952 |
| Chromium 140.0.7339.16 | IndexedDB | 40 MiB | 1,381.6 | n/a | 10,071 | 42,991,616 |
| Firefox 159.0.1 | OPFS | 4 MiB | 30 | 2 | 7 | 1,245,184 |
| Firefox 159.0.1 | OPFS | 40 MiB | 26 | 2 | 10 | 1,245,184 |
| Firefox 159.0.1 | IndexedDB | 4 MiB | 96 | n/a | 989 | 5,373,952 |
| Firefox 159.0.1 | IndexedDB | 40 MiB | 1,474 | n/a | 10,071 | 42,991,616 |

Both engines pass the 4 MiB delta limit with **0 bytes** of OPFS growth.
WebAssembly memory size is an allocated **high-water mark**, not live memory.
The storage bound excludes query working state, results and export blobs,
import input, v7 migration, a transaction's facts, pages and quarantine metadata
held for long-open cursors, and recovery scans. Browser-side IndexedDB memory,
OS cache state and cold-start latency are not measured.

## Reproduction

Use an already installed Playwright 1.55.0 engine; installing browser binaries is
a separate setup step. Build the companion with
`wasm-pack build --target web --no-opt --features browser`.

```bash
bash docs/benchmarks/441-opfs/build-fixtures.sh /tmp/opfs-fixtures
cd docs/benchmarks/441-opfs
npm install --no-save --package-lock=false playwright@1.55.0
BUILD_LABEL='Rust 1.94.0; release opt-level=3; wasm-opt disabled' \
  node measure.mjs /path/to/minigraf-wasm/pkg /tmp/opfs-fixtures chromium > chromium.json
node measure.mjs /path/to/minigraf-wasm/pkg /tmp/opfs-fixtures firefox > firefox.json
```

`measure-worker.js` wraps `FileSystemSyncAccessHandle.prototype.read` before open
and records open milliseconds, read calls before and after the query, bytes read
expressed in 4 KiB pages, and `memory.buffer.byteLength` after the query. The
script fails if OPFS's larger-file high-water exceeds the smaller-file mark by
more than 4 MiB. The Chromium run used byte-identical copies of these scripts
under `/tmp` for Playwright module resolution.

Playwright Firefox is absent locally. The recorded Firefox run instead served
the **same measurement worker** to the installed Firefox via geckodriver. Its
fallback driver and raw logs are local review artifacts in
`TandemKit/005-browser-opfs/Evidence/` in the main checkout, outside version
control. This is a real Firefox run, not a Playwright Firefox pass.

Build-size comparisons and exact toolchains are in [build-sizes.md](build-sizes.md).

## Browser execution and private browsing

The execution checks below were rerun after the import admission/export fixes.
The memory results above remain from the identified measured artifact: the
healthy OPFS open/read/cache paths are unchanged.

- Firefox 159.0.1 core: **81 main-thread and 113 worker tests pass**.
- Packaged consumer: **Chromium 140.0.7339.16 via Playwright** and
  **Firefox 159.0.1 via WebDriver** each pass 54 checks, including six
  sequential-commit termination timings, four import-interruption timings,
  cross-worker locking, IndexedDB in a worker and the README worker example.
- Chrome core wasm-bindgen tests: not run. Cached ChromeDriver is version 156;
  installed Playwright Chromium is version 140 (Brave is 157). No matching
  Chrome/driver pair exists locally; no browser binaries were downloaded.
- Playwright Firefox launch: unavailable executable
  `~/.cache/ms-playwright/firefox-1490/firefox/firefox`. The WebDriver result
  above is the explicitly recorded fallback.
- WebKit: **not run**. Playwright 1.55.0 cannot find
  `~/.cache/ms-playwright/webkit_ubuntu20.04_x64_special-2092/pw_run.sh`.
  No WebKit binary is installed and no manual Safari result exists. Reproduce
  with an installed engine using `BROWSER=webkit node driver.mjs 0.0.0-ci`
  from an installed consumer app; the local attempt failed before any test.
- Chromium incognito: an actual Playwright `browser.newContext()` opened
  OPFS, acknowledged a write and queried `[[42]]`. This records the automated
  incognito context, not a separate manual browser-window run.
- Firefox private: geckodriver launched with
  `browser.privatebrowsing.autostart = true`; `openOpfs` rejected with
  `[API-026] OPFS mode is not available here: the browser denies access to this origin's storage (SecurityError)`.
- WebKit private: **not tested** (see below).

CI specifies Chrome/Firefox core main-thread and worker lanes and
Chromium/Firefox/WebKit package-consumer lanes; this local record does not
claim those remote CI jobs have run.

### WebKit coverage in CI

Playwright's Linux WebKit has no `navigator.storage.getDirectory()` in a
dedicated worker, so its consumer lane only checks that `openOpfs` rejects
with API-026 for that reason. WebKit OPFS is covered on `macos-15` instead:

- core `wasm.yml` job `safari`: the main-thread and dedicated-worker
  wasm-bindgen suites in real Safari through `safaridriver`;
- minigraf-wasm consumer lane `browser, macos-15, webkit`: the packaged
  consumer test on Playwright's macOS WebKit, where OPFS is required (an
  API-026 there fails the run).

**Omitted: Safari private browsing.** Neither WebDriver nor Playwright can
open a Safari private window, so no automated run checks OPFS there, and no
manual check was made. WebKit documents OPFS as unavailable in Private
Browsing windows ([WebKit blog, 2022](https://webkit.org/blog/12257/the-file-system-access-api-with-origin-private-file-system/),
"Persistence"), so `openOpfs` is expected to reject with API-026 there, as
in Firefox private; that expectation is unverified. Firefox private and Chromium
incognito results are above.
