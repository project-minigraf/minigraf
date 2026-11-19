# Spec — 007-binding-nightlies (#858)

**Mission Type:** mixed (CI workflow/reporting code, binding benchmark harness code, empirical qualification, durable evidence and docs)

**Base branch:** `v3` for core work. `main` gets a companion PR with the identical shared-CI change, because scheduled runs use the default branch's `bench.yml`. Binding harness changes land on the binding repos' `v3` branches. No v2.x measurement or backport.

## 1. User Intent

> next v3.0.0 issue #858

Issue #858, "Bench: add JVM/WASI nightly lanes and qualify iOS/Android emulator coverage" (milestone v3.0.0), is the controlling scope. Maintainer clarification in the issue (2026-11-05): the #839 physical-device/dedicated-bench restriction is not a blanket rule. "A hosted iOS simulator or Android emulator may be included in nightlies **if evidence shows sufficiently close performance to the corresponding physical-device workloads**." This issue does not assume that condition is met and does not allow emulator results to be treated as physical-device results without qualification.

Planning answers (2026-11-18):

> Scope: A+B in one mission — JVM/WASI lanes plus the reviewed mobile qualification protocol, measurement and per-category decision; #858 closes when all are done.

> Device Farm: Yes, fresh matched runs — new physical series at the candidate revision, baseline and injected-slowdown variant, same device models as #839.

> Thresholds: the draft numbers (§4 AC12), fixed now in the spec.

> Shortlist: all 14 JVM + 3 WASI cases, v3-only like the existing binding lanes.

## 2. Goal

Extend the default-branch-owned nightly benchmark system with Linux/JVM and WASI lanes that measure the once-resolved core of each v3 run. Each lane gets exact-result validation, the existing selected = executed = uploaded reporting contract, its own Bencher history and working regression notification. Separately, qualify the iOS simulator and the Android emulator on GitHub-hosted runners against fresh, matched physical-device runs on AWS Device Farm. Use thresholds pre-registered in this spec. Enable a simulator/emulator nightly lane only for the cases that pass, and give every other case an explicit, evidenced disposition. No database runtime, public API, file format, core dependency or binary-size change.

## 3. Context & Investigation Findings

Full investigation: `Planner-Discussion/Claude-01.md` and `Codex-01.md`. Key references:

### Nightly system (identical on `main` and `origin/v3` today)
- `.github/workflows/bench.yml:1-43`: cron `0 2 * * *` measures `main`, `0 14 * * *` measures `v3`. Workflow concurrency group `bench`. Scheduled runs always use the default branch's file.
- `bench.yml:46-98`: the `compile` job resolves the measured SHA once. Helpers come from `github.workflow_sha` (`.bench-workflow` sparse checkout).
- `bench.yml:557-810`: the `bindings` job (python/browser/node/c). It is skipped when `benches/minigraf_bench/prepared.rs` is absent, which in practice means v3 only. It uses one `binding-ref` input for every repo, `binding-sha.txt`, testbed `binding-<runtime>-ubuntu24-v1`, t-test upper boundary `0.999` and max sample size 30, regression issues with a two-run repeat policy (`:654-790`) whose lookup assumes job name `bench / <suite>`, and artifacts kept for 30 days only.
- `.github/bench/bindings.sh:10-25` patches the binding to the measured core and asserts it through `cargo metadata`. Its build/run cases cover only the four existing runtimes.
- `.github/bench/bindings-report.cjs:4-17` hard-codes the runtime list and the 6 (+2 browser) IDs. `:23-43` expects Bencher JSON metrics plus exactly one `ok: <id>` line per ID. `report.cjs:20-47` (`audit`) rejects missing/extra/duplicate IDs and failed uploads.
- `.github/workflows/bench-report.yml` runs only the Node reporting tests (`bindings-report.test.cjs`, `report.test.cjs`, `history.test.cjs`). Changes under `benchmarks/bindings839/` do not trigger it.
- The #776/#840 snapshot had 427 native IDs. `docs/bench-coverage-v3.md:28-40` records #863's six further routed candidates. Current counts come from actual-run evidence, not a frozen number.

