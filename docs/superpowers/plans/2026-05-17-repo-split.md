# Repo Split (#231) — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `minigraf-python`, `minigraf-node`, and `minigraf-wasm` out of the monorepo into independent `project-minigraf` org repos wired into a `repository_dispatch` release cascade.

**Architecture:** A new `cascade.yml` workflow in the core repo publishes `minigraf-ffi` to crates.io and fans out `repository_dispatch` events to each binding repo on every version tag. Each binding repo receives the event, stamps the version into its manifests, and publishes to its registry. The monorepo is left with only core Rust + FFI bridge + C bindings after Phase 1.

**Tech Stack:** Rust/Cargo, GitHub Actions, `gh` CLI, maturin (Python), NAPI-RS (Node), wasm-pack (WASM), npm, PyPI

---

## Prerequisites

- [ ] You have `gh` CLI authenticated with an account that has **Owner** access to the `project-minigraf` org
- [ ] An org-level Actions secret `MINIGRAF_RELEASE_TOKEN` exists (PAT with `contents:write` + `actions:write` on the org). If not, create one at `https://github.com/organizations/project-minigraf/settings/secrets/actions` before starting.
- [ ] Confirm `NPM_TOKEN` and PyPI trusted publishing are already configured as org secrets (they were used in the monorepo release workflows).

---

## File Map

**Modified in core repo:**
- `minigraf-ffi/Cargo.toml` — remove `publish = false`, add `version` to `minigraf` path dep
- `Cargo.toml` — remove `minigraf-node` from workspace members
- `.github/workflows/cascade.yml` — **new** publish + dispatch workflow
- `README.md`, `ROADMAP.md`, `CHANGELOG.md` — update cross-references

**Deleted from core repo (after each split):**
- `minigraf-ffi/python/`
- `minigraf-node/`
- `minigraf-wasm/`, `minigraf-wasi/`
- `.github/workflows/python-ci.yml`, `python-release.yml`
- `.github/workflows/node-ci.yml`, `node-release.yml`
- `.github/workflows/wasm-browser.yml`, `wasm-wasi.yml`, `wasm-release.yml`

**New repo: `project-minigraf/minigraf-python`:**
- `Cargo.toml` — thin shim crate providing `uniffi-bindgen` binary
- `src/uniffi_bindgen.rs`
- `pyproject.toml`
- `build.sh`
- `minigraf/__init__.py` (copied)
- `tests/` (copied)
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

**New repo: `project-minigraf/minigraf-node`:**
- `Cargo.toml` — path dep replaced with crates.io version
- `build.rs`, `src/lib.rs`, `index.js`, `index.d.ts`, `package.json` (copied/updated)
- `packages/@minigraf/*/package.json` (copied)
- `test/basic.test.mjs` (copied)
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

**New repo: `project-minigraf/minigraf-wasm`:**
- `Cargo.toml` — thin wrapper crate, `minigraf` dep from crates.io with `browser` feature
- `src/lib.rs` — `pub use minigraf::*;`
- `wasm-pkg/package.json` (browser npm package metadata)
- `wasi-pkg/` (WASI npm package, copied from `minigraf-wasi/`)
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

---

## Task 1: Prepare `minigraf-ffi` for crates.io publication

**Files:**
- Modify: `minigraf-ffi/Cargo.toml`

The crate currently has `publish = false` and its `minigraf` dep has no version field. Both block `cargo publish`.

- [ ] **Step 1: Edit `minigraf-ffi/Cargo.toml`**

Change:
```toml
[package]
name = "minigraf-ffi"
version = "1.1.1"
edition = "2024"
description = "UniFFI mobile bindings for Minigraf (Android + iOS)"
publish = false
```
To:
```toml
[package]
name = "minigraf-ffi"
version = "1.1.1"
edition = "2024"
description = "UniFFI bridge for Minigraf — bi-temporal graph database"
```

And change the `minigraf` dependency from:
```toml
minigraf = { path = ".." }
```
To:
```toml
minigraf = { path = "..", version = "1.1.1" }
```

(`cargo publish` substitutes path deps with their crates.io version when the version field is present.)

- [ ] **Step 2: Dry-run publish to verify the crate is publishable**

```bash
cargo publish --dry-run -p minigraf-ffi 2>&1
```

Expected: output ends with `Uploading minigraf-ffi v1.1.1` with no errors. Warnings about `path` dep substitution are fine.

