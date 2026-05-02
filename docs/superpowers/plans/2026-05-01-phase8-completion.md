# Phase 8 Completion — v1.0.0 Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close Phase 8, bump the entire workspace to v1.0.0, sync all documentation, write per-platform READMEs, and update the wiki — producing a clean PR that is the sole commit required before tagging `v1.0.0`.

**Architecture:** Docs-only PR on a dedicated worktree. No source code changes. Wiki is a separate git repo in `.wiki/` and is committed and pushed independently after the main PR merges. All verification is via `cargo check`, `cargo test`, and targeted `grep` scans.

**Tech Stack:** Rust workspace (`cargo`), npm (`package.json`), Python (`pyproject.toml`), Gradle (`.kts`), Swift (`Package.swift`), Markdown.

**Spec:** `docs/superpowers/specs/2026-05-01-phase8-completion-design.md`

---

## File Map

**Modified — version manifests:**
- `Cargo.toml` — core crate version
- `minigraf-ffi/Cargo.toml` — UniFFI bridge version
- `minigraf-c/Cargo.toml` — C FFI version
- `minigraf-node/Cargo.toml` — Node.js crate version
- `minigraf-node/package.json` — Node.js npm version
- `minigraf-ffi/python/pyproject.toml` — Python version
- `minigraf-ffi/java/build.gradle.kts` — Java fallback version
- `Package.swift` — Swift tag URL

**Modified — main repo docs:**
- `CHANGELOG.md` — prepend v1.0.0 section
- `README.md` — badge, install version, test count, platform table, platform sections, scope
- `ROADMAP.md` — Phase 8 completion markers, timeline, Current Focus, Last Updated
- `CLAUDE.md` — Key Files for Next Phase section
- `TEST_COVERAGE.md` — header, add Phase 8.2–8.3d sections
- `BENCHMARKS.md` — prepend Phase 8 note
- `llms.txt` — full rewrite
- `CONTRIBUTING.md` — replace stale checklist, fix two sentences

**Deleted:**
- `pkg/README.md` — identical duplicate of `minigraf-wasm/README.md`; nothing to port

**Rewritten:**
- `minigraf-wasm/README.md` — browser-specific `@minigraf/browser` npm README

**Created:**
- `minigraf-node/README.md`
- `minigraf-ffi/python/README.md`
- `minigraf-c/README.md`
- `minigraf-ffi/java/README.md`

**Modified — wiki (`.wiki/` separate git repo):**
- `.wiki/Architecture.md` — file format v6→v7, workspace crates, thread safety note
- `.wiki/Comparison.md` — platform support row, WASM row update
- `.wiki/Home.md` — file format reference, add Packages section
- `.wiki/Use-Cases.md` — pkg/→minigraf-wasm/, @minigraf/core→@minigraf/browser, version refs, 4 new language sections

---

## Task 1: Create worktree

**Files:** none (setup only)

- [ ] **Step 1: Create worktree for this issue**

```bash
git worktree add .worktrees/release/v1.0.0 -b release/v1.0.0
cd .worktrees/release/v1.0.0
```

- [ ] **Step 2: Verify clean state**

```bash
cargo check --workspace
```

Expected: compiles without errors. Note the current version numbers printed by any warnings — they will all be `0.x` variants.

---

## Task 2: Version bumps

**Files:**
- Modify: `Cargo.toml`
- Modify: `minigraf-ffi/Cargo.toml`
- Modify: `minigraf-c/Cargo.toml`
- Modify: `minigraf-node/Cargo.toml`
- Modify: `minigraf-node/package.json`
- Modify: `minigraf-ffi/python/pyproject.toml`
- Modify: `minigraf-ffi/java/build.gradle.kts`
- Modify: `Package.swift`

- [ ] **Step 1: Bump core Rust crate**

In `Cargo.toml`, change:
```toml
version = "0.25.0"
```
to:
```toml
version = "1.0.0"
```

- [ ] **Step 2: Bump minigraf-ffi**

In `minigraf-ffi/Cargo.toml`, change:
```toml
version = "0.23.0"
```
to:
```toml
version = "1.0.0"
```

Also update the `minigraf` dependency version in `minigraf-ffi/Cargo.toml` if it pins a specific version (check with `grep 'minigraf' minigraf-ffi/Cargo.toml` and update any `version = "0.x"` dependency reference to `version = "1.0"`).

- [ ] **Step 3: Bump minigraf-c**

In `minigraf-c/Cargo.toml`, change:
```toml
version = "0.24.0"
```
to:
```toml
version = "1.0.0"
```

Also check for and update any `minigraf` or `minigraf-ffi` dependency pins to `"1.0"`.

- [ ] **Step 4: Bump minigraf-node Cargo.toml**

In `minigraf-node/Cargo.toml`, change:
```toml
version = "0.25.0"
```
to:
```toml
version = "1.0.0"
```

Also check for and update any `minigraf` dependency pin.

- [ ] **Step 5: Bump minigraf-node package.json**

In `minigraf-node/package.json`, change:
```json
"version": "0.25.0"
```
to:
```json
"version": "1.0.0"
```

- [ ] **Step 6: Bump Python pyproject.toml**

In `minigraf-ffi/python/pyproject.toml`, change:
```toml
version = "0.22.0"
```
to:
```toml
version = "1.0.0"
```

- [ ] **Step 7: Bump Java fallback version**

In `minigraf-ffi/java/build.gradle.kts`, change:
```kotlin
version = System.getenv("RELEASE_VERSION") ?: "0.23.0"
```
to:
```kotlin
version = System.getenv("RELEASE_VERSION") ?: "1.0.0"
```

- [ ] **Step 8: Update Package.swift tag URL**

In `Package.swift`, find the line:
```swift
url: "https://github.com/project-minigraf/minigraf/releases/download/v0.20.1/MinigrafKit-v0.20.1.xcframework.zip",
```
Change both version references in that URL:
```swift
url: "https://github.com/project-minigraf/minigraf/releases/download/v1.0.0/MinigrafKit-v1.0.0.xcframework.zip",
```

Note: also update the `checksum` field on the next line — the correct SHA256 for the v1.0.0 `.xcframework.zip` will be known only after the release CI runs. Insert a placeholder comment for now:
```swift
checksum: "PLACEHOLDER — replace with SHA256 of MinigrafKit-v1.0.0.xcframework.zip after CI produces the artifact"
```
This will be a follow-up edit after the tag is pushed and CI completes.

- [ ] **Step 9: Verify workspace compiles**

```bash
cargo check --workspace
```

Expected: compiles without errors. `Cargo.lock` is updated to reflect the new versions.

- [ ] **Step 10: Commit version bumps**

```bash
git add Cargo.toml Cargo.lock \
        minigraf-ffi/Cargo.toml \
        minigraf-c/Cargo.toml \
        minigraf-node/Cargo.toml \
        minigraf-node/package.json \
        minigraf-ffi/python/pyproject.toml \
        minigraf-ffi/java/build.gradle.kts \
        Package.swift
git commit -m "chore: bump all manifests to v1.0.0"
```

---

## Task 3: CHANGELOG.md

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Prepend the v1.0.0 entry**

Open `CHANGELOG.md`. After the header lines (the first two lines: `# Changelog` and the blank line), insert the following block before the existing `## v0.25.0` entry:

```markdown
## v1.0.0 — Phase 8 Complete (2026-05-01)

### Milestone

This is the **v1.0.0 release**. The public Rust API and the `.graph` file format are now stable
and committed to semantic versioning. File format stability is guaranteed from this release.

### Phase 8 summary

All Phase 8 cross-platform targets have shipped:

- **8.1a** — Browser WASM (`BrowserDb`, `IndexedDbBackend`, `@minigraf/browser` on npm) — v0.20.0
- **8.1b** — Server-side WASM (`wasm32-wasip1` / WASI, Wasmtime/Wasmer CI) — v0.20.0
- **8.2** — Mobile bindings (Android `.aar` on GitHub Packages, iOS `.xcframework` via SPM, UniFFI) — v0.21.0
- **8.3a** — Python (`minigraf` on PyPI, pre-built wheels) — v0.22.0
- **8.3b** — Java/JVM (`io.github.adityamukho:minigraf-jvm` on Maven Central, fat JAR) — v0.23.0
- **8.3c** — C FFI (`minigraf.h` + platform tarballs on GitHub Releases) — v0.24.0
- **8.3d** — Node.js (`minigraf` on npm, pre-built `.node` binaries) — v0.25.0

### Also in this release

- `pkg/` renamed to `minigraf-wasm/`, `swift/` renamed to `minigraf-swift/` — consistent
  top-level naming across all workspace packages (issue #179)
- `@minigraf/browser` now published to npm on every tagged release (issue #179)
- Per-platform READMEs added: `minigraf-wasm/`, `minigraf-node/`, `minigraf-ffi/python/`,
  `minigraf-c/`, `minigraf-ffi/java/`

### Tests

795 tests passing (788 passing + 7 ignored: confirmed `or`+neg-cycle stratification bug,
deferred to post-1.0 backlog).

```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): add v1.0.0 Phase 8 complete entry"
```

---

## Task 4: README.md

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the phase badge**

Find:
```markdown
[![Phase](https://img.shields.io/badge/phase-8.3c%20complete-blue.svg)](https://github.com/project-minigraf/minigraf/blob/main/ROADMAP.md)
```
Replace with:
```markdown
[![Release](https://img.shields.io/badge/release-v1.0.0-blue.svg)](https://github.com/project-minigraf/minigraf/releases/tag/v1.0.0)
```

- [ ] **Step 2: Update the installation version**

Find:
```toml
minigraf = "0.21"
```
Replace with:
```toml
minigraf = "1.0"
```

- [ ] **Step 3: Update the test count in shell commands**

Find:
```bash
cargo test         # run 780 tests
```
Replace with:
```bash
cargo test         # run 795 tests
```

- [ ] **Step 4: Add a platform support table after the feature comparison matrix**

Find the line:
```markdown
**Embedded graph memory for agents, mobile, and the browser — SQLite's simplicity + Datomic's temporal model.**
```
Insert the following block immediately before it:

```markdown
## Platform support

| Platform | Package | Install |
|---|---|---|
| Rust (native) | `minigraf` on crates.io | `cargo add minigraf` |
| Browser WASM | `@minigraf/browser` on npm | `npm install @minigraf/browser` |
| Node.js | `minigraf` on npm | `npm install minigraf` |
| Python | `minigraf` on PyPI | `pip install minigraf` |
| Java/JVM | `io.github.adityamukho:minigraf-jvm` on Maven Central | see [wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) |
| Android | `.aar` on GitHub Packages | see [wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) |
| iOS / macOS | `.xcframework` via Swift Package Manager | see [wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) |
| C / FFI | header + tarball on GitHub Releases | see [wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) |
| WASI | `.wasm` binary on GitHub Releases | see [wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) |

```

- [ ] **Step 5: Update the "For Mobile Apps" section**

Find:
```markdown
### For Mobile Apps

Offline-first storage with retroactive corrections — the bi-temporal model lets you correct a mis-entered value while preserving the original record. Phase 8 will ship iOS `.xcframework` and Android `.aar` via UniFFI.
```
Replace with:
```markdown
### For Mobile Apps

Offline-first storage with retroactive corrections — the bi-temporal model lets you correct a mis-entered value while preserving the original record. Ships as Android `.aar` (GitHub Packages) and iOS `.xcframework` (Swift Package Manager) via UniFFI. See the [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases).
```

- [ ] **Step 6: Update the "For WASM / Browser" section**

Find:
```markdown
### For WASM / Browser

Phase 8.1a complete: IndexedDB backend, `wasm-pack` packaging. Phase 8.1b complete: server-side WASM via `wasm32-wasip1` / WASI (Wasmtime, Wasmer). npm release as `@minigraf/browser` planned for Phase 8.2.

See the [Use Cases](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) wiki page for detailed guides on all three targets.
```
Replace with:
```markdown
### For WASM / Browser

Published as [`@minigraf/browser`](https://www.npmjs.com/package/@minigraf/browser) on npm (IndexedDB-backed, `wasm-pack`). WASI build (`wasm32-wasip1`) available as a GitHub Releases artifact (Wasmtime / Wasmer). See the [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases).

### For Python / Node.js / Java / C

Language bindings ship as `minigraf` on PyPI, `minigraf` on npm (Node.js native addon), `io.github.adityamukho:minigraf-jvm` on Maven Central, and a C header + prebuilt shared library on GitHub Releases. See the [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases).
```

- [ ] **Step 7: Update the Scope section**

Find:
```markdown
Minigraf runs as:
- ✅ An embedded library
- ✅ A standalone binary (interactive REPL)
- ✅ A WebAssembly module — browser (`wasm32-unknown-unknown`) and server-side WASI (`wasm32-wasip1`) (Phase 8.1a/b complete)
```
Replace with:
```markdown
Minigraf runs as:
- ✅ An embedded library
- ✅ A standalone binary (interactive REPL)
- ✅ Browser WASM — `@minigraf/browser` (IndexedDB-backed, `wasm-pack`)
- ✅ Server-side WASM — `wasm32-wasip1` / WASI (Wasmtime, Wasmer, Cloudflare Workers)
- ✅ Android, iOS, Python, Node.js, Java, C — via UniFFI / napi-rs / cbindgen
```

- [ ] **Step 8: Commit**

```bash
git add README.md
git commit -m "docs(readme): v1.0.0 badge, platform support table, update platform sections"
```

---

## Task 5: ROADMAP.md

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Mark Phase 8 header complete**

Find (line ~1108):
```markdown
## Phase 8: Cross-Platform Expansion 🔄 IN PROGRESS
```
Replace with:
```markdown
## Phase 8: Cross-Platform Expansion ✅ COMPLETE
```

- [ ] **Step 2: Update Phase 8 status line**

Find (line ~1112):
```markdown
**Status**: 🔄 In Progress (Phase 8.1 complete — v0.20.0)
```
Replace with:
```markdown
**Status**: ✅ Completed (May 2026) — v1.0.0
```

- [ ] **Step 3: Mark Phase 8.2 complete**

Find (line ~1206):
```markdown
### 8.2 Mobile Bindings

**Goal**: Ship Minigraf as a drop-in native library
```
Replace with:
```markdown
### 8.2 Mobile Bindings ✅ COMPLETE

**Status**: ✅ Completed (April 2026) — v0.21.0

**Goal**: Ship Minigraf as a drop-in native library
```

- [ ] **Step 4: Update the v1.0.0 version table entry**

Find (line ~1590):
```markdown
### v1.0.0 - 🎯 Phase 8 (Cross-platform)
- WASM support (browser + WASI) ✅ Phase 8.1 complete (v0.20.0)
- Mobile bindings (iOS + Android)
- Language bindings (Python, C, Node.js)
```
Replace with:
```markdown
### v1.0.0 - ✅ Phase 8 Complete (Cross-platform)
- Browser WASM (`@minigraf/browser` npm, IndexedDB backend) ✅ v0.20.0
- WASI (`wasm32-wasip1`, Wasmtime/Wasmer CI) ✅ v0.20.0
- Android `.aar` + iOS `.xcframework` (UniFFI) ✅ v0.21.0
- Python `minigraf` on PyPI ✅ v0.22.0
- Java/JVM `minigraf-jvm` on Maven Central ✅ v0.23.0
- C FFI `minigraf.h` + platform tarballs ✅ v0.24.0
- Node.js `minigraf` on npm ✅ v0.25.0
```

- [ ] **Step 5: Update the Timeline section stale lines**

Find (line ~1654):
```markdown
- 🔄 Phase 8.2: In progress — Mobile bindings (Android `.aar` + iOS `.xcframework` via UniFFI 0.31.1), 815 tests
- 🔄 Phase 8: In progress (Cross-platform — WASM ✅, mobile, language bindings → **v1.0.0**)
```
Replace with:
```markdown
- ✅ Phase 8.2: Complete (April 2026) — Mobile bindings (Android `.aar` + iOS `.xcframework` via UniFFI 0.31.1), 795 tests
- ✅ Phase 8.3a: Complete (April 2026) — Python `minigraf` on PyPI, 795 tests
- ✅ Phase 8.3b: Complete (April 2026) — Java/JVM `minigraf-jvm` on Maven Central, 795 tests
- ✅ Phase 8.3c: Complete (April 2026) — C FFI `minigraf.h` + platform tarballs, 795 tests
- ✅ Phase 8.3d: Complete (April 2026) — Node.js `minigraf` on npm, 795 tests
- ✅ Phase 8: Complete (May 2026) — v1.0.0
```

- [ ] **Step 6: Update the Current Focus section**

Find (line ~1662):
```markdown
## Current Focus

**Right Now**: Phase 8.3 ✅ COMPLETE — Language Bindings
```
Replace the entire "Current Focus" block (from `## Current Focus` down to and including the `**Last Updated**` line at ~1697) with:
```markdown
## Current Focus

**Phase 8**: ✅ COMPLETE — v1.0.0 released (May 2026)

**All Phase 8 sub-phases complete**:
- ✅ 8.1a: Browser WASM (`@minigraf/browser` npm) — v0.20.0
- ✅ 8.1b: WASI (`wasm32-wasip1`) — v0.20.0
- ✅ 8.2: Mobile (Android `.aar` + iOS `.xcframework`) — v0.21.0
- ✅ 8.3a: Python (PyPI) — v0.22.0
- ✅ 8.3b: Java/JVM (Maven Central) — v0.23.0
- ✅ 8.3c: C FFI (GitHub Releases) — v0.24.0
- ✅ 8.3d: Node.js (npm) — v0.25.0

**Next**: Phase 9 — Ecosystem & Tooling (post-release optimisation and benchmarking first)

**Key Decisions Made**:
- ✅ Datalog query language (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Recursive rules with semi-naive evaluation
- ✅ UTC-only timestamps (avoids chrono GHSA-wcg3-cvx6-7396)
- ✅ Packed pages over one-per-page (philosophy: small binary, efficient storage)
- ✅ Approximate LRU (read-lock on hits — avoids write-lock contention)
- ✅ Phase 8 = v1.0.0 (cross-platform completion is the 1.0 milestone)

See [GitHub Issues](https://github.com/project-minigraf/minigraf/issues) for specific tasks.

---

**Last Updated**: Phase 8 Complete (May 2026) — 795 tests passing, v1.0.0
```

- [ ] **Step 7: Commit**

```bash
git add ROADMAP.md
git commit -m "docs(roadmap): mark Phase 8 complete, update timeline and Current Focus to v1.0.0"
```

---

## Task 6: CLAUDE.md, TEST_COVERAGE.md, BENCHMARKS.md

**Files:**
- Modify: `CLAUDE.md`
- Modify: `TEST_COVERAGE.md`
- Modify: `BENCHMARKS.md`

- [ ] **Step 1: Update CLAUDE.md Key Files for Next Phase**

Find (line ~175):
```markdown
## Key Files for the Next Phase

See `ROADMAP.md` for the current next phase spec, including the relevant files and implementation details.
```
Replace with:
```markdown
## Key Files for the Next Phase

Phase 8 is complete — v1.0.0 released. Phase 9 (Ecosystem & Tooling) is next.

Phase 9 relevant areas (see `ROADMAP.md` for full spec):
- `examples/` — end-to-end annotated examples (agentic memory, offline-first mobile, browser PWA)
- `.wiki/` — cookbook-style guides for each Phase 9 deliverable
- `BENCHMARKS.md` — post-1.0 performance baseline updates
```

- [ ] **Step 2: Update CLAUDE.md header test count line**

Find the status line at the top of CLAUDE.md (it's inside the "Build and Run Commands" or test section). Look for:
```markdown
**795 tests passing** (unit + integration + doc).
```
This is already correct. Verify it reads `795` and leave it unchanged.

- [ ] **Step 3: Update TEST_COVERAGE.md header**

Find (line 3):
```markdown
**Last Updated**: Phase 8.3c COMPLETE - C bindings (GitHub Releases tarballs), 795 tests ✅
```
Replace with:
```markdown
**Last Updated**: Phase 8 COMPLETE — v1.0.0 (May 2026), 795 tests ✅
```

- [ ] **Step 4: Add Phase 8.2–8.3d completion sections to TEST_COVERAGE.md**

Find the line (line 34):
```markdown
## Phase 8.1 Completion Status: ✅ COMPLETE
```
Insert the following block immediately **before** that line:

```markdown
## Phase 8 Completion Status: ✅ COMPLETE — v1.0.0

All Phase 8 sub-phases complete. See per-phase sections below.

---

## Phase 8.3d Completion Status: ✅ COMPLETE

**Phase 8.3d Features** (Node.js, complete — v0.25.0):
- ✅ `minigraf-node/src/lib.rs` — napi-rs bindings: `MiniGrafDb` class (open, inMemory, execute, checkpoint)
- ✅ `minigraf-node/package.json` — `minigraf` npm package; prebuilt `.node` binaries for Linux x86_64/aarch64, macOS universal2, Windows x86_64
- ✅ `node-ci.yml` — PR test matrix on 4 platforms
- ✅ `node-release.yml` — cross-compile, assemble platform packages, publish to npm on tag

---

## Phase 8.3c Completion Status: ✅ COMPLETE

**Phase 8.3c Features** (C FFI, complete — v0.24.0):
- ✅ `minigraf-c/src/lib.rs` — `cdylib` + `staticlib`; 7 exported functions: `minigraf_open`, `minigraf_open_in_memory`, `minigraf_execute`, `minigraf_string_free`, `minigraf_checkpoint`, `minigraf_close`, `minigraf_last_error`
- ✅ `minigraf-c/include/minigraf.h` — committed stable header (cbindgen-generated); header drift check in CI
- ✅ `c-ci.yml` — PR test matrix on 4 platforms + header drift check
- ✅ `c-release.yml` — builds platform tarballs (`.tar.gz` / `.zip`), uploads to GitHub Releases

---

## Phase 8.3b Completion Status: ✅ COMPLETE

**Phase 8.3b Features** (Java/JVM, complete — v0.23.0):
- ✅ `minigraf-ffi/java/` — Gradle 8.11 project: `build.gradle.kts`, `settings.gradle.kts`, `NativeLoader.kt` (runtime native extraction from JAR resources)
- ✅ `minigraf-ffi/java/src/test/kotlin/.../BasicTest.kt` — JUnit 5 suite: in-memory, transact/query, error handling, file-backed persistence
- ✅ `java-ci.yml` — PR test matrix on 4 platforms (Linux x86_64, Linux aarch64, macOS universal2, Windows x86_64)
- ✅ `java-release.yml` — cross-compiles natives, assembles fat JAR, publishes to Maven Central via NMCP

---

## Phase 8.3a Completion Status: ✅ COMPLETE

**Phase 8.3a Features** (Python, complete — v0.22.0):
- ✅ `minigraf-ffi/python/` — maturin project: `pyproject.toml`, Python extension module via UniFFI
- ✅ Pre-built wheels for Linux x86_64/aarch64, macOS universal2, Windows x86_64; no Rust toolchain required by end users
- ✅ `python-ci.yml` — PR test matrix on 4 platforms
- ✅ `python-release.yml` — builds wheels, publishes to PyPI on tag

---

## Phase 8.2 Completion Status: ✅ COMPLETE

**Phase 8.2 Features** (Mobile, complete — v0.21.0):
- ✅ `minigraf-ffi/src/lib.rs` — UniFFI 0.31 bindings: `MiniGrafDb` (open, openInMemory, execute, checkpoint), `MiniGrafError` (Parse, Query, Storage, Other)
- ✅ Android `.aar` release artifact — published to GitHub Packages (`io.github.adityamukho:minigraf-android`)
- ✅ iOS `.xcframework` release artifact — distributed via Swift Package Manager (`Package.swift` at repo root)
- ✅ `mobile.yml` CI — cross-compiles Android targets with `cargo-ndk`, generates Kotlin/Swift UniFFI bindings, assembles AAR and xcframework, publishes both on every tag
- ✅ `docs-check` CI job added to `rust.yml` and `release.yml` — gates releases on `cargo doc --all-features` passing cleanly

---

```

- [ ] **Step 5: Prepend Phase 8 note to BENCHMARKS.md**

Open `BENCHMARKS.md`. After the first line (`# Minigraf Benchmarks`), insert:

```markdown

## Phase 8 Note

Phase 8 (v0.20.0–v1.0.0) added cross-platform targets: Browser WASM, WASI, Android, iOS,
Python, Node.js, Java, and C bindings. No changes were made to the native query or storage path.
All benchmark numbers below are unchanged from Phase 7 (v0.19.0).

```

- [ ] **Step 6: Commit**

```bash
git add CLAUDE.md TEST_COVERAGE.md BENCHMARKS.md
git commit -m "docs: update CLAUDE.md next-phase pointer, TEST_COVERAGE.md Phase 8 sections, BENCHMARKS.md Phase 8 note"
```

---

## Task 7: llms.txt

**Files:**
- Modify: `llms.txt` (full rewrite)

- [ ] **Step 1: Replace llms.txt with the following content**

```markdown
# Minigraf

> Embedded, single-file, bi-temporal graph database with Datalog queries — written in Rust.

Minigraf is the SQLite of graph databases: zero configuration, one `.graph` file, embedded as a library. It stores facts in an Entity-Attribute-Value (EAV) model and queries them with Datalog, including recursive rules for graph traversal and stratified negation (`not` / `not-join`). Every fact carries two independent time dimensions (transaction time and valid time), enabling full bi-temporal time travel.

**Current version**: 1.0.0
**Maturity**: ACID + WAL are production-quality. Covering indexes (EAVT/AEVT/AVET/VAET), packed page storage, LRU page cache, and on-disk B+tree indexes are complete. Stratified negation, scalar aggregation, arithmetic/predicate expression clauses, disjunction (`or`/`or-join`), window functions (`sum/count/min/max/avg/rank/row-number`), user-defined functions, and prepared statements are complete. Phase 8 complete: Browser WASM (`@minigraf/browser` npm), WASI (`wasm32-wasip1`), Android/iOS (UniFFI), Python (PyPI), Java/JVM (Maven Central), C FFI, and Node.js (npm) all ship at v1.0.0. File format stable from v1.0.0. 795 tests.

## Use for

- **Agent memory with provenance**: Store facts an agent asserted, when it asserted them, and query any past state. Retract and correct beliefs without losing history. Rewind to the exact knowledge state at the moment of a bad decision for root cause analysis.
- **Verifiable agent reasoning**: Preserve an agent's full decision-making lineage. Post-hoc audits can reconstruct what the agent believed at any transaction counter or timestamp.
- **Browser agent memory**: Run entirely client-side with `@minigraf/browser` (npm). Persists to IndexedDB; portable `.graph` files are byte-identical to native.
- **Mobile offline-first applications**: Android (Kotlin/Java) and iOS (Swift) native bindings via UniFFI. No Rust required. Same single-file `.graph` format.
- **Python/Node.js/Java scripting and server-side embedding**: Pre-built packages on PyPI, npm, and Maven Central. No build step required.
- **Task planning agents**: Model sub-task DAGs as a graph. Update dependencies and status over time. Query historical task states with `:as-of`.
- **Code dependency / debugging agents**: Embed call graphs or module dependency graphs; traverse with recursive Datalog rules.
- **Audit-trail applications**: Compliance-grade history where both "what was recorded when" (transaction time) and "what was true when" (valid time) matter independently.
- **Knowledge graphs**: Interconnected entity-relationship data with recursive traversal (e.g. reachability, ancestry, dependency chains).
- **Event sourcing / temporal debugging**: Replay past states exactly as they existed at any transaction counter or wall-clock timestamp.
- **Local-first / offline applications**: One portable `.graph` file, no server, no setup.
- **Embedded Rust applications**: Link as a library; no daemon, no network socket.

## Do not use for

- Distributed or replicated databases (no clustering by design).
- Client-server deployments (embedded-only by design).
- Workloads requiring more than ~1M facts with sub-second query latency (current query is O(N) full scan; predicate pushdown is planned — see ROADMAP).
- Applications needing SQL or Cypher (Datalog only).
- Fuzzy / semantic similarity search (no vector index). Use a vector store for retrieval; use Minigraf for the relational backbone and audit layer.
- Global shared memory across a distributed agent fleet. If multiple agent nodes need coherent read/write access to the same memory store, Minigraf is the wrong tool — use a distributed database. Minigraf is per-agent-instance memory, not a shared brain.

## Pair with (GraphRAG pattern)

Minigraf has no vector search by design. In agentic stacks that need both fuzzy retrieval and auditable relationships, the recommended split is:

- **Vector store** (Chroma, Pinecone, Qdrant, etc.) — holds embeddings + an entity UUID per document
- **Minigraf** — holds the graph of relationships and full bitemporal history keyed on those UUIDs

The vector store answers "what is similar to this prompt?"; Minigraf answers "given this entity, what are its exact relationships, who recorded them, and what did we believe at time T?" Each layer does what it's good at; neither duplicates the other.

## Key concepts

**Bi-temporal model**
Every fact has two independent time axes:
- *Transaction time* (`tx_count`): when the fact was recorded in the database. Immutable — set by the system.
- *Valid time* (`valid_from` / `valid_to`): when the fact was true in the real world. Set by the caller.

Query either axis independently or together:
- `:as-of <tx_count|timestamp>` — time-travel to a past database state
- `:valid-at <timestamp>` — query facts valid at a real-world point in time

**EAV facts**
The unit of storage is `(entity, attribute, value, valid_from, valid_to, tx_id, tx_count, asserted)`. Entities are UUIDs. Attributes are keywords (`:person/name`). Values are strings, integers, floats, booleans, entity refs, keywords, or null.

**Datalog queries**
Pattern matching with variable unification. Recursive rules use semi-naive fixed-point evaluation. Transitive closure and cycle-safe graph traversal are first-class. Stratified negation (`not` / `not-join`), scalar aggregation (`count`, `sum`, `min`, `max`, `count-distinct`, `sum-distinct`), arithmetic/predicate expression clauses (`[(< ?age 30)]`, `[(+ ?a ?b) ?c]`), disjunction (`or` / `or-join`), window functions (`sum/count/min/max/avg/rank/row-number :over (:partition-by … :order-by …)`), and user-defined functions (custom aggregates + predicates via `FunctionRegistry`) are all supported.

**Prepared statements**
Parse and plan a query once; execute thousands of times with different bind values. `$slot` tokens accepted in entity, value, `:as-of`, and `:valid-at` positions. `BindValue` variants: `Entity(Uuid)`, `Val(Value)`, `TxCount(u64)`, `Timestamp(i64)`, `AnyValidTime`.

**ACID transactions**
`begin_write()` → `commit()` / `rollback()`. Fact-level WAL with CRC32 protection. Crash recovery on open.

**Single-file storage**
Page-based `.graph` file (4 KB pages, magic `MGRF`, format v7). Packed fact pages (~25 facts/page). On-disk B+tree indexes (EAVT/AEVT/AVET/VAET) with LRU page cache. WAL sidecar (`.wal`) deleted on clean close. Automatic migration from v1–v6. Endian-safe, cross-platform. File format stable from v1.0.0.

## API (Rust)

```rust
use minigraf::OpenOptions;

// Open or create
let db = OpenOptions::new().path("memory.graph").open()?;

// Transact facts
db.execute(r#"(transact [[:agent-1 :belief/fact "Paris is in France"]
                          [:agent-1 :belief/confidence 0.98]])"#)?;

// Transact with explicit valid time
db.execute(r#"(transact {:valid-from "2024-01-01" :valid-to "2025-01-01"}
                        [[:agent-1 :employment/status :active]])"#)?;

// Recursive rule: reachability
db.execute(r#"(rule [(reachable ?a ?b) [?a :knows ?b]])
              (rule [(reachable ?a ?b) [?a :knows ?m] (reachable ?m ?b)])"#)?;

// Negation: exclude banned entities
db.execute(r#"(query [:find ?name
                      :where [?e :person/name ?name]
                             (not [?e :person/banned true])])"#)?;

// Existential negation: services with no deprecated dependency
db.execute(r#"(query [:find ?name
                      :where [?svc :service/name ?name]
                             (not-join [?svc]
                                       [?svc :depends-on ?lib]
                                       [?lib :lib/deprecated true])])"#)?;

// Query
db.execute(r#"(query [:find ?fact :where [:agent-1 :belief/fact ?fact]])"#)?;

// Time travel — as of past transaction counter
db.execute("(query [:find ?status :as-of 10 :where [:agent-1 :employment/status ?status]])")?;

// Time travel — valid at real-world date
db.execute(r#"(query [:find ?status :valid-at "2024-06-01"
                      :where [:agent-1 :employment/status ?status]])"#)?;

// Prepared statement — parse once, execute many times with $slot bind tokens
use minigraf::BindValue;
let pq = db.prepare(
    "(query [:find ?fact :as-of $tx :where [$entity :belief/fact ?fact]])"
)?;
let r1 = pq.execute(&[("tx", BindValue::TxCount(5)), ("entity", BindValue::Entity(agent_id))])?;
let r2 = pq.execute(&[("tx", BindValue::TxCount(10)), ("entity", BindValue::Entity(agent_id))])?;

// Explicit transaction
let mut tx = db.begin_write()?;
tx.execute(r#"(retract [[:agent-1 :belief/fact "Paris is in France"]])"#)?;
tx.commit()?;
```

## Datalog syntax reference

```
;; Transact
(transact [[ <entity> <attribute> <value> ] ...])

;; Transact with valid time
(transact {:valid-from "ISO8601" :valid-to "ISO8601"} [[ ... ]])

;; Retract
(retract [[ <entity> <attribute> <value> ] ...])

;; Query
(query [:find ?var ...                        ;; plain variable
              (count ?e)                      ;; aggregate: count, count-distinct,
              (sum ?salary)                   ;;   sum, sum-distinct, min, max
              (sum ?v :over (:order-by ?v))   ;; window: sum/count/min/max/avg/rank/row-number
        :with ?grouping-var ...               ;; optional, extra grouping variables
        :as-of <tx_count|"ISO8601">           ;; optional, transaction time
        :valid-at <"ISO8601"|:any-valid-time> ;; optional, valid time
        :where [<e> <a> <v>] ...
               (not [<e> <a> <v>] ...)       ;; optional, negation
               (not-join [?join-var ...] ...) ;; optional, existential negation
               (or branch1 branch2 ...)       ;; optional, disjunction
               (or-join [?v ...] b1 b2 ...)  ;; optional, existential disjunction
               (and clause1 clause2 ...)      ;; group clauses (inside or/or-join)
               [(<op> ?a ?b)]                 ;; filter predicate: <, >, <=, >=, =, !=, string?
               [(<op> ?a ?b) ?result]         ;; arithmetic binding: +, -, *, /
       ])

;; Prepared statement with $slot bind tokens
(query [:find ?fact :as-of $tx :where [$entity :belief/fact ?fact]])

;; Recursive rule
(rule [(<rule-name> ?arg ...) <body-clauses> ...])

;; Negation in rule body
(rule [(<rule-name> ?arg ...)
       [?arg <attr> <val>]
       (not [?arg :excluded true])
       (not-join [?arg] [?arg :depends-on ?d] [?d :status :bad])])
```

## Source layout

- `src/db.rs` — public API: `Minigraf`, `OpenOptions`, `WriteTransaction`, `PreparedQuery`, `BindValue`; `register_aggregate` / `register_predicate` for UDFs; `prepare(query_str)`
- `src/graph/types.rs` — `Fact`, `Value`, EAV types, bi-temporal fields
- `src/graph/storage.rs` — in-memory fact store with temporal query methods and `net_asserted_facts`
- `src/query/datalog/` — parser, executor, matcher, evaluator, optimizer, stratification, rules, types, functions, prepared
- `src/query/datalog/stratification.rs` — `DependencyGraph`, `stratify()` — negative edges + cycle detection
- `src/query/datalog/evaluator.rs` — `RecursiveEvaluator` (semi-naive), `StratifiedEvaluator`, `evaluate_not_join`
- `src/query/datalog/functions.rs` — `FunctionRegistry`: aggregate/window/predicate registry; UDF registration
- `src/query/datalog/prepared.rs` — `PreparedQuery`: parse-once/execute-many; `BindValue` enum; `$slot` substitution
- `src/storage/mod.rs` — `StorageBackend` trait, `FileHeader` v7, `CommittedFactReader` / `CommittedIndexReader` traits
- `src/storage/backend/` — `file.rs` (native), `memory.rs` (tests), `indexeddb.rs` (browser WASM)
- `src/storage/index.rs` — EAVT/AEVT/AVET/VAET index key types, `FactRef`, `encode_value`
- `src/storage/btree_v6.rs` — on-disk B+tree (current); `btree.rs` — legacy v5 (migration only)
- `src/storage/cache.rs` — LRU page cache (approximate-LRU, configurable capacity)
- `src/storage/packed_pages.rs` — packed fact page format (~25 facts/4KB page), `MAX_FACT_BYTES`
- `src/storage/persistent_facts.rs` — v7 save/load, auto-migration v1–v6→v7
- `src/wal.rs` — write-ahead log, CRC32 entries, crash recovery
- `src/temporal.rs` — UTC timestamp parsing (avoids chrono CVE GHSA-wcg3-cvx6-7396)
- `minigraf-ffi/src/lib.rs` — UniFFI bindings: `MiniGrafDb`, `MiniGrafError` (Android, iOS, Python, Java)
- `minigraf-c/src/lib.rs` — C FFI (`cdylib` + `staticlib`): `minigraf_open`, `minigraf_execute`, `minigraf_string_free`, `minigraf_checkpoint`, `minigraf_close`, `minigraf_last_error`
- `minigraf-node/src/lib.rs` — Node.js bindings via napi-rs: `MiniGrafDb` class
- `minigraf-wasm/` — wasm-pack output: `@minigraf/browser` npm package (IndexedDB-backed browser WASM)

## Performance summary (v0.19.0)

See [BENCHMARKS.md](https://github.com/project-minigraf/minigraf/blob/main/BENCHMARKS.md) for full tables and methodology. Phase 8 (v0.20.0–v1.0.0) added cross-platform targets without touching the native query or storage path — benchmark numbers are unchanged from Phase 7.

- **Insert**: ~2.7 µs/fact (in-memory), ~3.6 µs/fact (file-backed WAL). Flat across 1K–100K facts.
- **Query (point lookup)**: O(N) full scan — 4.3–4.5 s at 1M facts.
- **Open**: 1.31 s at 1M facts (indexes paged in on demand via B+tree; ~2.4× faster than v5).
- **Peak heap**: 1.05 GB at 1M facts (~21% less than v5 — indexes not loaded into RAM).

## Links

- [Repository](https://github.com/project-minigraf/minigraf)
- [crates.io](https://crates.io/crates/minigraf)
- [README](https://github.com/project-minigraf/minigraf/blob/main/README.md) — current status and quick start
- [ROADMAP](https://github.com/project-minigraf/minigraf/blob/main/ROADMAP.md) — phase-by-phase plan
- [BENCHMARKS](https://github.com/project-minigraf/minigraf/blob/main/BENCHMARKS.md) — Criterion results at 1K–1M facts
- [Philosophy](https://github.com/project-minigraf/minigraf/blob/main/PHILOSOPHY.md)
- [Security Policy](https://github.com/project-minigraf/minigraf/security/policy)
- [Wiki: Architecture](https://github.com/project-minigraf/minigraf/wiki/Architecture) — module structure, data model, file format, query pipeline
- [Wiki: Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference) — complete syntax reference
- [Wiki: Use Cases](https://github.com/project-minigraf/minigraf/wiki/Use-Cases) — AI agents, mobile, browser, Python, Node.js, Java, C
- [Wiki: Comparison](https://github.com/project-minigraf/minigraf/wiki/Comparison) — vs XTDB, Cozo, Datomic, Neo4j, SQLite and others
- [@minigraf/browser on npm](https://www.npmjs.com/package/@minigraf/browser) — Browser WASM
- [minigraf on npm](https://www.npmjs.com/package/minigraf) — Node.js
- [minigraf on PyPI](https://pypi.org/project/minigraf) — Python
- [minigraf-jvm on Maven Central](https://central.sonatype.com/artifact/io.github.adityamukho/minigraf-jvm) — Java/JVM
- [C header + libraries on GitHub Releases](https://github.com/project-minigraf/minigraf/releases) — C FFI, WASI `.wasm`, Android `.aar`, iOS `.xcframework`
```

- [ ] **Step 2: Commit**

```bash
git add llms.txt
git commit -m "docs(llms.txt): full rewrite for v1.0.0 — version, platforms, prepared statements, source layout, links"
```

---

## Task 8: CONTRIBUTING.md

**Files:**
- Modify: `CONTRIBUTING.md`

- [ ] **Step 1: Replace the Pre-Publishing Checklist section**

Find the entire section starting with:
```markdown
## Pre-Publishing Checklist (crates.io)
```
and ending just before `## Code of Conduct` (the section is ~40 lines). Replace it with:

```markdown
## Release Process

Releases are managed by the project maintainer. The process is documented in issue #133 and the
`docs/superpowers/specs/` design files. For each release:

1. All prerequisite issue PRs merged and CI green
2. Version bumped consistently across all manifests (`Cargo.toml`, `package.json`, `pyproject.toml`, `build.gradle.kts`, `Package.swift`)
3. `cargo check --workspace` passes cleanly
4. All docs synced (see `CLAUDE.md` — "Sync all docs at phase completion")
5. Tag pushed — CI publishes to crates.io, PyPI, npm, and Maven Central automatically

```

- [ ] **Step 2: Update the "What We Welcome" bullet**

Find:
```markdown
- Cross-platform compatibility fixes (Linux, macOS, Windows, eventually WASM/mobile)
```
Replace with:
```markdown
- Cross-platform compatibility fixes (Linux, macOS, Windows, WASM, mobile, language bindings)
```

- [ ] **Step 3: Update the stale coverage sentence**

Find:
```markdown
The Phase 7.5 target is ≥90% branch coverage on `src/query/datalog/` modules. Re-run after adding tests to confirm progress.
```
Replace with:
```markdown
Run branch coverage to check overall project health before submitting a PR.
```

- [ ] **Step 4: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs(contributing): replace stale pre-publish checklist with release process pointer"
```

---

## Task 9: pkg/ delete + minigraf-wasm/README.md rewrite

**Files:**
- Delete: `pkg/README.md`
- Modify: `minigraf-wasm/README.md` (full rewrite)

- [ ] **Step 1: Verify pkg/README.md is safe to delete**

```bash
diff pkg/README.md minigraf-wasm/README.md
```

Expected: differences exist (both are stale root-README copies at slightly different versions), but neither contains unique canonical content — confirm there is nothing in `pkg/README.md` that is absent from the new `minigraf-wasm/README.md` you are about to write. Proceed with deletion.

- [ ] **Step 2: Delete pkg/README.md**

```bash
git rm pkg/README.md
```

- [ ] **Step 3: Rewrite minigraf-wasm/README.md**

Replace the entire file with:

```markdown
# @minigraf/browser

[![npm](https://img.shields.io/npm/v/@minigraf/browser.svg)](https://www.npmjs.com/package/@minigraf/browser)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for the browser — Datalog queries, time travel, IndexedDB persistence

Minigraf in the browser: zero configuration, persistent graph storage backed by IndexedDB, Datalog queries with recursive rules and time travel. One API, no server required.

## Install

```sh
npm install @minigraf/browser
```

## Quick start

```javascript
import init, { BrowserDb } from '@minigraf/browser';
await init();

// Persistent database (survives page reloads — backed by IndexedDB)
const db = await BrowserDb.open('my-graph');

// In-memory database (ephemeral / testing)
const mem = BrowserDb.openInMemory();

// Transact facts
const r = JSON.parse(await db.execute(
  '(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])'
));
// { "transacted": 1 }

// Query with Datalog
const q = JSON.parse(await db.execute(
  '(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])'
));
// { "variables": ["?name", "?age"], "results": [["Alice", 30]] }

// Time travel — state as of transaction 1
const snap = JSON.parse(await db.execute(
  '(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])'
));
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Export / import

The `.graph` binary format is byte-identical between browser and native builds. Export a snapshot or load a native-generated file:

```javascript
// Export (Uint8Array — byte-identical to a native .graph file)
const bytes = db.exportGraph();

// Import a native .graph file (must be checkpointed — no pending WAL)
const file = document.querySelector('input[type=file]').files[0];
await db.importGraph(new Uint8Array(await file.arrayBuffer()));
```

## Constraints

- Requires a browser environment with IndexedDB. Not compatible with Node.js — use the [`minigraf` npm package](https://www.npmjs.com/package/minigraf) for Node.js instead.
- Single-threaded. Runs on the main thread or in a Web Worker; no shared state across workers.

## wasm-pack clobbering note (for maintainers)

`wasm-pack build` may overwrite this file. If it does, `wasm-release.yml` must copy the canonical
README from a stable source (e.g. a root-level `minigraf-wasm.README.md`) into the build output
directory before `npm publish`. Verify this on first release after any `wasm-pack` version upgrade.

## Links

- [Full browser integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#wasm--browser)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
```

- [ ] **Step 4: Commit**

```bash
git add pkg/README.md minigraf-wasm/README.md
git commit -m "docs: delete pkg/README.md (duplicate), rewrite minigraf-wasm/README.md as @minigraf/browser npm README"
```

---

## Task 10: Per-platform READMEs

**Files:**
- Create: `minigraf-node/README.md`
- Create: `minigraf-ffi/python/README.md`
- Create: `minigraf-c/README.md`
- Create: `minigraf-ffi/java/README.md`

- [ ] **Step 1: Create minigraf-node/README.md**

```markdown
# minigraf (Node.js)

[![npm](https://img.shields.io/npm/v/minigraf.svg)](https://www.npmjs.com/package/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Node.js — Datalog queries, time travel, native addon

Minigraf for Node.js: a native addon (no WASM, full file I/O) with pre-built binaries for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No build step required.

## Install

```sh
npm install minigraf
```

## Quick start

```typescript
import { MiniGrafDb } from 'minigraf';

// File-backed database
const db = new MiniGrafDb('myapp.graph');

// In-memory database (ephemeral / testing)
const mem = MiniGrafDb.inMemory();

// Transact facts
const r = JSON.parse(db.execute(
  '(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])'
));
// { "transacted": 1 }

// Query with Datalog
const q = JSON.parse(db.execute(
  '(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])'
));
// { "variables": ["?name", "?age"], "results": [["Alice", 30]] }

// Time travel — state as of transaction 1
const snap = JSON.parse(db.execute(
  '(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])'
));

// Flush dirty pages to disk
db.checkpoint();
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Links

- [Full Node.js integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#nodejs--typescript)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
```

- [ ] **Step 2: Create minigraf-ffi/python/README.md**

```markdown
# minigraf (Python)

[![PyPI](https://img.shields.io/pypi/v/minigraf.svg)](https://pypi.org/project/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Python — Datalog queries, time travel, pre-built wheels

Minigraf for Python: pre-built wheels for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No Rust toolchain required.

## Install

```sh
pip install minigraf
```

## Quick start

```python
from minigraf import MiniGrafDb

# File-backed database
db = MiniGrafDb.open("myapp.graph")

# In-memory database (ephemeral / testing)
mem = MiniGrafDb.open_in_memory()

# Transact facts
r = db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])')
# '{"transacted": 1}'

# Query with Datalog
q = db.execute('(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])')
# '{"variables": ["?name", "?age"], "results": [["Alice", 30]]}'

# Time travel — state as of transaction 1
snap = db.execute('(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])')

# Flush dirty pages to disk
db.checkpoint()
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Links

- [Full Python integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#python)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
```

- [ ] **Step 3: Create minigraf-c/README.md**

```markdown
# minigraf C bindings

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database — C header + prebuilt shared library

Minigraf for C and any language with a C FFI. Distributed as platform tarballs containing `minigraf.h` and prebuilt shared and static libraries.

## Install

Download the platform tarball from [GitHub Releases](https://github.com/project-minigraf/minigraf/releases):

```sh
# Linux x86_64 example
curl -L https://github.com/project-minigraf/minigraf/releases/download/v1.0.0/minigraf-c-v1.0.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
```

Each archive contains:
- `include/minigraf.h` — stable C header (cbindgen-generated)
- `lib/libminigraf_c.so` (Linux) / `libminigraf_c.dylib` (macOS) / `minigraf_c.dll` (Windows)
- `lib/libminigraf_c.a` (Linux/macOS) / `minigraf_c.lib` (Windows)

## Quick start

```c
#include "minigraf.h"
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    char *err = NULL;
    MiniGrafDb *db = minigraf_open("myapp.graph", &err);
    if (!db) { fprintf(stderr, "open: %s\n", err); free(err); return 1; }

    char *result = minigraf_execute(
        db,
        "(transact [[:alice :person/name \"Alice\"] [:alice :person/age 30]])",
        &err
    );
    if (!result) { fprintf(stderr, "execute: %s\n", err); free(err); }
    else { printf("%s\n", result); minigraf_string_free(result); }

    result = minigraf_execute(
        db,
        "(query [:find ?name :where [?e :person/name ?name]])",
        &err
    );
    if (!result) { fprintf(stderr, "execute: %s\n", err); free(err); }
    else { printf("%s\n", result); minigraf_string_free(result); }

    minigraf_checkpoint(db, NULL);
    minigraf_close(db);
    return 0;
}
```

## Memory contract

Mirrors SQLite:
- `minigraf_open` — caller owns the `MiniGrafDb*`; free with `minigraf_close`
- `minigraf_execute` — returns a heap-allocated JSON string; caller must free with `minigraf_string_free`
- Error strings (`char **err` out-param) are heap-allocated; caller frees with `free()`

## API summary

| Function | Description |
|---|---|
| `minigraf_open(path, err)` | Open or create a file-backed database |
| `minigraf_open_in_memory(err)` | Open an in-memory database |
| `minigraf_execute(db, datalog, err)` | Execute a Datalog command; returns JSON string |
| `minigraf_string_free(s)` | Free a string returned by `minigraf_execute` |
| `minigraf_checkpoint(db, err)` | Flush dirty pages to disk |
| `minigraf_close(db)` | Close the database |
| `minigraf_last_error()` | Get the last error message (thread-local) |

See `include/minigraf.h` for the complete API.

## Links

- [Full C FFI integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#c-ffi)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
```

- [ ] **Step 4: Create minigraf-ffi/java/README.md**

```markdown
# minigraf-jvm

[![Maven Central](https://img.shields.io/maven-central/v/io.github.adityamukho/minigraf-jvm.svg)](https://central.sonatype.com/artifact/io.github.adityamukho/minigraf-jvm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Java/Kotlin — Datalog queries, time travel, fat JAR with embedded natives

Minigraf for Java and Kotlin on the desktop JVM. Fat JAR with embedded native libraries for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No Rust toolchain required.

## Install

### Gradle (Kotlin DSL)

```kotlin
dependencies {
    implementation("io.github.adityamukho:minigraf-jvm:1.0.0")
}
```

### Maven

```xml
<dependency>
    <groupId>io.github.adityamukho</groupId>
    <artifactId>minigraf-jvm</artifactId>
    <version>1.0.0</version>
</dependency>
```

## Quick start

```kotlin
import io.github.adityamukho.minigraf.MiniGrafDb
import org.json.JSONObject

// File-backed database
val db = MiniGrafDb.open("/path/to/myapp.graph")

// In-memory database (ephemeral / testing)
val mem = MiniGrafDb.openInMemory()

// Transact facts
db.execute("""(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])""")

// Query with Datalog
val json = JSONObject(db.execute(
    "(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])"
))
// json.getJSONArray("results").getJSONArray(0).getString(0) == "Alice"

// Time travel — state as of transaction 1
val snap = db.execute("(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])")

// Flush dirty pages to disk
db.checkpoint()
```

## Links

- [Full Java/JVM integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#java--jvm)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
```

- [ ] **Step 5: Commit**

```bash
git add minigraf-node/README.md \
        minigraf-ffi/python/README.md \
        minigraf-c/README.md \
        minigraf-ffi/java/README.md
git commit -m "docs: add per-platform READMEs for Node.js, Python, C FFI, and Java/JVM packages"
```

---

## Task 11: Verification pass + open PR

**Files:** none (verification only, then PR)

- [ ] **Step 1: Run cargo check**

```bash
cargo check --workspace
```

Expected: exits 0, no errors.

- [ ] **Step 2: Run tests to confirm count**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: output includes `test result: ok. 788 passed; 7 ignored` (or similar — total should be 795).

- [ ] **Step 3: Run rustdoc check**

```bash
cargo doc --workspace --no-deps 2>&1 | grep -i "error\|warning" | head -20
```

Expected: no errors. Warnings about missing docs on new crates are acceptable if those crates have `#![allow(missing_docs)]`.

- [ ] **Step 4: Grep for missed version references**

```bash
grep -rn "0\.\(19\|20\|21\|22\|23\|24\|25\)\.0" \
  CHANGELOG.md README.md ROADMAP.md CLAUDE.md TEST_COVERAGE.md BENCHMARKS.md \
  llms.txt CONTRIBUTING.md \
  minigraf-wasm/README.md minigraf-node/README.md \
  minigraf-ffi/python/README.md minigraf-c/README.md minigraf-ffi/java/README.md \
  2>/dev/null
```

Expected: only historical references (e.g. inside CHANGELOG entries describing what shipped in each old version) — no forward-facing install instructions or status lines referencing old versions.

- [ ] **Step 5: Grep for stale pkg/ path references**

```bash
grep -rn "pkg/" \
  CHANGELOG.md README.md ROADMAP.md llms.txt CONTRIBUTING.md \
  minigraf-wasm/README.md \
  2>/dev/null
```

Expected: no matches (all `pkg/` references should have been updated to `minigraf-wasm/`).

- [ ] **Step 6: Grep for stale @minigraf/core references**

```bash
grep -rn "@minigraf/core" \
  CHANGELOG.md README.md ROADMAP.md llms.txt CONTRIBUTING.md \
  minigraf-wasm/README.md \
  2>/dev/null
```

Expected: no matches.

- [ ] **Step 7: Grep for Phase 8 IN PROGRESS markers**

```bash
grep -n "IN PROGRESS\|In Progress\|In progress" ROADMAP.md README.md
```

Expected: no matches remaining in Phase 8 context. (Phase 6 may still have an old marker — that is acceptable as a historical record.)

- [ ] **Step 8: Open the PR**

```bash
gh pr create \
  --title "docs: Phase 8 complete — v1.0.0 docs sync, version bumps, platform READMEs" \
  --body "$(cat <<'EOF'
## Summary

- Bumps all workspace manifests to `1.0.0` (Cargo, npm, PyPI, Gradle Java, Package.swift)
- Adds `v1.0.0` CHANGELOG entry summarising all Phase 8 sub-phases
- Updates README, ROADMAP, CLAUDE.md, TEST_COVERAGE.md, BENCHMARKS.md, llms.txt, CONTRIBUTING.md
- Writes per-platform READMEs: `@minigraf/browser`, Node.js, Python, C FFI, Java/JVM
- Deletes stale duplicate `pkg/README.md`
- Closes #133

## Wiki updates

Wiki changes (`.wiki/`) are in a separate commit/push after this PR merges — see Tasks 12–15 of the implementation plan.

## Pre-merge checklist

- [ ] `cargo check --workspace` green
- [ ] `cargo test --workspace` — 795 tests (788 passing + 7 ignored)
- [ ] `cargo doc --workspace --no-deps` — no errors
- [ ] No stale `pkg/`, `@minigraf/core`, or `IN PROGRESS` grep hits in changed files
- [ ] Package.swift checksum placeholder noted — will be updated after CI produces the v1.0.0 xcframework artifact

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Task 12: Wiki — Architecture.md

**Files:**
- Modify: `.wiki/Architecture.md`

All wiki tasks operate in the `.wiki/` directory, which is a separate git repo.

- [ ] **Step 1: Update file format section title**

Find:
```markdown
## File Format (v6)
```
Replace with:
```markdown
## File Format (v7)
```

- [ ] **Step 2: Update header byte layout**

Find the line inside the header table:
```
  bytes  4.. 8   version u32 LE (currently 6)
```
Replace with:
```
  bytes  4.. 8   version u32 LE (currently 7)
```

Find:
```
  bytes 72..80   fact_page_count u64 LE  (new in v6)
```
Replace with:
```
  bytes 72..80   fact_page_count u64 LE  (new in v6, retained in v7)
  bytes 80..84   header_checksum u32 LE  (new in v7 — CRC32 of header bytes 0..80)
```

Also update the section description line from `Page 0: FileHeader (80 bytes)` to `Page 0: FileHeader (84 bytes)`. Verify the exact byte layout from `src/storage/mod.rs` `FileHeader` struct before committing — the above is based on the CLAUDE.md description that v7 is 84 bytes; the source is authoritative.

- [ ] **Step 3: Update migration note**

Find:
```markdown
**Migration**: `from_bytes` auto-migrates v1/v2/v3/v4/v5 headers on open.
```
Replace with:
```markdown
**Migration**: `from_bytes` auto-migrates v1/v2/v3/v4/v5/v6 headers on open. v6 databases migrate to v7 on first checkpoint (header_checksum field added).
```

- [ ] **Step 4: Update module tree version references**

Find in the module tree:
```
    ├── mod.rs                  — StorageBackend trait, FileHeader v6, CommittedFactReader / CommittedIndexReader traits
```
Replace with:
```
    ├── mod.rs                  — StorageBackend trait, FileHeader v7, CommittedFactReader / CommittedIndexReader traits
```

Find:
```
    ├── persistent_facts.rs     — PersistentFactStorage: v6 save/load, CommittedFactLoaderImpl
```
Replace with:
```
    ├── persistent_facts.rs     — PersistentFactStorage: v7 save/load, auto-migration v1–v6→v7, CommittedFactLoaderImpl
```

- [ ] **Step 5: Add workspace crates section after the src/ tree**

Find the line immediately after the closing of the `src/` tree (the line `└── storage/` sub-tree end), then after the closing ` ``` ` of the code block, insert:

```markdown
### Workspace crates (cross-platform, Phase 8)

```
minigraf-ffi/src/lib.rs    — UniFFI bindings: MiniGrafDb, MiniGrafError (Android, iOS, Python, Java)
minigraf-c/src/lib.rs      — C FFI: cdylib + staticlib; minigraf.h via cbindgen
minigraf-node/src/lib.rs   — Node.js bindings via napi-rs: MiniGrafDb class
minigraf-wasm/             — wasm-pack output: @minigraf/browser npm package (IndexedDB-backed)
```
```

- [ ] **Step 6: Add Browser thread safety note**

Find in the Thread Safety section:
```markdown
- Page cache uses read-lock on hits, write-lock only on misses — minimises contention for read-heavy workloads
```
After that bullet, add:
```markdown
- `BrowserDb` (`wasm32-unknown-unknown`) runs single-threaded — all `Arc`/`RwLock`/`Mutex` calls compile as single-threaded stubs under the `browser` feature; no WASM thread support
```

- [ ] **Step 7: Commit wiki Architecture.md**

```bash
cd .wiki
git add Architecture.md
git commit -m "docs(wiki): Architecture — file format v6→v7, workspace crates, browser thread safety"
```

---

## Task 13: Wiki — Comparison.md + Home.md

**Files:**
- Modify: `.wiki/Comparison.md`
- Modify: `.wiki/Home.md`

- [ ] **Step 1: Update Comparison.md WASM row**

Find:
```markdown
| **WASM Ready** | ✅ Phase 8.1a/b complete (v0.20.0) | ❌ No | ⚠️ Limited | ❌ No | ✅ Yes |
```
Replace with:
```markdown
| **WASM Ready** | ✅ v1.0.0 (browser + WASI + mobile + 6 language targets) | ❌ No | ⚠️ Limited | ❌ No | ✅ Yes |
```

- [ ] **Step 2: Add Platform support row to Comparison.md feature matrix**

Find the last row of the feature matrix (the `| **WASM Ready** |` line you just updated). Insert the following row immediately after it:

```markdown
| **Platform support** | Native, WASM, WASI, Android, iOS, Python, Node.js, Java, C | JVM only | Native, WASM (limited) | JVM only | Native, WASM |
```

- [ ] **Step 3: Update Home.md Architecture entry**

Find:
```markdown
- **[Architecture](Architecture)** — Module structure, EAV data model, file format (v6), WAL layout, and storage internals
```
Replace with:
```markdown
- **[Architecture](Architecture)** — Module structure, EAV data model, file format (v7), WAL layout, storage internals, and cross-platform workspace crates
```

- [ ] **Step 4: Add Packages section to Home.md**

Find the `## Quick links` section header. Insert the following block immediately before it:

```markdown
## Packages

| Platform | Package |
|---|---|
| Rust | [crates.io/crates/minigraf](https://crates.io/crates/minigraf) |
| Browser WASM | [@minigraf/browser on npm](https://www.npmjs.com/package/@minigraf/browser) |
| Node.js | [minigraf on npm](https://www.npmjs.com/package/minigraf) |
| Python | [minigraf on PyPI](https://pypi.org/project/minigraf) |
| Java/JVM | [minigraf-jvm on Maven Central](https://central.sonatype.com/artifact/io.github.adityamukho/minigraf-jvm) |
| Android, iOS, C, WASI | [GitHub Releases](https://github.com/project-minigraf/minigraf/releases) |

```

- [ ] **Step 5: Commit wiki Comparison.md + Home.md**

```bash
cd .wiki
git add Comparison.md Home.md
git commit -m "docs(wiki): Comparison — platform support row, WASM v1.0.0; Home — file format v7, Packages section"
```

---

## Task 14: Wiki — Use-Cases.md

**Files:**
- Modify: `.wiki/Use-Cases.md`

- [ ] **Step 1: Fix @minigraf/core → @minigraf/browser (intro text)**

Find (in the WASM/Browser section):
```markdown
Published to npm as `@minigraf/core`.
```
Replace with:
```markdown
Published to npm as `@minigraf/browser`.
```

- [ ] **Step 2: Fix build output path**

Find:
```markdown
# Output: pkg/ — contains minigraf_bg.wasm, minigraf.js, minigraf.d.ts
```
Replace with:
```markdown
# Output: minigraf-wasm/ — contains minigraf_bg.wasm, minigraf.js, minigraf.d.ts
```

- [ ] **Step 3: Fix JS import path**

Find:
```markdown
import init, { BrowserDb } from './pkg/minigraf.js';
```
Replace with:
```markdown
import init, { BrowserDb } from './minigraf-wasm/minigraf.js';
```

- [ ] **Step 4: Fix the Constraints box**

Find:
```markdown
- **npm package**: `@minigraf/core` release planned for Phase 8.2.
```
Replace with:
```markdown
- **Install**: `npm install @minigraf/browser`
```

- [ ] **Step 5: Update Android dependency version**

Find:
```kotlin
    implementation("io.github.adityamukho:minigraf-android:0.21.1")
```
Replace with:
```kotlin
    implementation("io.github.adityamukho:minigraf-android:1.0.0")
```

- [ ] **Step 6: Update iOS SPM version (two occurrences)**

Find (first occurrence, in the `Package.swift` snippet):
```swift
    .package(url: "https://github.com/project-minigraf/minigraf", from: "0.21.0"),
```
Replace with:
```swift
    .package(url: "https://github.com/project-minigraf/minigraf", from: "1.0.0"),
```

Find (second occurrence, in the Xcode "Select Up to Next Major Version" guide):
```
Select **Up to Next Major Version** from `0.21.0`.
```
Replace with:
```
Select **Up to Next Major Version** from `1.0.0`.
```

- [ ] **Step 7: Add four new language-binding sections**

Find the end of the existing WASM/Browser section (the last line before the next top-level `---` separator after the WASM content). Insert the following four sections immediately before that `---`:

```markdown
---

## Python

Minigraf for Python ships as `minigraf` on PyPI with pre-built wheels for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No Rust toolchain required.

```sh
pip install minigraf
```

```python
from minigraf import MiniGrafDb

db = MiniGrafDb.open("myapp.graph")
db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])')
result = db.execute('(query [:find ?name :where [?e :person/name ?name]])')
# '{"variables":["?name"],"results":[["Alice"]]}'

db.checkpoint()
```

See [`minigraf-ffi/python/README.md`](https://github.com/project-minigraf/minigraf/blob/main/minigraf-ffi/python/README.md) for the full API reference.

---

## Node.js / TypeScript

Minigraf for Node.js ships as `minigraf` on npm — a native addon (not WASM) with pre-built binaries. No build step required.

```sh
npm install minigraf
```

```typescript
import { MiniGrafDb } from 'minigraf';

const db = new MiniGrafDb('myapp.graph');
db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])');
const result = db.execute('(query [:find ?name :where [?e :person/name ?name]])');
// '{"variables":["?name"],"results":[["Alice"]]}'

db.checkpoint();
```

See [`minigraf-node/README.md`](https://github.com/project-minigraf/minigraf/blob/main/minigraf-node/README.md) for the full API reference.

---

## Java / JVM

Minigraf for Java and Kotlin ships as `io.github.adityamukho:minigraf-jvm` on Maven Central — a fat JAR with embedded native libraries. No Rust toolchain required.

```kotlin
// build.gradle.kts
dependencies {
    implementation("io.github.adityamukho:minigraf-jvm:1.0.0")
}
```

```kotlin
import io.github.adityamukho.minigraf.MiniGrafDb

val db = MiniGrafDb.open("/path/to/myapp.graph")
db.execute("""(transact [[:alice :person/name "Alice"]])""")
val result = db.execute("""(query [:find ?name :where [?e :person/name ?name]])""")
```

See [`minigraf-ffi/java/README.md`](https://github.com/project-minigraf/minigraf/blob/main/minigraf-ffi/java/README.md) for the full API reference including Maven coordinates and threading notes.

---

## C FFI

Minigraf for C ships as platform tarballs on GitHub Releases — a stable C header (`minigraf.h`) and prebuilt shared and static libraries. Suitable for any language with a C FFI.

Download from [GitHub Releases](https://github.com/project-minigraf/minigraf/releases):

```sh
# Linux x86_64
curl -L https://github.com/project-minigraf/minigraf/releases/download/v1.0.0/minigraf-c-v1.0.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
```

```c
#include "minigraf.h"

MiniGrafDb *db = minigraf_open("myapp.graph", NULL);
char *result = minigraf_execute(
    db, "(transact [[:alice :person/name \"Alice\"]])", NULL
);
minigraf_string_free(result);
minigraf_close(db);
```

Memory contract mirrors SQLite: `minigraf_execute` returns a heap-allocated string; call `minigraf_string_free` to release it. See [`minigraf-c/README.md`](https://github.com/project-minigraf/minigraf/blob/main/minigraf-c/README.md) for the full API.
```

- [ ] **Step 8: Commit wiki Use-Cases.md**

```bash
cd .wiki
git add Use-Cases.md
git commit -m "docs(wiki): Use-Cases — fix pkg/@minigraf/core refs, version bumps, add Python/Node.js/Java/C sections"
```

---

## Task 15: Wiki push + close issue

- [ ] **Step 1: Push wiki**

```bash
cd .wiki
git push
```

Expected: push succeeds; GitHub wiki is updated.

- [ ] **Step 2: Verify main PR is merged and CI is green**

```bash
gh pr view --repo project-minigraf/minigraf
```

Wait until the PR is merged by the maintainer before proceeding.

- [ ] **Step 3: Close issue #133 with a comment**

```bash
gh issue comment 133 --repo project-minigraf/minigraf \
  --body "All doc-sync tasks complete. Wiki updated and pushed. Ready for tag + publish."
```

---

## Addendum: @minigraf/wasi (PR #222, merged 2026-05-02)

PR #222 (closes #178) adds `minigraf-wasi/` — the `@minigraf/wasi` npm package (ESM loader + TypeScript declarations for Node.js WASI consumers) and a `publish-npm-wasi` CI job in `wasm-release.yml`. `minigraf-wasi/package.json` stays at `0.0.0`; CI stamps the tag version at publish time (same pattern as `@minigraf/browser`). A basic `minigraf-wasi/README.md` was added by the PR.

Incorporate the following additions into the relevant tasks when executing:

### Task 3 (CHANGELOG) — add to "Also in this release" bullets:
```
- `@minigraf/wasi` published to npm on every tagged release (issue #178) — WASI binary packaged for Node.js WASI consumers
```

### Task 4 (README) — two changes:
**Step 4** — update the WASI row in the platform support table:
```markdown
| WASI | `@minigraf/wasi` on npm, `.wasm` on GitHub Releases | `npm install @minigraf/wasi` |
```

**Step 6** — in the "For WASM / Browser" replacement text, change the WASI sentence from:
```
WASI build (`wasm32-wasip1`) available as a GitHub Releases artifact (Wasmtime / Wasmer).
```
to:
```
WASI build (`wasm32-wasip1`) available as [`@minigraf/wasi`](https://www.npmjs.com/package/@minigraf/wasi) on npm and as a GitHub Releases artifact (Wasmtime / Wasmer).
```

### Task 5 (ROADMAP) — one change:
**Step 4** — in the v1.0.0 entry, update the WASI line to:
```
- WASI (`wasm32-wasip1`, Wasmtime/Wasmer CI) + `@minigraf/wasi` npm package ✅ v1.0.0
```

### Task 7 (llms.txt) — three changes:
1. In the maturity paragraph, change `WASI (\`wasm32-wasip1\`)` to `WASI (\`wasm32-wasip1\`, \`@minigraf/wasi\` npm)`.
2. In the source layout section, add after the `minigraf-wasm/` entry:
```
- `minigraf-wasi/` — `@minigraf/wasi` npm package: ESM loader, TypeScript declarations, `minigraf-wasi.wasm`
```
3. In the Links section, add after the `@minigraf/browser on npm` line:
```
- [@minigraf/wasi on npm](https://www.npmjs.com/package/@minigraf/wasi) — WASI / Node.js
```

### Task 13 (Wiki Home.md) — one change:
**Step 4** — in the Packages table, split the last row:
Replace:
```markdown
| Android, iOS, C, WASI | [GitHub Releases](https://github.com/project-minigraf/minigraf/releases) |
```
with:
```markdown
| WASI (Node.js) | [@minigraf/wasi on npm](https://www.npmjs.com/package/@minigraf/wasi) |
| Android, iOS, C, WASI binary | [GitHub Releases](https://github.com/project-minigraf/minigraf/releases) |
```

### Task 14 (Wiki Use-Cases.md) — one addition:
After the existing WASI section (Wasmtime/Wasmer usage), add a subsection:

```markdown
### Node.js

For Node.js consumers, the `@minigraf/wasi` npm package provides an ESM loader:

```sh
npm install @minigraf/wasi
```

```js
import { WASI } from "node:wasi";
import { startMinigrafWasi } from "@minigraf/wasi";

const wasi = new WASI({
  version: "preview1",
  args: ["minigraf"],
  env: process.env,
  preopens: { "/tmp": "/tmp" },
});

await startMinigrafWasi(wasi);
```

Use `MINIGRAF_WASI_WASM_PATH` or the `wasmPath` option to point the loader at an alternate `.wasm` file.
```

---

## Self-review notes

- **Task 2, Step 8**: Package.swift checksum is a placeholder — this must be updated after CI produces the `MinigrafKit-v1.0.0.xcframework.zip` artifact. This is a deliberate two-step: the PR merges with the placeholder, then a follow-up commit updates the checksum once the release tag is pushed and CI completes.
- **Task 9, Step 1**: `diff` before deleting — confirm both files are truly identical or that `pkg/README.md` has no unique content worth preserving.
- **Task 12, Step 2**: The v7 header byte layout (84 bytes, `header_checksum` at bytes 80..84) is derived from CLAUDE.md. Verify against `src/storage/mod.rs` before committing — the source struct is authoritative.
- **Task 14, Step 7**: Insert the four new sections before the *last* `---` separator in the file, not before an interior one. Read the end of `Use-Cases.md` to confirm the exact insertion point.
