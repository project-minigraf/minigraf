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

A named runner label alone is not a pin. No following TODO is a measured fact.
Every original run stream and retained manifest must record its actual public
configuration, source revisions, Release profile, flag state and executed
artifact hash. The raw decision input declares one immutable configuration and
build manifest for all its attempts; extraction must verify each original run
matches it. Device and simulator artifacts have their **own** hashes.

| Physical configuration | Pre-registered reference | Fields to complete before timing |
| --- | --- | --- |
| iOS physical | Device Farm iPhone 15 / iOS 18.5 | TODO: availability, exact public model/OS build; if unavailable, closest replacement and selection rationale |
| Android physical | Device Farm Pixel 7 / Android 14 | TODO: availability, exact public model/OS build; if unavailable, closest replacement and selection rationale |

| Candidate | Required pins (currently unchosen) |
| --- | --- |
| iOS simulator on hosted macOS | TODO: runner label; exact image/version; host architecture; full Xcode version/build; simulator runtime version/build; simulated device type; guest ABI; any translation |
| Android emulator on hosted Linux | TODO: runner label; exact image/version; host architecture; emulator version; API level; full system image package/version; guest ABI; acceleration backend/version; observed hardware acceleration evidence; guest/host ABI translation or explicit none |

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

## Pre-timing checklist

- [ ] Commit this protocol and tooling before any timing; record the commit/hash.
- [ ] Fill every configuration TODO; resolve physical-reference availability
  and document any replacement **before timing**.
- [ ] Pin core, binding and harness revisions. Use matched Release sources and
  boundaries; record workflow, compiler/build flags, source hashes and separate
  executed artifact hashes for every configuration and flag state.
- [ ] Implement and review the one source flag in both binding harnesses;
  confirm it is absent/disabled in production builds and both operations receive
  complete result checks. Preserve explicit physical/simulated identities.
- [ ] Prove Android hardware acceleration on the actual candidate; record ABI
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
