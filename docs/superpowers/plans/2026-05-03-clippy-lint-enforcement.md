# Clippy Lint Enforcement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add production-grade clippy lints to the entire workspace and fix all 336 existing violations so `cargo clippy --workspace` is error-free.

**Architecture:** Lint rules live in `[workspace.lints.clippy]` in the root `Cargo.toml`; every workspace member opts in via `[lints] workspace = true`. Test code is exempted via `#![cfg_attr(test, allow(...))]` at each crate root. Violations are fixed file-by-file using `?`/`try_from`/`.get()` patterns — no `unwrap`, no direct indexing, no lossy casts in production paths.

**Tech Stack:** Rust (edition 2024), anyhow for error propagation, `std::convert::TryFrom` for safe casts, `c"..."` CStr literals for FFI fallbacks.

---

## Fix Patterns Reference

Refer back to this section when applying fixes in Tasks 3–16.

**Pattern A — RwLock/Mutex poison (→ propagate)**
```rust
// Before
let mut d = self.data.write().unwrap();

// After (in functions that return anyhow::Result)
let mut d = self.data.write().map_err(|_| anyhow::anyhow!("lock poisoned"))?;
```

**Pattern B — Mutex in FFI (cannot propagate → recover from poison)**
```rust
// Before
*self.last_error.lock().unwrap() = Some(cs);

// After (FFI: recover from poison rather than abort)
*self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(cs);
```

**Pattern C — Result unwrap (→ propagate with ?)**
```rust
// Before
let x = some_fallible_fn().unwrap();

// After
let x = some_fallible_fn()?;
// or, if context helps:
let x = some_fallible_fn().map_err(|e| anyhow::anyhow!("context: {e}"))?;
```

**Pattern D — Option unwrap (→ propagate)**
```rust
// Before
let x = map.get(&key).unwrap();

// After
let x = map.get(&key).ok_or_else(|| anyhow::anyhow!("key not found"))?;
```

**Pattern E — expect (→ debug_assert + propagate)**
```rust
// Before
let x = val.expect("invariant: must be present after init");

// After
debug_assert!(val.is_some(), "invariant: must be present after init");
let x = val.ok_or_else(|| anyhow::anyhow!("invariant violated: must be present after init"))?;
```

**Pattern F — Slice-to-array try_into().unwrap() (→ map_err)**
```rust
// Before (inside a function that already bounds-checked)
u32::from_le_bytes(bytes[4..8].try_into().unwrap())

// After
u32::from_le_bytes(
    bytes.get(4..8)
        .ok_or_else(|| anyhow::anyhow!("header too short: need 4..8, got {} bytes", bytes.len()))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("slice 4..8 not exactly 4 bytes"))?,
)
```

**Pattern G — Direct indexing (→ .get() + ?)**
```rust
// Before
let byte = buf[offset];
let chunk = &buf[start..end];

// After
let byte = *buf.get(offset)
    .ok_or_else(|| anyhow::anyhow!("index {offset} out of bounds (len {})", buf.len()))?;
let chunk = buf.get(start..end)
    .ok_or_else(|| anyhow::anyhow!("slice {start}..{end} out of bounds (len {})", buf.len()))?;
```

**Pattern H — Proven-safe indexing: use #[allow] with comment**
```rust
// When bounds are structurally guaranteed by surrounding logic and using .get()
// would require unwrapping the Option in a loop hot path:
#[allow(clippy::indexing_slicing)] // invariant: i < self.slots.len() — enforced by entry_count field
let slot = &self.slots[i];
```

**Pattern I — Lossy numeric cast (→ try_from)**
```rust
// Before
let offset = page_count as u32;
let ts = unix_ms as i64;
let signed = unsigned_val as i64;

// After
let offset = u32::try_from(page_count)
    .map_err(|_| anyhow::anyhow!("page_count {page_count} exceeds u32::MAX"))?;
let ts = i64::try_from(unix_ms)
    .map_err(|_| anyhow::anyhow!("timestamp {unix_ms} overflows i64"))?;
let signed = i64::try_from(unsigned_val)
    .map_err(|_| anyhow::anyhow!("value {unsigned_val} overflows i64"))?;
```

**Pattern J — CString fallback in FFI (c-string literal)**
```rust
// Before
CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap())

// After (c"..." literal available since Rust 1.77 / edition 2024)
CString::new(msg).unwrap_or_else(|_| c"error".to_owned())
```

**Pattern K — explicit panic!/unimplemented! (→ return Err)**
```rust
// Before
panic!("unexpected state: {state:?}");
unimplemented!("variant X");

// After
return Err(anyhow::anyhow!("unexpected state"));
// or for a match arm that must be unreachable:
return Err(anyhow::anyhow!("internal error: reached unreachable branch"));
```