### Harnesses to reuse (#839, accepted; evidence merged in #852)
- JVM: `minigraf-java` v3 `bench/manual/` (`Benchmark.kt:79-240`, `ResultChecks.kt` + `ResultChecksTest`, `build.sh:7-52`, `build.gradle.kts:6-15`). 14 cases, JSONL output, smoke/benchmark modes, `-Xms512m -Xmx512m -XX:+UseG1GC`, 100 short-op warmups / 5 write warmups, 10 samples of 20 ops (1 op for the two write cases), Java 17 compatible. `build.sh` does its own core patching and refuses an existing `.cargo/config.toml`, which conflicts with `bindings.sh`'s patch step. One of them has to own the patch.
- WASI: core v3 `benchmarks/bindings839/wasi.py` (+ `validate.py`, `test_wasi.py`, `test_guardrails.py`, `README.md:8-35`). 3 REPL cases, Wasmtime with `-C cache=n -C parallel-compilation=n`, no preopened dir (in-memory), 3 warmups, 10 samples, 20 ops per query sample. It refuses Python `-O`, and `--core-revision` is only a label. `.github/workflows/wasm.yml:105-128` already pins Wasmtime `v49.0.2` (its test flags preopen a dir and must not be copied into timing).
- `benchmarks/bindings839/validate.py:11-49` is a strict validator for the 14-case and 3-case sets: full sets only, rejects smoke. Partial mobile activation needs an equally strict selection contract, not a loosened allowlist.
- Swift/iOS: `minigraf-swift` v3 `bench/device-farm/`. `Tests/BenchmarkTests.swift:152-181` rejects simulator timing. `build.sh:27-61` builds device + arm64-simulator libraries but records only the device library hash/ABI. `:70-81` auto-picks a simulator (not pinned). `.github/workflows/bench-839-build.yml` builds the IPA/XCTest.
- Android: `minigraf-android` v3 `bench/device-farm/`. It is ARM64-only (`build.sh:32-65`, `build.gradle.kts:16-32` `arm64-v8a`), non-debuggable Release. `BenchmarkTest.kt:88-134` defaults to smoke, and `-e mode benchmark` times. It records model/fingerprint/ABIs but has no first-class physical-versus-emulated identity.
- Physical baselines: `docs/benchmarks/839-bindings/results.md` (v3). Pixel 7 / Android 14 and iPhone 15 (`iPhone15,4`) / iOS 18.5, core `33e5dffa…`, AWS Device Farm public devices (a model/OS pin, not a reserved unit), with visible spread. These are historical and **not** the qualification reference (see §6).
- Status drift: `docs/benchmarks/839-bindings/README.md:3-4`, `dispositions.md:34-48` and `docs/BINDING_BENCHMARKS.md:79-102` still say "pending sign-off". #839 accepted both maintainer roles for evidence revision `10ff8003…` (merged in #852 as `cdad3e72…`).
- #840 completion record and tooling: `docs/benchmarks/840-regression/completion.md:14-129` (fresh independent runs, inherited main observations excluded, isolated notification scenarios, checksummed archives that reproduce byte-for-byte). This is the pattern for the evidence work here.

### Environment facts checked during planning
- `minigraf`, `minigraf-swift`, `minigraf-android`, `minigraf-java` are public repos, so hosted `macos-15` and `ubuntu-24.04` minutes are free. Hosted macOS concurrency/queue time is still a limit to document.
- The local AWS CLI has a `minigraf-devicefarm` profile. Device Farm work uses it, and no account identifiers go into public artifacts.

## 4. Acceptance Criteria

### Part A — JVM and WASI nightly lanes

**Common lane contract.** AC3 and AC5–AC9 apply to **every added lane**: JVM, WASI, and each mobile configuration activated under AC14. Read "binding SHA" as the binding repository SHA wherever the lane has one (JVM, Swift, Android), and read the suite names in AC9 as each lane's own suite (`binding_jvm`, `binding_wasi`, and each mobile lane's suite). Each lane has a pinned runtime configuration, records the identity actually executed, and has its own configuration-specific and experimental history. Runtime-specific details are in AC4 (JVM/WASI) and AC10 (mobile).

