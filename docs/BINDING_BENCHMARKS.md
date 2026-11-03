# Binding benchmarks (#776 P16)

These are foreign-runtime measurements, independent of native Criterion data.
The `bindings` matrix in `.github/workflows/bench.yml` selects the whole fixed
shortlist in each binding's `bench/nightly.*` entry point. Core and binding PRs
must land before enabling the default-v3 route. The default-main workflow needs
its identical companion; schedules then select v3 at 14:00 UTC. The 02:00 main
checkout skips all four jobs before building or uploading. Manual runs may use
`binding-ref` to select the common `bench/776-p16-bindings` PR branch during
phase B; no dispatch is part of phase A.

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
inherited. An experimental `binding-ref` uses a separate
`binding-experiment/<core-ref>/<binding-ref>` history branch and does not post
production regression issues. Each testbed has its own 0.999 t-test threshold with a 30-point
window; alerts use the existing two-consecutive-run issue policy, while missing
metrics, partial uploads and API failures fail the job immediately.

Reserve **5–15 job-minutes per binding plus build/runtime setup**, unverified.
The job timeout is 60 minutes including builds. Phase B must dispatch serially,
confirm every ID and upload, establish the new testbed histories, and revise
this allowance from actual Actions evidence. No latency or RSS measurement was
performed locally. These latency cases do not enforce RSS bounds.

## Deliberate dispositions requiring release sign-off

- Java/JVM: no JVM latency history yet. Python exercises the shared UniFFI
  adapter, but does not measure JVM allocation, JNI or JVM callbacks. JVM
  timings remain a manual release check, not claimed nightly coverage.
- Android: ART/ABI and device storage timing require a fixed physical device;
  emulator timings on a hosted Linux runner are not a substitute. Manual
  device benchmarks and release sign-off remain required.
- Swift: ARC/callback and Apple-device storage timing require stable Apple
  hardware. Manual macOS/iOS benchmarks and release sign-off remain required;
  Linux UniFFI tests do not provide these timings.
- WASI: browser Wasm does not measure the WASI runtime. Tier 2 WASI REPL startup
  and runtime overhead remain manual; it exposes no matching foreign prepared
  object or synchronous host callback API in the audited package.
- Options validation/error paths and fact-log/LogWriter conversions are outside
  this shortlist. Native tests/benchmarks cover engine work, not these foreign
  conversions. They remain explicitly unmeasured follow-up scope. Browser
  persistence coverage is write-through/open only; migration and pinned-reader
  churn remain manual. Deliberate callback errors stay correctness tests.

G15 therefore gains scheduled representatives for four host runtimes; the
listed manual/excluded paths need explicit release sign-off before #776 closes.