---

## Files Modified

| File | Violations | Primary patterns |
|---|---|---|
| `Cargo.toml` | — | add `[workspace.lints.clippy]` |
| `minigraf-ffi/Cargo.toml` | — | add `[lints] workspace = true` |
| `minigraf-c/Cargo.toml` | — | add `[lints] workspace = true` |
| `minigraf-node/Cargo.toml` | — | add `[lints] workspace = true` |
| `src/lib.rs` | — | add `cfg_attr(test, allow(...))` |
| `minigraf-ffi/src/lib.rs` | — | add `cfg_attr(test, allow(...))` |
| `minigraf-c/src/lib.rs` | 6 | B, J (FFI patterns) |
| `minigraf-node/src/lib.rs` | — | add `cfg_attr(test, allow(...))` |
| `src/graph/storage.rs` | 22 | A |
| `src/graph/types.rs` | 4 | I |
| `src/storage/cache.rs` | 9 | A, C |
| `src/storage/backend/memory.rs` | 3 | C |
| `src/storage/backend/file.rs` | 1 | C |
| `src/storage/index.rs` | 4 | I |
| `src/storage/packed_pages.rs` | 19 | G, F, I |
| `src/storage/btree.rs` | 17 | G, F |
| `src/storage/btree_v6.rs` | 51 | F, G, I |
| `src/storage/mod.rs` | 27 | F, G, I |
| `src/storage/persistent_facts.rs` | 29 | C, D, G |
| `src/query/datalog/types.rs` | 3 | I |
| `src/query/datalog/matcher.rs` | 9 | G |
| `src/query/datalog/functions.rs` | 6 | I |
| `src/query/datalog/executor.rs` | 27 | A, D, G |
| `src/query/datalog/evaluator.rs` | 23 | C, D, G |
| `src/query/datalog/parser.rs` | 64 | G, C, K |
| `src/db.rs` | 7 | A, C |
| `src/wal.rs` | 3 | C |
| `src/temporal.rs` | 1 | I |

---

## Task 1: Create worktree

**Files:** none (git operation)

- [ ] **Step 1: Create the worktree**

```bash
git worktree add .worktrees/fix/clippy-lint-enforcement -b fix/clippy-lint-enforcement
```

- [ ] **Step 2: Verify worktree is clean**

```bash
cd .worktrees/fix/clippy-lint-enforcement && cargo test --quiet 2>&1 | tail -3
```
Expected: all tests pass.

---

## Task 2: Add workspace lint configuration

**Files:**
- Modify: `Cargo.toml`
- Modify: `minigraf-ffi/Cargo.toml`
- Modify: `minigraf-c/Cargo.toml`
- Modify: `minigraf-node/Cargo.toml`
- Modify: `src/lib.rs`
- Modify: `minigraf-ffi/src/lib.rs`
- Modify: `minigraf-c/src/lib.rs`
- Modify: `minigraf-node/src/lib.rs`

- [ ] **Step 1: Add `[workspace.lints.clippy]` to root `Cargo.toml`**

Append this section at the end of `Cargo.toml` (after the existing `[profile.dist]` block):

```toml
[workspace.lints.clippy]
# Panic prevention — use Result/? or debug_assert! instead
unwrap_used              = "deny"
expect_used              = "deny"
panic                    = "deny"
todo                     = "deny"
unimplemented            = "deny"
unreachable              = "warn"
# Numeric safety — critical for page offsets, checksums, byte sizes
cast_possible_truncation = "deny"
cast_possible_wrap       = "deny"
cast_sign_loss           = "deny"
arithmetic_side_effects  = "warn"
# Bounds safety
indexing_slicing         = "deny"
# API documentation
missing_errors_doc       = "warn"
```

- [ ] **Step 2: Opt each sub-crate in**

Append to `minigraf-ffi/Cargo.toml`:
```toml
[lints]
workspace = true
```

Append to `minigraf-c/Cargo.toml`:
```toml
[lints]
workspace = true
```

Append to `minigraf-node/Cargo.toml`:
```toml
[lints]
workspace = true
```

- [ ] **Step 3: Add test exemptions to `src/lib.rs`**

Add this block immediately after the existing `#![warn(missing_docs)]` line:

```rust
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
    )
)]
```

- [ ] **Step 4: Add test exemptions to sub-crate roots**

Add the same `cfg_attr` block (above) to:
- `minigraf-ffi/src/lib.rs` — insert before the first `use` statement
- `minigraf-c/src/lib.rs` — insert after the existing `#![allow(clippy::not_unsafe_ptr_arg_deref)]` line
- `minigraf-node/src/lib.rs` — insert after the existing `#![deny(clippy::all)]` line

