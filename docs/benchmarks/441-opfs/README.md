# BrowserDb OPFS bounded-memory evidence (#441)

The measurement is **not completed in this environment**. The browser runner
cannot bind its localhost server (`Operation not permitted`, `EPERM`); an
independent Node `listen(0, '127.0.0.1')` probe also fails with `EPERM`.
No open-time, page-read or memory numbers are claimed. AC13(c)'s required
4 MiB/40 MiB comparison remains unverified.

## Reproduction

Run on the designated benchmark host, with Playwright 1.55.0 and its chosen
engine installed. Build `@minigraf/browser` from the companion wasm repository
using `wasm-pack build --target web --features browser` (release profile,
Cargo default `opt-level = 3` in the packaging shim; record any wasm-opt setting and version).
The offline comparison in this session explicitly disabled wasm-opt for both
revisions; memory measurement should identify the package used in its output.

```bash
bash docs/benchmarks/441-opfs/build-fixtures.sh /tmp/opfs-fixtures
cd docs/benchmarks/441-opfs
npm install --no-save --package-lock=false playwright@1.55.0
npx playwright install chromium firefox webkit
node measure.mjs /path/to/minigraf-wasm/pkg /tmp/opfs-fixtures chromium > chromium.json
node measure.mjs /path/to/minigraf-wasm/pkg /tmp/opfs-fixtures firefox > firefox.json
node measure.mjs /path/to/minigraf-wasm/pkg /tmp/opfs-fixtures webkit > webkit.json
```

`fixture.rs` builds and verifies two native v8 files of at least 4 MiB and
40 MiB with the same schema. Its data generation and fixture import occur
outside the measured wasm instance. Each mode/size is seeded in a worker which
is destroyed, then opened in another worker with a fresh wasm instance. The
query is always `(query [:find ?a :where [:probe :answer ?a]])`, whose single short
answer must be `42`.

`measure-worker.js` wraps `FileSystemSyncAccessHandle.prototype.read` before open
and records open milliseconds, read calls before and after the query, bytes read
expressed in 4 KiB pages, and `memory.buffer.byteLength` after the query.
IndexedDB's page count is recorded from the `getAll()` request's result length;
the output separately labels total fixture pages as `indexedDbImagePages`
(including free pages). Read-call counts specifically refer to OPFS.

WebAssembly memory size is an allocated **high-water mark**, not live memory.
The script fails if OPFS's larger-file high-water mark exceeds the smaller-file
mark by more than 4 MiB. Results are needed before claiming that bound.

The storage bound excludes query working state, results and export blobs,
import input, v7 migration, a transaction's facts, pages and quarantine metadata
held for long-open cursors, and recovery scans. Browser-side IndexedDB memory,
OS cache state and cold-start latency are not measured.

## Browser execution record

The preceding implementation session reported 101 Firefox worker and 81
Firefox main-thread tests passing. Those are inherited results from the handoff,
not fresh runs here. This session compiled the worker tests but could not start
the local test server. The built package's new cross-worker, termination and
import-interruption harnesses have syntax checks only; no engine pass is claimed.
CI now specifies Chrome/Firefox core worker lanes and Chromium/Firefox/WebKit
package-consumer lanes.

WebKit limitation here: no Playwright WebKit binary is installed, and the
sandbox's localhost bind restriction blocks the package harness before launch.
A reproducible WebKit command is the consumer harness with `BROWSER=webkit`;
its result here is **not run**, not passed. A manual result remains outstanding.

Private browsing: Chromium, Firefox and WebKit checks are **not run** here.
There is no evidence of how these engines expose OPFS in private windows in this
session. The same consumer page can be served on localhost and opened in a real
private window to record the actual `openOpfs` outcome; automated isolated browser
contexts are not represented as proof of UI private-browsing behavior.
