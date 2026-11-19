# #858 mobile qualification protocol — pre-registration

Status: **no qualification timing collected here; not yet committed**. The
requesting user fixed the parameters below before timing. Commit this protocol,
the completed configuration pins, schema and decision tooling **before any
qualification timing**. This task deliberately makes no commit. The supplied
fixture is synthetic and is never evidence of mobile qualification.

This is the local preparation for [mission 007, AC10–14a](../../superpowers/specs/2026-11-18-binding-nightlies-design.md).
The controlling spec for this preparation was
`TandemKit/007-binding-nightlies/Spec.md`, Part B AC10–14a, read in full.
It follows Minigraf's reliability and cross-platform philosophy: no production
runtime, API, dependency, file-format, binary-size or nightly-route change.

## AC12 gate (copied before timing)

The following is AC12's gate, with its original thresholds and outcome rules.
The fixed choices in the next section instantiate its pre-registration slots.

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

## Fixed user decisions for this study

- **N = 6 independent runs per set B1, B2 and V per configuration.** Every run
  starts a fresh app/process. Ten within-run sample means are not ten independent
  runs. The run value is their median (average of sorted entries 5 and 6).
- **Order:** `(B1, V, B2) × 6`, using run indices 1 through 6. Physical and
  simulated configurations use this identical order. Complete any allowed retry
  in its original slot before proceeding. No optional stopping or extra valid runs.
- **Retries:** only when there is no valid stream, due to provider/infrastructure
  failure or validation failure. At most **2 retries per run slot** (attempts
  1–3 total). Retain every attempt, partial stream and reason. A valid stream,
  regardless of its latency, is never replaced or discarded. Exhaustion or an
  unattempted slot is inconclusive. A recovered infrastructure failure does not
  invalidate six subsequently valid slots; an unresolved failure does.
- **V:** a harness-only source flag, recorded as
  `repeat-timed-operation-twice`, executes each case's timed operation **TWICE
  inside the timed region**, with real extra work and the same correct answers.
  B1/B2 disable this flag; V enables the same flag in physical and simulated
  builds from the same sources. Never enable it in production builds.
- **Expected fractional slowdown:** d approximately **+1.0**, with a documentation
  range **0.7–1.3** for compute/callback and allocation/conversion cases.
  Storage/startup may differ; report each measured magnitude and explain it.
  This expectation/range is **not an additional gate**. AC12's detection and
  fractional-delta agreement gates are the only regression-signal gates.
- **Physical references:** Device Farm **iPhone 15 / iOS 18.5** and **Pixel 7 /
  Android 14**. If unavailable, document the closest available model/OS and its
  selection before any timing. The historical #839 timings are not references
  for this study and cannot be reused as independent runs.

The extra operation must not be normalized away: retain the original logical
iteration count when computing `sample_mean_ns = total_ns / iterations`.
For a sample containing 20 logical invocations, V executes 40 operations but
still divides by 20. Preserve the original case's warmups, number of logical
invocations and timing boundaries; no sleeps, timestamp offsets or fabricated
latencies. Validation must check both operations completely, in the same
validation placement as the original harness.

Stateful operations need two independent, equivalently prepared fixtures when
reusing one would change semantics: both registrations must remain **first**
registrations, both read-only opens use equivalently seeded files, and each
LogWriter append/finish gets a fresh unfinished writer and destination. Prepare
both fixtures outside timing only where original fixture setup was outside
timing; keep originally timed setup inside both operations. Retain full
post-write/reopen validations and teardown. If an operation cannot be repeated
with identical boundaries and answers on both platforms, mark that case
inconclusive with the reason; do not substitute a different slowdown after
seeing results. Implementing these harness flags is subsequent work.

## Configuration pins — fill and record before timing

A named runner label alone is not a pin. The values below were observed in build-only runs before timing; every timed run re-records its own actual values.
Every original run stream and retained manifest must record its actual public
configuration, source revisions, Release profile, flag state and executed
artifact hash. The raw decision input declares one immutable configuration and
build manifest for all its attempts; extraction must verify each original run
matches it. Device and simulator artifacts have their **own** hashes.

