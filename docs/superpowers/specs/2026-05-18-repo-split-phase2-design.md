# Repo Split Phase 2 — Java, Android, Swift, C, Template

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split `minigraf-java`, `minigraf-android`, `minigraf-swift`, `minigraf-c`, and `minigraf-binding-template` out of the monorepo into independent repos under the `project-minigraf` org; retire `minigraf-ffi`; update cascade to dispatch to all four new binding repos.

---

## Context

Phase 1 of #231 split Python, Node, and WASM into separate repos. Phase 2 covers the remaining bindings. The key learnings from Phase 1:

- Binding repos depend directly on the published `minigraf` core crate, **not** on `minigraf-ffi`
- `minigraf-ffi` cannot be used as a Rust library dependency for building new bindings — UniFFI scaffolding must live in the final cdylib crate. This makes it unsuitable as a published crate
- Each binding repo embeds its own copy of the UniFFI scaffolding source (`src/lib.rs` + `src/uniffi_bindgen.rs`)

`minigraf-ffi` is therefore retired: all published versions yanked on crates.io, directory removed from monorepo.

---

## Repos Created

| Repo | Publishes to | Artifact coordinates |
|---|---|---|
| `minigraf-java` | Maven Central | `io.github.project-minigraf:minigraf-jvm:<version>` |
| `minigraf-android` | Maven Central | `io.github.project-minigraf:minigraf-android:<version>` |
| `minigraf-swift` | GitHub Release + SPM binary target | `MinigrafKit` xcframework zip |
| `minigraf-c` | GitHub Release | `minigraf-c-<version>-<platform>.(tar.gz\|zip)` |
| `minigraf-binding-template` | GitHub template repo (no publish) | — |

---

## Per-Repo Structure

All four binding repos share this skeleton:

```
<repo>/
├── Cargo.toml                  # crate-type cdylib (+ staticlib for C)
│                               # depends on minigraf = "<pinned version>"
├── src/
│   ├── lib.rs                  # UniFFI scaffolding (Java/Android/Swift) or C wrapper (C)
│   └── uniffi_bindgen.rs       # bindgen CLI entry point (Java/Android/Swift only)
├── <lang-tooling>/             # described per-repo below
└── .github/workflows/
    ├── ci.yml                  # runs on every PR and push to main
    └── release.yml             # triggered by repository_dispatch: core-release
```

### minigraf-java

- Lang tooling: `java/` — Gradle project containing generated Kotlin bindings and multi-platform natives in `src/main/resources/natives/{linux/x86_64,linux/aarch64,macos/universal,windows/x86_64}/`
- `ci.yml`: matrix on ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest — `cargo test` + JUnit
- `release.yml`: receive `core-release` dispatch → update `minigraf` pin → commit + tag → build natives on 4-platform matrix → assemble fat JAR → publish via NMCP to Maven Central
- README covers: Maven/Gradle coordinates, quick start snippet, build-from-source instructions, cascade release notes

### minigraf-android

- Lang tooling: `android/` — Gradle project producing an AAR; JNI libs populated by CI via `cargo-ndk`
- `ci.yml`: cross-compile for `arm64-v8a`, `armeabi-v7a`, `x86_64` on ubuntu-latest; run instrumented unit tests
- `release.yml`: receive dispatch → update pin → commit + tag → `cargo-ndk` build for all Android ABIs → assemble AAR → publish to Maven Central via NMCP
- README covers: Gradle dependency snippet, permissions/min-SDK notes, build-from-source instructions

### minigraf-swift

- Lang tooling: `Sources/MinigrafKit/` (generated Swift bindings) + `Package.swift` (SPM manifest with binary target)
- `ci.yml`: macOS-14 only — build for iOS simulator, run Swift tests
- `release.yml`: receive dispatch → update pin → commit + tag → build `aarch64-apple-ios` + `aarch64-apple-ios-sim` → lipo/xcframework → zip + SHA256 → upload to GitHub Release → update `Package.swift` checksum → commit to `swift-releases` branch → tag `swift-v<version>`
- README covers: Xcode SPM integration snippet, platforms supported (iOS 16+), build-from-source instructions

### minigraf-c

- Lang tooling: `include/minigraf.h` (cbindgen-generated, committed) + `cbindgen.toml`
- `ci.yml`: 4-platform matrix — `cargo test` + header drift check (`cbindgen` regenerate + diff)
- `release.yml`: receive dispatch → update pin → commit + tag → build `.so`/`.dylib`/`.dll` on 4-platform matrix → collect artifacts → create GitHub Release → upload archives
- README covers: download links per platform, C usage example, memory contract, build-from-source instructions

### minigraf-binding-template

