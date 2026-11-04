# Manual runtime evidence (#839)

The [scope and coverage plan](../../docs/benchmarks/839-bindings/README.md) and
[initial results](../../docs/benchmarks/839-bindings/results.md) distinguish these
manual runs from existing GitHub nightly benchmarks. No release acceptance or
latency threshold is implied.

## WASI reproduction

Use a clean core checkout at the recorded revision and Rust 1.99.1. Copy the
retained `wasi-Cargo.lock` into that disposable checkout as `Cargo.lock`, then:

```sh
rustup target add wasm32-wasip1
cargo build --locked --release --target wasm32-wasip1 --bin minigraf
python3 -m unittest discover -s benchmarks/bindings839 -p test_wasi.py
python3 benchmarks/bindings839/wasi.py --runtime /path/to/wasmtime-49.0.2 \
  --wasm target/wasm32-wasip1/release/minigraf.wasm \
  --core-revision 33e5dffadebe0124e6287fc91cede4ff2b7a2b2f \
  --output /fresh/output/smoke --mode smoke
```

The script requires a UTF-8 process locale, as used in the recorded runs. Run
the `benchmark` mode only on the dedicated bench, serially with CPU and memory
affinity on one NUMA node and no concurrent builds. Use three fresh output
directories. Record idle checks and toolchain/artifact hashes externally as well
as the emitted environment record: `--core-revision` is an asserted label, not
proof of a source build. The retained series additionally verifies all core
compilation inputs and the actual WASM/driver against their recorded hashes.

Startup includes uncached compilation. A persistent query includes pipe and
line-framing overhead. Results must have exact rows, count and sum, with successful
exit and empty stderr. This is the actual REPL contract, not a prepared/callback
binding API. Run `validate.py wasi OUTPUT/samples.jsonl` after each timed series.

## JVM and Android

Their binding repositories contain the source-build scripts and harnesses under
`bench/manual` and `bench/device-farm`. Run the JVM installed application in three
fresh processes with `-Xms512m -Xmx512m -XX:+UseG1GC` (the generated launcher sets
these), on the same idle NUMA-pinned bench. The read-only-open case deliberately
uses the short-operation policy: 100 warmups and 20 invocations per sample. The
two write cases use five warmups and one invocation per sample. Other JVM cases
use the short-operation policy too. The protocol records ten samples per case.

Android defaults to smoke when no instrumentation argument is supplied. A green
provider result alone is insufficient: use `-e mode benchmark` and validate the
returned JSONL with `validate.py android OUTPUT.jsonl`. The validator rejects
smoke-only, incomplete, duplicate and inconsistent timing evidence. Android uses
three warmups; this does not establish ART compilation/thermal stationarity.

The Kotlin harnesses fail on action or teardown errors. A second teardown failure
can obscure the original exception, but cannot turn failure into a valid sample.
Negative cache validation occurs in the binding before opening the file; the
pinned shim's `options.to_core()?` precedes `Minigraf::open_with_options`.

All result decoding and structural checks are part of the named foreign round
trip. Expected-value comparisons, setup and reporting are outside the clock.
Validate JSONL completeness and verify the packaged native artifact, not only a
build-directory library, before accepting or publishing a run. Preserve failures
and every sample; keep private machine/provider details out of public artifacts.
