# Phase 7.4 — Query Optimizer Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the four-index rebuild in `filter_facts_for_query` by returning an `Arc<[Fact]>` snapshot instead of a throwaway `FactStorage`, gated behind a flamegraph profiling step that confirms the index rebuild is the dominant cost.

**Architecture:** `filter_facts_for_query` currently streams all facts, computes the net-asserted view, applies temporal filters, then rebuilds four BTreeMap indexes into a throwaway `FactStorage` — on every query call. The fix returns `Arc<[Fact]>` from that function, gives the non-rules query path zero `FactStorage` construction, and updates all shared functions (`apply_or_clauses`, `evaluate_not_join`) to accept `Arc<[Fact]>` so their evaluator call sites convert `FactStorage → Arc` at call time via `get_asserted_facts()`. The rules path still builds a `FactStorage` for `StratifiedEvaluator` (which needs mutable accumulation), but post-evaluation calls use a derived-facts `Arc`.

**Tech Stack:** Rust, `std::sync::Arc`, Criterion 0.8, `cargo-flamegraph` (local profiling tool, not a dependency)

---

## File Map

| File | Role | Change type |
|---|---|---|
| `benches/minigraf_bench.rs` | Benchmarks used for profiling | Run only — no permanent edits |
| `src/query/datalog/matcher.rs` | `PatternMatcher` — add `from_slice` constructor | Modify |
| `src/query/datalog/executor.rs` | `filter_facts_for_query`, `apply_or_clauses`, `not_body_matches`, `evaluate_branch`, `execute_query`, `execute_query_with_rules` | Modify |
| `src/query/datalog/evaluator.rs` | `evaluate_not_join`, inline `PatternMatcher` call sites at lines 686 and 744, `apply_or_clauses` call at line 693, `evaluate_not_join` call at line 773 | Modify |

No new files are created. No test files are added — the 527-test suite is the semantic regression gate, plus one new unit test inside `executor.rs`.

---

## ⚠️ GATE: Task 1 must pass before any code changes

**Do not start Task 2 until Task 1 explicitly confirms the index rebuild is the dominant cost.** If profiling shows a different dominant cost, stop and surface the findings.

---

## Task 1: Profiling Gate

**Files:**
- Run: `benches/minigraf_bench.rs` (no edits)

**Context:** `filter_facts_for_query` does four things every query: (1) `get_all_facts()` — full I/O scan, (2) `net_asserted_facts()` — HashMap pass, (3) valid-time filter, (4) four BTreeMap index rebuilds via `load_fact` loop. The snapshot fix eliminates step 4. Profiling confirms step 4 actually dominates before we restructure code.

- [ ] **Step 1: Install cargo-flamegraph (if not already installed)**

```bash
cargo install flamegraph
```

Expected: installs `cargo flamegraph` subcommand. If already installed, this is a no-op.

- [ ] **Step 2: Compile benchmarks in release mode**

```bash
cargo bench --no-run
```

Expected: compiles cleanly. If compilation fails, check that the repo is on `main` and `cargo test` passes first.

- [ ] **Step 3: Run flamegraph on query benchmarks at 10K facts**

```bash
cargo flamegraph --bench minigraf_bench -- "query/point_entity/10k"
```

This profiles the `query/point_entity/10k` benchmark — a simple EAVT lookup at 10K facts. Flamegraph writes to `flamegraph.svg` in the working directory.

- [ ] **Step 4: Open and inspect flamegraph.svg**

Open `flamegraph.svg` in a browser. Look for `filter_facts_for_query` in the stack. Inside it, look for `load_fact` and BTreeMap insertion frames.

**Proceed condition:** `load_fact` / BTreeMap insertion (the four-index rebuild loop) must be visible and dominant within `filter_facts_for_query`. If `get_all_facts`, `net_asserted_facts`, or something else dominates instead, **stop here** and report findings before continuing.

- [ ] **Step 5: Run flamegraph on negation and disjunction at 10K facts**

```bash
cargo flamegraph --bench minigraf_bench -- "negation/not_scale/10k"
mv flamegraph.svg flamegraph_not.svg
cargo flamegraph --bench minigraf_bench -- "disjunction/or_scale/10k"
mv flamegraph.svg flamegraph_or.svg
```