- [ ] **Step 5: Verify the config is active and violations are visible**

```bash
cargo clippy --lib 2>&1 | grep "^error\[clippy" | wc -l
```
Expected: approximately 330 (confirms the lints are firing; build is now broken as expected).

- [ ] **Step 6: Commit the configuration**

```bash
git add Cargo.toml minigraf-ffi/Cargo.toml minigraf-c/Cargo.toml minigraf-node/Cargo.toml \
        src/lib.rs minigraf-ffi/src/lib.rs minigraf-c/src/lib.rs minigraf-node/src/lib.rs
git commit -m "chore: add workspace clippy lint enforcement config"
```

---

## Task 3: Fix `src/graph/storage.rs` and `src/graph/types.rs`

**Files:**
- Modify: `src/graph/storage.rs` (22 violations — Pattern A: lock poison)
- Modify: `src/graph/types.rs` (4 violations — Pattern I: `u64 as i64`)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::cast_possible_wrap 2>&1 \
  | grep -E "src/graph/(storage|types)\.rs"
```

- [ ] **Step 2: Fix `graph/storage.rs` — RwLock poison (Pattern A)**

Every occurrence of `self.data.write().unwrap()` and `self.data.read().unwrap()` becomes:
```rust
let mut d = self.data.write().map_err(|_| anyhow::anyhow!("data lock poisoned"))?;
let d = self.data.read().map_err(|_| anyhow::anyhow!("data lock poisoned"))?;
```
Apply to all occurrences in the file. The functions already return `anyhow::Result`, so `?` propagates correctly.

- [ ] **Step 3: Fix `graph/types.rs` — u64 → i64 cast (Pattern I)**

Two lines (292 and 333) have `valid_from: tx_id as i64` where `tx_id: u64`. Fix:
```rust
valid_from: i64::try_from(tx_id).unwrap_or(i64::MAX),
```
`tx_id` is a Unix millisecond timestamp; it will not exceed `i64::MAX` for centuries. Using `unwrap_or(i64::MAX)` is safe and avoids making the constructor fallible. Add `#[allow(clippy::unwrap_used)]` is NOT appropriate here — use `unwrap_or` instead which doesn't panic.

Wait: `unwrap_or` is on `Result<i64, _>`, not on `Option`. Use:
```rust
valid_from: i64::try_from(tx_id).unwrap_or(i64::MAX),
```
This does not call `.unwrap()`, so `clippy::unwrap_used` does not fire on it. ✓

- [ ] **Step 4: Verify zero remaining violations in these files**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::cast_possible_wrap -W clippy::cast_sign_loss 2>&1 \
  | grep -E "src/graph/(storage|types)\.rs" | wc -l
```
Expected: 0

- [ ] **Step 5: Commit**

```bash
git add src/graph/storage.rs src/graph/types.rs
git commit -m "fix(lint): resolve unwrap/cast violations in graph/"
```

---

## Task 4: Fix `src/storage/cache.rs`, `storage/backend/memory.rs`, `storage/backend/file.rs`

**Files:**
- Modify: `src/storage/cache.rs` (9 violations — Patterns A, C)
- Modify: `src/storage/backend/memory.rs` (3 violations — Pattern C)
- Modify: `src/storage/backend/file.rs` (1 violation — Pattern C)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::panic 2>&1 \
  | grep -E "src/storage/(cache|backend)" | head -40
```

- [ ] **Step 2: Fix `storage/cache.rs`**

Violations are `Mutex` lock calls and `unwrap()` on Results. Apply Pattern A for `Mutex` locks:
```rust
// Before
let mut cache = self.cache.lock().unwrap();

// After
let mut cache = self.cache.lock().map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
```
For any remaining `unwrap()` on a `Result`, replace with `?`. For `unwrap()` on `Option`, use Pattern D.

- [ ] **Step 3: Fix `storage/backend/memory.rs` and `storage/backend/file.rs`**

These are thin backends; violations are `unwrap()` on Results. Replace each with `?` (Pattern C). Functions already return `anyhow::Result`.

- [ ] **Step 4: Verify zero remaining violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::panic 2>&1 \
  | grep -E "src/storage/(cache|backend)" | wc -l
```
Expected: 0

- [ ] **Step 5: Commit**

```bash
git add src/storage/cache.rs src/storage/backend/memory.rs src/storage/backend/file.rs
git commit -m "fix(lint): resolve unwrap violations in storage/cache and backends"
```

---

## Task 5: Fix `src/storage/index.rs`

**Files:**
- Modify: `src/storage/index.rs` (4 violations — Pattern I: lossy casts)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::cast_possible_truncation \
  -W clippy::cast_possible_wrap -W clippy::cast_sign_loss 2>&1 \
  | grep "src/storage/index.rs"
```