- [ ] **Step 3: Verify workspace still builds and tests pass**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: all tests pass (935+).

- [ ] **Step 4: Commit**

```bash
git -C /home/aditya/workspaces/rustrover/minigraf add minigraf-ffi/Cargo.toml
git -C /home/aditya/workspaces/rustrover/minigraf commit -m "chore(ffi): prepare minigraf-ffi for crates.io publication (#231)"
```

- [ ] **Step 5: Publish `minigraf-ffi` to crates.io**

```bash
cargo publish -p minigraf-ffi
```

Expected: `Uploading minigraf-ffi v1.1.1` then `Published minigraf-ffi v1.1.1 to registry`. Wait ~60s for crates.io to index it before proceeding.

---

## Task 2: Create `minigraf-python` repo

**Files:** All new files in the new GitHub repo.

- [ ] **Step 1: Create the repo**

```bash
gh repo create project-minigraf/minigraf-python \
  --public \
  --description "Python bindings for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-python
```

- [ ] **Step 2: Create `Cargo.toml`**

The new repo needs its own Cargo workspace so maturin can find `uniffi-bindgen`. This thin shim provides only the binary; maturin's `manifest-path` points here.

```toml
[package]
name = "minigraf-python-shim"
version = "0.0.0"
edition = "2024"
publish = false
description = "Build shim: provides uniffi-bindgen binary for maturin"

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
minigraf-ffi = "1.1.1"
uniffi = { version = "0.31.1", features = ["cli"] }

[workspace]
members = ["."]
```

- [ ] **Step 3: Create `src/uniffi_bindgen.rs`**

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

- [ ] **Step 4: Create `pyproject.toml`**

```toml
[build-system]
requires = ["maturin>=1.5,<2"]
build-backend = "maturin"

[project]
name = "minigraf"
version = "1.1.1"
description = "Zero-config, single-file, embedded graph database with bi-temporal Datalog queries"
license = { text = "MIT OR Apache-2.0" }
requires-python = ">=3.9"
keywords = ["graph", "datalog", "bitemporal", "embedded", "database"]
classifiers = [
    "Programming Language :: Rust",
    "Programming Language :: Python :: Implementation :: CPython",
    "Programming Language :: Python :: 3",
    "License :: OSI Approved :: MIT License",
    "License :: OSI Approved :: Apache Software License",
]

[project.urls]
Homepage = "https://github.com/project-minigraf/minigraf"
Repository = "https://github.com/project-minigraf/minigraf-python"
"Bug Tracker" = "https://github.com/project-minigraf/minigraf-python/issues"

[tool.maturin]
manifest-path = "Cargo.toml"
bindings = "uniffi"
python-packages = ["minigraf"]
module-name = "minigraf.minigraf_ffi"
```

- [ ] **Step 5: Create `build.sh`**

```bash
#!/usr/bin/env bash
# build.sh — build and install the minigraf Python extension.
#
# maturin (bindings = "uniffi") calls `cargo run --bin uniffi-bindgen` to
# generate Python bindings. This repo contains a thin shim crate (Cargo.toml)
# whose sole purpose is to provide that binary via minigraf-ffi from crates.io.
#
# Requires an active virtualenv (or conda environment).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Pre-build the uniffi-bindgen binary from the shim crate.
cargo build --bin uniffi-bindgen

SUBCOMMAND="${1:-}"

if [[ "$SUBCOMMAND" == "build" ]]; then
    shift
    maturin build "$@"
else
    maturin develop
    [[ "$SUBCOMMAND" == "test" ]] && pytest tests/ -v
fi
```

```bash
chmod +x build.sh
```

- [ ] **Step 6: Copy Python package and tests from monorepo**

```bash
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/python/minigraf ./minigraf
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/python/tests ./tests
```

- [ ] **Step 7: Verify the build works locally**

```bash
python -m venv .venv
source .venv/bin/activate
pip install maturin pytest
bash build.sh test
```

Expected: all Python tests pass.

- [ ] **Step 8: Create `.github/workflows/ci.yml`**

```yaml
name: Python CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    name: Python tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install maturin
        run: pip install "maturin>=1.5"

      - name: Create virtualenv
        run: python -m venv .venv

      - name: Build and test
        run: |
          source .venv/bin/activate 2>/dev/null || source .venv/Scripts/activate
          pip install pytest
          bash build.sh test
        shell: bash
```

