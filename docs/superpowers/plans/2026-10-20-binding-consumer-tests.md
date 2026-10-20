# Binding consumer tests — Plan

Spec: `docs/superpowers/specs/2026-10-20-binding-consumer-tests-design.md`.

## Changes to the spec's mechanics

Writing the plan against the real workflows changed three mechanics. The
goals and the user decisions in the spec stand.

1. **No separate `package.yml` call in the release.** `ci.yml` builds the
   artifacts (`dist-*`) and runs the consumer test on them. It takes a
   `version` input (default `0.0.0-ci`; Python `0.0.0.dev0`). The release's
   existing `test` job calls `ci.yml` with the release `sha` and `version`,
   and `publish` downloads `dist-*` from the same run. Java already works
   this way for its natives. The artifacts are built once, before the tag,
   and are both tested and published, so spec §6 holds with one build
   instead of two. The build steps move from `release.yml` into a
   `package.yml` that `ci.yml` calls.
2. **readme-sync compares lines in order, not the whole file.** Kotlin,
   Java and Swift need imports at the top and statements inside a function,
   so a README block can't be the file verbatim. The check passes when
   every non-blank line of the README's first `## Quick start` block
   appears, trimmed of indentation and in the same order, in
   `consumer-test/quickstart.<ext>`. This is weaker than an exact match:
   the consumer can add lines (imports, a `main` wrapper, assertions). But
   it still fails when the README names a symbol, import or call that the
   consumer doesn't use, which was the Java failure.
3. **Python glibc floor in scope.** The spec (§9) left it out, but the fix
   is a build flag: `maturin --zig --compatibility manylinux2014`, the 2.17
   floor Java already uses, plus `pip install ziglang`. This widens the
   spec's scope, under the user's instruction of 2026-10-20 to fix minor
   issues along the way. The sdist stays out of scope. C gets the same
   2.17 floor through `cargo zigbuild`.

## Shared pieces (written once, copied into each repo)

- `ci.yml` in every repo gets three jobs:
  - `package`: calls `package.yml` with `ref` and `version`, and forwards
    `secrets: inherit` on that nested call, because secrets do not pass
    through a nested reusable workflow on their own;
  - `consumer`: needs `package`; runs the repo's runner matrix with
    `run.sh local`;
  - `readme-sync`.

  `release.yml`'s `test` job keeps calling `ci.yml`, now with `version` and
  `secrets: inherit`.

- `consumer-test/readme_sync.py`: implements change 2. It takes the README
  and the consumer file and prints the first missing line. Run as
  `python3 -I`.
- `consumer-test/run.sh` skeleton:
  - `set -euo pipefail`
  - mode `local <dist> <version>` or `registry <version>`
  - `wait_for <cmd> <seconds>` polls every 30 s
  - a fresh temporary directory per run
  - exit 0 only when every contract check passes (spec §4)
- `verify-published.yml` skeleton:
  - `workflow_call` and `workflow_dispatch`, input `version`
  - a matrix of `run.sh registry` legs
  - a final `report` job (`needs: [verify]`, `if: failure()`,
    `issues: write`) that runs
    `gh issue list --label release-broken --search "v<version>"`, then
    comments on the issue it finds or opens one (spec §6)
- The release `prepare` step also stamps the version in the README install
  snippet (Java, Android, Swift, Python), so the install snippet can't go
  stale again. The READMEs currently say 2.0.2, 1.2.0 and 2.0.1.
- Each repo's first PR creates the `release-broken` label
  (`gh label create`).

## Per repo

Each repo gets a `main` PR, then a `v3` PR that is the same change rebased
(the workflows differ only in branch triggers). Each PR is reviewed by Codex
and kept green. For each repo, the PR does the following.

### 1. minigraf-android

- Fixes:
  - apply `org.jetbrains.kotlin.android` in `android/build.gradle.kts`;
  - add `api("net.java.dev.jna:jna:5.14.0@aar")`;
  - build with `cargo ndk … build --release` without `-o`, and copy only
    `libminigraf_ffi.so` into `jniLibs/<abi>/`, so the core crate's
    `libminigraf-<hash>.so` stays out;
  - README version.
- `package.yml`: the cross-compile, bindgen and `assembleRelease` steps;
  `publishToMavenLocal` into `dist/m2`; upload `dist-aar`.
- `consumer-test/`: Gradle app module, `minigraf-android:<v>` from
  `maven { url = uri(<dist>/m2) }` (or Central in registry mode),
  instrumented test with the contract.
- Runner: `ubuntu-latest` with KVM enabled (the udev rule step), running
  `reactivecircus/android-emulator-runner`; matrix `api-level: [24, 34]`,
  `arch: x86_64`, one emulator per leg.
- Release: `build-and-publish` becomes `publish`. It downloads `dist-aar`,
  zips the signed m2 layout (with `.md5`/`.sha1` files) into a bundle, and
  uploads the bundle with the Central Portal publisher API
  (`POST /api/v1/publisher/upload?publishingType=AUTOMATIC`). nmcp publishes
  only from Gradle publications, so it can't upload a prebuilt artifact and
  is dropped. `publish` no longer runs `assembleRelease`. Signing moves into `package.yml` only under
  `version != 0.0.0-ci`, with the secrets passed to `ci.yml` as
  `secrets: inherit`.
- Check before the PR: build the AAR locally, `unzip -l classes.jar` lists
  `uniffi/minigraf_ffi/MiniGrafDb.class`, and the pom lists JNA.

