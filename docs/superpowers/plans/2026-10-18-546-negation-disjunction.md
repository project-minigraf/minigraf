# Negation and disjunction on the streaming engine (#546) — Plan

Spec: `docs/superpowers/specs/2026-10-16-negation-disjunction-engine-design.md`. PRs target
`v3`. Neither uses a closing keyword for #473; #546 closes by hand after PR 2.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–9: one comparison rule in `not` bodies, sub-plans, `AntiJoin` | `feat/546-not` |
| 2 | 10–15: `or`/`or-join` on the engine, `Union`, `Distinct` | `feat/546-or` |

Each PR leaves `v3` with no unused code, updates the test counts and `CHANGELOG.md` for
what it ships, and has a docs-site PR (`../minigraf-docs`).

## Already done

- Spec §3.4 on the executor: #615 (PR #616) and #617 (PR #619) made every executor path
  bind the query's valid time for `:db/valid-at` in bodies, branches and rule
  definitions; the CHANGELOG entries exist. §3.6 item 2 needs no new entry. PR 1 only
  builds body plans with the query's `as_of` and `Valid`.
- #620 (PR #622): an `or` branch whose expression reads an outer variable is seeded
  (`branch_outer_reads`). PR 2's correlated branches cover that shape on the engine.

## Where the executor substitutes imported values today

Spec §3.1 needs every comparison of an imported value to use join equality. Four places
substitute the outer value into the body as a literal (`substitute_pattern` /
`substitute_value`), so a float matches within `f64::EPSILON`:

| Place | Path |
|---|---|
| `executor::not_body_matches` | `not` per-row path (bodies with an expression or pseudo-attribute; `evaluate_branch`) |
| `executor::not_clause_violated_via_rules` | `not` in a query that invokes a rule |
| `evaluator::evaluate_not_join` | every `not-join` per-row path, rule or not |
| `StratifiedEvaluator` negation loop (`evaluator.rs`, `'binding:` loop) | `not` in a rule body |

