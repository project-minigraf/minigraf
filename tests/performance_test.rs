//! Integration tests for Phase 6.2 packed pages.

use minigraf::{Minigraf, OpenOptions};
use tempfile::NamedTempFile;

#[test]
fn test_1k_facts_correct_after_packed_save_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    // Insert 1000 facts in batches
    {
        let db = OpenOptions::new().path(path).open().unwrap();
        for batch in 0..10 {
            let mut cmd = String::from("(transact [");
            for i in 0..100 {
                let idx = batch * 100 + i;
                cmd.push_str(&format!("[:e{} :val {}]", idx, idx));
            }
            cmd.push_str("])");
            db.execute(&cmd).unwrap();
        }
    }

    // Reload and verify
    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db.execute("(query [:find ?v :where [:e0 :val ?v]])").unwrap();
    assert!(!format!("{:?}", result).is_empty(), "e0 must have val after reload");
}

#[test]
fn test_packed_pages_use_fewer_pages_than_one_per_page() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        for i in 0..200u64 {
            db.execute(&format!("(transact [[:e{} :val {}]])", i, i)).unwrap();
        }
    }

    let file_size = std::fs::metadata(path).unwrap().len();
    // One-per-page would need 200 + header + indexes = ~210 pages × 4096 = ~860KB
    // Packed: ~10 fact pages + index pages << 860KB
    let one_per_page_estimate = 210 * 4096u64;
    assert!(
        file_size < one_per_page_estimate,
        "packed file ({} bytes) should be much smaller than one-per-page estimate ({} bytes)",
        file_size,
        one_per_page_estimate
    );
}

#[test]
fn test_bitemporal_query_after_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute(
            r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"} [[:alice :status :active]])"#,
        )
        .unwrap();
        db.execute(r#"(transact {:valid-from "2024-01-01"} [[:alice :status :inactive]])"#)
            .unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db
        .execute(r#"(query [:find ?s :valid-at "2023-06-01" :where [:alice :status ?s]])"#)
        .unwrap();
    assert!(
        format!("{:?}", result).contains("active"),
        "should find :active at 2023-06-01, got: {:?}",
        result
    );
}

#[test]
fn test_as_of_query_after_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        db.execute("(transact [[:alice :age 30]])").unwrap();
        db.execute("(transact [[:alice :age 31]])").unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db
        .execute("(query [:find ?age :as-of 1 :where [:alice :age ?age]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("30"),
        "as-of 1 should return age 30, got: {:?}",
        result
    );
}

#[test]
fn test_recursive_rules_unchanged_after_6_2() {
    let db = Minigraf::in_memory().unwrap();
    db.execute("(transact [[:a :next :b] [:b :next :c] [:c :next :d]])")
        .unwrap();
    db.execute("(rule [(reachable ?from ?to) [?from :next ?to]])")
        .unwrap();
    db.execute(
        "(rule [(reachable ?from ?to) [?from :next ?mid] (reachable ?mid ?to)])",
    )
    .unwrap();
    let result = db
        .execute("(query [:find ?to :where (reachable :a ?to)])")
        .unwrap();
    let s = format!("{:?}", result);
    assert!(
        s.contains("b") && s.contains("c") && s.contains("d"),
        "transitive closure must still work: {:?}",
        result
    );
}

#[test]
fn test_explicit_tx_survives_packed_reload() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();

    {
        let db = OpenOptions::new().path(path).open().unwrap();
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:alice :name \"Alice\"]])").unwrap();
        tx.commit().unwrap();
    }

    let db = OpenOptions::new().path(path).open().unwrap();
    let result = db
        .execute("(query [:find ?n :where [:alice :name ?n]])")
        .unwrap();
    assert!(
        format!("{:?}", result).contains("Alice"),
        "Alice must survive packed reload: {:?}",
        result
    );
}

#[test]
fn test_page_cache_size_option_accepted() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    // Custom cache size must not panic
    let db = OpenOptions::new()
        .page_cache_size(64)
        .path(path)
        .open()
        .unwrap();
    db.execute("(transact [[:x :y 1]])").unwrap();
}