- [ ] **Step 2: Fix lossy casts with `try_from` (Pattern I)**

Each flagged `as` cast becomes a `try_from` call. Example:
```rust
// Before
let encoded_len = data.len() as u16;

// After
let encoded_len = u16::try_from(data.len())
    .map_err(|_| anyhow::anyhow!("encoded value too large for u16: {} bytes", data.len()))?;
```

- [ ] **Step 3: Verify**

```bash
cargo clippy --lib -- -W clippy::cast_possible_truncation \
  -W clippy::cast_possible_wrap -W clippy::cast_sign_loss 2>&1 \
  | grep "src/storage/index.rs" | wc -l
```
Expected: 0

- [ ] **Step 4: Commit**

```bash
git add src/storage/index.rs
git commit -m "fix(lint): resolve cast violations in storage/index"
```

---

## Task 6: Fix `src/storage/packed_pages.rs`

**Files:**
- Modify: `src/storage/packed_pages.rs` (19 violations — Patterns G, F, I)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing \
  -W clippy::cast_possible_truncation -W clippy::unwrap_used 2>&1 \
  | grep "src/storage/packed_pages.rs"
```

- [ ] **Step 2: Fix slice indexing (Pattern G)**

Direct `buf[i]` and `&buf[a..b]` become `.get()` calls. Example for byte reads:
```rust
// Before
let header_byte = page[0];
let payload = &page[HEADER_SIZE..HEADER_SIZE + fact_len];

// After
let header_byte = *page.get(0)
    .ok_or_else(|| anyhow::anyhow!("packed page empty"))?;
let payload = page.get(HEADER_SIZE..HEADER_SIZE + fact_len)
    .ok_or_else(|| anyhow::anyhow!("packed page truncated: need {}..{}", HEADER_SIZE, HEADER_SIZE + fact_len))?;
```

- [ ] **Step 3: Fix fixed-size slice-to-array conversions (Pattern F)**

```rust
// Before
u16::from_le_bytes(page[0..2].try_into().unwrap())

