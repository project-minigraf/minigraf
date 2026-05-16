#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = dir.path().join("fuzz.graph");
    let mut content = vec![0u8; 4096 * 3];
    content[0..4].copy_from_slice(b"MGRF");
    content[4..8].copy_from_slice(&7u32.to_le_bytes());
    let copy_len = data.len().min(4096);
    content[4096 * 2..4096 * 2 + copy_len].copy_from_slice(&data[..copy_len]);
    if std::fs::write(&path, &content).is_err() {
        return;
    }
    let _ = minigraf::db::Minigraf::open(&path);
});
