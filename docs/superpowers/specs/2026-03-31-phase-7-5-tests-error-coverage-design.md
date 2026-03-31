# Phase 7.5 Design: Tests + Error Coverage

**Date**: 2026-03-31
**Phase**: 7.5
**Status**: Approved

## Context

Phase 7.1–7.4 added stratified negation (`not`/`not-join`), aggregation, arithmetic/predicate expressions, disjunction (`or`/`or-join`), and the `filter_facts_for_query` snapshot fix. Each feature has focused unit and integration tests, but:

- Cross-feature compositions (e.g. aggregation + bi-temporal, recursion + `not`) are untested
- Error paths are tested lightly at integration level (3–6 `is_err` checks per file)
- No coverage measurement tooling exists; the ≥90% branch coverage target in the ROADMAP is aspirational without a baseline

Phase 7.5 addresses all three: tooling, cross-feature tests, and error path coverage.

## Constraints

- **No new production code** — this phase is tests and tooling only
- **No new crate dependencies** — `cargo-llvm-cov` is a `cargo` subcommand installed separately, not added to `Cargo.toml`
- **CodeQL convention**: never use `{:?}` of `Result`/`Fact`/`Value`/`EdnValue` in assert messages (use plain strings, `unwrap()`, or `expect()`)

## Design

### Coverage Tooling

Install `cargo-llvm-cov` as a `cargo` subcommand. Run a baseline branch-coverage report across the entire codebase:

```
cargo llvm-cov --branch --html
```

The per-module numbers from the baseline report determine where the ≥90% target applies — not pre-declared. Modules with low baseline coverage that contain Phase 7 code are the primary targets. Re-run after each test stream to track progress. Ship when ≥90% is confirmed on the under-covered modules.

A note in `CONTRIBUTING.md` documents the command for future contributors.

### Stream 1: `tests/production_patterns_test.rs` (new file)

8–12 cross-feature integration tests modeling realistic embedder workloads. Each test uses `Minigraf::open()` (in-memory backend), transacts realistic data, runs a query, and asserts on result shape and values.

| Scenario | Features Combined |
|---|---|
| "Who is NOT in a department" with time-travel | `not` + `:as-of` |
| "Users without any completed orders" | `not-join` + aggregation (`:with`) |
| Headcount per department, excluding contractors | aggregation + `not` |
| Active employees as of date X, grouped by role | aggregation + `:valid-at` bi-temporal |
| Recursive org-chart with leaf-node exclusion | recursion + `not` |
| Department count via `or` (two data sources) | `or-join` + aggregation |
| Sum of salaries for people matching either condition | `or` + aggregation |
| Historical headcount at multiple timestamps | aggregation + `:as-of` in sequence |

### Stream 2: `tests/error_handling_test.rs` (new file)

~8 integration-level error path tests driving `Minigraf::execute()` with invalid programs. Each asserts `is_err()` and optionally checks the error message content.

| Error scenario | Error origin |
|---|---|
| Negation cycle in registered rules | Stratification at query time |
| `or`-with-negative-cycle in rules | Stratification at query time |
| `sum` over a string attribute | Aggregation post-processing |
| `not` with unbound variable (runtime path) | Safety check in executor |
| `or-join` with mismatched new variables | Parser |
| Aggregate on unbound variable | Parser |
| `not-join` with unbound join variable | Parser |
| `min`/`max` on boolean values | Aggregation post-processing |

### Stream 3: Inline unit tests (guided by `llvm-cov`)

After streams 1 and 2, re-run `cargo llvm-cov --branch --html`. Identify runtime error branches in `executor.rs` and `evaluator.rs` that remain uncovered. Add targeted `#[test]` functions to existing `#[cfg(test)]` blocks in those modules — using manually constructed `DatalogQuery` structs where needed to reach branches not reachable from the public API.

Estimated 5–8 new unit tests.

## Order of Execution

1. Install `cargo-llvm-cov`, run baseline, record per-module coverage numbers
2. Document coverage command (`CONTRIBUTING.md` or `Makefile`)
3. Write `tests/production_patterns_test.rs` (stream 1)
4. Write `tests/error_handling_test.rs` (stream 2)
5. Re-run `cargo llvm-cov`, identify remaining gaps
6. Write inline unit tests in `executor.rs` / `evaluator.rs` (stream 3)
7. Final `cargo llvm-cov` run — confirm ≥90% on target modules
8. Update `CLAUDE.md` (test count), `TEST_COVERAGE.md`, `ROADMAP.md`, `CHANGELOG.md`

## Success Criteria

- All existing 568 tests continue to pass
- `tests/production_patterns_test.rs` exists with ≥8 passing tests
- `tests/error_handling_test.rs` exists with ≥8 passing tests
- `cargo llvm-cov --branch` confirms ≥90% branch coverage on modules identified as under-covered by the baseline
- Coverage command documented for future contributors