// After
u16::from_le_bytes(
    page.get(0..2)
        .ok_or_else(|| anyhow::anyhow!("packed page too short for u16 at 0..2"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("slice 0..2 not exactly 2 bytes"))?,
)
```

- [ ] **Step 4: Fix lossy casts (Pattern I)**

```rust
// Before
let fact_len = serialized.len() as u16;

// After
let fact_len = u16::try_from(serialized.len())
    .map_err(|_| anyhow::anyhow!("serialized fact too large: {} bytes", serialized.len()))?;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::cast_possible_truncation \
  -W clippy::unwrap_used 2>&1 | grep "src/storage/packed_pages.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/storage/packed_pages.rs
git commit -m "fix(lint): resolve indexing/cast violations in storage/packed_pages"
```

---

## Task 7: Fix `src/storage/btree.rs`

**Files:**
- Modify: `src/storage/btree.rs` (17 violations — Patterns G, F; legacy migration code)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used 2>&1 \
  | grep "src/storage/btree.rs"
```

- [ ] **Step 2: Apply Patterns G and F**

This is the legacy v5 B+tree used only for migration. Apply the same `.get()` and `try_into().map_err(...)` patterns as Task 6. The functions return `anyhow::Result` so `?` propagates.

Example:
```rust
// Before
let entry_count = u16::from_le_bytes(page[2..4].try_into().unwrap()) as usize;

// After
let entry_count = u16::from_le_bytes(
    page.get(2..4)
        .ok_or_else(|| anyhow::anyhow!("btree page too short at 2..4"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("slice 2..4 not exactly 2 bytes"))?,
) as usize;
```
Note: `as usize` from `u16` is always safe (u16 ≤ usize on all supported platforms); clippy does not flag this cast.

- [ ] **Step 3: Verify**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used 2>&1 \
  | grep "src/storage/btree.rs" | wc -l
```
Expected: 0

- [ ] **Step 4: Commit**

```bash
git add src/storage/btree.rs
git commit -m "fix(lint): resolve indexing/unwrap violations in legacy storage/btree"
```

---

## Task 8: Fix `src/storage/btree_v6.rs`

**Files:**
- Modify: `src/storage/btree_v6.rs` (51 violations — Patterns F, G, I)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap 2>&1 \
  | grep "src/storage/btree_v6.rs"
```

- [ ] **Step 2: Fix `read_u16_at` and `read_u64_at` helpers (Pattern F)**

These functions already bounds-check before indexing, but the `.try_into().unwrap()` and `page[offset..offset+N]` still fire. Replace both:
```rust
fn read_u16_at(page: &[u8], offset: usize) -> Result<u16> {
    let bytes = page.get(offset..offset + 2)
        .ok_or_else(|| anyhow!("out of bounds: read_u16 at {offset} (len {})", page.len()))?;
    Ok(u16::from_le_bytes(
        bytes.try_into().map_err(|_| anyhow!("slice at {offset} not 2 bytes"))?,
    ))
}

fn read_u64_at(page: &[u8], offset: usize) -> Result<u64> {
    let bytes = page.get(offset..offset + 8)
        .ok_or_else(|| anyhow!("out of bounds: read_u64 at {offset} (len {})", page.len()))?;
    Ok(u64::from_le_bytes(
        bytes.try_into().map_err(|_| anyhow!("slice at {offset} not 8 bytes"))?,
    ))
}
```
Remove the old manual bounds checks at the top of each function — `.get()` subsumes them.

- [ ] **Step 3: Fix remaining direct indexing across the file (Pattern G)**

Search for `page[` and `buf[` patterns outside the helpers and replace with `.get()`:
```rust
// Before
let slot_offset = LEAF_HEADER_SIZE + i * SLOT_SIZE;
let slot_data = &page[slot_offset..slot_offset + SLOT_SIZE];

// After
let slot_offset = LEAF_HEADER_SIZE + i * SLOT_SIZE;
let slot_data = page.get(slot_offset..slot_offset + SLOT_SIZE)
    .ok_or_else(|| anyhow!("slot {i} out of bounds in leaf page"))?;
```

- [ ] **Step 4: Fix lossy casts (Pattern I)**

Commonly: `entry_count as u16`, `node_count as u32`. Use `try_from`:
```rust
let entry_count_u16 = u16::try_from(entries.len())
    .map_err(|_| anyhow!("too many entries: {}", entries.len()))?;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap 2>&1 \
  | grep "src/storage/btree_v6.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree_v6.rs
git commit -m "fix(lint): resolve indexing/cast/unwrap violations in storage/btree_v6"
```

---

## Task 9: Fix `src/storage/mod.rs`

**Files:**
- Modify: `src/storage/mod.rs` (27 violations — Patterns F, G, I)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::cast_sign_loss 2>&1 | grep "src/storage/mod.rs"
```

- [ ] **Step 2: Fix file header deserialization (Pattern F)**

`storage/mod.rs` deserializes the 84-byte file header using `bytes[a..b].try_into().unwrap()`. The dominant pattern (as seen in lines 180–183) is parsing fixed-width little-endian fields. Introduce a helper at the top of the module:

```rust
fn read_u32_le(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes.get(offset..offset + 4)
            .ok_or_else(|| anyhow::anyhow!("header: need {offset}..{}, got {}", offset + 4, bytes.len()))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("header: slice at {offset} not 4 bytes"))?,
    ))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> anyhow::Result<u64> {
    Ok(u64::from_le_bytes(
        bytes.get(offset..offset + 8)
            .ok_or_else(|| anyhow::anyhow!("header: need {offset}..{}, got {}", offset + 8, bytes.len()))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("header: slice at {offset} not 8 bytes"))?,
    ))
}
```

Then replace the parsing block:
```rust
// Before
let version    = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
let page_count = u64::from_le_bytes(bytes[8..16].try_into().unwrap());

// After
let version    = read_u32_le(bytes, 4)?;
let page_count = read_u64_le(bytes, 8)?;
```

- [ ] **Step 3: Fix any direct indexing outside the header parse (Pattern G)**

Apply `.get()` to any remaining `bytes[i]` or `&bytes[a..b]` patterns.

- [ ] **Step 4: Fix lossy casts (Pattern I)**

```rust
// Before
let page_count = self.pages.len() as u64;

// After — usize→u64 is always safe on 64-bit; on 32-bit it's a widening cast.
// u64::try_from(usize) is always Ok on all platforms; use unwrap_or is not needed.
let page_count = self.pages.len() as u64; // usize→u64 is always widening — allowed with #[allow]
```
Actually `usize as u64`: on 32-bit usize is 32 bits, widening to 64-bit — never truncates. On 64-bit both are 64-bit. So this cast is always safe. Add a targeted allow:
```rust
#[allow(clippy::cast_possible_truncation)] // usize→u64 is always widening
let page_count = self.pages.len() as u64;
```

For `u64 as i64` (wrap risk): use Pattern I with `i64::try_from`.
For `usize as u16` or `usize as u32` (truncation risk): use Pattern I with `u16::try_from` or `u32::try_from`.

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::cast_sign_loss 2>&1 | grep "src/storage/mod.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/storage/mod.rs
git commit -m "fix(lint): resolve indexing/cast violations in storage/mod (file header)"
```

---

