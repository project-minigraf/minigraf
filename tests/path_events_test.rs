#![cfg(test)]
//! The `test-hooks` path events and memory peaks the nightly acceptance tests
//! read (#863): `minigraf::__path_events` and `Cursor::__memory_peak`.
#![cfg(all(feature = "test-hooks", not(target_arch = "wasm32")))]

use minigraf::{Minigraf, OpenOptions};

fn t(i: usize) -> String {
    let u = uuid::Uuid::from_u128(0x8410_0000_0000_0000_0000_0000_0000_0000 + i as u128);
    format!("#uuid \"{u}\"")
}

fn tree(db: &Minigraf) {
    let facts: String = (1..=1110)
        .map(|i| format!("[{} :s/parent {}]", t(i), t((i - 1) / 10)))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    db.execute("(rule [(anc ?x ?a) [?x :s/parent ?a]])")
        .unwrap();
    db.execute("(rule [(anc ?x ?a) [?x :s/parent ?p] (anc ?p ?a)])")
        .unwrap();
}

fn drain(db: &Minigraf, query: &str) -> usize {
    let mut cursor = db.query(query).unwrap();
    let mut n = 0;
    while let Some(batch) = cursor.next_batch(1000).unwrap() {
        n += batch.rows().len();
    }
    n
}

#[test]
fn path_events_name_the_paths_queries_took() {
    let dir = tempfile::tempdir().unwrap();
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(dir.path().join("pending.graph"))
        .open()
        .unwrap();
    tree(&db);
    minigraf::__path_events();
    let q = format!("(query [:find ?a :where (anc {} ?a)])", t(1110));
    assert_eq!(drain(&db, &q), 3);
    let events = minigraf::__path_events();
    assert!(
        events
            .get("ancestor_alternative_selected")
            .is_some_and(|n| *n >= 1),
        "pending small tree takes the alternative"
    );
    assert!(minigraf::__path_events().is_empty(), "taking clears");

    db.checkpoint().unwrap();
    let demand: String = (1095..=1110)
        .map(|i| format!("[{} :s/demand true]", t(i)))
        .collect();
    db.execute(&format!("(transact [{demand}])")).unwrap();
    db.checkpoint().unwrap();
    minigraf::__path_events();
    let q = "(query [:find ?l ?a :where [?l :s/demand true] (anc ?l ?a)])";
    assert_eq!(drain(&db, q), 48);
    let events = minigraf::__path_events();
    assert!(
        events
            .get("bound_value_avet_range")
            .is_some_and(|n| *n >= 1),
        "committed tree probes bound values"
    );
}

#[test]
fn memory_peak_is_read_while_the_stream_is_open() {
    let db = Minigraf::in_memory().unwrap();
    let facts: String = (0..200)
        .map(|i| format!("[:w{i} :v {i}] [:w{i} :p {}]", i % 3))
        .collect();
    db.execute(&format!("(transact [{facts}])")).unwrap();
    let q = "(query [:find ?i ?p (sum ?i :over (:partition-by ?p :order-by ?i)) \
             :where [?e :p ?p] [?e :v ?i]])";
    let mut cursor = db.query(q).unwrap();
    let mut n = 0;
    while n < 200 {
        // Exactly the rows left: a larger request ends the stream.
        n += cursor
            .next_batch(200 - n)
            .unwrap()
            .expect("rows")
            .rows()
            .len();
    }
    let window = cursor.__memory_peak(Some("Window")).expect("open stream");
    let total = cursor.__memory_peak(None).expect("open stream");
    assert!(window > 0, "window rows were charged");
    assert!(total >= window, "total covers the window");
    assert!(cursor.next_batch(1).unwrap().is_none());
    assert_eq!(cursor.__memory_peak(None), None, "ended stream");
}
