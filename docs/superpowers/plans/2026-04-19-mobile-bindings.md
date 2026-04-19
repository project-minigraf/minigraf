# Phase 8.2: Mobile Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Minigraf as a native library for Android (.aar) and iOS (.xcframework) via UniFFI, with artifacts published to GitHub Releases and GitHub Packages on every version tag.

**Architecture:** A new `minigraf-ffi` workspace crate wraps the public `minigraf` API with `#[uniffi::export]` proc-macro bindings, producing `.so` (Android) and `.a` (iOS) libraries. A separate `mobile.yml` CI workflow cross-compiles, assembles, and uploads release artifacts without touching the `cargo-dist`-managed `release.yml`. The existing WASM artifact jobs are extracted from `release.yml` into `wasm-release.yml` by the same pattern.

**Tech Stack:** Rust (UniFFI 0.28+, thiserror 2, serde_json 1), Gradle (Kotlin DSL, Android Library plugin, maven-publish), GitHub Actions, cargo-ndk, Swift Package Manager.

**Spec:** `docs/superpowers/specs/2026-04-19-mobile-bindings-design.md`

---

## File Map

### Create
- `minigraf-ffi/Cargo.toml` — FFI crate manifest (`publish = false`, cdylib + staticlib)
- `minigraf-ffi/src/lib.rs` — `MiniGrafDb`, `MiniGrafError`, JSON serialisation helpers, native tests
- `minigraf-ffi/src/uniffi_bindgen.rs` — `uniffi_bindgen_main()` binary entry point
- `minigraf-ffi/android/settings.gradle.kts` — Gradle project name
- `minigraf-ffi/android/build.gradle.kts` — Android Library plugin + maven-publish to GitHub Packages
- `minigraf-ffi/android/src/main/AndroidManifest.xml` — minimal manifest (required by Android plugin)
- `Package.swift` — SPM binary target (placeholder URL+checksum, CI updates on release)
- `swift/Sources/MinigrafKit/.gitkeep` — placeholder for UniFFI-generated Swift sources (CI populates)
- `.github/workflows/mobile.yml` — cross-compile + assemble + upload Android and iOS artifacts
- `.github/workflows/wasm-release.yml` — extracted WASM artifact jobs with release upload

### Modify
- `Cargo.toml` — add `[workspace]` table at top; add `-p minigraf` to `publish-crates-io` step in release.yml (not Cargo.toml — see Task 2 and Task 11)
- `.github/workflows/release.yml` — remove `build-wasm-wasi`, `build-wasm-browser`, trim `host` needs
- `.github/workflows/rust.yml` — add `ffi-check` job (`cargo check -p minigraf-ffi`)
- `ROADMAP.md` — add post-1.0 FFI items (UDF + prepared query over FFI)

---

## Task 1: Create git worktree

**Files:** none (meta-setup)

- [ ] **Step 1: Invoke worktree skill**

  Use the `superpowers:using-git-worktrees` skill to create an isolated worktree for this feature branch before touching any files. Branch name suggestion: `feat/phase-8.2-mobile-bindings`.

---

## Task 2: Convert `Cargo.toml` to workspace

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add `[workspace]` table at the top of `Cargo.toml`**

  Open `Cargo.toml`. Insert the following block as the very first section, before `[package]`:

  ```toml
  [workspace]
  members = [".", "minigraf-ffi"]
  resolver = "2"
  ```

  The full top of the file should look like:

  ```toml
  [workspace]
  members = [".", "minigraf-ffi"]
  resolver = "2"

  [package]
  name = "minigraf"
  version = "0.20.1"
  edition = "2024"
  # ... rest of existing [package] unchanged
  ```

- [ ] **Step 2: Verify workspace builds cleanly**

  ```bash
  cargo build
  ```

  Expected: builds successfully. `minigraf-ffi` does not exist yet — Cargo will warn about it or error. If it errors, create the `minigraf-ffi/` directory with an empty `Cargo.toml` placeholder first (full content comes in Task 3), then re-run.

  Minimal placeholder to unblock the build:
  ```toml
  # minigraf-ffi/Cargo.toml (temporary placeholder)
  [package]
  name = "minigraf-ffi"
  version = "0.0.0"
  edition = "2024"
  publish = false
  ```

- [ ] **Step 3: Verify tests still pass**

  ```bash
  cargo test -p minigraf
  ```

  Expected: all existing tests pass (count should remain 795+).

- [ ] **Step 4: Commit**

  ```bash
  git add Cargo.toml minigraf-ffi/
  git commit -m "chore: convert to Cargo workspace, add minigraf-ffi member"
  ```

---

## Task 3: Scaffold `minigraf-ffi` crate

**Files:**
- Create: `minigraf-ffi/Cargo.toml`
- Create: `minigraf-ffi/src/uniffi_bindgen.rs`

