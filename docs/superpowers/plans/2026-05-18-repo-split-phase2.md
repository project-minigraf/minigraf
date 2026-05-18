# Repo Split Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split Java, Android, Swift, and C bindings out of the monorepo into four independent repos; create a `minigraf-binding-template` repo for contributors; retire `minigraf-ffi`; update cascade to dispatch to all four new binding repos.

**Architecture:** Each new binding repo is a standalone Rust workspace that depends directly on the published `minigraf` core crate (not `minigraf-ffi`), plus the language-specific build tooling copied from the monorepo. On receiving a `core-release` repository_dispatch, each repo pins the new `minigraf` version in `Cargo.toml`, commits, tags, builds, and publishes independently. The monorepo then has its bindings-related source directories and workflows deleted; the cascade job is simplified to a single dispatch step.

**Tech Stack:** Rust (UniFFI 0.31.1, cbindgen 0.29.2), Kotlin/Gradle/NMCP (Java + Android), Swift/Xcode (iOS), C/cbindgen, GitHub Actions, Maven Central (Java + Android), GitHub Releases (Swift + C).

**Spec:** `docs/superpowers/specs/2026-05-18-repo-split-phase2-design.md`

---

## Prerequisites

Before starting, verify:

1. `gh auth status` — authenticated to GitHub with a token that has `repo` and `admin:org` scope on `project-minigraf`
2. `cargo login` — authenticated to crates.io (needed for Task 8)
3. Monorepo is at `/home/aditya/workspaces/rustrover/minigraf` — all copy commands use this absolute path

After creating each new repo (Tasks 1–5), configure these secrets in GitHub repo settings before the first release:

| Repo | Secrets needed |
|---|---|
| minigraf-binding-template | none |
| minigraf-c | `MINIGRAF_RELEASE_TOKEN` |
| minigraf-java | `MINIGRAF_RELEASE_TOKEN`, `CENTRAL_TOKEN_USERNAME`, `CENTRAL_TOKEN_PASSWORD`, `GPG_SIGNING_KEY`, `GPG_SIGNING_PASSWORD` |
| minigraf-android | `MINIGRAF_RELEASE_TOKEN`, `CENTRAL_TOKEN_USERNAME`, `CENTRAL_TOKEN_PASSWORD`, `GPG_SIGNING_KEY`, `GPG_SIGNING_PASSWORD` |
| minigraf-swift | `MINIGRAF_RELEASE_TOKEN` |

`MINIGRAF_RELEASE_TOKEN` is already configured as an org-level secret in `project-minigraf` — it should be inherited automatically. The Maven Central secrets must be added per-repo (or promoted to org-level).

---

## File Map

### New repos (created outside monorepo)

**`project-minigraf/minigraf-binding-template`**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/uniffi_bindgen.rs`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`

**`project-minigraf/minigraf-c`**
- Create: `Cargo.toml`
- Copy: `src/lib.rs` ← `minigraf-c/src/lib.rs`
- Copy: `include/minigraf.h` ← `minigraf-c/include/minigraf.h`
- Modify: `cbindgen.toml` ← `minigraf-c/cbindgen.toml` (update paths + crate name)
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `README.md`

**`project-minigraf/minigraf-java`**
- Create: `Cargo.toml`
- Copy: `src/lib.rs` ← `minigraf-ffi/src/lib.rs`
- Copy: `src/uniffi_bindgen.rs` ← `minigraf-ffi/src/uniffi_bindgen.rs`
- Copy: `java/` ← `minigraf-ffi/java/` (entire directory)
- Modify: `java/build.gradle.kts` (fix `repoRoot` path — one level up, not two)
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `README.md`

**`project-minigraf/minigraf-android`**
- Create: `Cargo.toml`
- Copy: `src/lib.rs` ← `minigraf-ffi/src/lib.rs`
- Copy: `src/uniffi_bindgen.rs` ← `minigraf-ffi/src/uniffi_bindgen.rs`
- Copy: `android/` ← `minigraf-ffi/android/` (entire directory)
- Replace: `android/build.gradle.kts` (switch from GitHub Packages to Maven Central via NMCP; fix groupId)
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `README.md`

**`project-minigraf/minigraf-swift`**
- Create: `Cargo.toml`
- Copy: `src/lib.rs` ← `minigraf-ffi/src/lib.rs`
- Copy: `src/uniffi_bindgen.rs` ← `minigraf-ffi/src/uniffi_bindgen.rs`
- Create: `Sources/MinigrafKit/.gitkeep` (populated by release CI)
- Create: `Package.swift` (URL updated from monorepo → minigraf-swift, path from `minigraf-swift/Sources/…` → `Sources/…`)
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `README.md`

### Monorepo changes

- Modify: `.github/workflows/cascade.yml`
- Delete: `.github/workflows/java-ci.yml`
- Delete: `.github/workflows/java-release.yml`
- Delete: `.github/workflows/mobile.yml`
- Delete: `.github/workflows/c-ci.yml`
- Delete: `.github/workflows/c-release.yml`
- Delete: `.github/workflows/publish-ffi.yml`
- Delete: `minigraf-ffi/` (entire directory)
- Delete: `minigraf-swift/` (entire directory)
- Delete: `minigraf-c/` (entire directory)
- Delete: `Package.swift`
- Modify: `Cargo.toml` (remove `minigraf-ffi` and `minigraf-c` from workspace members)

---

## Task 1: Create minigraf-binding-template

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/uniffi_bindgen.rs`, `.github/workflows/ci.yml`, `README.md`

- [ ] **Step 1: Create the GitHub repo**

```bash
gh repo create project-minigraf/minigraf-binding-template \
  --public \
  --description "Template repo for building a new Minigraf language binding" \
  --clone
cd minigraf-binding-template
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "minigraf-binding-example"
version = "0.0.0"
edition = "2024"
publish = false
description = "Template for building a new Minigraf language binding"

[lib]
name = "minigraf_ffi"
crate-type = ["cdylib"]

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
# Pin to the exact released version. The release workflow updates this line.
minigraf = "1.1.1"
uniffi = { version = "0.31.1", features = ["cli"] }
thiserror = "2.0.18"
serde_json = "1.0.149"
anyhow = "1"

[workspace]
members = ["."]
```

- [ ] **Step 3: Write src/uniffi_bindgen.rs**

```bash
mkdir -p src
```

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

- [ ] **Step 4: Write src/lib.rs**

This is the UniFFI scaffolding with comments at each extension point explaining what to customise for a new binding.

```rust
//! Minigraf binding template — UniFFI scaffolding.
//!
//! CUSTOMISATION GUIDE:
//! 1. Rename the crate in Cargo.toml (name, description).
//! 2. Pin minigraf to the version you want to bind.
//! 3. Implement any extra error variants or wrapper types your language needs.
//! 4. Add your language tooling (e.g. java/, android/, Sources/, etc.).
//! 5. Wire up a release.yml that receives `core-release` dispatch from
//!    the minigraf cascade and calls `cargo publish` / your language publisher.
//!
//! Why depend on `minigraf` directly and not `minigraf-ffi`?
//! UniFFI's `setup_scaffolding!()` macro generates extern "C" symbols in the
//! *final* cdylib crate. Re-exporting from a crate that already called it
//! causes duplicate symbol errors at link time. Always embed the scaffolding
//! in your own crate and depend on `minigraf` core.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
    )
)]