## Task 10: Fix `src/storage/persistent_facts.rs`

**Files:**
- Modify: `src/storage/persistent_facts.rs` (29 violations — Patterns C, D, G)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::indexing_slicing 2>&1 | grep "src/storage/persistent_facts.rs"
```

- [ ] **Step 2: Fix `unwrap()` on Results (Pattern C)**

Functions in this file already return `anyhow::Result`. Replace `.unwrap()` with `?`:
```rust
// Before
let page = backend.read_page(page_id).unwrap();

// After
let page = backend.read_page(page_id)?;
```

- [ ] **Step 3: Fix `unwrap()` on Options (Pattern D)**

```rust
// Before
let fact = self.fact_by_ref(fact_ref).unwrap();

// After
let fact = self.fact_by_ref(fact_ref)
    .ok_or_else(|| anyhow::anyhow!("fact ref {:?} not found", fact_ref))?;
```

- [ ] **Step 4: Fix direct indexing (Pattern G)**

```rust
// Before
let page_data = &all_pages[page_idx];

// After
let page_data = all_pages.get(page_idx)
    .ok_or_else(|| anyhow::anyhow!("page index {page_idx} out of range"))?;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::indexing_slicing 2>&1 \
  | grep "src/storage/persistent_facts.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/storage/persistent_facts.rs
git commit -m "fix(lint): resolve unwrap/indexing violations in storage/persistent_facts"
```

---

## Task 11: Fix `src/query/datalog/types.rs`, `matcher.rs`, `functions.rs`

**Files:**
- Modify: `src/query/datalog/types.rs` (3 violations — Pattern I)
- Modify: `src/query/datalog/matcher.rs` (9 violations — Pattern G)
- Modify: `src/query/datalog/functions.rs` (6 violations — Pattern I)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::cast_possible_wrap -W clippy::cast_possible_truncation \
  -W clippy::cast_sign_loss -W clippy::indexing_slicing 2>&1 \
  | grep -E "src/query/datalog/(types|matcher|functions)\.rs"
```

- [ ] **Step 2: Fix `query/datalog/types.rs` — casts (Pattern I)**

Three lossy casts, likely `u64 as i64` for timestamps. Apply `i64::try_from(x).unwrap_or(i64::MAX)` (same as Task 3, Step 3 for non-fallible contexts) or `i64::try_from(x)?` where the function is fallible.

- [ ] **Step 3: Fix `query/datalog/matcher.rs` — indexing (Pattern G)**

The matcher indexes into token/clause slices. Apply `.get()`:
```rust
// Before
let head = &clauses[0];
let rest = &clauses[1..];

// After
let head = clauses.get(0)
    .ok_or_else(|| "empty clause list".to_string())?;
let rest = clauses.get(1..)
    .ok_or_else(|| "clause list has no tail".to_string())?;
```
Note: `matcher.rs` returns `Result<_, String>` (not anyhow), so error values are `String`.

- [ ] **Step 4: Fix `query/datalog/functions.rs` — casts (Pattern I)**

Six casts, likely `f64 as i64` in arithmetic builtins and `u64 as i64` for timestamps. For `f64 as i64`, perform a range check first:
```rust
// Before
let i = val as i64;

// After
if !val.is_finite() || val < i64::MIN as f64 || val > i64::MAX as f64 {
    return Err(anyhow::anyhow!("float {val} out of i64 range"));
}
#[allow(clippy::cast_possible_truncation)] // range-checked above
let i = val as i64;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::cast_possible_wrap -W clippy::cast_possible_truncation \
  -W clippy::cast_sign_loss -W clippy::indexing_slicing 2>&1 \
  | grep -E "src/query/datalog/(types|matcher|functions)\.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/matcher.rs src/query/datalog/functions.rs
git commit -m "fix(lint): resolve cast/indexing violations in query/datalog types, matcher, functions"
```

---

## Task 12: Fix `src/query/datalog/executor.rs`

**Files:**
- Modify: `src/query/datalog/executor.rs` (27 violations — Patterns A, D, G)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::indexing_slicing 2>&1 \
  | grep "src/query/datalog/executor.rs"
```

- [ ] **Step 2: Fix RwLock reads (Pattern A)**

The executor holds an `Arc<RwLock<FactStorage>>`. Lock reads:
```rust
// Before
let facts = storage.read().unwrap();

// After
let facts = storage.read().map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
```

- [ ] **Step 3: Fix `unwrap()` on Options (Pattern D)**

Commonly in binding lookups:
```rust
// Before
let val = bindings.get(&var).unwrap();

// After
let val = bindings.get(&var)
    .ok_or_else(|| anyhow::anyhow!("unbound variable: {var}"))?;
