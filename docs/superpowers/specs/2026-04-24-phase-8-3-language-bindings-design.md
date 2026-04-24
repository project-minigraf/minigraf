# Phase 8.3: Language Bindings Design

**Date**: 2026-04-24
**Status**: Approved

---

## Goal

Extend Minigraf's reach beyond native Rust, mobile (Phase 8.2), and WASM (Phase 8.1) to four additional language ecosystems: Python, Java (desktop JVM), C, and Node.js. Each sub-phase ships an independently usable package to its ecosystem's canonical registry.

---

## Sub-phases and Versions

| Sub-phase | Language | Toolchain | Distribution | Version |
|-----------|----------|-----------|--------------|---------|
| 8.3a | Python | UniFFI + maturin | PyPI (`minigraf`) | v0.22.0 |
| 8.3b | Java desktop | UniFFI + Gradle | Maven Central (`io.github.adityamukho:minigraf-jvm`) | v0.23.0 |
| 8.3c | C | cbindgen | GitHub Releases tarballs | v0.24.0 |
| 8.3d | Node.js | napi-rs | npm (`minigraf`) | v0.25.0 |

Sub-phases are executed in order. Each gets its own PR, CI workflow, and release tag.

---

## Section 1: Repository Structure

### Workspace layout after Phase 8.3

```
minigraf/
  Cargo.toml                    ← workspace: [minigraf, minigraf-ffi, minigraf-c, minigraf-node]
  src/                          ← core library (unchanged)
  minigraf-ffi/                 ← UniFFI hub (all UniFFI-based bindings)
    Cargo.toml
    src/lib.rs                  ← #[uniffi::export] wrappers (unchanged from Phase 8.2)
    android/                    ← Phase 8.2, unchanged
    python/                     ← NEW (8.3a): maturin project
      pyproject.toml
      minigraf/
        __init__.py
      tests/
        test_basic.py
    java/                       ← NEW (8.3b): Gradle desktop JVM project
      build.gradle.kts
      settings.gradle.kts
  minigraf-c/                   ← NEW (8.3c): thin C API crate
    Cargo.toml
    src/lib.rs
    cbindgen.toml
    include/minigraf.h          ← generated, committed for reference
  minigraf-node/                ← NEW (8.3d): napi-rs Node.js addon crate
    Cargo.toml
    src/lib.rs
    package.json
    index.js
```

`minigraf-ffi` is the UniFFI hub — Android (8.2), Python (8.3a), and Java desktop (8.3b) all live under it because they share the same `#[uniffi::export]` Rust wrappers. `minigraf-c` and `minigraf-node` are separate crates because they use fundamentally different FFI mechanisms with conflicting `crate-type` requirements.

---

## Section 2: Phase 8.3a — Python

### Approach

`maturin` with UniFFI support: compiles `minigraf-ffi`, runs `uniffi-bindgen generate --language python`, and bundles the generated `.py` file + compiled shared library into a platform wheel. No new Rust code is required.

### `minigraf-ffi/python/pyproject.toml`

```toml
[build-system]
requires = ["maturin>=1.5"]
build-backend = "maturin"

[project]
name = "minigraf"
version = "0.22.0"

[tool.maturin]
manifest-path = "../Cargo.toml"   # points at minigraf-ffi crate
bindings = "uniffi"
python-packages = ["minigraf"]
```

### `minigraf-ffi/python/minigraf/__init__.py`

Re-exports from the generated module so users write `from minigraf import MiniGrafDb` rather than the generated module name.

### PyPI package name

`minigraf` — matches the crates.io package name.

### CI workflow (`python-release.yml`)

Triggered on release tag. Build matrix:

| Runner | Target | Notes |
|--------|--------|-------|
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` | manylinux2014 via `maturin build --manylinux 2014` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | manylinux2014 aarch64 |
| `macos-14` | `universal2` | arm64 + x86_64 lipo'd |
| `windows-latest` | `x86_64-pc-windows-msvc` | |

Each runner calls `maturin build --release`, uploads wheel as artifact. Final job calls `maturin publish` to PyPI using a stored `PYPI_API_TOKEN` secret.

### PR tests

Test matrix runs on all four platforms (Linux x86_64, Linux aarch64, macOS, Windows). Each runner installs the package via `maturin develop` and runs `pytest minigraf-ffi/python/tests/test_basic.py`.

`tests/test_basic.py` covers:
- Open in-memory db
- Transact a fact, query it back, assert result
- Open file-backed db, checkpoint, re-open, assert persistence
- Invalid Datalog raises an exception

---

## Section 3: Phase 8.3b — Java (Desktop JVM)

### How it differs from Android

Android ships `.aar` (bundled `.so` per ABI + generated Kotlin sources). Desktop JVM needs a plain JAR with:
- UniFFI-generated Kotlin/Java sources (identical to Android — same `MiniGrafDb`, `MiniGrafError` classes)
- Platform-native libraries embedded under `natives/<os>/<arch>/` and loaded at runtime via `System.load()`

This is the standard pattern used by SQLite JDBC, DuckDB JDBC, and others.

### Native targets

| Platform | Rust target |
|----------|-------------|
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Linux aarch64 | `aarch64-unknown-linux-gnu` |
| macOS universal2 | `aarch64-apple-darwin` + `x86_64-apple-darwin` (lipo'd) |
| Windows x86_64 | `x86_64-pc-windows-msvc` |

CI already cross-compiles `libminigraf_ffi` for Android ABIs (Phase 8.2); desktop adds these four targets.

### `NativeLoader.kt`

A small hand-written helper (not generated by UniFFI) that extracts the correct native library from the JAR's classpath resources at runtime based on `os.name` + `os.arch`, writes it to a temp file, and calls `System.load()`. Written once, not regenerated.

### `minigraf-ffi/java/build.gradle.kts` responsibilities

- Run `uniffi-bindgen generate --language kotlin` → emit sources into `src/main/kotlin/`
- Copy compiled platform natives into `src/main/resources/natives/`
- Produce a single JAR with embedded natives
- Configure Maven Central publishing (Sonatype OSSRH) with GPG signing

### Maven coordinates

```
io.github.adityamukho:minigraf-jvm:<version>
```

### CI workflow (`java-release.yml`)

- Cross-compiles natives on Linux x86_64, Linux aarch64, macOS, Windows runners → uploads as artifacts
- Assembly job merges all native artifacts into the fat JAR
- Publishes to Maven Central via Sonatype OSSRH API using stored `OSSRH_USERNAME`, `OSSRH_PASSWORD`, and `GPG_SIGNING_KEY` secrets

### PR tests

Test matrix runs on all four platforms. Each runner builds and tests the Gradle project locally without publishing. Test sourceset covers:
- Open in-memory db, transact, query, assert result
- Open file-backed db, checkpoint, re-open, assert persistence
- Invalid Datalog throws `MiniGrafException`

---

## Section 4: Phase 8.3c — C

### `minigraf-c` crate

A new workspace crate with `crate-type = ["cdylib", "staticlib"]`. All public functions are `extern "C" #[no_mangle]`. All database state is hidden behind an opaque pointer. `cbindgen` generates `include/minigraf.h` from these declarations.

### C API surface

```c
/* Lifecycle */
MiniGrafDb *minigraf_open(const char *path);        /* NULL on error */
MiniGrafDb *minigraf_open_in_memory(void);
void        minigraf_close(MiniGrafDb *db);

/* Execute any Datalog string — returns JSON, caller must free */
char       *minigraf_execute(MiniGrafDb *db, const char *datalog);
void        minigraf_string_free(char *s);

/* Checkpoint */
int         minigraf_checkpoint(MiniGrafDb *db);    /* 0 = ok, -1 = error */

/* Last error — valid until next call on same db */
const char *minigraf_last_error(MiniGrafDb *db);
```

Memory contract mirrors SQLite's `sqlite3_exec` + `sqlite3_free`: `minigraf_execute` returns a heap-allocated JSON string owned by the caller; `minigraf_string_free` must be called to release it. All other memory is managed by the library.

### `minigraf-c/cbindgen.toml`

