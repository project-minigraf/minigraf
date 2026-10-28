# #641: use proven input order in the streaming engine

Date: 2026-10-28. Base: `origin/v3` at `2abf53df05447755490d017bc2daf89f4d59cdf5`.
Status: Claude-reviewed; final conditional-approval corrections applied.
Maintainer approved implementation on 2026-10-28. Delivery is split into three slices (§9).
Issue: https://github.com/project-minigraf/minigraf/issues/641
Target: `v3`, following the issue's current v3.0.0 milestone. The older #306
spec called this v3.next; the current issue milestone takes precedence.

## Purpose and philosophy

Avoid a Sort when a scan/operator already produces the required full order;
reduce eligible grouped Aggregate state to one contiguous group; and buffer
only one Window partition when its input makes that possible. Preserve query
answers, grouping/peer rules, deterministic built-in aggregate errors, snapshot
isolation, and memory accounting. Existing top-k Sort already implements the
bounded `offset + limit` fallback and is retained.

This follows PHILOSOPHY.md §§4–8: reuse existing indexes and operators, add no
dependencies or configuration, and change no public API, syntax, database or WAL
format. Reliability takes priority over covering every possible index order.
Only proved cases use streaming strategies; other queries retain the existing
operators. This does not promise bounded memory for all grouped queries.

## Existing contracts that constrain the change

- A Scan emits committed ranges, pending facts, and derived rows in succession.
  Those runs are not one sorted stream. Pending keys use UUID/text order;
  committed entity/ident positions use DICT ids.
- `:order-by` sorts by explicit keys and every remaining `:find` column ascending.
  Matching only the explicit keys is insufficient for pagination.
- Index tag order, `Value::Ord` (group/partition equality and aggregate output),
  and `value_order` (Sort/window ranking/peers) are different comparators.
- Committed float keys canonicalize NaNs and distinguish signed zeros. Group
  equality equates all NaNs and equates the zeros. Composite grouping can split
  an equal group across different zero encodings.
- Long strings use prefix/hash/reference keys. Sets can have different encoded
  keys despite equal elements. Neither is presumed lexically ordered.
- Aggregate drains its input, finishes groups in `Value::Ord` grouping-key order,
  and returns no row until every group is validated. A first aggregate error can
  hide later errors or drop a group (`INT-042`). The new one-group mode admits
  only infallible built-ins over callback-free/data-error-free pipelines; fallible
  aggregates retain today's full-drain behavior, including under Limit.
- Window returns rows in input order. Its shared evaluator supports peers,
  deterministic row-number ties, lag/lead, row frames, and registered functions.
- ProbeValue can switch to either hash orientation while running. Joins and
  binding expressions can replace already-bound cells. Projected Distinct
  canonicalizes floats and set elements.

## 1. Two order properties, tied to a particular comparator

Add private engine metadata for:

1. **Lexicographic order**: a sequence of slot/comparator/direction entries and
   slots proved constant. Comparators distinguish encoded-id order, supported
   scalar value order, and `Value::Ord` group order. Consumers explicitly require
   the comparator they use; an id order is never presented as value order.
2. **Contiguous grouping/partition keys**: tuples whose equal values cannot
   reappear after a different tuple, tagged explicitly with the consumer's
   `Value::Ord` equality. This does
   not imply semantic ascending order. Constants and repeated slots can be
   removed when matching a requested key. A tuple prefix proves adjacency for
   that prefix, not for an arbitrary suffix or subset.

An Aggregate's complete grouping tuple additionally proves output uniqueness,
which can discharge later Sort tie columns once that entire tuple is matched.
Start conservatively: no general functional-dependency inference, index reversal,
statistics cache, schema/type annotation, or automatic merging of source runs.
Internal test fixtures may construct explicit proofs but production never trusts
a sample of rows. Unknown means use the existing operator.
For certified committed scalar/id slots, prove and test that `JoinKey::group`
classes coincide with `Value::Ord` classes before using the cell key as the
boundary test. Excluded string/set representations supply no such proof. Never
infer coarse-equality adjacency merely from a finer raw-key adjacency.

## 2. Bind physical choices to the pinned snapshot

The logical planner records order demand and legal physical alternatives.
Resolve these once for each Stream open, against its own snapshot/TxBound and
function registry. Prepared execution repeats that resolution; it does not reuse
an order claim from an earlier execution or checkpoint. Only the top-level plain
query pipeline is resolved; correlated/not/or bodies receive no order demand,
and `open_rules` keeps its current construction and strategies in this delivery.