```

- [ ] **Step 4: Fix direct indexing (Pattern G)**

Clause and result row indexing:
```rust
// Before
let first = &rows[0];

// After
let first = rows.get(0)
    .ok_or_else(|| anyhow::anyhow!("empty result set"))?;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::indexing_slicing 2>&1 \
  | grep "src/query/datalog/executor.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "fix(lint): resolve unwrap/indexing violations in query/datalog/executor"
```

---

## Task 13: Fix `src/query/datalog/evaluator.rs`

**Files:**
- Modify: `src/query/datalog/evaluator.rs` (23 violations — Patterns C, D, G)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::indexing_slicing 2>&1 | grep "src/query/datalog/evaluator.rs"
```

- [ ] **Step 2: Fix `unwrap()` and `expect()` on Results and Options**

Apply Patterns C and D. The evaluator returns `anyhow::Result`, so `?` propagates. For `expect("message")`, use Pattern E (debug_assert + ok_or_else).

- [ ] **Step 3: Fix direct indexing (Pattern G)**

The semi-naive evaluator iterates over strata and rule bodies with direct indexing:
```rust
// Before
let stratum = strata[i];
let head = &rule.body[0];

// After
let stratum = strata.get(i)
    .ok_or_else(|| anyhow::anyhow!("stratum index {i} out of range"))?;
let head = rule.body.get(0)
    .ok_or_else(|| anyhow::anyhow!("rule has empty body"))?;
```

- [ ] **Step 4: Verify**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::indexing_slicing 2>&1 | grep "src/query/datalog/evaluator.rs" | wc -l
```
Expected: 0

- [ ] **Step 5: Commit**

```bash
git add src/query/datalog/evaluator.rs
git commit -m "fix(lint): resolve unwrap/indexing violations in query/datalog/evaluator"
```

---

## Task 14: Fix `src/query/datalog/parser.rs`

**Files:**
- Modify: `src/query/datalog/parser.rs` (64 violations — Patterns G, C, K)

This is the largest file. The parser returns `Result<_, String>` throughout (not anyhow), so all error values are `String`. Use `.to_string()` or the `format!` macro for error messages.

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::panic -W clippy::unimplemented 2>&1 | grep "src/query/datalog/parser.rs"
```

- [ ] **Step 2: Fix direct token/slice indexing (Pattern G — String errors)**

The parser indexes into `tokens: Vec<Token>` and `chars: Vec<char>`:
```rust
// Before
let tok = tokens[pos];

// After
let tok = tokens.get(pos)
    .ok_or_else(|| format!("unexpected end of input at position {pos}"))?;
```
For slices:
```rust
// Before
let rest = &tokens[pos..];

// After
let rest = tokens.get(pos..)
    .ok_or_else(|| format!("token slice out of range at {pos}"))?;
```

- [ ] **Step 3: Fix `unwrap()` on Results (Pattern C — String errors)**

```rust
// Before
let n: i64 = s.parse().unwrap();

// After
let n: i64 = s.parse().map_err(|e| format!("invalid integer '{s}': {e}"))?;
```

- [ ] **Step 4: Fix `panic!` / `unimplemented!` (Pattern K — String errors)**

```rust
// Before
panic!("unexpected token");
unimplemented!("nested map parsing");

// After
return Err(format!("unexpected token"));
return Err(format!("unsupported syntax: nested map"));
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::indexing_slicing -W clippy::unwrap_used \
  -W clippy::panic -W clippy::unimplemented 2>&1 \
  | grep "src/query/datalog/parser.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "fix(lint): resolve indexing/unwrap/panic violations in query/datalog/parser"
```

---

## Task 15: Fix `src/db.rs`, `src/wal.rs`, `src/temporal.rs`

**Files:**
- Modify: `src/db.rs` (7 violations — Patterns A, C)
- Modify: `src/wal.rs` (3 violations — Pattern C)
- Modify: `src/temporal.rs` (1 violation — Pattern I)

- [ ] **Step 1: Find all violations**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::cast_sign_loss 2>&1 | grep -E "src/(db|wal|temporal)\.rs"
```

- [ ] **Step 2: Fix `db.rs`**

`db.rs` is the public API layer. Violations are likely `Mutex`/`RwLock` unwraps on the internal storage handle (Pattern A) and `unwrap()` on Results (Pattern C). Functions return `anyhow::Result`:
```rust
// Before
let storage = self.storage.lock().unwrap();

// After
let storage = self.storage.lock().map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
```

- [ ] **Step 3: Fix `wal.rs`**

WAL file operations return `anyhow::Result`; replace `.unwrap()` with `?`.

- [ ] **Step 4: Fix `temporal.rs`**

One cast violation (`u64 as i64` for timestamp). Apply Pattern I:
```rust
// Before
let ms = duration.as_millis() as i64;

