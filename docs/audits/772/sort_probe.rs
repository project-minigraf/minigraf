// Standalone algorithm probe, not an end-to-end database benchmark.
// rustc -O sort_probe.rs -o sort_probe
use std::{hint::black_box, time::Instant};

fn run<T: Clone + Ord>(label: &str, values: &[T], candidate: bool, descending: bool) {
    let loops = (1_000_000 / values.len()).clamp(20, 20_000);
    let mut elapsed = 0;
    let mut expected = values.to_vec();
    expected.sort();
    if descending {
        expected.reverse();
    }
    for _ in 0..loops {
        let mut data = values.to_vec();
        let start = Instant::now();
        if descending {
            if candidate {
                data.sort_unstable();
                data.reverse();
            } else {
                data.sort_unstable_by(|a, b| b.cmp(a));
            }
        } else if candidate {
            data.sort_unstable();
        } else {
            data.sort();
        }
        elapsed += start.elapsed().as_nanos();
        assert!(data == expected, "sort output changed");
        black_box(data);
    }
    println!(
        "{label},{},{candidate},{loops},{}",
        values.len(),
        elapsed / loops as u128
    );
}

fn main() {
    let candidate = std::env::args().nth(1).as_deref() == Some("candidate");
    for n in [8, 64, 1024, 100_000] {
        for shape in ["random", "ascending", "descending", "duplicates"] {
            let mut state = 0x123456789abcdef_u64;
            let values: Vec<u64> = (0..n)
                .map(|i| match shape {
                    "ascending" => i as u64,
                    "descending" => (n - i) as u64,
                    "duplicates" => (i % 8) as u64,
                    _ => {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        state
                    }
                })
                .collect();
            run(&format!("free-{shape}"), &values, candidate, true);
            let bytes: Vec<Vec<u8>> = values.iter().map(|v| v.to_be_bytes().to_vec()).collect();
            run(&format!("bytes-{shape}"), &bytes, candidate, false);
            let strings: Vec<String> = values.iter().map(|v| format!("?slot-{v:020}")).collect();
            run(&format!("strings-{shape}"), &strings, candidate, false);
        }
    }
}