Inspect both. Confirm `filter_facts_for_query` index rebuild still dominates (not `not_body_matches` inner loop, which is a known O(N²) post-1.0 item, not in scope here).

- [ ] **Step 6: Record baseline benchmark timings**

```bash
cargo bench -- "query/point_entity/10k" "negation/not_scale/10k" "disjunction/or_scale/10k" "aggregation/count_scale/10k" 2>&1 | tee /tmp/bench_baseline.txt
```

Save these numbers. We will compare after the fix to confirm improvement.

- [ ] **Step 7: Clean up flamegraph files (do not commit)**

```bash
rm -f flamegraph.svg flamegraph_not.svg flamegraph_or.svg
```

`flamegraph.svg` should not be committed. `/tmp/bench_baseline.txt` is kept for Task 7 comparison. Proceed only if all profiling checks passed.

---

## Task 2: `PatternMatcher` dual-constructor — `matcher.rs`

**Files:**
- Modify: `src/query/datalog/matcher.rs`

**Context:** `PatternMatcher` currently holds a `FactStorage` and calls `get_asserted_facts()` for a linear scan. We add a second internal variant (`Slice`) that holds `Arc<[Fact]>` directly. Both variants share `get_facts()` which replaces the two direct `get_asserted_facts()` calls. The existing `new(FactStorage)` constructor is unchanged — evaluator call sites continue to use it.

This task is purely additive. All existing tests continue to pass without modification.

- [ ] **Step 1: Write failing test for `from_slice` constructor**

**Before writing the test**, scan the existing `#[cfg(test)]` module in `matcher.rs` to verify the correct `Pattern` constructor signature. The test below assumes `Pattern::new(entity, attribute, value, None, None)` — adjust if the actual constructor differs.

In `src/query/datalog/matcher.rs`, inside the existing `#[cfg(test)]` module, add:

```rust
#[test]
fn test_from_slice_matches_same_as_owned() {
    let storage = FactStorage::new();
    let alice = Uuid::new_v4();
    storage
        .transact(
            vec![(alice, ":person/name".to_string(), Value::String("Alice".to_string()))],
            None,
        )
        .unwrap();

    // Build owned matcher the existing way
    let owned_matcher = PatternMatcher::new(storage.clone());

    // Build slice matcher via from_slice
    let facts: Arc<[Fact]> = Arc::from(storage.get_asserted_facts().unwrap());
    let slice_matcher = PatternMatcher::from_slice(facts);

    let pattern = Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":person/name".to_string()),
        EdnValue::Symbol("?name".to_string()),
        None,
        None,
    );

    let owned_results = owned_matcher.match_pattern(&pattern);
    let slice_results = slice_matcher.match_pattern(&pattern);

    assert_eq!(owned_results.len(), slice_results.len(), "result count mismatch");
    assert_eq!(
        owned_results[0].get("?name"),
        slice_results[0].get("?name"),
        "bound value mismatch"
    );
}
```

- [ ] **Step 2: Run the test — confirm it fails to compile**

```bash
cargo test test_from_slice_matches_same_as_owned 2>&1 | head -20
```

Expected: compile error — `from_slice` not found, `MatcherStorage` not defined.

- [ ] **Step 3: Add `MatcherStorage` enum and `from_slice` constructor**

In `src/query/datalog/matcher.rs`, replace the existing struct definition and `new` constructor with:

```rust
use std::sync::Arc;
use crate::graph::types::Fact;

enum MatcherStorage {
    Owned(FactStorage),
    Slice(Arc<[Fact]>),
}

pub struct PatternMatcher {
    storage: MatcherStorage,
}

impl PatternMatcher {
    pub fn new(storage: FactStorage) -> Self {
        PatternMatcher { storage: MatcherStorage::Owned(storage) }
    }

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

- [ ] **Step 4: Update the two `get_asserted_facts()` call sites in `match_pattern` and `match_pattern_with_bindings`**

In `match_pattern` (around line 27):
```rust
// Before:
let facts = self.storage.get_asserted_facts().unwrap_or_default();
// After:
let facts = self.get_facts();
```

In `match_pattern_with_bindings` (around line 222):
```rust
// Before:
let facts = self.storage.get_asserted_facts().unwrap_or_default();
// After:
let facts = self.get_facts();
```

- [ ] **Step 5: Run the new test — confirm it passes**

```bash
cargo test test_from_slice_matches_same_as_owned -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Run full test suite — confirm nothing is broken**

