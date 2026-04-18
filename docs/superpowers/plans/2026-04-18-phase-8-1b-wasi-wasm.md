# Phase 8.1b: WASI WASM Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Minigraf compile and run as a `wasm32-wasip1` binary with a working in-memory REPL, tested by Wasmtime and Wasmer in CI.

**Architecture:** Three targeted code changes — restrict browser-specific `uuid` `js` feature to non-WASI wasm32 targets, enable the REPL in `main.rs` for WASI, and add a WASI-gated in-memory smoke test — plus a rewritten CI workflow and expanded wiki docs. No new storage backend; no changes to `db.rs` cfg gates.

**Tech Stack:** Rust (stable), `cargo`, `wasm32-wasip1` target, Wasmtime v33, Wasmer v5, GitHub Actions.

---

## File Map

| File | Action | What changes |
|---|---|---|
| `Cargo.toml` | Modify | Narrow `target_arch = "wasm32"` dep/dev-dep blocks to exclude WASI |
| `src/main.rs` | Modify | Add `#[cfg(target_os = "wasi")]` branch running in-memory REPL |
| `src/db.rs` | Modify | Add `#[cfg(all(target_os = "wasi", test))]` smoke test at end of file |
| `.github/workflows/wasm-wasi.yml` | Replace | Single-job: build → WASI tests → Wasmtime smoke → Wasmer smoke |
| `.wiki/Use-Cases.md` | Modify | Expand WASI subsection with build/run/embed/constraints detail |
| `README.md` | Modify | Update WASI status from "Phase 8 target" to "Phase 8.1b complete" |
| `docs/wasi.md` | Delete | Wrong location; content moves to wiki |

---

## Task 1: Set up git worktree

**Files:** none (workspace setup only)

- [ ] **Step 1: Create worktree**

```bash
git worktree add .worktrees/phase-8-1b -b feat/phase-8-1b
cd .worktrees/phase-8-1b
```

- [ ] **Step 2: Verify clean state**

```bash
cargo test --quiet 2>&1 | tail -5
```

Expected: all tests pass (no failures).

---

## Task 2: Fix `Cargo.toml` — restrict browser-only deps to non-WASI wasm32

The `uuid` `js` feature activates `getrandom`'s JavaScript path. Under WASI (`wasm32-wasip1`), there is no JavaScript engine — UUID generation fails at runtime. All browser-only deps must be scoped to `cfg(all(target_arch = "wasm32", not(target_os = "wasi")))`.

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Verify the current state causes a runtime problem**

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release --bin minigraf 2>&1 | tail -20
```

Expected: compiles, but note the build pulls in `wasm-bindgen` (visible in `--verbose` output). The binary exits silently because `main.rs` returns `Ok(())` for all wasm32 targets. We'll fix both issues.

- [ ] **Step 2: Edit `Cargo.toml` — narrow the wasm32 dependency block**

Find the section (around line 34):

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
uuid                 = { version = "1.0", features = ["v4", "v5", "serde", "js"] }
wasm-bindgen         = { version = "0.2", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
js-sys               = { version = "0.3", optional = true }
web-sys              = { version = "0.3", optional = true, features = [
    "DomStringList",
    "Event",
    "IdbDatabase",
    "IdbFactory",
    "IdbIndex",
    "IdbObjectStore",
    "IdbOpenDbRequest",
    "IdbRequest",
    "IdbTransaction",
    "IdbTransactionMode",
    "Window",
] }
```

Replace with (change only the cfg expression on the section header — nothing else):

```toml
[target.'cfg(all(target_arch = "wasm32", not(target_os = "wasi")))'.dependencies]
uuid                 = { version = "1.0", features = ["v4", "v5", "serde", "js"] }
wasm-bindgen         = { version = "0.2", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
js-sys               = { version = "0.3", optional = true }
web-sys              = { version = "0.3", optional = true, features = [
    "DomStringList",
    "Event",
    "IdbDatabase",
    "IdbFactory",
    "IdbIndex",
    "IdbObjectStore",
    "IdbOpenDbRequest",
    "IdbRequest",
    "IdbTransaction",
    "IdbTransactionMode",
    "Window",
] }
```

