# Release binary size audit (#772)

Audit date: 2026-10-29. Baseline: `3d19abbce5dd8d5244641ff0d7ba8bad5424d07f`
on `v3`. This is an investigation with disposable prototypes, not a production
optimization patch. No public API, format, error policy, feature, size cap or
coverage threshold is changed.

## Reproduction and measurement boundaries

Measurements run on `s1c44t88`, Linux x86-64, two Intel Xeon E5-2696 v4 CPUs
(44 physical cores, 88 threads), with Rust/Cargo 1.99.1 and LLVM 23.1.1.
The release profile uses `opt-level="z"`, full LTO, one codegen unit,
`panic="abort"`, and symbol stripping. `dist` inherits release but uses thin
LTO. `envelope` inherits release but uses opt-level 3 and keeps symbols.

The native target is `x86_64-unknown-linux-gnu`; browser is
`wasm32-unknown-unknown` with `browser`; WASI is `wasm32-wasip1` with default
features. The `wasm` feature is deprecated and does nothing. Native `browser`
enables a dependency but does not expose the browser implementation.

The issue's historical cap was 1,259,520 bytes, with 736 bytes free after #771.
At this audit's baseline the checked-in cap is 1,263,616 bytes (#723), with
only **24 bytes free**. Both numbers are recorded rather than silently
comparing different revisions. This audit does not propose raising either cap.

Use the attached dependency lock snapshot as `Cargo.lock` in a fresh checkout
of the baseline. It is intentionally named `dependency-lock.toml`, since the
repository does not track its root Cargo.lock. The lock SHA-256 is
`08a1495662974810709834e9a3530faa0f1f63656020f868767a75bc20fd625f`.

```sh
git checkout --detach 3d19abbce5dd8d5244641ff0d7ba8bad5424d07f
cp /path/to/audit/dependency-lock.toml Cargo.lock
bash /path/to/audit/matrix.sh /tmp/772-results
python3 /path/to/audit/prototypes.py "$PWD" /tmp/772-work /tmp/772-probes
```

Use Rust 1.99.1 for numerical reproduction. Do not inherit `RUSTFLAGS`, profile
overrides or a different target. The scripts record compiler, feature tree,
manifest, artifact hashes and exact sizes. Prototype output directories must
not already exist. Each prototype starts from the baseline, uses `--locked`,
and restores its source before the next build. The Chrono feature-only probe
uses offline resolution because removing its serde edge requires a lockfile
update; no package versions changed. Savings are not additive.

The `rlib` (15,064,724 bytes) is an intermediate archive containing linkable
code and compiler metadata. The native `cdylib` (311,112 bytes) has no exported
database ABI (`nm -D --defined-only` is empty). Neither is a useful estimate
of an embedded application's linked database cost. The existing
`perf_envelope` example is used as a reproducible library consumer, with its
harness cost included, under both size and speed profiles.

Browser measurements include both raw Cargo Wasm modules and the result of
`wasm-bindgen 0.2.129 --target web --out-name minigraf` (no wasm-opt or
compression). The official tool archive checksum is
`82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e`.
The packaged baseline is 994,445 bytes of Wasm plus 44,633 bytes of JavaScript;
the JavaScript and TypeScript declaration files retain their sizes, with
linker-assigned internal function indices changed in both probes. Their
contents are identical after normalizing those function indices and the
`shim_idx` numbers in generated comments. WASI's CLI uses an in-memory
database; it does not exercise the native file backend. macOS, Windows, ARM,
binding package sizes and browser execution were not measured here.

## Attribution and code review

For attribution only, rebuild with `CARGO_PROFILE_RELEASE_STRIP=none`, then
run `nm -S --size-sort -C` and `size -A`. Keep the optimization settings fixed.
The stripped gate artifact remains the authoritative size measurement.

The attribution build contains 893,646 bytes of `.text`, 101,440 of `.rodata`,
150,848 of `.eh_frame`, 21,060 of `.eh_frame_hdr`, and 33,688 of `.data.rel.ro`.
Symbol-name grouping assigns 166,846 text bytes to query engine names, 151,178
to Datalog, 107,207 to storage, 39,907 to graph types/store, and 384,370 to
other runtime/dependency names. This is attribution by symbol spelling, not
an exclusive source-level accounting: inlining, generic arguments, aliases,
padding and linker decisions prevent that interpretation.

