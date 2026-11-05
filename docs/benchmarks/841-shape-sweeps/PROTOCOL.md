# X06: large-scale shapes and recursive optimization isolation

Execution plan for [#841](https://github.com/project-minigraf/minigraf/issues/841),
following [the coverage audit](../../bench-coverage-v3.md#remaining-exclusions-and-follow-ups).
Status: **execution protocol**. This document specifies the broader fixture and
control design. The finite executable selection is recorded in
[`cases.py`](../../../scripts/shape-sweep/cases.py); execution results and
deviations belong in the adjacent report. This is not a new nightly route.

Initial source revision: `5a0ff549271084018c7d4e6050658e64e7e6432e` (`v3`).
Freeze the eventual harness revision separately. If the source changes before
execution, record the new SHA and recheck every implementation assumption below.
No production behavior or public API change is part of this work.

## Executable selection and method adjustments

The sections below describe the broad design. The executable harness makes these
explicit reductions and adjustments; the report must distinguish them from
unmeasured extensions:

- Public API latency uses a standalone release profile (opt-level 3, full LTO,
  one codegen unit), not the root `envelope` profile. Diagnostic probes use the
  root bench profile with opt-level 3 overridden and remain a separate series.
- The public runner takes idle preflights before setup and each session. Paired
  and diagnostic controllers take one before their invocation's work; every
  child retains interval CPU/RSS observations. There is no separate per-family
  preflight or continuous swap counter. Local fallback checks are limited to
  millisecond-scale Python evidence/controller unit tests while the primary
  host is measuring and the alternate is unreachable; builds and database
  measurements remain on the benchmark host.
- Committed fixtures use `LogWriter`, its 65,536-fact internal checkpoint
  threshold, and fixed transaction/valid times. Pending fixtures use 1,000-fact
  Datalog statements and wall-clock transaction times. Binary fixture hashes
  identify the actual inputs; pending files are not byte-reproducible from the
  logical formula alone.
- Compound cases contain `3*floor(F/3)` facts, with no padding, one page shape,
  and committed storage only. Window cases fully drain; limited windows and
  ten-million-fact pending windows remain extensions. S5's pre-existing mixed
  `perf_envelope` example is not included in this finite selection.
- The executable set matrix uses integer members and the query strings in
  `cases.json`; it does not cover every set-element type or membership probe.
  UUID cases cover logical ordinal distributions, with actual DICT/cache paths
  observed separately, but not every threshold-boundary combination below.
- Parsed query open and full drain are recorded; first-batch, standalone close,
  prepared-slot and `execute` materialization comparisons are not measured.
  These are additional phase/API extensions, not implicit parts of the latency
  results. Fixture construction and database open have separate fields.
- Five timings share one process/handle after a complete verified warm-up;
  each case/session gets a new process. The executable deadline is **600 seconds
  for the entire batch**, including oracle/open/warm-up, not 600 seconds for
  each individual query. If a batch fails, its planned sample slots receive an
  explicit failure disposition; partially completed in-process timings are not
  emitted as successful samples. Earlier failed cases are explicitly deferred.
- Churn readers remain idle while held, except the rolling-reader schedule,
  which fully drains each evicted reader. All retained readers are fully checked
  at final release. Each checkpoint checks previously quarantined page headers;
  it does not trace every backend write or prove protection of all live pages.
  Oracle records and the observer's page map contribute to diagnostic RSS.
  Churn starts from fixed-timestamp `LogWriter` records, then performs ordinary
  retract/assert transactions with wall-clock timestamps. Schedules share the
  deterministic update recipe, not identical timestamp bytes.
- Recursive pairs use byte-identical independent file/WAL copies to avoid
  exclusive writable-handle lock waits, on separate sockets with placement
  swapped each session. The static-order control retains connectivity and
  initial seed priority; it isolates only adaptive reprioritization, not all
  historical delta ordering. A noninitial connected choice counter alone does
  not prove a different plan from the control. Missing path/plan evidence must
  leave that optimization's contribution unverified.
- Recursive `k > 1` queries invoke the rule with unbound variables, compute
  unrestricted recursion, and then filter selected leaves. They do not seed
  magic-set demand or control the ancestor alternative's source cardinality.
  In particular, `k=128/129` is not evidence for that selection cutoff; this
  threshold remains unmeasured.

## Execution gate and scope

Run builds, fixture generation, enumeration, tests and measurements on the
established benchmark host when idle. Retain busy preflights as deferred attempts;
do not stop another workload to make room. Host addresses, identifying hardware
details and private scratch paths
belong in the operator's private record, not this repository.

Before each session, retain ten one-second CPU/swap deltas from `/proc` (the
runner implements the current-sample equivalent of `vmstat 1 11`), available
memory, free disk, CPU affinity, NUMA placement, power-policy label and filesystem
type. Require all ten current samples to have at least 95% idle CPU, no swapping,
and no other build/load/benchmark job. Repeat after setup and between families;
retain interval samples during execution. Record interference and rerun affected
sessions, preserving the invalidated observations separately.

Use one NUMA node for measurements. For matched A/B controls, use the established
separate-socket policy with nodes swapped each round; record each assignment.
Untimed builds and correctness tests need no affinity. Check available memory on
the selected node, not just total host memory. Keep scratch on the same local
filesystem for all compared file workloads; tmpfs results are a separate series.

X06 covers C02/C03/C13/C14/C23/C25/C29/C31. X02/#837 owns finite query caps,
transaction memory and release acceptance. X03/#838 owns browser migration;
X04/#839 owns foreign-host lifecycle breadth. This native manual sweep cannot
close those scopes or establish a universal memory/latency bound.

## Common fixture and measurement contract

- Use deterministic entity UUIDs derived from a fixed namespace and decimal
  entity ordinal; retain the namespace and derivation in the fixture manifest.
  Freeze transaction timestamps explicitly. Record both logical input facts and
  physical records, live entities, attributes, set members and history records.
  A ten-million-fact fixture must actually contain ten million input facts;
  do not call ten million multi-attribute entities ten million facts.
- Insert in 1,000-fact statements. File variants disable automatic checkpoints
  with `wal_checkpoint_threshold(usize::MAX)` and use explicit checkpoints.
  For committed large fixtures, checkpoint every 100,000 facts and at completion;
  pending controls retain their entire input. Record any large-transaction path
  separately. Do not silently replace pending with committed to fit memory.
- Use default features and the repository's `envelope` profile (opt-level 3).
  Keep `bench`/size-optimized results separate. Record the actual resolved Cargo
  lock, `rustc -Vv`, `cargo -V`, target, features, rustflags, build command,
  source patch, executable hash, and every `OpenOptions` override.
- Default cache: 256 pages; matched cache sensitivity: 0 and 4,096 pages on the
  UUID subset below. OS cache is warm. Label a fresh database handle as a fresh
  *application* cache, never an OS cold-cache run. Do not drop system caches.
- Separate fixture build, open, query-open/first-batch/full-drain, checkpoint,
  close and reopen phases. Normal cursor batches are 256 rows; consume all
  returned values. `execute` includes full result materialization; report it
  separately from a cursor drain. Prepared and parsed calls are separate cases.
- First run an untimed complete oracle check. For timing, use three independent
  sessions, each with five raw samples per case, one untimed warm-up, and a fresh
  child process/handle per case/session. Five timings share that handle after
  one full verified warm-up. The initial separate-process-per-sample pilot is
  retained separately: at 1m pending facts, WAL open cost about 5.3 seconds versus
  0.9–1.4 seconds for the measured queries. Recreate mutating fixtures from the same frozen source.
  Retain all samples, including first-use latency, without reporting five-sample
  maxima as p99. Publish per-session median/min/max and the range across sessions.
- Verify every timed answer outside the timer. For a drained result, compute a
  rolling digest during drain and independently validate it afterward; report
  that digest work is included in latency. Full oracle construction/sorting is
  untimed. Record order-sensitive and bag digests separately where appropriate.
- Retain elapsed nanoseconds, current and peak process RSS, baseline RSS before
  the operation, and phase-specific observations. A new child isolates process
  high-water marks between cases/sessions, but setup, warm-up and earlier samples
  in a five-sample batch contribute to the peak: disclose it. Sample
  RSS during long phases as well. Never subtract two process high-water marks
  and describe the result as the operation's peak allocation.
- Set a 600-second whole-batch deadline (setup, warm-up and every timed query)
  and a 60-minute per-fixture setup deadline;
  churn has a 60-minute per-case deadline. Stop a child before exceeding 80% of
  initially available node memory or 80% of initially free scratch space.
  Record timeout/resource-limit/error/correctness-failure dispositions, elapsed
  lower bounds, stderr and exit status. Missing or failed samples are not zeros.
  Escalating a bound creates a new explicitly recorded attempt.

## S: sorting, pagination and windows

Use exact input-fact rungs F = 100,000 / 1,000,000 / 10,000,000. The smoke rung
is F = 1,000 and never enters the performance report. Run the following targeted
families; do not multiply every independent axis into a single Cartesian sweep.

| Family | Fixture and axes | Queries and independent oracle |
| --- | --- | --- |
| S1 scalar order | F entities, one `:s/value` integer each; insertion ordinal maps to `(37*j) % F`; committed and pending | Ascending and descending; offsets 0 and F/2; limits 10 and 1,000, plus an unbounded full-sort/drain control in each direction. Expected page is the corresponding slice of integers 0..F-1. |
| S2 compound ties | N=floor(F/3) entities with value i, group i%4, score floor(i/4)%8; append F-3N unrelated padding facts; committed and pending | Port `pagination.rs`'s group-descending, score-ascending, value tie-suffix query. Offsets 137 and floor(N/2), limits 10 and 1,000. Sort independent `(group, score, value)` tuples with the complete comparator, then slice. Record N and padding separately. |
| S3 partitions | N=F/2 entities with `:s/part` and `:s/value`; value i. Partition layouts: one group, 100 equal groups, singleton groups, 90% of rows in one group with the rest spread across 99 groups | Running sum and row-number with partition/order; full drain and limit 10. Check each row against independent per-partition sorted arithmetic prefixes/ranks. The oracle uses the explicit deterministic partition assignment below. |
| S4 incompatible windows | S3's 100-group and skew fixtures | Two row-number windows with swapped partition/order keys, following `operator_paths.rs`. Check both window values for every row. Compare only with the equivalent single-window case on the same fixture, and call the difference additional end-to-end work rather than isolated sort cost. |
| S5 envelope | Existing `perf_envelope` fixture at 100k/1m/10m | Retain the existing query strings and actual generated cardinalities. Add independent checks of each query's values/counts; the example's aggregate output alone is insufficient raw-sample/correctness evidence for X06. |

S3/S4 run committed at every rung; pending controls run at 100k and 1m initially.
Ten-million-fact pending windows are an explicit deferred expansion, not implied
by S1's pending scalar case. Equal partitions use `i % 100`. Skew assigns the
first `9*N/10` rows to group 0, then `1 + (i-9*N/10) % 99`. Singletons use i.
All tested rungs make these divisions exact. Within each partition, i is unique.

The scalar query template is:

```clojure
(query [:find ?v :where [?e :s/value ?v]
        :order-by [?v :asc] :offset 0 :limit 10])
```

Substitute the specified direction/offset/limit, removing offset/limit for the
full-drain control. Reuse the actual window syntax from
`benches/minigraf_bench/operator_paths.rs`, with the S3 attributes substituted.
Record the fully expanded query, not just a family name. At 100k, additionally
bind limit/offset through `PreparedQuery` and compare the exact same page with
literal parse/execute. Keep parse cost out of prepared execution timing.

## T: wider set cardinalities and distributions

Hold entity count fixed at N = 10,000. Sweep set cardinality
K = 0, 1, 4, 8, 16, 32, 64, 256, 1,024. Add N = 100,000 for K = 4, 32, 256.
Use three distributions: repeated (`0..K` for every entity), unique
(`i*K..(i+1)*K`), and skew (90% repeated, final 10% unique, using disjoint values
for that tail). Run both pending and committed storage states.

Keep two independent fixture families: two facts per entity (`:t/id` and a
set-valued `:t/set`), and collection fixtures with `:t/id` plus K scalar
`:t/member` facts. The latter has N*(K+1) input facts. State this difference
when comparing set projection with `collect-set`; it is not isolated codec cost.
K=0 collections still have id facts and must return the empty set.

Measure set projection, hit/miss `in?`, `not-in?`, `all-in?`, `any-in?`, `none-in?`
and collection, plus checkpoint/open for stored sets. Use the operator forms in
`benches/minigraf_bench/set_queries.rs`; retain the exact probe members and their
independently computed selectivity. Include empty subset probes and a negative
integer absent from all fixtures. Compare every returned member/value with the
fixture formula, including canonical ordering/deduplication; counts alone do not
establish correctness. Do not assume that K=8 or K=16 is a codec boundary: retain
the observed encoded sizes/path before attributing a transition.

## U: sparse and repeated UUID projection

Construct a committed dictionary with D = 100,000 / 1,000,000 entities. Record
the assigned internal ID order during an untimed diagnostic pass; sorted UUID
order and insertion order must not be assumed to equal that order. Project
N = 10,000 / 100,000 source references (N <= D) under these patterns:

- Dense ascending and descending target-ID runs as controls.
- Uniform sparse targets at stride floor(D/N); retain the actual distinct count
  and span. Where stride is 1, label it dense and add stride 16 against D=1m
  with N=10k as the sparse control.
- Repeated targets with working sets 1, 16, 64 and 1,024, using `i % working_set`.
- A deterministic permutation of distinct targets, and 90% hot/10% sparse
  targets. Retain the permutation algorithm/seed and the exact target manifest.

Project the entity, the ref, both, and a duplicate ref column. Add focused
distinct-ID batches of 63, 64 and 65 with spans 127, 128 and 129 where possible
(span = max_id-min_id+1). Use explicit ID lists for these cutoff probes.
The current batched UUID path gathers at most 1,024 distinct hints and chooses
dense traversal at >=64 IDs and max_id-min_id <=2*count; confirm this at execution SHA.

Run all patterns with the default cache. At N=100k repeat dense ascending,
stride-16 sparse (N=10k), and working-set-1,024 controls with 0/4,096 pages and
fresh/warmed application caches. Record IdSpace retained bytes, dictionary reads,
cache hits/misses and path counts in a separate untimed diagnostic pass when
available. RSS and latency by themselves do not isolate DICT/cache behavior.
The oracle compares every projected UUID to the fixture manifest, including
duplicates and column identity; use a bag comparison if output order is unspecified.

## P: sustained pins, checkpoints and FactLog readers

Base fixtures contain 10,000 and 100,000 one-attribute entities. Each generation
updates a deterministic ring of 100 existing entities: retract the old integer,
assert the next integer, then checkpoint. Record 200 history records per round,
live fact count, committed generation, file bytes and allocated page count.
Append-only history grows even after pins release; file shrinking is not an
acceptance criterion. Use the same transaction/timestamp sequence for controls.

For each base size, use these independent reader schedules:

| Case | Pin schedule |
| --- | --- |
| P0 | No held reader; open/drain/drop a current cursor each round |
| P1 | One old query cursor held at the initial generation |
| P2 | Up to 32 cursors, one created each round, held until release phase |
| P3 | Rolling 32-generation cursor window, dropping oldest after each checkpoint |
| P4 | One long-held FactLog in storage order |
| P5 | One long-held transaction-order FactLog, default window and separately 4,096 |
| P6 | One initial cursor plus one initial FactLog, with rotating short readers |

Each run has 32 unpinned baseline generations, 256 held/rolling generations,
an explicit drop of every held object, and 256 further unpinned generations.
Observe before/after **each** checkpoint and immediately after release. Sample
old readers at ages 1, 32, 128 and 256 without exhausting them; then drain and
compare their full original snapshot before dropping. Keep a separate idle-held
variant if incremental draining materially changes retention. Readers created
later have their own expected snapshot/tx upper bound. Validate fresh cursors
throughout and reopen/verify the database at the end.

Record live pins and oldest pinned generation, quarantine unique-page count
and bytes, free-list count, pages reused/appended per checkpoint, process RSS,
checkpoint latency and per-reader buffered/metered state. The free list includes
quarantined pages: its size is not a count of currently reusable pages. Record
the intersection of post-release writes with the previously quarantined IDs to
demonstrate reuse. Assert no write overlaps pages still required by a live pin.
After the last drop, assert quarantine is empty and subsequent checkpoints
reuse eligible pages; compare growth slopes with P0, accounting for history.
If any assertion fails, retain the exact generation and stop treating the run
as performance evidence.

Direct instrumentation is required before P is executable: `Pins` and its
state are crate-private (`src/storage/pins.rs`), while current cursor/meter
counters are test-only. Extend a dedicated ignored test probe using the existing
`persistent_facts.rs::check_pin_holds_pages` recording-backend pattern; retain
the probe patch and its build hash. Verify it on small cases before scaling.
Do not add a public statistics API for this audit. Keep instrumented latency
separate from public-API latency and publish the observation overhead.

## R: matched recursive controls

Use a chain with depth 10/100/1,000, a width-10 depth-3 fanout tree, and a
width-10 depth-4 ancestor tree, with deterministic IDs and exact edge counts.
For ancestors select 1/16/128/129 demand roots; exercise pending in-memory and
populated committed file variants. A BFS/DFS oracle outside Minigraf computes
the complete reachable relation with bag/set semantics matched to the query.
Retain actual derived cardinalities and all rule-limit options.

Create one baseline plus one diagnostic variant per optimization below from
the **same source revision**, with identical fixtures, profile and limits:

| Optimization | Matched control and attribution gate |
| --- | --- |
| Cached probes | Disable only entity-probe cache lookup/insertion in `engine/join.rs`, preserving its streaming fallback. Use repeated narrow probes and a unique-probe negative control. Record hits/misses and verify the alternative consumes identical keys. |
| Delta ordering | Compare the current seeded `optimizer::plan_after` ordering in `engine/plan.rs` with a valid connected alternative order. Preserve the same Old/Delta/Full relation spans and head bindings. Record per-round plans/intermediate rows. An invalid or keyless product is not a meaningful control. |
| Bound-value AVET joins | Replace only the eligible committed bound-value lookup with a semantically equivalent scan-and-filter on the same committed fixture. Keep delta ordering and caching identical; record keys visited and prove the AVET path was exercised. |
| Small in-memory ancestor alternative | Disable only `recursive.rs::choose_run`'s alternative selection, preserving the planned default run. Include source cardinalities 128/129, empty/nonempty committed dictionaries and already-built pending-value-map controls. Record selection per round. |

These controls are not currently public flags. Before implementing each variant,
review whether the isolated edit preserves correctness and the other paths.
Retain the exact patch; never substitute a broad historical before/after revision
and attribute its whole difference to one optimization. Use the same oracle on
every variant, then three matched rounds with placement/order swapped. Report
paired raw samples and ratios, not just a percentage of medians across hosts.

If isolation is infeasible, specify the concrete coupling or missing observation
for that optimization, record the combined-path result as end-to-end only, and
leave its isolated contribution **unverified**. Do not sum individual speedups:
cache, ordering and bound-value probes interact.

## Evidence and completion gates

Each run directory must retain:

1. `run.json`: source/harness revisions, dirty patch hash, dependency/executable
   hashes, toolchain/build/options, affinity/cache/power/filesystem labels,
   timestamps, idle preflights, resource bounds and exact commands.
2. `cases.json`: every planned case with fully expanded query/rules, fixture
   formula/hash/cardinalities/skew, execution state, oracle and intended controls.
3. `samples.jsonl`: case/session/sample/variant/phase, raw latency/RSS/file and
   diagnostic observations, expected/actual count+digest, verification status,
   error/disposition, exit status. Unsupported metrics must be null with a reason.
4. `generations.jsonl`: P's per-generation pin/release/reuse observations; endpoint
   summaries alone are insufficient. Keep observation units explicit.
5. Raw stdout/stderr, fixture manifests, correctness logs, source patches,
   summaries and checksums. Reject duplicate/missing samples and any success
   record whose oracle failed. Keep interrupted/invalidated attempts distinguishable.

Archive verified evidence before removing task-created build/fixture scratch.
Regenerate summaries from raw files and independently reconcile the planned
matrix with measured, timed-out, failed and explicitly deferred cases. A capped
run defines a measured lower bound, not successful completion at that scale.

Publish a report distinguishing measured envelope, unmeasured cases, cache and
platform limitations, and each recursive attribution. Link it from X06 while
leaving X06 MANUAL / ENVELOPE. Any proposed nightly representative needs a separate
cost/correctness justification and routing change. A protocol or a runnable
harness alone does not satisfy #841's measurement acceptance criteria.

## Implementation and run order

1. Add the manual harness and strict evidence reconciler, with small oracle and
   malformed-evidence checks first. Keep fixtures lazy so listing cases does not
   allocate ten million facts. Run those checks and the relevant baseline suites
   on idle hardware before collecting performance data.
2. Implement/test private P/U observations and each feasible R control separately;
   preserve the uninstrumented executable for public API latency. Record any
   infeasible control rather than extending production APIs.
3. Smoke every case; run 100k, then 1m, then 10m, enforcing the recorded bounds.
   Build everything before measurements and recheck idleness after setup.
4. Run P and R in separate sessions; complete the release/reclamation phases,
   not just pin growth. Retain invalidated and capped cases.
5. Reconcile evidence, publish the measured envelope and remaining exclusions,
   then clean only this task's scratch. Review completion against every issue
   acceptance criterion before requesting closure.

Existing starting points are `examples/perf_envelope.rs`,
`benches/minigraf_bench/{pagination,set_queries,cursor_lifecycle,fact_log,operator_paths}.rs`,
and `src/storage/persistent_facts.rs`'s pin tests. The executable harness and evidence reconciler are in
[`scripts/shape-sweep`](../../../scripts/shape-sweep/README.md). Current
Criterion representatives and historical #530/#723 results must not be relabeled
as fresh #841 evidence.