```bash
cargo test
```

Expected: all 527 tests pass (or current count).

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/matcher.rs
git commit -m "refactor: add PatternMatcher::from_slice for Arc<[Fact]> input

Adds MatcherStorage enum with Owned(FactStorage) and Slice(Arc<[Fact]>)
variants. from_slice constructor used by the executor snapshot path.
Existing new(FactStorage) constructor and all evaluator call sites
unchanged.

Part of phase 7.4 snapshot fix."
```

---

## Task 3: Unit test for `filter_facts_for_query` — write failing test first

**Files:**
- Modify: `src/query/datalog/executor.rs` (add tests to `#[cfg(test)]` module)

**Context:** `filter_facts_for_query` is a private method of `DatalogExecutor`. It can only be tested from within `executor.rs`. We write the tests NOW, before the return type changes in Task 5, so we follow TDD discipline: write failing test → implement → pass. The tests call `.len()` and index access on the return value, which are not defined on `FactStorage` — so they will fail to compile until Task 5 changes the return type to `Arc<[Fact]>`.

- [ ] **Step 1: Add unit tests inside `executor.rs` `#[cfg(test)]` module**

Find the `#[cfg(test)]` block in `src/query/datalog/executor.rs` and add:

```rust
#[test]
fn test_filter_facts_for_query_returns_net_asserted_slice() {
    use crate::graph::storage::FactStorage;
    use crate::query::datalog::types::{DatalogQuery, AsOf, ValidAt};
    use crate::query::datalog::rules::RuleRegistry;
    use std::sync::{Arc, RwLock};

    // Build a small in-memory database
    let storage = FactStorage::new();
    let alice = uuid::Uuid::new_v4();

    // Assert, then retract :person/name
    storage
        .transact(vec![(alice, ":person/name".to_string(), crate::graph::types::Value::String("Alice".to_string()))], None)
        .unwrap();
    storage
        .retract(alice, ":person/name".to_string(), crate::graph::types::Value::String("Alice".to_string()), None)
        .unwrap();

    // Assert :person/age — not retracted
    storage
        .transact(vec![(alice, ":person/age".to_string(), crate::graph::types::Value::Integer(30))], None)
        .unwrap();

    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let executor = DatalogExecutor::new(storage, rules);

    let query = DatalogQuery {
        find: vec![],
        where_clauses: vec![],
        rules: vec![],
        as_of: None,
        valid_at: None,
        with_vars: vec![],
    };

    let facts = executor.filter_facts_for_query(&query).unwrap();

    // Only the net-asserted :person/age fact should survive (name was retracted)
    assert_eq!(facts.len(), 1, "expected exactly 1 net-asserted fact");
    assert_eq!(facts[0].attribute, ":person/age", "expected :person/age");
}

#[test]
fn test_filter_facts_for_query_valid_time_filter() {
    use crate::graph::storage::FactStorage;
    use crate::query::datalog::types::{DatalogQuery, ValidAt};
    use crate::query::datalog::rules::RuleRegistry;
    use std::sync::{Arc, RwLock};

    let storage = FactStorage::new();
    let alice = uuid::Uuid::new_v4();

    // Assert with a past valid-time window that has already closed
    storage
        .transact_with_options(
            vec![(alice, ":status".to_string(), crate::graph::types::Value::String("active".to_string()))],
            Some(crate::graph::types::TransactOptions {
                valid_from: Some(1_000_000),
                valid_to: Some(2_000_000),
            }),
        )
        .unwrap();

    let rules = Arc::new(RwLock::new(RuleRegistry::new()));
    let executor = DatalogExecutor::new(storage, rules);

    // Query with valid_at in the past window — fact should appear
    let query_in_window = DatalogQuery {
        find: vec![],
        where_clauses: vec![],
        rules: vec![],
        as_of: None,
        valid_at: Some(ValidAt::Timestamp(1_500_000)),
        with_vars: vec![],
    };
    let facts_in = executor.filter_facts_for_query(&query_in_window).unwrap();
    assert_eq!(facts_in.len(), 1, "fact should be visible within valid window");

    // Query with valid_at outside the window — fact should not appear
    let query_outside = DatalogQuery {
        find: vec![],
        where_clauses: vec![],
        rules: vec![],
        as_of: None,
        valid_at: Some(ValidAt::Timestamp(3_000_000)),
        with_vars: vec![],
    };
    let facts_out = executor.filter_facts_for_query(&query_outside).unwrap();
    assert_eq!(facts_out.len(), 0, "fact should not be visible outside valid window");
}
```

