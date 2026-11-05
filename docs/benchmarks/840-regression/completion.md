# Regression-history follow-up

The production history fix is merged in [main #853](https://github.com/project-minigraf/minigraf/pull/853)
(`25fd6fb9bdb9bc69021a217c1f755e1780c0eb07`) and
[v3 #854](https://github.com/project-minigraf/minigraf/pull/854)
(`fc0a61545addd84eb0d6c300965082a6bf189101`). Both passed their required CI before
merge. Persistent main/v3 uploads now omit main start-point inheritance; ad hoc
comparison branches retain it. This preserves the existing v3 head and its independent observations.

The [initial audit](README.md) is a frozen pre-fix snapshot. Its null comparisons
and notification-policy fixtures describe that snapshot, not the subsequent
live checks below. This follow-up completes the evidence for #840.

## Fresh independent measurements

All **427 native and 26 binding IDs** now have finite numeric comparison baselines
and upper limits on their intended v3 testbeds. Each ID is traced to at least
**three independent v3 workflow runs** on head
`01a1a84c-e947-745c-98e7-018e0c2245a3`. The original 244 native and 26 binding
comparison gaps are closed. [comparison-inventory.json](comparison-inventory.json)
records every ID's value, baseline, upper limit, testbed, source hash, report UUID
and independent run IDs; [completion-summary.json](completion-summary.json)
records the five new dispatches.

| Run | Scope | IDs | Measured core SHA | Workflow SHA | Alerts |
| --- | --- | --- | --- | --- | --- |
| [41960942244](https://github.com/project-minigraf/minigraf/actions/runs/41960942244) | Full v3 | 453 | `cdad3e7270d0e5e791cd48f47a4ed35ff50df6a4` | `25fd6fb9bdb9bc69021a217c1f755e1780c0eb07` | 0 |
| [41965724402](https://github.com/project-minigraf/minigraf/actions/runs/41965724402) | Full v3 | 453 | `5a0ff549271084018c7d4e6050658e64e7e6432e` | `25fd6fb9bdb9bc69021a217c1f755e1780c0eb07` | 3 |
| [41970954512](https://github.com/project-minigraf/minigraf/actions/runs/41970954512) | C v3 | 6 | `5a0ff549271084018c7d4e6050658e64e7e6432e` | `44a8edb949a0b5a1e0b5cbc096ee8e7478216817` | 0 |
| [41971233145](https://github.com/project-minigraf/minigraf/actions/runs/41971233145) | UDF main | 4 | `25fd6fb9bdb9bc69021a217c1f755e1780c0eb07` | `44a8edb949a0b5a1e0b5cbc096ee8e7478216817` | 0 |
| [41971706654](https://github.com/project-minigraf/minigraf/actions/runs/41971706654) | UDF v3 | 10 | `5a0ff549271084018c7d4e6050658e64e7e6432e` | `44a8edb949a0b5a1e0b5cbc096ee8e7478216817` | 0 |

Every selection is nonempty and disjoint within its run. The offline verifier
requires selected = raw executed = raw uploaded = report/API IDs, checks core,
binding and workflow provenance, and verifies every stored 0.999/30 model.
Native uses `ubuntu-latest`; bindings retain their four
`binding-<runtime>-ubuntu24-v1` testbeds.

Two complete production-workflow v3 dispatches add fresh observations to the
retained original scheduled run. C needs one additional real C-only dispatch:
its original scheduled report belongs to the previous head and cannot count
here. All measurements use normal benchmark code and source pinning on
GitHub-hosted runners; none replay a log or upload a fixture. Existing inherited
main observations are excluded from the independent-v3 counts.

The perf query starts at the earliest actual v3 report timestamp, 2026-11-04
20:42:54 UTC. That report started one second before its upload created the head
at 20:42:55. A head-creation cutoff initially excluded its six B-tree observations;
`initial-query-window/` retains that diagnostic snapshot. The verifier derives
the corrected cutoff from the original API reports and still requires every
counted observation to have known, independent v3 provenance.

The real main UDF probe advances Bencher's measured main version from 84
(`e7923eb710b1e1d8aff517721643c0b0b1f2cb7a`) to 85
(`25fd6fb9bdb9bc69021a217c1f755e1780c0eb07`); a subsequent v3 UDF upload
preserves the same v3 head. Six before/after branch API snapshots
and both actual reports verify preservation across a measured main advance. The disposable probe workflow and its exact source
revision are retained in the archive.

These are initial numeric comparisons, not thirty completed independent runs.
With only a few observations, the limits can be wide; future independent nightly
runs continue to build the history. No latency percentage budget is claimed.

Reported latency alerts (retained as real results):

- Run 41965724402: `concurrent_file/serialized_writers/100k_16t`
- Run 41965724402: `concurrent_file/serialized_writers/100k_2t`
- Run 41965724402: `concurrent_file/serialized_writers/100k_4t`

| File-writer case | Value (ms) | Baseline (ms) | Upper limit (ms) |
| --- | --- | --- | --- |
| 100k, 2 threads | 1.5517 | 0.417969 | 1.0169 |
| 100k, 4 threads | 3.2361 | 0.85936 | 2.11223 |
| 100k, 16 threads | 10.804 | 3.58459 | 9.25215 |

The retained production job log records no alerts in the preceding run and
suppresses notification under the two-run policy. These are real threshold
crossings; this audit does not establish their performance cause. The verifier
checks the actual reporting-step outcome and timestamped suppression message.

Expansion to Java/JVM, WASI and qualified Swift/iOS simulator or Android emulator
lanes is tracked separately in [#858](https://github.com/project-minigraf/minigraf/issues/858).
The #839 platform restriction applies only to that manual evidence task; mobile
nightly inclusion depends on measured agreement with physical-device workloads.
This follow-up verifies the existing matrix and does not claim coverage of those
additional runtimes.

## Actual scheduled routes

The initial archive retains actual main and v3 `schedule` events, measured and
workflow hashes, and exact nonempty/disjoint selected, executed and uploaded
sets. Main covers 177 native IDs; v3 covers 427 native and 26 binding IDs across
all four binding jobs. The history fix changes upload ancestry, not schedules,
selection, measured-ref resolution or notification routing. The fresh runs are
explicitly dispatches; they are not presented as new scheduled events. The
observed schedules were delayed relative to their nominal 02:00/14:00 UTC crons.

## Live notification delivery

[The live summary](live-summary.json) records 16 job scenarios: eight for each
unchanged production notification body. The temporary workflow used source
`437d81e3566c5bb052981d65304e938873ca5eed` and isolated Bencher branches
`audit-840-20261105` and `audit-840-20261105-other`, dedicated native/binding
testbeds, and IDs under `audit/840/20261105/`. No synthetic value was written to
main/v3 or their production testbeds.

| Scenario | Actual result for both native and binding |
| --- | --- |
| First alert | Real Bencher alert; no issue/comment |
| Intervening different branch | Real alert; no issue/comment |
| Repeated same ID/branch/suite | Issue created; previous matching branch selected despite intervening other branch |
| Different suite | No previous matching job; no issue/comment |
| Reset first alert | No delivery after the intervening different suite |
| Different benchmark ID | No repeated-ID delivery |
| Previous-log lookup failure | Actual GitHub 404; warning and empty prior-alert set; no delivery |
| Failed upload | Actual Bencher 404; immediate comment; completeness audit fails as intended |

The repeated-alert run created [#856](https://github.com/project-minigraf/minigraf/issues/856)
and [#857](https://github.com/project-minigraf/minigraf/issues/857). Failed uploads
added [the binding comment](https://github.com/project-minigraf/minigraf/issues/856#issuecomment-6648972167)
and [the native comment](https://github.com/project-minigraf/minigraf/issues/857#issuecomment-6648970917).
Independent GitHub reads matched all four exact message bodies. The expected
failed-upload workflow failure is retained, not classified as a failed experiment.

For isolation, the driver redirected the notification body's hardcoded physical
workflow lookup from `main` to `audit/840-live-checks`. The script bodies themselves
are byte-checked against the production workflow. These checks establish real
GitHub delivery and prior-job log retrieval within that isolated scope; they do
not establish unmodified default-branch lookup or receipt in a human mailbox.

## Cleanup and durable evidence

After the original source/API evidence was committed and pushed, cleanup run
[41963024134](https://github.com/project-minigraf/minigraf/actions/runs/41963024134)
deleted exactly 34 synthetic reports, four thresholds, two branches, two testbeds
and four benchmarks. Guard tests rejected foreign production resource names.
All 46 resources returned 404 after deletion, verified again by an independent
reader. Both test issues are closed. The notification workflow ref is deleted
and returns 404; all five production v3 threshold models still have 0.999/30.
The history-probe workflow ref was also deleted and independently returned 404
only after its source and evidence were pushed in `9e1a5e8ad`. Historical Actions
run records remain as an audit trail; real Bencher observations are retained.

`completion.tar.gz` retains all five fresh runs, their exact workflow sources,
63 public report responses, complete active-head queries, six head snapshots,
the query-window diagnostic, production alert logs and probe-ref cleanup proof.
[completion-verification.txt](completion-verification.txt) records fresh-extraction
reproduction and rejection of missing comparisons, inherited-main substitutions
and missing delivery evidence.

`live-notifications.tar.gz` preserves source, real CLI output, full job logs,
complete paginated metadata, Bencher API responses, notification receipts,
independent read-backs and cleanup receipts. Its internal checksums and the
adjacent `SHA256SUMS` make the retained evidence independent of Actions' 30-day
artifact expiry. `verify-live.py` reproduces the live summary entirely offline.

The baseline model is a latency t-test with upper probability **0.999** and
maximum sample size **30**. The probability is not a latency percentage budget;
the maximum is not evidence of 30 independent runs. Fresh-run provenance must
be established separately, and inherited main history never counts as an
independent v3 run.

## Offline reproduction

From this directory:

```sh
sha256sum -c SHA256SUMS
tar -xzf evidence.tar.gz
tar -xzf live-notifications.tar.gz
tar -xzf completion.tar.gz
(cd evidence && sha256sum -c SHA256SUMS)
(cd live-evidence && sha256sum -c SHA256SUMS)
(cd completion-evidence && sha256sum -c SHA256SUMS)
python3 audit.py
node notification-preview.cjs
python3 verify-live.py
python3 verify-completion.py
```

The verifiers reject optimized Python (`-O`) because assertions are part of the
evidence checks. `audit.py` reproduces the initial snapshot; `verify-live.py`
reproduces notification/cleanup findings; `verify-completion.py` reproduces the
fresh-run comparison inventory and history-preservation result. Each archive
contains its own complete file manifest.

For a network refresh, stage the retained `runs.json`, six `head-probes/*.json`
files and two `orchestration/*.py` files together in one input directory. Run
`collect-completion.py INPUT/runs.json` from a fresh copy of the collector together
with `inventory.json` and `evidence.tar.gz`: it caches files in its adjacent
`completion-evidence/` directory for resumable collection. Preserve the frozen archives first. Later real observations can
legitimately change the active history and require review of the frozen verifier's
expected provenance.
