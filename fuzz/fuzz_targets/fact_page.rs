//! Fuzz format v7 packed fact page decoding, which runs only when a v7 file is
//! opened (read-only: loaded in memory; read-write: migrated to v8).
//!
//! The template is the golden v7 file `tests/golden/v7_basic.graph`. Input
//! byte 0 bit 0 picks read-only; the remaining bytes overwrite fact page 1.
//! The v7 header checksum covers only the header, so the page reaches the
//! decoder.
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "common/mod.rs"]
mod common;
use common::*;

const V7_BASIC: &[u8] = include_bytes!("../../tests/golden/v7_basic.graph");

fuzz_target!(|data: &[u8]| {
    let Some((&sel, rest)) = data.split_first() else {
        return;
    };
    let mut bytes = V7_BASIC.to_vec();
    let page = page_mut(&mut bytes, 1);
    let n = rest.len().min(PAGE_SIZE);
    page[..n].copy_from_slice(&rest[..n]);

    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    if let Some(db) = write_and_open(dir.path(), &bytes, sel & 0x01 != 0) {
        let _ = db.execute("(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])");
        let _ = db.execute("(query [:find ?n :where [?e :person/name ?n]])");
        let _ = db.verify();
    }
});
