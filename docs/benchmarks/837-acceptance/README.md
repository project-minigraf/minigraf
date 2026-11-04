# v3 acceptance execution audit (#837)

The revised scheduled route **executed all five jobs and seven intended tests**
successfully in [run 41729196180](https://github.com/project-minigraf/minigraf/actions/runs/41729196180).
This resolves X02's missing execution evidence for those tests. It does **not**
complete [#837](https://github.com/project-minigraf/minigraf/issues/837): nonlinear
and multistratum recursive scaling, custom transaction chunk bounds, and positive
confirmation that conditional RSS assertions executed remain unverified by this
route. Keep the issue open. This is an evidence audit; no engine, test, workflow,
release, or public API changes are made.

## Provenance and retained evidence

Audited on 2026-11-04. The event was `schedule`, selecting `0 17 * * *` from
the default-branch workflow. Actual execution began at 22:15 UTC on November 3;
the configured cron time is not a guarantee of on-time dispatch.

- Measured source: `c5863c79e4039e54c7fa67e0d7a8b8751f9b1599` (v3).
- Workflow: `4d65022ac963a10b582ac762c4ab2e73935c9115` (main). The run's
  `headSha` is this workflow SHA, **not** the measured source SHA.
- All five acceptance artifacts independently print both SHAs and the full command.
- Runner image: `ubuntu-26.04`, version `20261025.179.1`; native
  `x86_64-unknown-linux-gnu`, Rust `1.99.1 (58cf8f9c3 2026-10-13)`.
- [run.json](run.json) retains job/step status, timestamps, IDs, and URLs.
  [workflow.log](workflow.log) retains complete workflow output, including source
  selection, toolchain, and artifact uploads. [smoke.yml](smoke.yml) is the exact
  workflow at the workflow SHA. [SHA256SUMS](SHA256SUMS) covers these files and
  the five complete, unmodified acceptance outputs linked below.

The four integration targets and the storage test source are unchanged between
the measured SHA and audit base `948fdc93ea0866e5397a7e8f2d5d33aa04210466`.
Use the measured SHA when inspecting assertions. Cargo resolved dependencies at
runtime; logs preserve versions, but this workflow did not retain `Cargo.lock`.
This limits exact dependency replay. The evidence is copied here because the
workflow's artifacts expire after 30 days.

## Jobs, commands, and actual test inventory

Every acceptance job and its execution/upload steps succeeded. Each job ran on
its own hosted runner. Total job time includes setup, build, test, and upload;
command time includes the cold build. Harness time is the reported test duration.

| Job | Passed | Total / budget | Command / harness | Complete output |
| --- | ---: | --- | --- | --- |
| [recursive](https://github.com/project-minigraf/minigraf/actions/runs/41729196180/job/126241922775) | 1 | 2m 35s / 45m | 2m 15s / 47.25s | [recursive](acceptance-recursive.txt) |
| [memory-limit](https://github.com/project-minigraf/minigraf/actions/runs/41729196180/job/126241922561) | 3 | 1m 55s / 30m | 1m 44s / 30.63s | [memory-limit](acceptance-memory-limit.txt) |
| [limit](https://github.com/project-minigraf/minigraf/actions/runs/41729196180/job/126241922611) | 1 | 3m 10s / 30m | 2m 59s / 101.29s | [limit](acceptance-limit.txt) |
| [large-transaction](https://github.com/project-minigraf/minigraf/actions/runs/41729196180/job/126241922420) | 1 | 1m 21s / 30m | 1m 10s / 8.87s | [large-transaction](acceptance-large-transaction.txt) |
| [size-per-fact](https://github.com/project-minigraf/minigraf/actions/runs/41729196180/job/126241922435) | 1 | 1m 46s / 15m | 1m 38s / 51.14s | [size-per-fact](acceptance-size-per-fact.txt) |

Exact commands (default features; only large-transaction adds `test-hooks`):

```sh
cargo test --profile bench --test recursive_acceptance_test -- --ignored --nocapture
cargo test --profile bench --test memory_limit_acceptance_test -- --ignored --nocapture --test-threads=1
cargo test --profile bench --test limit_acceptance_test -- --ignored --nocapture
cargo test --profile bench --features test-hooks --test large_transaction_acceptance_test -- --ignored --nocapture
cargo test --lib size_per_fact_in_433_shape_1m -- --include-ignored --nocapture
```

The storage command uses the **unoptimized test profile**, not the bench profile.
The bench commands emit Cargo's warning that their `panic` setting is ignored.
No intended target ran zero tests: the integration summaries report 1/3/1/1
passed with zero failed, ignored, or filtered; storage reports one passed,
zero failed/ignored and 1,203 filtered out. Actual names and asserted bounds:

| Executed test | Fixture and unconditional assertions | Conditional RSS bound and printed observation |
| --- | --- | --- |
| `reach_5m` | 63 layers × 50 nodes, 155,000 edges; linear `reach` derives exactly 4,882,500 rows with cap 4,883,500; reopening a fresh fixture with the default 1,000,000 derived-fact cap must produce `INT-020` | Added RSS ≤ 256 bytes × 4,882,500 = 1,249,920,000 bytes; printed 168 bytes/derived fact, peak 955 MiB, before 172 MiB; query 38.61s |
| `cartesian_k13_streams` | 3 facts, 13 independent patterns; exactly 3^13 = 1,594,323 rows; 64-byte finite cap must produce `QRY-012` naming `HashJoin` | Added RSS < 64 MiB; printed 9,824 KiB added, peak 13 MiB; query 2.06s |
| `count_holds_no_rows` | 1,000,000 facts (10 checkpointed 100,000-fact chunks); count returns 1,000,000 under a 4 KiB meter cap | Added RSS < 32 MiB; printed 60 KiB added, peak 48 MiB; query 456.88ms |
| `grouped_count_1m_groups` | Same 1,000,000-fact shape; 1,000,000 groups without a finite cap; ordered count and sum also succeed under 32 MiB; floating `sum-distinct` exceeds that cap with `QRY-012` naming `Aggregate`; subsequent point query succeeds | No RSS assertion; unlimited grouped count printed 1.01s |
| `limit_and_top_k_over_10m_facts` | 10,000,000 integer facts via `LogWriter`, 10,000 facts per tx; limit 10 returns ten rows under 64 KiB in < 250ms; descending top-k returns exactly 9,999,999 through 9,999,990 under 1 MiB | Each query's added RSS < 32 MiB; limit printed 148 KiB / 558.44µs; top-k 0 KiB / 26.62s; peak 139 MiB; fixture build 74.41s |
| `one_million_fact_transact_holds_one_chunk` | 1,000,000 facts, alternating new entities and 32 hubs; hook reports 0 < held facts ≤ 65,536; one tx, WAL absent after commit, point/hub reads, index verification and reopen read succeed | **No RSS assertion**; hook observed 65,536 facts; printed peak 73 MiB / 52 MiB added, transact 7.09s |
| `storage::persistent_facts::tests::size_per_fact_in_433_shape_1m` | `MemoryBackend`, 80,000 entities × 10 attributes, 20% second values, 10% retract/reassert, 10 saves: 1,152,000 stored history facts; metadata count matches; count > 1,000,000 and integer `page_bytes / facts` ≤ 200 | No RSS assertion and no actual bytes/fact printed on success; this bounds allocated format pages, not a measured filesystem file or peak heap |

The storage assertion truncates integer division: it permits total bytes
strictly below 201 × 1,152,000 = 231,552,000, rather than enforcing an exact
floating-point ratio ≤ 200. There is no inferred observed storage byte count.
The limit/top-k fixture is built through `LogWriter`, so its construction does
not establish 10M-fact `transact` performance. Top-k has no explicit time bound.
These are fixed-size acceptance points, not measured scaling slopes.

Pinned assertions: [recursion](https://github.com/project-minigraf/minigraf/blob/c5863c79e4039e54c7fa67e0d7a8b8751f9b1599/tests/recursive_acceptance_test.rs),
[memory](https://github.com/project-minigraf/minigraf/blob/c5863c79e4039e54c7fa67e0d7a8b8751f9b1599/tests/memory_limit_acceptance_test.rs),
[limit](https://github.com/project-minigraf/minigraf/blob/c5863c79e4039e54c7fa67e0d7a8b8751f9b1599/tests/limit_acceptance_test.rs),
[transactions](https://github.com/project-minigraf/minigraf/blob/c5863c79e4039e54c7fa67e0d7a8b8751f9b1599/tests/large_transaction_acceptance_test.rs),
[storage](https://github.com/project-minigraf/minigraf/blob/c5863c79e4039e54c7fa67e0d7a8b8751f9b1599/src/storage/persistent_facts.rs).

## What the scheduled guard proves

The source job checks out v3 for the 17:00 schedule, resolves its SHA once, and
requires all four integration target files to exist. Missing any file sets
`acceptance=false` and **fails the scheduled source job**. Each acceptance job
then checks out that same SHA. The retained source log confirms the actual cron
expression and the five outputs confirm the common source. Main smoke was
intentionally skipped. `pipefail` propagates Cargo failure through `tee`;
artifact upload runs even on failure and rejects a missing output file.

That guard prevents silently skipping the entire matrix due to missing integration
files. It does **not** reject zero matching tests, a removed storage test, an
empty/cfg-disabled target, or a skipped conditional RSS assertion. Cargo can
succeed with zero tests. This audit reconciles the actual seven names and
nonzero counts for this run; it does not claim a future inventory guard exists.

## Remaining acceptance gaps and measurement limits

1. `reach_5m` has one recursive `reach` occurrence per rule body and no negation
   or higher stratum. Nonlinear and multistratum scaling did **not** run here.
   Ordinary correctness tests such as `linear_and_non_linear_transitive_closure`
   and `test_not_join_multi_stratum_chain` are outside these commands and cannot
   supply this route's missing scaling evidence.
2. The million-fact test uses only the default 65,536 chunk. The public
   `transaction_chunk_facts` option clamps to 1..=1,000,000, but custom sizes
   and clamp-boundary retained-state bounds did **not** execute here.
   `the_chunk_size_is_an_open_option` is an ordinary library test excluded by
   the storage name filter, not an executed acceptance test.
3. Recursion, memory, and limit RSS checks run only if writing `5` to
   `/proc/self/clear_refs` succeeds and `VmHWM` is nonzero. Their logs print
   measurements but omit the reset result, so **the run does not positively
   establish that those RSS assertions executed**. The separate transaction
   test reports reset failure explicitly (none here), but that is a different
   process/runner and cannot prove the other resets succeeded. Finite meter
   caps, error codes, row counts, hook bounds, and storage assertions above
   are unconditional. Non-Linux runs generally omit RSS checking entirely;
   all five targets exclude wasm32. No Windows/macOS/WASM measurement is claimed.
4. `/proc` high-water deltas are process-wide and allocator-sensitive. Saturating
   subtraction and rounded output, including top-k's 0 KiB, do not mean zero
   allocations. Memory tests run serially; other measured integration binaries
   each have one test. Hosted-runner logs do not prove physical-host idleness.
   No new heavy build or measurement was run locally for this audit.
5. Future enforcement needs explicit expected-test inventory checks (including
   storage), visible RSS measured/skipped status with a Linux gate, and named
   nonlinear/multistratum/custom-chunk acceptance fixtures with reviewed bounds.
   Verify a new run after those changes before closing #837. Passing these five
   existing jobs alone is insufficient.

C24's four `memory_meter/{count,hash_groups}/{unlimited,limited}/10k` IDs compare
no finite cap against an 8 MiB cap on a checkpointed fixture. Accounting remains
enabled in both; the 4 KiB exhaustion preflight is untimed. Nothing here adds an
accounting-disabled control or Bencher RSS/state enforcement. C19/C24/C30 latency
coverage remains distinct from these acceptance assertions and remaining gaps.

## Recheck the evidence

From this directory, `sha256sum -c SHA256SUMS` checks the retained files. To
retrieve the remote evidence again while it is retained:

```sh
gh run view 41729196180 --json jobs,displayTitle,event,headSha,createdAt,updatedAt,url
gh run download 41729196180 --dir /tmp/minigraf-837-recheck
gh run view 41729196180 --log
git show 4d65022ac963a10b582ac762c4ab2e73935c9115:.github/workflows/smoke.yml
git show c5863c79e4039e54c7fa67e0d7a8b8751f9b1599:tests/recursive_acceptance_test.rs
```

Compare every output's `Source`, `Workflow`, `Command`, executed test names and
summary against the pinned workflow and sources, not just the green job badge.
Compute job and command durations from `run.json` timestamps. Any new heavy
rerun or enumeration belongs on idle hardware; preserve its own SHA, toolchain,
dependencies and output, and remove temporary scratch after retaining evidence.
