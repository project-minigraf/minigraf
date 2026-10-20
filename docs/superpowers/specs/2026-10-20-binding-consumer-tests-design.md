# Binding consumer tests — Design

**Repos:** minigraf-node, minigraf-python, minigraf-wasm, minigraf-java,
minigraf-android, minigraf-swift, minigraf-c, minigraf-binding-template.
**Branches:** `main` and `v3` in every binding repo; also `v2.x` in
minigraf-java (§7).
**Core library:** unchanged.

## 1. Goal

No binding repo tests the artifact a user installs. CI runs the binding's
tests against a build sitting next to the sources. (The exception is
minigraf-node: since node#8, merged today, it packs a `0.0.0-ci` build and
installs it into an empty project. Even that is a CI rebuild, not the
release artifact; the release's lipo'd universal macOS binary is never
tested.) The release builds the
artifacts again in separate jobs and publishes them untested. So the release
can publish packages that cannot be used, and four did (§2).

From now on, every binding repo runs a **consumer test**: it installs the
artifact into an empty project with the language's own package tool and runs
the README quick start plus a fixed contract (§4). It runs:

1. on every PR and push, against artifacts built exactly as the release
   builds them;
2. in every cascade release, against the exact artifacts about to be
   published, before the tag is pushed;
3. after every publish, against the package fetched from the public registry
   by version.

## 2. Findings that motivate this (local check, 2026-10-20, v2.0.8)

| Binding | Published artifact | Result |
|---|---|---|
| Node | `minigraf` on npm | Broken on every platform in every version: the loader required unscoped `minigraf-<triple>`, and `optionalDependencies` were pinned at 1.0.0. Fixed by node#8/#9, closing node#7; the fix ships in v2.0.9. |
| Android | `minigraf-android` on Maven Central | Broken in every version from 1.2.0 to 2.0.8. `classes.jar` is empty: the Kotlin sources are on `java.srcDirs`, but the Kotlin Android plugin is never applied. The pom declares no JNA dependency. Each ABI also ships a stray `libminigraf-<hash>.so`. |
| Java | `minigraf-jvm` on Maven Central | Works, but only under `uniffi.minigraf_ffi.*`. The README's `io.github.project_minigraf.minigraf.MiniGrafDb` exists in no released jar; the rename (java#5) is on `main`, not on `v2.x`. |
| C | GitHub Release archives | Linux works. The Windows zip has no import library (`minigraf.dll.lib`), so MSVC cannot link it. No archive includes the license files. |
| Python | `minigraf` on PyPI | Works. Wheels are `manylinux_2_34` only and there is no sdist (§9). |
| WASM | `@minigraf/browser`, `@minigraf/wasi` | Both work. Neither ships license files. |
| Swift | `v2.0.8` xcframework | The checksum matches `Package.swift` and the slices look right. No SPM consumer build was possible on Linux. |

CI did not catch any of these, because until node#8 no job consumed the
packaged form of the artifact.

## 3. Shape in each binding repo

Three pieces in each repo.

**`.github/workflows/package.yml`** (`workflow_call`; inputs `ref`,
`version`). Builds every release artifact for every platform from `ref`,
stamped with `version`, and uploads each one as a workflow artifact named
`dist-<platform>`. This is the only place artifacts are built. The release
publish jobs download these artifacts and publish them byte for byte; they no
longer build anything.

**`consumer-test/`**: the consumer project or projects, plus `run.sh`
(bash; on Windows it runs under Git Bash):

```
consumer-test/run.sh local <dist-dir> <version>   # install from built artifacts
consumer-test/run.sh registry <version>           # install from the public registry
```

Both modes run the same consumer code (§4). They differ only in where the
package comes from. In `registry` mode the script retries installation until
the version appears, up to the registry's propagation window (§6).

**`.github/workflows/verify-published.yml`** (`workflow_call` and
`workflow_dispatch`; input `version`). Runs `run.sh registry <version>` on
the platform matrix. If any leg fails, it opens an issue in the binding repo
and the workflow stays failed (§6).

## 4. The consumer contract

Every consumer test checks the same things, in the binding's own idiom:

