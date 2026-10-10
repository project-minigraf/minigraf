//! `Minigraf::verify` and `Minigraf::rebuild_indexes` through the public API (#373).
#![cfg(not(target_arch = "wasm32"))]

use minigraf::QueryResult;
use minigraf::db::Minigraf;

const PAGE_SIZE: u64 = 4096;
const PAGE_TYPE_LEAF: u8 = 0x61;

fn count(db: &Minigraf, query: &str) -> usize {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => panic!("expected query results"),
    }
}

fn people(db: &Minigraf) -> usize {
    count(db, "(query [:find ?e ?n :where [?e :person/name ?n]])")
}

fn fill(db: &Minigraf, from: usize, to: usize) {
    for i in from..to {
        db.execute(&format!(
            r#"(transact [[:p{i} :person/name "Person {i}"] [:p{i} :person/friend :p{}]])"#,
            i / 2
        ))
        .unwrap();
    }
}

#[test]
fn in_memory_database_always_verifies() {
    let db = Minigraf::in_memory().unwrap();
    fill(&db, 0, 10);
    let report = db.verify().unwrap();
    assert!(report.is_ok());
    assert_eq!(report.facts, 0, "nothing is committed to pages");
    db.rebuild_indexes().unwrap();
    assert_eq!(people(&db), 10);
}

#[test]
fn checkpointed_file_verifies_clean() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clean.graph");
    let db = Minigraf::open(&path).unwrap();
    fill(&db, 0, 200);
    db.checkpoint().unwrap();
    let report = db.verify().unwrap();
    assert!(report.is_ok(), "an intact file has no problems");
    assert_eq!(report.facts, 400);
    assert!(report.pages > 0);
}

#[test]
fn rebuild_commits_wal_entries_and_deletes_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rebuild.graph");
    let wal = dir.path().join("rebuild.graph.wal");
    {
        let db = Minigraf::open(&path).unwrap();
        fill(&db, 0, 100);
        db.checkpoint().unwrap();
        fill(&db, 100, 150);
        db.execute("(retract [[:p3 :person/name \"Person 3\"]])")
            .unwrap();
        assert!(wal.exists(), "uncheckpointed writes are in the WAL");
        db.rebuild_indexes().unwrap();
        assert!(!wal.exists(), "the rebuild commits like a checkpoint");
        assert_eq!(people(&db), 149);
        let report = db.verify().unwrap();
        assert!(report.is_ok());
        assert_eq!(report.facts, 301);
    }
    let db = Minigraf::open(&path).unwrap();
    assert_eq!(people(&db), 149);
    assert!(db.verify().unwrap().is_ok());
}

/// Flip a byte in each leaf page in turn. `verify` reports the damage as
/// STG-029; `rebuild_indexes` either repairs it (an index leaf) or refuses
/// with STG-041 (a DICT leaf), never returning success on a damaged file.
#[test]
fn damaged_leaves_are_reported_and_index_damage_is_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("clean.graph");
    {
        let db = Minigraf::open(&clean).unwrap();
        fill(&db, 0, 300);
        db.checkpoint().unwrap();
    }
    let bytes = std::fs::read(&clean).unwrap();
    let leaves: Vec<u64> = (2..bytes.len() as u64 / PAGE_SIZE)
        .filter(|&p| bytes[(p * PAGE_SIZE) as usize] == PAGE_TYPE_LEAF)
        .collect();
    assert!(leaves.len() > 4, "fixture spans several leaves");

    let (mut repaired, mut refused) = (0, 0);
    for (i, &leaf) in leaves.iter().enumerate() {
        let path = dir.path().join(format!("damaged{i}.graph"));
        let mut damaged = bytes.clone();
        damaged[(leaf * PAGE_SIZE + 2000) as usize] ^= 0x5A;
        std::fs::write(&path, &damaged).unwrap();

        let db = Minigraf::open(&path).unwrap();
        let report = db.verify().unwrap();
        assert!(!report.is_ok(), "a flipped leaf is reported");
        assert!(report.problems.iter().any(|e| e.code() == "STG-029"));
        match db.rebuild_indexes() {
            Ok(()) => {
                repaired += 1;
                assert!(db.verify().unwrap().is_ok(), "clean after rebuild");
                assert_eq!(people(&db), 300);
            }
            Err(e) => {
                refused += 1;
                assert_eq!(e.code(), "STG-041");
                drop(db);
                assert!(
                    std::fs::read(&path).unwrap() == damaged,
                    "a refused rebuild writes nothing"
                );
            }
        }
    }
    assert!(repaired > 0, "index leaves are repaired");
    assert!(refused > 0, "DICT leaves are refused");
}

