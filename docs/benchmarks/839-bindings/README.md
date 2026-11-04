# Binding release evidence (#839)

Status: work in progress; binding and release-maintainer acceptance is pending.
This document is the execution and coverage plan, not a release sign-off.

## Approved scope

The 2026-11-04 decision limits **new #839 timings** to AWS Device Farm for
Android and Swift/iOS, and the dedicated Linux bench for JVM, WASI and any
additional host/browser cases. macOS timing is explicitly excluded because no
stable platform is available. GitHub Actions may build artifacts and check
correctness. The existing nightly benchmarks on GitHub Actions remain unchanged.

These harnesses assess portability and reliability without changing Minigraf's
runtime, dependencies, configuration or file format. They align with
`PHILOSOPHY.md`'s embedded, cross-platform and reliability principles.

Initial source builds use core commit
`33e5dffadebe0124e6287fc91cede4ff2b7a2b2f`. Each artifact also records its binding
revision, harness hash, native artifact hash and toolchain. This is source-build
evidence; it does not certify published packages.

## Coverage matrix

The six common cases are prepared execution, cursor drains in batches of 1 and
256, a two-element set round trip, a foreign predicate and a foreign aggregate.
Fixtures contain 1,000 scalar facts and a separate set fact. See
[the existing shortlist](../../BINDING_BENCHMARKS.md) for measured boundaries.

| Runtime | Existing coverage | #839 work and current status |
| --- | --- | --- |
| Python | Six P16 nightly cases | Audit omitted options/errors, preparation/registration and typed fact-log/LogWriter costs; disposition pending |
| Browser WASM | Six P16 cases plus IndexedDB write/reopen | Preparation/registration and pinned-reader churn disposition pending; raw LogWriter and FactLog are not exposed by the audited browser API |
| Node.js | Six P16 nightly cases | Audit omitted options/errors, preparation/registration and typed fact-log/LogWriter costs; disposition pending |
| C | Six P16 nightly cases | Audit omitted options/errors, preparation/registration and JSON fact-log/LogWriter costs; disposition pending |
| JVM | No P16 timing | Fourteen cases, smoke and three complete bench repetitions collected; review and acceptance pending |
| Android | No P16 timing | Fourteen cases; physical-device smoke and three complete timed repetitions collected; acceptance pending |
| Swift/iOS | No P16 timing | Fourteen-case harness and build/correctness pipeline under validation; physical-device runs pending |
| WASI | Browser WASM is not WASI evidence | Three actual REPL cases, smoke and three complete bench repetitions collected; review and acceptance pending |
| Swift/macOS | No qualifying stable platform | Excluded by explicit user decision; no macOS performance claim |

The existing **26 P16 metrics** are already covered by core PRs #832/#833 and
binding PRs Python #17, browser #21, Node #21 and C #14. They are not rerun or
reclassified as #839 fixed-platform evidence.

JVM, Android and Swift/iOS add eight cases to the common six: query preparation,
first predicate registration, first aggregate registration, typed fact-log drain
in batches of 256, rejected negative cache option, read-only opening with options,
file-backed transaction, and LogWriter append plus finish. Preparation and
registration are measured on their own; correctness queries run afterwards.
Storage results are reopened and checked. LogWriter checks every returned record,
including temporal metadata, against the source.

The WASI package exposes a REPL, not foreign prepared-query objects or callback
registration. Its cases are empty-input startup, startup plus a fixed
transaction/query script, and a query round trip through a pre-seeded persistent
process. Startup cases include compilation/instantiation with Wasmtime's cache
and parallel compilation disabled. Persistent round trips include Python pipe
and output-framing overhead. No subtraction is presented as engine latency.

## Execution and qualification

1. Retain the initial Android failure and corrected smoke as harness evidence.
   Run three timed repetitions total using identical APKs, one selected device
   model/OS and a ten-minute job limit. Keep all samples. Public Device Farm
   selection does not reserve one physical unit; report that limitation.
2. Build JVM and WASI on the bench, then stop builds before timing. Run serially
   with CPU and memory affinity on one NUMA node and idle checks before and after
   each run. Keep private machine identity and paths out of published evidence.
3. Run three fresh JVM processes with a fixed 512 MiB heap and G1, 100 memory
   warmups or five storage warmups, then ten samples of 20 memory operations or
   one storage operation. Report variation rather than assuming JIT stationarity.
4. Run three WASI series with three warmups and ten samples per case. Startup and
   script samples are single operations; persistent queries average 20 operations.
5. Build Swift and Rust in Release configuration on GitHub. Run only oracle tests
   and the smoke on an iOS simulator. Upload the resulting device IPA/XCTest
   packages to Device Farm, validate the uploads, and pass one physical smoke
   before three timed repetitions. Investigate failures before retrying.
6. Complete the omitted-API matrix with measurements or explicit, rationale-backed
   dispositions for binding and release-maintainer review. Existing engine
   benchmarks do not establish foreign conversion latency. No RSS bound is claimed.
7. Publish reviewed harnesses and sanitized evidence with exact revisions,
   conditions, every sample, per-run medians/ranges and validation results.
   Keep runtime histories separate from each other and from native Criterion.
8. Retain evidence before removing remote scratch; retain caches only while reused
   by a subsequent build. Do not delete Device Farm projects or runs as cleanup.

Every timed invocation must return its expected result: counts, sums and exact
rows/sets are checked, and errors fail the run. Setup and validation are outside
the measurement except for explicitly named costs such as decoding, process
startup and script seeding. An empty response is not success. Negative oracle
tests exercise empty output and wrong values with otherwise correct count/sum.

There is no new latency regression threshold in this work. High variance remains
visible and limits repeatability claims. A complete run is coverage evidence,
not evidence that every latency is stable or acceptable for a release.

## Completion criteria

- Publish JVM, Android, Swift/iOS and WASI evidence with exact conditions and
  repeatability limitations, then obtain explicit binding and release sign-off.
- Cover the six common operations wherever the runtime API supports them;
  document WASI's actual contract and absent APIs.
- Measure omitted API/lifecycle costs or obtain explicit acceptance of each
  documented exclusion and its rationale.
- Verify actual results, separate setup, and label source versus package builds.
- Retain raw evidence, keep runtime histories separate, and make no unmeasured
  RSS or fixed-physical-unit claim.

Neither permission to run the work nor a successful CI job supplies maintainer
sign-off. #839 remains open until these criteria are satisfied.

See [the initial results](results.md) for the nine retained JVM/Android/WASI runs,
and [the proposed remaining dispositions](dispositions.md) for the exact limits
requiring maintainer decisions. Swift/iOS is still in build/device validation.