**Note:** If `DatalogQuery`, `TransactOptions`, or `DatalogExecutor` constructors differ from the above, adjust to match the existing test patterns in the file. The key assertions (`facts.len()`, `facts[0].attribute`) are what matter.

- [ ] **Step 2: Run the tests — confirm they FAIL to compile**

```bash
cargo test test_filter_facts_for_query 2>&1 | head -20
```

Expected: compile error — `filter_facts_for_query` currently returns `FactStorage`; `.len()` and index access (`[0]`) are not defined on `FactStorage`. This is the expected "red" state. Task 5 will make these tests pass.

- [ ] **Step 3: Commit the failing tests**

```bash
git add src/query/datalog/executor.rs
git commit -m "test: pin filter_facts_for_query semantics — failing (red state)

Two unit tests for filter_facts_for_query in executor.rs #[cfg(test)]:
net-asserted view (retracted facts excluded) and valid-time filtering.
Tests currently fail to compile; Task 5 changes the return type to
Arc<[Fact]> to make them pass.

Phase 7.4."
```

---

## Task 4: Change `apply_or_clauses` and `evaluate_not_join` signatures

**Files:**
- Modify: `src/query/datalog/executor.rs` (signature of `apply_or_clauses`, all executor call sites)
- Modify: `src/query/datalog/evaluator.rs` (signature of `evaluate_not_join`, all evaluator call sites; evaluator call site of `apply_or_clauses`)

**Context:** `apply_or_clauses` (defined in `executor.rs`, imported by `evaluator.rs`) and `evaluate_not_join` (defined in `evaluator.rs`, imported by `executor.rs`) both currently accept `&FactStorage`. We change both to accept `Arc<[Fact]>`. At this stage, all existing call sites still have a `FactStorage` — they convert via `.get_asserted_facts()` before calling. This keeps the code compiling and all tests passing. The FactStorage-to-Arc conversion at call sites will be simplified in later tasks once `filter_facts_for_query` returns `Arc` directly.

**Important:** In `execute_query_with_rules`, `apply_or_clauses` and the not-filter must use `derived_storage` (the fully-derived fact set including rule-derived facts), NOT the base filtered facts. The conversion `Arc::from(derived_storage.get_asserted_facts().unwrap_or_default())` is correct for this case.

- [ ] **Step 1: Change `apply_or_clauses` signature in `executor.rs`**

Find the `apply_or_clauses` function signature in `src/query/datalog/executor.rs` and change the `storage` parameter:

```rust
// Before:
pub(crate) fn apply_or_clauses(
    clauses: &[WhereClause],
    bindings: Vec<Binding>,
    storage: &FactStorage,
    rules: &RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>>

// After:
pub(crate) fn apply_or_clauses(
    clauses: &[WhereClause],
    bindings: Vec<Binding>,
    storage: Arc<[Fact]>,
    rules: &RuleRegistry,
    as_of: Option<AsOf>,
    valid_at: Option<ValidAt>,
) -> anyhow::Result<Vec<Binding>>
```

Add `use std::sync::Arc;` and `use crate::graph::types::Fact;` to imports if not already present.

- [ ] **Step 2: Update `apply_or_clauses` body — `PatternMatcher::new` → `from_slice`**

Inside `apply_or_clauses` and `evaluate_branch` (which it delegates to), find any `PatternMatcher::new(storage.clone())` and change to `PatternMatcher::from_slice(storage.clone())`. `storage.clone()` is now an Arc refcount increment.

Also update `evaluate_branch`'s `storage` parameter type to `Arc<[Fact]>` and its internal `PatternMatcher::new(storage.clone())` calls to `PatternMatcher::from_slice(storage.clone())`.

- [ ] **Step 3: Fix `apply_or_clauses` call sites in `executor.rs`**

There are two call sites in `executor.rs` — in `execute_query` and `execute_query_with_rules`. Convert `FactStorage` to `Arc<[Fact]>` at each call site:

In `execute_query` (uses `filtered_storage`, a `FactStorage`):
```rust
// Before:
apply_or_clauses(&query.where_clauses, bindings, &filtered_storage, ...)?
// After:
apply_or_clauses(
    &query.where_clauses,
    bindings,
    Arc::from(filtered_storage.get_asserted_facts().unwrap_or_default()),
    ...
)?
```

In `execute_query_with_rules` (uses `derived_storage`):
```rust
// Before:
apply_or_clauses(&query.where_clauses, bindings, &derived_storage, ...)?
// After:
apply_or_clauses(
    &query.where_clauses,
    bindings,
    Arc::from(derived_storage.get_asserted_facts().unwrap_or_default()),
    ...
)?
```

- [ ] **Step 4: Fix `apply_or_clauses` call site in `evaluator.rs` (line ~693)**

```rust
// Before:
apply_or_clauses(&rule.body, raw_candidates, &accumulated, &registry_guard, None, None)?
// After:
apply_or_clauses(
    &rule.body,
    raw_candidates,
    Arc::from(accumulated.get_asserted_facts().unwrap_or_default()),
    &registry_guard,
    None,
    None,
)?
```

Add `use std::sync::Arc;` to imports in `evaluator.rs` if not already present.

- [ ] **Step 5: Change `evaluate_not_join` signature in `evaluator.rs`**

Find `pub fn evaluate_not_join` in `src/query/datalog/evaluator.rs` and change its `storage` parameter:

```rust
// Before:
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: &FactStorage,
) -> bool

// After:
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: Arc<[Fact]>,
) -> bool
```

- [ ] **Step 6: Update `evaluate_not_join` body — `PatternMatcher::new` → `from_slice`**

Inside `evaluate_not_join` (around line 447):
```rust
// Before:
let matcher = PatternMatcher::new(storage.clone());
// After:
let matcher = PatternMatcher::from_slice(storage.clone());
```

- [ ] **Step 7: Fix `evaluate_not_join` call sites in `executor.rs`**

**Note:** There are three call sites for `evaluate_not_join` in `executor.rs`: `execute_query` (line 263), `execute_query_with_rules` (line 477), and inside `evaluate_branch` (line 921). The third call site (line 921 inside `evaluate_branch`) is already handled by Step 2 above — `evaluate_branch`'s `storage` parameter is `Arc<[Fact]>` after that step, so it passes `storage` through to `evaluate_not_join` unchanged. Verify it compiles cleanly; no additional conversion is needed for line 921.

For the two explicit conversion call sites:

In `execute_query` (around line 263, in the not-filter closure):
```rust
// Before:
evaluate_not_join(join_vars, nj_clauses, binding, &not_storage)
// After:
evaluate_not_join(
    join_vars,
    nj_clauses,
    binding,
    Arc::from(not_storage.get_asserted_facts().unwrap_or_default()),
)
```

Note: `not_storage` in `execute_query` is `filtered_storage.clone()`. After Task 5, this will be simplified further.

In `execute_query_with_rules` (around line 477):
```rust
// Before:
evaluate_not_join(join_vars, nj_clauses, binding, &not_storage)
// After:
evaluate_not_join(
    join_vars,
    nj_clauses,
    binding,
    Arc::from(not_storage.get_asserted_facts().unwrap_or_default()),
)
```

Note: `not_storage` in `execute_query_with_rules` is `derived_storage.clone()`. This must remain `derived_storage`-based — it includes rule-derived facts.

- [ ] **Step 8: Fix `evaluate_not_join` call site in `evaluator.rs` (line ~773)**

```rust
// Before:
evaluate_not_join(join_vars, nj_clauses, &binding, &accumulated)
// After:
evaluate_not_join(
    join_vars,
    nj_clauses,
    &binding,
    Arc::from(accumulated.get_asserted_facts().unwrap_or_default()),
)
```

- [ ] **Step 9: Compile**

```bash
cargo build 2>&1 | head -30
```

Expected: compiles cleanly. If there are type errors, fix them before proceeding.

- [ ] **Step 10: Run full test suite**

```bash
cargo test
```

Expected: all tests pass. No semantic change has been made — just type conversions at call sites.

- [ ] **Step 11: Commit**

```bash
git add src/query/datalog/executor.rs src/query/datalog/evaluator.rs
git commit -m "refactor: change apply_or_clauses and evaluate_not_join to take Arc<[Fact]>

All call sites convert FactStorage → Arc at call time via get_asserted_facts().
No semantic change. Prepares for filter_facts_for_query returning Arc directly.

Part of phase 7.4 snapshot fix."
```