- [ ] **Step 1: Write `minigraf-ffi/Cargo.toml`**

  Replace any placeholder content with:

  ```toml
  [package]
  name = "minigraf-ffi"
  version = "0.20.1"
  edition = "2024"
  description = "UniFFI mobile bindings for Minigraf (Android + iOS)"
  publish = false

  [lib]
  crate-type = ["cdylib", "staticlib"]

  [[bin]]
  name = "uniffi-bindgen"
  path = "src/uniffi_bindgen.rs"

  [dependencies]
  minigraf = { path = ".." }
  uniffi = { version = "0.28", features = ["cli"] }
  thiserror = "2"
  serde_json = "1"
  ```

  > **Note:** Check crates.io for the latest stable `uniffi` version at implementation time (minimum `0.28`). Pin to a specific patch version (e.g. `"0.28.3"`) for reproducible builds.

- [ ] **Step 2: Write `minigraf-ffi/src/uniffi_bindgen.rs`**

  ```rust
  fn main() {
      uniffi::uniffi_bindgen_main()
  }
  ```

- [ ] **Step 3: Create stub `minigraf-ffi/src/lib.rs`**

  Just enough to compile — full content comes in Tasks 4–6:

  ```rust
  uniffi::setup_scaffolding!();
  ```

- [ ] **Step 4: Verify crate compiles**

  ```bash
  cargo build -p minigraf-ffi
  ```

  Expected: compiles. `uniffi` and `thiserror` are downloaded and compiled.

- [ ] **Step 5: Commit**

  ```bash
  git add minigraf-ffi/
  git commit -m "feat(ffi): scaffold minigraf-ffi crate with UniFFI setup"
  ```

---

## Task 4: TDD — `MiniGrafError` and JSON serialisation helpers

**Files:**
- Modify: `minigraf-ffi/src/lib.rs`

TDD cycle: write the failing test, run it, implement the minimum to pass, run again.

- [ ] **Step 1: Write the failing tests**

  Replace `minigraf-ffi/src/lib.rs` with:

  ```rust
  use minigraf::{Minigraf, QueryResult, Value};
  use std::sync::{Arc, Mutex};

  uniffi::setup_scaffolding!();

  // ─── Error type ──────────────────────────────────────────────────────────────

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

  // ─── MiniGrafDb stub (needed for test compilation) ───────────────────────────

  #[derive(uniffi::Object)]
  pub struct MiniGrafDb {
      inner: Arc<Mutex<Minigraf>>,
  }

  #[uniffi::export]
  impl MiniGrafDb {
      #[uniffi::constructor]
      pub fn open(_path: String) -> Result<Arc<Self>, MiniGrafError> {
          todo!()
      }

      #[uniffi::constructor]
      pub fn open_in_memory() -> Result<Arc<Self>, MiniGrafError> {
          todo!()
      }

      pub fn execute(&self, _datalog: String) -> Result<String, MiniGrafError> {
          todo!()
      }

      pub fn checkpoint(&self) -> Result<(), MiniGrafError> {
          todo!()
      }
  }

  // ─── JSON serialisation (internal helpers) ───────────────────────────────────

  fn query_result_to_json(result: QueryResult) -> String {
      todo!()
  }

  fn value_to_json(v: &Value) -> serde_json::Value {
      todo!()
  }

  // ─── Tests ───────────────────────────────────────────────────────────────────

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn value_to_json_string() {
          let v = Value::String("hello".into());
          let j = value_to_json(&v);
          assert_eq!(j, serde_json::Value::String("hello".into()));
      }

      #[test]
      fn value_to_json_integer() {
          let v = Value::Integer(42);
          let j = value_to_json(&v);
          assert_eq!(j, serde_json::json!(42));
      }

      #[test]
      fn value_to_json_null() {
          let j = value_to_json(&Value::Null);
          assert_eq!(j, serde_json::Value::Null);
      }

      #[test]
      fn query_result_to_json_transacted() {
          let json = query_result_to_json(QueryResult::Transacted(12345));
          let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
          assert_eq!(v["transacted"], serde_json::json!(12345));
      }

      #[test]
      fn query_result_to_json_query_results() {
          let result = QueryResult::QueryResults {
              vars: vec!["?name".into()],
              results: vec![vec![Value::String("Alice".into())]],
          };
          let json = query_result_to_json(result);
          let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
          assert_eq!(v["variables"][0], "?name");
          assert_eq!(v["results"][0][0], "Alice");
      }

      #[test]
      fn query_result_to_json_ok() {
          let json = query_result_to_json(QueryResult::Ok);
          let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
          assert_eq!(v["ok"], serde_json::json!(true));
      }
  }
  ```

- [ ] **Step 2: Run tests to confirm they fail**

  ```bash
  cargo test -p minigraf-ffi 2>&1 | head -30
  ```

  Expected: tests fail with "not yet implemented" panics from `todo!()`.

