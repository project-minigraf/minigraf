# Size-optimized release profile: supplemental #781 evidence

These results use the REPL’s size-optimized `release` profile (`opt-level="z"`).
The [primary report](781-ordered-aggregate.md) uses the established library
profile (`envelope`, opt-level 3 with frame pointers).

Date: 2026-10-29. Baseline: merged #789, `f1019c5a1`; measured implementation:
`90cb88924`. Subsequent edits strengthen native metadata/dictionary tests,
add the measurement harnesses and document the results; runtime code is unchanged. The [build manifest](781-ordered-aggregate-size-profile-build.json)
records source/library/probe hashes, compiler, CPU conditions and fixture sizes.

## Scope

The engine can release completed groups for count, count-distinct, sum and avg
when it proves their complete grouping order. Min/max additionally require
Integer/Float/Boolean/nil arguments; sum-distinct requires Integer/nil arguments.
Raw argument proofs include all committed attribute history, including
retractions and future valid times, and are invalidated by pending/staged writes.
They use the pinned reader and are cached per attribute per query open.

An effective tuple can include hidden `:with` keys, repeated slots and proved
Integer/Boolean/nil constants if it reduces to one ascending Integer/Boolean
dimension. Existing single numeric/Boolean grouping remains supported; mixed
numeric keys retain sorting. Every `:with` query retains any final Sort.
Callbacks, collect-set, rules, aggregate+window queries, arbitrary payload
extrema, floating distinct sums and unproved arguments retain full-input
evaluation. All-constant and independently varying tuples also fall back.

The approved error policy is explicit: common output scratch/row allocation or
meter failures may report QRY-012 before a later unread source failure. No
aggregate-specific fallible finalizer is enabled by this exception. Tests compare
the resource failure against full drain and retain registered-finalizer error
precedence. A limit counts emitted groups, so dropped extrema groups and a
retained Sort can prevent bounded startup.

## Protocol

Fixtures are built outside measuring processes with
`scripts/input_order_group_fixture.rs`: one integer `:group` fact per entity,
16 facts per group, at 100,000 / 1,000,000 / 10,000,000 committed facts. The
growing controls instead have one group and one fact per transaction, making
`:db/tx-count` a unique integer argument. Measurements open the checkpointed
file read-only. Results are checked and discarded batch by batch.

Release probes use the same Cargo.lock and Rust 1.99.1 for both revisions.
Each timing process has a 256 KiB worker stack; open and thread creation are
outside the timed drain. Group probes warm up once, then run at least five
drains and one second (capped at 20,000 drains). Two sequential process rounds
swap baseline/implementation CPU and memory placement between NUMA nodes 1
and 3. Guards use three warmups and six alternating process rounds. The machine
is idle during timings; its existing schedutil governor and enabled turbo are
recorded without changing root settings.

Cargo's benchmark profile shares output paths with release builds. After the
Criterion smoke run, `cargo build --release` restored the release artifacts;
both probes were recompiled and their hashes recorded before timings started.
The unstripped benchmark binary is not the release-size artifact.

## Holder memory and early stop

All [126 holder records](781-ordered-aggregate-meter.jsonl) are retained. The
private release tests compare ordered execution with forced full-input
evaluation of the same queries. Every fixed-group Aggregate peak is identical
at 100K, 1M and 10M facts, both with full drain and with limit 10. IdSpace peaks
are **zero** for these scalar-output fixtures; this does not claim zero dictionary
memory for arbitrary joins or reference outputs.

| Shape | Ordered Aggregate peak | Forced full input at 10M |
|---|---:|---:|
| Integer sum / avg | 480 B | 285,000,000 B |
| Floating sum / avg | 768 B | 465,000,000 B |
| Min and max together | 688 B | 385,000,000 B |
| Integer sum-distinct | 528 B | 315,000,000 B |
| Constant-first tuple avg / count | 560 B | 355,000,000 B |
| Hidden-key avg, without Sort | 480 B | 285,000,000 B |
| Repeated-key count-distinct | 1,200 B | 755,000,000 B |

These peaks include the active group, finish scratch and temporarily charged
result row. The first finite float adds one 36-limb Exact allocation:
**36 × 8 = 288 bytes** per sum/avg accumulator. Passing batches remain uncharged,
as in #549; they are not retained by the ordered Aggregate between calls.

On this 64-bit target, `Cell` is 24 bytes, a vector header is 24 bytes and
`Batch` is 32 bytes. A two-column batch allowance is
`2 × (32 + 2 × 24 + 1024 × 2 × 24) = 98,464 bytes`; three-column shapes use
`2 × (32 + 3 × 24 + 1024 × 3 × 24) = 147,664 bytes`. Float Exact storage is
already included in each measured 100K baseline (768 versus 480 bytes).
The factor two is the agreed two-batch slack; the width is the larger of the
fixture's input and output row widths.
These are numerical slack allowances, not charges for passing batches.
Zero peak growth satisfies both the Aggregate and IdSpace gates.

