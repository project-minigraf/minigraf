# Wave 3 PR 1 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `FaultInjectingBackend`, initialize the `fuzz/` crate, and add `proptest` as a dev-dependency — the three infrastructure pieces that unblock all other Wave 3 PRs.

**Architecture:** `FaultInjectingBackend<B>` wraps any `StorageBackend` and injects `io::Error` on configurable call counts; it lives in `src/storage/backend/fault_inject.rs` under `#[cfg(test)]` so zero production binary impact. The `fuzz/` crate is a standard cargo-fuzz workspace member; fuzz targets for PR 2–4 will populate it. `proptest` is a dev-dependency for PR 4's reference evaluator.

**Tech Stack:** Rust stable, cargo-fuzz (nightly for running fuzz), proptest 1.x, libfuzzer-sys

**Closes:** infrastructure prerequisite for #209, #210, #213, #214, #221

---

## File Map

| Action | Path | Purpose |
|---|---|---|
| Create | `src/storage/backend/fault_inject.rs` | `FaultInjectingBackend<B>` impl |
| Modify | `src/storage/backend/mod.rs` | re-export `FaultInjectingBackend` under `#[cfg(test)]` |
| Modify | `Cargo.toml` | add `proptest`, add `fuzz` workspace member |
| Create | `fuzz/Cargo.toml` | cargo-fuzz crate manifest |
| Create | `fuzz/fuzz_targets/.gitkeep` | placeholder so directory is tracked |

---

## Task 1: Add proptest dev-dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add proptest to dev-dependencies**

In `Cargo.toml`, in the `[dev-dependencies]` section (around line 68), add:

```toml
proptest = "1"
```

- [ ] **Step 2: Verify it resolves**

```bash
cargo check --tests 2>&1 | tail -5
```
Expected: no errors (proptest resolves, no code uses it yet).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add proptest dev-dependency for Wave 3 property tests"
```

---

## Task 2: Create FaultInjectingBackend

**Files:**
- Create: `src/storage/backend/fault_inject.rs`
- Modify: `src/storage/backend/mod.rs`

- [ ] **Step 1: Write the failing unit test first**

Create `src/storage/backend/fault_inject.rs` with just the test:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn fault_injecting_backend_fails_on_nth_write() {
        // Will fail until FaultInjectingBackend is implemented
        todo!()
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test fault_injecting_backend_fails 2>&1 | tail -10
```
Expected: FAIL with `not yet implemented`.

- [ ] **Step 3: Implement FaultInjectingBackend**

Replace the contents of `src/storage/backend/fault_inject.rs` with:

```rust
//! Fault-injecting storage backend for reliability testing.
//!
//! Wraps any [`StorageBackend`] and injects `io::Error` on configurable call counts.
//! Used by WAL crash-recovery and durability tests.
//!
//! This module is only compiled in test builds (`#[cfg(test)]`).

use crate::storage::StorageBackend;
use anyhow::Result;
use std::io;
use std::sync::{Arc, Mutex};

/// Configuration for fault injection. All counts start at 0 and increment per call.
/// Set `fail_*_after` to `Some(N)` to inject an error after N successful calls.
/// `None` means never fail.
#[derive(Debug, Default)]
pub struct FaultConfig {
    /// Fail `write_page` after this many successful calls.
    pub fail_write_after: Option<u64>,
    /// Fail `sync` after this many successful calls.
    pub fail_sync_after: Option<u64>,
    /// Fail `close` after this many successful calls.
    pub fail_close_after: Option<u64>,
    write_count: u64,
    sync_count: u64,
    close_count: u64,
}

impl FaultConfig {
    fn check_and_increment(count: &mut u64, limit: Option<u64>) -> Result<()> {
        if let Some(n) = limit {
            if *count >= n {
                return Err(anyhow::Error::new(io::Error::new(
                    io::ErrorKind::Other,
                    "fault injection: simulated I/O error",
                )));
            }
        }
        *count += 1;
        Ok(())
    }
}

/// A storage backend wrapper that injects failures at configurable call counts.
///
/// # Example
///
/// ```rust,ignore
/// let config = Arc::new(Mutex::new(FaultConfig { fail_sync_after: Some(1), ..Default::default() }));
/// let backend = FaultInjectingBackend::new(MemoryBackend::new(), config.clone());
/// let mut pfs = PersistentFactStorage::new(backend, 16)?;
/// // ... write facts ...
/// // First sync call succeeds (count=0 < limit=1).
/// // Second sync call fails (count=1 >= limit=1).
/// ```
pub struct FaultInjectingBackend<B: StorageBackend> {
    inner: B,
    config: Arc<Mutex<FaultConfig>>,
}

