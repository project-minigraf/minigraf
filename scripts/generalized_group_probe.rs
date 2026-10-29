//! Query-only #781 probe, compiled against the base and implementation libraries.
//! Usage: probe <fixture> <facts> <group-size> <mode> <limit-or-0> <offset>
//! Modes: avg, avg-float, tuple-avg, hidden-avg, tuple-count, tuple-distinct.
//! Fixtures come from input_order_group_fixture.rs; no results are retained.
use minigraf::{OpenOptions, Value};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        let db = OpenOptions::new().read_only(true).path(&args[1]).open().unwrap();
        let facts: usize = args[2].parse().unwrap();
        let size: usize = args[3].parse().unwrap();
        let mode = args[4].as_str();
        let limit: usize = args[5].parse().unwrap();
        let offset: usize = args[6].parse().unwrap();
        let (find, with, extra, order) = match mode {
            "avg" => ("?g (avg ?g)", "", "", "?g"),
            "sum" => ("?g (sum ?g)", "", "", "?g"),
            "sum-float" => ("?g (sum ?v)", "", "[(+ ?g 0.5) ?v]", "?g"),
            "extrema" => ("?g (min ?g) (max ?g)", "", "", "?g"),
            "sum-distinct" => ("?g (sum-distinct ?g)", "", "", "?g"),
            "avg-float" => ("?g (avg ?v)", "", "[(+ ?g 0.5) ?v]", "?g"),
            "tuple-avg" => ("?k ?g (avg ?g)", "", "[(+ 1 1) ?k]", "?k ?g"),
            "hidden-avg" => ("?k (avg ?g)", ":with ?g", "[(+ 1 1) ?k]", "?k (avg ?g)"),
            "tuple-count" => ("?k ?g (count ?e)", "", "[(+ 1 1) ?k]", "?k ?g"),
            "tuple-distinct" => ("?g ?g (count-distinct ?e)", "", "", "?g"),
            _ => panic!("unknown mode"),
        };
        let limit_clause = if limit == 0 { String::new() } else { format!(":limit {limit}") };
        let query = format!("(query [:find {find} {with} :where [?e :group ?g] {extra} :order-by [{order}] :offset {offset} {limit_clause}])");
        let available = facts.div_ceil(size).saturating_sub(offset);
        let expected = if limit == 0 { available } else { available.min(limit) };
        let drain = || {
            let mut cursor = db.query(&query).unwrap();
            let mut rows = 0;
            while let Some(batch) = cursor.next_batch(1000).unwrap() {
                for row in batch.rows() {
                    let g = offset + rows;
                    let group = Value::Integer(g as i64);
                    let avg = Value::Float(g as f64);
                    let count = Value::Integer(size.min(facts - g * size) as i64);
                    let want = match mode {
                        "avg" => vec![group, avg],
                        "sum" => vec![group, Value::Integer((g * size.min(facts - g * size)) as i64)],
                        "sum-float" => vec![group, Value::Float((g as f64 + 0.5) * size.min(facts - g * size) as f64)],
                        "extrema" => vec![group.clone(), group.clone(), group],
                        "sum-distinct" => vec![group.clone(), group],
                        "avg-float" => vec![group, Value::Float(g as f64 + 0.5)],
                        "tuple-avg" => vec![Value::Integer(2), group, avg],
                        "hidden-avg" => vec![Value::Integer(2), avg],
                        "tuple-count" => vec![Value::Integer(2), group, count],
                        "tuple-distinct" => vec![group.clone(), group, count],
                        _ => unreachable!(),
                    };
                    assert!(row == &want, "group value and aggregate result");
                    rows += 1;
                }
            }
            assert_eq!(rows, expected);
        };
        drain();
        let start = Instant::now();
        let mut times = Vec::new();
        while times.len() < 20_000 && (times.len() < 5 || start.elapsed() < Duration::from_secs(1)) {
            let t = Instant::now();
            drain();
            times.push(t.elapsed().as_secs_f64() * 1_000_000.0);
        }
        times.sort_by(f64::total_cmp);
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let rss = status.lines().find(|s| s.starts_with("VmHWM:")).unwrap()
            .split_whitespace().nth(1).unwrap();
        println!("{{\"mode\":\"{mode}\",\"facts\":{facts},\"limit\":{limit},\"offset\":{offset},\"runs\":{},\"median_us\":{},\"min_us\":{},\"max_us\":{},\"rows\":{expected},\"peak_rss_kb\":{rss}}}",
            times.len(), times[times.len()/2], times[0], times[times.len()-1]);
    }).unwrap().join().unwrap();
}
