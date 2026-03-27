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

**Step 2 is conditional on Step 1.** No code changes are made until profiling confirms the index
rebuild (step 4 of `filter_facts_for_query`) is the dominant cost. If profiling reveals a
different bottleneck, Step 2 does not proceed and scope is reassessed.

If Step 1 confirms the index rebuild, two query paths are treated differently by Step 2:

- **Non-rules path** (`execute_query`): no `FactStorage` is constructed at all — `PatternMatcher`,
  `apply_or_clauses`, `not_body_matches`, and `evaluate_not_join` all receive an `Arc<[Fact]>`
  snapshot directly.
- **Rules path** (`execute_query_with_rules`): still converts `Arc<[Fact]>` to a `FactStorage`
  (with index rebuild) for `StratifiedEvaluator`, which needs a mutable store for derived fact
  accumulation. Whether to eliminate this rebuild is deferred to profiling results.

Both paths share `apply_or_clauses` (defined in `executor.rs`, imported by `evaluator.rs`) and
`evaluate_not_join` (defined in `evaluator.rs`, imported by `executor.rs`). These function
signatures change in this phase; all call sites in both files are updated.

The evaluator's mixed-rules loop (`StratifiedEvaluator`) has inline `PatternMatcher::new(...)` and
`evaluate_not_join(...)` call sites that pass live-accumulating `FactStorage` objects — these are
updated to convert to `Arc<[Fact]>` at call time but the accumulation logic is otherwise
untouched.

---

## Step 1 — Profiling Gate

**Purpose**: confirm actual hotspots before making structural changes. Profiling is a local
validation step; no flamegraph files are committed to the repository.

### No `Cargo.toml` changes

Use `cargo flamegraph` (the `cargo-flamegraph` tool wrapping `perf`) rather than a
pprof/Criterion integration. This avoids a dependency version conflict: pprof's `criterion`
feature pins against Criterion `^0.5`, while the project uses `criterion = "0.8"`. These are
incompatible and cannot coexist. `cargo flamegraph` has no Criterion dependency and produces the
same flamegraph output.

**Prerequisite**: `cargo install flamegraph` (one-time, local).

### Validation run

```bash
cargo bench --no-run  # compile benchmarks in release mode
cargo flamegraph --bench minigraf_bench -- query_simple_10k
cargo flamegraph --bench minigraf_bench -- query_negation_10k
cargo flamegraph --bench minigraf_bench -- query_aggregation_10k
cargo flamegraph --bench minigraf_bench -- query_disjunction_10k
```

Flamegraphs land in `flamegraph.svg` in the working directory. Inspect to confirm
`filter_facts_for_query`, `net_asserted_facts`, and the index rebuild loop are top-of-stack.

**Gate rule — proceed condition**: Step 2 begins **only if** the flamegraph shows the `load_fact`
loop / BTreeMap insertions (step 4) as the dominant stack frame within `filter_facts_for_query`
at 10K+ facts. If steps 1–3 (`get_all_facts`, `net_asserted_facts`, valid-time filter) dominate
instead, **do not touch any executor, matcher, or evaluator code** — surface the findings and
reassess scope before proceeding.

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

- Step 1: addressed by using the on-disk B+tree for selective attribute/entity lookups (deferred,
  not scoped here)
- Step 2: would require caching the net-asserted view and invalidating on write (not currently
  scoped)

Both are captured in the post-1.0 backlog for future consideration.

---

## Step 2 — Snapshot Fix *(proceeds only if Step 1 gate passes)*

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
`PatternMatcher::new(FactStorage)` is kept **unchanged** for evaluator call sites that pass
live-accumulating storage.

