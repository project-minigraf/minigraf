//! Committed fixed-size groups for #641 query-only measurements.
//! Usage: group_fixture <new.graph> <facts> <rows-per-group>
//! Build separately from timing/RSS. One integer :group fact per entity.
use minigraf::{EntityId, FactRecord, LogWriter, OpenOptions, Value};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let n: usize = args[2].parse().unwrap();
    let size: usize = args[3].parse().unwrap();
    assert!(n > 0 && size > 0);
    let mut writer = LogWriter::create(&args[1], OpenOptions::new()).unwrap();
    for i in 0..n {
        let tx = (i / 1000 + 1) as u64;
        writer.append(&FactRecord {
            entity: EntityId::from_u128(i as u128 + 1),
            attribute: ":group".into(),
            value: Value::Integer((i / size) as i64),
            tx_count: tx, tx_id: tx,
            valid_from: 0, valid_to: i64::MAX, asserted: true,
        }).unwrap();
    }
    writer.finish().unwrap();
    println!("{{\"facts\":{n},\"rows_per_group\":{size},\"groups\":{},\"file_bytes\":{}}}",
        n.div_ceil(size), std::fs::metadata(&args[1]).unwrap().len());
}
