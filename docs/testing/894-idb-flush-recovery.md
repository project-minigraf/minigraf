# IndexedDB flush recovery — PR 1 evidence (#894)

Scope: `fix/894-idb-flush-recovery`, based on `df7e8170d4cf3bedcdbf91e85d81f7b7ea1ca959`
(`v3`). The [approved design](../superpowers/specs/2026-11-17-idb-flush-recovery-design.md)
is copied verbatim. No v2 backport or docs-site changes are included.

## Regression evidence (AC19)

Before any production change, the new wasm tests ran against the base production
code in headless Firefox. Each isolated run exited 1:

- `multipage_failed_write_refuses_successors`: the 400-fact write failed the
  STG-048 assertion, returning the raw `QuotaExceededError: injected queue failure`.
  Run result: 0 passed, 1 failed, 90 filtered out, 0.07 seconds.
- `adapter_abort_only_settles`: the real transaction aborted after its last
  request succeeded, with no request error. The promise remained pending and the
  test's 1,500 ms deadline rejected it (`deadline: flush did not settle`).
  Run result: 0 passed, 1 failed, 90 filtered out, 1.53 seconds.
- An initial combined run also failed the empty-checkpoint boundary assertion;
  it subsequently timed out in a failure-refusal test on the old implementation.
  The two required cases above were therefore run independently.

A second control run used an archived base with production files unchanged and
Rust 1.99.1. The strengthened multi-page test failed **the unchanged-durable-bytes
oracle**, and the adapter queue-throw test failed its unchanged-map assertion;
the abort-only case again reached its deadline. Result: **0 passed, 3 failed,
81 filtered out**, 1.63 seconds (`target/verification/red-integrity.log`).
This independently demonstrates partial persistence, not just the old error text.

The original red runs used Rust 1.94.0. Subsequent verification uses installed Rust 1.99.1,
`CARGO_BUILD_JOBS=4`, one build at a time, and debug information disabled for dev
and test profiles. Caches, build output, browser temporary profiles and logs live
under this worktree's ignored `target/verification/`. The browser test server
requires execution outside the network-restricted sandbox.

## Test map

All 13 new tests are `#[wasm_bindgen_test]` in `src/browser/flush_tests.rs` and
participate in the existing window and dedicated-worker browser lanes. Faults
wrap real IndexedDB connections/transactions from test code only. No fault
switch is compiled into production.

| Acceptance criteria | Tests / assertions |
|---|---|
| AC1–AC3, AC12, AC16 | Adapter abort-only deadline and usable connection; repeated real request errors with a page-error observer; second-put queue throw after a queued delete and put, with the entire key/bytes map unchanged. STG-048 names IndexedDB write and the DOM exception. |
| AC4–AC5, AC13–AC15 | Multi-page execute; transaction commit; checkpoint with its own dirty page; smaller (empty) import. Shared refusal checks transact, retract, repeated checkpoint/import, empty and nonempty commit, unchanged counters/facts/export, finished transactions, INT-001 precedence, reads, cursor, prepared query and rules. Closed connection test now keeps four names, excluding Erin. |
| AC6–AC7, AC14 | Delayed abort across execute→execute, commit→execute, execute→import, import→execute and execute→checkpoint: one transaction submitted, expected memory contents, successor STG-046, unchanged durable map. Successful mixed write/checkpoint/import/commit/retract sequence and a checkpoint excluding a later delayed write. |
| AC3, AC14 | Missing dirty page yields INT-052 for execute, commit and checkpoint and fails the handle. A later preparation failure waits for the earlier success and preserves its durable prefix. |
| AC8 | Close behind a delayed submitted flush prevents the queued execute, commit, checkpoint or import from submitting (API-018); a fresh handle's write survives late completion. Existing submitted-flush and OPFS lifecycle tests remain in their suite. |
| AC9–AC10 | Quiescent close/reopen checks exact logical content and the unchanged durable page map after failures. Reopened exports pass `image_of` and `PersistentFactStorage::verify`, the same image decoder and integrity walker used by native open/verify, compiled for the browser; this covers reachable pages while allowing absent free-page bytes. A failed v7 migration leaves original bytes unchanged and a fresh open migrates successfully. |
| AC21 | Healthy commits retain no settled barrier history; no schema/version, file format or public-method change. Only the existing web-sys dependency's `DomException` bindings are enabled. No whole-store rewrite added to commits. |

