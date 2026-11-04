//! Manual #838 sweep. Setup, validation and destruction are outside timings.
use minigraf::{Cursor, Minigraf, OpenOptions, QueryResult, Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::error::Error;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const QUERY: &str = "(query [:find ?e ?a ?v :where [?e ?a ?v]])";
const IDS: &str = "(query [:find ?i :where [?e :sweep/id ?i]])";
const ROUNDS: usize = 8;

#[derive(Serialize, Deserialize)]
struct Fixture {
    file: String,
    entities: usize,
    attributes: usize,
    shape: String,
    facts: usize,
    transactions: usize,
    bytes: u64,
    sha256: String,
    writer: String,
}

fn digest(path: &Path) -> Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn payload(shape: &str, i: usize) -> serde_json::Value {
    match shape {
        "integer" => json!(i),
        "repeated256" => json!(format!("{:016}{}", i % 16, "x".repeat(240))),
        "unique256" => json!(format!("{i:016}{}", "x".repeat(240))),
        _ => panic!("unknown fixture shape"),
    }
}

fn generate(out: &Path, smoke: bool) -> Result<()> {
    // Refuse to replace evidence, including a partially generated ladder.
    std::fs::create_dir(out)?;
    let ladder: &[(usize, usize)] = if smoke {
        &[(100, 16)]
    } else {
        &[(1_000, 16), (10_000, 256), (100_000, 4096)]
    };
    let mut fixtures = Vec::new();
    for &(entities, attributes) in ladder {
        for shape in ["integer", "repeated256", "unique256"] {
            let file = format!("{entities}-{attributes}-{shape}.graph");
            let path = out.join(&file);
            let db = v7::OpenOptions::new()
                .wal_checkpoint_threshold(usize::MAX)
                .path(&path)
                .open()?;
            for start in (0..entities).step_by(100) {
                let mut statement = String::from("(transact [");
                for i in start..(start + 100).min(entities) {
                    // One value per entity/attribute avoids v2's known #371.
                    statement.push_str(&format!(
                        "[:sweep/e{i} :sweep/id {i}] [:sweep/e{i} :sweep/a{} {}]",
                        i % attributes,
                        payload(shape, i)
                    ));
                }
                statement.push_str("])");
                db.execute(&statement)?;
            }
            db.checkpoint()?;
            drop(db);
            assert!(!path.with_extension("graph.wal").exists());
            let bytes = std::fs::metadata(&path)?.len();
            assert_eq!(version(&path)?, 7);
            fixtures.push(Fixture {
                file,
                entities,
                attributes,
                shape: shape.into(),
                facts: entities * 2,
                transactions: entities.div_ceil(100),
                bytes,
                sha256: digest(&path)?,
                writer: "minigraf 2.0.11 (crates.io)".into(),
            });
        }
    }
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&fixtures)?,
    )?;
    Ok(())
}

fn version(path: &Path) -> Result<u32> {
    let mut header = [0; 8];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    assert_eq!(&header[..4], b"MGRF");
    Ok(u32::from_le_bytes(header[4..].try_into()?))
}

fn rows(db: &Minigraf, query: &str) -> Result<Vec<Vec<Value>>> {
    match db.execute(query)? {
        QueryResult::QueryResults { results, .. } => Ok(results),
        _ => Err("expected query results".into()),
    }
}

fn check(db: &Minigraf, f: &Fixture, extra: usize) -> Result<()> {
    // Check every original value, not just row count or an aggregate.
    let result = rows(db, QUERY)?;
    assert_eq!(result.len(), f.facts + extra);
    let mut ids = HashMap::new();
    for row in &result {
        assert_eq!(row.len(), 3);
        assert!(matches!(row[0], Value::Ref(_)), "entity required");
        if row[1] == Value::Keyword(":sweep/id".into()) {
            let Value::Integer(i) = row[2] else {
                panic!("integer id required")
            };
            assert!(
                ids.insert(row[0].clone(), i).is_none(),
                "duplicate entity id"
            );
        }
    }
    assert_eq!(ids.len(), f.entities + extra);
    let mut seen = vec![[false; 2]; f.entities];
    let mut later = vec![false; extra];
    for row in result {
        assert_eq!(row.len(), 3);
        let i = *ids.get(&row[0]).ok_or("fact without a recipe id")?;
        let Value::Keyword(ref a) = row[1] else {
            panic!("attribute required")
        };
        if i < 0 {
            let j = usize::try_from(-i - 1)?;
            assert!(j < extra && !later[j]);
            assert_eq!(a, ":sweep/id");
            assert!(row[2] == Value::Integer(i));
            later[j] = true;
            continue;
        }
        let i = usize::try_from(i)?;
        assert!(i < f.entities);
        let slot = if a == ":sweep/id" {
            assert!(row[2] == Value::Integer(i as i64));
            0
        } else {
            assert_eq!(a, &format!(":sweep/a{}", i % f.attributes));
            let expected = payload(&f.shape, i);
            assert!(
                match &row[2] {
                    Value::Integer(v) => expected.as_i64() == Some(*v),
                    Value::String(v) => expected.as_str() == Some(v.as_str()),
                    _ => false,
                },
                "payload must match recipe"
            );
            1
        };
        assert!(!seen[i][slot], "duplicate fixture row");
        seen[i][slot] = true;
    }
    assert!(seen.iter().all(|v| v[0] && v[1]) && later.iter().all(|v| *v));
    assert_eq!(db.current_tx_count(), (f.transactions + extra) as u64);
    Ok(())
}

