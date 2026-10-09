//! Fuzz meta page decoding and the meta slot choice on open.
//!
//! The template has two valid meta pages (generation 1 in slot A, page 0, and
//! generation 2 in slot B, page 1). Input byte 0:
//! - bit 0: replace slot B (else slot A)
//! - bit 1: write the correct meta CRC over the result, so the field checks
//!   (page count, roots, free list, feature bits) are reached
//! - bit 2: zero the other slot, so the fuzzed one is the only candidate
//! - bit 3: keep the magic and version (bytes 0..16) and overwrite from the
//!   generation field on
//! - bit 4: open read-only
//!
//! Byte 1 picks the probe (`common::probe`). The remaining bytes overwrite the
//! slot.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "common/mod.rs"]
mod common;
use common::*;

fuzz_target!(|data: &[u8]| {
    let [sel, pick, rest @ ..] = data else {
        return;
    };
    let (slot, other) = if sel & 0x01 != 0 { (1, 0) } else { (0, 1) };
    let mut bytes = template().bytes.clone();
    let page = page_mut(&mut bytes, slot);
    let start = if sel & 0x08 != 0 { 16 } else { 0 };
    let n = rest.len().min(PAGE_SIZE - start);
    page[start..start + n].copy_from_slice(&rest[..n]);
    if sel & 0x02 != 0 {
        fix_meta_crc(page);
    }
    if sel & 0x04 != 0 {
        page_mut(&mut bytes, other).fill(0);
    }

    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    if let Some(db) = write_and_open(dir.path(), &bytes, sel & 0x10 != 0) {
        probe(&db, *pick);
    }
});
