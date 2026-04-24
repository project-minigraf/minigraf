# Phase 8.3c: C Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a minimal, stable C API for Minigraf as platform tarballs on GitHub Releases — a `minigraf-c` workspace crate with a `cbindgen`-generated `minigraf.h` header.

**Architecture:** A new `minigraf-c` workspace crate (`cdylib` + `staticlib`) wraps the `minigraf` core with a deliberately minimal C-friendly API (opaque pointer + `minigraf_open` / `minigraf_execute` / `minigraf_close` etc.). `cbindgen` generates `include/minigraf.h`, which is committed as a stable reference. Tests are written as Rust unit tests in `src/lib.rs` (calling the `pub extern "C"` functions directly) — no cross-language test binary required. GitHub Releases artifacts contain the prebuilt shared/static library + header for each platform.

**Tech Stack:** Rust `extern "C"` + `#[no_mangle]`, `cbindgen` 0.27, `minigraf` (workspace dep).

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| MODIFY | `Cargo.toml` (root) | Add `minigraf-c` to workspace members |
| CREATE | `minigraf-c/Cargo.toml` | Crate metadata, deps |
| CREATE | `minigraf-c/src/lib.rs` | C API implementation + unit tests |
| CREATE | `minigraf-c/cbindgen.toml` | cbindgen configuration |
| CREATE | `minigraf-c/include/minigraf.h` | Generated header (committed for reference) |
| CREATE | `.github/workflows/c-ci.yml` | PR tests (4 platforms, `cargo test`) |
| CREATE | `.github/workflows/c-release.yml` | Release: compile + package tarballs + upload to GitHub Releases |
| MODIFY | `Cargo.toml` (root) | Bump version to `0.24.0` |
| MODIFY | `CHANGELOG.md` | Add 8.3c entry |
| MODIFY | `ROADMAP.md` | Mark 8.3c complete |

---

## Task 1: Add `minigraf-c` to workspace and create crate

**Files:**
- Modify: `Cargo.toml` (root)
- Create: `minigraf-c/Cargo.toml`

- [ ] **Step 1: Add `minigraf-c` to the workspace**

In `Cargo.toml` (root), change:
```toml
[workspace]
members = [".", "minigraf-ffi"]
```
to:
```toml
[workspace]
members = [".", "minigraf-ffi", "minigraf-c"]
```

- [ ] **Step 2: Create `minigraf-c/Cargo.toml`**

```toml
[package]
name = "minigraf-c"
version = "0.24.0"
edition = "2024"
description = "C bindings for Minigraf — stable C API with cbindgen-generated header"
publish = false

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
minigraf = { path = ".." }
serde_json = "1.0"
```

- [ ] **Step 3: Create placeholder `minigraf-c/src/lib.rs`**

```rust
// Implementation in Task 2
```

- [ ] **Step 4: Verify workspace compiles**

```bash
cargo check -p minigraf-c
```

Expected: compiles (empty lib, no errors).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml minigraf-c/
git commit -m "feat(c): scaffold minigraf-c workspace crate"
```

---

## Task 2: Implement the C API

**Files:**
- Modify: `minigraf-c/src/lib.rs`

- [ ] **Step 1: Write `minigraf-c/src/lib.rs`**

```rust
#![deny(unsafe_op_in_unsafe_fn)]

use minigraf::{QueryResult, Value};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::Mutex;

// ─── Handle ──────────────────────────────────────────────────────────────────

pub struct MiniGrafDb {
    db: Mutex<minigraf::Minigraf>,
    last_error: Mutex<Option<CString>>,
}

impl MiniGrafDb {
    fn set_error(&self, msg: String) {
        *self.last_error.lock().unwrap() =
            Some(CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap()));
    }

    fn clear_error(&self) {
        *self.last_error.lock().unwrap() = None;
    }
}

// ─── Lifecycle ────────────────────────────────────────────────────────────────

