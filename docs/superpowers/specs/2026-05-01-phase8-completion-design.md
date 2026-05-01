# Design: Phase 8 Completion — v1.0.0 Release

**Date**: 2026-05-01
**Status**: Approved
**Issue**: #133
**Target version**: `1.0.0`

## Overview

Phase 8 (Cross-Platform Expansion) is feature-complete: Browser WASM (8.1a), WASI (8.1b),
Android/iOS mobile bindings (8.2), Python (8.3a), Java/JVM (8.3b), C FFI (8.3c), and Node.js
(8.3d) have all shipped. This spec covers the docs-sync, version bump, per-platform READMEs,
and wiki updates required to close Phase 8 and publish v1.0.0.

All changes land in a single PR on a dedicated worktree. Nothing is split across multiple PRs —
phase completion is one atomic event.

---

## 1. Version Bumps

Bump every versioned manifest to `1.0.0`. The `minigraf-wasm/package.json` version stays at
`0.0.0` — the CI stamps the real version from the Git tag at publish time (per the design in
`docs/superpowers/specs/2026-05-01-wasm-npm-publish-design.md`).

| File | Current | Target | Notes |
|---|---|---|---|
| `Cargo.toml` | `0.25.0` | `1.0.0` | |
| `minigraf-ffi/Cargo.toml` | `0.23.0` | `1.0.0` | |
| `minigraf-c/Cargo.toml` | `0.24.0` | `1.0.0` | |
| `minigraf-node/Cargo.toml` | `0.25.0` | `1.0.0` | |
| `minigraf-node/package.json` | `0.25.0` | `1.0.0` | |
| `minigraf-ffi/python/pyproject.toml` | `0.22.0` | `1.0.0` | |
| `minigraf-ffi/java/build.gradle.kts` | `0.23.0` | `1.0.0` | hardcoded fallback (`?: "0.23.0"`); CI uses `$RELEASE_VERSION` env var |
| `Package.swift` (tag URL) | `v0.20.1` | `v1.0.0` | |
| `minigraf-wasm/package.json` | `0.0.0` | unchanged | CI stamps version from tag at publish time |
| `minigraf-ffi/android/build.gradle.kts` | n/a | unchanged | fully env-var driven (`$VERSION`); no hardcoded version |

After all bumps: run `cargo check --workspace` to confirm `Cargo.lock` is consistent.

---

## 2. `CHANGELOG.md`

Prepend a new `## v1.0.0 — Phase 8 Complete` entry at the top of the file. Content:

- Mark this as the **1.0.0 milestone**: file format stability guaranteed from this release;
  public Rust API committed to semver.
- List all Phase 8 sub-phases with their shipped versions:
  - 8.1a: Browser WASM — `BrowserDb`, `IndexedDbBackend`, `@minigraf/browser` npm (v0.20.0)
  - 8.1b: WASI — `wasm32-wasip1`, Wasmtime/Wasmer CI (v0.20.0)
  - 8.2: Mobile — Android `.aar` (GitHub Packages), iOS `.xcframework` (SPM) via UniFFI (v0.21.0)
  - 8.3a: Python — `minigraf` on PyPI, pre-built wheels (v0.22.0)
  - 8.3b: Java/JVM — `io.github.adityamukho:minigraf-jvm` on Maven Central (v0.23.0)
  - 8.3c: C FFI — `minigraf.h` + platform tarballs on GitHub Releases (v0.24.0)
  - 8.3d: Node.js — `minigraf` on npm, pre-built `.node` binaries (v0.25.0)