| Area inspected | Finding and disposition |
| --- | --- |
| Parser, expressions, recursive execution, optimizer, aggregate/window engine | Large functions are real supported semantics. Keep exact summation, recursion, error checks, and stable selectivity/cost sorting. Window ordering already shares one comparator; error construction is already cold and out of line. |
| Query plan imports, prepared slot names, set ordering | Isolated sort probes below. Unique names and byte-identical values offer safer opportunities than semantic query tie order. Import-slot uniqueness needs explicit confirmation before shipping that probe. |
| Graph pending indexes and value representations | Pending and committed encodings have different layouts and semantics. Unifying them for size risks compatibility, signed-zero/NaN behavior and query correctness. No speculative container replacement. |
| B+tree, node codec, DICT, reader, cache, COW/free list | Tree operations already mostly accept `dyn StorageBackend`; reader wrappers still specialize for file and memory backends. Preserve checked decoding, CRCs, pinning, quarantine, and cache locking. Probe reader type erasure and free-ID sorting separately. |
| Persistence, migration, large transactions, WAL, fact log, log writer | Keep legacy readers, stable same-transaction replay ordering, recovery checks, durability syncs and streaming bounds. These are required behavior, not removable size overhead. Legacy sorting is already shared across backend types. |
| CLI/REPL and public library entry points | Inspect actual linked consumers, not archive sizes. Main includes inlined initialization work; its symbol size is not solely argument parsing. Release and distribution profiles produce materially different artifacts. |
| Browser buffer, IndexedDB, bindings and UDF conversion | Delete lists already use unstable integer sorts. Promise callbacks have different IDB lifecycle/error responsibilities. Removing JSON trees or sharing callbacks needs a separate compatibility/performance design; no unmeasured savings claimed. |
| WASI and platform backend code | Native locking/fsync/WAL are cfg-excluded from Wasm. Keep portability paths and platform-specific error behavior. Native savings cannot simply be assigned to WASI. |
| Dependencies and generated runtime code | Chrono feature probe below. `regex-lite` implements public predicates; backtrace machinery supports diagnostics. Removing regex, chrono, formatting or backtraces is not accepted merely because their symbols are large. |
| Tests, examples, Kani/grammar/model-checking support | Test-only code and dev dependencies do not justify production deletions. Keep all test and coverage gates. Consumer examples do unify dev-dependency features, so their sizes are reported separately. |

## Rejected directions without prototypes

- Remove stable legacy fact ordering: same-transaction replay must preserve
  input order; a smaller binary cannot justify changing stored history.
- Replace optimizer stable sorts with unstable sorts: tied clause order is a
  documented behavior and can affect planning and error precedence.
- Merge pending and persisted key formats, remove migrations, relax corruption
  checks, or remove fsync/locking: violates reliability and stability.
- Disable backtraces or unwind metadata with private dependency cfgs or build
  flags: changes diagnostics and portability, and creates unsupported build
  coupling. Their large attribution is not an actionable saving under #772.
- Hand-write timestamp/JSON/regex replacements: no measured safe saving;
  substantially larger semantic and maintenance burden than reusing primitives.

## Evidence and follow-ups

Each source probe is independent. Positive savings mean fewer bytes than the
1,263,592-byte native default release baseline.

