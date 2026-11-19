# Raw input and deterministic reports (version 1)

`decide.py` and `reproduce.py` use Python 3.10+ and its standard library only.
No network, provider commands, timing collection or third-party numerical
packages are involved. This is a decision-input format, not a replacement for
the original harness streams and provenance required by AC13/AC15.

## Input location and envelope

Accept either a JSON file, or a directory with **one canonical
`raw-samples.json`**. A directory is not globbed or recursively merged: this
avoids counting duplicate exports or stored reports as observations. Original
streams, sample totals/iterations, manifests and checksums may live alongside
that file; the collector must retain them. Refer to [PROTOCOL.md](PROTOCOL.md)
for the pre-timing requirements. The tool does not collect or authenticate them.

```json
{
  "schema_version": 1,
  "synthetic": false,
  "configurations": [
    {
      "id": "ios-physical-v1",
      "platform": "ios",
      "kind": "physical",
      "reference": null,
      "blocking_reason": null,
      "build": {
        "core_revision": "<40 lowercase hex characters>",
        "binding_revision": "<40 lowercase hex characters>",
        "harness_revision": "<40 lowercase hex characters>",
        "profile": "Release",
        "slowdown_flag": "repeat-timed-operation-twice",
        "artifact_sha256": {
          "B1": "<64 lowercase hex characters>",
          "B2": "<64 lowercase hex characters>",
          "V": "<64 lowercase hex characters>"
        }
      },
      "runtime": {
        "provider": "Device Farm",
        "model": "iPhone 15",
        "os": "iOS 18.5",
        "os_build": "TODO: pin before timing"
      },
      "cases": {
        "prepared_execute/1k": {
          "blocking_reason": null,
          "sets": {
            "B1": [
              {
                "run_index": 1,
                "attempt": 1,
                "valid": false,
                "reason": "infrastructure failure: no complete stream",
                "value_ns": null
              },
              {
                "run_index": 1,
                "attempt": 2,
                "valid": true,
                "reason": "",
                "value_ns": 100,
                "sample_means_ns": [95, 96, 97, 98, 99, 101, 102, 103, 104, 105]
              }
            ],
            "B2": [],
            "V": []
          }
        }
      }
    }
  ]
}
```

This abbreviated example is **not a valid completed study**: it has placeholders,
incomplete sets/cases and no candidate. The runnable example is
[fixtures/synthetic.json](fixtures/synthetic.json), clearly marked synthetic;
its hash-shaped identities are fabricated, not actual builds or devices.

## Configurations and eligibility

- `schema_version` is integer 1. `synthetic` is a required boolean and is copied
  to both reports; synthetic qualification is only a test assertion.
- `configurations` is a nonempty list. IDs are unique, public slugs matching
  `[a-z0-9][a-z0-9-]*`. Use no private account, device, host or provider IDs.
- `platform` is `ios` or `android`. `kind` is `physical` or `simulated` (the latter
  includes Android emulators). A candidate's `reference` names a physical
  configuration on the **same** platform. A physical reference is null/absent.
  There must be at least one candidate. Multiple candidates and physical
  configurations are supported; never combine their observations.
- `blocking_reason` is null when eligible, otherwise a nonempty explanation of
  unavailability, unresolved provider failure, unmatched timing boundaries or
  another evidenced blocker. It applies to every case in that configuration.
  A case has its own `blocking_reason` for API/boundary incompatibilities.
- All attempts in a configuration inherit its immutable `build` and `runtime`.
  Extraction must verify the actual identity **on every original run**, not
  merely paste labels into this file. Core, binding and harness revisions,
  Release profile and source flag must match the physical/candidate pair.
  B1/B2 disable the flag, V enables it; `slowdown_flag` names the common source
  switch, not its enabled state. Artifact hashes describe the executed bytes
  for each set on that configuration. Physical and simulated hashes need not
  match; copying the device hash for a simulator is invalid evidence.
