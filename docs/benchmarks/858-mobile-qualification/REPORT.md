# Mobile qualification report (#858, Part B)

**Result: neither candidate qualifies.** The iOS simulator and the Android
emulator were compared against fresh matched AWS Device Farm physical runs under
the pre-registered gate in `PROTOCOL.md` (N = 6 per set, `(B1, V, B2) × 6`,
one-sided Welch V>B1 p < 0.001, closeness [0.67, 1.5], CV ≤ 15% and ≤ 2×
physical, delta agreement within 50%, A/A p ≥ 0.001 both ways). All 14 cases on
each platform are **rejected** (0 qualified, 0 inconclusive). No conditional
nightly lane is activated.

| Candidate | Qualified | Rejected | Inconclusive |
| --- | --- | --- | --- |
| android-emulator-v1 | 0 | 14 | 0 |
| ios-simulator-v1 | 0 | 14 | 0 |

- **Android emulator:** every case fails the closeness bound (the emulator runs
  at roughly a third to a half of the physical Pixel's latency); three also fail
  a CV bound.
- **iOS simulator:** cases fail CV or the simulated-V detection test, and many
  also fail closeness; three fail delta agreement. Per-case reasons are in
  `reports/ios.md`.
- Per-case medians, ratios, CVs, deltas, p-values and A/A controls:
  `reports/android.md|json`, `reports/ios.md|json`.

## Evidence

| Path | Content |
| --- | --- |
| `data/<platform>/raw-samples.json`, `provenance.json` | decision input (all attempts, pins, build hashes) |
| `data/<platform>/device-farm-physical-evidence.tgz` | retained physical streams, run metadata and logs |
| `data/<platform>/device-farm-file-manifest.tsv` | every collected file with original size and SHA-256, whether it is retained, and the SHA-256 of the retained (redacted) copy |
| `data/<platform>/ci-simulator-evidence.tgz` | simulator/emulator run evidence from CI (text members) |
| `data/<platform>/ci-simulator-binary-manifest.tsv` | binary CI members (Xcode result bundles) not retained: path, size, SHA-256 |
| `data/collection-tooling.tgz` | collector, assembler, rebuild, recovery, archive-build and scan sources and tests (local absolute paths replaced by placeholders) |
| `reports/` | computed decisions |
| `SHA256SUMS` | checksums of all of the above |

Reproduce the reports byte-for-byte (offline, Python 3 standard library):

```sh
python3 -I reproduce.py data/android reports/android
python3 -I reproduce.py data/ios reports/ios
sha256sum -c SHA256SUMS
```

Retained physical evidence omits videos, tcpdump files, artifact listings and
device syslogs larger than 8 MiB (the manifest holds their size and SHA-256).
The usable streams are retained. Retained text artifacts are redacted for device
identifiers (Device Farm iPhone UDIDs and serial fields) by `build_archive.py`
(the manifest keeps the original and archived SHA-256 of each file), and
`scan_archive.py` fails the build on any UDID or serial pattern, account number,
presigned-URL token or local home or temporary path, over the files and over every
member of each tarball. The first version of this archive pushed unredacted UDIDs
for the four iOS slots collected after the state rebuild (the rebuilt collector had
lost its redaction step); that branch was rewritten before any pull request, so no
ref reaches those blobs.

## Collection

| Platform | Attempts | Valid | Device minutes |
| --- | --- | --- | --- |
| Android physical (Pixel 7, Android 14) | 23 | 18 | 53.6 |
| iOS physical (iPhone15,4) | 25 | 24 | 210.27 |

The physical iOS total is 210.27 device minutes (extra minutes beyond the first
budget were approved). Deviations 4 and 6–10 in `PROTOCOL.md` disclose the
Android unit-identity retries, the iOS parser defect and the earliest-valid-attempt
rule, the oversized syslog, the hand-abandoned attempt, the reboot and the
state rebuild.

## What this does and does not show

It shows that, on this protocol and these cases, the hosted simulator and emulator
are not close enough to physical devices to stand in for them: closeness, CV or
detection fails in every case. It does not show that the physical numbers
themselves are stable run to run beyond this one series, and one simulator
configuration per platform was tested.

## Proposal: costed physical-device nightly lane (not implemented)

A nightly physical series with the same 18-slot design costs about the measured
device minutes of one clean series: Android about 40 minutes (53.6 minutes covered
23 attempts) and iOS about 150 minutes (210.27 minutes covered 25 attempts, including
the wasted first batch). **Estimate** at AWS Device Farm's public metered price of
$0.17 per device minute (AWS pricing page, read 2026-11-27; the account's invoice is
authoritative, and a one-time free trial of 1,000 minutes may apply):

| Lane | Minutes per night | Per night | Per month (30 nights) |
| --- | --- | --- | --- |
| Android only | 40 | $6.80 | $204 |
| iOS only | 150 | $25.50 | $765 |
| Both | 190 | $32.30 | $969 |

The unmetered alternative is $250 per device slot per month (public price, same
date): one slot per platform covers a single nightly series for $500 per month in
total, below the metered cost of both. A reduced lane (fewer cases, N = 6, Android only)
is the cheapest useful first step. Nothing is added to CI by this report.