| Probe | Native bytes | Saving | Runtime, memory and maintenance tradeoff |
| --- | ---: | ---: | --- |
| Share `OnDiskReader` through a boxed backend trait object | 1,256,144 | 7,448 | One extra heap allocation per reader, trait-object dispatch; no new public API or storage algorithm. Test hot reads and reader creation before adoption. |
| Use existing unstable byte-vector sort in `keys::set_elements` | 1,260,568 | 3,024 | Equal byte vectors are indistinguishable and immediately deduplicated. Removes stable-sort scratch storage; same worst-case O(n log n), but distribution-dependent constants still need measurement. |
| Unstable sort for plan imports by slot | 1,262,936 | 656 | Removes stable scratch storage; verify that ties are identical imports, not distinct aliases, before adoption. Small saving. |
| Unstable sorts for unique prepared/branch variable names | 1,263,240 | 352 | Names come from sets/maps; equal entries have no observable tie identity. Removes scratch storage; small saving and possible distribution-dependent timing differences. |
| Ascending free-ID sort followed by reverse | 1,263,256 | 336 | **Reject:** adds an O(n) pass and measured 175% slower on 100K already-descending IDs; 336 bytes does not justify the regression. |
| Unstable set-value comparison sort | 1,263,688 | **-96** | Reject: increases binary size even though stability is unnecessary for equal values. |
| Disable Chrono's serde feature | 1,263,592 | **0** | Reject as a binary-size optimization: no shipped-byte reduction. Dependency-edge cleanup is separate work. |

The profile comparison is separate: switching the `dist` CLI from thin to full
LTO yields the existing release artifact, saving **239,296 bytes (15.9%)** from
the 1,502,888-byte distribution baseline. This is not 239 KB of headroom in
the current gate: that gate already uses full LTO. Build-time and runtime
tradeoffs require their own review.

| Artifact | Baseline | Set-encoding probe | Saving |
| --- | ---: | ---: | ---: |
| Native default CLI | 1,263,592 | 1,260,568 | 3,024 |
| Native `wasm` feature CLI | 1,263,592 | 1,260,568 | 3,024 |
| Native `browser` feature CLI | 1,263,576 | 1,260,552 | 3,024 |
| Distribution CLI, thin LTO | 1,502,888 | 1,498,776 | 4,112 |
| WASI CLI | 879,785 | 878,835 | 950 |
| Raw browser Wasm | 1,525,738 | 1,524,903 | 835 |
| Packaged browser Wasm | 994,445 | 993,569 | 876 |
| Library consumer, release | 1,464,112 | 1,461,072 | 3,040 |
| Library consumer, envelope, original symbols | 4,749,544 | 4,744,624 | 4,920 |
| Library consumer, envelope, stripped for comparison | 2,161,280 | 2,157,120 | 4,160 |

The reader probe yields 879,687 bytes on WASI (98 saved), 1,524,995 raw
browser bytes (743 saved), 993,809 packaged browser bytes (636 saved), and
2,140,888 stripped envelope-consumer bytes (20,392 saved). The native gain is
larger because both file and memory backends are linked. Do not extrapolate
the native saving to single-backend builds.

Raw browser sections are dominated by code (1,047,465 bytes), custom metadata
(251,889), exports (107,688) and data (105,632). The matching wasm-bindgen pass
removes packaging-only metadata/exports; those are not production-code
reduction opportunities. WASI has 770,451 code and 105,393 data bytes.


## Runtime and memory checks

`sort_probe.rs`, compiled with `rustc -O` on Rust 1.99.1, compares the exact
sort algorithm choices at 8, 64, 1,024 and
100,000 elements, with deterministic random, ascending, descending and
8-value repeating inputs. Each output is checked against an independently
sorted result. It excludes input cloning/output checking from its timer.
Four paired runs used NUMA nodes 1 and 3, swapping them each round, with no
competing builds/tests. These are algorithm measurements, not query latency
claims. Raw nanoseconds per sort are in `evidence/sort-*.csv`.

At 100K elements, the byte-vector unstable sort was 27.3% faster for random
input, 22.0% faster for repeated values, 2.1% faster for descending input and
0.8% slower for ascending input (median paired ratios). At 1,024 elements,
random/repeated bytes were 12.7%/35.4% faster. This supports following up the
encoded-set change, while retaining end-to-end set workloads as a shipping
gate. Stable sorting's temporary element storage disappears; these numbers
do not quantify whole-process allocation peaks.

