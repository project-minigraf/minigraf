# Manual v7→v8 migration sweep (#838)

Measured **2026-11-04**, with **810 checked timing samples**: two native sessions
of 225 samples each and 120 samples each in Chromium, Firefox and WebKit.
Native covers nine fixtures through 200,000 facts; browsers cover six fixtures
at 2,000 and 20,000 facts. The browser cap follows the maintainer's explicit scope
decision after an untimed 200k-fact Chromium exploration took minutes per open.

At 200k facts, native migration medians were 575 ms (integer), 975 ms (repeated
256-byte values), and 1,597 ms (unique 256-byte values). A second complete idle run
gave 567, 929, and 1,382 ms respectively. Browser migration medians at 20k facts
span 262–770 ms across engines and shapes. These results characterize this
fixture ladder and run environment; they are not release thresholds.

This completes the bounded manual measurements requested in
[#838](https://github.com/project-minigraf/minigraf/issues/838).
[X03](../../bench-coverage-v3.md#remaining-exclusions-and-follow-ups) stays MANUAL.
No production code, nightly benchmark route, C07 golden corpus, or Rust test
inventory changes. The remaining exclusions below remain unmeasured.

## Fixtures and method

The [protocol](PROTOCOL.md) defines every expected fact, the independent full-row
validator, timing boundaries, and commands. Each entity has one integer id and
one payload. Payload shapes are integer, repeated 256-byte strings (16 distinct
values), and unique 256-byte strings. The ladders increase both entities and
distinct payload attributes:

| Facts | Entities | Payload attributes | Total attributes | Transactions |
| ---: | ---: | ---: | ---: | ---: |
| 2,000 | 1,000 | 16 | 17 | 10 |
| 20,000 | 10,000 | 256 | 257 | 100 |
| 200,000 | 100,000 | 4,096 | 4,097 | 1,000 |

All fixtures have one final checkpoint and no pending WAL. The
[manifest](evidence/manifest.json) records exact byte counts, SHA-256 hashes,
writer, shape and counts. The [frozen archive](evidence/v7-fixtures.tar.xz)
contains those exact nine files and manifest (about 35 MiB compressed).
Archive SHA-256:
`d2bcbf6f2260aafe6569513fac44bac49ac88154ec383280380db91b38d99836`.
Generation uses real transaction timestamps; regenerating the recipe does not
reproduce these exact bytes.

Native fixture copy/fsync and browser seed-transaction completion are outside
timing. Every migration verifies all expected rows and then reopens and verifies
them again. Native additionally checks transaction counts and v8 integrity.
The read-only native control must remain byte-identical v7 and is **nonmigrating**.

Each browser fixture/sample uses a fresh context with three independently seeded
IndexedDB databases. WASM initialization, fixture fetch/hash, page load and browser
startup are untimed. Reopen is in the same page/context; the browser process is
reused. Browser profiles and native temporary files use ext4 via TMPDIR.
OS caches are not dropped. Copying, seeding and verification warm caches.

## Migration and reopen

All cells are **median [minimum, maximum] milliseconds**, five samples per cell.
The native repeat is a separate session, not pooled with the primary run.

| Facts / payload | Native migration | Native repeat migration | Read-only v7 (primary) | Reopen v8 (primary) |
| --- | ---: | ---: | ---: | ---: |
| 2,000 / integer | 3.63 [3.55, 4.35] | 3.66 [3.56, 4.37] | 2.14 [2.06, 4.56] | 0.24 [0.24, 0.27] |
| 2,000 / repeated256 | 4.84 [4.70, 4.95] | 4.94 [4.81, 5.03] | 2.50 [2.37, 2.97] | 0.50 [0.50, 0.52] |
| 2,000 / unique256 | 6.17 [6.09, 6.55] | 6.73 [6.13, 7.09] | 2.41 [2.35, 2.49] | 0.51 [0.50, 0.52] |
| 20,000 / integer | 33.37 [32.32, 45.26] | 34.62 [33.82, 37.16] | 26.45 [25.16, 29.93] | 2.46 [2.38, 2.91] |
| 20,000 / repeated256 | 48.86 [46.54, 49.53] | 48.64 [46.61, 49.60] | 37.57 [36.37, 38.69] | 5.60 [5.57, 5.65] |
| 20,000 / unique256 | 68.56 [68.13, 69.43] | 68.27 [67.89, 70.25] | 36.17 [33.18, 36.70] | 5.80 [5.72, 5.94] |
| 200,000 / integer | 574.89 [554.74, 579.27] | 567.17 [551.66, 601.92] | 425.58 [403.40, 463.86] | 26.27 [26.21, 26.63] |
| 200,000 / repeated256 | 974.55 [758.72, 1078.24] | 929.05 [715.34, 1028.18] | 554.83 [533.06, 574.87] | 57.28 [56.96, 58.02] |
| 200,000 / unique256 | 1597.08 [989.43, 4328.44] | 1382.00 [1179.98, 1505.95] | 591.16 [524.07, 592.67] | 57.07 [56.32, 58.03] |

The largest unique-value migration was noisy: the first session ranged from
989 to 4,328 ms, and the repeat from 1,180 to 1,506 ms. Both sessions are retained.
Idle checks before/after every suite showed 100% idle in the recorded one-second
intervals; they do not identify the cause of within-run variance. Five samples
cannot establish tail latency or attribute that variance to a particular subsystem.

| Facts / payload | Chromium migration | Firefox migration | WebKit migration |
| --- | ---: | ---: | ---: |
| 2,000 / integer | 80.30 [79.10, 87.90] | 79.00 [63.00, 79.00] | 44.00 [42.00, 56.00] |
| 2,000 / repeated256 | 130.70 [122.00, 145.30] | 105.00 [102.00, 114.00] | 79.00 [72.00, 80.00] |
| 2,000 / unique256 | 141.70 [115.60, 153.20] | 124.00 [113.00, 126.00] | 85.00 [82.00, 88.00] |
| 20,000 / integer | 396.70 [368.10, 420.70] | 451.00 [401.00, 467.00] | 262.00 [244.00, 283.00] |
| 20,000 / repeated256 | 587.50 [578.80, 591.10] | 645.00 [625.00, 730.00] | 413.00 [391.00, 426.00] |
| 20,000 / unique256 | 656.90 [631.00, 711.30] | 770.00 [738.00, 825.00] | 459.00 [450.00, 501.00] |

| Facts / payload | Chromium reopen | Firefox reopen | WebKit reopen |
| --- | ---: | ---: | ---: |
| 2,000 / integer | 8.10 [7.80, 8.20] | 14.00 [10.00, 16.00] | 4.00 [4.00, 5.00] |
| 2,000 / repeated256 | 12.20 [11.30, 12.60] | 12.00 [10.00, 14.00] | 6.00 [5.00, 8.00] |
| 2,000 / unique256 | 19.50 [18.90, 20.60] | 17.00 [13.00, 19.00] | 9.00 [8.00, 10.00] |
| 20,000 / integer | 41.00 [39.20, 47.00] | 37.00 [33.00, 47.00] | 22.00 [22.00, 26.00] |
| 20,000 / repeated256 | 66.70 [58.90, 67.70] | 56.00 [46.00, 57.00] | 36.00 [31.00, 43.00] |
| 20,000 / unique256 | 99.60 [97.60, 113.90] | 81.00 [79.00, 81.00] | 65.00 [61.00, 70.00] |

These browser reopens do not measure cold startup, persistent-profile reopening,
or a browser read-only v7 contract. BrowserDb exposes no read-only v7 open.
Native and browser builds use different optimization profiles and persistence
contracts; their timings must not be interpreted as an equal-durability race.

## Scaling

Ratios below divide migration medians at adjacent rungs. Each step increases
facts tenfold **and** payload-attribute cardinality sixteenfold. These are
observations of the combined ladder, not isolated cardinality costs or an
asymptotic complexity claim.

| Session | Payload | 2k→20k | 20k→200k |
| --- | --- | ---: | ---: |
| native | integer | 9.20× | 17.23× |
| native | repeated256 | 10.09× | 19.95× |
| native | unique256 | 11.12× | 23.29× |
| native-repeat | integer | 9.45× | 16.38× |
| native-repeat | repeated256 | 9.85× | 19.10× |
| native-repeat | unique256 | 10.15× | 20.24× |
| chromium | integer | 4.94× | excluded |
| chromium | repeated256 | 4.50× | excluded |
| chromium | unique256 | 4.64× | excluded |
| firefox | integer | 5.71× | excluded |
| firefox | repeated256 | 6.14× | excluded |
| firefox | unique256 | 6.21× | excluded |
| webkit | integer | 5.95× | excluded |
| webkit | repeated256 | 5.23× | excluded |
| webkit | unique256 | 5.40× | excluded |

## Supported cursor lifetimes and persistence churn

Migration finishes during open, before a cursor can be obtained. Holding a cursor
from that handle across its own migration is not a supported scenario. This
sweep instead times **eight sequential one-fact commits after migration**, with
explicit native checkpoints or awaited browser IndexedDB persistence.

The pinned case keeps one original cursor across all eight commits and overlaps
each commit with one new short-lived cursor. Full drains check the original
snapshot and each later snapshot. After releasing all cursors, a ninth untimed
write and reopen check the current data. This exercises supported lifetimes; it
does not measure pin counts, quarantine bytes, or a memory/reclamation bound.

The following cells contain the **sum of eight commits**, not per-commit latency.
Cursor construction/drain and validation are untimed. Fixed operation order and
the pinned case's extra cursor reads can change cache/JIT state, so a ratio below
one is not evidence that pinning intrinsically improves write speed.

| Session | Facts / payload | Unpinned ms | Pinned ms | Pinned / unpinned median |
| --- | --- | ---: | ---: | ---: |
| native | 2,000 / integer | 7.06 [6.77, 7.70] | 7.15 [7.09, 7.55] | 1.01× |
| native | 2,000 / repeated256 | 6.76 [6.73, 7.09] | 7.04 [6.84, 7.45] | 1.04× |
| native | 2,000 / unique256 | 6.97 [6.90, 7.08] | 7.17 [7.14, 7.32] | 1.03× |
| native | 20,000 / integer | 8.01 [7.36, 8.56] | 9.22 [8.83, 9.40] | 1.15× |
| native | 20,000 / repeated256 | 8.08 [7.75, 9.01] | 9.35 [8.97, 9.87] | 1.16× |
| native | 20,000 / unique256 | 8.68 [8.34, 9.16] | 9.73 [9.55, 10.00] | 1.12× |
| native | 200,000 / integer | 10.14 [9.96, 10.36] | 10.60 [10.46, 10.69] | 1.04× |
| native | 200,000 / repeated256 | 10.46 [10.00, 10.67] | 10.86 [10.66, 11.33] | 1.04× |
| native | 200,000 / unique256 | 9.93 [9.58, 10.05] | 10.66 [10.18, 10.99] | 1.07× |
| native-repeat | 2,000 / integer | 7.07 [6.54, 7.31] | 7.06 [6.85, 7.49] | 1.00× |
| native-repeat | 2,000 / repeated256 | 6.96 [6.80, 7.08] | 6.91 [6.84, 7.38] | 0.99× |
| native-repeat | 2,000 / unique256 | 6.94 [6.92, 7.53] | 7.20 [7.06, 7.92] | 1.04× |
| native-repeat | 20,000 / integer | 7.98 [7.95, 8.25] | 9.23 [8.93, 9.52] | 1.16× |
| native-repeat | 20,000 / repeated256 | 8.24 [8.19, 8.46] | 9.45 [9.26, 10.02] | 1.15× |
| native-repeat | 20,000 / unique256 | 8.67 [8.34, 9.47] | 9.88 [9.46, 10.12] | 1.14× |
| native-repeat | 200,000 / integer | 10.20 [9.82, 10.32] | 10.82 [10.63, 11.33] | 1.06× |
| native-repeat | 200,000 / repeated256 | 10.09 [9.74, 10.31] | 11.00 [10.16, 11.13] | 1.09× |
| native-repeat | 200,000 / unique256 | 10.00 [9.30, 10.18] | 10.81 [10.27, 11.28] | 1.08× |
| chromium | 2,000 / integer | 41.00 [39.90, 44.00] | 26.70 [26.60, 30.00] | 0.65× |
| chromium | 2,000 / repeated256 | 38.50 [36.90, 42.00] | 24.40 [21.60, 26.40] | 0.63× |
| chromium | 2,000 / unique256 | 45.60 [41.90, 52.80] | 28.80 [25.60, 29.30] | 0.63× |
| chromium | 20,000 / integer | 38.70 [37.50, 43.50] | 23.50 [22.30, 25.60] | 0.61× |
| chromium | 20,000 / repeated256 | 41.10 [31.00, 41.90] | 25.50 [23.70, 26.10] | 0.62× |
| chromium | 20,000 / unique256 | 40.20 [38.10, 43.20] | 25.70 [24.50, 26.80] | 0.64× |
| firefox | 2,000 / integer | 38.00 [27.00, 42.00] | 31.00 [29.00, 34.00] | 0.82× |
| firefox | 2,000 / repeated256 | 33.00 [32.00, 36.00] | 29.00 [25.00, 30.00] | 0.88× |
| firefox | 2,000 / unique256 | 38.00 [32.00, 40.00] | 30.00 [29.00, 32.00] | 0.79× |
| firefox | 20,000 / integer | 48.00 [42.00, 61.00] | 33.00 [27.00, 37.00] | 0.69× |
| firefox | 20,000 / repeated256 | 58.00 [55.00, 61.00] | 37.00 [35.00, 39.00] | 0.64× |
| firefox | 20,000 / unique256 | 59.00 [58.00, 68.00] | 39.00 [33.00, 41.00] | 0.66× |
| webkit | 2,000 / integer | 23.00 [21.00, 25.00] | 17.00 [15.00, 17.00] | 0.74× |
| webkit | 2,000 / repeated256 | 24.00 [24.00, 26.00] | 18.00 [16.00, 19.00] | 0.75× |
| webkit | 2,000 / unique256 | 26.00 [25.00, 27.00] | 20.00 [18.00, 27.00] | 0.77× |
| webkit | 20,000 / integer | 30.00 [28.00, 31.00] | 23.00 [21.00, 27.00] | 0.77× |
| webkit | 20,000 / repeated256 | 33.00 [32.00, 34.00] | 27.00 [23.00, 34.00] | 0.82× |
| webkit | 20,000 / unique256 | 37.00 [34.00, 40.00] | 26.00 [23.00, 30.00] | 0.70× |

## Provenance, validation and evidence

Core reader and browser build:
[`e073793`](https://github.com/project-minigraf/minigraf/commit/e07379323bca4c2c248c25c2e90707a8d8d9f134).
The writer is crates.io Minigraf 2.0.11. Browser bindings are generated from this
core checkout's wasm-bindgen facade; no external binding repository or published
npm Minigraf artifact is measured. Harness files in this report's commit match
the source SHA-256 values in [run.json](evidence/run.json).

Environment: Ubuntu 26.04.1 Linux x86_64, ext4, CPU/memory affinity NUMA node 1.
All timing suites ran sequentially on idle hardware with no competing build or
test workload. Host identity and hardware inventory are intentionally omitted.
Native release uses opt-level 3, LTO and one codegen unit; core browser release
uses opt-level z, LTO, one codegen unit and stripping. Rust 1.99.1 / LLVM 23.1.1,
wasm-pack 0.13.1, Node 22.23.4, Playwright 1.64.1. Engines: Chromium 156.0.8078.4,
Firefox 157.0, WebKit 27.2.

Native uses monotonic Instant; browsers use performance.now. Browser values are
converted from milliseconds to nanoseconds in raw JSON; this does not imply
nanosecond precision (Firefox/WebKit samples here are millisecond-quantized).
The tables round to two decimals for consistent display.

- [Run metadata](evidence/run.json): source/build hashes, versions, sample counts,
  UTC session windows, sanitized idle observations and exclusions.
- [All summaries as CSV](evidence/summary.csv); raw
  [native](evidence/native.jsonl), [native repeat](evidence/native-repeat.jsonl),
  [Chromium](evidence/chromium.json), [Firefox](evidence/firefox.json),
  [WebKit](evidence/webkit.json). JSON summaries alongside them contain unrounded
  median/min/max for every operation, including repeat read-only/reopen results.
- [Baseline log](evidence/baseline.log): golden corpus **2 passed** and migration
  matrix **7 passed**. Separate untimed checks: native **45**, each browser **24**.
  Every one of the **810** timed samples also passed the recipe/snapshot/reopen
  checks. Native/browser check records and successful build logs are in evidence.
- [Evidence-checker tests](evidence/evidence-tests.log): **5 passed**.
  [Corrupted fixture](evidence/corrupt-fixture-check.json) rejected before any
  successful sample; [wrong browser scope](evidence/scope-rejection-check.json)
  rejected. [Archive verification](evidence/fixture-validation.json) checked all
  nine decompressed hashes and the embedded manifest.
- Exact [native](evidence/native-dependencies.lock) and
  [core](evidence/core-dependencies.lock) dependency locks, plus the harness
  [npm lock](../../../scripts/migration-sweep/package-lock.json), are retained.
  [npm audit](evidence/npm-audit.json) reported zero known vulnerabilities at run
  time. Playwright 1.64.1 is above the 1.55.1 fix for
  [CVE-2025-59288](https://github.com/advisories/GHSA-7mvr-c777-76hp);
  Node 22.23.4 includes the
  [October 2026 security fixes](https://nodejs.org/en/blog/vulnerability/october-2026-security-releases).
- [SHA-256 list](evidence/checksums.sha256) covers the evidence files. Logs replace
  the private scratch path with `<scratch>`; raw measurements are unchanged.

## Large browser observation and exclusions

The [exploratory Chromium log](evidence/chromium-200k-exploratory.log) records an
untimed 200k-fact integer fixture completing migration/data/reopen checks and
both churn modes, then starting the repeated-value case before the run was
stopped. Opens were observed to take minutes. This was `--test` mode: no
numeric migration latency was collected, and no complete five-sample matrix
exists for that rung. Its initial exploratory profile placement also differs
from the final explicitly disk-backed suites. It is an observation motivating
the scope cap, not a comparable timing result.

All three browsers have measured 2k/20k coverage and exclude 200k. Native measures
all nine fixtures. Also excluded: more than 200k facts, independent attribute
cardinality sweeps, values over 256 bytes, temporal/history and multi-checkpoint
ladders, pending/stale v7 WALs, crash/power-loss migration, OS cold-cache opens,
persistent-profile/cold-start browser migration, real Safari/mobile hardware,
concurrent tabs/independent writers, cross-context snapshots, and sustained
many-pin/RSS or quarantine-byte scaling. Browser lifecycle breadth remains X04;
sustained pin scaling remains X06. Nothing here expands nightly coverage.

The [protocol](PROTOCOL.md) gives replay commands and cleanup requirements.