use minigraf::{QueryResult, Value};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

// ─── Error type ──────────────────────────────────────────────────────────────
// CUSTOMISE: add variants for any language-specific error categories.

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MiniGrafError {
    #[error("storage error: {msg}")]
    Storage { msg: String },
    #[error("query error: {msg}")]
    Query { msg: String },
    #[error("parse error: {msg}")]
    Parse { msg: String },
    #[error("unknown error: {msg}")]
    Other { msg: String },
}

impl From<anyhow::Error> for MiniGrafError {
    fn from(e: anyhow::Error) -> Self {
        let full = format!("{e:#}").to_lowercase();
        let msg = e.to_string();
        if full.contains("parse")
            || full.contains("unexpected")
            || full.contains("expected token")
            || full.contains("unknown command")
        {
            MiniGrafError::Parse { msg }
        } else if full.contains("storage") || full.contains(" page") || full.contains("wal ") {
            MiniGrafError::Storage { msg }
        } else if full.contains("query") || full.contains(":find") || full.contains(":where") {
            MiniGrafError::Query { msg }
        } else {
            MiniGrafError::Other { msg }
        }
    }
}

// ─── Database object ─────────────────────────────────────────────────────────
// CUSTOMISE: add methods your language needs (e.g. async wrappers, transactions).

#[derive(uniffi::Object)]
pub struct MiniGrafDb {
    inner: Arc<Mutex<minigraf::Minigraf>>,
}

#[uniffi::export]
impl MiniGrafDb {
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, MiniGrafError> {
        let db = minigraf::Minigraf::open(&path).map_err(MiniGrafError::from)?;
        Ok(Arc::new(Self {
            inner: Arc::new(Mutex::new(db)),
        }))
    }

    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Arc<Self>, MiniGrafError> {
        let db = minigraf::Minigraf::in_memory().map_err(MiniGrafError::from)?;
        Ok(Arc::new(Self {
            inner: Arc::new(Mutex::new(db)),
        }))
    }

    pub fn execute(&self, datalog: String) -> Result<String, MiniGrafError> {
        let result = self
            .inner
            .lock()
            .map_err(|_| MiniGrafError::Other {
                msg: "mutex poisoned".into(),
            })?
            .execute(&datalog)
            .map_err(MiniGrafError::from)?;
        Ok(query_result_to_json(result))
    }

    pub fn checkpoint(&self) -> Result<(), MiniGrafError> {
        self.inner
            .lock()
            .map_err(|_| MiniGrafError::Other {
                msg: "mutex poisoned".into(),
            })?
            .checkpoint()
            .map_err(MiniGrafError::from)
    }
}

// ─── JSON serialisation ───────────────────────────────────────────────────────

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as JVal;
    match v {
        Value::String(s) => JVal::String(s.clone()),
        Value::Integer(i) => JVal::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(JVal::Number)
            .unwrap_or(JVal::Null),
        Value::Boolean(b) => JVal::Bool(*b),
        Value::Ref(uuid) => JVal::String(uuid.to_string()),
        Value::Keyword(k) => JVal::String(k.clone()),
        Value::Null => JVal::Null,
    }
}

