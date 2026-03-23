# Nightly Benchmark CI with Regression Detection

**Date**: 2026-03-23
**Status**: Approved

## Goal

Run the full Criterion benchmark suite nightly on GitHub Actions, publish results to Bencher.dev, and automatically open a GitHub issue when a performance regression is detected.

## Context

Minigraf has a comprehensive Criterion benchmark suite (`benches/minigraf_bench.rs`) covering 9 groups at multiple scales:

- `insert/` — single fact, batch 100, explicit tx (in-memory, 1k–100k)
- `insert_file/` — same as insert but file-backed (1k–100k)
- `query/` — point entity, point attribute, 3-pattern join (in-memory, 1k–1m)
- `time_travel/` — `:as-of` counter, `:valid-at` timestamp (in-memory, 1k–1m)
- `recursion/` — chain depth 10/100, fanout w10_d3
- `open/` — checkpointed (1k–1m); WAL-replay (1k–10k)
- `checkpoint/` — WAL flush to packed pages (1k–10k)
- `concurrent/` — readers, readers+writer (4/8/16 threads); serialized writers (2/4/8/16 threads) — in-memory
- `concurrent_file/` — readers, readers+writer (4/8/16 threads); serialized writers (2/4/8/16 threads) — file-backed

Estimated full-suite runtime on `ubuntu-latest`: ~50 minutes. The 6-hour GitHub Actions job timeout is not a constraint.

## Decisions

### Runner: GitHub shared (`ubuntu-latest`)

A self-hosted runner was considered for lower noise, but Bencher.dev's statistical tests (Welch's t-test) compensate for shared-runner variance by building a baseline distribution over time. This avoids the operational overhead of provisioning and maintaining a self-hosted runner.

### Publishing: Bencher.dev (free tier for open source)

Bencher.dev provides persistent time-series storage and statistical regression detection. It is the only component that requires an external service. All alerting and issue creation logic lives in the workflow file — no Bencher-side webhooks or integrations are required.

### Regression alerting: GitHub issue (not job failure)

A regression alert opens a GitHub issue rather than failing the job. The job fails only on hard errors (build failure, bench panic). A Bencher API connectivity failure will also open a GitHub issue (false positive) because both failure modes are indistinguishable from the workflow's perspective — this is accepted as a low-frequency edge case for a nightly job.

## Workflow Design

**File**: `.github/workflows/bench.yml`

**Triggers**:
- `schedule`: nightly at 02:00 UTC (`cron: '0 2 * * *'`)
- `workflow_dispatch`: manual trigger for ad-hoc runs

**Permissions**:
- `contents: read`
- `issues: write`

### Steps

1. **Checkout** — `actions/checkout@v4`
2. **Rust toolchain** — `dtolnay/rust-toolchain@stable` (matches all other CI workflows)
3. **Cargo cache** — `actions/cache@v4`, keyed on `Cargo.lock`; paths: `~/.cargo/registry`, `~/.cargo/git`
   - `target/` is intentionally excluded: Criterion benchmark artifacts are large (1–2 GB) and would exhaust the cache quota with negligible build-time benefit on a nightly job.
4. **Run benchmarks** — shell step using `set -o pipefail` before the pipe to preserve `cargo bench` exit code:
   ```sh
   set -o pipefail
   CARGO_TERM_COLOR=never cargo bench 2>&1 | tee bench_output.txt
   ```
   - `set -o pipefail`: makes the pipeline fail if `cargo bench` exits non-zero, even though `tee` runs after it. Without this, a build failure or bench panic would be silently swallowed by `tee`'s zero exit code.
   - `CARGO_TERM_COLOR=never`: strips ANSI escape codes from output so Bencher's `rust_criterion` adapter can parse the plain text reliably.
   - `tee bench_output.txt`: captures output to file for the Bencher step while still streaming it to the run log.
   - **Fails the job** on any non-zero `cargo bench` exit (build failure, bench panic).
5. **Upload to Bencher** — `bencherdev/bencher@v0.4.25` with `continue-on-error: true`
   - Feeds `bench_output.txt` to Bencher; exits non-zero if a regression alert fires or on API error.
   - `continue-on-error: true` prevents either outcome from failing the job at this step.
6. **Open regression issue** — `actions/github-script@v6`, runs only `if: steps.bencher.outcome == 'failure'`
   - Deduplication query: list open issues with label `performance` whose title starts with `Benchmark regression -`; skip issue creation if any match exists.
   - Creates issue titled `Benchmark regression - YYYY-MM-DD` (ASCII hyphen, not em dash) with:
     - Body: brief description + direct link to the failing run (`${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}`)
     - Label: `performance`

### Bencher Configuration

| Parameter | Value | Rationale |
|---|---|---|
| `project` | `minigraf` | Bencher project slug (must match the project created in setup) |
| `token` | `${{ secrets.BENCHER_API_TOKEN }}` | Repository secret holding the Bencher API token |
| `adapter` | `rust_criterion` | Parses Criterion's default stdout/stderr; no `--output-format` flag needed |
| `testbed` | `ubuntu-latest` | Tags results by runner type for future comparisons |
| `branch` | `main` | Tracks regressions on the main branch |
| `threshold-measure` | `latency` | Measures wall-clock time (Criterion's primary output) |
| `threshold-test` | `t_test` | Welch's t-test (unequal variances assumed); handles shared-runner variance |
| `threshold-upper-boundary` | `0.99` | Flags if new measurement falls in the top 1% of the historical distribution |
| `err` | `true` | Exit non-zero on alert (caught by `continue-on-error` in the workflow step) |
| `file` | `bench_output.txt` | Pre-captured output file from step 4 |

> **Note on `t_test`**: Bencher's `t_test` threshold uses Welch's t-test (unequal variances), not Student's t-test. Upper boundary 0.99 means a regression alert fires when the one-sided p-value falls below 0.01 (i.e. the new measurement is significantly higher than the historical distribution at the 99% confidence level).

**Baseline period**: Bencher suppresses alerts until sufficient historical data is accumulated (~20 nightly runs). No manual baseline seeding is required.

**Tuning**: If false positives occur on noisy groups (e.g. `recursion/chain/depth_100`), raise `threshold-upper-boundary` to `0.999` for those specific metrics via Bencher's per-metric threshold UI.

## Setup Requirements (One-time, outside the workflow)

1. **Bencher account**: Create a free account at bencher.dev; create a project with slug `minigraf`
2. **API token**: Add `BENCHER_API_TOKEN` as a GitHub repository secret (`Settings → Secrets and variables → Actions → New repository secret`)
3. **Label**: Create a `performance` label on the GitHub repo (`Issues → Labels → New label`); the workflow does not auto-create labels

## Out of Scope

- **PR benchmarking**: The ~50-minute suite is too slow for PR gates. If per-PR regression detection is needed in future, a fast subset (in-memory 1k/10k only) could be extracted as a separate workflow.
- **Self-hosted runner**: Not needed given Bencher's statistical baseline approach. Can be revisited if shared-runner noise proves problematic after the baseline is established.
- **Bencher webhooks or Slack notifications**: All alerting flows through GitHub issues.
