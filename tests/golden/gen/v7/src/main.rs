//! Writes the v7 golden files in `tests/golden/` with minigraf 2.0.3 (#391).
//!
//! This program records how each committed file was made. It is never re-run to
//! replace one: golden files are only ever added. Usage:
//!
//! ```text
//! cargo run -- <out-dir>
//! ```
//!
//! It refuses to run if any target file already exists in `<out-dir>`, and prints
//! each file's CRC32 and final `tx_count` for its manifest.

use std::error::Error;
use std::path::{Path, PathBuf};

use minigraf::{Minigraf, OpenOptions};

type Res<T> = Result<T, Box<dyn Error>>;
type Recipe = fn(&Path) -> Res<()>;

const RECIPES: &[(&str, Recipe)] = &[
    ("v7_basic", basic),
    ("v7_multi_checkpoint", multi_checkpoint),
    ("v7_index_rebuilt", index_rebuilt),
    ("v7_pending_wal", pending_wal),
    ("v7_stale_wal", stale_wal),
    ("v7_multivalue", multivalue),
];

fn main() -> Res<()> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("usage: <out-dir>")?);
    for (name, _) in RECIPES {
        for ext in ["graph", "graph.wal"] {
            let p = out.join(format!("{name}.{ext}"));
            if p.exists() {
                return Err(
                    format!("{} exists; golden files are never regenerated", p.display()).into(),
                );
            }
        }
    }
    for (name, recipe) in RECIPES {
        let path = out.join(format!("{name}.graph"));
        recipe(&path)?;
        // Hash before anything else opens the file: an open can write to it.
        print!("{name}: graph=0x{:08x}", crc(&path)?);
        let wal = wal_path(&path);
        if wal.exists() {
            print!(" wal=0x{:08x}", crc(&wal)?);
        }
        println!(" tx_count={}", tx_count_of_copy(&path)?);
    }
    Ok(())
}

fn crc(path: &Path) -> Res<u32> {
    Ok(crc32fast::hash(&std::fs::read(path)?))
}

/// Reads `tx_count` from a scratch copy, so the written file stays as it is.
fn tx_count_of_copy(path: &Path) -> Res<u64> {
    let scratch = path.with_extension("probe.graph");
    std::fs::copy(path, &scratch)?;
    if wal_path(path).exists() {
        std::fs::copy(wal_path(path), wal_path(&scratch))?;
    }
    let n = open_no_auto_checkpoint(&scratch)?.current_tx_count();
    std::fs::remove_file(&scratch)?;
    let _ = std::fs::remove_file(wal_path(&scratch));
    Ok(n)
}

fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

/// Opens without the checkpoint-on-close, so only explicit checkpoints write the file.
fn open_no_auto_checkpoint(path: &Path) -> Res<Minigraf> {
    Ok(OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(path)
        .open()?)
}

/// The shared dataset: four transactions (tx_count 1..=4).
const BASE: &[&str] = &[
    // tx 1: every value type, refs, Unicode and a 2,000-byte string.
    r#"(transact [[:alice :person/name "Alice"]
                  [:alice :person/age 30]
                  [:alice :person/height 1.5]
                  [:alice :person/active true]
                  [:alice :person/role :role/admin]
                  [:alice :person/friend :bob]
                  [:alice :person/motto "Zoë 🚀"]
                  [:bob :person/name "Bob"]
                  [:bob :person/age 42]
                  [:bob :person/height -0.25]
                  [:bob :person/active false]
                  [:bob :person/role :role/user]
                  [:bob :person/friend :carol]
                  [:carol :person/name "Carol"]
                  [:carol :person/age -9223372036854775808]
                  [:carol :person/score 9223372036854775807]
                  [:carol :person/bio "BIO"]])"#,
    // tx 2: valid-time windows, one per entity.
    r#"(transact [[:carol :employment/status :intern {:valid-from "2020-01-01" :valid-to "2021-01-01"}]
                  [:alice :employment/status :employee {:valid-from "2021-01-01" :valid-to "2024-01-01"}]
                  [:bob :employment/status :contractor {:valid-from "2022-01-01"}]])"#,
    // tx 3: retraction.
    r#"(retract [[:alice :person/age 30]])"#,
    // tx 4: re-assertion with a new value.
    r#"(transact [[:alice :person/age 31]])"#,
];

/// The 2,000-byte string stored as `:carol :person/bio`.
fn bio() -> String {
    "0123456789".repeat(200)
}

