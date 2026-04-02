# Header Checksum Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a header_checksum field (CRC32) to protect page 0 (header) from corruption. Bump format to v7.

**Architecture:** Add 4-byte `header_checksum` field at bytes 80-83 of the 80-byte header, extending it to 84 bytes. Compute checksum over header bytes 0-79 at checkpoint time; validate on open with hard failure on mismatch.

**Tech Stack:** Rust, crc32fast crate (already in use for index_checksum)

---

### Task 1: Add header_checksum field to FileHeader struct

**Files:**
- Modify: `src/storage/mod.rs:83-101`

- [ ] **Step 1: Read current FileHeader struct**

```rust
#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub page_count: u64,
    pub node_count: u64,
    pub last_checkpointed_tx_count: u64,
    pub eavt_root_page: u64,
    pub aevt_root_page: u64,
    pub avet_root_page: u64,
    pub vaet_root_page: u64,
    pub index_checksum: u32,
    pub fact_page_format: u8,
    pub(crate) _padding: [u8; 3],
    pub fact_page_count: u64,
}
```

- [ ] **Step 2: Add header_checksum field**

Add after `fact_page_count`:
```rust
    pub header_checksum: u32,
```

- [ ] **Step 3: Update doc comment for header layout**

Change the doc comment at line 67 to reflect v7 (84 bytes):
```rust
/// File header for .graph files — 84 bytes in v7.
///
/// Layout (all fields little-endian):
///   0..4    magic ("MGRF")
///   4..8    version (u32)
///   8..16   page_count (u64)
///   16..24  node_count (u64)
///   24..32  last_checkpointed_tx_count (u64)
///   32..40  eavt_root_page (u64)
///   40..48  aevt_root_page (u64)
///   48..56  avet_root_page (u64)
///   56..64  vaet_root_page (u64)
///   64..68  index_checksum (u32)
///   68      fact_page_format (u8)
///   69..72  _padding ([u8; 3])
///   72..80  fact_page_count (u64)
///   80..84  header_checksum (u32)   — new in v7
```

- [ ] **Step 4: Update FileHeader::new() to initialize header_checksum**

Add to the `new()` function:
```rust
    fact_page_count: 0,
    header_checksum: 0,
```

- [ ] **Step 5: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat: add header_checksum field to FileHeader (v7)"
```

---

### Task 2: Update serialization/deserialization

**Files:**
- Modify: `src/storage/mod.rs:124-140` (to_bytes)
- Modify: `src/storage/mod.rs:149-230` (from_bytes)

- [ ] **Step 1: Update to_bytes() to serialize header_checksum**

Current code (lines 124-140):
```rust
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(80);
        b.extend_from_slice(&self.magic);
        b.extend_from_slice(&self.version.to_le_bytes());
        b.extend_from_slice(&self.page_count.to_le_bytes());
        b.extend_from_slice(&self.node_count.to_le_bytes());
        b.extend_from_slice(&self.last_checkpointed_tx_count.to_le_bytes());
        b.extend_from_slice(&self.eavt_root_page.to_le_bytes());
        b.extend_from_slice(&self.aevt_root_page.to_le_bytes());
        b.extend_from_slice(&self.avet_root_page.to_le_bytes());
        b.extend_from_slice(&self.vaet_root_page.to_le_bytes());
        b.extend_from_slice(&self.index_checksum.to_le_bytes());
        b.push(self.fact_page_format);
        b.extend_from_slice(&self._padding);
        b.extend_from_slice(&self.fact_page_count.to_le_bytes());
        b
    }
```

Change capacity to 84 and add header_checksum:
```rust
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(84);
        // ... same fields ...
        b.extend_from_slice(&self.fact_page_count.to_le_bytes());
        b.extend_from_slice(&self.header_checksum.to_le_bytes());
        b
    }
```

- [ ] **Step 2: Update from_bytes() to handle v7 header**

Find where version is checked and add v7 handling. Current code handles v3, v4/v5, v6. Add v7 case:
```rust
        // v7: full header with header_checksum
        let header_checksum = if bytes.len() >= 84 {
            u32::from_le_bytes(bytes[80..84].try_into().unwrap())
        } else {
            0
        };
        
        Ok(FileHeader {
            // ... existing fields ...
            fact_page_count,
            header_checksum,
        })