---

## Task 5: Change `filter_facts_for_query` return type and simplify `execute_query`

**Files:**
- Modify: `src/query/datalog/executor.rs`

**Context:** `filter_facts_for_query` currently returns `Result<FactStorage>` by building a new `FactStorage` and calling `load_fact` for each filtered fact — rebuilding all four BTreeMap indexes. After this task, it returns `Result<Arc<[Fact]>>` directly. `execute_query` (the non-rules path) then passes the `Arc` through to all downstream calls with no FactStorage construction at all. The expensive conversion calls added in Task 4 (`Arc::from(filtered_storage.get_asserted_facts()...)`) are replaced with direct Arc clones. The unit tests from Task 3 (`test_filter_facts_for_query_*`) will pass after Step 1 of this task.

- [ ] **Step 1: Change `filter_facts_for_query` return type and remove index rebuild**

In `src/query/datalog/executor.rs`, find `filter_facts_for_query`. Replace the final block:

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

Also update the function signature:
```rust
// Before:
fn filter_facts_for_query(&self, query: &DatalogQuery) -> Result<FactStorage>
// After:
fn filter_facts_for_query(&self, query: &DatalogQuery) -> Result<Arc<[Fact]>>
```

Replace the TODO comment on the function with:
```rust
/// **Post-1.0 backlog**: Use the on-disk B+tree indexes (EAVT/AEVT/AVET/VAET) for
/// selective attribute/entity lookups instead of the full `get_all_facts()` scan (step 1).
/// Also investigate caching the `net_asserted_facts()` result and invalidating on write (step 2).
```

- [ ] **Step 2a: Rename `filtered_storage` to `filtered_facts` in `execute_query` and update `PatternMatcher` call**

In `execute_query`, the `filtered_storage` variable is now `Arc<[Fact]>`. Rename it to `filtered_facts` for clarity:

```rust
let filtered_facts = self.filter_facts_for_query(&query)?;
let matcher = PatternMatcher::from_slice(filtered_facts.clone());
```

For `apply_or_clauses` — replace the `Arc::from(filtered_storage.get_asserted_facts()...)` conversion from Task 4 with a direct clone:
```rust
apply_or_clauses(&query.where_clauses, bindings, filtered_facts.clone(), ...)?
```

For the not-filter, `not_storage` was `filtered_storage.clone()` — replace with `filtered_facts.clone()`. The calls to `not_body_matches` and `evaluate_not_join` now receive direct Arc clones.

- [ ] **Step 2b: Update `not_body_matches` signature and internal `PatternMatcher` call**

Change `not_body_matches`'s `storage` parameter to accept `Arc<[Fact]>` (spec §2.4):
```rust
// Signature before:
fn not_body_matches(not_body: &[WhereClause], binding: &Binding, storage: &FactStorage) -> bool
// Signature after:
fn not_body_matches(not_body: &[WhereClause], binding: &Binding, storage: Arc<[Fact]>) -> bool
```

Inside `not_body_matches`, change `PatternMatcher::new(not_storage.clone())` → `PatternMatcher::from_slice(not_storage.clone())`.

- [ ] **Step 3: Update `execute_query_with_rules` — convert Arc back to FactStorage for evaluator**

`execute_query_with_rules` also calls `filter_facts_for_query`. It now receives `Arc<[Fact]>`. It must convert this to a `FactStorage` for `StratifiedEvaluator`:

```rust
let filtered_facts = self.filter_facts_for_query(&query)?;

// Convert to FactStorage for StratifiedEvaluator (needs mutable accumulation)
// TODO (post-1.0): use FactStorage::new_noindex() once profiling confirms rules-path
// index rebuild is also a bottleneck.
let filtered_storage = FactStorage::new();
for fact in filtered_facts.iter().cloned() {
    filtered_storage.load_fact(fact)?;
}
let evaluator = StratifiedEvaluator::new(filtered_storage, self.rules.clone(), 1000);
let derived_storage = evaluator.evaluate(&predicates)?;
```

After `derived_storage` is computed, compute a `derived_facts: Arc<[Fact]>` once and reuse it for both `apply_or_clauses` and the not-post-filter (the conversion calls added in Task 4):

