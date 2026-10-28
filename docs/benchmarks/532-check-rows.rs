//! Independent counts for the two unchecked envelope shapes (#532).
//! Run against the pinned source; this reads the fact log, not Datalog.
use minigraf::{FactFilter, FactOrder, OpenOptions};
use serde_json::{Value, json};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let report: Value = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let mut checks = Vec::new();
    for scale_report in report["scales"].as_array().unwrap() {
        let scale = scale_report["label"].as_str().unwrap();
        let fixture = dir.join(format!("fixture-v1-{scale}.graph"));
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(fixture.with_extension("json"))?)?;
        let entities = meta["entities"].as_u64().unwrap();
        let tx_half = meta["tx_half"].as_u64().unwrap();
        let db = OpenOptions::new().read_only(true).path(&fixture).open()?;
        let mut log = db.fact_log(
            &FactFilter::new()
                .tx_range(tx_half..=tx_half)
                .order(FactOrder::Storage),
        )?;
        let cutoff = log.next_batch(1)?.unwrap()[0].tx_id;
        drop(log);
        let mut ages = db.fact_log(
            &FactFilter::new()
                .attributes([":age"])
                .order(FactOrder::Storage),
        )?;
        let mut age_count = 0_u64;
        let mut as_of_count = 0_u64;
        while let Some(batch) = ages.next_batch(1000)? {
            for record in batch {
                anyhow::ensure!(record.asserted, "fixture age unexpectedly retracted");
                age_count += 1;
                as_of_count += u64::from(record.tx_id <= cutoff);
            }
        }
        anyhow::ensure!(age_count == entities, "fixture age count mismatch");
        let join_count = (0..entities)
            .filter(|i| ((i * 7919 + 1) % entities) % 100 == 7)
            .count() as u64;
        for (name, expected) in [("join_3_scan", join_count), ("as_of_ts_scan", as_of_count)] {
            let query = scale_report["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["name"] == name)
                .unwrap();
            let observed = query["rows"].as_u64().ok_or_else(|| {
                anyhow::anyhow!("{scale} {name}: no rows in report (error or cap)")
            })?;
            anyhow::ensure!(
                expected == observed,
                "{scale} {name}: expected {expected} rows, observed {observed}"
            );
            checks
                .push(json!({"scale":scale,"shape":name,"expected":expected,"observed":observed}));
        }
    }
    println!("{}", serde_json::to_string_pretty(&checks)?);
    Ok(())
}
