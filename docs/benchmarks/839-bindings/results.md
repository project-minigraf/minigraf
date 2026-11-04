# Initial measurements (pending acceptance)

Each cell is the median of ten sample means, in milliseconds. Runs remain separate.
These runtimes and operations are not equivalent workloads or a ranking.

| Runtime / case | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| binding/android/aggregate/1k | 75.4532 | 75.3482 | 77.9875 |
| binding/android/cursor_batch_1/1k | 106.1472 | 99.5056 | 103.3633 |
| binding/android/cursor_batch_256/1k | 2.4811 | 2.4759 | 2.5111 |
| binding/android/fact_log_batch_256/1k | 1.8159 | 1.7982 | 1.9574 |
| binding/android/file_transact/1k | 8.1239 | 10.7207 | 8.3775 |
| binding/android/log_writer_append_finish/1k | 23.3979 | 22.9484 | 24.0464 |
| binding/android/options_error/negative_cache | 0.2207 | 0.2014 | 0.6745 |
| binding/android/options_readonly_open/1k | 0.2461 | 0.2521 | 0.2523 |
| binding/android/predicate/1k | 79.7311 | 80.9272 | 80.5085 |
| binding/android/prepare/point | 0.1579 | 0.1553 | 0.1541 |
| binding/android/prepared_execute/1k | 1.2188 | 1.1788 | 1.1830 |
| binding/android/register_aggregate/first | 0.2159 | 0.2128 | 0.2264 |
| binding/android/register_predicate/first | 0.2125 | 0.2217 | 0.2176 |
| binding/android/set_roundtrip/2 | 0.3181 | 0.2826 | 0.2812 |
| binding/jvm/aggregate/1k | 23.0916 | 22.0246 | 22.6345 |
| binding/jvm/cursor_batch_1/1k | 29.1728 | 28.2909 | 28.7563 |
| binding/jvm/cursor_batch_256/1k | 1.2623 | 1.5953 | 1.3053 |
| binding/jvm/fact_log_batch_256/1k | 1.2636 | 1.1714 | 0.6269 |
| binding/jvm/file_transact/1k | 3.0063 | 3.0693 | 3.0600 |
| binding/jvm/log_writer_append_finish/1k | 6.1150 | 5.9070 | 6.2349 |
| binding/jvm/options_error/negative_cache | 0.0595 | 0.0568 | 0.0606 |
| binding/jvm/options_readonly_open/1k | 0.0723 | 0.0705 | 0.0740 |
| binding/jvm/predicate/1k | 25.3230 | 24.0100 | 23.7096 |
| binding/jvm/prepare/point | 0.0375 | 0.0354 | 0.0443 |
| binding/jvm/prepared_execute/1k | 0.7129 | 0.6957 | 0.6966 |
| binding/jvm/register_aggregate/first | 0.0443 | 0.0421 | 0.0430 |
| binding/jvm/register_predicate/first | 0.0414 | 0.0418 | 0.0410 |
| binding/jvm/set_roundtrip/2 | 0.0956 | 0.0953 | 0.1209 |
| binding/wasi/repl_query_roundtrip/1k | 11.1007 | 11.2152 | 12.2706 |
| binding/wasi/repl_script/1k | 1066.5595 | 1070.5115 | 1135.4664 |
| binding/wasi/repl_startup_empty | 1061.1044 | 1061.9715 | 1073.6101 |

Raw samples, individual sample ranges, coefficients of variation and per-case warmup counts
are retained in `evidence/`. No outlier was removed. The read-only-open JVM case uses
100 warmups and 20 operations per sample, like other short operations; only the two
write cases use five warmups and one operation per sample.

Android model/OS is pinned, not a reserved physical unit. Device and host condition snapshots
do not prove uninterrupted idle/thermal stability. Some JVM and Android cases show large
within-run or between-run spread; these results do not establish stationary performance.

Host package-vendor/version fields that disclose the private OS distribution are omitted
from the public JVM environment records; Java version, JVM flags and all samples remain.
The sample manifest retains hashes of both original and public JSONL. Full original logs,
APKs/jars/WASM, source snapshots and private conditions are retained outside the repository.

WASI provenance was checked after the run against all 93 tracked compilation inputs
(src, Cargo.toml and any build.rs) from the pinned core commit, the retained Cargo.lock,
WASM and executed driver. All matched. The actual driver locale was UTF-8. Android and
JVM packaged native library hashes matched their build-output hashes in this series.

The measured WASI driver is preserved in core commit
`3d43c993a61219f67e8e6131115bd7c86aa36646` (driver SHA-256 in the provenance).
The current driver additionally rejects Python `-O` with an unconditional error;
the recorded runs used normal Python. This guard-only change is outside all
timed regions, and the original samples and driver identity are retained.