- [ ] **Step 9: Create `.github/workflows/release.yml`**

```yaml
name: Python Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to release (e.g. v1.2.0)'
        required: true
        type: string

jobs:
  set-version:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.ver.outputs.version }}
    steps:
      - id: ver
        run: |
          if [ "${{ github.event_name }}" = "repository_dispatch" ]; then
            echo "version=${{ github.event.client_payload.version }}" >> "$GITHUB_OUTPUT"
          else
            echo "version=${{ inputs.version }}" >> "$GITHUB_OUTPUT"
          fi

  build-wheels:
    name: Build wheel (${{ matrix.os }})
    needs: set-version
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            build_args: --release --manylinux manylinux_2_34 --out dist
          - os: ubuntu-24.04-arm
            build_args: --release --manylinux manylinux_2_34 --out dist
          - os: macos-14
            build_args: --release --target universal2-apple-darwin --out dist
          - os: windows-latest
            build_args: --release --out dist

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-apple-darwin

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install maturin
        run: pip install "maturin>=1.5"

      - name: Stamp version into pyproject.toml
        shell: bash
        run: |
          TAG="${{ needs.set-version.outputs.version }}"
          VERSION="${TAG#v}"
          python3 -c "
          import re, pathlib
          p = pathlib.Path('pyproject.toml')
          p.write_text(re.sub(r'^version = .*', 'version = \"${VERSION}\"', p.read_text(), flags=re.MULTILINE))
          "

      - name: Build wheel
        run: bash build.sh build ${{ matrix.build_args }}
        shell: bash

      - name: Upload wheel
        uses: actions/upload-artifact@v4
        with:
          name: python-wheel-${{ matrix.os }}
          path: dist/*.whl

  publish:
    name: Publish to PyPI
    needs: [set-version, build-wheels]
    runs-on: ubuntu-latest
    environment:
      name: pypi
      url: https://pypi.org/p/minigraf
    permissions:
      id-token: write

    steps:
      - name: Download all wheels
        uses: actions/download-artifact@v4
        with:
          pattern: python-wheel-*
          path: dist
          merge-multiple: true

      - name: Publish to PyPI
        uses: pypa/gh-action-pypi-publish@release/v1
        with:
          packages-dir: dist/
```

- [ ] **Step 10: Create `.gitignore`**

```
/target/
/.venv/
dist/
*.whl
__pycache__/
*.pyc
# local dev override — never commit
Cargo.toml.local
```

- [ ] **Step 11: Initial commit and push**

```bash
git add .
git commit -m "feat: initial Python binding repo (split from minigraf monorepo)"
git push -u origin main
```

- [ ] **Step 12: Verify CI passes**

```bash
gh run watch --repo project-minigraf/minigraf-python
```

Expected: all matrix jobs green.

---

## Task 3: Create `minigraf-node` repo

**Files:** All new files in the new GitHub repo.

- [ ] **Step 1: Create the repo**

```bash
gh repo create project-minigraf/minigraf-node \
  --public \
  --description "Node.js bindings for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-node
```

- [ ] **Step 2: Copy source files from monorepo**

```bash
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-node/src ./src
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-node/build.rs ./build.rs
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-node/index.js ./index.js
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-node/index.d.ts ./index.d.ts
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-node/package.json ./package.json
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-node/packages ./packages
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-node/test ./test
```

- [ ] **Step 3: Create `Cargo.toml`** (path dep replaced with crates.io version)

```toml
[package]
name = "minigraf-node"
version = "1.1.1"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
napi = { version = "3.8", features = ["napi6"] }
napi-derive = "3"
minigraf = { version = "1.1.1" }
serde_json = "1.0"

[build-dependencies]
napi-build = "2.1"

[workspace]
members = ["."]
```

- [ ] **Step 4: Update `package.json` repository URL**

Edit `package.json` — change the `"url"` field in `"repository"`:
```json
"repository": {
  "type": "git",
  "url": "https://github.com/project-minigraf/minigraf-node.git"
},
```

- [ ] **Step 5: Verify local build**

```bash
npm install
npx napi build --platform --release
node --test test/basic.test.mjs
```

Expected: all Node tests pass.

- [ ] **Step 6: Create `.github/workflows/ci.yml`**

```yaml
name: Node.js CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    name: Node.js tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install npm dependencies
        run: npm install

      - name: Build native addon
        run: npx napi build --platform --release

      - name: Run tests
        run: node --test test/basic.test.mjs
```