```toml
language = "C"
include_guard = "MINIGRAF_H"
sys_includes = ["stdint.h"]
documentation = true
```

`include/minigraf.h` is committed to the repository as a stable reference. CI regenerates it and fails if it drifts from the committed copy.

### GitHub Releases artifacts (`c-release.yml`)

```
minigraf-c-v0.24.0-linux-x86_64.tar.gz        → libminigraf.so + minigraf.h
minigraf-c-v0.24.0-linux-aarch64.tar.gz       → libminigraf.so + minigraf.h
minigraf-c-v0.24.0-macos-universal2.tar.gz    → libminigraf.dylib + minigraf.h
minigraf-c-v0.24.0-windows-x86_64.zip         → minigraf.dll + minigraf.h
```

### PR tests

Test matrix runs on all four platforms (Linux x86_64, Linux aarch64, macOS, Windows). `minigraf-c/tests/test_basic.c` is compiled and run via the `cc` crate in `build.rs`. Covers:
- Open in-memory db
- `minigraf_execute` transact → assert JSON contains `"transacted"`
- `minigraf_execute` query → assert JSON result matches expected value
- `minigraf_string_free` called on every returned string (verified clean by Valgrind on Linux in a nightly job)
- `minigraf_last_error` returns non-NULL after a parse error

---

## Section 5: Phase 8.3d — Node.js

### `minigraf-node` crate

A new workspace crate (`crate-type = ["cdylib"]`) using `napi-rs` proc-macros. `@napi-rs/cli` generates TypeScript definitions from the Rust types automatically.

### Rust API

```rust
#[napi]
pub struct MiniGrafDb { ... }

#[napi]
impl MiniGrafDb {
    #[napi(constructor)]
    pub fn new(path: String) -> Result<Self>;

    #[napi(factory)]
    pub fn in_memory() -> Result<Self>;

    #[napi]
    pub fn execute(&self, datalog: String) -> Result<String>;   // returns JSON

    #[napi]
    pub fn checkpoint(&self) -> Result<()>;
}
```

### Auto-generated `index.d.ts`

```typescript
export class MiniGrafDb {
  constructor(path: string)
  static inMemory(): MiniGrafDb
  execute(datalog: string): string
  checkpoint(): void
}
```

### npm package structure

`@napi-rs/cli` platform-specific optional packages pattern:

- `minigraf` — main package; contains `index.js` + `index.d.ts`; lists platform packages as `optionalDependencies`
- `@minigraf/linux-x64-gnu` — prebuilt `.node` for Linux x86_64
- `@minigraf/linux-arm64-gnu` — prebuilt `.node` for Linux aarch64
- `@minigraf/darwin-universal` — prebuilt `.node` for macOS universal2
- `@minigraf/win32-x64-msvc` — prebuilt `.node` for Windows x86_64

Users run `npm install minigraf`; npm resolves the correct platform binary automatically via `optionalDependencies`. No build step required on the consumer side.

### CI workflow (`node-release.yml`)

Builds `.node` binaries on Linux x86_64, Linux aarch64, macOS (universal2), Windows runners → uploads artifacts → assembly job publishes all platform packages to npm, then publishes the main `minigraf` package using a stored `NPM_TOKEN` secret.

### PR tests

Test matrix runs on all four platforms. Each runner builds the addon with `napi build --platform` and runs `node --test minigraf-node/test/basic.test.mjs`. Covers:
- Import `minigraf`, open in-memory db
- Transact a fact, query it back, assert JSON result
- Open file-backed db, checkpoint, re-open, assert persistence
- Invalid Datalog throws a JS `Error`

---

## Deferred to Post-1.0

Per the Phase 8.2 design precedent, the following are not in scope for 8.3:

- `register_aggregate` / `register_predicate` over UniFFI (requires closure-passing across FFI, not supported by UniFFI 0.31.1)
- `prepare()` / `PreparedQuery` over UniFFI (stateful handle design TBD)
- Conan / vcpkg ports for the C library (community-maintainable once traction is established)
- Python type stubs beyond what `maturin` auto-generates
- Desktop JVM Kotlin-specific ergonomics (extension functions, coroutines)