The free-ID ascending-plus-reverse probe was **174.9% slower** at 100K
already-descending IDs, and 13.2% slower on repeated IDs; at 1,024 descending
IDs it was 66.5% slower. Reject it for a 336-byte saving. Unique-string sorting
was 24.2% faster at 100K random names and within 2.3% on ascending/descending
inputs, supporting a small, separate metadata-sorting follow-up.

`compare.py` used the existing opt-level-3 envelope consumer against a common
100K-fact fixture in bench tmpfs. It ran 17 warm-read shapes in four paired
rounds, swapping NUMA nodes 1/3, with the harness checking expected counts and
both variants returning the same row count in all 68 comparisons. This is a
query/read comparison, not a storage-write or cold-I/O benchmark. Apart from
the short point probe discussed below, median paired p50 changes ranged from
-1.28% (count) to +2.30% (transaction-counter time travel). These observations
do not establish a universal performance bound.

The first point-entity result was +5.08%, with individual paired ratios ranging
from 0.980 to 1.177. Because 200 samples of such a short query take only a few
milliseconds, the apparent regression required a focused rerun rather than a
blanket no-regression claim. Raw four-round evidence is in
`evidence/reader-runtime.json`. A focused 32-pair rerun of point, attribute/value
and transaction-counter time travel gave median p50 changes of **+3.91%**,
**+0.84%**, and **+2.02%**, respectively (`evidence/reader-point-runtime.json`).
The point cost is a real concern near the 5% budget, not a clean no-regression
result. Keep reader sharing conditional on a longer point/reader-creation
study and full storage/concurrency validation. An extracted non-generic helper
could be preferable to an allocation per reader.

`cli_compare.py` compared the existing thin-LTO distribution executable with
the full-LTO release executable in eight socket-swapped pairs. Each process
ingested 2,000 facts and executed 400 counts; every count was checked as 2,000.
Full LTO reduced elapsed time by a median **9.09%** (all eight pairs favored
it). This includes startup, ingestion and printing, and does not represent
all CLI workloads. See `evidence/cli-runtime.json`. Cross-platform build-time
and runtime costs remain gates for #796.

## Verification boundaries

- `cargo test --all-features` on the unchanged baseline, with
  `TMPDIR=/dev/shm/minigraf-772-tests`: exit 0. Its 80 printed test summaries
  total 2,072 passes, zero failures and 20 ignored (includes child-process
  test invocations). This is not a replacement for the disk durability gate.
- The initial disk-backed baseline run was interrupted after it spent its
  remaining time in filesystem journal waits. It is not counted as passing.
- Set prototype: all 23 `storage::keys::tests` and all 10
  `set_values_test` tests passed; the latter used disk-backed temp fixtures,
  covering checkpoint/reopen/WAL, retraction, large transactions, fact-log
  copying, plan independence, signed zero and rejected literals/oversize sets.
- Reader prototype: both `storage::reader` unit tests passed (order probing
  and UUID batching). Broader storage/transaction/concurrency coverage is a
  prerequisite for its implementation PR, not implied by those two tests.
- Both source prototypes passed `cargo +1.89.0 check --all-features --locked`.
- Native/browser/WASI builds and wasm-bindgen packaging succeeded as tabulated.
  Browser execution and non-Linux platform tests remain implementation gates.
- No production source is changed by this audit. Coverage floors, CI size caps,
  runtime dependency declarations and file format remain untouched.


## Independently reviewable follow-ups

Recommended order: the small set-encoding change first, the distribution
profile review second, conditional reader sharing third, and metadata sorts
last. This ranks safety and measured impact together.

- [#794: reuse the unstable byte-vector sort for set encoding](https://github.com/project-minigraf/minigraf/issues/794).
- [#795: evaluate smaller sorts for unique query metadata](https://github.com/project-minigraf/minigraf/issues/795).
- [#796: align distribution LTO and size policy with shipped artifacts](https://github.com/project-minigraf/minigraf/issues/796).
- [#797: evaluate shared readers without regressing point lookups](https://github.com/project-minigraf/minigraf/issues/797).

These are implementation/evaluation issues, not claims that the prototypes are
ready to merge. Each records the measured baseline, tradeoffs and required
shipping checks. The actual release head must be remeasured when implemented.