- `runtime` is a nonempty object of nonempty string values, with no `TODO`.
  Populate every applicable pin in PROTOCOL.md: model/OS/build for physical;
  runner image, Xcode/runtime/device type for iOS; runner image, emulator,
  API/system image/ABI, acceleration **evidence** and translation for Android.
  The tool checks presence and placeholder absence, not the authenticity or
  completeness of runtime descriptions. Review of actual pins, matched build
  commands and acceleration evidence remains mandatory before timing/activation.
- Missing/invalid build identities or hashes, non-Release profiles, runtime
  placeholders or build mismatches make each affected comparison inconclusive.
  Unavailable configurations should remain present with their blockers and
  empty attempts, so all cases receive an explicit disposition.

The envelope/configuration layout is structural: malformed JSON, duplicate
object keys, duplicate IDs, invalid references, cross-platform comparisons,
unknown cases/sets or invalid object shapes are input errors (exit 2), not
qualification results. Missing known cases/sets produce inconclusive rows
instead of disappearing. No structural error can yield a qualified report.

## Cases, sets and attempts

`cases` is keyed by the 14 suffixes in PROTOCOL.md / `decide.CATEGORIES`.
Do not include `binding/swift/` or `binding/android/` in these keys. Every
candidate always gets all 14 rows, including missing/incompatible cases. The
fixed category mapping comes from code; input cannot override it or supply a
category decision. `sets` keys are `B1`, `B2`, `V`.

Each set is a list of **all attempts**, including failed/aborted attempts:

| Field | Meaning |
| --- | --- |
| `run_index` | Integer 1–6, identifying the pre-registered slot within this set |
| `attempt` | Integer 1–3; consecutive from 1 per slot, maximum two retries |
| `valid` | Boolean, true only after complete exact-result, exit and teardown validation |
| `reason` | String retained for every attempt; nonempty for invalid attempts, empty string allowed for success |
| `value_ns` | A finite positive number for valid attempts, null for invalid attempts |
| `sample_means_ns` | Optional array of the ten original sample means in ns; checked when present on a valid attempt |

A valid value is the median of ten `total_ns / iterations` sample means.
When `sample_means_ns` is present, its length must be ten, all entries finite
and positive, and its computed median must **equal** `value_ns` without rounding
or tolerance. Do not round the exported median. The flag doubles real work,
not the logical iteration denominator. The ten samples may coincide; **zero
variance across the six run values** in any set makes the case inconclusive.

The requested minimal value-only input is accepted. For real evidence retain
all original raw samples in the archive even if they are not embedded in this
JSON; `reproduce.py` recomputes from the decision-input values and verifies any
embedded means, but does not parse the original Swift/Android log formats.
Optional partial `sample_means_ns` on invalid attempts are preserved in the
input and never counted. Nonfinite valid values, including JSON numeric
overflow, are inconclusive. The loader additionally tolerates nonstandard
`NaN`/`Infinity` input tokens to diagnose them as inconclusive; do not deliberately
write these in new archives. Output is always strict JSON with finite numbers.

A retry is legal only after no valid stream. A valid run is never retried,
replaced, trimmed, winsorized or discarded. Duplicated attempts, gaps, more
than three attempts, indices outside 1–6 or any attempt after a valid one are
inconclusive protocol violations. Exactly one valid attempt per slot is needed.
A missing slot is inconclusive even if fewer than the allowed retries have been
made; elapsed time cannot manufacture a valid run. The script counts invalid
attempts, but their values do not enter medians, CVs or Welch tests.

Attempt list order is immaterial to computation; slots and attempt numbers
identify values. The required actual acquisition schedule is `(B1,V,B2) × 6`;
per-set JSON arrays alone cannot prove chronological interleaving or process
independence. Preserve and audit the run schedule/process manifests. If one
process emits the whole 14-case suite, a failed stream invalidates that attempt
for the whole suite: the extractor must retain consistent stream validity,
reasons and attempts across cases, never selectively recover a convenient
case. This tool checks per-case attempts, not original process lifecycles.