fn query_result_to_json(result: QueryResult) -> String {
    use serde_json::json;
    let val = match result {
        QueryResult::Transacted(tx_id) => json!({"transacted": tx_id}),
        QueryResult::Retracted(tx_id) => json!({"retracted": tx_id}),
        QueryResult::Ok => json!({"ok": true}),
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<serde_json::Value>> = results
                .iter()
                .map(|row| row.iter().map(value_to_json).collect())
                .collect();
            json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_succeeds() {
        MiniGrafDb::open_in_memory().expect("open_in_memory");
    }

    #[test]
    fn execute_transact_returns_json() {
        let db = MiniGrafDb::open_in_memory().expect("open");
        let json = db
            .execute(r#"(transact [[:alice :name "Alice"]])"#.into())
            .expect("execute");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(v.get("transacted").is_some());
    }

    #[test]
    fn execute_query_returns_results() {
        let db = MiniGrafDb::open_in_memory().expect("open");
        db.execute(r#"(transact [[:alice :name "Alice"]])"#.into())
            .expect("transact");
        let json = db
            .execute(r#"(query [:find ?n :where [?e :name ?n]])"#.into())
            .expect("query");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["results"][0][0], "Alice");
    }
}
```

- [ ] **Step 5: Write .github/workflows/ci.yml**

```bash
mkdir -p .github/workflows
```

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test
```

- [ ] **Step 6: Write README.md**

```markdown
# minigraf-binding-template

Template repository for building a new [Minigraf](https://github.com/project-minigraf/minigraf) language binding.

## What's in this repo

| File | Purpose |
|---|---|
| `Cargo.toml` | Rust shim crate — depends on `minigraf` core, produces a cdylib |
| `src/lib.rs` | UniFFI scaffolding with comments at every extension point |
| `src/uniffi_bindgen.rs` | Entry point for the `uniffi-bindgen` CLI binary |
| `.github/workflows/ci.yml` | Starter CI — runs `cargo test` |

## Creating a new binding

1. Click **Use this template** on GitHub to create your repo under `project-minigraf/<language>`.
2. Update `Cargo.toml`: rename the crate, pin `minigraf` to the version you're targeting.
3. Run `cargo test` to verify the Rust layer compiles and tests pass.
4. Add your language tooling (e.g. `java/`, `android/`, `Sources/`, etc.).
5. Generate language bindings with `uniffi-bindgen`:
   ```bash
   cargo build --release
   cargo run --bin uniffi-bindgen -- generate \
     --library target/release/libminigraf_ffi.<so|dylib|dll> \
     --language <kotlin|swift|python|...> \
     --out-dir <output-dir>/
   ```
6. Add a `release.yml` that receives the `core-release` repository_dispatch event from
   the minigraf cascade, pins the new `minigraf` version in `Cargo.toml`, commits, tags,
   and publishes your artifact.

## Why not depend on `minigraf-ffi`?

UniFFI's `setup_scaffolding!()` macro generates `extern "C"` symbols in the **final** cdylib
crate. Re-exporting from a crate that already called it causes duplicate symbol errors at
link time. Always embed the scaffolding in your own crate and depend on `minigraf` core
directly. See the existing binding repos (`minigraf-python`, `minigraf-java`, etc.) for
complete examples.

## License

MIT OR Apache-2.0
```

- [ ] **Step 7: Verify tests pass**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 8: Enable template + commit + push**

```bash
git add .
git commit -m "feat: initial minigraf binding template"
git push origin main
```

Then in the GitHub UI (or via `gh`): Settings → check "Template repository".

```bash
gh api repos/project-minigraf/minigraf-binding-template \
  --method PATCH \
  -f is_template=true
```

---

## Task 2: Create minigraf-c repo

**Files:**
- Create: `Cargo.toml`, `cbindgen.toml`, `src/lib.rs`, `include/minigraf.h`
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `README.md`

The Rust source (`src/lib.rs`, `include/minigraf.h`) is copied verbatim from the monorepo. Only `Cargo.toml`, `cbindgen.toml`, and the workflows change.

- [ ] **Step 1: Create the GitHub repo and clone it**

```bash
gh repo create project-minigraf/minigraf-c \
  --public \
  --description "C bindings for Minigraf — stable C API with cbindgen-generated header" \
  --clone
cd minigraf-c
mkdir -p src include .github/workflows
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "minigraf-c-shim"
version = "0.0.0"
edition = "2024"
publish = false
description = "C bindings for Minigraf — stable C API with cbindgen-generated header"

[lib]
# "minigraf" produces libminigraf.so / libminigraf.dylib / minigraf.dll directly
# without the rename step needed in the monorepo (which used name "minigraf-c").
name = "minigraf"
crate-type = ["cdylib", "staticlib"]

[dependencies]
# Pin to the exact released version. The release workflow updates this line.
minigraf = "1.1.1"
serde_json = "1.0"

[workspace]
members = ["."]
```

- [ ] **Step 3: Copy src/lib.rs from monorepo**

Copy `/home/aditya/workspaces/rustrover/minigraf/minigraf-c/src/lib.rs` verbatim to `src/lib.rs`.

- [ ] **Step 4: Copy include/minigraf.h from monorepo**

Copy `/home/aditya/workspaces/rustrover/minigraf/minigraf-c/include/minigraf.h` verbatim to `include/minigraf.h`.

- [ ] **Step 5: Write cbindgen.toml**

Updated from the monorepo version: crate name changes from `minigraf-c` to `minigraf-c-shim`, paths are now relative to the repo root.

```toml
language = "C"
include_guard = "MINIGRAF_H"
autogen_warning = "/* DO NOT EDIT — generated by cbindgen. Run `cbindgen --config cbindgen.toml --crate minigraf-c-shim --output include/minigraf.h` to regenerate. */"
documentation = true
documentation_style = "c99"

[export]
include = [
    "minigraf_open",
    "minigraf_open_in_memory",
    "minigraf_close",
    "minigraf_execute",
    "minigraf_string_free",
    "minigraf_checkpoint",
    "minigraf_last_error",
    "MiniGrafDb",
]
```

- [ ] **Step 6: Verify tests pass**

```bash
cargo test
```

Expected: 7 tests pass (open_in_memory_returns_non_null, execute_transact_returns_json, execute_query_returns_results, execute_invalid_datalog_returns_null_and_sets_error, checkpoint_returns_zero_on_success, string_free_null_is_safe, close_null_is_safe).

- [ ] **Step 7: Write .github/workflows/ci.yml**

Adapted from monorepo's `c-ci.yml`. Key changes: `-p minigraf-c` → removed (standalone workspace); `minigraf-c/cbindgen.toml` → `cbindgen.toml`; `--crate minigraf-c` → `--crate minigraf-c-shim`; `minigraf-c/include/minigraf.h` → `include/minigraf.h`.

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    name: C tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -- --nocapture

  check-header-drift:
    name: Verify minigraf.h is up to date
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install cbindgen
        run: cargo install cbindgen --version 0.29.2 --locked
      - name: Regenerate header
        run: |
          cbindgen --config cbindgen.toml \
                   --crate minigraf-c-shim \
                   --output /tmp/minigraf_generated.h
      - name: Fail if header has drifted
        run: |
          diff include/minigraf.h /tmp/minigraf_generated.h || \
          (echo "minigraf.h is out of date — run cbindgen locally and commit the result" && exit 1)
```

- [ ] **Step 8: Write .github/workflows/release.yml**

Adapted from monorepo's `c-release.yml`. Key changes: triggered by `repository_dispatch` instead of tag push; adds `prepare` job (updates Cargo.toml pin, commits, tags, creates GitHub Release); `build` job checks out the tag and builds; `-p minigraf-c` removed; `minigraf-c/include/minigraf.h` → `include/minigraf.h`; no more waiting for cargo-dist to create a release.

```yaml
name: C Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g. v1.2.0)'
        required: true

permissions:
  contents: write

jobs:
  prepare:
    name: Pin version, commit, tag, create release
    runs-on: ubuntu-latest
    outputs:
      tag: ${{ steps.ver.outputs.tag }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}

      - id: ver
        run: |
          V="${{ github.event.client_payload.version || github.event.inputs.version }}"
          echo "tag=$V" >> "$GITHUB_OUTPUT"
          echo "semver=${V#v}" >> "$GITHUB_OUTPUT"

      - name: Pin minigraf version in Cargo.toml
        run: |
          sed -i 's/^minigraf = "[^"]*"/minigraf = "${{ steps.ver.outputs.semver }}"/' Cargo.toml

      - name: Commit and push tag
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml
          git commit -m "chore: release ${{ steps.ver.outputs.tag }}"
          git tag "${{ steps.ver.outputs.tag }}"
          git push origin main --follow-tags

      - name: Create GitHub Release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh release create "${{ steps.ver.outputs.tag }}" \
            --repo "$GITHUB_REPOSITORY" \
            --title "minigraf-c ${{ steps.ver.outputs.tag }}" \
            --notes "C bindings for minigraf ${{ steps.ver.outputs.tag }}. Download the library for your platform below."

  build:
    name: Build C library (${{ matrix.os }})
    needs: prepare
    runs-on: ${{ matrix.os }}
    permissions:
      contents: write
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact-name: linux-x86_64
            lib-name: libminigraf.so
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            artifact-name: linux-aarch64
            lib-name: libminigraf.so
          - os: macos-14
            target: universal2
            artifact-name: macos-universal2
            lib-name: libminigraf.dylib
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact-name: windows-x86_64
            lib-name: minigraf.dll

    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.prepare.outputs.tag }}

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target == 'universal2' && 'aarch64-apple-darwin x86_64-apple-darwin' || matrix.target }}

      - name: Build (universal2)
        if: matrix.target == 'universal2'
        run: |
          cargo build --release --target aarch64-apple-darwin
          cargo build --release --target x86_64-apple-darwin
          lipo -create \
            target/aarch64-apple-darwin/release/libminigraf.dylib \
            target/x86_64-apple-darwin/release/libminigraf.dylib \
            -output ${{ matrix.lib-name }}

      - name: Build (Linux)
        if: runner.os == 'Linux'
        run: |
          cargo build --release --target ${{ matrix.target }}
          cp target/${{ matrix.target }}/release/libminigraf.so ${{ matrix.lib-name }}

      - name: Build (Windows)
        if: runner.os == 'Windows'
        run: |
          cargo build --release --target ${{ matrix.target }}
          copy target\${{ matrix.target }}\release\minigraf.dll ${{ matrix.lib-name }}
        shell: pwsh

      - name: Package (unix)
        if: runner.os != 'Windows'
        run: |
          tar czf minigraf-c-${{ needs.prepare.outputs.tag }}-${{ matrix.artifact-name }}.tar.gz \
            ${{ matrix.lib-name }} include/minigraf.h

      - name: Package (windows)
        if: runner.os == 'Windows'
        run: |
          Compress-Archive -Path ${{ matrix.lib-name }},include\minigraf.h `
            -DestinationPath minigraf-c-${{ needs.prepare.outputs.tag }}-${{ matrix.artifact-name }}.zip
        shell: pwsh

      - name: Upload to GitHub Release (unix)
        if: runner.os != 'Windows'
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh release upload "${{ needs.prepare.outputs.tag }}" \
            minigraf-c-${{ needs.prepare.outputs.tag }}-${{ matrix.artifact-name }}.tar.gz \
            --repo "$GITHUB_REPOSITORY" --clobber

      - name: Upload to GitHub Release (windows)
        if: runner.os == 'Windows'
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh release upload "${{ needs.prepare.outputs.tag }}" `
            "minigraf-c-${{ needs.prepare.outputs.tag }}-${{ matrix.artifact-name }}.zip" `
            --repo "$GITHUB_REPOSITORY" --clobber
        shell: pwsh
```

