# Certified ordered aggregates (#781)

Date: 2026-10-29. Baseline: merged #789, `f1019c5a1`; measured implementation:
`a8afaee12`. Runtime code is unchanged from `90cb88924`; the later commit
strengthens native tests and adds the verified measurement harnesses. Later
nightly-test/documentation updates leave the measured runtime and probes unchanged. The [build manifest](781-ordered-aggregate-build.json)
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

Timing probes use the established `envelope` profile (opt-level 3) and
`-C force-frame-pointers=yes`, with the same Cargo.lock and Rust 1.99.1 for
both revisions. Holder diagnostics use the private release test executable;
the release-size gate uses the separate default size-optimized release artifact.
Each timing process has a 256 KiB worker stack; open and thread creation are
outside the timed drain. Group probes warm up once, then run at least five
drains and one second (capped at 20,000 drains). Two sequential process rounds
swap baseline/implementation CPU and memory placement between NUMA nodes 1
and 3. Guards use three warmups and six alternating process rounds. The machine
is idle during timings; its existing schedutil governor and enabled turbo are
recorded without changing root settings.

The [supplemental size-profile report](781-ordered-aggregate-size-profile.md)
preserves the complete earlier opt-level-z matrix. The primary timing run uses
the established application-profile flags. All task-owned build artifacts and
copied fixtures are synced before the timed processes start.

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

All [240 timing records](781-ordered-aggregate.jsonl) include per-process median/min/max, drain count and VmHWM. Each case has at least ten timed drains per variant across two fresh processes. Tables use the median of the two process medians; RSS uses the larger process high-water mark. The [machine-readable summary](781-ordered-aggregate-summary.json) includes every case and acceptance calculation.

At 10M facts, with 625,000 fixed-size groups:

| Shape | Base full drain (s) | Ordered full drain (s) | Change | Ordered RSS (KiB) | Ordered limit 10 (ms) |
|---|---:|---:|---:|---:|---:|
| `avg` | 6.075 | 4.492 | -26.06% | 5,228 | 0.600 |
| `sum` | 6.018 | 4.412 | -26.69% | 5,248 | 0.603 |
| `sum-float` | 7.633 | 6.292 | -17.57% | 5,276 | 0.801 |
| `extrema` | 6.877 | 4.734 | -31.16% | 5,200 | 0.620 |
| `sum-distinct` | 6.147 | 4.614 | -24.93% | 5,176 | 0.625 |
| `avg-float` | 7.604 | 6.285 | -17.35% | 5,248 | 0.805 |
| `tuple-avg` | 7.109 | 5.581 | -21.49% | 5,224 | 0.706 |
| `tuple-count` | 6.992 | 5.388 | -22.94% | 5,220 | 0.694 |
| `tuple-distinct` | 8.012 | 5.725 | -28.55% | 5,212 | 0.728 |

Baseline limit-10 medians are 5.572–7.389 seconds: they still materialize all groups. Ordered limit-10 medians are 0.600–0.805 milliseconds. Full drains improve by 17.35–31.16%.

| Limited shape | 100K (ms) | 1M (ms) | 10M (ms) | 10M / 100K |
|---|---:|---:|---:|---:|
| `avg` | 0.617 | 0.620 | 0.600 | 0.971 |
| `sum` | 0.611 | 0.601 | 0.603 | 0.987 |
| `sum-float` | 0.808 | 0.796 | 0.801 | 0.992 |
| `extrema` | 0.637 | 0.646 | 0.620 | 0.973 |
| `sum-distinct` | 0.614 | 0.613 | 0.625 | 1.018 |
| `avg-float` | 0.819 | 0.805 | 0.805 | 0.984 |
| `tuple-avg` | 0.721 | 0.717 | 0.706 | 0.979 |
| `tuple-count` | 0.706 | 0.699 | 0.694 | 0.983 |
| `tuple-distinct` | 0.742 | 0.732 | 0.728 | 0.982 |

All nine limited shapes pass the 2× startup gate; the largest ratio is **1.018**. All 18 full/limited bounded RSS comparisons pass: the largest 100K→10M increase is **192 KiB**, below the **16,384 KiB** allowance. These process measurements include open/runtime overhead; the separate holder results above explain the retained state.

The retained-Sort `hidden-avg` control is excluded from those bounded claims. Its ordered full drain takes **6.358 s** and peaks at **109,400 KiB**. Limit 10 reduces RSS to **5,088 KiB** through top-k sorting, but still takes **5.466 s** because Sort drains the child.

## Ordinary, history and fallback guards

The [360 ordinary-envelope records](781-ordered-aggregate-envelope.jsonl) cover all 30 existing #641 ordinary cases at nominal 100K/1M/10M scales, with six balanced process rounds per variant. Their median changes range from **-3.60% to +1.04%**; none exceeds the +5% regression ceiling.

The [396 supporting guard records](781-ordered-aggregate-guards.jsonl) also retain six balanced process rounds. The 12 unchanged #379 history cases at depths 1, 144 and 2,000 change by **-1.73% to +0.97%**. Supporting point/scan/limit/order controls change by **-4.50% to +1.31%**. The no-demand aggregate case changes by **-4.08% to -1.70%**; reader instrumentation separately proves zero optional kind-proof reads.

The `fallback_dynamic_min` control at 100K changes by **-0.14%**.
The `fallback_float_distinct` control at 100K changes by **-0.47%**.

## Proof startup and argument controls

Very small grouped point queries become slower. Their EAVT entity range remains selective, but the source's whole-attribute kind proof adds **176–188 µs**. These are newly eligible aggregate queries, reported separately from unchanged ordinary-query guards; this patch has no cost-based choice.

| Facts | Avg base → ordered (µs) | Min base → ordered (µs) |
|---:|---:|---:|
| 100,000 | 30.254 → 218.000 | 29.718 → 216.528 |
| 1,000,000 | 30.191 → 208.178 | 30.000 → 206.334 |
| 10,000,000 | 30.541 → 206.800 | 30.297 → 206.209 |

The [48 argument-control records](781-ordered-aggregate-argument-guards.jsonl) cover joined scalar min/max/integer sum-distinct and raw-history kind rejection. At 100K, the joined scalar query changes from **235.456 ms** to **38.251 ms**. The history control has a String grouping key: its source-kind proof rejects ordered grouping before argument certification, even at an earlier snapshot. It measures conservative source-proof startup, not an isolated argument-domain scan:

| History depth | Base (µs) | Ordered candidate fallback (µs) | Added (µs) |
|---:|---:|---:|---:|
| 1 | 21.409 | 111.087 | 89.678 |
| 144 | 42.782 | 136.791 | 94.010 |
| 2,000 | 419.313 | 542.265 | 122.952 |

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
The 100K–10M acceptance measurements use the standalone probe. The existing
1M-group nightly memory acceptance test also passes after replacing its stale
sum-failure assertion with bounded-sum success and a floating-distinct fallback
QRY-012 control; it runs under the repository's actual `bench` profile.

## Reproduction

Use isolated checkouts of the recorded revisions on the bench. Resolve and
copy one Cargo.lock between them before building; its measured hash is in the
manifest. Build both with
`RUSTFLAGS='-C force-frame-pointers=yes' cargo build --profile envelope --lib`. Compile the same probe source
against each library:

```sh
rustc --edition=2024 -C opt-level=3 -C panic=abort -C force-frame-pointers=yes \
  scripts/generalized_group_probe.rs \
  --extern minigraf=target/envelope/libminigraf.rlib \
  -L dependency=target/envelope/deps -o group-probe
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