- [ ] **Step 7: Create `.github/workflows/release.yml`**

```yaml
name: Node.js Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to release (e.g. v1.2.0)'
        required: true
        type: string

jobs:
  set-version:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.ver.outputs.version }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - id: ver
        run: |
          if [ "${{ github.event_name }}" = "repository_dispatch" ]; then
            TAG="${{ github.event.client_payload.version }}"
          else
            TAG="${{ inputs.version }}"
          fi
          echo "version=$TAG" >> "$GITHUB_OUTPUT"
          echo "semver=${TAG#v}" >> "$GITHUB_OUTPUT"

  build:
    name: Build .node binary (${{ matrix.settings.host }})
    needs: set-version
    runs-on: ${{ matrix.settings.host }}
    strategy:
      fail-fast: false
      matrix:
        settings:
          - host: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact-name: minigraf-linux-x64-gnu
          - host: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            artifact-name: minigraf-linux-arm64-gnu
          - host: macos-14
            target: universal-apple-darwin
            artifact-name: minigraf-darwin-universal
          - host: windows-latest
            target: x86_64-pc-windows-msvc
            artifact-name: minigraf-win32-x64-msvc

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.settings.target == 'universal-apple-darwin' && 'aarch64-apple-darwin x86_64-apple-darwin' || matrix.settings.target }}

      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'

      - name: Install npm dependencies
        run: npm install

      - name: Build native addon
        shell: bash
        run: |
          if [ "${{ matrix.settings.target }}" = "universal-apple-darwin" ]; then
            npx napi build --platform --release --target aarch64-apple-darwin
            npx napi build --platform --release --target x86_64-apple-darwin
            lipo -create -output minigraf.darwin-universal.node \
              minigraf.darwin-arm64.node minigraf.darwin-x64.node
            rm minigraf.darwin-arm64.node minigraf.darwin-x64.node
          else
            npx napi build --platform --release --target "${{ matrix.settings.target }}"
          fi

      - name: Upload .node binary
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.settings.artifact-name }}
          path: "*.node"

  publish:
    name: Publish to npm
    needs: [set-version, build]
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'

      - name: Stamp version into package.json
        run: |
          VERSION="${{ needs.set-version.outputs.semver }}"
          node -e "
            const fs = require('fs');
            const p = 'package.json';
            const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
            pkg.version = '${VERSION}';
            fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
          "

      - name: Install npm dependencies
        run: npm install

      - name: Download all binaries
        uses: actions/download-artifact@v4
        with:
          pattern: minigraf-*
          path: .
          merge-multiple: true

      - name: Distribute .node binaries to platform packages
        run: |
          cp minigraf.linux-x64-gnu.node   packages/@minigraf/linux-x64-gnu/
          cp minigraf.linux-arm64-gnu.node packages/@minigraf/linux-arm64-gnu/
          cp minigraf.darwin-universal.node packages/@minigraf/darwin-universal/
          cp minigraf.win32-x64-msvc.node  packages/@minigraf/win32-x64-msvc/

      - name: Sync platform package versions
        run: |
          VERSION="${{ needs.set-version.outputs.semver }}"
          for dir in packages/@minigraf/*/; do
            node -e "
              const fs = require('fs');
              const pkg = JSON.parse(fs.readFileSync('${dir}package.json', 'utf8'));
              pkg.version = '${VERSION}';
              fs.writeFileSync('${dir}package.json', JSON.stringify(pkg, null, 2) + '\n');
            "
          done

      - name: Publish platform packages
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          for dir in packages/@minigraf/*/; do
            npm publish "$dir" --access public || echo "Skipping $dir (already published or error)"
          done

      - name: Publish main minigraf package
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: npm publish --access public
```

- [ ] **Step 8: Create `.gitignore`**

```
/target/
node_modules/
*.node
```

- [ ] **Step 9: Initial commit and push**

```bash
git add .
git commit -m "feat: initial Node.js binding repo (split from minigraf monorepo)"
git push -u origin main
```

- [ ] **Step 10: Verify CI passes**

```bash
gh run watch --repo project-minigraf/minigraf-node
```

Expected: all matrix jobs green.

---

## Task 4: Create `minigraf-wasm` repo

**Files:** All new files in the new GitHub repo.

