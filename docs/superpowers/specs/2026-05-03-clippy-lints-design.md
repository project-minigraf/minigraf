# Clippy Lint Enforcement Design

**Date:** 2026-05-03  
**Status:** Approved  
**Scope:** Add production-grade clippy lints across the entire workspace and fix all existing violations

---

## Problem

Minigraf has no enforced clippy lint rules beyond `forbid(unsafe_code)` and `warn(missing_docs)`. Production database code that silently panics or silently truncates casts is a correctness risk — especially in the file format layer where a truncated page offset or a wrapped `i64` could corrupt a database file.

**Known violation count (as of 2026-05-03):** 336 total  
- `minigraf` (lib): 330  
- `minigraf-c`: 6  
- `minigraf-ffi`, `minigraf-node`: 0

---

## Mechanism

Use `[workspace.lints.clippy]` in the root `Cargo.toml`. Each workspace member opts in via `[lints] workspace = true` in its own `Cargo.toml`. This satisfies both constraints:

- **Local to the workspace** — downstream crates that depend on `minigraf` are not affected.
- **Enforced** — all lints are set to `deny` (or `warn` where noted); `cargo build` and `cargo clippy` fail on violations.

Test code is exempted via `#![cfg_attr(test, allow(...))]` at each crate root — panicking in tests is intentional and idiomatic.

---

## Lint Set

```toml
[workspace.lints.clippy]
# Panic prevention — use Result/? or debug_assert! instead
unwrap_used              = "deny"
expect_used              = "deny"
panic                    = "deny"
todo                     = "deny"
unimplemented            = "deny"
unreachable              = "warn"   # legitimate in exhaustive match arms

# Numeric safety — critical for page offsets, checksums, byte sizes
cast_possible_truncation = "deny"
cast_possible_wrap       = "deny"
cast_sign_loss           = "deny"
arithmetic_side_effects  = "warn"   # very noisy; warn so violations surface without blocking

# Bounds safety
indexing_slicing         = "deny"   # use .get() with ? instead of direct indexing

# API documentation
missing_errors_doc       = "warn"
```

---

## Violation Fix Strategy

### Panic prevention (`unwrap_used`, `expect_used`, `panic`, `unimplemented`)

**Pattern A — `RwLock`/`Mutex` poison** (most common in `graph/storage.rs`):
```rust
// Before
let mut d = self.data.write().unwrap();

// After
let mut d = self.data.write().map_err(|_| anyhow::anyhow!("lock poisoned"))?;
```

**Pattern B — `Option` to `Result`**:
```rust
// Before
let x = map.get(&key).expect("must be present");

// After — if it's a genuine invariant, assert in debug then propagate
debug_assert!(map.contains_key(&key), "invariant: key must be present after init");
let x = map.get(&key).ok_or_else(|| anyhow::anyhow!("key not found: {:?}", key))?;
```

**Pattern C — explicit `panic!` / `unimplemented!`**:
```rust
// Before
panic!("unexpected state");
unimplemented!("feature X");

// After
return Err(anyhow::anyhow!("unexpected state"));
// or remove/implement the unimplemented branch
```

**`debug_assert!` as invariant assertion:**  
Use `debug_assert!(condition, "message")` for invariants that should hold in correct code but must not crash in production if they don't. It compiles away in release builds. Follow it with a graceful `Err(...)` return for the release path.

### Numeric casts (`cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss`)

Use `try_from`/`try_into` for all potentially lossy casts:

```rust
// Before
let offset = page_count as u32;
let ts = unix_ms as i64;

// After
let offset = u32::try_from(page_count)?;
let ts = i64::try_from(unix_ms)?;
```

For the `f64 as i64` case in value handling, use explicit checked conversion:
```rust
let i = if v.is_finite() && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
    v as i64  // safe after range check — suppress with #[allow]
} else {
    return Err(anyhow::anyhow!("float out of i64 range"));
};
```

### Bounds safety (`indexing_slicing`)

Replace direct indexing with `.get()`:

```rust
// Before
let byte = buf[offset];
let chunk = &buf[start..end];

// After
let byte = buf.get(offset).ok_or_else(|| anyhow::anyhow!("offset {offset} out of bounds"))?;
let chunk = buf.get(start..end).ok_or_else(|| anyhow::anyhow!("slice {start}..{end} out of bounds"))?;
```

Where bounds are structurally guaranteed (e.g., inside a loop that already checked length), use `#[allow(clippy::indexing_slicing)]` with a comment explaining the invariant, rather than paying the `Option` unwrap cost everywhere.

---

## Violation Distribution by File

| File | Violations | Primary lint |
|---|---|---|
| `query/datalog/parser.rs` | 64 | indexing_slicing |
| `storage/btree_v6.rs` | 51 | indexing_slicing, cast_possible_truncation |
| `storage/persistent_facts.rs` | 29 | indexing_slicing, unwrap_used |
| `storage/mod.rs` | 27 | unwrap_used, cast_possible_wrap |
| `query/datalog/executor.rs` | 27 | indexing_slicing, unwrap_used |
| `query/datalog/evaluator.rs` | 23 | unwrap_used, indexing_slicing |
| `graph/storage.rs` | 22 | unwrap_used (lock poison pattern) |
| `storage/packed_pages.rs` | 19 | indexing_slicing, cast_possible_truncation |
| `storage/btree.rs` | 17 | indexing_slicing (legacy, migration only) |
| `storage/cache.rs` | 9 | unwrap_used |
| `query/datalog/matcher.rs` | 9 | indexing_slicing |
| `db.rs` | 7 | unwrap_used |
| `query/datalog/functions.rs` | 6 | cast_possible_wrap |
| `minigraf-c/src/lib.rs` | 6 | unwrap_used, panic |
| others | ~10 | mixed |

---

## Test Exemptions

Each crate root (`src/lib.rs`, `minigraf-c/src/lib.rs`, `minigraf-ffi/src/lib.rs`, `minigraf-node/src/lib.rs`) gets:

```rust
#![cfg_attr(test, allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
))]
```

The `tests/` integration test directory is already outside the lint scope for `--lib`; the above covers `#[cfg(test)]` modules within `src/`.

---

## Files Changed

**Configuration (4 `Cargo.toml` files):**
- Root `Cargo.toml` — add `[workspace.lints.clippy]`
- `minigraf-ffi/Cargo.toml` — add `[lints] workspace = true`
- `minigraf-c/Cargo.toml` — add `[lints] workspace = true`
- `minigraf-node/Cargo.toml` — add `[lints] workspace = true`

**Crate roots (4 files):**
- `src/lib.rs` — add `cfg_attr(test, allow(...))` header
- `minigraf-ffi/src/lib.rs` — add `cfg_attr(test, allow(...))` header
- `minigraf-c/src/lib.rs` — add `cfg_attr(test, allow(...))` header
- `minigraf-node/src/lib.rs` — add `cfg_attr(test, allow(...))` header

**Production source fixes (~14 files, 336 violations):**
- All files listed in the distribution table above

---

## Success Criteria

- `cargo clippy --workspace 2>&1 | grep "^error"` → no output
- All existing tests continue to pass (`cargo test --workspace`)
- No `#[allow(clippy::unwrap_used)]` or `#[allow(clippy::expect_used)]` outside of test code, except where the surrounding logic makes the safety guarantee explicit via comment
