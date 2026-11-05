# v3 nightly benchmark coverage close-out (#776)

Follow-up on 2026-11-05: [#840's completed verification](benchmarks/840-regression/completion.md)
establishes numeric comparisons for all 427 native and 26 binding IDs with at
least three independent v3 runs each. Merged fixes #853/#854 preserve the v3 head
after a measured main advance. Actual scheduled routes, isolated live notification
delivery and cleanup are verified; complete evidence is archived beyond 30 days.
JVM/WASI and qualified iOS-simulator/Android-emulator expansion are tracked in
#858. The original validation snapshot below is retained unchanged; its null
baselines and X05 open status describe that earlier snapshot.

Audit date: 2026-11-03. P01–P17 are merged. This closes out the implementation
plan and gives every matrix row a disposition; it does **not** close
[issue #776](https://github.com/project-minigraf/minigraf/issues/776) or certify
every release acceptance condition. Remaining manual checks, exclusions and
unverified reporting/scaling claims are listed below. This documentation-only
update follows PHILOSOPHY.md's reliability, performance through simplicity and
documentation principles; it changes no runtime, dependency, API or file format.

**X02 follow-up (2026-11-04):** [the acceptance enforcement report](benchmarks/837-acceptance-gaps.md)
records a successful revised-workflow dispatch: ten named tests, nine confirmed
RSS checks, nonlinear/multistratum scaling, custom chunk/clamp bounds and exact
storage bytes. It supersedes the missing-execution and missing-coverage status
for X02 below; those rows retain the historical audit context. Activation on the
default-branch schedule requires v3 PR #848 followed by main PR #849. This adds
no Bencher memory metric and does not resolve the other manual exclusions.

## Revisions and method

| Input | Revision |
| --- | --- |
| Supported v2 baseline | Annotated tag `v2.0.11`, peeled commit `709be06c391086ced67e3b0da4ab677de745ac25` |
| Audited and measured v3 | `0394c90ad90e9b95d416f4a0ec4a4c78b16e5b57` |
| Default-branch workflow/reporting revision, and inspected `origin/main` | `4d65022ac963a10b582ac762c4ab2e73935c9115` |
| `merge-base v2.0.11 origin/v3` | `75ffc9b11f58cee3494430677465d553e582847b` |
| Original feature inventory revision | `4685ca9cb7638353087410c31ad07e3f6261ddb0` |

The feature inventory below retains the original v2 endpoint comparison of
[CHANGELOG.md](../CHANGELOG.md), [ROADMAP.md](../ROADMAP.md), public APIs,
features and binding additions. `git diff` from the original inventory revision
to the audited v3 revision is empty for `src/`, `Cargo.toml`, `ROADMAP.md` and
`CHANGELOG.md`: the follow-ups add benchmark/workflow coverage, not production
features. Cherry-picked v2 backports are compared by endpoint behavior rather
than commit identity. The v3 Cargo package's old version is not the baseline.
Refresh this inventory again for production changes before the v3.0.0 cut.

The inspected main/v3 copies of `bench.yml`, `smoke.yml` and `.github/bench/`
are byte-identical. Main owns schedules; the measured checkout is separately
pinned. **V** below is the actual v3 validation dispatch
[41642356471](https://github.com/project-minigraf/minigraf/actions/runs/41642356471),
using those exact source/workflow revisions. All **31 jobs** succeeded:
compile, **26 native suites** and **four binding jobs**.

Verification used the downloaded `bench-enumeration`, all `bench-evidence-*`
and all `binding-evidence-*` artifacts, plus job metadata and the supplied
concatenated logs/ID list. Native enumeration contains **583 registered IDs**:
**427 selected, analyzed and uploaded**, and **156 deliberately unselected**.
The selected sets are nonempty and disjoint. Each raw Criterion `Analyzing`
set was independently compared with selection JSON, final report JSON and raw
Bencher result URLs. All sets match, all 26 native reports have zero errors and
zero alerts, and their union matches the supplied 427-ID list.

The query job's API-downloaded console log is truncated. Its check-run output
has null summary/text, so it does not supply the counts. The retained
`bench-evidence-2` artifact supplies the complete raw query log, upload output
and report: **146 selected; 146 analyzed; 146 uploaded; alerts 0**. The other
25 native suites total **281**. The four binding reports separately verify
**26 foreign-runtime IDs**, all executed/uploaded with zero errors/alerts;
these are not Criterion IDs and are not included in 427.

**B** means the public Bencher `/perf` API was checked for every result URL:
all 427 native and 26 binding IDs have a metric with version hash equal to the
audited v3 SHA, on branch `v3`, with t-test upper boundary **0.999** and maximum
sample size **30**. This establishes stored observations and threshold models,
not mature regression baselines. At inspection, numeric comparison baselines
exist for **183 native IDs**; they are null for **244 native IDs and all 26
binding IDs**. Those comparisons are not enforceable yet; baseline accumulation
and live alert delivery remain follow-ups. Returned older metrics can include
inherited main history and must not be counted as independent v3 runs.

No new build, enumeration or timed workload was run for this doc-only update;
all execution evidence comes from Actions artifacts. `simd_helpers` is an
automatically discovered helper target without Criterion registrations;
`minigraf_bench` is the explicit `harness = false` target. Native jobs use
default features (no test-hooks); browser binding builds use the browser target.

## Feature inventory: additions and changed v3 paths

The inventory groups related commits into independently auditable costs. A
pre-existing language feature is included when its engine/storage implementation
changed in v3; the language feature itself is not called new. Matrix references
below give the actual coverage disposition.

| Feature or changed path | Verified source / difference over v2.0.11 | Matrix rows |
| --- | --- | --- |
| v8 covering indexes and same-transaction multivalues (#287/#371/#433) | `storage/keys.rs`, `node.rs`, `btree.rs`, `dict.rs`, `reader.rs`: value/op in keys, sequential DICT IDs, prefix compression, shortest separators; fact pages removed | C01–C04, C07 |
| Atomic COW checkpoint, verified pages and WAL v2 (#374/#388/#434) | `meta.rs`, `page.rs`, `freelist.rs`, `persistent_facts.rs`, `wal.rs`: alternating meta pages, page CRC/id/generation, lazy free-list reuse; failed checkpoints cannot be retried on the same handle (#390) | C03, C05, C06, C37 |
| Deduplicated long values and bounds (#433) | `value_pages.rs`, `keys.rs`: long strings over 64 bytes; file strings up to 4,068 bytes, idents up to 1,024 | C04 |
| v7 migration, v1–v6 rejection; damaged-page errors, no query fallback | `persistent_facts.rs`, `reader.rs`, `btree.rs`; STG-028–037 and migration validation, including #496/#497/#535 | C07, C08 |
| Integrity and repair APIs (#373) | New `IntegrityReport { facts, pages, problems }`, `is_ok`, `Minigraf::verify` / `rebuild_indexes` | C08 |
| Net-assert on committed keys (#379/#435) | Skip superseded versions before decoding, historical index seeks; latest assertion replaces a triple's valid window | C09 |
| Write validation and statement order (#436/#477) | API-019 valid windows; last write statement wins; LogWriter's API-020 rules | C10, C30, C32 |
| Read-only open (#429) | New `OpenOptions::read_only` field/builder, shared lock, in-memory WAL replay and nonmigrating v7 reads | C11 |
| Cursor API (#432) | New `Cursor`/`Batch` exports and `Minigraf::query` / `PreparedQuery::query`; vars, batches, close, row iteration | C12, C13 |
| Snapshot reads and generation pins (#543/#550) | Immutable reader/pending cutoff, page quarantine, checkpoint publication, staged transaction snapshots | C13, C30 |
| Fact-log API (#430) | New `FactRecord`, `FactFilter`, `FactLog`, `FactOrder`; tx/storage order, filters, bounded window, batch/close | C31 |
| LogWriter (#431) | Native export; create/append/advance_tx_count/finish, retained timestamps and holes, partial-file rename | C32 |
| Streaming engine for every query (#473/#544–551) | `query/engine/`: scans, filter/bind, entity/value nested-loop and hash joins, pseudo metadata, anti/union/distinct, aggregates/windows, recursive relations; executor path removed | C01, C14–C19 |
| Plan-independent answers (#545/#594/#596/#606) | Join equality/first textual binding, lexical per-fact pseudo source (PRS-081), expression unification; all builds now reorder | C14–C16, C35 |
| Exact aggregation and deterministic windows (#547/#777/#789) | Shared exact accumulators/value order, tie/peer rules; ordinary avg added; sum skips nonnumbers and extrema accept all non-nil types | C18, C20 |
| Snapshot-bound input-order proofs (#641/#781) | Sort omission; ordered count/count-distinct/sum/avg/scalar min/max/Integer sum-distinct; effective grouping tuples; partition streaming, DICT cache eviction | C21–C23 |
| Query memory accounting (#549/#761/#764/#765) | New `OpenOptions::query_memory_limit`, QRY-012; operator, IdSpace, set, exact-sum, window result/scratch charges | C24 |
| Large/streaming writes (#550) | Chunked parser; new `transaction_chunk_facts` field/builder (default 65,536, clamp 1..=1,000,000); working trees, statement rollback and failed-handle handling | C30 |
| Pagination and sorting (#306) | `:limit`, `:offset`, `:order-by`, heap top-k, complete tie order, prepared limit/offset slots | C21, C25 |
| Optional final tuple distinct (#493) | `DistinctFind` after aggregates/windows, before sorting/pagination, canonical zeros/NaNs including set elements | C26 |
| Window neighbors (#182) and physical row frames (#183) | `lag`/`lead`, bounded `:rows N preceding`; incremental built-ins and replayed registered aggregates | C27, C28 |
| Sets (#316/#794) | New non-exhaustive `Value::Set(ValueSet)` and scalar-only `ValueSet`; EDN, serde, inline/long storage, WAL, comparison; set encoding uses unstable sort | C29 |
| Set predicates and collect-set (#316) | `in?`, `not-in?`, `any-in?`, `all-in?`, `none-in?`; per-entity temporal collection/probes and memory charging | C29 |
| Fallible/re-entrant UDFs (#180) | New `UdfError` export and `try_register_predicate` / `try_register_aggregate`; QRY-014, immutable per-query registry snapshot, callback re-entry | C33 |
| Prepared/binding additions (#181/#180/#316/#462/#465/#467/#322) | Browser prepared/cursor/UDF APIs and set conversions; other repositories inspected below; Entity and Val(Ref) binds interchangeable; API-017/018 constructors and API-022/023/024 binding failures | C34, C36 |
| Other engine optimizations (#530/#672/#694/#721/#723/#774/#795) | Batched/dense UUID projection; cached recursive probes, delta ordering and bound-value AVET joins; small in-memory ancestor alternative; literal-keyword equality cache; unique query metadata unstable sorts | C14, C19, C34, C35 |
| Public behavior/options compatibility | Rule limits total derived facts (#553), deprecated `max_results` alias; derived metadata nil and QRY-013 (#548), unknown predicates resolved at open (#551); PRS-080 and QRY-010/011 (#406/#414), parser codes/grammar tightening (#643/#651) | C19, C37 |
| Cargo/release changes (#439/#551/#596/#796) | Production uuid drops v4/js; `wasm` now deprecated no-op; new test-hooks and envelope profile; dist uses full LTO | C38, C39 |
| Testing/documentation/release infrastructure | New v8 corpus, property/model/fault tests, fuzz corpus, grammar verifier, Kani encoders/decoders, Shuttle schedules, soak/envelope, durability guide, browser Tier 1 promotion | C39 |
| Query profiler (#185) | ROADMAP scope and open issue; no public profiler API in `lib.rs`/`db.rs` at HEAD. Private engine counters are not the promised public profiler | C40 |

### Fixes also on v2, not separate v3-only coverage obligations

Verified against `git show v2.0.11:CHANGELOG.md` release entries and endpoint
diffs. Their **v3-specific execution costs** remain in the matrix where relevant.
The v3 Unreleased prose still contains backported fixes and older release-status
text; it must not be treated as a list of exclusively v3 functionality.

| Released on v2 | Also-on-v2 fixes / shared work | Treatment |
| --- | --- | --- |
| 2.0.3 | #445 pending >65,535 indexing; #447 WAL transaction counter | EXCLUDED as new features; pending-vs-committed engine costs remain C01/C14 |
| 2.0.4 | #470 browser dirty-page read errors; v7 golden files and crash test | EXCLUDED; v8/browser migration remains C07/C36 |
| 2.0.5 | #506 short files, #513 torn WAL, #528 rule result counts, #457 all-checkpointed WAL removal, #476 never-checkpoint sentinel; soak/coverage work | EXCLUDED as new features; v8 recovery/checkpoint costs remain C03/C06 |
| 2.0.6 | #554 whole-transaction visibility, #555 query snapshot, #558 v2 checkpoint publication race | EXCLUDED as new guarantees; v3 generation pins/quarantine and working-tree snapshots remain C13/C30 |
| 2.0.7 | #603 prepared literal time options; #608 demand/magic sets; #610 cross-product or | EXCLUDED as new behavior; new engine paths remain C16/C19/C34 |
| 2.0.8 | #615/#617 temporal metadata in negation/rules; #620/#625/#637 or expression/equality; #628/#633 strata/recursion and total result counting | EXCLUDED as new fixes; v3 operators and #553 alias/default change remain C16/C19/C37 |
| 2.0.9 | #649 extra fact elements; #661 rule metadata; #673 browser error codes; grammar-verifier infrastructure | EXCLUDED as new fixes. Other #643/#651 parser changes remain inventoried in C37 |
| 2.0.10 | #679 prepared rule limits, #686 million-fact WAL replay, #706 float constants; value-encoder Kani proofs (#627) | EXCLUDED as new fixes; chunked v3 writes and memory limit remain C24/C30 |
| 2.0.11 | #734 transaction call guards; #744 keyword/ref expression equality; #750 deterministic signed-zero rule calls (part of #545 on v3) | EXCLUDED as new semantics; #774 literal caching and v3 joins remain C14/C35 |

Baseline API inspection confirms Rust `prepare`/`PreparedQuery::execute`,
`register_predicate`/`register_aggregate`, `max_derived_facts`/`max_results`,
`page_cache_size`, `SyncMode`, explicit transactions, windows and recursion
already exist in v2. Their new paths, exposed bindings or changed behavior—not
their original introduction—are the v3 obligations here. #558 is the v2 race;
the v3-only pin/publication work is #543. File format v8, window replacement,
statement ordering and atomic save (#371/#435/#477/#374) are **not** backported.

### Binding evidence and distinct costs

V records exact binding and core revisions. Each measured binding is built
with a Cargo patch to the measured core checkout; the helper verifies Cargo
metadata and retains it with the metrics. These are source builds, not a
verification of published package artifacts. Source entry points at the pinned
revisions were checked against the executed IDs and measurements.

| Binding | Inspected/measured source revision | Disposition |
| --- | --- | --- |
| Python | [16f54fef94c75272873b3381fc83cf264ed05e99](https://github.com/project-minigraf/minigraf-python/blob/16f54fef94c75272873b3381fc83cf264ed05e99/bench/nightly.py) | COVERED six foreign-runtime cases in V; UniFFI/Python allocation, prepared integer/set binds, cursor batches, predicate/aggregate callbacks |
| Browser | [da29682e34e6f0a32a1d63882becfeb01dd5d55d](https://github.com/project-minigraf/minigraf-wasm/blob/da29682e34e6f0a32a1d63882becfeb01dd5d55d/bench/browser.mjs), core browser module at audited SHA | COVERED eight cases in V, including IndexedDB write-through and reopen |
| Node | [96fe7a1a9f7521007fefdfee54142694ab39251d](https://github.com/project-minigraf/minigraf-node/blob/96fe7a1a9f7521007fefdfee54142694ab39251d/bench/cases.mjs) | COVERED six N-API/JS cases in V |
| C | [653af45e874b15e54564a2d5e12d5583784c7861](https://github.com/project-minigraf/minigraf-c/blob/653af45e874b15e54564a2d5e12d5583784c7861/bench/nightly.c) | COVERED six C/JSON/handle/callback cases in V |
| Java/JVM | Original source audit [68de43c6c37e19c4783f7f709448429d52c6d4f3](https://github.com/project-minigraf/minigraf-java/blob/68de43c6c37e19c4783f7f709448429d52c6d4f3/src/lib.rs) | MANUAL JVM/JNA/allocation/callback timing; no V job |
| Android | Original source audit [110a9dc58aa61934f65eb026ba88792c35f413b5](https://github.com/project-minigraf/minigraf-android/blob/110a9dc58aa61934f65eb026ba88792c35f413b5/src/lib.rs) | MANUAL ART/ABI/device storage; fixed-device release sign-off required |
| Swift | Original source audit [4f2b296d081491ac1185052404f46e03cbd9dd19](https://github.com/project-minigraf/minigraf-swift/blob/4f2b296d081491ac1185052404f46e03cbd9dd19/src/lib.rs) | MANUAL ARC/callback/Apple-device storage; release sign-off required |
| WASI | Original WASM source audit [7f69e78250575840c4bedf97b46f6f73cad12455](https://github.com/project-minigraf/minigraf-wasm/blob/7f69e78250575840c4bedf97b46f6f73cad12455/src/lib.rs) | MANUAL separate runtime/startup overhead; browser timing does not measure WASI |

Other binding-repository performance histories are **unverified** here.
The originally inspected UniFFI/Node/C option structs expose read-only and query
memory limits, but not core `transaction_chunk_facts`; do not assume every
option has reached every binding. The measured shortlist does not exercise
option/error latency or foreign FactLog/LogWriter conversion. See
[BINDING_BENCHMARKS.md](BINDING_BENCHMARKS.md) for fixture/timing boundaries and
manual release checks. Python's larger cursor envelope remains separate from
its new 1k nightly shortlist.

## Coverage matrix

`COVERED` means the named representative IDs are selected by the nightly route,
executed/uploaded in V, and have a stored v3 metric/threshold model (B). It does
not mean every semantic variant, memory bound or numeric comparison baseline
exists. `MANUAL` and `ENVELOPE` explicitly lack verified nightly latency
coverage; `EXCLUDED` gives a deliberate reason. All covered rows cite **V+B**;
Appendix A links the individual jobs, exact filters and every selected ID.

Fixture shorthand: **M4** is pending in-memory scalar data at 1k/10k/100k/1m;
**M3/M2** are its first three/two sizes. **F4** is the corresponding checkpointed
file data, page cache 256; **W** adds uncheckpointed WAL. Multi-attribute fixture
sizes count entities/rows unless stated otherwise. New fixture modes are stated
per row. File copies/fsync/setup and checks stay outside measured operations
unless the case explicitly measures open/write/checkpoint; OS cache is warm,
with no cold-cache guarantee. ID prefixes/ellipses expand only to the actual
selected IDs in Appendix A, not every registered case in a family.

| Row | Feature/subpath | Benchmark IDs / source evidence | Fixture and measured operation | Nightly job/filter; evidence | Final disposition |
| --- | --- | --- | --- | --- | --- |
| C01 | Streaming scalar points/scans and expressions | `query/point_{entity,attribute}/…`, `query/regex_filter/…`, `expr/{filter_scale,binding_scale,binding_into_agg}/…` | M4 scalar, M3 expressions, M2 regex | query `^query/`; expr `^expr/`; V+B | COVERED native pending representatives |
| C02 | Committed scalar reads, UUID projection, B+tree cursor | `query/ordered_groups/…`, `query/partition_windows/…`, `concurrent_btree_scan/…` | F4 groups/windows; F 10k/100k, 2/4/8 reader threads | query; concurrent_btree_scan `^concurrent_btree_scan`; V+B | COVERED; sparse/repeated-ref projection remains MANUAL (X06) |
| C03 | COW checkpoint/allocation/meta/CRC | `checkpoint/{1k,10k,100k,1m}`, `checkpoint/{after_1_fact,after_100k_facts}/…` | W full flush; F + one fact or 100k in independent 1k statements; incremental fixture grows across samples | checkpoint `^checkpoint`; V+B | COVERED; X06 for sustained pin/churn scaling |
| C04 | Long values, dedup and same-statement multivalues | `checkpoint/storage_v3/multivalue_{inline_unique,long_repeated,long_unique}/10k`, `query/storage_v3/multivalue_long_scan/10k`; `storage_v3.rs` | 2,500 entities × four values, 32/256-byte strings; fresh-copy checkpoint or committed full scan | checkpoint; query; V+B | COVERED; invalid size bounds EXCLUDED as correctness paths (X08) |
| C05 | WAL append and explicit small transactions | `insert/{single_fact,batch_100,explicit_tx}/…`, `insert_file/{single_fact,batch_100,explicit_tx}/…` | M4/F4 plus pending writes; repeated benchmark triples | insert `^insert/`; insert_file `^insert_file/`; V+B | COVERED; chunked paths in C30 |
| C06 | Checkpointed v8 open and WAL v2 replay | `open/{checkpointed,wal_replay}/…` | F4 checkpointed; W 1k/10k/100k facts in 100-fact statements, read-write open/drop | open `^open/`; V+B | COVERED; million-fact single-entry replay EXCLUDED from latency timing; correctness-only (X08) |
| C07 | Native v7 migration and nonmigrating read-only v7 | `open/storage_v3/{migrate_v7,read_only_v7}/multi_checkpoint`; `storage_v3.rs` | Frozen v7 multi-checkpoint corpus, all manifest queries validated; copies outside timed open | open; V+B | COVERED small native representative; large/browser migration MANUAL (X03) |
| C08 | Integrity verification and index rebuild | `checkpoint/storage_v3/verify/100k_history`, `checkpoint/storage_v3/rebuild_damaged_avet/100k_history` | 100,150 stored/99,950 live facts; rebuild starts from CRC-damaged current AVET root, copied per iteration | checkpoint; V+B | COVERED; arbitrary failure-code latency EXCLUDED (X08) |
| C09 | Committed net-assert/history, current and as-of | `point_query_chain_depth/{churned_attr,sibling_attr,attr_scan,churned_attr_as_of}/{1,2000}`; `history.rs` | 2,000 filler facts, churned triple and sibling; depth includes initial assertion; checkpointed | history_retract shortlist in Appendix A; V+B | COVERED eight IDs; intermediate depth 500 EXCLUDED as redundant (X01) |
| C10 | Retraction, last statement and window replacement | `retract/{single_fact,batch_100}/10k`, `insert_file/chunked/{retract,write_tx}/{65535,65537}` | M 10k repeated retracts; fresh file chunk controls include later-statement window replacement | history_retract; insert_file; V+B | COVERED; other retract scales EXCLUDED (X01) |
| C11 | Read-only open/query and in-memory WAL replay | `open/storage_v3/read_only_v8/10k`, `open/storage_v3/read_only_wal/10k_plus_1k`, C07; ordered query groups | F 10k with/without 1k WAL facts; source bytes checked unchanged; read-only query fixtures | open; query; V+B | COVERED representative native opens and reads |
| C12 | Cursor full draining and early limit | `query/ordered_groups/…`, `query/partition_windows/…`, `query/cursor_lifecycle/drain/{10k,100k}` | F4 proof groups use batch 1000/full drain or limit 10; dedicated drain uses batch 256 | query; V+B | COVERED |
| C13 | Cursor lifecycle, pins and staged snapshot reads | `query/cursor_lifecycle/…`; `cursor_lifecycle.rs` | F 10k open/first batch/iterator/close; 10k/100k drain; old 10k snapshot across two setup checkpoints; staged 10k+100 pending+100 uncommitted | query; V+B | COVERED eight IDs; checkpoint setup is untimed, sustained quarantine growth MANUAL (X06) |
| C14 | Entity probes, pending filters and selective recursion | `query/join_3pattern/…`, `query/predicate_pushdown/…`, `query/operator_paths/entity_{pending,committed}/10k`, `recursion/ancestors/w10_d4` | M4 joins, M3 selective filters; paired 20k-fact joins; 11,110-edge ancestor tree | query; recursion `^recursion/`; V+B | COVERED; optimization-specific isolation remains MANUAL (X06) |
| C15 | Value/hash joins and per-fact metadata source | `query/operator_paths/{entity,value,hash}_{pending,committed}/10k`; `operator_paths.rs` | 10k output rows/20k facts; pending memory vs committed file; hash deliberately scans broader attribute domain; preflight operator checks | query; V+B | COVERED six natural-plan controls; exhaustive cross-kind/forced variants EXCLUDED (X07) |
| C16 | Anti/union operators and rule bodies | `negation/…`, `disjunction/…`, `query/operator_paths/{anti_committed,union_committed}/10k` | M 1k/10k selectivity/rule cases; committed correlated not plus hashed not-join, union with nested outer-variable expressions | negation `^negation/`; disjunction `^disjunction/`; query; V+B | COVERED representatives; forced strategy permutations EXCLUDED (X07) |
| C17 | Pseudo metadata and transaction/valid-time bounds | `temporal_metadata/…`, `time_travel/{as_of_counter,valid_at}/…`, `time_travel/operator_paths/timestamp_{monotone,irregular}/10k` | M3 metadata (tx_time case reads tx-count), M4 broad temporal points; committed 10k records, irregular timestamps require non-prefix cutoff | temporal_metadata `^temporal_metadata/`; time_travel `^time_travel/`; V+B | COVERED broad and selective committed timestamp representatives |
| C18 | General aggregates, exact arithmetic and :with | `aggregation/{count_scale,grouped_count_scale,sum_scale,with_grouped_sum,count_distinct_scale}/…` | M2 integer/count/:with; 10 departments, 50% count-distinct duplication | aggregation `^aggregation/`; V+B | COVERED; mixed/exact controls in C20 |
| C19 | Recursive engine/demand and rule limit checking | `recursion/{chain,fanout,ancestors}/…`, `recursion/operator_paths/ancestors_committed/10k`; rule-body negation/disjunction | Pending chain/tree; committed 10-edge ancestor chain amid 10k filler facts | recursion; negation; disjunction; V+B | COVERED positive recursion; nonlinear/multistratum/limit scaling ENVELOPE/acceptance (X02) |
| C20 | Ordinary avg, mixed/exact sums/extrema, tuple fallbacks | `aggregation/v3_controls/…`; `aggregate_controls.rs` | Four hash cases, M 10k mixed numerics/extrema; six 100k skew/tuple controls, pending or committed/read-only | aggregation; V+B | COVERED ten cases; exhaustive numeric permutations EXCLUDED (X07) |
| C21 | Sort omission/UUID tie fallback and top-k | `query/ordered_values/{index,uuid_ties}/…`, `query/pagination/…` | F4 ascending limit 10; pending/committed offset and descending controls in C25 | query; V+B | COVERED |
| C22 | Ordered aggregation and effective tuple proofs | `query/ordered_groups/…`, `aggregation/v3_controls/{skew_count_committed,skew_count_pending,skew_avg_committed,tuple_repeated_constant,tuple_float_constant_fallback,tuple_hidden_with_fallback}/100k` | F4 14 singleton modes (56 IDs), plus skew: 90k rows in one bucket/100 buckets of 100; hidden :with and float fallback | query; aggregation; V+B | COVERED; ordered_groups/distinct means count-distinct, not final :distinct |
| C23 | Partition streaming and incompatible windows | `query/partition_windows/{value,entity,value_limit,entity_limit}/…`, `query/operator_paths/windows_committed/10k` | F4 singleton partitions plus 100×100 rows and two incompatible window orders | query; V+B | COVERED; larger shape sweeps ENVELOPE (X06) |
| C24 | Memory-limit enforcement overhead and bounds | `memory_meter/{count,hash_groups}/{unlimited,limited}/10k`; `memory_meter.rs` | F 10k read-only; unlimited vs 8 MiB finite limit; 4 KiB QRY-012 preflight is untimed; accounting always enabled | memory_meter `^memory_meter/`; V+B | COVERED four latency controls; RSS/state bounds ENVELOPE; seven acceptance tests observed, conditional RSS enforcement unconfirmed (X02) |
| C25 | Limit/offset/order/top-k and prepared count slots | `query/pagination/…`, `prepared/{count_slots,unprepared_count_slots}/10k`; `pagination.rs`, `prepared.rs` | Eight 100k pending/committed controls, offset 0/half, descending k=10/1000; compound ties 10k/100k; prepared limit 10/offset 100 | query; prepared `^prepared/`; V+B | COVERED; 10m sweeps ENVELOPE (X06) |
| C26 | Final projected-tuple :distinct | `query/distinct/{bag,scalar,sorted_page,float_set_tuple}/{10k,100k}`; `distinct.rs` | M duplicate-heavy input, 1,000 unique keys/tuples; float/set signed zeros, descending sort/offset/limit composition | query; V+B | COVERED eight IDs |
| C27 | lag/lead | `window/neighbors/{lag,lead}/{one,hundred}/10k`, `window/neighbors/lag_lead/{one,hundred}/100k` | M 10k/100k rows, one/100 partitions, four-row peer groups and boundary nulls | window `^window/`; V+B | COVERED six IDs |
| C28 | Bounded rows and registered aggregate replay | 18 `window_rows/…` IDs; exact shortlist in Appendix A | M 10k/100k rows, 2n facts; sum zero/two/prefix one/100 partitions; min/max and bounded/full-prefix UDF controls | window_rows exact anchored shortlist in Appendix A; V+B | COVERED 18 IDs; remaining 110 MANUAL, not nightly (X01) |
| C29 | Set storage/queries/binds and collect-set | `{insert_file,checkpoint,query}/set_storage/…`, `query/set_queries/{in,not_in,all_in,any_none,collect}/{pending,committed}/10k` | 10k storage facts, 4-element inline/32-element long repeated/unique sets; 10k entities for pending/committed membership and empty collection; binds in C36 | insert_file; checkpoint; query; V+B | COVERED ten storage + ten query cases; host set binds in C36; cardinality sweeps MANUAL (X06) |
| C30 | Chunked execute/retract/write transactions/rollback | `insert_file/chunked/{execute,retract,write_tx}/{65535,65537}`, `insert_file/chunked/statement_rollback/65537`, C13 staged_read | Default 65,536-fact boundary, fresh file per iteration; two staged statements; late API-011 after first chunk, then valid commit | insert_file; query; V+B | COVERED seven chunk IDs and staged-read control; million-fact/scaling bounds ENVELOPE/acceptance (X02) |
| C31 | FactLog storage/tx order, filters and bounded window | `fact_log/…`; `fact_log.rs` | 10k/100k historical records +100 pending; batch 1024, default/4096 window, attribute-only and entity/prefix/tx-range filters | fact_log `^(fact_log\|log_writer)/`; V+B | COVERED six IDs; long-held log pin/churn MANUAL (X06) |
| C32 | LogWriter history/holes/windows and finish | `log_writer/{append,finish}/{10k,100k}`; `fact_log.rs` | Four historical phases, tx holes, long strings/sets; 10k below and 100k above 65,536 writer batch; append and finish isolated | fact_log; V+B | COVERED four IDs; foreign conversions MANUAL (X04) |
| C33 | Fallible UDFs, registry snapshot churn and re-entry | `udf/{aggregate_sum_dispatch,predicate_filter_dispatch}/…`, `udf/v3/…`; `udf_v3.rs` | Matched 10k fallible/infallible successful callbacks; fixed 32-UDF registry with held cursor; 1k callbacks re-enter same database | udf `^udf/`; V+B | COVERED baseline four + six v3 IDs; deliberate callback error timing EXCLUDED (X08) |
| C34 | Prepared planning/substitution/execute/cursor and slots | `prepared/…`; `prepared.rs` | M 1k/10k execute; 10k preparation + unprepared controls, cursor drain batch 256, count/time/entity slots | prepared; V+B | COVERED 12 IDs; foreign set binding in C36; exhaustive binds EXCLUDED (X07) |
| C35 | Expression-local literal keyword/ref cache | `expr/keyword_equality/{literal_eq,dynamic_eq,literal_neq,dynamic_neq,nested_literal_eq,prepared_keyword_eq}/10k` | Same M 10k keyword rows; literal/dynamic include different bind/clone costs, not an isolated cache-off switch | expr `^expr/`; V+B | COVERED six IDs, including nested/prepared cache construction |
| C36 | Foreign prepare/cursor/set/callback and browser persistence | `binding/{python,browser,node,c}/…` (Appendix B); binding sources above | 1k integer facts + one two-element set; full cursor drains batch 1/256, JSON/host conversions; browser fresh IndexedDB 1k write/reopen | binding_python/browser/node/c fixed shortlists; dedicated testbeds; V+B | COVERED 26 non-Criterion IDs; no numeric comparison baselines yet; remaining hosts/APIs MANUAL (X04) |
| C37 | Changed error/validation contracts | PRS/API/QRY codes, failed checkpoint retry refusal, corrupt-tree checks; correctness suites | Successful queries pay validation; rollback cost represented in C30 | No separate error-code latency route; no dedicated timed V case | EXCLUDED individual invalid-input timing; correctness tests retain the contracts (X08) |
| C38 | Synthetic SIMD candidates | `simd_{temporal,as_of,aggregate}/{scalar,simd}/…` (30 unselected) | Vec slices, not the database/streaming engine | No selecting nightly filter; no dedicated timed V case | EXCLUDED pre-existing experimental helper kernels; no production-feature claim |
| C39 | Proof/test/docs/release changes and shared-reader study | Kani/Shuttle/fault/property/fuzz/soak, uuid features, binary-size workflow, #796/#797 | Proof cfgs absent from normal builds; bench already full LTO; #797 is a study, not shipped shared-reader implementation | Separate correctness/size gates, no feature latency route; no dedicated timed V case | EXCLUDED from Criterion obligations; manual dist-size measurements and runtime verification stay separate |
| C40 | Planned profiler/OPFS/rule persistence | Public profiler #185, OPFS #441, rule persistence #241 remain open; no shipped API at audited revision | Not implemented/in the audited release code | No route; no dedicated timed V case | EXCLUDED until shipped; refresh inventory before release cut (X09) |
| C41 | Older btree_lookup groups | `btree_lookup/{entity_point,attribute_scan}/{1k,10k,100k}` (six unselected) | Pending memory fixture despite name; duplicates C01 scalar lookups | No selecting nightly filter; no dedicated timed V case | EXCLUDED redundant pending coverage; not committed B+tree evidence |

## Workflow, reporting and run evidence

The scheduled workflow is main's `.github/workflows/bench.yml`. Its unchanged
crons select main at 02:00 UTC and v3 at 14:00 UTC; dispatch selects `inputs.ref`.
V is a **workflow_dispatch**, not a newly observed cron run. `headSha` is the
main workflow revision; compile resolves v3 once, and every native/binding job
checks out the compile SHA. Reporting helpers separately use `workflow_sha`.
Suite availability is checked in the measured source so main/old refs skip
absent suites consistently rather than upload empty results.

Compile builds only `minigraf_bench` and enumerates IDs once with `--list`.
Each required native route selects from the shared listing, rejects empty or
duplicate selections, then checks exact analyzed and uploaded sets. The native
jobs use `pipefail`, ten-or-existing sample settings, 360-minute timeouts and
fail-fast false; compile timeout is 120 minutes. Workflow concurrency queues
runs rather than cancelling them. Binding jobs have 60-minute timeouts, verify
every case untimed, then validate finite JSON nanosecond latency metrics and
exact executed/uploaded sets.

Bencher CLI remains pinned to v0.4.25. Native data uses project `minigraf`,
branch `v3`, testbed `ubuntu-latest`, adapter `rust_criterion`, measure latency;
non-main native uploads inherit main's start point. Every suite now uses one
common upper probability **0.999**, maximum 30 samples. This resolves the former
parallel replacement of different models on the same branch/testbed/measure
threshold. It is a probability threshold, not a 0.1% latency budget. Previously
0.99 suites are less sensitive. Binding JSON uploads use separate
`binding-{python,browser,node,c}-ubuntu24-v1` testbeds, branch `v3`, with the same
model settings and no native-testbed history inheritance. Experimental binding
refs use separate history branches.

Uploads remain `continue-on-error`, but the always-run final audit fails missing,
partial, duplicate, unexpected or API-error uploads. Reports and artifacts
retain filters, exact IDs, measured/workflow hashes, outcome and alerts for
30 days. All V uploads succeeded and every Report regression step was skipped.
The source's notification policy still reports latency alerts only if the same
benchmark alerted in the previous same-branch/suite run; upload failures without
alerts are reported immediately. A previous-log lookup failure warns and leaves
an empty previous-alert set. Parser/audit tests cover success, alert and upload
failure formats, but **live failed-upload/alert notification delivery is
unverified end to end**. No alert injection was performed for this update.

P15's main/v3 `smoke.yml` agrees: main smoke at 05:00 UTC, pinned v3 acceptance
at 17:00 UTC. The [#837 execution audit](benchmarks/837-acceptance/README.md)
observed scheduled run
[41729196180](https://github.com/project-minigraf/minigraf/actions/runs/41729196180):
all five acceptance jobs and seven intended tests passed, within their budgets.
Measured source was `c5863c79e4039e54c7fa67e0d7a8b8751f9b1599`; workflow source
was `4d65022ac963a10b582ac762c4ab2e73935c9115`. Complete outputs, named tests,
asserted numeric bounds, durations and platform limitations are retained there.
This supersedes the earlier inspection of run 41637991836, which used the old
workflow and ran only main smoke. V does not invoke smoke acceptance.

The observed tests cover linear recursive derived-fact exhaustion, QRY-012,
limit/top-k finite caps, the default-chunk million-fact transaction and storage
size. Nonlinear/multistratum scaling and custom chunk bounds did not execute;
conditional RSS assertions do not log whether they were skipped. The scheduled
source guard rejects missing integration files but does not guard against zero
matching tests. X02 therefore remains open for those gaps. These acceptance
results are separate from V's Bencher latency evidence.

| Evidence | Run | Verified scope |
| --- | --- | --- |
| Before #776 (original audit) | [41538175832](https://github.com/project-minigraf/minigraf/actions/runs/41538175832), [41337918695](https://github.com/project-minigraf/minigraf/actions/runs/41337918695) | Original audit recorded 270 native IDs, 21 suites; measured `29dd5888498a55b4be24db08f1250ca5571bbd44`; these are historical evidence, not final v3 execution |
| Batch 1 | [41603905357](https://github.com/project-minigraf/minigraf/actions/runs/41603905357) | Issue comment records 338 native IDs/25 green jobs; run metadata rechecked successful. Its individual sets were not re-audited here |
| V, final dispatch | [41642356471](https://github.com/project-minigraf/minigraf/actions/runs/41642356471) | 31/31 successful; 427 native IDs across 26 suites +26 foreign IDs across four jobs; exact selection/analysis/upload sets and stored SHA-specific metrics independently checked |

Final native suite counts (each column has the same count; **all alerts 0**):

| Job in V | Filter | Selected = analyzed = uploaded | Elapsed including setup/upload |
| --- | --- | ---: | --- |
| [insert](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774972) | `^insert/` | 12 | 13m 51s |
| [insert_file](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774931) | `^insert_file/` | 23 | 6m 58s |
| [query](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774955) | `^query/` | 146 | 44m 44s |
| [memory_meter](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775113) | `^memory_meter/` | 4 | 2m 01s |
| [prepared](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774899) | `^prepared/` | 12 | 3m 27s |
| [history_retract](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774980) | `^(point_query_chain_depth/(churned_attr\|sibling_attr\|attr_scan\|churned_attr_as_of)/(1\|2000)\|retract/(single_fact\|batch_100)/10k)$` | 10 | 4m 08s |
| [fact_log](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775081) | `^(fact_log\|log_writer)/` | 10 | 3m 13s |
| [time_travel](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774937) | `^time_travel/` | 10 | 8m 30s |
| [recursion](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775090) | `^recursion/` | 5 | 2m 55s |
| [open](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774998) | `^open/` | 11 | 4m 30s |
| [checkpoint](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774925) | `^checkpoint` | 17 | 39m 43s |
| [concurrent/readers](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775314) | `^concurrent/readers/` | 6 | 3m 34s |
| [concurrent/readers_plus_writer](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775238) | `^concurrent/readers_plus_writer/` | 6 | 3m 33s |
| [concurrent/serialized_writers](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775269) | `^concurrent/serialized_writers/` | 8 | 4m 27s |
| [concurrent_file/readers](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775285) | `^concurrent_file/readers/` | 6 | 3m 24s |
| [concurrent_file/readers_plus_writer](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775219) | `^concurrent_file/readers_plus_writer/` | 6 | 3m 26s |
| [concurrent_file/serialized_writers](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775220) | `^concurrent_file/serialized_writers/` | 8 | 3m 58s |
| [concurrent_btree_scan](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775276) | `^concurrent_btree_scan` | 6 | 2m 50s |
| [negation](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775366) | `^negation/` | 16 | 5m 05s |
| [disjunction](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775292) | `^disjunction/` | 15 | 4m 17s |
| [aggregation](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775454) | `^aggregation/` | 20 | 5m 27s |
| [expr](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775272) | `^expr/` | 15 | 5m 00s |
| [window](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775437) | `^window/` | 18 | 5m 59s |
| [window_rows](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775341) | `^window_rows/((10000\|100000)/(one\|hundred)/sum/(zero\|two\|prefix)\|100000/hundred/(min\|max)/(two\|prefix)\|10000/hundred/frame_bench_sum/two\|100000/hundred/frame_bench_sum/prefix)$` | 18 | 6m 03s |
| [temporal_metadata](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775273) | `^temporal_metadata/` | 9 | 4m 26s |
| [udf](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939775260) | `^udf/` | 10 | 3m 23s |

These are whole-job elapsed times from V, not isolated incremental allowances
or portable latency promises. The run's sampling finished within the configured
job timeouts. New histories still need observations: a green upload plus a
threshold model cannot supply a missing numeric comparison baseline.

Public-history examples (the same SHA/threshold checks were made for every ID):

- [query/distinct/scalar/100k](https://api.bencher.dev/v0/projects/minigraf/perf?branches=01a11620-4681-7f30-801b-d9ad268088ca&heads=01a1a18f-7bfd-70cf-b3f9-61d5e0a8b037&testbeds=4ea63737-9f02-4ea0-8058-13ed4f643b49&benchmarks=01a1a0b4-6f2c-73c8-8c3b-9df292492271&measures=92472195-78d4-4ba0-ac1f-b575b6498171&start_time=1791122585000&end_time=1793714585000)
- [prepared/count_slots/10k](https://api.bencher.dev/v0/projects/minigraf/perf?branches=01a11620-4681-7f30-801b-d9ad268088ca&heads=01a1a18f-7bfd-70cf-b3f9-61d5e0a8b037&testbeds=4ea63737-9f02-4ea0-8058-13ed4f643b49&benchmarks=01a1a08f-0347-72a8-b841-f46c383b1acc&measures=92472195-78d4-4ba0-ac1f-b575b6498171&start_time=1791119902000&end_time=1793711902000)
- [binding/python/prepared_execute/1k](https://api.bencher.dev/v0/projects/minigraf/perf?branches=01a11620-4681-7f30-801b-d9ad268088ca&heads=01a1a18f-7bfd-70cf-b3f9-61d5e0a8b037&testbeds=01a1a191-294c-75f5-8458-9712f085ff64&benchmarks=01a1a191-294e-7569-9217-bd0cf33ae8d4&measures=92472195-78d4-4ba0-ac1f-b575b6498171&start_time=1791120007000&end_time=1793712007000)

## Closed-out G01–G18 implementation plan

All P01–P17 PRs below are **merged**, checked through GitHub PR metadata;
every core v3 merge is an ancestor of the audited SHA. Shared workflow changes
also landed on main in separate companion PRs. “Closed out” means implemented
representatives or an explicit residual disposition, not that every manual
release check has run. Source modules are under `benches/minigraf_bench/` unless
otherwise stated. V supplies the final execution/upload evidence for all the
native/foreign shortlists; exclusions remain in the next section.

| Gap | Merged follow-up PRs | Final resolution / remaining disposition | Matrix |
| --- | --- | --- | --- |
| G01 | P01 [#812](https://github.com/project-minigraf/minigraf/pull/812) (main [#813](https://github.com/project-minigraf/minigraf/pull/813)) | 18 routed bounded-frame IDs; 110-case manual sweep X01 | C28 |
| G02 | P02 [#815](https://github.com/project-minigraf/minigraf/pull/815) (main [#814](https://github.com/project-minigraf/minigraf/pull/814)) | `prepared.rs`, 12 routed preparation/execute/cursor/typed-slot controls | C25/C34 |
| G03 | P03 [#817](https://github.com/project-minigraf/minigraf/pull/817) | `distinct.rs`, eight bag/scalar/float-set/pagination cases | C26 |
| G04 | P04 [#818](https://github.com/project-minigraf/minigraf/pull/818) | `pagination.rs`, ten offset/descending/compound-tie controls; 10m envelope X06 | C25 |
| G05 | P05 [#819](https://github.com/project-minigraf/minigraf/pull/819) + P06 [#820](https://github.com/project-minigraf/minigraf/pull/820) | `set_storage.rs` ten storage cases + `set_queries.rs` ten pending/committed membership/collection cases | C29 |
| G06 | P06 [#820](https://github.com/project-minigraf/minigraf/pull/820) | `window_neighbors.rs`, six one/100-partition lag/lead cases | C27 |
| G07 | P15 [#831](https://github.com/project-minigraf/minigraf/pull/831) (main [#830](https://github.com/project-minigraf/minigraf/pull/830)) | `memory_meter.rs`, four finite/unlimited latency controls; v3 acceptance routing fixed; revised schedule observed in #837, remaining gaps X02 | C24 |
| G08 | P07 [#821](https://github.com/project-minigraf/minigraf/pull/821) | `operator_paths.rs`, 12 join/anti/union/window/time/committed-recursion controls; deeper scaling X02/X06 | C15–C17/C19/C23 |
| G09 | P08 [#822](https://github.com/project-minigraf/minigraf/pull/822) | `cursor_lifecycle.rs`, eight lifecycle/pin/staged-read cases; sustained churn X06 | C13/C30 |
| G10 | P09 [#823](https://github.com/project-minigraf/minigraf/pull/823) | `chunked_transactions.rs`, seven boundary/retract/multi-statement/rollback cases; large-bound acceptance X02 | C10/C30 |
| G11 | P10 [#826](https://github.com/project-minigraf/minigraf/pull/826) (main [#825](https://github.com/project-minigraf/minigraf/pull/825)) | `fact_log.rs`, six log reads +four writer append/finish cases | C31/C32 |
| G12 | P12 [#827](https://github.com/project-minigraf/minigraf/pull/827) | `storage_v3.rs`, ten migration/repair/long-multivalue/read-only cases; large/browser migration X03 | C04/C07/C08/C11 |
| G13 | P11 [#816](https://github.com/project-minigraf/minigraf/pull/816) (main [#809](https://github.com/project-minigraf/minigraf/pull/809)) | `history.rs`, eight history +two retract IDs; redundant lookups/scales excluded X01 | C09/C10/C41 |
| G14 | P13 [#828](https://github.com/project-minigraf/minigraf/pull/828) | `udf_v3.rs`, six fallible/control/churn/re-entry cases; error timing X08 | C33 |
| G15 | P16 core [#832](https://github.com/project-minigraf/minigraf/pull/832) (main [#833](https://github.com/project-minigraf/minigraf/pull/833)); [Python #17](https://github.com/project-minigraf/minigraf-python/pull/17), [browser #21](https://github.com/project-minigraf/minigraf-wasm/pull/21), [Node #21](https://github.com/project-minigraf/minigraf-node/pull/21), [C #14](https://github.com/project-minigraf/minigraf-c/pull/14) | 26 foreign metrics in V; remaining hosts/APIs/manual sign-off X04 | C36 |
| G16 | P14 [#829](https://github.com/project-minigraf/minigraf/pull/829) | `aggregate_controls.rs`, ten mixed/exact/hash/skew/effective-tuple controls | C20/C22 |
| G17 | P13 [#828](https://github.com/project-minigraf/minigraf/pull/828) | `keyword_equality.rs`, six literal/dynamic/nested/prepared cases | C35 |
| G18 | P17 [#810](https://github.com/project-minigraf/minigraf/pull/810) (main [#811](https://github.com/project-minigraf/minigraf/pull/811)), P16 reporting additions [#832](https://github.com/project-minigraf/minigraf/pull/832)/#833 | Enumeration/selection/provenance, complete uploads, common threshold; V+B verifies successful reporting, live alert path and baseline maturation remain X05 | Workflow audit |

## Remaining exclusions and follow-ups

Every item here lacks some nightly evidence or is deliberately outside the
shortlist. No manual timing or release sign-off is implied by this close-out.

| ID | Disposition | Remaining scope and reason |
| --- | --- | --- |
| X01 | MANUAL / EXCLUDED registered cases | Of 583 registered native IDs, 156 are unselected: 110 `window_rows/` cases are MANUAL wider width/function/partition sweeps; four history depth-500 IDs and six retract 1k/100k/1m IDs are EXCLUDED redundant intermediate/scaling points; six pending `btree_lookup/` IDs are EXCLUDED duplicates of C01; 30 synthetic SIMD IDs are EXCLUDED experimental non-engine kernels. Exact expansions are in Appendix C. The [#836 pinned reconciliation and per-shape dispositions](benchmarks/836-exclusions/README.md) retain all exclusions. Full-prefix one-partition UDF execution stays manual but uses a linear running accumulator, not quadratic replay; the unregistered 100k UDF half-frames remain excluded for replay cost |
| X02 | ENVELOPE / acceptance, partial execution verified | [#837 evidence](benchmarks/837-acceptance/README.md): all five scheduled jobs/seven tests passed; linear recursion/INT-020, QRY-012, limit/top-k caps, default-chunk million-fact transaction and storage bounds observed. Still open: nonlinear/multistratum scaling, custom chunk bounds, explicit RSS assertion execution and future zero-test protection. These are not Bencher memory metrics; accounting has no disabled-engine control, and unlimited only disables the finite cap |
| X03 | MANUAL migration | V retains its small native fixture. [#838 manual evidence](benchmarks/838-migration/README.md) separately measures native v7-to-v8 through 200k facts and Chromium/Firefox/WebKit through 20k facts, plus bounded post-migration cursor churn. Read-only v7 open is nonmigrating; same-context reopen is not cold startup. The 200k browser rung and sustained pin scaling remain excluded. These manual samples are not V or new nightly coverage |
| X04 | MANUAL binding checks | [#839's runtime evidence](benchmarks/839-bindings/README.md) records three series each for JVM, Android, Swift/iOS and WASI (1,350 samples); binding/release acceptance remains pending. New #839 timings use only Device Farm/the dedicated bench; macOS is excluded, existing GitHub nightlies unchanged. [The remaining API/lifecycle dispositions](benchmarks/839-bindings/dispositions.md) identify measured representatives, absent APIs and proposed exclusions, including reader churn beyond [#838's bounded persistence case](benchmarks/838-migration/README.md). Python shared UniFFI coverage does not measure other host runtimes; no binding RSS bound is asserted |
| X05 | Unverified operational follow-ups | Accumulate numeric regression baselines for 244 native and 26 foreign IDs (all have stored v3 observations/model settings, but baseline null in V). Verify live failed-upload and repeated-alert notification delivery; successful uploads and parser/audit unit tests do not exercise it. Cron execution of the final routes is unverified here: V was a dispatch. Preserve/refresh evidence beyond the 30-day artifact retention |
| X06 | MANUAL / ENVELOPE scaling and isolation | Large 10m sorting/window/envelope sweeps, wider set cardinalities, sustained pin/quarantine/log-reader churn and sparse/repeated-ref UUID projection are outside V's fixed shortlists. [#838](benchmarks/838-migration/README.md) measures only eight commits with one old cursor plus a short-lived cursor per commit; it establishes no sustained-pin or quarantine-memory bound. Dedicated isolation of each recursive-probe optimization is unverified; representative queries do not establish per-optimization timings |
| X07 | EXCLUDED exhaustive semantics | All numeric, bind, cross-kind join and forced-planner permutations remain correctness tests. Nightly controls select representative natural plans; broader hash controls include broader scans and keyword dynamic controls include binding/clone work |
| X08 | EXCLUDED individual failure latency | Invalid bounds/windows/binds, deliberate callback errors, corrupt-page rejection, v1–v6 refusal, heavy single-entry WAL replay and individual PRS/API/QRY codes belong to correctness testing. C08 measures a successful repair of known damage and C30 measures cross-chunk rollback; neither claims exhaustive failure timing |
| X09 | EXCLUDED unshipped/non-runtime scope | Public profiler #185, OPFS #441 and rule persistence #241 are open/unshipped at the audited revision. Proof/test/docs infrastructure and #797's study add no shipped latency path; production uuid/dist size uses separate size gates. Re-audit new production features through the release cut |

Issue #776 remains open. Its final release decision still needs the explicit
manual sign-offs and verification above; this PR neither closes it nor checks
its acceptance boxes automatically.

## Appendix A: exact native IDs executed and uploaded in V

These are **artifact-verified**, not statically inferred. Each block is the
actual selected set, independently matched to completed analyses and Bencher
result URLs. The filter/count/job link is in the suite table above. B confirms
a stored metric at the audited SHA for each ID. All native IDs use the shared
0.999/30 threshold on `v3`/`ubuntu-latest`/latency.

### `insert` — 12 IDs

```text
insert/single_fact/1k
insert/single_fact/10k
insert/single_fact/100k
insert/single_fact/1m
insert/batch_100/1k
insert/batch_100/10k
insert/batch_100/100k
insert/batch_100/1m
insert/explicit_tx/1k
insert/explicit_tx/10k
insert/explicit_tx/100k
insert/explicit_tx/1m
```

### `insert_file` — 23 IDs

```text
insert_file/single_fact/1k
insert_file/single_fact/10k
insert_file/single_fact/100k
insert_file/single_fact/1m
insert_file/batch_100/1k
insert_file/batch_100/10k
insert_file/batch_100/100k
insert_file/batch_100/1m
insert_file/explicit_tx/1k
insert_file/explicit_tx/10k
insert_file/explicit_tx/100k
insert_file/explicit_tx/1m
insert_file/set_storage/scalar/10k
insert_file/set_storage/inline_unique/10k
insert_file/set_storage/long_repeated/10k
insert_file/set_storage/long_unique/10k
insert_file/chunked/execute/65535
insert_file/chunked/retract/65535
insert_file/chunked/write_tx/65535
insert_file/chunked/execute/65537
insert_file/chunked/retract/65537
insert_file/chunked/write_tx/65537
insert_file/chunked/statement_rollback/65537
```

### `query` — 146 IDs

```text
query/set_storage/exact_inline_unique/10k
query/set_storage/exact_long_unique/10k
query/set_storage/scan_long_unique/10k
query/point_entity/1k
query/point_entity/10k
query/point_entity/100k
query/point_entity/1m
query/point_attribute/1k
query/point_attribute/10k
query/point_attribute/100k
query/point_attribute/1m
query/join_3pattern/1k
query/join_3pattern/10k
query/join_3pattern/100k
query/join_3pattern/1m
query/distinct/bag/10k
query/distinct/scalar/10k
query/distinct/sorted_page/10k
query/distinct/float_set_tuple/10k
query/distinct/bag/100k
query/distinct/scalar/100k
query/distinct/sorted_page/100k
query/distinct/float_set_tuple/100k
query/pagination/committed_asc_offset0_k10/100k
query/pagination/committed_asc_offset_half_k10/100k
query/pagination/committed_desc_k10/100k
query/pagination/committed_desc_k1000/100k
query/pagination/pending_asc_offset0_k10/100k
query/pagination/pending_asc_offset_half_k10/100k
query/pagination/pending_desc_k10/100k
query/pagination/pending_desc_k1000/100k
query/pagination/committed_compound_ties/10k
query/pagination/committed_compound_ties/100k
query/operator_paths/entity_pending/10k
query/operator_paths/value_pending/10k
query/operator_paths/hash_pending/10k
query/operator_paths/entity_committed/10k
query/operator_paths/value_committed/10k
query/operator_paths/hash_committed/10k
query/operator_paths/anti_committed/10k
query/operator_paths/union_committed/10k
query/operator_paths/windows_committed/10k
query/cursor_lifecycle/open/10k
query/cursor_lifecycle/first_batch/10k
query/cursor_lifecycle/drain/10k
query/cursor_lifecycle/drain/100k
query/cursor_lifecycle/iterator/10k
query/cursor_lifecycle/held_checkpoint_drain/10k
query/cursor_lifecycle/held_checkpoint_close/10k
query/cursor_lifecycle/staged_read/10k_plus_200
query/ordered_values/index/1k
query/ordered_values/uuid_ties/1k
query/ordered_values/index/10k
query/ordered_values/uuid_ties/10k
query/ordered_values/index/100k
query/ordered_values/uuid_ties/100k
query/ordered_values/index/1m
query/ordered_values/uuid_ties/1m
query/ordered_groups/count/1k
query/ordered_groups/distinct/1k
query/ordered_groups/count_limit/1k
query/ordered_groups/distinct_limit/1k
query/ordered_groups/avg/1k
query/ordered_groups/sum/1k
query/ordered_groups/min/1k
query/ordered_groups/max/1k
query/ordered_groups/sum_distinct/1k
query/ordered_groups/avg_limit/1k
query/ordered_groups/sum_limit/1k
query/ordered_groups/min_limit/1k
query/ordered_groups/max_limit/1k
query/ordered_groups/sum_distinct_limit/1k
query/ordered_groups/count/10k
query/ordered_groups/distinct/10k
query/ordered_groups/count_limit/10k
query/ordered_groups/distinct_limit/10k
query/ordered_groups/avg/10k
query/ordered_groups/sum/10k
query/ordered_groups/min/10k
query/ordered_groups/max/10k
query/ordered_groups/sum_distinct/10k
query/ordered_groups/avg_limit/10k
query/ordered_groups/sum_limit/10k
query/ordered_groups/min_limit/10k
query/ordered_groups/max_limit/10k
query/ordered_groups/sum_distinct_limit/10k
query/ordered_groups/count/100k
query/ordered_groups/distinct/100k
query/ordered_groups/count_limit/100k
query/ordered_groups/distinct_limit/100k
query/ordered_groups/avg/100k
query/ordered_groups/sum/100k
query/ordered_groups/min/100k
query/ordered_groups/max/100k
query/ordered_groups/sum_distinct/100k
query/ordered_groups/avg_limit/100k
query/ordered_groups/sum_limit/100k
query/ordered_groups/min_limit/100k
query/ordered_groups/max_limit/100k
query/ordered_groups/sum_distinct_limit/100k
query/ordered_groups/count/1m
query/ordered_groups/distinct/1m
query/ordered_groups/count_limit/1m
query/ordered_groups/distinct_limit/1m
query/ordered_groups/avg/1m
query/ordered_groups/sum/1m
query/ordered_groups/min/1m
query/ordered_groups/max/1m
query/ordered_groups/sum_distinct/1m
query/ordered_groups/avg_limit/1m
query/ordered_groups/sum_limit/1m
query/ordered_groups/min_limit/1m
query/ordered_groups/max_limit/1m
query/ordered_groups/sum_distinct_limit/1m
query/partition_windows/value/1k
query/partition_windows/entity/1k
query/partition_windows/value_limit/1k
query/partition_windows/entity_limit/1k
query/partition_windows/value/10k
query/partition_windows/entity/10k
query/partition_windows/value_limit/10k
query/partition_windows/entity_limit/10k
query/partition_windows/value/100k
query/partition_windows/entity/100k
query/partition_windows/value_limit/100k
query/partition_windows/entity_limit/100k
query/partition_windows/value/1m
query/partition_windows/entity/1m
query/partition_windows/value_limit/1m
query/partition_windows/entity_limit/1m
query/set_queries/in/pending/10k
query/set_queries/not_in/pending/10k
query/set_queries/all_in/pending/10k
query/set_queries/any_none/pending/10k
query/set_queries/collect/pending/10k
query/set_queries/in/committed/10k
query/set_queries/not_in/committed/10k
query/set_queries/all_in/committed/10k
query/set_queries/any_none/committed/10k
query/set_queries/collect/committed/10k
query/regex_filter/1k
query/regex_filter/10k
query/storage_v3/multivalue_long_scan/10k
query/predicate_pushdown/1k
query/predicate_pushdown/10k
query/predicate_pushdown/100k
```

### `memory_meter` — 4 IDs

```text
memory_meter/count/unlimited/10k
memory_meter/count/limited/10k
memory_meter/hash_groups/unlimited/10k
memory_meter/hash_groups/limited/10k
```

### `prepared` — 12 IDs

```text
prepared/value_lookup/1k
prepared/value_lookup/10k
prepared/threshold_filter/1k
prepared/threshold_filter/10k
prepared/prepare_value_lookup/10k
prepared/unprepared_value_lookup/10k
prepared/cursor_drain/10k
prepared/unprepared_cursor_drain/10k
prepared/count_slots/10k
prepared/unprepared_count_slots/10k
prepared/time_entity_slots/10k
prepared/unprepared_time_entity_slots/10k
```

### `history_retract` — 10 IDs

```text
retract/single_fact/10k
retract/batch_100/10k
point_query_chain_depth/churned_attr/1
point_query_chain_depth/churned_attr/2000
point_query_chain_depth/sibling_attr/1
point_query_chain_depth/sibling_attr/2000
point_query_chain_depth/attr_scan/1
point_query_chain_depth/attr_scan/2000
point_query_chain_depth/churned_attr_as_of/1
point_query_chain_depth/churned_attr_as_of/2000
```

### `fact_log` — 10 IDs

```text
fact_log/attribute_only_storage/10k
fact_log/tx_mixed_default/10k
fact_log/storage_mixed/100k
fact_log/tx_mixed_default/100k
fact_log/tx_mixed_window_4096/100k
fact_log/filter_attribute_entity_tx/100k
log_writer/append/10k
log_writer/finish/10k
log_writer/append/100k
log_writer/finish/100k
```

### `time_travel` — 10 IDs

```text
time_travel/operator_paths/timestamp_monotone/10k
time_travel/operator_paths/timestamp_irregular/10k
time_travel/as_of_counter/1k
time_travel/as_of_counter/10k
time_travel/as_of_counter/100k
time_travel/as_of_counter/1m
time_travel/valid_at/1k
time_travel/valid_at/10k
time_travel/valid_at/100k
time_travel/valid_at/1m
```

### `recursion` — 5 IDs

```text
recursion/operator_paths/ancestors_committed/10k
recursion/chain/depth_10
recursion/chain/depth_100
recursion/fanout/w10_d3
recursion/ancestors/w10_d4
```

### `open` — 11 IDs

```text
open/checkpointed/1k
open/checkpointed/10k
open/checkpointed/100k
open/checkpointed/1m
open/wal_replay/1k
open/wal_replay/10k
open/wal_replay/100k
open/storage_v3/migrate_v7/multi_checkpoint
open/storage_v3/read_only_v7/multi_checkpoint
open/storage_v3/read_only_v8/10k
open/storage_v3/read_only_wal/10k_plus_1k
```

### `checkpoint` — 17 IDs

```text
checkpoint/set_storage/inline_unique/10k
checkpoint/set_storage/long_repeated/10k
checkpoint/set_storage/long_unique/10k
checkpoint/1k
checkpoint/10k
checkpoint/100k
checkpoint/1m
checkpoint/after_1_fact/10k
checkpoint/after_1_fact/100k
checkpoint/after_1_fact/1m
checkpoint/after_100k_facts/10k
checkpoint/after_100k_facts/100k
checkpoint/storage_v3/verify/100k_history
checkpoint/storage_v3/rebuild_damaged_avet/100k_history
checkpoint/storage_v3/multivalue_inline_unique/10k
checkpoint/storage_v3/multivalue_long_repeated/10k
checkpoint/storage_v3/multivalue_long_unique/10k
```

### `concurrent/readers` — 6 IDs

```text
concurrent/readers/10k_4t
concurrent/readers/10k_8t
concurrent/readers/10k_16t
concurrent/readers/100k_4t
concurrent/readers/100k_8t
concurrent/readers/100k_16t
```

### `concurrent/readers_plus_writer` — 6 IDs

```text
concurrent/readers_plus_writer/10k_4t
concurrent/readers_plus_writer/10k_8t
concurrent/readers_plus_writer/10k_16t
concurrent/readers_plus_writer/100k_4t
concurrent/readers_plus_writer/100k_8t
concurrent/readers_plus_writer/100k_16t
```

### `concurrent/serialized_writers` — 8 IDs

```text
concurrent/serialized_writers/10k_2t
concurrent/serialized_writers/10k_4t
concurrent/serialized_writers/10k_8t
concurrent/serialized_writers/10k_16t
concurrent/serialized_writers/100k_2t
concurrent/serialized_writers/100k_4t
concurrent/serialized_writers/100k_8t
concurrent/serialized_writers/100k_16t
```

### `concurrent_file/readers` — 6 IDs

```text
concurrent_file/readers/10k_4t
concurrent_file/readers/10k_8t
concurrent_file/readers/10k_16t
concurrent_file/readers/100k_4t
concurrent_file/readers/100k_8t
concurrent_file/readers/100k_16t
```

### `concurrent_file/readers_plus_writer` — 6 IDs

```text
concurrent_file/readers_plus_writer/10k_4t
concurrent_file/readers_plus_writer/10k_8t
concurrent_file/readers_plus_writer/10k_16t
concurrent_file/readers_plus_writer/100k_4t
concurrent_file/readers_plus_writer/100k_8t
concurrent_file/readers_plus_writer/100k_16t
```

### `concurrent_file/serialized_writers` — 8 IDs

```text
concurrent_file/serialized_writers/10k_2t
concurrent_file/serialized_writers/10k_4t
concurrent_file/serialized_writers/10k_8t
concurrent_file/serialized_writers/10k_16t
concurrent_file/serialized_writers/100k_2t
concurrent_file/serialized_writers/100k_4t
concurrent_file/serialized_writers/100k_8t
concurrent_file/serialized_writers/100k_16t
```

### `concurrent_btree_scan` — 6 IDs

```text
concurrent_btree_scan/10k_2t
concurrent_btree_scan/10k_4t
concurrent_btree_scan/10k_8t
concurrent_btree_scan/100k_2t
concurrent_btree_scan/100k_4t
concurrent_btree_scan/100k_8t
```

### `negation` — 16 IDs

```text
negation/not_scale/1k
negation/not_scale/10k
negation/not_join_scale/1k
negation/not_join_scale/10k
negation/not_selectivity/excl_0pct
negation/not_selectivity/excl_25pct
negation/not_selectivity/excl_50pct
negation/not_selectivity/excl_75pct
negation/not_selectivity/excl_100pct
negation/not_pushdown_selectivity/excl_0pct
negation/not_pushdown_selectivity/excl_25pct
negation/not_pushdown_selectivity/excl_50pct
negation/not_pushdown_selectivity/excl_75pct
negation/not_pushdown_selectivity/excl_100pct
negation/not_rule_body/1k
negation/not_rule_body/10k
```

### `disjunction` — 15 IDs

```text
disjunction/or_scale/1k
disjunction/or_scale/10k
disjunction/or_join_scale/1k
disjunction/or_join_scale/10k
disjunction/or_selectivity/match_0pct
disjunction/or_selectivity/match_25pct
disjunction/or_selectivity/match_50pct
disjunction/or_selectivity/match_75pct
disjunction/or_selectivity/match_100pct
disjunction/or_short_circuit/1k
disjunction/or_short_circuit/10k
disjunction/or_join_short_circuit/1k
disjunction/or_join_short_circuit/10k
disjunction/or_rule_body/1k
disjunction/or_rule_body/10k
```

### `aggregation` — 20 IDs

```text
aggregation/count_scale/1k
aggregation/count_scale/10k
aggregation/grouped_count_scale/1k
aggregation/grouped_count_scale/10k
aggregation/sum_scale/1k
aggregation/sum_scale/10k
aggregation/with_grouped_sum/1k
aggregation/with_grouped_sum/10k
aggregation/v3_controls/hash_avg/10k
aggregation/v3_controls/hash_exact_float_sum/10k
aggregation/v3_controls/hash_mixed_sum_distinct/10k
aggregation/v3_controls/hash_mixed_extrema/10k
aggregation/v3_controls/skew_count_committed/100k
aggregation/v3_controls/skew_count_pending/100k
aggregation/v3_controls/skew_avg_committed/100k
aggregation/v3_controls/tuple_repeated_constant/100k
aggregation/v3_controls/tuple_float_constant_fallback/100k
aggregation/v3_controls/tuple_hidden_with_fallback/100k
aggregation/count_distinct_scale/1k
aggregation/count_distinct_scale/10k
```

### `expr` — 15 IDs

```text
expr/filter_scale/1k
expr/filter_scale/10k
expr/filter_scale/100k
expr/binding_scale/1k
expr/binding_scale/10k
expr/binding_scale/100k
expr/binding_into_agg/1k
expr/binding_into_agg/10k
expr/binding_into_agg/100k
expr/keyword_equality/literal_eq/10k
expr/keyword_equality/dynamic_eq/10k
expr/keyword_equality/literal_neq/10k
expr/keyword_equality/dynamic_neq/10k
expr/keyword_equality/nested_literal_eq/10k
expr/keyword_equality/prepared_keyword_eq/10k
```

### `window` — 18 IDs

```text
window/running_sum/1k
window/running_sum/10k
window/running_sum/100k
window/rank/1k
window/rank/10k
window/rank/100k
window/many_partitions/1k
window/many_partitions/10k
window/many_partitions/100k
window/row_number/1k
window/row_number/10k
window/row_number/100k
window/neighbors/lag/one/10k
window/neighbors/lag/hundred/10k
window/neighbors/lead/one/10k
window/neighbors/lead/hundred/10k
window/neighbors/lag_lead/one/100k
window/neighbors/lag_lead/hundred/100k
```

### `window_rows` — 18 IDs

```text
window_rows/10000/one/sum/zero
window_rows/10000/one/sum/two
window_rows/10000/one/sum/prefix
window_rows/10000/hundred/sum/zero
window_rows/10000/hundred/sum/two
window_rows/10000/hundred/sum/prefix
window_rows/10000/hundred/frame_bench_sum/two
window_rows/100000/one/sum/zero
window_rows/100000/one/sum/two
window_rows/100000/one/sum/prefix
window_rows/100000/hundred/sum/zero
window_rows/100000/hundred/sum/two
window_rows/100000/hundred/sum/prefix
window_rows/100000/hundred/min/two
window_rows/100000/hundred/min/prefix
window_rows/100000/hundred/max/two
window_rows/100000/hundred/max/prefix
window_rows/100000/hundred/frame_bench_sum/prefix
```

### `temporal_metadata` — 9 IDs

```text
temporal_metadata/tx_time/1k
temporal_metadata/tx_time/10k
temporal_metadata/tx_time/100k
temporal_metadata/valid_from/1k
temporal_metadata/valid_from/10k
temporal_metadata/valid_from/100k
temporal_metadata/valid_to/1k
temporal_metadata/valid_to/10k
temporal_metadata/valid_to/100k
```

### `udf` — 10 IDs

```text
udf/aggregate_sum_dispatch/1k
udf/aggregate_sum_dispatch/10k
udf/predicate_filter_dispatch/1k
udf/predicate_filter_dispatch/10k
udf/v3/predicate_infallible/10k
udf/v3/aggregate_infallible/10k
udf/v3/predicate_fallible/10k
udf/v3/aggregate_fallible/10k
udf/v3/registry_snapshot_churn/32
udf/v3/callback_reentry/1k
```

## Appendix B: exact foreign-runtime IDs in V

Fixed shortlists, no Criterion regex. Each case includes the actual host
conversion/result handling. All use 1k scalar facts plus a two-element set;
IndexedDB cases use a fresh 1k-fact database. Fixtures, preparation and callback
registration are untimed. Full cursor drains use batches 1/256. Browser reopen
is in the same page/context, not cold browser startup. Native and foreign
results have distinct testbeds and must not be combined into one latency series.

### [browser](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774470) — 8 IDs

Binding `da29682e34e6f0a32a1d63882becfeb01dd5d55d`; testbed `binding-browser-ubuntu24-v1`; selected = executed = uploaded **8**, alerts **0**. V+B; numeric comparison baselines absent.

```text
binding/browser/prepared_execute/1k
binding/browser/cursor_batch_1/1k
binding/browser/cursor_batch_256/1k
binding/browser/set_roundtrip/2
binding/browser/predicate/1k
binding/browser/aggregate/1k
binding/browser/indexeddb_write/1k
binding/browser/indexeddb_reopen/1k
```

### [c](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774245) — 6 IDs

Binding `653af45e874b15e54564a2d5e12d5583784c7861`; testbed `binding-c-ubuntu24-v1`; selected = executed = uploaded **6**, alerts **0**. V+B; numeric comparison baselines absent.

```text
binding/c/prepared_execute/1k
binding/c/cursor_batch_1/1k
binding/c/cursor_batch_256/1k
binding/c/set_roundtrip/2
binding/c/predicate/1k
binding/c/aggregate/1k
```

### [node](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774523) — 6 IDs

Binding `96fe7a1a9f7521007fefdfee54142694ab39251d`; testbed `binding-node-ubuntu24-v1`; selected = executed = uploaded **6**, alerts **0**. V+B; numeric comparison baselines absent.

```text
binding/node/prepared_execute/1k
binding/node/cursor_batch_1/1k
binding/node/cursor_batch_256/1k
binding/node/set_roundtrip/2
binding/node/predicate/1k
binding/node/aggregate/1k
```

### [python](https://github.com/project-minigraf/minigraf/actions/runs/41642356471/job/125939774468) — 6 IDs

Binding `16f54fef94c75272873b3381fc83cf264ed05e99`; testbed `binding-python-ubuntu24-v1`; selected = executed = uploaded **6**, alerts **0**. V+B; numeric comparison baselines absent.

```text
binding/python/prepared_execute/1k
binding/python/cursor_batch_1/1k
binding/python/cursor_batch_256/1k
binding/python/set_roundtrip/2
binding/python/predicate/1k
binding/python/aggregate/1k
```

## Appendix C: all deliberately unselected registered IDs

The enumeration/selected-set difference is exactly **156** IDs. These have no
V analysis or upload. Counts below are not omitted-feature counts.
The [#836 follow-up](benchmarks/836-exclusions/README.md) re-enumerates a later
pinned revision and records an explicit cost/disposition for every manual shape;
it adds no latency measurements and leaves this historical validation unchanged.

| Prefix / expansion | Count | Disposition and reason |
| --- | ---: | --- |
| `window_rows/…` | 110 | MANUAL wider sweep; C28's 18 representatives bound routine nightly work |
| `point_query_chain_depth/{churned_attr,sibling_attr,attr_scan,churned_attr_as_of}/500` | 4 | EXCLUDED intermediate depth; 1/2000 endpoints are routed |
| `retract/{single_fact,batch_100}/{1k,100k,1m}` | 6 | EXCLUDED extra scales; 10k and chunk-boundary retractions are routed |
| `btree_lookup/{entity_point,attribute_scan}/{1k,10k,100k}` | 6 | EXCLUDED duplicate pending lookups, not committed storage |
| `simd_{temporal,as_of,aggregate}/{scalar,simd}/{100,1k,10k,100k,1m}` | 30 | EXCLUDED synthetic helper kernels, no shipped database path |

The complete 128 registered `window_rows` IDs expand from:
`{10000,100000}/{one,hundred}/{sum,count,min,max,avg}/{default,zero,two,sqrt,half,prefix}`
(120), plus `10000/{one,hundred}/frame_bench_sum/{zero,two,half}` (six) and
`100000/{one,hundred}/frame_bench_sum/prefix` (two). Subtract the 18 literal
selected IDs in Appendix A to obtain the exact 110 manual cases. Shapes each
have 2n facts; widths are zero, two, square root, half or prefix of partition
size, with default using the peer frame. No other registered prefix is omitted.

## Reproducing the audit (read-only)

```sh
gh issue view 776 --comments
git rev-parse 'v2.0.11^{commit}' origin/v3 origin/main
git merge-base v2.0.11 origin/v3
git diff 4685ca9cb7638353087410c31ad07e3f6261ddb0 0394c90ad90e9b95d416f4a0ec4a4c78b16e5b57 -- src Cargo.toml ROADMAP.md CHANGELOG.md
git diff origin/main origin/v3 -- .github/workflows/bench.yml .github/workflows/smoke.yml .github/bench
gh run view 41642356471 --json headSha,conclusion,jobs
gh api repos/project-minigraf/minigraf/actions/jobs/125939774955
gh api repos/project-minigraf/minigraf/check-runs/125939774955
gh run download 41642356471 -D /tmp/776-evidence
```

For each native report, compare its `selected`, `executed` and `uploaded`
arrays with the raw Criterion `Benchmarking <ID>: Analyzing` lines and Bencher
`- <ID> (Latency): …/perf?…` lines. Compare the disjoint union with the listing
and Appendix A. For bindings, compare their fixed shortlist, `ok:` execution
lines, raw JSON latency keys, report/upload arrays and retained Cargo metadata.
Resolve public result URLs through `https://api.bencher.dev/v0/projects/minigraf/perf`
using their branch/head/testbed/benchmark/measure/time query parameters; inspect
the metric version hash, threshold model and numeric baseline separately.
