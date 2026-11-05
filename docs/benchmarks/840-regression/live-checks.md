# Controlled live notification checks

Status: prepared protocol, not executed. This is the remaining delivery work for
#840. The real nightly histories must receive only real measurements; synthetic
values belong exclusively to disposable audit history.

## Scope and externally visible messages

Use a unique `audit-840-<timestamp>` Bencher branch, dedicated testbeds and
benchmark IDs under `audit/840/`. Never use `main`, `v3`, `ubuntu-latest` or any
`binding-<runtime>-ubuntu24-v1` testbed for synthetic values. Retain the normal
t-test probability 0.999 and maximum sample size 30. Pin the tested workflow and
notification-script revisions and archive the exact test driver before running.

The two notification suites are `audit_840_native` and `binding_audit_840`.
The production script creates issues with titles:

- `Benchmark regression: audit_840_native (audit-840-<timestamp>)`
- `Benchmark regression: binding_audit_840 (audit-840-<timestamp>)`

Further failures comment on those same issues. The expected production message
templates are in [notification-preview.json](notification-preview.json); replace
the preview branch, SHA, benchmark ID and example.invalid run links with the
actual isolated test values. Do not mention users or teams. Archive the returned
issue/comment URLs and bodies, then close only these test issues and delete the
test resources after evidence is saved. GitHub can deliver notifications to repo
watchers; closing an issue does not retract delivered mail.

## Execution and assertions

1. Deploy the history-preservation fix through its separate reviews. Accumulate
   actual independent v3 benchmark runs with the normal workflow on its ordinary
   idle CI runners. Do not re-upload an old log or count inherited main data as a
   new observation. Query the API until all 453 intended IDs have finite numeric
   comparison baselines and upper limits; verify a main advance does not replace
   the v3 head. Keep actual schedules distinct from dispatches.
2. Run the delivery experiment in disposable workflow/history scope. Reuse the
   exact native and binding notification bodies and real Actions job logs. If
   the isolated driver redirects the hardcoded `branch: main` workflow lookup
   to its disposable workflow ref, record that single isolation substitution;
   this does not verify the unmodified default-branch lookup itself.
3. Seed each synthetic benchmark with independent controlled fixture uploads
   until the API returns a numeric baseline. Use a conspicuously larger next
   fixture and verify that Bencher actually emits an alert for the intended ID.
   Do not fabricate an alert line and describe it as an end-to-end check.
4. First alert: production reporting must suppress delivery. Second consecutive
   same-ID, same-branch, same-suite alert: verify a real issue or comment exists
   through an independent GitHub read. Record both report UUIDs, job logs, run
   URLs and the resulting message. Issue creation proves GitHub delivery, not
   receipt by a human mailbox.
5. Interleave another branch and another suite, and use a different benchmark ID
   on one repeated run. None may satisfy the prior same-ID branch/suite condition.
   Preserve the candidate run/job list and the actual chosen previous run/log.
6. Force a real failed upload in isolated history with no alert result. Preserve
   the failing CLI outcome and error. Verify immediate delivery and the
   always-run completeness audit's failure. Also exercise missing, partial,
   duplicate and unexpected uploads against the audit; distinguish local
   parser checks from transport failures and do not claim they prove delivery.
7. Inject a controlled previous-log lookup failure. Require a warning, an empty
   previous-alert set and suppressed repeated-alert notification; preserve the
   failed API call and resulting run summary.
8. Archive all source, fixture data, raw logs, Actions and Bencher API records,
   notification bodies, verification reads, and cleanup receipts in Git with
   checksums before deleting disposable Bencher reports/testbeds/branch and the
   temporary workflow ref. Never delete normal run history. Close the two test
   issues. Re-query to verify cleanup, and leave #840 open for any unmet gate.

Authorization needed: merge the separately reviewed workflow fixes after CI
passes; create and close the clearly scoped test issues/comments and disposable
test history. Repository issue #840 explicitly requires a separate review for
production changes, and external test notifications need explicit permission.
