//! Same-transaction multi-valued facts (#371, #287): every query path must
//! return every value, before checkpoint, after WAL replay, and after
//! checkpoint + reopen.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::{Minigraf, QueryResult, Value};
use std::collections::BTreeSet;

mod common;
use common::{CrashTx, run_crashing_child};

fn rows(r: QueryResult) -> Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults"),
    }
}

fn kw(v: &Value) -> String {
    match v {
        Value::Keyword(k) => k.clone(),
        _ => panic!("expected keyword value"),
    }
}

/// Values of `attr` on the entity `ent` through EAVT (entity bound), AEVT
/// (attribute bound, joined to a marker attribute) and a full scan.
fn three_paths(db: &Minigraf, ent: &str, attr: &str, marker: &str) -> [BTreeSet<String>; 3] {
    let eavt = rows(
        db.execute(&format!("(query [:find ?v :where [{ent} {attr} ?v]])"))
            .unwrap(),
    )
    .iter()
    .map(|r| kw(&r[0]))
    .collect();
    let aevt = rows(
        db.execute(&format!(
            r#"(query [:find ?v :where [?e {attr} ?v] [?e :note "{marker}"]])"#
        ))
        .unwrap(),
    )
    .iter()
    .map(|r| kw(&r[0]))
    .collect();
    let scan = rows(
        db.execute(&format!(
            r#"(query [:find ?a ?v :where [?e ?a ?v] [?e :note "{marker}"]])"#
        ))
        .unwrap(),
    )
    .iter()
    .filter(|r| matches!(&r[0], Value::Keyword(a) if a == attr))
    .map(|r| kw(&r[1]))
    .collect();
    [eavt, aevt, scan]
}

fn setup_statements(i: usize) -> Vec<String> {
    let mut facts = vec![
        format!("[:t/x{i} :kind :k/a]"),
        format!("[:t/x{i} :kind :k/b]"),
        format!(r#"[:t/x{i} :note "two{i}"]"#),
    ];
    for f in 0..30 {
        facts.push(format!("[:t/f{i}-{f} :kind :k/c]"));
        facts.push(format!(r#"[:t/f{i}-{f} :note "f"]"#));
    }
    vec![format!("(transact [{}])", facts.join(" "))]
}

fn both() -> BTreeSet<String> {
    [":k/a", ":k/b"].iter().map(|s| s.to_string()).collect()
}

#[test]
fn multi_value_visible_on_every_path_before_checkpoint() {
    let db = Minigraf::in_memory().unwrap();
    for s in setup_statements(0) {
        db.execute(&s).unwrap();
    }
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x0", ":kind", "two0"))
    {
        assert_eq!(
            got,
            both(),
            "{name} path must return both values before checkpoint"
        );
    }
}

#[test]
fn multi_value_visible_on_every_path_after_checkpoint_and_reopen() {
    // Several graph shapes: the issue saw EAVT and AEVT disagree depending on content.
    for i in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        {
            let db = Minigraf::open(&path).unwrap();
            for s in setup_statements(i) {
                db.execute(&s).unwrap();
            }
            db.checkpoint().unwrap();
        }
        let db = Minigraf::open(&path).unwrap();
        let x = format!(":t/x{i}");
        let marker = format!("two{i}");
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, &x, ":kind", &marker))
        {
            assert_eq!(
                got,
                both(),
                "{name} path must return both values after reopen"
            );
        }
    }
}

#[test]
fn multi_value_visible_on_every_path_after_wal_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.graph");
    let stmts = setup_statements(0);
    let refs: Vec<&str> = stmts.iter().map(String::as_str).collect();
    run_crashing_child(&path, 1_000_000, &refs, CrashTx::Implicit);
    let db = Minigraf::open(&path).unwrap();
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x0", ":kind", "two0"))
    {
        assert_eq!(
            got,
            both(),
            "{name} path must return both values after WAL replay"
        );
    }
}

#[test]
fn batched_retract_of_both_values_hides_both_on_every_path() {
    for i in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        {
            let db = Minigraf::open(&path).unwrap();
            for s in setup_statements(i) {
                db.execute(&s).unwrap();
            }
            db.checkpoint().unwrap();
        }
        {
            let db = Minigraf::open(&path).unwrap();
            db.execute(&format!(
                "(retract [[:t/x{i} :kind :k/a] [:t/x{i} :kind :k/b]])"
            ))
            .unwrap();
            db.checkpoint().unwrap();
        }
        let db = Minigraf::open(&path).unwrap();
        let x = format!(":t/x{i}");
        let marker = format!("two{i}");
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, &x, ":kind", &marker))
        {
            assert!(
                got.is_empty(),
                "{name} path must hide both retracted values"
            );
        }
    }
}