- Marked as a GitHub template repository
- Contains: `Cargo.toml` (depends on `minigraf`, placeholder version `0.0.0`), `src/lib.rs` (UniFFI scaffolding with inline comments at every hook explaining what to customise), `src/uniffi_bindgen.rs`, `.github/workflows/ci.yml` (cargo test only), `README.md`
- README explains: the Phase 1/2 split pattern, why bindings depend on `minigraf` not `minigraf-ffi`, how to wire up `cascade.yml` dispatch, how to add language tooling

---

## Release Flow (Data Flow)

```
v* tag pushed to monorepo
  └─► release.yml         publishes minigraf core to crates.io
  └─► cascade.yml
        ├─ publishes nothing (minigraf-ffi retired)
        └─ dispatches core-release to:
             minigraf-python, minigraf-node, minigraf-wasm  ← Phase 1
             minigraf-java, minigraf-android,               ← Phase 2
             minigraf-swift, minigraf-c

Each binding repo (on core-release dispatch):
  1. Extract version from client_payload.version (e.g. v1.2.0 → 1.2.0)
  2. Pin minigraf = "1.2.0" in Cargo.toml
  3. Commit + push + tag v1.2.0
  4. Run platform build matrix
  5. Publish artifact
```

**Failure isolation:** dispatch is fire-and-forget. A failure in one binding repo does not affect others. Each can be re-triggered via `workflow_dispatch` once fixed.

---

## Cascade Update (monorepo)

`cascade.yml` `dispatch-bindings` step becomes:

```yaml
for REPO in minigraf-python minigraf-node minigraf-wasm \
            minigraf-java minigraf-android minigraf-swift minigraf-c; do
  echo "Dispatching core-release to project-minigraf/$REPO @ $VERSION"
  gh api repos/project-minigraf/$REPO/dispatches \
    -f event_type=core-release \
    -f "client_payload[version]=$VERSION"
done
```

The `publish-ffi` job is removed entirely (no FFI crate to publish). Its "wait for minigraf to be indexed on crates.io" polling step is moved into `dispatch-bindings` as a pre-flight (so binding repos don't try to `cargo add minigraf@<version>` before crates.io has indexed it). `dispatch-bindings` no longer has a `needs:` dependency on any other job — it polls crates.io itself, then dispatches.

---

## Monorepo Cleanup

**Directories removed:**
- `minigraf-ffi/` (entire directory)
- `minigraf-swift/` (entire directory)
- `minigraf-c/` (entire directory)

**Workflows removed:**
- `.github/workflows/java-ci.yml`
- `.github/workflows/java-release.yml`
- `.github/workflows/mobile.yml`
- `.github/workflows/c-ci.yml`
- `.github/workflows/c-release.yml`
- `.github/workflows/publish-ffi.yml`

**`Cargo.toml` workspace:**
```toml
[workspace]
members = ["."]   # minigraf-ffi and minigraf-c removed
exclude = ["fuzz"]
```

**`Package.swift`** (root): removed — lives in `minigraf-swift` repo.

---

## minigraf-ffi Retirement

1. Yank `minigraf-ffi 1.1.1` and `1.1.2` on crates.io with message:
   > "This crate has been retired. Depend on `minigraf` directly. To build a new language binding, see https://github.com/project-minigraf/minigraf-binding-template"
2. Update `minigraf-ffi` README on crates.io (via a final yanked publish or docs.rs redirect) with the same tombstone notice.

---

## Secrets Required per New Repo

Each new binding repo needs these org-level or repo-level secrets configured in GitHub:

| Secret | Used by |
|---|---|
| `MINIGRAF_RELEASE_TOKEN` | Receive `repository_dispatch` (write:actions scope) |
| `CENTRAL_TOKEN_USERNAME` | Maven Central publish (Java, Android) |
| `CENTRAL_TOKEN_PASSWORD` | Maven Central publish (Java, Android) |
| `GPG_SIGNING_KEY` | Maven Central publish (Java, Android) |
| `GPG_SIGNING_PASSWORD` | Maven Central publish (Java, Android) |

`minigraf-swift` and `minigraf-c` only need `MINIGRAF_RELEASE_TOKEN` (GitHub Release uploads use `GITHUB_TOKEN`).

---

## Testing Strategy

- **Rust layer**: `cargo test` in CI on all supported platforms per repo
- **Java/Android**: JUnit tests in CI matrix; release workflow runs tests as pre-flight
- **Swift**: `swift test` on macOS-14 in CI; xcframework build verified on release
- **C**: `cargo test` + cbindgen header drift check in CI on all 4 platforms
- **Cascade integration**: verified manually after first release that all 7 repos receive dispatch and publish successfully