The browser WASM and WASI builds both compile the `minigraf` crate directly (it contains the `#[wasm_bindgen]` exports). The new repo provides a thin re-export crate so `wasm-pack build` can target it.

- [ ] **Step 1: Create the repo**

```bash
gh repo create project-minigraf/minigraf-wasm \
  --public \
  --description "Browser WASM and WASI bindings for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-wasm
```

- [ ] **Step 2: Create `Cargo.toml`**

```toml
[package]
name = "minigraf-wasm"
version = "1.1.1"
edition = "2024"
publish = false
description = "wasm-pack build shim for Minigraf browser and WASI targets"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
minigraf = { version = "1.1.1", features = ["browser"] }

[target.'cfg(all(target_arch = "wasm32", not(target_os = "wasi")))'.dependencies]
minigraf = { version = "1.1.1", features = ["browser"] }

[features]
browser = ["minigraf/browser"]

[workspace]
members = ["."]
```

- [ ] **Step 3: Create `src/lib.rs`**

```rust
// Re-export all wasm_bindgen exports from the published minigraf crate.
// wasm-pack discovers #[wasm_bindgen] symbols at link time from the cdylib,
// so transitive re-exports are included in the generated JS/TS bindings.
pub use minigraf::*;
```

- [ ] **Step 4: Copy WASI npm package from monorepo**

```bash
mkdir wasi-pkg
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-wasi/index.js        ./wasi-pkg/index.js
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-wasi/index.d.ts      ./wasi-pkg/index.d.ts
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-wasi/package.json    ./wasi-pkg/package.json
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-wasi/package.test.js ./wasi-pkg/package.test.js
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-wasi/README.md       ./wasi-pkg/README.md
```

- [ ] **Step 5: Update `wasi-pkg/package.json` repository URL**

Edit `wasi-pkg/package.json` — change the `"url"` field:
```json
"repository": {
  "type": "git",
  "url": "https://github.com/project-minigraf/minigraf-wasm.git"
},
```

- [ ] **Step 6: Verify browser WASM build works**

```bash
rustup target add wasm32-unknown-unknown
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
wasm-pack build --target web --features browser
```

Expected: `pkg/` directory created with `.wasm`, `.js`, `.d.ts` files. If `wasm-pack` does not pick up the re-exported `#[wasm_bindgen]` items (empty JS bindings), see the note at the bottom of this task.

- [ ] **Step 7: Verify WASI build works**

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release \
  --manifest-path /home/aditya/workspaces/rustrover/minigraf/Cargo.toml \
  --bin minigraf
```

Note: the WASI build compiles the `minigraf` REPL binary from the core repo. In the new `minigraf-wasm` repo CI, this is done by checking out the core repo at the matching tag and running `cargo build --target wasm32-wasip1 --release --bin minigraf` from it. The `wasi-pkg/` directory in this repo contains only the JS wrapper; the `.wasm` binary is produced during CI.

- [ ] **Step 8: Create `.github/workflows/ci.yml`**

```yaml
name: WASM CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build-browser:
    name: Build browser WASM
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
      - name: Build browser WASM
        run: wasm-pack build --target web --features browser

  build-wasi:
    name: Build WASI binary
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: project-minigraf/minigraf
          ref: main
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1
      - name: Build WASI binary
        run: cargo build --target wasm32-wasip1 --release --bin minigraf
```

- [ ] **Step 9: Create `.github/workflows/release.yml`**

```yaml
name: WASM Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to release (e.g. v1.2.0)'
        required: true
        type: string

