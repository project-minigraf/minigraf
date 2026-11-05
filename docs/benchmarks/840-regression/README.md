# Regression history and scheduled reporting audit

Audit date: 2026-11-05. [Issue #840](https://github.com/project-minigraf/minigraf/issues/840)
remains open. Actual main/v3 scheduled routes are verified, but the same **244
native and 26 binding IDs still lack numeric comparisons**. Native uploads
rebase the shared v3 Bencher head when main advances, preventing those histories
from accumulating reliably. The proposed fix has separate draft reviews for
[main #853](https://github.com/project-minigraf/minigraf/pull/853) and
[v3 #854](https://github.com/project-minigraf/minigraf/pull/854).

This is operational evidence under PHILOSOPHY.md's reliability and documentation
principles. No benchmark was built, enumerated or timed locally. The observations
come from existing Actions runs. The proposed workflow fix has not been merged;
no live notification has been sent by this audit.

## Scheduled routes and exact coverage

| Run | Event | Route | Native suites / IDs | Binding suites / IDs | Numeric native / binding comparisons |
| --- | --- | --- | --- | --- | --- |
| [41642356471](https://github.com/project-minigraf/minigraf/actions/runs/41642356471) | workflow_dispatch | v3 | 26 / 427 | 4 / 26 | 183 / 0 |
| [41711848743](https://github.com/project-minigraf/minigraf/actions/runs/41711848743) | schedule | v3 | 26 / 427 | 4 / 26 | 183 / 0 |
| [41801278030](https://github.com/project-minigraf/minigraf/actions/runs/41801278030) | schedule | main | 21 / 177 | 0 / 0 | 177 / 0 |
| [41908407297](https://github.com/project-minigraf/minigraf/actions/runs/41908407297) | schedule | v3 | 26 / 427 | 4 / 26 | 183 / 0 |

All four runs succeeded. Their nonempty native selections are disjoint, and
selected = raw executed = raw uploaded = API report IDs in every enabled suite.
The v3 binding jobs cover browser (8), C (6), Node (6) and Python (6), with their
separate `binding-<runtime>-ubuntu24-v1` testbeds. Native uses `ubuntu-latest`.
The complete per-ID values, comparison limits, suite, testbed, report UUID,
head UUID and measured SHA are in [inventory.json](inventory.json).
[summary.json](summary.json) retains per-suite counts and binding source hashes.

| Run | Measured core SHA | Workflow SHA | Created UTC |
| --- | --- | --- | --- |
| 41642356471 | `0394c90ad90e9b95d416f4a0ec4a4c78b16e5b57` | `4d65022ac963a10b582ac762c4ab2e73935c9115` | Nov 3 13:13:01 |
| 41711848743 | `c5863c79e4039e54c7fa67e0d7a8b8751f9b1599` | `4d65022ac963a10b582ac762c4ab2e73935c9115` | Nov 3 20:21:41 |
| 41801278030 | `e7923eb710b1e1d8aff517721643c0b0b1f2cb7a` | `e7923eb710b1e1d8aff517721643c0b0b1f2cb7a` | Nov 4 08:40:10 |
| 41908407297 | `33e5dffadebe0124e6287fc91cede4ff2b7a2b2f` | `e7923eb710b1e1d8aff517721643c0b0b1f2cb7a` | Nov 4 20:37:44 |

The two archived workflow revisions are byte-identical for `bench.yml`. They
map main to `0 2 * * *` and v3 to `0 14 * * *`, and the event, run title and
retained measured-ref reports agree. Creation times show delayed execution;
this verifies the route, not punctual execution at the nominal cron time.
The Actions REST metadata does not include the original event JSON.

## Why the missing baselines have not accumulated

Every native v3 upload supplies `--start-point main
--start-point-clone-thresholds`. Bencher creates a new branch head when the
start point changes. That behavior is documented in
[Branches and Start Points](https://bencher.dev/docs/explanation/branches/).
Bindings omit the start-point options, but share branch `v3`: changing its head
affects their testbeds too.

The retained public report responses establish the change directly:

| v3 head | Main start-point hash | Created UTC | Replaced UTC |
| --- | --- | --- | --- |
| `01a18921-537c-719a-9b63-5f2b0506420a` | `4a77092aa8baffee58a8cf57f2d7c8805abf2d9c` | Oct 29 19:27:05 | Nov 3 13:18:17 |
| `01a1a18f-7bfd-70cf-b3f9-61d5e0a8b037` | `c72d9d1f3a24bcd41d568aa6f06765b23b0f74c2` | Nov 3 13:18:17 | Nov 4 20:42:55 |
| `01a1a84c-e947-745c-98e7-018e0c2245a3` | `e7923eb710b1e1d8aff517721643c0b0b1f2cb7a` | Nov 4 20:42:55 | active at collection |

The original validation dispatch split 2/28 reports between the first two heads.
The Nov 4 v3 schedule split 1/29 reports between the last two. Its C report,
`01a1a84b-b12d-7303-8ee2-da6a1a73a533`, was accepted at 20:41:34, before the head
changed at 20:42:55. Its six observations remain in the old report/head; they
are not missing uploads.

The active-head `/perf` API was queried for all 270 affected IDs from epoch zero
through the recorded collection cutoff, in batches of at most eight benchmark
UUIDs as required by the [Perf API](https://bencher.dev/docs/api/projects/perf/).
Results: 244 native and 20 browser/Node/Python series contain exactly one
observation each, all belonging to the latest v3 run. The six C series have no
observation on this active head. See
[active-head-summary.json](active-head-summary.json) and the raw queries/responses
in the archive. No inherited main observation is counted as an independent v3
run. The original 270-ID inventory and latest null comparison values are in
[missing-baselines.json](missing-baselines.json).

All 1,536 observations across the four audited runs use latency t-test models
with upper probability **0.999** and maximum sample size **30**. A usable
comparison here requires finite numeric `baseline` and `upper_limit`, not merely
a stored model. The 0.999 parameter is a statistical probability, not a latency
percentage budget; 30 is a maximum sample count, not proof of 30 independent
runs. See [Thresholds and Alerts](https://bencher.dev/docs/explanation/thresholds/).

The proposed fix stops main inheritance for persistent main/v3 histories and
preserves it for ad hoc comparison branches. Its test executes the actual
workflow upload shell with a capture-only Bencher function: v3 failed before the
fix, and main/v3/ad hoc cases pass afterward. Both existing reporting test files
also pass. This does not restore old heads or prove future accumulation. After
review and deployment, collect fresh independent runs until every intended ID
has a finite baseline and upper limit on the correct active head/testbed.

## Notification evidence and remaining live work

All 30 `Report regression` steps were skipped in each inspected run, including
the disabled v3-only jobs on main. Therefore successful uploads prove neither
failed-upload notification nor repeated-alert delivery.

[notification-preview.cjs](notification-preview.cjs) executes both archived,
unmodified notification script bodies offline. Twelve checks cover immediate
failed-upload reporting, a repeated same-ID alert, different ID/branch/suite
suppression, and previous-log lookup failure. They pass; the latter warns and
uses an empty prior-alert set. [notification-preview.json](notification-preview.json)
contains the exact preview messages and simulated calls. This is policy
verification with fixtures, **not live end-to-end evidence**.

The [controlled live check protocol](live-checks.md) scopes the remaining test
history, messages, assertions and cleanup. It requires actual GitHub issue/comment
delivery and real prior-run log retrieval. Posting those notifications and
merging the separately reviewed production fixes remain pending authorization.

## Durable evidence and reproduction

[evidence.tar.gz](evidence.tar.gz) preserves all four runs' retained artifact
contents: native enumeration/selection, raw Criterion logs and Bencher output,
binding output/metrics/source hashes, report JSON, complete paginated job and
artifact metadata, run metadata, 111 public Bencher reports, and 35 active-head
API query responses. It also contains the two executed workflow revisions.
This Git archive is independent of Actions' 30-day retention. The adjacent
[SHA256SUMS](SHA256SUMS) checks the archive and analysis files; the archive's
`evidence/SHA256SUMS` checks every individual evidence file.

From this directory:

```sh
sha256sum -c SHA256SUMS
tar -xzf evidence.tar.gz
(cd evidence && sha256sum -c SHA256SUMS)
python3 audit.py
node notification-preview.cjs
```

`audit.py` independently compares raw execution/upload IDs with selection and
API records, rejects duplicates and overlapping suites, verifies hashes/models,
and regenerates all four inventory/summary files. It also verifies the active
head's one-observation and absent-C findings. It is a frozen-audit verifier;
its fixed expected results must be reviewed when collecting newer runs.

For a network refresh, download each run with `gh run download RUN --repo
project-minigraf/minigraf --pattern 'bench-evidence-*' --pattern
'binding-evidence-*' --pattern bench-enumeration --dir evidence/RUN`, then run
`python3 collect.py`. The collector uses read-only GitHub and public Bencher
requests. Preserve the existing archive before refreshing; a newer active head
or additional observations can legitimately invalidate this frozen audit's
assertions. Repack and regenerate checksums only after reviewing the new results.

## Acceptance status

| Requirement | Status |
| --- | --- |
| Inventory original 244 native / 26 foreign gaps | Complete; exact IDs and API values retained |
| Usable independent v3 comparisons for every intended ID | **Open**; 270 remain without comparisons; review/deploy the history fix and accumulate fresh runs |
| Confirm 0.999/30 settings and separate testbeds | Complete for every observation in the four audited runs |
| Observe final main/v3 scheduled routes and exact sets | Complete for the workflow revisions and measured SHAs above |
| Live failed-upload and repeated-alert delivery, scoping and cleanup | **Open**; offline previews are not delivery evidence |
| Preserve complete evidence beyond artifact expiry | Complete for this snapshot in the Git archive; future runs still need collection |