Every limited positive holder fixture consumes **1,024 child rows**. With offset
2 and limit 10, the bound is `(2 + 10) × 16 + 1024 = 1216` rows; hidden-key avg
without Sort has no offset and satisfies `10 × 16 + 1024 = 1184`.
Full-input controls consume all N rows even with a limit.

The hidden-key holder fixture deliberately omits Sort to isolate Aggregate's
bound. The public timing probe includes its required final Sort and is a
separate retained-sort control; its whole-query memory/startup is not advertised
as bounded.

## Growing distinct controls

One group with N distinct integer transaction counts retains **40 × N + 480**
Aggregate bytes for count-distinct and **48 × N + 480** for sum-distinct.
At 10M those are 400,000,480 and 480,000,480 bytes. Both produce the exact
expected result (N or N × (N + 1) / 2) and release all charges on drain.
IdSpace is zero because these are integer arguments. The linear distinct sets
are expected; ordered execution bounds completed groups, not the active group.

## Timings and process RSS

All [240 timing records](781-ordered-aggregate-size-profile.jsonl) include per-process
median/min/max, drain count and VmHWM. Each case has at least ten timed drains
per variant across two fresh processes. The tables use the median of the two
process medians; RSS uses the larger process high-water mark.

At 10M facts, with 625,000 fixed-size groups:

| Shape | Base full drain (s) | Ordered full drain (s) | Change | Ordered RSS (KiB) | Ordered limit 10 (ms) |
|---|---:|---:|---:|---:|---:|
| `avg` | 9.691 | 7.439 | -23.24% | 4,892 | 0.938 |
| `sum` | 9.743 | 7.310 | -24.97% | 4,896 | 0.942 |
| `sum-float` | 11.654 | 9.925 | -14.83% | 4,932 | 1.214 |
| `extrema` | 10.148 | 7.769 | -23.44% | 4,952 | 0.982 |
| `sum-distinct` | 9.707 | 7.732 | -20.35% | 4,900 | 0.999 |
| `avg-float` | 11.643 | 9.931 | -14.71% | 4,916 | 1.235 |
| `tuple-avg` | 10.743 | 8.848 | -17.64% | 4,952 | 1.123 |
| `tuple-count` | 10.378 | 8.566 | -17.46% | 4,892 | 1.079 |
| `tuple-distinct` | 12.464 | 9.778 | -21.55% | 4,920 | 1.248 |

Baseline limit-10 medians are 8.644–11.534 seconds for these shapes: they still
materialize all groups. Ordered limit-10 medians are 0.938–1.248 milliseconds.
Full drains improve by 14.71–24.97%; the larger gain is releasing completed
state and avoiding later input when a limit permits it.

| Limited shape | 100K (ms) | 1M (ms) | 10M (ms) | 10M / 100K |
|---|---:|---:|---:|---:|
| `avg` | 0.989 | 0.961 | 0.938 | 0.949 |
| `sum` | 0.972 | 0.960 | 0.942 | 0.969 |
| `sum-float` | 1.233 | 1.216 | 1.214 | 0.984 |
| `extrema` | 1.015 | 0.999 | 0.982 | 0.968 |
| `sum-distinct` | 1.014 | 1.001 | 0.999 | 0.985 |
| `avg-float` | 1.243 | 1.221 | 1.235 | 0.994 |
| `tuple-avg` | 1.131 | 1.105 | 1.123 | 0.993 |
| `tuple-count` | 1.090 | 1.075 | 1.079 | 0.991 |
| `tuple-distinct` | 1.242 | 1.216 | 1.248 | 1.005 |

All nine limited shapes pass the 2× startup gate; the largest ratio is
**1.005**. All 18 full/limited bounded RSS comparisons pass: the largest
100K→10M increase is **164 KiB**, below the **16,384 KiB** allowance. These
process measurements include open/runtime overhead; the separate holder results
above explain the retained state.

The retained-Sort `hidden-avg` control is deliberately excluded from those
bounded claims. Its ordered full drain takes **9.821 s** and peaks at
**109,072 KiB**. Limit 10 reduces RSS to
**4,744 KiB** through top-k sorting, but still takes
**8.786 s** because Sort drains the child.

## Selective startup, fallback and history guards

