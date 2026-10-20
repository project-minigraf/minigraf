//! Performance envelope (#394): query latency (p50/p99), open time, checkpoint
//! time and RSS at 100K, 1M and 10M facts.
//!
//! Usage:
//!
//! ```text
//! cargo run --profile envelope --example perf_envelope -- [--scales 100k,1m,10m]
//!     [--dir target/perf-envelope] [--json out.json] [--budget-secs 15]
//!     [--cap-secs 60] [--rebuild]
//! ```
//!
//! The `envelope` profile is `release` at opt-level 3, which is how an
//! application's release build compiles the library; `release` itself (and
//! `bench`, which inherits it) is tuned for the REPL binary's size.
//!
//! Each fixture is built once and cached in `--dir`. The build, the storage
//! measurements and every query shape run in their own child process, so each
//! peak RSS is that step's alone. Storage measurements run on a copy, because
//! checkpoints write to it; queries open the fixture read-only. The OS page
//! cache is warm: a cold open needs root to drop caches.
//!
//! Each shape runs until 200 runs or `--budget-secs`, at least 5 runs; with
//! fewer than 100 runs p99 is the maximum. A shape whose first run is still
//! running after `--cap-secs` is reported as capped.
//!
//! RSS comes from `/proc/self/status` and is reported as 0 off Linux.
//! Design: `docs/superpowers/specs/2026-10-11-perf-envelope-design.md`.

// A measurement tool, not library code: a malformed argument or report is a bug
// in the tool and may panic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

#[cfg(not(target_arch = "wasm32"))]
use minigraf::{Minigraf, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value as Json, json};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// Bump when the fixture's contents change, so stale cached files are not reused.
#[cfg(not(target_arch = "wasm32"))]
const FIXTURE_VERSION: u32 = 1;
/// Facts per entity (see `build`).
#[cfg(not(target_arch = "wasm32"))]
const FACTS_PER_ENTITY: usize = 6;
/// Entities per transaction.
#[cfg(not(target_arch = "wasm32"))]
const BATCH_ENTITIES: usize = 200;
/// Explicit checkpoint interval during the build, in facts; keeps RSS bounded.
#[cfg(not(target_arch = "wasm32"))]
const BUILD_CHECKPOINT_FACTS: usize = 200_000;
/// Retract/reassert rounds of `:hot :counter`.
#[cfg(not(target_arch = "wasm32"))]
const HOT_HISTORY: usize = 1_000;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("__build") => {
            let out = build(Path::new(&args[1]), args[2].parse()?)?;
            println!("{out}");
            Ok(())
        }
        Some("__storage") => {
            let out = measure_storage(Path::new(&args[1]), Path::new(&args[2]))?;
            println!("{out}");
            Ok(())
        }
        Some("__query") => {
            let out = measure_query(
                Path::new(&args[1]),
                args[2].parse()?,
                args[3].parse()?,
                args[4].parse()?,
            )?;
            println!("{out}");
            Ok(())
        }
        _ => run(&args),
    }
}

