# Shared committed reader evaluation (#797)

Evaluation date: 2026-10-30. Base: `15badca89db60bbd458b52af59a064b9c0e7da1f`
on `v3`. Design: `docs/superpowers/specs/2026-10-30-797-shared-reader-design.md`.

## Verdict

**Neither candidate passes. No production change is proposed.**

- **H** (shared non-generic `ReaderCore`, `patches/shared-reader.patch`)
  passes the size gate and every correctness gate, but fails the primary
  runtime gate. In the second envelope (opt-level 3) campaign, the upper
  bounds for attribute/value and `:as-of` point queries are 1.0245 and 1.0260,
  against a limit of 1.02. The B:B control in the same campaign stays within
  0.7% (0.9937–1.0022), so this is a measured slowdown of about 1–2% on envelope point
  queries, not noise. In the release (opt-level z) configuration, H costs at
  most 0.6% (upper bound ≤ 1.0092).
- **P** (the #772 boxed-reader prototype,
  `docs/audits/772/evidence/patches/reader-erasure.patch`) saves more bytes and
  passes the primary gate. It fails the secondary gate on direct reader
  construction (`con_direct`, ratio 1.166, upper bound 1.174 native; 1.119 /
  1.130 under WASI): the boxed reader makes two allocations per construction, B one. The spec
  needs a new maintainer decision to ship P in any case.

Two more categories are outstanding for both candidates (see Limitations):
the cold category, under the residency rule fixed before acceptance, and the
CI-only checks a PR would run.

## Sizes (AC 6)

