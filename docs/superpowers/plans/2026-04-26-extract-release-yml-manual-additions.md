# Extract Manual Additions from release.yml — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> Also REQUIRED: invoke superpowers:using-git-worktrees before writing any code.

**Goal:** Remove all manually-added sections from `release.yml` so it is safe to regenerate with cargo-dist, by converting each custom publish job into a standalone tag-triggered workflow.

**Architecture:** Each language-binding release workflow (`python-release.yml`, `c-release.yml`, `node-release.yml`, `java-release.yml`) currently uses a `workflow_call` trigger so it can be called from `release.yml`. We change each to `push: tags` so it fires independently on every version tag, parallel to `release.yml`. A new `docs-check.yml` replaces the inline `docs-check` job. After all standalone workflows exist, `release.yml` is cleaned of all manual additions in a single final commit.

**Tech Stack:** GitHub Actions YAML; `gh` CLI (for wait-loop pattern); `softprops/action-gh-release@v2` replaced by `gh release upload` in `c-release.yml`.

---

### Task 1: Create `docs-check.yml`

**Files:**
- Create: `.github/workflows/docs-check.yml`

- [ ] **Step 1: Create the workflow file**

```yaml
name: Docs Check

on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'
  workflow_dispatch:

jobs:
  docs-check:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@nightly
      - name: Check docs.rs build
        env:
          RUSTDOCFLAGS: "--cfg docsrs"
        run: cargo doc --all-features --no-deps
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/docs-check.yml
git commit -m "ci: add standalone docs-check workflow triggered on tag push"
```

---

### Task 2: Convert `python-release.yml` to standalone

**Files:**
- Modify: `.github/workflows/python-release.yml`

The `tag` input is currently accepted from `workflow_call` but is never used in any step — it was reserved for future use. The `workflow_dispatch` input stays for manual runs.

- [ ] **Step 1: Replace the `on:` block**

Replace:
```yaml
on:
  workflow_call:
    inputs:
      tag:
        # Reserved: the release tag (e.g. "v0.22.0") passed from release.yml.
        # Available as ${{ inputs.tag }} for future use (e.g. annotating wheel metadata).
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        # The release tag to publish wheels for (e.g. "v0.22.0").
        required: true
        type: string
```

With:
```yaml
on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'
  workflow_dispatch:
    inputs:
      tag:
        description: 'Release tag to publish wheels for (e.g. "v0.22.0")'
        required: false
        type: string
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/python-release.yml
git commit -m "ci: convert python-release to standalone tag-triggered workflow"
```

---

### Task 3: Convert `node-release.yml` to standalone

**Files:**
- Modify: `.github/workflows/node-release.yml`

The `tag` input is accepted but unused in any step. Same treatment as Python.

- [ ] **Step 1: Replace the `on:` block**

Replace:
```yaml
on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string
```

With:
```yaml
on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'
  workflow_dispatch:
    inputs:
      tag:
        description: 'Release tag to publish to npm (e.g. "v0.25.0")'
        required: false
        type: string
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/node-release.yml
git commit -m "ci: convert node-release to standalone tag-triggered workflow"
```

---

### Task 4: Convert `java-release.yml` to standalone

**Files:**
- Modify: `.github/workflows/java-release.yml`

`java-release.yml` had `workflow_call` but was never wired into `release.yml` — Java releases were only triggerable via `workflow_dispatch`. This also fixes that automation gap.

- [ ] **Step 1: Replace the `on:` block**

Replace:
```yaml
on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string
```

With:
```yaml
on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'
  workflow_dispatch:
    inputs:
      tag:
        description: 'Release tag to publish to Maven Central (e.g. "v0.23.0")'
        required: false
        type: string
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/java-release.yml
git commit -m "ci: convert java-release to standalone tag-triggered workflow (fixes automation gap)"
```

---

### Task 5: Convert `c-release.yml` to standalone