- [ ] **Step 9: Write README.md**

```markdown
# minigraf-c

C bindings for [Minigraf](https://github.com/project-minigraf/minigraf) — zero-config,
single-file, embedded bi-temporal graph database.

## Installation

Download the pre-built library for your platform from the
[latest release](https://github.com/project-minigraf/minigraf-c/releases/latest):

| Platform | Archive |
|---|---|
| Linux x86_64 | `minigraf-c-<version>-linux-x86_64.tar.gz` |
| Linux aarch64 | `minigraf-c-<version>-linux-aarch64.tar.gz` |
| macOS universal | `minigraf-c-<version>-macos-universal2.tar.gz` |
| Windows x86_64 | `minigraf-c-<version>-windows-x86_64.zip` |

Each archive contains `libminigraf.{so|dylib|dll}` and `minigraf.h`.

## Quick start

```c
#include "minigraf.h"
#include <stdio.h>

int main(void) {
    MiniGrafDb *db = minigraf_open_in_memory();
    char *result = minigraf_execute(db, "(transact [[:alice :name \"Alice\"]])");
    printf("%s\n", result);
    minigraf_string_free(result);
    minigraf_close(db);
    return 0;
}
```

Compile: `cc -o example example.c -L. -lminigraf -Wl,-rpath,.`

## Memory contract

- Strings returned by `minigraf_execute` must be freed with `minigraf_string_free`.
- Databases must be closed with `minigraf_close`.
- Passing `NULL` to any function is safe (no-op or returns NULL/error).

## API

| Function | Description |
|---|---|
| `minigraf_open(path)` | Open a file-backed database |
| `minigraf_open_in_memory()` | Open an in-memory database |
| `minigraf_execute(db, datalog)` | Execute Datalog, returns JSON string |
| `minigraf_string_free(s)` | Free a string returned by `execute` |
| `minigraf_checkpoint(db)` | Flush WAL to disk; returns 0 on success |
| `minigraf_last_error(db)` | Return last error message (valid until next call) |
| `minigraf_close(db)` | Close the database and free all memory |

## Building from source

```bash
cargo build --release
# produces target/release/libminigraf.{so|dylib|dll}
```

Regenerate the header after changing the public API:
```bash
cbindgen --config cbindgen.toml --crate minigraf-c-shim --output include/minigraf.h
```

## License

MIT OR Apache-2.0
```

- [ ] **Step 10: Commit and push**

```bash
git add .
git commit -m "feat: initial minigraf-c repo — C bindings split from monorepo"
git push origin main
```

---

## Task 3: Create minigraf-java repo

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/uniffi_bindgen.rs`
- Copy: `java/` directory from monorepo's `minigraf-ffi/java/`
- Modify: `java/build.gradle.kts` (fix `repoRoot`)
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `README.md`

- [ ] **Step 1: Create the GitHub repo and clone it**

```bash
gh repo create project-minigraf/minigraf-java \
  --public \
  --description "JVM binding for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-java
mkdir -p src .github/workflows
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "minigraf-java-shim"
version = "0.0.0"
edition = "2024"
publish = false
description = "JVM binding for Minigraf — bi-temporal graph database"

[lib]
name = "minigraf_ffi"
crate-type = ["cdylib"]

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
# Pin to the exact released version. The release workflow updates this line.
minigraf = "1.1.1"
uniffi = { version = "0.31.1", features = ["cli"] }
thiserror = "2.0.18"
serde_json = "1.0.149"
anyhow = "1"

[workspace]
members = ["."]
```

- [ ] **Step 3: Copy src/lib.rs and src/uniffi_bindgen.rs from monorepo**

```bash
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/lib.rs src/lib.rs
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/uniffi_bindgen.rs src/uniffi_bindgen.rs
```


- [ ] **Step 4: Copy java/ directory from monorepo**

```bash
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/java ./java
```

- [ ] **Step 5: Fix repoRoot in java/build.gradle.kts**

In the monorepo, `java/build.gradle.kts` computed the repo root as two levels up from the Gradle project dir (`minigraf-ffi/java/ → minigraf-ffi/ → monorepo root`). In the split repo, `java/` is one level below the repo root.

Find this line in `java/build.gradle.kts`:

```kotlin
val repoRoot = rootProject.projectDir.parentFile.parentFile.absolutePath
```

Replace with:

```kotlin
val repoRoot = rootProject.projectDir.parentFile.absolutePath
```

- [ ] **Step 6: Verify Rust tests pass**

```bash
cargo test
```

Expected: all tests pass (same test suite as minigraf-ffi/src/lib.rs).

- [ ] **Step 7: Write .github/workflows/ci.yml**

Adapted from monorepo's `java-ci.yml`. Key changes: `-p minigraf-ffi` → removed; `working-directory: minigraf-ffi/java` → `working-directory: java`.

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  rust-test:
    name: Rust tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test

  java-test:
    name: Java tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'
      - name: Build Rust library (release)
        run: cargo build --release
      - name: Generate Kotlin bindings + copy local native
        working-directory: java
        run: ./gradlew generateKotlinBindings copyLocalNative
      - name: Run tests
        working-directory: java
        run: ./gradlew test
```

- [ ] **Step 8: Write .github/workflows/release.yml**

Adapted from monorepo's `java-release.yml`. Key changes: triggered by `repository_dispatch`; adds `prepare` job (update Cargo.toml, commit, tag); `-p minigraf-ffi` → removed; all `minigraf-ffi/java/` paths → `java/`; `RESOURCES=minigraf-ffi/java/src/main/resources/natives` → `RESOURCES=java/src/main/resources/natives`; `working-directory: minigraf-ffi/java` → `working-directory: java`.

