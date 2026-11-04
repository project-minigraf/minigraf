# Remaining API and lifecycle dispositions

These are **proposals for binding and release-maintainer acceptance**, not
approved exclusions. If a disposition is rejected, its applicable cases must be
measured on the bench or Device Farm before #839 closes. Existing P16 coverage
does not substitute for any cost below.

| Runtime / API | Evidence or proposed disposition | Limitation requiring acceptance |
| --- | --- | --- |
| JVM/Android/Swift-iOS options and errors | Measure one rejected negative cache option and one read-only open with cache/memory options | Representative validation/conversion cost only; other option combinations and errors remain correctness scope |
| JVM/Android/Swift-iOS FactLog/LogWriter | Measure scalar 1k-record conversion/drain and append+finish; verify complete temporal records and reopened queries | No claim for wide strings/sets, large histories, memory bounds or sustained scaling |
| JVM/Android/Swift-iOS prepare/register | Measure preparation separately and first predicate/aggregate registration on fresh databases | No unregister API; duplicate registration rejection stays correctness scope |
| Python options/errors and FactLog/LogWriter | Propose exclusion from this initial manual timing series | Existing shared UniFFI correctness does not measure Python object allocation or typed conversion. This would defer Python-specific latency characterization, not mark it covered |
| Node options/errors and FactLog/LogWriter | Propose exclusion from this initial manual timing series | N-API/BigInt conversion and JS allocation remain unmeasured; native timings cannot establish these costs |
| C options/errors and FactLog/LogWriter | Propose exclusion from this initial manual timing series | JSON parsing/allocation/freeing costs remain unmeasured by the common execution shortlist |
| Python/Node/C/browser preparation and registration | Propose keeping one-time setup outside the representative steady-use shortlist | Preparation/registration can dominate short-lived usage. Existing execution timings do not bound these setup costs; this is a workload boundary, not evidence of negligible cost |
| Browser file-backed options and raw logs | API absence in the audited wrapper | BrowserDb has no file lock/WAL; native file-backed options do not apply. FactLog is not exposed and raw LogWriter is not built for wasm32; import/export is a different API |
| Browser pinned-reader churn beyond #838 | Propose exclusion of sustained churn from this bounded release snapshot | #838's bounded persistence case does not establish behavior under arbitrarily long-lived readers, repeated generations or memory pressure. Those require an explicit duration/retention/memory workload before a meaningful timing or RSS limit can be claimed |
| WASI prepared objects/callbacks and typed logs | API absence in the packaged REPL contract | Browser or native bindings do not supply missing WASI foreign APIs; retain actual REPL startup/script/query evidence instead |
| Swift/macOS | Excluded by the user's platform decision for #839 | No repeatable macOS measurement platform; correctness/build CI remains allowed |
| All runtimes: deliberate callback failure, all error codes, RSS bounds | Keep callback errors and invalid inputs in correctness tests; no RSS performance claim | Failure handling is not represented as successful-query latency. No quantitative memory bound has been measured |

The proposed Python/Node/C exclusions limit this manual series to the runtime
gaps plus representative additional costs on JVM/mobile; they do **not** follow
automatically from shared native code or Tier 1/Tier 2 status. A maintainer must
explicitly accept that narrower release evidence or request the missing timings.

API audit revisions: Python `16f54fef94c75272873b3381fc83cf264ed05e99`,
Node `96fe7a1a9f7521007fefdfee54142694ab39251d`,
browser `da29682e34e6f0a32a1d63882becfeb01dd5d55d`,
C `653af45e874b15e54564a2d5e12d5583784c7861`.
These source audits establish API availability, not new timing measurements.

## Sign-off record

| Decision | Binding maintainer | Release maintainer | Status |
| --- | --- | --- | --- |
| Runtime evidence and its variability | Pending | Pending | Not accepted |
| Public Device Farm model/OS selection instead of a reserved physical unit | Pending | Pending | Not accepted |
| Each proposed omitted-API/lifecycle disposition above | Pending | Pending | Not accepted |

The sign-off must identify the accepted evidence revisions and dispositions.
Permission to execute benchmarks, provider PASS status and draft PR creation
are not sign-off.