The exclusion-set paths (`apply_not_clause`, `apply_not_join_clause`) already compare
`normalize_value` keys, which is join equality (`Value`'s `Eq`/`Hash` treat `-0.0` as
`0.0` and NaN as equal to nothing, #545).

## Design decisions not fixed by the spec

1. **The executor seeds instead of substituting.** One helper,
   `executor::body_rows(body, seed: Binding, matcher) -> Vec<Binding>`: the body's
   patterns (and rule invocations, through `rule_invocation_to_pattern`, unsubstituted)
   are matched with `match_patterns_seeded(patterns, vec![seed])`, then its expressions
   applied. `seed` holds the imported values only: the body's variables bound in the
   outer row for `not`, the join variables for `not-join` (hidden metadata variables
   are already join variables, #594). The matcher joins a seeded variable through its
   hash path, keyed by `normalize_join_value`, so the comparison is join equality. The
   four places above call it; `substitute_pattern` stays for prepared queries and
   magic sets. The merge-the-outer-row-afterwards step goes, since the seed is already
   in every body row.
2. **Sub-plans start from a seed.** `plan.rs` splits into `plan(query)` and
   `plan_clauses(clauses, Start, …) -> Option<SubPlan>`, where `Start` is either
   nothing (the first pattern is a `Scan`, as today) or a seed: imported variables
   given slots `0..k` with their outer `Slots::kinds` (spec §4.5). With a seed, every
   pattern is a `Join`; the first one's outer side is the seed, so it is an
   `IndexNestedLoopJoin` when its entity is imported and otherwise a `HashJoin` built on
   the seed (`build_inner = false`). `Plan` gets `source: Source` (`Scan(ScanSpec)` or
   `Seed(width)`) in place of `scan`.
3. **One new leaf and one new trait method.** `Seed`: a leaf that returns one row (the
   imported cells) and ends. `Operator::rewind(&mut self, seed: &[Cell])`: start again
   from the beginning, reopening a closed operator; it passes `seed` down the outer
   (left) spine to the `Seed`; a join's inner `Scan` restarts without it. `Scan` keeps
   its resolved constants across rewinds. Every operator implements it; only correlated
   sub-plans call it.
4. **One body abstraction, used by `not` now and `or` in PR 2.** `engine/sub.rs`:
   - `Correlated { root: Box<dyn Operator>, imports: Vec<usize> }`: `imports` are the
     outer slots copied into the seed, in seed order. `first(row) -> bool` rewinds with
     the seed, pulls one batch, closes; `rows(row)` (PR 2) yields every body row.
   - `Hashed { root, keys: Vec<Key>, set or table }`: the body's standalone plan, run
     once on the first use, its rows keyed by the imported variables it binds
     (`JoinKey`, `Key::normalise` as a join's; a row with a NaN key cell is skipped). A
     rewind of the enclosing tree keeps the set: a closed body does not depend on the
     outer row.
5. **`AntiJoin` is one operator over either body.** `engine/anti.rs`: pulls an outer
   batch, keeps the rows whose body is empty (`Correlated::first` is false, or the key
   is absent from `Hashed`'s set; an empty key with a non-empty set removes every row,
   spec §4.2 step 3). It never deduplicates. Counters: its own, then the body's, then
   the outer side's.
6. **Closed.** `plan::closed(body)`: every variable read by an expression or a
   pseudo-attribute pattern (its `pseudo_source`) is bound by a stored-attribute
   pattern of the body. A body with no stored-attribute pattern is never hashed (there
   is nothing to scan once).
7. **Hash or correlated (spec §4.4).** Correlated when the body is not closed, has no
   stored-attribute pattern, or its first pattern in `optimizer::plan` order has a
   constant or imported entity; hashed otherwise. A test switch beside `REORDER`,
   `optimizer::SUBPLANS` (`Option<Mode>`, `None` = choose), forces one or the other,
   with `minigraf::__subplans(Mode) -> SubplanGuard` under `test-oracle` only. Forcing
   `Hash` on a body that cannot be hashed (not closed, or no pattern) leaves it
   correlated: the switch tests the choice, not shapes only one form runs.
8. **Body order counts imported variables as bound.** `optimizer::plan` scores an
   imported entity as a variable, so a body such as `[?x :a ?y] [?e :b ?x]` with `?e`
   imported could start with the unprobed pattern. `optimizer::plan_seeded(clauses,
   bound: &HashSet<String>)` scores a bound variable as a constant; `plan(c)` is
   `plan_seeded(c, &empty)`. Under the test switch off it keeps text order, as `plan`
   does. Cost only: answers do not depend on it (the differential test checks).
9. **Placement.** The engine runs `optimizer::plan`'s clauses in its order, then the
   deferred `not`s (sorted by `clause_cost` when reordering), as
   `execute_query_on_executor` does. A `not` with no required variable, which `plan`
   puts first, runs right after the first pattern instead: it reads nothing from the
   row, so its place does not change the rows (the engine has no row before the first
   pattern).
10. **Body binders.** Expressions in a body bind with `Out::New` for a body variable and
   `Out::Check` for an imported or already-bound one; body joins replace nothing
   (`JoinSpec::replace` empty). For a `not`, only whether a row exists matters, so
   replacing a cell could not change an answer; this matches the executor, whose body
   paths pass `replace = false`.

## Steps

### 1. Executor: seeded bodies (PR 1)
- `body_rows` (decision 1) in the four places of the table. `evaluate_not_join` keeps its
  signature (the evaluator and executor both call it).
- Unit tests: a float imported into each path matches by IEEE `==` (`1e-20` vs `2e-20`
  no longer equal; `0.0` vs `-0.0` equal); NaN matches nothing; a keyword imported into
  an entity position equals the ref; a body constant float still matches within
  `f64::EPSILON`.

### 2. Answer-change tests (PR 1)
- `tests/negation_equality_test.rs`: spec §3.6 item 1's two queries both return `:e1`,
  on `execute` and on the cursor, in memory and after a checkpoint; the same for `not`
  (body with and without an expression), a rule query's `not`, and a rule body's `not`.

### 3. `optimizer::plan_seeded` and the switch (PR 1)
- Decisions 7 and 8. Unit tests: an imported entity sorts first; text order with
  `REORDER` off; `SUBPLANS` is per thread and restored by the guard.

### 4. Sub-plans (PR 1)
- Decision 2: `Source`, `plan_clauses`, `Seed` (decision 3) in `engine/seed.rs`, and
  `rewind` on `Scan`, `Join`, `Apply`.
- `engine/tests.rs`: a seeded plan with an imported entity probes (counter
  `IndexNestedLoopJoin`, `rows_in` per seed); rewinding a scan after `close` reads
  again; a seeded `HashJoin` builds on the seed.

### 5. `AntiJoin` (PR 1)
- `engine/sub.rs` (decision 4, `Correlated::first` and `Hashed` set only; `rows` and the
  table come in PR 2), `engine/anti.rs` (decision 5), `Step::Anti(AntiSpec)`, and
  `plan` accepting `Not`/`NotJoin` (decisions 6, 9, 10). Queries holding `or`/`or-join`
  still return `None`.
- Module docs: `engine/mod.rs`, `plan.rs` (no longer "patterns and expression clauses").

### 6. Tests (PR 1)
- Routing (`RUNS`): `not`, `not-join`, deferred `not`, `not` with no required variable,
  pseudo-attribute and `:db/valid-at` bodies run on the engine; a body with a rule
  invocation does not; a query with `or` does not.
- `engine/tests.rs`: hashed vs correlated choice per decision 7 (counters name the
  operators); empty key removes every row; NaN outer value kept; multiplicity kept
  (duplicate outer rows both survive); a failing expression in a body keeps the row
  (spec §3.3), including an unknown predicate.
- `tests/plan_independence_test.rs`: the spec §6 negation matrix (floats near zero,
  `-0.0`, NaN, keywords and refs to the same entities, `:db/valid-at` and per-fact
  pseudo-attributes in bodies, placed and deferred), each run with reordering on/off ×
  `SUBPLANS` hash/correlated/choose; all must be the same bag.
- `tests/property_test.rs`: generated queries gain `not` and `not-join` bodies over the
  existing schema (the rerun with the switches off covers them).
- Existing tests that pinned the old float matching in bodies: each change listed in the
  PR body.

### 7. Measurement (PR 1)
- `examples/perf_envelope.rs`: `not_probe` (`[?e :name ?n] (not [?e :banned true])`,
  correlated) and `not_join_hash` (`[?e :score ?s] (not-join [?s] [?b :blocked ?s])`,
  hashed). On the bench machine, idle: 100K, 1M, 10M against `v3` before the PR; no
  existing workload regresses. Binary size (`binary-size.yml`).

### 8. PR 1 checks and docs
- `CHANGELOG.md` (v3.0.0): Breaking, spec §3.6 item 1 with its example; Internal, the
  engine runs `not`/`not-join` (`AntiJoin`, sub-plans).
- `cargo fmt`; clippy native, `--features wasm`, `browser`/wasm32; `cargo test`;
  `cargo test --features test-oracle`; fuzz crate `cargo check` if `reference_model`
  changes.
- Test counts: `CLAUDE.md`, `docs/TEST_COVERAGE.md`, `README.md`; `CLAUDE.md` module list
  (engine `seed.rs`, `sub.rs`, `anti.rs`; `plan.rs` scope; `optimizer.rs`
  `plan_seeded`/`SUBPLANS`).
- Docs site: `content/datalog-reference.md` (comparison inside `not`, `@since v3.0.0` /
  `@until v3.0.0`), `content/architecture.md` (engine operators).
- Codex review; comment on #546 with the PR 1 summary.

### 9. Before PR 2
- Record in the PR 1 body any shape the differential test found that the spec does not
  cover; file it as its own issue (v3.0.0) unless PR 1 fixes it.

### 10. Executor: `or` by join equality (PR 2)
- `apply_or_clauses` fast path: the hash join of branch rows onto incoming rows keys on
  `normalize_join_value` for `or` and `or-join` (spec §5.3). The dedup keys (`seen`,
  `seen_keys`, `seen_result`, `seen_proj`) stay exact `Value` keys (spec §5.2).
- Answer-change tests (spec §3.6 item 3): a keyword in a branch meets a ref outside, in
  both paths (with and without a `not` in the branch).

### 11. `Union` and `Distinct` (PR 2)
- `Correlated::rows` and `Hashed`'s table of new cells (decision 4).
- `engine/union.rs`: per outer row, every branch's merged rows (outer row, then the
  branch's new slots); a closed branch by `Hashed` lookup, the others `Correlated`.
  Imported variables per spec §2 (every outer variable for `or`, join variables for
  `or-join`, hidden metadata variables). The parser requires every `or` branch to bind
  the same new variables (`same set of new variables`), so every branch fills the same
  slots; `or-join` adds none.
- `engine/distinct.rs`: a hash set of rows keyed by `JoinKey::of(cell, false)` per slot,
  NaN never equal (so such rows are kept), as the executor's `Value` keys behave.
- `plan`: `Or`/`OrJoin` after every pattern, expression and placed `not`, before the
  deferred `not`s, in text order (spec §5.1); a branch holding a rule invocation keeps
  the query on the executor. Closed for a branch: decision 6, plus every variable a
  nested `not`, `or` or `or-join` needs from outside is bound by the branch.
- The #250 early stop is not reproduced: it is an executor optimisation.

### 12. Tests (PR 2)
- Routing for `or`/`or-join` (and nested `not`/`or` in branches); disjunction matrix in
  `tests/plan_independence_test.rs` with reordering and `SUBPLANS` both ways; Distinct
  collapses duplicate outer rows; property-test `or`/`or-join` branches.

### 13. Measurement (PR 2)
- `or_2` workload (two closed branches over different attributes); 100K, 1M, 10M against
  `v3` after PR 1.

### 14. PR 2 checks and docs
- As step 8, plus `CHANGELOG.md` §3.6 item 3; docs site `content/datalog-reference.md`
  (`or` comparison), `content/architecture.md` (`Union`, `Distinct`).
- Codex review; close #546 by hand after merge; check off the #546 boxes.

### 15. After #546
- `docs/superpowers/specs/2026-10-11-streaming-engine-design.md` stays as written (spec
  §9 lists the deviations). Next sub-issue of #473: #547.

## Bindings

No public API or error code changes; `minigraf::__subplans` exists only under the
`test-oracle` feature. No binding cascade.