| Physical configuration | Pre-registered reference | Fields to complete before timing |
| --- | --- | --- |
| iOS physical | Device Farm iPhone 15 / iOS 18.5 | Available on 2026-11-19 (Device Farm listing: `Apple iPhone 15`, iOS 18.5, public fleet, highly available); no replacement. Upload acceptance and a physical smoke (`BenchmarkTests/testSmoke`, 1 test selected, PASSED, 8.06 device minutes) completed before timing. The exact OS build comes from each run's own environment record. |
| Android physical | Device Farm Pixel 7 / Android 14 | Available on 2026-11-19 (`Google Pixel 7`, Android 14, public fleet, highly available); no replacement. Upload acceptance and a physical smoke (`BenchmarkTest#verifyAndMeasure`, 1 test selected, PASSED, 0.35 device minutes) completed before timing. ABI of the physical device library: arm64-v8a. |

| Candidate | Required pins (currently unchosen) |
| --- | --- |
| iOS simulator on hosted macOS | Runner label `macos-15`; image `macos15` version `20261118.0550.1` (macOS 15.8.2, build 24H232); host ARM64; Xcode 16.4 (16F6); simulator runtime iOS 18.6 (22G86); device type `iPhone-15`; guest/native slice `aarch64-apple-ios-sim` on the arm64 host, no guest/host translation (the build script records `sysctl.proc_translated`). **Disclosed difference:** the simulator runtime is iOS 18.6 and the physical reference iOS 18.5; no 18.5 runtime was selected because the pinned device type and newest iOS 18.x runtime available on the image are used, and the selection is recorded per run. Values observed in the build-only run 44500184518 of the Swift qualification workflow; each run re-records them. |
| Android emulator on hosted Linux | Runner label `ubuntu-24.04`; image `ubuntu24` version `20261111.127.1`; host x86_64 (kernel `7.0.0-1014-aws`); emulator 37.2.13 (build 16740729); API level 34 (Android 14), system image `system-images;android-34;google_apis;x86_64` revision 14; AVD device profile `pixel_7`; guest ABI x86_64; acceleration backend KVM: `/dev/kvm` present and `emulator -accel-check` reports "KVM (version 12) is installed and usable", and the executing guest records `acceleration_active: true` (build-only run 44506574166). x86_64 guest on an x86_64 host: no guest/host ABI translation. **Disclosed difference:** the emulator runs the x86_64 build of the native library, the physical Pixel the arm64-v8a build; each has its own hash. Boot policy: one cold boot with `-no-snapshot`, then a warm guest with the app and test package force-stopped before every attempt. |

Use a new configuration ID and repeat qualification if image, Xcode, runtime,
system image, ABI, acceleration or timing boundary changes. Do not pool
configurations or physical platforms. Record the Android hardware acceleration
check from the actual executing guest/host empirically; a requested KVM option
is not evidence that acceleration worked. Record any ABI translation explicitly.
Do not retain account numbers, private host identifiers, device serials or
private provider identifiers in public artifacts.

## Fourteen cases and categories

