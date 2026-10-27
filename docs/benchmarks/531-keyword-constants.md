# Keyword pattern constants after the streaming engine (#531)

Source audited: [`v3` at `c89d648f5`](https://github.com/project-minigraf/minigraf/tree/c89d648f5863db43ac6dfd394c78f0afe6ac6745/src/query) (2026-10-27). The original
`PatternMatcher::match_component` loop is absent from `src/` after #551.
Keyword entity and value constants in patterns resolve to UUIDs when their
operator plans are built. This supersedes #531's original per-candidate
pattern-constant hashing path; it does not eliminate all keyword hashing.

## Source audit

Line references below describe the measured revision, not later releases.

| Call site | When conversion runs |
|---|---|
| `engine/plan.rs:1530,1549` (`entity_const`, `value_const`) | When building a pattern's entity/value terms. A keyword value constant stores both its spelling and resolved UUID. |
| `engine/mod.rs:220`; `datalog/magic_sets.rs:27,88` | While opening a rule query or rewriting its invocation arguments into magic seeds. |
| `engine/recursive.rs:233` | A constant rule-head entity is resolved at the first emitted row and cached in `konst` for that `heads()` body run. Each full/delta run can resolve it again; it is not a per-candidate-fact operation. |
| `engine/ids.rs:232` (`keyword_eid`) | Normalizing a stored keyword id to the entity it names. Entries are query-long only when both the keyword id and entity id are provisional (`ids.rs:236–237`). Otherwise `end_pull` clears them (line 76). Ordinary stream pulls call it after each pull; window output streams defer that call until stream end (`engine/mod.rs:275,368–373`). `release` clears both maps (`ids.rs:82`). |
| `engine/plan.rs:92` (`names`), called through `ValueConst::matches` | UUID constants compared with stored keyword values. Committed `KeyTest::Uuid` memoizes by keyword id (`scan.rs:161–167`); pending facts (`scan.rs:571`) and derived cells (`scan.rs:685`) can convert each candidate's keyword. These are data conversions, not hashing a fixed keyword pattern constant. |
| `datalog/executor.rs:630` (`expr_eq`) | Keyword/ref expression equality evaluates once per row and can rehash a keyword literal. This separate expression path remains; see the follow-up below. |
| `datalog/parser.rs:2158` | Parsing: compare entity terms while assigning pseudo-attribute sources. |
| `datalog/parser/statement.rs:454,472`; `datalog/executor.rs:238,274` | Entity conversion for transact/retract statements, not query fact matching. |
| Other calls in `matcher.rs`, `magic_sets.rs`, `executor.rs` | Tests only. `matcher.rs` retains `edn_to_entity_id` and `edn_to_value`; the module name alone does not imply the old matcher executes. |

References are relative to `src/query/`. Plain queries enter
`DatalogExecutor::open_query` (`datalog/executor.rs:315`) and always use
`engine::plan`/`engine::open`. Rule queries enter `open_rule_query` (line 354),
use `plan_rules` (`engine/plan.rs:670`) and execute on the same engine.
`rule_plan` (line 782) translates body rule calls to patterns before planning
full/delta runs. No legacy evaluator matches rule-body facts.

First scans (`engine/plan.rs:1060`) and later joins (line 1446) use the same
constant resolvers. `Builder::sub` (line 1303) plans both hashed and correlated
`not`/`not-join` bodies and `or`/`or-join` branches before reading their rows.
`anti.rs` and `union.rs` construct operators from those plans; scan rewinds retain
resolved DICT constants. Prepared queries substitute slots, then call the same
executor open/execute paths (`datalog/prepared.rs:129,141,159`), planning on each
execution rather than each row. Cursors pull the resulting engine stream.
Write-transaction reads use that executor too (`src/db.rs:1774–1788`), as does
`BrowserDb::execute` (`src/browser/mod.rs:136`).

## Measurements

**Date:** 2026-10-27. Idle benchmark server, Intel Xeon E5-2696 v4,
Linux 6.8.0-139-generic, Rust 1.99.1; `envelope` profile (opt-level 3),
`-C force-frame-pointers=yes`, CPU and memory bound to NUMA node 1. The compiler
used the whole host before timing. No build/test ran alongside the measurements;
queries used checkpointed files opened read-only, with warm caches. The CPU
frequency governor was observed as `schedutil`. A [post-run environment
readback](531-environment-after.txt) confirms the toolchain and governor after
scratch cleanup; it is not a during-run measurement. These are current absolute measurements,
not an A/B against the old executor's historical 8% profile.

The [raw envelope](531-envelope.json) contains all 23 shapes at each scale,
including storage and RSS. All 69 records completed without `error` or
`capped_ms`, and each timed at least five runs. The JSON's `minigraf: 2.0.5`
comes from the unreleased branch's Cargo manifest; the measured code is the
`v3` revision above, not the published v2.0.5 release.

| Scale | Shape | Rows | Runs | p50 (ms) | p99 (ms) |
|---|---|---:|---:|---:|---:|
| 100k | `attribute_scan` | 16,666 | 200 | 12.629947 | 14.461266 |
| 100k | `join_3_selective` | 10 | 200 | 0.221847 | 0.230036 |
| 100k | `join_3_scan` | 167 | 200 | 43.020344 | 49.900300 |
| 1m | `attribute_scan` | 166,666 | 111 | 134.231385 | 147.596192 |
| 1m | `join_3_selective` | 10 | 200 | 0.207256 | 0.232138 |
| 1m | `join_3_scan` | 1,667 | 13 | 1158.606786 | 1176.507335 |
| 10m | `attribute_scan` | 1,666,666 | 11 | 1372.169005 | 1387.937067 |
| 10m | `join_3_selective` | 10 | 200 | 0.198616 | 0.237596 |
| 10m | `join_3_scan` | 16,667 | 5 | 13601.212862 | 13624.536369 |

The harness checks `join_3_selective` against its `siblings` formula (10 at
these scales). `join_3_scan` skips that expected-count assertion; independently
summing `((i * 7919 + 1) % E) % 100 == 7` over `0 <= i < E`, with
`E = floor(N / 6)`, gives 167 / 1,667 / 16,667, matching the output. The other
unchecked expected count is `as_of_ts_scan`, whose result depends on timestamp
sharing; the harness still checks that every timed run returns the first run's
count. With fewer than 100 runs its reported p99 is the sample maximum.

### Profiles at 1M

Captured separately with `perf record -e cycles:u -F 499 --call-graph fp`.
The [profile summary](531-profile-summary.json) records 21,850 selective samples
and 7,770 attribute-scan samples, with zero lost samples in both captures.
The fast selective shape repeats `__query` 200 times (each child times 200
queries plus warmup); all 200 children returned 10 rows. The attribute profile
runs one `__query` child and returns 166,666 rows.

Each child also calls `fixture_shapes`, which scans the fact log to resolve the
fixture's transaction time. That setup accounts for 77.09% of sampled cycles in
the selective capture. To separate it, classify `perf script` stacks containing
`fixture_shapes` as setup, then stacks containing `perf_envelope::drain` as
query draining, and the remainder as other; percentages sum the samples' cycle
periods, not just their counts. There are 4,385 selective and 7,679 attribute
query-drain samples (22.77% and 98.93% of capture cycles, respectively).

The selective capture has 16 stacks containing `edn_to_entity_id`: seven below
parser `entity_term`/`walk_sources`, six below planner `term<..., entity_const>`
and three below `IdSpace::keyword_eid` while joining stored keyword values.
Together they account for about 0.37% of sampled query-drain cycles. The parser
and planner conversions run while opening each query, not per candidate fact;
the stored-keyword case uses the cache described above. The attribute capture
has no stack containing that helper. Neither contains an old matcher stack.

This supports the source-audit conclusion: the old per-fact fixed-pattern-keyword
conversion is gone. Sampling alone does not prove an operation never executes,
and optimized/inlined functions can lose their own frames. System-libc leaf
symbols are unresolved for 42.68% of selective and 26.69% of attribute
query-drain cycles, so these profiles are not a complete hotspot breakdown.
No existing envelope shape has a keyword constant in value position: that
constant's resolution is verified by the source audit, not these profiles.
The expression-equality path below is not exercised by these two shapes.

### Reproducing

On an otherwise idle bench host, check out the measured revision, copy the
[dependency lockfile](531-Cargo.lock) to its root as `Cargo.lock`, and set
`run_dir` to a fresh scratch directory outside the checkout:

```bash
mkdir -p "$run_dir"
export CARGO_TARGET_DIR="$run_dir/target"
export RUSTFLAGS='-Cforce-frame-pointers=yes'
cargo build --profile envelope --example perf_envelope --locked
numactl --cpunodebind=1 --membind=1 \
  "$CARGO_TARGET_DIR/envelope/examples/perf_envelope" \
  --scales 100k,1m,10m --budget-secs 15 --cap-secs 300 \
  --dir "$run_dir/fixtures" --json "$run_dir/envelope.json"
```

For the selective capture, write the following to `$run_dir/selective.sh`:

```bash
set -eu
for run in $(seq 1 200); do
  "$CARGO_TARGET_DIR/envelope/examples/perf_envelope" __query \
    "$run_dir/fixtures/fixture-v1-1m.graph" 4 15 300
done
```

Then export `run_dir` for that child shell and record both shapes separately:

```bash
export run_dir
perf record -e cycles:u -F 499 --call-graph fp -o "$run_dir/selective.data" -- \
  numactl --cpunodebind=1 --membind=1 sh "$run_dir/selective.sh"
perf record -e cycles:u -F 499 --call-graph fp -o "$run_dir/attribute.data" -- \
  numactl --cpunodebind=1 --membind=1 \
  "$CARGO_TARGET_DIR/envelope/examples/perf_envelope" __query \
  "$run_dir/fixtures/fixture-v1-1m.graph" 2 15 300
perf report -i "$run_dir/selective.data" --stdio --header --children
perf script -i "$run_dir/selective.data" | c++filt -s rust
```

Indexes 4 and 2 select `join_3_selective` and `attribute_scan` in the measured
source. Repeat the reporting commands for `attribute.data`. Inspect JSON records
for errors/caps even if the harness exits zero. Remove scratch fixtures, builds
and captures after retaining the reports. The dependency lockfile SHA-256 is
`b03965119fd8ce5b5cf3ad3d37fb194ffeae24b6a862a0c85ae8fbcb6833ee77`.

## Separate expression-equality follow-up (draft)

A query such as:

```datalog
[:find ?f :where [?f :name ?n] [(= ?f :p5)]]
```

compares a ref with the same keyword literal once per input row.
`engine/filter.rs:155–173` evaluates expressions per row, and
`datalog/executor.rs:627–634` clones and resolves the keyword for every
keyword/ref equality pair. This path was added by #744, independently of the
removed pattern matcher; it is also present on `main` in v2.0.11.

Its performance impact is unmeasured. Expression evaluation also allocates a
binding `HashMap` per row, so the UUID computation is only one potential cost.
A follow-up should measure this shape before choosing a milestone or cache
design. Preserve equality/inequality semantics, both operand orders, dynamic
operands and prepared slots. This is performance-only; under the support policy
it does not warrant a v2.x backport. This draft has not been posted as an issue.

## Deviations from the approved design

None in scope: this is verification and documentation only, with no engine,
API, format, dependency or release change. The source archive omits the ignored
`Cargo.lock`; it is generated from the bench host's existing dependency cache
before the locked build and retained above. For sufficient selective samples,
the existing fast child mode is repeated 200 times rather than changing the
harness. No query implementation or benchmark source was changed.

The pre-run toolchain/governor were observed in command output; the standalone
environment artifact was captured only after the run. Post-run load artifacts
retained `uptime`, not the full active-CPU-process list requested by the plan.
The initial CPU-process list and intervening progress checks showed no competing
heavy workload, but the retained post-run load artifacts alone cannot prove that.