- [ ] **Step 3: Edit `Cargo.toml` — narrow the wasm32 dev-dependency block**

Find (around line 60):

```toml
[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
wasm-bindgen-test = "0.3"
```

Replace with:

```toml
[target.'cfg(all(target_arch = "wasm32", not(target_os = "wasi")))'.dev-dependencies]
wasm-bindgen-test = "0.3"
```

`wasm-bindgen-test` is a browser test harness; it must not appear in the WASI build.

- [ ] **Step 4: Verify native build still works**

```bash
cargo build --quiet
```

Expected: no errors, no warnings about changed deps.

- [ ] **Step 5: Verify WASI build still compiles (no wasm-bindgen imports)**

```bash
cargo build --target wasm32-wasip1 --release --bin minigraf --verbose 2>&1 | grep -E "wasm.bindgen|error"
```

Expected: no `wasm-bindgen` in the dependency list, no errors.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml
git commit -m "fix(deps): restrict uuid js feature and wasm-bindgen-test to browser wasm only

wasm32-wasip1 (WASI) has no JavaScript engine. The previous cfg gate
'target_arch = wasm32' applied to WASI too, pulling in wasm-bindgen and
getrandom's JS path. Narrowed to 'all(target_arch = wasm32, not(target_os = wasi))'.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: Enable REPL in `main.rs` for WASI targets

Currently `main.rs` returns `Ok(())` immediately for all `wasm32` targets. The WASI binary compiles but does absolutely nothing. We need a `#[cfg(target_os = "wasi")]` branch that opens an in-memory database and runs the REPL.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Confirm the binary currently does nothing under WASI**

If you have Wasmtime installed locally, run:

```bash
echo '(transact [{:db/id "e1" :name "Alice"}])' | \
  wasmtime run --dir /tmp target/wasm32-wasip1/release/minigraf.wasm
```

Expected: no output (binary exits immediately). If Wasmtime is not installed locally, skip — this will be verified in CI.

- [ ] **Step 2: Edit `src/main.rs`**

Replace the entire file with:

```rust
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use minigraf::Minigraf;
#[cfg(not(target_arch = "wasm32"))]
use minigraf::OpenOptions;

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "wasi")]
    {
        let db = Minigraf::in_memory()?;
        db.repl().run();
        Ok(())
    }
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    {
        // Browser WASM — entry point is the BrowserDb JS/WASM API, not a REPL binary.
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let args: Vec<String> = std::env::args().collect();
        let file_flag_pos = args.iter().position(|a| a == "--file");
        let db_path = file_flag_pos.and_then(|i| args.get(i + 1)).cloned();

        if file_flag_pos.is_some() && db_path.is_none() {
            eprintln!("error: --file requires a path argument");
            std::process::exit(1);
        }

        let db = if let Some(path) = db_path {
            OpenOptions::new().path(path).open()?
        } else {
            Minigraf::in_memory()?
        };

        db.repl().run();
        Ok(())
    }
}
```

Key change: the `use minigraf::Minigraf` import is now gated with `any(not(target_arch = "wasm32"), target_os = "wasi")` so it is in scope for both native and WASI. The WASI branch creates an in-memory database and runs the REPL.

- [ ] **Step 3: Verify native build is unaffected**

```bash
cargo build --quiet
```

Expected: no errors.

- [ ] **Step 4: Verify WASI binary compiles**

```bash
cargo build --target wasm32-wasip1 --release --bin minigraf 2>&1 | grep -E "^error"
```

Expected: no output (no errors).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(wasi): enable in-memory REPL for wasm32-wasip1 targets

The previous wasm32 branch returned Ok(()) immediately, making the
binary a no-op under WASI. Add a target_os = wasi branch that opens
an in-memory Minigraf database and runs the REPL via stdin/stdout.
File-backed storage is out of scope for WASI (in-memory only).

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Add WASI-gated smoke test to `src/db.rs`

