# SIGKILL crash test data check — Plan

Spec: `docs/superpowers/specs/2026-10-08-crash-kill-data-check-design.md` (#384).

1. **Generator and model** in `tests/crash_kill_test.rs`: a xorshift RNG seeded
   per round; `Model { live: BTreeSet<(e, a, v)>, history: Vec<BTreeSet<…>> }`;
   `next_tx(&mut rng, &model) -> (edn, new_live)`. The child and the parent
   both drive it, so they produce the same statements.
2. **Child entrypoint** reads the env (db path, log path, seed, variant), opens
   with the variant's options, loops: execute, append `k\n` to the log,
   checkpoint per variant.
3. **Parent round**: spawn, poll the log until a random target count, jitter,
   kill, reap, parse `N`.
4. **Checks**: tx count is `N` or `N + 1`; full scan, AEVT, EAVT, AVET equal
   `M(T)`; `:as-of` samples; `verify()`. Run on reopen, second reopen, and
   checkpoint + reopen.
5. **Self-check before relying on it**: break the model on purpose (drop the
   last transaction from `M`) and confirm the test fails; then restore.
6. Run per-PR rounds and `MINIGRAF_CRASH_KILL_ROUNDS=200` locally; clippy, fmt.
7. Docs: test count in CLAUDE.md, README.md, docs/TEST_COVERAGE.md (the
   replaced test keeps the count unless a test is added); comment in
   `crash-kill.yml`. No CHANGELOG entry beyond the v3.0.0 test-suite line, if
   one exists.
8. PR to `v3`, "Refs #384", monitor CI.
