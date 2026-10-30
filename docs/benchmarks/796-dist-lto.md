# Full LTO for distribution builds (#796)

cargo-dist builds the released CLI with the `dist` profile. Before #796 it
inherited `release` but set `lto = "thin"`, while `binary-size.yml` built
`--release` (full LTO): the gate measured a binary nobody downloaded. `dist`
now keeps release's full LTO, and the gate builds `--profile dist`.

## Method

Revision: v3 `2ff91059e` (before #794) plus this change's profile edit, as
temporary commits `36598e361` (matrix and bench) and `ac7e661c2` (Windows
static-CRT check) that also added the measuring workflow; rustc 1.99.1
everywhere. Thin LTO is the same profile with
`CARGO_PROFILE_DIST_LTO=thin`.

- **CI matrix**: the five cargo-dist targets on the runners `dist plan` assigns
  (macos-14, ubuntu-22.04-arm, macos-15-intel, windows-2022, ubuntu-22.04),
  run once from a temporary branch (`measure/796`, workflow run 41012917889;
  the branch is deleted). Per runner: [`builds.py`](796-dist-lto/builds.py)
  times four clean builds (thin, full, full, thin; dependencies fetched, not
  compiled), [`runtime.py`](796-dist-lto/runtime.py) runs 10 alternating
  rounds, and `dist build --artifacts=local` packages the target, whose
  archive is unpacked and run on `demos/demo_commands.txt`.
- **Bench machine**: the same two scripts on x86_64 Linux, bound to one NUMA
  node, with 20 runtime rounds.
- **Runtime workloads** (each a CLI process fed a script on stdin; times include
  start-up, ingestion and printing): `count` (#772's 2,000 facts, 400 counts),
  `recursive` (closure of a 300-node chain, 10 queries), `aggregate` (5,000
  people in 20 departments, a join and grouped sum, 50 queries), `file` (20
  transactions of 500 facts into a `.graph` file, one count). Both builds must
  print the same output, transaction timestamps aside.

## Size

| Target | Thin LTO | Full LTO | Saved |
| --- | ---: | ---: | ---: |
| x86_64-unknown-linux-gnu | 1,501,232 | **1,262,584** | 238,648 (15.9%) |
| aarch64-unknown-linux-gnu | 1,262,544 | 1,094,504 | 168,040 (13.3%) |
| x86_64-apple-darwin | 1,503,392 | 1,075,840 | 427,552 (28.4%) |
| aarch64-apple-darwin | 1,372,560 | 954,192 | 418,368 (30.5%) |
| x86_64-pc-windows-msvc, as packaged (static CRT) | 1,139,712 | 1,060,864 | 78,848 (6.9%) |

The x86_64 Linux full-LTO binary is what the gate measures: 1,262,584 bytes
against the unchanged 1,263,616-byte cap. It equals the `--release` binary, so
the change gives the gate no headroom. Sizes were identical across a runner's
two builds.

**Packaging.** On Linux and macOS the packaged binary has the full-LTO size
(aarch64-apple-darwin: 16 bytes more, the build is under a different target
directory); its bytes differ from the separate build. On Windows cargo-dist
links the C runtime statically (`+crt-static`), which the plain builds above
do not: a second run (41014187879) built thin and full LTO with
`RUSTFLAGS=-Ctarget-feature=+crt-static` and the `--target` cargo-dist uses.
The full build matches the packaged 1,060,864 bytes, differing only in the PE
header; thin is 1,139,712
([`package-x86_64-pc-windows-msvc-static-crt.json`](796-dist-lto/package-x86_64-pc-windows-msvc-static-crt.json)). Without the static CRT the Windows binaries are
1,041,920 (thin) and 963,072 (full). All five packaged binaries ran the demo.

The gate checks x86_64 Linux only. The other targets' sizes are recorded here,
not gated.

## Build time

Clean builds, seconds, in run order (thin, full, full, thin):

| Runner | Thin | Full | Full | Thin |
| --- | ---: | ---: | ---: | ---: |
| ubuntu-22.04 | 43 | 30 | 31 | 28 |
| ubuntu-22.04-arm | 30 | 28 | 28 | 26 |
| macos-15-intel | 124 | 97 | 77 | 67 |
| macos-14 | 42 | 41 | 42 | 37 |
| windows-2022 | 64 | 46 | 47 | 46 |
| bench, x86_64 Linux | 28 | 31 | 31 | 28 |

The first build on each runner is slower whatever its profile (cold caches).
Comparing the warmer pairs, full LTO costs 0–10 seconds (0–16%) per build,
3 seconds on the bench machine. A release builds each target once.

## Runtime

Median full-vs-thin change in elapsed time, and the rounds full LTO won:

| Runner | count | recursive | aggregate | file |
| --- | ---: | ---: | ---: | ---: |
| ubuntu-22.04 | −12.7% (10/10) | −9.4% (10/10) | −8.7% (10/10) | −9.0% (10/10) |
| ubuntu-22.04-arm | −9.7% (10/10) | −16.0% (10/10) | −17.4% (10/10) | −3.6% (10/10) |
| macos-15-intel | −7.4% (7/10) | −3.9% (9/10) | −7.8% (8/10) | −1.5% (6/10) |
| macos-14 | −6.3% (8/10) | −10.3% (10/10) | −5.7% (8/10) | +3.5% (5/10) |
| windows-2022 | −5.7% (10/10) | −9.4% (10/10) | −6.7% (10/10) | −2.1% (7/10) |
| bench, x86_64 Linux | −9.5% (20/20) | −8.4% (20/20) | −11.0% (20/20) | −4.3% (13/20) |

Full LTO is faster on every query-bound workload on every target. `file`
(about 0.1–0.3 s, mostly process start-up and file I/O) is within noise on
macOS. Hosted runners are shared machines; the bench rows are the controlled
measurement.

## Evidence

[`796-dist-lto/`](796-dist-lto/): `builds-*.json`, `runtime-*.json` (every
round) and `package-*.json` per target, plus the two scripts.