impl<B: StorageBackend> FaultInjectingBackend<B> {
    pub fn new(inner: B, config: Arc<Mutex<FaultConfig>>) -> Self {
        FaultInjectingBackend { inner, config }
    }

    /// Convenience constructor: returns the backend AND a shared config handle.
    pub fn with_config(inner: B) -> (Self, Arc<Mutex<FaultConfig>>) {
        let config = Arc::new(Mutex::new(FaultConfig::default()));
        let backend = FaultInjectingBackend {
            inner,
            config: config.clone(),
        };
        (backend, config)
    }
}

impl<B: StorageBackend> StorageBackend for FaultInjectingBackend<B> {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        FaultConfig::check_and_increment(&mut cfg.write_count, cfg.fail_write_after)?;
        drop(cfg);
        self.inner.write_page(page_id, data)
    }

    fn read_page(&self, page_id: u64) -> Result<Vec<u8>> {
        // Reads are never faulted (mirrors real hardware: reads don't fail mid-operation).
        self.inner.read_page(page_id)
    }

    fn sync(&mut self) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        FaultConfig::check_and_increment(&mut cfg.sync_count, cfg.fail_sync_after)?;
        drop(cfg);
        self.inner.sync()
    }

    fn page_count(&self) -> Result<u64> {
        self.inner.page_count()
    }

    fn close(&mut self) -> Result<()> {
        let mut cfg = self.config.lock().unwrap();
        FaultConfig::check_and_increment(&mut cfg.close_count, cfg.fail_close_after)?;
        drop(cfg);
        self.inner.close()
    }

    fn backend_name(&self) -> &'static str {
        "fault-injecting"
    }

    fn is_new(&self) -> bool {
        self.inner.is_new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::MemoryBackend;
    use crate::storage::PAGE_SIZE;

    fn make_page() -> Vec<u8> {
        vec![0xAB; PAGE_SIZE]
    }

    #[test]
    fn fault_injecting_backend_fails_on_nth_write() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());

        // First write succeeds.
        assert!(backend.write_page(0, &make_page()).is_ok(), "first write should succeed");

        // Configure: fail after 1 successful write.
        config.lock().unwrap().fail_write_after = Some(1);

        // Second write should now fail.
        let result = backend.write_page(1, &make_page());
        assert!(result.is_err(), "second write should fail after fault injection");
    }

    #[test]
    fn fault_injecting_backend_fails_on_nth_sync() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());

        // Succeed first sync.
        assert!(backend.sync().is_ok(), "first sync should succeed");

        // Fail on sync #2.
        config.lock().unwrap().fail_sync_after = Some(1);
        let result = backend.sync();
        assert!(result.is_err(), "second sync should fail after fault injection");
    }

    #[test]
    fn reads_are_never_faulted() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());
        backend.write_page(0, &make_page()).unwrap();

        // Even with writes faulted, reads still work.
        config.lock().unwrap().fail_write_after = Some(0);
        assert!(backend.read_page(0).is_ok(), "reads should never be faulted");
    }

    #[test]
    fn config_can_be_updated_mid_scenario() {
        let (mut backend, config) = FaultInjectingBackend::with_config(MemoryBackend::new());

        // No fault configured — 3 writes succeed.
        for i in 0..3 {
            assert!(backend.write_page(i, &make_page()).is_ok());
        }

        // Inject fault now.
        config.lock().unwrap().fail_write_after = Some(3);
        assert!(backend.write_page(3, &make_page()).is_err(), "4th write should fail");

        // Remove fault — writes work again.
        config.lock().unwrap().fail_write_after = None;
        assert!(backend.write_page(4, &make_page()).is_ok(), "write should succeed after removing fault");
    }
}
```

- [ ] **Step 4: Register the module (gated under cfg(test))**

In `src/storage/backend/mod.rs`, add after the existing `pub mod memory;`:

```rust
#[cfg(test)]
pub mod fault_inject;
#[cfg(test)]
pub use fault_inject::{FaultConfig, FaultInjectingBackend};
```

- [ ] **Step 5: Run the unit tests**

```bash
cargo test fault_injecting 2>&1 | tail -20
```
Expected: 4 tests pass (`fails_on_nth_write`, `fails_on_nth_sync`, `reads_are_never_faulted`, `config_can_be_updated_mid_scenario`).

- [ ] **Step 6: Commit**

```bash
git add src/storage/backend/fault_inject.rs src/storage/backend/mod.rs
git commit -m "test(storage): add FaultInjectingBackend for fault-injection testing"
```

---

## Task 3: Initialize fuzz/ crate

**Files:**
- Modify: `Cargo.toml`
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/.gitkeep`

