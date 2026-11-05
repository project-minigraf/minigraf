# X06: manual shape sweeps and recursive controls

This report records the executed finite matrix of
[#841](https://github.com/project-minigraf/minigraf/issues/841): every planned
slot, including rejected inputs, capped batches, limit errors and deferred
slots, with its disposition. The [protocol](PROTOCOL.md) also describes broader
extensions that were not selected (see [Remaining scope](#remaining-scope)),
and the [harness runbook](../../../scripts/shape-sweep/README.md) contains
commands.

The core revision is `5a0ff549271084018c7d4e6050658e64e7e6432e` on `v3`.
The harness lives outside the library workspace. Diagnostic changes are applied
only in a disposable checkout. There is no production API, nightly-route,
release-acceptance, or regular Rust test-inventory change.

## Evidence

[evidence.tar.gz](evidence.tar.gz) (SHA-256 in [SHA256SUMS](SHA256SUMS))
contains every retained text artifact of the public, retry, recursive, churn and
diagnostic series and of the smokes and excluded pilots: case lists, raw
per-sample rows, stdout/stderr, resource intervals, preflights, summaries,
build and test logs, patches, dependency locks, the orchestration scripts, the
exact measured (A) and reviewed (B) source inputs, and
`execution-manifest.json`, which maps each stage to its binary, source and
controller. `fixture-hashes.json` and `binary-hashes.json` give the SHA-256 of
all 340 fixture files (8.75 GB) and the 12 executables, which are not
included; paired database copies are represented by their hash files.
`artifact-index.json` lists each file's original and published hash. Absolute
scratch and home paths were replaced with `<scratch>` and `<home>` in 36 text
files; nothing else was altered.

Every count and table in this report was regenerated from the extracted
archive with the committed harness (`evidence.py`, `analyze.py ratios`,
`analyze.py churn`), and each matches the summaries written during the runs.
The measured binaries were built from source A; the reviewed harness B, which
is the committed `scripts/shape-sweep/`, adds the partial-fixture guard and
controller memory caps and drove the retry, recursive, churn and diagnostic
stages without changing measured query code.

```sh
sha256sum -c SHA256SUMS
mkdir evidence && tar -xzf evidence.tar.gz -C evidence
python3 ../../../scripts/shape-sweep/evidence.py evidence/public-batch
python3 ../../../scripts/shape-sweep/evidence.py evidence/window-retry --samples 1
python3 ../../../scripts/shape-sweep/evidence.py evidence/recursive
python3 ../../../scripts/shape-sweep/analyze.py ratios evidence/recursive/samples.jsonl
python3 ../../../scripts/shape-sweep/analyze.py churn evidence/churn/generations.jsonl
```

## Finite matrix

| Family | Actual fixture sizes | Shapes | Cases |
| --- | --- | --- | ---: |
| Scalar order | 100k / 1m / 10m facts | Committed and pending; ascending/descending; top 10, half-offset plus 1,000 rows, full drain | 36 |
| Compound order | 99,999 / 999,999 / 9,999,999 facts | Committed, group descending plus score ascending with value tie suffix; offset 137 and limit 1,000 | 3 |
| Windows | 100k / 1m / 10m facts, two facts per row | One, 100 equal, singleton, and 90%-skewed partitions; running sum and row number; two incompatible-order cases per rung; two smaller pending controls | 32 |
| Sets | 10k entities, cardinality 0/1/4/8/16/32/64/256/1,024; selected 100k-entity cases | Repeated/unique/skewed integer sets; projection, membership, collection, and narrower pending controls | 81 |
| UUID projection | 10k / 100k output rows; 18 facts per output row | Dense/descending/stride-16/permuted/hot references; repeated working sets 1/16/64/1,024; selected cache and mixed-projection controls | 36 |
| Recursion | Chains with 10/100/1,000 edges; width-10 trees with 1,110/11,110 edges | Pending/committed, literal single-leaf calls and unbound calls filtered to 16/128/129 selected leaves | 22 |
| Pin/FactLog churn | 10k / 100k initial facts | No hold, one cursor, 32 held cursors, rolling 32, storage-order log, default/4,096-window transaction-order log, mixed cursor/log | 16 per session |

The public latency selection has 188 cases. Recursive runs compare each of four
controls with its own baseline, giving 176 case/variant combinations. Every
public/recursive case plans three fresh-process sessions and five warmed timings
per process. Churn plans three sessions, each with 32 baseline, 256 held, and
256 post-release checkpoint generations per case. Every checkpoint changes 100
entities through a retract/assert pair, retaining bi-temporal history.

A set projection/membership fixture stores two facts per entity. Collection
stores `N*(K+1)` facts; member count is not fact count for stored sets. UUID
fixtures allocate `16*N` target entities before adding `N` sources with two facts
each. Compound fixtures have no padding and are not exactly ten million facts.
The archived case and fixture manifests are authoritative.
Set membership projects six Boolean predicate results for every entity,
including an empty-subset probe and a negative-value miss. It does not filter
out nonmembers; its timings combine those predicates rather than isolating the
cost of any one membership operator.

## Measurement contract

All public fixtures are file-backed. Pending means committed transactions remain
uncheckpointed in the WAL and are replayed on open; it does not mean an open
uncommitted write transaction.

Public latency includes query parsing/open, full cursor drain, and streaming
answer fingerprints. Oracle construction and comparison are outside the timer. The clock stops after
iteration, before cursor destruction; database close and result serialization
are also outside it. The process deadline includes these phases.
Each process first verifies one complete warm-up, then verifies every timed
answer. A fresh handle is an application-cache condition, not an OS cold-cache
claim. Five samples share the handle; the three sessions reopen independently.
Report per-session medians and ranges; five-sample maxima are not p99 estimates.

The public native binaries use opt-level 3, full LTO, and one codegen unit.
The instrumented unit-test binary uses the root bench profile with opt-level 3
overridden; its observations and checkpoint timings are a separate series.
Toolchain, dependency locks, patches, executable hashes, and exact source inputs
must accompany the final evidence. Measurements use local ext4 and NUMA binding;
private host identifiers and absolute paths are omitted from published artifacts.

Process RSS high-water marks include fixture open/WAL replay, oracle setup,
warm-up, and previous samples in the same process. RSS excludes the operating
system's filesystem page cache. Query-meter peaks are separate
charged-allocation observations and do not include every allocator overhead.
Churn RSS additionally includes expected snapshot/log records and the observer's
page maps and sets. Do not interpret either RSS series as isolated operator
allocation, or subtract process high-water marks to estimate operator peaks.
Each query-memory diagnostic opens a fresh process/handle and drains one query
without the public runner's oracle construction or warm-up. Its process RSS is
a separate series, not interchangeable with public batch RSS. It checks the
public answer's row count and zero charged memory after cursor drop; full answer
fingerprints are checked by the public executable.

A public/recursive batch has a 600-second total deadline, including setup and
warm-up. Churn has a 3,600-second per-case deadline. A failed batch contributes
explicit failed sample slots, not successful partial timings; later attempts for
that case are deferred. Counts of failed slots are not counts of independent
failed process executions. A timeout lower bound describes the whole batch; it
is not a lower bound on any single query. Preflights use current CPU/swap deltas, not load
averages. Interval resource observations and all failed/partial pilots are kept.
The serial runner repeats idle preflights each session; paired and diagnostic
controllers preflight once per invocation. Child CPU/RSS sampling continues
throughout, but swap activity is checked only at preflight. No standalone
first-batch, close, prepared-slot or `execute` materialization timing is claimed.
RSS limits are sampled process guards at roughly 100 ms, not operating-system
allocation limits or finite query-memory settings. Brief peaks between samples
can escape the guard; successful processes also report their RSS high-water mark.

## Sorting and pagination results

All 39 scalar/compound cases have completed three sessions and five timed
samples per session: **585 verified timings**. Cells show latency in milliseconds:
the median of the three session medians, followed by their minimum–maximum.
These ranges are between-session medians, not percentiles or the range of all
15 individual timings. The raw summaries retain each session's sample min/max.
Scalar fact counts are exact; compound rungs contain 99,999 / 999,999 /
9,999,999 facts.

| Query | Storage | 100k rung | 1m rung | 10m rung | Largest-rung peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| asc top 10 | committed | 0.482 (0.460–0.518) | 0.497 (0.494–0.509) | 0.497 (0.495–0.502) | 4.4 |
| asc top 10 | pending | 64.467 (64.213–68.835) | 813.363 (810.359–817.038) | 9,116.940 (9,043.173–9,662.818) | 9,422.3 |
| asc half offset + 1,000 | committed | 11.867 (11.765–12.144) | 111.631 (110.700–112.975) | 1,154.149 (1,132.138–1,189.424) | 5.5 |
| asc half offset + 1,000 | pending | 87.376 (87.291–87.663) | 1,181.543 (1,176.498–1,196.894) | 13,180.303 (13,114.977–13,190.128) | 10,221.7 |
| asc full drain | committed | 29.790 (29.671–30.177) | 313.204 (288.289–321.004) | 3,053.098 (3,027.334–3,165.122) | 5.6 |
| asc full drain | pending | 111.875 (110.489–112.256) | 1,486.983 (1,473.366–1,492.049) | 16,551.040 (16,477.841–16,574.506) | 10,650.6 |
| desc top 10 | committed | 25.423 (25.045–25.514) | 247.033 (245.571–249.878) | 2,536.597 (2,510.585–2,571.340) | 5.5 |
| desc top 10 | pending | 77.458 (72.104–77.836) | 855.387 (854.723–865.604) | 9,782.713 (9,711.311–9,909.733) | 9,422.3 |
| desc half offset + 1,000 | committed | 62.810 (62.651–63.161) | 894.132 (893.055–899.522) | 13,259.347 (13,196.792–13,365.325) | 806.7 |
| desc half offset + 1,000 | pending | 100.383 (99.939–100.757) | 1,373.135 (1,365.025–1,377.206) | 15,842.066 (15,786.940–15,917.728) | 10,221.4 |
| desc full drain | committed | 80.503 (79.879–80.716) | 1,209.471 (1,195.163–1,213.383) | 16,410.723 (16,351.424–17,200.498) | 1,608.3 |
| desc full drain | pending | 110.498 (110.279–112.737) | 1,538.445 (1,529.545–1,547.614) | 17,812.650 (17,644.724–17,874.952) | 10,649.7 |
| compound page | committed | 910.931 (894.150–913.447) | 8,408.452 (8,294.515–8,609.616) | 85,821.400 (82,548.052–90,310.241) | 6.2 |

At ten million committed scalar facts, ascending top-10 stays near 0.5 ms while
descending top-10 takes about 2.54 seconds, despite both process peaks staying
below 6 MiB. Small output does not imply little input work. Descending half-offset
and full-drain cases reach about 807 MiB and 1,608 MiB respectively. The final
sort's retained row count depends on offset plus limit; upstream work and value
sizes are separate costs.

The pending scalar processes reach roughly 9.2–10.4 GiB of RSS, including WAL
replay and setup. Their ascending top-10 median is about 9.12 seconds at ten
million facts. These are whole-process observations, not isolated sort allocation
or finite-query-cap acceptance. The compound page keeps a small process peak
but takes about 85.8 seconds at its largest rung: bounded output and retention
do not bound the cost of its multi-attribute query.

## Window results: original batches

The original 32-case window matrix reconciles to **420 verified timings**,
20 timeout slots from four capped batches, and 40 later deferred slots. Successful
cases have three sessions of five warmed timings. Units and parentheses follow
the sort table above. Each fixture entity has two input facts and yields one
output row; ten million facts therefore means five million output rows. “Incompatible” projects two row-number windows
with swapped partition/order keys.

| Partition / operation | Storage | 100k facts | 1m facts | 10m facts | 10m peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| one / sum | committed | 742.888 (713.326–745.303) | 7,134.275 (7,102.405–7,223.454) | batch capped | — |
| one / row-number | committed | 714.576 (712.411–729.510) | 6,904.038 (6,896.118–6,914.105) | batch capped | — |
| hundred / sum | committed | 886.724 (865.513–890.444) | 8,736.607 (8,577.375–8,791.286) | 86,548.499 (86,104.921–90,459.078) | 22.2 |
| hundred / row-number | committed | 882.609 (872.779–882.657) | 8,745.400 (8,626.647–8,841.631) | 87,118.713 (85,999.241–87,827.479) | 22.0 |
| singleton / sum | committed | 712.747 (699.642–714.434) | 6,395.284 (6,359.071–6,438.026) | 65,153.878 (64,065.086–66,237.685) | 6.0 |
| singleton / row-number | committed | 706.564 (702.490–715.961) | 6,598.038 (6,406.797–6,975.002) | 64,855.187 (63,785.432–65,750.265) | 6.0 |
| skew / sum | committed | 718.680 (714.023–738.538) | 7,274.886 (7,047.466–7,291.361) | 97,895.352 (96,590.009–98,230.460) | 1,373.5 |
| skew / row-number | committed | 745.435 (713.625–745.881) | 7,101.030 (6,998.368–7,284.709) | 98,120.831 (96,561.578–99,706.302) | 1,373.8 |
| hundred / incompatible | committed | 727.823 (704.363–733.111) | 7,261.953 (7,174.245–7,339.789) | batch capped | — |
| skew / incompatible | committed | 735.029 (727.215–752.347) | 7,284.655 (7,240.701–7,285.330) | batch capped | — |
| skew / sum | pending | 117.235 (116.958–123.269) | 1,466.418 (1,458.765–1,468.906) | not selected | — |

At the largest rung, singleton windows finish in about 65 seconds with roughly
6 MiB process peaks; 100-partition windows take about 87 seconds with roughly
22 MiB peaks. The 90%-skewed windows take about 98 seconds and reach roughly
1.34 GiB. Equal fact counts do not give equal retention or latency. These are
process RSS observations; the separate diagnostic pass reports charged query
memory.

The one-partition sum/row-number and both incompatible-order batches reached
the 600-second whole-batch deadline at ten million facts. This does not establish
a 600-second lower bound for an individual query. Ten-million-fact pending
windows were not selected.

## Window results: capped-batch retry

The four capped cases were rerun as a separate series: three fresh-process
sessions of **one** timed sample each, after one verified warm-up per process,
under the same 600-second whole-batch deadline. All **12 timings verified**;
no slot timed out. With one sample per session, each figure is the median of
three single timings, with their range in parentheses. These are not
comparable to the five-sample cells above as a matched series.

| Partition / operation | Storage | 10m facts (ms) | Peak RSS (MiB) |
| --- | --- | ---: | ---: |
| one / sum | committed | 101,270.871 (98,432.190–101,577.354) | 1,449.5 |
| one / row-number | committed | 100,397.765 (98,883.779–100,884.962) | 1,449.1 |
| hundred / incompatible | committed | 101,867.209 (101,056.968–102,983.560) | 2,059.3 |
| skew / incompatible | committed | 103,807.097 (103,731.578–103,822.677) | 2,001.3 |

At roughly 100 seconds a query, a warm-up plus five timed samples exceeds the
original 600-second batch deadline, which accounts for the capped batches. The
one-partition cases retain about 1.4 GiB and the incompatible-order cases about
2 GiB of process peak, against 6–22 MiB for singleton and 100-partition windows
at the same rung.

## Set and UUID results

The set matrix completed **1,155 verified timings** across 77 cases; four
construction rejections account for 60 deferred slots. UUID projection completed
**540 verified timings** across all 36 cases. The [complete tables](SET-UUID-RESULTS.md)
retain every selected cardinality, distribution, storage/cache control, and
operation, using the same session-median convention as the tables above.

For 10,000 committed entities, repeated-set membership rises from 144 ms at
cardinality zero to 168 ms at 32, 1,113 ms at 256, and 5,401 ms at 1,024.
The corresponding successful process peaks are about 6, 8, 22, and 73 MiB.
At 100,000 entities, the cardinality-256 case takes about 11.1 seconds while
its process peak remains about 22 MiB. These queries project six Boolean results
per entity; they do not isolate individual predicate costs. Pending cardinality-32
controls take about 97–99 ms for 10,000 entities, with about 77 MiB process peaks
that include WAL replay and setup. Unique/skewed cardinality-1,024 inputs hit the
construction boundary described below.

For 100,000 UUID output rows with a 256-page storage cache, the reference-only
distribution medians span about 1.28–1.42 seconds, with process peaks near 6 MiB.
These queries project each reference twice; mixed projection adds the source
UUID. Disabling the storage page cache gives about 2.23 seconds for dense
references and 2.69 seconds for stride-16 references. Raising it to 4,096 pages
gives about 1.27 and 1.32 seconds, with process peaks of 18 and 22 MiB.
These are sequential end-to-end cases, not matched ablations of UUID translation
caching. Attribution to internal cache/hint paths requires the separate diagnostic
counters; storage page-cache capacity is a different control.

## Fixture storage and open costs

These selected file lengths come from the completed public samples. They are
logical file sizes, not allocated filesystem blocks or transient construction
peaks. The raw records retain `.graph`, WAL, and their sum separately for every
successful sample. The values below are MiB.

| Fixture | Input facts | `.graph` | WAL |
| --- | ---: | ---: | ---: |
| Scalar, committed | 10,000,000 | 1,098.555 | 0 |
| Scalar, pending | 10,000,000 | 0.008 | 571.266 |
| Compound page | 9,999,999 | 655.371 | 0 |
| Window, 100 partitions | 10,000,000 | 758.691 | 0 |
| Window, singleton partitions | 10,000,000 | 788.293 | 0 |
| Window, skewed partitions | 10,000,000 | 728.934 | 0 |
| Stored repeated sets, N=10k / K=256 | 20,000 | 2.715 | 0 |
| Collected repeated sets, N=10k / K=256 | 2,570,000 | 111.059 | 0 |
| Stored repeated sets, N=100k / K=256 | 200,000 | 27.055 | 0 |
| Dense UUID references, N=100k | 1,800,000 | 154.852 | 0 |

Database open is outside the query timer. For the ten-million-fact scalar
ascending top-10 case, pending open has a three-session median of 52.94 seconds
(52.91–53.02 seconds), before warm-up or the five timed queries. Committed open
for that case is about 0.12 ms with the warm OS cache. These are separate observed
open costs, not additions to each warmed query or cold-start guarantees.

## Recursive control results

The recursive matrix pairs each of the 22 cases with four controls: three
sessions of five samples for the baseline and for the control, **2,640 planned
slots**. Strict reconciliation gives **2,355 verified timings**, 80 error slots,
15 timeout slots and 190 deferred slots. Every non-measured slot has one of two
causes:

- Both 1,000-node chain cases (committed and pending, every control and every
  baseline) fail with `INT-020`, the default limit of 1,000 semi-naive rounds.
  The `k=1` query names a literal leaf, so magic-set rewriting adds a demand
  relation that shares the rules' stratum: demand walks down the chain one
  round per edge before answers walk back, about two rounds per edge. A local
  check on the measured revision answers a 500-edge chain and rejects a
  501-edge chain with `INT-020`. This is a configured limit boundary,
  identical in every binary. No timing is reported.
- The attribute-scan control of committed 11,110-node trees with `k=16`, `128`
  and `129` reached the 600-second batch deadline in its first session; later
  sessions were deferred. Their baselines were measured. No ratio is reported,
  and the deadline does not give a per-query bound.

Each cell is the pooled median ratio of control to baseline over 15 matched
pairs. Parentheses give the range of the per-placement medians (baseline on
node 1/control on node 3, and the reverse). The baseline column is the median,
over the four pairings, of each pairing's median of session medians, in
milliseconds. Each pair's baseline and control processes run at the same time,
one per socket, on separate copies of the fixture. A ratio above 1 means the
control is slower.

| Shape n | k | Storage | Baseline ms | no-cache | no-ancestor-alternative | attribute-scan | static-connected-order |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| chain 10 | k=1 | committed | 0.745 | 0.952 (0.944–0.956) | 1.002 (0.971–1.034) | 1.137 (1.135–1.158) | 0.988 (0.981–1.000) |
| chain 10 | k=1 | pending | 0.490 | 0.989 (0.967–1.025) | 0.991 (0.988–0.991) | 1.006 (1.003–1.023) | 0.995 (0.969–0.997) |
| chain 100 | k=1 | committed | 48.812 | 0.980 (0.975–1.019) | 0.996 (0.991–0.998) | 2.882 (2.874–2.895) | 0.994 (0.990–1.001) |
| chain 100 | k=1 | pending | 11.378 | 0.940 (0.937–0.957) | 0.991 (0.991–0.993) | 0.986 (0.965–1.004) | 1.003 (1.001–1.003) |
| chain 1,000 | k=1 | committed | `INT-020` | — | — | — | — |
| chain 1,000 | k=1 | pending | `INT-020` | — | — | — | — |
| tree 1,110 | k=1 | committed | 0.724 | 0.996 (0.962–1.003) | 1.010 (1.008–1.023) | 4.039 (4.032–4.092) | 0.983 (0.983–0.997) |
| tree 1,110 | k=16 | committed | 54.969 | 1.024 (1.024–1.028) | 0.986 (0.979–0.988) | 16.483 (15.898–16.523) | 0.993 (0.988–1.013) |
| tree 1,110 | k=128 | committed | 59.275 | 1.019 (1.015–1.019) | 1.015 (0.997–1.016) | 15.390 (15.077–15.522) | 1.005 (1.001–1.052) |
| tree 1,110 | k=129 | committed | 60.004 | 1.035 (1.029–1.042) | 1.005 (1.003–1.041) | 15.516 (15.437–16.032) | 0.987 (0.982–1.011) |
| tree 1,110 | k=1 | pending | 0.205 | 0.980 (0.952–0.987) | 5.238 (5.206–5.315) | 1.000 (1.000–1.008) | 0.956 (0.908–1.014) |
| tree 1,110 | k=16 | pending | 5.656 | 1.050 (1.050–1.052) | 0.965 (0.964–0.970) | 1.011 (0.999–1.023) | 0.993 (0.976–1.037) |
| tree 1,110 | k=128 | pending | 5.732 | 1.049 (1.049–1.063) | 1.010 (1.002–1.023) | 1.006 (1.001–1.015) | 1.002 (0.995–1.006) |
| tree 1,110 | k=129 | pending | 5.712 | 1.073 (1.019–1.098) | 0.992 (0.969–1.023) | 1.003 (1.002–1.003) | 0.990 (0.986–0.993) |
| tree 11,110 | k=1 | committed | 1.044 | 1.019 (0.998–1.071) | 1.006 (0.960–1.017) | 29.926 (29.092–30.397) | 0.980 (0.967–1.012) |
| tree 11,110 | k=16 | committed | 783.370 | 1.032 (0.999–1.042) | 0.998 (0.982–1.038) | batch capped | 0.962 (0.952–1.001) |
| tree 11,110 | k=128 | committed | 804.596 | 0.970 (0.957–1.000) | 0.998 (0.988–1.007) | batch capped | 1.000 (0.996–1.001) |
| tree 11,110 | k=129 | committed | 800.810 | 0.992 (0.990–1.007) | 0.996 (0.993–1.038) | batch capped | 0.990 (0.938–1.333) |
| tree 11,110 | k=1 | pending | 0.179 | 0.976 (0.953–1.011) | 34.319 (34.016–34.319) | 0.998 (0.998–1.004) | 0.987 (0.983–1.065) |
| tree 11,110 | k=16 | pending | 62.949 | 1.044 (1.032–1.064) | 1.009 (0.998–1.022) | 1.000 (0.990–1.002) | 1.000 (1.000–1.007) |
| tree 11,110 | k=128 | pending | 63.304 | 1.059 (1.040–1.063) | 0.990 (0.976–0.996) | 1.007 (1.001–1.008) | 1.001 (1.000–1.006) |
| tree 11,110 | k=129 | pending | 63.189 | 1.052 (1.050–1.055) | 1.009 (0.999–1.009) | 0.997 (0.995–0.998) | 1.005 (1.002–1.009) |

Two end-to-end differences hold across both placements. Replacing bound-value
AVET probes with attribute scans slows every committed case with a ratio by
2.9× to 30×, except the 10-node chain at 1.14× (1.135–1.158); the three capped
committed cases have no ratio, and pending cases are unchanged. Disabling the
small ancestor alternative slows the pending `k=1` trees by 5.2× (1,110 nodes)
and 34× (11,110 nodes) and changes no other case beyond the range of
placement noise. Both patterns match the stated eligibility of those paths
(committed AVET probes; an empty committed dictionary for the ancestor
alternative); the baseline path counters below show those paths in use in
exactly the affected cases. The entity-probe cache and static connected-pattern
controls keep pooled medians within about ±7%; per-placement medians reach
1.098 (no-cache) and one 1.333 outlier (static order). Several pending `no-cache` ratios sit
at 1.04–1.07 in both placements, a small consistent slowdown, not a
qualified effect. Individual ratios are not additive.

## Sustained reader churn results

Each of 16 schedules (two fixture sizes × eight reader mixes) ran in three
fresh-process sessions: **48 cases**, each of 32 baseline, 256 held-reader and
256 post-release checkpoint generations, with 100 entities retracted and
reasserted before each checkpoint. All 48 cases are measured and pass the strict
summary: every generation verified, generations advancing, quarantine empty
from the first post-release generation, released pages reused, integrity and
reopen checked. No failed or missing case.

Checkpoint figures are the median over sessions of each phase's median
checkpoint time. File sizes are the `.graph` length after the last checkpoint
of each phase (the median of three sessions; they agree in every case except
where a range is shown). Quarantine, released and reused pages are identical in
all three sessions of every schedule. Peak RSS includes the harness's
independent update model, FactLog oracle exports and page maps; the
`none` schedule has the same peak, so these figures do not measure reader cost.

| Facts | Readers | Checkpoint ms baseline / held / after release | File MiB end baseline → end held → end | Peak quarantine pages | Released / reused pages | Peak RSS MiB |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 | none | 1.276 / 1.461 / 1.713 | 1.4 → 4.2 → 7.5 | 0 | 0 / 0 | 22.9 |
| 10,000 | cursor | 1.303 / 1.591 / 1.643 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 22.8 (22.8–23.0) |
| 10,000 | many | 1.306 / 1.578 / 1.546 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 22.9 (22.2–23.0) |
| 10,000 | rolling | 1.328 / 1.459 / 1.739 | 1.4 → 6.4 → 7.5 | 591 | 586 / 586 | 22.9 (22.9–23.0) |
| 10,000 | log-storage | 1.294 / 1.611 / 1.640 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 23.0 (22.9–23.0) |
| 10,000 | log-default | 1.288 / 1.632 / 1.656 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 23.0 (22.9–23.1) |
| 10,000 | log-4096 | 1.280 / 1.556 / 1.659 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 22.9 (22.8–22.9) |
| 10,000 | mixed | 1.281 / 1.590 / 1.630 | 1.4 → 18.5 → 18.5 | 3,661 | 3,661 / 858 | 22.9 |
| 100,000 | none | 1.557 / 1.552 / 1.540 | 9.7 → 13.4 → 17.0 | 0 | 0 / 0 | 125.8 |
| 100,000 | cursor | 1.530 / 1.679 / 1.441 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.7 (125.6–125.9) |
| 100,000 | many | 1.515 / 1.714 / 1.437 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.7 (125.0–125.7) |
| 100,000 | rolling | 1.495 / 1.509 / 1.503 | 9.7 → 15.2 → 17.0 | 511 | 492 / 492 | 125.8 (125.4–125.8) |
| 100,000 | log-storage | 1.509 / 1.664 / 1.419 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.7 (125.0–126.4) |
| 100,000 | log-default | 1.503 / 1.667 / 1.421 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.8 (125.8–125.9) |
| 100,000 | log-4096 | 1.514 / 1.663 / 1.428 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.8 (125.7–125.9) |
| 100,000 | mixed | 1.484 / 1.657 / 1.418 | 9.7 → 28.9 → 28.9 | 3,978 | 3,978 / 951 | 125.8 (125.7–125.9) |

A reader held across 256 checkpoints keeps every page those checkpoints free:
the file grows to 18.5 MiB (10k facts) and 28.9 MiB (100k facts), against
4.2 and 13.4 MiB with no reader. One held cursor, many cursors, held FactLogs
of either order or window, and a mix all quarantine the same pages, because the
oldest pin decides what is kept. A rolling reader that is replaced keeps
quarantine near 500–600 pages. After release, the held schedules stop growing:
the 256 post-release checkpoints reuse 858 (10k) and 951 (100k) released pages
while the file length stays put. The no-reader file keeps growing with history
in the same phase (to 7.5 and 17.0 MiB), and no schedule's file shrinks. Held
readers change a median checkpoint of 1.3–1.7 ms by −0.04 to +0.17 ms against
the no-reader schedule in the same phase; quarantine bookkeeping does not
dominate checkpoint time at these sizes.

## Charged query memory (diagnostic series)

A separate optimized diagnostic binary reran each successful original public
case once in a fresh process: one drain, no oracle, no warm-up. It reports the
query's `Meter` peak and its window and `IdSpace` components, checks the row
count and that the meter returns to zero after the cursor drops, and reports
fresh-process RSS. All **180** successful public cases were observed; every one
returned the expected row count and a zero meter after drop. This series is
not comparable to the public RSS high-water marks, and the meter is an estimate
of charged state, not of every allocation.

Ten-million-fact scalar order (MiB; “other” is meter peak minus the window and
`IdSpace` components, here the `Sort` state):

| Order / page | Storage | Meter peak | `IdSpace` | Other | Fresh RSS |
| --- | --- | ---: | ---: | ---: | ---: |
| asc top 10 / half offset / full | committed | 0.0 / 0.0 / 0.0 | 0.0 | 0.0 | 7.7–8.8 |
| desc top 10 | committed | 0.0 | 0.0 | 0.0 | 8.8 |
| desc half offset + 1,000 | committed | 267.1 | 0.0 | 267.1 | 809.4 |
| desc full drain | committed | 534.1 | 0.0 | 534.1 | 1,609.6 |
| asc or desc top 10 | pending | 762.9 | 762.9 | 0.0 | 10,423.4–10,424.1 |
| asc or desc half offset + 1,000 | pending | 1,030.0 | 762.9 | 267.1 | 10,538.1–10,538.4 |
| asc or desc full drain | pending | 1,297.0 | 762.9 | 534.1 | 10,591.3–10,592.2 |

Committed ascending order charges nothing: the input order already satisfies
the sort (#641) and rows stream. Committed descending order keeps a `Sort` heap
of `offset + limit` rows, so a half offset charges half of what a full drain
does, and top 10 charges almost nothing. Pending fixtures add 763 MiB of
provisional ids for ten million uncheckpointed entities before any sort state;
their roughly 10 GiB fresh RSS is dominated by the pending store itself, which
the meter does not charge.

Five-million-row windows (ten million facts) charge 0.1 MiB for singleton
partitions, 9.7 MiB for 100 partitions and 858.5 MiB for the 90%-skewed
partition, almost all of it window state: partition-at-a-time execution holds
one partition, so the largest partition decides the charge. The one-partition
and incompatible-order cases at this rung come from the retry series below.
The 74 committed set cases charge 80–92 KiB, mostly `IdSpace`; the three
pending membership controls charge 781 KiB. UUID cases charge 80–160 KiB and
compound cases 0.2 MiB.

The four retried ten-million-fact windows were observed the same way (4 of 4):

| Partition / operation | Meter peak | Window | `IdSpace` | Fresh RSS |
| --- | ---: | ---: | ---: | ---: |
| one / sum | 953.7 | 953.7 | 0.1 | 1,419.9 |
| one / row-number | 953.7 | 953.7 | 0.1 | 1,419.3 |
| hundred / incompatible | 1,487.7 | 1,106.3 | 381.5 | 1,971.8 |
| skew / incompatible | 1,487.7 | 1,106.3 | 381.5 | 1,971.5 |

A single partition holds all five million rows. The incompatible-order cases
need two orders of the whole input, so they fall back to full-input window
mode and are the largest charge in the sweep. Component peaks are separate
high-water marks and need not sum to the meter peak.

### Recursive path counters

The 20 successful recursive cases were observed with the **baseline** binary
and its `cfg(test)` path counters (the two `INT-020` chain cases have no
observation). The counters show which baseline paths each case exercises; the
controls were not instrumented, so a control's changed path follows from its
construction, not from a measured counter.

| Path counter | Cases where it is nonzero | Matching ratio result |
| --- | --- | --- |
| `bound_value_avet_range` | every committed case (6 to 43,210 ranges); no pending case | attribute-scan slows committed cases 1.14× (10-node chain) to 30×; pending cases unchanged |
| `ancestor_alternative_selected` | every pending `k=1` case (1–12 selections); no other case | disabling it slows pending `k=1` trees 5.2× and 34×; the pending chains, with 1 and 12 selections, are unchanged |
| `entity_probe_cache_hit` | every `k ≥ 16` tree (999 or 9,999 hits); no `k=1` case | no-cache within 0.97–1.07 of baseline in those cases |
| `connected_delta_step` | every case (1 or 6 steps) | static order: pooled medians 0.956–1.005, placement medians 0.908–1.065 apart from one 1.333 outlier |

So the two large effects coincide with the paths they disable, and the
counters exclude both effects outside those cases. The ancestor counts
confirm that the alternative is chosen only where the committed dictionary is
empty. Recursive charged memory is at most 10.7 MiB (pending 11,110-node
trees, `k ≥ 16`); `k=1` cases charge 4–15 KiB, except the 100-edge chains at
0.84–0.86 MiB. The counters do not
show how much of each case's time a path accounts for.

## Attribution boundaries

The recursive controls disable the entity-probe cache, disable the small ancestor
alternative, replace bound-value AVET probes with attribute scans, or replace
adaptive connected-pattern scoring with static priority. They share a core
revision and independent answer oracle. Each pair uses byte-identical separate
database/WAL copies; placements swap between separate sockets each session.
Ratio summaries retain each session and each baseline/control placement as well
as pooled samples; three sessions give two observations of one placement and one
of the reverse. A pooled median alone cannot establish a placement-independent
effect.

Path counters must demonstrate use before a result is attributed to a named
optimization. A noninitial connected-pattern choice is not proof that the static
control produced a different plan. That control retains connectivity and initial
seed priority, so it does not isolate the entire historical delta-ordering change.

Only literal rule arguments seed magic-set demand in this engine. The `k > 1`
cases compute unrestricted recursion and then select leaves. In particular,
`k=128/129` does not exercise the ancestor alternative's source-cardinality
cutoff; that threshold remains unmeasured. Controls that do not demonstrably
change an exercised path are end-to-end comparisons only.

## Observed input boundary

The 1,024-member unique and skewed set fixtures were rejected during construction
with `WAL-003`: the set element-encoding bounds reached 4,096 and 5,120 bytes,
respectively, above the file-backed value limit of 4,068 bytes. Their projection
and membership cases were not timed. The repeated-small-integer fixture fits.
This is a byte-size boundary, not a universal maximum set cardinality: integer
encoding width changes with member values. The original rejections remain in
the evidence; the input was not changed to make those cases pass.

The 1,000-edge chain recursion is rejected with `INT-020` by the default limit of
1,000 semi-naive rounds (see the recursive results). With a literal argument,
magic-set demand and answers share a stratum and take about two rounds per
edge, so the default answers a chain of at most 500 edges; a local check
confirmed 500 passes and 501 fails. The frozen binaries use the default limits;
the cases were not rerun with a raised limit.

## Remaining scope

These native measurements do not establish X02's finite-cap/transaction/release
acceptance, X03's browser migration envelope, or X04's foreign-host lifecycle
coverage. They are not universal memory or latency bounds.

The finite selection excludes limited windows, ten-million-fact pending windows,
the older mixed `perf_envelope` example, wider compound paging combinations,
non-integer set members, broader membership probes, explicit UUID threshold
boundaries, and the full Cartesian product of cache/storage/distribution choices.
Pending fixture transaction timestamps are wall-clock values: hashes identify
actual files, while the logical recipe alone does not reproduce identical bytes.
Churn likewise uses fixed initial records followed by wall-clock retract/assert
transactions; its schedules share a logical update sequence, not identical
timestamp bytes.

Churn checks every previously quarantined page's generation across each checkpoint,
complete old/fresh answers, final release, later page reuse, integrity, and reopen.
Cursor values are checked against an independent update model; held FactLogs
are compared with complete pre-pin exports, which checks snapshot stability
rather than independently proving the initial log reader complete. It does not
trace every backend write or prove protection of every live page.
Held readers are idle until final drain, apart from rolling-reader evictions.
Per-FactLog buffer allocations and final log-drain latency are not isolated;
default/4,096-window comparisons concern the instrumented process and its
checkpoint/pin observations.
History growth continues after release, and file length need not shrink when
quarantined pages become reusable.

## Nightly candidates (proposals only)

No sweep case is added to nightly CI by this change. The following small
representatives each exercise one behavior this sweep found, at a cost that
looks compatible with a nightly route. Times are this host's warmed medians;
CI hardware, fixture construction, warm-up and process start-up are unmeasured,
so none of these is a qualified route.

| Candidate | Behavior it guards | Measured here |
| --- | --- | ---: |
| 100k facts, 90%-skewed running sum, committed | Partition-at-a-time window retention follows the largest partition | 718.680 ms |
| 10k entities, `K=256` skewed set membership, committed | Set membership over large value-page sets | 1,124.625 ms |
| 100k facts, descending half offset + 1,000, committed | `Sort` heap of `offset + limit` rows; ascending input-order omission as its pair | 62.810 ms |
| 1,110-node tree ancestors, `k=1`, pending | Small ancestor alternative (5.2× without it) | 0.205 ms |
| 1,110-node tree ancestors, `k=16`, committed | Bound-value AVET probes (16× with attribute scans) | 54.969 ms |
| 10k facts, one held cursor across the churn schedule | Quarantine of freed pages, release and reuse | 0.86–0.94 s of checkpoints per run |

A route would pin answers and path counters as well as time, so that a
regression in the path is distinguished from a slower host.
