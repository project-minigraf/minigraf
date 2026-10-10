//! Golden-file compatibility corpus (#391).
//!
//! Every `tests/golden/*.json` manifest names a committed `.graph` file (and
//! optionally its `.wal`), the CRC32 of each, the expected `tx_count`, and a list of
//! queries with their expected rows. This test opens a copy of each file with the
//! current build and checks the manifest before and after a checkpoint, then after
//! one more write. Golden files are never regenerated, only added: see
//! `tests/golden/README.md`.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};
use serde_json::Value as Json;

/// The major version of this reader, matched against a query's `min_reader`.
const READER: u64 = 3;

/// The newest file format this reader opens. Manifests for newer formats are skipped.
const MAX_FORMAT: u64 = 8;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

/// Canonical text for one value, as written in manifest `rows`.
fn render(v: &Value) -> String {
    match v {
        Value::String(s) => serde_json::to_string(s).unwrap(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Keyword(k) if k.starts_with(':') => k.clone(),
        Value::Keyword(k) => format!(":{k}"),
        Value::Null => "nil".to_string(),
        // Entity ids are not a stable output; manifests never return them.
        Value::Ref(_) => "#ref".to_string(),
    }
}

/// True when meta slot A or B (pages 0 and 1) is a valid-looking v8 meta page:
/// `"MGRF"`, version 8, `"META"` (v8 spec §4.1).
fn has_v8_meta(path: &Path) -> bool {
    let bytes = std::fs::read(path).unwrap();
    bytes.chunks(4096).take(2).any(|page| {
        page.len() >= 12
            && &page[0..4] == b"MGRF"
            && page[4..8] == 8u32.to_le_bytes()
            && &page[8..12] == b"META"
    })
}

fn parse_crc(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("manifest crc32 must be hex")
}

struct Manifest {
    name: String,
    json: Json,
}

fn load_manifests() -> Vec<Manifest> {
    let mut out: Vec<Manifest> = std::fs::read_dir(golden_dir())
        .expect("tests/golden must exist")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| Manifest {
            name: p.file_name().unwrap().to_string_lossy().into_owned(),
            json: serde_json::from_slice(&std::fs::read(&p).unwrap())
                .expect("manifest must be valid JSON"),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Runs every query that applies to this reader and records each mismatch.
fn check_queries(m: &Manifest, db: &Minigraf, phase: &str, failures: &mut Vec<String>) {
    for q in m.json["queries"]
        .as_array()
        .expect("queries must be an array")
    {
        let qname = q["name"].as_str().expect("query name");
        if q["min_reader"].as_u64().is_some_and(|r| r > READER) {
            continue;
        }
        let expected: BTreeSet<Vec<String>> = q["rows"]
            .as_array()
            .expect("rows must be an array")
            .iter()
            .map(|row| {
                row.as_array()
                    .expect("row must be an array")
                    .iter()
                    .map(|c| c.as_str().expect("cell must be a string").to_string())
                    .collect()
            })
            .collect();
        let got: BTreeSet<Vec<String>> = match db.execute(q["query"].as_str().expect("query")) {
            Ok(QueryResult::QueryResults { results, .. }) => results
                .iter()
                .map(|row| row.iter().map(render).collect())
                .collect(),
            Ok(_) => {
                failures.push(format!("{} [{phase}] {qname}: not a query result", m.name));
                continue;
            }
            Err(e) => {
                failures.push(format!("{} [{phase}] {qname}: error {}", m.name, e.code()));
                continue;
            }
        };
        if got != expected {
            failures.push(format!(
                "{} [{phase}] {qname}: expected {expected:?}, got {got:?}",
                m.name
            ));
        }
    }
}

fn check_tx_count(m: &Manifest, db: &Minigraf, want: u64, phase: &str, f: &mut Vec<String>) {
    let got = db.current_tx_count();
    if got != want {
        f.push(format!(
            "{} [{phase}] tx_count: expected {want}, got {got}",
            m.name
        ));
    }
}

fn check_manifest(m: &Manifest, failures: &mut Vec<String>) {
    let dir = golden_dir();
    let file = m.json["file"].as_str().expect("manifest file");
    let src = dir.join(file);
    let wal = m.json["wal"].as_str().map(|w| dir.join(w));

    // 1. Frozen: the committed files match the manifest CRCs.
    let src_bytes = std::fs::read(&src).expect("golden file must exist");
    let wal_bytes = wal
        .as_ref()
        .map(|w| std::fs::read(w).expect("golden WAL must exist"));
    let crc = &m.json["crc32"];
    if crc32fast::hash(&src_bytes) != parse_crc(crc["graph"].as_str().expect("crc32.graph")) {
        failures.push(format!("{}: {file} does not match its crc32", m.name));
    }
    match (&wal_bytes, crc["wal"].as_str()) {
        (Some(b), Some(c)) if crc32fast::hash(b) == parse_crc(c) => {}
        (None, None) => {}
        _ => failures.push(format!("{}: WAL does not match its crc32", m.name)),
    }

    // 2. Skip formats this reader cannot open.
    if m.json["format"].as_u64().expect("format") > MAX_FORMAT {
        return;
    }

    // 3. Open a copy; never touch the committed file.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(file);
    std::fs::write(&path, &src_bytes).unwrap();
    if let Some(b) = &wal_bytes {
        std::fs::write(wal_path(&path), b).unwrap();
    }
    let tx_count = m.json["tx_count"].as_u64().expect("tx_count");

    // Read-only first (#429): the same view, and neither file changes.
    {
        let opts = OpenOptions::new().read_only(true);
        match Minigraf::open_with_options(&path, opts) {
            Ok(db) => {
                check_tx_count(m, &db, tx_count, "read-only", failures);
                check_queries(m, &db, "read-only", failures);
            }
            Err(e) => failures.push(format!(
                "{}: read-only open failed with {}",
                m.name,
                e.code()
            )),
        }
        if std::fs::read(&path).unwrap() != src_bytes {
            failures.push(format!("{}: read-only open modified {file}", m.name));
        }
        let wal_now = std::fs::read(wal_path(&path)).ok();
        if wal_now != wal_bytes {
            failures.push(format!("{}: read-only open changed the WAL", m.name));
        }
    }

    {
        let db = match Minigraf::open(&path) {
            Ok(db) => db,
            Err(e) => {
                failures.push(format!("{}: open failed with {}", m.name, e.code()));
                return;
            }
        };
        check_tx_count(m, &db, tx_count, "open", failures);
        check_queries(m, &db, "open", failures);
    }

    // On v3, the first open migrates a v7 file to v8 in place.
    if !has_v8_meta(&path) {
        failures.push(format!("{}: no v8 meta page after open", m.name));
    }

    // Reopen the migrated copy, then checkpoint.
    {
        let db = Minigraf::open(&path).unwrap();
        check_tx_count(m, &db, tx_count, "migrated", failures);
        check_queries(m, &db, "migrated", failures);
        db.checkpoint().unwrap();
    }

    // 4. Persist: reopen after the checkpoint, which removes the WAL, including
    // one whose entries are all checkpointed already (`"stale_wal": true`, #457).
    if wal_path(&path).exists() {
        failures.push(format!("{}: WAL still present after checkpoint", m.name));
    }
    {
        let db = Minigraf::open(&path).unwrap();
        check_tx_count(m, &db, tx_count, "reopen", failures);
        check_queries(m, &db, "reopen", failures);

        // 5. Write after: the counter moves on from the file's floor (#447).
        db.execute("(transact [[:golden/probe :golden/n 1]])")
            .unwrap();
        check_tx_count(m, &db, tx_count + 1, "write", failures);
    }
    {
        let db = Minigraf::open(&path).unwrap();
        check_tx_count(m, &db, tx_count + 1, "write-reopen", failures);
        match db.execute("(query [:find ?n :where [:golden/probe :golden/n ?n]])") {
            Ok(QueryResult::QueryResults { results, .. }) if results.len() == 1 => {}
            _ => failures.push(format!("{} [write-reopen]: probe fact missing", m.name)),
        }
        check_queries(m, &db, "write-reopen", failures);
    }

    // 6. Untouched: the committed files are unchanged.
    if std::fs::read(&src).unwrap() != src_bytes {
        failures.push(format!("{}: committed {file} was modified", m.name));
    }
}

#[test]
fn golden_corpus() {
    let manifests = load_manifests();
    assert!(!manifests.is_empty(), "tests/golden holds no manifests");
    let mut failures = Vec::new();
    for m in &manifests {
        check_manifest(m, &mut failures);
    }
    assert!(
        failures.is_empty(),
        "golden corpus failures:\n{}",
        failures.join("\n")
    );
}

/// Every golden file has a manifest, so none goes unchecked.
#[test]
fn every_golden_file_has_a_manifest() {
    let manifests = load_manifests();
    let named: BTreeSet<String> = manifests
        .iter()
        .flat_map(|m| {
            let mut v = vec![m.json["file"].as_str().unwrap().to_string()];
            if let Some(w) = m.json["wal"].as_str() {
                v.push(w.to_string());
            }
            v
        })
        .collect();
    for entry in std::fs::read_dir(golden_dir()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.ends_with(".graph") || name.ends_with(".wal") {
            assert!(
                named.contains(&name),
                "golden file without a manifest: {name}"
            );
        }
    }
}