**Files:**
- Modify: `src/db.rs` (append after line 1729, inside the file — after the last `}` closing the test module)

- [ ] **Step 1: Locate the end of the existing test module**

The `#[cfg(test)]` module in `src/db.rs` closes at the last `}` of the file (line 1730). The WASI test is a *separate* module, added after it.

- [ ] **Step 2: Append the WASI test module to `src/db.rs`**

Add after the final closing `}` of the file:

```rust
// ─── WASI smoke test ─────────────────────────────────────────────────────────
// Gated to target_os = "wasi" only. Regular #[test] works here because
// cargo test --target wasm32-wasip1 uses Wasmtime as the runner
// (CARGO_TARGET_WASM32_WASIP1_RUNNER). Not gated on target_arch = "wasm32"
// because the browser target (wasm32-unknown-unknown) requires
// #[wasm_bindgen_test] instead, which is a separate harness.
#[cfg(all(target_os = "wasi", test))]
mod wasi_tests {
    use crate::db::Minigraf;

    #[test]
    fn in_memory_smoke() {
        let db = Minigraf::in_memory().expect("open in-memory db");
        db.execute(r#"(transact [{:db/id "e1" :name "hello"}])"#)
            .expect("transact");
        let r = db
            .execute("(query [:find ?e :where [?e :name _]])")
            .expect("query");
        assert!(!r.is_empty());
    }
}
```

- [ ] **Step 3: Verify native test suite is unaffected**

```bash
cargo test --quiet 2>&1 | tail -5
```

Expected: same pass count as before (the new module is invisible to native builds).

- [ ] **Step 4: Verify WASI build includes the test module**

```bash
cargo build --target wasm32-wasip1 --tests 2>&1 | grep -E "^error"
```

Expected: no output (no errors). The `--tests` flag compiles test binaries without running them.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "test(wasi): add WASI-gated in-memory smoke test

Uses #[cfg(all(target_os = wasi, test))] to avoid firing in the browser
WASM environment where wasm-pack requires #[wasm_bindgen_test] instead.
Run via: CARGO_TARGET_WASM32_WASIP1_RUNNER='wasmtime run --dir /tmp'
cargo test --target wasm32-wasip1

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Rewrite `.github/workflows/wasm-wasi.yml`

Replace the existing file entirely. The old workflow had two jobs (build + test) requiring artifact upload/download — fragile and unnecessary. The new workflow is a single job: build → run WASI tests via Wasmtime runner → Wasmtime smoke test → Wasmer smoke test.

**Files:**
- Replace: `.github/workflows/wasm-wasi.yml`

- [ ] **Step 1: Replace the workflow file**

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

      - name: Run WASI tests (Wasmtime runner)
        run: cargo test --target wasm32-wasip1
        env:
          CARGO_TARGET_WASM32_WASIP1_RUNNER: wasmtime run --dir /tmp

      - name: Smoke test — Wasmtime transact
        run: |
          echo '(transact [{:db/id "eid-1" :name "Alice"}])' | \
            wasmtime run --dir /tmp \
              target/wasm32-wasip1/release/minigraf.wasm

      - name: Smoke test — Wasmtime transact + query
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

      - name: Smoke test — Wasmer transact + query
        run: |
          printf '(transact [{:db/id "eid-1" :name "Alice"}])\n(query [:find ?e :where [?e :name _]])\n' | \
            wasmer run --dir /tmp \
              target/wasm32-wasip1/release/minigraf.wasm
```

- [ ] **Step 2: Verify the YAML is valid**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/wasm-wasi.yml'))" && echo "valid"
```

Expected: `valid`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/wasm-wasi.yml
git commit -m "ci(wasi): rewrite wasm-wasi workflow as single job with Wasmtime + Wasmer

Previous workflow had a fragile two-job split with artifact upload/download
and no Wasmer step. New workflow: build -> WASI unit tests (via
CARGO_TARGET_WASM32_WASIP1_RUNNER) -> Wasmtime smoke -> Wasmer smoke,
all in one job.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 6: Update documentation