jobs:
  set-version:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.ver.outputs.version }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - id: ver
        run: |
          if [ "${{ github.event_name }}" = "repository_dispatch" ]; then
            TAG="${{ github.event.client_payload.version }}"
          else
            TAG="${{ inputs.version }}"
          fi
          echo "version=$TAG" >> "$GITHUB_OUTPUT"
          echo "semver=${TAG#v}" >> "$GITHUB_OUTPUT"

  build-browser:
    name: Build browser WASM
    needs: set-version
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'
      - name: Build browser WASM
        run: wasm-pack build --target web --features browser
      - name: Stamp package name and version
        env:
          VERSION: ${{ needs.set-version.outputs.semver }}
        run: |
          if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            echo "ERROR: VERSION '$VERSION' is not semver. Aborting."
            exit 1
          fi
          node -e "
            const fs = require('fs');
            const p = 'pkg/package.json';
            const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
            pkg.name = '@minigraf/browser';
            pkg.version = process.env.VERSION;
            fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
          "
      - name: Publish @minigraf/browser to npm
        working-directory: pkg
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: npm publish --access public

  build-wasi:
    name: Build and publish WASI
    needs: set-version
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: project-minigraf/minigraf
          ref: ${{ needs.set-version.outputs.version }}
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1
      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'
      - name: Build WASI binary
        run: cargo build --target wasm32-wasip1 --release --bin minigraf
      - name: Check out wasm binding repo for wasi-pkg
        uses: actions/checkout@v4
        with:
          repository: project-minigraf/minigraf-wasm
          path: wasm-binding
      - name: Stage WASI npm package
        run: cp target/wasm32-wasip1/release/minigraf.wasm wasm-binding/wasi-pkg/minigraf-wasi.wasm
      - name: Stamp package version
        env:
          VERSION: ${{ needs.set-version.outputs.semver }}
        run: |
          if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            echo "ERROR: VERSION '$VERSION' is not semver. Aborting."
            exit 1
          fi
          node -e "
            const fs = require('fs');
            const p = 'wasm-binding/wasi-pkg/package.json';
            const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
            pkg.version = process.env.VERSION;
            fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
          "
      - name: Test WASI package
        working-directory: wasm-binding/wasi-pkg
        run: npm test
      - name: Publish @minigraf/wasi to npm
        working-directory: wasm-binding/wasi-pkg
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: npm publish --access public
```

- [ ] **Step 10: Create `.gitignore`**

```
/target/
/pkg/
node_modules/
wasi-pkg/minigraf-wasi.wasm
```

- [ ] **Step 11: Initial commit and push**

```bash
git add .
git commit -m "feat: initial WASM/WASI binding repo (split from minigraf monorepo)"
git push -u origin main
```

- [ ] **Step 12: Verify CI passes**

```bash
gh run watch --repo project-minigraf/minigraf-wasm
```

Expected: both browser and WASI build jobs green.

> **Note on wasm-pack re-exports:** If Step 6 produces empty JS bindings (no exported functions), it means `wasm-pack` is not picking up the `#[wasm_bindgen]` items through the `pub use minigraf::*` re-export. The fix: add `wasm-bindgen = "0.2"` as a direct dependency and explicitly re-export the types in `src/lib.rs`:
> ```rust
> pub use minigraf::{MiniGrafDb, /* other exported types */};
> ```
> Then re-run `wasm-pack build --target web --features browser` and verify `pkg/minigraf.js` contains the expected exports.

---

## Task 5: Add `cascade.yml` to core and clean up monorepo

**Files:**
- Create: `.github/workflows/cascade.yml`
- Modify: `Cargo.toml`
- Delete: binding directories and their workflows
- Modify: `README.md`, `ROADMAP.md`, `CHANGELOG.md`

All changes are in the `minigraf` core repo at `/home/aditya/workspaces/rustrover/minigraf/`.

- [ ] **Step 1: Create `.github/workflows/cascade.yml`**

```yaml
name: Release Cascade

on:
  push:
    tags:
      - '**[0-9]+.[0-9]+.[0-9]+*'

jobs:
  publish-ffi:
    name: Publish minigraf-ffi to crates.io
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Publish minigraf-ffi
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        run: cargo publish -p minigraf-ffi

      - name: Wait for minigraf-ffi to be indexed on crates.io
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          echo "Waiting for minigraf-ffi@$VERSION on crates.io..."
          for i in $(seq 1 18); do
            STATUS=$(curl -s "https://crates.io/api/v1/crates/minigraf-ffi/$VERSION" \
              -H "User-Agent: minigraf-cascade/1.0" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('version',{}).get('num',''))" 2>/dev/null || echo "")
            if [ "$STATUS" = "$VERSION" ]; then
              echo "minigraf-ffi@$VERSION is live (attempt $i)"
              break
            fi
            echo "Attempt $i/18: not yet available, waiting 10s..."
            sleep 10
          done

      - name: Wait for minigraf to be indexed on crates.io
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          echo "Waiting for minigraf@$VERSION on crates.io..."
          for i in $(seq 1 18); do
            STATUS=$(curl -s "https://crates.io/api/v1/crates/minigraf/$VERSION" \
              -H "User-Agent: minigraf-cascade/1.0" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('version',{}).get('num',''))" 2>/dev/null || echo "")
            if [ "$STATUS" = "$VERSION" ]; then
              echo "minigraf@$VERSION is live (attempt $i)"
              break
            fi
            echo "Attempt $i/18: not yet available, waiting 10s..."
            sleep 10
          done

  dispatch-bindings:
    name: Dispatch to binding repos
    needs: publish-ffi
    runs-on: ubuntu-latest
    steps:
      - name: Dispatch to Phase 1 binding repos
        env:
          GH_TOKEN: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}
        run: |
          VERSION="${GITHUB_REF_NAME}"
          for REPO in minigraf-python minigraf-node minigraf-wasm; do
            echo "Dispatching core-release to project-minigraf/$REPO @ $VERSION"
            gh api repos/project-minigraf/$REPO/dispatches \
              -f event_type=core-release \
              -f "client_payload[version]=$VERSION"
          done
```

