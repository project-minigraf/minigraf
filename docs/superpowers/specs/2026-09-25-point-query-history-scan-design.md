# Point-query history scan mitigation (#323) — Design

**Issue:** #323 — point query on a bound entity scans the entity's whole version history
**Milestone:** v2.3.0 (`main`, file format v7 — no format change)
**Follow-up:** structural O(live) fix on the `v3` branch (separate issue, see §6)

## 1. Problem

`[:e/x :attr ?v]` costs O(every record ever written for `:e/x`), not O(live facts).
An entity whose attribute is retracted and reasserted N times gets ~N× slower to read.
Downstream (temporal_reasoning#239) this is ~⅓ of ingestion wall clock. Attribute-only
scans (`[?e :attr ?v]`) show the same history-bound behaviour.

Reproduced locally (2000 filler facts, checkpointed, median of 20):

| depth | `[:e/hot :hash ?v]` | `[:e/hot :other ?v]` | `[?e :hash ?v]` |
|---:|---:|---:|---:|
| 1 | 0.026 ms | 0.024 ms | 0.022 ms |
| 500 | 1.769 ms | 1.749 ms | 1.688 ms |
| 2000 | 7.156 ms | 7.250 ms | 7.067 ms |

`:other` is written once and never churned, yet costs the same as `:hash`: the scan is
entity-wide, not attribute-wide.

## 2. Where the time goes

`perf` at depth 2000 (4001 EAVT records for the entity):

| component | share |
|---|---:|
| `net_asserted_facts` — SipHash on cloned `(Uuid, String, Vec<u8>, i64, i64)` keys | ~40% |
| `get_facts_by_entity` — `resolve_fact_ref` + postcard decode per record | ~30% |
| `selective_fact_fetch` dedup `HashSet<(Uuid, String, u64, bool)>` (redundant — §4.2) | ~16% |

It is CPU overhead per dead record, not I/O.

**Why v2 cannot be O(live):** v7 `EavtKey`/`AevtKey` carry neither the value nor the
`asserted` flag. Deciding whether a record is live requires knowing its `(e, a, v)` and
whether a later retraction of that `v` exists — only obtainable by resolving the record.
Retraction records also have their own validity window (`[tx_id, ∞)`), so key-level
valid-time pruning cannot safely discard them. v2 can therefore only shrink the constant
and narrow the scan to the queried attribute.

## 3. Goals / non-goals

**Goals**
- Other attributes on a churned entity no longer pay for that entity's churn.
- Constant factor per history record cut by ≥3× on the churned attribute itself.
- The attribute-scan and full-scan paths benefit from the cheaper `net_asserted_facts`.
- Zero change to query results, file format, public API, or dependencies.

**Non-goals**
- O(live) resolve cost (needs v8 keys — §6).
- Changes to `retract`/`transact` write paths.
- Changes to the `:as-of` path beyond the shared `net_asserted_facts` speed-up.

## 4. Design

### 4.1 Attribute-narrowed entity scan

**Storage** (`src/graph/storage.rs`): new production method

```rust
pub(crate) fn get_facts_by_entity_attribute_indexed(
    &self, entity_id: &EntityId, attribute: &Attribute,
) -> Result<Vec<Fact>>
```

- EAVT range `[(e, a, i64::MIN, i64::MIN, 0), (e, next_string_prefix(a), i64::MIN, i64::MIN, 0))`.
  If `next_string_prefix` returns `None`, upper bound is `(e+1, "", …)` and results are
  filtered by `key.attribute == a`.
- Pending (in-memory `BTreeMap`) and committed (`range_scan_eavt`) merged exactly as
  `get_facts_by_entity` does; both paths post-filter `attribute == a` so prefix-sharing
  attributes (`:a` vs `:ab`) never leak.
- No-index fallback: filter `d.facts` / `loader.stream_all()` on entity **and** attribute.
- The existing `#[cfg(test)] get_facts_by_entity_attribute` (full-scan) helper stays as is.

**Executor** (`selective_fact_fetch` in `src/query/datalog/executor.rs`):

- Replace `entity_ids: HashSet<Uuid>` with `entity_attrs: HashMap<Uuid, Option<HashSet<String>>>`.
  - Pattern with bound entity and `AttributeSpec::Real(EdnValue::Keyword(a))` → add `a`
    to that entity's set (unless it is already `None`).
  - Pattern with bound entity and any other attribute (variable, pseudo-attribute) →
    set that entity to `None` (whole-entity scan).
- Narrowed lookup count = Σ over entities of (1 if `None` else number of attributes) +
  number of attribute-only lookups. If it exceeds the existing `threshold` (4), every
  entity collapses back to one whole-entity scan (today's behaviour) and the count is
  recomputed; only if *that* still exceeds the threshold → `None` (full scan), as today.
  This guarantees narrowing never turns a query that used the selective path into a
  full scan (e.g. five attributes of one bound entity).
- Fetch `get_facts_by_entity` for `None` entries and
  `get_facts_by_entity_attribute_indexed` per `(e, a)` otherwise.

**Correctness argument:** `net_asserted_facts` groups by `(e, a, v)` and the valid-time
filter is per-fact, so restricting input to one `(e, a)` never splits a group.
`collect_all_patterns` already includes patterns inside `not`, `not-join`, `or`,
`or-join`, so every attribute any clause could match on that entity is fetched.
Rule-using queries never reach this path.

### 4.2 Remove the executor dedup

`selective_fact_fetch` has exactly one caller, `filter_facts_for_query`, which passes its
output straight into `net_asserted_facts`. That function is idempotent under duplicated
input records: an identical assertion lands in the same `(e, a, v, valid_from, valid_to)`
window and the first one wins the tie; a duplicated retraction leaves the per-`(e, a, v)`
max `tx_count` unchanged. So the `seen: HashSet<(Uuid, String, u64, bool)>` pass is
redundant and is deleted. It cannot simply be skipped "for single-source lookups" instead:
between `set_committed_index_reader` and `post_checkpoint_clear` a concurrent reader can
see a fact in both the pending and the committed index. The idempotence is pinned by a
unit test (§5) so a future change to `net_asserted_facts` cannot silently break it.

### 4.3 Cheaper `net_asserted_facts`

- Add a small in-crate FxHash-style hasher (`FxHasher` + `type FxBuildHasher =
  BuildHasherDefault<FxHasher>`) in a new private module `src/graph/fxhash.rs`
  (~30 lines, no dependency). Word-at-a-time multiply-rotate as in rustc's FxHash.
- Restructure `net_asserted_facts` so each fact's value is encoded once and group keys
  borrow from the input rather than cloning `attribute`/`value_bytes` into every key:
  1. `encoded: Vec<Vec<u8>> = facts.iter().map(|f| encode_value(&f.value)).collect()`.
  2. `max_retract_tx: HashMap<(&Uuid, &str, &[u8]), u64, FxBuildHasher>`.
  3. `by_window: HashMap<(&Uuid, &str, &[u8], i64, i64), usize, FxBuildHasher>` storing the
     index of the winning assertion.
  4. Collect surviving indices, then move the winning facts out of `facts`
     (e.g. mark survivors in a `Vec<bool>` and `into_iter().zip(...).filter`).
- Output **order**: callers must not depend on it today (current output is HashMap
  iteration order). Tests that compare results already sort or use sets; verify during
  implementation.
- HashDoS: not a concern — keys are the embedding application's own data, in-process.

## 5. Testing

TDD; correctness first, then measurement.

**Storage unit tests** (`src/graph/storage.rs`), for `get_facts_by_entity_attribute_indexed`:
- pending-only, committed-only, pending+committed after checkpoint;
- no-index fallback path;
- `:a` vs `:ab` prefix isolation;
- other entities / other attributes excluded.

**`net_asserted_facts` tests**: existing tests keep passing; add a duplicate-idempotence
test (`net_asserted_facts(xs ++ dup) == net_asserted_facts(xs)` as sorted sets); add a property-style test
comparing the new implementation against the old one (kept as a `#[cfg(test)]` reference
function) on randomized assert/retract sequences incl. multiple windows, same-tx
assert+retract, and multi-valued attributes — compared as sorted sets.

**Executor integration tests** (`tests/point_query_history_test.rs`, new):
- churned attribute returns exactly the live value after N retract/reassert cycles
  (in-memory and file-backed, before and after checkpoint);
- `[:e :a ?x] [:e :b ?y]` on one entity (two narrowed ranges, dedup path);
- bound entity with variable attribute `[:e ?a ?v]` (whole-entity fallback);
- bound entity with pseudo-attribute under `:any-valid-time`;
- `not` / `not-join` / `or` referencing another attribute of the same bound entity;
- entity + attribute-only pattern mix exceeding threshold → full-scan path, same results;
- `:valid-at` past timestamp on a churned attribute.

All assert messages follow the CLAUDE.md testing convention (no `{:?}` of UUID-bearing types).

**Benchmark** (`benches/minigraf_bench.rs`): `point_query_chain_depth/{1,500,2000}` for
`[:e/hot :hash ?v]`, `[:e/hot :other ?v]`, `[?e :hash ?v]` on a checkpointed file DB.
Record before/after numbers in the PR and `docs/BENCHMARKS.md`.

**Acceptance:**
- `[:e/hot :other ?v]` at depth 2000 within 2× of depth 1.
- `[:e/hot :hash ?v]` and `[?e :hash ?v]` at depth 2000 ≥3× faster than the baseline in §1.
- Full suite green; clippy/fmt clean.

## 6. Follow-up: v3 structural fix (separate issue, milestone v3.0.0)

v8 `EavtKey`/`AevtKey` (on `v3`) carry `value_bytes` and `asserted`. Net-assert can then be
computed over index keys *before* `resolve_fact_ref`, resolving only surviving records:
resolve cost O(live), key walk O(history) but on leaf pages already in cache. File as a
new issue referencing #323; not part of this change.

## 7. Docs

On merge: `CHANGELOG.md` (Unreleased → Performance), `docs/BENCHMARKS.md`,
`docs/TEST_COVERAGE.md` + `CLAUDE.md` test count. No wiki or file-format docs change.
