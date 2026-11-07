# Manual shape sweep (#841)

This directory is an audit harness, outside the library workspace and nightly
benchmark routes. The [protocol](../../docs/benchmarks/841-shape-sweeps/PROTOCOL.md)
defines the broader planned matrix. `cases.py` is the authoritative finite
execution matrix; its selection and explicit exclusions belong in the report.

Run only on idle benchmark hardware. Editing/formatting does not require a
benchmark machine. Source changes made by `instrument.py`, `variants.py` and
`events.py` are restricted to a **disposable checkout**. Never run them in a
working checkout with user edits. None adds a shipping feature flag or public API.

`benches/helpers/shape.rs` is a trimmed port of `native/src/main.rs`'s fixtures,
queries and oracles for the six nightly candidates qualified by #863
([evidence](../../docs/benchmarks/863-nightly-candidates/README.md)). This harness
stays authoritative for the sweep; change the port only to follow it.

## Build and verify

From a fresh disposable checkout of the recorded core revision, with the harness
copied in and the exact recorded lock files restored:

```sh
set -euo pipefail
mkdir evidence
cargo test --locked --offline --lib > evidence/baseline.log 2>&1
python3 -m unittest discover -s scripts/shape-sweep
cargo test --locked --offline --manifest-path scripts/shape-sweep/native/Cargo.toml
python3 scripts/shape-sweep/instrument.py .
python3 scripts/shape-sweep/variants.py .
python3 scripts/shape-sweep/events.py .
cargo test --locked --offline --lib db::shape_sweep::smoke_churn_oracles
CARGO_PROFILE_BENCH_OPT_LEVEL=3 cargo test --locked --offline --profile bench --lib --no-run
```

The initial run resolves dependencies offline from the host cache and retains
both resulting lock files; `--locked` applies to reproduction after restoring
those files. All five public executables use the standalone release profile:
opt-level 3, full LTO, one codegen unit. The diagnostic unit-test executable uses
the root bench profile with opt-level 3 overridden (unwind, debug info, full LTO).
It is an observational build, not the public latency executable. Record the path
of the unit-test executable printed by Cargo as `DIAGNOSTIC_BINARY` in the commands
below. Keep that variable outside committed configuration.

`variants.py` restores source after each variant and copies the baseline back
over the native executable. Each variant must pass the independent recursive
walk test before it is measured. Its exact source patch and executable hash are
retained in `evidence/variants`. `events.py` must run **after** variant builds;
its counters exist only with `cfg(test)`.

The probes check every previously quarantined page's generation before/after a checkpoint,
complete snapshot/log answers, empty quarantine after final release, reuse of
released page IDs, integrity and reopen. Log snapshot oracles retain complete
records and cursor oracles retain expected values. Their memory contributes to
diagnostic process RSS; direct quarantine vector capacities and query meter
peaks are reported separately. Neither metric includes every allocator overhead.

## Execute

```sh
CORE_REVISION=5a0ff549271084018c7d4e6050658e64e7e6432e
python3 scripts/shape-sweep/run.py --binary evidence/variants/baseline \
  --out evidence/smoke --fixtures fixtures-smoke --smoke \
  --sessions 1 --samples 2 --families scalar,compound,window,set,uuid,recursion \
  --core-revision "$CORE_REVISION"

# Build every selected fixture first, then run the three five-sample sessions.
python3 scripts/shape-sweep/run.py --binary evidence/variants/baseline \
  --out evidence/public --fixtures fixtures --core-revision "$CORE_REVISION"

python3 scripts/shape-sweep/run_variants.py --binaries evidence/variants \
  --out evidence/recursive --fixtures fixtures

python3 scripts/shape-sweep/run_diagnostics.py --binary "$DIAGNOSTIC_BINARY" \
  --out evidence/churn --fixtures churn-scratch --churn
python3 scripts/shape-sweep/run_diagnostics.py --binary "$DIAGNOSTIC_BINARY" \
  --out evidence/query-memory --fixtures fixtures --public-results evidence/public
python3 scripts/shape-sweep/run_diagnostics.py --binary "$DIAGNOSTIC_BINARY" \
  --out evidence/recursive-memory --fixtures fixtures --public-results evidence/recursive/baseline
```

`run_variants.py` creates `recursive/baseline` by selecting the baseline paired
with the first control and restoring its original case IDs. It reconciles that
selection separately and preserves the original records in the recursive evidence.
The public query, fixture and row count are the same. This derived input is a
diagnostic selection, never an additional latency run.

Query-memory diagnostics use a fresh process/handle for one drain, with no
public-runner oracle construction or warm-up. They compare the row count with
the verified public result and require zero charged memory after cursor drop.
Their process RSS is a separate series from public batch RSS; do not combine
the two as equivalent allocation measurements.