/// Retracting exactly the values that were asserted together (same
/// order or not, one transaction or split across several) is not, on its
/// own, a sufficient regression guard for the pre-fix dedup key: the old
/// `(entity, attribute, tx_count, asserted)` key — no value — collapses
/// every fact sharing that tuple down to whichever one sorts first by
/// encoded value bytes. When the retracted set is *exactly* the asserted
/// set, that same smallest-byte member wins the collision on both the
/// assert side and the retract side, so it always cancels itself out
/// correctly and the net result is empty either way — this was verified
/// empirically (reordering the retract list, and splitting the two
/// retracts across separate transactions, both still pass against the
/// pre-fix key). What actually exercises the retract-side collision is a
/// batched retract whose value set is *not* identical to what was
/// asserted: here, `:k/A` (uppercase — encodes to fewer bytes than
/// `:k/a`) is retracted alongside the two real values but was never
/// itself asserted. Under the pre-fix key, `:k/A` wins the three-way
/// collision in the retract batch, and the real retraction records for
/// `:k/a` and `:k/b` are silently dropped — leaving `:k/a` visible.
#[test]
fn batched_retract_with_non_asserted_colliding_value_hides_real_values_on_every_path() {
    for i in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.graph");
        {
            let db = Minigraf::open(&path).unwrap();
            for s in setup_statements(i) {
                db.execute(&s).unwrap();
            }
            db.checkpoint().unwrap();
        }
        {
            let db = Minigraf::open(&path).unwrap();
            db.execute(&format!(
                "(retract [[:t/x{i} :kind :k/A] [:t/x{i} :kind :k/a] [:t/x{i} :kind :k/b]])"
            ))
            .unwrap();
            db.checkpoint().unwrap();
        }
        let db = Minigraf::open(&path).unwrap();
        let x = format!(":t/x{i}");
        let marker = format!("two{i}");
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, &x, ":kind", &marker))
        {
            assert!(
                got.is_empty(),
                "{name} path must hide real values even when the retract batch also \
                 contains a never-asserted colliding value"
            );
        }
    }
}

/// Same construction as
/// `batched_retract_with_non_asserted_colliding_value_hides_real_values_on_every_path`,
/// checked on the same handle immediately after the retract — no
/// intervening checkpoint or reopen — to cover the pending in-memory
/// index path as well as the on-disk one.
#[test]
fn batched_retract_with_non_asserted_colliding_value_hides_real_values_before_checkpoint() {
    let db = Minigraf::in_memory().unwrap();
    for s in setup_statements(0) {
        db.execute(&s).unwrap();
    }
    db.execute("(retract [[:t/x0 :kind :k/A] [:t/x0 :kind :k/a] [:t/x0 :kind :k/b]])")
        .unwrap();
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x0", ":kind", "two0"))
    {
        assert!(
            got.is_empty(),
            "{name} path must hide real values even when the retract batch also \
             contains a never-asserted colliding value, before checkpoint"
        );
    }
}

/// `tests/fixtures/v7_multivalue.graph` was written by the v7 writer at
/// 84bf375 (v2.0.0 format) with this generator:
///
/// ```ignore
/// fn main() -> anyhow::Result<()> {
///     use std::path::PathBuf;
///     let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
///     let tmp = dir.join("v7_multivalue.graph.tmp");
///     let _ = std::fs::remove_file(&tmp);
///     let _ = std::fs::remove_file(dir.join("v7_multivalue.graph.tmp.wal"));
///
///     let db = minigraf::Minigraf::open(&tmp)?;
///     let mut facts = vec![
///         "[:t/x :kind :k/a]".to_string(),
///         "[:t/x :kind :k/b]".to_string(),
///         r#"[:t/x :note "two"]"#.to_string(),
///     ];
///     for i in 0..30 {
///         facts.push(format!("[:t/f-{i} :kind :k/c]"));
///         facts.push(format!(r#"[:t/f-{i} :note "f"]"#));
///     }
///     db.execute(&format!("(transact [{}])", facts.join(" ")))?;
///     db.execute("(transact [[:t/y :tag :g/a] [:t/y :tag :g/b]])")?;
///     db.execute("(retract [[:t/y :tag :g/a] [:t/y :tag :g/b]])")?;
///     db.checkpoint()?;
///     drop(db);
///     let _ = std::fs::remove_file(dir.join("v7_multivalue.graph.tmp.wal"));
///     std::fs::rename(&tmp, dir.join("v7_multivalue.graph"))?;
///     Ok(())
/// }
/// ```
///
/// Opening it must migrate to v8 and return both values on every path, and
/// the batched retract of `:t/y`'s two `:tag` values must hide both.
#[test]
fn v7_multivalue_fixture_migrates_and_reads_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v7.graph");
    std::fs::write(&path, include_bytes!("fixtures/v7_multivalue.graph")).unwrap();

    {
        let db = Minigraf::open(&path).unwrap();
        for (name, got) in ["eavt", "aevt", "scan"]
            .iter()
            .zip(three_paths(&db, ":t/x", ":kind", "two"))
        {
            assert_eq!(
                got,
                both(),
                "{name} path must return both values after migration"
            );
        }
        let tags = rows(
            db.execute("(query [:find ?v :where [:t/y :tag ?v]])")
                .unwrap(),
        );
        assert!(
            tags.is_empty(),
            "batched retract in the v7 file must hide both values"
        );
    }

    let raw = std::fs::read(&path).unwrap();
    assert_eq!(
        u32::from_le_bytes(raw[4..8].try_into().unwrap()),
        8,
        "file must be v8 after open"
    );

    // Second open is a plain v8 open (no rebuild) and reads the same.
    let db = Minigraf::open(&path).unwrap();
    for (name, got) in ["eavt", "aevt", "scan"]
        .iter()
        .zip(three_paths(&db, ":t/x", ":kind", "two"))
    {
        assert_eq!(
            got,
            both(),
            "{name} path must return both values on second open"
        );
    }
}