```rust
let derived_facts: Arc<[Fact]> =
    // NOTE: must use derived_storage (includes rule-derived facts), not filtered_facts (base facts only)
    Arc::from(derived_storage.get_asserted_facts().unwrap_or_default());
```

Then replace the two `Arc::from(derived_storage.get_asserted_facts()...)` calls from Task 4 with `derived_facts.clone()`.

- [ ] **Step 4: Compile**

```bash
cargo build 2>&1 | head -30
```

Expected: compiles cleanly.

- [ ] **Step 5: Run full test suite — including Task 3 unit tests**

```bash
cargo test
```

Expected: all tests pass, including the two `test_filter_facts_for_query_*` tests written in Task 3 that were previously failing to compile. Those tests are now green because `filter_facts_for_query` returns `Arc<[Fact]>` with `.len()` and index access.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "perf: eliminate FactStorage index rebuild in execute_query path

filter_facts_for_query now returns Arc<[Fact]> instead of FactStorage,
eliminating the O(N) BTreeMap rebuild (4 indexes × N facts) on every
non-rules query call. execute_query constructs no FactStorage at all.
execute_query_with_rules still converts Arc → FactStorage for
StratifiedEvaluator (deferred to post-profiling decision).

Phase 7.4."
```

---

## Task 6: Simplify evaluator inline call sites — `evaluator.rs`

**Files:**
- Modify: `src/query/datalog/evaluator.rs`

**Context:** The evaluator's mixed-rules loop has inline `PatternMatcher::new(accumulated.clone())` calls at lines 686 and 744, and `evaluate_not_join` / `apply_or_clauses` calls that currently convert `accumulated` via `get_asserted_facts()` (added in Task 4). We compute `accumulated_facts: Arc<[Fact]>` once per rule iteration and reuse it for all four usages, eliminating redundant `get_asserted_facts()` calls.

- [ ] **Step 1: Compute `accumulated_facts` once per rule iteration**

In `src/query/datalog/evaluator.rs`, in the mixed-rules loop, find the block around line 686. Declare `accumulated_facts` at the **outermost scope of the per-rule loop body** — before line 686 — so it is visible through lines 693, 744, and 773:

```rust
// Compute once; reuse for matcher, apply_or_clauses, not-body matching, and evaluate_not_join.
// Declared at loop body scope so it remains in scope for all four usages below.
let accumulated_facts: Arc<[Fact]> =
    Arc::from(accumulated.get_asserted_facts().unwrap_or_default());
```

- [ ] **Step 2: Replace line 686 PatternMatcher call**

```rust
// Before:
let matcher = PatternMatcher::new(accumulated.clone());
// After:
let matcher = PatternMatcher::from_slice(accumulated_facts.clone());
```

- [ ] **Step 3: Replace line 693 `apply_or_clauses` call**

Replace the `Arc::from(accumulated.get_asserted_facts()...)` conversion from Task 4 with:
```rust
apply_or_clauses(&rule.body, raw_candidates, accumulated_facts.clone(), ...)?
```

- [ ] **Step 4: Replace line 744 PatternMatcher call (not-body inner loop)**

```rust
// Before:
let not_matcher = PatternMatcher::new(accumulated.clone());
// After:
let not_matcher = PatternMatcher::from_slice(accumulated_facts.clone());
```

- [ ] **Step 5: Replace line 773 `evaluate_not_join` call**

Replace the `Arc::from(accumulated.get_asserted_facts()...)` conversion from Task 4 with:
```rust
evaluate_not_join(join_vars, nj_clauses, &binding, accumulated_facts.clone())
```

- [ ] **Step 6: Compile**

```bash
cargo build 2>&1 | head -30
```

Expected: compiles cleanly.

- [ ] **Step 7: Run full test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "refactor: compute accumulated_facts Arc once per evaluator rule iteration

Replaces three separate accumulated.clone() + get_asserted_facts() calls
with a single Arc<[Fact]> computed once and reused for PatternMatcher,
apply_or_clauses, not-body matching, and evaluate_not_join.

Part of phase 7.4 snapshot fix."
```

---

## Task 7: Benchmark regression check

**Files:**
- Run only — no code changes

- [ ] **Step 1: Run the same benchmark groups as Task 1**