1. **README quick start, verbatim.** The first code block under
   `## Quick start` in `README.md` is the consumer test's quick-start file.
   A CI step extracts that block and fails if it differs from
   `consumer-test/quickstart.<ext>`. A README that drifts from the package
   (as Java's did) fails CI.
2. **In memory:** open, `(transact [[:alice :name "Alice"]])`, then
   `(query [:find ?n :where [?e :name ?n]])` returns `[["Alice"]]`.
3. **File-backed:** open a path in a fresh temporary directory, transact,
   checkpoint, close, reopen, and the query returns the row. This covers
   the file lock, the WAL and the native file I/O through the package.
4. **Errors:** invalid Datalog raises the binding's documented error type,
   not a crash or a generic exception.
5. **Installed version:** the package manager reports `<version>` for the
   installed package. In `local` mode this is the stamped version.

The consumer project depends on the package only through the package manager.
It never adds a path to the repo's sources or build directory.

## 5. Per-binding consumer tests

| Binding | Artifacts (`dist-*`) | Consumer | Runners |
|---|---|---|---|
| Node | root tarball + 4 platform tarballs | `npm install` of the root and host platform tarballs into an empty project; CJS `require` and ESM named `import` | ubuntu x64, ubuntu arm64, macos-14, windows x64 |
| Python | 4 wheels | new venv, `pip install --no-index --find-links <dist> minigraf==<v>` | same 4 |
| WASM | 2 npm tarballs | `@minigraf/browser`: a static page that imports it, driven by Playwright Chromium: `BrowserDb.open` (IndexedDB), transact, reload the page, query. `@minigraf/wasi`: `node:wasi` runs the REPL on piped commands. | ubuntu x64 |
| Java | fat jar + pom | Gradle project depending on `minigraf-jvm:<v>` from a local Maven repo dir; one Java and one Kotlin `main` | ubuntu x64, ubuntu arm64, macos-14, windows x64, windows arm64, Alpine (musl) container |
| Android | AAR + pom/module | Android app module depending on `minigraf-android:<v>` from a local Maven repo dir, so the pom's dependencies are resolved as a user's would be; instrumented test on an emulator | ubuntu x64 with KVM, emulator API 24 and 34, x86_64 image |
| Swift | xcframework zip | SPM package with a test target. `local`: `binaryTarget(path:)` to the unzipped xcframework next to the generated sources. The unzipped xcframework is copied inside the consumer package's directory, because SwiftPM resolves `path:` relative to the package root. `registry`: `.package(url:, exact: <v>)`. `xcodebuild test` on an iOS simulator. | macos-14 |
| C | 4 archives | compile and link the quick start: `cc` on Unix, MSVC `cl` plus `minigraf.dll.lib` on Windows | same 4 as Node |

In `registry` mode each one fetches from where users get the package: npm,
PyPI, Maven Central, the SPM tag, or `gh release download`.

`macos-14` is arm64. GitHub no longer offers an x86_64 macOS runner, so the
x86_64 slice of each universal binary is checked only with `lipo -archs`
(Node, Java, C) or the xcframework `Info.plist` (Swift).

## 6. Wiring

### Per PR and push (`ci.yml`)

```
ci.yml:  existing tests
         package.yml(ref = this commit, version = 0.0.0-ci)  ──►  consumer (local mode, matrix)
         readme-sync
```

The CI version is `0.0.0-ci`, or `0.0.0.dev0` for Python, which needs a
PEP 440 version.

### Cascade release (`release.yml`)

```
prepare ──► test (ci.yml on sha) ─────────────────────────────┐
        └─► package.yml(sha, version) ──► consumer (local) ────┴─► tag ──► publish ──► verify-published
```

- `package.yml` builds from the prepared `sha`, in parallel with `test`. The
  tag is a fast-forward to the same commit, so artifacts built from `sha` are
  the artifacts of the tag.
- `tag` needs both `test` and `consumer`. This keeps the existing rule (no
  tag and no main update until tests pass on the pinned commit) and extends
  it to the artifacts.
- `publish` downloads the `dist-*` artifacts and publishes them. It builds
  nothing.
- **Swift is the exception to "publish builds nothing".** Its SPM version
  tag must point at the `swift-releases` commit whose `Package.swift` holds
  the release asset's URL and checksum. So `publish` still writes that
  commit and tag and uploads the asset, but it uploads the tested
  `dist-xcframework` zip and computes the checksum from it.
  `verify-published` starts only after the semver tag and the asset exist.
- `verify-published` runs after `publish`. Retry windows: npm and PyPI
  10 minutes; GitHub Release and SPM tag 5 minutes; Maven Central 90 minutes.
  Central's sync is the slowest and the least predictable, so it gets the
  widest window. While it waits, the job sleeps between polls.

### Post-publish failure

The package is already public, so the job:

1. fails, so the release run is red and the cascade monitoring sees it;
2. opens an issue in the binding repo, titled
   `v<version> published but unusable: consumer test failed (<registry>)`,
   with labels `bug` and `release-broken`. The body links to the failing run
   and names the failing legs. If such an issue already exists for the
   version, it comments there instead.

The job needs `issues: write` on `GITHUB_TOKEN`, and each repo needs a `release-broken` label, which the first PR creates. A release-broken issue
follows the core known-issues process: it is listed in core #421 until a
fixed version is published.

## 7. Branches

- **`main` and `v3`** in every binding repo get all three pieces, as
  separate PRs. The `v3` consumer test uses the v3 API for the same contract.
- **minigraf-java `v2.x`**: 2.0.x patch releases build from `v2.x`, but
  they run `main`'s `release.yml`, and through it `main`'s `package.yml`. So
  `main`'s `package.yml` must build `v2.x` sources. The checkout of `v2.x`
  brings its own `consumer-test/`, which therefore has to exist on `v2.x`
  too. `v2.x` gets a PR with `consumer-test/` and the README fix (§8). The `main`
  PR must also pass against a `v2.x` checkout before merge: its CI gets a
  job that calls `package.yml` and the consumer test with `ref: v2.x`. The
  `main` and `v2.x` PRs both merge before the v2.0.9 cascade, because that
  cascade runs `main`'s `release.yml` on `v2.x` sources.
- **minigraf-binding-template**: gets the skeleton (`package.yml`,
  `verify-published.yml`, `consumer-test/run.sh`, readme-sync), so a new
  binding starts with the check.

## 8. Fixes that ship with the checks

Each repo's PR must pass its own new check, so it carries the fixes for §2:

| Repo | Fix | Ships in |
|---|---|---|
| Android | apply `org.jetbrains.kotlin.android`; add `net.java.dev.jna:jna:<v>@aar` as an `api` dependency; copy only `libminigraf_ffi.so` into `jniLibs` | v2.0.9 (user decision 2026-10-20: the artifact never worked, so no user depends on it) |
| Java `v2.x` | README documents `uniffi.minigraf_ffi.MiniGrafDb` and the current version | v2.0.9. The `io.github…` rename stays for v3.0.0 (user decision 2026-10-20; behaviour breaks only in a major release) |
| C | Windows archive adds `minigraf.dll.lib`; all archives add `LICENSE-MIT` and `LICENSE-APACHE`; the header is under `include/` in every archive | v2.0.9 |
| WASM | both packages ship the license files | v2.0.9 |
| Node | `package.yml` + consumer test of the exact release artifacts, including the lipo'd universal macOS binary, which CI never built | v2.0.9 |
| Python, Swift | none known; the checks may find some | — |

## 9. Out of scope

- **Python glibc floor and sdist.** Wheels need glibc 2.34+, which rules out
  Ubuntu 20.04, Debian 11 and Amazon Linux 2, and there is no sdist to fall
  back to. This is a packaging decision, not a defect the check would catch.
  It gets its own issue.
- **x86_64 macOS execution** (§5).
- **Core repo changes.** `cascade.yml` stays a fan-out. The cascade
  monitoring rule still applies; the post-publish job turns its manual check
  into a red run plus an issue.

## 10. Order

One PR per repo and branch. The first four repos have known-broken or
missing pieces, so they go first:

1. Android `main`, `v3`
2. C `main`, `v3`
3. Java `main`, `v2.x`, `v3`
4. Node `main`, `v3` (`package.yml` and verify-published on top of node#8/#9)
5. Python `main`, `v3`
6. WASM `main`, `v3`
7. Swift `main`, `v3`
8. binding-template

v2.0.9 is cut only after steps 1–7 are merged on the branch it releases
from. Its cascade is the first run of the full release wiring.