- [ ] **Step 3: Implement `value_to_json` and `query_result_to_json`**

  Replace the `todo!()` bodies of both functions:

  ```rust
  fn value_to_json(v: &Value) -> serde_json::Value {
      use serde_json::Value as JVal;
      match v {
          Value::String(s)  => JVal::String(s.clone()),
          Value::Integer(i) => JVal::Number((*i).into()),
          Value::Float(f)   => serde_json::Number::from_f64(*f)
              .map(JVal::Number)
              .unwrap_or(JVal::Null),
          Value::Boolean(b) => JVal::Bool(*b),
          Value::Ref(uuid)  => JVal::String(uuid.to_string()),
          Value::Keyword(k) => JVal::String(k.clone()),
          Value::Null       => JVal::Null,
      }
  }

  fn query_result_to_json(result: QueryResult) -> String {
      use serde_json::json;
      let val = match result {
          QueryResult::Transacted(tx_id) => json!({"transacted": tx_id}),
          QueryResult::Retracted(tx_id)  => json!({"retracted": tx_id}),
          QueryResult::Ok                => json!({"ok": true}),
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
  ```

- [ ] **Step 4: Run tests to confirm they pass**

  ```bash
  cargo test -p minigraf-ffi
  ```

  Expected: all 5 serialisation tests pass.

- [ ] **Step 5: Commit**

  ```bash
  git add minigraf-ffi/src/lib.rs
  git commit -m "feat(ffi): implement MiniGrafError and JSON serialisation helpers"
  ```

---

## Task 5: TDD — `MiniGrafDb::open_in_memory` and `execute`

**Files:**
- Modify: `minigraf-ffi/src/lib.rs`

- [ ] **Step 1: Add tests for open_in_memory and execute to the existing `tests` module**

  Add to the `#[cfg(test)] mod tests` block:

  ```rust
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
      assert!(v.get("transacted").is_some(), "expected transacted key");
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
      assert_eq!(v["variables"][0], "?n");
      assert_eq!(v["results"][0][0], "Alice");
  }

  #[test]
  fn execute_invalid_datalog_returns_parse_error() {
      let db = MiniGrafDb::open_in_memory().expect("open");
      let result = db.execute("not valid datalog at all !!!".into());
      assert!(
          matches!(result, Err(MiniGrafError::Parse { .. })),
          "expected Parse error"
      );
  }
  ```

- [ ] **Step 2: Run tests to confirm new ones fail**

  ```bash
  cargo test -p minigraf-ffi 2>&1 | grep -E "FAILED|panicked"
  ```

  Expected: the 4 new tests fail with panics from `todo!()`.

- [ ] **Step 3: Implement `open_in_memory` and `execute`**

  First add the `anyhow::Error` → `MiniGrafError` conversion. Add this function after the `MiniGrafError` definition:

  ```rust
  impl From<anyhow::Error> for MiniGrafError {
      fn from(e: anyhow::Error) -> Self {
          let full = format!("{e:#}").to_lowercase();
          let msg = e.to_string();
          if full.contains("parse") || full.contains("unexpected") || full.contains("expected token") {
              MiniGrafError::Parse { msg }
          } else if full.contains("storage") || full.contains("page") || full.contains("wal") {
              MiniGrafError::Storage { msg }
          } else if full.contains("query") || full.contains(":find") || full.contains(":where") {
              MiniGrafError::Query { msg }
          } else {
              MiniGrafError::Other { msg }
          }
      }
  }
  ```

  Then replace the `todo!()` bodies of `open_in_memory` and `execute`:

  ```rust
  #[uniffi::constructor]
  pub fn open_in_memory() -> Result<Arc<Self>, MiniGrafError> {
      let db = Minigraf::in_memory().map_err(MiniGrafError::from)?;
      Ok(Arc::new(Self {
          inner: Arc::new(Mutex::new(db)),
      }))
  }

  pub fn execute(&self, datalog: String) -> Result<String, MiniGrafError> {
      let result = self
          .inner
          .lock()
          .map_err(|_| MiniGrafError::Other { msg: "mutex poisoned".into() })?
          .execute(&datalog)
          .map_err(MiniGrafError::from)?;
      Ok(query_result_to_json(result))
  }
  ```

- [ ] **Step 4: Run tests to confirm they pass**

  ```bash
  cargo test -p minigraf-ffi
  ```

  Expected: all 9 tests pass.

- [ ] **Step 5: Commit**

  ```bash
  git add minigraf-ffi/src/lib.rs
  git commit -m "feat(ffi): implement MiniGrafDb::open_in_memory and execute"
  ```

---

## Task 6: TDD — `MiniGrafDb::open` (file-backed) and `checkpoint`

**Files:**
- Modify: `minigraf-ffi/src/lib.rs`

