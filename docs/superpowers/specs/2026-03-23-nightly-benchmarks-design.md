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
- `open/` — checkpointed file open, WAL-replay open (1k–1m)
- `checkpoint/` — WAL flush to packed pages (1k–10k)
- `concurrent/` — readers, readers+writer, serialized writers (in-memory, 4/8/16 threads)
- `concurrent_file/` — same as concurrent but file-backed

Estimated full-suite runtime on `ubuntu-latest`: ~50 minutes. The 6-hour GitHub Actions job timeout is not a constraint.

## Decisions

### Runner: GitHub shared (`ubuntu-latest`)

A self-hosted runner was considered for lower noise, but Bencher.dev's statistical tests (Welch t-test) compensate for shared-runner variance by building a baseline distribution over time. This avoids the operational overhead of provisioning and maintaining a self-hosted runner.

### Publishing: Bencher.dev (free tier for open source)

Bencher.dev provides persistent time-series storage and statistical regression detection. It is the only component that requires an external service. All alerting and issue creation logic lives in the workflow file — no Bencher-side webhooks or integrations are required.

### Regression alerting: GitHub issue (not job failure)

A regression alert opens a GitHub issue rather than failing the job. The job fails only on hard errors (build failure, bench panic, Bencher API error). This keeps the nightly run green for human review while still surfacing regressions.

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
3. **Cargo cache** — `actions/cache@v4`, keyed on `Cargo.lock`; paths: `~/.cargo/registry`, `~/.cargo/git`, `target/`
4. **Run benchmarks** — `cargo bench 2>&1 | tee bench_output.txt`
   - Fails the job on any non-zero exit (build failure, bench panic)
   - `tee` preserves output for the Bencher step
5. **Upload to Bencher** — `bencherdev/bencher@main` with `continue-on-error: true`
   - Sends `bench_output.txt` to Bencher; exits non-zero if a regression alert fires
   - `continue-on-error: true` prevents a regression from failing the job
6. **Open regression issue** — `actions/github-script@v7`, runs only if step 5 outcome is `failure`
   - Queries open issues with label `performance`; skips creation if one already exists (deduplication)
   - Creates issue titled `Benchmark regression – YYYY-MM-DD` with link to the failing run

### Bencher Configuration

| Parameter | Value | Rationale |
|---|---|---|
| `adapter` | `rust_criterion` | Parses Criterion's default stdout/stderr; no `--output-format` flag needed |
| `testbed` | `ubuntu-latest` | Tags results by runner type for future comparisons |
| `branch` | `main` | Tracks regressions on the main branch |
| `threshold-test` | `t_test` | Welch t-test handles shared-runner variance |
| `threshold-upper-boundary` | `0.99` | Flags if new measurement is in top 1% of historical distribution |
| `err` | `true` | Exit non-zero on alert (caught by `continue-on-error`) |

**Baseline period**: Bencher suppresses alerts until sufficient historical data is accumulated (~20 nightly runs). No manual baseline seeding is required.

**Tuning**: If false positives occur on noisy groups (e.g. `recursion/chain/depth_100`), raise `threshold-upper-boundary` to `0.999` for those measurements via Bencher's per-metric threshold UI.

## Setup Requirements (One-time, outside the workflow)

1. **Bencher account**: Create a free account at bencher.dev, create a project named `minigraf`
2. **API token**: Add `BENCHER_API_TOKEN` as a GitHub repository secret (`Settings → Secrets → Actions`)
3. **Label**: Create a `performance` label on the GitHub repo (the workflow does not auto-create labels)

## Out of Scope

- **PR benchmarking**: The ~50-minute suite is too slow for PR gates. If per-PR regression detection is needed in future, a fast subset (in-memory 1k/10k only) could be extracted as a separate workflow.
- **Self-hosted runner**: Not needed given Bencher's statistical baseline approach. Can be revisited if shared-runner noise proves problematic after the baseline is established.
- **Bencher webhooks or Slack notifications**: All alerting flows through GitHub issues.