- [ ] **Step 2: Remove binding directories from monorepo**

```bash
cd /home/aditya/workspaces/rustrover/minigraf
git rm -r minigraf-node/
git rm -r minigraf-ffi/python/
git rm -r minigraf-wasm/
git rm -r minigraf-wasi/
```

- [ ] **Step 3: Remove binding CI workflows**

```bash
git rm .github/workflows/python-ci.yml
git rm .github/workflows/python-release.yml
git rm .github/workflows/node-ci.yml
git rm .github/workflows/node-release.yml
git rm .github/workflows/wasm-browser.yml
git rm .github/workflows/wasm-wasi.yml
git rm .github/workflows/wasm-release.yml
```

- [ ] **Step 4: Update `Cargo.toml` workspace members**

In `Cargo.toml`, change:
```toml
members = [".", "minigraf-ffi", "minigraf-c", "minigraf-node"]
```
To:
```toml
members = [".", "minigraf-ffi", "minigraf-c"]
```

Also remove the workspace `uniffi-bindgen` stub binary entry (no longer needed since python split is out):

In `Cargo.toml`, remove this section entirely:
```toml
# Stub that lets `cargo run --bin uniffi-bindgen` work from the workspace root.
# maturin calls this when bindings = "uniffi"; it execs the binary pre-built by
# build.sh (cargo build --bin uniffi-bindgen --manifest-path minigraf-ffi/Cargo.toml).
# Uses only std — no extra dependencies. Excluded from `cargo build --lib` so
# WASM builds are unaffected.
[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi_bindgen_stub.rs"
doc = false
```

And remove the stub file:
```bash
git rm src/bin/uniffi_bindgen_stub.rs
# Remove src/bin/ if now empty
rmdir src/bin/ 2>/dev/null || true
```

- [ ] **Step 5: Verify workspace still builds cleanly**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: all tests pass (count should be stable since no Rust tests moved — Node and Python tests are in their own repos now).

- [ ] **Step 6: Update `README.md`**

Find the section that lists binding packages (look for `minigraf-python`, `minigraf-node`, `@minigraf/browser`). Add a note that each binding now lives in its own repo:

```markdown
## Language Bindings

| Language | Package | Repo |
|---|---|---|
| Python | [`minigraf` on PyPI](https://pypi.org/p/minigraf) | [minigraf-python](https://github.com/project-minigraf/minigraf-python) |
| Node.js | [`minigraf` on npm](https://www.npmjs.com/package/minigraf) | [minigraf-node](https://github.com/project-minigraf/minigraf-node) |
| Browser WASM | [`@minigraf/browser` on npm](https://www.npmjs.com/package/@minigraf/browser) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |
| WASI | [`@minigraf/wasi` on npm](https://www.npmjs.com/package/@minigraf/wasi) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |
| Java | (deferred — Phase 2) | — |
| Android | (deferred — Phase 2) | — |
| iOS/macOS | (deferred — Phase 2) | — |
| C | [`minigraf-c`](./minigraf-c) (in this repo) | — |
```

- [ ] **Step 7: Update `ROADMAP.md` "Current Focus"**

Find the `**Next gate**: #231` line and update it to reflect completion. Add a note about the Phase 2 deferred splits (`minigraf-java`, `minigraf-android`, `minigraf-swift`).

- [ ] **Step 8: Update `CHANGELOG.md`**

Add an entry under the latest version (or a new `## Unreleased` section) noting the Phase 1 repo split.

- [ ] **Step 8a: Update `CLAUDE.md`**

