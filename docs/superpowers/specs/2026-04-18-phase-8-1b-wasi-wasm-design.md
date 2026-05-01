# Phase 8.1b: Server-side WASM Support (wasm32-wasip1 / WASI) — Design Spec

**Date**: 2026-04-18
**Issue**: project-minigraf/minigraf#130
**Status**: Approved — ready for implementation

---

## Goal

Compile Minigraf to `wasm32-wasip1` (WASI) so it runs inside server-side WASM runtimes — Wasmtime, Wasmer, Cloudflare Workers (WASI), and Fastly Compute — with a working in-memory REPL and no JavaScript bridge required.

This is orthogonal to Phase 8.1a (browser WASM with IndexedDB). Both share the `wasm32` architecture but use different targets and storage models:
- **8.1a**: `wasm32-unknown-unknown` + `wasm-bindgen` → browser (IndexedDB)
- **8.1b**: `wasm32-wasip1` → server-side WASM runtimes (in-memory; file-backed is out of scope)

---

## Scope Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Storage backend | In-memory only (`MemoryBackend`) | File-backed requires threading `FileBackend`/WAL through new cfg gates across `db.rs`, `lib.rs`, `storage/`; adds significant complexity for a use case better served by native binaries. WASI value is as an embeddable library, not a persistent CLI. |
| Thread model | Single-threaded only — WASI p1 has no threads | No code changes needed; audit confirms existing code is safe (see Thread Audit below) |
| UUID/`getrandom` | Fix: restrict `uuid` `js` feature to browser wasm only | WASI supports `getrandom` natively; `js` feature would fail at runtime under WASI |
| REPL | Enable for WASI via `#[cfg(target_os = "wasi")]` | Allows smoke-testing via piped stdin |
| Documentation | Expand `.wiki/Use-Cases.md`; update `README.md` | Convention: detailed docs in wiki, brief mention in README; no `docs/wasi.md` |

---

## Thread Safety Audit

**Result: no code changes required.**

WASI p1 enforces single-threading at the runtime level. The library itself does not spawn threads. Specifically:

- `Arc<Mutex<...>>` / `RwLock` — compile as single-threaded stubs under WASI; safe
- `thread_local! { WRITE_TX_ACTIVE }` in `db.rs` — works under WASI (single-thread global)
- `std::thread::spawn` — only in tests/benchmarks, already gated behind `#[cfg(not(target_arch = "wasm32"))]` dev-dependencies (`tempfile`, `criterion`)
- `FileBackend` / WAL — already excluded from all `wasm32` targets by existing `#[cfg(not(target_arch = "wasm32"))]` gates; no new exposure

The spec and `docs/wasi.md` (replaced by wiki section) should note that concurrent `Minigraf` instances from multiple threads are not supported under WASI, enforced by the runtime.

---

## cfg Strategy

Two distinct `wasm32` sub-targets require different cfg expressions going forward:

| Target | `target_arch` | `target_os` | Expression |
|---|---|---|---|
| Browser | `wasm32` | `unknown` | `cfg(all(target_arch = "wasm32", not(target_os = "wasi")))` |
| WASI | `wasm32` | `wasi` | `cfg(target_os = "wasi")` |
| Native | other | other | `cfg(not(target_arch = "wasm32"))` |

Only two files need new cfg branches: `Cargo.toml` and `src/main.rs`.

---

## Changes Required

### 1. `Cargo.toml`

The existing `[target.'cfg(target_arch = "wasm32")'.dependencies]` block applies to all wasm32 targets, including WASI. The `js` feature on `uuid` activates `getrandom`'s JavaScript path, which fails at runtime under WASI (no JS engine).

**Change**: Split the wasm32 dependency block into browser-only vs. WASI-safe:

```toml
# Browser WASM only (wasm32-unknown-unknown) — needs JS APIs
[target.'cfg(all(target_arch = "wasm32", not(target_os = "wasi")))'.dependencies]
uuid                 = { version = "1.0", features = ["v4", "v5", "serde", "js"] }
wasm-bindgen         = { version = "0.2", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
js-sys               = { version = "0.3", optional = true }
web-sys              = { version = "0.3", optional = true, features = [ ... ] }
```

WASI uses the base `[dependencies]` `uuid` (no `js` feature). `getrandom` 0.2 supports WASI natively via `wasi_snapshot_preview1::random_get` — no explicit `getrandom` dependency needed.

### 2. `src/main.rs`

Add a `#[cfg(target_os = "wasi")]` branch that runs the REPL with an in-memory database. The existing `#[cfg(not(target_arch = "wasm32"))]` branch (native, with optional `--file`) remains unchanged.

```rust
fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "wasi")]
    {
        use minigraf::Minigraf;
        let db = Minigraf::in_memory()?;
        db.repl().run();
        Ok(())
    }
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    {
        Ok(())  // browser — REPL not applicable
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // ... existing native code unchanged ...
    }
}
```

### 3. `src/db.rs` — WASM-gated test

Add at least one `#[cfg(all(target_os = "wasi", test))]` gated test verifying the in-memory backend compiles and works. Placed in the existing `#[cfg(test)]` section of `src/db.rs`:

