#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "datalog_eval/guard.rs"]
mod guard;
use guard::{join_rows_bound, MAX_JOIN_ROWS};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // The executor holds every intermediate binding, so patterns that share no
        // variables build their whole cross product: 3^k rows over these 3 facts.
        // Negation bodies still run there, pattern by pattern in text order, so a
        // long body builds every partial binding even when its last pattern
        // matches nothing (#597). That is a known limit, not a new bug (#575,
        // fixed by #473/#549, and #546 for negation).
        // Estimate nested scopes independently: a not-join body can run without
        // outer constraints and can be repeated for each outer binding (#802).
        if join_rows_bound(s) > MAX_JOIN_ROWS {
            return;
        }
        let db = match minigraf::db::Minigraf::in_memory() {
            Ok(db) => db,
            Err(_) => return,
        };
        let _ = db
            .execute(r#"(transact [[:alice :name "Alice"] [:bob :name "Bob"] [:alice :age 30]])"#);
        let _ = db.execute(s);
    }
});