**Files:**
- Modify: `.wiki/Use-Cases.md`
- Modify: `README.md`
- Delete: `docs/wasi.md` (untracked — just don't commit it; remove it)

- [ ] **Step 1: Remove `docs/wasi.md`**

```bash
rm docs/wasi.md
```

This file was added by the previous agent in the wrong location. Content moves to the wiki.

- [ ] **Step 2: Expand the WASI subsection in `.wiki/Use-Cases.md`**

Find this block (around line 187):

```markdown
- **Server-side WASM (`wasm32-wasip1` / WASI)**: Standard `cargo build` to a WASI target; runs inside Wasmtime, Wasmer, or Cloudflare Workers. `FileBackend` works as-is via the WASI filesystem API. More secure than Docker for agent sandboxing.
```

Replace from that line through to (but not including) the `The wasm feature flag...` line with:

```markdown
- **Server-side WASM (`wasm32-wasip1` / WASI)**: Standard `cargo build` to a WASI target; runs inside Wasmtime, Wasmer, Cloudflare Workers, and Fastly Compute. In-memory operation only (Phase 8.1b); more secure than Docker for agent sandboxing.

### Server-side WASM (WASI) — Phase 8.1b

WASI (WebAssembly System Interface) is a capability-based standard that gives WebAssembly programs access to OS-like primitives — stdin/stdout, clocks — without requiring a JavaScript engine. Minigraf compiles to `wasm32-wasip1` and runs in any WASI-compatible runtime.

#### Build

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release --bin minigraf
# Output: target/wasm32-wasip1/release/minigraf.wasm
```

#### Run

```bash
# Wasmtime — pipe Datalog commands via stdin
echo '(transact [{:db/id "e1" :name "Alice"}])' | \
  wasmtime run target/wasm32-wasip1/release/minigraf.wasm

# Wasmer
echo '(transact [{:db/id "e1" :name "Alice"}])' | \
  wasmer run target/wasm32-wasip1/release/minigraf.wasm
```

The REPL reads commands from stdin and writes results to stdout, one result per line. All data is in-memory and discarded when the process exits.

#### Constraints

- **In-memory only**: WASI Phase 8.1b does not expose file-backed storage. Each invocation starts with an empty database.
- **Single-threaded**: WASI Preview 1 has no thread support. `Mutex`/`RwLock` compile as single-threaded stubs. Concurrent access from multiple threads is not possible by design.

#### Embedding in host applications

Minigraf can be embedded in any Wasmtime or Wasmer host. The host pipes Datalog commands to stdin and reads results from stdout.

**Wasmtime (Rust):**

```rust
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;

let engine = Engine::default();
let mut linker = Linker::new(&engine);
wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;

let wasi = WasiCtxBuilder::new()
    .inherit_stdio()
    .build();
let mut store = Store::new(&engine, wasi);

let module = Module::from_file(&engine, "minigraf.wasm")?;
let instance = linker.instantiate(&mut store, &module)?;
let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
start.call(&mut store, ())?;
```

**Wasmer (Go):**

```go
import (
    "github.com/wasmerio/wasmer-go/wasmer"
)

engine := wasmer.NewEngine()
store := wasmer.NewStore(engine)
module, _ := wasmer.NewModule(store, wasmBytes)
wasiEnv, _ := wasmer.NewWasiStateBuilder("minigraf").Finalize()
importObject, _ := wasiEnv.GenerateImportObject(store, module)
instance, _ := wasmer.NewInstance(module, importObject)
start, _ := instance.Exports.GetFunction("_start")
start()
```

#### Cloudflare Workers / Fastly Compute

Both platforms support WASI. Build the binary as above and follow the platform's WASM deployment guide. Note that these platforms' WASI support is evolving — check the respective documentation for the latest compatibility status.
```

- [ ] **Step 3: Update `README.md` — WASI status line**

Find (around line 134):

```markdown
Phase 8 target: page-granular IndexedDB backend, `wasm-pack` packaging, npm release as `@minigraf/core`. Also supports server-side WASM via WASI.
```

Replace with:

```markdown
Phase 8.1a complete: IndexedDB backend, `wasm-pack` packaging. Phase 8.1b complete: server-side WASM via `wasm32-wasip1` / WASI (Wasmtime, Wasmer). npm release as `@minigraf/core` planned for Phase 8.2.
```

- [ ] **Step 4: Commit**

```bash
git add .wiki/Use-Cases.md README.md
git commit -m "docs: move WASI docs to wiki, update README phase status

Replace one-liner WASI mention in Use-Cases.md with full build/run/
embedding/constraints section. Remove docs/wasi.md (wrong location).
Update README to reflect Phase 8.1b complete.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

- [ ] **Step 5: Push wiki separately**

```bash
cd .wiki
git add Use-Cases.md
git commit -m "docs(wasi): add Phase 8.1b WASI build, run, and embedding guide

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
git push
cd ..
```

---

## Task 7: Final verification

- [ ] **Step 1: Native test suite passes**

```bash
cargo test --quiet 2>&1 | tail -5
```

Expected: all tests pass, same count as before Task 1.

- [ ] **Step 2: Browser WASM build unaffected**

```bash
cargo build --target wasm32-unknown-unknown --features browser --quiet 2>&1 | grep -E "^error"
```

Expected: no output (no errors).

- [ ] **Step 3: WASI binary builds cleanly**

```bash
cargo build --target wasm32-wasip1 --release --bin minigraf 2>&1 | grep -E "^error|^warning"
```

Expected: no errors. Warnings about dead code in non-WASI paths are acceptable.

- [ ] **Step 4: Run WASI tests locally (if Wasmtime is installed)**

```bash
CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --dir /tmp" \
  cargo test --target wasm32-wasip1 2>&1 | tail -10
```

Expected: `test wasi_tests::in_memory_smoke ... ok`

If Wasmtime is not installed locally, skip — CI will catch this.

---

## Task 8: Open PR

- [ ] **Step 1: Push branch**

```bash
git push -u origin feat/phase-8-1b
```

- [ ] **Step 2: Open PR**

```bash
gh pr create \
  --title "feat(wasi): Phase 8.1b — wasm32-wasip1 server-side WASM support" \
  --body "$(cat <<'EOF'
Closes #130.

## What

Compiles Minigraf to `wasm32-wasip1` with a working in-memory REPL, tested in CI by both Wasmtime and Wasmer.

## Changes

- **`Cargo.toml`**: Narrowed `target_arch = "wasm32"` dep/dev-dep blocks to `all(target_arch = "wasm32", not(target_os = "wasi"))` — prevents `uuid`'s `js` feature (and `wasm-bindgen-test`) from applying to WASI targets where there is no JavaScript engine.
- **`src/main.rs`**: Added `#[cfg(target_os = "wasi")]` branch that opens an in-memory database and runs the REPL. File-backed storage is out of scope for WASI (in-memory only per design decision).
- **`src/db.rs`**: Added `#[cfg(all(target_os = "wasi", test))]` smoke test, run via `CARGO_TARGET_WASM32_WASIP1_RUNNER`.
- **`.github/workflows/wasm-wasi.yml`**: Rewrote as a single job: build → WASI unit tests (Wasmtime runner) → Wasmtime smoke → Wasmer smoke.
- **`.wiki/Use-Cases.md`**: Expanded WASI section with build/run/constraints/embedding guide.
- **`README.md`**: Updated phase status.
- **`docs/wasi.md`**: Removed (wrong location; content is in wiki).

## Testing sign-off checklist

- [ ] `cargo build --target wasm32-wasip1 --release` succeeds
- [ ] `cargo test --target wasm32-wasip1` passes via Wasmtime runner (CI)
- [ ] Wasmtime smoke test (transact + query) passes (CI)
- [ ] Wasmer smoke test (transact + query) passes (CI)
- [ ] `cargo test` (native) still passes (CI)
- [ ] `wasm-pack build --features browser` still passes (CI)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Monitor CI**

Watch the `wasm-wasi` and `Rust` workflow runs. If either fails, fix before merging — do not merge with a red CI.
