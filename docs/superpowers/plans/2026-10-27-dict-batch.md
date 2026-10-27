# #530 implementation plan

Spec: `../specs/2026-10-27-dict-batch-design.md`. Claude reviewed the original
spec/plan and the size-driven revision before implementation on 2026-10-27.
Base: `80120758`, branch `perf/530-dict-batch`.

1. Profile the 1M-fact attribute scan on the idle bench with the envelope profile
   and frame pointers. Preserve the baseline executable and identical fixtures.
2. Use the isolated `.worktrees/530-dict-batch` worktree from `origin/v3`.
3. Add dictionary/reader correctness and zero-cache read-count regressions.
   Establish the dense read-count failure with scalar lookup, then implement
   bounded dense leaf walks and sparse/small exact reads with shared decoding.
4. Add projection regressions before batching: mixed ids and fallback fail
   without it. Implement bounded stack hints, scalar-order cache insertion and
   all-or-nothing fallback. Guard unused entities/count before `end_pull`.
5. Maintain sorted distinct hints by bounded binary insertion. Model-check the
   buffer against a set at capacity boundaries, including descending and repeated
   inputs; repeated ids must not force sorting or consume duplicate slots.
6. Cover partial callback failure, missing/malformed requested values, skipped
   malformed values, wrong pages and cycles. Compare reader results and raw reads
   at 63/64 ids and spans `2c`/`2c+1`. Keep B+tree/key production code unchanged.
7. Run full default and `test-hooks` suites, CI clippy, coverage and release-size
   gates on the bench. Preserve the existing coverage floors and 1,230 KiB cap.
8. Rebuild the optimized envelope and the same public-API probe against both
   libraries. Assign subject ids before refs to make reverse refs strictly
   descending. Alternate baseline/change timings on an idle, NUMA-pinned bench;
   cover the original scan, point, sparse, density thresholds, descending/random/
   repeated refs and multiple columns on a 256 KiB worker stack. Profile apart.
9. Synchronize changelog, benchmarks/raw data, roadmap, module notes and test
   counts; cross-check README and docs-site claims without a version bump.
10. Obtain Claude's final review, resolve findings, push the `v3` PR follow-up and
    docs-site PR. Retain scratch until CI is green, save logs locally, then remove
    every task-owned bench fixture, executable, target and scratch directory.

CI follow-up: the initial generic batch walker exceeded the unchanged binary
budget and introduced coverage gaps. Claude approved reusing `LeafCursor` only
for dense chunks and scalar restart-point reads otherwise, removing the generic
walker, encoder helper and unused trait default. Test-only counting-backend reuse
avoids duplicate unexercised backend methods. The final design also avoids a
new general sort specialization in the embedded binary.

Small-query follow-up: the original 200-iteration cap produced only about 20 ms
of samples for the threshold cases. A longer probe and profiles did not reproduce
the apparent slowdown. Increase the public probe cap to 20,000 iterations and
repeat both A/B rounds; keep timing separate from profiling.

Claude approved the final code, evidence and documentation after reviewing the
64-id cutoff cost and repeated-ref variability. Final CI and bench cleanup follow
the push; no budget or coverage-floor exception was needed.