All [396 guard records](781-ordered-aggregate-size-profile-guards.jsonl) retain six balanced
process rounds. The 12 unchanged #379 history cases use depths 1, 144 and 2,000;
median changes range from **−5.99% to +0.36%**, within the +5% regression ceiling.
The supporting point/scan/limit/order controls range from **−2.03% to +1.07%**.
The new no-demand aggregate case changes by at most **+1.43%**; direct reader
instrumentation separately proves it makes zero optional kind-proof reads.
Dynamic-argument extrema and floating distinct-sum fallback controls change by
**+0.13%** and **+0.24%** at 100K.

Very small grouped point queries become slower. Their EAVT entity range remains
selective, but the source's whole-attribute kind proof adds roughly 259–275 µs.
These are newly eligible aggregate queries, reported separately from the
unchanged ordinary-query guards; there is no cost-based choice in this patch.

| Facts | Avg base → ordered (µs) | Min base → ordered (µs) |
|---:|---:|---:|
| 100,000 | 49.846 → 324.494 | 49.580 → 323.703 |
| 1,000,000 | 50.278 → 310.525 | 49.884 → 310.919 |
| 10,000,000 | 50.082 → 308.685 | 50.098 → 309.651 |

The [360 ordinary-envelope records](781-ordered-aggregate-size-profile-envelope.jsonl)
cover all 30 existing ordinary cases across six balanced rounds, with median
changes from −3.25% to +2.02%. The [48 argument-control records](781-ordered-aggregate-size-profile-argument-guards.jsonl)
show the joined scalar min/max/integer-distinct query at 100K changing from
386.26 ms to 64.56 ms. The historical String grouping key rejects ordered grouping at the source-kind
proof, before argument certification; its full-input fallback adds startup: 37.77 → 163.15 µs at depth 1,
79.79 → 214.75 µs at depth 144, and 788.09 → 958.87 µs at depth 2,000.
The [complete supplemental summary](781-ordered-aggregate-size-profile-summary.json)
retains each case and acceptance calculation. These remain size-profile results;
the primary report contains the application-profile acceptance measurements.

## Validation and nightly routing

The exact implementation passes the full default suite, 202 engine tests,
targeted test-hook plan comparisons, native/browser/WASI strict lint, WASI
execution and release build, and 63 Firefox tests. See
[TEST_COVERAGE](../TEST_COVERAGE.md) for inventory and run boundaries.
The release binary is **1,263,592 bytes**, 24 bytes below the unchanged
**1,263,616-byte** cap (baseline: 1,261,736 bytes). No dependency, syntax,
configuration, public API or file/WAL format changes are introduced.

The existing nightly `^query/` filter includes the new
`query/ordered_groups/{avg,sum,min,max,sum_distinct}{,_limit}/{1k,10k,100k,1m}`
IDs. All ten new **1k** IDs were actually executed using Criterion `--test`;
that is a smoke check, not a claim that the larger nightly IDs were timed here.
The 100K–10M acceptance measurements use the standalone probe.

## Reproduction

Use isolated checkouts of the recorded revisions on the bench. Resolve and
copy one Cargo.lock between them before building; its measured hash is in the
manifest. Build both with `cargo build --release`. Compile the same probe source
against each library:

```sh
rustc --edition=2024 -O -C panic=abort scripts/generalized_group_probe.rs \
  --extern minigraf=target/release/libminigraf.rlib \
  -L dependency=target/release/deps -o group-probe
```

Build `input_order_group_fixture.rs` the same way against the base library.
For each N in 100000, 1000000 and 10000000, create a fresh fixed fixture with
`group-fixture fixed.graph N 16`, or a growing fixture with
`group-fixture growing.graph N N 1`. Never create fixtures in a query timing
process. Drain a fixed fixture with
`group-probe fixed.graph N 16 avg 0 0`, or use `avg 10 2` for pagination.
The other mode names appear in the probe's match expression. Pin each process
with `numactl --cpunodebind=1 --membind=1` (node 3 for the other variant),
then swap the assignments for the next round.

For private holder evidence, build `cargo test --release --lib --no-run` and
invoke its reported test executable with
`generalized_committed_memory_probe --ignored --nocapture --test-threads=1`.
Set `MINIGRAF_GROUP_FIXTURE`, `MINIGRAF_GROUP_ROWS=N` and
`MINIGRAF_GROUP_SIZE=16`; set `MINIGRAF_GROUP_FULL=1` for the forced full-input
control. The `certified_growing_distinct_memory_probe` test uses the growing
fixture and ROWS variable. Run these separately from timing processes.

The public probes validate exact rows and discard batches. Their RSS is the
query process's VmHWM, including database open and runtime overhead, while the
private meter reports estimated retained operator state. These quantities
measure different things and must not be equated.
