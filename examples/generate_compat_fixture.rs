//! Generates a `.graph` file with two known facts, for inspecting what the
//! current storage layer writes.
//!
//!   cargo run --example generate_compat_fixture
//!
//! `tests/fixtures/compat.graph` is the frozen v7 fixture written by Minigraf
//! v2.0.0; the cross-platform tests use it to check v7→v8 migration on open.
//! This example must never overwrite it — running it now would write a v8
//! file, silently invalidating that migration coverage. It writes to
//! `target/generate_compat_fixture/compat_generated.graph` instead, which is
//! not read by any test.

// wasm-pack compiles examples for the browser target; provide a no-op entry
// point so the example compiles cleanly. The actual generator only makes sense
// on native (it needs the file system and Minigraf::open).
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let out_dir = PathBuf::from(manifest_dir).join("target/generate_compat_fixture");
    std::fs::create_dir_all(&out_dir)?;
    let fixture_path = out_dir.join("compat_generated.graph");
    let tmp_path = fixture_path.with_extension("graph.tmp");

    // Remove any leftover from a previous run.
    let _ = std::fs::remove_file(&tmp_path);
    let _ = std::fs::remove_file(tmp_path.with_extension("wal"));

    // Populate with known facts.
    let db = minigraf::Minigraf::open(&tmp_path)?;
    db.execute(r#"(transact [[:alice :name "Alice"]])"#)?;
    db.execute("(transact [[:alice :age 30]])")?;
    // Checkpoint flushes WAL → main file so the bytes are self-contained.
    db.checkpoint()?;
    drop(db);

    // Remove WAL sidecar before copying.
    let wal_path = tmp_path.with_extension("graph.tmp.wal");
    let _ = std::fs::remove_file(&wal_path);

    std::fs::rename(&tmp_path, &fixture_path)?;
    println!("Written: {}", fixture_path.display());
    Ok(())
}