`sizes.sh` (the #772 `matrix.sh` method) on each variant's commit, one
`Cargo.lock` (SHA-256 `82fa4d92e6389dc4a44d47bac6fdc361ffb4e736433735da140624eb40807660`).
Bytes; `evidence/sizes-{B,H,P}.tsv` has the artifact hashes.

Artifact | B | H | P | H − B | P − B
---|---:|---:|---:|---:|---:
native CLI (release, stripped; the size gate's artifact) | 1,259,384 | 1,256,096 | 1,251,936 | **−3,288** | −7,448
WASI CLI | 876,947 | 876,884 | 876,849 | −63 | −98
browser raw Wasm | 1,582,757 | 1,582,447 | 1,582,014 | −310 | −743
browser wasm-bindgen Wasm | 1,001,653 | 1,001,392 | 1,001,017 | −261 | −636
browser wasm-bindgen JS | 43,712 | 43,712 | 43,712 | 0 | 0
library consumer, opt-level 3, stripped | 2,150,336 | 2,137,128 | 2,129,960 | −13,208 | −20,376
library consumer, opt-level 3, unstripped | 4,737,512 | 4,722,448 | 4,710,704 | −15,064 | −26,808
library consumer, release (opt-level z) | 1,459,888 | 1,459,936 | 1,459,952 | +48 | +64

H passes AC 6: the native CLI is 3,288 bytes smaller, and no AC 6 artifact
grows. The opt-level-z library consumer is not an AC 6 artifact; it is listed
because it grows for both candidates.

## Correctness gates (AC 8–11), H

`gates.sh` on H's commit `1b5169ef8e31e3f1bf848d52ffc2e08f3c17068f`
(`evidence/gates-H.tsv`; logs are not kept, since they hold bench paths):

Step | Result
---|---
`cargo test --all-features` | pass: 2,074 passed, 0 failed, 20 ignored
`test-hooks` suites | pass: 2,074 passed, 0 failed, 20 ignored
CI-toolchain clippy (CI's arguments) | clean
clippy `--all-targets` | fails identically on B: the same targets with the same error counts (3,753 log lines each), pre-existing lints in tests, examples and benches
`cargo fmt --check` | clean
`cargo +1.89.0 check --all-features --locked` | clean
clippy, wasm32 browser and WASI | clean
WASI tests under Wasmtime 49.0.2 | pass (782 + doc/integration suites)
BrowserDb headless Firefox | pass: 63 passed
`minigraf_shuttle` deterministic suite | pass: 3 passed (seed 760, 1,000 iterations)
coverage gates (nightly-2026-10-01) | pass; `src/storage/reader.rs` 98.45% lines / 94.74% branches (floors 97/94), and the extracted logic stays in that file

`patches/entry-point-tests.patch` adds direct tests of the reader entry points
that had none (AC 5). These tests pass on both B and H. The gate run above
predates that commit; the tests were run on their own on both variants.
Not run: the Chrome browser suite, macOS/Windows native, and `semver-checks`,
which only a PR's CI provides.

## Runtime (AC 12–14)

### Method

- **Variants.** B, H and P are built from `15badca89` with the same lock,
  toolchain (`evidence/compiler.txt`: rustc 1.99.1, LLVM 23.1.1), features and
  profiles. Every variant gets the same measurement hooks under
  `--cfg minigraf_ab` (`ab-hook.patch`): committed-memory WASI reader
  construction and a direct reader-construction entry point. These hooks are
  measurement-only. `raw/manifest.json` records each variant's commit, lock
  hash, `reader.rs` hash, binary hashes and build commands, plus the
  harness/hook hashes and the fixture checksum.
- **Harnesses.** `reader_ab.rs` covers native and WASI, and `ab_browser.rs`
  covers BrowserDb in headless Firefox via `wasm-bindgen-test-runner`
  0.2.130, with `webdriver.json` for full-precision timers. The fixture is
  100,000 facts over 16,666 entities, built by B's `perf_envelope`
  (SHA-256 `59e0023aad11…`, in the manifest).
- **Unit and estimand.** The unit is one process block. Each block runs a
  warm-up and at least 1 s of timed work per workload, then validates and
  hashes the complete results. The estimand is the candidate's mean ns/op over
  the base's (for `cc_*`, wall time per operation), compared as natural logs.
  The bound is the one-sided 95% Student t upper bound on the mean log ratio,
  exponentiated (`analyze.py`). Every block's result digests must match B.
- **Placement.** Paired campaigns run the base and the candidate at the same
  time, one on NUMA node 1 and one on node 3 (`numactl` CPU and memory
  binding), swapping every block. Pair order rotates. Cold, concurrency and
  browser campaigns run one process at a time, alternating the order.
- **Fixed in advance.** Block counts, the analysis method and the campaign
  order were fixed from the pilots (`raw/pilot-*.jsonl.gz`, not counted)
  before any acceptance run, and committed as `acceptance-plan.json`: primary
  128 blocks; secondary, cold and concurrency 48; WASI and browser 32. Each
  campaign ran once, from a script that waited for a 1-minute load average
  below 2. `run.py` refuses to overwrite a raw file, so no campaign was
  rerun.
- **Primary families.** `pt_ea` (entity/attribute), `pt_av`
  (attribute/value) and `pt_asof` (`:as-of` transaction counter). Each covers
  fixed and varied keys and existing and missing entities, through ordinary
  queries, prepared queries and cursors pulled in several batches.
  Profiles: `envelope` (the opt-level-3 consumer) and `release` (the gate's
  configuration).

### Primary gate (AC 13): pt families, B:H upper bound ≤ 1.02 in each campaign

Campaign | pt_ea | pt_av | pt_asof | B:B control range (lower–upper)
---|---:|---:|---:|---
primary-1 envelope | 1.0155 | 1.0172 | 1.0179 | 0.9959–1.0050
primary-1 release | 1.0067 | 1.0058 | 1.0049 | 0.9957–1.0070
primary-2 envelope | 1.0199 | **1.0245** | **1.0260** | 0.9937–1.0022
primary-2 release | 1.0092 | 1.0071 | 1.0044 | 0.9924–1.0017

The B:P upper bounds are at most 1.0055 in envelope and 1.0161 in release.
`evidence/tables.md` has the ratios, p50/p99, allocations and RSS.

### Secondary gate (AC 14): every other workload, upper bound ≤ 1.05

H passes every secondary workload. Its highest upper bounds:

Workload | B:H ratio | upper
---|---:|---:
cc_44 (44 threads, inverse throughput) | 1.034 | 1.047
cold_point | 0.990 | 1.027
sh_attribute_value | 1.019 | 1.026
cc_8 | 1.010 | 1.021
sh_point_entity | 1.013 | 1.021

P fails on `con_direct`: native 1.166 (upper bound 1.174), and WASI
`wasi_committed_con_direct` 1.119 (upper bound 1.130). All of P's other
workloads pass.

No campaign has a result-digest mismatch, for either candidate.

### Per-workload tables

`evidence/tables.md` (from `report.py`) covers every campaign, workload and
pair, with the blocks, ratio, upper bound, verdict, mean, p50/p99,
allocations per operation and RSS. `evidence/summary.json` holds the
analyzer's output.

## Limitations and deviations

- **Cold residency (AC 14).** `acceptance-plan.json` requires zero resident
  pages after eviction for every cold observation. This was not met.
  Residency was checked after every eviction (`posix_fadvise(DONTNEED)`, then
  `mmap` and `mincore` on the copy), but the harness recorded the sum over a
  process's observations. Per observation (sum ÷ observations), point queries
  leave a median of 1.1 of 2,628 pages resident (worst process 148) and scans
  leave 7.9, likely the pages the open handle mapped or touched. Under the
  fixed rule, the cold category is outstanding for both candidates. The
  latency ratios are reported but do not count as a pass. This changes no
  verdict: H already fails the primary gate.
- `analyze.py` was changed after the campaigns in two ways, neither touching
  the ratio, bound or digest computation. It now reads gzipped raw files, and
  it reports residency per observation instead of the maximum of per-process
  sums.
- The browser harness reports no allocations or RSS (they read as 0). It
  also does not cover prepared queries.
- Bench machine (non-identifying): Linux 6.8 x86-64 (Ubuntu 24.04), two Intel
  Xeon E5-2696 v4 sockets, 44 cores and 88 threads with SMT on, 4 NUMA nodes.
  The `schedutil` governor was used, with transparent huge pages set to
  `madvise`. The fixture was on ext4 on a hardware-RAID volume that the kernel
  reports as rotational. Firefox 158.0 with geckodriver 0.37.1, Wasmtime
  49.0.2 and Python 3.12. Wasmtime, wasm-pack and wasm-bindgen are user-local
  and are removed after the evaluation, with the fixtures and temporary
  directories.

## Files

- `patches/shared-reader.patch`: H, as measured (`1b5169ef8`).
- `patches/entry-point-tests.patch`: direct entry-point tests; pass on B and H.
- `reader_ab.rs`, `ab_browser.rs`, `ab-hook.patch`, `webdriver.json`: harnesses.
- `run.py` (build and campaigns), `analyze.py`, `report.py`, `sizes.sh`,
  `gates.sh`, `wasm_funcs.py` / `elf_funcs.sh` (size attribution).
- `specs/*.json`, `acceptance-plan.json`: campaign definitions and the plan
  fixed in advance.
- `raw/*.jsonl.gz`: every process's reports, by campaign, including pilots.
  `raw/*.spec.json` are the specs each run used. `raw/manifest.json` has the
  pinned inputs.
- `evidence/`: sizes, gate status, compiler versions, `summary.json`,
  `tables.md`.

Reproduce the analysis:

```sh
python3 analyze.py $(ls raw/*.jsonl.gz | grep -v pilot) > summary.json
python3 report.py summary.json
```
