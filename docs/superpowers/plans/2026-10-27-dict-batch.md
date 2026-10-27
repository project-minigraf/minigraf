# #530 implementation plan

Spec: `../specs/2026-10-27-dict-batch-design.md`. Claude approved the spec and plan
on 2026-10-27 before implementation. Base: `80120758`, branch `perf/530-dict-batch`.

1. Profile the current 1M-fact attribute scan on the idle benchmark server, envelope profile
   and frame pointers. Keep a baseline executable and identical fixtures.
2. Create the ignored `.worktrees/530-dict-batch` worktree from `origin/v3`.
3. Add B+tree batch correctness/read-count regressions. Establish failure using
   scalar fallback, then implement child-grouped descent with scalar `leaf_get`.
   Observed: the dense read-count assertion failed with scalar lookup; all three
   B+tree regressions then passed with grouped descent.
4. Add fixed DICT key and UUID batch tests; implement bounded key encoding,
   `DictReader::uuids_of`, the default `KeySource` method and on-disk override.
5. Add projection regressions. Observe that mixed-id and fallback tests fail
   before batching; implement bounded stack hints, scalar-order insertion and
   fallback. Add the unused-entity/count guard before `end_pull`.
6. Run full default and `test-hooks` tests, CI clippy, and formatting checks on the
   bench (formatting may also run locally). Keep heavy work off the laptop.
7. Build the optimized envelope and matching probe, then alternate baseline and
   change timings on an idle machine. Check sparse/random/repeated projections,
   a 256 KiB worker stack and point reads. Profile separately from timing.
8. Synchronize changelog, benchmark evidence, roadmap, architecture notes and
   test counts; cross-check README and docs-site claims. Do not bump versions.
9. Get Claude's final review, address findings, and verify affected work before
   opening the `v3` PR. Save logs locally, then remove every task-owned bench
   fixture, executable, target and scratch directory.

Implementation review follow-up: compact duplicate hint cells before enforcing
the distinct-id cap (regression observed failing, then passing); count both scalar
and speculative reads for hidden entity cells; test every integer byte boundary;
guard the on-disk override with a zero-cache read-count test. Fsync-heavy broad suites run sequentially after contention; timing runs are
pinned to one NUMA node on the idle benchmark server. Incomplete interrupted runs do not count as verification.

The final review approved the implementation conditional on remaining checks.
Its repeated-id cost note was also addressed: a sort-count regression failed
before the buffer reused its sorted prefix, then passed with all six projection
tests. Full default and test-hooks runs passed; final focused checks cover this
last local optimization.