## Verification

- `cargo build`: passed.
- `cargo test`: **2,149 passed, 23 ignored, 0 failed**, across 87 suite summaries.
- `cargo clippy --target wasm32-unknown-unknown --features browser -- -D warnings`: passed.
- Native strict Clippy initially found `clippy::for_unbounded_range` in the
  pre-existing `tests/crash_kill_test.rs:543`; the archived base revision fails
  with exactly the same diagnostic. The counter now uses `1..=u64::MAX`, the
  suggested explicit bound, with no production-code change. The exact requested
  `cargo clippy --all-targets -- -D warnings` then passed. The affected crash-kill
  suite was rerun: **2 passed, 0 failed**. This one-line harness change is the
  only implementation change outside the browser/error/documentation scope.
- `cargo fmt --check`: passed; the spec copy also matches byte-for-byte.
- Distribution gate, `cargo build --profile dist`: **1,270,880 bytes**, versus
  **1,270,864 bytes** before the native error-message change (**+16 bytes**),
  below the unchanged **1,280,000-byte** limit by **9,120 bytes**. Both builds
  used Rust 1.99.1 and the same dependency lockfile.
- `wasm-pack test --headless --firefox --features browser` (explicit local
  `--geckodriver` path): **94 passed, 0 failed**, 1.83 seconds. Firefox 159.0.1,
  GeckoDriver 0.37.1.
- Dedicated-worker Firefox, `RUSTFLAGS='--cfg minigraf_browser_worker'` with
  the same command plus `--lib`: **127 passed, 0 failed**, 12.21 seconds,
  including the OPFS suite and submitted-flush close/reopen regression. The
  current worker inventory is 114 inherited tests plus 13 new tests (the older
  #441 record's 113 count describes its earlier snapshot).

No Chrome or Safari run, CDP quota run, or separate native process opening a
browser-produced export was performed here. The AC9 equivalent integrity route
is the shared native decoder/walker compiled into the browser test, as described
above. The deterministic browser tests pass in both local Firefox contexts.

## Files changed

| File | Change |
|---|---|
| `src/browser/indexeddb.rs` | Atomic queue rollback, owned listeners, idempotent abort/error/complete settlement and coded write errors. |
| `src/browser/mod.rs` | Ordered per-handle flushes, failure admission, checkpoint boundaries, import-swap-safe failure state and rustdoc. |
| `src/browser/flush_tests.rs` | Thirteen new browser regressions and test-only JavaScript fault wrappers. |
| `src/browser/tx_tests.rs` | Closed-connection first error STG-048, Erin's refused commit STG-046 and four readable names. |
| `Cargo.toml` | Enable `DomException` on the existing optional web-sys dependency. |
| `src/error.rs` | STG-046 text includes IndexedDB flush failures; STG-048's existing format already names operation and exception. |
| `tests/crash_kill_test.rs` | Explicit maximum for the existing counter range to satisfy current strict Clippy. |
| `docs/ERROR_REFERENCE.md` | IndexedDB failed-handle and browser-write error semantics. |
| `docs/TEST_COVERAGE.md` | Browser inventory and regression coverage/evidence link. |
| `CLAUDE.md` | Browser architecture and separate browser test counts. |
| `CHANGELOG.md` | Unreleased v3.0.0 recovery entry. |
| `docs/superpowers/specs/2026-11-17-idb-flush-recovery-design.md` | Immutable approved spec copied verbatim. |
| `docs/testing/894-idb-flush-recovery.md` | This report. |

## Remaining external work

- AC20 is evaluator-run realistic Chromium quota evidence, outside this round's
  requested implementation scope. The deterministic tests here inject real
  transaction aborts and synchronous DOM exceptions, not CDP quota exhaustion.
- PR 2 (the next v2.x patch) and the docs-site PR remain separate.
- The user required writes only inside this worktree. No remote issue was edited.
  The corresponding prepared update for #421's #894 entry is:

  **Fix:** v3.0.0; backport planned for the next v2.x patch (PR 2 after PR 1 merges).
  Keep #894 open until the fix is published.