- Note 795 tests passing.
- Note: `@minigraf/browser` npm published; `pkg/` renamed to `minigraf-wasm/`, `swift/` renamed
  to `minigraf-swift/` (issue #179).

---

## 3. `README.md`

### Phase badge
`phase-8.3c%20complete-blue` → `v1.0.0-blue` (or `release-v1.0.0-blue` for clarity).

### Installation section
Change `minigraf = "0.21"` → `minigraf = "1.0"`.

### Quick-start shell commands
Update test count: `cargo test         # run 780 tests` → `# run 795 tests`.

### Platform support — add below the existing feature matrix

Add a second table titled "Platform support":

| Platform | Package | Install |
|---|---|---|
| Rust (native) | `minigraf` on crates.io | `cargo add minigraf` |
| Browser WASM | `@minigraf/browser` on npm | `npm install @minigraf/browser` |
| Node.js | `minigraf` on npm | `npm install minigraf` |
| Python | `minigraf` on PyPI | `pip install minigraf` |
| Java/JVM | `io.github.adityamukho:minigraf-jvm` on Maven Central | see wiki |
| Android | `.aar` on GitHub Packages | see wiki |
| iOS / macOS | `.xcframework` via SPM | see wiki |
| C / FFI | header + tarball on GitHub Releases | see wiki |
| WASI | `.wasm` binary on GitHub Releases | see wiki |

### Platform sections (replace existing stubs)
- "For AI Agents" — keep as-is (already good)
- "For Mobile Apps" — update: remove "Phase 8 will ship..." forward-reference; replace with "Ships as
  Android `.aar` (GitHub Packages) and iOS `.xcframework` (Swift Package Manager). See
  [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases)."
- "For WASM / Browser" — update: remove "planned for Phase 8.2" forward-reference; replace with
  "Published as `@minigraf/browser` on npm. WASI build available as a GitHub Releases artifact.
  See [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases)."
- Add "For Python / Node.js / Java / C" — one paragraph: "Language bindings ship as `minigraf` on
  PyPI, `minigraf` on npm (Node.js), `io.github.adityamukho:minigraf-jvm` on Maven Central,
  and a C header + prebuilt shared library on GitHub Releases. See
  [Use Cases wiki](https://github.com/project-minigraf/minigraf/wiki/Use-Cases)."

### Scope section
Add to the "Minigraf runs as" list:
- `✅ Browser WASM — \`@minigraf/browser\` (IndexedDB-backed)`
- `✅ Server-side WASM — \`wasm32-wasip1\` / WASI`
- `✅ Android, iOS, Python, Node.js, Java, C — via UniFFI / napi-rs / cbindgen`

Remove forward-looking Phase 8 references from the WASM entry.

---

## 4. `ROADMAP.md`

### Phase 8 header
`## Phase 8: Cross-Platform Expansion 🔄 IN PROGRESS` →
`## Phase 8: Cross-Platform Expansion ✅ COMPLETE`

### Phase 8 status line
`**Status**: 🔄 In Progress (Phase 8.1 complete — v0.20.0)` →
`**Status**: ✅ Completed (May 2026) — v1.0.0`

### Phase 8.2 section
Add completion status (currently the section body is present but has no status badge):
`### 8.2 Mobile Bindings` → `### 8.2 Mobile Bindings ✅ COMPLETE`
Add: `**Status**: ✅ Completed (April 2026) — v0.21.0`

### Phase 8 status line at page bottom
The version table near the bottom currently reads:
`🔄 Phase 8: In progress (Cross-platform — WASM ✅, mobile, language bindings → **v1.0.0**)`
Update to: `✅ Phase 8: Complete — v1.0.0 (May 2026)`

Also update the summary line:
`🔄 Phase 8.2: In progress — Mobile bindings ...` → `✅ Phase 8.2: Complete (v0.21.0)`

### Version table entry for v1.0.0
The entry `### v1.0.0 - 🎯 Phase 8 (Cross-platform)` exists but is forward-looking.
Replace body with completed summary: all 8 sub-phase deliverables, 795 tests, stability promise
now in effect.

### Phase 9 header
Change `## Phase 9: Ecosystem & Tooling 🎯 FUTURE` → `## Phase 9: Ecosystem & Tooling 🎯 NEXT`
(or keep FUTURE — either is fine; not a blocker).

---

## 5. `CLAUDE.md`

### Test count
`**795 tests passing**` — already correct in the file. Verify and leave.

### "Key Files for the Next Phase" section
Replace the Phase 8 file list with the Phase 9 relevant files:
- `ROADMAP.md` — Phase 9 spec (examples, benchmarks, import scripts)
- Any `examples/` scaffolding for end-to-end agent/mobile/browser demos
- Wiki pages for any new Phase 9 concepts

No other changes to CLAUDE.md are needed.

---

## 6. `TEST_COVERAGE.md`

### Header
Update: `**Last Updated**: Phase 8.3c COMPLETE` → `**Last Updated**: Phase 8 COMPLETE — v1.0.0`

### Add Phase 8.2–8.3d completion sections
After the existing `Phase 8.1 Completion Status` block, add:

**Phase 8.2** (Mobile):
- `minigraf-ffi/src/lib.rs` — UniFFI bindings, `MiniGrafDb`, `MiniGrafError`
- `minigraf-ffi/android/` — Gradle project, Kotlin bindings, JUnit 5 suite
- `mobile.yml` CI — cross-compile, AAR/xcframework assembly and publish

**Phase 8.3a** (Python):
- `minigraf-ffi/python/` — maturin project, `pyproject.toml`, Python test suite
- `python-ci.yml` / `python-release.yml` CI

**Phase 8.3b** (Java/JVM):
- `minigraf-ffi/java/` — Gradle 8.11 project, `NativeLoader.kt`, JUnit 5 `BasicTest.kt`
- `java-ci.yml` / `java-release.yml` CI

**Phase 8.3c** (C FFI):
- `minigraf-c/src/lib.rs` — `cdylib` + `staticlib`, 7 exported functions
- `minigraf-c/include/minigraf.h` — committed stable header (cbindgen-generated)
- `c-ci.yml` / `c-release.yml` CI — header drift check, platform tarballs

**Phase 8.3d** (Node.js):
- `minigraf-node/src/lib.rs` — napi-rs bindings, `MiniGrafDb` class
- `minigraf-node/package.json` — `minigraf` npm package, prebuilt binary config
- `node-ci.yml` / `node-release.yml` CI

---

## 7. `BENCHMARKS.md`

Add a section at the top (or after the summary):

```
## Phase 8 Note

Phase 8 (v0.20.0–v1.0.0) added cross-platform targets: Browser WASM, WASI, Android, iOS,
Python, Node.js, Java, and C bindings. No changes were made to the native query or storage path.
All benchmark numbers below are unchanged from Phase 7 (v0.19.0).
```

---

## 8. `llms.txt`

Full rewrite. Changes from current content:

- **Version**: `0.14.0` → `1.0.0`
- **Maturity blurb**: update test count (617 → 795), remove Phase 7.5 reference, note Phase 8
  completion and that file format stability is now guaranteed (v1.0.0 promise)
- **"Use for" section**: add three new bullets:
  - Browser agent memory (IndexedDB-backed, offline PWA, `@minigraf/browser`)
  - Mobile offline-first apps (Android/iOS via UniFFI)
  - Python/Node.js/Java scripting and server-side embedding
- **"Do not use for" section**: no changes needed
- **API section**: add `PreparedQuery` / `$slot` example (was added in v0.18.0, still missing from
  `llms.txt`)
- **Source layout**: 
  - Update `StorageBackend` and `persistent_facts.rs` references from v6 → v7
  - Add `minigraf-ffi/src/lib.rs` — UniFFI bindings (mobile, Python, Java)
  - Add `minigraf-c/src/lib.rs` — C FFI (`cdylib` + `staticlib`)
  - Add `minigraf-node/src/lib.rs` — Node.js bindings (napi-rs)
  - Add `minigraf-wasm/` — browser WASM package (`@minigraf/browser`)
- **"Single-file storage" paragraph**: `v6` → `v7`; migration note: `v1–v6 → v7`
- **Performance summary**: update header from `v0.13.0` to `v0.19.0` (last version before
  Phase 8; Phase 8 didn't change native path)
- **Links section**: add:
  - crates.io: `https://crates.io/crates/minigraf`
  - npm (`@minigraf/browser`): `https://www.npmjs.com/package/@minigraf/browser`
  - npm (Node.js `minigraf`): `https://www.npmjs.com/package/minigraf`
  - PyPI: `https://pypi.org/project/minigraf`
  - Maven Central: `io.github.adityamukho:minigraf-jvm`
  - Swift Package Index (once indexed)
  - C releases: GitHub Releases page

---

## 9. `CONTRIBUTING.md`

### Pre-Publishing Checklist section
The checklist was a working document for pre-1.0 hygiene and is now fully obsolete. Replace the
entire `## Pre-Publishing Checklist (crates.io)` section with a brief "Release Process" pointer:

```markdown
## Release Process

Releases are managed by the project maintainer. The process is documented in issue #133 and
the `docs/superpowers/specs/` design files. For each release:

1. All changes merged and CI green
2. Version bumped consistently across all manifests
3. `cargo check --workspace` clean
4. Tag pushed — CI publishes to crates.io, PyPI, npm, and Maven Central automatically
```

### "What We Welcome" section
`Cross-platform compatibility fixes (Linux, macOS, Windows, eventually WASM/mobile)` →
`Cross-platform compatibility fixes (Linux, macOS, Windows, WASM, mobile, language bindings)`

### "Measuring Code Coverage" section
Remove the stale "Phase 7.5 target is ≥90%" sentence. Replace with:
`Run branch coverage to check overall project health before submitting a PR.`

---

## 10. Per-Platform READMEs

### `minigraf-wasm/README.md` — rewrite
This is the npm package README for `@minigraf/browser`. Current content is a stale copy of the
root README. Rewrite as a focused browser-WASM README:
- Title: `@minigraf/browser`
- One-paragraph description: embedded bi-temporal graph database for the browser, powered by
  IndexedDB, Datalog queries, time travel
- Install: `npm install @minigraf/browser`
- Quick-start JavaScript snippet (init + `BrowserDb.open` + execute + query)
- Table: response shapes (`transact`/`retract`/`query`/`rule`)
- Export/import portability note (byte-identical to native `.graph`)
- Link to full docs: Use Cases wiki and main repo

### `minigraf-node/README.md` — new
npm package README for the `minigraf` Node.js package:
- Title: `minigraf (Node.js)`
- Description: native Node.js addon, no WASM, pre-built binaries
- Install: `npm install minigraf`
- Quick-start TypeScript/JS snippet (`new MiniGrafDb(path)` / `MiniGrafDb.inMemory()` +
  execute + query)
- Table: response shapes
- Link to Use Cases wiki

### `minigraf-ffi/python/README.md` — new
PyPI package README:
- Title: `minigraf (Python)`
- Description: Python bindings via UniFFI, pre-built wheels
- Install: `pip install minigraf`
- Quick-start Python snippet (`MiniGrafDb.open(path)` / `open_in_memory()` + execute + query)
- Link to Use Cases wiki

### `minigraf-c/README.md` — new
C FFI README:
- Title: `minigraf C bindings`
- Description: C header + prebuilt shared library for any language with a C FFI
- Install: download platform tarball from GitHub Releases
- Quick-start C snippet (open → execute → string_free → close)
- Memory contract note (mirrors SQLite: caller frees execute result via `minigraf_string_free`)
- Link to `include/minigraf.h` and Use Cases wiki

### `minigraf-ffi/java/README.md` — new
Maven README:
- Title: `minigraf-jvm`
- Description: Java/Kotlin desktop bindings, fat JAR with embedded natives
- Install: Gradle snippet `implementation("io.github.adityamukho:minigraf-jvm:1.0.0")`
- Quick-start Kotlin snippet
- Link to Use Cases wiki

---

## 11. Wiki Updates (`.wiki/` repo)

The wiki is a separate git repo. All wiki changes are committed and pushed separately after the
main PR is merged.

### `Architecture.md`

**File Format section title and header**:
`## File Format (v6)` → `## File Format (v7)`
The wiki currently says `FileHeader (80 bytes)` but v7 is 84 bytes — it added a `header_checksum`
field. The implementer must verify the exact v7 byte layout from `src/storage/mod.rs` and update
the full byte-offset table accordingly. Version field value comment: `(currently 6)` → `(currently 7)`.
`fact_page_count` note says "new in v6" — update to "new in v6, retained in v7".
Migration note: `auto-migrates v1/v2/v3/v4/v5` → `auto-migrates v1/v2/v3/v4/v5/v6`.

**Module tree**:
- `mod.rs`: `FileHeader v6` → `FileHeader v7`
- `persistent_facts.rs`: `v6 save/load` → `v7 save/load`
- Add after the `src/` tree a new `Workspace crates` section listing:
  ```
  minigraf-ffi/src/lib.rs    — UniFFI bindings: MiniGrafDb, MiniGrafError (mobile, Python, Java)
  minigraf-c/src/lib.rs      — C FFI: cdylib + staticlib, minigraf.h (cbindgen)
  minigraf-node/src/lib.rs   — Node.js bindings via napi-rs
  minigraf-wasm/             — wasm-pack output: @minigraf/browser npm package
  ```

**Thread Safety section**:
Add a note: `BrowserDb` runs single-threaded in the browser (WASM has no threads); all
Arc/RwLock/Mutex calls compile as single-threaded stubs under `wasm32-unknown-unknown`.

### `Comparison.md`

**Feature matrix**: add a "Platform support" row after the existing rows:

| Feature | Minigraf | XTDB | Cozo | Neo4j | SQLite |
|---|---|---|---|---|---|
| **Platform support** | Native, WASM, WASI, Android, iOS, Python, Node.js, Java, C | JVM only | Native, WASM (limited) | JVM only | Native, WASM |

Also update the WASM row:
`✅ Phase 8.1a/b complete (v0.20.0)` → `✅ v1.0.0 (browser + WASI + mobile + 6 language targets)`

### `Use-Cases.md`

**Browser WASM section — corrections** (all from the `pkg/` → `minigraf-wasm/` and
`@minigraf/core` → `@minigraf/browser` rename in issue #179):
- Section intro: `Published to npm as \`@minigraf/core\`` → `Published to npm as \`@minigraf/browser\``
- Build command: `# Output: pkg/` → `# Output: minigraf-wasm/`
- JS import: `from './pkg/minigraf.js'` → `from './minigraf-wasm/minigraf.js'`
- Constraints box: remove "npm package: `@minigraf/core` release planned for Phase 8.2"
  bullet entirely (it shipped); replace with: "Install: `npm install @minigraf/browser`"

**Android version reference**:
`implementation("io.github.adityamukho:minigraf-android:0.21.1")` → `:1.0.0`

**iOS SPM version**:
`from: "0.21.0"` → `from: "1.0.0"` (two occurrences: `Package.swift` snippet and Xcode guide)

**Add four new language-binding sections** after the existing WASM section.
Each is light: description paragraph + install line + quick-start snippet + link to platform README.

#### Python

```python
from minigraf import MiniGrafDb

db = MiniGrafDb.open("myapp.graph")
db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])')
result = db.execute('(query [:find ?name :where [?e :person/name ?name]])')
# '{"variables":["?name"],"results":[["Alice"]]}'
```

Link: `minigraf-ffi/python/README.md`

#### Node.js / TypeScript

```typescript
import { MiniGrafDb } from 'minigraf';

const db = new MiniGrafDb('myapp.graph');
db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])');
const result = db.execute('(query [:find ?name :where [?e :person/name ?name]])');
// '{"variables":["?name"],"results":[["Alice"]]}'
```

Link: `minigraf-node/README.md`

#### Java / JVM

```kotlin
import io.github.adityamukho.minigraf.MiniGrafDb

val db = MiniGrafDb.open("/path/to/myapp.graph")
db.execute("""(transact [[:alice :person/name "Alice"]])""")
val result = db.execute("""(query [:find ?name :where [?e :person/name ?name]])""")
```

Install: `implementation("io.github.adityamukho:minigraf-jvm:1.0.0")`
Link: `minigraf-ffi/java/README.md`

#### C FFI

```c
#include "minigraf.h"

MiniGrafDb *db = minigraf_open("myapp.graph", NULL);
char *result = minigraf_execute(db, "(transact [[:alice :person/name \"Alice\"]])", NULL);
minigraf_string_free(result);
minigraf_close(db);
```

Install: download `minigraf-c-v1.0.0-<platform>.tar.gz` from GitHub Releases.
Link: `minigraf-c/README.md`

### `Home.md`

- Update Architecture entry: `file format (v6)` → `file format (v7)`
- Add a "Packages" section below the Pages list:
  ```
  ## Packages
  - [crates.io](https://crates.io/crates/minigraf) — Rust
  - [@minigraf/browser](https://www.npmjs.com/package/@minigraf/browser) — Browser WASM
  - [minigraf (npm)](https://www.npmjs.com/package/minigraf) — Node.js
  - [minigraf (PyPI)](https://pypi.org/project/minigraf) — Python
  - [minigraf-jvm](https://central.sonatype.com/artifact/io.github.adityamukho/minigraf-jvm) — Java/JVM
  - GitHub Releases — Android .aar, iOS .xcframework, C header + library, WASI .wasm
  ```

---

## 12. Verification Steps

After all edits are made, before opening the PR:

1. `cargo check --workspace` — Cargo.lock consistent
2. `cargo test --workspace 2>&1 | tail -5` — confirm test count ≥ 795
3. `cargo doc --workspace --no-deps` — no rustdoc errors
4. Grep for `v0.2` across all changed doc files — catch any missed version references
5. Grep for `pkg/` across all changed files — catch any missed path references
6. Grep for `@minigraf/core` across all files — catch any missed package name references
7. Grep for `Phase 8.1 complete` status strings in README/ROADMAP — ensure they're updated

---

## Out of Scope

- `cargo publish` — triggered by CI on tag push; not part of this PR
- GitHub Release creation — triggered by CI on tag push
- GitHub Discussions announcement — authored separately by the maintainer after publish succeeds
- `@minigraf/wasi` npm publish — tracked as a follow-up issue (per the wasm-npm-publish design)
- The ignored `or+neg-cycle` test — known bug, deferred to post-1.0 backlog; no change here