**Files:**
- Modify: `.github/workflows/c-release.yml`

C release uploads tarballs to the GitHub Release created by cargo-dist. Unlike PyPI/npm/Maven, this requires the GitHub Release to already exist. We replace `softprops/action-gh-release@v2` (which creates a release if absent, risking a race with cargo-dist) with the same wait-loop + `gh release upload` pattern used by `wasm-release.yml`.

`${{ inputs.tag }}` is used in tarball filenames — replace with `${{ inputs.tag || github.ref_name }}` so push-triggered runs get the tag from `github.ref_name`.

- [ ] **Step 1: Replace the `on:` block**

Replace:
```yaml
on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string
```

With:
```yaml
on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'
  workflow_dispatch:
    inputs:
      tag:
        description: 'Release tag to upload C artifacts to (e.g. "v0.24.0")'
        required: false
        type: string
```

- [ ] **Step 2: Replace `${{ inputs.tag }}` with `${{ inputs.tag || github.ref_name }}` in the `build` job**

There are three occurrences in the tarball packaging steps (unix, universal2, windows). Replace all three:

```yaml
      - name: Rename and package (unix)
        if: runner.os != 'Windows' && matrix.target != 'universal2'
        run: |
          cp target/${{ matrix.target }}/release/${{ matrix.src-lib }} ${{ matrix.lib-name }}
          tar czf minigraf-c-${{ inputs.tag || github.ref_name }}-${{ matrix.artifact-name }}.tar.gz \
            ${{ matrix.lib-name }} minigraf-c/include/minigraf.h

      - name: Rename and package (universal2)
        if: matrix.target == 'universal2'
        run: |
          tar czf minigraf-c-${{ inputs.tag || github.ref_name }}-${{ matrix.artifact-name }}.tar.gz \
            ${{ matrix.lib-name }} minigraf-c/include/minigraf.h

      - name: Rename and package (windows)
        if: runner.os == 'Windows'
        run: |
          copy target\${{ matrix.target }}\release\${{ matrix.src-lib }} ${{ matrix.lib-name }}
          Compress-Archive -Path ${{ matrix.lib-name }},minigraf-c\include\minigraf.h `
            -DestinationPath minigraf-c-${{ inputs.tag || github.ref_name }}-${{ matrix.artifact-name }}.zip
        shell: pwsh
```

Also update the artifact upload steps to use the new expression in the `path:` fields:

```yaml
      - name: Upload artifact (unix)
        if: runner.os != 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: c-release-${{ matrix.artifact-name }}
          path: minigraf-c-${{ inputs.tag || github.ref_name }}-${{ matrix.artifact-name }}.tar.gz

      - name: Upload artifact (windows)
        if: runner.os == 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: c-release-${{ matrix.artifact-name }}
          path: minigraf-c-${{ inputs.tag || github.ref_name }}-${{ matrix.artifact-name }}.zip
