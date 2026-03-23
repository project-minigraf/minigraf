# Nightly Benchmark CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a GitHub Actions workflow that runs the full Criterion benchmark suite nightly, publishes results to Bencher.dev, and opens a GitHub issue when a regression is detected.

**Architecture:** Single workflow file (`.github/workflows/bench.yml`) with four logical steps: run benchmarks (capturing output to a file), upload to Bencher for statistical regression analysis, and conditionally open a GitHub issue if Bencher exits non-zero. Cargo bench failure fails the job; Bencher regression does not.

**Tech Stack:** GitHub Actions, `bencherdev/bencher@v0.4.25`, `actions/github-script@v6`, Criterion (already in repo), Bencher.dev free tier.

---

## Pre-flight checks

Before starting, confirm these are already done (they are, per the user):
- `BENCHER_API_TOKEN` added as a GitHub repository secret
- Bencher project slug `minigraf` created at bencher.dev
- `performance` label exists on the repo
- Repository Actions settings allow `issues: write` (default for public repos; check `Settings → Actions → General → Workflow permissions` if issue creation fails)

---

### Task 1: Create the workflow file

**Files:**
- Create: `.github/workflows/bench.yml`

This is a GitHub Actions workflow, not library code, so there is no unit test to write first. Validation is: YAML parses without errors, then a live `workflow_dispatch` run succeeds.

- [ ] **Step 1: Create `.github/workflows/bench.yml`**

```yaml
name: Nightly Benchmarks

on:
  schedule:
    - cron: '0 2 * * *'  # 02:00 UTC nightly
  workflow_dispatch:       # allow manual runs

permissions:
  contents: read
  issues: write

jobs:
  benchmark:
    name: Run benchmarks
    runs-on: ubuntu-latest

    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
          key: ${{ runner.os }}-cargo-bench-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-bench-

      - name: Run benchmarks
        run: |
          set -o pipefail
          CARGO_TERM_COLOR=never cargo bench 2>&1 | tee bench_output.txt

      - name: Upload to Bencher
        id: bencher
        continue-on-error: true
        uses: bencherdev/bencher@v0.4.25
        with:
          project: minigraf
          token: ${{ secrets.BENCHER_API_TOKEN }}
          branch: main
          testbed: ubuntu-latest
          threshold-measure: latency
          threshold-test: t_test
          threshold-upper-boundary: "0.99"
          err: true
          adapter: rust_criterion
          file: bench_output.txt

      - name: Open regression issue
        if: steps.bencher.outcome == 'failure'
        uses: actions/github-script@v6
        with:
          script: |
            const today = new Date().toISOString().slice(0, 10);
            const title = `Benchmark regression - ${today}`;

            // Deduplication: skip if an open regression issue already exists
            const { data: openIssues } = await github.rest.issues.listForRepo({
              owner: context.repo.owner,
              repo: context.repo.repo,
              labels: 'performance',
              state: 'open',
            });
            const existing = openIssues.filter(i =>
              i.title.startsWith('Benchmark regression -')
            );
            if (existing.length > 0) {
              console.log(`Skipping: open regression issue already exists (#${existing[0].number})`);
              return;
            }

            const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
            await github.rest.issues.create({
              owner: context.repo.owner,
              repo: context.repo.repo,
              title: title,
              body: [
                'Bencher detected a performance regression (or encountered an API error) in the nightly benchmark run.',
                '',
                `**Run:** ${runUrl}`,
                '',
                'Please review the Bencher dashboard for details on which benchmarks regressed.',
              ].join('\n'),
              labels: ['performance'],
            });
```

- [ ] **Step 2: Validate YAML syntax locally**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/bench.yml')); print('YAML OK')"
```

Expected output: `YAML OK`

If Python is not available: `cat .github/workflows/bench.yml` and visually verify indentation is consistent (2 spaces throughout, no tabs).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/bench.yml
git commit -m "ci: add nightly benchmark workflow with Bencher.dev regression detection"
```

- [ ] **Step 4: Push and trigger a manual run**

```bash
git push origin main
```

Then in the GitHub UI: `Actions → Nightly Benchmarks → Run workflow → Run workflow`.

Expected: the workflow appears in the Actions tab and starts running.

- [ ] **Step 5: Verify the run succeeds**

Wait for the run to complete (30–50 minutes; the `open/checkpointed/1m` and `recursion/chain/depth_100` groups are the long tail). Check:

1. **Run benchmarks** step: exits 0, `bench_output.txt` is populated with Criterion output. Criterion emits lines in the format:
   ```
   insert/single_fact/1k   time:   [123.45 ns 125.00 ns 126.55 ns]
   ```
   (three-value `[low estimate mid high]` per benchmark, not the libtest `bench: N ns/iter` format)
2. **Upload to Bencher** step: exits 0 on first run (no historical baseline yet, so no regression alert). Check the Bencher dashboard at `bencher.dev` — results should appear under the `minigraf` project.
3. **Open regression issue** step: should be skipped (outcome of Bencher step is `success`, not `failure`).

If the Bencher step fails with an auth error: verify `BENCHER_API_TOKEN` is set correctly under `Settings → Secrets and variables → Actions`.

If the Bencher step fails with "project not found": verify the project slug in the workflow matches the one created on bencher.dev exactly (`minigraf`).

- [ ] **Step 6: Confirm no issue was opened**

Check the repo's Issues tab — no new `performance`-labelled issue should have been created. The issue creation step only fires when `steps.bencher.outcome == 'failure'`.
