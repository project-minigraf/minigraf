# v3 nightly benchmark coverage audit (#776), phase 1

Audit date: 2026-11-03. This is an inventory and evidence report, not an
implementation or a claim that issue #776 can close. No benchmark or workflow
was changed. The work supports PHILOSOPHY.md's reliability, performance through
simplicity, and documentation principles; it adds no runtime dependency, public
API, or format change.

## Revisions and method

| Input | Revision |
| --- | --- |
| Supported v2 baseline | Annotated tag `v2.0.11`, peeled commit `709be06c391086ced67e3b0da4ab677de745ac25` |
| Audited v3 code, HEAD and local `origin/v3` | `4685ca9cb7638353087410c31ad07e3f6261ddb0` |
| Local `origin/main`, default-branch workflow inspected | `cabd3e2bfb90ceb354217f74033665d6d4af398a` |
| `merge-base v2.0.11 origin/v3` | `75ffc9b11f58cee3494430677465d553e582847b` |
| Default-branch workflow revision for both inspected scheduled runs | `4a77092aa8baffee58a8cf57f2d7c8805abf2d9c` |
| Actual v3 checkout in every job of both inspected runs | `29dd5888498a55b4be24db08f1250ca5571bbd44` |

Sources: [issue #776](https://github.com/project-minigraf/minigraf/issues/776),
[CHANGELOG.md](../CHANGELOG.md) Unreleased section,
[ROADMAP.md](../ROADMAP.md) v3.0.0 scope,
[CLAUDE.md](../CLAUDE.md), [PHILOSOPHY.md](../PHILOSOPHY.md),
`git log v2.0.11..origin/v3` (including inspection without merge commits), and
endpoint diffs of `src/lib.rs`, `src/db.rs`, `src/query/datalog/prepared.rs`,
`Cargo.toml` and the browser module. Endpoint diffs matter: cherry-picked v2
backports have different commit identities, so the log range alone overcounts
v3-only work. The v3 Cargo package still says 2.0.5; that is not the baseline.

The two `bench.yml` files at the audited v3 and `origin/main` revisions are
identical. The workflow used by the inspected runs is also identical to that
`origin/main` file. `benches/`, `Cargo.toml` and `bench.yml` have no differences
between the actual measured v3 checkout and the audited HEAD. Thus the names and
routes can be compared directly; the runs do **not** prove execution at HEAD.

**Criterion IDs below are derived statically, not listed.** A requested
`cargo bench --bench minigraf_bench -- --list` build was interrupted before
enumeration. The user subsequently prohibited remote builds and all new timed
runs, and authorized static derivation. No completed listing or new performance
measurement is claimed. The interrupted build's remote scratch was deleted.
Static expansion uses each `benchmark_group`, `BenchmarkId`, loop and conditional
in [benches/minigraf_bench.rs](../benches/minigraf_bench.rs), and the final
`criterion_group!` registration. Every one of its 31 registered functions and
73 group names is accounted for: **458 IDs, 270 selected, 188 unselected**.
Both inspected runs independently confirm the complete selected set through
`Benchmarking <ID>: Analyzing` and Bencher `View results` lines.

`Cargo.toml` explicitly declares `minigraf_bench` with `harness = false`.
`cargo metadata --no-deps --format-version 1` also reports an automatically
discovered `simd_helpers` bench target. That file contains helper functions, no
Criterion groups or benchmark registrations; compiling it adds no Criterion
coverage. No feature flags are passed by `bench.yml` (default features are empty).

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

The following v3 source revisions were read through GitHub's API. Links pin
`src/lib.rs`; `Cargo.toml` was also inspected. These are source audits, not
consumer executions or verification of released package artifacts.

| Binding | Inspected source | Additions/costs requiring their own disposition |
| --- | --- | --- |
| Python | [ade0eeadf2e2b2db3b4d18c529e4d2420fbb07ef](https://github.com/project-minigraf/minigraf-python/blob/ade0eeadf2e2b2db3b4d18c529e4d2420fbb07ef/src/lib.rs) | UniFFI prepare/execute/query, set values and bind conversion, foreign predicate/aggregate callbacks, open options/read-only/memory limit, cursor/log/log writer; Python allocation and crossing costs |
| Java | [68de43c6c37e19c4783f7f709448429d52c6d4f3](https://github.com/project-minigraf/minigraf-java/blob/68de43c6c37e19c4783f7f709448429d52c6d4f3/src/lib.rs) | Same exposed UniFFI paths; JVM conversion/callback costs |
| Android | [110a9dc58aa61934f65eb026ba88792c35f413b5](https://github.com/project-minigraf/minigraf-android/blob/110a9dc58aa61934f65eb026ba88792c35f413b5/src/lib.rs) | Same UniFFI paths; mobile runtime cost unmeasured here |
| Swift | [4f2b296d081491ac1185052404f46e03cbd9dd19](https://github.com/project-minigraf/minigraf-swift/blob/4f2b296d081491ac1185052404f46e03cbd9dd19/src/lib.rs) | Same UniFFI paths; Swift conversion/callback costs |
| Node | [823cf3a83a157fd22178f928dfb3ce562170ea07](https://github.com/project-minigraf/minigraf-node/blob/823cf3a83a157fd22178f928dfb3ce562170ea07/src/lib.rs) | Prepared/cursor objects, recursive set JSON/FactValue conversion, thread-safe JS predicate/aggregate callbacks, open options and logs |
| C | [22e4a8521018ce00c5d70ced3283b81dbb2bcbf8](https://github.com/project-minigraf/minigraf-c/blob/22e4a8521018ce00c5d70ced3283b81dbb2bcbf8/src/lib.rs) | Prepared handles/execute/query/free, JSON bindings/results/sets, callback contexts, registration, options, cursor/log/writer handles |
| Browser | Core `src/browser/{mod,binds,udfs,buffer,indexeddb}.rs` at audited HEAD; [WASM wrapper 7f69e78250575840c4bedf97b46f6f73cad12455](https://github.com/project-minigraf/minigraf-wasm/blob/7f69e78250575840c4bedf97b46f6f73cad12455/src/lib.rs) re-exports core | `BrowserPreparedQuery::execute/query`, `BrowserCursor::next_batch`, typed binds/JS Set, synchronous JS UDF conversions/registry, IndexedDB flush/migration/pins; needs browser target + `browser` feature |
| WASI | Same WASM repository | WASI runtime overhead, separate from browser persistence; no native file backend on wasm32 |

The UniFFI sources expose `MiniGrafError.Api` and foreign callback traits;
`query_memory_limit` and `read_only` are visible in their options. Core's new
`transaction_chunk_facts` option is not present in the inspected UniFFI, Node or
C options structs; do not assume every core option has reached every binding.
Python's existing [bench/envelope.py](https://github.com/project-minigraf/minigraf-python/blob/ade0eeadf2e2b2db3b4d18c529e4d2420fbb07ef/bench/envelope.py)
drains `db.query` with `next_batch(1000)` and measures latency/RSS. It is
MANUAL-or-ENVELOPE-ONLY for this audit: no Python invocation exists in the core
nightly benchmark workflow. Other repositories' scheduled performance histories
were not audited and are **unverified**, not evidence of core nightly coverage.

## Coverage matrix

`COVERED` means the named representative case has a selecting scheduled filter,
actual v3 execution/upload evidence and public latency history. It does not
mean every semantic variant or a memory bound is covered. A separate row marks
each materially different uncovered subpath. `MISSING` includes registered but
unrouted cases; `MANUAL-or-ENVELOPE-ONLY` includes reports/acceptance tests without
Criterion/Bencher coverage; `EXCLUDED` states a deliberate reason.

Fixture keys (from `benches/helpers/mod.rs` and benchmark bodies):

- **M4**: pending in-memory `:e{i} :val i`, 1k/10k/100k/1m facts, inserted in
  100-fact statements. **M3/M2**: first three/two sizes. Multi-attribute helpers
  add facts beyond the named entity count (e.g. names, department, tags, edges).
- **F4**: fully checkpointed scalar `.graph`, same four sizes, page cache 256.
  **F3/F2**: stated subsets. File fixtures are copied and fsynced; OS cache state
  is not controlled. Query proof fixtures open read-only before the timed loop.
- **W**: file + uncheckpointed WAL, never-checkpoint sentinel. Not a single
  transaction of that many facts. **CF**: F fixture plus pending writes during
  a mixed/writer run; automatic checkpoints suppressed.
- Every case here is native Rust with default features unless explicitly
  described as missing binding coverage. No registered case sets a nonzero
  `query_memory_limit` or custom `transaction_chunk_facts`.

Group prefixes in the matrix expand to the exact IDs in the route appendix.
`R1/R2 + B` means execution and successful upload in both scheduled runs below,
plus public Bencher latency history on project `minigraf`, branch `v3`, testbed
`ubuntu-latest`. Thresholds **T99/T999** mean requested t-test upper probability
0.99/0.999 with max sample size 30, not a 1%/0.1% latency budget; see reporting
audit. `None` in evidence/reporting means no verified scheduled evidence or
Bencher history for that subpath.

| Row | Feature/subpath | Criterion IDs / code evidence | Fixture and mode | Scheduled job/filter; evidence/reporting | Disposition |
| --- | --- | --- | --- | --- | --- |
| C01 | Streaming scalar point/scan and expression paths | `query/point_entity/…`, `query/point_attribute/…`, `expr/{filter_scale,binding_scale,binding_into_agg}/…`, `query/regex_filter/…` | M4 for points/scans; M3 expr; M2 string regex | query `^query/`, expr `^expr/`; R1/R2 + B, T99 | COVERED (native pending paths) |
| C02 | v8 committed scalar reads, UUID projection (#530), B+tree cursor | `query/ordered_groups/…`, `query/partition_windows/…`, `concurrent_btree_scan/…` | F4 IDs projected through cursor groups; F 10k/100k, 2/4/8 threads for entity point reads | query `^query/`, concurrent_btree_scan `^concurrent_btree_scan`; R1/R2 + B, T99/T999 | COVERED (representatives; sparse/repeated-ref projection unverified) |
| C03 | v8 checkpoint/COW allocation/meta/CRC cost | `checkpoint/{1k,10k,100k,1m}`, `checkpoint/after_1_fact/{10k,100k,1m}`, `checkpoint/after_100k_facts/{10k,100k}` | W full flush; F + one new fact; F + 100k added as 100 independent 1k statements | checkpoint `^checkpoint`; R1/R2 + B, T999 | COVERED; fixture grows across incremental samples |
| C04 | Long string value pages, dedup, multi-valued batches and file bounds | No representative long-value/multivalue Criterion case; scalar checkpoint is insufficient | Missing file inline/long, repeated/unique payload, multiple values per triple prefix | None | MISSING (G12); invalid bounds deliberately use correctness tests |
| C05 | v8 write/WAL append and explicit small transaction | `insert/…`, `insert_file/…` (single_fact, batch_100, explicit_tx) | M4; F4 + pending WAL writes; repeated writes of the same benchmark triples, not unbounded live-key growth | insert `^insert/`, insert_file `^insert_file/`; R1/R2 + B, T99/T999 | COVERED for small writes; not chunked transactions |
| C06 | Open checkpointed v8 and replay WAL v2 | `open/checkpointed/…`, `open/wal_replay/…` | F4 read-write open/drop; W 1k/10k/100k facts in 100-fact transactions, page cache 256 | open `^open/`; R1/R2 + B, T999 | COVERED; no large-entry replay fixture |
| C07 | v7→v8 migration, nonmigrating read-only v7, browser migration | No Criterion IDs; `persistent_facts.rs` and browser source | Missing historical v7 files/large dictionaries/browser persistence | None | MISSING (G12/G15); v1–v6 rejection and corrupt-page errors EXCLUDED from latency benchmarking (correctness paths) |
| C08 | verify/rebuild_indexes | No Criterion IDs; `storage/verify.rs`, `db.rs` | Missing file scale, history and rebuild after damage | None | MISSING (G12); refusing mismatched WAL #535/short files is correctness-only EXCLUDED |
| C09 | Long-history committed net-assert, current/as-of | `point_query_chain_depth/{churned_attr,sibling_attr,attr_scan,churned_attr_as_of}/{1,500,2000}` | Checkpointed file, 2,000 filler facts + hot triple history/sibling; depth includes initial version | No filter selects these 12 IDs; no R1/R2/B | MISSING (G13); envelope `hot_current/hot_as_of` does not route them |
| C10 | Retraction/last statement/window replacement | `retract/{single_fact,batch_100}/…` only tests repeated in-memory retractions | M4; no timed replace-window or multi-statement last-write fixture | No filter selects these 8 IDs; no R1/R2/B | MISSING (G13/G10) |
| C11 | Read-only query versus read-only open itself | Query proof groups use `.read_only(true)` outside loop | F4 open cost excluded from query iteration; no read-only WAL/open timing | query `^query/` covers queries only; R1/R2 + B, T99 | COVERED for read-only queries; MISSING open/WAL variant (G12) |
| C12 | Cursor full draining and built-in early limits | `query/ordered_groups/…`, `query/partition_windows/…` call `query`/`next_batch(1000)`; `_limit` modes request 10 | F4, one row per group/partition | query `^query/`; R1/R2 + B, T99 | COVERED for these cursor pulls |
| C13 | Cursor first batch/close/iterator, held snapshot across writes/checkpoints, pin/quarantine overhead | No dedicated IDs. `concurrent*/…` queries capture snapshots but do not hold a cursor across checkpoints | M and CF 10k/100k, 4/8/16 readers (writers also 2 threads); no held-generation churn | concurrent job filters in appendix; R1/R2 + B for baseline concurrency only | MISSING requested lifecycle/pin path (G09) |
| C14 | Entity nested-loop joins, native pending filters and selective recursion probes | `query/join_3pattern/…`, `query/predicate_pushdown/…`, `recursion/ancestors/w10_d4` | M4 chain, value+next facts; names/value M3 with top 10% filter; reversed 10-ary depth-4 tree (11,110 edges) | query `^query/`, recursion `^recursion/`; R1/R2 + B, T99 | COVERED for these queries (#672/#694/#721/#723 paths not individually instrumented) |
| C15 | Committed/pending value-probe vs hash join, pseudo source across joins, cross-kind keys | No paired Criterion IDs; envelope `join_2_pseudo`, `join_3_selective`, `join_3_scan`; tests cover plan forms | File envelope 100k/1m/10m; `query/join_3pattern` is pending entity probes | None for committed/hash controls | MANUAL-or-ENVELOPE-ONLY (G08) |
| C16 | Anti/union paths and rule bodies | `negation/{not_scale,not_join_scale,not_selectivity,not_pushdown_selectivity,not_rule_body}/…`, `disjunction/{or_scale,or_join_scale,or_selectivity,or_short_circuit,or_join_short_circuit,or_rule_body}/…` | M2, 10% exclusion; 10k varying exclusion/matches; tag/exclusion helpers add facts | negation `^negation/`, disjunction `^disjunction/`; R1/R2 + B, T99 | COVERED for listed pending queries; forced hash/correlated, committed and nested expression branches MISSING (G08) |
| C17 | Pseudo metadata and time travel on new engine | `temporal_metadata/{tx_time,valid_from,valid_to}/…`, `time_travel/{as_of_counter,valid_at}/…` | M3 metadata, M4 entity points; `tx_time` actually queries `:db/tx-count`; counter 999999 and valid-at 2099 let all fixture facts pass | temporal_metadata `^temporal_metadata/`, time_travel `^time_travel/`; R1/R2 + B, T99 | COVERED for these simple cases; committed timestamp/irregular TxBound path envelope-only (G08) |
| C18 | General aggregate holder, exact numeric sum, :with | `aggregation/{count_scale,grouped_count_scale,sum_scale,with_grouped_sum,count_distinct_scale}/…` | M2 integers; 10 department keys; count-distinct 50% duplication | aggregation `^aggregation/`; R1/R2 + B, T99 | COVERED for integer/count/:with representatives |
| C19 | Recursive relations, semi-naive/demand and rule limit checking | `recursion/chain/{depth_10,depth_100}`, `recursion/fanout/w10_d3`, `recursion/ancestors/w10_d4`; negation/disjunction rule-body IDs | M chains, 1,110-edge down tree and 11,110-edge up tree; no configured-limit exhaustion or recursive committed control | recursion `^recursion/` plus rule-body jobs; R1/R2 + B, T99 | COVERED for positive pending recursion; multistratum/nonlinear/committed/limit scaling MISSING (G08) |
| C20 | Ordinary avg, mixed-type/float exact aggregates, effective grouping tuples | `query/ordered_groups/avg/…` and ordered built-ins cover integer singleton groups only; no hash-avg/mixed/extrema/domain-proof fallback cases | F4 integer key/value; no big exact sums, skew, mixed types, hidden :with tuple or repeated dimension fixture | query `^query/`; R1/R2 + B only for singleton cases | MISSING additional #547/#777/#789/#781 costs (G16) |
| C21 | Scalar Sort omission and UUID tie fallback, top-k control | `query/ordered_values/{index,uuid_ties}/…` | F4 ascending integers, :limit 10; index omits Sort, UUID suffix retains it | query `^query/`; R1/R2 + B, T99 | COVERED for this #641 pair |
| C22 | Ordered aggregates (#641/#781) | `query/ordered_groups/{count,distinct,avg,sum,min,max,sum_distinct,count_limit,distinct_limit,avg_limit,sum_limit,min_limit,max_limit,sum_distinct_limit}/…` | F4 unique integer groups, full drains or :limit 10; `distinct` here is **count-distinct**, not final :distinct | query `^query/`; R1/R2 + B, T99 | COVERED for all 14 modes (56 IDs); skew/proof/fallback controls missing in C20 |
| C23 | Ordered partition-at-a-time window | `query/partition_windows/{value,entity,value_limit,entity_limit}/…` | F4 one-row partitions, row-number, full drain/:limit 10 | query `^query/`; R1/R2 + B, T99 | COVERED for 16 IDs; growing/multirow and incompatible-window controls MISSING (G08) |
| C24 | Meter overhead, memory limits, RSS/scaling | No Criterion ID sets a memory limit or uploads memory metrics; tests `memory_limit_acceptance_test`, `recursive_acceptance_test`, `limit_acceptance_test`, `large_transaction_acceptance_test` | Manual/acceptance and envelope scales up to 10m; retained-state estimates differ from process RSS | No Bencher memory measure; default-main smoke scheduling does not check out v3 | MANUAL-or-ENVELOPE-ONLY (G07); v3 nightly acceptance execution unverified |
| C25 | General limit/offset/order/top-k/slots | C21/C22/C23 cover fixed limit 10 and one ascending tie fallback; no offset, descending k sweep, pending top-k, compound output or slot IDs | Envelope `limit`, `top_k`; no Criterion offset/prepared counts | query only covers named controls; none for remaining paths | MANUAL-or-ENVELOPE-ONLY for broader sorting/limit, MISSING offset/slots (G04/G02) |
| C26 | Final output :distinct | No IDs contain or execute `:distinct`; aggregate count-distinct is different | Missing duplicate-heavy mixed/set tuples, bag control, pagination | None | MISSING (G03) |
| C27 | lag/lead | No Criterion query uses either function; existing `window/…` is sum/rank/row-number only | Missing one/many partitions, peer/tie and large partition cost | None | MISSING (G06) |
| C28 | Bounded rows and registered frame replay | `window_rows/…` (128 IDs, exact expansion below) | M 10k/100k rows, 2n facts (:val + :part), one/100 partitions; built-ins sum/count/min/max/avg, six widths; UDF zero/two/half at 10k and prefix at 100k | `^window/` selects **zero** of these; no other job selects them | MISSING nightly route (G01); existing manual #183 evidence is not nightly history |
| C29 | Set storage/serde/binds/predicates/collect-set | No Criterion case uses a set, membership predicate or collect-set | Missing inline ≤64 bytes/long sets, cardinality, exact-value index scans, per-row collection, pending/committed storage | None | MISSING (G05); #794 manual set measurements do not establish nightly coverage |
| C30 | Chunked parser/execute/retract/WriteTransaction, staged reads/rollback | No large-transaction IDs; insert explicit_tx stages one fact, checkpoint/after_100k uses independent 1k statements | No statement crossing 65,536 in measured loop; ignored million-fact acceptance exists | None; default-main smoke does not select v3 | MANUAL-or-ENVELOPE-ONLY acceptance; MISSING regression benches (G10/G09) |
| C31 | FactLog orders/filters/window/pinned reader | No Criterion IDs | Missing pending+committed history, tx vs storage order, selective filters, window sweeps | None | MISSING (G11) |
| C32 | LogWriter preserving history/holes/windows | No Criterion IDs; envelope fixture/acceptance setup uses it, outside query timing | Missing append/finish, checkpoint-boundary scaling, long/set values | None | MISSING (G11); fixture construction is not measurement |
| C33 | UDF registry snapshot baseline vs new fallible/re-entry paths | `udf/{aggregate_sum_dispatch,predicate_filter_dispatch}/…` use old infallible registration, query after setup | M2, integer aggregation/predicate; no registration churn, try_register, failure/re-entry or foreign callbacks | udf `^udf/`; R1/R2 + B, T99 for baseline dispatch only | COVERED baseline; MISSING #180-specific costs (G14) |
| C34 | Prepared execute/planning/substitution | `prepared/{value_lookup,threshold_filter}/{1k,10k}` | M2 integer slots, prepare outside timed loop; no prepared cursor, set/entity/time/count binds or preparation timing | No scheduled prepared filter; no R1/R2/B | MISSING (G02); old Rust API still matters to new bindings/#795 |
| C35 | Expression literal-keyword UUID cache (#774), plan/source metadata | No expression benchmark compares keyword/ref: existing expr cases use numeric <, arithmetic and sum | Manual `docs/benchmarks/774-expression-keywords.md`; no literal/dynamic/nested/prepared controls | None for keyword cases | MANUAL-or-ENVELOPE-ONLY (G17) |
| C36 | Binding prepare/UDF/set/options/cursor/log overhead, IndexedDB/browser snapshots | No binding execution in `benches/*.rs` or core `bench.yml`; Python manual envelope above | No JS/WASM/foreign conversion fixture, callback crossing or browser persistence | None in core nightly; other-repo histories unverified | MISSING (G15), Python cursor MANUAL-or-ENVELOPE-ONLY |
| C37 | Error contracts / changed validation, limit exhaustion | PRS/API/QRY codes, failed-checkpoint retry refusal (#390), corrupt-tree monotonicity/cycle refusal (#597), and derived-fact metadata/alias changes in inventory | Existing successful queries pay normal validation; no need for timing of every invalid input | None specific | EXCLUDED individual failure-code latency; preserve correctness tests. Rule/memory/large-write scaling remains C19/C24/C30 |
| C38 | Synthetic SIMD candidates | `simd_{temporal,as_of,aggregate}/{scalar,simd}/{100,1k,10k,100k,1m}` | Vec slices, neither database nor streaming query engine | No scheduled filter, no R1/R2/B | EXCLUDED from v3 feature obligations: pre-existing experimental helper kernels, not production feature coverage |
| C39 | Test/proof/documentation changes, uuid features, release LTO (#796), shared-reader study (#797) | Kani, Shuttle, fault/property/fuzz/soak, binary-size workflow; #797 adds audit harness/docs, not production shared-reader implementation | Normal builds exclude proof/schedule cfgs; bench already specifies full LTO, so it is not a before/after dist-profile test | Not a latency feature job | EXCLUDED from Criterion obligations; size gate/manual dist measurements are separate. Any future shipped profiler/runtime change needs a new row |
| C40 | Planned public profiler (#185), OPFS/rule persistence | #185 open; OPFS #441 and rule persistence #241 are ROADMAP v3.next | Not shipped at audited revision | None | EXCLUDED until implemented/in release; refresh before release cut |
| C41 | Older lookup groups, despite their B+tree name | `btree_lookup/{entity_point,attribute_scan}/{1k,10k,100k}` | In-memory names+values, no committed B+tree fixture | No filter selects these 6 IDs | EXCLUDED from required routing as redundant pending scalar lookup coverage; C01 supplies that path. Add a committed fixture rather than calling these file coverage |

## Workflow and regression-reporting audit

The authoritative scheduled file is `origin/main:.github/workflows/bench.yml`,
not the copy checked out from v3. Its cron is `0 2 * * *` for main and
`0 14 * * *` for v3. `BENCH_REF`, run-name and checkout use the cron string;
manual dispatch selects `inputs.ref`. Actual start times can be much later than
the cron, as R1 shows. `gh run list --branch v3` returned no runs; these schedules
are recorded under main even when `BENCH_REF=v3`.

Compile runs `cargo bench --no-run`, timeout 120 minutes. Each of 21 parallel
suite jobs depends on compile, restores target, and runs
`CARGO_TERM_COLOR=never cargo bench -- "${{ matrix.filter }}" 2>&1 | tee FILE`
with `pipefail`, timeout 360 minutes, fail-fast false. Workflow concurrency group
`bench` queues runs rather than cancelling. Compile success proves no case ran.
There is no step asserting the expected selected ID count; an empty filter is
not explicitly rejected by the workflow. Static matching finds **no currently
empty filters**, and no ID matches two scheduled filters.

Bencher CLI is pinned to v0.4.25, project `minigraf`, branch `$BENCH_REF`,
testbed `ubuntu-latest`, adapter `rust_criterion`, measure `latency`, test
`t_test`, `max-sample-size 30`, upper boundary from each matrix entry, `--err`.
Non-main uploads use `--start-point main --start-point-clone-thresholds`.
Only latency is configured; throughput annotations do not establish an RSS,
allocation, retained-state or bounded-memory regression gate.

Upload is `continue-on-error: true`. A failing upload invokes Report regression:
it parses latency alert URLs, reads up to 20 completed default-branch workflow
runs, chooses the previous same-title branch run, and matches the same suite job.
Only benchmark names alerting in both runs trigger a performance issue/comment.
An upload failure with no parsed alerts is reported immediately. Issues are
per suite and branch; existing issues receive comments. A previous-log lookup
failure warns and leaves the previous alert set empty, suppressing repeat-alert
reporting for that invocation. These are code-verified behaviors, not a tested
alert injection in this audit. R1/R2 had successful uploads and skipped reporting;
the issue-creation/notification path is **unverified end to end** here.

**Threshold scope needs a follow-up (G18).** The public `/thresholds` response
contains one active v3/ubuntu-latest/latency threshold, UUID
`01a11620-46a3-7582-b2b1-ab58de905220`, whose latest model is t_test, max 30,
upper boundary 0.999. All suites send the same branch/testbed/measure tuple,
although their commands request different boundaries. They do not create
per-suite thresholds: successive uploads replace this shared model. Historical
API points inspected for `window/rank/100k`, `query/ordered_groups/avg/1m`,
`udf/predicate_filter_dispatch/1k` and `checkpoint/after_1_fact/1m` have their
intended 0.99/0.999 values in each of the latest five reports. Thus a wrong
boundary was **not demonstrated** for those samples; stable isolation between
parallel suite uploads is not established. Choose a common boundary or separate
testbeds/measures and verify resulting boundaries before claiming per-suite
threshold isolation.

The v3 `.github/workflows/smoke.yml` defines ignored memory/recursion/limit/large
transaction acceptance jobs, but its `actions/checkout` has no ref override.
`origin/main`'s smoke file only has the original smoke job. Its default-branch
schedule therefore cannot supply the v3-only acceptance jobs claimed by v3's
comments. Their actual scheduled v3 execution is unverified; unit tests and
manual results cannot substitute for Bencher history (G07).

## Run evidence and public history

Read-only commands used:

```sh
gh issue view 776
git rev-parse 'v2.0.11^{commit}' origin/v3 origin/main
git merge-base v2.0.11 origin/v3
git log v2.0.11..origin/v3
git diff v2.0.11 HEAD -- src/lib.rs src/db.rs Cargo.toml
git show origin/main:.github/workflows/bench.yml
gh run list --workflow bench.yml --branch main --limit 12 --json databaseId,event,headBranch,headSha,status,conclusion,createdAt,url
gh run list --workflow bench.yml --branch v3 --limit 8 --json databaseId,event,headBranch,headSha,status,conclusion,createdAt,url
gh run view 41538175832 --json displayTitle,headSha,event,jobs,url
gh run view 41538175832 --log
gh run view 41337918695 --json displayTitle,headSha,event,jobs,url
gh run view 41337918695 --log
```

| Evidence | Run | Result verified from logs and job metadata |
| --- | --- | --- |
| R1 | [41538175832](https://github.com/project-minigraf/minigraf/actions/runs/41538175832), scheduled 2026-11-02 22:32:48 UTC, title Nightly Benchmarks (v3) | All 21 suites successful; 270 distinct `Analyzing` names, 270 matching Bencher result names, all uploads success |
| R2 | [41337918695](https://github.com/project-minigraf/minigraf/actions/runs/41337918695), scheduled 2026-11-01 18:41:40 UTC, same title | Same exact 270 executed/uploaded IDs, all 21 uploads success |

Every job's checkout `git log -1 --format=%H` returned the measured v3 hash in
the revisions table, not the workflow's main `headSha`. No benchmark difference
between that checkout and HEAD was inferred from a green badge: `git diff` of
the benchmark sources/Cargo/workflow was checked separately.

Examples from R1 (timestamps UTC, job prefix abbreviated):

```text
compile  22:32:56  git log -1 --format=%H
compile  22:32:56  29dd5888498a55b4be24db08f1250ca5571bbd44
query    22:47:07  Benchmarking query/ordered_values/index/1k: Collecting ...
window   22:36:42  Benchmarking window/running_sum/1k
query    23:04:03  View results:
query    23:04:03  - query/join_3pattern/100k (Latency): https://bencher.dev/console/projects/minigraf/perf?...
```

Neither full log contains executed or uploaded `window_rows/`, `prepared/`,
`retract/`, `btree_lookup/`, `point_query_chain_depth/` or `simd_` names. The route
appendix enumerates what each scheduled job did select and the exact omitted
names; this verifies the issue's `window_rows/` versus `^window/` observation.

Public API checks on the audit date (no credentials):

- [Project](https://api.bencher.dev/v0/projects/minigraf) returned HTTP 200,
  visibility `public`, UUID `b4d413b8-c826-4017-9ddf-40a20d282766`.
- [v3 branch](https://api.bencher.dev/v0/projects/minigraf/branches/v3) UUID
  `01a11620-4681-7f30-801b-d9ad268088ca`; current head
  `01a18921-537c-719a-9b63-5f2b0506420a` was created 2026-10-29 and has a main
  start point. [Testbed](https://api.bencher.dev/v0/projects/minigraf/testbeds/ubuntu-latest)
  UUID `4ea63737-9f02-4ea0-8058-13ed4f643b49`; latency measure
  `92472195-78d4-4ba0-ac1f-b575b6498171`.
- For **every one of the 270 uploaded IDs**, the benchmark UUID in R1's result
  URL was queried at `/v0/projects/minigraf/perf` with `branches`, `heads`,
  `testbeds`, `benchmarks`, `measures` set to the above UUIDs. Every response
  contained metrics, latest date 2026-11-02, and non-null boundaries on every
  returned point: 5–33 points per ID. Example:
  [ancestors history](https://api.bencher.dev/v0/projects/minigraf/perf?branches=01a11620-4681-7f30-801b-d9ad268088ca&heads=01a18921-537c-719a-9b63-5f2b0506420a&testbeds=4ea63737-9f02-4ea0-8058-13ed4f643b49&benchmarks=01a16ee6-0eae-7468-afee-d067fc3b9513&measures=92472195-78d4-4ba0-ac1f-b575b6498171).
  **Returned historical points can include inherited main history.** They are
  not 5–33 independent scheduled v3 runs. R1/R2 establish two genuine v3
  observations for all selected IDs; the newer ordered-group modes have five
  API points, not a filled 30-sample v3-only baseline.
- Paginated [benchmark catalogue](https://api.bencher.dev/v0/projects/minigraf/benchmarks?per_page=255&page=1)
  (pages 1–2, 293 entries) contains none of the eight omitted families. This
  checks the active returned catalogue, not deleted/archived historical data.
- [Thresholds](https://api.bencher.dev/v0/projects/minigraf/thresholds?per_page=100)
  confirms the shared threshold described above. The browser-facing project
  page returned a Cloudflare challenge to the command-line request; the API
  was usable. Dashboard rendering and delivery of notifications were not
  verified.

The following appendices preserve the derived names, run comparison, history
counts, and source function mapping so this report does not depend on retained
local log files. Temporary audit downloads are deleted at completion.

## Gaps and follow-up plan

There are **18 follow-up gap categories** below. This is a count of work areas,
not missing individual benchmark IDs or release features. Partial coverage in
a feature's other matrix rows does not close the listed subpath. EXCLUDED
error-code timing, synthetic SIMD, redundant pending lookup cases and unshipped
features do not count as gaps.

| Gap | What must be resolved | Matrix |
| --- | --- | --- |
| G01 | Route a practical bounded-row-frame selection; all 128 registered IDs are currently omitted | C28 |
| G02 | Route prepared execute and add representative preparation, prepared cursor and new slot/bind costs | C25/C34 |
| G03 | Add final `:distinct` performance cases, including duplicates and composition | C26 |
| G04 | Add offset, descending/general top-k and compound/tie/fallback cases; current fixed limit 10 is narrow | C25 |
| G05 | Add inline/long set writes/reads, membership and collect-set | C29 |
| G06 | Add lag/lead, including partition scaling | C27 |
| G07 | Enforce/observe v3 memory scaling and metering, and correctly schedule v3 acceptance jobs | C24 |
| G08 | Add committed/pending operator controls: value/hash joins, anti/union forms, timestamp bounds, recursive and window partition shape | C15–C17/C19/C23 |
| G09 | Add cursor lifecycle, snapshot pin/churn and staged transaction read costs | C13/C30 |
| G10 | Add transactions around the chunk boundary, chunked retract, multiple statements and rollback paths | C10/C30 |
| G11 | Add FactLog orders/filters/window scaling and LogWriter append/finish | C31/C32 |
| G12 | Add migration, verify/rebuild, long/multivalue storage and read-only open/WAL costs | C04/C07/C08/C11 |
| G13 | Route representative committed-history and retraction cases; assess duplicate pending lookup groups explicitly | C09/C10/C41 |
| G14 | Add #180 fallible registry/callback/re-entry costs beyond baseline infallible dispatch | C33 |
| G15 | Add binding conversion/callback/cursor/persistence coverage, starting with Tier 1 Python and browser | C36 |
| G16 | Add ordinary hash avg, mixed/exact sums/extrema, skewed groups and effective tuple/proof fallbacks | C20/C22 |
| G17 | Add literal/dynamic keyword equality cache controls (#774), including nested/prepared use | C35 |
| G18 | Resolve shared threshold isolation, assert ID selection, retain measured-checkout provenance and validate reporting end to end | Workflow/reporting audit |

Suggested PR sequence below deliberately separates code additions from shared
workflow changes. All implementation/benchmark PRs target **v3**. Every shared
workflow change must also land on **main in a separate companion PR** per
CLAUDE.md, since main owns schedules. V3-only benchmarks must not be copied to
v2 just to make a main workflow route work: the workflow checks out v3 before
running them. Gate v3-only suites by the selected ref to avoid empty main jobs.

**Runtime estimates are unverified planning allowances**, not measured results.
No new timed runs were permitted. Existing Criterion setup uses mostly ten
samples and the usual 3-second warmup/5-second target (visible in R1); small
cases have roughly an eight-second sampling floor, but fixture construction,
fsyncs, long iterations, repeated benchmark setup and job setup can dominate.
R1's query job was 29m39s, checkpoint 41m13s and compile 1m44s. Do not assume a
new case costs eight seconds or that parallel jobs have no billable runtime.
Measure shortlisted cases on idle hardware later, cap case counts, and revise
these allowances before enabling the routes.

| PR | Small reviewable scope / files | Gaps | Provisional incremental nightly allowance and validation |
| --- | --- | --- | --- |
| P01 | Route a shortlist of `window_rows/` in `.github/workflows/bench.yml`; keep IDs unchanged. Cover built-in zero/two versus prefix, one/100 partitions, 10k/100k, and a bounded UDF plus full-prefix UDF control. Document remaining manual sweep | G01 | About 12–18 cases, reserve 5–15 job-minutes; full 128-case expansion has >17 minutes of sampling targets alone and expensive replay cases may take much longer. Workflow main companion required; inspect dispatch names/uploads |
| P02 | Add prepared routing, then preparation/cursor/new count/time/entity slot cases in `benches/minigraf_bench.rs`; `.github/workflows/bench.yml` v3 gate + main companion | G02 | Existing four cases: reserve 2–5 minutes; 4–8 new cases another 2–5 minutes. Verify prepared versus unprepared controls and fresh v3 history |
| P03 | Add `:distinct` cases in `benches/minigraf_bench.rs`, helper duplicate fixture if needed; use a covered prefix or explicit route | G03 | 4–8 cases at 10k/100k, reserve 2–5 minutes; bag/control and set/float tuple variants |
| P04 | Add general sorting/pagination controls: offset 0/large, descending k=10/1000, committed/pending and complete tie order in benchmark/helpers files | G04 | 6–10 cases at 10k/100k, reserve 3–8 minutes; keep 10m envelope manual. Include no-Sort versus retained-Sort controls |
| P05 | Set storage benchmarks: inline/long, repeated/unique sets, ingest/checkpoint/exact lookup/scan; `benches/minigraf_bench.rs`, `benches/helpers/mod.rs` | G05 (storage) | 6–10 cases, reserve 3–10 minutes; avoid 100k×large cardinality by default. Validate runtime, files and scalar controls before routing |
| P06 | Membership/collect-set query cases and lag/lead cases, split into two PRs if fixture changes grow; same bench/helper files | G05 (queries), G06 | 6–10 set-query + 4–6 neighbor cases, reserve 4–10 minutes combined; compare one large versus many partitions and pending/committed probes |
| P07 | Operator fixtures: paired value/hash/entity joins, forced-body controls only where a stable test-hook build is needed, timestamp/irregular as-of, committed recursion; benchmark/helpers files | G08 | 8–12 representative cases, reserve 5–15 minutes. Keep deep nonlinear recursion in manual/acceptance suites; verify strategy using existing counters/test controls, not names alone |
| P08 | Cursor open/first-batch/drain/close with held snapshots across checkpoint; staged transaction reads, same source files | G09 | 6–8 cases, reserve 3–8 minutes. Separate query timing from setup/teardown/checkpoint timing and verify bounded batches |
| P09 | Chunked execute/retract/write-transaction benchmarks at just below/above 65,536; statement rollback control, custom small chunk only in a separately routed test-hooks target if justified | G10 | 4–8 cases, reserve 5–20 minutes; large writes/fsync cost unmeasured. No 1m statement in every latency iteration until measured; acceptance covers large bound separately |
| P10 | FactLog order/filter/window and LogWriter append/finish fixtures preserving temporal records; bench/helper files | G11 | 6–10 cases at 10k/100k, reserve 5–15 minutes; verify record/tx counts outside timing and separate append from finish |
| P11 | Route `point_query_chain_depth/` and useful `retract/` subset; leave `btree_lookup/` duplicate pending groups excluded with explanation. `.github/workflows/bench.yml` + main companion | G13 | About 6–10 cases, reserve 3–10 minutes. Beware history fixtures built outside filtered closures even on unselected jobs; make setup lazy in a separate benchmark-only change |
| P12 | v7 migration + verify/rebuild + long/multivalue/read-only-WAL storage cases; bench/helper files, use existing frozen v7 fixtures | G12 | Split migration/repair from storage values if >8–10 cases; reserve 5–15 minutes per split. Copy/prepare fixtures outside timed routine, validate migrated facts/verify result |
| P13 | Fallible successful UDF vs infallible control, registration/registry snapshot churn, callback re-entry representative; keyword literal/dynamic/prepared controls; bench/helper files, split by concern | G14/G17 | Two small PRs, each 4–6 cases, reserve 2–5 minutes each; deliberate callback errors stay correctness tests unless latency is a product concern |
| P14 | Hash avg/exact floats/mixed extrema plus skewed ordered group and effective-tuple fallback controls; bench/helper files | G16 | 6–10 cases, reserve 3–8 minutes; keep exhaustive numeric permutations in tests, not nightly timing |
| P15 | Route v3 memory acceptance from default-main schedule; add a small explicit metering-on/off latency pair and a separate RSS/state metric or fixed-bound gate. `.github/workflows/smoke.yml`, `bench.yml`, existing acceptance tests/benchmark files only as needed | G07 | Acceptance jobs already declare 30/45-minute budgets; actual added schedule runtime **unverified**. Reserve 2–5 minutes for 4 small latency cases. Main companion mandatory; record actual v3 checkout and executed test names, not v3 comments |
| P16 | Binding PRs in Python first, browser second: prepared execute/cursor batches/set conversion and foreign callbacks, then Node/C; Java/Android/Swift targeted follow-ups or documented hardware exclusions | G15 | Reserve 5–15 minutes per initial binding job plus build/runtime setup (unverified). Reuse Python envelope where useful; add serializable metrics and stable dedicated testbeds, do not pretend native Criterion times cover FFI. Device-only timing can remain deliberate manual coverage with release sign-off |
| P17 | Workflow selection/reporting hardening: compare enumerated IDs with expected filters, fail empty required routes, report executed counts and checkout hash, resolve threshold tuple scope, inspect successful/failed upload handling | G18 | Negligible sampling cost; enumeration/setup overhead unverified. May need two small PRs (selection/provenance and thresholds/alerts), each with a main companion. Validate via later v3 dispatch and inspect Bencher point boundaries and reporting summaries |

No dispatch, new benchmark, envelope run or alert injection was performed in
phase 1. Before closing #776, run the accepted changes later on idle hardware,
dispatch from the correct default workflow with v3 selected, compare actual IDs
to the intended set, confirm uploads and meaningful branch/testbed history, and
update this document for additions through the release cut. An omitted heavy
case needs a specific manual/envelope exclusion, not an assumed nightly route.

## Appendix A: every scheduled filter and its selected IDs
All IDs in this appendix are **derived statically, not listed**, then matched
against execution and upload records in **both R1 and R2**. All 21 filters are
nonempty; their sets are disjoint. Every selected ID has public history B.
`P` is the range of returned public metric-point counts per ID (including
inherited history). Job elapsed time is from R1 job metadata, including setup,
benchmarking and upload; it is not a new measurement.
| Job (`bench / …`) | Filter | IDs | Requested upper boundary | Public points P | R1 job elapsed |
| --- | --- | ---: | --- | --- | --- |
| insert | `^insert/` | 12 | 0.99 | 33–33 | 10m 49s |
| insert_file | `^insert_file/` | 12 | 0.999 | 33–33 | 5m 37s |
| query | `^query/` | 97 | 0.99 | 5–33 | 29m 39s |
| time_travel | `^time_travel/` | 8 | 0.99 | 33–33 | 9m 44s |
| recursion | `^recursion/` | 4 | 0.99 | 5–33 | 3m 29s |
| open | `^open/` | 7 | 0.999 | 33–33 | 4m 57s |
| checkpoint | `^checkpoint` | 9 | 0.999 | 5–33 | 41m 13s |
| concurrent/readers | `^concurrent/readers/` | 6 | 0.999 | 33–33 | 7m 05s |
| concurrent/readers_plus_writer | `^concurrent/readers_plus_writer/` | 6 | 0.999 | 33–33 | 4m 37s |
| concurrent/serialized_writers | `^concurrent/serialized_writers/` | 8 | 0.999 | 33–33 | 5m 00s |
| concurrent_file/readers | `^concurrent_file/readers/` | 6 | 0.999 | 33–33 | 4m 03s |
| concurrent_file/readers_plus_writer | `^concurrent_file/readers_plus_writer/` | 6 | 0.999 | 33–33 | 4m 15s |
| concurrent_file/serialized_writers | `^concurrent_file/serialized_writers/` | 8 | 0.999 | 33–33 | 4m 01s |
| concurrent_btree_scan | `^concurrent_btree_scan` | 6 | 0.999 | 33–33 | 3m 17s |
| negation | `^negation/` | 16 | 0.99 | 33–33 | 7m 37s |
| disjunction | `^disjunction/` | 15 | 0.99 | 33–33 | 4m 51s |
| aggregation | `^aggregation/` | 10 | 0.99 | 33–33 | 6m 13s |
| expr | `^expr/` | 9 | 0.99 | 33–33 | 4m 55s |
| window | `^window/` | 12 | 0.99 | 5–33 | 5m 31s |
| temporal_metadata | `^temporal_metadata/` | 9 | 0.99 | 33–33 | 5m 42s |
| udf | `^udf/` | 4 | 0.99 | 33–33 | 3m 38s |

### `insert` — `^insert/`

Output file: `bench_insert.txt`. R1/R2 + B, requested 0.99.

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

### `insert_file` — `^insert_file/`

Output file: `bench_insert_file.txt`. R1/R2 + B, requested 0.999.

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
```

### `query` — `^query/`

Output file: `bench_query.txt`. R1/R2 + B, requested 0.99.

```text
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
query/regex_filter/1k
query/regex_filter/10k
query/predicate_pushdown/1k
query/predicate_pushdown/10k
query/predicate_pushdown/100k
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
query/ordered_values/index/1k
query/ordered_values/uuid_ties/1k
query/ordered_values/index/10k
query/ordered_values/uuid_ties/10k
query/ordered_values/index/100k
query/ordered_values/uuid_ties/100k
query/ordered_values/index/1m
query/ordered_values/uuid_ties/1m
```

### `time_travel` — `^time_travel/`

Output file: `bench_time_travel.txt`. R1/R2 + B, requested 0.99.

```text
time_travel/as_of_counter/1k
time_travel/as_of_counter/10k
time_travel/as_of_counter/100k
time_travel/as_of_counter/1m
time_travel/valid_at/1k
time_travel/valid_at/10k
time_travel/valid_at/100k
time_travel/valid_at/1m
```

### `recursion` — `^recursion/`

Output file: `bench_recursion.txt`. R1/R2 + B, requested 0.99.

```text
recursion/chain/depth_10
recursion/chain/depth_100
recursion/fanout/w10_d3
recursion/ancestors/w10_d4
```

### `open` — `^open/`

Output file: `bench_open.txt`. R1/R2 + B, requested 0.999.

```text
open/checkpointed/1k
open/checkpointed/10k
open/checkpointed/100k
open/checkpointed/1m
open/wal_replay/1k
open/wal_replay/10k
open/wal_replay/100k
```

### `checkpoint` — `^checkpoint`

Output file: `bench_checkpoint.txt`. R1/R2 + B, requested 0.999.

```text
checkpoint/1k
checkpoint/10k
checkpoint/100k
checkpoint/1m
checkpoint/after_1_fact/10k
checkpoint/after_1_fact/100k
checkpoint/after_1_fact/1m
checkpoint/after_100k_facts/10k
checkpoint/after_100k_facts/100k
```

### `concurrent/readers` — `^concurrent/readers/`

Output file: `bench_concurrent_readers.txt`. R1/R2 + B, requested 0.999.

```text
concurrent/readers/10k_4t
concurrent/readers/10k_8t
concurrent/readers/10k_16t
concurrent/readers/100k_4t
concurrent/readers/100k_8t
concurrent/readers/100k_16t
```

### `concurrent/readers_plus_writer` — `^concurrent/readers_plus_writer/`

Output file: `bench_concurrent_mixed.txt`. R1/R2 + B, requested 0.999.

```text
concurrent/readers_plus_writer/10k_4t
concurrent/readers_plus_writer/10k_8t
concurrent/readers_plus_writer/10k_16t
concurrent/readers_plus_writer/100k_4t
concurrent/readers_plus_writer/100k_8t
concurrent/readers_plus_writer/100k_16t
```

### `concurrent/serialized_writers` — `^concurrent/serialized_writers/`

Output file: `bench_concurrent_writers.txt`. R1/R2 + B, requested 0.999.

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

### `concurrent_file/readers` — `^concurrent_file/readers/`

Output file: `bench_concurrent_file_readers.txt`. R1/R2 + B, requested 0.999.

```text
concurrent_file/readers/10k_4t
concurrent_file/readers/10k_8t
concurrent_file/readers/10k_16t
concurrent_file/readers/100k_4t
concurrent_file/readers/100k_8t
concurrent_file/readers/100k_16t
```

### `concurrent_file/readers_plus_writer` — `^concurrent_file/readers_plus_writer/`

Output file: `bench_concurrent_file_mixed.txt`. R1/R2 + B, requested 0.999.

```text
concurrent_file/readers_plus_writer/10k_4t
concurrent_file/readers_plus_writer/10k_8t
concurrent_file/readers_plus_writer/10k_16t
concurrent_file/readers_plus_writer/100k_4t
concurrent_file/readers_plus_writer/100k_8t
concurrent_file/readers_plus_writer/100k_16t
```

### `concurrent_file/serialized_writers` — `^concurrent_file/serialized_writers/`

Output file: `bench_concurrent_file_writers.txt`. R1/R2 + B, requested 0.999.

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

### `concurrent_btree_scan` — `^concurrent_btree_scan`

Output file: `bench_concurrent_btree_scan.txt`. R1/R2 + B, requested 0.999.

```text
concurrent_btree_scan/10k_2t
concurrent_btree_scan/10k_4t
concurrent_btree_scan/10k_8t
concurrent_btree_scan/100k_2t
concurrent_btree_scan/100k_4t
concurrent_btree_scan/100k_8t
```

### `negation` — `^negation/`

Output file: `bench_negation.txt`. R1/R2 + B, requested 0.99.

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

### `disjunction` — `^disjunction/`

Output file: `bench_disjunction.txt`. R1/R2 + B, requested 0.99.

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

### `aggregation` — `^aggregation/`

Output file: `bench_aggregation.txt`. R1/R2 + B, requested 0.99.

```text
aggregation/count_scale/1k
aggregation/count_scale/10k
aggregation/grouped_count_scale/1k
aggregation/grouped_count_scale/10k
aggregation/sum_scale/1k
aggregation/sum_scale/10k
aggregation/with_grouped_sum/1k
aggregation/with_grouped_sum/10k
aggregation/count_distinct_scale/1k
aggregation/count_distinct_scale/10k
```

### `expr` — `^expr/`

Output file: `bench_expr.txt`. R1/R2 + B, requested 0.99.

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
```

### `window` — `^window/`

Output file: `bench_window.txt`. R1/R2 + B, requested 0.99.

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
```

### `temporal_metadata` — `^temporal_metadata/`

Output file: `bench_temporal_metadata.txt`. R1/R2 + B, requested 0.99.

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

### `udf` — `^udf/`

Output file: `bench_udf.txt`. R1/R2 + B, requested 0.99.

```text
udf/aggregate_sum_dispatch/1k
udf/aggregate_sum_dispatch/10k
udf/predicate_filter_dispatch/1k
udf/predicate_filter_dispatch/10k
```

## Appendix B: registered IDs selected by no scheduled job
**Derived statically, not listed.** These 188 IDs belong to 14 Criterion group
names in eight top-level families. None occurs in R1/R2 executed/uploaded sets
or the returned public Bencher benchmark catalogue. SIMD kernels (30 IDs) are
deliberately EXCLUDED from v3 runtime-feature requirements; the remaining
158 unrouted IDs require routing decisions. `^window/` does select the four
`window/…` groups in Appendix A; it does not match the underscore in
`window_rows/`. `^query/` similarly does not match `prepared/`,
`btree_lookup/` or `point_query_chain_depth/`.

### `prepared/value_lookup` (2 IDs)

Source: `bench_prepared`, [minigraf_bench.rs:1781](../benches/minigraf_bench.rs#L1781).

```text
prepared/value_lookup/1k
prepared/value_lookup/10k
```

### `prepared/threshold_filter` (2 IDs)

Source: `bench_prepared`, [minigraf_bench.rs:1781](../benches/minigraf_bench.rs#L1781).

```text
prepared/threshold_filter/1k
prepared/threshold_filter/10k
```

### `retract/single_fact` (4 IDs)

Source: `bench_retract`, [minigraf_bench.rs:1836](../benches/minigraf_bench.rs#L1836).

```text
retract/single_fact/1k
retract/single_fact/10k
retract/single_fact/100k
retract/single_fact/1m
```

### `retract/batch_100` (4 IDs)

Source: `bench_retract`, [minigraf_bench.rs:1836](../benches/minigraf_bench.rs#L1836).

```text
retract/batch_100/1k
retract/batch_100/10k
retract/batch_100/100k
retract/batch_100/1m
```

### `btree_lookup/entity_point` (3 IDs)

Source: `bench_btree_lookup`, [minigraf_bench.rs:1890](../benches/minigraf_bench.rs#L1890).

```text
btree_lookup/entity_point/1k
btree_lookup/entity_point/10k
btree_lookup/entity_point/100k
```

### `btree_lookup/attribute_scan` (3 IDs)

Source: `bench_btree_lookup`, [minigraf_bench.rs:1890](../benches/minigraf_bench.rs#L1890).

```text
btree_lookup/attribute_scan/1k
btree_lookup/attribute_scan/10k
btree_lookup/attribute_scan/100k
```

### `window_rows` (128 IDs)

Source: `bench_window_rows`, [minigraf_bench.rs:1536](../benches/minigraf_bench.rs#L1536).

```text
window_rows/10000/one/sum/default
window_rows/10000/one/sum/zero
window_rows/10000/one/sum/two
window_rows/10000/one/sum/sqrt
window_rows/10000/one/sum/half
window_rows/10000/one/sum/prefix
window_rows/10000/one/count/default
window_rows/10000/one/count/zero
window_rows/10000/one/count/two
window_rows/10000/one/count/sqrt
window_rows/10000/one/count/half
window_rows/10000/one/count/prefix
window_rows/10000/one/min/default
window_rows/10000/one/min/zero
window_rows/10000/one/min/two
window_rows/10000/one/min/sqrt
window_rows/10000/one/min/half
window_rows/10000/one/min/prefix
window_rows/10000/one/max/default
window_rows/10000/one/max/zero
window_rows/10000/one/max/two
window_rows/10000/one/max/sqrt
window_rows/10000/one/max/half
window_rows/10000/one/max/prefix
window_rows/10000/one/avg/default
window_rows/10000/one/avg/zero
window_rows/10000/one/avg/two
window_rows/10000/one/avg/sqrt
window_rows/10000/one/avg/half
window_rows/10000/one/avg/prefix
window_rows/10000/one/frame_bench_sum/zero
window_rows/10000/one/frame_bench_sum/two
window_rows/10000/one/frame_bench_sum/half
window_rows/10000/hundred/sum/default
window_rows/10000/hundred/sum/zero
window_rows/10000/hundred/sum/two
window_rows/10000/hundred/sum/sqrt
window_rows/10000/hundred/sum/half
window_rows/10000/hundred/sum/prefix
window_rows/10000/hundred/count/default
window_rows/10000/hundred/count/zero
window_rows/10000/hundred/count/two
window_rows/10000/hundred/count/sqrt
window_rows/10000/hundred/count/half
window_rows/10000/hundred/count/prefix
window_rows/10000/hundred/min/default
window_rows/10000/hundred/min/zero
window_rows/10000/hundred/min/two
window_rows/10000/hundred/min/sqrt
window_rows/10000/hundred/min/half
window_rows/10000/hundred/min/prefix
window_rows/10000/hundred/max/default
window_rows/10000/hundred/max/zero
window_rows/10000/hundred/max/two
window_rows/10000/hundred/max/sqrt
window_rows/10000/hundred/max/half
window_rows/10000/hundred/max/prefix
window_rows/10000/hundred/avg/default
window_rows/10000/hundred/avg/zero
window_rows/10000/hundred/avg/two
window_rows/10000/hundred/avg/sqrt
window_rows/10000/hundred/avg/half
window_rows/10000/hundred/avg/prefix
window_rows/10000/hundred/frame_bench_sum/zero
window_rows/10000/hundred/frame_bench_sum/two
window_rows/10000/hundred/frame_bench_sum/half
window_rows/100000/one/sum/default
window_rows/100000/one/sum/zero
window_rows/100000/one/sum/two
window_rows/100000/one/sum/sqrt
window_rows/100000/one/sum/half
window_rows/100000/one/sum/prefix
window_rows/100000/one/count/default
window_rows/100000/one/count/zero
window_rows/100000/one/count/two
window_rows/100000/one/count/sqrt
window_rows/100000/one/count/half
window_rows/100000/one/count/prefix
window_rows/100000/one/min/default
window_rows/100000/one/min/zero
window_rows/100000/one/min/two
window_rows/100000/one/min/sqrt
window_rows/100000/one/min/half
window_rows/100000/one/min/prefix
window_rows/100000/one/max/default
window_rows/100000/one/max/zero
window_rows/100000/one/max/two
window_rows/100000/one/max/sqrt
window_rows/100000/one/max/half
window_rows/100000/one/max/prefix
window_rows/100000/one/avg/default
window_rows/100000/one/avg/zero
window_rows/100000/one/avg/two
window_rows/100000/one/avg/sqrt
window_rows/100000/one/avg/half
window_rows/100000/one/avg/prefix
window_rows/100000/one/frame_bench_sum/prefix
window_rows/100000/hundred/sum/default
window_rows/100000/hundred/sum/zero
window_rows/100000/hundred/sum/two
window_rows/100000/hundred/sum/sqrt
window_rows/100000/hundred/sum/half
window_rows/100000/hundred/sum/prefix
window_rows/100000/hundred/count/default
window_rows/100000/hundred/count/zero
window_rows/100000/hundred/count/two
window_rows/100000/hundred/count/sqrt
window_rows/100000/hundred/count/half
window_rows/100000/hundred/count/prefix
window_rows/100000/hundred/min/default
window_rows/100000/hundred/min/zero
window_rows/100000/hundred/min/two
window_rows/100000/hundred/min/sqrt
window_rows/100000/hundred/min/half
window_rows/100000/hundred/min/prefix
window_rows/100000/hundred/max/default
window_rows/100000/hundred/max/zero
window_rows/100000/hundred/max/two
window_rows/100000/hundred/max/sqrt
window_rows/100000/hundred/max/half
window_rows/100000/hundred/max/prefix
window_rows/100000/hundred/avg/default
window_rows/100000/hundred/avg/zero
window_rows/100000/hundred/avg/two
window_rows/100000/hundred/avg/sqrt
window_rows/100000/hundred/avg/half
window_rows/100000/hundred/avg/prefix
window_rows/100000/hundred/frame_bench_sum/prefix
```

### `point_query_chain_depth/churned_attr` (3 IDs)

Source: `bench_point_query_chain_depth`, [minigraf_bench.rs:1931](../benches/minigraf_bench.rs#L1931).

```text
point_query_chain_depth/churned_attr/1
point_query_chain_depth/churned_attr/500
point_query_chain_depth/churned_attr/2000
```

### `point_query_chain_depth/sibling_attr` (3 IDs)

Source: `bench_point_query_chain_depth`, [minigraf_bench.rs:1931](../benches/minigraf_bench.rs#L1931).

```text
point_query_chain_depth/sibling_attr/1
point_query_chain_depth/sibling_attr/500
point_query_chain_depth/sibling_attr/2000
```

### `point_query_chain_depth/attr_scan` (3 IDs)

Source: `bench_point_query_chain_depth`, [minigraf_bench.rs:1931](../benches/minigraf_bench.rs#L1931).

```text
point_query_chain_depth/attr_scan/1
point_query_chain_depth/attr_scan/500
point_query_chain_depth/attr_scan/2000
```

### `point_query_chain_depth/churned_attr_as_of` (3 IDs)

Source: `bench_point_query_chain_depth`, [minigraf_bench.rs:1931](../benches/minigraf_bench.rs#L1931).

```text
point_query_chain_depth/churned_attr_as_of/1
point_query_chain_depth/churned_attr_as_of/500
point_query_chain_depth/churned_attr_as_of/2000
```

### `simd_temporal` (10 IDs)

Source: `bench_simd`, [minigraf_bench.rs:2020](../benches/minigraf_bench.rs#L2020).

```text
simd_temporal/scalar/100
simd_temporal/simd/100
simd_temporal/scalar/1k
simd_temporal/simd/1k
simd_temporal/scalar/10k
simd_temporal/simd/10k
simd_temporal/scalar/100k
simd_temporal/simd/100k
simd_temporal/scalar/1m
simd_temporal/simd/1m
```

### `simd_as_of` (10 IDs)

Source: `bench_simd`, [minigraf_bench.rs:2020](../benches/minigraf_bench.rs#L2020).

```text
simd_as_of/scalar/100
simd_as_of/simd/100
simd_as_of/scalar/1k
simd_as_of/simd/1k
simd_as_of/scalar/10k
simd_as_of/simd/10k
simd_as_of/scalar/100k
simd_as_of/simd/100k
simd_as_of/scalar/1m
simd_as_of/simd/1m
```

### `simd_aggregate` (10 IDs)

Source: `bench_simd`, [minigraf_bench.rs:2020](../benches/minigraf_bench.rs#L2020).

```text
simd_aggregate/scalar/100
simd_aggregate/simd/100
simd_aggregate/scalar/1k
simd_aggregate/simd/1k
simd_aggregate/scalar/10k
simd_aggregate/simd/10k
simd_aggregate/scalar/100k
simd_aggregate/simd/100k
simd_aggregate/scalar/1m
simd_aggregate/simd/1m
```

## Appendix C: source registration and fixture map
All functions below appear in `criterion_group!(benches, …)`; names are
Criterion group names, not inferred aliases from comments. Fixtures are built
in `benches/helpers/mod.rs` unless the benchmark body does so directly.
| Registered function / definition | Group names | Cases | Fixture note |
| --- | --- | ---: | --- |
| [`bench_insert`](../benches/minigraf_bench.rs#L29) | `insert/single_fact`, `insert/batch_100`, `insert/explicit_tx` | 12 | M4; single/100/explicit transaction, repeated benchmark triples |
| [`bench_insert_file`](../benches/minigraf_bench.rs#L97) | `insert_file/single_fact`, `insert_file/batch_100`, `insert_file/explicit_tx` | 12 | F4 + pending WAL, checkpoint suppressed |
| [`bench_query`](../benches/minigraf_bench.rs#L176) | `query/point_entity`, `query/point_attribute`, `query/join_3pattern` | 12 | M4; join fixture adds next edges |
| [`bench_ordered_groups`](../benches/minigraf_bench.rs#L237) | `query/ordered_groups` | 56 | F4 read-only; unique integer groups, full drain/limit 10, 14 modes |
| [`bench_partition_windows`](../benches/minigraf_bench.rs#L297) | `query/partition_windows` | 16 | F4 read-only; one-row value/entity partitions, full drain/limit 10 |
| [`bench_input_order`](../benches/minigraf_bench.rs#L344) | `query/ordered_values` | 8 | F4 read-only, fixed limit 10; index and UUID-tie control |
| [`bench_time_travel`](../benches/minigraf_bench.rs#L387) | `time_travel/as_of_counter`, `time_travel/valid_at` | 8 | M4 point query; no timestamp as-of shape |
| [`bench_recursion`](../benches/minigraf_bench.rs#L436) | `recursion/chain`, `recursion/fanout`, `recursion/ancestors` | 4 | M, chain depth 10/100; fanout w10_d3; ancestors w10_d4 |
| [`bench_open`](../benches/minigraf_bench.rs#L498) | `open/checkpointed`, `open/wal_replay` | 7 | F4 read-write checkpointed; W first 3 sizes, open/drop |
| [`bench_checkpoint`](../benches/minigraf_bench.rs#L558) | `checkpoint` | 4 | W at all four scales; full initial flush |
| [`bench_checkpoint_after_1_fact`](../benches/minigraf_bench.rs#L595) | `checkpoint/after_1_fact` | 3 | F 10k/100k/1m + one fact per iteration; growing history |
| [`bench_checkpoint_after_100k_facts`](../benches/minigraf_bench.rs#L631) | `checkpoint/after_100k_facts` | 2 | F 10k/100k + 100k more per iteration, inserted as 100 x 1k |
| [`bench_concurrent`](../benches/minigraf_bench.rs#L667) | `concurrent/readers`, `concurrent/readers_plus_writer`, `concurrent/serialized_writers` | 20 | M 10k/100k; readers/mixed 4/8/16 threads; writers 2/4/8/16 |
| [`bench_concurrent_file`](../benches/minigraf_bench.rs#L826) | `concurrent_file/readers`, `concurrent_file/readers_plus_writer`, `concurrent_file/serialized_writers` | 20 | CF 10k/100k; same thread counts |
| [`bench_negation`](../benches/minigraf_bench.rs#L992) | `negation/not_scale`, `negation/not_join_scale`, `negation/not_selectivity`, `negation/not_pushdown_selectivity`, `negation/not_rule_body` | 16 | M2 for scale/rule cases; 10k 0/25/50/75/100% exclusions; extra attributes |
| [`bench_concurrent_btree_scan`](../benches/minigraf_bench.rs#L1123) | `concurrent_btree_scan` | 6 | F 10k/100k; 2/4/8 threads, entity point read |
| [`bench_disjunction`](../benches/minigraf_bench.rs#L1179) | `disjunction/or_scale`, `disjunction/or_join_scale`, `disjunction/or_selectivity`, `disjunction/or_short_circuit`, `disjunction/or_join_short_circuit`, `disjunction/or_rule_body` | 15 | M2 scale/rule/short-circuit; 10k 0/25/50/75/100% matches; extra tags |
| [`bench_aggregation`](../benches/minigraf_bench.rs#L1322) | `aggregation/count_scale`, `aggregation/grouped_count_scale`, `aggregation/sum_scale`, `aggregation/with_grouped_sum` | 8 | M2; integer/count, 10 department keys, :with |
| [`bench_expr`](../benches/minigraf_bench.rs#L1398) | `expr/filter_scale`, `expr/binding_scale`, `expr/binding_into_agg` | 9 | M3 numeric filter/arithmetic/sum |
| [`bench_window`](../benches/minigraf_bench.rs#L1461) | `window/running_sum`, `window/rank`, `window/many_partitions`, `window/row_number` | 12 | M3 unpartitioned sum/rank/row-number or singleton value partitions |
| [`bench_window_rows`](../benches/minigraf_bench.rs#L1536) | `window_rows` | 128 | M 10k/100k rows, 2n facts; one/100 partitions; frame widths default/0/2/sqrt/half/prefix; UDF subset |
| [`bench_temporal_metadata`](../benches/minigraf_bench.rs#L1613) | `temporal_metadata/tx_time`, `temporal_metadata/valid_from`, `temporal_metadata/valid_to` | 9 | M3; tx-count (group named tx_time), valid-from/to |
| [`bench_udf`](../benches/minigraf_bench.rs#L1667) | `udf/aggregate_sum_dispatch`, `udf/predicate_filter_dispatch` | 4 | M2 integers; infallible predicate/aggregate, registration outside timing |
| [`bench_aggregation_extras`](../benches/minigraf_bench.rs#L1726) | `aggregation/count_distinct_scale` | 2 | M2 with 50% duplicate values, count-distinct |
| [`bench_query_extras`](../benches/minigraf_bench.rs#L1749) | `query/regex_filter` | 2 | M2 string values and regex predicate |
| [`bench_prepared`](../benches/minigraf_bench.rs#L1781) | `prepared/value_lookup`, `prepared/threshold_filter` | 4 | M2 integer value/threshold slots; prepare outside timing |
| [`bench_retract`](../benches/minigraf_bench.rs#L1836) | `retract/single_fact`, `retract/batch_100` | 8 | M4 repeated single/100 retractions |
| [`bench_btree_lookup`](../benches/minigraf_bench.rs#L1890) | `btree_lookup/entity_point`, `btree_lookup/attribute_scan` | 6 | M3 names+values (pending, despite btree name) |
| [`bench_point_query_chain_depth`](../benches/minigraf_bench.rs#L1931) | `point_query_chain_depth/churned_attr`, `point_query_chain_depth/sibling_attr`, `point_query_chain_depth/attr_scan`, `point_query_chain_depth/churned_attr_as_of` | 12 | F, 2k filler facts + sibling + hot history, depth 1/500/2000 |
| [`bench_predicate_pushdown`](../benches/minigraf_bench.rs#L1981) | `query/predicate_pushdown` | 3 | M3 names+values; upper 10% pass numeric filter |
| [`bench_simd`](../benches/minigraf_bench.rs#L2020) | `simd_temporal`, `simd_as_of`, `simd_aggregate` | 30 | Synthetic Vec slices, 100/1k/10k/100k/1m; scalar and SIMD kernels |