fn check_cursor(cursor: &mut Cursor, n: usize, extra: usize) -> Result<()> {
    let mut seen = vec![false; n + extra];
    while let Some(batch) = cursor.next_batch(256)? {
        assert!(!batch.is_empty() && batch.len() <= 256);
        for row in batch.rows() {
            assert_eq!(row.len(), 1);
            let Value::Integer(i) = row[0] else {
                panic!("integer id required")
            };
            let slot = if i < 0 {
                n + usize::try_from(-i - 1)?
            } else {
                usize::try_from(i)?
            };
            assert!(slot < seen.len() && !seen[slot], "snapshot mismatch");
            seen[slot] = true;
        }
    }
    assert!(seen.iter().all(|v| *v), "missing snapshot row");
    assert!(cursor.next_batch(1)?.is_none());
    Ok(())
}

fn open(path: &Path, read_only: bool) -> Result<Minigraf> {
    Ok(OpenOptions::new()
        .read_only(read_only)
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()?)
}

fn emit(f: &Fixture, operation: &str, sample: usize, elapsed: Option<std::time::Duration>) {
    println!(
        "{}",
        json!({"fixture":f.file, "sha256":f.sha256, "operation":operation,
        "sample":sample, "ns":elapsed.map(|d| d.as_nanos()), "correctness":"passed"})
    );
}

fn run(root: &Path, samples: usize, test: bool) -> Result<()> {
    assert!(samples > 0);
    let fixtures: Vec<Fixture> =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    for f in fixtures {
        let source = root.join(&f.file);
        assert_eq!(digest(&source)?, f.sha256);
        assert_eq!(std::fs::metadata(&source)?.len(), f.bytes);
        assert_eq!(version(&source)?, 7);
        for sample in 0..samples {
            for operation in [
                "read_only_v7",
                "migrate_v7",
                "churn_unpinned",
                "churn_pinned",
            ] {
                let scratch = tempfile::tempdir()?;
                let path = scratch.path().join("copy.graph");
                std::fs::copy(&source, &path)?;
                std::fs::File::open(&path)?.sync_all()?;
                let churn = operation.starts_with("churn");
                let read_only = operation == "read_only_v7";
                let start = (!test && !churn).then(Instant::now);
                let db = open(&path, read_only)?;
                let elapsed = start.map(|s| s.elapsed());
                check(&db, &f, 0)?;
                assert_eq!(version(&path)?, if read_only { 7 } else { 8 });
                if churn {
                    let mut old = if operation == "churn_pinned" {
                        Some(db.query(IDS)?)
                    } else {
                        None
                    };
                    let mut elapsed = std::time::Duration::ZERO;
                    for j in 0..ROUNDS {
                        // A short-lived cursor overlaps each commit alongside the old pin.
                        let mut recent = if old.is_some() {
                            Some(db.query(IDS)?)
                        } else {
                            None
                        };
                        let statement =
                            format!("(transact [[:sweep/later{j} :sweep/id -{}]])", j + 1);
                        let start = (!test).then(Instant::now);
                        db.execute(&statement)?;
                        db.checkpoint()?;
                        if let Some(s) = start {
                            elapsed += s.elapsed();
                        }
                        if let Some(c) = recent.as_mut() {
                            check_cursor(c, f.entities, j)?;
                        }
                    }
                    if let Some(c) = old.as_mut() {
                        check_cursor(c, f.entities, 0)?;
                    }
                    drop(old);
                    check(&db, &f, ROUNDS)?;
                    // Commit after releasing the pins so reclaim becomes eligible.
                    db.execute("(transact [[:sweep/later8 :sweep/id -9]])")?;
                    db.checkpoint()?;
                    assert!(db.verify()?.is_ok());
                    drop(db);
                    let reopened = open(&path, false)?;
                    check(&reopened, &f, ROUNDS + 1)?;
                    assert!(reopened.verify()?.is_ok());
                    emit(&f, operation, sample, (!test).then_some(elapsed));
                } else {
                    if !read_only {
                        let report = db.verify()?;
                        assert!(report.is_ok());
                        assert_eq!(report.facts, f.facts as u64);
                    }
                    drop(db);
                    let start = (!test && !read_only).then(Instant::now);
                    let reopened = open(&path, read_only)?;
                    let reopen_elapsed = start.map(|s| s.elapsed());
                    check(&reopened, &f, 0)?;
                    drop(reopened);
                    if read_only {
                        assert_eq!(digest(&path)?, f.sha256);
                    }
                    emit(&f, operation, sample, elapsed);
                    if !read_only {
                        emit(&f, "reopen_v8", sample, reopen_elapsed);
                    }
                }
            }
        }
        assert_eq!(digest(&source)?, f.sha256, "source fixture changed");
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let command = args
        .get(1)
        .ok_or("usage: generate DIR [--smoke] | run DIR SAMPLES [--test]")?;
    let root = Path::new(args.get(2).ok_or("missing directory")?);
    match command.as_str() {
        "generate" => generate(root, args.iter().any(|a| a == "--smoke")),
        "run" => run(
            root,
            args.get(3).ok_or("missing sample count")?.parse()?,
            args.iter().any(|a| a == "--test"),
        ),
        _ => Err("unknown command".into()),
    }
}