/// A checkpointed file closed cleanly, with no WAL: `(path, file length)`.
fn checkpointed_file(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, u64) {
    let path = dir.join(name);
    {
        let db = Minigraf::open(&path).unwrap();
        fill(&db, 0, 200);
        db.checkpoint().unwrap();
    }
    assert!(!dir.join(format!("{name}.wal")).exists());
    let len = std::fs::metadata(&path).unwrap().len();
    (path, len)
}

fn open_code(path: &std::path::Path, read_only: bool) -> String {
    let opts = minigraf::OpenOptions::new().read_only(read_only);
    match Minigraf::open_with_options(path, opts) {
        Ok(_) => "opened".to_string(),
        Err(e) => e.code().to_string(),
    }
}

/// A file cut short after its last checkpoint (an interrupted copy) is refused
/// at open with STG-044, read-write and read-only, before a WAL exists to take
/// writes it could never checkpoint; the file is left as it was (#497).
#[test]
fn truncated_file_is_refused_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let (path, len) = checkpointed_file(dir.path(), "cut.graph");
    for pages_cut in [1, 3] {
        let cut_len = len - pages_cut * PAGE_SIZE;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(cut_len)
            .unwrap();
        assert_eq!(open_code(&path, false), "STG-044");
        assert_eq!(open_code(&path, true), "STG-044");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), cut_len);
        assert!(!dir.path().join("cut.graph.wal").exists(), "no WAL created");
    }
}

/// A file shorter than one page opens as a new database only when it is a
/// torn first write: zeros, or a prefix of the empty generation-1 meta. Any
/// other short file (here a database cut to under a page, or foreign bytes) is refused with STG-045, read-write and read-only, and is
/// left as it was; it used to be re-initialised (#506).
#[test]
fn sub_page_file_is_refused_unless_a_torn_first_write() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = checkpointed_file(dir.path(), "short.graph");
    // A second checkpoint moves page 0 off the empty generation-1 meta. A
    // file cut below a page after one checkpoint is indistinguishable from a
    // torn first write, and holds nothing to keep: every tree lies past it.
    {
        let db = Minigraf::open(&path).unwrap();
        fill(&db, 200, 210);
        db.checkpoint().unwrap();
    }
    let template = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let foreign = vec![0x5Au8; 100];
    for bytes in [&template[..2222], &template[..20], &foreign[..]] {
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(open_code(&path, false), "STG-045");
        assert_eq!(open_code(&path, true), "STG-045");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "file unchanged");
        assert!(
            !dir.path().join("short.graph.wal").exists(),
            "no WAL created"
        );
    }

    // A torn first write: init_empty's page 0 cut short, or zeros.
    let fresh = dir.path().join("fresh.graph");
    drop(Minigraf::open(&fresh).unwrap());
    let initial = std::fs::read(&fresh).unwrap();
    std::fs::remove_file(&fresh).unwrap();
    for bytes in [&initial[..8], &initial[..40], &[0u8; 300][..]] {
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(open_code(&path, true), "opened");
        let db = Minigraf::open(&path).unwrap();
        db.execute("(transact [[:a :b 1]])").unwrap();
        db.checkpoint().unwrap();
        drop(db);
        assert_eq!(std::fs::metadata(&path).unwrap().len() % PAGE_SIZE, 0);
        std::fs::remove_file(&path).unwrap();
    }
}

/// A CRC-valid meta whose `page_count` is far past the end of the file is
/// refused at open: it used to open, and its next checkpoint wrote at that
/// page id, leaving a 4 EiB sparse file (#497).
#[test]
fn meta_page_count_past_end_is_refused_and_file_never_grows() {
    let dir = tempfile::tempdir().unwrap();
    let (path, len) = checkpointed_file(dir.path(), "forged.graph");
    let mut bytes = std::fs::read(&path).unwrap();
    let generation = |page: usize| -> u64 {
        let off = page * PAGE_SIZE as usize;
        u64::from_le_bytes(bytes[off + 16..off + 24].try_into().unwrap())
    };
    let active = if generation(0) >= generation(1) { 0 } else { 1 };
    let meta = &mut bytes[active * PAGE_SIZE as usize..(active + 1) * PAGE_SIZE as usize];
    meta[24..32].copy_from_slice(&(1u64 << 50).to_le_bytes());
    meta[12..16].fill(0);
    let crc = crc32fast::hash(meta);
    meta[12..16].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();

    assert_eq!(open_code(&path, false), "STG-044");
    assert_eq!(open_code(&path, true), "STG-044");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        len,
        "file not grown"
    );
    assert!(
        !dir.path().join("forged.graph.wal").exists(),
        "no WAL created"
    );
}