```yaml
name: Java Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g. v1.2.0)'
        required: true

jobs:
  prepare:
    name: Pin version, commit, tag
    runs-on: ubuntu-latest
    outputs:
      tag: ${{ steps.ver.outputs.tag }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}

      - id: ver
        run: |
          V="${{ github.event.client_payload.version || github.event.inputs.version }}"
          echo "tag=$V" >> "$GITHUB_OUTPUT"
          echo "semver=${V#v}" >> "$GITHUB_OUTPUT"

      - name: Pin minigraf version in Cargo.toml
        run: |
          sed -i 's/^minigraf = "[^"]*"/minigraf = "${{ steps.ver.outputs.semver }}"/' Cargo.toml

      - name: Commit and push tag
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml
          git commit -m "chore: release ${{ steps.ver.outputs.tag }}"
          git tag "${{ steps.ver.outputs.tag }}"
          git push origin main --follow-tags

  build-natives:
    name: Build native (${{ matrix.target }})
    needs: prepare
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            native-dir: linux/x86_64
            lib-name: libminigraf_ffi.so
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            native-dir: linux/aarch64
            lib-name: libminigraf_ffi.so
          - os: macos-14
            target: universal2
            native-dir: macos/universal
            lib-name: libminigraf_ffi.dylib
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            native-dir: windows/x86_64
            lib-name: minigraf_ffi.dll
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.prepare.outputs.tag }}

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target == 'universal2' && 'aarch64-apple-darwin x86_64-apple-darwin' || matrix.target }}

      - name: Build (universal2)
        if: matrix.target == 'universal2'
        run: |
          cargo build --release --target aarch64-apple-darwin
          cargo build --release --target x86_64-apple-darwin
          lipo -create \
            target/aarch64-apple-darwin/release/libminigraf_ffi.dylib \
            target/x86_64-apple-darwin/release/libminigraf_ffi.dylib \
            -output libminigraf_ffi.dylib

      - name: Build (other)
        if: matrix.target != 'universal2'
        run: cargo build --release --target ${{ matrix.target }}

      - name: Upload native
        uses: actions/upload-artifact@v4
        with:
          name: native-${{ matrix.native-dir == 'linux/x86_64' && 'linux-x64' || matrix.native-dir == 'linux/aarch64' && 'linux-arm64' || matrix.native-dir == 'macos/universal' && 'macos' || 'windows' }}
          path: |
            ${{ matrix.target == 'universal2' && 'libminigraf_ffi.dylib' || format('target/{0}/release/{1}', matrix.target, matrix.lib-name) }}

  assemble-and-publish:
    name: Assemble fat JAR and publish to Maven Central
    needs: [prepare, build-natives]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.prepare.outputs.tag }}

      - uses: dtolnay/rust-toolchain@stable

      - uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Download all natives
        uses: actions/download-artifact@v4
        with:
          pattern: native-*
          path: natives-staging
          merge-multiple: false

      - name: Copy natives into resources
        run: |
          RESOURCES=java/src/main/resources/natives
          mkdir -p "$RESOURCES/linux/x86_64" "$RESOURCES/linux/aarch64" \
                   "$RESOURCES/macos/universal" "$RESOURCES/windows/x86_64"
          cp natives-staging/native-linux-x64/libminigraf_ffi.so  "$RESOURCES/linux/x86_64/"
          cp natives-staging/native-linux-arm64/libminigraf_ffi.so "$RESOURCES/linux/aarch64/"
          cp natives-staging/native-macos/libminigraf_ffi.dylib    "$RESOURCES/macos/universal/"
          cp natives-staging/native-windows/minigraf_ffi.dll       "$RESOURCES/windows/x86_64/"

      - name: Build Rust library for bindgen
        run: cargo build --release

      - name: Generate Kotlin bindings
        working-directory: java
        run: ./gradlew generateKotlinBindings

      - name: Publish to Maven Central
        working-directory: java
        env:
          CENTRAL_TOKEN_USERNAME: ${{ secrets.CENTRAL_TOKEN_USERNAME }}
          CENTRAL_TOKEN_PASSWORD: ${{ secrets.CENTRAL_TOKEN_PASSWORD }}
          GPG_SIGNING_KEY: ${{ secrets.GPG_SIGNING_KEY }}
          GPG_SIGNING_PASSWORD: ${{ secrets.GPG_SIGNING_PASSWORD }}
          RELEASE_VERSION: ${{ needs.prepare.outputs.semver }}
        run: ./gradlew publishAllPublicationsToCentralPortal
```

- [ ] **Step 9: Write README.md**

```markdown
# minigraf-java

JVM binding for [Minigraf](https://github.com/project-minigraf/minigraf) — zero-config,
single-file, embedded bi-temporal graph database with Datalog queries.

## Installation

### Gradle (Kotlin DSL)

```kotlin
dependencies {
    implementation("io.github.project-minigraf:minigraf-jvm:1.1.1")
}
```

### Maven

```xml
<dependency>
    <groupId>io.github.project-minigraf</groupId>
    <artifactId>minigraf-jvm</artifactId>
    <version>1.1.1</version>
</dependency>
```

## Quick start

```kotlin
import io.github.project_minigraf.minigraf.MiniGrafDb

val db = MiniGrafDb.openInMemory()
val result = db.execute("""(transact [[:alice :name "Alice"]])""")
println(result)  // {"transacted":1}
```

## Building from source

Requires Rust stable toolchain and JDK 17.

```bash
cargo build --release
cd java
./gradlew generateKotlinBindings copyLocalNative test
```

## License

MIT OR Apache-2.0
```

- [ ] **Step 10: Commit and push**

```bash
git add .
git commit -m "feat: initial minigraf-java repo — JVM binding split from monorepo"
git push origin main
```

---

## Task 4: Create minigraf-android repo

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/uniffi_bindgen.rs`
- Copy: `android/` directory from monorepo's `minigraf-ffi/android/`
- Replace: `android/build.gradle.kts` (switch to Maven Central via NMCP)
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `README.md`

- [ ] **Step 1: Create the GitHub repo and clone it**

```bash
gh repo create project-minigraf/minigraf-android \
  --public \
  --description "Android binding for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-android
mkdir -p src .github/workflows
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "minigraf-android-shim"
version = "0.0.0"
edition = "2024"
publish = false
description = "Android binding for Minigraf — bi-temporal graph database"

[lib]
name = "minigraf_ffi"
crate-type = ["cdylib"]

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
# Pin to the exact released version. The release workflow updates this line.
minigraf = "1.1.1"
uniffi = { version = "0.31.1", features = ["cli"] }
thiserror = "2.0.18"
serde_json = "1.0.149"
anyhow = "1"

[workspace]
members = ["."]
```

- [ ] **Step 3: Copy src/lib.rs and src/uniffi_bindgen.rs from monorepo**

```bash
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/lib.rs src/lib.rs
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/uniffi_bindgen.rs src/uniffi_bindgen.rs
```

- [ ] **Step 4: Copy android/ directory from monorepo**

```bash
cp -r /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/android ./android
```

- [ ] **Step 5: Replace android/build.gradle.kts**

The existing file publishes to GitHub Packages with groupId `io.github.adityamukho`. Replace it entirely with NMCP-based Maven Central publishing under `io.github.project-minigraf`.

```kotlin
plugins {
    id("com.android.library") version "8.2.2"
    id("maven-publish")
    signing
    id("com.gradleup.nmcp") version "0.0.8"
}

android {
    namespace = "io.github.project_minigraf.minigraf"
    compileSdk = 34
    defaultConfig {
        minSdk = 24
        targetSdk = 34
    }
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("jniLibs")
            java.srcDirs("src/main/java")
        }
    }
    publishing {
        singleVariant("release")
    }
}

afterEvaluate {
    publishing {
        publications {
            create<MavenPublication>("release") {
                from(components["release"])
                groupId = "io.github.project-minigraf"
                artifactId = "minigraf-android"
                version = System.getenv("RELEASE_VERSION") ?: "0.0.0-local"

                pom {
                    name.set("Minigraf Android")
                    description.set("Zero-config, single-file, embedded graph database with bi-temporal Datalog queries — Android bindings")
                    url.set("https://github.com/project-minigraf/minigraf-android")
                    licenses {
                        license {
                            name.set("MIT OR Apache-2.0")
                            url.set("https://github.com/project-minigraf/minigraf-android/blob/main/LICENSE-MIT")
                        }
                    }
                    developers {
                        developer {
                            id.set("adityamukho")
                            name.set("Aditya Mukhopadhyay")
                        }
                    }
                    scm {
                        connection.set("scm:git:git://github.com/project-minigraf/minigraf-android.git")
                        developerConnection.set("scm:git:ssh://github.com/project-minigraf/minigraf-android.git")
                        url.set("https://github.com/project-minigraf/minigraf-android")
                    }
                }
            }
        }
    }

    signing {
        val signingKey = System.getenv("GPG_SIGNING_KEY")
        val signingPassword = System.getenv("GPG_SIGNING_PASSWORD")
        if (signingKey != null && signingPassword != null) {
            useInMemoryPgpKeys(signingKey, signingPassword)
            sign(publishing.publications["release"])
        }
    }
}