- [ ] **Step 1: Add tests for file-backed open and checkpoint**

  Add to the `#[cfg(test)] mod tests` block:

  ```rust
  #[test]
  fn open_file_backed_roundtrip() {
      let dir = std::env::temp_dir();
      let path = dir.join("minigraf_ffi_test.graph");
      // Clean up any leftover file
      let _ = std::fs::remove_file(&path);
      let path_str = path.to_str().expect("utf8 path").to_string();

      {
          let db = MiniGrafDb::open(path_str.clone()).expect("open");
          db.execute(r#"(transact [[:alice :name "Alice"]])"#.into())
              .expect("transact");
          db.checkpoint().expect("checkpoint");
      }

      // Re-open and verify fact persisted
      let db2 = MiniGrafDb::open(path_str).expect("re-open");
      let json = db2
          .execute(r#"(query [:find ?n :where [?e :name ?n]])"#.into())
          .expect("query");
      let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
      assert_eq!(v["results"][0][0], "Alice");

      // Clean up
      let _ = std::fs::remove_file(path);
      let wal = dir.join("minigraf_ffi_test.graph.wal");
      let _ = std::fs::remove_file(wal);
  }

  #[test]
  fn checkpoint_in_memory_succeeds() {
      let db = MiniGrafDb::open_in_memory().expect("open");
      db.checkpoint().expect("checkpoint on in-memory db");
  }
  ```

- [ ] **Step 2: Run tests to confirm new ones fail**

  ```bash
  cargo test -p minigraf-ffi 2>&1 | grep -E "FAILED|panicked"
  ```

  Expected: the 2 new tests fail with panics from `todo!()`.

- [ ] **Step 3: Implement `open` and `checkpoint`**

  Replace the `todo!()` bodies:

  ```rust
  #[uniffi::constructor]
  pub fn open(path: String) -> Result<Arc<Self>, MiniGrafError> {
      let db = Minigraf::open(&path).map_err(MiniGrafError::from)?;
      Ok(Arc::new(Self {
          inner: Arc::new(Mutex::new(db)),
      }))
  }

  pub fn checkpoint(&self) -> Result<(), MiniGrafError> {
      self.inner
          .lock()
          .map_err(|_| MiniGrafError::Other { msg: "mutex poisoned".into() })?
          .checkpoint()
          .map_err(MiniGrafError::from)
  }
  ```

- [ ] **Step 4: Run all tests to confirm they pass**

  ```bash
  cargo test -p minigraf-ffi
  ```

  Expected: all 11 tests pass.

- [ ] **Step 5: Run full test suite to confirm no regressions**

  ```bash
  cargo test
  ```

  Expected: all tests pass (795 core tests + 11 FFI tests = 806+).

- [ ] **Step 6: Commit**

  ```bash
  git add minigraf-ffi/src/lib.rs
  git commit -m "feat(ffi): implement MiniGrafDb::open and checkpoint; all FFI tests green"
  ```

---

## Task 7: Android Gradle project

**Files:**
- Create: `minigraf-ffi/android/settings.gradle.kts`
- Create: `minigraf-ffi/android/build.gradle.kts`
- Create: `minigraf-ffi/android/src/main/AndroidManifest.xml`

This Gradle project is a configuration-only Android library. It has no source code — the JNI libs and Kotlin bindings are written into it by CI at build time.

- [ ] **Step 1: Create `minigraf-ffi/android/settings.gradle.kts`**

  ```kotlin
  pluginManagement {
      repositories {
          google()
          mavenCentral()
          gradlePluginPortal()
      }
  }
  dependencyResolutionManagement {
      repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
      repositories {
          google()
          mavenCentral()
      }
  }

  rootProject.name = "minigraf-android"
  ```

- [ ] **Step 2: Create `minigraf-ffi/android/build.gradle.kts`**

  ```kotlin
  plugins {
      id("com.android.library") version "8.2.2"
      id("maven-publish")
  }

  android {
      namespace = "io.github.adityamukho.minigraf"
      compileSdk = 34
      defaultConfig {
          minSdk = 24
      }
      sourceSets {
          getByName("main") {
              jniLibs.srcDirs("jniLibs")
              java.srcDirs("src/main/java")
          }
      }
  }

  afterEvaluate {
      publishing {
          publications {
              create<MavenPublication>("release") {
                  from(components["release"])
                  groupId = "io.github.adityamukho"
                  artifactId = "minigraf-android"
                  version = System.getenv("VERSION") ?: "0.0.0-local"
              }
          }
          repositories {
              maven {
                  name = "GitHubPackages"
                  url = uri("https://maven.pkg.github.com/adityamukho/minigraf")
                  credentials {
                      username = System.getenv("GITHUB_ACTOR") ?: ""
                      password = System.getenv("GITHUB_TOKEN") ?: ""
                  }
              }
          }
      }
  }
  ```

- [ ] **Step 3: Create `minigraf-ffi/android/src/main/AndroidManifest.xml`**

  ```xml
  <?xml version="1.0" encoding="utf-8"?>
  <manifest xmlns:android="http://schemas.android.com/apk/res/android" />
  ```

- [ ] **Step 4: Create placeholder directories that CI will populate**

  ```bash
  mkdir -p minigraf-ffi/android/src/main/java
  touch minigraf-ffi/android/src/main/java/.gitkeep
  mkdir -p minigraf-ffi/android/jniLibs
  touch minigraf-ffi/android/jniLibs/.gitkeep
  ```

