//! The existing #323/#379 point_query_chain_depth fixture, built separately
//! from query-only timing/RSS measurement. Link against the comparison base.
//! Usage: history_fixture <new.graph> <depth>

use minigraf::Minigraf;
use std::path::Path;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(&args[1]);
    let depth: usize = args[2].parse().unwrap();
    assert!(depth > 0 && !path.exists(), "use a fresh fixture path");
    let db = Minigraf::open(path).unwrap();
    for i in 0..2000 {
        db.execute(&format!("(transact [[:f/{i} :x {i}]])"))
            .unwrap();
    }
    db.execute("(transact [[:e/hot :other \"o\"]])").unwrap();
    db.execute("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [[:e/hot :hash \"h0\"]])")
        .unwrap();
    assert_eq!(db.current_tx_count(), 2002, "the as-of fixture boundary");
    for i in 1..depth {
        db.execute(&format!("(retract [[:e/hot :hash \"h{}\"]])", i - 1))
            .unwrap();
        db.execute(&format!(
            "(transact {{:valid-from \"2020-01-01T00:00:00Z\"}} [[:e/hot :hash \"h{i}\"]])"
        ))
        .unwrap();
    }
    db.checkpoint().unwrap();
    drop(db);
    println!(
        "{{\"depth\":{depth},\"file_bytes\":{}}}",
        std::fs::metadata(path).unwrap().len()
    );
}