nmcp {
    publish("release") {
        username = System.getenv("CENTRAL_TOKEN_USERNAME") ?: ""
        password = System.getenv("CENTRAL_TOKEN_PASSWORD") ?: ""
        publicationType = "AUTOMATIC"
    }
}
```

- [ ] **Step 6: Verify Rust tests pass**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 7: Write .github/workflows/ci.yml**

Adapted from the Android half of monorepo's `mobile.yml`. Runs cross-compile + assemble as a smoke test on PRs.

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  rust-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test

  android-build:
    name: Android cross-compile smoke test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android,armv7-linux-androideabi,x86_64-linux-android

      - name: Install cargo-ndk
        run: cargo install cargo-ndk --locked

      - name: Set ANDROID_NDK_HOME
        run: echo "ANDROID_NDK_HOME=$ANDROID_NDK_LATEST_HOME" >> $GITHUB_ENV

      - uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Cross-compile Android targets
        run: |
          cargo ndk \
            -t arm64-v8a \
            -t armeabi-v7a \
            -t x86_64 \
            -o android/jniLibs \
            build --release

      - name: Generate Kotlin bindings
        run: |
          cargo run --bin uniffi-bindgen -- generate \
            --library target/aarch64-linux-android/release/libminigraf_ffi.so \
            --language kotlin \
            --out-dir android/src/main/java/

      - name: Assemble AAR
        run: cd android && ./gradlew assembleRelease
```

- [ ] **Step 8: Write .github/workflows/release.yml**

Adapted from the Android half of monorepo's `mobile.yml`. Key changes: triggered by `repository_dispatch`; adds `prepare` job; `minigraf-ffi/android/` → `android/`; publishes to Maven Central instead of GitHub Packages.

```yaml
name: Android Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g. v1.2.0)'
        required: true

jobs:
  prepare:
    name: Pin version, commit, tag
    runs-on: ubuntu-latest
    outputs:
      tag: ${{ steps.ver.outputs.tag }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}

      - id: ver
        run: |
          V="${{ github.event.client_payload.version || github.event.inputs.version }}"
          echo "tag=$V" >> "$GITHUB_OUTPUT"
          echo "semver=${V#v}" >> "$GITHUB_OUTPUT"

      - name: Pin minigraf version in Cargo.toml
        run: |
          sed -i 's/^minigraf = "[^"]*"/minigraf = "${{ steps.ver.outputs.semver }}"/' Cargo.toml

      - name: Commit and push tag
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml
          git commit -m "chore: release ${{ steps.ver.outputs.tag }}"
          git tag "${{ steps.ver.outputs.tag }}"
          git push origin main --follow-tags

  build-and-publish:
    name: Build AAR and publish to Maven Central
    needs: prepare
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.prepare.outputs.tag }}

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android,armv7-linux-androideabi,x86_64-linux-android

      - name: Install cargo-ndk
        run: cargo install cargo-ndk --locked

      - name: Set ANDROID_NDK_HOME
        run: echo "ANDROID_NDK_HOME=$ANDROID_NDK_LATEST_HOME" >> $GITHUB_ENV

      - uses: actions/setup-java@v4
        with:
          java-version: '17'
          distribution: 'temurin'

      - name: Cross-compile Android targets
        run: |
          cargo ndk \
            -t arm64-v8a \
            -t armeabi-v7a \
            -t x86_64 \
            -o android/jniLibs \
            build --release

      - name: Generate Kotlin bindings
        run: |
          cargo run --bin uniffi-bindgen -- generate \
            --library target/aarch64-linux-android/release/libminigraf_ffi.so \
            --language kotlin \
            --out-dir android/src/main/java/

      - name: Assemble AAR
        run: cd android && ./gradlew assembleRelease

      - name: Publish to Maven Central
        env:
          CENTRAL_TOKEN_USERNAME: ${{ secrets.CENTRAL_TOKEN_USERNAME }}
          CENTRAL_TOKEN_PASSWORD: ${{ secrets.CENTRAL_TOKEN_PASSWORD }}
          GPG_SIGNING_KEY: ${{ secrets.GPG_SIGNING_KEY }}
          GPG_SIGNING_PASSWORD: ${{ secrets.GPG_SIGNING_PASSWORD }}
          RELEASE_VERSION: ${{ needs.prepare.outputs.semver }}
        run: cd android && ./gradlew publishAllPublicationsToCentralPortal
```

- [ ] **Step 9: Write README.md**

```markdown
# minigraf-android

Android binding for [Minigraf](https://github.com/project-minigraf/minigraf) — zero-config,
single-file, embedded bi-temporal graph database with Datalog queries.

## Installation

```kotlin
dependencies {
    implementation("io.github.project-minigraf:minigraf-android:1.1.1")
}
```

Minimum SDK: 24 (Android 7.0). Supports arm64-v8a, armeabi-v7a, x86_64.

## Quick start

```kotlin
import io.github.project_minigraf.minigraf.MiniGrafDb

val db = MiniGrafDb.openInMemory()
val result = db.execute("""(transact [[:alice :name "Alice"]])""")
println(result)  // {"transacted":1}
```

## Building from source

Requires Rust stable toolchain, Android NDK, and JDK 17.

```bash
cargo install cargo-ndk --locked
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o android/jniLibs build --release
cargo run --bin uniffi-bindgen -- generate \
  --library target/aarch64-linux-android/release/libminigraf_ffi.so \
  --language kotlin \
  --out-dir android/src/main/java/
cd android && ./gradlew assembleRelease
```

## License

MIT OR Apache-2.0
```

- [ ] **Step 10: Commit and push**

```bash
git add .
git commit -m "feat: initial minigraf-android repo — Android binding split from monorepo"
git push origin main
```

---

## Task 5: Create minigraf-swift repo

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/uniffi_bindgen.rs`
- Copy: `src/lib.rs`, `src/uniffi_bindgen.rs` from monorepo's `minigraf-ffi/src/`
- Create: `Sources/MinigrafKit/.gitkeep`
- Create: `Package.swift` (updated URLs and paths)
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `README.md`

- [ ] **Step 1: Create the GitHub repo and clone it**

```bash
gh repo create project-minigraf/minigraf-swift \
  --public \
  --description "Swift/iOS binding for Minigraf — bi-temporal graph database" \
  --clone
cd minigraf-swift
mkdir -p src Sources/MinigrafKit .github/workflows
```

- [ ] **Step 2: Write Cargo.toml**

iOS requires staticlib (cdylib is not supported on iOS device). Keep both so the `uniffi-bindgen` binary (which runs on host) can also link dynamically.

```toml
[package]
name = "minigraf-swift-shim"
version = "0.0.0"
edition = "2024"
publish = false
description = "Swift/iOS binding for Minigraf — bi-temporal graph database"

[lib]
name = "minigraf_ffi"
crate-type = ["staticlib", "cdylib"]

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
# Pin to the exact released version. The release workflow updates this line.
minigraf = "1.1.1"
uniffi = { version = "0.31.1", features = ["cli"] }
thiserror = "2.0.18"
serde_json = "1.0.149"
anyhow = "1"