/// Open a file-backed Minigraf database. Returns NULL on error.
#[no_mangle]
pub extern "C" fn minigraf_open(path: *const c_char) -> *mut MiniGrafDb {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let path = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    match minigraf::Minigraf::open(path) {
        Ok(db) => Box::into_raw(Box::new(MiniGrafDb {
            db: Mutex::new(db),
            last_error: Mutex::new(None),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Open an in-memory Minigraf database. Returns NULL on error.
#[no_mangle]
pub extern "C" fn minigraf_open_in_memory() -> *mut MiniGrafDb {
    match minigraf::Minigraf::in_memory() {
        Ok(db) => Box::into_raw(Box::new(MiniGrafDb {
            db: Mutex::new(db),
            last_error: Mutex::new(None),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Close a database and free all associated memory.
#[no_mangle]
pub extern "C" fn minigraf_close(handle: *mut MiniGrafDb) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

// ─── Execute ─────────────────────────────────────────────────────────────────

/// Execute a Datalog string. Returns a JSON string on success (caller must free
/// with `minigraf_string_free`), or NULL on error (call `minigraf_last_error`).
#[no_mangle]
pub extern "C" fn minigraf_execute(
    handle: *mut MiniGrafDb,
    datalog: *const c_char,
) -> *mut c_char {
    if handle.is_null() || datalog.is_null() {
        return std::ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let datalog = match unsafe { CStr::from_ptr(datalog) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            handle.set_error("invalid UTF-8 in datalog string".into());
            return std::ptr::null_mut();
        }
    };

    let result = handle.db.lock().unwrap().execute(datalog);
    match result {
        Ok(qr) => {
            handle.clear_error();
            let json = query_result_to_json(qr);
            match CString::new(json) {
                Ok(s) => s.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        }
        Err(e) => {
            handle.set_error(format!("{e:#}"));
            std::ptr::null_mut()
        }
    }
}

/// Free a string returned by `minigraf_execute`.
#[no_mangle]
pub extern "C" fn minigraf_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

// ─── Checkpoint ───────────────────────────────────────────────────────────────

/// Flush the WAL to the database file. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn minigraf_checkpoint(handle: *mut MiniGrafDb) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &*handle };
    match handle.db.lock().unwrap().checkpoint() {
        Ok(_) => {
            handle.clear_error();
            0
        }
        Err(e) => {
            handle.set_error(format!("{e:#}"));
            -1
        }
    }
}

// ─── Error ────────────────────────────────────────────────────────────────────

/// Return the last error message. Valid until the next call on the same handle.
/// Returns NULL if no error has occurred.
#[no_mangle]
pub extern "C" fn minigraf_last_error(handle: *mut MiniGrafDb) -> *const c_char {
    if handle.is_null() {
        return std::ptr::null();
    }
    let handle = unsafe { &*handle };
    let guard = handle.last_error.lock().unwrap();
    match guard.as_ref() {
        Some(s) => s.as_ptr(),
        None => std::ptr::null(),
    }
}

// ─── JSON helpers ─────────────────────────────────────────────────────────────

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::String(s) => J::String(s.clone()),
        Value::Integer(i) => serde_json::json!(i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Boolean(b) => J::Bool(*b),
        Value::Ref(u) => J::String(u.to_string()),
        Value::Keyword(k) => J::String(k.clone()),
        Value::Null => J::Null,
    }
}

fn query_result_to_json(result: QueryResult) -> String {
    let val = match result {
        QueryResult::Transacted(tx) => serde_json::json!({"transacted": tx}),
        QueryResult::Retracted(tx) => serde_json::json!({"retracted": tx}),
        QueryResult::Ok => serde_json::json!({"ok": true}),
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<serde_json::Value>> =
                results.iter().map(|r| r.iter().map(value_to_json).collect()).collect();
            serde_json::json!({"variables": vars, "results": rows})
        }
    };
    val.to_string()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_returns_non_null() {
        let db = minigraf_open_in_memory();
        assert!(!db.is_null());
        minigraf_close(db);
    }

    #[test]
    fn execute_transact_returns_json() {
        let db = minigraf_open_in_memory();
        let datalog = CString::new(r#"(transact [[:alice :name "Alice"]])"#).unwrap();
        let result = minigraf_execute(db, datalog.as_ptr());
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(s.contains("transacted"), "expected transacted in: {s}");
        minigraf_string_free(result);
        minigraf_close(db);
    }

    #[test]
    fn execute_query_returns_results() {
        let db = minigraf_open_in_memory();
        let tx = CString::new(r#"(transact [[:alice :name "Alice"]])"#).unwrap();
        let r = minigraf_execute(db, tx.as_ptr());
        assert!(!r.is_null());
        minigraf_string_free(r);

        let q = CString::new("(query [:find ?n :where [?e :name ?n]])").unwrap();
        let result = minigraf_execute(db, q.as_ptr());
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(s.contains("Alice"), "expected Alice in: {s}");
        minigraf_string_free(result);
        minigraf_close(db);
    }

    #[test]
    fn execute_invalid_datalog_returns_null_and_sets_error() {
        let db = minigraf_open_in_memory();
        let bad = CString::new("not valid datalog !!!").unwrap();
        let result = minigraf_execute(db, bad.as_ptr());
        assert!(result.is_null(), "expected NULL for invalid datalog");

        let err = minigraf_last_error(db);
        assert!(!err.is_null(), "expected non-NULL error");
        let msg = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert!(!msg.is_empty(), "expected non-empty error message");
        minigraf_close(db);
    }

    #[test]
    fn checkpoint_returns_zero_on_success() {
        let db = minigraf_open_in_memory();
        let rc = minigraf_checkpoint(db);
        assert_eq!(rc, 0);
        minigraf_close(db);
    }

    #[test]
    fn string_free_null_is_safe() {
        // Should not panic or crash
        minigraf_string_free(std::ptr::null_mut());
    }

    #[test]
    fn close_null_is_safe() {
        minigraf_close(std::ptr::null_mut());
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p minigraf-c -- --nocapture
```

Expected: 7 tests pass.

- [ ] **Step 3: Commit**

```bash
git add minigraf-c/src/lib.rs
git commit -m "feat(c): implement C API with unit tests (minigraf_open, execute, checkpoint, close)"
```

---

## Task 3: Generate and commit the header

**Files:**
- Create: `minigraf-c/cbindgen.toml`
- Create: `minigraf-c/include/minigraf.h`

- [ ] **Step 1: Install cbindgen**

```bash
cargo install cbindgen
```

- [ ] **Step 2: Create `minigraf-c/cbindgen.toml`**

```toml
language = "C"
include_guard = "MINIGRAF_H"
sys_includes = ["stdint.h"]
documentation = true
documentation_style = "c99"
no_includes = false

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

- [ ] **Step 3: Generate the header**

```bash
mkdir -p minigraf-c/include
cbindgen --config minigraf-c/cbindgen.toml \
         --crate minigraf-c \
         --output minigraf-c/include/minigraf.h
```

Inspect the output. Expected `minigraf-c/include/minigraf.h`:

```c
#ifndef MINIGRAF_H
#define MINIGRAF_H

#include <stdint.h>

typedef struct MiniGrafDb MiniGrafDb;

/**
 * Open a file-backed Minigraf database. Returns NULL on error.
 */
MiniGrafDb *minigraf_open(const char *path);

/**
 * Open an in-memory Minigraf database. Returns NULL on error.
 */
MiniGrafDb *minigraf_open_in_memory(void);

/**
 * Close a database and free all associated memory.
 */
void minigraf_close(MiniGrafDb *db);

/**
 * Execute a Datalog string. Returns a JSON string on success (caller must free
 * with `minigraf_string_free`), or NULL on error (call `minigraf_last_error`).
 */
char *minigraf_execute(MiniGrafDb *db, const char *datalog);

/**
 * Free a string returned by `minigraf_execute`.
 */
void minigraf_string_free(char *s);

/**
 * Flush the WAL to the database file. Returns 0 on success, -1 on error.
 */
int minigraf_checkpoint(MiniGrafDb *db);

/**
 * Return the last error message. Valid until the next call on the same handle.
 * Returns NULL if no error has occurred.
 */
const char *minigraf_last_error(MiniGrafDb *db);

#endif /* MINIGRAF_H */
```

If the generated file looks different, update `cbindgen.toml` to match the expected output.

- [ ] **Step 4: Commit**

```bash
git add minigraf-c/cbindgen.toml minigraf-c/include/minigraf.h
git commit -m "feat(c): add cbindgen config and generated minigraf.h header"
```

---

## Task 4: Add PR CI workflow

**Files:**
- Create: `.github/workflows/c-ci.yml`

- [ ] **Step 1: Create `.github/workflows/c-ci.yml`**

```yaml
name: C CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

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

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Run tests
        run: cargo test -p minigraf-c -- --nocapture

  check-header-drift:
    name: Verify minigraf.h is up to date
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Install cbindgen
        run: cargo install cbindgen

      - name: Regenerate header
        run: |
          cbindgen --config minigraf-c/cbindgen.toml \
                   --crate minigraf-c \
                   --output /tmp/minigraf_generated.h

      - name: Fail if header has drifted
        run: |
          diff minigraf-c/include/minigraf.h /tmp/minigraf_generated.h || \
          (echo "minigraf.h is out of date — run cbindgen locally and commit the result" && exit 1)
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/c-ci.yml
git commit -m "ci(c): add PR test matrix + header drift check (4 platforms)"
```

---

## Task 5: Add release workflow

**Files:**
- Create: `.github/workflows/c-release.yml`

- [ ] **Step 1: Create `.github/workflows/c-release.yml`**

```yaml
name: C Release

on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string

jobs:
  build:
    name: Build C library (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact-name: linux-x86_64
            lib-name: libminigraf.so
            src-lib: libminigraf_c.so
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            artifact-name: linux-aarch64
            lib-name: libminigraf.so
            src-lib: libminigraf_c.so
          - os: macos-14
            target: universal2
            artifact-name: macos-universal2
            lib-name: libminigraf.dylib
            src-lib: libminigraf_c.dylib
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact-name: windows-x86_64
            lib-name: minigraf.dll
            src-lib: minigraf_c.dll

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target == 'universal2' && 'aarch64-apple-darwin x86_64-apple-darwin' || matrix.target }}

      - name: Build (universal2)
        if: matrix.target == 'universal2'
        run: |
          cargo build --release -p minigraf-c --target aarch64-apple-darwin
          cargo build --release -p minigraf-c --target x86_64-apple-darwin
          lipo -create \
            target/aarch64-apple-darwin/release/libminigraf_c.dylib \
            target/x86_64-apple-darwin/release/libminigraf_c.dylib \
            -output ${{ matrix.lib-name }}

      - name: Build (other)
        if: matrix.target != 'universal2'
        run: cargo build --release -p minigraf-c --target ${{ matrix.target }}

      - name: Rename and package (unix)
        if: runner.os != 'Windows' && matrix.target != 'universal2'
        run: |
          cp target/${{ matrix.target }}/release/${{ matrix.src-lib }} ${{ matrix.lib-name }}
          tar czf minigraf-c-${{ inputs.tag }}-${{ matrix.artifact-name }}.tar.gz \
            ${{ matrix.lib-name }} minigraf-c/include/minigraf.h

      - name: Rename and package (universal2)
        if: matrix.target == 'universal2'
        run: |
          tar czf minigraf-c-${{ inputs.tag }}-${{ matrix.artifact-name }}.tar.gz \
            ${{ matrix.lib-name }} minigraf-c/include/minigraf.h

      - name: Rename and package (windows)
        if: runner.os == 'Windows'
        run: |
          copy target\${{ matrix.target }}\release\${{ matrix.src-lib }} ${{ matrix.lib-name }}
          Compress-Archive -Path ${{ matrix.lib-name }},minigraf-c\include\minigraf.h `
            -DestinationPath minigraf-c-${{ inputs.tag }}-${{ matrix.artifact-name }}.zip
        shell: pwsh

      - name: Upload artifact (unix)
        if: runner.os != 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: c-release-${{ matrix.artifact-name }}
          path: minigraf-c-${{ inputs.tag }}-${{ matrix.artifact-name }}.tar.gz

      - name: Upload artifact (windows)
        if: runner.os == 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: c-release-${{ matrix.artifact-name }}
          path: minigraf-c-${{ inputs.tag }}-${{ matrix.artifact-name }}.zip

  upload-to-release:
    name: Upload artifacts to GitHub Release
    needs: build
    runs-on: ubuntu-latest

    steps:
      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          pattern: c-release-*
          path: artifacts
          merge-multiple: true

      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: ${{ inputs.tag }}
          files: artifacts/*
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/c-release.yml
git commit -m "ci(c): add release workflow — platform tarballs uploaded to GitHub Releases"
```

---

## Task 6: Bump version and update docs

**Files:**
- Modify: `Cargo.toml` (root)
- Modify: `minigraf-c/Cargo.toml`
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`

- [ ] **Step 1: Bump versions to `0.24.0`**

In `Cargo.toml` (root): `version = "0.24.0"`
In `minigraf-c/Cargo.toml`: `version = "0.24.0"`

- [ ] **Step 2: Run `cargo check`**

```bash
cargo check --workspace
```

Expected: no errors.

- [ ] **Step 3: Add CHANGELOG entry**

```markdown
## [0.24.0] — 2026-04-XX

### Added
- **Phase 8.3c**: C bindings distributed as GitHub Releases tarballs.
  Download `minigraf-c-v0.24.0-<platform>.tar.gz` from the release page.
  Includes `libminigraf.so`/`.dylib`/`.dll` + `minigraf.h`.
  API: `minigraf_open`, `minigraf_open_in_memory`, `minigraf_execute`,
  `minigraf_string_free`, `minigraf_checkpoint`, `minigraf_close`,
  `minigraf_last_error`.
```

- [ ] **Step 4: Mark 8.3c complete in ROADMAP.md**

- [ ] **Step 5: Commit and tag**

```bash
git add Cargo.toml minigraf-c/Cargo.toml CHANGELOG.md ROADMAP.md
git commit -m "chore(release): bump version to v0.24.0 — Phase 8.3c C bindings"
git tag -a v0.24.0 -m "Phase 8.3c complete — C bindings on GitHub Releases"
git push origin v0.24.0
```
