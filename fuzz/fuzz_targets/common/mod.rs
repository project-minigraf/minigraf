//! Shared by the page targets: a real format v8 file built through the public
//! API, page lookups by type, checksum fix-ups, and the probe queries.
//!
//! The fuzz crate only sees the public API, so a target that wants to reach a
//! decoder behind a checksum builds a valid file, overwrites one page with
//! fuzzer bytes, and recomputes that page's checksum itself. The layouts are in
//! `src/storage/page.rs` and `src/storage/meta.rs` (v8 spec §4).
#![allow(dead_code)]

use minigraf::{FactFilter, LogWriter, Minigraf, OpenOptions};
use std::path::Path;
use std::sync::OnceLock;

pub const PAGE_SIZE: usize = 4096;

pub const PAGE_TYPE_VALUE: u8 = 0x51;
pub const PAGE_TYPE_LEAF: u8 = 0x61;
pub const PAGE_TYPE_INTERNAL: u8 = 0x62;
pub const PAGE_TYPE_FREELIST: u8 = 0x81;

/// CRC field of a non-meta page (common page header) and of a meta page.
const PAGE_CRC: std::ops::Range<usize> = 4..8;
const META_CRC: std::ops::Range<usize> = 12..16;

/// A file written by one checkpoint, with no WAL left. Every tree page is
/// reachable from the meta page (one checkpoint frees nothing), and the trees
/// are big enough to have internal nodes.
pub struct Template {
    pub bytes: Vec<u8>,
    /// Page ids by type, from the page headers.
    pub leaves: Vec<usize>,
    pub internals: Vec<usize>,
    pub values: Vec<usize>,
}

pub fn template() -> &'static Template {
    static T: OnceLock<Template> = OnceLock::new();
    T.get_or_init(build_template)
}

fn build_template() -> Template {
    let dir = tempfile::tempdir().expect("tempdir");
    let draft = dir.path().join("draft.graph");
    {
        let db = Minigraf::open(&draft).expect("open draft");
        let mut facts = String::new();
        // 90 entities give the four fact indexes two leaves and an internal root
        // each, while keeping a full scan cheap.
        for i in 0..90 {
            facts.push_str(&format!(
                "[:e{i} :a/num {i}] [:e{i} :a/name \"name{i}\"] [:e{i} :a/next :e{}] ",
                i + 1
            ));
        }
        db.execute(&format!("(transact [{facts}])"))
            .expect("transact");
        let long = "L".repeat(200);
        db.execute(&format!(
            "(transact {{:valid-from \"2001-01-01\" :valid-to \"2005-01-01\"}} \
             [[:e0 :a/long \"{long}\"] [:e1 :a/flag true] [:e2 :a/kw :k/x]])"
        ))
        .expect("transact");
        db.execute("(retract [[:e3 :a/num 3]])").expect("retract");
    }
    // Rewrite with fixed tx times, so the bytes, and the seeds cut from them,
    // are the same in every process. One batch: one checkpoint, nothing freed.
    let path = dir.path().join("template.graph");
    {
        let src = Minigraf::open_with_options(&draft, OpenOptions::new().read_only(true))
            .expect("open draft");
        let mut out = LogWriter::create(&path, OpenOptions::new()).expect("log writer");
        for rec in src.fact_log(&FactFilter::new()).expect("fact log") {
            let mut rec = rec.expect("fact record");
            let tx_id = 1_000_000_000_000 + rec.tx_count * 1000;
            if rec.valid_from == rec.tx_id as i64 {
                rec.valid_from = tx_id as i64;
            }
            rec.tx_id = tx_id;
            out.append(&rec).expect("append");
        }
        out.advance_tx_count(src.current_tx_count())
            .expect("advance");
        out.finish().expect("finish");
    }
    let bytes = std::fs::read(&path).expect("read template");
    let (mut leaves, mut internals, mut values) = (Vec::new(), Vec::new(), Vec::new());
    for (id, page) in bytes.chunks(PAGE_SIZE).enumerate().skip(2) {
        let own_id = page
            .get(8..16)
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
            .map(u64::from_le_bytes);
        if own_id != Some(id as u64) {
            continue;
        }
        match page[0] {
            PAGE_TYPE_LEAF => leaves.push(id),
            PAGE_TYPE_INTERNAL => internals.push(id),
            PAGE_TYPE_VALUE => values.push(id),
            _ => {}
        }
    }
    assert!(
        !leaves.is_empty() && !internals.is_empty() && !values.is_empty(),
        "template must have leaf, internal and value pages"
    );
    Template {
        bytes,
        leaves,
        internals,
        values,
    }
}

fn crc_with_zeroed(page: &[u8], field: std::ops::Range<usize>) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(&page[..field.start]);
    h.update(&[0u8; 4]);
    h.update(&page[field.end..]);
    h.finalize()
}

/// Recompute the CRC of a non-meta page in place.
pub fn fix_page_crc(page: &mut [u8]) {
    let crc = crc_with_zeroed(page, PAGE_CRC);
    page[PAGE_CRC].copy_from_slice(&crc.to_le_bytes());
}

/// Recompute the CRC of a meta page in place.
pub fn fix_meta_crc(page: &mut [u8]) {
    let crc = crc_with_zeroed(page, META_CRC);
    page[META_CRC].copy_from_slice(&crc.to_le_bytes());
}

pub fn page_mut(bytes: &mut [u8], id: usize) -> &mut [u8] {
    &mut bytes[id * PAGE_SIZE..(id + 1) * PAGE_SIZE]
}

/// Write `bytes` as a database file and open it, read-only or not. The temp
/// dir must outlive the handle.
pub fn write_and_open(dir: &Path, bytes: &[u8], read_only: bool) -> Option<Minigraf> {
    let path = dir.join("fuzz.graph");
    std::fs::write(&path, bytes).ok()?;
    Minigraf::open_with_options(&path, OpenOptions::new().read_only(read_only)).ok()
}

/// Queries that between them scan every index (EAVT, AEVT, AVET, VAET), the
/// DICT (idents and long values) and the time-travel paths. Results are
/// ignored: on a damaged file, errors are expected.
pub const PROBES: [&str; 7] = [
    "(query [:find ?a ?v :where [:e1 ?a ?v]])",
    "(query [:find ?e ?v :where [?e :a/num ?v]])",
    "(query [:find ?e :where [?e :a/num 7]])",
    "(query [:find ?e :where [?e :a/next :e5]])",
    "(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])",
    "(query [:find ?v :valid-at \"2002-01-01\" :where [:e0 :a/long ?v]])",
    "(query [:find ?e ?v :as-of 1 :where [?e :a/name ?v]])",
];

/// Run one probe query, or `verify()` for `pick % 8 == 7`. One per run keeps
/// executions fast; the fuzzer varies `pick` like any other input byte.
pub fn probe(db: &Minigraf, pick: u8) {
    match PROBES.get(usize::from(pick % 8)) {
        Some(q) => {
            let _ = db.execute(q);
        }
        None => {
            let _ = db.verify();
        }
    }
}
