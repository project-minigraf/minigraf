# Committed net-assert after the streaming engine (#532)

Source audited: [`v3` at `d0ad89c1d`](https://github.com/project-minigraf/minigraf/tree/d0ad89c1d6df734afd5d4669ca9139c8f55d7702)
(2026-10-28). #551 removed `filter_facts_for_query` and the old rule evaluator.
Production queries read checkpointed facts through the engine's covering
key scans. No surviving production call to `net_asserted_facts` takes those
committed scan results. This supersedes #532's original repeated hashing of
committed facts; pending records still need fact-level net-assert.

## Source audit

Here, **committed** means the checkpointed keys in a snapshot's committed
reader. A durable transaction may still be in the snapshot's **pending** store,
including records replayed from a WAL. These terms describe the query's read
sources, not whether a transaction has committed durably. A later checkpoint
does not change an already-open snapshot's sources. Inside a large transaction,
the same committed-reader interface points at working trees, as described below.

Line references describe the pinned source; paths are relative to `src/`.

| Path | Evidence |
|---|---|
| Plain queries and cursors | `query/datalog/executor.rs:299,308,315`: collecting and cursor entry points use `open_query`, which calls `engine::plan` and `engine::open`; rule queries use `open_rule_query` (line 354). There is no legacy fallback. |
| Prepared queries | `query/datalog/prepared.rs:105,122,135,145`: substitute slots, set one snapshot, then call the same executor's `execute` or `open`. |
| Rules, including recursive/semi-naive evaluation | `query/engine/mod.rs:200` opens rules; `query/engine/recursive.rs:53,103,151` evaluates strata, scans stored rows with the ordinary `Scan`, and builds full/delta body operator trees. Derived relations hold cells, not a committed-plus-pending `Vec<Fact>` passed through the old filter. |
| Joins, negation, disjunction, aggregates and windows | `query/engine/mod.rs:297` builds the operators. Joins and the body trees of `AntiJoin` and `Union` use the same scan implementation; aggregates/windows consume their batches. None directly calls `net_asserted_facts`; their scans can reach it through the pending path below. |
| Committed scans and pending shadows | `query/engine/scan.rs:404,454` reads `KeyFact`s with `KeySource::live_keys`; `storage/reader.rs:156,408` decides net-assert on index keys before decoding surviving keys. `scan.rs:759` builds the shadow set from pending records only; value probes do the same at lines 274–325. Valid-time and shadow checks filter surviving keys, without rehashing them as facts. When the shadow set is nonempty, each surviving key still needs an EAV-prefix hash lookup (lines 470–474); fully checkpointed fixtures have no such pending shadow. |
| Pending records | `graph/storage.rs:505,577,614`: `pending_live` walks pending/staged `(entity, attribute)` runs, and `flush_run` calls `net_asserted_facts`. `PendingView::fact` (line 223) reads only those stores. This is the only production call site of `net_asserted_facts`. |
| Transaction queries | `db.rs:1777,1795`: small transactions add staged facts to the pending view (`graph/storage.rs:307,329`). Large transactions use `storage/persistent_facts/large_tx.rs:218`'s `OnDiskReader` over working trees, with the snapshot's pending length cleared (`graph/storage.rs:319`). Both use the same engine. |
| WAL-backed and replayed records | `db.rs:687` skips WAL entries already in the checkpoint and loads newer entries through `FactStorage::load_facts` (`graph/storage.rs:793`), which appends to pending. Fact-level net-assert there is necessary, not a second pass over committed scan output. |
| Temporal queries | `query/engine/mod.rs:178` resolves counter/timestamp bounds for both plain and rule reads. `Snapshot::tx_bound` (`graph/storage.rs:340`) uses a counter bound for monotonic timestamps or committed transaction ranges plus pending timestamp checks for irregular timestamps. `live_keys` applies that bound during the index walk; `scan.rs:888,896` applies valid-time visibility afterward. No historical fallback uses fact-level filtering of committed output. |
| Browser and optional builds | `browser/mod.rs:136,312,335` uses the same executor for execute, prepared and cursor APIs. The core dispatch/scan code has no alternate WASM query path. Cargo's `wasm` feature is a deprecated no-op; `browser` adds browser APIs/storage; `test-hooks` adds test controls, not another engine. This is a static cfg audit, not a new WASM build or browser runtime test. |

`CommittedReader::live_facts` (`storage/mod.rs:401`),
`OnDiskReader::live_facts` (`storage/reader.rs:480`), the snapshot helper that
combines live facts (`graph/storage.rs:454`) and `get_current_value`
(`graph/storage.rs:1335`'s `cfg(test)` impl) are test-only. Of these, only the
trait's default `live_facts` and `get_current_value` directly call
`net_asserted_facts`; the on-disk implementation resolves live keys, and the
snapshot helper delegates to it. None ships in the production query path.
The remaining direct calls in `persistent_facts.rs` and the storage test
module are tests.

An empty pending walk still ends in `flush_run(Vec::new())`, which can call
`net_asserted_facts` with no input. Its symbol may therefore occur in a profile
of a fully checkpointed fixture. That is distinct from hashing every committed
row, and this task does not change it.

## Measurements

**Date:** 2026-10-28. Idle benchmark server, Linux 6.8.0-139-generic,
Rust 1.99.1; `envelope` profile (opt-level 3),
`CARGO_PROFILE_ENVELOPE_DEBUG=2`, `RUSTFLAGS='-Cforce-frame-pointers=yes'`.
CPU and memory were bound to NUMA node 1 for timings and captures; compilation
used the whole host beforehand. No build/test ran alongside timings. Queries
drained read-only cursors over checkpointed fixtures with warm page caches.
Pre/post toolchain, governor and process/load observations are retained in the
[environment record](532-environment.txt). Host identity and private hardware
details are omitted.

The [raw envelope](532-envelope.json) covers all 23 shapes at each scale,
including storage and RSS. All 69 records completed without errors or caps,
with at least five timed runs each. Its `minigraf: 2.0.5` field comes from the unreleased branch's Cargo
manifest; the measured code is the revision above, not the published release.
These are absolute measurements, not a paired A/B against the old executor's
historical 10% profile or the earlier [#551 snapshot](../BENCHMARKS.md#executor-removal-551).
That snapshot predates later optimizations such as #530, and this build records
its own frame-pointer/debuginfo flags. No numerical 10% speedup is inferred.

The envelope's built-in count assertions cover all shapes except
`join_3_scan` and `as_of_ts_scan`. The retained checker validates those two:
the [six independent checks](532-row-checks.json) all match. The join count is the fixture generator's arithmetic mapping
`((i * 7919 + 1) % E) % 100 == 7`, summed over `0 <= i < E`, independently
of the query engine; it does not independently check the stored `:follows`
and `:group` records. The timestamp scan count reads raw `:age` records with
`fact_log`, keeping `tx_id <=` the fixture's `tx_half` timestamp, including
transactions sharing that millisecond. This log uses `LogSource::walk`,
not the query engine's `live_keys` path. With fewer than 100 timed runs,
the envelope's p99 is the maximum sample.

| Scale | Shape | Rows | Runs | p50 (ms) | p99 (ms) | Peak RSS (MiB) |
|---|---|---:|---:|---:|---:|---:|
| 100k | `attribute_scan` | 16,666 | 200 | 12.516 | 13.492 | 6.5 |
| 100k | `join_3_selective` | 10 | 200 | 0.209 | 0.244 | 5.3 |
| 100k | `join_3_scan` | 167 | 200 | 43.212 | 50.092 | 14.5 |
| 100k | `recursive_ancestors` | 5 | 200 | 1.162 | 1.247 | 5.3 |
| 1m | `attribute_scan` | 166,666 | 110 | 136.239 | 148.003 | 7.2 |
| 1m | `join_3_selective` | 10 | 200 | 0.204 | 0.247 | 5.0 |
| 1m | `join_3_scan` | 1,667 | 13 | 1170.474 | 1221.751 | 46.3 |
| 1m | `recursive_ancestors` | 6 | 200 | 1.446 | 1.723 | 5.3 |
| 10m | `attribute_scan` | 1,666,666 | 11 | 1366.540 | 1391.023 | 7.4 |
| 10m | `join_3_selective` | 10 | 200 | 0.197 | 0.242 | 5.1 |
| 10m | `join_3_scan` | 16,667 | 5 | 13373.934 | 13422.707 | 61.4 |
| 10m | `recursive_ancestors` | 7 | 200 | 2.057 | 2.804 | 5.6 |

### Profiles at 1M

The [profile summary](532-profile-summary.json) validates each child's shape,
row count and timed runs, and reports zero lost samples for all four retained
captures. All use DWARF call graphs and inline-aware reports. The original
499 Hz recursive capture lost 210 samples in three chunks; it was discarded
and repeated at 249 Hz, with zero loss. No envelope timing was repeated or
changed by this profiling adjustment.

| Shape | Hz | Children | Samples | Identified query samples | `net_asserted_facts` samples | Share of identified query cycles |
|---|---:|---:|---:|---:|---:|---:|
| `attribute_scan` | 499 | 1 | 7,784 | 7,631 | 0 | 0% |
| `join_3_selective` | 499 | 200 | 22,010 | 4,515 | 2 | 0.045% |
| `join_3_scan` | 499 | 1 | 8,280 | 8,155 | 1 | 0.013% |
| `recursive_ancestors` | 249 | 40 | 4,891 | 3,055 | 4 | 0.128% |

All [seven sampled stacks](532-net-assert-stacks.txt) containing
`net_asserted_facts` reach it through `Snapshot::pending_live`, `RunWalk` and
`flush_run`. Source analysis establishes that their input is empty on these
fixtures: the builder checkpoints before closing, all three fixtures have no
WAL, readers open read-only, and rule magic seeds live in derived relations,
not the pending store. These samples do not represent a second pass over
committed facts. No capture contains `filter_facts_for_query`,
`RecursiveEvaluator` or `StratifiedEvaluator` frames.

Each child also calls `fixture_shapes` before measuring, which reads the fact
log to construct the queries' timestamp parameters. This setup accounts for
76.2% of sampled cycles in the fast selective capture. The
[summarizer](532-summarize-profiles.py) classifies stacks containing that helper
as setup first, then stacks with `drain`, engine or pending-net-assert frames
as identifiable query work, and the remainder as other. Rust crate
disambiguators are removed only for symbol matching. Cycle percentages sum
the samples' periods; the net-assert percentages above use identifiable query
periods as their denominator.

These 16 KiB DWARF captures identify 7,323 / 4,133 / 8,150 / 14 query samples
respectively without a `drain` frame: visible engine/pending frames establish
their category. Almost all identified recursive-query samples do reach
`drain`. Separately, 7,320 / 4,331 / 8,149 / 2,982 stacks end at an unknown
outer frame; those counts include chains that already reach `drain` or `main`
and do not measure missing `drain` callers. The summary retains both
diagnostics without inventing ancestry. Unresolved leaf symbols account for 25.7% / 43.3% / 31.3% / 38.4%
of identifiable query cycles respectively. These are corroborating profiles,
not a complete hotspot breakdown or proof of absence by sampling. The static
caller/cfg audit supplies that proof for committed-fact inputs.

**Outcome:** #532's closing checks are met: all audited v3 query paths bypass
the deleted committed-fact pass, the full envelope completes, and usable
scan/join/rule profiles corroborate the distinction between committed keys
and empty pending walks. Close #532 as superseded by #551; no engine patch
is needed.

## Reproducing

On an otherwise idle bench host, check out the measured revision. Copy the
linked [dependency lockfile](531-Cargo.lock) to its root as `Cargo.lock`, and
the [row checker](532-check-rows.rs) to `examples/check_532_rows.rs`. Save the
[profile driver](532-profile.sh) and [summarizer](532-summarize-profiles.py)
outside that checkout and set `profile_driver` and `summary_script` to their
paths. Set `run_dir` to a fresh scratch directory outside the checkout:

```bash
mkdir -p "$run_dir"
export CARGO_TARGET_DIR="$run_dir/target"
export CARGO_PROFILE_ENVELOPE_DEBUG=2
export RUSTFLAGS='-Cforce-frame-pointers=yes'
cargo build --profile envelope --example perf_envelope --example check_532_rows --locked
numactl --cpunodebind=1 --membind=1 \
  "$CARGO_TARGET_DIR/envelope/examples/perf_envelope" \
  --scales 100k,1m,10m --budget-secs 15 --cap-secs 60 \
  --dir "$run_dir/fixtures" --json "$run_dir/envelope.json"
"$CARGO_TARGET_DIR/envelope/examples/check_532_rows" \
  "$run_dir/fixtures" "$run_dir/envelope.json" > "$run_dir/row-checks.json"
bash "$profile_driver" "$run_dir"
python3 "$summary_script" "$run_dir" > "$run_dir/profile-summary.json"
```

The driver verifies that the 1M fixture has metadata and no nonempty WAL,
records CPU/process snapshots, then captures each shape separately with
`perf record -e cycles:u --call-graph dwarf,16384`, at 499 Hz for scans/joins
and 249 Hz for the recursive shape, with both CPU and memory pinned. The
indices were verified against the pinned `shapes` list:
2 = `attribute_scan`, 4 = `join_3_selective`, 5 = `join_3_scan`,
10 = `recursive_ancestors`. The selective and rule shapes repeat the existing
`__query` child 200 and 40 times respectively, since each child stops at 200
timed iterations. The other two shapes run one child each. No query source
or benchmark harness is modified.

It writes `perf report --stdio --header --children --inline` output and
`perf script --inline -F comm,pid,tid,time,event,period,ip,sym,dso` stacks,
demangled with `c++filt -s rust`. Check shape names, row counts, errors/caps,
sample counts and lost samples before drawing conclusions. Keep the reports,
then remove scratch fixtures, builds and raw captures. The dependency lockfile
SHA-256 is `b03965119fd8ce5b5cf3ad3d37fb194ffeae24b6a862a0c85ae8fbcb6833ee77`.

A clean reproduction has an empty `discarded_captures` summary. The summarizer
adds that diagnostic only when `discarded-recursive/` contains manually
preserved record/report files from a failed earlier capture; the driver does
not create that directory automatically.

## Deviations from the approved design

None in scope: this task verifies existing behavior and documents evidence.
It changes no query implementation, API, format, dependency or release.

Measurement adjustment: the initial recursive recording lost data, so only
that capture was repeated at 249 Hz instead of 499 Hz. Its discarded-run
diagnostics remain in the summary. The stack classification uses identifiable
query frames rather than requiring a complete chain to `drain`, because many
16 KiB captures lack outer callers; the limits are stated above.

The standalone row checker is compiled as a temporary example against the
pinned library; it is retained under `docs/benchmarks/` and adds no core test
or public API. README, test-coverage counts and release history were
cross-checked and are unchanged. The docs site already documents #551's engine
dispatch; this evidence-only task changes no site behavior or release entry.
