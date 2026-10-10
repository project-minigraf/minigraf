//! Opening a damaged file must return an error or correct data, never wrong
//! data.
//!
//! The template is built once per process from fixed statements over three
//! checkpoints, with no WAL left; after each checkpoint the answers of a fixed
//! set of queries are recorded. The input damages a copy of it:
//!
//! - byte 0: bit 0 opens read-only; bits 1–2 pick a truncation: 1 = to a page
//!   multiple, 2 = to any length, else none
//! - bytes 1..5: the truncation length (u32 LE, modulo the file length)
//! - then up to 16 edits of 5 bytes: offset (u32 LE, modulo the file length)
//!   and a byte to xor in (0 becomes 1)
//!
//! Checksums are **not** recomputed: the CRCs are the contract under test.
//! Each query must fail or return exactly the answer of the last generation or
//! of the one before it (the meta fallback when the active meta page is
//! damaged), and every query that answers must agree on one generation. A
//! mismatch panics. `verify()` runs too and may fail, but must not panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use minigraf::{Minigraf, OpenOptions, QueryResult};
use std::sync::OnceLock;

const PAGE_SIZE: usize = 4096;
const MAX_EDITS: usize = 16;

/// Queries that between them read every index (EAVT, AEVT, AVET, VAET), the
/// DICT (idents and long values) and the time-travel paths.
const QUERIES: [&str; 8] = [
    "(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])",
    "(query [:find ?a ?v :where [:e1 ?a ?v]])",
    "(query [:find ?e ?v :where [?e :a/num ?v]])",
    "(query [:find ?e :where [?e :a/num 7]])",
    "(query [:find ?e :where [?e :a/next :e5]])",
    "(query [:find ?v :valid-at \"2002-01-01\" :where [:e0 :a/long ?v]])",
    "(query [:find ?e ?v :as-of 1 :where [?e :a/name ?v]])",
    "(query [:find ?e ?v :where [?e :a/note ?v]])",
];

/// One generation's answers: the tx count, then one sorted row list per query.
/// Rows are compared, never printed (a `Ref` renders its `Uuid`).
#[derive(PartialEq)]
struct Answers {
    tx_count: u64,
    rows: Vec<Vec<String>>,
}

struct Template {
    bytes: Vec<u8>,
    /// Answers after each of the three checkpoints, oldest first.
    generations: Vec<Answers>,
}

fn rows(db: &Minigraf, query: &str) -> Option<Vec<String>> {
    match db.execute(query).ok()? {
        QueryResult::QueryResults { results, .. } => {
            let mut out: Vec<String> = results.iter().map(|r| format!("{r:?}")).collect();
            out.sort();
            Some(out)
        }
        _ => None,
    }
}

fn answers(db: &Minigraf) -> Answers {
    Answers {
        tx_count: db.current_tx_count(),
        rows: QUERIES
            .iter()
            .map(|q| rows(db, q).expect("template query"))
            .collect(),
    }
}

fn template() -> &'static Template {
    static T: OnceLock<Template> = OnceLock::new();
    T.get_or_init(build_template)
}

fn build_template() -> Template {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("template.graph");
    let long = |c: &str| c.repeat(300);
    let mut generations = Vec::new();
    {
        let db = Minigraf::open(&path).expect("open");
        // Checkpoint 1: enough entities for internal nodes in every index.
        let mut facts = String::new();
        for i in 0..90 {
            facts.push_str(&format!(
                "[:e{i} :a/num {i}] [:e{i} :a/name \"name{i}\"] [:e{i} :a/next :e{}] ",
                i + 1
            ));
        }
        db.execute(&format!("(transact [{facts}])"))
            .expect("transact");
        db.checkpoint().expect("checkpoint");
        generations.push(answers(&db));

        // Checkpoint 2: long values (value pages), a valid-time window, a
        // retraction, and new values for existing triples.
        db.execute(&format!(
            "(transact {{:valid-from \"2001-01-01\" :valid-to \"2005-01-01\"}} \
             [[:e0 :a/long \"{}\"] [:e1 :a/flag true] [:e2 :a/kw :k/x]])",
            long("L")
        ))
        .expect("transact");
        db.execute("(retract [[:e3 :a/num 3] [:e7 :a/num 7]])")
            .expect("retract");
        db.execute(&format!(
            "(transact [[:e1 :a/note \"{}\"] [:e7 :a/num 70] [:e9 :a/next :e5]])",
            long("N")
        ))
        .expect("transact");
        db.checkpoint().expect("checkpoint");
        generations.push(answers(&db));

        // Checkpoint 3: copy-on-write over the same leaves, freeing pages.
        db.execute(&format!(
            "(transact [[:e40 :a/note \"{}\"] [:e7 :a/num 7] [:e1 :a/name \"renamed\"]])",
            long("M")
        ))
        .expect("transact");
        db.execute("(retract [[:e4 :a/next :e5] [:e1 :a/flag true]])")
            .expect("retract");
        db.checkpoint().expect("checkpoint");
        generations.push(answers(&db));
    }
    let mut wal = path.as_os_str().to_owned();
    wal.push(".wal");
    assert!(
        !std::path::Path::new(&wal).exists(),
        "template must have no WAL"
    );
    let bytes = std::fs::read(&path).expect("read template");
    assert!(
        generations[1] != generations[2],
        "the last two generations must differ"
    );
    Template { bytes, generations }
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    let b = data.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

fuzz_target!(|data: &[u8]| {
    let Some(&flags) = data.first() else {
        return;
    };
    let t = template();
    let len = t.bytes.len();
    let mut bytes = t.bytes.clone();
    for edit in data.get(5..).unwrap_or(&[]).chunks_exact(5).take(MAX_EDITS) {
        let off = u32_at(edit, 0).unwrap_or(0) % len;
        bytes[off] ^= edit[4].max(1);
    }
    let trunc = u32_at(data, 1).unwrap_or(0) % len;
    match (flags >> 1) & 0x03 {
        1 => bytes.truncate(trunc / PAGE_SIZE * PAGE_SIZE),
        2 => bytes.truncate(trunc),
        _ => {}
    }
    // An empty file opens as a new, empty database. A file shorter than one
    // page does too today (#506); skipped until that is fixed.
    if bytes.len() < PAGE_SIZE {
        return;
    }

    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    let path = dir.path().join("fuzz.graph");
    if std::fs::write(&path, &bytes).is_err() {
        return;
    }
    let opts = OpenOptions::new().read_only(flags & 0x01 != 0);
    let Ok(db) = Minigraf::open_with_options(&path, opts) else {
        return;
    };

    // The last generation, or the one before it; narrowed by every answer.
    let mut candidates: Vec<&Answers> = t.generations[1..].iter().collect();
    candidates.retain(|g| g.tx_count == db.current_tx_count());
    assert!(
        !candidates.is_empty(),
        "opened with a tx count of no recent generation"
    );
    for (i, q) in QUERIES.iter().enumerate() {
        let Some(got) = rows(&db, q) else {
            continue;
        };
        candidates.retain(|g| g.rows[i] == got);
        assert!(
            !candidates.is_empty(),
            "query {i} answered with data of no recent generation, or of a \
             different generation than the queries before it"
        );
    }
    let _ = db.verify();
});
