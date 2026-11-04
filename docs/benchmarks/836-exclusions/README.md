# Registered benchmark exclusions (#836)

This is the explicit per-shape disposition option in
[#836](https://github.com/project-minigraf/minigraf/issues/836), following
[X01 and Appendix C](../../bench-coverage-v3.md#appendix-c-all-deliberately-unselected-registered-ids).
The 110 wider row-frame cases remain **MANUAL / DEFERRED**. The other 46
registered cases remain **EXCLUDED**. No latency sweep was performed for this
report, and no nightly route, runtime code, dependency, or release version changes.
This follows PHILOSOPHY.md's reliability, simplicity, and documentation principles.

## Pinned inventory and evidence

Core, benchmark harness, and workflow revision:
`d5df1f22c32c0c1abc486fbcb3a574d2d1d0ce99` (`v3`, 2026-11-04 audit).
The public source archive's SHA-256 is
`b810d40cd23d2e45a0437acabd7a6508ec2cae04ccafacada2aa81881433b022`.
This is a fresh enumeration, distinct from #776's validation run
[41642356471](https://github.com/project-minigraf/minigraf/actions/runs/41642356471).

| Evidence | Purpose |
| --- | --- |
| [enumeration.txt](enumeration.txt) | Fresh Criterion `--list` output, not benchmark measurements |
| [run.json](run.json) | Build/enumeration conditions, toolchain, elapsed time, exit status and hashes |
| [dependencies.lock](dependencies.lock) | Exact resolved dependencies for this build |
| [reconciliation.json](reconciliation.json) | Input hashes, counts and the 427 selected IDs |
| [dispositions.csv](dispositions.csv) | All 156 omitted registered IDs, including each of the 110 manual shapes |
| [unregistered.csv](unregistered.csv) | Two hypothetical 100k UDF half-frame IDs; outside the 583-ID denominator |
| [reconcile.py](reconcile.py) | Standard-library reproduction of expansions, set differences and cost formulas |

The reconciliation rejects duplicate registrations, empty/overlapping nightly
routes, changed row-frame registration or selection, unexpected omitted IDs,
missing expected exclusions, and unexpected registration of a 100k UDF half-frame.
The exact result is **583 = 427 selected + 156 unselected**, with the latter
split **110 + 4 + 6 + 6 + 30**. Selection means matching the pinned workflow's
26 native regexes; it does not mean execution, analysis or upload in this audit.
There are no measured cases, correctness-run results or uploads to report here.
The prior #183 timing CSV remains historical evidence at its own revision; its
numbers are not promoted to measurements of this pin.

## Row-frame fixture and cost model

Every shape uses an in-memory database with `n` entities, values `0..n-1`,
and two facts per entity (`:val` and `:part`), inserted in batches of 100.
Partition labels are `floor(row / (n/100))`, including in the one-partition
queries where the labels are joined but not used for partitioning. Each query
joins both attributes, orders by the unique integer value, computes one window
aggregate and materializes `n` entity/result pairs. Setup and registration are
outside the timed loop. There are no tied order keys in this fixture.

For `p` partitions, let `m=n/p`. `:rows w preceding` contains the current row
plus up to `w` predecessors: its maximum cardinality is `min(m,w+1)`, not `w`.
The `default` frame omits `:rows` and uses peers; unique values make it a
prefix for these fixtures. The CSV distinguishes frame name, predecessor count
and maximum frame cardinality.

| n | p | Facts | m | sqrt predecessors | half predecessors | prefix predecessors |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 1 | 20,000 | 10,000 | 100 | 5,000 | 10,000 |
| 10,000 | 100 | 20,000 | 100 | 10 | 50 | 100 |
| 100,000 | 1 | 200,000 | 100,000 | 316 | 50,000 | 100,000 |
| 100,000 | 100 | 200,000 | 1,000 | 31 | 500 | 1,000 |

The cost columns are **source-derived operation counts/bounds, not elapsed-time
estimates**. All shapes additionally pay for query preparation, scans/join,
window ordering and output materialization. A linear aggregate pass does not
make the entire query linear. These built-in integer fixtures use linear
sum/count/avg updates; bounded min/max use an amortized-linear deque pass.
For bounded min/max, the CSV records the requested deque reservation in slots
per partition, excluding allocator rounding and all other query allocations.
It is neither measured RSS nor a full memory bound.

For a replayed registered aggregate, let `L=min(m,w+1)`. The exact number of
step calls per query is `p * (L*(L+1)/2 + (m-L)*L)`; each output also constructs
and finalizes its accumulator. A full-prefix frame (`w >= m-1`) instead keeps
one accumulator per partition, with **n step calls and n finalizations**.
See the `WindowFunc::Udf` branch in
[aggregate.rs](../../../src/query/datalog/aggregate.rs) at the pinned revision.
This corrects the original X01 wording: **the registered one-partition prefix
case is not quadratic replay**. It stays manual as requested, because the
nightly 100-partition prefix case already exercises the running UDF path.

## Per-shape deferred dispositions

Each manual CSV row has one of the following reasons. All 110 are deliberately
deferred, even where a previous revision has timings. None is newly measured.

| Reason | Shapes | Cost and rationale | When to revisit |
| --- | --- | --- | --- |
| R1 | Remaining built-in sum/count/avg and default min/max shapes | Linear aggregate work after ordering; repeats complete 10k/100k joins and result allocation. The 18-case nightly shortlist bounds routine work; this audit adds no distinct engine path by measuring every width. | A window implementation change or a targeted function/width regression; compare the same shape's default and bounded frames. |
| R2 | Remaining explicit min/max frames | Amortized linear aggregate work, but reservation grows with `min(m,w+1)`; exact slots are in each CSV row. Nightly min/max two/prefix controls cover the deque path. | A deque allocation/eviction change; include zero/sqrt/half and one-partition cases in the targeted sweep. |
| R3 | Five remaining 10k registered UDF frames | Replay work ranges from 10,000 to 37,507,500 step calls per query. Nightly 10k/hundred/two already exercises replay. Wider callbacks multiply every Criterion iteration. | A replay/dispatch change, with a separately budgeted manual run; keep the 10k one-partition half case manual. |
| R4 | 100k/one registered UDF prefix | 100,000 steps, 100,000 finalizations and one factory per query; full query still joins/sorts/materializes 100k rows. Nightly 100k/hundred/prefix covers the running-accumulator branch. | A partition-size or prefix-state regression; keep this case manual, without calling it quadratic. |
| R5 | Unregistered 100k UDF half-frames | One partition: **3,750,075,000** steps/query; 100 partitions: **37,575,000**. Both replay; the one-partition shape is particularly expensive. They remain outside registration and nightly coverage. | Only a separately budgeted callback-scaling investigation; do not silently add them to the 110-case sweep. |

There are 104 deferred built-in cases and six deferred registered UDF cases.
The CSV's exact IDs, not a family count alone, define the disposition.

## Other excluded families, rechecked at the pin

| Reason | Exact expansion | Count | Source evidence, cost and retained rationale |
| --- | --- | ---: | --- |
| E1 | `point_query_chain_depth/{churned_attr,sibling_attr,attr_scan,churned_attr_as_of}/500` | 4 | `benches/minigraf_bench/history.rs` registers depths 1/500/2000 with the same four queries, 2,000 filler facts, sibling attribute and checkpointed churn history. Depth 500 adds intermediate history setup/storage; selected 1/2000 endpoints already cover the paths. It could reveal a nonlinear threshold, but no new threshold evidence here justifies routing it. |
| E2 | `retract/{single_fact,batch_100}/{1k,100k,1m}` | 6 | `bench_retract` uses pending in-memory value fixtures and repeatedly retracts the same facts. Scaling the fixture to 1m adds setup/memory cost without a fresh retraction each iteration. Selected 10k and file chunk-boundary cases cover the existing distinct paths. Revisit for a concrete scaling regression; a live-target-per-iteration workload would need separate harness review. |
| E3 | `btree_lookup/{entity_point,attribute_scan}/{1k,10k,100k}` | 6 | `bench_btree_lookup` calls `populate_with_names`, which uses `Minigraf::in_memory`; no file open or checkpoint. These repeat pending-memory point/attribute scans already covered by C01, with extra fixture and result materialization cost. They do not establish committed B+tree coverage. |
| E4 | `simd_{temporal,as_of,aggregate}/{scalar,simd}/{100,1k,10k,100k,1m}` | 30 | `bench_simd` and `benches/simd_helpers.rs` operate on synthetic numeric vectors. Work scales with slice length, excluding database extraction, joins and materialization. They remain experimental kernels, not shipped engine paths. Revisit only alongside a separately reviewed engine integration. |

No additional nightly representative is proposed: the full-prefix correction
changes the explanation, not the covered execution path. The 18 row-frame
representatives and all other routing remain unchanged.

## Reproduction and future manual measurements

Use an idle host and an isolated checkout of the pin above. Do not run the
unfiltered benchmark command: `--list` still runs fixture setup in some other
registration functions, and a full measurement run is much more expensive.
Use the retained lockfile for dependency reproducibility. The recorded run
uses default features and the repository's unmodified bench profile.

```sh
cp /path/to/836-exclusions/dependencies.lock Cargo.lock
cargo bench --locked --offline --bench minigraf_bench --no-run
cargo bench --locked --offline --bench minigraf_bench -- --list > enumeration.txt 2> enumeration-stderr.txt
python3 /path/to/836-exclusions/reconcile.py enumeration.txt .github/workflows/bench.yml /tmp/836-reconciled
```

Offline mode assumes dependencies are already cached; otherwise fetch the locked
dependencies first. Compare the regenerated CSVs and selected-ID sets with this
report; elapsed times and raw stderr are expected to differ across machines.

For a future manual sweep, select individual literal IDs from `dispositions.csv`
with `^ID$` (these IDs contain no regex metacharacters needing escaping), and
run them serially. Explicitly choose costly replay cases; do not automatically
include `unregistered.csv`. Before timing, check the full answer against an
independent oracle: row `i` in partition starting at `a` has lower bound
`lo=max(a,i-w)` for bounded frames, or `lo=a` for default/prefix. Expected count
is `i-lo+1`, sum/UDF sum is `(lo+i)*(i-lo+1)/2`, min is `lo`, max is `i`, and
avg is `(lo+i)/2`. Verify all entity/result pairs and `n` rows, not just successful
execution. The current harness's `expect("window query")` is not such an oracle.

Retain, per measured ID: fixture/function/frame/partition, core and harness
SHA, lockfile/toolchain/build flags, permissible hardware description and
idle/power/affinity conditions, wall time, Criterion samples/iterations, and
oracle outcome. Capture correctness outside the timed region. No such rows
are claimed by this disposition-only report. Remove temporary targets,
fixtures, source copies and scripts after retaining the evidence.
