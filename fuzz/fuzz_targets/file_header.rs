#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    let mut page = vec![0u8; 4096];
    let copy_len = data.len().min(4096);
    page[..copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &page).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&path);
});