An index-derived guarantee requires a committed key source, no pending or staged
record in the affected scan range, and no derived run. Use existing
`Snapshot::pending_len()` (which includes staged facts) for the zero-record fast
check. Otherwise add a private existence probe over Snapshot's captured pending
index view, short-circuiting at the first in-range record; no full pending clone,
net-assert or TxBound filtering. Assertions/retractions from any time force
fallback for that range; unrelated pending attributes do not disable it. Cover
staged facts through the same view and never use the live store's current length.
Later writes/checkpoints cannot change the choice for that pinned snapshot.
Pending-only/in-memory and derived scans keep the current plan.

The large-transaction `with_committed` path is eligible: `large_reader` exposes
the working MetaPage's complete four index roots and dictionary after finished
statements (`persistent_facts/large_tx.rs:218`, `:424`); it already incorporates
the prior pending store. Test read-own-writes, rejected statement rollback,
all index alternatives, and no stale proof across successive queries there.

Reuse existing selective index choices. Add an AVET alternative for a source scan
with constant attribute and unbound value when downstream demand needs value
order/adjacency; its cost must be justified by removing a buffering operator.
Do not replace a more selective entity/exact-value range by a whole attribute
scan. No whole-database scan is introduced to prove an order.
Only the outermost source scan supplies initial properties; optimizer join
order is unchanged. Initially require an unbound value position, no repeated
variable across source positions, and exactly one committed key range. Literal
keyword/ref/float/set matches can read several ranges or encodings, so supply
no order certificate here. `[:E :a ?v]` can retain its existing selective EAVT
range and use an AVET attribute kind proof as a conservative superset; no switch
to a broader range is needed.

