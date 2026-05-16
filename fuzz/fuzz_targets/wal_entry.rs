#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let db_path = dir.path().join("fuzz.graph");
    let wal_path = dir.path().join("fuzz.graph.wal");
    if std::fs::write(&wal_path, data).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&db_path);
});