1. **Workflow on both branches.** The JVM and WASI lanes are in `bench.yml` on `main` and `v3`. After both PRs merge, `.github/workflows/bench.yml`, `.github/workflows/bench-report.yml` and `.github/bench/` are byte-identical across the two branches. Main-branch (v2) runs skip the new lanes the way they skip the existing binding lanes: no upload, and a step-summary note. A v3 run where a required harness/artifact is missing fails; it does not report success with nothing measured.
2. **Shortlist.** The JVM lane measures exactly the 14 IDs `binding/jvm/{prepared_execute/1k, cursor_batch_1/1k, cursor_batch_256/1k, set_roundtrip/2, predicate/1k, aggregate/1k, prepare/point, register_predicate/first, register_aggregate/first, fact_log_batch_256/1k, options_error/negative_cache, options_readonly_open/1k, file_transact/1k, log_writer_append_finish/1k}`. The WASI lane measures exactly `binding/wasi/{repl_startup_empty, repl_script/1k, repl_query_roundtrip/1k}`. Timing boundaries are unchanged from #839 and documented in `docs/BINDING_BENCHMARKS.md`. The existing native routes and the 26 existing binding IDs are unchanged.
3. **Measured-core provenance.** Each run of an added lane records the run's once-resolved core SHA, workflow SHA, binding SHA (JVM), harness/driver SHA-256, and the SHA-256 of the executed artifact (JVM native library as loaded or packaged; `minigraf.wasm`). The JVM build is asserted to use the measured core checkout, at least as strictly as `bindings.sh`'s metadata check. A binding's release pin or a supplied revision label cannot satisfy this.
4. **Runtime contracts.** JVM runs on a pinned JDK release (distribution and full version, not only a major version like `jvmToolchain(17)`), records the JDK vendor/version and the JVM flags actually used, and keeps the #839 heap/GC/warmup/sample policy. WASI uses a Wasmtime version pinned by version **and** binary checksum, cache and parallel compilation disabled, no preopened directory, and the #839 warmup/sample policy. Lane docs state the startup-compilation and pipe/framing overheads. Neither lane claims engine-only latency.
5. **Exact results.** Every timed invocation checks the complete expected result (rows/sets/counts/sums, and error code where applicable). A wrong or empty result, nonzero exit, unexpected stderr (WASI) or failed teardown fails the job before upload. Untimed/smoke mode emits no timing data. Each new path has a negative test that shows a wrong-but-plausible result is rejected.
6. **Reporting contract.** The shortlist definition and `bindings-report.test.cjs` cover `jvm` and `wasi`. Metrics are Bencher JSON in nanoseconds. Each metric can be recomputed from the retained raw samples with the documented statistic (median of 10 sample means, matching P16/#839). Selected = executed = uploaded is enforced, with one `ok:` per ID. Missing, extra, duplicate, nonfinite or nonpositive values fail.
7. **Histories.** The new testbeds are `binding-jvm-ubuntu24-v1` and `binding-wasi-ubuntu24-v1` (or equally explicit versioned names), with no inheritance from native or other binding histories. Non-v3 binding refs keep using `binding-experiment/...` history. The #840 head-reset bug does not come back: a main advance does not reset v3 history.
8. **Usable baselines.** After merge, every new ID of an added lane has a finite numeric baseline and upper limit on its v3 testbed. These come from at least 3 independent v3 runs (separate workflow runs, at least one of them a real 14:00 UTC scheduled event), with t-test upper boundary 0.999 and max sample size 30 confirmed through the Bencher API. Inherited main data, re-uploads and #839 samples do not count.
9. **Notification.** For each added lane's suite, isolated live scenarios (the #840 pattern, without polluting production histories) show three things: a repeated exact-ID alert on the same branch/suite opens/comments the regression issue; an upload failure notifies immediately; a failed prior-log lookup warns with an empty prior set. Synthetic resources are removed afterwards.

### Part B — iOS simulator / Android emulator qualification

10. **Configurations pinned.** The spec's qualification report names, for each platform, the physical reference and the candidate CI configuration, and records them on every run. Physical references: iPhone 15 / iOS 18.5 and Pixel 7 / Android 14 on Device Farm (same models as #839; if unavailable, the closest available model/OS, documented, before any timing). Candidate CI: hosted macOS runner image + Xcode + simulator runtime + device type; hosted Linux runner + emulator API level + system image + ABI + acceleration. The Generator must verify empirically that the Android configuration runs with hardware acceleration and must disclose any guest/host ABI translation. Simulator/emulator runs record their own artifact hash, not the device library's.
11. **Matched builds.** Physical and simulated runs for one comparison use the same core, binding and harness revisions, Release builds, and identical cases and timing boundaries. The 14-case mobile suite is the candidate set. Any case that cannot run identically on both is reported as inconclusive with the reason.
12. **Pre-registered gate (per case, per platform configuration).**
    - **Datasets.** For each platform configuration (physical, and each candidate simulator/emulator), collect three sets of independent runs (fresh app/process each): baseline **B1**, a second baseline **B2**, and slowdown variant **V**. Each set has **N runs, N ≥ 5**. N, the run order (for example interleaved B1/V/B2 batches) and the retry rule are recorded in the protocol **before timing**. A run's value is the median of its 10 sample means.
    - **Set membership is fixed.** A retry is allowed only for a run that produced no valid stream (provider/infrastructure failure, validation failure), up to a retry cap declared before timing. Every attempt is retained with its reason. A valid run is never replaced or discarded, whatever its value.
    - **Closeness** uses B1: median(sim B1 run values) ÷ median(physical B1 run values) within **[0.67, 1.5]**.
    - **Repeatability** uses B1: CV = **sample** standard deviation (n − 1 denominator) ÷ arithmetic mean of the run values. Sim CV **≤ 15%** and sim CV **≤ 2 ×** physical CV.
    - **Regression signal.** The slowdown variant does real extra work on the timed path, keeps correct answers, is built identically for both platforms, and has its expected magnitude stated before timing. On each platform: Δ = median(V run values) ÷ median(B1 run values) − 1 (the fractional increase, not the multiplier). Detection = one-sided Welch t-test of V > B1 on run values, **p < 0.001**. Pass requires detection on both platforms and |Δ_sim − Δ_phys| ≤ **0.5 × Δ_phys**.
    - **A/A** compares B1 with B2 on each platform with the same Welch test. Pass requires p ≥ 0.001 in **both** one-sided directions, on both platforms.
    - **Degenerate data.** Zero variance in a set, an undefined test statistic, a nonfinite or nonpositive value, or Δ_phys ≤ 0 makes the case **inconclusive** for that configuration. It never passes.
    - The Welch test here is a two-group qualification test. It is a different procedure from Bencher's production `t_test` threshold (a prediction interval for a new value against history, upper boundary 0.999, max sample size 30). Equal nominal tail probabilities do not make their decisions equivalent, and neither setting changes the other.
    - **Outcome:** pass closeness, repeatability, regression signal and A/A → **qualified**. Valid data that fails any of them → **rejected**. Too few valid runs after the declared retries, an unmatched build, an unavailable device, a provider failure or degenerate data → **inconclusive**, never reported as rejected or qualified.
    - Each case is reported under its category (compute/callback, allocation/conversion, storage/startup), with the mapping stated in the report. The decision is per case; a category summary cannot override a case result.
    - These numbers are fixed by this spec before any qualification timing. They may not be changed after seeing results without a new user decision recorded in the report.
13. **Qualification report.** `docs/benchmarks/858-mobile-qualification/` (or equivalent) contains: the protocol (AC12, copied before timing), every raw sample from every run (physical and simulated, including failed/aborted runs with reasons), build identities and hashes, per-case tables (physical and simulated distributions, ratio, CVs, A/B and A/A results, outcome), category summaries, and a reproduction script that recomputes every decision from the raw data. Negative and boundary controls show the decision script: rejects a fabricated "close" dataset that fails each criterion in turn; applies the sample-CV convention at the 15% boundary (for example run values 80, 90, 100, 110, 120 have sample CV 15.81% and fail); uses fractional Δ, not the multiplier; and returns inconclusive for zero-variance, nonfinite and too-few-run inputs.
14. **Conditional activation.** Each platform configuration with at least one qualified case gets a nightly lane for exactly its qualified cases. Each lane has its own testbed (named to say simulator/emulator and its configuration, e.g. `binding-swift-ios-sim-<runner>-v1`, `binding-android-emu-<abi>-<runner>-v1`), meets the common lane contract (AC3, AC5–AC9), and its docs say it is a qualified simulator/emulator lane, not physical-device timing. Rejected and inconclusive cases stay excluded and are listed with their evidence. Where a platform has no qualified case, the report contains a costed physical-device-lane proposal (not implemented). Physical mode stays runnable in both harnesses, and the simulator guard in Swift is replaced by an explicit platform identity, not just deleted.
14a. **Part B completion rule.** Part B, and therefore #858, is complete only when, for **both** iOS and Android, the matched B1/B2/V datasets of AC12 were actually collected on the physical reference and at least one candidate CI configuration, and every case has a decision computed from them. An outcome where the comparison was never performed does **not** complete Part B. Examples: Device Farm never produced a valid physical series, the simulator/emulator never ran, or every case is inconclusive only for those reasons. Such an outcome is reported as unfinished work with its blocker, and #858 stays open. A completed study may reject every case, and individual cases may stay inconclusive for an evidenced, case-specific reason (for example an API that cannot run identically on both). No mobile lane is required for success.

### Shared

15. **Durable evidence.** Raw samples, reports, Bencher API evidence (baselines/limits/models), runtime/runner metadata and the qualification data are kept beyond Actions retention in a checksummed archive (committed under `docs/benchmarks/858-*/`, or committed hashes plus an external archive location). A fresh extraction reproduces the derived reports byte-for-byte. No private host, account or device-farm identifiers. Task-owned scratch (local, bench-machine, and temporary CI refs/branches) is removed only after the archive has been verified. Historical runs, production Bencher histories, Device Farm projects/runs and retained source evidence are never deleted as cleanup.
16. **Operational limits documented.** Measured (not estimated) per-lane setup/build/measure durations, hosted runner availability/queueing observed, Device Farm minutes used, and repeatability limits are in the docs. Estimates are labelled as estimates.
17. **Docs in sync.** `docs/BINDING_BENCHMARKS.md`, `docs/bench-coverage-v3.md`, `docs/BENCHMARKS.md` (v3 content; main's v2 file only if the main companion changes something user-visible), the #839 status overlays (accepted; history kept), `CHANGELOG.md` v3 Unreleased, `ROADMAP.md`, and the #858 status statements on v3 in `CLAUDE.md` (line 19), `README.md` (line 43) and `docs/TEST_COVERAGE.md` (line 103), which are updated to the actual enabled lanes and exclusions even though Rust test counts stay unchanged. #840's historical inventory stays as historical evidence. The docs site (`../minigraf-docs`) is checked; any benchmark-coverage text there is updated with `@since v3.0.0`. The approved spec is copied to `docs/superpowers/specs/2026-11-18-binding-nightlies-design.md`.
18. **Nothing else changes.** No change to `src/`, public API, core `Cargo.toml` dependencies, file format or binary size. The existing Node reporting tests, `test_wasi.py`, `test_guardrails.py`, JVM/Android `ResultChecksTest` and the Swift oracle test pass. `bench-report.yml` also runs whenever the paths the new lanes depend on change (including `benchmarks/bindings839/**` or its successor).

## 5. Edge Cases & Boundaries

- **One `binding-ref` for all repos:** a JVM-only experiment branch does not exist in the Python/browser/Node/C repos. An experiment on one binding must not break the other lanes, and the SHA actually used is recorded either way.
- **Premature success:** samples printed before a later check, teardown or exit failure must not count. A provider "PASS" (Device Farm, Gradle, XCTest) is not success without the validated complete stream.
- **iOS log recovery:** #839 needed the JSONL from the automation log (artifact attachment failed). Recovered evidence must be exactly one complete stream: no duplicated application/syslog copies, no truncation, no smoke/benchmark mixing.
- **Stale outputs:** a fresh output location per invocation, so an old timing file can never satisfy a new run.
- **JIT/ART/thermal:** the configured warmups do not prove stationarity. Record conditions and spread; do not normalize away a closeness failure (#863's runner-speed normalization is not allowed for AC12).
- **Platform identity:** a simulator device name is not hardware identity. Emulator host ISA, guest ABI, image/API level and acceleration are separate recorded facts. A run that cannot prove which platform it ran on is invalid.
- **Same SHA, independent runs:** repeated runs of one revision are fine. Independence comes from separate processes/workflow runs, not distinct commits.
- **Configuration drift:** a changed runner image, Xcode, system image, ABI, JDK, Wasmtime or timing boundary needs a new testbed version, and for mobile, requalification. Do not merge histories because suffixes match.
- **Hosted runner variability:** a macOS queue timeout or runner shortage is "unavailable", not a performance failure.
- **Injected slowdown:** it must do real, representative work on the timed path (not a sleep or a timestamp offset) and keep correct answers. It must never be in a production nightly build. Synthetic latency is fine only for notification plumbing tests (AC9), never for qualification or production baselines.
- **Main schedule:** the 02:00 UTC main run must keep its current coverage and timing; the new lanes skip cleanly.

## 6. Key Decisions

- **One mission for A and B** (user). #858 closes only when both are done. PRs are split for review (§8).
- **Fresh matched physical runs on Device Farm** (user), not a comparison with the #839 data, because #839 is at an old revision and has no slowdown A/B. #839 stays historical evidence.
- **Thresholds fixed now** (user approved the draft, AC12). Pre-registration exists to stop choosing thresholds after seeing the data.
- **All 14 JVM + 3 WASI cases** (user). The eight lifecycle/storage JVM cases are the costs #839 added beyond P16. Lane cost is measured under AC16, not assumed.
- **v3-only measurement** (Claude + Codex), the same as the existing binding lanes. The main PR only carries the identical workflow.
- **Integrate with the existing nightly system**, not a separate one. The new lanes reuse its provenance, reporting, history and notification behaviour, and its four-runtime assumptions are resolved rather than bypassed. The job layout (a generalized matrix or sibling jobs, for example for WASI with no binding repo, or for a macOS-hosted iOS lane) is the Generator's choice.
- **Separate qualification from activation.** Experimental simulator/emulator timing during qualification goes only to experiment histories (or none), never to a production testbed.
- **Historical docs keep their wording**, with current-status overlays (#839 accepted, #840 complete).

## 7. Out of Scope

- Engine optimizations, API/format/dependency/size changes.
- Reopening #839 sign-offs or #776's native audit.
- A macOS desktop Swift lane (an iOS simulator on a macOS host is in scope; desktop Swift is not).
- Implementing a physical-device nightly lane (only a costed proposal when qualification fails).
- An unqualified simulator/emulator "trend monitor".
- RSS/memory bounds, scaling sweeps, published-package certification.
- v2.x JVM/WASI measurement; releases/version bumps.

## 8. Possible Directions & Ideas (non-binding)

- **PR sequence:** (1) core v3 + main companion: generalize the binding job/reporting for JVM + WASI, plus the Java v3 nightly adapter; (2) collect fresh baselines and run the notification scenarios; (3) Swift/Android qualification modes (platform identity, simulator/emulator builds, slowdown variant) in the binding repos; (4) the qualification runs and report; (5) activation PRs for qualified cases only, with baselines/notification for them; (6) docs + evidence archive.
- **Job layout:** extending the existing `bindings` matrix fits JVM well. WASI (no binding repo) and an iOS simulator lane (macOS host) may fit better as sibling jobs that share the same reporting helpers.
- **Suggested binding:** a thin `bench/nightly.*` entry in `minigraf-java` that reuses `Benchmark.kt` cases and emits the existing contract, rather than changing the #839 manual mode.
- **WASI:** a small adapter from `samples.jsonl` to Bencher JSON plus `ok:` lines, keeping `wasi.py` and `validate.py` the source of truth.
- **Android emulator:** the x86_64 image on `ubuntu-24.04` with KVM needs an x86_64 build of the harness; an arm64 image on an arm runner may avoid that if acceleration works. Check both before committing to one.
- **Injected slowdown idea:** a harness-only compile flag that repeats the timed operation's real work a stated number of extra times, for example 1 extra time per 2 invocations, about 1.5×. Same flag on device and simulator.
- **Prior evidence tooling:** `docs/benchmarks/840-regression/` collectors/verifiers (exact API reconciliation, lineage, negative controls, archive reproduction).
- **Heavy local work** (if any) goes on the dedicated bench machine, never the laptop. Mobile builds run on hosted CI, and timing runs on Device Farm or hosted runners.

---

## Changes from Claude-02 (response to Codex-02)

All Codex-02 findings accepted after rechecking the sources:
- **H1:** job topology moved from a decision to a non-binding suggestion (§6, §8).
- **H2:** new AC14a, the Part B completion rule. Non-execution is unfinished work, not completion.
- **M1:** AC12 now defines datasets B1/B2/V, fixed set membership and retries, sample CV, fractional Δ, the A/A rule in both directions, and degenerate data → inconclusive. The approved numbers are unchanged, and the Welch test is distinguished from Bencher's `t_test`. AC13 adds boundary controls (CV example checked: run values 80–120 give sample CV 15.81% against population 14.14%).
- **M2:** common lane contract added at the top of §4, so AC3 and AC5–AC9 cover every added lane, including activated mobile lanes. The JDK release is pinned (AC4).
- **M3:** AC17 names the #858 status lines in v3 `CLAUDE.md:19`, `README.md:43` and `docs/TEST_COVERAGE.md:103` (verified with `git show origin/v3`).
- **M4:** AC15 requires scratch cleanup only after the archive is verified.
- **Low:** 427 is worded as the snapshot; "cost is small" removed.
