# Literal keyword/ref expression equality (#774)

The v3 engine resolves keyword literals in `=` / `!=` expressions when the
filter or binding operator is built and reuses those UUIDs across its rows.
The immutable cache contains only literals from that expression, including
nested expressions. Prepared slots are substituted before this step. Rule
body operators resolve again when a new full/delta operator tree is built;
rewinding an existing operator retains its resolutions.

Dynamic operands still use the existing conversion on a cache miss, and never
populate the cache. Keyword/keyword equality, structural set equality, numeric
equality and returned value types are unchanged. The cache is expression
metadata, bounded by query text rather than input cardinality; like the parsed
expression, it is outside the query's data-memory meter. There is no global
cache, new dependency, public API or file-format change. This performance-only
change targets v3, with no v2.x backport under the support policy.

## Measurement before implementation

Baseline: [`bee2af00ce2008261a68993f625082fc4c8ffdb9`](https://github.com/project-minigraf/minigraf/tree/bee2af00ce2008261a68993f625082fc4c8ffdb9),
2026-10-29. The [initial measurements](774-baseline.json) ran before choosing
the implementation: two sequential rounds, reversing case order, on fresh
100K- and 1M-fact #394 fixtures. The query is:

```datalog
[:find ?f :where [?f :name ?n] [(= ?f :p5)]]
```

The eight new envelope shapes cover `=` and `!=`, either operand order,
and paired UUID-keyword controls. A control replaces `:p5` with
`:031dbb04-22ac-58e7-a7c4-315e86b720d7`, naming the same entity. It still
clones a keyword and parses its UUID, but bypasses UUID-v5 hashing. Expression
syntax does not accept a `#uuid` literal (`PRS-070`), so this is not a
pre-resolved-ref control. Differences also include string length and parsing
cost; these controls do not isolate a pure SHA-1 cost or predict the cache's
complete benefit.

At 100K, the named-keyword cases took about 30–31 ms and the controls about
28 ms. At 1M, they took about 319–323 ms and 295–302 ms respectively. This
justified a small expression-local cache, without attributing #531's old
matcher's historical 8% profile to this separate path.

## Paired comparison

Final comparison baseline: `v3` at `5f80732509a37e0674e40657468149c1af83acb1`,
after #723 landed during the initial run. Both baseline and change were rebuilt
on that revision for the retained final comparison. The earlier measurements
above remain evidence gathered before implementation.

Both binaries use Rust 1.99.1 on Linux 6.8.0-139-generic, the `envelope`
profile (opt-level 3, LTO, one codegen unit), and
`RUSTFLAGS='-Cforce-frame-pointers=yes'`. The observed governor was
`schedutil`. Compilation and tests finished before timing; process/load checks
showed no competing heavy workload. Each query drains a read-only cursor in
batches of 1,000 over the same checkpointed fixture, with warm page caches.
The [build manifest](774-build.json) records the tested/measured source hashes
and dependency lockfile hash; local and remote source hashes were checked equal.
The [post-run environment check](774-environment-after.txt) records the toolchain,
governor and process/load snapshot after all timings, not continuous monitoring.

The [comparison driver](774-compare.py) runs baseline and change concurrently
on separate sockets (CPU and memory pinned to NUMA nodes 1 and 3), swapping
nodes and reversing case order every round. Four pairs per shape and scale
use a three-second budget, at least five and at most 200 timed drains per
process. Fast point/join cases reach the 200-run cap well before three seconds;
their short duration makes them more sensitive to startup noise. With fewer
than 100 runs, the envelope's p99 is the sample maximum. This is a warm-query
comparison, not a cold-cache or production-workload claim.

[All 96 paired records](774-comparison.jsonl) are retained. The table gives the
mean of the four process p50s, in milliseconds; negative change means faster.

| Scale | Shape | Before (ms) | After (ms) | Change |
|---|---|---:|---:|---:|
| 100k | `expr_eq_keyword` | 29.98023 | 26.87894 | -10.34% |
| 100k | `expr_eq_uuid_keyword` | 27.84967 | 26.94552 | -3.25% |
| 100k | `expr_eq_keyword_reverse` | 30.07180 | 26.74923 | -11.05% |
| 100k | `expr_eq_uuid_keyword_reverse` | 27.81720 | 27.38756 | -1.54% |
| 100k | `expr_neq_keyword` | 30.25585 | 27.26363 | -9.89% |
| 100k | `expr_neq_uuid_keyword` | 28.06772 | 27.14339 | -3.29% |
| 100k | `expr_neq_keyword_reverse` | 30.34863 | 27.04602 | -10.88% |
| 100k | `expr_neq_uuid_keyword_reverse` | 28.12431 | 27.12391 | -3.56% |
| 100k | `point_entity` | 0.01802 | 0.01842 | +2.19% |
| 100k | `attribute_scan` | 11.35259 | 11.21210 | -1.24% |
| 100k | `join_3_selective` | 0.20989 | 0.21047 | +0.28% |
| 100k | `join_2_pseudo` | 0.04603 | 0.04593 | -0.21% |
| 1m | `expr_eq_keyword` | 319.48420 | 287.86908 | -9.90% |
| 1m | `expr_eq_uuid_keyword` | 302.44041 | 287.53118 | -4.93% |
| 1m | `expr_eq_keyword_reverse` | 319.95405 | 286.34977 | -10.50% |
| 1m | `expr_eq_uuid_keyword_reverse` | 298.90469 | 289.12424 | -3.27% |
| 1m | `expr_neq_keyword` | 320.75728 | 291.80473 | -9.03% |
| 1m | `expr_neq_uuid_keyword` | 300.73391 | 291.11344 | -3.20% |
| 1m | `expr_neq_keyword_reverse` | 323.82792 | 291.06117 | -10.12% |
| 1m | `expr_neq_uuid_keyword_reverse` | 301.18889 | 289.70200 | -3.81% |
| 1m | `point_entity` | 0.01817 | 0.01813 | -0.19% |
| 1m | `attribute_scan` | 121.51658 | 122.17158 | +0.54% |
| 1m | `join_3_selective` | 0.20553 | 0.20354 | -0.97% |
| 1m | `join_2_pseudo` | 0.04513 | 0.04559 | +1.01% |

The named-keyword shapes improve by **9.0–11.0%** across the measured scales.
UUID-keyword controls improve by 1.5–4.9%. Attribute scans and join guards
change by at most 1.2% in either direction. Fast point-query timings
were also checked with a [20-pair repeat](774-point-check.jsonl) on the
same binaries, swapping sockets. Means of process p50s:

| Scale | Main run point change | Repeat before (µs) | Repeat after (µs) | Repeat change |
|---|---:|---:|---:|---:|
| 100k | +2.19% | 18.631 | 18.697 | +0.35% |
| 1m | -0.19% | 18.289 | 18.144 | -0.79% |

These short runs do not establish zero overhead or a general-query speedup.
Both the main run and focused repeat are retained. Reproduce the repeat by
adding `point` after the comparison driver’s output path.

All 192 main-run processes and 80 focused-repeat processes completed without
errors or caps, with matching expected row counts.

The fixture has `floor(N / 6)` named entities. Equality returns one row;
inequality returns 16,665 rows at 100K and 166,665 at 1M. The harness asserts
these expected counts and checks that every timed run returns the same count.
The comparison driver also rejects errors, caps and mismatched before/after
counts.

## Correctness and scope

- A test-only counter in `edn_to_entity_id` failed before the fix: 8 input rows
  caused 8 keyword resolutions; 2,048 caused 2,048. After the fix the count is
  independent of rows for equality/inequality in both orders, nested
  expressions, binding expressions, negation, disjunction and a rule body.
- Existing semantic tests were extended with reversed inequality, nested
  equality and prepared queries that alternate keyword slots across repeated
  executions. They run in memory, after checkpoint and after reopen. Existing
  dynamic-operand, UUID-keyword and structural-set tests also pass.
- `cargo test` after rebasing onto #723: 1,989 passed, 17 ignored. Targeted `test-hooks` runs of
  `plan_independence_test` and `join_rules_test`: 31 passed.
- `cargo clippy --all-features -- -D warnings`, `cargo fmt --check`, and the
  companion docs-site build passed (13 versions, 368 pages).

The per-row binding `HashMap` and value cloning remain. Reusing `IdSpace` was
not chosen: its committed-id cache expires at pull boundaries, while these
constants belong to an expression and should survive operator rewinds. The
literal-only cache cannot grow with dynamic data. A compact vector keeps the
release binary within the existing size budget, avoiding a separate hash-map
instantiation. Lookup is linear in the expression’s distinct equality literals;
the timed expressions have one. The final release binary is 1,263,016 bytes
against a 1,263,616-byte cap. Nested/rule/prepared paths
are correctness-tested; the timed shapes are the plain literal expressions.

## Reproduce

Use the same [retained dependency lockfile](774-Cargo.lock) for both checkouts. Add the eight
#774 envelope shapes to the baseline checkout without changing its engine,
then build each with:

```sh
RUSTFLAGS='-Cforce-frame-pointers=yes' cargo build --locked --profile envelope --example perf_envelope
```

Create fresh fixtures using the baseline binary's `__build` mode, saving each
JSON output beside its graph as `fixture-v1-100k.json` or `fixture-v1-1m.json`:

```sh
"$before" __build "$fixtures/fixture-v1-100k.graph" 100000 > "$fixtures/fixture-v1-100k.json"
"$before" __build "$fixtures/fixture-v1-1m.graph" 1000000 > "$fixtures/fixture-v1-1m.json"
python3 docs/benchmarks/774-compare.py "$before" "$after" "$fixtures" comparison.jsonl
```

`before` and `after` are absolute binary paths; `fixtures` is a new scratch
directory. On this harness revision, indexes 24–31 are the eight expression
cases; indexes 0, 2, 4 and 6 are the point/scan/join guards. The baseline source
revision is unreleased v3 even though its Cargo manifest says `2.0.5`.