// ── Parent ────────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn run(args: &[String]) -> anyhow::Result<()> {
    let mut scales = vec!["100k".to_string(), "1m".to_string(), "10m".to_string()];
    let mut dir = PathBuf::from("target/perf-envelope");
    let mut json_out: Option<PathBuf> = None;
    let mut budget_secs: u64 = 15;
    let mut cap_secs: u64 = 60;
    let mut rebuild = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{a} needs a value"))
        };
        match a.as_str() {
            "--scales" => scales = val()?.split(',').map(str::to_string).collect(),
            "--dir" => dir = PathBuf::from(val()?),
            "--json" => json_out = Some(PathBuf::from(val()?)),
            "--budget-secs" => budget_secs = val()?.parse()?,
            "--cap-secs" => cap_secs = val()?.parse()?,
            "--rebuild" => rebuild = true,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?;

    let mut results = Vec::new();
    for label in &scales {
        let n = parse_scale(label)?;
        let fixture = dir.join(format!("fixture-v{FIXTURE_VERSION}-{label}.graph"));
        let meta = fixture.with_extension("json");
        if rebuild || !fixture.exists() || !meta.exists() {
            let _ = std::fs::remove_file(&fixture);
            let _ = std::fs::remove_file(fixture.with_extension("graph.wal"));
            eprintln!("perf_envelope: building {label} fixture ({n} facts)");
            let out = child(&exe, &["__build", path_str(&fixture), &n.to_string()])?;
            std::fs::write(&meta, out.to_string())?;
        }
        eprintln!("perf_envelope: measuring {label}");
        let copy = dir.join(format!("measure-{label}.graph"));
        let storage = child(&exe, &["__storage", path_str(&fixture), path_str(&copy)]);
        let _ = std::fs::remove_file(&copy);
        let _ = std::fs::remove_file(copy.with_extension("graph.wal"));
        let mut out = storage?;
        let meta = read_meta(&fixture)?;
        let mut queries = Vec::new();
        for (i, shape) in fixture_shapes(&meta).iter().enumerate() {
            let q = child(
                &exe,
                &[
                    "__query",
                    path_str(&fixture),
                    &i.to_string(),
                    &budget_secs.to_string(),
                    &cap_secs.to_string(),
                ],
            )?;
            match q.get("p50_ms") {
                Some(_) => eprintln!(
                    "  {:<20} p50 {:>10.3} ms  p99 {:>10.3} ms  ({} runs, {} rows)",
                    shape.name,
                    q["p50_ms"].as_f64().unwrap(),
                    q["p99_ms"].as_f64().unwrap(),
                    q["runs"],
                    q["rows"]
                ),
                None if q.get("error").is_some() => {
                    eprintln!("  {:<20} fails: {}", shape.name, q["error"])
                }
                None => eprintln!("  {:<20} capped at {} ms", shape.name, q["capped_ms"]),
            }
            queries.push(q);
        }
        out["queries"] = json!(queries);
        out["label"] = json!(label);
        out["build"] = meta;
        results.push(out);
    }

    let report = json!({
        "minigraf": env!("CARGO_PKG_VERSION"),
        "fixture_version": FIXTURE_VERSION,
        "budget_secs": budget_secs,
        "cap_secs": cap_secs,
        "scales": results,
    });
    if let Some(p) = json_out {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    print_markdown(&report);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn child(exe: &Path, args: &[&str]) -> anyhow::Result<Json> {
    let out = std::process::Command::new(exe)
        .args(args)
        .stderr(std::process::Stdio::inherit())
        .output()?;
    anyhow::ensure!(out.status.success(), "child {} failed", args[0]);
    let stdout = String::from_utf8(out.stdout)?;
    let last = stdout.lines().last().unwrap_or_default();
    Ok(serde_json::from_str(last)?)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_scale(label: &str) -> anyhow::Result<usize> {
    let l = label.to_ascii_lowercase();
    let (num, mul) = if let Some(n) = l.strip_suffix('m') {
        (n, 1_000_000)
    } else if let Some(n) = l.strip_suffix('k') {
        (n, 1_000)
    } else {
        (l.as_str(), 1)
    };
    let n = num.parse::<usize>()? * mul;
    anyhow::ensure!(
        n >= FACTS_PER_ENTITY * BATCH_ENTITIES,
        "scale {label} is below {} facts",
        FACTS_PER_ENTITY * BATCH_ENTITIES
    );
    Ok(n)
}

#[cfg(not(target_arch = "wasm32"))]
fn path_str(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

#[cfg(not(target_arch = "wasm32"))]
fn print_markdown(report: &Json) {
    let scales = report["scales"].as_array().unwrap();
    let labels: Vec<&str> = scales
        .iter()
        .map(|s| s["label"].as_str().unwrap())
        .collect();
    let header = |first: &str| {
        let mut h = format!("| {first} |");
        let mut sep = String::from("|---|");
        for l in &labels {
            h.push_str(&format!(" {l} |"));
            sep.push_str("---:|");
        }
        println!("{h}\n{sep}");
    };
    let row = |name: &str, f: &dyn Fn(&Json) -> String| {
        let mut r = format!("| {name} |");
        for s in scales {
            r.push_str(&format!(" {} |", f(s)));
        }
        println!("{r}");
    };

    println!("### Storage\n");
    header("Metric");
    row("Facts", &|s| s["build"]["facts"].to_string());
    row("File size", &|s| {
        mib(s["build"]["file_bytes"].as_u64().unwrap())
    });
    row("Build time", &|s| {
        secs(s["build"]["build_ms"].as_f64().unwrap())
    });
    row("Build peak RSS", &|s| {
        kib(s["build"]["peak_rss_kb"].as_u64().unwrap())
    });
    row("Open", &|s| ms(s["open_ms"].as_f64().unwrap()));
    row("RSS after open", &|s| {
        kib(s["rss_after_open_kb"].as_u64().unwrap())
    });
    row("Checkpoint, 1 new fact (p50 / p99)", &|s| {
        let c = &s["checkpoint_1_fact"];
        format!(
            "{} / {}",
            ms(c["p50_ms"].as_f64().unwrap()),
            ms(c["p99_ms"].as_f64().unwrap())
        )
    });
    row("Checkpoint, 10K new facts", &|s| {
        ms(s["checkpoint_10k_ms"].as_f64().unwrap())
    });
    row("Peak RSS (open + checkpoints)", &|s| {
        kib(s["peak_rss_kb"].as_u64().unwrap())
    });

    println!("\n### Query latency (p50 / p99)\n");
    header("Shape");
    let names: Vec<String> = scales[0]["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["name"].as_str().unwrap().to_string())
        .collect();
    for name in &names {
        row(&format!("`{name}`"), &|s| {
            let q = query(s, name);
            match (q["p50_ms"].as_f64(), q["error"].as_str()) {
                (Some(p50), _) => format!("{} / {}", ms(p50), ms(q["p99_ms"].as_f64().unwrap())),
                (None, Some(e)) => format!("fails: {}", error_code(e)),
                (None, None) => format!("> {} s", report["cap_secs"]),
            }
        });
    }

    println!("\n### Rows, runs and peak RSS per shape\n");
    header("Shape");
    for name in &names {
        row(&format!("`{name}`"), &|s| {
            let q = query(s, name);
            format!(
                "{} rows, {} runs, {}",
                q.get("rows").map_or("?".into(), Json::to_string),
                q.get("runs").map_or("0".into(), Json::to_string),
                kib(q["peak_rss_kb"].as_u64().unwrap())
            )
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn query<'a>(scale: &'a Json, name: &str) -> &'a Json {
    scale["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["name"] == name)
        .unwrap()
}

/// The `[XXX-NNN]` code at the start of a `MinigrafError` message, or the message.
#[cfg(not(target_arch = "wasm32"))]
fn error_code(msg: &str) -> &str {
    msg.strip_prefix('[')
        .and_then(|m| m.split_once(']'))
        .map_or(msg, |(code, _)| code)
}

#[cfg(not(target_arch = "wasm32"))]
fn ms(v: f64) -> String {
    if v < 1.0 {
        format!("{:.0} µs", v * 1000.0)
    } else if v < 1000.0 {
        format!("{v:.2} ms")
    } else {
        secs(v)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn secs(v: f64) -> String {
    format!("{:.2} s", v / 1000.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn kib(v: u64) -> String {
    mib(v * 1024)
}

#[cfg(not(target_arch = "wasm32"))]
fn mib(v: u64) -> String {
    format!("{:.0} MiB", v as f64 / (1024.0 * 1024.0))
}

// ── Build ─────────────────────────────────────────────────────────────────────

/// Builds `n` facts over `n / 6` entities `:p{i}` (six facts each) plus a hot
/// entity with a long history, fully checkpointed. Returns the build metadata
/// the measurements need.
#[cfg(not(target_arch = "wasm32"))]
fn build(path: &Path, n: usize) -> anyhow::Result<Json> {
    let start = Instant::now();
    let e = n / FACTS_PER_ENTITY;
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()?;
    let mut since_checkpoint = 0;
    let mut tx_half = 0;
    for lo in (0..e).step_by(BATCH_ENTITIES) {
        let hi = (lo + BATCH_ENTITIES).min(e);
        let mut cmd = String::from("(transact [");
        for i in lo..hi {
            let follows = (i * 7919 + 1) % e;
            cmd.push_str(&format!(
                "[:p{i} :name \"p{i}\"][:p{i} :age {}][:p{i} :group {}][:p{i} :follows :p{follows}]",
                i % 90,
                i % 100
            ));
            // :p0 is the root of the :manager tree.
            if i > 0 {
                cmd.push_str(&format!("[:p{i} :manager :p{}]", i / 10));
            }
        }
        cmd.push_str("])");
        db.execute(&cmd)?;
        let mut a = String::from(
            "(transact {:valid-from \"2020-01-01T00:00:00Z\" :valid-to \"2023-01-01T00:00:00Z\"} [",
        );
        let mut b = String::from("(transact {:valid-from \"2023-01-01T00:00:00Z\"} [");
        for i in lo..hi {
            a.push_str(&format!("[:p{i} :status \"a\"]"));
            b.push_str(&format!("[:p{i} :status \"b\"]"));
        }
        a.push_str("])");
        b.push_str("])");
        db.execute(&a)?;
        db.execute(&b)?;
        if lo < e / 2 && hi >= e / 2 {
            tx_half = db.current_tx_count();
        }
        since_checkpoint += (hi - lo) * FACTS_PER_ENTITY;
        if since_checkpoint >= BUILD_CHECKPOINT_FACTS {
            db.checkpoint()?;
            since_checkpoint = 0;
        }
    }
    db.execute("(transact [[:hot :counter 0]])")?;
    let mut hot_tx_half = 0;
    for i in 1..HOT_HISTORY {
        db.execute(&format!("(retract [[:hot :counter {}]])", i - 1))?;
        db.execute(&format!("(transact [[:hot :counter {i}]])"))?;
        if i == HOT_HISTORY / 2 {
            hot_tx_half = db.current_tx_count();
        }
    }
    db.checkpoint()?;
    drop(db);
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "facts": n,
        "entities": e,
        "tx_half": tx_half,
        "hot_tx_half": hot_tx_half,
        "build_ms": build_ms,
        "peak_rss_kb": proc_status_kb("VmHWM"),
        "file_bytes": std::fs::metadata(path)?.len(),
    }))
}

// ── Measure ───────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
struct Shape {
    name: &'static str,
    query: String,
    /// Expected row count, checked on every run.
    rows: usize,
}

#[cfg(not(target_arch = "wasm32"))]
fn shapes(e: usize, tx_half: u64, hot_tx_half: u64) -> Vec<Shape> {
    let m = e / 2;
    let leaf = e - 1;
    // :manager depth from the leaf to :p0.
    let mut depth = 0;
    let mut i = leaf;
    while i > 0 {
        i /= 10;
        depth += 1;
    }
    // Entities whose manager is :p{m / 10}: 10 siblings, all below `e` for m = e/2.
    let siblings = (10 * (m / 10)..10 * (m / 10) + 10)
        .filter(|&s| s > 0)
        .count();
    vec![
        Shape {
            name: "point_entity",
            query: format!("[:find ?a :where [:p{m} :age ?a]]"),
            rows: 1,
        },
        Shape {
            name: "attribute_value",
            query: format!("[:find ?e :where [?e :name \"p{m}\"]]"),
            rows: 1,
        },
        Shape {
            name: "attribute_scan",
            query: "[:find ?e ?a :where [?e :age ?a]]".into(),
            rows: e,
        },
        Shape {
            name: "join_2",
            query: format!("[:find ?n :where [:p{m} :follows ?f] [?f :name ?n]]"),
            rows: 1,
        },
        Shape {
            name: "join_3_selective",
            query: format!("[:find ?n :where [:p{m} :manager ?b] [?s :manager ?b] [?s :name ?n]]"),
            rows: siblings,
        },
        Shape {
            name: "join_3_scan",
            query: "[:find ?e ?n :where [?e :follows ?f] [?f :group 7] [?f :name ?n]]".into(),
            // Not checked: depends on how :follows maps onto group 7.
            rows: usize::MAX,
        },
        Shape {
            name: "join_2_pseudo",
            query: format!(
                "[:find ?n ?t :valid-at :any-valid-time \
                 :where [:p{m} :follows ?f] [?f :name ?n] [?f :db/tx-count ?t]]"
            ),
            rows: 1,
        },
        // #546: a `not` probed once per outer row (its entity is imported).
        Shape {
            name: "not_probe",
            query: "[:find ?e :where [?e :group 7] (not [?e :age 7])]".into(),
            rows: (0..e).filter(|i| i % 100 == 7 && i % 90 != 7).count(),
        },
        // #546: a closed `not-join` keyed on a value: its body read once.
        Shape {
            name: "not_join_hash",
            query: "[:find ?e :where [?e :group 7] [?e :age ?a] \
                    (not-join [?a] [?f :group 17] [?f :age ?a] [(< ?a 50)])]"
                .into(),
            rows: {
                let ages: std::collections::HashSet<usize> = (0..e)
                    .filter(|i| i % 100 == 17 && i % 90 < 50)
                    .map(|i| i % 90)
                    .collect();
                (0..e)
                    .filter(|i| i % 100 == 7 && !ages.contains(&(i % 90)))
                    .count()
            },
        },
        // #546: an `or-join` of two closed branches over different
        // groups, each read once and keyed on a value.
        Shape {
            name: "or_2",
            query: "[:find ?e :where [?e :group 7] [?e :age ?a] \
                    (or-join [?a] (and [?f :group 17] [?f :age ?a] [(< ?a 30)]) \
                    (and [?g :group 27] [?g :age ?a] [(> ?a 60)]))]"
                .into(),
            rows: {
                let ages: std::collections::HashSet<usize> = (0..e)
                    .filter(|i| (i % 100 == 17 && i % 90 < 30) || (i % 100 == 27 && i % 90 > 60))
                    .map(|i| i % 90)
                    .collect();
                (0..e)
                    .filter(|i| i % 100 == 7 && ages.contains(&(i % 90)))
                    .count()
            },
        },
        Shape {
            name: "recursive_ancestors",
            query: format!("[:find ?b :where (boss :p{leaf} ?b)]"),
            rows: depth,
        },
        Shape {
            name: "as_of",
            query: format!("[:find ?a :as-of {tx_half} :where [:p{} :age ?a]]", m / 4),
            rows: 1,
        },
        Shape {
            name: "valid_at",
            query: format!(
                "[:find ?s :valid-at \"2021-06-01T00:00:00Z\" :where [:p{m} :status ?s]]"
            ),
            rows: 1,
        },
        Shape {
            name: "count",
            query: "[:find (count ?v) :where [?e ?a ?v]]".into(),
            rows: 1,
        },
        Shape {
            name: "count_grouped",
            query: "[:find ?g (count ?e) :where [?e :group ?g]]".into(),
            rows: 100.min(e),
        },
        // #547: a window over every row of an attribute, held until the
        // input ends.
        Shape {
            name: "window",
            query: "[:find ?e (rank :over (:order-by ?a)) :where [?e :age ?a]]".into(),
            rows: e,
        },
        Shape {
            name: "hot_current",
            query: "[:find ?v :where [:hot :counter ?v]]".into(),
            rows: 1,
        },
        Shape {
            name: "hot_as_of",
            query: format!(
                "[:find ?v :as-of {hot_tx_half} :valid-at :any-valid-time :where [:hot :counter ?v]]"
            ),
            rows: 1,
        },
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn read_meta(fixture: &Path) -> anyhow::Result<Json> {
    Ok(serde_json::from_str(&std::fs::read_to_string(
        fixture.with_extension("json"),
    )?)?)
}

#[cfg(not(target_arch = "wasm32"))]
fn fixture_shapes(meta: &Json) -> Vec<Shape> {
    shapes(
        meta["entities"].as_u64().unwrap() as usize,
        meta["tx_half"].as_u64().unwrap(),
        meta["hot_tx_half"].as_u64().unwrap(),
    )
}

/// Open time and RSS after open, then checkpoint times, on a copy of the fixture.
#[cfg(not(target_arch = "wasm32"))]
fn measure_storage(fixture: &Path, copy: &Path) -> anyhow::Result<Json> {
    std::fs::copy(fixture, copy)?;
    std::fs::File::open(copy)?.sync_all()?;

    let t = Instant::now();
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(copy)
        .open()?;
    let open_ms = t.elapsed().as_secs_f64() * 1000.0;
    let rss_after_open_kb = proc_status_kb("VmRSS");

    let mut samples = Vec::new();
    for i in 0..20 {
        db.execute(&format!("(transact [[:extra/{i} :x {i}]])"))?;
        let t = Instant::now();
        db.checkpoint()?;
        samples.push(t.elapsed());
    }
    let ck1 = summarize(&mut samples);

    for lo in (0..10_000).step_by(1_000) {
        let mut cmd = String::from("(transact [");
        for i in lo..lo + 1_000 {
            cmd.push_str(&format!("[:bulk/{i} :x {i}]"));
        }
        cmd.push_str("])");
        db.execute(&cmd)?;
    }
    let t = Instant::now();
    db.checkpoint()?;
    let checkpoint_10k_ms = t.elapsed().as_secs_f64() * 1000.0;
    drop(db);

    Ok(json!({
        "open_ms": open_ms,
        "rss_after_open_kb": rss_after_open_kb,
        "checkpoint_1_fact": {"p50_ms": ck1.p50_ms, "p99_ms": ck1.p99_ms, "runs": ck1.runs},
        "checkpoint_10k_ms": checkpoint_10k_ms,
        "peak_rss_kb": proc_status_kb("VmHWM"),
    }))
}

/// Times one query shape on the fixture, opened read-only. Runs in its own
/// process, so its peak RSS is this shape's alone (plus the open baseline).
#[cfg(not(target_arch = "wasm32"))]
fn measure_query(
    fixture: &Path,
    index: usize,
    budget_secs: u64,
    cap_secs: u64,
) -> anyhow::Result<Json> {
    let shape = fixture_shapes(&read_meta(fixture)?).swap_remove(index);
    let db = OpenOptions::new().read_only(true).path(fixture).open()?;
    db.execute("(rule [(boss ?x ?y) [?x :manager ?y]])")?;
    db.execute("(rule [(boss ?x ?z) [?x :manager ?y] (boss ?y ?z)])")?;
    let q = format!("(query {})", shape.query);

    // The first run doubles as the cap check: if it is still running after
    // `cap_secs`, report the shape as capped and exit without waiting for it.
    let db = std::sync::Arc::new(db);
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let (db, q) = (db.clone(), q.clone());
        std::thread::spawn(move || {
            let t = Instant::now();
            let r = drain(&db, &q).map(|rows| (rows, t.elapsed()));
            let _ = tx.send(r);
        });
    }
    let mut out = json!({"name": shape.name, "query": shape.query});
    let (rows, first) = match rx.recv_timeout(Duration::from_secs(cap_secs)) {
        Ok(Ok(r)) => r,
        // A query error (such as a result limit) is a result, not a harness failure.
        Ok(Err(e)) => {
            out["error"] = json!(e.to_string().lines().next().unwrap_or_default());
            out["peak_rss_kb"] = json!(proc_status_kb("VmHWM"));
            return Ok(out);
        }
        Err(_) => {
            out["capped_ms"] = json!(cap_secs * 1000);
            out["peak_rss_kb"] = json!(proc_status_kb("VmHWM"));
            println!("{out}");
            std::process::exit(0);
        }
    };
    if shape.rows != usize::MAX {
        anyhow::ensure!(
            rows == shape.rows,
            "{}: expected {} rows, got {rows}",
            shape.name,
            shape.rows
        );
    }
    out["rows"] = json!(rows);
    // Warm-up: the first run plus two more, unless a run takes over a second.
    if first < Duration::from_secs(1) {
        drain(&db, &q)?;
        drain(&db, &q)?;
    }

    let budget = Duration::from_secs(budget_secs);
    let mut samples = Vec::new();
    let start = Instant::now();
    while samples.len() < 200 && (samples.len() < 5 || start.elapsed() < budget) {
        let t = Instant::now();
        let r = drain(&db, &q)?;
        samples.push(t.elapsed());
        anyhow::ensure!(r == rows, "{}: row count changed", shape.name);
    }
    let s = summarize(&mut samples);
    out["runs"] = json!(s.runs);
    out["p50_ms"] = json!(s.p50_ms);
    out["p99_ms"] = json!(s.p99_ms);
    out["max_ms"] = json!(s.max_ms);
    out["peak_rss_kb"] = json!(proc_status_kb("VmHWM"));
    Ok(out)
}

/// Runs `q` and drains its cursor in batches of 1,000 rows; returns the row count.
#[cfg(not(target_arch = "wasm32"))]
fn drain(db: &Minigraf, q: &str) -> anyhow::Result<usize> {
    let mut cursor = db.query(q)?;
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch(1_000)? {
        rows += batch.len();
    }
    Ok(rows)
}

#[cfg(not(target_arch = "wasm32"))]
struct Summary {
    runs: usize,
    p50_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

/// Nearest-rank percentiles; with fewer than 100 runs p99 is the maximum.
#[cfg(not(target_arch = "wasm32"))]
fn summarize(samples: &mut [Duration]) -> Summary {
    samples.sort();
    let at = |p: f64| {
        let rank = ((p * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
        samples[rank - 1].as_secs_f64() * 1000.0
    };
    Summary {
        runs: samples.len(),
        p50_ms: at(0.50),
        p99_ms: at(0.99),
        max_ms: at(1.0),
    }
}

/// A `kB` field of `/proc/self/status`; 0 where it does not exist.
#[cfg(not(target_arch = "wasm32"))]
fn proc_status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(field)?.strip_prefix(':').map(str::to_owned))
        })
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn main() {}