```

- [ ] **Step 3: Replace the `upload-to-release` job with a wait-loop pattern**

Replace the entire `upload-to-release` job:

```yaml
  upload-to-release:
    name: Upload artifacts to GitHub Release
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write

    steps:
      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          pattern: c-release-*
          path: artifacts
          merge-multiple: true

      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: ${{ inputs.tag }}
          files: artifacts/*
```

With:

```yaml
  upload-to-release:
    name: Upload artifacts to GitHub Release
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write

    steps:
      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          pattern: c-release-*
          path: artifacts
          merge-multiple: true

      - name: Wait for GitHub Release to exist, then upload
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAG: ${{ inputs.tag || github.ref_name }}
        run: |
          echo "Waiting for release $TAG to be created by cargo-dist..."
          for i in $(seq 1 40); do
            if gh release view "$TAG" --repo "$GITHUB_REPOSITORY" > /dev/null 2>&1; then
              echo "Release found on attempt $i"
              break
            fi
            echo "Attempt $i/40: release not yet available, waiting 15s..."
            sleep 15
          done
          if ! gh release view "$TAG" --repo "$GITHUB_REPOSITORY" > /dev/null 2>&1; then
            echo "ERROR: Release $TAG not found after 40 attempts (10 minutes). Aborting."
            exit 1
          fi
          gh release upload "$TAG" artifacts/* \
            --repo "$GITHUB_REPOSITORY" \
            --clobber
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/c-release.yml
git commit -m "ci: convert c-release to standalone tag-triggered workflow with wait-for-release loop"
```

---

### Task 6: Clean up `release.yml`

**Files:**
- Modify: `.github/workflows/release.yml`

This is the only task that touches `release.yml`, and it is the last time it will ever be manually edited. After this commit, `release.yml` is cargo-dist territory.

- [ ] **Step 1: Remove the `docs-check` job**

Remove the entire block (lines 47–58 in the current file):

```yaml
  # Verify docs.rs build passes before publishing
  docs-check:
    runs-on: "ubuntu-22.04"
    steps:
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@nightly
      - name: Check docs.rs build
        env:
          RUSTDOCFLAGS: "--cfg docsrs"
        run: cargo doc --all-features --no-deps
```

- [ ] **Step 2: Remove `docs-check` from the `host` job**

In the `host` job, remove `- docs-check` from the `needs:` list and remove `&& needs.docs-check.result == 'success'` from the `if:` condition.

Before:
```yaml
  host:
    needs:
      - plan
      - build-local-artifacts
      - build-global-artifacts
      - docs-check
    # Only run if we're "publishing", and only if all build jobs didn't fail (skipped is fine)
    if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && needs.docs-check.result == 'success' && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success') && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') }}
```

After:
```yaml
  host:
    needs:
      - plan
      - build-local-artifacts
      - build-global-artifacts
    # Only run if we're "publishing", and only if all build jobs didn't fail (skipped is fine)
    if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success') && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') }}
```

- [ ] **Step 3: Remove the `publish-python`, `publish-c`, and `publish-node` jobs**

Remove the entire blocks for all three jobs (currently at the bottom of `release.yml`):

```yaml
  # Publish Python wheels to PyPI after the GitHub release is created
  publish-python:
    name: Publish Python wheels to PyPI
    needs:
      - plan
      - host
    if: ${{ always() && needs.host.result == 'success' && needs.plan.outputs.publishing == 'true' }}
    uses: ./.github/workflows/python-release.yml
    with:
      tag: ${{ needs.plan.outputs.tag }}
    secrets: inherit

  # Build and publish C library tarballs to GitHub Release after the release is created
  publish-c:
    name: Publish C library to GitHub Release
    needs:
      - plan
      - host
    if: ${{ always() && needs.host.result == 'success' && needs.plan.outputs.publishing == 'true' }}
    uses: ./.github/workflows/c-release.yml
    with:
      tag: ${{ needs.plan.outputs.tag }}
    secrets: inherit
    permissions:
      contents: write

  # Build and publish Node.js bindings to npm after the release is created
  publish-node:
    name: Publish Node.js bindings to npm
    needs:
      - plan
      - host
    if: ${{ always() && needs.host.result == 'success' && needs.plan.outputs.publishing == 'true' }}
    uses: ./.github/workflows/node-release.yml
    with:
      tag: ${{ needs.plan.outputs.tag }}
    secrets: inherit
```

- [ ] **Step 4: Verify `release.yml` looks clean**

After the edits, `release.yml` should contain only these jobs: `plan`, `build-local-artifacts`, `build-global-artifacts`, `host`, `announce`, `publish-crates-io`. Confirm by running:

```bash
grep "^  [a-z]" .github/workflows/release.yml
```

Expected output:
```
  plan:
  build-local-artifacts:
  build-global-artifacts:
  host:
  announce:
  publish-crates-io:
```

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: remove all manual additions from release.yml (now cargo-dist owned)"
```