```rust
#[cfg(all(target_os = "wasi", test))]
mod wasi_tests {
    use super::*;

    #[test]
    fn in_memory_smoke() {
        let db = Minigraf::in_memory().expect("open");
        db.execute(r#"(transact [{:db/id "e1" :name "hello"}])"#).expect("transact");
        let r = db.execute("(query [:find ?e :where [?e :name _]])").expect("query");
        assert!(!r.is_empty());
    }
}
```

**Why `target_os = "wasi"` and not `target_arch = "wasm32"`**: the broader `wasm32` gate would also match `wasm32-unknown-unknown` (browser), where `wasm-pack test` requires `#[wasm_bindgen_test]` instead of `#[test]`. A plain `#[test]` in that context is silently ignored or panics. WASI-specific gating keeps the two targets cleanly separated.

This test IS run in CI. Wasmtime is already installed in the workflow, and Cargo supports a custom test runner via the `CARGO_TARGET_WASM32_WASIP1_RUNNER` environment variable:

```
CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --dir /tmp" cargo test --target wasm32-wasip1
```

Cargo invokes this as `wasmtime run --dir /tmp <test_binary.wasm> <test-args>`. The existing test suite causes no issues — `tempfile` and `criterion` are already gated behind `#[cfg(not(target_arch = "wasm32"))]` dev-dependencies and will not appear in the WASI test binary. Only the new `wasi_tests` module (and any other wasm32-compatible tests) will run.

### 4. `.github/workflows/wasm-wasi.yml` — rewrite

Single job (build + smoke test together). No artifact upload/download. Installs Wasmtime and Wasmer, runs transact + query smoke test against each.

```yaml
name: WASM WASI

on:
  push:
    branches: ["main"]
  pull_request:
    branches: ["main"]

env:
  CARGO_TERM_COLOR: always
  WASMTIME_VERSION: v33.0.0
  WASMER_VERSION: v5.0.4

permissions:
  contents: read

jobs:
  wasm-wasi:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1

      - name: Build (release)
        run: cargo build --target wasm32-wasip1 --release --bin minigraf

      - name: Install Wasmtime
        run: |
          curl -sSfL \
            "https://github.com/bytecodealliance/wasmtime/releases/download/${WASMTIME_VERSION}/wasmtime-${WASMTIME_VERSION}-x86_64-linux.tar.xz" \
            -o /tmp/wasmtime.tar.xz
          tar -xf /tmp/wasmtime.tar.xz -C /tmp
          echo "/tmp/wasmtime-${WASMTIME_VERSION}-x86_64-linux" >> "$GITHUB_PATH"

      - name: Run WASI tests
        run: cargo test --target wasm32-wasip1
        env:
          CARGO_TARGET_WASM32_WASIP1_RUNNER: wasmtime run --dir /tmp

      - name: Smoke test (Wasmtime — transact)
        run: |
          echo '(transact [{:db/id "eid-1" :name "Alice"}])' | \
            wasmtime run --dir /tmp \
              target/wasm32-wasip1/release/minigraf.wasm

      - name: Smoke test (Wasmtime — query)
        run: |
          printf '(transact [{:db/id "eid-1" :name "Alice"}])\n(query [:find ?e :where [?e :name _]])\n' | \
            wasmtime run --dir /tmp \
              target/wasm32-wasip1/release/minigraf.wasm

      - name: Install Wasmer
        run: |
          curl -sSfL \
            "https://github.com/wasmerio/wasmer/releases/download/${WASMER_VERSION}/wasmer-linux-amd64.tar.gz" \
            -o /tmp/wasmer.tar.gz
          tar -xf /tmp/wasmer.tar.gz -C /tmp wasmer/bin/wasmer
          echo "/tmp/wasmer/bin" >> "$GITHUB_PATH"

      - name: Smoke test (Wasmer — transact + query)
        run: |
          printf '(transact [{:db/id "eid-1" :name "Alice"}])\n(query [:find ?e :where [?e :name _]])\n' | \
            wasmer run --dir /tmp \
              target/wasm32-wasip1/release/minigraf.wasm
```

### 5. Documentation

- **`docs/wasi.md`**: Delete (not committed). Content moves to wiki.
- **`.wiki/Use-Cases.md`**: Expand the existing WASI subsection with:
  - Build command
  - Running with Wasmtime and Wasmer (with `--dir` capability model explanation)
  - In-memory-only constraint and rationale
  - Embedding guide (Wasmtime Rust API, Wasmer Go API)
  - Cloudflare Workers / Fastly Compute notes
  - Single-threaded constraint (WASI p1)
- **`README.md`**: Update the WASI mention from "Phase 8 target" to "Phase 8.1b complete"

---

## What Does NOT Change

- `src/lib.rs` — no new exports needed
- `src/db.rs` — no cfg gate changes (FileBackend/WAL remain `#[cfg(not(target_arch = "wasm32"))]`)
- `src/storage/` — no changes
- Native `cargo test` — unaffected
- Browser `wasm-pack build --features browser` — unaffected; browser deps now correctly scoped to non-WASI wasm32

---

## Definition of Done

1. `cargo build --target wasm32-wasip1 --release --bin minigraf` succeeds with no errors
2. Wasmtime smoke test passes (transact + query) in CI
3. Wasmer smoke test passes (transact + query) in CI
4. `cargo test` (native) still passes
5. WASI section in `.wiki/Use-Cases.md` documents usage
6. README updated to reflect phase completion