### 2. minigraf-c

- Fixes:
  - the Windows zip adds `minigraf.dll.lib`, and puts `minigraf.h` under
    `include/`;
  - every archive adds `LICENSE-MIT` and `LICENSE-APACHE`;
  - Linux builds use `cargo zigbuild` at glibc 2.17, with the same floor
    check as Java.
- `package.yml`: the current `build` matrix, without the upload to the
  release; upload `dist-<platform>`.
- `consumer-test/`: `quickstart.c` is the README block, plus `contract.c`.
  On Unix, `cc` and an rpath to `lib/`. On Windows, `cl` with
  `minigraf.dll.lib`, and the DLL copied next to the exe.
- Release: `build` becomes `publish`. It downloads `dist-*` and runs
  `gh release upload`. `create-release` stays.

### 3. minigraf-java

- `main`:
  - `package.yml` takes `build-natives` from `ci.yml` and adds the fat-jar
    assembly from `assemble-and-publish`, so the jar is built once;
    `publishToMavenLocal` into `dist/m2`.
  - `consumer-test/`: a Gradle project with Java `Check.java` and Kotlin
    `quickstart.kt`.
  - Consumer matrix: `ubuntu-latest`, `ubuntu-24.04-arm`, `macos-14`,
    `windows-latest`, `windows-11-arm`, and Alpine (the existing musl
    container on `ubuntu-latest`).
  - A CI job runs `package.yml` and the consumer test with `ref: v2.x`
    (spec §7).
- `v2.x`:
  - `consumer-test/` imports `uniffi.minigraf_ffi.*`;
  - the README import and version are fixed.
- `v3`: same as `main`.
- Release: `assemble-and-publish` downloads `dist-jar` and uploads the
  bundle the way Android does (the publisher API instead of nmcp).

### 4. minigraf-node

- On top of node#8/#9:
  - `package.yml` builds the 4 platform binaries (including the lipo'd
    universal one) and packs the root and the platform tarballs;
  - the CI pack step moves into `consumer-test/run.sh`;
  - the macOS leg tests the universal binary, not the arm64-only build.
- Release: `publish` runs `npm publish <tarball>` per `dist-*` file. It
  keeps the skip-if-published logic.

### 5. minigraf-python

- Fixes: `pip install ziglang` and `--zig --compatibility manylinux2014`
  for Linux wheels (change 3).
- `package.yml`: the `build-wheels` matrix; upload `dist-wheel-<os>`.
- `consumer-test/`: new venv,
  `pip install --no-index --find-links <dist> minigraf==<v>`, then run
  `quickstart.py` and `contract.py`. In registry mode,
  `pip install --no-cache-dir --only-binary :all: minigraf==<v>`.
- An extra Linux leg in a `quay.io/pypa/manylinux2014_x86_64` container
  proves the floor.
- Release: `publish` already only downloads; it now downloads `dist-wheel-*`.

### 6. minigraf-wasm

- Fixes:
  - both packages ship the license files;
  - the `@minigraf/browser` description replaces "wasm-pack build shim";
  - the README's WASI snippet uses `startMinigrafWasi`, as
    `wasi-pkg/README.md` does, because the module is the REPL binary, not a
    library.
- `package.yml`: wasm-pack build plus the stamp step, then `npm pack`. The
  WASI binary is built from core at `core-ref`, then the stamp step and
  `npm pack`. Upload `dist-browser` and `dist-wasi`.
- `consumer-test/`:
  - browser: a static page served by `npx http-server`, driven by Playwright
    Chromium, running the contract with `BrowserDb.open` and a page reload;
  - WASI: `node:wasi` with piped commands, plus a file-backed run with a
    preopened temporary directory.
- Release: the publish jobs download and run `npm publish <tgz>`.

### 7. minigraf-swift

- `package.yml` (`macos-14`; the release moves from `macos-latest` to
  `macos-14`, to match CI): the device, simulator and lipo steps, bindgen,
  `-create-xcframework` and the zip; upload `dist-xcframework`, which holds
  the zip and the generated sources.
- `consumer-test/`:
  - an SPM package with a test target;
  - in local mode, the xcframework is unzipped inside it and wired with
    `binaryTarget(path:)`, plus the generated sources;
  - in registry mode, `.package(url:, exact:)`;
  - `xcodebuild test -scheme … -destination 'platform=iOS Simulator,name=…'`.
- Release: `publish` downloads `dist-xcframework`, computes the checksum,
  writes the `swift-releases` commit and tag, and uploads the asset. It does
  not rebuild. `verify-published` starts after the asset upload (spec §6).

### 8. minigraf-binding-template

- Add `package.yml` and the shared pieces with placeholders, and a README section on filling
  them in.

## Order and release

1. Repos in the order above. The `main` PR merges before the `v3` PR, one
   repo at a time.
2. For each repo, after the `main` merge, dispatch
   `verify-published.yml -f version=2.0.8`. It must fail where §2 of the
   spec says 2.0.8 is broken (Android, C on Windows, Java's README
   compile), open the `release-broken` issue, and pass for the others. This
   tests the reporting path against real registries before v2.0.9.
3. v2.0.9 is cut after all `main` PRs (and Java `v2.x`) merge. Watch every
   cascade until `verify-published` is green. Close the 2.0.8
   `release-broken` issues with a link to the green 2.0.9 run.
4. Update the core `CLAUDE.md` and memory: the cascade monitoring rule now
   reads the `verify-published` result.