- [ ] **Step 5: Generate the Gradle wrapper**

  From inside `minigraf-ffi/android/`, generate the wrapper files. Gradle 8.4+ is required:

  ```bash
  cd minigraf-ffi/android
  gradle wrapper --gradle-version 8.4
  cd ../..
  ```

  If Gradle is not installed locally, use the Docker image:

  ```bash
  docker run --rm -v "$(pwd)/minigraf-ffi/android:/project" -w /project \
    gradle:8.4-jdk17 gradle wrapper --gradle-version 8.4
  ```

  This creates `gradlew`, `gradlew.bat`, and `gradle/wrapper/gradle-wrapper.{jar,properties}`. All four must be committed. Make `gradlew` executable:

  ```bash
  chmod +x minigraf-ffi/android/gradlew
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add minigraf-ffi/android/
  git commit -m "feat(ffi): add Android Gradle project for .aar assembly"
  ```

---

## Task 8: `Package.swift` and Swift sources placeholder

**Files:**
- Create: `Package.swift`
- Create: `swift/Sources/MinigrafKit/.gitkeep`

`Package.swift` must live at the repo root for Swift Package Manager to resolve the package from the GitHub URL. The placeholder URL and checksum are updated by the `release-upload-mobile` CI job after each release.

- [ ] **Step 1: Create `Package.swift` at the repo root**

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
              // Updated by CI: release-upload-mobile job
              url: "https://github.com/adityamukho/minigraf/releases/download/v0.20.1/MinigrafKit-v0.20.1.xcframework.zip",
              checksum: "0000000000000000000000000000000000000000000000000000000000000000"
          ),
          .target(
              name: "MinigrafKit",
              dependencies: [.target(name: "minigrafFFI")],
              path: "swift/Sources/MinigrafKit"
          ),
      ]
  )
  ```

- [ ] **Step 2: Create Swift sources placeholder directory**

  ```bash
  mkdir -p swift/Sources/MinigrafKit
  touch swift/Sources/MinigrafKit/.gitkeep
  ```

- [ ] **Step 3: Commit**

  ```bash
  git add Package.swift swift/
  git commit -m "feat(spm): add Package.swift skeleton and Swift sources placeholder"
  ```

---

## Task 9: Add FFI check to `rust.yml`

**Files:**
- Modify: `.github/workflows/rust.yml`

Add a lightweight `ffi-check` job that runs `cargo check -p minigraf-ffi` on every PR. No cross-compilation, no Gradle — just verifies the FFI crate compiles and proc-macros parse correctly. This is fast and catches type errors before the full release CI.

- [ ] **Step 1: Add `ffi-check` job to `.github/workflows/rust.yml`**

  The existing file has a `build` job with a matrix. Add the new job after it:

  ```yaml
  ffi-check:
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v3
    - name: Check FFI crate compiles
      run: cargo check -p minigraf-ffi
    - name: Run FFI native tests
      run: cargo test -p minigraf-ffi
  ```

  The full file after modification:

  ```yaml
  name: Rust

  on:
    push:
      branches: [ "main" ]
    pull_request:
      branches: [ "main" ]

  env:
    CARGO_TERM_COLOR: always

  permissions:
    contents: read

  jobs:
    build:
      strategy:
        matrix:
          os: [ubuntu-latest, macos-latest, windows-latest]
      runs-on: ${{ matrix.os }}
      steps:
      - uses: actions/checkout@v3
      - name: Build
        run: cargo build --verbose
      - name: Run tests
        run: cargo test --verbose

    ffi-check:
      runs-on: ubuntu-latest
      steps:
      - uses: actions/checkout@v3
      - name: Check FFI crate compiles
        run: cargo check -p minigraf-ffi
      - name: Run FFI native tests
        run: cargo test -p minigraf-ffi
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/rust.yml
  git commit -m "ci: add ffi-check job to rust.yml for PR validation of minigraf-ffi"
  ```

---

## Task 10: Create `wasm-release.yml`

**Files:**
- Create: `.github/workflows/wasm-release.yml`

Extract the `build-wasm-wasi` and `build-wasm-browser` jobs from `release.yml` into a standalone workflow that uploads WASM artifacts to the GitHub Release. This makes `release.yml` safe to regenerate with `cargo-dist`.

- [ ] **Step 1: Create `.github/workflows/wasm-release.yml`**

  ```yaml
  name: WASM Release Artifacts

  on:
    push:
      tags:
        - '**[0-9]+.[0-9]+.[0-9]+*'
    workflow_dispatch:
      inputs:
        tag:
          description: 'Release tag to upload artifacts to (e.g. v0.21.0)'
          required: true

  permissions:
    contents: write

  jobs:
    build-wasm-wasi:
      runs-on: ubuntu-22.04
      steps:
        - uses: actions/checkout@v6
          with:
            persist-credentials: false
        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: wasm32-wasip1
        - name: Build WASI binary
          run: cargo build --target wasm32-wasip1 --release --bin minigraf
        - name: Stage artifact
          run: |
            mkdir -p target/wasm-artifacts
            cp target/wasm32-wasip1/release/minigraf.wasm target/wasm-artifacts/minigraf-wasi.wasm
        - uses: actions/upload-artifact@v6
          with:
            name: artifacts-wasm-wasi
            path: target/wasm-artifacts/

    build-wasm-browser:
      runs-on: ubuntu-22.04
      steps:
        - uses: actions/checkout@v6
          with:
            persist-credentials: false
        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: wasm32-unknown-unknown
        - name: Install wasm-pack
          run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
        - name: Build browser WASM
          run: wasm-pack build --target web --features browser
        - name: Package pkg/ directory
          run: |
            mkdir -p target/wasm-artifacts
            tar -czf target/wasm-artifacts/minigraf-browser-wasm.tar.gz -C . pkg/
        - uses: actions/upload-artifact@v6
          with:
            name: artifacts-wasm-browser
            path: target/wasm-artifacts/

    release-upload-wasm:
      needs: [build-wasm-wasi, build-wasm-browser]
      runs-on: ubuntu-22.04
      steps:
        - uses: actions/download-artifact@v7
          with:
            pattern: artifacts-wasm-*
            path: wasm-artifacts/
            merge-multiple: true
        - name: Wait for GitHub Release to exist, then upload
          env:
            GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
            TAG: ${{ github.ref_name || inputs.tag }}
          run: |
            echo "Waiting for release $TAG to be created by cargo-dist..."
            for i in $(seq 1 20); do
              if gh release view "$TAG" --repo "$GITHUB_REPOSITORY" > /dev/null 2>&1; then
                echo "Release found on attempt $i"
                break
              fi
              echo "Attempt $i/20: release not yet available, waiting 15s..."
              sleep 15
            done
            gh release upload "$TAG" wasm-artifacts/* \
              --repo "$GITHUB_REPOSITORY" \
              --clobber
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/wasm-release.yml
  git commit -m "ci: extract WASM artifact jobs to wasm-release.yml (release.yml safe to regenerate)"
  ```

---

## Task 11: Update `release.yml`

**Files:**
- Modify: `.github/workflows/release.yml`

Remove `build-wasm-wasi` and `build-wasm-browser` jobs and their references from the `host` job's `needs` list. Also scope the `cargo publish` call to `-p minigraf` for workspace safety.

- [ ] **Step 1: Remove the two WASM build jobs**

  Delete the entire `build-wasm-wasi` job block (lines `# Build the WASI binary...` through the end of its `upload-artifact` step) and the entire `build-wasm-browser` job block.

- [ ] **Step 2: Update the `host` job's `needs` list**

  Find the `host` job. Change:

  ```yaml
  host:
    needs:
      - plan
      - build-local-artifacts
      - build-global-artifacts
      - build-wasm-wasi
      - build-wasm-browser
    if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success') && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') && (needs.build-wasm-wasi.result == 'skipped' || needs.build-wasm-wasi.result == 'success') && (needs.build-wasm-browser.result == 'skipped' || needs.build-wasm-browser.result == 'success') }}
  ```

  To:

  ```yaml
  host:
    needs:
      - plan
      - build-local-artifacts
      - build-global-artifacts
    if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success') && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') }}
  ```

- [ ] **Step 3: Scope `cargo publish` to `-p minigraf`**

  Find the `publish-crates-io` job. Change:

  ```yaml
  run: cargo publish --locked --token ${{ secrets.CARGO_REGISTRY_TOKEN }}
  ```

  To:

  ```yaml
  run: cargo publish --locked -p minigraf --token ${{ secrets.CARGO_REGISTRY_TOKEN }}
  ```

- [ ] **Step 4: Commit**

  ```bash
  git add .github/workflows/release.yml
  git commit -m "ci: remove WASM jobs from release.yml (now in wasm-release.yml); scope cargo publish to -p minigraf"
  ```

---

## Task 12: Create `mobile.yml`

**Files:**
- Create: `.github/workflows/mobile.yml`

This workflow cross-compiles `minigraf-ffi` for Android and iOS targets, assembles the `.aar` and `.xcframework`, uploads them to the GitHub Release, publishes the `.aar` to GitHub Packages, and updates `Package.swift` with the release checksum.

- [ ] **Step 1: Create `.github/workflows/mobile.yml`**

  ```yaml
  name: Mobile Release Artifacts

  on:
    push:
      tags:
        - '**[0-9]+.[0-9]+.[0-9]+*'
    workflow_dispatch:
      inputs:
        tag:
          description: 'Release tag to upload artifacts to (e.g. v0.21.0)'
          required: true

  permissions:
    contents: write
    packages: write

  jobs:
    # ── Android ────────────────────────────────────────────────────────────────

    mobile-android:
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

        - name: Set up JDK 17
          uses: actions/setup-java@v4
          with:
            java-version: '17'
            distribution: 'temurin'

        - name: Cross-compile Android targets
          run: |
            cargo ndk \
              -t arm64-v8a \
              -t armeabi-v7a \
              -t x86_64 \
              -o minigraf-ffi/android/jniLibs \
              build --release -p minigraf-ffi

        - name: Generate Kotlin bindings
          run: |
            cargo run -p minigraf-ffi --bin uniffi-bindgen -- generate \
              --library target/aarch64-linux-android/release/libminigraf_ffi.so \
              --language kotlin \
              --out-dir minigraf-ffi/android/src/main/java/

        - name: Assemble AAR
          run: |
            cd minigraf-ffi/android
            ./gradlew assembleRelease

        - name: Rename artifact
          run: |
            TAG="${{ github.ref_name || inputs.tag }}"
            cp minigraf-ffi/android/build/outputs/aar/minigraf-android-release.aar \
               "minigraf-android-${TAG}.aar"

        - uses: actions/upload-artifact@v4
          with:
            name: minigraf-android
            path: "minigraf-android-*.aar"

    # ── iOS ────────────────────────────────────────────────────────────────────

    mobile-ios:
      runs-on: macos-latest
      steps:
        - uses: actions/checkout@v4

        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: aarch64-apple-ios,aarch64-apple-ios-sim

        - name: Build iOS device library
          run: cargo build --target aarch64-apple-ios --release -p minigraf-ffi

        - name: Build iOS simulator library
          run: cargo build --target aarch64-apple-ios-sim --release -p minigraf-ffi

        - name: Generate Swift bindings
          run: |
            mkdir -p swift/Sources/MinigrafKit
            cargo run -p minigraf-ffi --bin uniffi-bindgen -- generate \
              --library target/aarch64-apple-ios/release/libminigraf_ffi.a \
              --language swift \
              --out-dir swift/Sources/MinigrafKit/

        - name: Prepare headers for xcframework
          run: |
            mkdir -p swift/includes
            # uniffi-bindgen generates .h and .modulemap alongside .swift
            cp swift/Sources/MinigrafKit/*.h swift/includes/ 2>/dev/null || true
            cp swift/Sources/MinigrafKit/*.modulemap swift/includes/ 2>/dev/null || true

        - name: Assemble xcframework
          run: |
            TAG="${{ github.ref_name || inputs.tag }}"
            xcodebuild -create-xcframework \
              -library target/aarch64-apple-ios/release/libminigraf_ffi.a \
              -headers swift/includes/ \
              -library target/aarch64-apple-ios-sim/release/libminigraf_ffi.a \
              -headers swift/includes/ \
              -output "MinigrafKit.xcframework"
            zip -r "MinigrafKit-${TAG}.xcframework.zip" MinigrafKit.xcframework

        - uses: actions/upload-artifact@v4
          with:
            name: minigraf-ios
            path: "MinigrafKit-*.xcframework.zip"
            # Also upload the generated Swift sources so release-upload job can commit them
        - uses: actions/upload-artifact@v4
          with:
            name: swift-sources
            path: swift/Sources/MinigrafKit/

    # ── Upload to release + GitHub Packages ───────────────────────────────────

    release-upload-mobile:
      needs: [mobile-android, mobile-ios]
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
          with:
            # Need write access to push Package.swift update + tag move
            token: ${{ secrets.GITHUB_TOKEN }}
            fetch-depth: 0

        - uses: actions/download-artifact@v4
          with:
            name: minigraf-android
            path: artifacts/

        - uses: actions/download-artifact@v4
          with:
            name: minigraf-ios
            path: artifacts/

        - uses: actions/download-artifact@v4
          with:
            name: swift-sources
            path: swift/Sources/MinigrafKit/

        - name: Set up JDK 17
          uses: actions/setup-java@v4
          with:
            java-version: '17'
            distribution: 'temurin'

        - name: Wait for GitHub Release to exist
          env:
            GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
            TAG: ${{ github.ref_name || inputs.tag }}
          run: |
            echo "Waiting for release $TAG..."
            for i in $(seq 1 20); do
              if gh release view "$TAG" --repo "$GITHUB_REPOSITORY" > /dev/null 2>&1; then
                echo "Release found on attempt $i"
                break
              fi
              echo "Attempt $i/20: not yet, waiting 15s..."
              sleep 15
            done

        - name: Upload artifacts to GitHub Release
          env:
            GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
            TAG: ${{ github.ref_name || inputs.tag }}
          run: |
            gh release upload "$TAG" artifacts/* \
              --repo "$GITHUB_REPOSITORY" \
              --clobber

        - name: Compute xcframework checksum and update Package.swift
          env:
            TAG: ${{ github.ref_name || inputs.tag }}
          run: |
            XCFW_ZIP=$(ls artifacts/MinigrafKit-*.xcframework.zip)
            CHECKSUM=$(shasum -a 256 "$XCFW_ZIP" | awk '{print $1}')
            URL="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/MinigrafKit-${TAG}.xcframework.zip"
            # Update Package.swift url and checksum in-place
            sed -i \
              -e "s|url: \"https://github.com/.*/releases/download/.*/MinigrafKit-.*\\.xcframework\\.zip\"|url: \"${URL}\"|" \
              -e "s|checksum: \"[0-9a-f]*\"|checksum: \"${CHECKSUM}\"|" \
              Package.swift

        - name: Commit Package.swift update and Swift sources, then move tag
          env:
            TAG: ${{ github.ref_name || inputs.tag }}
            GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          run: |
            git config user.email "github-actions[bot]@users.noreply.github.com"
            git config user.name "github-actions[bot]"
            git add Package.swift swift/Sources/MinigrafKit/
            git commit -m "chore(release): update Package.swift and Swift bindings for ${TAG}" \
              || echo "Nothing to commit"
            git push origin HEAD:main
            # Force-move the release tag to include this commit
            NEW_SHA=$(git rev-parse HEAD)
            gh api "repos/${GITHUB_REPOSITORY}/git/refs/tags/${TAG}" \
              -X PATCH \
              -f sha="$NEW_SHA" \
              -f force=true

        - name: Publish AAR to GitHub Packages
          env:
            GITHUB_ACTOR: ${{ github.actor }}
            GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          run: |
            TAG="${{ github.ref_name || inputs.tag }}"
            # Strip leading 'v' so Maven version is '0.21.0' not 'v0.21.0'
            export VERSION="${TAG#v}"
            # Copy aar into Gradle's expected output directory so publish picks it up
            mkdir -p minigraf-ffi/android/build/outputs/aar/
            cp artifacts/minigraf-android-*.aar \
               minigraf-ffi/android/build/outputs/aar/minigraf-android-release.aar
            cd minigraf-ffi/android
            ./gradlew publishReleasePublicationToGitHubPackagesRepository
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/mobile.yml
  git commit -m "ci: add mobile.yml for Android/iOS cross-compilation and release upload"
  ```

---

## Task 13: Update `ROADMAP.md`

**Files:**
- Modify: `ROADMAP.md`

Add post-1.0 FFI items to the Phase 9 section (or create a subsection in Phase 8.3/9) noting that UDF registration and prepared query support over UniFFI are deferred.

- [ ] **Step 1: Find the Phase 8.3 or Phase 9 section in `ROADMAP.md`**

  Search for `Phase 8.3` or `Phase 9`. Add the following note under the language bindings section. Locate the line containing `Phase 8.3 Language Bindings` and add after its feature list:

  ```markdown
  **Post-1.0 FFI features (deferred from 8.2):**
  - 🎯 `register_aggregate` / `register_predicate` over UniFFI — requires closure-passing across FFI (not supported by UniFFI 0.28); needs a callback-based redesign
  - 🎯 `prepare()` / `PreparedQuery` over FFI — bind-slot substitution requires a stateful handle; design TBD once basic FFI API is proven stable
  ```

- [ ] **Step 2: Update the phase completion history at the bottom of ROADMAP.md**

  Find the timeline section and add (do not fill in until the phase is complete):

  ```markdown
  - 🔄 Phase 8.2: In progress — Mobile bindings (Android .aar + iOS .xcframework via UniFFI)
  ```

- [ ] **Step 3: Commit**

  ```bash
  git add ROADMAP.md
  git commit -m "docs(roadmap): add Phase 8.2 in-progress entry and post-1.0 FFI deferral notes"
  ```

---

## Self-Review Checklist

After all tasks are complete, verify:

- [ ] `cargo test` passes with 806+ tests (795 core + 11 FFI)
- [ ] `cargo check -p minigraf-ffi` succeeds on the CI matrix (ubuntu/macos/windows)
- [ ] `release.yml` has no references to `build-wasm-wasi` or `build-wasm-browser`
- [ ] `Package.swift` is at the repo root
- [ ] `minigraf-ffi` has `publish = false` in its `Cargo.toml`
- [ ] `cargo publish --locked -p minigraf` is correctly scoped in `release.yml`
- [ ] All new workflow files have `permissions: contents: write` where needed
- [ ] `ROADMAP.md` lists post-1.0 FFI deferrals

---

## Phase Completion Docs (after all CI is green on a real tag)

Once the first tagged release with mobile artifacts is confirmed green, update:
- `CLAUDE.md` — bump test count to 806+
- `ROADMAP.md` — mark Phase 8.2 complete with date and version
- `CHANGELOG.md` — add Phase 8.2 entry
- `.wiki/Architecture.md` — add `minigraf-ffi` crate to the module structure
- `.wiki/Datalog-Reference.md` — no changes needed (Datalog itself unchanged)
- Tag: `git tag -a v0.21.0 -m "Phase 8.2 complete — Android/iOS mobile bindings via UniFFI"`