fn run_base(db: &Minigraf, checkpoint_each: bool) -> Res<()> {
    for cmd in BASE {
        db.execute(&cmd.replace("\"BIO\"", &format!("\"{}\"", bio())))?;
        if checkpoint_each {
            db.checkpoint()?;
        }
    }
    Ok(())
}

/// 300 filler entities in three transacts (tx_count 5..=7), with a checkpoint
/// after each, so the facts span several fact pages and saves.
fn run_fillers(db: &Minigraf) -> Res<()> {
    for batch in 0..3 {
        let mut cmd = String::from("(transact [");
        for i in (batch * 100)..(batch * 100 + 100) {
            cmd.push_str(&format!(
                "[:f/e{i} :filler/n {i}] [:f/e{i} :filler/group :g/{}] ",
                i % 3
            ));
        }
        cmd.push_str("])");
        db.execute(&cmd)?;
        db.checkpoint()?;
    }
    Ok(())
}

/// `base`, one checkpoint. tx_count 4.
fn basic(path: &Path) -> Res<()> {
    let db = open_no_auto_checkpoint(path)?;
    run_base(&db, false)?;
    db.checkpoint()?;
    Ok(())
}

/// `base` with a checkpoint after each transaction, then the fillers. tx_count 7.
fn multi_checkpoint(path: &Path) -> Res<()> {
    let db = open_no_auto_checkpoint(path)?;
    run_base(&db, true)?;
    run_fillers(&db)?;
    Ok(())
}

/// `multi_checkpoint`, then a damaged `index_checksum` (header re-sealed), so
/// 2.0.3 rebuilds the indexes on open and rewrites the header (#370 path).
fn index_rebuilt(path: &Path) -> Res<()> {
    multi_checkpoint(path)?;
    let mut bytes = std::fs::read(path)?;
    bytes[64] ^= 0xFF; // index_checksum, bytes 64..68
    let mut h = [0u8; 80];
    h.copy_from_slice(&bytes[..80]);
    let seal = crc32fast::hash(&h); // header_checksum covers bytes 0..80
    bytes[80..84].copy_from_slice(&seal.to_le_bytes());
    std::fs::write(path, &bytes)?;
    let db = open_no_auto_checkpoint(path)?;
    drop(db);
    Ok(())
}

/// `base` checkpointed, then two transactions written to the WAL by a session that
/// ends without dropping the handle (a crash after the WAL fsync). tx_count 6.
fn pending_wal(path: &Path) -> Res<()> {
    let db = wal_session(path)?;
    // Skip every destructor, as a killed process would. The lock is released at exit.
    std::mem::forget(db);
    if !wal_path(path).exists() {
        return Err("pending_wal: expected a WAL next to the file".into());
    }
    Ok(())
}

/// As `pending_wal`, but the handle is dropped. 2.0.3 then saves the facts into the
/// file and leaves the WAL in place, so every WAL entry is already checkpointed.
/// A crash between a checkpoint's save and its WAL delete leaves the same shape
/// (#447). tx_count 6.
fn stale_wal(path: &Path) -> Res<()> {
    drop(wal_session(path)?);
    if !wal_path(path).exists() {
        return Err("stale_wal: expected a WAL next to the file".into());
    }
    Ok(())
}

fn wal_session(path: &Path) -> Res<Minigraf> {
    basic(path)?;
    let db = open_no_auto_checkpoint(path)?;
    db.execute(r#"(transact [[:dave :person/name "Dave"] [:dave :person/age 25]])"#)?;
    db.execute(r#"(retract [[:bob :person/active false]])"#)?;
    Ok(db)
}

/// The #371 shape: same-transaction multi-values and a batched retract. tx_count 3.
fn multivalue(path: &Path) -> Res<()> {
    let db = open_no_auto_checkpoint(path)?;
    let mut cmd =
        String::from(r#"(transact [[:t/x :kind :k/a] [:t/x :kind :k/b] [:t/x :note "two"] "#);
    for i in 0..30 {
        cmd.push_str(&format!(r#"[:t/f-{i} :kind :k/c] [:t/f-{i} :note "f"] "#));
    }
    cmd.push_str("])");
    db.execute(&cmd)?;
    db.execute("(transact [[:t/y :tag :g/a] [:t/y :tag :g/b]])")?;
    db.execute("(retract [[:t/y :tag :g/a] [:t/y :tag :g/b]])")?;
    db.checkpoint()?;
    Ok(())
}
