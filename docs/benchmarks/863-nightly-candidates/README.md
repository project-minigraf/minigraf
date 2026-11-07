# #863: qualifying the #841 nightly candidates

This report qualifies the six small representatives that the
[#841 shape sweep](../841-shape-sweeps/README.md#nightly-candidates-proposals-only)
proposed for nightly CI
([issue #863](https://github.com/project-minigraf/minigraf/issues/863);
[design](../../superpowers/specs/2026-11-06-863-nightly-candidates-design.md),
[plan](../../superpowers/plans/2026-11-06-863-nightly-candidates.md)). All six
qualify. Each one now runs nightly in two parts:

- **Time:** a Criterion case under an existing route (`^window/`, `^query/`,
  `^recursion/`), with Bencher latency and t-test alerts. N6 has no
  Criterion case.
- **Answer, path and memory:** an ignored `test-hooks` test in
  `tests/nightly_candidates_acceptance_test.rs`, run by `smoke.yml`'s v3
  acceptance matrix. A path regression fails this job; a slower runner can
  only raise a Bencher alert.

A nightly route detects a regression in one representative. It does not
re-establish the #841 envelope. Criterion uses the bench profile (opt-level
z), so its times are not comparable with the #841 opt-level 3 medians.

## Candidates and routes

| ID | Candidate | Criterion ID | Acceptance test: what it pins |
| --- | --- | --- | --- |
| N1 | 50,000 rows (100k facts), 90%-skewed partitions, running `sum`, committed | `window/nightly_candidates/skew_running_sum/100k` | `n1_skew_running_sum_streams_by_partition`: answer; `WindowPartitioned` in the plan; Window peak in [0.85, 0.95] of a full-input control (`__input_order(false)`) |
| N2 | 10k entities, `K=256` skewed sets, six membership predicates, committed | `query/nightly_candidates/set_membership_k256/10k` | `n2_set_membership_k256`: answer only (no event identifies the long value-page path) |
| N3 | 100k facts, `:offset 50000 :limit 1000`, committed | `query/nightly_candidates/sort_offset_desc/100k`, `…/sort_offset_asc/100k` | `n3_sort_heap_holds_offset_plus_limit`: answers; descending has `Sort`, ascending none; heap peak ≤ 0.6 of a full Sort |
| N4 | 1,110-edge width-10 tree, one leaf's ancestors, pending | `recursion/nightly_candidates/tree_ancestors_k1_pending/1110` | `n4_small_tree_takes_ancestor_alternative`: answer; `ancestor_alternative_selected` event |
| N5 | The same tree, 16 demanded leaves, committed | `recursion/nightly_candidates/tree_ancestors_k16_committed/1110` | `n5_committed_tree_uses_bound_value_probes`: answer; `bound_value_avet_range` event, no alternative |
| N6 | 10k facts, one cursor held across 256 of 544 churn checkpoints | none (acceptance only) | `n6_held_cursor_quarantine_and_reuse`: held answer unchanged; file grows while held and stops growing after release (growth ratio ≤ 0.5); `verify()` clean |

Fixtures, queries and oracles are those of the #841 harness, ported to
`benches/helpers/shape.rs`. `shape_port_matches_the_database`, which is not
ignored, checks the port against the database at small sizes on every
`test-hooks` test run. The path events and `Cursor::__memory_peak` exist only
in test and `test-hooks` builds. The `dist` binary is byte-for-byte the same
size as its base, `921b3935`.

## Protocol

A temporary workflow (`qualify-863.yml`, removed before merge) ran each
Criterion case on its own and then each acceptance test, on separate
`ubuntu-latest` runners: three jobs at `ad326585` (the PR merge commit of head `b6b864a2` into v3) on 2026-11-06 (attempt 1),
and three more on 2026-11-07 (GitHub attempt 3; attempt 2 was cancelled). The
measured source is the same in all six. `scripts/qualify_863.py` produces the
table and [`summary.json`](summary.json) from the six artifacts:

```sh
sha256sum -c SHA256SUMS
python3 -I ../../../scripts/qualify_863.py . attempt-1/qualify-863-run-{1,2,3} attempt-2/qualify-863-run-{1,2,3}
```

**The bar** (design D3, amended): a candidate is rejected if its fixture
construction, warm-up and measurement, plus its acceptance test, take more than
600 s (10 job-minutes), or if its spread is over 25%. Spread is
`(max − min) / min` of the Criterion medians, measured **after dividing each
run by its speed factor**: the geometric mean of all timed cases in that run.

The six runs landed on three CPU models. Within one model a runner can also be
about 1.46× slower on every case at once: of the four EPYC 9V74 runs, run 6
measured N1 at 1,424 ms against 972–981 ms. This is the same whole-runner
effect the existing nightly suites have (#678), and Bencher's t-test over the
last 30 runs absorbs it. Raw same-model and cross-model spreads (43–52% and
83–103%) are reported for that reason, but they are not the bar. The first bar,
raw spread over three runs, then raw spread within one CPU model, rejected
every timed candidate for this reason alone (user decisions 2026-11-06 and
2026-11-07).

## Results

| Candidate | Max cost (s) | Criterion medians (ms), runs 1–6 | Normalized spread | Same-model spread | Cross-model spread | Verdict |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| N1 | 204.1 | `skew_running_sum/100k`: 704.780 / 973.084 / 980.904 / 1153.193 / 972.050 / 1423.700 | 3.5% | 46.5% | 102.0% | qualified |
| N2 | 96.1 | `set_membership_k256/10k`: 785.832 / 946.438 / 1024.328 / 1192.207 / 971.735 / 1440.915 | 10.7% | 52.2% | 83.4% | qualified |
| N3 | 113.4 | `sort_offset_desc/100k`: 56.792 / 78.097 / 82.022 / 86.982 / 80.991 / 115.462<br>`sort_offset_asc/100k`: 11.111 / 15.133 / 14.845 / 18.629 / 14.631 / 21.557 | 13.1% | 47.8% | 103.3% | qualified |
| N4 | 49.8 | `tree_ancestors_k1_pending/1110`: 0.103 / 0.133 / 0.133 / 0.178 / 0.134 / 0.200 | 12.3% | 50.9% | 93.8% | qualified |
| N5 | 44.8 | `tree_ancestors_k16_committed/1110`: 54.733 / 77.000 / 75.848 / 91.329 / 75.514 / 108.398 | 5.5% | 43.5% | 98.0% | qualified |
| N6 | 2.5 | acceptance only | — | — | — | qualified |

Runs: 1: AMD EPYC 9V45 96-Core Processor (speed factor 35.43 ms), 2: AMD EPYC 9V74 80-Core Processor (speed factor 47.25 ms), 3: AMD EPYC 9V74 80-Core Processor (speed factor 48.06 ms), 4: AMD EPYC 7763 64-Core Processor (speed factor 57.54 ms), 5: AMD EPYC 9V74 80-Core Processor (speed factor 47.38 ms), 6: AMD EPYC 9V74 80-Core Processor (speed factor 69.31 ms).

The shared compile step is not counted per candidate. Criterion's
fixture-plus-measurement time per run, all five timed cases together, was
315–439 s. The six acceptance tests took 37–57 s together. Every acceptance
run measured the same values: N1 9,073,864 / 10,000,032 bytes (0.907), N3
2,856,056 / 5,600,000 bytes (0.51), N6 10,280,960 bytes of growth while held
and 0 after release. On the alternate bench host the six acceptance tests take
6 s, and the five Criterion cases 146 s.

## Limits

- N2 pins only the answer: no counter separates the long value-page set path
  from another plan.
- The memory bounds are the query meter's charged bytes, not RSS, and they
  hold only for these fixtures.
- Normalizing by run speed removes changes that slow every case at once,
  including a real regression shared by all five. The bar keeps out
  candidates whose times are unstable on their own; the existing alert path
  handles shared shifts.