// After
let ms = i64::try_from(duration.as_millis())
    .map_err(|_| anyhow::anyhow!("timestamp overflows i64"))?;
```

- [ ] **Step 5: Verify**

```bash
cargo clippy --lib -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::cast_sign_loss 2>&1 | grep -E "src/(db|wal|temporal)\.rs" | wc -l
```
Expected: 0

- [ ] **Step 6: Commit**

```bash
git add src/db.rs src/wal.rs src/temporal.rs
git commit -m "fix(lint): resolve unwrap/cast violations in db, wal, temporal"
```

---

## Task 16: Fix `minigraf-c/src/lib.rs`

**Files:**
- Modify: `minigraf-c/src/lib.rs` (6 violations — Patterns B, J)

This is a C FFI crate. Functions cannot propagate errors with `?` — instead they return raw pointers or C integers. The fix must not panic across FFI boundaries.

- [ ] **Step 1: Find all violations**

```bash
cargo clippy -p minigraf-c -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::panic 2>&1 | grep "minigraf-c/src/lib.rs"
```

- [ ] **Step 2: Fix `set_error` and `clear_error` — Mutex (Pattern B)**

```rust
// Before
fn set_error(&self, msg: String) {
    *self.last_error.lock().unwrap() =
        Some(CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap()));
}

fn clear_error(&self) {
    *self.last_error.lock().unwrap() = None;
}

// After
fn set_error(&self, msg: String) {
    // Recover from lock poison — in FFI, stale error is better than aborting
    let cs = CString::new(msg).unwrap_or_else(|_| c"error".to_owned());
    *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(cs);
}

fn clear_error(&self) {
    *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
```

Note: `c"error"` is a `&'static CStr` literal (stable since Rust 1.77, available in edition 2024). `.to_owned()` yields `CString`.

- [ ] **Step 3: Fix any remaining `unwrap()` calls**

For any `unwrap()` on `Result` in FFI functions (which return `*mut T` or `c_int`):
```rust
// Before
let result = db.execute(query).unwrap();

// After — FFI: capture error into last_error, return null/error code
match db.execute(query) {
    Ok(r) => { /* use r */ }
    Err(e) => {
        handle.set_error(e.to_string());
        return std::ptr::null_mut(); // or -1 for c_int returns
    }
}
```

- [ ] **Step 4: Verify**

```bash
cargo clippy -p minigraf-c -- -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::panic 2>&1 | grep "minigraf-c/src/lib.rs" | wc -l
```
Expected: 0

- [ ] **Step 5: Commit**

```bash
git add minigraf-c/src/lib.rs
git commit -m "fix(lint): resolve unwrap/panic violations in minigraf-c FFI layer"
```

---

## Task 17: Final verification and PR

**Files:** none

- [ ] **Step 1: Full workspace clippy — zero errors**

```bash
cargo clippy --workspace 2>&1 | grep "^error" | wc -l
```
Expected: 0

- [ ] **Step 2: Check for residual deny violations specifically**

```bash
cargo clippy --workspace 2>&1 | grep -E "clippy::(unwrap_used|expect_used|panic|indexing_slicing|cast_possible)" | wc -l
```
Expected: 0

- [ ] **Step 3: All tests pass**

```bash
cargo test --workspace 2>&1 | tail -5
```
Expected: all tests pass with no failures.

- [ ] **Step 4: Check warn-level lints are visible but not blocking**

```bash
cargo clippy --workspace 2>&1 | grep "^warning\[clippy::" | grep -E "(unreachable|arithmetic_side_effects|missing_errors_doc)" | wc -l
```
Note the count for future reference — these are advisory and do not fail the build.

- [ ] **Step 5: Create PR**

```bash
git push -u origin fix/clippy-lint-enforcement
gh pr create \
  --title "chore: add production clippy lints and fix 336 violations" \
  --body "$(cat <<'EOF'
## Summary
- Adds `[workspace.lints.clippy]` with 11 lint rules (deny/warn) across the workspace
- Fixes all 336 existing violations in production code (330 lib + 6 minigraf-c)
- Test code exempted via `cfg_attr(test, allow(...))` — panicking in tests is intentional
- Panic-prevention lints: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` → deny
- Numeric safety lints: `cast_possible_truncation/wrap/sign_loss` → deny
- Bounds safety: `indexing_slicing` → deny
- Advisory: `unreachable`, `arithmetic_side_effects`, `missing_errors_doc` → warn

## Test plan
- [ ] `cargo clippy --workspace` produces zero errors
- [ ] `cargo test --workspace` all pass
- [ ] No `#[allow(clippy::unwrap_used)]` outside of proven-safe documented cases
EOF
)"
```
