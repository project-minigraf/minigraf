#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let db = match minigraf::db::Minigraf::in_memory() {
            Ok(db) => db,
            Err(_) => return,
        };
        let _ = db
            .execute(r#"(transact [[:alice :name "Alice"] [:bob :name "Bob"] [:alice :age 30]])"#);
        let _ = db.execute(s);
    }
});
