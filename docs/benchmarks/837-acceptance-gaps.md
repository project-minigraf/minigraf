# Acceptance enforcement follow-up (#837)

The historical audit is [PR #847](https://github.com/project-minigraf/minigraf/pull/847). This follow-up adds enforcement for the gaps it
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

## Validation evidence

Dedicated [run 41780988595](https://github.com/project-minigraf/minigraf/actions/runs/41780988595)
passed all five jobs on 2026-11-04. Both measured source and workflow SHA were
`4f7f1caa3ae2aa5f03b5e0a28b8dc3ceeef567bc`. This is a manual dispatch of the
revised workflow, not a claim that the default branch already runs these gates.
Merge [v3 PR #848](https://github.com/project-minigraf/minigraf/pull/848) before
[main PR #849](https://github.com/project-minigraf/minigraf/pull/849) to activate
the same workflow for the scheduled route.

| Job | Tests passed | Total job time / budget | Complete output and resolved dependencies |
| --- | ---: | --- | --- |
| [recursive](https://github.com/project-minigraf/minigraf/actions/runs/41780988595/job/126412284872) | 3 | 3m 12s / 45m | [output](837-acceptance-gaps/acceptance-recursive/acceptance.txt), [lock](837-acceptance-gaps/dependencies.lock) |
| [memory-limit](https://github.com/project-minigraf/minigraf/actions/runs/41780988595/job/126412284883) | 3 | 1m 41s / 30m | [output](837-acceptance-gaps/acceptance-memory-limit/acceptance.txt), [lock](837-acceptance-gaps/dependencies.lock) |
| [limit](https://github.com/project-minigraf/minigraf/actions/runs/41780988595/job/126412285096) | 1 | 2m 43s / 30m | [output](837-acceptance-gaps/acceptance-limit/acceptance.txt), [lock](837-acceptance-gaps/dependencies.lock) |
| [large-transaction](https://github.com/project-minigraf/minigraf/actions/runs/41780988595/job/126412284938) | 2 | 2m 54s / 30m | [output](837-acceptance-gaps/acceptance-large-transaction/acceptance.txt), [lock](837-acceptance-gaps/dependencies.lock) |
| [size-per-fact](https://github.com/project-minigraf/minigraf/actions/runs/41780988595/job/126412284935) | 1 | 1m 37s / 15m | [output](837-acceptance-gaps/acceptance-size-per-fact/acceptance.txt), [lock](837-acceptance-gaps/dependencies.lock) |

All ten tests passed with zero ignored or failed, and every job's verifier step
succeeded. All nine required `RSS_CHECK` records say `measured`. Exact commands,
features and source/workflow headers are in each output. The four integration
jobs use the bench profile; large transactions enable `test-hooks`; storage uses
the unoptimized test profile and a name filter with `--include-ignored`.

Observed results for the new fixtures:

- Multistratum: 255/511 rows; 2.483/16.241 seconds; RSS additions
  15,691,776/66,232,320 bytes against 50,201,088/150,733,312-byte bounds.
- Nonlinear: 32,640/130,816 rows; 0.638/5.883 seconds; RSS additions
  57,344/26,447,872 bytes against 33,488,896/83,755,008-byte bounds.
- Both recursion sizes and shapes produced `INT-020` under the 1,000-fact cap.
  The existing 4,882,500-row case added 824,057,856 bytes against 1,249,920,000.
- All seven custom chunk configurations held exactly the effective chunk size,
  including 1,000,000 held facts for the 1,000,001-fact upper-clamp controls.
- Cartesian and count RSS additions were 9,969,664 and 61,440 bytes. Limit/top-k
  additions were 167,936 and 0 bytes. All enforced caps and QRY-012 controls passed.
- Storage used 164,614,144 allocated page bytes for 1,152,000 history facts,
  below the exact 230,400,000-byte maximum.

The [complete workflow log](837-acceptance-gaps/workflow.log) preserves setup,
platform/toolchain and verifier execution; [run.json](837-acceptance-gaps/run.json)
preserves timestamps and job/step status. [SHA256SUMS](837-acceptance-gaps/SHA256SUMS)
protects all retained files, copied byte-for-byte before remote artifact expiry.
All five downloaded `Cargo.lock` files were byte-identical (SHA-256
`f92a0efd757d1201883e5383f87d0c97730c70a2456525ac1f46eeb2eab441f9`);
one shared `dependencies.lock` retains those bytes.
From the evidence directory, run `sha256sum -c SHA256SUMS`. From the repository
root, recheck any suite, for example:

```sh
python3 .github/acceptance/verify.py recursive docs/benchmarks/837-acceptance-gaps/acceptance-recursive/acceptance.txt
python3 -m unittest discover -s .github/acceptance -v
```

The existing recursion baseline passed 37 tests; the two RSS-helper tests and
all eight verifier regressions also passed. Formatting and source/documentation
whitespace checks pass; raw evidence retains its original whitespace. No full
Cargo-suite or PR-CI result is claimed here.

Measurements came from independent Linux hosted runners; physical-host idleness
is not observable. RSS deltas are allocator/process-sensitive, and zero added RSS
does not imply zero allocations. These fixtures establish their stated bounds at
the listed scales; they do not prove every shape, platform or asymptotic bound.
Accounting remains enabled in both C24 latency controls. No Bencher memory
metric or accounting-disabled measurement is claimed.
