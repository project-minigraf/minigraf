# Phase 8.2: Mobile Bindings Design

**Date**: 2026-04-19
**Issue**: [project-minigraf/minigraf#131](https://github.com/project-minigraf/minigraf/issues/131)
**Status**: Approved

---

## Goal

Ship Minigraf as a drop-in native library for Android (Kotlin/Java) and iOS (Swift), with pre-built artifacts published to GitHub Releases and GitHub Packages on every version tag. Mobile developers should not need to touch Rust.

---

## Approach

UniFFI (Mozilla, proc-macro approach) with a `minigraf-ffi` workspace crate. Chosen over manual JNI/C FFI (too much boilerplate) and Diplomat (less mature, incompatible with Phase 8.3 Python reuse).

---

## Section 1: Repository Structure

### Workspace conversion

The root `Cargo.toml` gains a `[workspace]` table. The existing `minigraf` crate remains a workspace member at the root — no files move, no imports change.

```
minigraf/                        ← repo root
  Cargo.toml                     ← workspace root [workspace] + [package] for minigraf
  Package.swift                  ← SPM manifest (must be at root for SPM resolution)
  src/                           ← core library, unchanged
  minigraf-ffi/
    Cargo.toml                   ← publish = false
    src/
      lib.rs                     ← #[uniffi::export] thin wrappers
      uniffi_bindgen.rs          ← [[bin]] entry point for uniffi-bindgen CLI
    android/                     ← Gradle project for .aar assembly
      build.gradle.kts
      settings.gradle.kts
      src/main/
        AndroidManifest.xml
        java/                    ← uniffi-bindgen Kotlin output (written by CI)
      jniLibs/                   ← cargo-ndk .so output (written by CI)
```

iOS has no parallel folder — `.xcframework` is assembled directly from compiled `.a` files in CI; no Xcode project required.

### `minigraf-ffi/Cargo.toml` key fields

```toml
[package]
name = "minigraf-ffi"
version = "0.21.0"           # versioned in lockstep with minigraf
publish = false

[lib]
crate-type = ["cdylib", "staticlib"]   # .so for Android, .a for iOS

[[bin]]
name = "uniffi-bindgen"
path = "src/uniffi_bindgen.rs"

[dependencies]
minigraf = { path = "..", features = ["serde_json"] }
uniffi = { version = "<pinned at implementation time>" }
thiserror = { version = "<pinned at implementation time>" }
```

`serde_json` is enabled on the `minigraf` dependency — it only bloats the FFI compilation, not the native `minigraf` binary published to crates.io.

### Impact on existing tooling

- `cargo test` at the root runs all `minigraf` tests unchanged.
- `cargo publish` in CI gets `-p minigraf` to scope publication to the core crate only.
- `cargo-dist` only sees `minigraf` as publishable (`publish = false` on `minigraf-ffi`).

---

## Section 2: `minigraf-ffi` API

### `src/lib.rs`

Thin wrappers only — no business logic. All heavy lifting is delegated to the `minigraf` core.

```rust
#[derive(uniffi::Object)]
pub struct MiniGrafDb {
    inner: Arc<Mutex<minigraf::Minigraf>>,
}

#[uniffi::export]
impl MiniGrafDb {
    /// Open or create a file-backed database at the given path.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, MiniGrafError>;

    /// Open an in-memory database (no persistence).
    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Arc<Self>, MiniGrafError>;

    /// Execute a Datalog string (transact, query, rule, retract).
    /// Returns the result serialized as a JSON string.
    /// Callers should JSON.parse() / json.loads() the result.
    pub fn execute(&self, datalog: String) -> Result<String, MiniGrafError>;

    /// Flush WAL and compact to the main file.
    /// Always call before releasing the handle to avoid WAL data loss.
    pub fn checkpoint(&self) -> Result<(), MiniGrafError>;
}
```

`execute()` calls `inner.lock().execute(&datalog)` then `serde_json::to_string(&result)`.

### Error type

```rust
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MiniGrafError {
    #[error("storage error: {msg}")]  Storage { msg: String },
    #[error("query error: {msg}")]    Query   { msg: String },
    #[error("parse error: {msg}")]    Parse   { msg: String },
    #[error("unknown error: {msg}")]  Other   { msg: String },
}
```

`anyhow::Error` from the core is mapped to `MiniGrafError` by inspecting the error chain for known patterns (storage / query / parse), falling back to `Other`.

### `src/uniffi_bindgen.rs`

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

Standard UniFFI pattern. Used in CI as:
```bash
cargo run -p minigraf-ffi --bin uniffi-bindgen -- generate \
  --library target/.../libminigraf_ffi.so --language kotlin --out-dir ...
```

### Intentionally excluded from 8.2 FFI API

The following are deferred — complex to represent safely over UniFFI and not needed to cover the basic use case:

- `begin_write` / `WriteTransaction` — explicit transaction handles; `execute()` covers all cases via implicit transactions
- `register_aggregate` / `register_predicate` — UDFs require passing closures over FFI (not supported by UniFFI)
- `prepare()` / `PreparedQuery` — bind-slot substitution over FFI needs further design

**Roadmap note**: UDF registration and prepared query support in the mobile FFI are post-1.0 items. Add to ROADMAP.md Phase 9 or later when the basic API is proven stable.

---

## Section 3: CI Workflows

### PR validation (every push)

A lightweight `cargo check -p minigraf-ffi` job runs on every PR. No cross-compilation, no Gradle. Catches proc-macro and type errors in the FFI crate early. Fast and cheap.

### `mobile.yml` (new, tag-only for artifact jobs)

Triggers on: same tag pattern as `release.yml` (`**[0-9]+.[0-9]+.[0-9]+*`) + `workflow_dispatch`.

**`mobile-android`** (`ubuntu-latest`, tag-only for full build):
1. Install Android NDK + `cargo-ndk`
2. Cross-compile `minigraf-ffi` for `arm64-v8a`, `armeabi-v7a`, `x86_64` → `android/jniLibs/`
3. Run `uniffi-bindgen` to generate Kotlin bindings → `android/src/main/java/`
4. `cd minigraf-ffi/android && ./gradlew assembleRelease` → `.aar`
5. Upload artifact

**`mobile-ios`** (`macos-latest`, tag-only):
1. Build `aarch64-apple-ios` and `aarch64-apple-ios-sim` static libs
2. Run `uniffi-bindgen` to generate Swift bindings
3. `xcodebuild -create-xcframework` to assemble `MinigrafKit.xcframework`
4. Zip the `.xcframework`
5. Upload artifact

**`release-upload-mobile`** (`ubuntu-latest`, tag-only, needs both build jobs):
1. Download artifacts
2. Poll for the GitHub Release to exist (retry loop, max 5 min, 15s intervals):
   ```bash
   for i in $(seq 1 20); do
     gh release view "$TAG" && break
     echo "Waiting for release..."
     sleep 15
   done
   ```
3. `gh release upload $TAG minigraf-android.aar MinigrafKit.xcframework.zip`
4. Compute `.xcframework.zip` SHA256 checksum (`shasum -a 256`)
5. Update `Package.swift` with the new artifact URL + checksum, commit to `main`, then force-update the release tag to point to the new commit via `gh api repos/{owner}/{repo}/git/refs/tags/{tag} -X PATCH -f sha={new_sha} -f force=true`. The artifact URL is deterministic (`releases/download/vX.Y.Z/MinigrafKit-vX.Y.Z.xcframework.zip`) so this is the only unknown resolved post-build.
6. Publish `.aar` to GitHub Packages via `cd minigraf-ffi/android && ./gradlew publishReleasePublicationToGitHubPackagesRepository`

### `wasm-release.yml` (new, tag-only)

Extracts `build-wasm-wasi` and `build-wasm-browser` from `release.yml` into a standalone workflow. Adds a `release-upload-wasm` job with the same retry-poll pattern to upload WASM artifacts to the GitHub Release.

Trigger: same tag pattern + `workflow_dispatch`.

### `release.yml` (modified)

Remove `build-wasm-wasi`, `build-wasm-browser`, and their references from the `host` job's `needs` list. The `host` job reverts to waiting only on `build-local-artifacts` and `build-global-artifacts` — cargo-dist's native concern. Future `dist generate` regenerations are now safe.

---

## Section 4: Release Artifacts & Distribution

### Artifact naming (versioned in lockstep with `minigraf`)

```
minigraf-android-v0.21.0.aar
MinigrafKit-v0.21.0.xcframework.zip
```

### Android `.aar`

The `android/` Gradle project uses `minSdk 24` (Android 7.0; ~3% of active devices below this as of 2026). Can be lowered if user demand requires it.

GitHub Packages coordinates:
```
implementation("io.github.adityamukho:minigraf-android:0.21.0")
```
Consumers must add the GitHub Packages Maven repo + a read-scoped PAT to their `settings.gradle.kts`. Accepted friction for now; Maven Central deferred to a later phase.

### iOS `.xcframework` + Swift Package Manager

`Package.swift` at the repo root declares a binary target pointing to the `.xcframework.zip` on GitHub Releases. The checksum is updated automatically by the `release-upload-mobile` CI job after each release.

```
# In Xcode: File > Add Package Dependencies
https://github.com/project-minigraf/minigraf
```

---

## Section 5: Lifecycle Safety & Testing

### Lifecycle contract

`MiniGrafDb` is wrapped in `Arc` — UniFFI handles reference counting across the FFI boundary. The documented contract for callers:

- **Always call `checkpoint()` before releasing the handle** to avoid WAL data loss.
- Kotlin: wrap in a `use` block or call `checkpoint()` in `finally`.
- Swift: call `checkpoint()` in `defer` before the variable goes out of scope.
- `Arc<Mutex<Minigraf>>` ensures thread safety if the handle is shared across threads.

### Test plan

| Layer | What | How | When |
|---|---|---|---|
| FFI compilation | `minigraf-ffi` compiles, proc-macros parse | `cargo check -p minigraf-ffi` | Every PR |
| FFI correctness | `open_in_memory`, `execute`, `checkpoint` round-trip | `cargo test -p minigraf-ffi` (native) | Every PR |
| Android integration | Transact + query in Kotlin on emulator | Android emulator `x86_64` in CI | Tags |
| iOS integration | Transact + query in Swift | Manual (macOS CI is tag-only, expensive) | Tags |
| Core regression | All existing tests pass | `cargo test` | Every PR |

`cargo test -p minigraf-ffi` runs natively — no cross-compilation needed. Tests call the Rust FFI wrappers directly, catching logic errors in the thin wrapper layer without requiring a device or emulator.

---

## Out of Scope for 8.2

- Maven Central publishing (deferred; GitHub Packages sufficient for now)
- CocoaPods support (SPM is the modern standard)
- `begin_write` / `WriteTransaction` over FFI
- UDF registration over FFI (post-1.0)
- Prepared query (`$slot`) over FFI (post-1.0)
- Node.js / Python / C bindings (Phase 8.3; will reuse this UniFFI work)
