# #861 plan: configurable recursion round limit

Spec: `docs/superpowers/specs/2026-11-06-862-override-audit-design.md` §1–§5 (the `MaxIterations` rows), and the decisions posted on #861 (2026-11-06).
Branch `feat/861-max-iterations` off `v3` at `83fbaa8d` (after #870). One PR to `v3`, with `Refs #861, #862` in its body. Closing keywords do not fire on `v3`, so close #861 with `gh issue close` after the merge.

## Decisions (from #861)

- Default rounds per stratum: 1,000 → **1,000,000**, equal to `DEFAULT_MAX_DERIVED_FACTS`. A round that continues has derived at least one new fact, so rounds per stratum ≤ derived facts + 1 and, at equal defaults, the derived-facts limit binds first. v3 answer change: chains of more than ~500 edges now return rows instead of INT-020 (CHANGELOG Breaking changes).
- Rounds stay counted per stratum (`src/query/engine/recursive.rs`).
- `:max-iterations N` replaces the database value for that query, up or down.
- The INT-020 round message says how to raise the limit.
- Size gate: raise by the measured growth only if over 1,240 KiB and ≤ 4 KiB; more needs approval.

## 1. Limits plumbing

- `engine::Limits` (`max_iterations`, `max_derived_facts`, `query_memory`) becomes the one type for a query's limits. `DatalogExecutor` keeps a `Limits` instead of two fields; `set_limits(Limits)`; `PreparedQuery::limits: Limits`; `prepare_query(…, Limits)`.
- `OpenOptions::limits(&self) -> Limits` (private) replaces the four `(derived_fact_limit(), query_memory_limit)` pairs in `db.rs`.
- `DEFAULT_MAX_ITERATIONS` = 1,000,000, no longer `#[allow(dead_code)]`.

## 2. `OpenOptions::max_iterations`

Public field `max_iterations: usize` (default `DEFAULT_MAX_ITERATIONS`) and builder `max_iterations(n)`. `0` fails every rule query with INT-020 (the first round is over the limit); documented, not rejected, like `max_derived_facts(0)`.

## 3. Query keyword

- `DatalogQuery::max_iterations: Option<Count>`. Parser arm joins the limit-keyword arm with min 1: INT-012 duplicate, INT-013 missing, INT-014 for `< 1`, INT-015 non-integer, `$slot` → `Count::Slot`.
- Executor `iteration_limit(query) -> Result<usize>`: the literal, else the executor's.
- `Overridable::MaxIterations` (`:max-iterations`), in `DEFAULTS`; `check_overrides` checks it after `MaxResults`, before `QueryMemoryLimit`.
- Slots: `collect_slot_names` and `substitute` (via `resolve_limit_slot`, min 1).
- `PreparedQuery::with_max_iterations(n)`, ungated; an allowed keyword overrides it. Doc test.

## 4. INT-020 message

`Max iterations ({limit}) exceeded. A stratum's rules were still deriving new facts after {limit} rounds; raise the limit with :max-iterations N or OpenOptions::max_iterations.` (`ERROR_REFERENCE.md` updated.)

## 5. Grammar and verifier

`kw_max_iterations` + `max_iterations_section` in `grammar.pest`; `GRAMMAR.md` regenerated; corpus `valid/max_iterations.edn` (literal and slot), `invalid/semantic/max_iterations_zero.edn`, `max_iterations_twice.edn`, `invalid/syntax/max_iterations_missing.edn`; fuzz seed; verifier TOKENS 41 → 42 with new filler counts. Bound 4 on the bench machine.

## 6. Tests (TDD, `tests/limit_keywords_test.rs`, codes only)

1. A chain of 1,500 edges: the transitive rule returns rows at the default (was INT-020 at 1,000 rounds).
2. `OpenOptions::max_iterations(3)` → INT-020 on the 4-edge chain; the message names `:max-iterations`. `max_iterations(10)` passes.
3. `:max-iterations 3` lowers, `:max-iterations 10` raises on a `max_iterations(3)` database.
4. Gate: `allow_overrides([])` refuses `:max-iterations` with API-025; `DEFAULTS` allows it.
5. Validation: INT-012/013/014/015.
6. Slot: `$n` = 3 fails, 10 passes; `0` → INT-014; `Val(String)` → API-024.
7. Setter: `with_max_iterations(3)` fails, keyword overrides the setter, ungated under `allow_overrides([])`, applies to `PreparedQuery::query`.
8. Unit: `check_overrides` order with all four keywords; parser unit test for the slot form.
9. Rounds are per stratum: two strata of 3 rounds each pass at `max_iterations(4)` (a total count would fail).

## 7. Docs

`docs/CONFIGURATION.md` (round-limit row, `allow_overrides` table, limit-keyword table, setters), `ERROR_REFERENCE.md` (INT-012..015 causes, INT-020), `CHANGELOG.md` (Added; Breaking: default 1,000 → 1,000,000; Changed: INT-020 text), `CLAUDE.md` + `docs/TEST_COVERAGE.md` counts and module lines, docs site (`datalog-reference.md`, `performance-tuning.md`, `@since v3.0.0`).

## 8. Verification

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, full tests plain and `--features test-hooks` on the bench machine, wasm32 browser build, `--profile dist` size vs the 1,240 KiB gate. Codex review skipped (quota out until 2026-11-11). CI green.

## Deviations found while implementing

1. **Dead constructor removed.** `DatalogExecutor::new_with_limits` (crate-private, `#[allow(dead_code)]`, no callers) is deleted rather than given a third limit; `engine::Limits` gains `Default` (the `OpenOptions` defaults), which the executor constructors and `BrowserDb::prepare` use.
2. **Boundary.** A stratum runs one more round than the chain depth: the 4-edge chain derives in rounds 1–4 and round 5 finds nothing new, so it needs `max_iterations` 5 (4 fails). Tests use that calibrated value (`REACH_ROUNDS`).
3. **BrowserDb test.** A `#[wasm_bindgen_test]` checks `:max-iterations` runs by default (it is in `DEFAULTS`) and that a low value is INT-020.
4. **`allow_overrides_test` pinned `DEFAULTS`.** `defaults_read_back_and_compose` asserted the two-variant list; it now lists `MaxIterations` too (the intended change).
5. **Size.** `--profile dist` grows 624 B over base `83fbaa8d` (local stable 1.94: 1,300,968 → 1,301,592 B). CI measured the base at 1,263,928 B in #874, so about 5 KiB stays under the 1,240 KiB gate; no gate change.
6. **Bench.** The primary bench machine was saturated, so the suites ran on the alternate: plain 2,083 active / 21 ignored, test-hooks 2,118 / 24; verifier bound 4 clean (430 s).
