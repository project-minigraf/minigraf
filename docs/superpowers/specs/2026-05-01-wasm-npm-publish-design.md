# Design: WASM npm Publish + Package Folder Naming Consistency

**Date**: 2026-05-01
**Status**: Approved

## Problem

Two oversights in the current release pipeline:

1. The browser WASM package (`pkg/`) is never published to npm. The `wasm-release.yml` workflow
   builds it and uploads it as a GitHub Release asset (`.tar.gz`) but has no `npm publish` step.
2. Top-level package folder names are inconsistent: `minigraf-node/` and `minigraf-c/` use the
   `minigraf-*` prefix, but `swift/` does not, and `pkg/` is the wasm-pack default output name
   rather than a meaningful package name.

## Goals

- Publish the browser WASM package to npm as `@minigraf/browser` on every tagged release.
- Rename `pkg/` → `minigraf-wasm/` and `swift/` → `minigraf-swift/` for uniform top-level naming.
- Keep the committed-source policy consistent with `minigraf-node/` (glue files checked in).

## Out of Scope

- WASI npm publish (`@minigraf/wasi`) — the WASI build produces a raw `.wasm` with no JS glue
  and needs its own package structure. Tracked as a follow-up GitHub issue.
- `minigraf-ffi/` sub-packages (`java/`, `python/`, `android/`) — already consistently named.

## Design

### Directory Renames

| Before | After |
|--------|-------|
| `pkg/` | `minigraf-wasm/` |
| `swift/` | `minigraf-swift/` |

`pkg/.gitignore` (contains `*`, a wasm-pack default) is deleted. `minigraf-wasm/` is a committed
source directory; no blanket ignore belongs there.

### `minigraf-wasm/package.json`

- `"name"`: `"minigraf"` → `"@minigraf/browser"`
- `"version"`: reset to `"0.0.0"` (CI stamps the real version from the tag at publish time)
- All other fields unchanged

### CI Workflow Changes

**`wasm-browser.yml`** (build + test on PR/push to main):
- Add `--out-dir minigraf-wasm` to `wasm-pack build` and `wasm-pack test` invocations
- Update gzip size check path: `pkg/minigraf_bg.wasm` → `minigraf-wasm/minigraf_bg.wasm`
- Update artifact upload path: `pkg/` → `minigraf-wasm/`

**`wasm-release.yml`** (tag-triggered release):
- Add `--out-dir minigraf-wasm` to the `wasm-pack build` invocation
- Update `tar` path: `-C . pkg/` → `-C . minigraf-wasm/`
- Keep the GitHub Release tarball upload — the browser WASM bundle is useful as a direct
  download (CDN embedding, `<script type="module">` without npm). Zero maintenance cost to
  keep both distribution channels. Consistent with WASI, which is Release-asset-only.
- Add a new `publish-npm` job (needs: `build-wasm-browser`):
  - Checks out repo at the release tag
  - Downloads the `artifacts-wasm-browser` artifact (freshly-built `minigraf-wasm/` contents)
  - Stamps version from tag using `node -e` (same pattern as `node-release.yml`)
  - Runs `npm publish --access public` from the downloaded artifact directory
  - Authenticates via `NPM_TOKEN` secret
  - Note: uses the artifact from `build-wasm-browser`, not the committed `minigraf-wasm/`
    files, to ensure the published WASM matches the release build

**`mobile.yml`** (Swift XCFramework build):
- All `swift/Sources/MinigrafKit` paths → `minigraf-swift/Sources/MinigrafKit`
- `git add Package.swift swift/Sources/MinigrafKit/` → `git add Package.swift minigraf-swift/Sources/MinigrafKit/`

**`Package.swift`** (root Swift Package Manager manifest):
- `path: "swift/Sources/MinigrafKit"` → `path: "minigraf-swift/Sources/MinigrafKit"`

### Example and Documentation Updates

- `examples/browser/app.js`: import path `../../pkg/minigraf.js` → `../../minigraf-wasm/minigraf.js`
- `examples/browser/README.md`: `pkg/` → `minigraf-wasm/`
- Living docs (`README.md`, `ROADMAP.md`, `CHANGELOG.md`, `TEST_COVERAGE.md`): `pkg/` → `minigraf-wasm/`
- Historical plan/spec docs under `docs/superpowers/`: left unchanged (frozen records)

## Follow-up

Create a GitHub issue to track `@minigraf/wasi` npm publish (needs its own `package.json` and
a JS loader wrapper before it can be published).
