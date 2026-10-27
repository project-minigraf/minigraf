//! Reproducible UUID projection fixture/query probe for #530.
//!
//! Compile against either comparison library with rustc (docs/BENCHMARKS.md).
//! Usage: probe build <new.graph> <entities>
//!        probe query <existing.graph> '<query>' <expected-rows>
//! Query timings exclude file open and thread creation; the worker stack is 256 KiB.

use minigraf::{Minigraf, OpenOptions};
use std::time::{Duration, Instant};

fn uuid(i: usize) -> String {
    format!("00000000-0000-0000-0000-{i:012x}")
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args[1] == "build" {
        let n: usize = args[3].parse().unwrap();
        assert!(
            !std::path::Path::new(&args[2]).exists(),
            "fixture must be built from scratch"
        );
        let db = Minigraf::open(&args[2]).unwrap();
        for start in (0..n).step_by(1000) {
            let mut command = String::from("(transact [");
            for i in start..(start + 1000).min(n) {
                command.push_str(&format!("[#uuid \"{}\" :friend #uuid \"{}\"] [#uuid \"{}\" :boss #uuid \"{}\"] [#uuid \"{}\" :bucket {}]", uuid(i), uuid((i*7919+1)%n), uuid(i), uuid(i/100), uuid(i), i%100));
            }
            command.push_str("])");
            db.execute(&command).unwrap();
        }
        db.checkpoint().unwrap();
        return;
    }
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let db = OpenOptions::new()
                .read_only(true)
                .path(&args[2])
                .open()
                .unwrap();
            let expected: usize = args[4].parse().unwrap();
            let drain = || {
                let mut cursor = db.query(&args[3]).unwrap();
                let mut count = 0;
                while let Some(batch) = cursor.next_batch(1000).unwrap() {
                    count += batch.len();
                }
                assert_eq!(count, expected);
            };
            for _ in 0..3 {
                drain();
            }
            let start = Instant::now();
            let mut times = Vec::new();
            while times.len() < 200 && (times.len() < 5 || start.elapsed() < Duration::from_secs(5))
            {
                let t = Instant::now();
                drain();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "runs={},p50_ms={:.6},max_ms={:.6},rows={}",
                times.len(),
                times[times.len() / 2],
                times.last().unwrap(),
                expected
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
