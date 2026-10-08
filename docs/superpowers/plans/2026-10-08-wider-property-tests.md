# Wider property tests — Plan

Spec: `docs/superpowers/specs/2026-10-08-wider-property-tests-design.md` (#386).

1. **Shared model:** move pools, time grid, `Window`/`Effective`, `valid_at`,
   statement types and `Model` from `tests/model_based_test.rs` to
   `tests/reference_model/mod.rs`; `mod reference_model;` in both files. The
   model-based test must pass unchanged.
2. **Error fix:** the three existing properties fail on a write or query error.
3. **Histories:** `arb_history()` over `Transact`/`Retract`/`WriteTx`/`Checkpoint`,
   accepted statements only; apply to the model with `Model::commit`.
4. **Queries:** `Query { patterns, neg, pred, temporal, find }` with a renderer
   to EDN and `arb_query()` that keeps each pattern after the first joined.
5. **Reference evaluator:** visible facts, nested-loop join, `not`/`not-join`,
   predicates as `eval_binop`, projection; multiset or set comparison.
6. **Properties:** in-memory and file-backed (queries before and after reopen).
7. **Self-check:** each mutation in spec §8 applied to the reference must fail
   the test; then restore.
8. **Timing:** measure per-case time; set nightly `PROPTEST_CASES` for ~3 h and
   comment it in `property-tests.yml`.
9. **Docs:** test count in CLAUDE.md, README.md, docs/TEST_COVERAGE.md.
10. **PR** to `v3`, "Refs #386", monitor CI. Then the v2.x backport PR to `main`
    (spec §9).