## Calculation and report

For independent groups a, b, use sample variances `s_a²`, `s_b²` with n−1:

```text
u = s_a² / n_a; v = s_b² / n_b
t = (mean(a) - mean(b)) / sqrt(u + v)
df = (u + v)² / (u²/(n_a-1) + v²/(n_b-1))
p(a > b) = Student-t survival(t, df)
```

For t > 0 the survival function is
`0.5 * I_x(df/2, 1/2)`, where `x = df / (df + t²)` and `I` is the regularized
incomplete beta. The implementation uses a modified Lentz continued fraction,
log-gamma normalization, symmetry and direct upper-tail evaluation to avoid
subtracting a small probability from one. A log-space small-x limit preserves
representable tails when x itself underflows. Tests cover analytic df=1/2
values, df=10 at t=2 and extreme tails, and unequal-variance Welch df. Undefined
or nonconvergent calculations are inconclusive; no normal approximation is used.

`decide.py INPUT --output PREFIX` writes `PREFIX.json` and `PREFIX.md`.
A supplied suffix is replaced with `.json`/`.md`; use a dedicated output prefix.
Exit 0 means the report was generated, **not** that any case qualified. Exit 2
means an input or I/O error. Existing output files are overwritten on success;
use fresh destinations and do not confuse earlier output with a failed run.

The JSON contains protocol/schema versions, the synthetic marker, configuration
metadata, sorted per-case comparisons and category **counts**. Each comparison
includes physical/candidate IDs, category, outcome, reasons, per-set attempt
counts, and (when all data are eligible) distributions with run values,
n/median/mean/sample stdev/CV, fractional deltas, closeness ratio, all Welch
statistics/df/p-values and individual checks:

| Check/rejection reason | Pass rule |
| --- | --- |
| `closeness` | B1 median ratio in [0.67, 1.5] |
| `cv_absolute` | simulated B1 sample CV <= 0.15 |
| `cv_relative` | simulated B1 sample CV <= 2 × physical B1 sample CV |
| `detection_physical`, `detection_simulated` | one-sided V > B1 p < 0.001 |
| `delta_agreement` | absolute fractional delta difference <= 0.5 × physical delta |
| `aa_physical_b1_gt_b2`, `aa_physical_b2_gt_b1` | indicated one-sided p >= 0.001 |
| `aa_simulated_b1_gt_b2`, `aa_simulated_b2_gt_b1` | indicated one-sided p >= 0.001 |

Every check must pass for `qualified`; any failed check with otherwise valid
data is `rejected`. Degenerate/ineligible/insufficient evidence takes precedence
and produces `inconclusive`, an explanation, empty metrics/checks and retained
attempt counts. Original values/failed attempts remain in the input archive.
The Markdown table shows medians, ratio, CVs, deltas, detection and both A/A
p-values, outcome and reasons, followed by counts per category. It is never an
activation instruction, and does not certify AC14a completion.

All decisions use unrounded binary64 calculations. Outputs use sorted keys,
stable configuration/case ordering, LF line endings and a final newline. JSON
floats have 12 decimal places in scientific notation (integers remain integers);
Markdown statistics use 9 decimal places in scientific notation. Thus small
p-values remain visible. No wall-clock timestamp, absolute path or machine
identifier is inserted. Preserve the Python version and platform with the
archive: identical input/tooling/interpreter reproduces bytes; no promise is
made about last-bit libm differences across arbitrary interpreters/platforms.
Do not make decisions by reading rounded display values back into the gate.

`reproduce.py INPUT STORED_PREFIX` recomputes **all** cases and compares both
stored files byte-for-byte without rewriting them. It exits 0 for identical
JSON and Markdown, 1 for a mismatch, and 2 for invalid/missing input or files.
This covers numerical report reproduction; archive checksums, per-run build
identity, stream validation and original extraction still require their own
retained evidence before a real study can complete.
