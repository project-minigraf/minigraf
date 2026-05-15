# Wave 1 Performance Design: O(N²) → O(N)

**Date**: 2026-05-15  
**Issues**: #208, #202, #203, #204  
**Delivery**: 2 PRs

---

## Overview

Wave 1 addresses four O(N²) performance bottlenecks in the query engine. All four are
pure algorithmic fixes — no API changes, no file-format changes, no new dependencies.

**PR 1** — #208: B+Tree selective lookup (prerequisite for #229 SIMD benchmarking)  
**PR 2** — #202 + #203 + #204: Hash-join cluster (negation, disjunction, general join)

---

## PR 1 — #208: Selective B+Tree Lookup

### Problem

`filter_facts_for_query` (executor.rs:300) always calls `get_all_facts()`, which reads
every committed page from disk plus all pending in-memory facts — a full sequential scan
regardless of how selective the query patterns are.

`get_facts_by_entity` and `get_facts_by_attribute` already exist in `graph/storage.rs`
with correct EAVT/AEVT B+tree index logic (committed + pending layers), but are gated
`#[cfg(test)]` only.

### Storage layer changes (`src/graph/storage.rs`)

Remove `#[cfg(test)]` from:
- `get_facts_by_entity`
- `get_facts_by_attribute`
- `get_facts_by_entity_attribute`

No logic changes to these methods.

### Executor changes (`src/query/datalog/executor.rs`)

Add private helper:

```
fn selective_fact_fetch(
    storage: &FactStorage,
    patterns: &[Pattern],
    threshold: usize,          // suggested: 4
) -> Option<Vec<Fact>>
```

Logic:
1. Walk main-query patterns (not not/not-join bodies).
2. Collect distinct bound entity literals (`EdnValue::Uuid`) and bound attribute strings
   (non-variable `AttributeSpec::Real`).
3. Count total lookups = `distinct_entities + distinct_attributes`.
4. If count == 0 or count > threshold: return `None` → caller falls back to `get_all_facts()`.
5. Otherwise: call `get_facts_by_entity` per entity + `get_facts_by_attribute` per
   attribute, union into one `Vec<Fact>`, deduplicate by `(entity, attribute, value,
   tx_count)` key.

Modify `filter_facts_for_query`:
- In the `(None, None)` arm (no `facts_override`, no `as_of`): call
  `selective_fact_fetch` first; use its result if `Some`, fall back to `get_all_facts()`
  otherwise.
- The `as_of` arms always use `get_facts_as_of` (time-travel queries require the full
  log for correctness).
- Steps 2 and 3 (net-assert, valid-time filter) are unchanged.

### Guard rationale

Threshold of 4: at 5+ distinct index lookups, the sum of B+tree traversal overhead
(page cache lookups, per-entry allocation) can exceed a single sequential page scan,
especially on a warm page cache. `not`/`not-join` bodies are excluded from the count
because their attributes are often high-cardinality; they receive the same `Arc<[Fact]>`
and are already correct with the reduced set (not bodies constrain variables already
bound by main patterns).

### Tests

- Entity-bound point query uses selective path (verify via correct result count against
  known population, not via spy — avoids coupling tests to internals).
- Attribute-bound query (e.g. `[?e :person/name ?n]`) uses selective path.
- Query with 5+ distinct entity/attribute lookups falls back to full scan (result correct).
- `as_of` queries always take full-scan path (result correct).
- Full existing test suite passes with no regressions.

### Benchmarks

Add new benchmark groups (at 1k/10k/100k scale):
- `btree_lookup/entity_point` — single entity literal, all attributes
- `btree_lookup/attribute_scan` — single bound attribute, all entities

These establish the performance baseline that #229 (SIMD) will build on.

---

## PR 2 — #202 + #203 + #204: Hash-Join Cluster

### #202 — Pre-computed not/not-join exclusion sets

**File**: `src/query/datalog/executor.rs` (execute_query not-filter section)  
**File**: `src/query/datalog/evaluator.rs` (evaluate_not_join)

**Problem**: `not_body_matches` and `evaluate_not_join` create a fresh `PatternMatcher`
and run a full pattern-match for every outer binding → O(outer × inner).

**Fix in `execute_query`**:

Before `bindings.into_iter().filter(...)`, for each not-body:
1. Run `PatternMatcher::match_patterns(not_body_patterns)` once against `filtered_facts`.
2. Collect results into `HashSet<Vec<(String, Value)>>` keyed on the join variables
   only (sorted for determinism).
3. In the filter loop, probe the set in O(1) instead of re-running the matcher.

**Fix in `evaluate_not_join`**:

Same pre-compute-once / probe-per-binding pattern for the rules path.

**Edge cases**:
- Expr-only not bodies (no patterns): keep current behaviour unchanged.
- Not bodies with rule invocations: routed to `execute_query_with_rules` unchanged.

### #203 — Empty-seed branch evaluation in `apply_or_clauses`

**File**: `src/query/datalog/executor.rs` (apply_or_clauses)

**Problem**: `evaluate_branch(branch, bindings.clone(), ...)` seeds each branch with
all incoming bindings → O(seeds × facts × branches).

**Fix for `WhereClause::Or`**:
1. Evaluate each branch from a single empty-map seed (independent of incoming bindings).
2. Union branch results across all branches (deduplicated — same as today's `seen` set).
3. Hash-join the union back onto incoming `bindings` on the shared variables (variables
   present in both the incoming bindings and the branch results). Incoming bindings with
   no match are dropped; matching ones are extended with any new variables introduced by
   the branch → O(N) probe per binding.

**Fix for `WhereClause::OrJoin`**:
1. Same empty-seed branch evaluation.
2. Project branch results to `join_vars` only, build `HashMap<join_key, Vec<Binding>>`
   keyed on the `join_vars` tuple.
3. Hash-join incoming `bindings` against the map on `join_vars`.

**Correctness invariant**: The hash-join produces the same result as the current
seeded-branch evaluation but in O(N) rather than O(seeds × facts). Incoming bindings
with no match in any branch are dropped, matching Datomic `or`/`or-join` semantics.

### #204 — Hash-join in `join_with_pattern`

**File**: `src/query/datalog/matcher.rs` (join_with_pattern)

**Problem**: For each existing binding, scan all facts for the next pattern →
O(existing_bindings × facts).

**Fix**: Detect shared variables between the new pattern and existing bindings:

1. Identify the **join variable**: the first variable in entity, attribute, or value
   position of the new pattern that also appears as a key in the existing bindings.
   Entity position is checked first (covers the dominant `?e`-join case).
2. If a join variable is found:
   - Scan candidate facts for the new pattern once.
   - Group into `HashMap<join_key_value, Vec<Bindings>>`.
   - For each existing binding, look up its join-variable value in the map → O(1).
3. If no join variable is detected (unrelated patterns or fully-literal patterns):
   fall back to the current nested-loop path.

This fix lives entirely in `join_with_pattern`. All callers — `match_patterns`,
`match_patterns_seeded`, `match_patterns_with_hints` — benefit automatically.

### Tests

Each fix gets its own test coverage:
- **#202**: `not` and `not-join` shapes at 1k/10k — assert correct result counts;
  all existing negation tests pass.
- **#203**: `or` and `or-join` shapes at 1k/10k — union semantics preserved; all
  existing or/or-join tests pass.
- **#204**: two-pattern shared-`?e` query at 1k/10k — correct result counts; all
  existing join tests pass.

Full `cargo test` suite passes with no regressions before merging PR 2.

### Benchmarks

The existing benchmark groups will show the improvement automatically — no new groups
needed:
- `negation/not_scale`, `negation/not_join_scale` (#202)
- `disjunction/or_scale`, `disjunction/or_join_scale` (#203)
- `aggregation/with_grouped_sum` (#204)

---

## Acceptance Criteria Summary

| Issue | Criterion |
|-------|-----------|
| #208  | Entity-bound and attribute-bound queries use selective index fetch; `as_of` uses full scan; ≥5 distinct lookups fall back to full scan; new benchmarks added |
| #202  | Negation inner loop pre-computes exclusion set once; `not_scale`/`not_join_scale` benchmarks show sub-linear growth |
| #203  | Or/or-join branches evaluated from empty seed; `or_scale`/`or_join_scale` show sub-linear growth |
| #204  | `join_with_pattern` uses hash-join when shared variable detected; `with_grouped_sum` shows sub-linear growth |

All: existing 795-test suite passes; no new `{:?}` debug format in test assert messages.
