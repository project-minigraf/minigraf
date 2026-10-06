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