For a single AVET attribute range, use at most a fixed number of raw
tag-prefix existence seeks through the Stream snapshot's own committed-reader
KeySource (the large transaction's working reader where applicable) to prove its values are
all one supported kind. Add an internal KeySource prefix-existence method backed
by a LeafCursor seek and at most one matching raw entry. Do not implement this
with `live_keys(max=1)`: finding no live key could walk arbitrarily many retracted
or excluded historical entries. Extend the internal kind-prefix helper as
necessary; do not hardcode byte tags in the planner. Only integers, booleans and canonical
committed floats initially prove scalar `value_order`. Empty input is trivially
ordered. Any other/multiple kind retains Sort. Over-approximating visibility is
safe: retracted/history keys and keys outside the transaction/valid-time filter
may cause fallback, never false proof. This trades optimization coverage for a
bounded number of tree seeks independently of historical data size.
I/O failure during optional proof reads abandons the optimization; the ordinary
query path remains authoritative for errors it would actually encounter.

For Window partition adjacency, EAVT/AEVT id prefixes and AVET integer/boolean/keyword/ref
value prefixes can be useful without matching semantic value order. Exclude
float/set/long-string grouping components unless the precise equality-adjacency
claim is proved. A *single* committed float grouping/partition column is safe:
the two zero encodings are consecutive and all committed NaNs are canonical.
Do not certify multi-column float grouping. A mixture of integer and float tags
also matches `Value::Ord` grouping order (integers before floats), while it does
not match `value_order` for Sort. No long-string/set order is inferred.

## 3. Propagation rules

These rules refer to the actual implementation, not to an operator's name:

| Operator | Initial rule |
|---|---|
| Filter, filtering Pseudo, Limit/Offset | Preserve properties on unchanged cells. |
| Bind New/Check, Pseudo Bind, CollectSet New/Check | Preserve properties on old unchanged slots; infer no order for new slots. |
| Bind Replace, CollectSet Replace, Join replacement | Truncate the guaranteed key prefix at the first changed component; clear affected adjacency claims. |
| Entity nested-loop probe | Outer rows are processed in sequence; preserve outer properties on unchanged outer slots. Infer no global order for new inner slots. |
| ProbeValue | Preserve an outer guarantee only if every possible fallback streams the outer in sequence (`build_inner=true`); otherwise clear it. |
| Hash join | Initially clear all guarantees, even if one orientation could preserve a prefix. |
| AntiJoin | Preserve unchanged outer properties; initially do not push a new index demand through a callback-containing body. |
| Union | Initially clear properties, despite its current outer-major implementation. |
| Distinct | Preserve properties on untouched slots; canonicalization in DistinctFind invalidates affected prefixes unless a specific monotonic/equality proof exists. |
| Hash Aggregate | Clear optimization proofs in this delivery; its output uses `Value::Ord`, which alone cannot satisfy Sort's different comparator. |
| Streaming Aggregate | Remap its certified scalar group domain/order to the output group slot, and mark that complete key unique. Convert to `value_order` only for one supported scalar kind, never mixed integers/floats. |
| Window | Preserve properties only on unchanged kept input slots; no inferred order for window results. |
| Sort | Establish its actual full `value_order` key on remapped output slots. |

Any query subtree containing user callbacks retains its existing physical access
order, Sort, and materialized aggregate/window behavior in this delivery. Disable
every #641 strategy for such a pipeline, even if no source index would change.
Resolve this
by registry implementation descriptors and recursive expression/body traversal,
not by guessed names or text matching. Rule-derived input remains a fallback.
Do not reuse plan::udfs as a complete detector: it sees only top-level unary
UDF clauses. Traverse nested expressions, bind clauses, nested not/or bodies,
aggregate descriptors and WindowFunc::Udf explicitly.
CollectSet also disables access reordering and early-stopping strategies in its
containing pipeline: its `QRY-015` data error must not disappear behind Limit.
Apply this restriction recursively through not/or bodies and rule plans.

## 4. Sort omission

No user callback or CollectSet may occur anywhere in the optimized top-level
pipeline or its nested bodies. If either occurs, keep Sort and the original
access path, even under Limit.

At the final Sort decision, translate every full-order column through `plan.find`
to its producing slot. Remove only proved constants and repeated slots. Omit
Sort only when the remaining complete lexicographic order follows from the input
proof with matching comparator and directions. A stronger input key can satisfy
a prefix of itself; an unrequested leading key cannot be ignored.
Once a complete unique grouping tuple is matched, later columns cannot change
the relative order of any groups and require no further ordering proof.

At open-time removal of `Step::Sort`, restore `plan.find = Sort.cols`: the logical
planner has already moved the original slots there and remapped `find` to
`0..find.len()`. Simply deleting the step would project the wrong slots. The
resolver must restore that mapping even with a following Limit. Limit/Offset remains above the chosen child. No descending
index scan is added. A missing proof keeps today's full Sort or top-k heap.

Positive example: a committed integer attribute, `[:find ?v :where [?e :score ?v]
:order-by [?v] :limit 10]`, can read AVET and stop after the required batches.
Counterexample: adding `?e` to `:find` requires UUID tie order and keeps Sort;
DICT entity ids do not satisfy that suffix. Mixed integer/float values also
keep Sort despite looking numeric.

Omitting Sort allows a cursor to return batches before a later IO/allocation
error, and Limit can avoid errors in unread input. This is ordinary cursor
laziness, not a promise to validate unread storage. User callbacks and CollectSet
disable omission; fallible materialized Aggregate still drains/validates before
returning any row. Test both callback counts and later data-error propagation.

## 5. One-group Aggregate without hiding data errors

Use streaming only when there is exactly one grouping column, `:with` is empty,
the query has no window, input is proved increasing by that column under
`Value::Ord`, and every aggregate's descriptor is built-in `count` or
`count-distinct`. These finish without an `INT-041/043/044` data error for arbitrary
argument values (`Agg::finish`); count-distinct retains its existing set growth
and allocation errors. Exclude callbacks and CollectSet throughout the input
pipeline, with or without a Limit.

Initially certify integer, boolean or canonical committed-float group columns,
and mixed integer/float columns (the tag order matches `Value::Ord`). Do not
stream DICT-ordered entity/keyword groups merely because they are adjacent:
their output would violate the existing semantic group order. Multi-column
grouping, `:with`, sum/sum-distinct/min/max, registered aggregates, and unknown
order keep the existing hash-grouped strategy. Ungrouped behavior is unchanged.
Grouped `avg` is not supported by the current parser/Agg implementation; this
optimization adds no aggregate function. Window `avg` remains supported.

Retain one group's representative cells and existing `Acc`s, an input batch and
position, and a bounded output batch. Compare boundaries by group equality,
so both zero signs stay in one group. Finish a group, append its result to the
output batch, and release its state before starting the next. Output follows
the same `Value::Ord` key order and float representative choice as today's hash
path. Reuse a one-group finish helper and preserve empty/zero-row rules. Do not
retain all finished results. Limit can stop after its final group, with the
usual batch/lookahead overread.

Memory is one group's accumulators/distinct members and bounded input/output
batches, independent of completed group count. A single huge count-distinct
group can still grow. IO/allocation errors may follow batches already returned
to a cursor, as its contract allows; execute returns an error on full drain.
No aggregate data error can be hidden: fallible aggregates retain full drain.

## 6. Partition-at-a-time Window

Use a partition strategy when all windows have the same nonconstant
`:partition-by` input slot, that slot has group-equality adjacency, and the
affected subtree contains no user callbacks. Windows without a partition, mixed
partition keys or unknown adjacency keep today's full-input strategy. CollectSet
also forces fallback. Initially exclude aggregate queries from this strategy,
keeping the combination of strategy changes bounded in scope.

Hold the current partition's kept cells and named values, plus a bounded
lookahead batch and row position. On a boundary, invoke the existing `windows`
evaluator for that partition, including each window's own sort/peers/frame,
then return its rows in original input order. Emit/release its result columns
before retaining the next complete partition. Reset rank, row-number, running
aggregates, neighbor reads and row frames at every boundary. An empty partition
is never emitted; partition boundaries crossing batches work the same way.

No retained full-query position vector, result list or partition index. The
partition operator's memory is O(largest partition), output batches and existing
window state; a surrounding Sort/Distinct/Aggregate can still dominate query
memory. A Limit may stop before later partitions; this is an expected benefit
for built-in window pipelines. UDF windows/predicates/aggregates and unproved
paths retain their current behavior. I/O/allocation errors occur when the cursor
pulls the corresponding input, as for other streaming operators.

Without a final query `:order-by`, row order is unspecified across physical
plans, as with current optimizer reordering. An AVET alternative can change that
encounter order; Window preserves its chosen input's order, with the same answer
bag and window values. Compare lists for a fixed chosen input order and bags
across index alternatives unless an explicit final order is requested.

Retain DICT caches for a whole query only for the existing full-input Window
strategy. Partition windows allow normal `IdSpace::end_pull` eviction; ids and
value-page references remain valid under the pinned snapshot. Test per-query
IdSpace peak as well as Window's own meter: a bounded operator with an unbounded
cache is not accepted as bounded query memory.
Streaming Aggregate and partition Window also evict transient DICT lookup caches
at safe internal child-batch boundaries; waiting for Stream::next_rows alone is
insufficient when a single operator pull consumes many input batches. This never
evicts provisional ids or invalidates retained Cell ids/value references.

## 7. Accounting and lifecycle

Charge retained batches/lookahead, keys, accumulator/distinct members, result
rows, partition values and window columns through existing Held/Meter paths.
Charge before growth; failed `try_reserve` reports `QRY-012`. Transfers between
buffers are charged once for the actual live ownership. Accounting continues
through errors, partial pulls, rewind, close and drop. The profiler is not yet
public; private test strategy markers/counters distinguish choices for tests and #185.
No production feature/config switch is added.

## 8. Required correctness tests

- Forced optimized/fallback strategies over the exact same snapshot: compare
  ordered lists (including float bits), unordered bags and error codes. Use a
  private/test-hooks switch, not a public API. No removed executor oracle.
  Frozen answer digests compare bags and are an additional regression guard,
  never the ordered oracle; do not re-bless tests/answers for this optimization.
- Sort positive and fallback examples, repeated projected columns/constants,
  complete tie suffixes, descending keys, mixed numeric kinds, NaNs/zeros,
  scrambled DICT allocation, long strings sharing prefixes, sets and nil.
- Proofs operate on full ranges, not first-row samples. Put a disqualifying kind
  late in an attribute, in historical data, or outside current valid time.
- Reopen prepared queries before/after pending writes and checkpoint; in-range
  mixed/pending/staged and rule-derived paths use fallback; unrelated pending
  attributes and large-transaction working snapshots exercise eligible paths. Open cursor
  keeps its result while another writer checkpoints/reuses pages.
- Join preservation and invalidation, particularly replacements and a mid-run
  ProbeValue fallback that builds the outer. Canonicalizing Distinct and nested
  not/or bodies cannot accidentally resurrect a discarded proof.
- Streaming groups spanning batches, nil/empty groups and zero row, counts over
  arbitrary argument kinds, signed-zero representatives and distinct release.
  Fallible aggregates, `:with` and multi-key grouping retain materialized behavior:
  a semantic earlier error encountered last, dropped-group aggregate precedence,
  upstream failures and invalid groups after a limited prefix retain their errors.
  UDF callback sequences and CollectSet failures remain unchanged.
- Window partition boundaries at and across batches, one huge/many small
  partitions, all built-ins including lag/lead and bounded row frames, peers,
  multiple compatible windows, incompatible/unpartitioned fallbacks, input-order
  preservation and Limit/Offset. Include id-heavy retained rows.
- Every holder and IdSpace releases memory on success/error/close/drop/rewind.
  Enforce small-worker-stack, binary-size and portable-target gates.

At group/partition boundaries validate monotonicity with the physical/semantic
comparator carried by the certificate; a violation fails with existing INT-032,
never falls back after emitting rows. This is a defensive check, not permission
to weaken planner proofs. Include injected out-of-order operator fixtures.

## 8.1 Acceptance gates

- On the positive ordered scalar fixture, scan rows read are at most
  `offset + limit + BATCH_ROWS`; zero limit reads no facts. Streaming aggregate
  group-limit reads at most all rows of the requested groups plus one input
  batch, with maximum fixture group size stated. No proof reads occur with no
  applicable consumer demand, measured by counters rather than timings.
- On fixed-size partitions/groups at 100K/1M/10M committed facts, streaming
  Window/Aggregate and transient IdSpace peak must stay within the 100K peak plus
  two maximally-sized charged input batches. Compute that allowance using the
  test fixture's cell/row byte helpers. No finished-results list is allowed.
  Distinct members within a growing single group and a growing single Window
  partition are explicitly linear controls, not bounded-memory claims.
- In query-only subprocess measurements on prebuilt fixtures, peak RSS at 10M
  must be within the 100K baseline plus max(16 MiB, 15%). Investigate any failure
  using engine holder/IdSpace peaks; do not count fixture-building memory as
  query memory. Existing buffering paths need no claim of flat memory.
- For the bounded positive limit fixtures, 10M median query time is at most
  twice 100K after warmup, with at least five idle-machine A/B samples. Existing
  #379 point-query medians must not regress by more than 5%; investigate noise
  before trusting either result. Include proof-startup cost as a separate case.
- Required tests/coverage/portability checks pass, the 1,230 KiB release binary
  cap remains unchanged, and frozen answers are unchanged. Run an optimized
  committed-memory-backend fixture on WASI and a corresponding browser fixture
  where its runner is available, rather than exercising only in-memory fallback.

## 9. Measurement and delivery

Three dependent PRs to `v3`: (1) order metadata, conservative scan choices and
Sort omission; (2) streaming Aggregate; (3) partition Window and cache lifetime.
Keep #641 open until all three land; do not close #473. No release/tag bump.
Specs/plans and the exact fallback scope accompany PR 1.
Unique-group Sort omission lands with streaming Aggregate in PR 2; PR 1 does
not infer a Sort-compatible order from the existing hash Aggregate.

Run all Cargo tests, clippy, coverage and measurements on the designated idle
bench machine first. Non-timing checks use the whole machine; timing runs use
the established NUMA A/B protocol with immutable base/PR revisions and at least
five samples after warmup. Keep private machine details out of committed docs.
Record command, revision, workload, engine peak, RSS, median and range.

At 100K, 1M and 10M committed facts: ordered value-only limit/offset versus Sort
fallback (scan counters establish early stopping); high-cardinality grouped
count/count-distinct (one active group, bounded output batches, and
materialized sum-distinct as a fallback control); fixed-size Window partitions with total rows growing
(Window/IdSpace peak bounded by partition size); one growing partition (document
linear bound). Include mixed/pending, tie-suffix and type fallbacks as controls.
Run the relevant existing envelope and #379 point-query guards, avoiding timing
claims from the laptop. Review proof-seek overhead on small/selective queries;
ordinary queries without an applicable demand should perform no proof reads.

Record benchmark IDs/filter coverage for #776; adding a benchmark is not evidence
that the nightly filter runs it. Reuse existing CI routes where sufficient;
coordinate shared workflow changes on both release branches if needed.

Synchronize affected architecture/status/perf documentation across CLAUDE.md,
ROADMAP.md, README.md, CHANGELOG.md and docs/TEST_COVERAGE.md. Update the docs site
architecture description in its own PR with `@since v3.0.0` if release targeting
remains as currently assigned; cross-check the reference/comparison/use-cases
pages and change only claims affected by this work. No new syntax/error entries.
Claude reviews each code slice and the combined evidence before PR creation.
Delete task-created remote scratch after runs unless explicitly reused, report
anything retained and why; do not remove preexisting shared caches or fixtures.
