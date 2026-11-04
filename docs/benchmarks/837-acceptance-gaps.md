# Acceptance enforcement follow-up (#837)

The historical audit is PR #847. This follow-up adds enforcement for the gaps it
found; it does not reinterpret that earlier successful run as broader coverage.
No production behavior, dependency, file format, public API, or release changes.

## Assertions added

- `nonlinear_closure_scaling`: checkpointed 256- and 512-node chains with
  `reach(x,z) :- reach(x,y), reach(y,z)`, returning 32,640 and 130,816 pairs.
- `multistratum_recursion_scaling`: the same chains with linear `reach`, a
  derived `blocked` terminal, `allowed = reach minus blocked destinations`, and
  `eligible = reach minus allowed`. Negative rule dependencies require three
  strata; the output is 255/511 paths to the terminal. Total derived relations
  are bounded by twice the closure size plus one.
- Both cases assert added RSS <= 16 MiB + 512 bytes per derived fact, each query
  < 60 seconds, and the larger query <= 16 times the smaller duration + 1 second.
  These are coarse two-size regression guards, not asymptotic complexity claims.
  Each size also must fail with `INT-020` under a 1,000-derived-fact cap.
- `custom_chunk_bounds`: configured chunks 0/1 with 17 facts; 2 with 257 facts;
  257/131,072 with 1,000,000 facts; 1,000,000/`usize::MAX` with 1,000,001 facts.
  The materialized-fact high-water mark must equal `min(facts, clamp(chunk,
  1, 1_000_000))`; the tx counter, index verification and full count after reopen
  also pass. This tests the public option, including both clamp boundaries.
- Existing query RSS assertions now fail on Linux if reset, VmRSS or VmHWM is
  unavailable, and print `RSS_CHECK ... measured` only after checking the bound.
  Non-Linux runs explicitly report skipped RSS. Existing strict `<` bounds are
  preserved. The transaction's printed RSS remains descriptive; its assertion
  is on held facts. The Linux helper has two normal tests for failure/boundary paths.
- Storage checks exact allocated page bytes <= 200 × 1,152,000 history facts,
  and prints `STORAGE_CHECK` with actual bytes and the maximum. It still uses
  `MemoryBackend`; it does not measure filesystem allocation or heap RSS.

## Workflow enforcement

`.github/acceptance/verify.py` requires the exact ten test names and nine measured
RSS records across five jobs, plus one fully passing harness summary per job.
Zero tests, missing/renamed/duplicated tests, ignored tests, incomplete output and
missing RSS evidence fail. Eight Python regressions cover these rejection paths.
The verifier comes from a separate checkout pinned to `github.workflow_sha`,
while the tests use the source SHA resolved by the source job. Recursive and
transaction tests now run serially; memory tests already did. Each artifact keeps
`Cargo.lock` alongside complete output for dependency replay.

The same workflow and verifier must land on `main` and `v3` in separate PRs.
Merge the v3 tests before activating the new default-branch gate: an older v3
source intentionally fails the new inventory. The existing missing-file guard
continues to fail a scheduled v3 checkout rather than silently skip its matrix.
The 45/30/30/30/15-minute budgets are unchanged. No Bencher memory metric or
accounting-disabled control is introduced.

## Validation status

The existing recursion baseline passed 37 tests. The two RSS helper tests and
all eight Python verifier regressions passed. Formatting and whitespace checks
pass. Full heavy acceptance validation is pending a dedicated Actions dispatch;
the historical audit PR is not being monitored. No bench machine is reserved.
