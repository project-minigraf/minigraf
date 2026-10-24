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

## PR 2 — `:order-by`, `:offset`, `Sort` (outline; detailed after PR 1 merges)

10. `Direction`, `DatalogQuery.order_by`/`offset`; parser for `:order-by [..]` (matching
    `FindSpec` by `==`) and `:offset`; `PRS-116`–`118`.
11. Prepared `:offset` slots (`resolve_count_slot(…, ":offset", …)`).
12. `aggregate.rs`: a row comparator over the full order (`value_order` per column, keys'
    directions, then the other columns ascending).
13. `engine/sort.rs`: full sort and top-k heap, metered (`QRY-012`), `keep = 0` closes without
    pulling.
14. `plan_with`: `Step::Sort` then `Step::Limit { offset, limit }`.
15. Tests: ordered lists equal with reordering on/off; every `value_order` class; ties;
    aggregates by `(count ?e) :desc`; windows; rule queries; cursors; prepared slots;
    `QRY-012` in `Sort`; grammar, verifier `SEMANTIC` witnesses for `PRS-116`, `117`, `118`.
16. Acceptance `top_k` (memory independent of the fact count) and envelope shape `top_k`.
17. Docs: CHANGELOG, ERROR_REFERENCE (`PRS-116`–`118`), README, CLAUDE.md, TEST_COVERAGE,
    ROADMAP; docs site `content/datalog-reference.md` and `content/comparison.md`
    (`@since v3.0.0`), PR to minigraf-docs merged after PR 2. Close #306 by hand.