Update the "Key Files for the Next Phase" section to reflect that Wave 5 is now unblocked. Change the line:

```
**Next gate**: #231 — Repo Split (gates Wave 5 and beyond; ecosystem work is cleaner post-split)
```

to note #231 is complete and Wave 5 is next.

- [ ] **Step 9: Commit everything**

```bash
git add Cargo.toml .github/workflows/cascade.yml README.md ROADMAP.md CHANGELOG.md
git commit -m "feat(split): Phase 1 repo split — python, node, wasm to separate repos (#231)

- Remove minigraf-node, minigraf-ffi/python, minigraf-wasm, minigraf-wasi
- Remove 7 binding CI workflows (python-{ci,release}, node-{ci,release}, wasm-{browser,wasi,release})
- Add cascade.yml: publishes minigraf-ffi + dispatches to binding repos on tag
- Update workspace members and README binding table"
```

- [ ] **Step 10: Verify core CI still passes**

```bash
gh run watch --repo project-minigraf/minigraf
```

Expected: `rust.yml`, `rust-clippy.yml`, `rustfmt.yml`, `coverage.yml` all green. `cascade.yml` only triggers on tag, so it won't run here.

---

## Task 6: Smoke-test the full cascade

Verify the end-to-end release cascade works before closing the issue.

- [ ] **Step 1: Check `CARGO_REGISTRY_TOKEN` is present in core repo secrets**

```bash
gh secret list --repo project-minigraf/minigraf | grep CARGO
```

Expected: `CARGO_REGISTRY_TOKEN` listed. If absent, add it before proceeding.

- [ ] **Step 2: Test `cascade.yml` via `workflow_dispatch` dry run**

`cascade.yml` is tag-triggered only and doesn't support `workflow_dispatch`. Instead, verify by inspecting that the workflow YAML is valid:

```bash
cd /home/aditya/workspaces/rustrover/minigraf
gh workflow view cascade.yml
```

Expected: workflow listed with correct triggers.

- [ ] **Step 3: Test each binding repo's release workflow via `workflow_dispatch`**

For each binding repo, trigger a dry run with the current version (won't re-publish since the version is already on the registry):

```bash
gh workflow run release.yml \
  --repo project-minigraf/minigraf-python \
  -f version=v1.1.1

gh workflow run release.yml \
  --repo project-minigraf/minigraf-node \
  -f version=v1.1.1

gh workflow run release.yml \
  --repo project-minigraf/minigraf-wasm \
  -f version=v1.1.1
```

- [ ] **Step 4: Watch all three run to completion**

```bash
gh run watch --repo project-minigraf/minigraf-python
gh run watch --repo project-minigraf/minigraf-node
gh run watch --repo project-minigraf/minigraf-wasm
```

Expected: all three complete. Publish steps will either succeed (if not yet published at that version) or skip with "already published" messages.

- [ ] **Step 5: Close issue #231 with a comment**

```bash
gh issue comment 231 \
  --repo project-minigraf/minigraf \
  --body "Phase 1 complete: minigraf-python, minigraf-node, and minigraf-wasm are now in separate repos under the project-minigraf org. The cascade.yml workflow in this repo publishes minigraf-ffi and dispatches releases to all three on every version tag. Phase 2 (Java, Android, Swift) is deferred."

gh issue close 231 --repo project-minigraf/minigraf
```

---

## Self-Review Notes

- **`uniffi-bindgen` stub**: `src/bin/uniffi_bindgen_stub.rs` is removed from the monorepo in Task 5. The stub's only purpose was to let maturin find `uniffi-bindgen` during Python builds — that's now handled by the `minigraf-python` repo's own shim crate.
- **`wasm-pack` re-export caveat**: Task 4 Step 6 includes a fallback note if `pub use minigraf::*` doesn't surface the `#[wasm_bindgen]` exports. Resolve before pushing CI.
- **WASI `cargo build` in `minigraf-wasm` CI**: the WASI build checks out the core `minigraf` repo at the release tag to compile the REPL binary. This requires the core tag to be pushed before the dispatch fires — which is guaranteed because `cascade.yml` runs after the tag is pushed.
- **`CARGO_REGISTRY_TOKEN`**: Must exist in core repo secrets for `cascade.yml`'s publish step. This is the same token already used for `cargo publish minigraf` — confirm it's in the repo/org secrets before the next real release.
