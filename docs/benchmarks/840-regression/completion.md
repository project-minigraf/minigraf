# Regression-history follow-up

The production history fix is merged in [main #853](https://github.com/project-minigraf/minigraf/pull/853)
(`25fd6fb9bdb9bc69021a217c1f755e1780c0eb07`) and
[v3 #854](https://github.com/project-minigraf/minigraf/pull/854)
(`fc0a61545addd84eb0d6c300965082a6bf189101`). Both passed their required CI before
merge. Persistent main/v3 uploads now omit main start-point inheritance; ad hoc
comparison branches retain it. This preserves the existing v3 head rather than
migrating or manufacturing observations.

The [initial audit](README.md) is a frozen pre-fix snapshot. Its null comparisons
and notification-policy fixtures describe that snapshot, not the subsequent
live checks below. The issue was manually closed during this follow-up; its
GitHub state alone does not establish an acceptance result.

## Fresh independent measurements

Collection is in progress. Do not treat this section as evidence of completed
baseline maturation until the retained API responses pass `verify-completion.py`.

The measurement plan uses two complete v3 dispatches through the production
workflow, followed by a real C-binding dispatch to give its six IDs the missing
third observation. C's original scheduled observation belongs to the previous
head and cannot count here. Real main and v3 UDF probes then exercise the fix
across an actual main Bencher version advance. Every probe uses the normal
benchmark code, source pinning, reporting and testbed, on GitHub-hosted runners.
The probes narrow the selected suite; they do not replay logs or upload fixtures.

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
Historical Actions run records remain as an audit trail.

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