- [ ] **Step 1: Add fuzz/ as a workspace member**

In `Cargo.toml`, find the `[workspace]` section (or add one after `[package]`) and add `fuzz` as a member. If there's no `[workspace]` section, check if there's a workspace root. If the file has `[package]` at the top with no `[workspace]`, add:

```toml
[workspace]
members = [".", "fuzz"]
resolver = "2"
```

If a `[workspace]` section already exists, just add `"fuzz"` to the members array.

- [ ] **Step 2: Create fuzz/Cargo.toml**

```toml
[package]
name = "minigraf-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
minigraf = { path = ".." }

# Prevent this from interfering with workspace lints
[lints]
workspace = false

[[bin]]
name = "wal_entry"
path = "fuzz_targets/wal_entry.rs"
test = false
doc = false

[[bin]]
name = "file_header"
path = "fuzz_targets/file_header.rs"
test = false
doc = false

[[bin]]
name = "fact_page"
path = "fuzz_targets/fact_page.rs"
test = false
doc = false

[[bin]]
name = "btree_page"
path = "fuzz_targets/btree_page.rs"
test = false
doc = false

[[bin]]
name = "datalog_parser"
path = "fuzz_targets/datalog_parser.rs"
test = false
doc = false

[[bin]]
name = "datalog_eval"
path = "fuzz_targets/datalog_eval.rs"
test = false
doc = false
```

- [ ] **Step 3: Create fuzz_targets/ directory with a placeholder**

```bash
mkdir -p fuzz/fuzz_targets fuzz/corpus
touch fuzz/fuzz_targets/.gitkeep
```

- [ ] **Step 4: Add .gitignore for fuzz artifacts**

Create `fuzz/.gitignore`:

```
artifacts/
coverage/
corpus/*/
!corpus/
```

Wait — we want seed corpus entries committed, so only ignore the runtime-discovered corpus (which cargo-fuzz writes back to the corpus dir). Adjust:

Create `fuzz/.gitignore`:
```
artifacts/
coverage/
```

Corpus seed files ARE committed (they're in `fuzz/corpus/<target>/`). cargo-fuzz adds new entries to the corpus directory when it finds interesting inputs; these can be committed or ignored per preference. For now, commit seed files and ignore runtime-generated entries using per-target `.gitignore` files added in PR 2/4.

- [ ] **Step 5: Verify cargo check passes on fuzz crate**

```bash
cargo +nightly check --manifest-path fuzz/Cargo.toml 2>&1 | tail -15
```
Expected: error about missing `fuzz_targets/wal_entry.rs` etc. — that's expected. The crate structure is correct; targets are added in PRs 2 and 4. Verify no manifest parse errors.

- [ ] **Step 6: Create stub targets so cargo check passes**

For each target listed in `fuzz/Cargo.toml`, create a minimal stub:

`fuzz/fuzz_targets/wal_entry.rs`:
```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|_data: &[u8]| {
    // TODO: implemented in Wave 3 PR 2
});
```

Create identical stubs for `file_header.rs`, `fact_page.rs`, `btree_page.rs`, `datalog_parser.rs`, `datalog_eval.rs` (same content, just different comments).

- [ ] **Step 7: Verify fuzz crate builds**

```bash
cargo +nightly check --manifest-path fuzz/Cargo.toml 2>&1 | tail -10
```
Expected: no errors.

- [ ] **Step 8: Verify main workspace still builds and all tests pass**

```bash
cargo test 2>&1 | tail -20
```
Expected: all existing tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml fuzz/
git commit -m "chore: initialize cargo-fuzz crate with stub targets for Wave 3"
```

---

## Task 4: Open PR

- [ ] **Push branch and open PR targeting main**

```bash
git push -u origin HEAD
gh pr create \
  --title "chore(wave3): foundation — FaultInjectingBackend, fuzz/ crate, proptest" \
  --body "$(cat <<'EOF'
## Wave 3 PR 1 — Foundation

Infrastructure prerequisite for all Wave 3 reliability work.

### Changes
- `src/storage/backend/fault_inject.rs`: `FaultInjectingBackend<B>` wrapper that injects `io::Error` on configurable write/sync/close call counts. Gated `#[cfg(test)]`. 4 unit tests.
- `Cargo.toml`: `proptest = "1"` added to `[dev-dependencies]`; `fuzz` added to workspace members.
- `fuzz/`: cargo-fuzz crate with stub targets for WAL, file-format, and Datalog fuzzing. Stubs are replaced in PRs 2 and 4.

### Closes
Unblocks #209, #210, #213, #214, #221

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Monitor CI until green before merging**