```rust
enum MatcherStorage {
    Owned(FactStorage),
    Slice(Arc<[Fact]>),
}

pub struct PatternMatcher {
    storage: MatcherStorage,
}

impl PatternMatcher {
    // Existing — unchanged
    pub fn new(storage: FactStorage) -> Self {
        PatternMatcher { storage: MatcherStorage::Owned(storage) }
    }

    // New — used by executor and evaluator call sites that provide a pre-filtered slice
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

**Two call sites** in `matcher.rs` call `self.storage.get_asserted_facts()` and must both be
updated to `self.get_facts()`:

- `match_pattern` (line 27)
- `match_pattern_with_bindings` (line 222)

No other logic changes. `match_patterns` and `match_patterns_seeded` delegate to these two methods
and require no further changes.

**Per-join allocation note**: `s.to_vec()` in the `Slice` arm produces a fresh `Vec<Fact>` on
every `match_pattern` / `match_pattern_with_bindings` call. This is pre-existing behaviour (the
`Owned` arm also allocates via `get_asserted_facts()`). The snapshot fix eliminates the index
rebuild (step 4) — it does not eliminate per-join allocations. That would require iterating the
slice directly in the match loop, which is out of scope for this phase.

**Correctness note**: the `Slice` variant receives a pre-filtered, net-asserted slice from
`filter_facts_for_query`. This is the same fact set that `get_asserted_facts()` would have
returned from the old `FactStorage`. No additional filtering is needed.

### 2.3 Shared function signature changes

Two functions are shared between `executor.rs` and `evaluator.rs` and have their signatures
changed in this phase. All call sites in both files are updated.

#### `apply_or_clauses` — defined in `executor.rs`, imported by `evaluator.rs`

Change `storage: &FactStorage` parameter to `storage: Arc<[Fact]>`. Inside, all
`PatternMatcher::new(storage.clone())` calls become `PatternMatcher::from_slice(storage.clone())`.
The `Arc` clone is a refcount increment.

**Call sites**:

| File | Location | Old | New |
|---|---|---|---|
| `executor.rs` | `execute_query` | `&filtered_storage` (FactStorage) | `filtered_facts.clone()` (Arc) |
| `executor.rs` | `execute_query_with_rules` | `&derived_storage` (FactStorage) | `Arc::from(derived_storage.get_asserted_facts().unwrap_or_default())` |
| `evaluator.rs` | mixed-rules loop line 693 | `&accumulated` (FactStorage) | `Arc::from(accumulated.get_asserted_facts().unwrap_or_default())` |

In `execute_query_with_rules`, the `apply_or_clauses` call operates on `derived_storage` — the
fully-derived fact set produced by `StratifiedEvaluator` (base facts + all rule-derived facts).
The slice must include derived facts; converting via `get_asserted_facts()` is correct.

#### `evaluate_not_join` — defined in `evaluator.rs`, imported by `executor.rs`

Change `storage: &FactStorage` parameter to `storage: Arc<[Fact]>`. Inside,
`PatternMatcher::new(storage.clone())` becomes `PatternMatcher::from_slice(storage.clone())`.

**Call sites**:

| File | Location | Old | New |
|---|---|---|---|
| `executor.rs` | `execute_query` not-filter (line 263) | `&not_storage` (FactStorage clone) | `not_facts.clone()` (Arc clone) — same slice as `filtered_facts` |
| `executor.rs` | `execute_query_with_rules` not-filter (line 477) | `&not_storage` (derived_storage clone) | `Arc::from(derived_storage.get_asserted_facts().unwrap_or_default())` |
| `evaluator.rs` | mixed-rules loop line 773 | `&accumulated` (FactStorage) | `Arc::from(accumulated.get_asserted_facts().unwrap_or_default())` |

In `execute_query_with_rules`, the not-post-filter at lines 388–483 must operate on
`derived_storage` (fully-derived facts), not the base `Arc<[Fact]>` from
`filter_facts_for_query`. `not` in the query body tests against the fully-derived state,
including rule-derived predicates. Converting via `derived_storage.get_asserted_facts()` is
correct.

### 2.4 Executor-internal function signature changes

These functions are only called within `executor.rs`; no `evaluator.rs` changes needed.

| Function | Old param | New param |
|---|---|---|
| `not_body_matches` | `storage: &FactStorage` | `storage: Arc<[Fact]>` |
| `evaluate_branch` | `storage: &FactStorage` | `storage: Arc<[Fact]>` |

Inside each, `PatternMatcher::new(storage.clone())` becomes `PatternMatcher::from_slice(storage.clone())`.

The `evaluate_not_join` call at line 921 (inside `evaluate_branch`) propagates the type change
automatically — when `evaluate_branch`'s `storage` parameter becomes `Arc<[Fact]>`, that same
value is passed through to `evaluate_not_join` unchanged.

### 2.5 Evaluator inline `PatternMatcher` call sites — `evaluator.rs`

The mixed-rules loop in `StratifiedEvaluator` has two inline `PatternMatcher::new(...)` call sites
that pass `accumulated` (a live-accumulating `FactStorage`). These are updated to use
`from_slice`:

- Line 686: insert `let accumulated_facts: Arc<[Fact]> = Arc::from(accumulated.get_asserted_facts().unwrap_or_default());`
  **before** the existing line, then replace the line itself with
  `PatternMatcher::from_slice(accumulated_facts.clone())`. Reuse the same `Arc` for the
  `apply_or_clauses` call at line 693 to avoid a second `get_asserted_facts()` pass.
- Line 744: `PatternMatcher::new(accumulated.clone())` (in the not-body inner loop) →
  `PatternMatcher::from_slice(accumulated_facts.clone())` using the same `Arc` computed above if
  it is still in scope; otherwise compute fresh.

### 2.6 `execute_query` call site

Receives `Arc<[Fact]>` from `filter_facts_for_query` (renamed `filtered_facts` for clarity).
Uses `PatternMatcher::from_slice(filtered_facts.clone())`. Passes `filtered_facts.clone()` to
`apply_or_clauses` and `not_body_matches` / `evaluate_not_join`. No `FactStorage` is constructed
at any point in the non-rules path.

### 2.7 `execute_query_with_rules` call site

Receives `Arc<[Fact]>` from `filter_facts_for_query`. Converts to `FactStorage` for
`StratifiedEvaluator` (which needs mutable storage for derived fact accumulation):

```rust
let filtered_storage = FactStorage::new();
for fact in filtered_facts.iter().cloned() {
    filtered_storage.load_fact(fact)?;
}
let evaluator = StratifiedEvaluator::new(filtered_storage, ...);
let derived_storage = evaluator.evaluate(&predicates)?;
```

After evaluation, `derived_storage` is used for pattern matching, `apply_or_clauses`, and the
not-post-filter — all converted to `Arc<[Fact]>` via `derived_storage.get_asserted_facts()` at
call time (one conversion per distinct call, not one per binding).

The index rebuild cost for the initial `filtered_storage` is still paid here. A TODO comment notes
`FactStorage::new_noindex()` as the next step if profiling confirms the rules-path rebuild also
dominates.

---

## Conditional Extension: `FactStorage::new_noindex()`

**Only implement if profiling confirms the rules-path index rebuild is a significant cost.**

Add a `skip_indexing: bool` field to `FactData`. When set:
- `load_fact` and `transact` skip all `pending_indexes.insert()` calls
- `get_all_facts()` always uses the full `d.facts` scan (bypassing the `eavt.is_empty()` check,
  which would fail once derived facts start populating the index)

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

**One new unit test** in the `#[cfg(test)]` module of `executor.rs`: assert that
`filter_facts_for_query` returns the correct temporally-filtered, net-asserted fact set for a
small database covering assert/retract/valid-time scenarios. This pins the return type and
semantics against future refactors. The function is private to `DatalogExecutor` and can only be
tested from within `executor.rs`.

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
- Per-join allocation elimination in `PatternMatcher` (requires iterating slice directly in match
  loop; out of scope)