```

- [ ] **Step 3: Update capacity constant if exists**

Search for `WAL_HEADER_SIZE` or similar constants - there isn't one for file header but verify Vec::with_capacity is 84.

- [ ] **Step 4: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat: serialize/deserialize header_checksum field"
```

---

### Task 3: Add checksum computation and validation

**Files:**
- Modify: `src/storage/persistent_facts.rs`
- Add: Test in same file

- [ ] **Step 1: Add checksum computation function**

Add a public function to compute header checksum:
```rust
/// Compute CRC32 checksum over header bytes 0-79 (header_checksum field zeroed).
pub fn compute_header_checksum(header: &FileHeader) -> u32 {
    use crc32fast::Hasher;
    let mut bytes = header.to_bytes();
    // Zero out the header_checksum field (bytes 80-83) for computation
    bytes[80] = 0;
    bytes[81] = 0;
    bytes[82] = 0;
    bytes[83] = 0;
    Hasher::new().update(&bytes[..80]).finalize()
}
```

- [ ] **Step 2: Find where checkpoint happens and add checksum computation**

Search for `index_checksum = computed` in persistent_facts.rs. Add similar line:
```rust
new_header.header_checksum = compute_header_checksum(&new_header);
```

- [ ] **Step 3: Add validation on load**

Find where header is validated (after from_bytes). Add:
```rust
if header.header_checksum != 0 && header.header_checksum != compute_header_checksum(&header) {
    anyhow::bail!("Header checksum mismatch: possible file corruption. Database may be damaged.");
}
```

Note: header_checksum = 0 means v6 file being opened (backward compat), skip validation.

- [ ] **Step 4: Add unit test**

```rust
#[test]
fn test_header_checksum_computation() {
    use crate::storage::FileHeader;
    
    let mut header = FileHeader::new();
    header.page_count = 10;
    header.node_count = 5;
    
    let checksum = compute_header_checksum(&header);
    assert_ne!(checksum, 0, "checksum must be non-zero");
    
    // Verify: same header produces same checksum
    let mut header2 = FileHeader::new();
    header2.page_count = 10;
    header2.node_count = 5;
    assert_eq!(compute_header_checksum(&header2), checksum);
    
    // Verify: different header produces different checksum
    let mut header3 = FileHeader::new();
    header3.page_count = 11;
    assert_ne!(compute_header_checksum(&header3), checksum);
}
```

- [ ] **Step 5: Add corruption detection test**

```rust
#[test]
fn test_header_checksum_corruption_detection() {
    use crate::storage::{FileHeader, MAGIC_NUMBER, FORMAT_VERSION};
    
    let mut header = FileHeader::new();
    header.version = FORMAT_VERSION;
    let valid_checksum = compute_header_checksum(&header);
    header.header_checksum = valid_checksum;
    
    // Corrupt a field
    header.page_count = 999;
    
    // Validation should fail
    let computed = compute_header_checksum(&header);
    assert_ne!(computed, header.header_checksum);
}
```

- [ ] **Step 6: Commit**

```bash
git add src/storage/persistent_facts.rs
git commit -m "feat: add header_checksum computation and validation"
```

---

### Task 4: Bump format version to v7

**Files:**
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: Change FORMAT_VERSION from 6 to 7**

```rust
pub const FORMAT_VERSION: u32 = 7;
```

- [ ] **Step 2: Update doc comment**

Change from "v6" to "v7" in the FileHeader doc comment.

- [ ] **Step 3: Commit**

```bash
git add src/storage/mod.rs
git commit -m "feat: bump format version to v7"
```

---

### Task 5: Run full test suite

**Files:**
- Run: `cargo test`

- [ ] **Step 1: Run all tests**

```bash
cargo test
```

- [ ] **Step 2: If failures, debug and fix**

Common issues:
- Off-by-one in byte offsets
- Version check in from_bytes needs updating
- Tests checking exact header size

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test: header checksum implementation complete"
```

---

### Task 6: Push and create PR

**Files:**
- Run: `git push` and `gh pr create`

- [ ] **Step 1: Push branch**

```bash
git push -u origin fix/issue-39-header-checksum
```

- [ ] **Step 2: Create PR**

```bash
gh pr create --title "fix: add header checksum to protect page 0 from corruption" --body "Fixes #39"
```

- [ ] **Step 3: Commit**

After merge, note completion.