```bash
cargo bench -- "query/point_entity/10k" "negation/not_scale/10k" "disjunction/or_scale/10k" "aggregation/count_scale/10k" 2>&1 | tee /tmp/bench_after.txt
```

- [ ] **Step 2: Compare against baseline**

Compare the median latency numbers from `/tmp/bench_after.txt` against the numbers recorded in Task 1 Step 6 (`/tmp/bench_baseline.txt`).

Expected: non-rules query groups (`query/point_entity`, `aggregation/count_scale`) show measurable improvement. `negation/not_scale` and `disjunction/or_scale` may show partial improvement (those paths also had FactStorage overhead eliminated) but are bounded by their O(N²) inner loops (post-1.0 items). Rules-based benchmarks (`bench_recursion`) are expected to be unchanged.

If a group shows no improvement or regression, inspect with flamegraph to confirm the index rebuild is gone and something else now dominates.

- [ ] **Step 3: Clean up temp files**

```bash
rm -f /tmp/bench_after.txt /tmp/bench_baseline.txt
```

---

## Task 8: Documentation update

**Files:**
- Modify: `CLAUDE.md` (update test count)
- Modify: `ROADMAP.md` (mark 7.4 complete, update Current Focus to 7.5)
- Modify: `CHANGELOG.md` (add 7.4 entry)

**Context:** Per CLAUDE.md §8: "when a phase is marked complete, update and cross-check ALL of: CLAUDE.md, ROADMAP.md, README.md, TEST_COVERAGE.md, CHANGELOG.md." Also update affected wiki pages.

- [ ] **Step 1: Get current test count**

```bash
cargo test 2>&1 | tail -5
```

Note the exact number.

- [ ] **Step 2: Update `CLAUDE.md` test count**

In `CLAUDE.md`, find the line:
```
**527 tests passing** (365 unit + 156 integration + 6 doc).
```
Update to the new test count. If the unit/integration/doc breakdown changed, update those too.

- [ ] **Step 3: Update `ROADMAP.md`**

In `ROADMAP.md`:
- Mark phase 7.4 as `✅ Complete (March 2026)` in the progress list
- Update "Current Focus" section to point to Phase 7.5
- Update the "Last Updated" footer line

- [ ] **Step 4: Add `CHANGELOG.md` entry**

Add a new entry for the 7.4 changes. Follow the existing format in the file. Key points:
- `filter_facts_for_query` now returns `Arc<[Fact]>` instead of throwaway `FactStorage`
- Eliminates O(N) four-index rebuild on every non-rules query call
- `PatternMatcher::from_slice` constructor added
- `apply_or_clauses` and `evaluate_not_join` signatures changed to accept `Arc<[Fact]>`
- 527+ tests passing

- [ ] **Step 5: Update `TEST_COVERAGE.md`**

Update the test count and any per-file breakdown that changed (executor.rs gained 2 tests).

- [ ] **Step 6: Update `README.md` if needed**

Check if README mentions test count or phase status — update if so. Only modify `README.md` if it requires changes.

- [ ] **Step 7: Update wiki pages**

```bash
cd .wiki
```

Check `Architecture.md` — update if the query execution path description mentions `filter_facts_for_query` or `FactStorage` construction. No syntax changes, so `Datalog-Reference.md` should not need changes.

```bash
git add Architecture.md
git commit -m "docs: phase 7.4 complete — filter_facts_for_query snapshot fix"
git push
cd ..
```

Only include files that were actually modified in the `git add`.

- [ ] **Step 8: Commit all doc changes**

```bash
git add CLAUDE.md ROADMAP.md CHANGELOG.md TEST_COVERAGE.md
# Only include README.md if it was modified in Step 6:
# git add README.md
git commit -m "docs: phase 7.4 complete — snapshot fix, eliminate index rebuild

filter_facts_for_query returns Arc<[Fact]>; execute_query path constructs
no FactStorage. PatternMatcher::from_slice added. N tests passing."
```

- [ ] **Step 9: Tag the version**

Check the current version in `Cargo.toml`. Phase 7.4 is a performance fix within v0.13.x. Bump the patch version:

```bash
# In Cargo.toml: version = "0.13.0" → "0.13.1"
# Edit manually, then:
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to v0.13.1"
git tag -a v0.13.1 -m "phase 7.4 complete — filter_facts_for_query snapshot fix, eliminate 4-index rebuild"
git push origin v0.13.1
```
