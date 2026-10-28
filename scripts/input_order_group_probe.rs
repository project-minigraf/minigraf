//! Group-count probe compiled against both slice-1 and slice-2 libraries (#641).
//! Usage: probe <checkpointed.graph> '<query>' <expected-rows> <members-per-group>
//! Build fixtures separately with input_order_group_fixture.rs. Open/thread creation
//! are excluded from timings; one warmup, at least three timed drains per process; process RSS is query-only. Worker stack: 256 KiB.

use minigraf::{OpenOptions, Value};
use std::time::{Duration, Instant};

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        let db = OpenOptions::new().read_only(true).path(&args[1]).open().unwrap();
        let expected: usize = args[3].parse().unwrap();
        let members: i64 = args[4].parse().unwrap();
        let drain = || {
            let mut cursor = db.query(&args[2]).unwrap();
            let mut rows = 0;
            while let Some(batch) = cursor.next_batch(1000).unwrap() {
                for row in batch.rows() {
                    assert!(row == &[Value::Integer(rows as i64), Value::Integer(members)], "ordered group and count");
                    rows += 1;
                }
            }
            assert_eq!(rows, expected, "expected row count");
        };
        for _ in 0..1 { drain(); }
        let start = Instant::now();
        let mut times = Vec::new();
        while times.len() < 20_000 && (times.len() < 3 || start.elapsed() < Duration::from_secs(1)) {
            let t = Instant::now();
            drain();
            times.push(t.elapsed().as_secs_f64() * 1_000_000.0);
        }
        times.sort_by(f64::total_cmp);
        println!("{{\"runs\":{},\"median_us\":{:.3},\"min_us\":{:.3},\"max_us\":{:.3},\"rows\":{},\"peak_rss_kb\":{}}}",
            times.len(), times[times.len() / 2], times[0], times[times.len()-1], expected, rss_kb());
    }).unwrap().join().unwrap();
}
