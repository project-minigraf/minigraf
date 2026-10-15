# Plan-independent answers (#594, #596) — Plan

Spec: `docs/superpowers/specs/2026-10-15-plan-independent-answers-design.md`. PRs target
`v3`. #594 and #596 close by hand (`gh issue close`) when their PR merges. No closing
keyword for #473.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–6: the `REORDER` switch, the differential test, expressions unify (#596) | `feat/596-unify` |
| 2 | 7–13: pseudo-attribute sources at parse time, PRS-081 (#594) | `feat/594-pseudo-sources` |
| 3 | 14–17: every build reorders; the `wasm` feature becomes a no-op | `feat/reorder-everywhere` |

Each PR leaves `v3` with no unused code, and updates the test counts and `CHANGELOG.md`
for what it ships. Each has a docs-site PR (`../minigraf-docs`).

## Where the native build differs from `wasm` today

"`wasm`" here is the Cargo feature, not the target. No first-party build enables it:
`minigraf-wasm` depends on `minigraf` with `browser` only, and `wasm.yml` builds and tests
wasm32 without it, so the browser and WASI builds already reorder. Only builds that ask
for `--features wasm` themselves, and docs.rs (`all-features = true`), keep source order.

Every place is a `#[cfg(not(feature = "wasm"))]` block or a cost choice:

| Place | What it does natively |
|---|---|
| `optimizer::plan` | stable sort of patterns by `selectivity_score`; `not`/`not-join` sort by `clause_cost` before placement |
| `execute_query_on_executor`, `execute_query_with_rules` | deferred `not`/`not-join` sort by `clause_cost` |
| `evaluator.rs` (stratified loop, rules with negation) | `not` and `not-join` bodies sorted by `clause_cost` |
| `apply_or_clauses` (`or`, `or-join`) | branches sorted by `branch_cost`; the #250 short-circuits (plain `or` when it is a pure filter; `or-join` once every required key has a match) |
| `engine/plan.rs` | `HashJoin` build side by `branch_cost` (not `wasm`-gated: it runs under `wasm` too) |

## Design decisions not fixed by the spec

1. **The switch is a function, not a `cfg`.** `optimizer::reorder() -> bool` replaces each
   `#[cfg(not(feature = "wasm"))]` above with `if optimizer::reorder()`. It is
   `!cfg!(feature = "wasm")`, and, under `cfg(test)` or `test-oracle`, also the thread-local
   `REORDER` (default on). Release builds get a constant, so the code they compile is
   today's. `selectivity_score` loses its `cfg` and the `allow(dead_code)` attributes on
   the cost functions go.
2. **The #250 short-circuits are under the switch too.** Spec §4.5 lists the sorts; the
   short-circuits are also native-only, and an answer that depends on them is a bug by
   spec §1, so the differential test should cover them.
3. **Switch off builds the `HashJoin` on the inner pattern.** "As under `wasm`" would keep
   the cost choice, which `wasm` makes today; a fixed side gives the differential test a
   second build side to compare. Only row order can differ.
4. **Test hook.** `minigraf::__reorder(on: bool) -> ReorderGuard`, `#[doc(hidden)]`, under
   `test-oracle` only; the guard restores the previous value on drop, so a panicking test
   leaves the thread as it found it.
5. **One unify helper.** `executor::bind_expr(b: &mut Binding, var: &str, v: Value, replace:
   bool) -> bool`: unbound → insert, keep; bound → keep only if `normalize_join_value`
   of both is equal, and insert `v` when `replace`. All four executor paths and the
   evaluator call it (`apply_expr_clauses_in_evaluator` keeps its own loop but uses the
   helper). `normalize_join_value` becomes `pub(crate)`.
6. **`Binders` records expressions in text order.** `Binders::new` also collects each
   top-level `Expr` with a binding and its `:where` index; `by_var` becomes
   `HashMap<String, usize>`. `Binders::expression(&mut self, clause: &WhereClause) -> bool`
   takes the expression by value equality (`Expr: PartialEq`, as patterns are found
   today), records its position when the variable is new or the expression is earlier
   in the text than its binder, and returns whether it replaces (spec §3.5). Paths
   without `Binders` (`or` branches, `not` bodies, rule bodies) pass `replace = false`.
7. **The engine's `Step::Bind` gets a mode.** `Bind(expr, vars, slot, Out)` with
   `Out::New`, `Out::Check` or `Out::Replace`, fixed by the planner from `Slots` and
   `Binders`. `Check`/`Replace` compare with `cell::join_eq(…, normalise = true)`, as the
   executor normalises every bound variable. NaN never matches.
8. **The model-based and property reruns live in the test files.** `tests/reference_model`
   is compiled by the `ops_sequence` fuzz target too, which has no `test-oracle` feature. PR 3 changes no binding: none enables `wasm`.
   The driver takes the query runner as a parameter; `model_based_test.rs` and
   `property_test.rs` pass one that, under `test-oracle`, also runs with the switch off
   and requires the same bag.
9. **(PR 2) Bodies that read an outer source match seeded.** `not_body_matches` and
   `evaluate_not_join` match substituted patterns from an empty seed and merge the outer
   row afterwards. A pseudo-attribute pattern needs the hidden variable while it
   matches, so when a body holds one, they seed with the outer row
   (`match_patterns_seeded`). The `apply_not_clause`/`apply_not_join_clause` exclusion-set
   fast paths and the `or` empty-seed fast path are skipped for such bodies (spec §4.1).

10. **(PR 3) Reorder on every build; keep `wasm` as a no-op.** The feature was added in 6.1
   for small browser datasets and "deterministic, debuggable" order (`selectivity_score`'s
   comment). The sort costs microseconds per query against a scan it can avoid, the
   native order is deterministic too (stable sort, static scores), and after PRs 1–2 the
   plan cannot change an answer. Removing the feature from `Cargo.toml` would break any
   build that names it, so it stays declared, does nothing, and is marked deprecated
   (CHANGELOG; removal is a v4 question). PR 3 waits for PR 2: before it, dropping the
   gate changes answers for #594's shapes under `--features wasm`.

## Steps

### 1. The switch (PR 1)
- `optimizer::reorder`, `REORDER`, `ReorderGuard`, `minigraf::__reorder` (decisions 1, 4).
- Every place in the table above goes through `reorder()` (decisions 2, 3).
- Unit tests: `plan` keeps source order with the switch off; the switch is per thread.
- Clippy with `--features wasm` and for `wasm32` (no dead code, no unused `mut`).

### 2. Differential test (PR 1)
- `tests/plan_independence_test.rs`, `#![cfg(all(feature = "test-oracle", not(target_arch = "wasm32")))]`.
- A helper runs one query through `Minigraf::execute` and `Minigraf::query` (cursor,
  drained) with the switch on and off; all four answers must be the same bag (rows
  sorted by a value key built without `Debug`). Under `test-oracle` the engine's oracle
  also compares engine and executor on each run.
- The matrix: patterns in both text orders, three-way joins, cross products,
  expressions (filter and binding), rule invocations (recursive and with `not`),
  `or`/`or-join` (including the #250 pure-filter and covered-key shapes), `not`/`not-join`
  (placed and deferred), keyword vs ref, `0.0`/`-0.0`, long strings, `:as-of` and
  `:valid-at`; in-memory and file-backed after a checkpoint.
- #594 shapes (a per-fact pseudo-attribute with more than one candidate pattern, or none
  before it) are left out, with a comment naming #594.
- A failure the matrix finds that is not #594 or #596 gets its own issue (v3.0.0).

### 3. Expressions unify in the executor and evaluator (PR 1)
- `bind_expr` (decision 5) in: the two plan loops (`execute_query_on_executor`,
  `execute_query_with_rules`, with `Binders::expression`, decision 6),
  `apply_expr_clauses` (`evaluate_branch`, `not` bodies), `apply_expr_clauses_in_evaluator`
  (`RecursiveEvaluator`, `evaluate_not_join`), and the stratified loop's `Expr` arm.
- `Binders::pattern`: an expression's position now counts like a pattern's (no more
  `None` = "fixed").
- Unit tests per path: keyword/ref pair (and which value is returned, both text orders),
  `-0.0`/`0.0`, NaN, `1` vs `1.0`, `[(+ ?v 1) ?v]`, two expressions on one variable, a
  failing expression.

### 4. Expressions unify in the engine (PR 1)
- `Step::Bind` with `Out` (decision 7); planner picks the mode. Module doc of `plan.rs`
  and `Step::Bind`'s doc no longer say "overwrites".
- `engine/tests.rs`: the same cases on the engine, plus the oracle agreeing.

### 5. Reruns in the generated tests (PR 1)
- Decision 8: the #385 model-based test and the #386 query property tests.

### 6. PR 1 checks and docs
- `CHANGELOG.md` (v3.0.0, Breaking changes): spec §6 rule 4, with #596's example.
- `cargo fmt`; clippy native, `--features wasm`, `browser`/wasm32;
  `cargo test`; `cargo test --features test-oracle`; `cargo test --features wasm`.
- Fuzz crate builds (`cargo check` in `fuzz/`), since `reference_model` changed.
- Test counts: `CLAUDE.md`, `docs/TEST_COVERAGE.md`, `README.md`; `CLAUDE.md` module list
  (`optimizer.rs` switch; engine `filter.rs` `Bind`).
- Docs site: `content/datalog-reference.md` expression clauses, `@since v3.0.0` /
  `@until v3.0.0`.
- Codex review; comment on #596; close #596 by hand after merge.

### 7. Hidden variables on `Pattern` (PR 2)
- `Pattern::meta: Vec<(PseudoAttr, String)>`, `Pattern::pseudo_source: Option<String>`,
  empty from every constructor. `prepared::substitute`, `substitute_pattern` and
  magic-sets copy them unchanged (check every struct literal of `Pattern`).

### 8. `resolve_pseudo_sources` and PRS-081 (PR 2)
- `parser.rs`: the pass of spec §4.1 over `:where`, rule bodies and prepared templates,
  with the enclosing-list stack and the §3.2 crossing rule; appends hidden variables to
  `not-join`/`or-join` join variables it crosses. Runs before `check_not_safety`. Entity
  terms compare as spec §2 (`edn_to_entity_id` for constants, slot names for `$slot`).
- `ErrorCode::Prs081` in `src/error.rs` (category, message, registry list) and
  `docs/ERROR_REFERENCE.md` (table row and section).
- Parser tests: spec §7 PR 2 list.

### 9. Executor (PR 2)
- `matcher.rs`: a source binds its `meta` variables on both join paths (hash and nested
  loop) and in `match_pattern`; a `pseudo_source` pattern is a bind-or-compare on the
  row, never a scan. The `__fvf_`/`__fvt_`/`__ftc_`/`__fti_` keys and their fast path and
  fallback go; so do the `__f` retains in `apply_not_clause`/`apply_not_join_clause`.
- `optimizer::plan`: pseudo-attribute patterns are placed like expressions (spec §4.2);
  a `not` whose body reads a hidden variable waits for its source.
- Decision 9 for `not`/`not-join` bodies and `or` branches.

### 10. Engine (PR 2)
- `plan.rs`: a source's scan or join fills one metadata column per `meta` entry, the
  hidden variable's slot. `Step::Pseudo { slot, value }` binds or compares. `pseudo_forms`,
  the per-row `pick` and the `[e _ _]` form for per-fact attributes go; `:db/valid-at`
  keeps its `[e _ _]` join. Module doc and `CLAUDE.md` module list updated.

### 11. Tests (PR 2)
- Spec §7 PR 2 list; the pseudo-attribute shapes join the differential matrix.
- Existing tests that relied on the fallback or plan order (`temporal_metadata_test.rs`,
  `point_query_history_test.rs`, `statement_order_test.rs`, `error_codes_prs_test.rs`,
  the matcher, executor, parser, `db.rs` and engine unit tests; spec §7 names
  `bitemporal_test.rs` and `join_rules_test.rs`, which hold no per-fact pseudo-attribute):
  each change listed in the PR body.
- `cargo test --features wasm` runs #594's examples without reordering.

### 12. Measurement (PR 2)
- #394 envelope on the bench machine, idle: `join_2_pseudo` at 1M no slower than after
  #545 PR 3 (56 µs at 10M); point queries unchanged. Binary size (`binary-size.yml`).

### 13. PR 2 checks and docs
- As step 6, plus `CHANGELOG.md` spec §6 rules 1–3 and PRS-081.
- Docs site: `content/datalog-reference.md` pseudo-attributes (`@since`/`@until v3.0.0`),
  `content/architecture.md` query-engine section.
- Codex review; close #594 by hand after merge; next is #546.

### 14. Remove the gate (PR 3, decision 10)
- `optimizer::reorder()` is `true`, or the thread-local `REORDER` under `cfg(test)` /
  `test-oracle`. Every `cfg(feature = "wasm")` / `cfg_attr(feature = "wasm", …)` in
  `src/query/` goes. `wasm = []` stays in `Cargo.toml` with a comment saying it is a no-op.
- The `#[cfg(not(feature = "wasm"))]` optimizer unit tests lose the gate; tests that
  checked source order under `wasm` set the switch off instead.

### 15. Tests (PR 3)
- The differential test stays the only source-order path and must still pass.
- The plan-dependent shapes from PRs 1–2 pass natively, on wasm32 (`wasm.yml`, WASI), and
  with `--features wasm` (now the same plan).

### 16. Measurement (PR 3)
- #394 envelope at 100K and 1M with `--features wasm`, before and after: no slower on any
  shape (expect the multi-pattern shapes to gain). Binary size for `browser`/wasm32
  (`binary-size.yml`).

### 17. PR 3 checks and docs
- `CHANGELOG.md` (v3.0.0): the `wasm` feature no longer changes query planning;
  deprecated.
- `CLAUDE.md` module list (`optimizer.rs`: "disabled under `wasm` feature" goes),
  `README.md` and the docs site (`content/architecture.md` optimizer section,
  `content/use-cases.md` browser notes) under `@since v3.0.0` / `@until v3.0.0`.
- The 6.1 and 2026-05-15 optimizer specs stay as they are (frozen records).
- Codex review.

## Bindings

PRS-081 is a new code but bindings surface codes as strings (as PRS-080 did), and no
public API changes, so no binding cascade. `minigraf::__reorder` exists only under the
`test-oracle` feature. PR 3 changes no binding: none enables `wasm`.