Use a separate output directory for every attempt. The runner refuses an existing
output directory so a retry cannot erase a failed/capped run. `--prepare-only`
creates fixtures/manifests without query measurements. `--families` narrows the
matrix, with all selected cases retained in `cases.json`. `--case-ids FILE`
accepts a nonempty JSON list of exact IDs, rejects missing/duplicate IDs, and
keeps the authoritative matrix order. Use it for a separate retry, for example
`--families window --case-ids capped-windows.json --sessions 3 --samples 1`;
retain the original capped run and label the reduced repetition count.
A later run may reuse
the completed fixture directory on the same source; retain its manifests and
hash every `.graph` and WAL before cleanup. An interrupted fixture has no complete
manifest and must be inspected or removed explicitly before rebuilding.

Each public case/session opens a fresh child process and database handle, runs
one full verified warm-up, then measures five query-open/drains on that handle
through 256-row batches. Process RSS peaks include setup, warm-up and earlier
samples in the same batch; they are not independent per-sample allocation peaks.
Open/warm-up timings repeated on sample rows describe the one shared setup and
must not be summed. Recursive controls run as matched baseline/control pairs on
separate sockets, with placement swapped each session; each control gets its own
baseline. Paired processes use separate byte-identical fixture copies, including
WALs, to avoid exclusive file-lock waits; `pair-fixtures/hashes.json` records
the common input hashes. A streaming row fingerprint is included in latency;
oracle construction and fingerprint comparison are outside the timer. The
clock stops before cursor destruction; database close and JSON serialization
are outside the query timing but inside the process deadline. Scalar
and compound fingerprints are order-sensitive; other families check count plus
two independent commutative 64-bit accumulations. These are deterministic
correctness fingerprints, not cryptographic integrity hashes. Source/build and
fixture artifacts use SHA-256 separately.

The committed loader is `LogWriter` with explicit fixed transaction/valid times;
its internal threshold is 65,536 facts. Pending fixtures use 1,000-fact Datalog
statements, wall-clock transaction times and WAL checkpoint threshold `usize::MAX`.
Open/first verified pass/timed warm pass are separate fields. Pending handles
replay the whole WAL; the reported process RSS includes that fixture state.
There is no claim that a pending fixture has the same memory contract as a
committed file. The default query memory limit is unlimited, page cache is 256
pages unless the case specifies 0/4,096, and remaining options are defaults.

`resources.json` rows are `[elapsed_ns, resident_bytes, host_idle_percent]`, sampled
at roughly 100ms. Memory ceilings use 80% of initial host `MemAvailable` and
selected-node `MemFree`, divided among concurrent children sharing each budget.
The recursive and diagnostic controllers additionally cap each child at 48 GiB
and retain the effective bounds in `run.json`. The runner stops a child after sustained host utilization above
5%, a deadline or its memory/disk bound. Ten-second preflights use instantaneous
CPU deltas and swap activity, not the load average. External contention below
that threshold remains a limitation. Retain failed attempts and raw stderr.
The RSS guard is sampled, not enforced by a cgroup or the query allocator;
brief overshoots between polls may not trigger it. Keep the process high-water
marks alongside the interval samples.

## Reconcile and archive

```sh
python3 scripts/shape-sweep/evidence.py evidence/public > evidence/public/reconciled.json
python3 scripts/shape-sweep/evidence.py evidence/recursive > evidence/recursive/reconciled.json
python3 scripts/shape-sweep/analyze.py ratios evidence/recursive/samples.jsonl > evidence/recursive/ratios.json
python3 scripts/shape-sweep/analyze.py churn evidence/churn/generations.jsonl > evidence/churn/summary.json
```

Reconcile any reduced-repetition retry with its actual sample count, for example
`python3 scripts/shape-sweep/evidence.py evidence/window-retry --samples 1`.
Keep that summary separate from the original three-by-five matrix. Paired ratio
summaries include per-session and per-placement results; check whether the
direction persists when the baseline and control swap nodes before attributing
a pooled difference to the control.

The reconciler requires exactly one disposition for every planned sample. A
measured result needs a successful exit, positive finite latency and matching
expected/actual answer fingerprints. Errors, interference, resource limits and
timeouts cannot masquerade as zero latency. A first failed case defers its later
samples explicitly; it does not vanish from the matrix.

Freeze raw logs, all JSON, manifests, patches, exact locks and source/build hashes
before cleaning scratch. Test/build logs may contain absolute private paths;
redact those paths in the published archive and document the normalization. Keep
original private artifacts locally until the archive's checksums and independent
reconciliation pass. Do not publish host addresses or identifying hardware data.

Current scope limits of the finite matrix: full drains only for window memory
measurements; narrower pending/window/set combinations than the protocol's broad
axes; compound fixtures have 3*floor(F/3) facts; UUID fixtures have 18*N facts and
references follow logical UUID ordinal distributions (actual DICT paths are
observed separately). The static-connected-order control retains the connectivity
constraint and changes only adaptive reprioritization; it does not measure the
entire historical delta-ordering optimization. Per-path contribution requires
the diagnostic counters to show that the path actually ran.

Recursive `k > 1` cases filter the output of an unbound ancestor invocation.
Only literal rule arguments seed magic-set demand in this engine, so these
cases compute unrestricted recursion and then select leaves. Their `k=128/129`
labels do **not** exercise the small-ancestor source-cardinality cutoff. Literal
`k=1` cases and path counters provide narrower evidence; the cutoff itself is
unmeasured by this matrix.
