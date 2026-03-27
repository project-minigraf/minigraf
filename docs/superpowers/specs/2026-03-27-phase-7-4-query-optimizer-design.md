# Phase 7.4 — Query Optimizer Improvements Design

## Overview

Phase 7.4 targets the primary query performance bottleneck: `filter_facts_for_query` in
`executor.rs`, which rebuilds all four in-memory indexes (`EAVT`/`AEVT`/`AVET`/`VAET`) from
scratch on every query call. The fix is gated behind a profiling step that confirms the bottleneck
before any structural change is made.

**Scope (confirmed)**:
- Profiling integration (local validation gate only — not committed to `BENCHMARKS.md`)
- `filter_facts_for_query` snapshot overhead fix

**Deferred to post-1.0 backlog** (added to `ROADMAP.md`):
- Cost-based optimizer extensions for new clause types (`not`/`or`/aggregates)
- Rule evaluation optimization for rules with `not`/`or`/aggregates
- Predicate push-down (`Expr` clauses filtered early rather than as a final post-pass)
- Optional: `FactStorage::new_noindex()` for the rules query path (conditional on profiling)

---

## Architecture Overview

Two query paths exist and are treated differently by this phase:

- **Non-rules path** (`execute_query`): after this phase, no `FactStorage` is constructed at
  all — `PatternMatcher`, `apply_or_clauses`, `not_body_matches`, and `evaluate_not_join` all
  receive an `Arc<[Fact]>` snapshot directly.
- **Rules path** (`execute_query_with_rules`): still converts `Arc<[Fact]>` to a `FactStorage`
  (with index rebuild) for `StratifiedEvaluator`, which needs a mutable store for derived fact
  accumulation. Whether to eliminate this rebuild is deferred to profiling results.

Evaluator-internal `PatternMatcher::new(FactStorage)` call sites in `evaluator.rs` are
**untouched** — they operate on live-accumulating storage and are not part of this phase.

---

## Step 1 — Profiling Gate

**Purpose**: confirm actual hotspots before making structural changes. Profiling is a local
validation step; no flamegraph files are committed to the repository.

### `Cargo.toml`

```toml
[dev-dependencies]
pprof = { version = "0.14", features = ["flamegraph", "criterion"] }
```

### `benches/minigraf_bench.rs`

Replace the existing TODO stub with a working pprof-profiled Criterion group:

```rust
use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default()
        .with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = /* existing benchmark functions */
}
```

**Validation run**: `cargo bench -- --profile-time 10` on the query, negation, aggregation, and
disjunction groups at 10K and 100K facts. Flamegraphs land in
`target/criterion/*/profile/flamegraph.svg`. Inspect to confirm `filter_facts_for_query`,
`net_asserted_facts`, and the index rebuild loop are top-of-stack.

**Gate rule**: if profiling shows a different dominant cost, stop and surface the findings before
touching executor or matcher code.

### What the profiling reveals

`filter_facts_for_query` performs four steps on every query call:

| Step | What | Cost |
|---|---|---|
| 1 | `get_all_facts()` — full scan of committed on-disk fact pages + clone of pending `Vec<Fact>` | O(N) I/O |
| 2 | `net_asserted_facts()` — HashMap pass to resolve net-asserted state per `(entity, attribute, value)` | O(N) |
| 3 | Valid-time filter | O(N) |
| 4 | 4-index rebuild via `load_fact` loop — BTreeMap insertions for EAVT/AEVT/AVET/VAET | O(N) |

**The snapshot fix eliminates step 4 only.** Steps 1–3 remain after this phase. If profiling shows
that step 1 (`get_all_facts` I/O) or step 2 (`net_asserted_facts`) dominate, further work is
needed:

- Step 1: addressed by using the on-disk B+tree for selective attribute/entity lookups (deferred
  Option B, not scoped here)
- Step 2: would require caching the net-asserted view and invalidating on write (not currently
  scoped)

This is captured in the post-1.0 backlog for future consideration.

---

## Step 2 — Snapshot Fix

### 2.1 `filter_facts_for_query` — `executor.rs`

Change return type from `Result<FactStorage>` to `Result<Arc<[Fact]>>`. The temporal filtering
logic (steps 1–3 above) is unchanged. Only the final construction step is removed:

```rust
// Before:
let filtered_storage = FactStorage::new();
for fact in valid_filtered {
    filtered_storage.load_fact(fact)?;
}
Ok(filtered_storage)

// After:
Ok(Arc::from(valid_filtered))
```

The `Arc<[Fact]>` slice contains temporally-filtered, net-asserted facts — identical semantics to
the previous `FactStorage`, just without the indexes. No additional filtering is needed downstream
when matching against this slice.

Replace the TODO comment on the function with a note pointing to the post-1.0 backlog for the
optional B+tree selective-lookup integration (the step-1 fix above).

### 2.2 `PatternMatcher` — `matcher.rs`

Add a second internal representation and a new `from_slice` constructor. The existing
`PatternMatcher::new(FactStorage)` is kept **unchanged** for evaluator call sites.

