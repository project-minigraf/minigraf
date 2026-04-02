# Issue #39: Header Page Not Protected by Checksum

## Problem

The `index_checksum` field in the file header only protects index pages, not page 0 (the header itself). If header fields (version, page counts, B+tree root pointers) are corrupted, this is undetectable.

**Impact:**
- Corrupted root pointers cause crashes/wrong data
- Corrupted `fact_page_count` causes missing facts on load

## Design

### Approach: Add `header_checksum` field

Add a new 4-byte `header_checksum` field (u32 CRC32) at bytes 80-83, extending header to 84 bytes. Bump format version to v7.

**Header layout (v7) — 84 bytes:**
```
0..4    magic ("MGRF")
4..8    version (u32) = 7
8..16   page_count (u64)
16..24  node_count (u64)
24..32  last_checkpointed_tx_count (u64)
32..40  eavt_root_page (u64)
40..48  aevt_root_page (u64)
48..56  avet_root_page (u64)
56..64  vaet_root_page (u64)
64..68  index_checksum (u32)
68      fact_page_format (u8)
69..72  _padding ([u8; 3])
72..80  fact_page_count (u64)
80..84  header_checksum (u32)  ← NEW
```

### Checksum computation

- **What it covers:** All header fields EXCEPT the `header_checksum` field itself (bytes 0-79, with bytes 80-83 zeroed during computation)
- **When computed:** At checkpoint time, after all data is written
- **When validated:** On database open, before any operations

**Hard failure** — If header checksum doesn't match, fail immediately with clear error:
```
Header checksum mismatch: possible file corruption. Database may be damaged.
```

The database cannot be opened. This follows the SQLite philosophy: silent corruption is worse than a loud crash.

### Implementation

1. **Add field to `FileHeader` struct** in `storage/mod.rs`
2. **Update serialization/deserialization** in `to_bytes()` / `from_bytes()`
3. **Add checksum computation** in `persistent_facts.rs` — compute CRC32 over header bytes 0-79
4. **Add validation on load** — fail fast with clear error if header checksum doesn't match
5. **Bump version to v7** in `FORMAT_VERSION`

### Testing

- Unit test for header checksum computation/validation
- Corruption test: modify header bytes, verify detection on open
- Migration test: open v6 file, verify still works (backward compat)