Suffixes below are the fixed IDs in `decide.py`. Full harness IDs expand under
`binding/swift/` for iOS and `binding/android/` for Android. The operations come
from [BINDING_BENCHMARKS.md](../../BINDING_BENCHMARKS.md) (six common operations
plus eight lifecycle/storage operations) and the mobile cost distinctions in
[bench-coverage-v3.md](../../bench-coverage-v3.md#binding-evidence-and-distinct-costs).
The [#839 dispositions](../839-bindings/dispositions.md) establish that the eight
additional operations apply to JVM, Android and Swift/iOS. Those documents do
not supply a complete three-category mapping; the following mapping is fixed
here before timing. Categories describe representative costs, not isolated
engine costs or measured attribution.

| Case suffix | Category | Basis for assignment |
| --- | --- | --- |
| `prepared_execute/1k` | allocation/conversion | Prepared integer bind and point-result allocation/decoding; query work remains included |
| `cursor_batch_1/1k` | allocation/conversion | Full 1k-row drain with conversion and one-row batch crossings |
| `cursor_batch_256/1k` | allocation/conversion | Same full drain with larger batches, partial batch and EOF |
| `set_roundtrip/2` | allocation/conversion | Host set binding and exact returned-set conversion |
| `predicate/1k` | compute/callback | 1k foreign callbacks; check the complete 500-even-value result |
| `aggregate/1k` | compute/callback | Foreign accumulator init, 1k steps and finish |
| `prepare/point` | compute/callback | Parsing/planning an in-memory point query; one-time setup does not imply storage or process startup |
| `register_predicate/first` | compute/callback | First callback installation on a fresh database; excludes database creation where the original harness does |
| `register_aggregate/first` | compute/callback | First accumulator callback installation, not steady-state execution |
| `fact_log_batch_256/1k` | allocation/conversion | Open/drain/close 1k fact records; representative foreign record conversion, not a file-open latency claim |
| `options_error/negative_cache` | allocation/conversion | Host option/error conversion and exact error code; rejected open performs no successful storage startup |
| `options_readonly_open/1k` | storage/startup | Open already seeded file with read-only/cache/memory options |
| `file_transact/1k` | storage/startup | One file-backed 1k-fact transaction, reopened and validated outside timing |
| `log_writer_append_finish/1k` | storage/startup | Append 1k complete records and finish a file; includes conversion but persistence is the representative cost |

There are 5 compute/callback, 6 allocation/conversion and 3 storage/startup
cases. These labels are immutable input-independent mappings. A category count
never qualifies or rejects any individual case.

## Matched-build procedure (pre-registered)

For each platform the timed collection is started by one commit to the binding
repository that sets `collect: true` in the qualification workflow's committed
`run-config.json` (core revision `b0d2d014d57c90680a6adc1d10e749a3db5df40c`). That
workflow run builds the device package and the simulator/emulator package from
the same core, binding and harness revisions, runs the simulator or emulator
batches, and uploads the device package. The physical Device Farm series uses
exactly that uploaded package, so physical and simulated runs share revisions,
Release profile and flag handling. Only the one source flag differs between
baseline and slowdown methods. A build whose revisions differ from the simulated
run's is not used.

## Pre-timing checklist

- [ ] Commit this protocol and tooling before any timing; record the commit/hash. (Committed with the pins below filled; the commit hash is recorded in the run manifests.)
- [x] Fill every configuration TODO; resolve physical-reference availability
  and document any replacement **before timing**.
- [ ] Pin core, binding and harness revisions. Use matched Release sources and
  boundaries; record workflow, compiler/build flags, source hashes and separate
  executed artifact hashes for every configuration and flag state.
- [ ] Implement and review the one source flag in both binding harnesses;
  confirm it is absent/disabled in production builds and both operations receive
  complete result checks. Preserve explicit physical/simulated identities.
- [x] Prove Android hardware acceleration on the actual candidate; record ABI
  translation and the public runtime details for both platforms.
- [ ] Run untimed exact-result, wrong-but-plausible-result and teardown controls;
  keep physical mode runnable. A provider PASS alone is not stream validity.
- [ ] Freeze N=6, `(B1,V,B2) × 6`, two retries, categories and expectation above.
  Set up fresh processes/output locations and attempt accounting. Reject stale,
  duplicate, truncated, mixed smoke/timing or failed-teardown streams.
- [ ] Preserve original ten sample means (and totals/iterations), failed/aborted
  streams, reasons and manifests. Collect setup/build/measurement durations,
  observed queue/availability and provider minutes; label estimates separately.
- [ ] Run local controls and byte-for-byte reproduction; record Python version.
  Archive evidence with checksums beyond Actions retention before scratch cleanup.

## Reporting and subsequent activation

Use [SCHEMA.md](SCHEMA.md), `decide.py` and `reproduce.py`. Valid numerical data
that fails any gate is rejected. Insufficient, degenerate or unmatched evidence
is inconclusive, even if another criterion would have rejected it. Protocol
violations (extra valid runs, replacement of a valid run, retry-cap breaches)
are inconclusive; no post-hoc selection repairs them. A/A uses independent
Welch tests, not paired tests despite interleaving. There is no multiple-testing
correction or pooled/category-level decision in AC12.

All thresholds are fixed. Changing them after results requires a **new recorded
user decision in the report**, a versioned protocol and tooling, and retention
of the original decisions. Welch qualification is different from Bencher's
production `t_test`; neither its prediction interval nor production histories
are changed by this tooling.

This preparation does **not** complete Part B or #858 (AC14a). Both platforms
still need actual matched physical and candidate B1/B2/V collections and all
14 per-case decisions. Never performing a comparison is unfinished work, not a
completed rejection. A completed study may reject every case; evidenced
case-specific inconclusive results remain possible.

Only qualified cases can later enter configuration-specific simulator/emulator
lanes under AC14 and the common AC3/AC5–9 contract, including three independent
baselines, a real scheduled run and notification evidence. Rejected/inconclusive
cases stay excluded with reasons. If a platform has no qualified case, report a
costed physical-device-lane proposal; do not implement it here. No lane is
activated by this report generator, including when synthetic cases pass.