[workspace]
members = ["."]
```

- [ ] **Step 3: Copy src/lib.rs and src/uniffi_bindgen.rs from monorepo**

```bash
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/lib.rs src/lib.rs
cp /home/aditya/workspaces/rustrover/minigraf/minigraf-ffi/src/uniffi_bindgen.rs src/uniffi_bindgen.rs
```

- [ ] **Step 4: Create Sources/MinigrafKit/.gitkeep**

The Swift sources are generated and committed by the release CI (to the `swift-releases` branch). The directory must exist on main so the `Package.swift` target path resolves.

```bash
touch Sources/MinigrafKit/.gitkeep
```

- [ ] **Step 5: Write Package.swift**

Updated from monorepo: URL points to `minigraf-swift` repo (not `minigraf`); path is `Sources/MinigrafKit` (not `minigraf-swift/Sources/MinigrafKit`).

```swift
// swift-tools-version: 5.9
import PackageDescription

// This file is automatically updated by CI after each release.
// The URL and checksum below are updated to point to the latest .xcframework.zip.
let package = Package(
    name: "MinigrafKit",
    platforms: [
        .iOS(.v16),
    ],
    products: [
        .library(
            name: "MinigrafKit",
            targets: ["minigrafFFI", "MinigrafKit"]
        ),
    ],
    targets: [
        .binaryTarget(
            name: "minigrafFFI",
            // Updated by CI: release.yml
            url: "https://github.com/project-minigraf/minigraf-swift/releases/download/v1.1.1/MinigrafKit-v1.1.1.xcframework.zip",
            checksum: "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        .target(
            name: "MinigrafKit",
            dependencies: [.target(name: "minigrafFFI")],
            path: "Sources/MinigrafKit"
        ),
    ]
)
```

- [ ] **Step 6: Verify Rust tests pass (on any platform)**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 7: Write .github/workflows/ci.yml**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  rust-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test

  ios-build:
    name: iOS simulator build smoke test
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-ios-sim
      - name: Build for iOS simulator
        run: cargo build --target aarch64-apple-ios-sim --release
```

- [ ] **Step 8: Write .github/workflows/release.yml**

Adapted from the iOS + swift-publish half of monorepo's `mobile.yml`. Key changes: triggered by `repository_dispatch`; adds `prepare` job; all xcframework artifact URLs updated to `minigraf-swift` repo; `Package.swift` path is now repo root (not monorepo root); `minigraf-swift/Sources/MinigrafKit` path → `Sources/MinigrafKit`; creates its own GitHub Release (no waiting for cargo-dist).

```yaml
name: Swift Release

on:
  repository_dispatch:
    types: [core-release]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g. v1.2.0)'
        required: true

permissions:
  contents: write

jobs:
  prepare:
    name: Pin version, commit, tag
    runs-on: ubuntu-latest
    outputs:
      tag: ${{ steps.ver.outputs.tag }}
      semver: ${{ steps.ver.outputs.semver }}
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}

      - id: ver
        run: |
          V="${{ github.event.client_payload.version || github.event.inputs.version }}"
          echo "tag=$V" >> "$GITHUB_OUTPUT"
          echo "semver=${V#v}" >> "$GITHUB_OUTPUT"

      - name: Pin minigraf version in Cargo.toml
        run: |
          sed -i 's/^minigraf = "[^"]*"/minigraf = "${{ steps.ver.outputs.semver }}"/' Cargo.toml

      - name: Commit and push tag
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml
          git commit -m "chore: release ${{ steps.ver.outputs.tag }}"
          git tag "${{ steps.ver.outputs.tag }}"
          git push origin main --follow-tags

  build-and-publish:
    name: Build xcframework and publish
    needs: prepare
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.prepare.outputs.tag }}
          token: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}
          fetch-depth: 0

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-ios,aarch64-apple-ios-sim

      - name: Build iOS device library
        run: cargo build --target aarch64-apple-ios --release

      - name: Build iOS simulator library
        run: cargo build --target aarch64-apple-ios-sim --release

      - name: Generate Swift bindings
        run: |
          mkdir -p Sources/MinigrafKit
          cargo run --bin uniffi-bindgen -- generate \
            --library target/aarch64-apple-ios/release/libminigraf_ffi.a \
            --language swift \
            --out-dir Sources/MinigrafKit/

      - name: Prepare headers for xcframework
        run: |
          mkdir -p includes
          cp Sources/MinigrafKit/*.h includes/ 2>/dev/null || true
          cp Sources/MinigrafKit/*.modulemap includes/ 2>/dev/null || true

      - name: Assemble xcframework
        run: |
          TAG="${{ needs.prepare.outputs.tag }}"
          xcodebuild -create-xcframework \
            -library target/aarch64-apple-ios/release/libminigraf_ffi.a \
            -headers includes/ \
            -library target/aarch64-apple-ios-sim/release/libminigraf_ffi.a \
            -headers includes/ \
            -output "MinigrafKit.xcframework"
          zip -r "MinigrafKit-${TAG}.xcframework.zip" MinigrafKit.xcframework

      - name: Create GitHub Release and upload xcframework
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          TAG="${{ needs.prepare.outputs.tag }}"
          gh release create "$TAG" \
            --repo "$GITHUB_REPOSITORY" \
            --title "MinigrafKit $TAG" \
            --notes "Swift/iOS binding for minigraf $TAG."
          gh release upload "$TAG" "MinigrafKit-${TAG}.xcframework.zip" \
            --repo "$GITHUB_REPOSITORY"

      - name: Compute checksum and update Package.swift
        run: |
          TAG="${{ needs.prepare.outputs.tag }}"
          CHECKSUM=$(shasum -a 256 "MinigrafKit-${TAG}.xcframework.zip" | awk '{print $1}')
          URL="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/MinigrafKit-${TAG}.xcframework.zip"
          sed -i '' \
            -e "s|url: \"https://github.com/.*/releases/download/.*/MinigrafKit-.*\\.xcframework\\.zip\"|url: \"${URL}\"|" \
            -e "s|checksum: \"[^\"]*\"|checksum: \"${CHECKSUM}\"|" \
            Package.swift

      - name: Commit Package.swift and Swift sources to swift-releases branch
        env:
          TAG: ${{ needs.prepare.outputs.tag }}
        run: |
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git config user.name "github-actions[bot]"

          TMPDIR_SWIFT=$(mktemp -d)
          cp Package.swift "$TMPDIR_SWIFT/Package.swift"
          cp -r Sources/MinigrafKit/ "$TMPDIR_SWIFT/MinigrafKit"

          if git fetch origin swift-releases 2>/dev/null; then
            git checkout -fB swift-releases origin/swift-releases
          else
            git checkout --orphan swift-releases
            git rm -rf . --quiet 2>/dev/null || true
          fi

          cp "$TMPDIR_SWIFT/Package.swift" Package.swift
          mkdir -p Sources/MinigrafKit/
          cp -r "$TMPDIR_SWIFT/MinigrafKit/." Sources/MinigrafKit/
          rm -rf "$TMPDIR_SWIFT"

          git add Package.swift Sources/MinigrafKit/
          git commit -m "chore(release): update Package.swift and Swift bindings for ${TAG}" \
            || echo "Nothing to commit"
          git push origin swift-releases --force

          NEW_SHA=$(git rev-parse HEAD)
          SWIFT_TAG="swift-${TAG}"
          gh api "repos/${GITHUB_REPOSITORY}/git/refs" \
            -X POST \
            -f ref="refs/tags/${SWIFT_TAG}" \
            -f sha="$NEW_SHA" 2>/dev/null \
          || gh api "repos/${GITHUB_REPOSITORY}/git/refs/tags/${SWIFT_TAG}" \
            -X PATCH \
            -f sha="$NEW_SHA" \
            -F force=true
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

- [ ] **Step 9: Write README.md**

```markdown
# minigraf-swift