```rust
enum MatcherStorage {
    Owned(FactStorage),
    Slice(Arc<[Fact]>),
}

pub struct PatternMatcher {
    storage: MatcherStorage,
}

impl PatternMatcher {
    // Existing — unchanged, used by evaluator
    pub fn new(storage: FactStorage) -> Self {
        PatternMatcher { storage: MatcherStorage::Owned(storage) }
    }

    // New — used by executor non-rules path
    pub(crate) fn from_slice(facts: Arc<[Fact]>) -> Self {
        PatternMatcher { storage: MatcherStorage::Slice(facts) }
    }

    fn get_facts(&self) -> Vec<Fact> {
        match &self.storage {
            MatcherStorage::Owned(s) => s.get_asserted_facts().unwrap_or_default(),
            MatcherStorage::Slice(s) => s.to_vec(),
        }
    }
}
```

`match_pattern` replaces its `self.storage.get_asserted_facts()` call with `self.get_facts()`.
No other logic changes.

**Correctness note**: the `Slice` variant receives a pre-filtered, net-asserted slice from
`filter_facts_for_query` — the same set that `get_asserted_facts()` would have returned from the
old `FactStorage`. No additional filtering is needed.

### 2.3 Cascading signature changes — `executor.rs`

All functions in `executor.rs` that accept `&FactStorage` purely to construct a `PatternMatcher`
change to accept `Arc<[Fact]>` instead. These functions never call index-based methods on the
storage; the change is mechanical.

| Function | Old param | New param |
|---|---|---|
| `apply_or_clauses` | `storage: &FactStorage` | `storage: Arc<[Fact]>` |
| `evaluate_branch` | `storage: &FactStorage` | `storage: Arc<[Fact]>` |
| `not_body_matches` | `storage: &FactStorage` | `storage: Arc<[Fact]>` |
| `evaluate_not_join` (executor's own) | `storage: &FactStorage` | `storage: Arc<[Fact]>` |

Inside each, `PatternMatcher::new(storage.clone())` becomes
`PatternMatcher::from_slice(storage.clone())`. The `Arc` clone is a refcount increment, not a
copy.

`evaluate_not_join` in `evaluator.rs` (the evaluator's own version) is **unchanged** — it
operates on the live-accumulating `FactStorage` path.

### 2.4 `execute_query` call site

Receives `Arc<[Fact]>` from `filter_facts_for_query`. Uses `PatternMatcher::from_slice` and
passes the `Arc<[Fact]>` to all or/not functions. No `FactStorage` is constructed at any point in
the non-rules path.

### 2.5 `execute_query_with_rules` call site

Receives `Arc<[Fact]>` from `filter_facts_for_query`, then converts to `FactStorage` for
`StratifiedEvaluator`:

```rust
let filtered_storage = FactStorage::new();
for fact in filtered_facts.iter().cloned() {
    filtered_storage.load_fact(fact)?;
}
let evaluator = StratifiedEvaluator::new(filtered_storage, ...);
```

The index rebuild cost is still paid here. A TODO comment notes `FactStorage::new_noindex()` as
the next step if profiling confirms the rules-path rebuild also dominates.

---

## Conditional Extension: `FactStorage::new_noindex()`

**Only implement if profiling confirms the rules-path index rebuild is a significant cost.**

Add a `skip_indexing: bool` field to `FactData`. When set:
- `load_fact` and `transact` skip all `pending_indexes.insert()` calls
- `get_all_facts()` always uses the full `d.facts` scan (bypassing the index fallback check)

`execute_query_with_rules` would use `FactStorage::new_noindex()` instead of `FactStorage::new()`
when constructing the evaluator's initial storage from `Arc<[Fact]>`. `StratifiedEvaluator`'s
internal `PatternMatcher` calls only use `get_asserted_facts()` — they are safe with no indexes.

**Prerequisite**: verify that `StratifiedEvaluator` never calls index-based methods
(`get_facts_by_entity`, `get_facts_by_attribute`) directly. If it does, `new_noindex()` would
silently return empty results for those calls and must not be used.

---

## Testing Strategy

**No new test files.** The snapshot fix is a pure performance change — query semantics are
unchanged. The primary correctness gate is the existing 527-test suite.

**One new unit test** in `executor.rs` (or `tests/snapshot_test.rs`): assert that
`filter_facts_for_query` returns the correct temporally-filtered, net-asserted fact set for a
small database covering assert/retract/valid-time scenarios. This pins the return type and
semantics against future refactors.

**Benchmark regression gate**: run `cargo bench` before and after the snapshot fix on the existing
query groups at 10K facts. Record median latency delta. Expected: measurable improvement on
non-rules query groups (simple query, negation, aggregation, disjunction); rules groups may be
unchanged until the conditional extension is implemented.

---

## Out of Scope

- Predicate push-down (post-1.0 backlog)
- Cost-based optimizer extensions for `not`/`or`/aggregate clause types (post-1.0 backlog)
- Rule evaluation optimization (post-1.0 backlog)
- B+tree selective-lookup integration in `filter_facts_for_query` step 1 (deferred to profiling)
- `net_asserted_facts` caching (not currently scoped)
