# Binding benchmarks (#776 P16)

The accepted [#839 manual evidence](benchmarks/839-bindings/README.md) remains
historical evidence. #858 adds core workflow support for JVM (14 IDs) and WASI
(3 IDs), alongside the existing 26 Python/browser/Node/C IDs. The Java nightly
adapter is merged in `minigraf-java` v3 (`0a2d4d657`) and the identical default-branch
CI companion is merged (main `b2ad8c24f`). The first scheduled v3 run (2026-11-19) reported
both lanes; live notification checks and the evidence archive are in
[the evidence directory](benchmarks/858-binding-nightlies/). The mobile
simulator/emulator study rejected every case on both platforms and activates no mobile lane
([report](benchmarks/858-mobile-qualification/REPORT.md)). Measured lane limits are at the end of
this file.

The `bindings` matrix in `.github/workflows/bench.yml` measures the once-resolved
core SHA. JVM checks out `minigraf-java`; WASI has no binding repository. The
14:00 UTC schedule measures v3; main/v2 checkouts skip with a step-summary note
and no uploads. A required harness or artifact missing on v3 fails the job.
Reporting JS/shell helpers come from `github.workflow_sha`. All core harness
Python, including `validate.py` for both JVM and WASI and the WASI driver/adapter,
runs from the measured core checkout. The WASI provenance hashes that same
`validate.py`; a different or missing workflow-checkout copy has no effect.
Both checkout identities are recorded.

This is the F1 option (b) design: the default-branch sparse checkout needs only
`.github/bench/`, and main does not need `benchmarks/bindings839/`. The shared
`bench-report.yml` keeps its harness path triggers and runs Python unit tests
only when `hashFiles('benchmarks/bindings839/test_*.py')` finds tests. Its Node
tests have self-contained fixtures and run on both branches. This reporting-CI
guard does not apply to measurement: a missing measured v3 validator fails both
JVM and WASI jobs, and missing WASI drivers also fail.

Manual dispatch can set `jvm-ref` for a Java-only experiment, leaving every
other binding on `binding-ref` (default `v3`). `binding-ref` still selects a
common ref for all binding repositories, so use it only when that ref exists
in all of them. WASI ignores binding refs and always uses the measured core.
Non-v3 core or binding refs use `binding-experiment/<core-ref>/<binding-ref>`
histories, with no production notifications.

For the two-run repeat policy (F2), search the latest 20 completed `bench.yml`
runs on the default branch, newest first, considering only runs older than the
current run with the same measured-branch title. For this suite, skip missing
jobs and explicitly identified experiment histories, including JVM-only
experiments (even if their job failed). Stop at the first matching
production log, even if it has no alerts: a clean production run breaks the
streak. The matching line is `History branch: v3; testbed: <suite's testbed>;`.
If a completed suite job has conclusion `failure` but no matching production
line or explicit experiment history, stop with an empty prior set and a warning:
a build/verification failure before the summary must not bridge two alerts.
A failed job with a matching production line still supplies its actual alerts;
Bencher regression failures can therefore repeat. Other nonmatching logs are
skipped. Only exact IDs repeated in the matching log can notify. A failed API/log lookup
stops the search and warns with an empty prior set; exhausting the 20-run window
without a match also warns with an empty set. Upload failures still notify
immediately. This bound can leave an older production run out of reach after
many experiments; it never treats an experiment as production repetition.
An early failure without a history line cannot prove whether it was an experiment;
it conservatively breaks the streak too.

## Existing Python/browser/Node/C lanes

All cases use 1,000 deterministic in-memory integer facts, plus one two-element
set fact. The suffixes below expand under `binding/{python,browser,node,c}/`:

| Suffix | Measured operation |
| --- | --- |
| `prepared_execute/1k` | Prepared integer binding, point query, foreign result allocation and JSON decoding |
| `cursor_batch_1/1k` | Prepared cursor open, full drain of 1k rows in batches of 1, decode and close |
| `cursor_batch_256/1k` | Same drain in batches of up to 256; final partial batch and EOF included |
| `set_roundtrip/2` | Host set (C: typed JSON) binding, exact set lookup, set returned/decoded |
| `predicate/1k` | 1k foreign predicate callbacks returning the 500 even values |
| `aggregate/1k` | Foreign accumulator init, 1k steps, finish and returned scalar |

Browser adds `binding/browser/indexeddb_write/1k` (one 1k-fact commit including
its write-through flush) and `binding/browser/indexeddb_reopen/1k` (open an
already flushed database). Each persistence sample has a fresh database, with
creation/seeding outside measurement as appropriate; the row count is checked
after measurement. Reopen uses a fresh BrowserDb over IndexedDB in the same
page, not a cold browser/page startup. Each run has a fresh Chromium context.

Fixture creation, query preparation and callback registration are outside all
measurements. Results are checked against fixed counts AND sums, exact point
results or the exact returned set, rather than treating an empty result as
success. Python/Node/browser decoding is part of the round trip. C decodes the
fixed integer result shape itself, checks cursor errors and frees every
returned string/handle. Preparation and foreign callback registration latency
are not measured by these representative cases.

Python and the JS in-memory cases collect ten samples averaging 20 invocations
following three untimed warmups; this amortizes browser clock resolution.
C and browser persistence collect ten single-operation samples. Medians are
serialized as Bencher JSON `latency.value` in **nanoseconds**. There are no
Criterion IDs or fabricated Criterion output: only the executed-ID list is
adapted internally to reuse the existing upload completeness checker.

Run from each binding worktree after building it:

```sh
# No timers, one invocation of every new ID, no metrics file written.
python bench/nightly.py --test          # Python (installed local extension)
node bench/nightly.mjs --test           # Node; browser with pkg/ and Playwright
./target/p16/nightly --test             # C (compile recipe in bench/nightly.c)
```

The core helper `.github/bench/bindings.sh` builds a binding under `binding/`
with a Cargo patch pointing at the exact measured core checkout and verifies
Cargo metadata resolves that source. Release pins/manifests are unchanged.
Artifacts retain both commit hashes, workflow hash, resolved Cargo metadata,
executed IDs, raw metrics, upload output and the completeness report. Bencher
uses `binding-{python,browser,node,c}-ubuntu24-v1` testbeds, Python 3.11,
Node 20 and Playwright 1.55.0 Chromium on Ubuntu 24.04. Native histories are not
inherited. An experimental core or binding ref uses the separate
`binding-experiment/<core-ref>/<binding-ref>` history branch and does not post
production regression issues. Each testbed has its own 0.999 t-test threshold with a 30-point
window; alerts use the existing two-consecutive-run issue policy, while missing
metrics, partial uploads and API failures fail the job immediately.

Reserve **5–15 job-minutes per binding plus build/runtime setup**, unverified.
The job timeout is 60 minutes including builds. Phase B must dispatch serially,
confirm every ID and upload, establish the new testbed histories, and revise
this allowance from actual Actions evidence. No latency or RSS measurement was
performed locally. These latency cases do not enforce RSS bounds.

## JVM lane (#858)

Suite `binding_jvm` uses testbed `binding-jvm-ubuntu24-v1` on Ubuntu 24.04,
Temurin **17.0.17+10** (vendor `Eclipse Adoptium`) and exactly
`-Xms512m -Xmx512m -XX:+UseG1GC`. The environment of the measured JVM records
vendor, full VM version and actual input arguments; the reporter checks them.
The manual #839 harness remains authoritative for operations and exact results.
The six common suffixes above expand under `binding/jvm/`; eight more are:

| Suffix | Timed boundary |
| --- | --- |
| `prepare/point` | Create a prepared point query; validate it outside timing |
| `register_predicate/first` | First predicate registration on a fresh database |
| `register_aggregate/first` | First aggregate registration on a fresh database |
| `fact_log_batch_256/1k` | Open, fully drain and close a 1k-record fact log; includes foreign record conversion |
| `options_error/negative_cache` | Reject a negative cache option and check the error code |
| `options_readonly_open/1k` | Open an already seeded file with read-only/cache/memory options |
| `file_transact/1k` | One 1k-fact file transaction; reopen/validate outside timing |
| `log_writer_append_finish/1k` | Append 1k records and finish; reopened full records checked outside timing |

Short operations use 100 warmups and ten samples of 20 invocations. The two
write cases use five warmups and ten single-invocation samples. JVM startup is
outside these timed calls; JIT warmups do not prove stationarity. JNA, foreign
allocation/conversion and callback overhead remain included. Neither JVM nor
WASI results are engine-only latency.

### Contract for minigraf-java's nightly adapter

The core helper runs from the measured checkout with the Java repository at
`binding/`. **`bench/manual/build.sh CORE_CHECKOUT` owns the Cargo patch**; the
core helper does not create one for JVM. It sets `CARGO_BUILD_JOBS=4`,
`CARGO_TARGET_DIR=<binding>/target`, `CORE_REVISION=BENCH_SHA`, and
`BINDING_REVISION=<actual binding HEAD>`. The manual builder must keep its
metadata assertion before compilation and run `ResultChecksTest`. After it
returns, the helper runs `cargo metadata --locked` in **the binding directory**
again and requires exactly
one `minigraf` package, `source=null`, at the measured checkout's manifest.
This lock is valid: the builder's `cargo update -p minigraf` has already resolved
`binding/Cargo.lock`. A path dependency does not require a separate core lockfile.
The manual builder's existing `target/release` and Gradle copy paths agree.

The core helper unsets `LD_LIBRARY_PATH` for the JVM build, untimed verification
and timed measurement. Hosted `actions/setup-python` exports that variable;
the Java adapter continues to reject native search overrides. After a successful
timed adapter invocation, the helper adds
`launchEnvironment: {"LD_LIBRARY_PATH": null}` to its provenance, recording the
actual unset environment. The reporter requires this field before upload.
This cleanup is scoped to JVM: Python/browser/Node/C and WASI have no corresponding
adapter rejection, and Python keeps its setup-provided runtime environment.

The later binding-repo implementation must provide **`bash bench/nightly.sh`**,
invoked from the binding root after the build. Its only optional argument is
**`--test`**. It inherits `BENCH_SHA`, `WORKFLOW_SHA`, `JAVA_HOME` and
`CARGO_BUILD_JOBS=4`; read the actual binding HEAD with Git. Do not rebuild or
repatch in this entry point. Launch the installed manual harness with the pinned
JDK and flags, preserving #839's timing and exact-result checks. Require successful
process exit and all teardown before publishing anything accepted for upload.
A complete record printed before a later teardown failure is insufficient.

- `--test`: fresh `smoke/` directory, one untimed invocation per case; retain raw
  checks, emit no sample records, `binding-results.json` or
  `binding-provenance.json`. Include a wrong-but-plausible result rejection test
  in the binding adapter's own test suite.
- No arguments: fresh `benchmark/` directory. Preserve the manual stream as
  `benchmark/samples.jsonl` (copy/rename `minigraf-bench.jsonl` only after successful
  process exit). It must pass core `validate.py jvm`: one benchmark environment,
  ten samples per exact ID (`sample=0..9`, `total_ns`, `iterations`, `ns_per_op`),
  one `checked` record per ID and one matching `complete` record.
- Write `binding-results.json` as `{ "binding/jvm/<suffix>": { "latency": {
  "value": N } } }`, with exactly the 14 IDs. `N` is **nanoseconds**, the median
  of the ten `total_ns / iterations` values (average of sorted indices 4 and 5).
  The core reporter recomputes every metric from retained raw samples.
- Print exactly one `ok: binding/jvm/<suffix>` line per ID, only after all
  validation and teardown succeeds. Other output must not start with `ok: `.
  Missing, extra or duplicate IDs/keys and nonpositive/nonfinite metrics fail.
- Write `binding-provenance.json` with `coreSha`, `workflowSha`, `bindingSha`
  (actual 40-character Git SHAs), `artifactPath: "benchmark/libminigraf_ffi.so"`,
  `artifactSha256`, `harnessFiles`, `harnessSha256`, and `runtime` as below.
  Retain at that artifact path the native bytes **actually loaded or extracted
  from the installed harness's packaged JAR**, and verify them against the build
  library. A hash of an unused build-directory library is insufficient. Prevent
  JNA/library-path overrides from selecting another native library.
- `harnessFiles` maps binding-relative source filenames to lowercase SHA-256.
  Include all harness/adapter source and configuration files, at minimum
  `bench/nightly.sh`, `bench/manual/build.sh`, `bench/manual/build.gradle.kts`,
  and `bench/manual/src/main/kotlin/io/github/project_minigraf/benchmark/`'s
  `Benchmark.kt` and `ResultChecks.kt`. Exclude generated build/results files.
  `harnessSha256` hashes the UTF-8 JSON of this map with lexically sorted keys,
  no spaces and no final newline. The reporter rehashes the source files.
- `runtime` is `{ "distribution": "temurin", "vendor": "Eclipse Adoptium",
  "version": "17.0.17+10", "flags": ["-Xms512m", "-Xmx512m", "-XX:+UseG1GC"] }`.
  Derive vendor/version/flags from the actual measured JVM environment, not a
  shell label. Preserve raw `java_vendor`, `java_version` (`17.0.17`),
  `vm_version` (`17.0.17+10`), `jvm_arguments`, `warmup_memory=100`,
  `warmup_storage=5`, and `build.core_revision`/`build.binding_revision`.

Existing output directories or metric/provenance files must fail, never satisfy
another invocation. The core helper also rejects stale files and smoke metrics.
The core reporter fails before upload on missing evidence, mismatched identities,
artifact/harness hashes, runtime configuration, samples or medians.

## WASI lane (#858)

Suite `binding_wasi` uses `binding-wasi-ubuntu24-v1` on Ubuntu 24.04. The core
checkout has no tracked `Cargo.lock`. The workflow first runs
`cargo generate-lockfile`, then builds `minigraf.wasm` from the measured core
with `--locked --release` and
`wasm32-wasip1`, then runs `benchmarks/bindings839/nightly.py`. It wraps the
unchanged `wasi.py` and `validate.py`; smoke and benchmark directories are fresh.
The adapter publishes metrics and `ok:` lines only after successful driver exit
and complete stream validation. `benchmark/samples.jsonl` retains all 30 samples;
metrics are the median of ten sample means in nanoseconds.

| ID | Timed boundary |
| --- | --- |
| `binding/wasi/repl_startup_empty` | New Wasmtime process, uncached compilation, empty REPL and exit |
| `binding/wasi/repl_script/1k` | New process, uncached compilation, 1k-fact transaction and full query output |
| `binding/wasi/repl_query_roundtrip/1k` | Query in a persistent seeded REPL, including pipes, flushing and host line framing |

All three use three warmups and ten samples. Startup/script samples contain one
invocation; persistent-query samples average 20 invocations. Every invocation
checks exact rows, count and sum, successful exit and empty stderr. Persistent
process teardown must also pass. Startup/compilation and pipe/framing overheads
are not subtracted; these are actual REPL measurements.

Wasmtime is pinned to **49.0.2**, official `x86_64-linux` binary SHA-256
`d0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635`
(the binary recorded in #839's retained WASI streams). The checksum is checked
before execution and the version must be
`wasmtime 49.0.2 (3c8a3e79a 2026-10-02)`.
Arguments are exactly `run -C cache=n -C parallel-compilation=n`, followed by the
WASM path: **no preopened directory**. The adapter records core/workflow SHAs,
WASM SHA-256, runtime version/checksum/flags, and individual plus combined
SHA-256 hashes of `nightly.py`, `wasi.py` and `validate.py`.
The generated build lockfile is retained as `binding/Cargo.lock`; provenance
records `cargoLockPath: "Cargo.lock"` (relative to the evidence directory) and
`cargoLockSha256`. The adapter requires the retained copy to match the core
lockfile and checks both before and after execution. The reporter rehashes the
retained bytes before upload and includes the provenance in the final report.
The existing WASM and Wasmtime hashes are still checked before and after each run.
Dependency resolution can vary across fresh runs; the retained lockfile captures
the exact resolution used for each build and permits locked reproduction.

The remaining locked commands have existing lockfiles: JVM metadata uses the
builder's binding lock as above; `cargo install wasm-pack --locked` uses that
tool's published lockfile, not the core's. Python/browser/Node/C metadata runs
after their binding-local `cargo update` without `--locked`. Historical #839
WASI reproduction explicitly copies its retained lockfile before its locked build.

## Rollout, histories and remaining exclusions

Both new lanes share the existing selected = executed = uploaded audit,
0.999 t-test upper boundary and 30-sample window. They inherit no native or other
binding history; v3 uploads use no `--start-point`, so main advances cannot reset
them. Evidence (raw samples, provenance, executed IDs, metrics, native/WASM
artifact, Cargo metadata and upload/report output) is retained for **30 days**.
A durable checksummed archive and measured setup/build/measure durations remain
post-rollout work; no new hosted-runtime timings or cost claims were collected
for this core implementation. The 60-minute timeout is a limit, not a cost estimate.
Changing JDK, Wasmtime, runner configuration or timing boundaries requires a new
testbed version.

#839's maintainer acceptance applies to its recorded physical/dedicated-bench
evidence, not new nightly baselines. Android and Swift/iOS simulator/emulator
lanes stay excluded: #858's matched physical B1/B2/V qualification rejected all 14 cases on
each platform.
Desktop macOS Swift remains excluded. JVM now has representative lifecycle,
options and log conversion routes; other bindings' unmeasured permutations,
wide values, scaling, RSS bounds and deliberate callback failures remain outside
this shortlist. WASI exposes no matching prepared-object or host-callback API.
Browser migration and pinned-reader churn remain manual.

## Operational limits (#858, measured)

Measured from the GitHub Actions and Device Farm records of the study. Anything not
measured is labelled an estimate.

| Lane or step | Runner | Queue | Duration |
| --- | --- | --- | --- |
| `binding_jvm` (scheduled v3 run 44623689870) | `ubuntu-24.04` | none | 4 min 28 s |
| `binding_wasi` (same run) | `ubuntu-24.04` | none | 1 min 23 s |
| iOS simulator qualification (14 cases x 18 runs) | `macos-15` | 10 min 25 s | 1 h 13 min |
| Android emulator qualification (same design) | `ubuntu-24.04` | about 2 s | 27 min |
| Device Farm physical Android series (23 attempts) | Pixel 7 / Android 14 | n/a | 53.6 device minutes |
| Device Farm physical iOS series (25 attempts) | iPhone 15 (iPhone15,4) / iOS 18.5 | n/a | 210.27 device minutes |

- The scheduled v3 run started about 8 hours after its 14:00 UTC slot, so a late
  scheduled start is a normal observed behaviour and the lane does not assume punctuality.
- The first scheduled JVM run raised one alert (`binding/jvm/prepare/point`, 42,824 ns against an upper
  limit of 38,991 ns) against a ten-point baseline; it is a young-history signal, not a regression finding.
- Physical Android units differ: Device Farm may assign another Pixel 7 build; the collector
  rejected four attempts that landed on a different build and retried them.
- Repeatability limits: the qualification gate requires a sample CV of at most 15% and at most twice the
  physical CV, and both candidates failed it or the closeness bound for every case.
- Estimate: one clean physical series is about 40 device minutes on Android and about 150 on iOS (the
  measured totals include retries and the wasted first iOS batch).
