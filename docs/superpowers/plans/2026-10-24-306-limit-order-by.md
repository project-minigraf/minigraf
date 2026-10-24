# `:limit`, `:offset` and `:order-by` (#306) — Plan

Spec: `docs/superpowers/specs/2026-10-24-limit-order-by-design.md` (PR #729), "the spec"
below. PRs target `v3`, merge commits, no closing keyword; #306 closes by hand after PR 2.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–9: `:limit` (spec §7.1) | `feat/306-limit-op` |
| 2 | 10–17: `:order-by`, `:offset`, `Sort` (spec §7.2) | `feat/306-order-by` |

TDD in each step: the test first, run it to see it fail, then the code. Tests run on the bench
machine (`ssh 192.168.1.16`, `PATH=$HOME/.cargo/bin:$PATH`); the laptop only as a fallback.

## Decisions not fixed by the spec

1. **`DatalogQuery` is crate-internal** (`pub(crate) mod query`, `lib.rs:93`), so its new
   fields are not a public API change.
2. **`Count`** (`types.rs`): `enum Count { Literal(usize), Slot(String) }`. A literal is
   `usize::try_from(i64)`; a value above `usize::MAX` cannot occur on 64-bit targets, and on
   32-bit wasm a literal above `u32::MAX` saturates to `usize::MAX` (no limit is lost: no
   answer has that many rows).
3. **The planner never sees a slot.** `plan_with` reads `Count::Literal`; `Count::Slot` there
   is `INT-032` (planner invariant, as in the #551 plan, decision 2). `execute` rejects an
   unbound slot first (`API-010`), and `PreparedQuery` substitutes it.
4. **Counters are not public**, so the acceptance test (integration, `#[ignore]`) checks wall
   time and peak RSS, and the "reads at most one batch past `n`" check is an engine unit test
   over `Stream::counters` (deviation 1 from spec §5, which puts both in the acceptance test).
5. **Error text of `PRS-115`**: `` `{}` must be a non-negative integer or a bind slot, got {} ``
   (option, value).
6. **`Limit` and offset in PR 1.** `Step::Limit { offset, limit }` and the operator take an
   offset from PR 1, always `0` until PR 2 parses `:offset`; its unit tests cover a non-zero
   offset, so the code is not untested.

## PR 1 — `:limit`

### Step 1. `DatalogQuery::limit` and the parser

- `types.rs`: `Count`; `DatalogQuery.limit: Option<Count>`, `None` in `new`/`from_patterns`.
- `parser.rs` `parse_query`: a `:limit` arm beside `:max-results` (`parser.rs:885`): repeated is
  `INT-012`, no value `INT-013`; `Integer(n)` with `n >= 0` is `Literal`, `BindSlot(name)` is
  `Slot`, any other value (negative, float, string, keyword…) `PRS-115`.
- `error.rs`: `Prs115` in the enum, the registry (code, title, text), the parser category arm
  and the `ALL` list (four sites, as `Prs114`: `error.rs:163`, `1003`, `2272`, `2534`).
  `docs/ERROR_REFERENCE.md`: table row and section, with cause, resolution and example.
- Tests (`parser.rs` unit): literal, `0`, slot, each section order (`:limit` first, between
  `:find` and `:where`, last), duplicate, missing value (end of vector and followed by a
  keyword), negative, float, string. `tests/error_codes_prs_test.rs`: `prs_115_…`.

### Step 2. Prepared slots

- `prepared.rs`: `collect_slot_names` adds a `Count::Slot` name; `substitute` resolves it with
  `resolve_count_slot(name, ":limit", bindings)`: `Val(Integer(n))`, `n >= 0` → `Literal`;
  `n < 0` → `PRS-115`; other bind value → `INT-035` (`":limit"`, `"Val(Integer)"`); none →
  `INT-033`.
- Tests (`prepared.rs` unit, `tests/prepared_statements_test.rs`): `slot_names` lists it;
  `execute` with the slot unbound is `API-010`; each resolver branch; one prepared query run
  with `$n = 1, 3, 0`.

### Step 3. `Limit` operator

- `engine/batch.rs`: `Batch::slice(&mut self, skip, take)` keeping rows `skip..skip + take`
  (no new allocation: `drain`/`truncate` per column).
- `engine/limit.rs`: `Limit { child, skip, left, counters }`. `next_batch`: if `left == 0`,
  close the child and return `None` without pulling; else pull, drop up to `skip` rows from
  the front, keep up to `left`, pass on a non-empty batch; when `left` reaches 0, close the
  child. `rewind` restores `skip`/`left` from the step; `close` closes the child. Counters
  op `"Limit"`.
- `plan.rs`: `Step::Limit { offset: usize, limit: Option<usize> }` (`None` = offset only, for
  PR 2); `filter::Apply::new`'s op-name match gets the arm (unreachable there, since `build`
  routes `Step::Limit` to `limit::Limit`).
- Engine unit tests (`engine/tests.rs`): over a 5,000-row scan, `limit` 0, 1, 1,023, 1,024,
  1,025, 5,000, 6,000 give `min(n, 5000)` rows; `limit 0` leaves the scan's `rows_in` at 0;
  `limit 10` leaves it at most `BATCH_ROWS`, and a further `next_rows` reads nothing more;
  offset 3 with limit 2 gives rows 3 and 4 of the scan's order; `rewind` gives the same rows
  again.

### Step 4. Planning `:limit`

- `plan_with`: after the `Window` step (`plan.rs:536`), push `Step::Limit { offset: 0,
  limit: Some(n) }` for `Some(Literal(n))`; `Some(Slot)` is `INT-032`.
- Engine tests: plain, join, `not`, `or`, aggregate (grouped and ungrouped, including the zero
  row with `:limit 0` → no rows), window, rule query (`open_rules` goes through `plan_with`),
  `:as-of`, `:valid-at`. Each checks `min(n, |answer|)` rows, every one in the answer.

### Step 5. Integration tests

`tests/limit_order_test.rs` (native only, `#![cfg(not(target_arch = "wasm32"))]`), a helper
`is_sub_bag(rows, answer)`:

- Every step-4 shape through `Minigraf::execute`, `Minigraf::query` (cursor, pulled one row at
  a time), and `PreparedQuery::execute`/`query`, each with `minigraf::__reorder(true)` and
  `(false)`: the row count is `min(n, |answer|)` and the rows are a sub-bag of the unlimited
  answer.
- A file-backed database with committed and pending facts (checkpoint, then more writes).
- `WriteTransaction` reads with `:limit`, including the transaction's own staged facts.

### Step 6. Grammar and verifier

- `grammar.pest`: `kw_limit`; `limit_section = { kw_limit ~ (integer_lit | bind_slot) }` in
  `query_section`.
- Corpus (`grammar_conformance.rs`): `tests/grammar/valid/query_limit.edn`,
  `query_limit_zero.edn`, `query_limit_slot.edn`; `tests/grammar/invalid/syntax/`
  `query_limit_missing.edn`, `query_limit_string.edn`; `tests/grammar/invalid/semantic/`
  `query_limit_negative.edn` (pest's `integer_lit` takes a sign), `query_limit_twice.edn`.
- `verifier.rs`: `":limit"` in `TOKENS` (33 entries); the template
  `("(query [:find ?x :where [?x :a ?y]", "])")` already reaches `:limit` fillers. `SEMANTIC`:
  `PRS-115` with witness `(query [:find ?x :where [?x :a ?y] :limit -1])` and `INT-012` with
  `… :limit 1 :limit 1])` (no `INT-012` entry exists yet; a 4-token filler reaches it at the
  nightly bound 4). `KNOWN_DRIFT` stays empty at bound 3
  locally and bound 4 on the bench.

### Step 7. Acceptance and envelope

- `tests/limit_acceptance_test.rs` (`#[ignore]`, run as `memory_limit_acceptance_test` is, in
  `smoke.yml` nightly): 10M facts `[:e{i} :a i]`, checkpointed in 100K chunks;
  `[:find ?e ?v :where [?e :a ?v] :limit 10]` returns 10 rows in under 50 ms (debug-safe bound
  set from a bench measurement) and adds under 32 MB peak RSS; the same at 1M within 2× of
  the 10M time either way (time independent of fact count).
- `examples/perf_envelope.rs`: shape `limit`, `[:find ?e ?a :where [?e :age ?a] :limit 10]`,
  rows `10.min(e)`. Run on the bench at 100K, 1M, 10M (`--profile envelope`); numbers in the
  PR body and `BENCHMARKS.md`.
- `smoke.yml`: add the new test to the nightly acceptance job (on `v3`; `main` has no engine,
  so no main PR).

### Step 8. Docs

`CHANGELOG.md` (Unreleased → Added: `:limit`, `PRS-115`), `README.md` (query features),
`CLAUDE.md` (module notes: `limit.rs`, `Count`; test count), `docs/TEST_COVERAGE.md`,
`ROADMAP.md` (#306 PR 1 done). Docs site waits for PR 2 (spec §7).

### Step 9. Review and PR

`cargo fmt`, `cargo clippy --all-targets` (and `--features test-hooks`), full suite on the bench
(plain and `test-hooks`), wasm32 build (`cargo build --target wasm32-unknown-unknown
--features browser`), release binary size against the 1,200 KiB gate. Codex review
(`codex exec -s read-only … < /dev/null`). PR to `v3` "Part of #306".

## PR 2 — `:order-by`, `:offset`, `Sort`

Branch `feat/306-order-by` from `v3` 59ce7f24 (PR 1 merged as #745), worktree
`.worktrees/306-order-by`.

### Decisions for PR 2 (not fixed by the spec)

7. **No new `Direction` type.** `types::Order { Asc, Desc }`, which a window's `:over`
   already uses, is the direction: `DatalogQuery.order_by: Vec<(FindSpec, Order)>`
   (deviation 2 from spec §4, which names a `Direction`).
8. **Entries are matched after the section loop.** `:order-by` may come before `:find`
   (spec §3), so `parse_query` keeps the parsed entries and checks them against
   `find_specs` once every section is read: shape errors (`PRS-118`, and `PRS-117` for an
   entry that is not a variable or a list) as the vector is read, `PRS-117` for an entry no
   `:find` element equals and `PRS-118` for a repeat after the loop, then `PRS-116`. A list
   entry is read with `parse_aggregate`, as `:find` reads one; an entry it rejects is
   `PRS-117` (it is not a `:find` element), so the grammar-accepted `(foo ?x)` is semantic.
9. **`Sort` holds values, not cells.** Each held row is the `Vec<Value>` of its `:find`
   columns (the full order compares all of them), turned back into cells with
   `Cell::of_value` only for the rows it returns, as `Window` does for its results. Holding
   both cells and values would double the memory for no gain; a value's cell round-trips
   (`Iid` ↔ keyword, `Eid` ↔ ref, a long string back to its value ref).
10. **`Sort` returns the `:find` columns only.** `Step::Sort { cols, order, keep }` reads the
    slots `cols` (= `plan.find`), returns batches of width `cols.len()`, and `plan.find`
    becomes `0..cols.len()`, so `Limit` and `Project` after it see only those columns.
    `order` is the full order as `(column index, desc)`: the order keys, then every other
    `:find` index ascending.
11. **Top-k is a hand-written binary heap** over `Vec<Vec<Value>>` with the full-order
    comparator (worst row on top): `std::collections::BinaryHeap` needs `Ord` on the element,
    which would carry the order spec in every row. ~30 lines, unit-tested against a full
    sort.
12. **A `Sort` stream keeps its DICT lookups** (`Stream::hold_ids`, as for `Window`): it turns
    every row's ids into values in its first pull. With top-k it holds at most `k` rows'
    ids' worth of lookups beyond what the scan touched; without `keep` it holds the answer
    anyway.
13. **`:offset` alone (with `:order-by`, no `:limit`)** plans `Sort` without `keep` and
    `Limit { offset, limit: None }`.
14. **Error texts**: `PRS-116` `` `:offset` requires `:order-by` ``; `PRS-117`
    `` `:order-by` entry {} is not a `:find` element ``; `PRS-118`
    `` `:order-by` takes a non-empty vector of `:find` elements, each optionally followed by
    `:asc` or `:desc`; {} `` (the fault: `not a vector`, `empty`, `` `?x` twice ``,
    `` `:desc` without an element ``, `` `:foo` ``).

### Step 10. Parser: `:order-by` and `:offset`

- `types.rs`: `DatalogQuery.order_by: Vec<(FindSpec, Order)>` (empty = none) and
  `offset: Option<Count>`; `new`/`from_patterns` set them empty/`None`; `Count`'s doc names
  `:offset`.
- `parser.rs` `parse_query`: an `:offset` arm as `:limit`'s (`INT-012` repeated, `INT-013`
  no value, `PRS-115` otherwise). An `:order-by` arm: repeated `INT-012`; no value (end, or a
  keyword next) `INT-013`; not a vector `PRS-118`; empty `PRS-118`; then per item: `?sym` or
  a list starts an entry, `:asc`/`:desc` sets the direction of the entry before it (none
  before it, or that entry already has one: `PRS-118`), any other keyword `PRS-118`,
  anything else `PRS-117`. After the loop (decision 8): each entry must equal a `:find`
  element (`PRS-117`) and appear once (`PRS-118`); `offset.is_some() && order_by.is_empty()`
  is `PRS-116`.
- `error.rs`: `Prs116`–`Prs118` at the four sites (enum after `Prs115`, registry, parser
  category arm, `ALL`); `docs/ERROR_REFERENCE.md` rows and sections.
- Tests (`parser.rs` unit): defaults; `:asc` default; `:desc`; aggregate and window entries;
  every section order (`:order-by` before `:find`, between, last; `:offset` before
  `:order-by`); `:offset 0`, slot; every row of spec §3.1 for the two sections, incl.
  `?y` bound in `:where` not in `:find`, a `:with` variable, `(count ?e)` not in `:find`,
  `1`, `"x"`, `(foo ?x)`, `[?x ?x]`, `[(count ?e) (count ?e)]`, `[:desc ?x]`,
  `[?x :asc :desc]`, `[?x :foo]`, `[]`, `?x` (not a vector). `tests/error_codes_prs_test.rs`:
  `prs_116_…`, `prs_117_…`, `prs_118_…`.

### Step 11. Prepared `:offset` slots

- `prepared.rs`: `collect_slot_names` adds `offset`'s slot; `substitute` resolves it with
  `resolve_count_slot(name, ":offset", bindings)` (its doc names both options).
- Tests (`prepared.rs` unit): listed in `slot_names`; unbound `API-010`; `n >= 0`, `n < 0`
  `PRS-115`, wrong type `API-024`; one slot name used by both `:limit` and `:offset`.

### Step 12. The full-order comparator

- `aggregate.rs`: `pub(crate) fn row_order(a: &[Value], b: &[Value], order: &[(usize, bool)])
  -> Ordering`: the first non-equal `value_order` of the listed columns, reversed where
  `desc`.
- Tests: each direction; ties falling to the next column; every `value_order` class in one
  column (Integer before an equal Float, `-0.0` before `0.0`, NaN after numbers, strings,
  booleans, refs, keywords, `nil`, sets), ascending and descending.

### Step 13. `Sort` operator

- `engine/sort.rs`: `Sort { child, cols, order, keep: Option<usize>, rows: Vec<Vec<Value>>,
  out: Option<Vec<Vec<Value>>>, held: Held, counters }`.
  - `keep == Some(0)`: close the child without pulling, return `None`.
  - Otherwise read the child to its end; each row's values at `cols` (a missing slot reads
    `nil`, as `Window`), charged (`size_of::<Vec<Value>>()` + `value_bytes`) before it is
    held, `try_reserve` for growth.
  - Without `keep`: push; at the end sort by `row_order` (`sort_unstable_by`: rows equal
    under the full order are equal in every column, so stability is not needed).
  - With `keep = k`: a max-heap (worst on top) of at most `k` rows; while below `k`, push and
    sift up; else, if the row is better than the top, replace it (release the old row's
    charge) and sift down; otherwise drop the row (never charged: compare first, then
    charge). At the end, sort the heap's rows.
  - Return the sorted rows in batches of `BATCH_ROWS` (`Cell::of_value` per value),
    uncharging each row as it leaves.
  - `rewind` clears and rewinds the child; `close` releases and closes the child;
    `counters` reports `bytes_held`.
- `plan.rs`: `Step::Sort { cols: Vec<usize>, order: Vec<(usize, bool)>, keep: Option<usize> }`;
  `mod.rs` `build` routes it; `hold_ids` also for `Sort` (decision 12).
- Engine unit tests (`sort.rs`, with a test child as `limit.rs` has): sort of 5,000 shuffled
  rows equals `sort_by(row_order)`; top-k for k = 1, 10, 1,024, 4,999, 5,000, 6,000 equals
  the full sort's first `k`; `keep 0` pulls nothing and closes the child; ties broken by
  the second column; descending keys; `bytes_held` with `keep 10` over 100,000 rows stays
  under the bytes of ~10 rows (bounded, not growing); `QRY-012` with a small meter limit,
  with and without `keep`; `rewind` sorts again.

### Step 14. Planning

- `plan_with`, after `finish` and before the `Limit` push: with `order_by` non-empty, map
  each entry to its `:find` index (`position(==)`, an invariant error if none: the parser
  checked it), add every other index ascending, push `Step::Sort { cols: plan.find.clone(),
  order, keep }` with `keep = limit.map(|l| offset.saturating_add(l))`, set
  `plan.find = (0..n).collect()`. Then `Step::Limit { offset, limit }` when either is given
  (`offset` default 0); a slot in either is `INT-032`.
- Engine tests: ordered plain, join, `not`, `or`, aggregate (`(count ?e) :desc`, grouped and
  ungrouped zero row), window (sort by a window column), rule query, `:as-of`, `:valid-at`;
  each equals the unlimited answer sorted by the full order, sliced `offset..offset+limit`.

### Step 15. Integration tests (`tests/limit_order_test.rs`)

- Ordered shapes (each step-14 shape with `:order-by`): through `execute`, the cursor (one
  row at a time), `PreparedQuery::execute`/`query` with `$n`/`$o` slots, reordering on and
  off: the list equals `answer` sorted by the full order (computed in the test with its own
  comparator over `value_order` classes, on the test's value keys), sliced; all four paths
  and both reorder settings give the same list.
- Pages: `:offset 0, 3, 6 … :limit 3` concatenate to the whole ordered answer.
- `:offset` past the answer → no rows; `:limit 0` → none; `:offset` without `:limit`.
- A value of every class in one ordered column, ascending and descending.
- File database with committed and pending facts; `WriteTransaction` reads.
- `QRY-012` from `Sort` with a small `query_memory_limit`, and none with top-k `:limit 10`
  under the same limit over enough rows to exceed it unsorted.
- Grammar (step-6 style): `grammar.pest` `kw_offset`, `order_by_section = { kw_order_by ~
  "[" ~ order_entry+ ~ "]" }`, `order_entry = { find_spec ~ (kw_asc | kw_desc)? }`,
  `offset_section = { kw_offset ~ (integer_lit | bind_slot) }`, both in `query_section`;
  `docs/GRAMMAR.md` regenerated (`UPDATE_GRAMMAR_DOC=1`). Corpus: valid `query_order_by.edn`,
  `query_order_by_desc_aggregate.edn`, `query_order_by_offset_limit.edn`,
  `query_offset_slot.edn`; syntax `query_order_by_not_vector.edn`,
  `query_order_by_empty.edn`, `query_order_by_leading_desc.edn`; semantic
  `query_offset_without_order_by.edn`, `query_order_by_not_in_find.edn`,
  `query_order_by_twice_entry.edn`, `query_offset_negative.edn`.
- Verifier: `":offset"`, `":asc"`, `":desc"` in `TOKENS` (37); template 16
  `("(query [:find ?x :where [?x :a ?y] :order-by [", "]])")`; `SEMANTIC` `PRS-116`
  (`… :offset 1])`), `PRS-117` (`… :order-by [?y]])`), `PRS-118` (`… :order-by [?x ?x]])`).
  `KNOWN_DRIFT` stays empty at bound 3 locally, bound 4 on the bench (`:asc`/`:desc` are
  new in the window template too: any drift there is a finding, fixed or filed).

### Step 16. Acceptance and envelope

- `tests/limit_acceptance_test.rs`: `top_k_10_over_10m_facts` over the same fixture,
  `[:find ?e ?v :where [?e :a ?v] :order-by [?v :desc] :limit 10]`: the rows are
  `9_999_990..10M` descending; peak RSS added under 32 MB (memory independent of the fact
  count; time follows the fact count: the input is read in full). The fixture is built once
  for both tests (a `OnceLock` path in a kept temp dir), not twice.
- `examples/perf_envelope.rs`: shape `top_k`, `[:find ?e ?a :where [?e :age ?a]
  :order-by [?a :desc] :limit 10]`, rows `10.min(e)`. Bench run at 100K, 1M, 10M
  (`--profile envelope`); numbers in the PR body and `BENCHMARKS.md`.

### Step 17. Docs and PR

`CHANGELOG.md` (Added: `:order-by`, `:offset`, `PRS-116`–`118`, under PR 1's `:limit`
entry), `docs/ERROR_REFERENCE.md`, `README.md`, `CLAUDE.md` (`sort.rs`, `order_by`/`offset`,
verifier template count 16, test count), `docs/TEST_COVERAGE.md`, `ROADMAP.md` (#306 done).
Docs site (`../minigraf-docs`, worktree): `content/datalog-reference.md` (the three sections,
`@since v3.0.0`), `content/comparison.md`; PR merged after PR 2. Review as step 9 (fmt,
clippy plain and `test-hooks`, bench suite plain and `test-hooks`, wasm32 build, size gate,
Codex). PR to `v3` "Part of #306"; after merge close #306 by hand and tick it in #383.
