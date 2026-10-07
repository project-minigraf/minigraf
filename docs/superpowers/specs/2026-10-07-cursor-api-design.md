# Cursor API (#432, v3.0.0 scope) — design

**Issue:** #432 (streaming, pull-based query engine). **Branch:** `v3`.
**Scope decision (2026-10-01):** option 2 of the issue — land the public cursor API in
3.0.0 with the current materialising evaluator behind it, then replace operators one by
one in 3.x without further breaking changes. The spill/memory-budget design (goal 1)
is explicitly deferred: the issue comment of 2026-10-03 asks for a revisit (index-ordered
streaming, scratch pages inside the `.graph` file, or a clear error over budget) and the
cursor API does not depend on it.

## Goal

Fix the public shape now, so that a client written against 3.0.0 never has to change when
the engine starts streaming:

- `Minigraf::query(&str) -> Result<Cursor, MinigrafError>`
- `PreparedQuery::query(&[(&str, BindValue)]) -> Result<Cursor, MinigrafError>`
- `Cursor::vars()`, `Cursor::next_batch(max_rows) -> Result<Option<Batch>, MinigrafError>`,
  `Cursor::close(self)`, and `impl Iterator for Cursor` yielding
  `Result<Vec<Value>, MinigrafError>` rows.
- `Batch`: `len`, `is_empty`, `rows() -> &[Vec<Value>]`, `into_rows() -> Vec<Vec<Value>>`,
  `IntoIterator`.

`execute()` keeps its signature and behaviour; every existing test passes unchanged.

## Decisions

1. **`Cursor` is owned, `Send`, and has no lifetime.** A streaming cursor will need read
   access to the store after `query()` returns. It gets that through the same `Arc`s
   `PreparedQuery` already holds, never a borrow of `&Minigraf` and never a lock held for
   the cursor's life: the store is append-only, so a snapshot is a bound on `tx_count`, not
   a copy or a lock. Fixing this now is the one thing that would otherwise be a breaking
   change later.
2. **`next_batch` returns `Result`.** Today it cannot fail after `query()` succeeds, but a
   streaming engine can fail mid-stream (a damaged page, the recursive operator's
   `max_derived_facts` limit). Iterator items are `Result` for the same reason.
3. **The public `Batch` is row-major.** Internal operator batches may become columnar; the
   conversion happens once, at the cursor boundary, where rows leave the engine anyway.
   Fields are private so the representation can change behind `rows()`/`into_rows()`.
4. **`max_rows == 0` is treated as 1**, so a loop of `next_batch(n)` always makes progress
   and `None` is the only end-of-results signal. Batches are never empty.
5. **Snapshot.** A cursor's answer is fixed at `query()`: writes that commit while it is
   open do not change it. Today this holds because the answer is computed at open; the
   streaming engine must keep it by bounding reads at the `tx_count` current at open.
   A test pins the guarantee.
6. **Limits.** `query()` uses the same executor and limits as `execute()`.
   `max_results` and `max_derived_facts` bound recursive rule evaluation (`INT-020`); they
   do not cap the rows of a plain query, for either entry point. The issue's "a cursor has
   no result cap" therefore needs no change now. When the recursive operator is rewritten,
   `max_derived_facts` stays as its safety limit. `PreparedQuery::query` matches
   `PreparedQuery::execute`, which does not apply the `OpenOptions` limits (unchanged here).
7. **Commands other than `(query ...)`.** `query()` rejects `transact`, `retract` and `rule`
   with new **`API-012`**: "only (query ...) commands can be opened as a cursor; got {}".
   A query with bind slots fails with `API-010`, as `execute()` does. Called on a thread
   that holds a `WriteTransaction`, it fails with `INT-001`, as `execute()` does (a cursor
   there would silently miss the transaction's own writes).
8. **`close(self)`** drops the cursor. It exists so the API names the early-stop operation
   the streaming engine will propagate upstream; dropping without `close` is equivalent.
9. **Out of scope:** `WriteTransaction::query` (additive later), `:limit` (#306, v3.1.0),
   operator rewrite, spill/memory budget, FFI. The FFI `Cursor` object (`vars()`,
   `next_batch(n) -> String`, `close()`) lives in the binding repos and gets its own issue;
   it is additive over the core API.

## Implementation

- New module `src/cursor.rs` with `Cursor` and `Batch`; re-exported from `lib.rs`.
  `Cursor { vars: Vec<String>, rows: std::vec::IntoIter<Vec<Value>> }`.
- `Minigraf::query` parses, rejects non-queries (`API-012`) and unbound slots (`API-010`),
  runs the read path `execute()` uses, and wraps the `QueryResults`.
- `PreparedQuery::query` calls the existing substitute + execute path and wraps the result.
- Error code `API-012` registered in `src/error.rs` and `docs/ERROR_REFERENCE.md`.

## Tests (`tests/cursor_test.rs`)

- Batches partition the `execute()` answer: same vars, same multiset of rows, each batch
  `<= max_rows`, no empty batch, then `None` (and `None` again after the end).
- `max_rows == 0` makes progress.
- The iterator yields the same rows as `execute()`.
- Empty result: `vars` set, first `next_batch` is `None`.
- Aggregates, rules and `:as-of` through `query()` match `execute()`.
- Snapshot: a cursor opened before a `transact` (and before a `WriteTransaction` commit)
  returns the pre-transaction answer to the end; on a file-backed database too.
- `transact`/`retract`/`rule` → `API-012`; bind slots → `API-010`; inside a
  `WriteTransaction` thread → `INT-001`; parse error → the parser's code.
- `PreparedQuery::query` with binds matches `PreparedQuery::execute`.
- A recursive rule over `max_results` → `INT-020` from `query()`, as from `execute()`.
- `Cursor` is `Send` and can be consumed on another thread after `db` is dropped.
- Close after the first batch is accepted (counters for upstream work arrive with the
  operator rewrite).
