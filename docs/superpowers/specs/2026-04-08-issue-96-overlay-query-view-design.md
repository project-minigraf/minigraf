# Issue 96 Overlay Query View Design

**Goal:** Remove the full `FactStorage` deep-copy/reindex path from `WriteTransaction` read-your-own-writes queries without changing public API or query semantics.

**Non-goals:**
- No API changes in `Minigraf`, `WriteTransaction`, or query types
- No behavior changes for commit, rollback, rule registration, or transaction visibility
- No broad query engine refactor beyond what is needed to avoid the rebuild

## Problem

`WriteTransaction::execute()` currently routes `Query` and `Rule` commands through `build_query_view()`, which materializes a fresh `FactStorage` by:

1. loading all committed facts from the shared store
2. rebuilding pending indexes in the temporary store
3. appending the transaction's buffered facts

That makes every transactional read with pending writes pay an O(N) copy/reindex cost over committed state.

## Constraints

- Transactional reads must still see committed facts plus all buffered facts in the current transaction
- Retractions, temporal filters, pseudo-attributes, prepared query behavior, and rule evaluation results must remain unchanged
- The shared `FactStorage` must not be mutated by transactional reads
- The fix should stay internal and keep the code understandable

## Proposed Approach

Introduce a lightweight transactional query path that operates on a merged fact slice instead of a rebuilt `FactStorage`.

### Query path split

- Keep the existing fast path when `pending_facts` is empty: reuse `self.inner.fact_storage.clone()`
- When `pending_facts` is non-empty:
  - gather committed facts from the shared store
  - append cloned pending facts into one contiguous `Arc<[Fact]>`
  - execute the query against that merged slice

### Execution strategy

- Add a transaction-focused execution helper in the Datalog executor layer that accepts an `Arc<[Fact]>`
- Reuse the existing slice-based query machinery already used by non-rules query execution where possible
- For rule-aware transactional reads, build only the minimum adapter required for the evaluator path rather than rebuilding indexes eagerly for every read

### Safety and semantics

- The merged fact slice is read-only and transaction-local
- Ordering remains append-only: committed facts first, pending facts after
- Existing filtering rules still determine visible state, including net assertions/retractions and temporal windows

## File Impact

- `src/db.rs`
  - replace the deep-copy transactional read path
  - remove or narrow `build_query_view()`
- `src/query/datalog/executor.rs`
  - add or extend a helper for executing from an `Arc<[Fact]>` in the transactional path
- `tests` / `src/db.rs` unit tests
  - add regression coverage proving transactional reads still work and the old rebuild path is no longer required

## Testing Plan

TDD order:

1. Add a failing regression test for transactional query execution with pending writes
2. Verify the test fails for the intended reason
3. Implement the slice/overlay query path
4. Re-run the focused test
5. Run broader transactional and query suites

Focused verification targets:

- `cargo test db::tests::test_write_transaction_read_your_own_writes -- --nocapture`
- relevant query tests if executor changes require them
- full `cargo test` before completion

## Risks

- Rule evaluation still expects storage-oriented inputs in some paths; the fix must not accidentally reintroduce the same copy through a different helper
- Merged-slice execution must preserve the same temporal and retraction semantics as the current storage-backed path

## Success Criteria

- No public API changes
- Transactional reads with pending facts no longer rebuild a full temporary `FactStorage`
- Existing read-your-own-writes behavior remains intact
- Tests pass in the issue worktree