Swift/iOS binding for [Minigraf](https://github.com/project-minigraf/minigraf) — zero-config,
single-file, embedded bi-temporal graph database with Datalog queries.

## Installation

### Swift Package Manager

In Xcode: File → Add Package Dependencies → enter this repo URL.

Or add to `Package.swift`:

```swift
dependencies: [
    .package(url: "https://github.com/project-minigraf/minigraf-swift", from: "1.1.1")
]
```

> SPM resolves via the `swift-v<version>` tag which points to the `swift-releases` branch
> containing the updated `Package.swift` and generated Swift sources.

Requires iOS 16+.

## Quick start

```swift
import MinigrafKit

let db = try MiniGrafDb.openInMemory()
let result = try db.execute(datalog: #"(transact [[:alice :name "Alice"]])"#)
print(result)  // {"transacted":1}
```

## Building from source

Requires Rust stable toolchain with iOS targets and Xcode.

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --target aarch64-apple-ios --release
cargo build --target aarch64-apple-ios-sim --release
cargo run --bin uniffi-bindgen -- generate \
  --library target/aarch64-apple-ios/release/libminigraf_ffi.a \
  --language swift \
  --out-dir Sources/MinigrafKit/
```

## License

MIT OR Apache-2.0
```

- [ ] **Step 10: Commit and push**

```bash
git add .
git commit -m "feat: initial minigraf-swift repo — Swift/iOS binding split from monorepo"
git push origin main
```

---

## Task 6: Update monorepo cascade

**Files:**
- Modify: `.github/workflows/cascade.yml`

- [ ] **Step 1: Read the current cascade.yml**

File: `/home/aditya/workspaces/rustrover/minigraf/.github/workflows/cascade.yml`

- [ ] **Step 2: Replace cascade.yml with updated version**

The `publish-ffi` job is removed entirely. `dispatch-bindings` now polls for minigraf core itself (moved from `publish-ffi`), then dispatches to all 7 repos.

```yaml
name: Release Cascade

on:
  push:
    tags:
      - 'v[0-9]*.[0-9]*.[0-9]*'

jobs:
  dispatch-bindings:
    name: Dispatch to binding repos
    runs-on: ubuntu-latest
    steps:
      - name: Wait for minigraf to be indexed on crates.io
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          echo "Waiting for minigraf@$VERSION on crates.io..."
          FOUND=0
          for i in $(seq 1 18); do
            STATUS=$(curl -s "https://crates.io/api/v1/crates/minigraf/$VERSION" \
              -H "User-Agent: minigraf-cascade/1.0" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('version',{}).get('num',''))" 2>/dev/null || echo "")
            if [ "$STATUS" = "$VERSION" ]; then
              echo "minigraf@$VERSION is live (attempt $i)"
              FOUND=1
              break
            fi
            echo "Attempt $i/18: not yet available, waiting 10s..."
            sleep 10
          done
          if [ "$FOUND" != "1" ]; then
            echo "ERROR: minigraf@$VERSION did not appear on crates.io within 3 minutes"
            exit 1
          fi

      - name: Dispatch to binding repos
        env:
          GH_TOKEN: ${{ secrets.MINIGRAF_RELEASE_TOKEN }}
        run: |
          VERSION="${GITHUB_REF_NAME}"
          for REPO in minigraf-python minigraf-node minigraf-wasm \
                      minigraf-java minigraf-android minigraf-swift minigraf-c; do
            echo "Dispatching core-release to project-minigraf/$REPO @ $VERSION"
            gh api repos/project-minigraf/$REPO/dispatches \
              -f event_type=core-release \
              -f "client_payload[version]=$VERSION"
          done
```

- [ ] **Step 3: Validate the YAML**

```bash
cd /home/aditya/workspaces/rustrover/minigraf
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/cascade.yml'))" && echo "YAML valid"
```

Expected: `YAML valid`

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/cascade.yml
git commit -m "ci(cascade): remove publish-ffi job; dispatch to 7 binding repos"
```

---

## Task 7: Clean up monorepo

**Files:**
- Delete: `minigraf-ffi/`, `minigraf-swift/`, `minigraf-c/`, `Package.swift`
- Delete: `java-ci.yml`, `java-release.yml`, `mobile.yml`, `c-ci.yml`, `c-release.yml`, `publish-ffi.yml`
- Modify: `Cargo.toml`

- [ ] **Step 1: Remove binding source directories**

```bash
cd /home/aditya/workspaces/rustrover/minigraf
git rm -r minigraf-ffi/ minigraf-swift/ minigraf-c/ Package.swift
```

- [ ] **Step 2: Remove workflow files**

```bash
git rm \
  .github/workflows/java-ci.yml \
  .github/workflows/java-release.yml \
  .github/workflows/mobile.yml \
  .github/workflows/c-ci.yml \
  .github/workflows/c-release.yml \
  .github/workflows/publish-ffi.yml
```

- [ ] **Step 3: Update workspace members in Cargo.toml**

Current `[workspace]` section (lines 1-4 of Cargo.toml):

```toml
[workspace]
members = [".", "minigraf-ffi", "minigraf-c"]
exclude = ["fuzz"]
resolver = "2"
```

Replace with:

```toml
[workspace]
members = ["."]
exclude = ["fuzz"]
resolver = "2"
```

- [ ] **Step 4: Verify the monorepo still builds and tests pass**

```bash
cargo test
```

Expected: all existing tests pass (962 tests). No compilation errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git commit -m "chore(cleanup): remove binding directories and workflows — split to separate repos (#231)"
```

- [ ] **Step 6: Push**

```bash
git push origin main
```

---

## Task 8: Retire minigraf-ffi on crates.io

**Note:** `cargo yank` requires being logged in to crates.io with `cargo login`. Ensure the session has a valid token before running these commands.

- [ ] **Step 1: Yank minigraf-ffi 1.1.1**

```bash
cargo yank minigraf-ffi --version 1.1.1 \
  --message "This crate has been retired. Depend on \`minigraf\` directly. To build a new language binding, see https://github.com/project-minigraf/minigraf-binding-template"
```

Expected output: `Updating crates.io index` then `minigraf-ffi@1.1.1 is yanked`

- [ ] **Step 2: Yank minigraf-ffi 1.1.2**

```bash
cargo yank minigraf-ffi --version 1.1.2 \
  --message "This crate has been retired. Depend on \`minigraf\` directly. To build a new language binding, see https://github.com/project-minigraf/minigraf-binding-template"
```

Expected output: `minigraf-ffi@1.1.2 is yanked`

- [ ] **Step 3: Verify on crates.io**

Visit `https://crates.io/crates/minigraf-ffi` and confirm both versions show as yanked. The yank message should appear alongside each version.