/// Two valid-time stints for the *same* (entity, attribute, value) written in
/// one transaction must both be individually visible: a `:valid-at` query
/// inside the second window's range must return that stint (not the first,
/// not neither), and `:any-valid-time` must return both, on every selective
/// query path. The pre-fix `selective_fact_fetch` dedup key
/// `(entity, attribute, tx_count, asserted, value_bytes)` has no valid-time
/// component, so these two facts — same tx_count (one transact call), same
/// value bytes (`true`), same asserted flag — collapse into a single
/// arbitrary survivor in the entity-bound and attribute+join loops. A full
/// scan does not go through `selective_fact_fetch` (its patterns are not
/// selective — unbound attribute/value), so it is unaffected and serves as
/// the control.
#[test]
fn multi_value_time_stints_visible_at_correct_valid_time_on_every_path() {
    let db = Minigraf::in_memory().unwrap();
    // The `:note` marker's valid-time window must cover both stints' windows —
    // otherwise the query's `:valid-at "2022-06-01"` filter (applied to every
    // candidate fact, including the join marker) would drop `:note` itself,
    // since its default valid_from is the real transaction wall-clock time.
    db.execute(
        r#"(transact [[:vt/alice :employed true {:valid-from "2020-01-01" :valid-to "2021-01-01"}] [:vt/alice :employed true {:valid-from "2022-01-01" :valid-to "2023-01-01"}] [:vt/alice :note "vt" {:valid-from "2000-01-01"}]])"#,
    )
    .unwrap();

    // Inside the second window only: exactly one row on every path.
    let eavt = rows(
        db.execute(r#"(query [:find ?v :valid-at "2022-06-01" :where [:vt/alice :employed ?v]])"#)
            .unwrap(),
    );
    assert_eq!(
        eavt.len(),
        1,
        "entity-bound path must return the stint valid in the second window"
    );

    let aevt = rows(
        db.execute(
            r#"(query [:find ?v :valid-at "2022-06-01" :where [?e :employed ?v] [?e :note "vt"]])"#,
        )
        .unwrap(),
    );
    assert_eq!(
        aevt.len(),
        1,
        "attribute+join path must return the stint valid in the second window"
    );

    let scan = rows(
        db.execute(
            r#"(query [:find ?a ?v :valid-at "2022-06-01" :where [?e ?a ?v] [?e :note "vt"]])"#,
        )
        .unwrap(),
    )
    .into_iter()
    .filter(|r| matches!(&r[0], Value::Keyword(a) if a == ":employed"))
    .count();
    assert_eq!(
        scan, 1,
        "full scan path must return the stint valid in the second window"
    );

    // Under :any-valid-time, both stints must be visible on the selective paths.
    let eavt_any = rows(
        db.execute(r#"(query [:find ?v :any-valid-time :where [:vt/alice :employed ?v]])"#)
            .unwrap(),
    );
    assert_eq!(
        eavt_any.len(),
        2,
        "entity-bound path must return both stints under :any-valid-time"
    );

    let aevt_any = rows(
        db.execute(
            r#"(query [:find ?v :any-valid-time :where [?e :employed ?v] [?e :note "vt"]])"#,
        )
        .unwrap(),
    );
    assert_eq!(
        aevt_any.len(),
        2,
        "attribute+join path must return both stints under :any-valid-time"
    );
}

/// Index entries now carry the value bytes; a near-maximum string value must
/// still checkpoint and read back through every index.
#[test]
fn near_max_size_values_checkpoint_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.graph");
    let big_a = "a".repeat(3_800);
    let big_b = "b".repeat(3_800);
    {
        let db = Minigraf::open(&path).unwrap();
        db.execute(&format!(
            r#"(transact [[:t/big :blob "{big_a}"] [:t/big :blob "{big_b}"] [:t/big :note "big"]])"#
        ))
        .unwrap();
        db.checkpoint().unwrap();
    }
    let db = Minigraf::open(&path).unwrap();
    let by_entity = rows(
        db.execute("(query [:find ?v :where [:t/big :blob ?v]])")
            .unwrap(),
    );
    assert_eq!(
        by_entity.len(),
        2,
        "EAVT path must return both large values"
    );
    let by_attr = rows(
        db.execute(r#"(query [:find ?v :where [?e :blob ?v] [?e :note "big"]])"#)
            .unwrap(),
    );
    assert_eq!(by_attr.len(), 2, "AEVT path must return both large values");
    let by_value = rows(
        db.execute(&format!(
            r#"(query [:find ?e :where [?e :blob "{big_a}"]])"#
        ))
        .unwrap(),
    );
    assert_eq!(by_value.len(), 1, "AVET path must find the large value");
}
