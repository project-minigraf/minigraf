//! Generates `tests/fixtures/compat.graph` — a minimal v7 `.graph` file containing
//! two known facts used by the cross-platform compatibility tests.
//!
//! Run once when the file format changes or the fixture needs regenerating:
//!   cargo run --example generate_compat_fixture
//!
//! The fixture is committed to the repository. Do not regenerate it unless the
//! v7 file format itself has changed — regenerating changes the binary and
//! every cross-platform test that embeds it via `include_bytes!`.

// wasm-pack compiles examples for the browser target; provide a no-op entry
// point so the example compiles cleanly. The actual generator only makes sense
// on native (it needs the file system and Minigraf::open).
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures/compat.graph");
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
