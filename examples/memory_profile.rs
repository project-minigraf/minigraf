//! Memory profiling target for heaptrack.
//!
//! Usage: cargo run --example memory_profile --release -- <fact_count>
//!
//! Inserts <fact_count> `:eN :val N` facts into a checkpointed file-backed DB,
//! then runs a single point-entity query. Run under heaptrack to capture
//! peak heap and allocation counts.

use minigraf::OpenOptions;

fn main() -> anyhow::Result<()> {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("profile.graph");
    let path_str = path.to_str().unwrap();

    let db = OpenOptions::new().path(path_str).open()?;

    // Insert in batches of 100, matching the bench helper pattern
    const BATCH: usize = 100;
    for batch_start in (0..n).step_by(BATCH) {
        let batch_end = (batch_start + BATCH).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :val {}]", i, i));
        }
        cmd.push_str("])");
        db.execute(&cmd)?;
    }

    db.checkpoint()?;

    // Representative point-entity query
    let _ = db.execute("(query [:find ?v :where [:e0 :val ?v]])")?;

    eprintln!("memory_profile: inserted and queried {} facts", n);
    Ok(())
}
