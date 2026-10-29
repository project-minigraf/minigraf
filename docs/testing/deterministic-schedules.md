# Deterministic storage schedule tests (#760)

Investigation on `v3`, based on `9117392e9` (2026-10-29), for the v3.0.0
release gate in #383. The bounded prototype is adopted as a native CI suite.
It adds no database configuration and does not prove concurrency correct.

## Boundary audit

| Boundary | Current synchronization | What this prototype checks |
|---|---|---|
| Transaction append / snapshot | `FactStorage::load_facts` holds the pending write lock for the whole batch; `snapshot` reads its length under the same lock | A reader observes zero or two facts from a two-fact transaction, never one (#554) |
| Separate scans of one snapshot | `Snapshot` retains one pending store and cutoff | Reads of `:pair/x` and `:pair/y` agree even if another pair is appended between scans (#555-like failure) |
| Checkpoint publication / reader / append | `FactData` write lock publishes the committed reader, generation and replacement pending store together; appends hold its read lock | No mixed generation, duplicate or missing fact; a late append survives; an old reader and a concurrently captured reader stay stable |
| Snapshot lifetime / page reuse | `Pins` mutex maintains generation pins and quarantine; snapshot capture and publication hold `FactData` while taking/recording pins | A page remains quarantined while an old snapshot exists and is released after its last snapshot drops |
| Rule registration / query planning | `execute_rule` holds the registry write lock through registration and stratification; `open_rule_query` holds its read lock through rewriting/planning | Audited only; the registry is not instrumented by this prototype |

The facade serializes writes with `Inner::write_lock`. The storage-level late
append scenario also checks the explicitly supported `publish` tail-copy
contract, even though the facade currently prevents that overlap. Reopen has
no shared in-process snapshot and remains covered by recovery tests.

## Instrumented suite

`src/graph/storage/deterministic.rs` runs three small workloads against the
**real** `FactStorage`, `Snapshot` and `Pins` implementations. Only their locks
and transaction-counter atomic are substituted. The checkpoint reader is an
immutable list of facts: disk pages, WAL and the allocator are outside this
experiment. A yield in the batch's input iterator exposes the per-fact gap in
the negative control; in the correct code the batch lock spans that yield.

The substitutions require **all** of `cfg(test)`, `cfg(minigraf_shuttle)` and a
native target. Shuttle 0.9.8 is a conditional, native dev dependency. Ordinary
tests, production builds and WASM use `std`; no public API, file format or
production algorithm changes. `std::sync::Arc` remains the ownership primitive;
the instrumented locks schedule access to the shared contents. Other modules'
locks are not instrumented: do not extend these workloads to the full facade
without auditing them for blocking outside Shuttle.

Facts use fixed ids and timestamps. Although `snapshot()` captures the wall
clock, these tests do not use it in their assertions or scheduling decisions.
Hash iteration does not select operations: comparisons use ordered results.
Each search has a fixed seed and iteration budget, plus a 10,000-step failure
bound per execution. Passing a randomized search does not establish complete
schedule coverage, weak-memory correctness, fairness or absence of starvation.

## Reproduction

Use the designated test machine for these commands. Use the debug test profile;
the project's release profile aborts on panic.

```bash
RUSTFLAGS='--cfg minigraf_shuttle' cargo test --lib \
  graph::storage::deterministic:: -- --nocapture

MINIGRAF_DST_ITERATIONS=10000 MINIGRAF_DST_SEED=761 \
  RUSTFLAGS='--cfg minigraf_shuttle' cargo test --locked --lib \
  graph::storage::deterministic:: -- --nocapture
```

Default: 1,000 executions **per workload**, seed 760. Do not run the ordinary
unit suite with this cfg: those tests are not wrapped in a Shuttle runtime.
Retain the generated `Cargo.lock`, exact source and compiler version with a
failure; the root lockfile is intentionally untracked in this repository.
Schedule encodings are tied to the instrumented program and dependency versions.

Shuttle prints a `failing schedule`. Replay it with the exact test selected:

```bash
MINIGRAF_DST_REPLAY='<schedule string>' RUSTFLAGS='--cfg minigraf_shuttle' \
  cargo test --locked --lib \
  graph::storage::deterministic::batch_append_is_atomic -- --exact --nocapture
```

`tests/dst-mutations/*.patch` are three **deliberate faults**, never changes to
ship. In a disposable checkout, after the first command generates a lockfile:

```bash
python3 tests/dst-mutations/check.py > dst-controls.json
```

The script applies each patch separately, requires the expected assertion
failure, replays its saved schedule ten times, restores the original source
in `finally`, then rebuilds and runs all three correct workloads. Keep its
stderr if a check fails. This is a harness validation experiment, not part of
the normal test suite. Do not interrupt it with SIGKILL or edit its source file
concurrently; use a disposable checkout so a process kill cannot affect work.

## Alternatives and compatibility

| Technique | Fit and cost for Minigraf |
|---|---|
| Shuttle | The prototype instruments the two storage modules with a small import change. Randomized schedule search and saved-schedule replay suit these bounded lock-based invariants. It does not control unmodified `std` locks, external I/O or SIGKILL recovery. [Primary documentation](https://docs.rs/shuttle/0.9.8/shuttle/) |
| Loom | Requires synchronization substitutions too; offers memory-model exploration valuable for future lock-free/relaxed-atomic code. Full-engine models would add state-space and maintenance cost. Not prototyped or timed here. [Primary documentation](https://docs.rs/loom/0.7.2/loom/) |
| Explicit channels/barriers | Works with real OS threads, `std` locks and file I/O, without another dependency. Good for a specific known interleaving, but requires hooks placed at the dangerous internal boundaries and a deadlock-safe protocol. It does not automatically explore other schedules. No second implementation was built here. |
| ffs | Traces unmodified native threads, so it could cover more of the facade. Its documented Linux/x86-64, ptrace/perf-counter requirements and external-side-effect replay obligations make it a separate platform-specific experiment. Not run here. [Primary repository](https://github.com/aoli-al/ffs) |

WASM is excluded from the instrumented suite; its production imports remain
unchanged. The initial prototype was measured on Linux x86-64. The CI matrix
also runs macOS and Windows; its PR checks provide the native-platform evidence. File-backed real-OS concurrency tests, fault injection,
model-based reopen tests and the SIGKILL suite must remain. The immutable reader
cannot establish filesystem ordering, torn-write handling, durable commit or
correct page reclamation by the actual allocator. #558's page-overwrite defect
was v2.x-only and is not claimed to exist on v3.

## Measurements and recommendation

On the designated bench machine (Linux x86-64, Rust 1.99.1), all three deliberate
faults were detected with seed 760 and a budget of 1,000 executions. Each saved
schedule reproduced the intended assertion failure **10/10 times**:

| Deliberate mutation | Saved failing schedule |
|---|---|
| Per-fact locking in `load_facts` (#554-style) | `9101149fd8bda2a2a0cbf6e901a0288aa202` |
| Live pending length instead of the captured snapshot cutoff (#555-like) | `910115b9befafab08ae7c8f301000020282800` |
| Release/reacquire the publication lock between reader and pending replacement | `91022bf8050000000000001228499208112400400200` |

These were mutations of current v3 source, not runs of the original historical
commits. The ordinary stress tests can also catch these classes of fault; this
experiment demonstrates saved-schedule replay and small-workload detection,
not a measured improvement in bug-finding rate over those tests.

After restoration, 3,000 executions passed. Extended searches then passed
10,000 executions per workload for each of seeds 760, 761 and 762:
**90,000 executions, no failures**. These are execution counts, not counts of
distinct schedules or a completeness bound. No new Minigraf defect was found.

Five idle-machine runs, pinned to one NUMA node, ran the three workloads
sequentially in the debug test executable. Each ran 1,000 schedules per workload:
**1.0531–1.0848 seconds**, median **1.0638 seconds**, excluding compilation.
The 30,000-execution runs took 10.1808, 10.2648 and 10.1196 seconds for seeds
760, 761 and 762 respectively. These are local test-cost estimates, not hosted-CI
timings or database performance measurements. An instrumented build has its
own Cargo artifacts and incurs compilation of Shuttle and its transitive dev
dependencies; the runtime numbers exclude that cost.

Verification on the same source: 1,156 ordinary library unit tests passed
(five ignored), all eight real-thread snapshot tests passed, the repository's
`cargo clippy --all-features -- -D warnings` command passed, and library compile
checks passed for browser WASM (`--features browser`) and WASI. Formatting and
patch whitespace checks passed. The repository's dependency-policy command
(`cargo deny check advisories bans licenses sources`) passed. `cargo tree -e
normal` contains no Shuttle dependency. The complete integration/doc suite was
not rerun for this prototype.

An additional `cargo clippy --lib --tests -- -D warnings` run with the
instrumentation cfg failed on existing test-code lints (including integration
test unwraps and existing unit-test sort/cast/style warnings). It produced no
diagnostic in `deterministic.rs`. That broader command is not the repository's
CI lint gate; its pre-existing findings were left unchanged.

Replay environment: Shuttle 0.9.8, shuttle-engine 0.1.7, shuttle-std 0.1.5,
shuttle-schedulers 0.1.1. The retained local root lockfile has SHA-256
`08a1495662974810709834e9a3530faa0f1f63656020f868767a75bc20fd625f`.
For a fresh resolution, the mutation script regenerates and validates schedules
instead of assuming that the strings above remain valid forever.

## CI adoption

`.github/workflows/deterministic-schedules.yml` runs the three workloads on
Linux, macOS and Windows with stable Rust, and on Linux with Rust 1.89.0.
Relevant PRs targeting `v3` run 1,000 executions per workload with seed 760.
Nightly (19:23 UTC) and manual runs check out `v3` by default and run 10,000
executions per workload for seeds 760, 761 and 762. A manual run can select
another source ref. Each job is capped at 30 minutes, and the per-execution
step bound remains 10,000. The workflow fails if the filter selects anything
other than three tests or if any workload fails to finish its requested budget.

The identical workflow needs to land on `main` as a separate companion PR
**after** the suite lands on `v3`: GitHub executes schedules only from the
default branch. The companion adds no test implementation to v2.x. At the
v3 release cut, change the workflow's PR target and scheduled/manual default
source ref to `main`. This job does not change branch-protection settings.

Every job uploads a `dst-<os>-<toolchain>-<attempt>` artifact for 14 days,
even on test failure. It contains seed logs (including the failing test and
Shuttle schedule), the resolved `Cargo.lock`, compiler/Cargo versions,
configuration, source revision and a tracked-source archive. To replay:

1. Extract `source.tar.gz` into an empty directory and copy the artifact's
   `Cargo.lock` into that directory.
2. Select the recorded Rust version and native target; take the failing test
   name and schedule from `seed-*.log`.
3. Use the `MINIGRAF_DST_REPLAY` command above with `--locked` and `--exact`.

Do not substitute the current branch tip or resolve fresh dependencies when
replaying an archived failure. Archive the artifact elsewhere if it must be
kept beyond retention. No credentials or runner environment dump are collected.
The deliberate-fault script remains a manual harness-validation tool, not a
scheduled failing job. The seed log pipeline preserves Cargo's failure status.

The actual workflow exploration block was validated on the bench under Rust
1.89.0, including PR/nightly budgets, rejection of a build selecting zero tests,
and propagation/logging of the injected partial-transaction failure. See the
PR's native matrix for hosted-runner results. Source and runtime behavior are
unchanged from the measured prototype; CI configuration and documentation are
the adoption changes.

This costs two small synchronization import substitutions and three bounded
workloads. The maintenance cost is keeping those substitutions aligned with
the real lock graph and keeping fixtures representative. New shared-state
paths need an explicit audit before inclusion. Full-engine simulation,
filesystem simulation and a general-purpose hook layer remain deferred; the
present evidence does not justify making them v3 release dependencies.
