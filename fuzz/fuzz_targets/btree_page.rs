//! Fuzz B+tree node decoding (leaf and internal, every tree) (#375).
//!
//! Input byte 0 picks the node: bit 7 an internal node (else a leaf), bits 0-6
//! which one. Byte 1: bit 0 flips the type byte between leaf and internal, bit
//! 1 opens read-only. Byte 2 picks the probe (`common::probe`). The remaining
//! bytes overwrite the node's `count` field (offset 2) and body (offset 24).
//! The type, page id and generation stay, and the page CRC is recomputed, so
//! the page reaches the node decoder instead of failing the header check.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "common/mod.rs"]
mod common;
use common::*;

fuzz_target!(|data: &[u8]| {
    let [sel, flags, pick, rest @ ..] = data else {
        return;
    };
    let t = template();
    let pages = if sel & 0x80 != 0 {
        &t.internals
    } else {
        &t.leaves
    };
    let id = pages[usize::from(sel & 0x7f) % pages.len()];

    let mut bytes = t.bytes.clone();
    let page = page_mut(&mut bytes, id);
    if flags & 0x01 != 0 {
        page[0] = if page[0] == PAGE_TYPE_LEAF {
            PAGE_TYPE_INTERNAL
        } else {
            PAGE_TYPE_LEAF
        };
    }
    let (count, body) = rest.split_at(rest.len().min(2));
    page[2..2 + count.len()].copy_from_slice(count);
    let n = body.len().min(PAGE_SIZE - 24);
    page[24..24 + n].copy_from_slice(&body[..n]);
    fix_page_crc(page);

    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    if let Some(db) = write_and_open(dir.path(), &bytes, flags & 0x02 != 0) {
        probe(&db, *pick);
    }
});
